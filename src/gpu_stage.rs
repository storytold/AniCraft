//! The GPU stage switch (ADR 0019). With the `gpu-stage` feature, Preferences › Stage ›
//! Renderer can hand the stage to the wgpu renderer (`anicraft-gpu`): it starts off the UI
//! thread, draws what it can, and the CPU oracle draws the rest (masks, symbol opacity, blend
//! modes, filters, bitmaps) and everything when the GPU is missing or fails. Without the feature
//! this is a stub and the CPU always draws. Exports never use it.
//!
//! `ANICRAFT_STAGE_RENDERER=gpu|cpu` overrides the preference (also in `--screenshot` runs,
//! which otherwise use default preferences).

use crate::prefs::StageRenderer;
use anicraft_engine::render_service::RenderService;

/// Where the GPU stage is.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(not(feature = "gpu-stage"), allow(dead_code))]
pub enum State {
    /// The CPU draws (by choice, or because this build has no GPU stage).
    Off,
    Starting,
    /// Drawing, on this adapter.
    Running(String),
    /// Could not start or stopped working; the CPU draws. The reason, for Preferences.
    Failed(String),
}

/// Whether this build has the GPU stage at all.
pub const AVAILABLE: bool = cfg!(feature = "gpu-stage");

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
    #[cfg(feature = "gpu-stage")]
    imp: imp::Imp,
}

impl GpuStage {
    pub fn state(&self) -> State {
        #[cfg(feature = "gpu-stage")]
        return self.imp.state.clone();
        #[cfg(not(feature = "gpu-stage"))]
        State::Off
    }

    /// Applies the choice to the render service (called every frame; cheap when nothing
    /// changes). Starting the renderer happens on its own thread and repaints when done.
    #[allow(unused_variables)]
    pub fn sync(&mut self, pref: StageRenderer, svc: &RenderService, ctx: &egui::Context) {
        #[cfg(feature = "gpu-stage")]
        self.imp.sync(wanted(pref) == StageRenderer::Gpu, svc, ctx);
    }
}

#[cfg(feature = "gpu-stage")]
mod imp {
    use super::State;
    use anicraft_engine::render_service::{Rasterizer, RenderService};
    use anicraft_gpu::{GpuError, GpuRenderer, Output};
    use std::sync::{Arc, mpsc};

    pub struct Imp {
        pub state: State,
        renderer: Option<Arc<GpuRenderer>>,
        starting: Option<mpsc::Receiver<Result<GpuRenderer, String>>>,
    }

    impl Default for Imp {
        fn default() -> Self {
            Imp { state: State::Off, renderer: None, starting: None }
        }
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
                let spawned = std::thread::Builder::new().name("anicraft-gpu-start".into()).spawn(move || {
                    let _ = tx.send(GpuRenderer::new().map_err(|e| e.to_string()));
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
                    self.state = State::Running(g.adapter.clone());
                    let raster: Rasterizer = Arc::new(move |dl, view| match g.render(dl, view, Output::Premul8) {
                        Ok(f) => Ok((f.premul8, f.stats)),
                        Err(GpuError::NeedsFull(why)) => Err(why.to_string()),
                        Err(e) => Err(e.to_string()),
                    });
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
    }
}
