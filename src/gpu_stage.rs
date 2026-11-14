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
//! **Failing mid-session (ADR 0092).** When the renderer breaks while drawing (its device lost,
//! an error in a frame, out of graphics memory), the CPU takes the stage over at once (the
//! frame being drawn is drawn again by the oracle), the status bar says so and the log says
//! why. The GPU is tried again later, after a growing pause ([`RETRY_SECS`]), or sooner when
//! the stage changes size; a renderer that then runs [`STABLE_SECS`] without trouble starts the
//! count afresh. When the window's own device was lost there is nothing to retry on: the window
//! closes to open again another way (`gpu_watch`).
//!
//! `ANICRAFT_STAGE_RENDERER=gpu|cpu` overrides the preference (also in `--screenshot` runs,
//! which otherwise use default preferences).

use crate::prefs::StageRenderer;
use anicraft_engine::render_service::{NativeImage, RenderService};

/// Where the GPU stage is.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(not(gpu_stage), allow(dead_code))]
pub enum State {
    /// The CPU draws (by choice, or because this build has no GPU stage).
    Off,
    Starting,
    /// Drawing, on this adapter; `direct` when straight into the window's textures.
    Running { adapter: String, direct: bool },
    /// Could not start; the CPU draws. The reason, for Preferences.
    Failed(String),
    /// Stopped working mid-session (ADR 0092); the CPU draws. Why, and when the GPU is tried
    /// again (app time, seconds; `None`: not again this session).
    Stopped { why: String, retry_at: Option<f64> },
}

/// What happened to the GPU stage this frame, for the status bar.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(not(gpu_stage), allow(dead_code))]
pub enum Notice {
    /// It stopped and the CPU draws now; the GPU is tried again in this many seconds (`None`:
    /// not again this session).
    Stopped { retry_in: Option<f64> },
    /// It was tried again and draws again.
    Back,
}

/// Pauses before trying the GPU stage again after each failure in a row (seconds); after the
/// last the CPU keeps the stage for the session.
pub const RETRY_SECS: [f64; 4] = [3.0, 15.0, 60.0, 300.0];
/// A renderer that draws this long without failing starts the count of failures afresh.
pub const STABLE_SECS: f64 = 600.0;
/// A change of the stage's size brings a retry forward, but never sooner than this after the
/// failure.
pub const RESIZE_SOONEST_SECS: f64 = 1.0;

/// When to try the GPU again (ADR 0092): a bounded backoff. Pure, so its rules are tested.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Retry {
    /// Failures in a row (since the last stable run).
    failures: u32,
    failed_at: Option<f64>,
    /// When the next try is due.
    at: Option<f64>,
    running_since: Option<f64>,
}

impl Retry {
    /// The renderer failed at `now`: when it is tried again, or `None` when it has failed too
    /// often in a row.
    pub fn failed(&mut self, now: f64) -> Option<f64> {
        if self.running_since.is_some_and(|t| now - t >= STABLE_SECS) {
            self.failures = 0;
        }
        self.running_since = None;
        self.failed_at = Some(now);
        let wait = RETRY_SECS.get(self.failures as usize).copied();
        self.failures = self.failures.saturating_add(1);
        self.at = wait.map(|w| now + w);
        self.at
    }

    /// Whether a try is due at `now` (`resized`: the stage changed size this frame).
    pub fn due(&self, now: f64, resized: bool) -> bool {
        let Some(at) = self.at else { return false };
        now >= at || (resized && self.failed_at.is_some_and(|f| now - f >= RESIZE_SOONEST_SECS))
    }

    /// A try began: no other is due until it ends.
    pub fn trying(&mut self) {
        self.at = None;
    }

    /// The renderer is drawing again (or for the first time).
    pub fn running(&mut self, now: f64) {
        self.at = None;
        self.running_since = Some(now);
    }

