//! AniCraft GPU stage renderer (layer L3, beside the engine; ADR 0019, ADR 0029, ADR 0031).
//!
//! Draws the same scene as the CPU oracle (`anicraft-render`) with wgpu compute: the oracle
//! prepares it ([`anicraft_render::prepare_scene`]: flattening, strokes, paints and their
//! gradient tables, clips and groups), [`binning`] sorts the edges into 16 × 16 tiles on the
//! CPU, and one compute pass computes every pixel's exact area coverage and composites the
//! fills in order, inside their masks and groups (alpha and blend modes) as the oracle's band
//! compositor does. Bitmap fills are sampled from textures uploaded once per image
//! ([`images`]); a filtered symbol is drawn into a layer of its own, filtered by compute passes
//! that run the oracle's filter steps ([`layers`]), and composited like any fill (ADR 0031).
//! The output is the oracle's within float rounding (the parity tests check mean and maximum
//! error against it).
//!
//! What it doesn't draw (nesting deeper than the shaders hold, more bitmaps in one picture than
//! its texture slots) is reported as [`GpuError::NeedsFull`], and the caller renders that
//! picture on the CPU.
//!
//! Two ways to run: on a **headless device of its own** ([`GpuRenderer::new`]: tests, tools,
//! and windows that run on OpenGL), reading frames back; or on the **window's device**
//! ([`GpuRenderer::shared`]), drawing into a texture ([`GpuRenderer::render_texture`]) that
//! the window shows as it is, with no readback.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod binning;
pub mod images;
mod layers;

