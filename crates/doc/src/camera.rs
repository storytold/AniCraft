//! The camera layer (ADR 0027): a special layer at the top of a scene's timeline whose camera
//! views the stage. Its keyframes hold one [`ElementKind::Camera`] element, so the camera
//! tweens like any instance (classic tweens and motion tweens): the element's transform is the
//! camera on the stage, `T(x, y) · R(rotation) · S(zoom)`, with the camera's centre at `(x, y)`,
//! and its colour effect's tint colours the whole view.
//!
//! What the movie shows is the stage seen through the camera: every layer is drawn through
//! [`Camera::view`], which puts the camera's centre at the middle of the output, zoomed and
//! turned the other way. A layer's **depth** (Animate's Layer Depth) gives parallax: a layer
//! `depth` pixels behind the stage moves `FOCAL / (FOCAL + depth)` as far when the camera pans,
//! keeping its size (so a layer at depth 0 moves with the stage, a far background barely moves).
//! Everything here is pure: the camera at a frame is a function of the document.

use crate::{ColorEffect, ColorTransform, Element, ElementId, ElementKind, Layer, LayerKind, Rgba, Stage, Timeline, evaluate_layer_elements};
use anicraft_geom::{Affine, Decomposed, Vec2};

/// Smallest and largest camera zoom (5 % to 2,000 %).
pub const MIN_ZOOM: f32 = 0.05;
pub const MAX_ZOOM: f32 = 20.0;
/// Layer depth range, in pixels behind (positive) or in front of (negative) the stage.
pub const MIN_DEPTH: f32 = -500.0;
pub const MAX_DEPTH: f32 = 10_000.0;
/// Distance from the camera to the stage plane, in pixels: a layer this far behind the stage
/// moves half as much as the stage when the camera pans.
pub const FOCAL: f32 = 1_000.0;

/// The camera at one frame, resolved and sanitized.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    /// The stage point at the middle of the view.
    pub center: Vec2,
    /// `1.0` shows the stage at its size; `2.0` doubles sizes about the centre.
    pub zoom: f32,
    /// Radians, clockwise on the stage (the view turns the other way).
    pub rotation: f32,
    /// The tint colouring the whole view (`tint_amount` 0: none).
    pub tint: Rgba,
    pub tint_amount: f32,
}

impl Camera {
    /// The camera at rest on `stage`: its centre at the stage's, zoom 100 %, no turn, no tint.
    /// The view through it is the stage itself.
    pub fn rest(stage: &Stage) -> Camera {
        Camera { center: stage_center(stage), zoom: 1.0, rotation: 0.0, tint: Rgba::WHITE, tint_amount: 0.0 }
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
        // The camera's colour effect is a tint (ADR 0027); other styles leave the view as it is.
        let (tint, amount) = match el.color {
            ColorEffect::Tint { color, amount } => (color.sanitized(), fin(amount, 0.0).clamp(0.0, 1.0)),
            _ => (Rgba::WHITE, 0.0),
        };
        Camera { center, zoom, rotation, tint, tint_amount: amount }
    }

    /// The camera element holding this camera (for commands).
    pub fn to_element(&self, id: ElementId) -> Element {
        Element { id, name: String::new(), transform: self.transform(), color: ColorEffect::Tint { color: self.tint, amount: self.tint_amount }, kind: ElementKind::Camera }
    }

    /// The camera on the stage: `T(centre) · R(rotation) · S(zoom)`.
    pub fn transform(&self) -> Affine {
        Decomposed { tx: self.center.x, ty: self.center.y, sx: self.zoom, sy: self.zoom, rotation: self.rotation, skew: 0.0 }.compose()
    }

    /// The colour effect the camera puts on everything it shows.
    pub fn color(&self) -> ColorTransform {
        ColorTransform::tint(self.tint, self.tint_amount)
    }

    /// Stage space → output space for a layer `depth` pixels behind the stage: the camera's
    /// centre (moved by the layer's parallax) lands at the middle of the output, zoomed, and
    /// turned against the camera's rotation.
    pub fn view(&self, stage: &Stage, depth: f32) -> Affine {
        let mid = stage_center(stage);
        let c = mid + (self.center - mid) * parallax(depth);
        let m = Affine::translate(mid.x, mid.y).mul(&Affine::scale(self.zoom, self.zoom)).mul(&Affine::rotate(-self.rotation)).mul(&Affine::translate(-c.x, -c.y));
        if m.is_finite() { m } else { Affine::IDENTITY }
    }

    /// The camera's frame on the stage (what it sees), corners clockwise from the top left.
    pub fn frame(&self, stage: &Stage) -> [Vec2; 4] {
        let (w, h) = (stage.width as f32, stage.height as f32);
        let inv = self.view(stage, 0.0).inverse().unwrap_or(Affine::IDENTITY);
        [Vec2::new(0.0, 0.0), Vec2::new(w, 0.0), Vec2::new(w, h), Vec2::new(0.0, h)].map(|p| inv.apply(p))
    }

