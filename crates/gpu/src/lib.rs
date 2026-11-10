//! AniCraft GPU stage renderer (layer L3, beside the engine; ADR 0019, ADR 0029).
//!
//! Draws the same scene as the CPU oracle (`anicraft-render`) with wgpu compute: the oracle
//! prepares it ([`anicraft_render::prepare_scene`]: flattening, strokes, paints and their
//! gradient tables, clips and groups), [`binning`] sorts the edges into 16 × 16 tiles on the
//! CPU, and one compute pass computes every pixel's exact area coverage and composites the
//! fills in order, inside their masks and groups (alpha and blend modes) as the oracle's band
//! compositor does. The output is the oracle's within float rounding (the parity tests check
//! mean and maximum error against it).
//!
//! What it doesn't draw yet (filters, bitmaps, nesting deeper than the shader holds) is
//! reported as [`GpuError::NeedsFull`], and the caller renders that picture on the CPU.
//!
//! Two ways to run: on a **headless device of its own** ([`GpuRenderer::new`]: tests, tools,
//! and windows that run on OpenGL), reading frames back; or on the **window's device**
//! ([`GpuRenderer::shared`]), drawing into a texture ([`GpuRenderer::render_texture`]) that
//! the window shows as it is, with no readback.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod binning;

use anicraft_doc::DisplayList;
use anicraft_render::{MAX_PIXELS, NeedsFull, Pixmap, RenderStats, View, paint::premultiply, prepare_scene};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;
use wgpu::util::DeviceExt;

#[derive(Debug, thiserror::Error)]
pub enum GpuError {
    /// No adapter, no compute shaders, or the device could not be created.
    #[error("no usable GPU: {0}")]
    Unavailable(String),
    /// This frame uses something the GPU path doesn't draw yet; render it on the CPU.
    #[error("this frame needs the CPU renderer ({0})")]
    NeedsFull(NeedsFull),
    #[error("{0}")]
    TooLarge(String),
    /// Validation, out-of-memory, mapping or a lost device.
    #[error("GPU error: {0}")]
    Device(String),
}

/// What the renderer reads back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Output {
    /// Premultiplied RGBA8, rounded like [`Pixmap::to_premul_rgba8`] (what the stage uploads).
    Premul8,
    /// Premultiplied f32 RGBA, like the oracle's [`Pixmap`] (exports and parity tests).
    Float,
}

/// A rendered frame, read back.
pub struct GpuFrame {
    pub width: u32,
    pub height: u32,
    /// Premultiplied RGBA8 ([`Output::Premul8`]).
    pub premul8: Vec<u8>,
    /// Premultiplied f32 pixels ([`Output::Float`]).
    pub pixmap: Option<Pixmap>,
    pub stats: RenderStats,
    pub timings: Timings,
    /// Tile entries and edge pieces the fine pass walked.
    pub entries: usize,
    pub pieces: usize,
}

/// A frame rendered into a texture on the renderer's device: premultiplied RGBA8
/// (`Rgba8Unorm`, the bytes [`Pixmap::to_premul_rgba8`] gives), sampleable, ready for the window
/// to show (egui's `register_native_texture`). The work is submitted, not waited for: anything
/// later on the same queue (the window's own drawing) sees it finished.
pub struct GpuTexture {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub width: u32,
    pub height: u32,
    pub stats: RenderStats,
    pub timings: Timings,
    pub entries: usize,
    pub pieces: usize,
}

/// Where a frame's time went (milliseconds, on the calling thread).
#[derive(Clone, Copy, Debug, Default)]
pub struct Timings {
    /// Cull, flatten, stroke and paint set-up (shared with the oracle, parallel).
    pub prepare: f32,
    /// Tile binning (parallel per fill and per tile row) and buffer packing.
    pub bin: f32,
    /// Upload and submit; for read-back frames also the wait for the GPU and the copy back.
    pub gpu: f32,
}

impl Timings {
    pub fn total(&self) -> f32 {
        self.prepare + self.bin + self.gpu
    }
}

/// Most work one dispatch may carry (edge pieces plus entries, summed over tiles, each walked
/// by 256 pixels). Far above real scenes (the 10k-shape 4K stress frame is about 1M), it
/// keeps a pathological frame from running long enough for the driver to reset the GPU, which
/// on a shared device would take the window with it. Such a frame is drawn on the CPU.
pub const MAX_WORK: u64 = 1 << 25;

