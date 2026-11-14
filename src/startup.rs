//! Starting the window whatever the graphics driver does (#207, ADR 0006), and the start-up log
//! users can attach to a bug report.
//!
//! The window is opened by a list of attempts ([`plan`]): the interface asked for (`--renderer`,
//! `ANICRAFT_RENDERER`, or Preferences › Stage › Window graphics), then the platform's order
//! ([`crate::platform::window_attempts`]), ending with OpenGL. An attempt that returns an error
//! or panics before the window has drawn falls through to the next one; when none works, a
//! system message box says so and where the log is, so AniCraft never just disappears.
//!
//! A driver can also crash the process or hang, which nothing inside it can catch. Each attempt
//! leaves a marker in the log folder until the window has drawn a few frames; if the next start
//! finds one, the previous start died in that attempt, so it goes last this time. The log (the
//! previous run's is kept beside it) records each attempt, the graphics adapter and driver,
//! panics, and a note when the window is still not up after a while.

use crate::prefs::WindowBackend;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};

/// One way to open the window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Attempt {
    pub backend: WindowBackend,
    /// Ask for the low-power (integrated) GPU rather than the fastest one.
    pub low_power: bool,
}

impl Attempt {
    /// A stable name for the log and the crash marker (English: it is a log).
    pub fn label(&self) -> String {
        let name = match self.backend {
            WindowBackend::Auto => "wgpu (automatic)", // i18n-exempt: log text
            WindowBackend::Vulkan => "wgpu Vulkan", // i18n-exempt: log text
            WindowBackend::Dx12 => "wgpu DirectX 12", // i18n-exempt: log text
            WindowBackend::Metal => "wgpu Metal", // i18n-exempt: log text
            WindowBackend::OpenGl => "OpenGL", // i18n-exempt: log text
        };
        if self.low_power && self.backend != WindowBackend::OpenGl { format!("{name}, low-power GPU") } else { name.to_string() }
    }

    /// `base` set up for this attempt.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn options(&self, mut base: eframe::NativeOptions) -> eframe::NativeOptions {
        use eframe::egui_wgpu::{WgpuSetup, WgpuSetupCreateNew, wgpu};
        if self.backend == WindowBackend::OpenGl {
            base.renderer = eframe::Renderer::Glow;
            return base;
        }
        base.renderer = eframe::Renderer::Wgpu;
        // A surface that fails is reconfigured, then made anew, instead of the frame being
        // skipped for ever (ADR 0092).
        base.wgpu_options.on_surface_status = std::sync::Arc::new(crate::gpu_watch::on_surface_status);
        let mut setup = WgpuSetupCreateNew::without_display_handle();
        match self.backend {
            WindowBackend::Vulkan => setup.instance_descriptor.backends = wgpu::Backends::VULKAN,
            WindowBackend::Dx12 => setup.instance_descriptor.backends = wgpu::Backends::DX12,
            WindowBackend::Metal => setup.instance_descriptor.backends = wgpu::Backends::METAL,
            WindowBackend::Auto | WindowBackend::OpenGl => {}
        }
        if self.low_power {
            setup.power_preference = wgpu::PowerPreference::LowPower;
        }
        base.wgpu_options.wgpu_setup = WgpuSetup::CreateNew(setup);
        base
    }
}

/// The attempts in order: `forced` (the command line) or else `chosen` (Preferences) first, then
/// the platform's order `platform`, each once. The attempts named in `last` (the previous start
/// crashed or hung in one; a session lost its graphics device in another, ADR 0092) go last, in
/// that order.
pub fn plan(forced: Option<WindowBackend>, chosen: WindowBackend, platform: &[(WindowBackend, bool)], last: &[String]) -> Vec<Attempt> {
    let mut out: Vec<Attempt> = Vec::new();
    let first = forced.unwrap_or(chosen);
    if first != WindowBackend::Auto {
        out.push(Attempt { backend: first, low_power: false });
    }
    for &(backend, low_power) in platform {
        let a = Attempt { backend, low_power };
        if !out.iter().any(|o| o.label() == a.label()) {
            out.push(a);
        }
    }
    if out.is_empty() {
        out.push(Attempt { backend: WindowBackend::Auto, low_power: false });
    }
    for dead in last {
        if let Some(i) = out.iter().position(|a| a.label() == *dead)
            && out.len() > 1
        {
            let a = out.remove(i);
            out.push(a);
        }
    }
    out
}

