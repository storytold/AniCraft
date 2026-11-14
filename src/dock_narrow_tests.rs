//! Every panel at the narrowest a side dock goes (ADR 0087): no widget drawn past the panel's
//! edge, in English and in the pseudo-locale (longer words), with the states that fill
//! Properties most.

use crate::app::App;
use crate::dock::{Group, Layout, Panel, Side, Zone};
use crate::dock_ui::MIN_SIDE;
use egui::{Rect, vec2};

fn app(pseudo: bool) -> (App, egui::Context) {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let session = anicraft_engine::Session::new(anicraft_engine::doc::sample::demo());
    let mut app = App::with_context(&ctx, session, Some(std::path::PathBuf::from("unused.png")));
    if pseudo {
        app.prefs.language = crate::i18n::PSEUDO.into();
        app.apply_prefs(&ctx);
    }
    (app, ctx)
}

/// A few frames; the last one's accessibility tree (every label and control, with its bounds).
fn run(ctx: &egui::Context, app: &mut App) -> Vec<egui::accesskit::Node> {
    let mut nodes = Vec::new();
    for _ in 0..4 {
        let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(1600.0, 940.0))), ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| app.frame(ui));
        out.textures_delta.clear();
        nodes = out.platform_output.accesskit_update.map(|u| u.nodes.into_iter().map(|(_, n)| n).collect()).unwrap_or_default();
    }
    nodes
}

/// `p` alone in the right dock at its narrowest; the labels and controls drawn past its body's
/// edge.
fn overflow(app: &mut App, ctx: &egui::Context, p: Panel) -> Vec<String> {
    use egui::accesskit::Role;
    let mut l = Layout { tools_bar: Some(Side::Left), ..Layout::default() };
    l.right = Zone { groups: vec![Group { tabs: vec![p], ..Group::default() }], size: Some(MIN_SIDE), collapsed: false };
    app.dock = l;
    app.dock_view.epoch += 1;
    let nodes = run(ctx, app);
    let Some(d) = app.dock_view.drawn.iter().find(|d| d.first == p) else { return vec![format!("{p:?} not drawn")] };
    let body = Rect::from_min_max(egui::pos2(d.rect.left(), d.header.bottom()), d.rect.max);
    assert!(body.width() <= MIN_SIDE + 1.0, "{p:?} at its narrowest: {}", body.width());
    let containers = [Role::GenericContainer, Role::ScrollView, Role::Window, Role::Pane, Role::Group, Role::Unknown, Role::ScrollBar];
    nodes
        .iter()
        .filter(|n| !containers.contains(&n.role()))
        .filter_map(|n| n.bounds().map(|b| (n, b)))
        // In the panel (scrolled out of view or not)...
        .filter(|(_, b)| b.x1 > b.x0 && b.x0 >= f64::from(body.left()) - 0.5 && b.x0 < f64::from(body.right()) && b.y0 >= f64::from(body.top()) - 0.5 && b.y0 < f64::from(body.bottom()))
        // ...and running past its edge.
        .filter(|(_, b)| b.x1 > f64::from(body.right()) + 0.5)
        .map(|(n, b)| format!("{p:?}: {:?} {:?} at x {:.0}–{:.0} (panel to {:.0}), y {:.0}", n.role(), n.label().or(n.value()).unwrap_or_default(), b.x0, b.x1, body.right(), b.y0))
        .collect()
}

fn check(pseudo: bool, state: &str, setup: impl Fn(&mut App)) -> Vec<String> {
    let mut found = Vec::new();
    for p in Panel::ALL {
        let (mut app, ctx) = app(pseudo);
        setup(&mut app);
        found.extend(overflow(&mut app, &ctx, p).into_iter().map(|f| format!("[{}{state}] {f}", if pseudo { "pseudo, " } else { "" })));
    }
    found
}

#[test]
fn every_panel_fits_the_narrowest_dock() {
    let mut found = Vec::new();
    for pseudo in [false, true] {
        found.extend(check(pseudo, "document", |_| {}));
        found.extend(check(pseudo, "shape selected", |a| {
            a.run("layer.select", serde_json::json!({"index": 0}));
            a.run("edit.select_all", serde_json::json!({}));
        }));
        found.extend(check(pseudo, "rectangle tool", |a| a.tool = crate::app::Tool::Rect));
        found.extend(check(pseudo, "text tool", |a| a.tool = crate::app::Tool::Text));
        found.extend(check(pseudo, "brush tool", |a| a.tool = crate::app::Tool::Brush));
    }
    assert!(found.is_empty(), "{} widgets past a panel's edge at {MIN_SIDE} pt:\n{}", found.len(), found.join("\n"));
}

