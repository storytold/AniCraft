//! Colour effects through the command registry (ADR 0056): every style, switching styles,
//! values on a mixed selection, classic and motion tweens, undo, the file round trip and the
//! renders they give.

use super::*;
use anicraft_doc::{ColorEffect, ColorStyle, ColorTransform, Property, Rgba};
use serde_json::json;

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    match s.execute(id, &p) {
        Ok(v) => v,
        Err(e) => panic!("{id} {p}: {e}"),
    }
}

/// A white 20 × 20 square at the origin, selected.
fn square() -> Session {
    let mut s = Session::default();
    run(&mut s, "draw.rect", json!({"x": 0, "y": 0, "w": 20, "h": 20, "fill": "#336699", "stroke": null}));
    run(&mut s, "edit.select_all", json!({}));
    s
}

fn effect(s: &Session) -> ColorEffect {
    s.visible_elements().first().map(|e| e.color).unwrap_or_default()
}

/// The square's colour at the stage centre of its frame (CPU oracle, 1:1), 0–255.
fn pixel(s: &Session, frame: u32) -> [u8; 4] {
    let pm = anicraft_render::render_frame(&s.doc, frame, 1.0).unwrap();
    let px = pm.to_rgba8();
    let i = ((10 * pm.width + 10) * 4) as usize;
    [px[i], px[i + 1], px[i + 2], px[i + 3]]
}

fn near(a: [u8; 4], b: [u8; 4]) -> bool {
    a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= 1)
}

#[test]
fn every_style_is_set_and_reported() {
    let mut s = square();
    let v = run(&mut s, "element.set_color_effect", json!({"brightness": -50}));
    assert_eq!(v, json!({"style": "brightness", "brightness": -50.0}));
    assert_eq!(effect(&s), ColorEffect::Brightness(-0.5));
    let v = run(&mut s, "element.set_color_effect", json!({"tint": "#ff0000", "tint_amount": 25}));
    assert_eq!(v, json!({"style": "tint", "tint": "#ff0000", "tint_amount": 25.0}));
    let v = run(&mut s, "element.set_color_effect", json!({"alpha": 40}));
    assert_eq!(v, json!({"style": "alpha", "alpha": 40.0}));
    run(&mut s, "element.set_color_effect", json!({"red_multiplier": -100, "red_offset": 255, "alpha_offset": -10.5}));
    assert_eq!(effect(&s), ColorEffect::Advanced(ColorTransform { mul: [-1.0, 1.0, 1.0, 0.4], add: [1.0, 0.0, 0.0, -10.5 / 255.0] }), "Advanced starts from the alpha it had");
    let v = run(&mut s, "element.set_color_effect", json!({"style": "none"}));
    assert_eq!(v, json!({"style": "none"}));
    assert_eq!(effect(&s), ColorEffect::None);
}

#[test]
fn a_new_style_keeps_the_look_it_can() {
    let mut s = square();
    run(&mut s, "element.set_color_effect", json!({"tint": "#00ff00", "tint_amount": 60}));
    let before = pixel(&s, 0);
    run(&mut s, "element.set_color_effect", json!({"style": "advanced"}));
    assert_eq!(effect(&s).style(), ColorStyle::Advanced);
    assert!(near(pixel(&s, 0), before), "Advanced is exactly the tint");
    run(&mut s, "element.set_color_effect", json!({"style": "brightness"}));
    assert_eq!(effect(&s), ColorEffect::Brightness(0.0));
    run(&mut s, "element.set_color_effect", json!({"style": "tint"}));
    assert_eq!(effect(&s), ColorEffect::Tint { color: Rgba::WHITE, amount: 0.0 });
}