/// Where the fine pass writes.
enum Target<'a> {
    Buffer(&'a wgpu::Buffer),
    Texture(&'a wgpu::TextureView),
}

/// The renderer: a device (its own, or the window's) and the fine pipelines.
pub struct GpuRenderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    texture_pipeline: wgpu::ComputePipeline,
    texture_layout: wgpu::BindGroupLayout,
    limits: wgpu::Limits,
    /// Adapter name and backend, for the status bar and reports.
    pub adapter: String,
    /// Whether the device is the window's (frames go to textures the window shows).
    pub shared: bool,
    /// First device failure (an error in a render, an uncaptured error or the device's loss).
    /// Once set, the renderer refuses to draw and callers fall back to the CPU.
    broken: Arc<Mutex<Option<String>>>,
}

/// Checks an adapter and device can run the stage; the device's own limits are what count
/// (a window's device may have asked for less than its adapter has).
fn check(adapter: &wgpu::Adapter, limits: &wgpu::Limits, name: &str) -> Result<(), GpuError> {
    if !adapter.get_downlevel_capabilities().flags.contains(wgpu::DownlevelFlags::COMPUTE_SHADERS) {
        return Err(GpuError::Unavailable(format!("{name} has no compute shaders")));
    }
    if limits.max_storage_buffers_per_shader_stage < 4
        || limits.max_storage_textures_per_shader_stage < 1
        || limits.max_compute_invocations_per_workgroup < 256
        || limits.max_compute_workgroup_size_x < 16
        || limits.max_compute_workgroup_size_y < 16
    {
        return Err(GpuError::Unavailable(format!("{name} is below the compute limits the stage needs")));
    }
    if !adapter.get_texture_format_features(wgpu::TextureFormat::Rgba8Unorm).allowed_usages.contains(wgpu::TextureUsages::STORAGE_BINDING) {
        return Err(GpuError::Unavailable(format!("{name} can't write RGBA8 textures from compute")));
    }
    Ok(())
}

impl GpuRenderer {
    /// Opens a headless device on the best adapter (high-performance first), builds the
    /// pipelines and checks them with a small known render. `Err` means "use the CPU".
    pub fn new() -> Result<GpuRenderer, GpuError> {
        // A renderer bug must never take the caller down: wgpu reports misuse by panicking.
        std::panic::catch_unwind(Self::open).unwrap_or_else(|_| Err(GpuError::Unavailable("the GPU driver failed while starting".into())))
    }

