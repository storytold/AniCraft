use super::*;
use crate::{Easing, Element, ElementId, ElementKind, Keyframe, Layer, LayerId, Paint, Rgba, Tween, evaluate_layer_elements};
use anicraft_geom::Affine;

fn pin(id: u64, rest: (f32, f32), at: (f32, f32)) -> Pin {
    Pin { id: PinId(id), rest: Vec2::new(rest.0, rest.1), at: Vec2::new(at.0, at.1) }
}

fn blob() -> Arc<Path> {
    Arc::new(Path::ellipse(100.0, 50.0, 100.0, 50.0))
}

fn anchors(p: &Path) -> Vec<Vec2> {
    p.cmds
        .iter()
        .filter_map(|c| match *c {
            PathCmd::MoveTo(p) | PathCmd::LineTo(p) | PathCmd::QuadTo(_, p) | PathCmd::CubicTo(_, _, p) => Some(p),
            PathCmd::Close => None,
        })
        .collect()
}

#[test]
fn rest_pose_reproduces_the_artwork_exactly() {
    let rest = blob();
    let mut w = Warp::new(rest.clone(), 12);
    // No pins, then pins exactly where they were placed: the very path, shared.
    assert!(Arc::ptr_eq(&w.outline(), &rest));
    w.pins = vec![pin(1, (10.0, 50.0), (10.0, 50.0)), pin(2, (190.0, 50.0), (190.0, 50.0))];
    assert!(w.at_rest());
    assert!(Arc::ptr_eq(&w.outline(), &rest));
    assert_eq!(w.map(Vec2::new(33.3, 12.5)), Vec2::new(33.3, 12.5));
    assert_eq!(w.mesh().map(|m| m.rest == m.shown), Some(true));
}

#[test]
fn mls_interpolates_pins_and_is_rigid_with_one_or_two() {
    // One pin: everything moves with it.
    let one = [(Vec2::new(0.0, 0.0), Vec2::new(5.0, -3.0))];
    assert_eq!(mls_rigid(&one, Vec2::new(40.0, 40.0)), Vec2::new(45.0, 37.0));
    // Two pins turned a quarter turn about their middle: every point turns with them.
    let two = [(Vec2::new(-10.0, 0.0), Vec2::new(0.0, -10.0)), (Vec2::new(10.0, 0.0), Vec2::new(0.0, 10.0))];
    for v in [Vec2::new(3.0, 7.0), Vec2::new(-20.0, 4.0), Vec2::new(0.0, 0.0)] {
        let f = mls_rigid(&two, v);
        let want = Vec2::new(-v.y, v.x);
        assert!(f.distance(want) < 1e-3, "{v:?} → {f:?}, want {want:?}");
    }
    // On a pin: exactly its target.
    assert_eq!(mls_rigid(&two, Vec2::new(10.0, 0.0)), Vec2::new(0.0, 10.0));
    assert_eq!(mls_rigid(&[], Vec2::new(1.0, 2.0)), Vec2::new(1.0, 2.0));
}

#[test]
fn a_single_moved_pin_moves_nearby_points_most() {
    let mut w = Warp::new(Arc::new(Path::rect(0.0, 0.0, 300.0, 100.0)), 12);
    // Three pins along the strip; the right one is pulled down.
    w.pins = vec![pin(1, (10.0, 50.0), (10.0, 50.0)), pin(2, (150.0, 50.0), (150.0, 50.0)), pin(3, (290.0, 50.0), (290.0, 90.0))];
    let moved = |x: f32| w.map(Vec2::new(x, 50.0)).distance(Vec2::new(x, 50.0));
    let (near, mid, far) = (moved(270.0), moved(200.0), moved(40.0));
    assert!(near > mid && mid > far, "{near} > {mid} > {far}");
    assert!(near > 25.0 && far < 3.0, "near {near}, far {far}");
    // The pins themselves are where they were put (to within the mesh's interpolation).
    for p in &w.pins {
        assert!(w.map(p.rest).distance(p.at) < 1.5, "{p:?} → {:?}", w.map(p.rest));
    }
}