/// Starts the record of a mid-session loss counts for: that many starts try another way first,
/// then the fastest way gets its chance again (the driver may have been updated, or the load
/// that brought it down gone).
pub const LOST_STARTS: u32 = 3;

/// The record a mid-session loss leaves (ADR 0092): the attempt's label and the starts it still
/// counts for, one per line.
pub fn lost_record(text: &str) -> Option<(String, u32)> {
    let mut lines = text.lines();
    let label = lines.next()?.trim().chars().take(200).collect::<String>();
    let left = lines.next().and_then(|l| l.trim().parse::<u32>().ok()).unwrap_or(0).min(LOST_STARTS);
    (!label.is_empty() && left > 0).then_some((label, left))
}

fn lost_record_text(label: &str, left: u32) -> String {
    format!("{label}\n{left}\n")
}

/// Longest log written in one run: enough for any start-up, bounded however long AniCraft runs.
const MAX_LOG_BYTES: u64 = 1024 * 1024;
/// Frames the window draws before the start counts as working.
const STARTED_AFTER: u32 = 3;
/// A note goes in the log when the window still isn't up after this long.
const SLOW_START_SECS: u64 = 20;

const LOG_NAME: &str = "anicraft.log";
const PREVIOUS_LOG_NAME: &str = "anicraft.previous.log";
const MARKER_NAME: &str = "starting";
const LOST_NAME: &str = "graphics-lost";

struct Log {
    dir: PathBuf,
    file: Option<std::fs::File>,
    since: std::time::Instant,
    written: u64,
}

static LOG: Mutex<Option<Log>> = Mutex::new(None);
static FRAMES: AtomicU32 = AtomicU32::new(0);
/// The attempt the window runs on now (for the record a mid-session loss leaves).
static CURRENT: Mutex<Option<String>> = Mutex::new(None);
/// The window closed to open again another way (ADR 0092), with the document to open then.
static REOPEN: Mutex<Option<Option<PathBuf>>> = Mutex::new(None);

/// Opens the log in `dir` for this run (the previous run's becomes `anicraft.previous.log`) and
/// returns the attempts to try last: the one an earlier session lost its graphics device in (for
/// [`LOST_STARTS`] starts, ADR 0092), then the one the previous start died in, if its marker is
/// still there. Panics are written to the log from now on.
pub fn begin(dir: Option<PathBuf>) -> Vec<String> {
    let Some(dir) = dir else { return Vec::new() };
    let _ = std::fs::create_dir_all(&dir);
    let died_in = std::fs::read_to_string(dir.join(MARKER_NAME)).ok().map(|s| s.trim().chars().take(200).collect::<String>()).filter(|s| !s.is_empty());
    // This start uses up one of the record's starts.
    let lost = std::fs::read_to_string(dir.join(LOST_NAME)).ok().and_then(|t| lost_record(&t));
    match &lost {
        Some((label, left)) if *left > 1 => {
            let _ = std::fs::write(dir.join(LOST_NAME), lost_record_text(label, left - 1));
        }
        _ => {
            let _ = std::fs::remove_file(dir.join(LOST_NAME));
        }
    }
    let _ = std::fs::rename(dir.join(LOG_NAME), dir.join(PREVIOUS_LOG_NAME));
    let file = std::fs::File::create(dir.join(LOG_NAME)).ok();
    if let Ok(mut g) = LOG.lock() {
        *g = Some(Log { dir, file, since: std::time::Instant::now(), written: 0 });
    }
    FRAMES.store(0, Ordering::Relaxed);
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // Never waits: the panic may have happened while the log was being written.
        write(&format!("panic: {info}"), false);
        previous(info);
    }));
    let unix = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    line(&format!("AniCraft {} ({}), {} {}, started at {unix} (Unix time)", env!("CARGO_PKG_VERSION"), crate::app::RELEASE_STAGE, std::env::consts::OS, std::env::consts::ARCH));
    if let Some((l, left)) = &lost {
        line(&format!("a session lost its graphics device with {l}: trying it late ({left} more start(s) counting this one)"));
    }
    if let Some(d) = &died_in {
        line(&format!("the previous start stopped while opening the window with {d}: trying it last"));
    }
    lost.map(|(l, _)| l).into_iter().chain(died_in).collect()
}

/// The window's graphics device was lost after it was up (ADR 0092): the next [`LOST_STARTS`]
/// starts try the attempt it ran on late.
pub fn lost_mid_session() {
    let Some(label) = current() else { return };
    let dir = LOG.lock().ok().and_then(|g| g.as_ref().map(|l| l.dir.clone()));
    if let Some(d) = dir {
        let _ = std::fs::write(d.join(LOST_NAME), lost_record_text(&label, LOST_STARTS));
    }
    line(&format!("the next {LOST_STARTS} starts try {label} late"));
}

