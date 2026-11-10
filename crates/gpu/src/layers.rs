//! Filters on the GPU (ADR 0032): the host side of `filters.wgsl`.
//!
//! A filtered group's layer is drawn (premultiplied f32) into half 0 of its `img` buffer by the
//! fine pass, then each of its filters runs as compute passes over it, step for step as the CPU
//! oracle's `filters::apply` does, with the numbers `filters::resolve` gives: blurs as box
//! passes along rows and then columns (ping-ponging between the buffer's halves, or between a
//! coverage slot and slot 2), shadows, glows and bevels from moved and blurred coverage, the
//! colour matrix per pixel. The finished layer is copied into the paint buffer of the scene that
//! shows it.

use anicraft_doc::FilterSide;
use anicraft_render::filters::{BevelPaint, BoxBlur, FilterOp};
use anicraft_render::paint::Premul;
use wgpu::util::DeviceExt;

/// The filter pipelines (one layout, one entry point each).
pub(crate) struct FilterPipelines {
    layout: wgpu::BindGroupLayout,
    box4: wgpu::ComputePipeline,
    box1: wgpu::ComputePipeline,
    coverage: wgpu::ComputePipeline,
    shadow: wgpu::ComputePipeline,
    bevel: wgpu::ComputePipeline,
    gradient_glow: wgpu::ComputePipeline,
    adjust: wgpu::ComputePipeline,
}

const INNER: u32 = 1;
const KNOCKOUT: u32 = 2;
const HIDE: u32 = 4;
const INVERSE: u32 = 8;
const RAMP: u32 = 16;

/// The coverage slot a blur uses as its other half.
const TEMP: u32 = 2;

/// Words in the pass parameters (`filters.wgsl` › `Params`: 11 `vec4`s).
const PARAM_WORDS: usize = 44;

/// Whether any of `ops` needs the coverage slots.
pub(crate) fn needs_coverage(ops: &[FilterOp]) -> bool {
    ops.iter().any(|o| !matches!(o, FilterOp::Blur(_) | FilterOp::AdjustColor(_)))
}

/// One pass's parameters.
#[derive(Clone, Copy)]
struct Params {
    words: [u32; PARAM_WORDS],
}

impl Params {
    fn new(w: u32, h: u32) -> Params {
        let mut words = [0u32; PARAM_WORDS];
        words[0] = w;
        words[1] = h;
        Params { words }
    }
    fn size(mut self, n: u32, horizontal: bool) -> Self {
        self.words[2] = n;
        self.words[3] = u32::from(horizontal);
        self
    }
    fn io(mut self, src: u32, dst: u32, flags: u32, side: u32) -> Self {
        self.words[4..8].copy_from_slice(&[src, dst, flags, side]);
        self
    }
    fn offset(mut self, dx: i64, dy: i64) -> Self {
        // Past ±2^24 every read is outside the layer either way (layers are far smaller).
        let c = |v: i64| v.clamp(-(1 << 24), 1 << 24) as i32 as u32;
        self.words[8] = c(dx);
        self.words[9] = c(dy);
        self
    }
    fn colors(mut self, c0: [f32; 4], c1: [f32; 4]) -> Self {
        for (w, v) in self.words[12..16].iter_mut().zip(c0) {
            *w = v.to_bits();
        }
        for (w, v) in self.words[16..20].iter_mut().zip(c1) {
            *w = v.to_bits();
        }
        self
    }
    fn strength(mut self, k: f32) -> Self {
        self.words[20] = k.to_bits();
        self
    }
    /// The 4 × 5 matrix as five columns.
    fn matrix(mut self, m: &[[f32; 5]; 4]) -> Self {
        for col in 0..5 {
            for (row, r) in m.iter().enumerate() {
                if let (Some(w), Some(v)) = (self.words.get_mut(24 + col * 4 + row), r.get(col)) {
                    *w = v.to_bits();
                }
            }
        }
        self
    }
}

fn side_code(s: FilterSide) -> u32 {
    match s {
        FilterSide::Inner => 0,
        FilterSide::Outer => 1,
        FilterSide::Full => 2,
    }
}