    /// The stage point under output point `p` (layers at depth 0).
    pub fn to_stage(&self, stage: &Stage, p: Vec2) -> Vec2 {
        self.view(stage, 0.0).inverse().map_or(p, |m| m.apply(p))
    }

    /// Whether the view is the stage as it is (nothing to apply).
    pub fn is_rest(&self, stage: &Stage) -> bool {
        *self == Camera::rest(stage) || (self.view(stage, 0.0) == Affine::IDENTITY && self.tint_amount <= 0.0)
    }
}

/// The middle of the stage.
pub fn stage_center(stage: &Stage) -> Vec2 {
    Vec2::new(stage.width as f32 * 0.5, stage.height as f32 * 0.5)
}

/// How far a layer `depth` pixels behind the stage moves when the camera pans, as a fraction
/// of the stage's movement (1 at depth 0).
pub fn parallax(depth: f32) -> f32 {
    FOCAL / (FOCAL + clamp_depth(depth))
}

/// A layer depth kept in range (non-finite: 0).
pub fn clamp_depth(depth: f32) -> f32 {
    if depth.is_finite() { depth.clamp(MIN_DEPTH, MAX_DEPTH) } else { 0.0 }
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
        assert_eq!(c.view(&stage(), 0.0), Affine::IDENTITY);
        assert_eq!(c.view(&stage(), 5000.0), Affine::IDENTITY, "depth only matters when the camera moves");
        assert!(c.is_rest(&stage()));
        let f = c.frame(&stage());
        assert!(close(f[2], Vec2::new(200.0, 100.0)));
    }

    #[test]
    fn zoom_doubles_sizes_about_the_camera_centre() {
        let c = Camera { center: Vec2::new(60.0, 40.0), zoom: 2.0, ..Camera::rest(&stage()) };
        let v = c.view(&stage(), 0.0);
        assert!(close(v.apply(Vec2::new(60.0, 40.0)), Vec2::new(100.0, 50.0)), "the centre lands mid-output");
        assert!(close(v.apply(Vec2::new(70.0, 40.0)), Vec2::new(120.0, 50.0)), "10 px become 20");
        // The frame sees half the stage's width.
        let f = c.frame(&stage());
        assert!((f[1].x - f[0].x - 100.0).abs() < 1e-3);
    }

    #[test]
    fn rotation_turns_the_view_the_other_way() {
        let c = Camera { rotation: std::f32::consts::FRAC_PI_2, ..Camera::rest(&stage()) };
        let v = c.view(&stage(), 0.0);
        // A point right of the centre ends up above it (the world turns counter-clockwise).
        assert!(close(v.apply(Vec2::new(110.0, 50.0)), Vec2::new(100.0, 40.0)), "{:?}", v.apply(Vec2::new(110.0, 50.0)));
        // Round trip through the element.
        let back = Camera::of_element(&c.to_element(ElementId(1)), &stage());
        assert!((back.rotation - c.rotation).abs() < 1e-5 && (back.zoom - 1.0).abs() < 1e-5);
    }

    #[test]
    fn depth_scales_the_pan_only() {
        let c = Camera { center: Vec2::new(150.0, 50.0), zoom: 1.5, ..Camera::rest(&stage()) };
        assert!((parallax(FOCAL) - 0.5).abs() < 1e-6);
        let near = c.view(&stage(), 0.0).apply(Vec2::new(150.0, 50.0));
        let far = c.view(&stage(), FOCAL).apply(Vec2::new(125.0, 50.0));
        assert!(close(near, Vec2::new(100.0, 50.0)) && close(far, Vec2::new(100.0, 50.0)), "a far layer moved half as far");
        assert!((c.view(&stage(), FOCAL).mean_scale() - 1.5).abs() < 1e-4, "zoom is the same at every depth");
        assert_eq!(clamp_depth(f32::NAN), 0.0);
        assert_eq!(clamp_depth(-1e9), MIN_DEPTH);
    }

    #[test]
    fn hostile_cameras_stay_finite() {
        let mut el = Camera::rest(&stage()).to_element(ElementId(1));
        el.transform = Affine { m: [f32::NAN, 0.0, 0.0, f32::INFINITY, f32::NAN, 1e30] };
        el.color = ColorEffect::Tint { color: Rgba::WHITE, amount: f32::NAN };
        let c = Camera::of_element(&el, &stage());
        assert!(c.view(&stage(), 0.0).is_finite());
        assert!((MIN_ZOOM..=MAX_ZOOM).contains(&c.zoom));
        el.transform = Affine::scale(0.0, 0.0);
        assert_eq!(Camera::of_element(&el, &stage()).zoom, MIN_ZOOM);
    }
}
