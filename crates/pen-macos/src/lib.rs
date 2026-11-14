//! Pen pressure on macOS (ADR 0096).
//!
//! winit 0.30 delivers a mouse event for a stylus and drops the tablet point, and egui never
//! sees trackpad force either. This crate reads that one tablet point and hands back
//! `0.0..=1.0`. The shell stays free of `unsafe`. A mouse is not a pen: turning Speed as
//! pressure off leaves a real pen alone and paints a mouse at full width.
//!
//! Hover is not a sample. A tablet point with the tip up and no pressure is ignored, so a pen
//! resting nearby does not thin a mouse stroke. A tip-down tablet point from the last 30 ms
//! wins over the mouse event the system sends beside it (that copy is often pressure 1, which
//! is the fat dot at the start of a stroke).

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

/// How long a tablet point still belongs to the mouse event the system sends with it.
pub const PEN_WINDOW_SECS: f64 = 0.03;

/// `NSEventTypeTabletPoint`.
const TABLET_POINT: u64 = 23;
/// `NSEventSubtypeTabletPoint` on a mouse event. The same number means something else on other events.
const SUBTYPE_TABLET_POINT: i16 = 1;

static HAVE: AtomicBool = AtomicBool::new(false);
static BITS: AtomicU32 = AtomicU32::new(0);
static AT: AtomicU64 = AtomicU64::new(0);

fn mono() -> f64 {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    START.get_or_init(std::time::Instant::now).elapsed().as_secs_f64()
}