    #[cfg(test)]
    pub fn failures(&self) -> u32 {
        self.failures
    }
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
    /// `size` is the stage's in pixels (a change brings a retry forward). What happened, for
    /// the status bar.
    #[allow(unused_variables)]
    pub fn sync(&mut self, pref: StageRenderer, svc: &RenderService, ctx: &egui::Context, size: (u32, u32)) -> Option<Notice> {
        // Unit tests never open a graphics device (as they never open an audio one): a start-up
        // thread still inside the driver when the test process exits can crash it.
        #[cfg(gpu_stage)]
        return self.imp.sync(wanted(pref) == StageRenderer::Gpu && !cfg!(test), svc, ctx, ctx.input(|i| i.time), size);
        #[cfg(not(gpu_stage))]
        None
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

/// The status bar's words for a notice.
pub fn notice_text(n: &Notice) -> String {
    match n {
        Notice::Stopped { retry_in: Some(s) } => tr!("The graphics card stopped drawing the stage, so the CPU draws it now; AniCraft tries the graphics card again in {seconds} s.", seconds = s.round() as u32),
        Notice::Stopped { retry_in: None } => tr!("The graphics card stopped drawing the stage, so the CPU draws it for the rest of this session.").into(),
        Notice::Back => tr!("The graphics card draws the stage again.").into(),
    }
}

#[cfg(gpu_stage)]
mod imp {
    use super::{NativeTile, Notice, Retry, State};
    use anicraft_engine::render_service::{NativeImage, Raster, Rasterizer, RenderService};
    use anicraft_gpu::{GpuError, GpuRenderer, GpuTexture, Output};
    use eframe::egui_wgpu::{RenderState, Renderer};
    use std::sync::{Arc, mpsc};

    /// What the stage needs of a renderer: [`GpuRenderer`], or a stand-in in tests.
    pub trait StageDevice: Send + Sync + 'static {
        fn adapter(&self) -> String;
        /// Whether it draws on the window's device.
        fn shared(&self) -> bool;
        fn broken(&self) -> Option<String>;
        fn fail(&self, why: &str);
        fn rasterizer(self: Arc<Self>) -> Rasterizer;
    }

    impl StageDevice for GpuRenderer {
        fn adapter(&self) -> String {
            self.adapter.clone()
        }
        fn shared(&self) -> bool {
            self.shared
        }
        fn broken(&self) -> Option<String> {
            GpuRenderer::broken(self)
        }
        fn fail(&self, why: &str) {
            GpuRenderer::fail(self, why);
        }
        fn rasterizer(self: Arc<Self>) -> Rasterizer {
            let g = self;
            let why = |e: GpuError| match e {
                GpuError::NeedsFull(why) => why.to_string(),
                e => e.to_string(),
            };
            if g.shared {
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
            }
        }
    }

    /// Opens a renderer (on its own thread).
    pub type Starter<D> = Arc<dyn Fn(Option<RenderState>) -> Result<D, String> + Send + Sync>;

    pub struct Imp<D: StageDevice = GpuRenderer> {
        pub state: State,
        pub window: Option<RenderState>,
        renderer: Option<Arc<D>>,
        starting: Option<mpsc::Receiver<Result<D, String>>>,
        retry: Retry,
        size: Option<(u32, u32)>,
        starter: Starter<D>,
        /// The window's device's loss, if it happened (`gpu_watch`).
        window_lost: fn() -> Option<String>,
    }

    impl Default for Imp {
        fn default() -> Self {
            Imp::with(Arc::new(start), crate::gpu_watch::lost)
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
            // Shown in Preferences: the driver's English, joined in the interface language.
            Some(s) => tr!("{shared}; and on a device of its own: {e}", shared = s, e = e),
            None => e.to_string(),
        })
    }

    impl<D: StageDevice> Imp<D> {
        pub fn with(starter: Starter<D>, window_lost: fn() -> Option<String>) -> Self {
            Imp { state: State::Off, window: None, renderer: None, starting: None, retry: Retry::default(), size: None, starter, window_lost }
        }

