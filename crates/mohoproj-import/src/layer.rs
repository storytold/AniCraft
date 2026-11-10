//! The inline layer tree → AniCraft's flat layer list with parent links.
//!
//! Moho stores `layers[]` bottom-first (corpus: CharWalk lists a rig's right limbs before
//! Body/Head and its left limbs last, and a head group lists the eye white before the pupil);
//! AniCraft's index 0 is the top. So the walk visits siblings back-to-front and emits a
//! container before the children it flattens, which keeps every folder's children in one
//! unbroken run under it (what `Timeline::repair_masks` and `group_range` expect).
//!
//! Type mapping: Mesh/Image/Text layers carry content; Group/Bone/Switch layers flatten to
//! folders (stage 1 freezes a switch at its current selection and keeps the variants as
//! hidden children); bones, effect stacks, physics and the camera are counted in the report.

use crate::anim;
use crate::channel::{self, Channel};
use crate::import::Builder;
use crate::json::{self, array, get};
use crate::mesh;
use crate::text;
use anicraft_doc::{ColorEffect, Element, ElementId, ElementKind, Keyframe, Layer, LayerId, LayerKind, Paint, Shape};
use anicraft_geom::{Affine, FillRule, Path};
use serde_json::Value;
use std::sync::Arc;

/// How deep Moho nesting may go before the importer gives up on a subtree (AniCraft's own
/// timeline cap is 16).
const MAX_TREE_DEPTH: usize = 16;
/// Most layers one document may produce.
const MAX_LAYERS: usize = 5_000;

/// One flattened layer, ready for the timeline.
pub(crate) struct FlatLayer {
    pub layer: Layer,
}