#[test]
fn bad_params_change_nothing() {
    let mut s = square();
    let before = s.doc.clone();
    for p in [
        json!({}),
        json!({"style": "sepia"}),
        json!({"brightness": 101}),
        json!({"tint_amount": -1}),
        json!({"red_offset": 300}),
        json!({"alpha": 50, "brightness": 10}),
        json!({"style": "tint", "alpha": 50}),
        json!({"tint": "red"}),
        json!({"alpha": f64::NAN}),
    ] {
        assert!(s.execute("element.set_color_effect", &p).is_err(), "{p}");
    }
    assert_eq!(s.doc, before);
    run(&mut s, "edit.deselect", json!({}));
    assert!(s.execute("element.set_color_effect", &json!({"style": "alpha"})).is_err(), "nothing selected");
}

#[test]
fn renders_follow_the_arithmetic() {
    // #336699 = (51, 102, 153).
    let mut s = square();
    run(&mut s, "element.set_color_effect", json!({"brightness": 50}));
    assert!(near(pixel(&s, 0), [153, 179, 204, 255]), "{:?}", pixel(&s, 0));
    run(&mut s, "element.set_color_effect", json!({"tint": "#ff0000", "tint_amount": 50}));
    assert!(near(pixel(&s, 0), [153, 51, 77, 255]), "{:?}", pixel(&s, 0));
    run(&mut s, "element.set_color_effect", json!({"style": "advanced", "red_multiplier": -100, "red_offset": 255, "green_multiplier": 50, "blue_offset": -100}));
    // The tint carried over into Advanced (×50 %, +127.5 red), then these channels changed.
    let fx = effect(&s).transform();
    assert_eq!((fx.mul[0], fx.add[0], fx.mul[1], fx.add[2]), (-1.0, 1.0, 0.5, -100.0 / 255.0));
    assert!(near(pixel(&s, 0), [204, 51, 0, 255]), "{:?}", pixel(&s, 0));
}

#[test]
fn a_change_on_a_mixed_selection_sets_only_that_value() {
    let mut s = Session::default();
    let a = run(&mut s, "draw.rect", json!({"x": 0, "y": 0, "w": 10, "h": 10, "object": true}))["element"].as_u64().unwrap();
    let b = run(&mut s, "draw.rect", json!({"x": 20, "y": 0, "w": 10, "h": 10, "object": true}))["element"].as_u64().unwrap();
    run(&mut s, "element.select", json!({"ids": [a]}));
    run(&mut s, "element.set_color_effect", json!({"tint": "#ff0000", "tint_amount": 50}));
    run(&mut s, "element.select", json!({"ids": [b]}));
    run(&mut s, "element.set_color_effect", json!({"tint": "#0000ff", "tint_amount": 10}));
    run(&mut s, "element.select", json!({"ids": [a, b]}));
    run(&mut s, "element.set_color_effect", json!({"tint_amount": 80}));
    let fx: Vec<ColorEffect> = s.visible_elements().iter().map(|e| e.color).collect();
    assert_eq!(fx, [ColorEffect::Tint { color: Rgba::hex(0xff0000), amount: 0.8 }, ColorEffect::Tint { color: Rgba::hex(0x0000ff), amount: 0.8 }], "each keeps its colour");
    // One undo step puts both back.
    s.undo().unwrap();
    assert_eq!(s.visible_elements()[1].color, ColorEffect::Tint { color: Rgba::hex(0x0000ff), amount: 0.1 });
}

#[test]
fn set_alpha_keeps_the_rest_of_the_effect() {
    let mut s = square();
    run(&mut s, "element.set_color_effect", json!({"brightness": 20}));
    run(&mut s, "element.set_alpha", json!({"alpha": 0.5}));
    assert_eq!(effect(&s).transform(), ColorTransform::alpha(0.5).concat(&ColorTransform::brightness(0.2)));
    run(&mut s, "element.set_color_effect", json!({"style": "none"}));
    run(&mut s, "element.set_alpha", json!({"alpha": 0.5}));
    assert_eq!(effect(&s), ColorEffect::Alpha(0.5));
}