        /// One frame at app time `now` (seconds) with the stage `size` pixels: starts, checks
        /// and (after a failure) retries the renderer, and gives the render service the
        /// rasterizer that should draw. What happened, for the status bar.
        pub fn sync(&mut self, want: bool, svc: &RenderService, ctx: &egui::Context, now: f64, size: (u32, u32)) -> Option<Notice> {
            let mut notice = None;
            let resized = self.size.is_some_and(|s| s != size);
            self.size = Some(size);
            if let Some(rx) = &self.starting
                && let Ok(r) = rx.try_recv()
            {
                self.starting = None;
                let retrying = matches!(self.state, State::Stopped { .. });
                match r {
                    Ok(g) => {
                        crate::startup::line(&format!("stage on the GPU{}: {}", if retrying { " again" } else { "" }, g.adapter()));
                        self.renderer = Some(Arc::new(g));
                        self.retry.running(now);
                        if retrying {
                            notice = Some(Notice::Back);
                        }
                    }
                    // A retry that can't start waits longer, as a failure does.
                    Err(e) if retrying => {
                        let retry_at = self.retry.failed(now);
                        crate::startup::line(&format!("the GPU stage did not start again ({e}); {}", retry_line(retry_at, now)));
                        self.state = State::Stopped { why: e, retry_at };
                    }
                    Err(e) => {
                        crate::startup::line(&format!("the stage is drawn on the CPU: {e}"));
                        self.state = State::Failed(e);
                    }
                }
            }
            let window_lost = (self.window_lost)();
            if let Some(g) = &self.renderer
                && g.shared()
                && let Some(w) = &window_lost
            {
                g.fail(&format!("the window's graphics device was lost: {w}")); // i18n-exempt: the driver's reason, as GPU errors are
            }
            if let Some(g) = &self.renderer
                && let Some(e) = g.broken()
            {
                self.renderer = None;
                // Nothing to retry on when the window's device is gone (the window reopens).
                let retry_at = if window_lost.is_some() { None } else { self.retry.failed(now) };
                crate::startup::line(&format!("the GPU stage stopped, the CPU draws: {e}; {}", retry_line(retry_at, now)));
                self.state = State::Stopped { why: e, retry_at };
                notice = Some(Notice::Stopped { retry_in: retry_at.map(|t| t - now) });
            }
            let retry_due = matches!(self.state, State::Stopped { .. }) && window_lost.is_none() && self.retry.due(now, resized);
            let fresh = !matches!(self.state, State::Failed(_) | State::Stopped { .. });
            if want && self.renderer.is_none() && self.starting.is_none() && (fresh || retry_due) {
                let (tx, rx) = mpsc::channel();
                let c = ctx.clone();
                let window = self.window.clone();
                let starter = self.starter.clone();
                let spawned = std::thread::Builder::new().name("anicraft-gpu-start".into()).spawn(move || {
                    let _ = tx.send(starter(window));
                    c.request_repaint();
                });
                match spawned {
                    Ok(_) => {
                        self.starting = Some(rx);
                        if retry_due {
                            self.retry.trying();
                            crate::startup::line("trying the GPU stage again"); // i18n-exempt: log text
                            if let State::Stopped { retry_at, .. } = &mut self.state {
                                *retry_at = None;
                            }
                        } else {
                            self.state = State::Starting;
                        }
                    }
                    Err(e) => self.state = State::Failed(e.to_string()),
                }
            }
            // Wake for the retry even when nothing else happens.
            if let State::Stopped { retry_at: Some(t), .. } = self.state {
                ctx.request_repaint_after(std::time::Duration::from_secs_f64((t - now).clamp(0.05, 3600.0)));
            }
            let drawing = self.renderer.as_ref().filter(|_| want);
            match (drawing, svc.has_rasterizer()) {
                (Some(g), false) => {
                    let g = g.clone();
                    self.state = State::Running { adapter: g.adapter(), direct: g.shared() };
                    svc.set_rasterizer(Some(g.rasterizer()));
                    ctx.request_repaint();
                }
                (None, true) => {
                    svc.set_rasterizer(None);
                    ctx.request_repaint();
                }
                _ => {}
            }
            if !want && !matches!(self.state, State::Failed(_) | State::Stopped { .. }) {
                self.state = State::Off;
            }
            notice
        }

        pub fn register(&self, native: &NativeImage) -> Option<NativeTile> {
            let rs = self.window.as_ref()?;
            let tex = native.downcast_ref::<GpuTexture>()?;
            // The texture lives on the window's device (only a shared renderer makes these).
            let id = rs.renderer.write().register_native_texture(&rs.device, &tex.view, eframe::wgpu::FilterMode::Linear);
            Some(NativeTile { id, _keep: Registered { id, renderer: rs.renderer.clone(), _texture: native.clone() } })
        }
    }

    fn retry_line(retry_at: Option<f64>, now: f64) -> String {
        match retry_at {
            Some(t) => format!("trying the GPU again in {:.0} s", t - now), // i18n-exempt: log text
            None => "not trying the GPU again this session".into(), // i18n-exempt: log text
        }
    }

    /// ADR 0092: the stage's fallback driven by an injected device loss, with a stand-in
    /// renderer (unit tests never open a graphics device).
    #[cfg(test)]
    mod tests {
        use super::*;
        use anicraft_engine::doc::sample;
        use anicraft_engine::render_service::DEFAULT_CACHE_BYTES;
        use std::sync::Mutex;
        use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

        /// Draws by asking the CPU oracle, and fails like a lost device once told to.
        struct Fake {
            broken: Arc<Mutex<Option<String>>>,
        }