    /// Runs on the window's device and queue (eframe's `egui_wgpu::RenderState`), so frames
    /// can go straight into textures the window shows. Nothing is installed on the device:
    /// every call the renderer makes runs inside its own (thread-local) error scopes, the
    /// window's error handling stays as it was. `Err` means "use a device of our own, or the
    /// CPU".
    pub fn shared(adapter: &wgpu::Adapter, device: wgpu::Device, queue: wgpu::Queue) -> Result<GpuRenderer, GpuError> {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let info = adapter.get_info();
            let name = format!("{} ({:?})", info.name, info.backend);
            let limits = device.limits();
            check(adapter, &limits, &name)?;
            Self::build(device, queue, limits, name, true, Arc::new(Mutex::new(None)))
        }))
        .unwrap_or_else(|_| Err(GpuError::Unavailable("the GPU driver failed while starting".into())))
    }

    fn open() -> Result<GpuRenderer, GpuError> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions { power_preference: wgpu::PowerPreference::HighPerformance, ..Default::default() }))
            .map_err(|e| GpuError::Unavailable(e.to_string()))?;
        let info = adapter.get_info();
        let name = format!("{} ({:?})", info.name, info.backend);
        let have = adapter.limits();
        let want = wgpu::Limits {
            max_storage_buffer_binding_size: have.max_storage_buffer_binding_size,
            max_buffer_size: have.max_buffer_size,
            max_texture_dimension_2d: have.max_texture_dimension_2d,
            ..wgpu::Limits::downlevel_defaults()
        };
        check(&adapter, &have, &name)?;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor { label: Some("anicraft-gpu"), required_limits: want.clone(), ..Default::default() }))
            .map_err(|e| GpuError::Unavailable(format!("{name}: {e}")))?;
        // Our own device: its uncaptured errors and its loss are ours to catch.
        let broken: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        {
            let b = broken.clone();
            device.on_uncaptured_error(Arc::new(move |e: wgpu::Error| {
                let mut g = b.lock().unwrap_or_else(PoisonError::into_inner);
                g.get_or_insert_with(|| e.to_string());
            }));
            let b = broken.clone();
            device.set_device_lost_callback(move |_, msg| {
                let mut g = b.lock().unwrap_or_else(PoisonError::into_inner);
                g.get_or_insert_with(|| format!("device lost: {msg}"));
            });
        }
        Self::build(device, queue, want, name, false, broken)
    }

    fn build(device: wgpu::Device, queue: wgpu::Queue, limits: wgpu::Limits, name: String, shared: bool, broken: Arc<Mutex<Option<String>>>) -> Result<GpuRenderer, GpuError> {
        let scopes = Scopes::push(&device);
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("anicraft-fine"), source: wgpu::ShaderSource::Wgsl(include_str!("fine.wgsl").into()) });
        let entry = |binding: u32, ty: wgpu::BindingType| wgpu::BindGroupLayoutEntry { binding, visibility: wgpu::ShaderStages::COMPUTE, ty, count: None };
        let buffer = |ty| wgpu::BindingType::Buffer { ty, has_dynamic_offset: false, min_binding_size: None };
        let read = || buffer(wgpu::BufferBindingType::Storage { read_only: true });
        let common = [entry(0, buffer(wgpu::BufferBindingType::Uniform)), entry(1, read()), entry(2, read()), entry(3, read())];
        let make = |label: &str, out: wgpu::BindGroupLayoutEntry, entry_point: &str| {
            let mut entries = common.to_vec();
            entries.push(out);
            let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some(label), entries: &entries });
            let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some(label), bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
            let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(label),
                layout: Some(&pl),
                module: &module,
                entry_point: Some(entry_point),
                compilation_options: Default::default(),
                cache: None,
            });
            (pipeline, layout)
        };
        let (pipeline, layout) = make("anicraft-fine", entry(4, buffer(wgpu::BufferBindingType::Storage { read_only: false })), "main");
        let storage_texture = wgpu::BindingType::StorageTexture { access: wgpu::StorageTextureAccess::WriteOnly, format: wgpu::TextureFormat::Rgba8Unorm, view_dimension: wgpu::TextureViewDimension::D2 };
        let (texture_pipeline, texture_layout) = make("anicraft-fine-texture", entry(5, storage_texture), "main_texture");
        if let Some(e) = scopes.pop() {
            return Err(GpuError::Unavailable(format!("{name}: the stage shader did not build: {e}")));
        }
        let r = GpuRenderer { device, queue, pipeline, layout, texture_pipeline, texture_layout, limits, adapter: name, shared, broken };
        r.self_test()?;
        Ok(r)
    }

    /// A 24 × 20 render of two known rectangles must come back exactly, read back and through a
    /// texture: catches drivers that build the pipelines but compute garbage.
    fn self_test(&self) -> Result<(), GpuError> {
        use anicraft_doc::{ColorEffect, DisplayItem, ElementId, LayerId, Paint, Rgba};
        use anicraft_geom::{Affine, FillRule, Path};
        let item = |x: f32, c: Rgba| DisplayItem {
            path: Arc::new(Path::rect(x, 2.0, 8.0, 16.0)),
            transform: Affine::IDENTITY,
            fill: Some(Paint::Solid(c)),
            stroke: None,
            fill_rule: FillRule::NonZero,
            color: ColorEffect::default(),
            layer: LayerId(0),
            element: ElementId(0),
            outline: None,
            clip: None,
            masks: None,
            guide: false,
            group: None,
            image: None,
        };
        let dl = DisplayList { items: vec![item(2.0, Rgba::new(1.0, 0.0, 0.0, 1.0)), item(14.0, Rgba::new(0.0, 0.0, 1.0, 0.5))], ..Default::default() };
        let view = View { transform: Affine::IDENTITY, width: 24, height: 20, background: Some(Rgba::WHITE) };
        let ok = |bytes: &[u8]| {
            let px = |x: usize, y: usize| bytes.get((y * 24 + x) * 4..(y * 24 + x) * 4 + 4).map(<[u8]>::to_vec);
            px(5, 10) == Some(vec![255, 0, 0, 255]) && px(18, 10) == Some(vec![128, 128, 255, 255]) && px(12, 1) == Some(vec![255; 4])
        };
        let f = self.render(&dl, &view, Output::Premul8)?;
        let t = self.render_texture(&dl, &view)?;
        if !ok(&f.premul8) || !ok(&self.read_texture(&t)?) {
            return Err(GpuError::Unavailable(format!("{}: the self-test render came back wrong", self.adapter)));
        }
        Ok(())
    }

    /// Whether the device has failed (lost, or an error in or outside a render); then use the
    /// CPU.
    pub fn broken(&self) -> Option<String> {
        self.broken.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    fn mark_broken(&self, why: &str) {
        self.broken.lock().unwrap_or_else(PoisonError::into_inner).get_or_insert_with(|| why.to_string());
    }

    /// Renders a display list and reads it back. [`GpuError::NeedsFull`] means "draw this one
    /// on the CPU".
    pub fn render(&self, dl: &DisplayList, view: &View, output: Output) -> Result<GpuFrame, GpuError> {
        self.guard(|| {
            let (b, stats, mut timings) = self.prepare(dl, view)?;
            let t = Instant::now();
            let (premul8, pixmap) = self.run_readback(&b, view, output)?;
            timings.gpu = ms(t);
            Ok(GpuFrame { width: view.width, height: view.height, premul8, pixmap, stats, timings, entries: b.entries, pieces: b.pieces })
        })
    }

    /// Renders a display list into a new texture on this renderer's device (no readback).
    pub fn render_texture(&self, dl: &DisplayList, view: &View) -> Result<GpuTexture, GpuError> {
        self.guard(|| {
            let (b, stats, mut timings) = self.prepare(dl, view)?;
            let t = Instant::now();
            let (w, h) = (view.width, view.height);
            let max = self.limits.max_texture_dimension_2d;
            if w > max || h > max {
                return Err(GpuError::TooLarge(format!("image {w}×{h} is larger than this GPU's textures ({max} px)")));
            }
            let scopes = Scopes::push(&self.device);
            let texture = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("anicraft-stage"),
                size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let tex_view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            let enc = self.encode(&b, view, Output::Premul8, Target::Texture(&tex_view))?;
            self.queue.submit([enc]);
            self.pop(scopes)?;
            timings.gpu = ms(t);
            Ok(GpuTexture { texture, view: tex_view, width: w, height: h, stats, timings, entries: b.entries, pieces: b.pieces })
        })
    }

    /// Reads a texture from [`GpuRenderer::render_texture`] back (tests, benchmarks and the
    /// self-test; the stage never does).
    pub fn read_texture(&self, t: &GpuTexture) -> Result<Vec<u8>, GpuError> {
        self.guard(|| {
            let row = (t.width * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
            let scopes = Scopes::push(&self.device);
            let read = self.device.create_buffer(&wgpu::BufferDescriptor { label: Some("readback"), size: u64::from(row) * u64::from(t.height), usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
            let mut enc = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("anicraft-readback") });
            enc.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo { texture: &t.texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                wgpu::TexelCopyBufferInfo { buffer: &read, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(t.height) } },
                wgpu::Extent3d { width: t.width, height: t.height, depth_or_array_layers: 1 },
            );
            self.queue.submit([enc.finish()]);
            let bytes = self.map(&read, scopes)?;
            let mut out = Vec::with_capacity(t.width as usize * t.height as usize * 4);
            for r in bytes.chunks(row as usize) {
                out.extend_from_slice(r.get(..t.width as usize * 4).unwrap_or(&[]));
            }
            Ok(out)
        })
    }

    /// Blocks until everything submitted so far has run (benchmarks).
    pub fn wait_idle(&self) -> Result<(), GpuError> {
        self.device.poll(wgpu::PollType::wait_indefinitely()).map(|_| ()).map_err(|e| GpuError::Device(e.to_string()))
    }

    /// Every call runs inside `catch_unwind` (wgpu reports misuse by panicking), refuses once the
    /// device is broken, and marks it broken on a device error.
    fn guard<R>(&self, f: impl FnOnce() -> Result<R, GpuError>) -> Result<R, GpuError> {
        if let Some(e) = self.broken() {
            return Err(GpuError::Device(e));
        }
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).unwrap_or_else(|_| Err(GpuError::Device("the GPU renderer failed on this frame".into())));
        if let Err(GpuError::Device(e)) = &r {
            self.mark_broken(e);
        }
        r
    }

    /// The oracle's preparation, then the binning; checked against the device's limits.
    fn prepare(&self, dl: &DisplayList, view: &View) -> Result<(binning::Binned, RenderStats, Timings), GpuError> {
        let (w, h) = (view.width, view.height);
        if w == 0 || h == 0 || u64::from(w) * u64::from(h) > MAX_PIXELS {
            return Err(GpuError::TooLarge(format!("image {w}×{h} is outside the GPU stage's range")));
        }
        let t0 = Instant::now();
        let (scene, stats) = prepare_scene(dl, view).map_err(GpuError::NeedsFull)?;
        let t1 = Instant::now();
        let b = binning::bin(&scene, w, h).map_err(GpuError::NeedsFull)?;
        // Free the prepared polygons on the pool, off this frame's path (as the oracle does).
        rayon::spawn(move || drop(scene));
        if b.work > MAX_WORK {
            return Err(GpuError::TooLarge(format!("this frame is too much work for one GPU pass ({} M)", b.work >> 20)));
        }
        let cap = self.limits.max_storage_buffer_binding_size.min(self.limits.max_buffer_size);
        let sizes = [("tile lists", b.index.len() * 4), ("edges", b.geo.len() * 16), ("paints", b.paints.len() * 16)];
        for (what, n) in sizes {
            if n as u64 > cap {
                return Err(GpuError::TooLarge(format!("{what} need {} MB, more than this GPU binds ({} MB)", n >> 20, cap >> 20)));
            }
        }
        Ok((b, stats, Timings { prepare: (t1 - t0).as_secs_f32() * 1000.0, bin: ms(t1), gpu: 0.0 }))
    }

    /// Uploads the binned scene and records the fine pass into `target`.
    fn encode(&self, b: &binning::Binned, view: &View, output: Output, target: Target) -> Result<wgpu::CommandBuffer, GpuError> {
        let bg = view.background.map_or([0.0; 4], premultiply);
        let mut uniform = [0u32; 12];
        uniform[..4].copy_from_slice(&[view.width, view.height, b.tiles_x, u32::from(output == Output::Float)]);
        uniform[4..6].copy_from_slice(&[b.entries_at, b.ops_at]);
        for (u, f) in uniform[8..].iter_mut().zip(bg) {
            *u = f.to_bits();
        }
        let storage = |label: &str, bytes: &[u8]| {
            // Bindings can't be empty: pad to one vec4.
            let pad = [0u8; 16];
            let contents = if bytes.is_empty() { &pad[..] } else { bytes };
            self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some(label), contents, usage: wgpu::BufferUsages::STORAGE })
        };
        let ubuf = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("params"), contents: bytemuck::cast_slice(&uniform), usage: wgpu::BufferUsages::UNIFORM });
        let ibuf = storage("index", bytemuck::cast_slice(&b.index));
        let gbuf = storage("geo", bytemuck::cast_slice(&b.geo));
        let pbuf = storage("paints", bytemuck::cast_slice(&b.paints));
        let mut entries = vec![
            wgpu::BindGroupEntry { binding: 0, resource: ubuf.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: ibuf.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 2, resource: gbuf.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 3, resource: pbuf.as_entire_binding() },
        ];
        let (pipeline, layout) = match target {
            Target::Buffer(out) => {
                entries.push(wgpu::BindGroupEntry { binding: 4, resource: out.as_entire_binding() });
                (&self.pipeline, &self.layout)
            }
            Target::Texture(view) => {
                entries.push(wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::TextureView(view) });
                (&self.texture_pipeline, &self.texture_layout)
            }
        };
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("anicraft-fine"), layout, entries: &entries });
        let mut enc = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("anicraft-fine") });
        {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("anicraft-fine"), timestamp_writes: None });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups(b.tiles_x, b.tiles_y, 1);
        }
        Ok(enc.finish())
    }

    fn run_readback(&self, b: &binning::Binned, view: &View, output: Output) -> Result<(Vec<u8>, Option<Pixmap>), GpuError> {
        let (w, h) = (view.width, view.height);
        let out_bytes = u64::from(w) * u64::from(h) * if output == Output::Float { 16 } else { 4 };
        let cap = self.limits.max_storage_buffer_binding_size.min(self.limits.max_buffer_size);
        if out_bytes > cap {
            return Err(GpuError::TooLarge(format!("the image needs {} MB, more than this GPU binds ({} MB)", out_bytes >> 20, cap >> 20)));
        }
        let scopes = Scopes::push(&self.device);
        let obuf = self.device.create_buffer(&wgpu::BufferDescriptor { label: Some("out"), size: out_bytes, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false });
        let read = self.device.create_buffer(&wgpu::BufferDescriptor { label: Some("readback"), size: out_bytes, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let fine = self.encode(b, view, output, Target::Buffer(&obuf))?;
        let mut enc = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("anicraft-readback") });
        enc.copy_buffer_to_buffer(&obuf, 0, &read, 0, out_bytes);
        self.queue.submit([fine, enc.finish()]);
        let bytes = self.map(&read, scopes)?;
        Ok(match output {
            Output::Premul8 => (bytes, None),
            Output::Float => {
                let mut pm = Pixmap::new(w, h).map_err(|e| GpuError::TooLarge(e.to_string()))?;
                for (p, q) in pm.data.iter_mut().zip(bytes.as_chunks::<16>().0) {
                    for (c, v) in p.iter_mut().zip(q.as_chunks::<4>().0) {
                        *c = f32::from_le_bytes(*v);
                    }
                }
                (Vec::new(), Some(pm))
            }
        })
    }

    /// Waits for `read`'s copy, pops the error scopes and returns its bytes.
    fn map(&self, read: &wgpu::Buffer, scopes: Scopes) -> Result<Vec<u8>, GpuError> {
        let (tx, rx) = std::sync::mpsc::channel();
        read.slice(..).map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        let polled = self.device.poll(wgpu::PollType::wait_indefinitely());
        self.pop(scopes)?;
        polled.map_err(|e| GpuError::Device(e.to_string()))?;
        match rx.recv() {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return Err(GpuError::Device(e.to_string())),
            Err(_) => return Err(GpuError::Device("the readback never completed".into())),
        }
        let bytes = read.slice(..).get_mapped_range().map_err(|e| GpuError::Device(e.to_string()))?.to_vec();
        read.unmap();
        Ok(bytes)
    }

    /// Pops a render's scopes: out of memory fails just this frame (the CPU draws it); any
    /// other error is a device error and stops the GPU stage.
    fn pop(&self, scopes: Scopes) -> Result<(), GpuError> {
        let (oom, other) = scopes.pop_split();
        if let Some(e) = other {
            return Err(GpuError::Device(e.to_string()));
        }
        if let Some(e) = oom {
            return Err(GpuError::TooLarge(format!("the GPU ran out of memory: {e}")));
        }
        Ok(())
    }
}