#[test]
fn deformation_is_as_rigid_as_possible() {
    // Rigidity: how much each triangle's map stretches or shears (its singular values' distance
    // from 1). A pure turn of the pins turns the mesh rigidly; a bend stays near rigid.
    let distortion = |w: &Warp| {
        let m = w.mesh().unwrap_or_else(unreachable_mesh);
        let mut worst = 0.0f32;
        for t in &m.tris {
            let [a, b, c] = t.map(|i| i as usize);
            let (r0, r1, r2) = (m.rest[a], m.rest[b], m.rest[c]);
            let (s0, s1, s2) = (m.shown[a], m.shown[b], m.shown[c]);
            // The linear part J with J·(r1 − r0, r2 − r0) = (s1 − s0, s2 − s0).
            let (e1, e2, f1, f2) = (r1 - r0, r2 - r0, s1 - s0, s2 - s0);
            let det = e1.cross(e2);
            let inv = [e2.y / det, -e2.x / det, -e1.y / det, e1.x / det];
            let j = [f1.x * inv[0] + f2.x * inv[2], f1.x * inv[1] + f2.x * inv[3], f1.y * inv[0] + f2.y * inv[2], f1.y * inv[1] + f2.y * inv[3]];
            let (p, q, r, s) = (j[0], j[1], j[2], j[3]);
            let e = (p * p + q * q + r * r + s * s) * 0.5;
            let d = (p * s - q * r).abs();
            let disc = (e * e - d * d).max(0.0).sqrt();
            let (big, small) = ((e + disc).sqrt(), (e - disc).max(0.0).sqrt());
            worst = worst.max((big - 1.0).abs()).max((small - 1.0).abs());
        }
        worst
    };
    let mut w = Warp::new(Arc::new(Path::rect(0.0, 0.0, 200.0, 100.0)), 10);
    let turn = Affine::translate(100.0, 50.0).mul(&Affine::rotate(0.6)).mul(&Affine::translate(-100.0, -50.0));
    w.pins = [(20.0, 20.0), (180.0, 30.0), (100.0, 90.0)].iter().enumerate().map(|(i, &(x, y))| Pin { id: PinId(i as u64), rest: Vec2::new(x, y), at: turn.apply(Vec2::new(x, y)) }).collect();
    assert!(distortion(&w) < 1e-3, "a turn is rigid: {}", distortion(&w));
    // A bend: one end of a strip turned down by 30°.
    w.pins = vec![pin(1, (10.0, 50.0), (10.0, 50.0)), pin(2, (100.0, 50.0), (100.0, 50.0)), pin(3, (190.0, 50.0), (178.0, 95.0))];
    let d = distortion(&w);
    assert!(d < 0.25, "a bend stays near rigid: {d}");
}

fn unreachable_mesh() -> WarpMesh {
    WarpMesh { rest: Vec::new(), shown: Vec::new(), tris: Vec::new() }
}

#[test]
fn straight_edges_follow_the_mesh_exactly() {
    // A bitmap's outline is a rectangle: bent, it is exactly the mesh's border.
    let mut w = Warp::new(Arc::new(Path::rect(0.0, 0.0, 120.0, 60.0)), 6);
    w.pins = vec![pin(1, (0.0, 0.0), (0.0, 0.0)), pin(2, (120.0, 60.0), (150.0, 90.0))];
    let m = w.mesh().unwrap_or_else(unreachable_mesh);
    let g = w.grid().unwrap_or(Grid { min: Vec2::ZERO, max: Vec2::ZERO, nx: 1, ny: 1 });
    assert_eq!((g.nx, g.ny), (6, 3));
    let pts = anchors(&w.outline());
    // Every border vertex of the mesh is on the bent outline.
    for (i, v) in m.rest.iter().enumerate() {
        let border = v.x == 0.0 || v.y == 0.0 || v.x == 120.0 || v.y == 60.0;
        if border {
            assert!(pts.iter().any(|p| p.distance(m.shown[i]) < 1e-3), "border vertex {v:?} missing");
        }
    }
    // And the outline has no point the border doesn't (straight lines between them).
    assert_eq!(pts.len(), 2 * (6 + 3) + 1);
}