        impl StageDevice for Fake {
            fn adapter(&self) -> String {
                "Test GPU (Vulkan)".into()
            }
            fn shared(&self) -> bool {
                false
            }
            fn broken(&self) -> Option<String> {
                self.broken.lock().ok().and_then(|g| g.clone())
            }
            fn fail(&self, why: &str) {
                if let Ok(mut g) = self.broken.lock() {
                    g.get_or_insert_with(|| why.to_string());
                }
            }
            fn rasterizer(self: Arc<Self>) -> Rasterizer {
                Arc::new(move |dl, view| {
                    if let Some(e) = self.broken() {
                        return Err(format!("GPU error: {e}"));
                    }
                    let (p, st) = anicraft_engine::render::render(dl, view).map_err(|e| e.to_string())?;
                    Ok((Raster::Premul8(p.to_premul_rgba8()), st))
                })
            }
        }

        static STARTS: AtomicU32 = AtomicU32::new(0);
        static START_FAILS: AtomicBool = AtomicBool::new(false);
        /// The newest stand-in's failure, for the test to set (as a driver would).
        static LAST: Mutex<Option<Arc<Mutex<Option<String>>>>> = Mutex::new(None);

        fn lose(why: &str) {
            if let Some(b) = LAST.lock().unwrap_or_else(std::sync::PoisonError::into_inner).as_ref() {
                *b.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(why.to_string());
            }
        }

        fn no_window_loss() -> Option<String> {
            None
        }

        fn window_lost() -> Option<String> {
            Some("DXGI_ERROR_DEVICE_REMOVED".into())
        }

        fn imp(lost: fn() -> Option<String>) -> Imp<Fake> {
            Imp::with(
                Arc::new(|_| {
                    STARTS.fetch_add(1, Ordering::SeqCst);
                    if START_FAILS.load(Ordering::SeqCst) {
                        return Err("no usable GPU: the driver is resetting".into());
                    }
                    let broken = Arc::new(Mutex::new(None));
                    *LAST.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(broken.clone());
                    Ok(Fake { broken })
                }),
                lost,
            )
        }

