//! The camera layer (ADR 0027, 0082): a special layer at the top of a scene's timeline whose
//! camera views the stage. Its keyframes hold one [`ElementKind::Camera`] element, so the camera
//! tweens like any instance (classic tweens and motion tweens): the element's transform is the
//! camera on the stage, `T(x, y) · R(rotation) · S(zoom)`, with the camera's centre at `(x, y)`,
//! and its colour effect (None, Brightness, Tint or Advanced) colours the whole view.
//!
//! What the movie shows is the stage seen through the camera: every layer is drawn through
//! [`Camera::view`], which puts the camera's centre at the middle of the output, zoomed and
//! turned the other way. A layer's **depth** (Animate's Layer Depth, keyable over time,
//! [`Layer::depth_at`]) places it in a pinhole perspective with a fixed focal length
//! ([`FOCAL`]): the camera at rest shows every layer as drawn, and its pans and zooms move and
//! scale a deeper layer less (ADR 0082). A layer at depth 0 is the stage plane, exactly as
//! before; a layer **attached** to the camera ignores it (titles, HUDs). Layers are drawn
//! deepest first, ties in timeline order ([`Timeline::draw_order`]). Everything here is pure:
//! the camera at a frame is a function of the document.

use crate::{ColorEffect, ColorTransform, Element, ElementId, ElementKind, Layer, LayerKind, Rgba, Stage, Timeline, evaluate_layer_elements};
use anicraft_geom::{Affine, Decomposed, Vec2};
use serde::{Deserialize, Serialize};

/// Smallest and largest camera zoom (5 % to 2,000 %).
pub const MIN_ZOOM: f32 = 0.05;
pub const MAX_ZOOM: f32 = 20.0;
/// Layer depth range, in pixels behind (positive) or in front of (negative) the stage.
pub const MIN_DEPTH: f32 = -500.0;
pub const MAX_DEPTH: f32 = 10_000.0;
/// Distance from the camera to the stage plane at zoom 100 %, in pixels (the pinhole's focal
/// length): a layer this far behind the stage moves half as much as the stage when the camera
/// pans. Zooming moves the camera towards the stage: at zoom `z` it is `FOCAL / z` away.
pub const FOCAL: f32 = 1_000.0;
/// Nearest a layer may be to the camera and still show (pixels): a layer the camera has
/// reached or passed (a near layer under a strong zoom) isn't drawn.
pub const NEAR: f32 = FOCAL / 100.0;
/// Most depth keys one layer keeps.
pub const MAX_DEPTH_KEYS: usize = 4_096;

/// A layer's depth at one frame (ADR 0082): between two keys the depth travels linearly; before
/// the first and after the last it holds.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct DepthKey {
    pub frame: u32,
    pub depth: f32,
}

impl PartialEq for DepthKey {
    fn eq(&self, o: &DepthKey) -> bool {
        self.frame == o.frame && self.depth.to_bits() == o.depth.to_bits()
    }
}

/// The camera at one frame, resolved and sanitized.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    /// The stage point at the middle of the view.
    pub center: Vec2,
    /// `1.0` shows the stage at its size; `2.0` doubles sizes about the centre.
    pub zoom: f32,
    /// Radians, clockwise on the stage (the view turns the other way).
    pub rotation: f32,
    /// The colour effect on everything the camera shows: None, Brightness, Tint or Advanced
    /// (the camera has no opacity: Alpha reads as None and Advanced keeps alpha as it is).
    pub color: ColorEffect,
}

impl Camera {
    /// The camera at rest on `stage`: its centre at the stage's, zoom 100 %, no turn, no colour
    /// effect. The view through it is the stage itself.
    pub fn rest(stage: &Stage) -> Camera {
        Camera { center: stage_center(stage), zoom: 1.0, rotation: 0.0, color: ColorEffect::Tint { color: Rgba::WHITE, amount: 0.0 } }
    }

