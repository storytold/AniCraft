//! The GPU stage switch (ADR 0019, ADR 0029, ADR 0032). With the `gpu-stage` feature (on by
//! default), the stage is drawn by the wgpu renderer (`anicraft-gpu`) unless Preferences ›
//! Stage › Renderer chooses the CPU: it starts off the UI thread (the CPU draws until it is
//! ready), draws masks, groups, blend modes, bitmaps and filters, and the CPU oracle draws a
//! tile it can't (nesting or bitmap counts past its limits) and everything when the GPU is
//! missing or fails. Without the feature this is a stub and the CPU always draws. Exports never
//! use it.
//!
//! The web build has no GPU stage yet (ADR 0036): in the browser wgpu's objects belong to one
//! thread and start asynchronously, so the renderer can't run behind the render service there.
//!
//! When the window runs on wgpu, the renderer shares the window's device and draws each stage
//! tile into a texture the window shows as it is (no copy back to the CPU, ADR 0029). When it
//! can't (the window on OpenGL, or the window's device lacks what the stage needs) it opens a
//! device of its own and reads the tiles back, as in the first slice.
//!
//! `ANICRAFT_STAGE_RENDERER=gpu|cpu` overrides the preference (also in `--screenshot` runs,
//! which otherwise use default preferences).

use crate::prefs::StageRenderer;
use anicraft_engine::render_service::{NativeImage, RenderService};

/// Where the GPU stage is.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(not(gpu_stage), allow(dead_code))]
pub enum State {
    /// The CPU draws (by choice, or because this build has no GPU stage).
    Off,
    Starting,
    /// Drawing, on this adapter; `direct` when straight into the window's textures.
    Running { adapter: String, direct: bool },
    /// Could not start or stopped working; the CPU draws. The reason, for Preferences.
    Failed(String),
}

/// Whether this build has the GPU stage at all.
pub const AVAILABLE: bool = cfg!(gpu_stage);

/// The renderer the user asked for, after the environment override.
pub fn wanted(pref: StageRenderer) -> StageRenderer {
    match std::env::var("ANICRAFT_STAGE_RENDERER").ok().map(|v| v.to_ascii_lowercase()).as_deref() {
        Some("gpu") => StageRenderer::Gpu,
        Some("cpu") => StageRenderer::Cpu,
        _ => pref,
    }
}

#[derive(Default)]
pub struct GpuStage {
    #[cfg(gpu_stage)]
    imp: imp::Imp,
}

/// A stage tile the GPU drew into a texture of the window's device, registered with the
/// window's renderer for as long as this lives.
pub struct NativeTile {
    id: egui::TextureId,
    #[cfg(gpu_stage)]
    _keep: imp::Registered,
}

impl NativeTile {
    pub fn id(&self) -> egui::TextureId {
        self.id
    }
}

impl GpuStage {
    pub fn state(&self) -> State {
        #[cfg(gpu_stage)]
        return self.imp.state.clone();
        #[cfg(not(gpu_stage))]
        State::Off
    }

    /// The window's wgpu device, when it runs on wgpu (from eframe's creation context).
    #[cfg(gpu_stage)]
    pub fn set_window_device(&mut self, rs: Option<&eframe::egui_wgpu::RenderState>) {
        self.imp.window = rs.cloned();
    }

    /// Applies the choice to the render service (called every frame; cheap when nothing
    /// changes). Starting the renderer happens on its own thread and repaints when done.
    #[allow(unused_variables)]
    pub fn sync(&mut self, pref: StageRenderer, svc: &RenderService, ctx: &egui::Context) {
        // Unit tests never open a graphics device (as they never open an audio one): a start-up
        // thread still inside the driver when the test process exits can crash it.
        #[cfg(gpu_stage)]
        self.imp.sync(wanted(pref) == StageRenderer::Gpu && !cfg!(test), svc, ctx);
    }

    /// Shows a picture the GPU drew into a texture: registers it with the window's renderer.
    /// `None` when it isn't one of ours (then nothing is shown and the tile is drawn again).
    #[allow(unused_variables)]
    pub fn register(&self, native: &NativeImage) -> Option<NativeTile> {
        #[cfg(gpu_stage)]
        return self.imp.register(native);
        #[cfg(not(gpu_stage))]
        None
    }
}

#[cfg(gpu_stage)]
mod imp {
    use super::{NativeTile, State};
    use anicraft_engine::render_service::{NativeImage, Raster, Rasterizer, RenderService};
    use anicraft_gpu::{GpuError, GpuRenderer, GpuTexture, Output};
    use eframe::egui_wgpu::{RenderState, Renderer};
    use std::sync::{Arc, mpsc};