#[test]
fn a_classic_tween_between_styles_travels_in_the_advanced_space() {
    let mut s = square();
    run(&mut s, "element.set_color_effect", json!({"tint": "#ff0000", "tint_amount": 100}));
    run(&mut s, "timeline.goto", json!({"frame": 10}));
    run(&mut s, "timeline.insert_keyframe", json!({}));
    run(&mut s, "edit.select_all", json!({}));
    run(&mut s, "element.set_color_effect", json!({"alpha": 50}));
    run(&mut s, "timeline.goto", json!({"frame": 0}));
    run(&mut s, "timeline.create_tween", json!({"easing": "Linear"}));
    s.set_frame(5);
    let mid = effect(&s);
    assert_eq!(mid.style(), ColorStyle::Advanced);
    let want = ColorTransform::tint(Rgba::hex(0xff0000), 1.0).lerp(&ColorTransform::alpha(0.5), 0.5);
    let got = mid.transform();
    assert!(got.mul.iter().zip(want.mul).chain(got.add.iter().zip(want.add)).all(|(a, b)| (a - b).abs() < 1e-5), "{got:?} vs {want:?}");
    // Within one style the value itself travels: brightness −100 % to +100 % is 0 halfway.
    let mut s = square();
    run(&mut s, "element.set_color_effect", json!({"brightness": -100}));
    run(&mut s, "timeline.goto", json!({"frame": 10}));
    run(&mut s, "timeline.insert_keyframe", json!({}));
    run(&mut s, "edit.select_all", json!({}));
    run(&mut s, "element.set_color_effect", json!({"brightness": 100}));
    run(&mut s, "timeline.goto", json!({"frame": 0}));
    run(&mut s, "timeline.create_tween", json!({"easing": "Linear"}));
    s.set_frame(5);
    assert_eq!(effect(&s), ColorEffect::Brightness(0.0));
    assert!(near(pixel(&s, 5), [51, 102, 153, 255]));
}

#[test]
fn motion_tweens_key_brightness_tint_and_advanced() {
    let mut s = square();
    run(&mut s, "timeline.create_motion_tween", json!({}));
    run(&mut s, "motion.set_key", json!({"property": "brightness", "frame": 10, "value": 60}));
    s.set_frame(5);
    assert_eq!(effect(&s), ColorEffect::Brightness(0.3));
    // A second style composes with the first into Advanced, exactly.
    run(&mut s, "motion.set_key", json!({"property": "tint", "frame": 10, "value": 50}));
    run(&mut s, "motion.set_key", json!({"property": "tint_red", "frame": 0, "value": 255}));
    run(&mut s, "motion.set_key", json!({"property": "tint_green", "frame": 0, "value": 0}));
    run(&mut s, "motion.set_key", json!({"property": "tint_blue", "frame": 0, "value": 0}));
    s.set_frame(10);
    let want = ColorTransform::brightness(0.6).concat(&ColorTransform::tint(Rgba::hex(0xff0000), 0.5));
    assert_eq!(effect(&s).transform(), want);
    // Advanced channels on their own curves.
    let mut s = square();
    run(&mut s, "timeline.create_motion_tween", json!({}));
    run(&mut s, "motion.set_key", json!({"property": "red_offset", "frame": 10, "value": 255}));
    run(&mut s, "motion.set_key", json!({"property": "alpha_multiplier", "frame": 10, "value": 0}));
    s.set_frame(5);
    let fx = effect(&s);
    assert_eq!(fx.style(), ColorStyle::Advanced);
    assert_eq!(fx.transform(), ColorTransform { mul: [1.0, 1.0, 1.0, 0.5], add: [0.5, 0.0, 0.0, 0.0] });
    assert!(s.execute("motion.set_key", &json!({"property": "red_offset", "frame": 3, "value": 256})).is_err());
    // Info lists the keys by their names.
    let info = run(&mut s, "motion.info", json!({}));
    assert!(info.to_string().contains("red_offset"), "{info}");
}