    /// The camera a camera element describes (any element: its transform and colour effect).
    /// Scale X is the zoom; Scale Y, skew and opacity play no part. Non-finite values fall
    /// back to the rest camera's.
    pub fn of_element(el: &Element, stage: &Stage) -> Camera {
        let rest = Camera::rest(stage);
        let d = el.transform.decompose();
        let fin = |v: f32, or: f32| if v.is_finite() { v } else { or };
        let center = Vec2::new(fin(d.tx, rest.center.x), fin(d.ty, rest.center.y));
        let zoom = fin(d.sx.abs(), 1.0).clamp(MIN_ZOOM, MAX_ZOOM);
        let rotation = fin(d.rotation, 0.0);
        Camera { center, zoom, rotation, color: camera_effect(&el.color) }
    }

    /// The camera element holding this camera (for commands).
    pub fn to_element(&self, id: ElementId) -> Element {
        Element { id, name: String::new(), transform: self.transform(), color: camera_effect(&self.color), kind: ElementKind::Camera }
    }

    /// The camera on the stage: `T(centre) · R(rotation) · S(zoom)`.
    pub fn transform(&self) -> Affine {
        Decomposed { tx: self.center.x, ty: self.center.y, sx: self.zoom, sy: self.zoom, rotation: self.rotation, skew: 0.0 }.compose()
    }

    /// The colour effect the camera puts on everything it shows.
    pub fn color(&self) -> ColorTransform {
        camera_effect(&self.color).transform()
    }

    /// Stage space → output space for the stage plane (a layer at depth 0): the camera's centre
    /// lands at the middle of the output, zoomed, and turned against the camera's rotation.
    pub fn stage_view(&self, stage: &Stage) -> Affine {
        let mid = stage_center(stage);
        let c = self.center;
        let m = Affine::translate(mid.x, mid.y).mul(&Affine::scale(self.zoom, self.zoom)).mul(&Affine::rotate(-self.rotation)).mul(&Affine::translate(-c.x, -c.y));
        if m.is_finite() { m } else { Affine::IDENTITY }
    }

    /// Stage space → output space for a layer `depth` pixels behind the stage, by a pinhole
    /// camera `FOCAL / zoom` from the stage plane (ADR 0082). The layer is drawn as it would be
    /// seen if it stood that far back, enlarged so that the camera at rest shows it exactly as
    /// drawn: for a stage point `p`, with `d = FOCAL / zoom + depth`,
    /// `out = mid + R(−rotation) · ((FOCAL + depth) / d · (p − mid) − FOCAL / d · (centre − mid))`.
    /// Depth 0 is [`Camera::stage_view`] exactly. `None` when the layer is at or behind the
    /// camera (nearer than [`NEAR`]): it isn't drawn.
    pub fn view(&self, stage: &Stage, depth: f32) -> Option<Affine> {
        let depth = clamp_depth(depth);
        if depth == 0.0 {
            return Some(self.stage_view(stage));
        }
        let dist = FOCAL / self.zoom + depth;
        if dist.is_nan() || dist < NEAR {
            return None;
        }
        let mid = stage_center(stage);
        let size = (FOCAL + depth) / dist;
        let pan = (self.center - mid) * (FOCAL / dist);
        let m = Affine::translate(mid.x, mid.y).mul(&Affine::rotate(-self.rotation)).mul(&Affine::translate(-pan.x, -pan.y)).mul(&Affine::scale(size, size)).mul(&Affine::translate(-mid.x, -mid.y));
        Some(if m.is_finite() { m } else { Affine::IDENTITY })
    }

    /// How `layer` is placed at `frame` through this camera: by its depth then, or not at all
    /// when attached (it stays on screen as drawn). `None`: the camera has passed it.
    pub fn layer_view(&self, stage: &Stage, layer: &Layer, frame: u32) -> Option<Affine> {
        if layer.attached { Some(Affine::IDENTITY) } else { self.view(stage, layer.depth_at(frame)) }
    }

    /// The camera's frame on the stage (what it sees), corners clockwise from the top left.
    pub fn frame(&self, stage: &Stage) -> [Vec2; 4] {
        let (w, h) = (stage.width as f32, stage.height as f32);
        let inv = self.stage_view(stage).inverse().unwrap_or(Affine::IDENTITY);
        [Vec2::new(0.0, 0.0), Vec2::new(w, 0.0), Vec2::new(w, h), Vec2::new(0.0, h)].map(|p| inv.apply(p))
    }