impl FilterPipelines {
    pub(crate) fn new(device: &wgpu::Device) -> FilterPipelines {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("anicraft-filters"), source: wgpu::ShaderSource::Wgsl(include_str!("filters.wgsl").into()) });
        let entry = |binding: u32, ty: wgpu::BufferBindingType| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer { ty, has_dynamic_offset: false, min_binding_size: None },
            count: None,
        };
        let rw = wgpu::BufferBindingType::Storage { read_only: false };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("anicraft-filters"),
            entries: &[entry(0, wgpu::BufferBindingType::Uniform), entry(1, rw), entry(2, rw), entry(3, wgpu::BufferBindingType::Storage { read_only: true })],
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("anicraft-filters"), bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
        let make = |name: &str| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some(name), layout: Some(&pl), module: &module, entry_point: Some(name), compilation_options: Default::default(), cache: None })
        };
        FilterPipelines {
            box4: make("box4"),
            box1: make("box1"),
            coverage: make("coverage"),
            shadow: make("shadow"),
            bevel: make("bevel"),
            gradient_glow: make("gradient_glow"),
            adjust: make("adjust"),
            layout,
        }
    }
}

/// Records one layer's filters: `img` holds the drawn layer in half 0 (`w × h`), `cov` three
/// coverage slots (or nothing when no filter needs them).
pub(crate) struct Recorder<'a> {
    pub device: &'a wgpu::Device,
    pub pipes: &'a FilterPipelines,
    pub img: &'a wgpu::Buffer,
    pub cov: &'a wgpu::Buffer,
    pub w: u32,
    pub h: u32,
    /// Bound as the ramp where a pass reads none.
    pub no_ramp: &'a wgpu::Buffer,
}

/// Passes a filter chain dispatches (for the work bound): about one per box pass and step.
pub(crate) fn passes(ops: &[FilterOp]) -> u64 {
    let blur = |b: &BoxBlur| 2 * u64::from(b.passes.max(1)) + 1;
    ops.iter()
        .map(|o| match o {
            FilterOp::Blur(b) => blur(b),
            FilterOp::Shadow { blur: b, .. } | FilterOp::Bevel { blur: b, .. } => blur(b) + 2,
            FilterOp::GradientGlow { blur: b, .. } => 2 * (blur(b) + 1) + 1,
            FilterOp::AdjustColor(_) => 1,
        })
        .sum()
}