/// The attempt the window runs on now.
pub fn current() -> Option<String> {
    CURRENT.lock().ok().and_then(|g| g.clone())
}

/// The window is closing to open again another way, then with `file` (the app asks this when
/// its graphics device is gone).
pub fn ask_reopen(file: Option<PathBuf>) {
    if let Ok(mut g) = REOPEN.lock() {
        *g = Some(file);
    }
}

/// Whether the window that just closed asked to open again, and with which document.
pub fn take_reopen() -> Option<Option<PathBuf>> {
    REOPEN.lock().ok().and_then(|mut g| g.take())
}

/// Whether a failure after the window was up came from the graphics (wgpu, its surface, the
/// driver): then it counts as a mid-session loss.
pub fn is_graphics_failure(why: &str) -> bool {
    let w = why.to_ascii_lowercase();
    ["wgpu", "surface", "device lost", "gpu", "dxgi", "vk_error", "vulkan", "direct3d"].iter().any(|k| w.contains(k)) // i18n-exempt: words looked for in a log line
}

/// Adds a line to the log (and the console in debug builds), with the seconds since it began.
pub fn line(text: &str) {
    write(text, true);
}

fn write(text: &str, wait: bool) {
    #[cfg(debug_assertions)]
    eprintln!("{text}");
    let Ok(mut g) = (if wait { LOG.lock().map_err(|_| ()) } else { LOG.try_lock().map_err(|_| ()) }) else { return };
    let Some(log) = g.as_mut() else { return };
    let Some(f) = log.file.as_mut() else { return };
    if log.written > MAX_LOG_BYTES {
        return;
    }
    let entry = format!("[{:8.3} s] {}\n", log.since.elapsed().as_secs_f64(), text.chars().take(4000).collect::<String>());
    log.written += entry.len() as u64;
    // Flushed at once: the next thing may be the driver taking the process down.
    let _ = f.write_all(entry.as_bytes()).and_then(|()| f.flush());
}

/// Where the log is, when there is one.
pub fn log_path() -> Option<PathBuf> {
    LOG.lock().ok()?.as_ref().map(|l| l.dir.join(LOG_NAME))
}

/// About to try `a`: logged, and the marker says so until the window has drawn. A watchdog
/// notes in the log if the window is still not up after a while (a driver that hangs).
pub fn trying(a: &Attempt, n: usize, of: usize) {
    line(&format!("opening the window with {} (attempt {n} of {of})", a.label()));
    if let Ok(mut g) = CURRENT.lock() {
        *g = Some(a.label());
    }
    let marker = LOG.lock().ok().and_then(|g| g.as_ref().map(|l| l.dir.join(MARKER_NAME)));
    if let Some(m) = marker {
        let _ = std::fs::write(m, a.label());
    }
    FRAMES.store(0, Ordering::Relaxed);
    let label = a.label();
    let _ = std::thread::Builder::new().name("anicraft-start-watch".into()).spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(SLOW_START_SECS));
        if FRAMES.load(Ordering::Relaxed) == 0 {
            line(&format!("the window has not drawn after {SLOW_START_SECS} s with {label} (a graphics driver that hangs?)"));
        }
    });
}

/// Called once a frame by the window: after a few frames the start counts as working and the
/// marker goes.
pub fn frame() {
    if FRAMES.fetch_add(1, Ordering::Relaxed) + 1 == STARTED_AFTER {
        line(&format!("the window is up ({STARTED_AFTER} frames drawn)"));
        let marker = LOG.lock().ok().and_then(|g| g.as_ref().map(|l| l.dir.join(MARKER_NAME)));
        if let Some(m) = marker {
            let _ = std::fs::remove_file(m);
        }
    }
}

/// Whether the window of the current attempt has drawn (then a failure is not retried: it ran).
pub fn started() -> bool {
    FRAMES.load(Ordering::Relaxed) >= STARTED_AFTER
}

/// The adapter and driver the window got (wgpu), for the log.
#[cfg(not(target_arch = "wasm32"))]
pub fn log_adapter(rs: Option<&eframe::egui_wgpu::RenderState>) {
    match rs {
        Some(rs) => {
            let i = rs.adapter.get_info();
            line(&format!("window graphics: {} ({:?}, {:?}), driver {} {}", i.name, i.backend, i.device_type, i.driver, i.driver_info));
        }
        None => line("window graphics: OpenGL"), // i18n-exempt: log text
    }
}