    /// The stage point under output point `p` (layers at depth 0).
    pub fn to_stage(&self, stage: &Stage, p: Vec2) -> Vec2 {
        self.stage_view(stage).inverse().map_or(p, |m| m.apply(p))
    }

    /// Whether the view is the stage as it is (nothing to apply).
    pub fn is_rest(&self, stage: &Stage) -> bool {
        let still = (self.center, self.zoom, self.rotation) == (stage_center(stage), 1.0, 0.0) || self.stage_view(stage) == Affine::IDENTITY;
        still && self.color().is_identity()
    }
}

/// The colour effect a camera can have: Alpha reads as None, and Advanced keeps alpha as it is
/// (the movie never fades out through its camera).
pub fn camera_effect(fx: &ColorEffect) -> ColorEffect {
    match fx.sanitized() {
        ColorEffect::Alpha(_) => ColorEffect::None,
        ColorEffect::Advanced(mut t) => {
            t.mul[3] = 1.0;
            t.add[3] = 0.0;
            ColorEffect::Advanced(t)
        }
        other => other,
    }
}

/// The middle of the stage.
pub fn stage_center(stage: &Stage) -> Vec2 {
    Vec2::new(stage.width as f32 * 0.5, stage.height as f32 * 0.5)
}

/// How far a layer `depth` pixels behind the stage moves when the camera pans at zoom 100 %,
/// as a fraction of the stage's movement (1 at depth 0).
pub fn parallax(depth: f32) -> f32 {
    FOCAL / (FOCAL + clamp_depth(depth))
}

/// A layer depth kept in range (non-finite: 0).
pub fn clamp_depth(depth: f32) -> f32 {
    if depth.is_finite() { depth.clamp(MIN_DEPTH, MAX_DEPTH) } else { 0.0 }
}

impl Layer {
    /// The layer's depth at `frame` (ADR 0082): its keys' when it has any (linear between two,
    /// held outside them), else its one depth. Always in range.
    pub fn depth_at(&self, frame: u32) -> f32 {
        let keys = &self.depth_keys;
        let Some(first) = keys.first() else { return clamp_depth(self.depth) };
        // The first key after `frame` (keys are sorted by frame).
        let next = keys.partition_point(|k| k.frame <= frame);
        let d = match (next.checked_sub(1).and_then(|i| keys.get(i)), keys.get(next)) {
            (Some(a), Some(b)) if b.frame > a.frame => {
                let t = (frame - a.frame) as f32 / (b.frame - a.frame) as f32;
                a.depth + (b.depth - a.depth) * t
            }
            (Some(a), _) => a.depth,
            (None, _) => first.depth,
        };
        clamp_depth(d)
    }

    /// Whether the layer has a depth anywhere (a key or its one depth off the stage plane).
    pub fn has_depth(&self) -> bool {
        self.depth != 0.0 || self.depth_keys.iter().any(|k| k.depth != 0.0)
    }

    /// Whether the depth keys are sorted, one per frame, in range and not too many.
    pub fn depth_keys_clean(&self) -> bool {
        let keys = &self.depth_keys;
        keys.len() <= MAX_DEPTH_KEYS && keys.windows(2).all(|w| w[0].frame < w[1].frame) && keys.iter().all(|k| clamp_depth(k.depth).to_bits() == k.depth.to_bits())
    }

    /// Keeps the depth keys sorted, one per frame, finite and in range, and at most
    /// [`MAX_DEPTH_KEYS`]. Returns whether anything changed.
    pub fn sanitize_depth_keys(&mut self) -> bool {
        if self.depth_keys_clean() {
            return false;
        }
        let keys = &mut self.depth_keys;
        keys.truncate(MAX_DEPTH_KEYS);
        for k in keys.iter_mut() {
            k.depth = clamp_depth(k.depth);
        }
        // The last key given for a frame wins.
        keys.reverse();
        keys.sort_by_key(|k| k.frame);
        keys.dedup_by_key(|k| k.frame);
        true
    }

