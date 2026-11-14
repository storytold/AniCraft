//! Pen pressure on Linux. winit 0.30 drops it, and native Wayland gives a pen no input at all, so
//! with a pen attached on Wayland the window opens through Xwayland (`display_session`), where
//! `anicraft-tablet` reads XInput2 raw events and parks the latest sample here for the canvas.
//! Elsewhere nothing writes the feed and [`pressure`] is `None` (touch force covers Windows/macOS).

use std::sync::Mutex;

static FEED: Mutex<Option<f32>> = Mutex::new(None);

/// The pen's pressure 0..1 while a pen is in use, `None` for a mouse or without a reader.
pub fn pressure() -> Option<f32> {
    FEED.lock().ok().and_then(|g| *g)
}

#[cfg(target_os = "linux")]
fn set(p: Option<f32>) {
    if let Ok(mut g) = FEED.lock() {
        *g = p;
    }
}

/// The key a pen's stylus reports (linux/input-event-codes.h `BTN_TOOL_PEN`).
#[cfg(any(target_os = "linux", test))]
const BTN_TOOL_PEN: u32 = 0x140;

/// Is a pen among the devices `/proc/bus/input/devices` lists? Each `B: KEY=` line is the key
/// bitmap as hex words of the kernel's `long`, most significant first.
#[cfg(any(target_os = "linux", test))]
fn has_pen(devices: &str) -> bool {
    let bits = usize::BITS;
    let word = usize::try_from(BTN_TOOL_PEN / bits).unwrap_or(usize::MAX);
    devices.lines().filter_map(|l| l.strip_prefix("B: KEY=")).any(|keys| {
        keys.split_whitespace().rev().nth(word).and_then(|w| u64::from_str_radix(w, 16).ok()).is_some_and(|w| (w >> (BTN_TOOL_PEN % bits)) & 1 == 1)
    })
}

/// Should the window open through Xwayland? On a Wayland session with a pen attached and
/// `$DISPLAY` set, unless `ANICRAFT_NATIVE_WAYLAND=1` keeps native Wayland.
#[cfg(any(target_os = "linux", test))]
fn needs_xwayland(var: impl Fn(&str) -> Option<String>, pen: impl FnOnce() -> bool) -> bool {
    let set = |k: &str| var(k).is_some_and(|v| !v.is_empty() && v != "0");
    (set("WAYLAND_DISPLAY") || set("WAYLAND_SOCKET")) && !set("ANICRAFT_NATIVE_WAYLAND") && set("DISPLAY") && pen()
}

/// Whether to force the X11 backend (Xwayland on Wayland) because a pen is attached.
#[cfg(target_os = "linux")]
pub fn use_xwayland() -> bool {
    let pen = || std::fs::read_to_string("/proc/bus/input/devices").is_ok_and(|d| has_pen(&d));
    let yes = needs_xwayland(|k| std::env::var(k).ok(), pen);
    if yes {
        eprintln!("anicraft: pen found; opening the window through Xwayland, where the pen works (ANICRAFT_NATIVE_WAYLAND=1 keeps Wayland)");
    }
    yes
}

/// Make eframe open on X11 (Xwayland on a Wayland session).
#[cfg(target_os = "linux")]
pub fn force_x11(options: &mut eframe::NativeOptions) {
    use winit::platform::x11::EventLoopBuilderExtX11 as _;
    options.event_loop_builder = Some(Box::new(|builder| {
        builder.with_x11();
    }));
}

/// Start the XInput2 reader. Only on X11: on native Wayland `$DISPLAY` sees none of this window's input.
#[cfg(target_os = "linux")]
pub fn spawn(on_x11: bool) {
    if !on_x11 {
        return;
    }
    if let Err(e) = anicraft_tablet::x11::spawn(None, |s| set(s.map(|s| s.pressure))) {
        eprintln!("anicraft: {e}"); // i18n-exempt: a console message
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_a_pen_in_the_kernels_device_list() {
        // The key bitmap's words are the kernel's `long`s, most significant first; BTN_TOOL_PEN
        // (0x140) is bit 0 of word 320 / BITS counted from the least significant end.
        let word = (BTN_TOOL_PEN / usize::BITS) as usize;
        let words = |set: Option<(usize, u32)>| {
            (0..=word + 1).rev().map(|w| format!("{:x}", set.filter(|(sw, _)| *sw == w).map_or(0u64, |(_, b)| 1 << b))).collect::<Vec<_>>().join(" ")
        };
        let pen = format!("N: Name=\"Wacom Intuos S Pen\"\nB: KEY={}\n", words(Some((word, BTN_TOOL_PEN % usize::BITS))));
        assert!(has_pen(&pen), "{pen}");
        assert!(!has_pen(&format!("B: KEY={}\n", words(None))), "a keyboard-only bitmap");
        assert!(!has_pen(&format!("B: KEY={}\n", words(Some((word, 1))))), "a neighbouring key");
        assert!(!has_pen("B: KEY=ffffffff\n"), "bitmap too short to reach the key");
        assert!(!has_pen(""));
    }

    #[test]
    fn xwayland_only_for_a_pen_on_wayland() {
        let env = |wayland: bool| {
            move |k: &str| match k {
                "WAYLAND_DISPLAY" if wayland => Some("wayland-1".into()),
                "DISPLAY" => Some(":0".into()),
                _ => None,
            }
        };
        assert!(needs_xwayland(env(true), || true));
        assert!(!needs_xwayland(env(true), || false), "no pen");
        assert!(!needs_xwayland(env(false), || true), "already X11");
        let opt_out = |k: &str| if k == "ANICRAFT_NATIVE_WAYLAND" { Some("1".into()) } else { env(true)(k) };
        assert!(!needs_xwayland(opt_out, || true));
    }
}
