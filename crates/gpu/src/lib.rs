//! AniCraft GPU stage renderer (layer L3, beside the engine; ADR 0014).
//!
//! Draws the same fills as the CPU oracle (`anicraft-render`) with wgpu compute: the oracle
//! prepares the fills ([`anicraft_render::plain_fills`]: flattening, strokes, paints and their
//! gradient tables), [`binning`] sorts their edges into 16 × 16 tiles on the CPU, and one
//! compute pass computes every pixel's exact area coverage and composites the fills in order.
//! The output is the oracle's within float rounding (the parity tests check mean and maximum
//! error against it).
//!
//! What the plain path doesn't cover yet (masks, group opacity, blend modes, filters,
//! bitmaps) is reported as [`GpuError::NeedsFull`], and the caller renders that frame on the
//! CPU. The renderer owns a headless device, so it works without a window, in tests and on
//! machines where the window itself runs on OpenGL.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod binning;

use anicraft_doc::DisplayList;
use anicraft_render::{MAX_PIXELS, NeedsFull, Pixmap, RenderStats, View, paint::premultiply, plain_fills};
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

/// A rendered frame.
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

/// Where a frame's time went (milliseconds).
#[derive(Clone, Copy, Debug, Default)]
pub struct Timings {
    /// Cull, flatten, stroke and paint set-up (shared with the oracle, parallel).
    pub prepare: f32,
    /// Tile binning (parallel per fill) and buffer packing.
    pub bin: f32,
    /// Upload, compute pass, readback.
    pub gpu: f32,
}

impl Timings {
    pub fn total(&self) -> f32 {
        self.prepare + self.bin + self.gpu
    }
}

/// The renderer: a headless device and the fine pipeline.
pub struct GpuRenderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    limits: wgpu::Limits,
    /// Adapter name and backend, for the status bar and reports.
    pub adapter: String,
    /// First error the device reported outside our error scopes (or its loss). Once set, the
    /// renderer refuses to draw and callers fall back to the CPU.
    broken: Arc<Mutex<Option<String>>>,
}

impl GpuRenderer {
    /// Opens a headless device on the best adapter (high-performance first), builds the
    /// pipeline and checks it with a small known render. `Err` means "use the CPU".
    pub fn new() -> Result<GpuRenderer, GpuError> {
        // A renderer bug must never take the caller down: wgpu reports misuse by panicking.
        std::panic::catch_unwind(Self::open).unwrap_or_else(|_| Err(GpuError::Unavailable("the GPU driver failed while starting".into())))
    }