/// What a leaf layer draws, read from its type.
enum Content<'doc> {
    /// A vector layer's mesh (or `Null` when it has none).
    Mesh(&'doc Value),
    /// An image layer.
    Image(&'doc Value),
    /// A text layer.
    Text(&'doc Value),
}

/// What a layer inherits from its ancestors: where it sits in the tree, its accumulated
/// inch-space transform and alpha, and the switch-variant names a parent froze out of view.
pub(crate) struct Inherit<'a> {
    pub parent: Option<LayerId>,
    pub world: Affine,
    pub alpha: f32,
    pub hide: Option<&'a [String]>,
}

/// Walks Moho's layer array (bottom-first) into `out` (top-first, containers first).
pub(crate) fn walk(b: &mut Builder, layers: &[Value], into: &Inherit, depth: usize, out: &mut Vec<FlatLayer>) {
    if depth > MAX_TREE_DEPTH {
        b.report.skip("layers nested deeper than 16");
        return;
    }
    for layer in layers.iter().rev() {
        if out.len() >= MAX_LAYERS {
            b.report.skip("layers past 5,000");
            return;
        }
        build_layer(b, layer, into, depth, out);
    }
}

fn build_layer(b: &mut Builder, layer: &Value, into: &Inherit, depth: usize, out: &mut Vec<FlatLayer>) {
    let ty = json::string(get(layer, "type")).unwrap_or_default();
    let name = json::string(get(layer, "name")).unwrap_or_else(|| "Layer".into());
    let id = LayerId(b.doc.alloc_id());
    let mut l = Layer::new(id, name);
    l.parent = into.parent;
    l.visible = layer_visibility(b, layer) && into.hide.is_none_or(|h| !h.contains(&l.name));
    // The layer's own transform: animated channels (stage 2) on a leaf layer, or the static
    // bake of stage 1. A container's transform propagates to its children in Moho; AniCraft
    // folders carry nothing, so an animated container keeps the static bake and says so.
    let channels = anim::layer_transform_channels(get(layer, "transforms").unwrap_or(&Value::Null), &mut b.report);
    let animated_leaf = channels.is_animated() && !matches!(ty.as_str(), "GroupLayer" | "BoneLayer" | "SwitchLayer");
    if channels.is_animated() && !animated_leaf {
        b.report.skip("group transform animation (children drawn without it)");
    }
    let (local, alpha) = if animated_leaf {
        if into.world != Affine::IDENTITY {
            b.report.skip("animated layer inside a transformed group (its motion composes outside the group transform)");
        }
        (Affine::IDENTITY, layer_alpha(b, layer))
    } else {
        layer_transform(b, layer)
    };
    let alpha = alpha * into.alpha;
    let world = into.world.mul(&local);
    if animated_leaf {
        l.translation = channels.translation;
        l.scale = channels.scale;
        l.rotation = channels.rotation;
        l.shear = channels.shear;
        l.flip_h = channels.flip_h;
        l.flip_v = channels.flip_v;
        if let Some(origin) = get(layer, "origin").and_then(|v| json::vec2(Some(v))) {
            l.origin = b.to_px.apply(origin);
        }
    }
    match ty.as_str() {
        "MeshLayer" => {
            count_layer_extras(b, layer);
            let m = get(layer, "mesh").unwrap_or(&Value::Null);
            finish_leaf(b, l, world, alpha, Content::Mesh(m), out);
        }
        "ImageLayer" => {
            count_layer_extras(b, layer);
            finish_leaf(b, l, world, alpha, Content::Image(layer), out);
        }
        "TextLayer" => {
            count_layer_extras(b, layer);
            finish_leaf(b, l, world, alpha, Content::Text(layer), out);
        }
        "GroupLayer" | "BoneLayer" | "SwitchLayer" => {
            l.kind = LayerKind::Folder;
            if ty == "BoneLayer" || ty == "SwitchLayer" {
                count_skeleton(b, get(layer, "skeleton"));
            }
            count_layer_extras(b, layer);
            out.push(FlatLayer { layer: l });
            let kids = array(get(layer, "layers")).unwrap_or(&[]);
            if ty == "SwitchLayer" {
                // A switch freezes at its selection: everything but the selected variant hides.
                let hide_kids = switch_hide(b, layer);
                let mut combined: Vec<String> = into.hide.unwrap_or(&[]).to_vec();
                combined.extend(hide_kids.unwrap_or_default());
                let kids_into = Inherit { parent: Some(id), world, alpha, hide: (!combined.is_empty()).then_some(combined.as_slice()) };
                walk(b, kids, &kids_into, depth + 1, out);
            } else {
                let kids_into = Inherit { parent: Some(id), world, alpha, hide: into.hide };
                walk(b, kids, &kids_into, depth + 1, out);
            }
        }
        "NoteLayer" => b.report.skip("note layers"),
        "ParticleLayer" => b.report.skip("particle layers"),
        "PatchLayer" => b.report.skip("patch layers"),
        "AudioLayer" => b.report.skip("audio layers"),
        "" => b.report.skip("layers without a type"),
        other => b.report.skip(&format!("layers of unknown type ({other})")),
    }
}

/// The plain `visible` bool ANDs with the (channelled) visibility switch of the effect stack.
fn layer_visibility(b: &Builder, layer: &Value) -> bool {
    let plain = json::boolean(get(layer, "visible")).unwrap_or(true);
    let channel = get(layer, "layer_effects").and_then(|fx| get(fx, "visibility")).and_then(Channel::parse);
    plain && channel.is_none_or(|c| channel::sample_bool(&c, b.frame).unwrap_or(true))
}

/// A switch freezes at its current selection: the selected variant stays visible, the rest
/// hide (a stage-1 stand-in for the animated selector; the channel rides in the sidecar).
fn switch_hide(b: &mut Builder, layer: &Value) -> Option<Vec<String>> {
    let ch = get(layer, "switch_keys").and_then(Channel::parse)?;
    if ch.is_animated() {
        b.report.skip("switch layer animation (selection frozen)");
    }
    let selected = channel::sample_string(&ch, b.frame)?;
    if selected.is_empty() {
        b.report.notes.push("switch layer without a selection keeps every variant visible".into());
        return None;
    }
    b.report.skip(&format!("switch selection (frozen: {selected})"));
    let names: Vec<String> = array(get(layer, "layers"))
        .unwrap_or(&[])
        .iter()
        .filter_map(|l| json::string(get(l, "name")))
        .filter(|n| *n != selected)
        .collect();
    Some(names)
}

/// Skeletons (and the smart-bone actions riding on them) arrive in a later milestone; the
/// sidecar keeps every number.
fn count_skeleton(b: &mut Builder, skeleton: Option<&Value>) {
    let Some(bones) = skeleton.and_then(|s| get(s, "bones")).and_then(|a| json::array(Some(a))) else { return };
    if bones.is_empty() {
        return;
    }
    *b.report.skipped.entry("skeleton bones".into()).or_default() += bones.len();
    // Smart-bone actions: `actions` arrays inside the bones' anim channels, plus per-bone
    // references. Each one is a dial-driven pose the document model can't hold yet.
    let mut actions = 0usize;
    for bone in bones {
        for ch in ["anim_angle", "anim_pos", "anim_scale"] {
            actions += get(bone, ch).and_then(|c| get(c, "actions")).and_then(|a| json::array(Some(a))).map(|a| a.len()).unwrap_or(0);
        }
    }
    if actions > 0 {
        *b.report.skipped.entry("smart bone actions".into()).or_default() += actions;
    }
}

/// Per-layer features beyond the static transform, counted when non-default.
fn count_layer_extras(b: &mut Builder, layer: &Value) {
    let frame = b.frame;
    let ch = |k: &str| get(layer, k).and_then(Channel::parse);
    // Effect stack (alpha/visibility applied; the rest reported).
    let fx = get(layer, "layer_effects");
    for (key, what) in [("blur", "layer blur"), ("noise", "layer noise"), ("pixelation", "layer pixelation"), ("threshold", "layer threshold"), ("ambient_occlusion", "layer ambient occlusion")] {
        if fx.and_then(|f| get(f, key)).and_then(Channel::parse).is_some_and(|c| channel::sample_f64(&c, frame).is_some_and(|v| v.abs() > 1e-6)) {
            b.report.skip(what);
        }
    }
    let on = |k: &str| json::boolean(get(layer, k).and_then(|g| get(g, "on"))).unwrap_or(false);
    if on("layer_outline") {
        b.report.skip("layer outlines");
    }
    if on("layer_shading") {
        b.report.skip("layer shading");
    }
    if on("layer_shadow") {
        b.report.skip("layer shadows");
    }
    if on("motion_blur") {
        b.report.skip("motion blur");
    }
    if on("perspective_shadow") {
        b.report.skip("perspective shadows");
    }
    if json::integer(get(layer, "blend_mode")).is_some_and(|m| m != 0) {
        b.report.skip("layer blend modes");
    }
    if !json::string(get(layer, "follow_layer_uuid")).is_none_or(|u| u.is_empty()) {
        b.report.skip("follow-path relations (positions are already baked)");
    }
    if json::boolean(get(layer, "rotate_to_follow")).unwrap_or(false) {
        b.report.skip("rotate to follow");
    }
    if ch("timeline_markers").is_some_and(|c| channel::sample_string(&c, frame).is_some_and(|s| !s.is_empty())) {
        b.report.skip("layer timeline markers");
    }
    if array(get(layer, "actions")).is_some_and(|a| !a.is_empty()) {
        *b.report.skipped.entry("smart bone actions".into()).or_default() += 1;
    }
    if json::boolean(get(layer, "animated_layer_effects")).unwrap_or(false) {
        b.report.skip("animated layer effects");
    }
    if json::integer(get(layer, "parent_bone")).is_some_and(|i| i != -1) {
        b.report.skip("layers bound to bones");
    }
    if json::integer(get(layer, "timing_offset")).is_some_and(|i| i != 0) {
        b.report.skip("layer timing offsets");
    }
    if json::integer(get(layer, "masking")).is_some_and(|m| m != 0) {
        b.report.skip("mask layers (drawn unmasked)");
    }
    if json::integer(get(layer, "3d_mode")).is_some_and(|m| m != 0) {
        b.report.skip("3D layers");
    }
    if json::boolean(get(layer, "face_camera")).unwrap_or(false) {
        b.report.skip("3D face-camera layers");
    }
}

/// The layer's static alpha: the effect stack's alpha channel at the import frame (its
/// animation is a stage-3 effect-stack matter; visibility stays on the layer).
fn layer_alpha(b: &Builder, layer: &Value) -> f32 {
    let a = get(layer, "layer_effects")
        .and_then(|fx| get(fx, "alpha"))
        .and_then(Channel::parse)
        .and_then(|c| channel::sample_f64(&c, b.frame))
        .unwrap_or(1.0);
    (a as f32).clamp(0.0, 1.0)
}

/// Reads a layer's static transform channels (inch space, y up) around its origin pivot.
/// 3D-only channels are counted when they carry values. Called for layers whose transform
/// channels don't animate — animated leaves go through `anim::layer_transform_channels`.
fn layer_transform(b: &mut Builder, layer: &Value) -> (Affine, f32) {
    let frame = b.frame;
    let empty = Value::Null;
    let tr = get(layer, "transforms").unwrap_or(&empty);
    let ch = |k: &str| get(tr, k).and_then(Channel::parse);
    let (tx, ty, tz) = ch("translation").and_then(|c| channel::sample_vec3(&c, frame)).unwrap_or((0.0, 0.0, 0.0));
    if tz.abs() > 1e-6 {
        b.report.skip("3D layer translation");
    }
    let rot = ch("rotation_z").and_then(|c| channel::sample_f64(&c, frame)).unwrap_or(0.0) as f32;
    if ch("rotation_x").and_then(|c| channel::sample_f64(&c, frame)).is_some_and(|v| v.abs() > 1e-6)
        || ch("rotation_y").and_then(|c| channel::sample_f64(&c, frame)).is_some_and(|v| v.abs() > 1e-6)
    {
        b.report.skip("3D layer rotation");
    }
    let (sx, sy, sz) = ch("scale").and_then(|c| channel::sample_vec3(&c, frame)).unwrap_or((1.0, 1.0, 1.0));
    if (sz - 1.0).abs() > 1e-6 {
        b.report.skip("3D layer scale");
    }
    let (mut sx, mut sy) = (sx.max(1e-6), sy.max(1e-6));
    if ch("flip_h").and_then(|c| channel::sample_bool(&c, frame)).unwrap_or(false) {
        sx = -sx;
    }
    if ch("flip_v").and_then(|c| channel::sample_bool(&c, frame)).unwrap_or(false) {
        sy = -sy;
    }
    let (shx, shy, _) = ch("shear").and_then(|c| channel::sample_vec3(&c, frame)).unwrap_or((0.0, 0.0, 0.0));
    if ch("following").is_some_and(|c| c.is_animated()) {
        b.report.skip("follow progress animation");
    }
    if ch("physics_nudge").is_some_and(|c| c.is_animated()) {
        b.report.skip("physics nudge animation (drawn without it)");
    }
    let origin = get(layer, "origin").and_then(|v| json::vec2(Some(v))).unwrap_or(anicraft_geom::Vec2::ZERO);
    let shear = Affine { m: [1.0, shy, shx, 1.0, 0.0, 0.0] };
    // Moho pivots layer rotation, shear and scale at the layer origin:
    // p ↦ origin + translation + M · (p − origin).
    let m = Affine::translate(origin.x + tx, origin.y + ty).mul(&Affine::rotate(rot)).mul(&shear).mul(&Affine::scale(sx, sy)).mul(&Affine::translate(-origin.x, -origin.y));
    let m = if m.is_finite() {
        m
    } else {
        b.report.skip("unreadable layer transforms");
        Affine::IDENTITY
    };
    (m, layer_alpha(b, layer))
}

/// A leaf layer's content: one keyframe spanning the imported frame range.
fn finish_leaf(b: &mut Builder, mut l: Layer, world: Affine, alpha: f32, content: Content, out: &mut Vec<FlatLayer>) {
    let mut k = Keyframe::new(0, b.duration);
    let mut els: Vec<Arc<Element>> = Vec::new();
    match content {
        Content::Mesh(m) => mesh::mesh_elements(b, m, world, alpha, &mut els),
        Content::Image(src) => image_elements(b, src, world, alpha, &mut els),
        Content::Text(src) => text::text_elements(b, src, world, alpha, &mut els),
    }
    k.elements = els;
    l.keyframes = vec![Arc::new(k)];
    out.push(FlatLayer { layer: l });
}

/// An image layer: its file (found in the package or beside the document) becomes a bitmap
/// rectangle of the image's natural size, anchored at the layer origin.
fn image_elements(b: &mut Builder, layer: &Value, world: Affine, alpha: f32, out: &mut Vec<Arc<Element>>) {
    let Some(file) = fileref(layer) else {
        b.report.skip("image layers without a file reference");
        return;
    };
    let bytes = match (b.read)(&file) {
        Ok(Some(bytes)) => bytes,
        Ok(None) => {
            b.report.skip("missing image files");
            return;
        }
        Err(e) => {
            b.report.notes.push(format!("image file unreadable: {e}"));
            return;
        }
    };
    let short: String = file.rsplit(['/', '\\']).next().unwrap_or(&file).chars().take(256).collect();
    let Some(id) = b.register_bitmap(short, bytes) else {
        b.report.skip("image files that are not PNG or JPEG");
        return;
    };
    // Moho shows an image one-to-one at its natural pixel size: inches = pixels / dpi.
    let (bw, bh) = b.doc.bitmap(id).map(|m| (m.width, m.height)).unwrap_or((0, 0));
    let w = bw as f32 / b.dpi;
    let h = bh as f32 / b.dpi;
    let path = Path::rect(-w * 0.5, -h * 0.5, w, h);
    let paint = Paint::Bitmap { bitmap: id, repeat: false, matrix: Affine::IDENTITY };
    let fx = ColorEffect { alpha: alpha.clamp(0.0, 1.0), ..ColorEffect::default() };
    out.push(Arc::new(Element {
        id: ElementId(b.doc.alloc_id()),
        name: String::new(),
        transform: b.to_px.mul(&world),
        color: fx,
        kind: ElementKind::Shape(Shape { path: Arc::new(path), fill: Some(paint), stroke: None, fill_rule: FillRule::NonZero, merge: false }),
    }));
}

/// An image (or texture) file reference: `{path, relativeTo}`. Moho 14 names the image
/// layer's own file `image_fileref` (the corpus's only populated form; `relativeTo` values
/// seen so far: the project, absolute paths, and Moho's internal support folder — that last
/// one is Moho's own content, which this reader never reaches into, so those images land in
/// the report).
pub(crate) fn fileref(layer: &Value) -> Option<String> {
    let f = get(layer, "image_fileref").or_else(|| get(layer, "fileref")).or_else(|| get(layer, "layer_ref_fileref"))?;
    json::string(get(f, "path")).filter(|p| !p.is_empty())
}