        /// Frames until the start thread has answered (bounded).
        fn settle(m: &mut Imp<Fake>, svc: &RenderService, ctx: &egui::Context, now: f64, size: (u32, u32)) -> Vec<Notice> {
            let mut out = Vec::new();
            for _ in 0..400 {
                out.extend(m.sync(true, svc, ctx, now, size));
                if m.starting.is_none() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            out
        }

        fn draws_on_gpu(svc: &RenderService) -> bool {
            let doc = sample::demo();
            let view = anicraft_engine::render::View::stage(&doc, 0.25);
            // A new document state each time, so no cached picture answers.
            static STATE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
            let state = STATE.fetch_add(1, Ordering::SeqCst);
            let img = svc.render_now_batch(vec![svc.job(state, &doc, 0, view, None)]);
            let img = img.first().expect("a frame");
            assert!(img.error.is_none(), "{:?}", img.error);
            img.gpu
        }

        #[test]
        fn a_lost_device_hands_the_stage_to_the_cpu_and_the_gpu_comes_back_later() {
            let _g = crate::gpu_watch::TEST_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            STARTS.store(0, Ordering::SeqCst);
            START_FAILS.store(false, Ordering::SeqCst);
            let ctx = egui::Context::default();
            let svc = RenderService::new(DEFAULT_CACHE_BYTES, None);
            let mut m = imp(no_window_loss);
            assert!(settle(&mut m, &svc, &ctx, 0.0, (800, 600)).is_empty());
            assert!(matches!(m.state, State::Running { .. }) && svc.has_rasterizer());
            assert!(draws_on_gpu(&svc));
            // The device is lost mid-session: the frame fails on the GPU and the CPU draws it.
            lose("device lost: DXGI_ERROR_DEVICE_HUNG");
            assert!(!draws_on_gpu(&svc), "the oracle draws the failed frame");
            let n = m.sync(true, &svc, &ctx, 10.0, (800, 600));
            assert_eq!(n, Some(Notice::Stopped { retry_in: Some(super::super::RETRY_SECS[0]) }));
            assert!(matches!(&m.state, State::Stopped { why, retry_at: Some(_) } if why.contains("DEVICE_HUNG")));
            assert!(!svc.has_rasterizer() && !draws_on_gpu(&svc), "the CPU draws the stage now");
            // Not tried again before its time.
            assert_eq!(m.sync(true, &svc, &ctx, 11.0, (800, 600)), None);
            assert_eq!(STARTS.load(Ordering::SeqCst), 1);
            // Then it is, and draws again.
            let n = settle(&mut m, &svc, &ctx, 13.5, (800, 600));
            assert_eq!(n, vec![Notice::Back]);
            assert_eq!(STARTS.load(Ordering::SeqCst), 2);
            m.sync(true, &svc, &ctx, 13.6, (800, 600));
            assert!(matches!(m.state, State::Running { .. }) && draws_on_gpu(&svc));
            // A second failure waits longer; a resize brings the try forward.
            lose("device lost again");
            let n = m.sync(true, &svc, &ctx, 20.0, (800, 600));
            assert_eq!(n, Some(Notice::Stopped { retry_in: Some(super::super::RETRY_SECS[1]) }));
            assert!(settle(&mut m, &svc, &ctx, 20.5, (800, 600)).is_empty() && STARTS.load(Ordering::SeqCst) == 2, "too soon even when resized");
            // A retry that can't start waits longer still, and the CPU keeps drawing.
            START_FAILS.store(true, Ordering::SeqCst);
            assert!(settle(&mut m, &svc, &ctx, 22.0, (1024, 600)).is_empty());
            assert_eq!(STARTS.load(Ordering::SeqCst), 3, "the resize brought it forward");
            assert!(matches!(&m.state, State::Stopped { retry_at: Some(t), .. } if (*t - (22.0 + super::super::RETRY_SECS[2])).abs() < 1e-9), "{:?}", m.state);
            assert!(!svc.has_rasterizer());
        }

        #[test]
        fn a_lost_window_device_is_not_retried() {
            let _g = crate::gpu_watch::TEST_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            START_FAILS.store(false, Ordering::SeqCst);
            let ctx = egui::Context::default();
            let svc = RenderService::new(DEFAULT_CACHE_BYTES, None);
            let mut m = imp(no_window_loss);
            settle(&mut m, &svc, &ctx, 0.0, (640, 480));
            assert!(matches!(m.state, State::Running { .. }));
            // The window's device goes: the stage stops for good (the window reopens).
            m.window_lost = window_lost;
            lose("the window's graphics device was lost");
            assert_eq!(m.sync(true, &svc, &ctx, 5.0, (640, 480)), Some(Notice::Stopped { retry_in: None }));
            let before = STARTS.load(Ordering::SeqCst);
            assert!(settle(&mut m, &svc, &ctx, 5000.0, (800, 480)).is_empty());
            assert_eq!(STARTS.load(Ordering::SeqCst), before);
            assert!(!svc.has_rasterizer());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The backoff (ADR 0092): growing pauses, a bounded number of tries, a resize bringing a
    /// try forward (not too soon), and a long good run starting the count afresh.
    #[test]
    fn the_gpu_is_tried_again_after_growing_pauses_and_not_for_ever() {
        let mut r = Retry::default();
        assert!(!r.due(0.0, true), "nothing failed");
        assert_eq!(r.failed(100.0), Some(100.0 + RETRY_SECS[0]));
        assert!(!r.due(101.0, false) && r.due(100.0 + RETRY_SECS[0], false));
        assert!(!r.due(100.5, true), "a resize right after the failure waits");
        assert!(r.due(101.5, true), "a resize a moment later tries at once");
        r.trying();
        assert!(!r.due(10_000.0, true), "one try at a time");
        // Failing again at once: each pause longer, then no more.
        let mut t = 200.0;
        for wait in RETRY_SECS.iter().skip(1) {
            assert_eq!(r.failed(t), Some(t + wait));
            t += wait;
        }
        assert_eq!(r.failed(t), None, "after {} failures in a row the CPU keeps the stage", RETRY_SECS.len() + 1);
        assert!(!r.due(1e9, true));
        // A renderer that ran a long while without trouble starts afresh.
        let mut r = Retry::default();
        r.failed(0.0);
        r.failed(10.0);
        r.running(20.0);
        assert_eq!(r.failed(20.0 + STABLE_SECS), Some(20.0 + STABLE_SECS + RETRY_SECS[0]));
        assert_eq!(r.failures(), 1);
        // And a short run doesn't.
        r.running(1000.0);
        assert_eq!(r.failed(1005.0), Some(1005.0 + RETRY_SECS[1]));
    }

    #[test]
    fn notices_read_as_sentences() {
        assert!(notice_text(&Notice::Stopped { retry_in: Some(3.0) }).contains('3'));
        assert!(!notice_text(&Notice::Stopped { retry_in: None }).is_empty());
        assert!(!notice_text(&Notice::Back).is_empty());
    }
}