/// Remember a tip-down tablet point. `at` is seconds on the same clock as the read.
fn remember(pressure: f32, at: f64) {
    if !(pressure.is_finite() && at.is_finite()) {
        return;
    }
    BITS.store(pressure.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    AT.store(at.to_bits(), Ordering::Relaxed);
    HAVE.store(true, Ordering::Relaxed);
}

/// The tablet point remembered just now, for the frame that draws it.
fn live() -> Option<f32> {
    pressure_as_of(mono())
}

/// The tablet point at `now`, when it is still inside [`PEN_WINDOW_SECS`].
pub fn pressure_as_of(now: f64) -> Option<f32> {
    if !HAVE.load(Ordering::Relaxed) {
        return None;
    }
    let at = f64::from_bits(AT.load(Ordering::Relaxed));
    if !(at.is_finite() && now.is_finite()) || now + 1e-3 < at || now - at > PEN_WINDOW_SECS {
        return None;
    }
    let p = f32::from_bits(BITS.load(Ordering::Relaxed));
    if p.is_finite() { Some(p.clamp(0.0, 1.0)) } else { None }
}

/// Pressure from one AppKit event, or nothing when it is a mouse, a hover, or not finite.
///
/// `kind` is `NSEventType`, `subtype` is `NSEventSubtype`, `tip_down` is the pen-tip button
/// (tablet points only; pass false for a mouse event).
pub fn classify(kind: u64, subtype: i16, tip_down: bool, pressure: f32) -> Option<f32> {
    if !pressure.is_finite() {
        return None;
    }
    let p = pressure.clamp(0.0, 1.0);
    if kind == TABLET_POINT {
        // Hover reports pressure 0 with the tip up. A light touch is either the tip button or
        // a pressure above none, including a driver that leaves the button mask empty.
        if tip_down || p > 0.0 { Some(p) } else { None }
    } else if is_mouse(kind) && subtype == SUBTYPE_TABLET_POINT {
        Some(p)
    } else {
        None
    }
}

fn is_mouse(kind: u64) -> bool {
    // Left/right/other mouse down, up, and dragged. Not moved, not entered.
    matches!(kind, 1 | 2 | 3 | 4 | 6 | 7 | 25 | 26 | 27)
}

/// Start listening. Safe to call more than once. No-op off the main thread and off macOS.
pub fn install() {
    #[cfg(target_os = "macos")]
    imp::install();
}

/// The pen's pressure for the event being handled, when it is a stylus.
pub fn current() -> Option<f32> {
    #[cfg(target_os = "macos")]
    {
        imp::current()
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use super::{classify, live, remember};
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSEvent, NSEventButtonMask, NSEventMask, NSEventType};
    use std::ptr::NonNull;
    use std::sync::atomic::{AtomicBool, Ordering};

    pub fn install() {
        static STARTED: AtomicBool = AtomicBool::new(false);
        if STARTED.swap(true, Ordering::Relaxed) {
            return;
        }
        if MainThreadMarker::new().is_none() {
            STARTED.store(false, Ordering::Relaxed);
            return;
        }
        let block = block2::RcBlock::new(|event: NonNull<NSEvent>| -> *mut NSEvent {
            // A throw from a tablet field must not leave this callback. Rust cannot unwind through it.
            let _ = objc2::exception::catch(std::panic::AssertUnwindSafe(|| {
                // SAFETY: AppKit calls the monitor with the event it is dispatching, alive for this call.
                let ev = unsafe { event.as_ref() };
                note(ev);
            }));
            event.as_ptr()
        });
        let mask = NSEventMask::LeftMouseDown
            | NSEventMask::LeftMouseUp
            | NSEventMask::LeftMouseDragged
            | NSEventMask::RightMouseDown
            | NSEventMask::RightMouseUp
            | NSEventMask::RightMouseDragged
            | NSEventMask::OtherMouseDown
            | NSEventMask::OtherMouseUp
            | NSEventMask::OtherMouseDragged
            | NSEventMask::TabletPoint;
        // SAFETY: the block returns the same event pointer it was given, so delivery is unchanged.
        // AppKit removes the monitor if the token is dropped, so both live for the process.
        let token = unsafe { NSEvent::addLocalMonitorForEventsMatchingMask_handler(mask, &block) };
        std::mem::forget(block);
        std::mem::forget(token);
    }

    pub fn current() -> Option<f32> {
        install();
        if let Some(mtm) = MainThreadMarker::new()
            && let Some(event) = NSApplication::sharedApplication(mtm).currentEvent()
        {
            note(&event);
            // The tablet point wins over the mouse event sent with it, which often says pressure 1.
            if let Some(p) = live() {
                return Some(p);
            }
            return event_pressure(&event);
        }
        live()
    }

    fn note(event: &NSEvent) {
        if event.r#type() == NSEventType::TabletPoint
            && let Some(p) = event_pressure(event)
        {
            remember(p, super::mono());
        }
    }

    /// Pressure for a tablet point or a mouse event that carries one.
    ///
    /// `pressure` and `subtype` raise if the event is a key, a scroll, or anything else.
    /// That exception cannot cross into Rust, so the read is limited to the event kinds
    /// that have those fields, and a throw becomes "no pen" instead of aborting the process.
    fn event_pressure(event: &NSEvent) -> Option<f32> {
        let read = objc2::exception::catch(std::panic::AssertUnwindSafe(|| {
            let ty = event.r#type();
            let kind = ty.0 as u64;
            if ty != NSEventType::TabletPoint && !super::is_mouse(kind) {
                return None;
            }
            let subtype = event.subtype().0;
            let tip = ty == NSEventType::TabletPoint && event.buttonMask().contains(NSEventButtonMask::PenTip);
            classify(kind, subtype, tip, event.pressure())
        }));
        read.unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pen_is_a_tablet_point_and_a_mouse_is_not() {
        assert_eq!(classify(TABLET_POINT, 0, true, 0.0), Some(0.0), "tip down, no force: thinnest");
        assert_eq!(classify(TABLET_POINT, 0, true, 0.12), Some(0.12));
        assert_eq!(classify(TABLET_POINT, 0, false, 0.2), Some(0.2), "a driver that omits the button");
        assert_eq!(classify(TABLET_POINT, 0, false, 0.0), None, "hover");
        assert_eq!(classify(TABLET_POINT, 0, true, f32::NAN), None);
        assert_eq!(classify(TABLET_POINT, 0, true, 2.0), Some(1.0));
        // Left mouse dragged, carrying a tablet point.
        assert_eq!(classify(6, SUBTYPE_TABLET_POINT, false, 0.05), Some(0.05));
        // The same drag as an ordinary mouse, even if something left a pressure in the event.
        assert_eq!(classify(6, 0, false, 0.4), None);
        assert_eq!(classify(1, 0, false, 1.0), None, "mouse down is full width, not a pen");
        assert_eq!(classify(10, SUBTYPE_TABLET_POINT, false, 0.5), None, "a key is not a pen");
    }

    #[test]
    fn a_tablet_point_covers_the_mouse_copy_and_then_expires() {
        remember(0.12, 10.0);
        assert_eq!(pressure_as_of(10.02), Some(0.12));
        assert_eq!(pressure_as_of(10.0 + PEN_WINDOW_SECS + 0.01), None);
        remember(0.0, 20.0);
        assert_eq!(pressure_as_of(20.0), Some(0.0), "a light touch is a real sample");
    }
}