    /// Sets the depth at `frame` as a key (replacing one there), keeping the keys sorted.
    pub fn set_depth_key(&mut self, frame: u32, depth: f32) {
        let depth = clamp_depth(depth);
        match self.depth_keys.binary_search_by_key(&frame, |k| k.frame) {
            Ok(i) => {
                if let Some(k) = self.depth_keys.get_mut(i) {
                    k.depth = depth;
                }
            }
            Err(i) => self.depth_keys.insert(i, DepthKey { frame, depth }),
        }
    }
}

impl Timeline {
    /// The camera layer, if the timeline has one (it is always the top layer).
    pub fn camera_layer(&self) -> Option<&Layer> {
        self.layers.first().map(|l| &**l).filter(|l| l.kind == LayerKind::Camera)
    }

    /// Keeps the camera invariant: at most one camera layer, the top one, inside nothing.
    /// Extra camera layers become normal layers (their camera elements draw nothing), so no
    /// keyframe is lost. Touches only layers that change.
    pub fn repair_camera(&mut self) {
        let Some(first) = self.layers.iter().position(|l| l.kind == LayerKind::Camera) else { return };
        if first != 0 {
            let cam = self.layers.remove(first);
            self.layers.insert(0, cam);
        }
        if let Some(cam) = self.layers.first_mut()
            && cam.parent.is_some()
        {
            std::sync::Arc::make_mut(cam).parent = None;
        }
        for l in self.layers.iter_mut().skip(1).filter(|l| l.kind == LayerKind::Camera) {
            std::sync::Arc::make_mut(l).kind = LayerKind::Normal;
        }
    }

    /// The order the layers are drawn in at `frame`, bottom first, as indices into `layers`
    /// (ADR 0082): the deepest first, ties in timeline order (lower layers first). A masked
    /// layer goes with its mask's depth, so a mask stays with the layers it masks. With every
    /// layer on the stage plane this is the timeline order, bottom up.
    pub fn draw_order(&self, frame: u32) -> Vec<usize> {
        let n = self.layers.len();
        let mut order: Vec<usize> = (0..n).rev().collect();
        if !self.layers.iter().any(|l| l.has_depth()) {
            return order;
        }
        let depth = |l: &Layer| -> f32 {
            let mask = l.parent.and_then(|p| self.layer(p)).filter(|p| p.kind == LayerKind::Mask);
            mask.unwrap_or(l).depth_at(frame)
        };
        let depths: Vec<f32> = self.layers.iter().map(|l| depth(l)).collect();
        let at = |i: usize| depths.get(i).copied().unwrap_or(0.0);
        // Stable: equal depths keep the bottom-up order. Depths are finite (clamped).
        order.sort_by(|&a, &b| at(b).total_cmp(&at(a)));
        order
    }

    /// The camera at `frame`, when the timeline has a visible camera layer with a camera on that
    /// frame. A hidden camera layer is a camera turned off; a keyframe with no camera element is
    /// the camera at rest.
    pub fn camera_at(&self, frame: u32, stage: &Stage) -> Option<Camera> {
        self.camera_layer().filter(|l| l.visible)?;
        self.camera_state_at(frame, stage).filter(|c| !c.is_rest(stage))
    }