/// The text of a panic's payload.
pub fn panic_text(p: &(dyn std::any::Any + Send)) -> String {
    p.downcast_ref::<&str>().map(|s| (*s).to_string()).or_else(|| p.downcast_ref::<String>().cloned()).unwrap_or_else(|| "a panic".into()) // i18n-exempt: log text
}

/// Every attempt failed: the log says why, and a system message box says so (the window
/// can't), with where the log is.
#[cfg(not(target_arch = "wasm32"))]
pub fn give_up(why: &str) {
    line(&format!("no way to open the window worked; last: {why}"));
    let log = log_path().map_or_else(String::new, |p| p.display().to_string());
    let text = if log.is_empty() {
        tr!("AniCraft couldn't open its window with this computer's graphics driver ({why}). Updating the graphics driver usually helps.", why = why).to_string()
    } else {
        tr!("AniCraft couldn't open its window with this computer's graphics driver ({why}). Updating the graphics driver usually helps. If it doesn't, please attach this file to a bug report: {log}", why = why, log = log).to_string()
    };
    let _ = rfd::MessageDialog::new().set_level(rfd::MessageLevel::Error).set_title(tr!("AniCraft couldn't start")).set_description(text).set_buttons(rfd::MessageButtons::Ok).show();
}

#[cfg(test)]
mod tests {
    use super::*;
    use WindowBackend::*;

    const WINDOWS: &[(WindowBackend, bool)] = &[(Auto, false), (Dx12, true), (OpenGl, false)];

    fn labels(v: &[Attempt]) -> Vec<String> {
        v.iter().map(Attempt::label).collect()
    }

    #[test]
    fn automatic_tries_the_platform_order_ending_with_opengl() {
        assert_eq!(labels(&plan(None, Auto, WINDOWS, &[])), ["wgpu (automatic)", "wgpu DirectX 12, low-power GPU", "OpenGL"]);
    }

    #[test]
    fn a_chosen_or_forced_interface_goes_first_and_still_falls_back() {
        assert_eq!(labels(&plan(None, Vulkan, WINDOWS, &[])), ["wgpu Vulkan", "wgpu (automatic)", "wgpu DirectX 12, low-power GPU", "OpenGL"]);
        // The command line wins over Preferences; OpenGL isn't tried twice.
        assert_eq!(labels(&plan(Some(OpenGl), Dx12, WINDOWS, &[])), ["OpenGL", "wgpu (automatic)", "wgpu DirectX 12, low-power GPU"]);
        assert_eq!(labels(&plan(Some(Auto), Dx12, WINDOWS, &[])), ["wgpu (automatic)", "wgpu DirectX 12, low-power GPU", "OpenGL"], "--renderer wgpu is automatic");
    }

    /// Each attempt sets the window up as it says: OpenGL through glow, the others through
    /// wgpu on their interface and GPU.
    #[test]
    fn each_attempt_sets_up_the_window_as_it_says() {
        use eframe::egui_wgpu::{WgpuSetup, wgpu};
        let setup = |a: Attempt| {
            let o = a.options(eframe::NativeOptions::default());
            let wgpu = match o.wgpu_options.wgpu_setup {
                WgpuSetup::CreateNew(c) => Some((c.instance_descriptor.backends, c.power_preference)),
                _ => None,
            };
            (o.renderer, wgpu)
        };
        assert_eq!(setup(Attempt { backend: OpenGl, low_power: false }).0, eframe::Renderer::Glow);
        assert_eq!(setup(Attempt { backend: Dx12, low_power: true }), (eframe::Renderer::Wgpu, Some((wgpu::Backends::DX12, wgpu::PowerPreference::LowPower))));
        assert_eq!(setup(Attempt { backend: Vulkan, low_power: false }).1.map(|w| w.0), Some(wgpu::Backends::VULKAN));
        assert_eq!(setup(Attempt { backend: Metal, low_power: false }).1.map(|w| w.0), Some(wgpu::Backends::METAL));
        assert_eq!(setup(Attempt { backend: Auto, low_power: false }).0, eframe::Renderer::Wgpu);
        assert_eq!(Attempt { backend: OpenGl, low_power: true }.label(), "OpenGL", "OpenGL has no GPU choice");
    }

    #[test]
    fn panics_read_as_text() {
        assert_eq!(panic_text(&"static"), "static");
        assert_eq!(panic_text(&String::from("owned")), "owned");
        assert_eq!(panic_text(&42), "a panic");
        // No window: logged as OpenGL, never a panic (nothing is logged without `begin`).
        log_adapter(None);
    }