use anicraft_doc::DisplayList;
use anicraft_render::{MAX_PIXELS, NeedsFull, Pixmap, RenderStats, View, paint::premultiply, prepare_scene};
use rayon::prelude::*;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;
use wgpu::util::DeviceExt;
pub use images::IMAGE_BUDGET;

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
    /// A buffer, its first `size` bytes.
    Buffer(&'a wgpu::Buffer, u64),
    Texture(&'a wgpu::TextureView),
}

/// The fine shader's first bitmap binding (`fine.wgsl`: `img0` … `img15`).
const IMAGE_BINDING: u32 = 6;

/// The renderer: a device (its own, or the window's), the fine and filter pipelines, and the
/// bitmaps it has uploaded.
pub struct GpuRenderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    texture_pipeline: wgpu::ComputePipeline,
    texture_layout: wgpu::BindGroupLayout,
    filters: layers::FilterPipelines,
    /// Bound where a texture slot or a buffer is unused.
    blank: wgpu::TextureView,
    /// Placeholders for a filter pass's coverage (written) and ramp (read): one buffer can't be
    /// both in a dispatch.
    blank_coverage: wgpu::Buffer,
    blank_ramp: wgpu::Buffer,
    images: Mutex<images::Images>,
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
        || limits.max_sampled_textures_per_shader_stage < binning::MAX_IMAGES as u32
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

/// Deepest nesting of filtered symbols drawn on the GPU (each level is a layer drawn and
/// filtered inside its parent's); deeper goes to the CPU.
pub const MAX_FILTER_DEPTH: usize = 4;

/// Most filtered-layer pixels one call draws (each costs 44 bytes of buffers while the frame
/// draws: two f32 halves and three coverage slots). A stage tile's layers are far smaller; a
/// call beyond this is drawn on the CPU.
pub const MAX_LAYER_PIXELS: u64 = 1 << 23;

/// A scene binned for the fine pass, with the filtered layers it shows (each its own scene,
/// drawn and filtered before the pass).
struct Job {
    binned: binning::Binned,
    layers: Vec<LayerJob>,
}

struct LayerJob {
    /// Where its pixels go in the parent's paint buffer (`vec4`s).
    at: u32,
    job: Box<Job>,
    view: View,
    ops: Vec<anicraft_render::filters::FilterOp>,
}

/// What a job tree costs, for the limits.
#[derive(Default)]
struct Cost {
    work: u64,
    layer_pixels: u64,
    entries: usize,
    pieces: usize,
}

impl Job {
    fn cost(&self, c: &mut Cost) {
        c.work += self.binned.work;
        c.entries += self.binned.entries;
        c.pieces += self.binned.pieces;
        for l in &self.layers {
            let n = u64::from(l.view.width) * u64::from(l.view.height);
            c.layer_pixels += n;
            // Each filter pass walks the layer once (per pixel or per line).
            c.work += n.div_ceil(256) * layers::passes(&l.ops);
            l.job.cost(c);
        }
    }
}

impl GpuRenderer {
    /// Opens a headless device on the best graphics card (high-performance first), builds the
    /// pipelines and checks them with a small render against the oracle. A software adapter
    /// (WARP, llvmpipe, SwiftShader) doesn't count: the CPU oracle draws the stage faster than
    /// a GPU emulated on the same CPU. `Err` means "use the CPU".
    pub fn new() -> Result<GpuRenderer, GpuError> {
        // A renderer bug must never take the caller down: wgpu reports misuse by panicking.
        std::panic::catch_unwind(|| Self::open(false)).unwrap_or_else(|_| Err(GpuError::Unavailable("the GPU driver failed while starting".into())))
    }

    /// The same on the system's software adapter (WARP on Windows, lavapipe or llvmpipe on
    /// Linux): for parity tests on machines without a graphics card, such as CI runners.
    pub fn software() -> Result<GpuRenderer, GpuError> {
        std::panic::catch_unwind(|| Self::open(true)).unwrap_or_else(|_| Err(GpuError::Unavailable("the software GPU failed while starting".into())))
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
            if info.device_type == wgpu::DeviceType::Cpu {
                return Err(GpuError::Unavailable(format!("the window draws with {name}, a software renderer: the CPU draws the stage faster")));
            }
            let limits = device.limits();
            check(adapter, &limits, &name)?;
            Self::build(device, queue, limits, name, true, Arc::new(Mutex::new(None)))
        }))
        .unwrap_or_else(|_| Err(GpuError::Unavailable("the GPU driver failed while starting".into())))
    }

    fn open(software: bool) -> Result<GpuRenderer, GpuError> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let options = wgpu::RequestAdapterOptions { power_preference: wgpu::PowerPreference::HighPerformance, force_fallback_adapter: software, ..Default::default() };
        let adapter = pollster::block_on(instance.request_adapter(&options)).map_err(|e| GpuError::Unavailable(e.to_string()))?;
        let info = adapter.get_info();
        let name = format!("{} ({:?})", info.name, info.backend);
        if !software && info.device_type == wgpu::DeviceType::Cpu {
            return Err(GpuError::Unavailable(format!("only {name}, a software renderer: the CPU draws the stage faster")));
        }
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
        let mut common = vec![entry(0, buffer(wgpu::BufferBindingType::Uniform)), entry(1, read()), entry(2, read()), entry(3, read())];
        let image = wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false };
        common.extend((0..binning::MAX_IMAGES as u32).map(|i| entry(IMAGE_BINDING + i, image)));
        let make = |label: &str, out: wgpu::BindGroupLayoutEntry, entry_point: &str| {
            let mut entries = common.clone();
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
        let filters = layers::FilterPipelines::new(&device);
        let blank = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("anicraft-blank"),
                size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&wgpu::TextureViewDescriptor::default());
        let blank_buffer = |label| device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size: 16, usage: wgpu::BufferUsages::STORAGE, mapped_at_creation: false });
        let (blank_coverage, blank_ramp) = (blank_buffer("anicraft-blank-coverage"), blank_buffer("anicraft-blank-ramp"));
        if let Some(e) = scopes.pop() {
            return Err(GpuError::Unavailable(format!("{name}: the stage shaders did not build: {}", describe(&e))));
        }
        let r = GpuRenderer {
            device,
            queue,
            pipeline,
            layout,
            texture_pipeline,
            texture_layout,
            filters,
            blank,
            blank_coverage,
            blank_ramp,
            images: Mutex::new(images::Images::default()),
            limits,
            adapter: name,
            shared,
            broken,
        };
        r.self_test()?;
        Ok(r)
    }

    /// A 24 × 20 render must come back as the CPU oracle draws it, read back and through a
    /// texture: two known rectangles exactly, and a bitmap fill and a blurred, colour-adjusted
    /// symbol within the parity target. Catches drivers that build the pipelines but compute
    /// garbage.
    fn self_test(&self) -> Result<(), GpuError> {
        use anicraft_doc::{ColorEffect, DisplayItem, ElementId, Filter, Group, LayerId, Paint, Pixels, Rgba};
        use anicraft_geom::{Affine, FillRule, Path};
        let item = |path: Path, fill: Paint| DisplayItem {
            path: Arc::new(path),
            transform: Affine::IDENTITY,
            fill: Some(fill),
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
        let solid = |x: f32, c: Rgba| item(Path::rect(x, 2.0, 8.0, 16.0), Paint::Solid(c));
        let mut bitmap = item(Path::rect(2.0, 18.0, 20.0, 2.0), Paint::Bitmap { bitmap: anicraft_doc::BitmapId(1), repeat: true, matrix: Affine::scale(3.0, 1.0) });
        bitmap.image = Some(Arc::new(Pixels { width: 2, height: 1, rgba: vec![255, 0, 0, 255, 0, 0, 255, 128] }));
        let mut blurred = item(Path::rect(10.0, 0.0, 4.0, 4.0), Paint::Solid(Rgba::new(0.0, 0.6, 0.0, 1.0)));
        blurred.group = Some(0);
        let filters = Arc::from(vec![Filter::Blur { blur_x: 3.0, blur_y: 2.0, quality: 2 }, Filter::AdjustColor { brightness: 10.0, contrast: 0.0, saturation: -50.0, hue: 30.0 }]);
        let groups = vec![Group { parent: None, alpha: 1.0, blend: anicraft_doc::BlendMode::Normal, filters, clip: None }];
        let dl = DisplayList { items: vec![solid(2.0, Rgba::new(1.0, 0.0, 0.0, 1.0)), solid(14.0, Rgba::new(0.0, 0.0, 1.0, 0.5)), bitmap, blurred], groups, ..Default::default() };
        let view = View { transform: Affine::IDENTITY, width: 24, height: 20, background: Some(Rgba::WHITE) };
        let (oracle, _) = anicraft_render::render(&dl, &view).map_err(|e| GpuError::Unavailable(e.to_string()))?;
        let oracle = oracle.to_premul_rgba8();
        // `Err` says where it went wrong (for Preferences and bug reports).
        let check = |how: &str, bytes: &[u8]| -> Result<(), GpuError> {
            let px = |x: usize, y: usize| bytes.get((y * 24 + x) * 4..(y * 24 + x) * 4 + 4).map(<[u8]>::to_vec);
            let d = compare_premul8(bytes, &oracle);
            if px(5, 10) == Some(vec![255, 0, 0, 255]) && px(18, 10) == Some(vec![128, 128, 255, 255]) && d.is_some_and(|d| d.max <= 2 && d.mean <= 0.1) {
                return Ok(());
            }
            let first = bytes.as_chunks::<4>().0.iter().zip(oracle.as_chunks::<4>().0).position(|(a, b)| a.iter().zip(b).any(|(x, y)| x.abs_diff(*y) > 2));
            let at = first.map_or(String::new(), |i| format!(", first at ({}, {}): {:?} for {:?}", i % 24, i / 24, bytes.get(i * 4..i * 4 + 4), oracle.get(i * 4..i * 4 + 4)));
            let off = d.map_or("a wrong size".to_string(), |d| format!("{} of {} pixels off, by up to {}/255{at}", d.differing, d.pixels, d.max));
            Err(GpuError::Unavailable(format!("{}: the self-test render came back wrong {how} ({off})", self.adapter)))
        };
        check("read back", &self.render(&dl, &view, Output::Premul8)?.premul8)?;
        let t = self.render_texture(&dl, &view)?;
        check("through a texture", &self.read_texture(&t)?)
    }

    /// Whether the device has failed (lost, or an error in or outside a render); then use the
    /// CPU.
    pub fn broken(&self) -> Option<String> {
        self.broken.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    fn mark_broken(&self, why: &str) {
        self.broken.lock().unwrap_or_else(PoisonError::into_inner).get_or_insert_with(|| why.to_string());
    }

    /// Bitmaps uploaded and kept, and their bytes (at most about [`images::IMAGE_BUDGET`]).
    pub fn cached_images(&self) -> (usize, u64) {
        self.images.lock().unwrap_or_else(PoisonError::into_inner).held()
    }

    /// Renders a display list and reads it back. [`GpuError::NeedsFull`] means "draw this one
    /// on the CPU".
    pub fn render(&self, dl: &DisplayList, view: &View, output: Output) -> Result<GpuFrame, GpuError> {
        self.guard(|| {
            let (job, stats, mut timings, cost) = self.prepare(dl, view)?;
            let t = Instant::now();
            let (premul8, pixmap) = self.run_readback(&job, view, output)?;
            timings.gpu = ms(t);
            Ok(GpuFrame { width: view.width, height: view.height, premul8, pixmap, stats, timings, entries: cost.entries, pieces: cost.pieces })
        })
    }

    /// Renders a display list into a new texture on this renderer's device (no readback).
    pub fn render_texture(&self, dl: &DisplayList, view: &View) -> Result<GpuTexture, GpuError> {
        self.guard(|| {
            let (job, stats, mut timings, cost) = self.prepare(dl, view)?;
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
            let mut enc = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("anicraft-stage") });
            let recorded = self.record(&mut enc, &job, view, Output::Premul8, Target::Texture(&tex_view));
            // Pop the scopes whatever happened while recording (they are this thread's).
            if let Err(e) = recorded {
                let _ = self.pop(scopes);
                return Err(e);
            }
            self.queue.submit([enc.finish()]);
            self.pop(scopes)?;
            timings.gpu = ms(t);
            Ok(GpuTexture { texture, view: tex_view, width: w, height: h, stats, timings, entries: cost.entries, pieces: cost.pieces })
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

    /// The oracle's preparation and the binning, for the scene and each filtered layer it shows
    /// (in parallel); checked against the limits and the device's.
    fn prepare(&self, dl: &DisplayList, view: &View) -> Result<(Job, RenderStats, Timings, Cost), GpuError> {
        let (w, h) = (view.width, view.height);
        if w == 0 || h == 0 || u64::from(w) * u64::from(h) > MAX_PIXELS {
            return Err(GpuError::TooLarge(format!("image {w}×{h} is outside the GPU stage's range")));
        }
        let t0 = Instant::now();
        let mut bin_ms = 0.0;
        let (job, stats) = self.plan(dl, view, 0, &mut bin_ms)?;
        let total = ms(t0);
        let mut cost = Cost::default();
        job.cost(&mut cost);
        if cost.work > MAX_WORK {
            return Err(GpuError::TooLarge(format!("this frame is too much work for the GPU ({} M)", cost.work >> 20)));
        }
        if cost.layer_pixels > MAX_LAYER_PIXELS {
            return Err(GpuError::TooLarge(format!("this frame's filters need {} M pixels of layers", cost.layer_pixels >> 20)));
        }
        self.check_sizes(&job)?;
        Ok((job, stats, Timings { prepare: (total - bin_ms).max(0.0), bin: bin_ms, gpu: 0.0 }, cost))
    }

    /// One scene: prepared and binned, then its shown layers planned the same way.
    fn plan(&self, dl: &DisplayList, view: &View, depth: usize, bin_ms: &mut f32) -> Result<(Job, RenderStats), GpuError> {
        let (mut scene, stats) = prepare_scene(dl, view).map_err(GpuError::NeedsFull)?;
        let t = Instant::now();
        let binned = binning::bin(&scene, view.width, view.height).map_err(GpuError::NeedsFull)?;
        *bin_ms += ms(t);
        let shown: Vec<(u32, anicraft_render::FilterLayer)> = std::mem::take(&mut scene.layers).into_iter().zip(&binned.layer_at).filter_map(|(l, at)| Some(((*at)?, l))).collect();
        // Free the prepared polygons on the pool, off this frame's path (as the oracle does).
        rayon::spawn(move || drop(scene));
        if !shown.is_empty() && depth >= MAX_FILTER_DEPTH {
            return Err(GpuError::NeedsFull(NeedsFull::Filters));
        }
        let planned: Vec<Result<(LayerJob, f32), GpuError>> = shown
            .into_par_iter()
            .map(|(at, l)| {
                let mut ms = 0.0;
                let (job, _) = self.plan(&l.list, &l.view, depth + 1, &mut ms)?;
                Ok((LayerJob { at, job: Box::new(job), view: l.view, ops: l.filters }, ms))
            })
            .collect();
        let mut layers = Vec::with_capacity(planned.len());
        for p in planned {
            let (l, ms) = p?;
            *bin_ms += ms;
            layers.push(l);
        }
        Ok((Job { binned, layers }, stats))
    }

    /// Every buffer a job binds fits what the device binds.
    fn check_sizes(&self, job: &Job) -> Result<(), GpuError> {
        let cap = self.limits.max_storage_buffer_binding_size.min(self.limits.max_buffer_size);
        let b = &job.binned;
        let sizes = [("tile lists", b.index.len() as u64 * 4), ("edges", b.geo.len() as u64 * 16), ("paints and layers", b.paints_len as u64 * 16)];
        for (what, n) in sizes {
            if n > cap {
                return Err(GpuError::TooLarge(format!("{what} need {} MB, more than this GPU binds ({} MB)", n >> 20, cap >> 20)));
            }
        }
        for l in &job.layers {
            let n = u64::from(l.view.width) * u64::from(l.view.height);
            if 32 * n > cap {
                return Err(GpuError::TooLarge(format!("a filtered layer needs {} MB, more than this GPU binds ({} MB)", (32 * n) >> 20, cap >> 20)));
            }
            self.check_sizes(&l.job)?;
        }
        Ok(())
    }

    /// Records a job into `enc`: each shown layer drawn, filtered and copied into the paint
    /// buffer, then the fine pass into `target`.
    fn record(&self, enc: &mut wgpu::CommandEncoder, job: &Job, view: &View, output: Output, target: Target) -> Result<(), GpuError> {
        let b = &job.binned;
        // The CPU's paints, with room after them for the layers' pixels.
        let pbuf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("paints"),
            size: (b.paints_len.max(1) * 16) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: true,
        });
        {
            let bytes: &[u8] = bytemuck::cast_slice(&b.paints);
            if !bytes.is_empty() {
                pbuf.slice(..bytes.len() as u64).get_mapped_range_mut().map_err(|e| GpuError::Device(e.to_string()))?.copy_from_slice(bytes);
            }
        }
        pbuf.unmap();
        for l in &job.layers {
            let (w, h) = (l.view.width, l.view.height);
            let n = u64::from(w) * u64::from(h);
            let img = self.device.create_buffer(&wgpu::BufferDescriptor { label: Some("layer"), size: 32 * n, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false });
            let cov = if layers::needs_coverage(&l.ops) {
                self.device.create_buffer(&wgpu::BufferDescriptor { label: Some("layer-coverage"), size: 12 * n, usage: wgpu::BufferUsages::STORAGE, mapped_at_creation: false })
            } else {
                self.blank_coverage.clone()
            };
            self.record(enc, &l.job, &l.view, Output::Float, Target::Buffer(&img, 16 * n))?;
            layers::Recorder { device: &self.device, pipes: &self.filters, img: &img, cov: &cov, w, h, no_ramp: &self.blank_ramp }.record(enc, &l.ops);
            enc.copy_buffer_to_buffer(&img, 0, &pbuf, u64::from(l.at) * 16, 16 * n);
        }
        let views = self.images.lock().unwrap_or_else(PoisonError::into_inner).views(&self.device, &self.queue, self.limits.max_texture_dimension_2d, &b.images)?;
        self.fine_pass(enc, b, &pbuf, &views, view, output, target);
        Ok(())
    }

    /// Uploads the binned scene and records the fine pass into `target`.
    #[allow(clippy::too_many_arguments)]
    fn fine_pass(&self, enc: &mut wgpu::CommandEncoder, b: &binning::Binned, pbuf: &wgpu::Buffer, images: &[wgpu::TextureView], view: &View, output: Output, target: Target) {
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
        let mut entries = vec![
            wgpu::BindGroupEntry { binding: 0, resource: ubuf.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: ibuf.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 2, resource: gbuf.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 3, resource: pbuf.as_entire_binding() },
        ];
        for i in 0..binning::MAX_IMAGES {
            let v = images.get(i).unwrap_or(&self.blank);
            entries.push(wgpu::BindGroupEntry { binding: IMAGE_BINDING + i as u32, resource: wgpu::BindingResource::TextureView(v) });
        }
        let (pipeline, layout) = match target {
            Target::Buffer(out, size) => {
                entries.push(wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: out, offset: 0, size: wgpu::BufferSize::new(size) }) });
                (&self.pipeline, &self.layout)
            }
            Target::Texture(view) => {
                entries.push(wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::TextureView(view) });
                (&self.texture_pipeline, &self.texture_layout)
            }
        };
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("anicraft-fine"), layout, entries: &entries });
        let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("anicraft-fine"), timestamp_writes: None });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &bind, &[]);
        pass.dispatch_workgroups(b.tiles_x, b.tiles_y, 1);
    }

    fn run_readback(&self, job: &Job, view: &View, output: Output) -> Result<(Vec<u8>, Option<Pixmap>), GpuError> {
        let (w, h) = (view.width, view.height);
        let out_bytes = u64::from(w) * u64::from(h) * if output == Output::Float { 16 } else { 4 };
        let cap = self.limits.max_storage_buffer_binding_size.min(self.limits.max_buffer_size);
        if out_bytes > cap {
            return Err(GpuError::TooLarge(format!("the image needs {} MB, more than this GPU binds ({} MB)", out_bytes >> 20, cap >> 20)));
        }
        let scopes = Scopes::push(&self.device);
        let obuf = self.device.create_buffer(&wgpu::BufferDescriptor { label: Some("out"), size: out_bytes, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false });
        let read = self.device.create_buffer(&wgpu::BufferDescriptor { label: Some("readback"), size: out_bytes, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let mut enc = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("anicraft-fine") });
        if let Err(e) = self.record(&mut enc, job, view, output, Target::Buffer(&obuf, out_bytes)) {
            let _ = self.pop(scopes);
            return Err(e);
        }
        enc.copy_buffer_to_buffer(&obuf, 0, &read, 0, out_bytes);
        self.queue.submit([enc.finish()]);
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
            return Err(GpuError::Device(describe(&e)));
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

/// A wgpu error with what it says happened (its `Display` alone is just "Validation Error").
pub(crate) fn describe(e: &wgpu::Error) -> String {
    match e {
        wgpu::Error::Validation { description, .. } | wgpu::Error::Internal { description, .. } => description.trim().to_string(),
        wgpu::Error::OutOfMemory { .. } => e.to_string(),
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