#[test]
fn curves_split_and_stay_close_to_the_true_deformation() {
    let mut w = Warp::new(blob(), 16);
    w.pins = vec![pin(1, (0.0, 50.0), (0.0, 50.0)), pin(2, (100.0, 50.0), (100.0, 50.0)), pin(3, (200.0, 50.0), (190.0, 100.0))];
    let bent = w.outline();
    assert!(bent.cmds.len() > blob().cmds.len());
    // Points of the bent outline lie near where the exact MLS deformation puts the drawn curve.
    let pins: Vec<(Vec2, Vec2)> = w.pins.iter().map(|p| (p.rest, p.at)).collect();
    let truth: Vec<Vec2> = (0..400).map(|i| {
        let a = i as f32 / 400.0 * std::f32::consts::TAU;
        mls_rigid(&pins, Vec2::new(100.0 + 100.0 * a.cos(), 50.0 + 50.0 * a.sin()))
    }).collect();
    for p in anchors(&bent) {
        let d = truth.iter().map(|t| t.distance(p)).fold(f32::MAX, f32::min);
        assert!(d < 2.0, "{p:?} is {d} from the true curve");
    }
}

#[test]
fn unmap_finds_the_drawn_point() {
    let mut w = Warp::new(Arc::new(Path::rect(0.0, 0.0, 200.0, 100.0)), 8);
    w.pins = vec![pin(1, (20.0, 50.0), (20.0, 50.0)), pin(2, (180.0, 50.0), (200.0, 80.0))];
    for p in [Vec2::new(50.0, 30.0), Vec2::new(150.0, 70.0), Vec2::new(190.0, 10.0)] {
        let shown = w.map(p);
        let back = w.unmap(shown).unwrap_or(Vec2::new(f32::NAN, 0.0));
        assert!(back.distance(p) < 1e-2, "{p:?} → {shown:?} → {back:?}");
    }
    assert_eq!(w.unmap(Vec2::new(-500.0, 0.0)), None);
}

#[test]
fn deterministic_and_bounded() {
    let mut w = Warp::new(blob(), 1000);
    assert_eq!(w.resolution, MAX_RESOLUTION);
    w.pins = (0..MAX_PINS as u64).map(|i| pin(i, (i as f32, 50.0), (i as f32 + 3.0, 50.0 + (i % 7) as f32))).collect();
    let (a, b) = (w.outline(), w.outline());
    assert_eq!(fingerprint(&a), fingerprint(&b));
    let m = w.mesh().unwrap_or_else(unreachable_mesh);
    assert!(m.rest.len() <= ((MAX_RESOLUTION + 1) * (MAX_RESOLUTION + 1)) as usize);
    // Tiny, flat and enormous artwork never fails.
    for p in [Path::line(Vec2::new(0.0, 0.0), Vec2::new(0.0, 0.0)), Path::line(Vec2::new(0.0, 5.0), Vec2::new(300.0, 5.0)), Path::rect(0.0, 0.0, 1e7, 1.0), Path::default()] {
        let mut w = Warp::new(Arc::new(p), 12);
        w.pins = vec![pin(1, (0.0, 5.0), (40.0, 9.0)), pin(2, (100.0, 5.0), (100.0, 5.0))];
        let out = w.outline();
        assert!(out.cmds.iter().all(|c| match *c {
            PathCmd::MoveTo(p) | PathCmd::LineTo(p) => p.is_finite(),
            _ => true,
        }));
    }
    // Hostile pins are dropped on load; the rest stays.
    let mut h = Warp::new(blob(), 0);
    h.pins = vec![pin(1, (f32::NAN, 0.0), (0.0, 0.0)), pin(2, (1.0, 1.0), (2.0, 2.0)), pin(2, (3.0, 3.0), (3.0, 3.0))];
    assert!(h.sanitize());
    assert_eq!(h.pins.len(), 1);
    assert_eq!(h.resolution, MIN_RESOLUTION);
}