    /// The camera as its layer has it at `frame`, shown or hidden: the rest camera on a frame
    /// whose keyframe holds none. `None` without a camera layer or past its last frame.
    pub fn camera_state_at(&self, frame: u32, stage: &Stage) -> Option<Camera> {
        let (_, els) = evaluate_layer_elements(self.camera_layer()?, frame)?;
        Some(els.iter().find(|e| matches!(e.kind, ElementKind::Camera)).map_or_else(|| Camera::rest(stage), |e| Camera::of_element(e, stage)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stage() -> Stage {
        Stage { width: 200, height: 100, ..Stage::default() }
    }

    fn close(a: Vec2, b: Vec2) -> bool {
        a.distance(b) < 1e-3
    }

    #[test]
    fn the_rest_camera_shows_the_stage_as_it_is() {
        let c = Camera::rest(&stage());
        assert_eq!(c.stage_view(&stage()), Affine::IDENTITY);
        assert_eq!(c.view(&stage(), 5000.0), Some(Affine::IDENTITY), "depth only matters when the camera moves");
        assert_eq!(c.view(&stage(), -400.0), Some(Affine::IDENTITY));
        assert!(c.is_rest(&stage()));
        let f = c.frame(&stage());
        assert!(close(f[2], Vec2::new(200.0, 100.0)));
    }

    #[test]
    fn zoom_doubles_sizes_about_the_camera_centre() {
        let c = Camera { center: Vec2::new(60.0, 40.0), zoom: 2.0, ..Camera::rest(&stage()) };
        let v = c.stage_view(&stage());
        assert!(close(v.apply(Vec2::new(60.0, 40.0)), Vec2::new(100.0, 50.0)), "the centre lands mid-output");
        assert!(close(v.apply(Vec2::new(70.0, 40.0)), Vec2::new(120.0, 50.0)), "10 px become 20");
        // The frame sees half the stage's width.
        let f = c.frame(&stage());
        assert!((f[1].x - f[0].x - 100.0).abs() < 1e-3);
    }

    #[test]
    fn rotation_turns_the_view_the_other_way() {
        let c = Camera { rotation: std::f32::consts::FRAC_PI_2, ..Camera::rest(&stage()) };
        let v = c.stage_view(&stage());
        // A point right of the centre ends up above it (the world turns counter-clockwise).
        assert!(close(v.apply(Vec2::new(110.0, 50.0)), Vec2::new(100.0, 40.0)), "{:?}", v.apply(Vec2::new(110.0, 50.0)));
        // Round trip through the element.
        let back = Camera::of_element(&c.to_element(ElementId(1)), &stage());
        assert!((back.rotation - c.rotation).abs() < 1e-5 && (back.zoom - 1.0).abs() < 1e-5);
    }

    #[test]
    fn depth_zero_is_the_stage_view_bit_for_bit() {
        let c = Camera { center: Vec2::new(131.7, 12.3), zoom: 1.37, rotation: 0.41, ..Camera::rest(&stage()) };
        assert_eq!(c.view(&stage(), 0.0), Some(c.stage_view(&stage())));
        assert_eq!(c.view(&stage(), -0.0), Some(c.stage_view(&stage())));
        assert_eq!(c.view(&stage(), f32::NAN), Some(c.stage_view(&stage())), "a NaN depth is the stage plane");
    }

    #[test]
    fn pans_move_a_layer_by_the_pinhole_ratio() {
        // Analytic: a pan of 40 px at zoom 1 moves a layer at depth d by 40 · F / (F + d).
        let c = Camera { center: Vec2::new(140.0, 50.0), ..Camera::rest(&stage()) };
        for d in [-500.0_f32, -250.0, 0.0, 250.0, 1000.0, 3000.0, 10_000.0] {
            let v = c.view(&stage(), d).unwrap_or(Affine::IDENTITY);
            let moved = 100.0 - v.apply(Vec2::new(100.0, 50.0)).x;
            let want = 40.0 * FOCAL / (FOCAL + d);
            assert!((moved - want).abs() < 1e-3, "depth {d}: moved {moved}, want {want}");
            assert!((v.mean_scale() - 1.0).abs() < 1e-5, "a pan keeps sizes at every depth");
        }
        assert!((parallax(FOCAL) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn zooms_scale_a_deeper_layer_less() {
        // Zoom 2: the camera is F / 2 from the stage. A layer F back is 1.5 F away: it grows by
        // 2F / 1.5F = 4/3, the stage plane by 2.
        let c = Camera { zoom: 2.0, ..Camera::rest(&stage()) };
        let s = |d: f32| c.view(&stage(), d).map_or(0.0, |v| v.mean_scale());
        assert!((s(0.0) - 2.0).abs() < 1e-5);
        assert!((s(FOCAL) - 4.0 / 3.0).abs() < 1e-4, "{}", s(FOCAL));
        assert!(s(MAX_DEPTH) > 1.0 && s(MAX_DEPTH) < s(FOCAL));
        assert!(s(-250.0) > 2.0, "a nearer layer grows more");
        // The centre stays put at every depth (the zoom is about the camera's axis).
        let v = c.view(&stage(), 600.0).unwrap_or(Affine::IDENTITY);
        assert!(close(v.apply(Vec2::new(100.0, 50.0)), Vec2::new(100.0, 50.0)));
        // A layer the camera has passed isn't drawn: zoom 4 puts the camera 250 px from the
        // stage, past a layer 400 px in front.
        let past = Camera { zoom: 4.0, ..Camera::rest(&stage()) };
        assert_eq!(past.view(&stage(), -400.0), None);
        assert!(past.view(&stage(), -200.0).is_some());
    }

    #[test]
    fn depth_keys_travel_linearly_and_hold_outside() {
        let mut l = Layer::new(crate::LayerId(1), "x");
        l.depth = 50.0;
        assert_eq!(l.depth_at(7), 50.0, "no keys: the one depth");
        l.set_depth_key(10, 100.0);
        l.set_depth_key(20, 300.0);
        assert_eq!(l.depth_at(0), 100.0);
        assert_eq!(l.depth_at(15), 200.0, "the midpoint");
        assert_eq!(l.depth_at(20), 300.0);
        assert_eq!(l.depth_at(u32::MAX), 300.0);
        l.set_depth_key(20, 1e30);
        assert_eq!(l.depth_at(20), MAX_DEPTH);
        // Hostile keys: unsorted, repeated, NaN, too many.
        l.depth_keys = vec![DepthKey { frame: 5, depth: f32::NAN }, DepthKey { frame: 1, depth: -1e9 }, DepthKey { frame: 5, depth: 7.0 }];
        assert!(l.sanitize_depth_keys());
        assert_eq!(l.depth_keys, vec![DepthKey { frame: 1, depth: MIN_DEPTH }, DepthKey { frame: 5, depth: 7.0 }]);
        assert!(!l.sanitize_depth_keys(), "already clean");
    }

    #[test]
    fn camera_colour_effects_have_no_opacity() {
        let mut c = Camera::rest(&stage());
        c.color = ColorEffect::Brightness(0.5);
        assert_eq!(c.color(), ColorTransform::brightness(0.5));
        assert!(!c.is_rest(&stage()));
        c.color = ColorEffect::Alpha(0.2);
        assert!(c.color().is_identity() && c.is_rest(&stage()), "a camera never fades");
        let mut t = ColorTransform::IDENTITY;
        t.mul = [0.5, 1.0, 1.0, 0.1];
        t.add = [0.2, 0.0, 0.0, -0.5];
        c.color = ColorEffect::Advanced(t);
        let got = c.color();
        assert_eq!((got.mul[0], got.add[0], got.mul[3], got.add[3]), (0.5, 0.2, 1.0, 0.0));
        let back = Camera::of_element(&c.to_element(ElementId(1)), &stage());
        assert_eq!(back.color(), got);
    }

    #[test]
    fn hostile_cameras_stay_finite() {
        let mut el = Camera::rest(&stage()).to_element(ElementId(1));
        el.transform = Affine { m: [f32::NAN, 0.0, 0.0, f32::INFINITY, f32::NAN, 1e30] };
        el.color = ColorEffect::Tint { color: Rgba::WHITE, amount: f32::NAN };
        let c = Camera::of_element(&el, &stage());
        assert!(c.stage_view(&stage()).is_finite());
        for d in [f32::NAN, f32::INFINITY, -1e30, 1e30, MIN_DEPTH, MAX_DEPTH] {
            assert!(c.view(&stage(), d).is_none_or(|v| v.is_finite()));
        }
        assert!((MIN_ZOOM..=MAX_ZOOM).contains(&c.zoom));
        el.transform = Affine::scale(0.0, 0.0);
        assert_eq!(Camera::of_element(&el, &stage()).zoom, MIN_ZOOM);
    }
}