    fn open() -> Result<GpuRenderer, GpuError> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions { power_preference: wgpu::PowerPreference::HighPerformance, ..Default::default() }))
            .map_err(|e| GpuError::Unavailable(e.to_string()))?;
        let info = adapter.get_info();
        let name = format!("{} ({:?})", info.name, info.backend);
        if !adapter.get_downlevel_capabilities().flags.contains(wgpu::DownlevelFlags::COMPUTE_SHADERS) {
            return Err(GpuError::Unavailable(format!("{name} has no compute shaders")));
        }
        let have = adapter.limits();
        let want = wgpu::Limits {
            max_storage_buffer_binding_size: have.max_storage_buffer_binding_size,
            max_buffer_size: have.max_buffer_size,
            ..wgpu::Limits::downlevel_defaults()
        };
        if have.max_storage_buffers_per_shader_stage < 4 || have.max_compute_invocations_per_workgroup < 256 || have.max_compute_workgroup_size_x < 16 || have.max_compute_workgroup_size_y < 16 {
            return Err(GpuError::Unavailable(format!("{name} is below the compute limits the stage needs")));
        }
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor { label: Some("anicraft-gpu"), required_limits: want.clone(), ..Default::default() }))
            .map_err(|e| GpuError::Unavailable(format!("{name}: {e}")))?;
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
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("anicraft-fine"), source: wgpu::ShaderSource::Wgsl(include_str!("fine.wgsl").into()) });
        let storage = |binding: u32, read_only: bool| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only }, has_dynamic_offset: false, min_binding_size: None },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("anicraft-fine"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                storage(1, true),
                storage(2, true),
                storage(3, true),
                storage(4, false),
            ],
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("anicraft-fine"), bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("anicraft-fine"),
            layout: Some(&pl),
            module: &module,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        if let Some(e) = pollster::block_on(scope.pop()) {
            return Err(GpuError::Unavailable(format!("{name}: the stage shader did not build: {e}")));
        }
        let r = GpuRenderer { device, queue, pipeline, layout, limits: want, adapter: name, broken };
        r.self_test()?;
        Ok(r)
    }

    /// A 24 × 20 render of two known rectangles must come back exactly: catches drivers that
    /// build the pipeline but compute garbage.
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
        let f = self.render(&dl, &view, Output::Premul8)?;
        let px = |x: usize, y: usize| f.premul8.get((y * 24 + x) * 4..(y * 24 + x) * 4 + 4).map(<[u8]>::to_vec);
        let ok = px(5, 10) == Some(vec![255, 0, 0, 255]) && px(18, 10) == Some(vec![128, 128, 255, 255]) && px(12, 1) == Some(vec![255; 4]);
        if !ok {
            return Err(GpuError::Unavailable(format!("{}: the self-test render came back wrong", self.adapter)));
        }
        Ok(())
    }

    /// Whether the device has failed (lost, or an error outside a render); then use the CPU.
    pub fn broken(&self) -> Option<String> {
        self.broken.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// Renders a display list. [`GpuError::NeedsFull`] means "draw this one on the CPU".
    pub fn render(&self, dl: &DisplayList, view: &View, output: Output) -> Result<GpuFrame, GpuError> {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.render_inner(dl, view, output))).unwrap_or_else(|_| Err(GpuError::Device("the GPU renderer failed on this frame".into())))
    }

    fn render_inner(&self, dl: &DisplayList, view: &View, output: Output) -> Result<GpuFrame, GpuError> {
        if let Some(e) = self.broken() {
            return Err(GpuError::Device(e));
        }
        let (w, h) = (view.width, view.height);
        if w == 0 || h == 0 || u64::from(w) * u64::from(h) > MAX_PIXELS {
            return Err(GpuError::TooLarge(format!("image {w}×{h} is outside the GPU stage's range")));
        }
        let t0 = Instant::now();
        let (fills, stats) = plain_fills(dl, view).map_err(GpuError::NeedsFull)?;
        let t1 = Instant::now();
        let b = binning::bin(&fills, w, h);
        // Free the prepared polygons on the pool, off this frame's path (as the oracle does).
        rayon::spawn(move || drop(fills));
        let t2 = Instant::now();
        let (premul8, pixmap) = self.run(&b, view, output)?;
        let t3 = Instant::now();
        let ms = |a: Instant, b: Instant| (b - a).as_secs_f32() * 1000.0;
        Ok(GpuFrame { width: w, height: h, premul8, pixmap, stats, timings: Timings { prepare: ms(t0, t1), bin: ms(t1, t2), gpu: ms(t2, t3) }, entries: b.entries, pieces: b.pieces })
    }

    fn run(&self, b: &binning::Binned, view: &View, output: Output) -> Result<(Vec<u8>, Option<Pixmap>), GpuError> {
        let (w, h) = (view.width, view.height);
        let px = u64::from(w) * u64::from(h);
        let out_bytes = px * if output == Output::Float { 16 } else { 4 };
        let index: &[u8] = bytemuck::cast_slice(&b.index);
        let geo: &[u8] = bytemuck::cast_slice(&b.geo);
        let paints: &[u8] = bytemuck::cast_slice(&b.paints);
        let cap = self.limits.max_storage_buffer_binding_size.min(self.limits.max_buffer_size);
        for (what, n) in [("tile lists", index.len() as u64), ("edges", geo.len() as u64), ("paints", paints.len() as u64), ("the image", out_bytes)] {
            if n > cap {
                return Err(GpuError::TooLarge(format!("{what} need {} MB, more than this GPU binds ({} MB)", n >> 20, cap >> 20)));
            }
        }
        let bg = view.background.map_or([0.0; 4], premultiply);
        let mut uniform = [0u32; 12];
        uniform[..4].copy_from_slice(&[w, h, b.tiles_x, u32::from(output == Output::Float)]);
        uniform[4..6].copy_from_slice(&[b.entries_at, b.ops_at]);
        for (u, f) in uniform[8..].iter_mut().zip(bg) {
            *u = f.to_bits();
        }

        let oom = self.device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let valid = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let storage = |label: &str, bytes: &[u8]| {
            // Bindings can't be empty: pad to one vec4.
            let pad = [0u8; 16];
            let contents = if bytes.is_empty() { &pad[..] } else { bytes };
            self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some(label), contents, usage: wgpu::BufferUsages::STORAGE })
        };
        let ubuf = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("params"), contents: bytemuck::cast_slice(&uniform), usage: wgpu::BufferUsages::UNIFORM });
        let ibuf = storage("index", index);
        let gbuf = storage("geo", geo);
        let pbuf = storage("paints", paints);
        let obuf = self.device.create_buffer(&wgpu::BufferDescriptor { label: Some("out"), size: out_bytes, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false });
        let read = self.device.create_buffer(&wgpu::BufferDescriptor { label: Some("readback"), size: out_bytes, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("anicraft-fine"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: ubuf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: ibuf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: gbuf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: pbuf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: obuf.as_entire_binding() },
            ],
        });
        let mut enc = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("anicraft-fine") });
        {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("anicraft-fine"), timestamp_writes: None });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups(b.tiles_x, b.tiles_y, 1);
        }
        enc.copy_buffer_to_buffer(&obuf, 0, &read, 0, out_bytes);
        self.queue.submit([enc.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        read.slice(..).map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        let polled = self.device.poll(wgpu::PollType::wait_indefinitely());
        let invalid = pollster::block_on(valid.pop());
        let no_memory = pollster::block_on(oom.pop());
        if let Some(e) = invalid.or(no_memory) {
            return Err(GpuError::Device(e.to_string()));
        }
        polled.map_err(|e| GpuError::Device(e.to_string()))?;
        match rx.recv() {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return Err(GpuError::Device(e.to_string())),
            Err(_) => return Err(GpuError::Device("the readback never completed".into())),
        }
        let result = {
            let view = read.slice(..).get_mapped_range().map_err(|e| GpuError::Device(e.to_string()))?;
            let bytes: &[u8] = &view;
            match output {
                Output::Premul8 => (bytes.to_vec(), None),
                Output::Float => {
                    let mut pm = Pixmap::new(w, h).map_err(|e| GpuError::TooLarge(e.to_string()))?;
                    for (p, q) in pm.data.iter_mut().zip(bytes.as_chunks::<16>().0) {
                        for (c, v) in p.iter_mut().zip(q.as_chunks::<4>().0) {
                            *c = f32::from_le_bytes(*v);
                        }
                    }
                    (Vec::new(), Some(pm))
                }
            }
        };
        read.unmap();
        Ok(result)
    }
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