fn warped_layer(at_end: (f32, f32), easing: Easing) -> (Layer, Warp) {
    let mut shape = Shape::filled(Path::rect(0.0, 0.0, 200.0, 100.0), Paint::Solid(Rgba::hex(0x3080ff)));
    let mut w = Warp::new(shape.path.clone(), 8);
    w.pins = vec![pin(1, (10.0, 50.0), (10.0, 50.0)), pin(2, (190.0, 50.0), (190.0, 50.0))];
    shape.set_warp(w.clone());
    let el = |s: Shape| Arc::new(Element { id: ElementId(5), name: String::new(), transform: Affine::IDENTITY, color: Default::default(), kind: ElementKind::Shape(s) });
    let mut end = shape.clone();
    let mut w2 = w.clone();
    if let Some(p) = w2.pins.get_mut(1) {
        p.at = Vec2::new(at_end.0, at_end.1);
    }
    end.set_warp(w2);
    let mut layer = Layer::new(LayerId(1), "Art");
    let mut a = Keyframe::new(0, 10);
    a.elements = vec![el(shape)];
    a.tween = Some(Tween { easing, ..Default::default() });
    let mut b = Keyframe::new(10, 1);
    b.elements = vec![el(end)];
    layer.keyframes = vec![Arc::new(a), Arc::new(b)];
    (layer, w)
}

fn shape_at(layer: &Layer, frame: u32) -> Shape {
    let (_, els) = evaluate_layer_elements(layer, frame).unwrap_or((0, std::borrow::Cow::Owned(Vec::new())));
    match els.first().map(|e| &e.kind) {
        Some(ElementKind::Shape(s)) => s.clone(),
        _ => Shape::filled(Path::default(), Paint::Solid(Rgba::BLACK)),
    }
}

#[test]
fn pins_tween_between_keyframes() {
    let (layer, w) = warped_layer((190.0, 150.0), Easing::Linear);
    let mid = shape_at(&layer, 5);
    let mw = mid.live_warp().cloned().unwrap_or_else(|| w.clone());
    // Halfway the pulled pin is halfway, and the outline is the mesh bent by the halfway pins.
    assert_eq!(mw.pin(PinId(2)).map(|p| p.at), Some(Vec2::new(190.0, 100.0)));
    let mut want = w.clone();
    if let Some(p) = want.pins.get_mut(1) {
        p.at = Vec2::new(190.0, 100.0);
    }
    assert_eq!(*mid.path, *want.outline());
    // At the keyframes: exactly what they hold.
    assert!(Arc::ptr_eq(&shape_at(&layer, 0).path, &w.rest));
    // Easing applies to the pins as it does to bone poses.
    let (eased, _) = warped_layer((190.0, 150.0), Easing::QuadIn);
    let ep = shape_at(&eased, 5).live_warp().and_then(|w| w.pin(PinId(2)).map(|p| p.at.y)).unwrap_or(0.0);
    assert!(ep < 100.0 && ep > 50.0, "{ep}");
}

#[test]
fn a_shape_edited_by_another_tool_drops_out_of_its_warp() {
    let (layer, _) = warped_layer((190.0, 150.0), Easing::Linear);
    let mut s = shape_at(&layer, 10);
    assert!(s.live_warp().is_some());
    s.path = Arc::new(s.path.transformed(&Affine::translate(1.0, 0.0)));
    assert!(s.live_warp().is_none());
    let w = s.warp.as_deref().cloned().unwrap_or_else(|| Warp::new(blob(), 4)).rebased(s.path.clone());
    // Rebased, nothing moves: the edited outline is the new rest and the pins stay where they show.
    assert!(w.at_rest() && w.pins.iter().any(|p| p.rest == Vec2::new(190.0, 150.0)));
}