/// The error scopes around one render on this thread: nothing the renderer does can reach the
/// device's uncaptured-error handler (on the window's device that would be the window's).
struct Scopes {
    internal: wgpu::ErrorScopeGuard,
    oom: wgpu::ErrorScopeGuard,
    valid: wgpu::ErrorScopeGuard,
}

impl Scopes {
    fn push(device: &wgpu::Device) -> Scopes {
        let internal = device.push_error_scope(wgpu::ErrorFilter::Internal);
        let oom = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let valid = device.push_error_scope(wgpu::ErrorFilter::Validation);
        Scopes { internal, oom, valid }
    }

    /// `(out of memory, any other error)`, popped innermost first.
    fn pop_split(self) -> (Option<wgpu::Error>, Option<wgpu::Error>) {
        let valid = pollster::block_on(self.valid.pop());
        let oom = pollster::block_on(self.oom.pop());
        let internal = pollster::block_on(self.internal.pop());
        (oom, valid.or(internal))
    }

    fn pop(self) -> Option<wgpu::Error> {
        let (oom, other) = self.pop_split();
        other.or(oom)
    }
}

fn ms(t: Instant) -> f32 {
    t.elapsed().as_secs_f32() * 1000.0
}

/// Mean and maximum per-channel difference of two premultiplied RGBA8 images, in 1/255 steps,
/// and how many pixels differ at all. `None` when the sizes differ.
pub fn compare_premul8(a: &[u8], b: &[u8]) -> Option<Diff> {
    if a.len() != b.len() || !a.len().is_multiple_of(4) {
        return None;
    }
    let mut sum = 0u64;
    let mut max = 0u8;
    let mut differing = 0usize;
    for (p, q) in a.as_chunks::<4>().0.iter().zip(b.as_chunks::<4>().0) {
        let mut any = false;
        for (x, y) in p.iter().zip(q) {
            let d = x.abs_diff(*y);
            sum += u64::from(d);
            max = max.max(d);
            any |= d > 0;
        }
        differing += usize::from(any);
    }
    let channels = a.len().max(1) as f64;
    Some(Diff { mean: sum as f64 / channels, max, differing, pixels: a.len() / 4 })
}

/// How far two images are apart (see [`compare_premul8`]).
#[derive(Clone, Copy, Debug)]
pub struct Diff {
    /// Mean absolute difference per channel, in 1/255 steps.
    pub mean: f64,
    /// Largest difference in any channel, in 1/255 steps.
    pub max: u8,
    pub differing: usize,
    pub pixels: usize,
}

#[cfg(test)]
mod tests;