    pub struct Imp {
        pub state: State,
        pub window: Option<RenderState>,
        renderer: Option<Arc<GpuRenderer>>,
        starting: Option<mpsc::Receiver<Result<GpuRenderer, String>>>,
    }

    impl Default for Imp {
        fn default() -> Self {
            Imp { state: State::Off, window: None, renderer: None, starting: None }
        }
    }

    /// Keeps a tile's texture alive and registered; frees the registration when dropped.
    pub struct Registered {
        id: egui::TextureId,
        renderer: Arc<egui::mutex::RwLock<Renderer>>,
        _texture: NativeImage,
    }

    impl Drop for Registered {
        fn drop(&mut self) {
            self.renderer.write().free_texture(&self.id);
        }
    }

    /// The window's device first (no readback); a device of our own when that can't work.
    fn start(window: Option<RenderState>) -> Result<GpuRenderer, String> {
        let shared = match &window {
            Some(rs) => match GpuRenderer::shared(&rs.adapter, rs.device.clone(), rs.queue.clone()) {
                Ok(g) => return Ok(g),
                Err(e) => Some(e.to_string()),
            },
            None => None,
        };
        GpuRenderer::new().map_err(|e| match shared {
            Some(s) => format!("{s}; and on a device of its own: {e}"),
            None => e.to_string(),
        })
    }

    impl Imp {
        pub fn sync(&mut self, want: bool, svc: &RenderService, ctx: &egui::Context) {
            if let Some(rx) = &self.starting
                && let Ok(r) = rx.try_recv()
            {
                self.starting = None;
                match r {
                    Ok(g) => self.renderer = Some(Arc::new(g)),
                    Err(e) => self.state = State::Failed(e),
                }
            }
            if let Some(g) = &self.renderer
                && let Some(e) = g.broken()
            {
                self.renderer = None;
                self.state = State::Failed(e);
            }
            if want && self.renderer.is_none() && self.starting.is_none() && !matches!(self.state, State::Failed(_)) {
                let (tx, rx) = mpsc::channel();
                let ctx = ctx.clone();
                let window = self.window.clone();
                let spawned = std::thread::Builder::new().name("anicraft-gpu-start".into()).spawn(move || {
                    let _ = tx.send(start(window));
                    ctx.request_repaint();
                });
                match spawned {
                    Ok(_) => {
                        self.starting = Some(rx);
                        self.state = State::Starting;
                    }
                    Err(e) => self.state = State::Failed(e.to_string()),
                }
            }
            let drawing = self.renderer.as_ref().filter(|_| want);
            match (drawing, svc.has_rasterizer()) {
                (Some(g), false) => {
                    let g = g.clone();
                    self.state = State::Running { adapter: g.adapter.clone(), direct: g.shared };
                    let why = |e: GpuError| match e {
                        GpuError::NeedsFull(why) => why.to_string(),
                        e => e.to_string(),
                    };
                    let raster: Rasterizer = if g.shared {
                        Arc::new(move |dl, view| match g.render_texture(dl, view) {
                            Ok(t) => {
                                let (width, height, stats) = (t.width, t.height, t.stats);
                                Ok((Raster::Native { image: Arc::new(t), width, height }, stats))
                            }
                            Err(e) => Err(why(e)),
                        })
                    } else {
                        Arc::new(move |dl, view| match g.render(dl, view, Output::Premul8) {
                            Ok(f) => Ok((Raster::Premul8(f.premul8), f.stats)),
                            Err(e) => Err(why(e)),
                        })
                    };
                    svc.set_rasterizer(Some(raster));
                    ctx.request_repaint();
                }
                (None, true) => {
                    svc.set_rasterizer(None);
                    ctx.request_repaint();
                }
                _ => {}
            }
            if !want && !matches!(self.state, State::Failed(_)) {
                self.state = State::Off;
            }
        }

        pub fn register(&self, native: &NativeImage) -> Option<NativeTile> {
            let rs = self.window.as_ref()?;
            let tex = native.downcast_ref::<GpuTexture>()?;
            // The texture lives on the window's device (only a shared renderer makes these).
            let id = rs.renderer.write().register_native_texture(&rs.device, &tex.view, eframe::wgpu::FilterMode::Linear);
            Some(NativeTile { id, _keep: Registered { id, renderer: rs.renderer.clone(), _texture: native.clone() } })
        }
    }
}