    /// #207: when the last start died inside a driver (nothing to catch), that way goes last.
    #[test]
    fn the_way_the_last_start_died_in_goes_last() {
        assert_eq!(labels(&plan(None, Auto, WINDOWS, &["wgpu (automatic)".into()])), ["wgpu DirectX 12, low-power GPU", "OpenGL", "wgpu (automatic)"]);
        assert_eq!(labels(&plan(None, Auto, WINDOWS, &["something else".into()])), ["wgpu (automatic)", "wgpu DirectX 12, low-power GPU", "OpenGL"]);
        assert_eq!(labels(&plan(None, Auto, &[], &[])), ["wgpu (automatic)"], "never nothing to try");
    }

    /// ADR 0092: a session that lost its graphics device mid-way makes the next starts try that
    /// way late (after a start that died in a driver, which goes last of all), for
    /// [`LOST_STARTS`] starts; then the fastest way gets its chance again.
    #[test]
    fn the_plan_after_a_mid_session_death_tries_that_way_late() {
        assert_eq!(lost_record("wgpu (automatic)\n3\n"), Some(("wgpu (automatic)".into(), 3)));
        assert_eq!(lost_record("wgpu (automatic)\n99"), Some(("wgpu (automatic)".into(), LOST_STARTS)), "never more than the limit");
        assert_eq!(lost_record("wgpu (automatic)\n0"), None);
        assert_eq!(lost_record("\n2"), None);
        assert_eq!(lost_record("garbage"), None);
        let last = ["wgpu (automatic)".to_string(), "wgpu DirectX 12, low-power GPU".to_string()];
        assert_eq!(labels(&plan(None, Auto, WINDOWS, &last)), ["OpenGL", "wgpu (automatic)", "wgpu DirectX 12, low-power GPU"]);
        assert_eq!(labels(&plan(None, Auto, WINDOWS, &last[..1])), ["wgpu DirectX 12, low-power GPU", "OpenGL", "wgpu (automatic)"]);
        assert!(is_graphics_failure("wgpu error: Validation Error\n\nCaused by:\n  In Surface::configure"));
        assert!(is_graphics_failure("Failed to wait for GPU to come idle before reconfiguring the Surface"));
        assert!(!is_graphics_failure("index out of bounds: the len is 3 but the index is 7"));
    }

    /// The log, the crash marker and the previous run's log, in a folder of their own.
    #[test]
    fn the_log_keeps_the_previous_run_and_finds_a_crashed_start() {
        let dir = std::env::temp_dir().join(format!("anicraft-startup-log-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(begin(Some(dir.clone())).is_empty(), "a first start");
        trying(&Attempt { backend: Dx12, low_power: true }, 1, 3);
        line("hello");
        // Killed here (a driver crash): the marker stays, and the next start finds it.
        assert_eq!(begin(Some(dir.clone())), ["wgpu DirectX 12, low-power GPU"]);
        let previous = std::fs::read_to_string(dir.join(PREVIOUS_LOG_NAME)).unwrap();
        assert!(previous.contains("opening the window with wgpu DirectX 12, low-power GPU (attempt 1 of 3)") && previous.contains("hello"), "{previous}");
        let now = std::fs::read_to_string(dir.join(LOG_NAME)).unwrap();
        assert!(now.contains("the previous start stopped while opening the window with wgpu DirectX 12"), "{now}");
        // This time the window comes up: the marker goes.
        trying(&Attempt { backend: OpenGl, low_power: false }, 1, 1);
        for _ in 0..STARTED_AFTER {
            frame();
        }
        assert!(started());
        assert!(!dir.join(MARKER_NAME).exists());
        assert_eq!(log_path(), Some(dir.join(LOG_NAME)));
        // The window lost its device mid-session: the next starts try it late, then not.
        lost_mid_session();
        for left in (1..=LOST_STARTS).rev() {
            assert_eq!(begin(Some(dir.clone())).first().map(String::as_str), Some("OpenGL"), "{left} start(s) left");
        }
        assert!(begin(Some(dir.clone())).is_empty(), "the record is used up");
        // A panic is written down.
        let _ = std::panic::catch_unwind(|| panic!("the driver said no"));
        assert!(std::fs::read_to_string(dir.join(LOG_NAME)).unwrap().contains("the driver said no"));
        // Closed, so the folder can go (Windows keeps open files).
        if let Ok(mut g) = LOG.lock() {
            *g = None;
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