#[test]
fn a_motion_tween_has_one_colour_effect_style() {
    let mut s = square();
    run(&mut s, "element.set_color_effect", json!({"tint": "#00ff00", "tint_amount": 40}));
    run(&mut s, "timeline.create_motion_tween", json!({}));
    // Untracked, the target's own tint shows all along.
    s.set_frame(5);
    assert_eq!(effect(&s), ColorEffect::Tint { color: Rgba::hex(0x00ff00), amount: 0.4 });
    // Keying alpha makes it an alpha tween, as in Animate.
    run(&mut s, "motion.set_key", json!({"property": "alpha", "frame": 10, "value": 0}));
    s.set_frame(5);
    assert_eq!(effect(&s), ColorEffect::Alpha(0.5));
    // Changing the alpha in Properties inside the span keys it there.
    s.set_frame(4);
    run(&mut s, "edit.select_all", json!({}));
    run(&mut s, "element.set_color_effect", json!({"alpha": 90}));
    s.set_frame(4);
    assert!((effect(&s).alpha() - 0.9).abs() < 1e-4, "{:?}", effect(&s));
    assert!(layer_motion_has(&s, Property::Alpha) && !layer_motion_has(&s, Property::Tint));
    // Keying tint as well composes the two (older files keyed both); the tint colour is the
    // target's own.
    run(&mut s, "motion.set_key", json!({"property": "tint", "frame": 10, "value": 50}));
    s.set_frame(10);
    assert_eq!(effect(&s).transform(), ColorTransform::alpha(0.0).concat(&ColorTransform::tint(Rgba::hex(0x00ff00), 0.5)), "the tint keys start from the target's own colour");
}

fn layer_motion_has(s: &Session, p: Property) -> bool {
    s.active_layer().and_then(|l| l.keyframes.first()).and_then(|k| k.motion.as_ref()).is_some_and(|m| m.track(p).is_some())
}

#[test]
fn files_keep_every_style() {
    let mut s = Session::default();
    for (i, p) in [json!({"brightness": -30}), json!({"tint": "#123456", "tint_amount": 0}), json!({"style": "advanced", "blue_multiplier": -40, "green_offset": 12}), json!({"alpha": 100})].into_iter().enumerate() {
        let id = run(&mut s, "draw.rect", json!({"x": i * 20, "y": 0, "w": 10, "h": 10, "object": true}))["element"].as_u64().unwrap();
        run(&mut s, "element.select", json!({"ids": [id]}));
        run(&mut s, "element.set_color_effect", p);
    }
    let text = crate::format::to_json(&s.doc).unwrap();
    let back = crate::format::from_json(&text).unwrap();
    assert_eq!(back, s.doc);
    let styles: Vec<ColorStyle> = back.timeline(0).layers[0].keyframes[0].elements.iter().map(|e| e.color.style()).collect();
    assert_eq!(styles, [ColorStyle::Brightness, ColorStyle::Tint, ColorStyle::Advanced, ColorStyle::Alpha]);
}

#[test]
fn plug_ins_may_set_colour_effects() {
    assert!(crate::cmds_plugin::plugin_may_call("element.set_color_effect").is_ok());
}

#[test]
fn a_nine_slice_instance_takes_its_colour_effect() {
    // ADR 0055 with ADR 0056: the slices are geometry; the colour effect applies to all of them.
    let mut s = square();
    run(&mut s, "modify.convert_to_symbol", json!({"name": "Panel"}));
    run(&mut s, "symbol.set_scale_grid", json!({"on": true, "left": 5, "right": 15, "top": 5, "bottom": 15}));
    run(&mut s, "element.set_transform", json!({"sx": 3.0, "sy": 3.0}));
    run(&mut s, "element.set_color_effect", json!({"tint": "#ff0000", "tint_amount": 50}));
    let dl = anicraft_doc::evaluate(&s.doc, 0);
    assert!(!dl.items.is_empty());
    let want = ColorTransform::tint(Rgba::hex(0xff0000), 0.5);
    assert!(dl.items.iter().all(|i| i.color == want), "every slice is tinted");
}