impl Recorder<'_> {
    fn dispatch(&self, enc: &mut wgpu::CommandEncoder, pipe: &wgpu::ComputePipeline, p: Params, ramp: &wgpu::Buffer, groups: (u32, u32)) {
        let ubuf = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("filter-params"), contents: bytemuck::cast_slice(&p.words), usage: wgpu::BufferUsages::UNIFORM });
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("anicraft-filters"),
            layout: &self.pipes.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: ubuf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: self.img.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: self.cov.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: ramp.as_entire_binding() },
            ],
        });
        let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("anicraft-filter"), timestamp_writes: None });
        pass.set_pipeline(pipe);
        pass.set_bind_group(0, &bind, &[]);
        pass.dispatch_workgroups(groups.0, groups.1, 1);
    }

    fn pixels(&self) -> (u32, u32) {
        (self.w.div_ceil(16), self.h.div_ceil(16))
    }

    /// One box pass along rows or columns, `src` → `dst` (halves of `img`, or coverage slots).
    fn box_pass(&self, enc: &mut wgpu::CommandEncoder, four: bool, n: usize, horizontal: bool, src: u32, dst: u32) {
        let lines = if horizontal { self.h } else { self.w };
        let p = Params::new(self.w, self.h).size(u32::try_from(n).unwrap_or(u32::MAX), horizontal).io(src, dst, 0, 0);
        self.dispatch(enc, if four { &self.pipes.box4 } else { &self.pipes.box1 }, p, self.no_ramp, (lines.div_ceil(64), 1));
    }

    /// The oracle's `blur_channels`: `passes` × (rows, then columns) over half 0 of `img`
    /// (`four`) or coverage slot `slot`, ending where it started. Widths of 0 or 1 change
    /// nothing and are skipped.
    fn blur(&self, enc: &mut wgpu::CommandEncoder, four: bool, slot: u32, b: &BoxBlur) {
        let other = if four { 1 } else { TEMP };
        let mut cur = slot;
        for _ in 0..b.passes.max(1) {
            for (n, horizontal) in [(b.nx, true), (b.ny, false)] {
                if n > 1 {
                    let next = if cur == slot { other } else { slot };
                    self.box_pass(enc, four, n, horizontal, cur, next);
                    cur = next;
                }
            }
        }
        if cur != slot {
            // A width-1 pass copies.
            self.box_pass(enc, four, 1, true, cur, slot);
        }
    }

    /// The layer's opacity (`inverse`: transparency) moved by `(dx, dy)`, into slot `dst`.
    fn coverage(&self, enc: &mut wgpu::CommandEncoder, dx: i64, dy: i64, inverse: bool, dst: u32) {
        let p = Params::new(self.w, self.h).io(0, dst, if inverse { INVERSE } else { 0 }, 0).offset(dx, dy);
        self.dispatch(enc, &self.pipes.coverage, p, self.no_ramp, self.pixels());
    }

    /// Records `ops`, in order.
    pub(crate) fn record(&self, enc: &mut wgpu::CommandEncoder, ops: &[FilterOp]) {
        let dummy = self.no_ramp;
        let ramp_buffer = |r: &[Premul]| self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("filter-ramp"), contents: bytemuck::cast_slice(r), usage: wgpu::BufferUsages::STORAGE });
        for op in ops {
            match op {
                FilterOp::Blur(b) => self.blur(enc, true, 0, b),
                FilterOp::Shadow { blur, dx, dy, strength, color, inner, knockout, hide_object } => {
                    self.coverage(enc, *dx, *dy, *inner, 0);
                    self.blur(enc, false, 0, blur);
                    let flags = (u32::from(*inner) * INNER) | (u32::from(*knockout) * KNOCKOUT) | (u32::from(*hide_object) * HIDE);
                    let p = Params::new(self.w, self.h).io(0, 0, flags, 0).colors([color.r, color.g, color.b, color.a], [0.0; 4]).strength(*strength);
                    self.dispatch(enc, &self.pipes.shadow, p, dummy, self.pixels());
                }
                FilterOp::Bevel { blur, dx, dy, strength, paint, side, knockout } => {
                    self.coverage(enc, 0, 0, false, 0);
                    self.blur(enc, false, 0, blur);
                    let flags = u32::from(*knockout) * KNOCKOUT;
                    let p = Params::new(self.w, self.h).offset(*dx, *dy).strength(*strength);
                    match paint {
                        BevelPaint::Colors { highlight, shadow } => {
                            self.dispatch(enc, &self.pipes.bevel, p.io(0, 0, flags, side_code(*side)).colors(*highlight, *shadow), dummy, self.pixels());
                        }
                        BevelPaint::Ramp(r) => {
                            let ramp = ramp_buffer(r);
                            self.dispatch(enc, &self.pipes.bevel, p.io(0, 0, flags | RAMP, side_code(*side)), &ramp, self.pixels());
                        }
                    }
                }
                FilterOp::GradientGlow { blur, dx, dy, strength, ramp, side, knockout } => {
                    // Inside: the transparency into slot 0; outside: the opacity into slot 1.
                    if *side != FilterSide::Outer {
                        self.coverage(enc, *dx, *dy, true, 0);
                        self.blur(enc, false, 0, blur);
                    }
                    if *side != FilterSide::Inner {
                        self.coverage(enc, *dx, *dy, false, 1);
                        self.blur(enc, false, 1, blur);
                    }
                    let ramp = ramp_buffer(ramp);
                    let p = Params::new(self.w, self.h).io(0, 0, u32::from(*knockout) * KNOCKOUT, side_code(*side)).strength(*strength);
                    self.dispatch(enc, &self.pipes.gradient_glow, p, &ramp, self.pixels());
                }
                FilterOp::AdjustColor(m) => {
                    let p = Params::new(self.w, self.h).matrix(m);
                    self.dispatch(enc, &self.pipes.adjust, p, dummy, self.pixels());
                }
            }
        }
    }
}
