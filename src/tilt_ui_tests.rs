//! The brushes' Tilt options (ADR 0093) show only once a pen has reported its tilt: until then
//! they would do nothing.

use crate::app::{App, Tool};
use egui::{Rect, vec2};

/// The labels drawn in a few frames of the app with the Brush (or the Paint Brush) in hand.
fn labels(tool: Tool, tilt_seen: bool) -> Vec<String> {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let session = anicraft_engine::Session::new(anicraft_engine::doc::sample::demo());
    let mut app = App::with_context(&ctx, session, Some(std::path::PathBuf::from("unused.png")));
    app.tool = tool;
    crate::brushtool::note_tilt(&mut app, tilt_seen.then_some(anicraft_engine::geom::brush::Tilt { altitude: 40.0, azimuth: 10.0 }));
    let mut nodes = Vec::new();
    for _ in 0..4 {
        let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(1600.0, 1400.0))), ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| app.frame(ui));
        out.textures_delta.clear();
        nodes = out.platform_output.accesskit_update.map(|u| u.nodes.into_iter().map(|(_, n)| n).collect()).unwrap_or_default();
    }
    nodes.iter().filter_map(|n| n.label().map(str::to_string)).collect()
}

fn count(labels: &[String], l: &str) -> usize {
    labels.iter().filter(|x| *x == l).count()
}

#[test]
fn tilt_options_hide_until_a_pen_reports_tilt() {
    // The rows around it show, so Properties is drawn.
    let before = labels(Tool::Brush, false);
    assert!(before.iter().any(|l| l == "Texture"), "the Brush options are drawn: {before:?}");
    // No tilt reported: no Tilt row. Once a pen has reported tilt: its Angle and Width.
    let after = labels(Tool::Brush, true);
    assert_eq!(count(&after, "Angle"), count(&before, "Angle") + 1);
    assert_eq!(count(&after, "Width"), count(&before, "Width") + 1);
    // The Paint Brush has Width only.
    let (pb0, pb1) = (labels(Tool::PaintBrush, false), labels(Tool::PaintBrush, true));
    assert_eq!(count(&pb1, "Angle"), count(&pb0, "Angle"));
    assert_eq!(count(&pb1, "Width"), count(&pb0, "Width") + 1);
    // On the desktop nothing reports tilt yet.
    assert!(crate::brushtool::frame_tilt(&egui::Context::default()).is_none());
}
