//! The AniCraft shell: view state plus the frame loop. Document changes go through
//! `Session::execute`, so everything here is also reachable headless.

use crate::a11y::Named;
use crate::theme::{self, T};
use crate::{autosave, dialogs, keymap, menus, motion_editor, panels, pathtools, platform, prefs, stage, timeline};
use std::collections::HashSet;
use anicraft_engine::{Session, commands};
use egui::{Key, Modifiers};
use serde_json::{Value, json};
use std::path::PathBuf;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tool {
    Select,
    Subselect,
    Transform,
    /// Asset Warp (ADR 0079): pins on shapes and bitmaps that bend them.
    Warp,
    GradientTransform,
    Width,
    Bone,
    Bind,
    Pen,
    Text,
    Pencil,
    Brush,
    PaintBrush,
    Eraser,
    Rect,
    Oval,
    Star,
    Line,
    PaintBucket,
    InkBottle,
    Eyedropper,
    Camera,
    Hand,
    Zoom,
}

/// Paint Bucket gap size: how wide an opening in an outline still counts as closed. In screen
/// points, so it feels the same at any zoom.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GapSize {
    #[default]
    None,
    Small,
    Medium,
    Large,
}

impl GapSize {
    pub const ALL: [GapSize; 4] = [GapSize::None, GapSize::Small, GapSize::Medium, GapSize::Large];
    pub fn label(self) -> &'static str {
        match self {
            GapSize::None => tr_noop!("Don't close gaps"),
            GapSize::Small => tr_noop!("Close small gaps"),
            GapSize::Medium => tr_noop!("Close medium gaps"),
            GapSize::Large => tr_noop!("Close large gaps"),
        }
    }
    pub fn screen_px(self) -> f32 {
        match self {
            GapSize::None => 0.0,
            GapSize::Small => 4.0,
            GapSize::Medium => 10.0,
            GapSize::Large => 20.0,
        }
    }
}

impl Tool {
    pub const ALL: [Tool; 24] = [Tool::Select, Tool::Subselect, Tool::Transform, Tool::Warp, Tool::GradientTransform, Tool::Width, Tool::Bone, Tool::Bind, Tool::Pen, Tool::Text, Tool::Pencil, Tool::Brush, Tool::PaintBrush, Tool::Eraser, Tool::Rect, Tool::Oval, Tool::Star, Tool::Line, Tool::PaintBucket, Tool::InkBottle, Tool::Eyedropper, Tool::Camera, Tool::Hand, Tool::Zoom];

    pub fn label(self) -> &'static str {
        match self {
            Tool::Select => tr_noop!("Selection"),
            Tool::Subselect => tr_noop!("Subselection"),
            Tool::Transform => tr_noop!("Free Transform"),
            Tool::Warp => tr_noop!("Asset Warp"),
            Tool::GradientTransform => tr_noop!("Gradient Transform"),
            Tool::Width => tr_noop!("Width"),
            Tool::Bone => tr_noop!("Bone"),
            Tool::Bind => tr_noop!("Bind"),
            Tool::Pen => tr_noop!("Pen"),
            Tool::Text => tr_noop!("Text"),
            Tool::Pencil => tr_noop!("Pencil"),
            Tool::Brush => tr_noop!("Brush"),
            Tool::PaintBrush => tr_noop!("Paint Brush"),
            Tool::Eraser => tr_noop!("Eraser"),
            Tool::Rect => tr_noop!("Rectangle"),
            Tool::Oval => tr_noop!("Oval"),
            Tool::Star => tr_noop!("PolyStar"),
            Tool::Line => tr_noop!("Line"),
            Tool::PaintBucket => tr_noop!("Paint Bucket"),
            Tool::InkBottle => tr_noop!("Ink Bottle"),
            Tool::Eyedropper => tr_noop!("Eyedropper"),
            Tool::Camera => tr_noop!("Camera"),
            Tool::Hand => tr_noop!("Hand"),
            Tool::Zoom => tr_noop!("Zoom"),
        }
    }
    /// Default shortcut in the command-registry notation; the live key is in `App::keymap`.
    pub fn shortcut(self) -> Option<&'static str> {
        match self {
            Tool::Select => Some("V"),
            Tool::Subselect => Some("A"),
            Tool::Transform => Some("Q"),
            Tool::Warp => Some("Shift+Q"),
            Tool::GradientTransform => Some("F"),
            Tool::Width => Some("U"),
            Tool::Bone => Some("M"),
            Tool::Bind => Some("Shift+M"),
            Tool::Pen => Some("P"),
            Tool::Text => Some("T"),
            Tool::Pencil => Some("Shift+Y"),
            Tool::Brush => Some("B"),
            Tool::PaintBrush => Some("Y"),
            Tool::Eraser => Some("E"),
            Tool::Rect => Some("R"),
            Tool::Oval => Some("O"),
            Tool::Star => None,
            Tool::Line => Some("N"),
            Tool::PaintBucket => Some("K"),
            Tool::InkBottle => Some("S"),
            Tool::Eyedropper => Some("I"),
            Tool::Camera => Some("C"),
            Tool::Hand => Some("H"),
            Tool::Zoom => Some("Z"),
        }
    }
    /// English (a key): translate where it is drawn with `crate::i18n::t`.
    pub fn hint(self) -> &'static str {
        match self {
            Tool::Select => tr_noop!("Click to select, drag to move, drag on empty space to marquee-select. Shift adds to the selection."),
            Tool::Subselect => tr_noop!("Click a shape to show its points; on a merged drawing, the points of everything connected to it. Drag points or handles to reshape (a shared point moves every shape that has it; a smooth point's handles stay in line, Alt moves one alone); drag a line or fill to move everything connected. Shift adds to the point selection, drag on empty space to marquee points, arrow keys nudge them. Double-click an edge to add a point, Delete removes points, and right-click makes a point smooth or a corner. A motion path's points edit the same way (its ends stay)."),
            Tool::Pen => tr_noop!("Click to place corner points, drag to pull smooth handles (Alt moves one handle). Click the first point to close; Enter, Esc or a click on the last point finishes. Backspace removes the last point. Alt-click a selected shape, a merged drawing's line or a motion path to add a point to it. Points snap to points and lines while Snap to Objects is on."),
            Tool::Text => tr_noop!("Click to type new text, or click text to edit it. Esc or a click elsewhere finishes. The options set the font, size and colour; double-click text with the Selection tool to edit it too."),
            Tool::Pencil => tr_noop!("Drag to draw freehand. The options bar sets Smooth, Straighten or Ink."),
            Tool::Brush => tr_noop!("Drag to paint filled shapes in the fill colour (a click paints a dab). [ and ] change the size; the options set the brush, its shape, pressure, smoothing and paint mode."),
            Tool::PaintBrush => tr_noop!("Drag to draw a line in the stroke colour whose width follows the pen's pressure and the brush's tapers. Shape it afterwards with the Width tool. [ and ] change the size; the options set the brush and its pressure curve."),
            Tool::Eraser => tr_noop!("Drag to erase lines and fills drawn with Object Drawing off (drawing objects and symbols are kept). [ and ] change the size; the options set the shape and erase mode, and the Faucet deletes a whole fill or line with a click. Double-click the tool to erase everything on the stage."),
            Tool::Width => tr_noop!("Hover a stroke to see its width points. Drag a point's handle to widen or narrow the stroke there (Alt: one side only), drag its dot to slide it along, or drag anywhere on the stroke to add a point. Click a point and press Delete to remove it."),
            Tool::Bone => tr_noop!("Drag from one symbol instance to another to join them with a bone (they move to an armature layer), or drag inside a shape to draw a bone in it (the shape bends with its bones); start on a bone's end to continue the chain. Click a bone to select it. Pose the armature by dragging bones or instances with the Selection tool."),
            Tool::Bind => tr_noop!("Shows the points of shapes with bones: a square follows one bone, a triangle several. Click a bone to see the points it moves (filled). Click a point, or drag a box around points, to make them follow only that bone; Shift shares the bone with the ones they follow, Ctrl takes them from it. Turn on Paint Weights in Properties to brush the bone's influence on, off or smooth, shown as a heat map."),
            Tool::Warp => tr_noop!("Click a shape or a bitmap to pin it (the first pin covers it with a warp mesh). Drag a pin to bend the artwork around the other pins; with one pin it moves. Click a pin and press Delete to remove it. A pin moved at another frame keys it, and pins tween."),
            Tool::GradientTransform => tr_noop!("Click a shape with a gradient or bitmap fill to show its handles. Drag the centre to move the fill, the square to change its width, the circle to change its size, the knob to rotate it (Shift: 15° steps) and, on a radial gradient, the triangle to move its focal point. A bitmap fill also has a square for its height and diamonds to skew it. Each drag is one undo step."),
            Tool::Transform => tr_noop!("Drag a corner or edge to scale (Shift keeps proportions, Alt scales from the centre), just outside a corner to rotate (Shift snaps 15°), inside to move."),
            Tool::Rect => tr_noop!("Drag to draw a rectangle. Shift draws a square."),
            Tool::Oval => tr_noop!("Drag to draw an oval. Shift draws a circle."),
            Tool::Star => tr_noop!("Drag from the centre to draw a star or a polygon (Shape in the options)."),
            Tool::Line => tr_noop!("Drag to draw a line. Shift snaps to 45°."),
            Tool::PaintBucket => tr_noop!("Click inside an outline to fill it (on the layer's lines), or on a fill to recolour it. The gap size closes small openings."),
            Tool::InkBottle => tr_noop!("Click a shape to give it the current stroke colour and width."),
            Tool::Eyedropper => tr_noop!("Click a fill to pick its colour, gradient or bitmap (then pour it with the Paint Bucket), or a line to pick its colour, width and profile (then apply it with the Ink Bottle). With shapes selected, the picked style goes onto them. Shift-click picks into the other swatch only."),
            Tool::Camera => tr_noop!("Drag on the stage to pan the camera, drag the round handle to turn it, and use the wheel to zoom (each drag is one undo step). Picking the tool adds a camera if the scene has none."),
            Tool::Hand => tr_noop!("Drag to pan the stage. Space works with any tool."),
            Tool::Zoom => tr_noop!("Click to zoom in, Alt-click to zoom out. Ctrl+wheel zooms anywhere."),
        }
    }
}

/// Text being typed on the stage (view state until it is committed as one command).
pub struct TextEditing {
    /// Top-left of the text box, in stage space.
    pub at: egui::Pos2,
    /// The text element being edited, or `None` for new text.
    pub element: Option<anicraft_engine::doc::ElementId>,
    pub text: String,
    /// The text when editing started (unchanged text commits nothing).
    pub original: String,
    /// Focus the field on its first frame.
    pub focus: bool,
}

/// Stage camera.
pub struct StageView {
    pub zoom: f32,
    pub pan: egui::Vec2,
    pub fit: bool,
    /// The canvas size and zoom of the last fit: while the view is still as fitted, it fits
    /// again when the canvas changes size (window resized, panels or interface scale changed).
    pub fitted: Option<(egui::Vec2, f32)>,
    /// View › Clip Content Outside the Stage (#251): hide artwork on the pasteboard (it is still
    /// there, and still exported only where it is on the stage). Off: it shows, as in Animate.
    pub clip_to_stage: bool,
}

/// In-progress pointer gesture on the stage.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Drag {
    None,
    Move { last: egui::Pos2 },
    /// Selection tool on the camera preview (ADR 0082): moving the selection with the pointer,
    /// which was at picture point `last`; each move maps back through the camera and the
    /// active layer's depth.
    CameraMove { last: anicraft_engine::geom::Vec2 },
    /// Selection tool: moving the selection, snapping as it goes (ADR 0041): from stage point
    /// `start`, `applied` moved so far.
    SnapMove { start: egui::Pos2, applied: egui::Vec2 },
    /// Selection tool: bending segment `cmd` of raw shape `id` at `t` (ADR 0041); the pointer
    /// was at `last` last frame.
    Bend { id: anicraft_engine::doc::ElementId, cmd: usize, t: f32, last: egui::Pos2 },
    /// Selection tool: moving a shared corner or end (ADR 0041), now at `at`; `off` is from the
    /// pointer to the point. The points that move are `App::raw_points`.
    RawPoint { at: egui::Pos2, off: egui::Vec2 },
    Marquee { start: egui::Pos2, cur: egui::Pos2 },
    /// Edit Multiple Frames (ADR 0047): moving the selection on every keyframe it is on, the
    /// pointer at `last` last frame; or a marquee over them.
    MultiMove { last: egui::Pos2 },
    MultiMarquee { start: egui::Pos2, cur: egui::Pos2 },
    /// Free Transform on Edit Multiple Frames' selection (ADR 0051): as `Transform`, on every
    /// keyframe the selection is on, about one shared pivot.
    MultiTransform { kind: XfKind, start: egui::Pos2, bounds: [f32; 4], applied: anicraft_engine::geom::Affine },
    Shape { start: egui::Pos2, cur: egui::Pos2 },
    /// Subselection: dragging the selected points (stage position of the last frame).
    Points { last: egui::Pos2 },
    /// Subselection: dragging handle `slot` of segment `cmd` of raw shape `id` (ADR 0042); the
    /// pointer was at `last` last frame.
    RawHandle { id: anicraft_engine::doc::ElementId, cmd: usize, slot: u8, last: egui::Pos2 },
    /// Subselection: marquee over the selected shapes' anchors.
    PointMarquee { start: egui::Pos2, cur: egui::Pos2 },
    /// Pencil stroke; the samples are in `App::pencil`.
    Pencil,
    /// Brush stroke; the samples are in `App::brush_samples`.
    Brush,
    /// Eraser stroke; the samples are in `App::brush_samples` too (ADR 0034).
    Eraser,
    /// Width tool: resizing (or sliding) width point `index` of shape `id`, found on subpath
    /// `sub`; `start` is its left and right width when the drag began.
    /// `index` is the grabbed width point (None: one the drag adds at `t`).
    Width { id: anicraft_engine::doc::ElementId, sub: usize, index: Option<usize>, t: f32, grip: crate::widthtool::Grip, start: (f32, f32) },
    /// Gradient Transform tool (ADR 0074): dragging `grip` of shape `id`'s fill, whose frame was
    /// `frame` when the drag began at stage point `press`.
    Gradient { id: anicraft_engine::doc::ElementId, grip: crate::gradient_tool::Grip, frame: anicraft_engine::geom::Affine, press: egui::Pos2 },
    /// Envelope (ADR 0080): dragging boundary handle `index` of the mesh `mesh` (as it was when
    /// the drag began at stage point `press`).
    Envelope { index: usize, mesh: anicraft_engine::geom::envelope::Mesh, press: anicraft_engine::geom::Vec2 },
    /// Free transform: `applied` is the transform already sent to the engine this gesture.
    Transform { kind: XfKind, start: egui::Pos2, bounds: [f32; 4], applied: anicraft_engine::geom::Affine },
    /// Dragging shape hint `index` on the side the frame shows (ADR 0023).
    Hint { index: usize, side: anicraft_engine::doc::HintSide },
    /// Editing the motion path (ADR 0030) from stage point `start`; the path as it was when the
    /// drag began is `App::motion_drag`.
    MotionPath { grab: crate::motion_path::Grab, start: egui::Pos2 },
    /// Moving the whole motion path (ADR 0038), from stage point `last` last frame.
    MotionMove { last: egui::Pos2 },
    /// Camera tool (ADR 0027): panning from screen point `last`, turning from pointer angle
    /// `last` (radians about the camera centre), or zooming with the wheel until `until`
    /// (seconds; the turns of one burst are one undo step).
    CameraPan { last: egui::Pos2 },
    CameraRotate { last: f32 },
    CameraWheel { until: f64 },
    /// Bone tool (ADR 0043): drawing a bone from instance `from` (or inside shape `from`, or on
    /// from a shape bone's joint node, ADR 0071), joint `head` to `cur` (stage).
    Bone { from: anicraft_engine::doc::ElementId, head: egui::Pos2, cur: egui::Pos2 },
    /// Bind tool (ADR 0071): a box from `start` to `cur` (stage) binding the points inside it.
    BindMarquee { start: egui::Pos2, cur: egui::Pos2 },
    /// Bind tool, Paint Weights (ADR 0090): a brush stroke whose last dab was at `last` (stage).
    BindPaint { last: egui::Pos2 },
    /// Posing by inverse kinematics: point `local` of the bone's instance follows the pointer.
    BonePose { bone: anicraft_engine::doc::BoneId, local: egui::Pos2 },
    /// Moving a whole armature by its root, from stage point `last` last frame.
    ArmatureMove { layer: anicraft_engine::doc::LayerId, last: egui::Pos2 },
    /// Asset Warp (ADR 0079): pin `pin` of `element` follows the pointer.
    WarpPin { element: anicraft_engine::doc::ElementId, pin: anicraft_engine::doc::warp::PinId },
    /// Moving one of the open symbol's 9-slice guides (ADR 0055).
    SliceGuide { guide: crate::scale_grid::Guide },
    /// Selection tool: moving the scene's guide `index` (ADR 0076); `off`: the pointer is over a
    /// ruler or off the stage, where letting go deletes it.
    Guide { index: usize, off: bool },
    Pan,
}

/// What a free-transform drag does.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum XfKind {
    Move,
    /// Handle position: -1/0/1 on each axis (corner, edge or centre).
    Scale { hx: i8, hy: i8 },
    Rotate,
}

/// A small text prompt standing in for native file dialogs (M0).
pub struct Prompt {
    /// English (a key): translated where it is drawn.
    pub title: &'static str,
    pub cmd: &'static str,
    pub key: &'static str,
    pub text: String,
    pub focus: bool,
}

pub struct Status {
    pub text: String,
    pub error: bool,
    pub at: f64,
}

/// `--screenshot` state.
pub struct Shot {
    pub path: PathBuf,
    pub frames: u32,
    pub requested: bool,
    /// Scaled down to this width when written (`--shot-width`).
    pub width: Option<u32>,
}

pub const SHOT_FRAMES: u32 = 120;

/// The item picked in the Library panel (view state).
pub use anicraft_engine::doc::LibraryItem;

/// The PolyStar tool's settings (ADR 0080): a polygon or a star, how many sides or points (3 to
/// 32), and how deep a star's points are (0 to 1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PolyStarOptions {
    pub star: bool,
    pub sides: u32,
    pub point_size: f32,
}

impl Default for PolyStarOptions {
    fn default() -> Self {
        PolyStarOptions { star: true, sides: 5, point_size: 0.5 }
    }
}

impl PolyStarOptions {
    /// `draw.star`'s shape params.
    pub fn params(&self) -> serde_json::Value {
        serde_json::json!({"shape": if self.star { "star" } else { "polygon" }, "sides": self.sides.clamp(3, 32), "point_size": self.point_size.clamp(0.0, 1.0)})
    }
}

pub struct App {
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) updates: crate::updates::Updates,
    pub s: Session,
    pub tool: Tool,
    pub view: StageView,
    pub stage_cache: stage::Cache,
    pub drag: Drag,
    /// The shape hint a right-click landed on (its menu is showing).
    pub hint_menu: Option<usize>,
    pub playing: bool,
    pub looping: bool,
    play_acc: f64,
    /// Onion skin and Edit Multiple Frames (ADR 0047): view state, passed with each render.
    pub onion: anicraft_engine::onion::OnionSkin,
    pub onion_ui: crate::onion::OnionUi,
    /// View › Preview Through Camera: the stage shows what the movie shows (ADR 0027).
    pub camera_preview: bool,
    /// The tool in hand last frame (picking the Camera tool adds a camera).
    pub last_tool: Tool,
    pub status: Option<Status>,
    pub prompt: Option<Prompt>,
    /// The Convert to Symbol dialog, and what it chose last (ADR 0053).
    pub convert: Option<crate::convert_symbol::ConvertDialog>,
    /// The Layer Properties dialog while it is open (ADR 0061).
    pub layer_props: Option<crate::layer_props::LayerPropsDialog>,
    /// Open the timeline's Frame View menu on the next frame (screenshots, the audit).
    pub tl_frame_view_open: bool,
    /// How far the timeline header's content overflowed its width last frame (points; ≤ 0: it
    /// fits). Checked by a test at the default window size.
    pub tl_header_spill: f32,
    /// Open this layer's colour popup on the next frame (screenshots, the audit).
    pub tl_color_open: Option<anicraft_engine::doc::LayerId>,
    pub convert_last: crate::convert_symbol::Last,
    pub show_about: bool,
    pub show_files: bool,
    pub show_licenses: bool,
    pub shot: Option<Shot>,
    pub cursor_stage: Option<egui::Pos2>,
    pub tl_scroll: f32,
    /// Timeline frame width (zoom), in points.
    pub tl_cell_w: f32,
    /// The zoom the timeline had before a button opened and widened its frames for their state
    /// names, and the zoom it set (ADR 0057): it goes back on closing, unless zoomed meanwhile.
    pub tl_button_zoom: Option<(f32, f32)>,
    /// Timeline vertical scroll, in rows.
    pub tl_vscroll: f32,
    pub tl_last_layer: Option<anicraft_engine::doc::LayerId>,
    /// Folders shown closed on the timeline (view state: not saved, not undoable).
    pub tl_collapsed: HashSet<anicraft_engine::doc::LayerId>,
    pub library_sel: Option<LibraryItem>,
    /// The Library panel's folders, selection, sorting, search and preview (ADR 0050).
    pub library: crate::library::LibraryView,
    /// Text being typed on the stage.
    pub text_edit: Option<TextEditing>,
    /// Attach to Path is waiting for a click on a shape (ADR 0070): the text to set along it.
    pub path_pick: Option<anicraft_engine::doc::ElementId>,
    /// Font families on this computer, listed once on first need.
    pub font_families: Option<Vec<String>>,
    /// The PolyStar tool's settings (ADR 0080), as Animate's Tool Settings.
    pub polystar: PolyStarOptions,
    /// An envelope is open (ADR 0080): it closes when it no longer shows.
    pub envelope_open: bool,
    pub rect_radius: f32,
    /// Pen path being drawn (view state until it is committed as one command).
    pub pen: Option<pathtools::PenPath>,
    /// Pencil samples of the stroke in progress (stage space).
    pub pencil: Vec<anicraft_engine::geom::Vec2>,
    /// A Selection-tool point drag's points (`raw.move_point`'s `points`), and the raw shapes a
    /// bend or point drag changed (merged again when it ends; ADR 0041).
    pub raw_points: serde_json::Value,
    pub raw_touched: Vec<u64>,
    /// A Subselection point drag: the other selected anchors that move with the one dragged
    /// (`raw.move_point`'s `with`, ADR 0042).
    pub raw_with: Vec<anicraft_engine::geom::Vec2>,
    /// Subselection on raw shapes (ADR 0042): the selected anchors (stage positions, shared by
    /// every shape that has them), and the ends of the edge last clicked, whose handles show.
    pub raw_anchors: Vec<anicraft_engine::geom::Vec2>,
    pub raw_ends: Vec<anicraft_engine::geom::Vec2>,
    /// Where what is being dragged snapped this frame (stage space): the snap ring.
    pub snap_ring: Option<anicraft_engine::geom::Vec2>,
    /// A guide being dragged out of a ruler (ADR 0076).
    pub new_guide: Option<crate::rulers::NewGuide>,
    /// Edit Grid and Edit Guides, while open.
    pub aids_dialogs: crate::aids_dialog::AidsDialogs,
    pub pencil_mode: pathtools::PencilMode,
    /// Brush tool options, the samples of the stroke in progress (stage space, pen pressure
    /// when there is one) and the pen's last pressure (ADR 0033).
    pub brush: crate::brushtool::BrushOptions,
    /// The Paint Brush's options (ADR 0068), its own as in Animate.
    pub paint_brush: crate::brushtool::BrushOptions,
    /// When the last sample came (seconds), for Velocity.
    pub brush_clock: f64,
    /// Window › Brush Library and the brush pickers (ADR 0068).
    pub brush_lib: crate::brush_panel::BrushLibraryView,
    /// A filter preset being named in Properties (ADR 0091).
    pub filter_naming: crate::panels::FilterNaming,
    pub brush_samples: Vec<(anicraft_engine::geom::Vec2, Option<f32>)>,
    pub pen_pressure: Option<f32>,
    /// Whether a pointer event has carried a pen's tilt this session (ADR 0093). Until one has,
    /// the brushes' Tilt options stay hidden: they would do nothing.
    pub pen_tilt_seen: bool,
    /// Eraser tool options (ADR 0034); its stroke's samples are `brush_samples`.
    pub eraser: crate::erasertool::EraserOptions,
    pub bucket_gap: GapSize,
    /// Align panel: align to the stage rather than the selection's bounds.
    pub align_to_stage: bool,
    /// Whether swatch clicks set the stroke or the fill (the Colour and Swatches panels).
    pub colour_target: crate::swatches_panel::ColourTarget,
    /// The swatch picked in the Swatches panel (Duplicate and Delete act on it).
    pub swatch_sel: Option<usize>,
    /// Subselection: selected anchors and handles.
    pub points: Vec<pathtools::PointRef>,
    /// Width tool: the selected width point (shape, index).
    pub width_sel: Option<(anicraft_engine::doc::ElementId, usize)>,
    /// Opens the Tools bar's fill swatch picker on the next frame (the `swatches` screenshot).
    pub open_fill_swatch: bool,
    /// The bone picked on the stage (ADR 0043; view state).
    pub bone_sel: Option<anicraft_engine::doc::BoneId>,
    /// The Bind tool's options: Paint Weights, the brush, the focused point (ADR 0090).
    pub bind: crate::bind_tool::BindOptions,
    /// The warp pin picked with the Warp tool, and the artwork it is on (ADR 0079; view state).
    pub warp_pin: Option<(anicraft_engine::doc::ElementId, anicraft_engine::doc::warp::PinId)>,
    /// Subselection: the selected motion path anchors, whose handles show (ADR 0030, 0038).
    pub motion_anchors: Vec<usize>,
    /// The motion path as it was when the drag on it began.
    pub motion_drag: Option<anicraft_engine::geom::Path>,
    /// Selected frames on the timeline, and the drag on the frame grid.
    pub frame_sel: Option<timeline::FrameSel>,
    pub tl_drag: timeline::TlDrag,
    /// Where the panels are (ADR 0075): the current workspace's arrangement.
    pub dock: crate::dock::Layout,
    /// The dock's view state: a tab being dragged, where groups were drawn.
    pub dock_view: crate::dock_ui::DockView,
    /// Window › Workspace › Manage Workspaces… (ADR 0087).
    pub manage_workspaces: crate::workspaces_window::Manage,
    /// The Motion Editor's view state: the property whose curve it shows, the picked key
    /// (frames into the span), its drag and its list scroll.
    pub me_prop: anicraft_engine::doc::Property,
    pub me_key: Option<u32>,
    /// The Motion Editor's picked keys (property, offset into the span), across properties,
    /// and the span they belong to (layer id, first frame) (ADR 0083).
    pub me_sel: Vec<(anicraft_engine::doc::Property, u32)>,
    pub me_sel_at: (u64, u32),
    /// A screenshot shows a Motion Editor drag in time (no pointer then): its readout sits at
    /// the dragged keys.
    pub me_shot_drag: bool,
    pub me_drag: Option<motion_editor::MeDrag>,
    pub me_vscroll: f32,
    /// Colour effect styles the Motion Editor lists besides those the tween uses (ADR 0056).
    pub me_styles: Vec<anicraft_engine::doc::ColorStyle>,
    /// The frame grid was clicked more recently than the stage: Ctrl+X/C/V act on frames.
    pub timeline_focus: bool,
    /// A destructive command waiting for the user's decision (ADR 0004).
    pub confirm: Option<dialogs::PendingConfirm>,
    /// Undoable-tier confirmations the user chose not to see again this session.
    pub skip_confirm: HashSet<String>,
    pub palette: Option<dialogs::Palette>,
    /// The open plug-in dialog (its preview is an open gesture).
    pub plugin_dialog: Option<crate::plugins_ui::Dialog>,
    /// File › Export with an exporter plug-in: its window, and the export while it runs (ADR 0088).
    pub plugin_export: Option<crate::plugin_export_ui::Export>,
    /// Commands › Manage Plug-ins.
    pub show_plugins: bool,
    /// The last Test Movie's Output, for the Output panel once it has closed.
    pub output_log: Vec<(u64, String)>,
    /// Edit › Find and Replace (Ctrl+F) and its results (ADR 0058).
    pub find: crate::find_panel::FindState,
    /// File › Publish Settings and the publish running or last run (ADR 0060).
    pub publish: crate::publish_panel::PublishState,
    /// Window › Movie Explorer, the Library dock's other tab (ADR 0058).
    pub explorer: crate::movie_explorer::ExplorerView,
    pub actions: crate::actions_panel::State,
    /// Control › Test Movie: the movie playing in its own window.
    pub test_movie: Option<crate::test_movie::TestMovie>,
    /// Edit Envelope: a keyframe sound's volume per channel, in its own window (ADR 0039).
    pub envelope: Option<crate::envelope::EnvelopeEditor>,
    /// The Custom Ease window (ADR 0081), when open.
    pub ease: Option<crate::ease_editor::EaseEditor>,
    pub show_shortcuts: bool,
    /// The window close was intercepted because of unsaved changes.
    pub close_dialog: bool,
    pub allow_close: bool,
    /// Close the window once the pending Save As succeeds.
    pub close_after_save: bool,
    pub prefs: prefs::Prefs,
    pub show_prefs: bool,
    /// Installed fonts, scanned off the UI thread on first need.
    pub system_fonts: Option<std::sync::Arc<prefs::SystemFonts>>,
    fonts_rx: Option<std::sync::mpsc::Receiver<prefs::SystemFonts>>,
    /// Re-check which glyphs the active fonts have (after fonts change).
    glyph_check: bool,
    /// Document revision at the end of the last frame (edits schedule a repaint).
    last_revision: u64,
    title: String,
    /// Keyboard shortcuts in effect (defaults plus the user's changes) and their lookup table.
    pub keymap: keymap::Keymap,
    pub(crate) shortcuts: Vec<(Modifiers, Key, &'static str)>,
    /// The shortcut editor is waiting for a key press: global shortcuts pause.
    pub key_capture: Option<dialogs::KeyCapture>,
    /// Crash-recovery autosave and the copies earlier sessions left behind (ADR 0015).
    pub recovery: autosave::Recovery,
    /// Sound output for playback, scrubbing and library previews.
    pub sound: crate::sound::SoundOut,
    /// Playback follows the sound's clock (the movie has sounds and the device plays them).
    audio_clock: bool,
    /// The scene and frame last heard, so a moved playhead plays a snippet (scrubbing).
    heard: Option<(usize, u32)>,
    /// The panel Ctrl+F6 last moved focus to (where the next press continues from).
    pub last_panel: Option<crate::a11y::Region>,
    /// The stage's elements as keyboard stops and screen-reader nodes (ADR 0064).
    pub stage_nav: crate::stage_a11y::StageNav,
    /// The layer and frame the focused timeline last announced (ADR 0064).
    pub tl_heard: Option<(anicraft_engine::doc::LayerId, u32)>,
    /// The arrows moved the active layer: its row takes keyboard focus when drawn.
    pub tl_refocus: bool,
    /// The menu bar from the keyboard: Alt, F10 and the arrows (ADR 0064).
    pub menu_keys: menus::MenuKeys,
    /// What the system last said about contrast and light or dark (ADR 0064, ADR 0077): each part
    /// `None` until it has, or if it can't. Screenshot runs never ask, so they look the same
    /// everywhere.
    pub system_look: platform::SystemLook,
    /// Keys to press, a few frames apart (screenshots of keyboard states).
    pub key_script: std::collections::VecDeque<(Key, Modifiers)>,
    /// Screenshots only: the ease menu (by its id salt) to show open on the next frame.
    pub shot_ease_menu: std::cell::Cell<Option<&'static str>>,
    /// Screenshots only: where to click once the frames have settled.
    pub shot_click: std::cell::Cell<Option<egui::Pos2>>,
    shot_step: u8,
    key_script_wait: u8,
    /// The system's look being read off the UI thread, and when it last was.
    look_probe: Option<std::sync::mpsc::Receiver<platform::SystemLook>>,
    look_asked: Option<f64>,
    /// The start screen, while it is up (ADR 0059).
    pub start: Option<crate::start::StartScreen>,
    /// Files dragged over and dropped onto the window (ADR 0059).
    pub drop: crate::drop::DropState,
    /// Recent files' pictures for the start screen.
    pub thumbs: crate::recent::Thumbs,
    /// Templates: their pictures, the user's list and the Save as Template dialog (ADR 0065).
    pub templates: crate::templates::Templates,
}

/// How often (seconds) the system's contrast and light or dark setting are asked again (ADR 0064).
const SYSTEM_LOOK_EVERY: f64 = 4.0;

/// How finished this version is, shown next to the version number (docs/release.md). English:
/// translated where the interface shows it (`--version` prints it as is).
pub const RELEASE_STAGE: &str = tr_noop!("early alpha");

/// View-state keys, as Animate users know them: `(keys, action)`.
pub const VIEW_SHORTCUTS: &[(&str, &str)] = &[
    ("Ctrl+2", "view.fit"),
    ("Alt+Shift+O", "view.onion"),
    ("Ctrl+Alt+A", "view.select_all_frames"),
    ("Ctrl+Alt+8", "view.align_to_stage"),
    // Keyboard focus between the panels (ADR 0022). F6 alone is Insert Keyframe, as animators
    // expect; Ctrl+F6 is the next-pane key of Windows apps.
    ("Ctrl+F6", "view.next_panel"),
    ("Ctrl+Shift+F6", "view.prev_panel"),
    ("F9", "view.actions"),
    // Find Next and Find Previous, and Window › Movie Explorer, as in Animate (ADR 0058).
    ("F3", "view.find_next"),
    ("Shift+F3", "view.find_prev"),
    ("Alt+F3", "view.movie_explorer"),
    // Brush size, as in paint programs.
    ("[", "view.brush_smaller"),
    ("]", "view.brush_larger"),
];

impl App {
    /// Show the last update's result once the native window begins drawing.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn show_update_outcome_on_start(&mut self) {
        self.updates.startup();
    }

    pub fn new(cc: &eframe::CreationContext<'_>, session: Session, shot: Option<PathBuf>) -> App {
        #[cfg_attr(not(gpu_stage), allow(unused_mut))]
        let mut app = App::with_context(&cc.egui_ctx, session, shot);
        // The GPU stage shares the window's device when it runs on wgpu (ADR 0029).
        #[cfg(gpu_stage)]
        app.stage_cache.gpu.set_window_device(cc.wgpu_render_state.as_ref());
        // The driver's errors and the device's loss are written down instead of closing the
        // window (ADR 0092).
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(rs) = cc.wgpu_render_state.as_ref() {
            crate::gpu_watch::watch(&rs.device, &cc.egui_ctx);
        }
        app
    }

    /// The app on an egui context (the window's, or a headless one in tests).
    pub fn with_context(ctx: &egui::Context, session: Session, shot: Option<PathBuf>) -> App {
        theme::apply(ctx);
        let shot_mode = shot.is_some();
        // Screenshot runs use defaults so automated checks are reproducible.
        let prefs = if shot_mode { prefs::Prefs::default() } else { prefs::Prefs::load() };
        let keymap = keymap::Keymap::new(&prefs.shortcuts);
        let shortcuts = keymap.table();
        let library_sel = crate::library::first_item(&session.doc);
        let (recovery, recovery_err) = autosave::Recovery::start(shot_mode);
        // Text lays out with the fonts on this computer plus the embedded interface font (so new
        // text has the same default everywhere). Scan them now, off the UI thread.
        anicraft_engine::text::Fonts::register(crate::theme::embedded_fonts());
        // (No thread in a browser, ADR 0036: there the library is only the embedded faces and
        // is built on first use.)
        let _ = std::thread::Builder::new().name("anicraft-font-scan".into()).spawn(|| {
            let _ = anicraft_engine::text::Fonts::get();
        });
        let mut session = session;
        // Open Recent is the session's list, kept in the preferences (ADR 0059).
        session.recent = prefs.recent.clone();
        session.brushes = prefs.brushes.clone();
        session.filter_presets = prefs.filter_presets.clone();
        // Whether rulers, grid and guides show and snap carries over between sessions (ADR 0076).
        session.aids = prefs.aids;
        // The editor draws as Animate does: merge drawing, until Object Drawing (J) is switched on.
        session.style.object_drawing = false;
        let mut app = App {
            #[cfg(not(target_arch = "wasm32"))]
            updates: Default::default(),
            s: session,
            tool: Tool::Select,
            view: StageView { zoom: 1.0, pan: egui::Vec2::ZERO, fit: true, fitted: None, clip_to_stage: false },
            stage_cache: stage::Cache::default(),
            drag: Drag::None,
            hint_menu: None,
            playing: false,
            looping: true,
            play_acc: 0.0,
            onion: crate::onion::from_prefs(&prefs.onion),
            onion_ui: crate::onion::OnionUi::default(),
            camera_preview: false,
            last_tool: Tool::Select,
            status: None,
            prompt: None,
            convert: None,
            layer_props: None,
            tl_frame_view_open: false,
            tl_header_spill: 0.0,
            tl_color_open: None,
            convert_last: Default::default(),
            show_about: false,
            show_files: false,
            show_licenses: false,
            shot: shot.map(|path| Shot { path, frames: 0, requested: false, width: None }),
            cursor_stage: None,
            tl_scroll: 0.0,
            tl_cell_w: timeline::CELL_DEFAULT,
            tl_button_zoom: None,
            tl_vscroll: 0.0,
            tl_last_layer: None,
            tl_collapsed: HashSet::new(),
            library_sel,
            library: Default::default(),
            text_edit: None,
            path_pick: None,
            font_families: None,
            polystar: PolyStarOptions::default(),
            envelope_open: false,
            rect_radius: 0.0,
            pen: None,
            pencil: Vec::new(),
            raw_points: serde_json::Value::Null,
            raw_touched: Vec::new(),
            raw_with: Vec::new(),
            raw_anchors: Vec::new(),
            raw_ends: Vec::new(),
            snap_ring: None,
            new_guide: None,
            aids_dialogs: Default::default(),
            pencil_mode: pathtools::PencilMode::default(),
            brush: crate::brushtool::BrushOptions::default(),
            paint_brush: crate::brushtool::BrushOptions::paint_brush(),
            brush_clock: 0.0,
            brush_lib: crate::brush_panel::BrushLibraryView::default(),
            filter_naming: Default::default(),
            brush_samples: Vec::new(),
            pen_pressure: None,
            pen_tilt_seen: false,
            eraser: crate::erasertool::EraserOptions::default(),
            bucket_gap: GapSize::default(),
            align_to_stage: false,
            colour_target: Default::default(),
            swatch_sel: None,
            points: Vec::new(),
            width_sel: None,
            open_fill_swatch: false,
            bone_sel: None,
            bind: crate::bind_tool::BindOptions::default(),
            warp_pin: None,
            motion_anchors: Vec::new(),
            motion_drag: None,
            frame_sel: None,
            tl_drag: timeline::TlDrag::None,
            dock: prefs.workspaces.layout(),
            dock_view: Default::default(),
            manage_workspaces: Default::default(),
            me_prop: anicraft_engine::doc::Property::X,
            me_key: None,
            me_sel: Vec::new(),
            me_sel_at: (0, 0),
            me_shot_drag: false,
            me_drag: None,
            me_vscroll: 0.0,
            me_styles: Vec::new(),
            timeline_focus: false,
            confirm: None,
            skip_confirm: HashSet::new(),
            palette: None,
            plugin_dialog: None,
            plugin_export: None,
            show_plugins: false,
            output_log: Vec::new(),
            find: Default::default(),
            publish: Default::default(),
            explorer: Default::default(),
            actions: crate::actions_panel::State::default(),
            test_movie: None,
            envelope: None,
            ease: None,
            show_shortcuts: false,
            close_dialog: false,
            allow_close: false,
            close_after_save: false,
            prefs,
            show_prefs: false,
            system_fonts: None,
            fonts_rx: None,
            glyph_check: true,
            last_revision: 0,
            title: String::new(),
            keymap,
            shortcuts,
            key_capture: None,
            recovery,
            sound: crate::sound::SoundOut::new(),
            audio_clock: false,
            heard: None,
            last_panel: None,
            stage_nav: Default::default(),
            tl_heard: None,
            tl_refocus: false,
            menu_keys: Default::default(),
            system_look: platform::SystemLook::default(),
            key_script: Default::default(),
            shot_ease_menu: std::cell::Cell::new(None),
            shot_click: std::cell::Cell::new(None),
            shot_step: 0,
            key_script_wait: 0,
            look_probe: None,
            look_asked: None,
            start: None,
            drop: Default::default(),
            thumbs: Default::default(),
            templates: Default::default(),
        };
        app.apply_prefs(ctx);
        app.decode_sounds();
        app.status_ok(tr!("Welcome to AniCraft. Press Enter to play the sample animation."));
        if let Some(e) = recovery_err {
            app.status_err(e);
        }
        crate::plugins_ui::load_at_start(&mut app);
        app
    }

    /// View › Preview Through Camera (ADR 0027). With no camera yet there is nothing to look
    /// through, so it says how to add one.
    pub fn toggle_camera_preview(&mut self) {
        self.camera_preview = !self.camera_preview;
        if self.camera_preview && self.s.timeline().camera_layer().is_none() {
            self.status_ok(tr!("Preview through camera is on; add a camera (the camera button on the timeline, or the Camera tool) to look through it."));
        }
    }

    /// The Undo key as this computer shows it (rebindable; ⌘Z on macOS), for messages that
    /// say how to undo.
    pub fn undo_key(&self) -> String {
        platform::shortcut_label(self.keymap.primary("edit.undo").unwrap_or("Ctrl+Z"))
    }

    pub fn status_ok(&mut self, text: impl Into<String>) {
        self.status = Some(Status { text: text.into(), error: false, at: 0.0 });
    }
    pub fn status_err(&mut self, text: impl Into<String>) {
        self.status = Some(Status { text: text.into(), error: true, at: 0.0 });
    }

    /// Runs an engine command, reporting failures in the status bar.
    /// After a move, selected raw shapes merge where they land (ADR 0031).
    pub fn drop_raw(&mut self) {
        let raw = self.s.selected_elements().iter().any(|(_, e)| matches!(&e.kind, anicraft_engine::doc::ElementKind::Shape(sh) if sh.merge));
        if raw {
            self.run("draw.drop", json!({}));
        }
    }

    /// Runs a drawing tool's command and leaves nothing selected, as Animate's drawing tools
    /// do: the next stroke goes on without a box around the last one (#205, #243). The command
    /// itself selects what it drew, for scripts that style it next.
    pub fn draw(&mut self, id: &str, params: Value) -> Option<Value> {
        let v = self.run(id, params)?;
        let _ = self.s.execute("edit.deselect", &json!({}));
        Some(v)
    }

    pub fn run(&mut self, id: &str, params: Value) -> Option<Value> {
        let result = self.s.execute(id, &params);
        if platform::WEB && id.starts_with("file.") {
            // In a browser, what a save or an export wrote is in memory: hand it to the
            // browser as downloads (ADR 0036).
            platform::deliver(anicraft_engine::vfs::take_written());
        }
        match result {
            Ok(v) => Some(v),
            Err(e) => {
                // The engine's English, translated when a catalog has that exact message.
                self.status_err(crate::i18n::t_dyn(&e.to_string()).into_owned());
                None
            }
        }
    }

    /// Something the system asked of the app outside its windows (ADR 0062): documents open as
    /// a drop on the window does, through the unsaved-changes guard; a quit goes through the
    /// close guard.
    pub fn system_request(&mut self, ctx: &egui::Context, r: platform::SystemRequest) {
        match r {
            platform::SystemRequest::Open(files) => crate::drop::open_from_system(self, &files),
            platform::SystemRequest::Quit => ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Close),
        }
    }

    /// A file the browser handed over: it goes where the command reads it, and the command runs
    /// (through `request`, so opening over unsaved work asks first).
    fn open_picked(&mut self, p: platform::Picked) {
        // Only the name: the browser never says which folder it came from.
        let name: String = std::path::Path::new(&p.name).file_name().map_or_else(|| "file".into(), |n| n.to_string_lossy().chars().take(128).collect());
        let path = std::path::PathBuf::from(&name);
        match p.data.and_then(|data| anicraft_engine::vfs::put(&path, data).map_err(|e| e.to_string())) {
            Ok(()) => self.request(p.cmd, json!({"path": name})),
            Err(e) => self.status_err(tr!("Could not open {name}: {e}", name = name, e = e)),
        }
    }

    // ----- preferences ----------------------------------------------------------------------

    /// Applies scale and fonts from `self.prefs`. System fonts need the font scan; until it
    /// finishes the bundled fonts stay in place, and they are swapped in when it completes.
    pub fn apply_prefs(&mut self, ctx: &egui::Context) {
        // The interface language first: everything drawn from here on is in it (ADR 0040).
        // (`--lang` holds for the run, over the saved choice: `i18n::force`.)
        crate::i18n::set_language(&self.prefs.language);
        ctx.request_repaint();
        ctx.set_zoom_factor(self.prefs.ui_scale);
        theme::apply_palette(ctx, self.prefs.palette(self.system_look));
        crate::onion::refresh_colors(self);
        self.keymap = keymap::Keymap::new(&self.prefs.shortcuts);
        self.shortcuts = self.keymap.table();
        let wants_system = [&self.prefs.ui_font, &self.prefs.data_font].iter().any(|f| matches!(f, prefs::FontChoice::System(_)));
        if wants_system && self.system_fonts.is_none() {
            self.scan_system_fonts();
        }
        let load = |choice: &prefs::FontChoice, sys: Option<&prefs::SystemFonts>| -> Option<theme::UserFont> {
            match (choice, sys) {
                (prefs::FontChoice::System(name), Some(s)) => s.regular(name).map(|(b, i)| (name.clone(), b, i)),
                _ => None,
            }
        };
        let sys = self.system_fonts.clone();
        let ui_font = load(&self.prefs.ui_font, sys.as_deref());
        let data_font = load(&self.prefs.data_font, sys.as_deref());
        if sys.is_some() {
            let missing: Vec<String> = [(&self.prefs.ui_font, ui_font.is_some()), (&self.prefs.data_font, data_font.is_some())]
                .into_iter()
                .filter_map(|(c, ok)| match c {
                    prefs::FontChoice::System(name) if !ok => Some(name.clone()),
                    _ => None,
                })
                .collect();
            for name in missing {
                self.status_err(tr!("Font “{name}” is not installed; using the bundled font", name = name));
            }
        }
        theme::install_fonts(ctx, ui_font.as_ref(), data_font.as_ref());
        self.glyph_check = true;
    }

    /// `--theme` / `--scale`: a palette and interface scale for this run only (clamped like the
    /// preference), not saved.
    pub fn preview_look(&mut self, ctx: &egui::Context, theme: Option<theme::Palette>, scale: Option<f32>) {
        if theme.is_none() && scale.is_none() {
            return;
        }
        let mut p = self.prefs.clone();
        if let Some(t) = theme {
            p.theme = t;
            p.theme_follows_system = Some(false);
        }
        p.ui_scale = scale.unwrap_or(p.ui_scale);
        self.prefs = p.sanitized();
        self.apply_prefs(ctx);
    }

    /// Help › User Guide: the bundled guide in the browser (ADR 0069).
    pub fn open_user_guide(&mut self) {
        match platform::open_user_guide() {
            Ok(()) => self.status_ok(tr!("Opened the user guide in your browser")),
            Err(e) => self.status_err(e),
        }
    }

    pub fn open_prefs(&mut self) {
        self.show_prefs = true;
        self.scan_system_fonts();
    }

    /// Changes preferences: applies them live and saves them.
    pub fn set_prefs(&mut self, ctx: &egui::Context, p: prefs::Prefs) {
        let p = p.sanitized();
        if p == self.prefs {
            return;
        }
        self.prefs = p;
        self.apply_prefs(ctx);
        if self.shot.is_none()
            && let Err(e) = self.prefs.save()
        {
            self.status_err(tr!("Could not save preferences: {e}", e = e));
        }
    }

    /// Presses `keys` once the window has settled, a few frames apart.
    fn script_keys(&mut self, keys: impl IntoIterator<Item = (Key, Modifiers)>) {
        self.key_script.extend(keys);
        self.key_script_wait = 10;
    }

    /// The next scripted key press, once the frames before it have settled.
    fn press_scripted_key(&mut self, ctx: &egui::Context) {
        if self.key_script.is_empty() {
            return;
        }
        ctx.request_repaint();
        if self.key_script_wait > 0 {
            self.key_script_wait -= 1;
            return;
        }
        if let Some((key, modifiers)) = self.key_script.pop_front() {
            ctx.input_mut(|i| i.events.push(egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }));
            self.key_script_wait = 3;
        }
    }

    /// Follows the system's look (ADR 0064, ADR 0077): asks it on a worker now and then (at
    /// start, every few seconds, and when the window comes back to the front), and switches the
    /// palette in the same frame the answer changes, while Preferences says "Follow system".
    fn follow_system_look(&mut self, ctx: &egui::Context) {
        if self.shot.is_some() {
            return;
        }
        if let Some(rx) = &self.look_probe {
            match rx.try_recv() {
                Ok(v) => {
                    self.look_probe = None;
                    self.set_system_look(ctx, v);
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => return,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => self.look_probe = None,
            }
        }
        let (now, came_back) = ctx.input(|i| (i.time, i.events.iter().any(|e| matches!(e, egui::Event::WindowFocused(true)))));
        let due = self.look_asked.is_none_or(|t| now - t >= SYSTEM_LOOK_EVERY || came_back);
        if !due {
            return;
        }
        self.look_asked = Some(now);
        let (tx, rx) = std::sync::mpsc::channel();
        #[cfg(not(target_arch = "wasm32"))]
        let started = {
            let ctx2 = ctx.clone();
            std::thread::Builder::new()
                .name("anicraft-look".into())
                .spawn(move || {
                    let _ = tx.send(platform::system_look());
                    ctx2.request_repaint();
                })
                .is_ok()
        };
        // (No threads in a browser: the media query answers at once.)
        #[cfg(target_arch = "wasm32")]
        let started = tx.send(platform::system_look()).is_ok();
        if started {
            self.look_probe = Some(rx);
        }
        // Ask again later even if nothing else happens.
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(SYSTEM_LOOK_EVERY));
    }

    /// What the system says about its look: the palette follows at once, if it follows.
    pub fn set_system_look(&mut self, ctx: &egui::Context, v: platform::SystemLook) {
        if v == self.system_look {
            return;
        }
        let before = self.prefs.palette(self.system_look);
        self.system_look = v;
        let after = self.prefs.palette(v);
        if after != before {
            theme::apply_palette(ctx, after);
            crate::onion::refresh_colors(self);
            ctx.request_repaint();
        }
    }

    /// Starts scanning installed fonts on a background thread (once).
    pub fn scan_system_fonts(&mut self) {
        if self.system_fonts.is_some() || self.fonts_rx.is_some() {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let spawned = std::thread::Builder::new().name("anicraft-fonts".into()).spawn(move || {
            let _ = tx.send(prefs::SystemFonts::scan());
        });
        if spawned.is_ok() {
            self.fonts_rx = Some(rx);
        }
    }

    fn poll_font_scan(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.fonts_rx else { return };
        match rx.try_recv() {
            Ok(fonts) => {
                self.system_fonts = Some(std::sync::Arc::new(fonts));
                self.fonts_rx = None;
                self.apply_prefs(ctx);
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => ctx.request_repaint_after(std::time::Duration::from_millis(50)),
            Err(std::sync::mpsc::TryRecvError::Disconnected) => self.fonts_rx = None,
        }
    }

    /// `--open` at start-up: shows a panel, for visual checks and documentation screenshots.
    /// Imports a two-second beat made here (decaying 220 Hz pulses every half second) to the
    /// stage, for screenshots; whether it worked.
    fn import_demo_beat(&mut self) -> bool {
        let rate = 22_050u32;
        let beat: Vec<f32> = (0..rate * 2)
            .map(|i| {
                let t = i as f32 / rate as f32;
                let env = (-(t % 0.5) * 9.0).exp() * if ((t / 0.5) as u32).is_multiple_of(2) { 0.9 } else { 0.5 };
                env * (t * 220.0 * std::f32::consts::TAU).sin()
            })
            .collect();
        let path = std::env::temp_dir().join("anicraft-beat.wav");
        if std::fs::write(&path, anicraft_audio::wav::encode(rate, 1, &beat, anicraft_audio::wav::Bits::Int16)).is_err() {
            return false;
        }
        self.perform("file.import", json!({"path": path.display().to_string()}));
        true
    }

    pub fn open_panel(&mut self, which: Option<&str>) {
        use egui::{Key as K, Modifiers as M};
        match which {
            // Keyboard states for screenshots (ADR 0064): keys pressed a few frames apart.
            Some("stagekeys") => self.script_keys([(K::F6, M::COMMAND), (K::Tab, M::NONE), (K::Space, M::NONE), (K::ArrowRight, M::NONE)]),
            Some("menukeys") => self.script_keys([(K::F10, M::NONE), (K::ArrowRight, M::NONE), (K::ArrowDown, M::NONE), (K::ArrowDown, M::NONE)]),
            Some("pluginexport") => {
                // File › Export open, the exporter plug-ins below the built-in formats (run with
                // ANICRAFT_PLUGINS set to the built samples, examples/plugins/dist; ADR 0088).
                // Export is the 13th entry of the File menu (New … Recover Autosaved Documents…,
                // then the submenus); ANICRAFT_SHOT_DOWNS moves along it if that changes.
                let n = std::env::var("ANICRAFT_SHOT_DOWNS").ok().and_then(|v| v.parse().ok()).unwrap_or(13);
                let mut keys = vec![(K::F10, M::NONE)];
                keys.extend(std::iter::repeat_n((K::ArrowDown, M::NONE), n));
                keys.push((K::Enter, M::NONE));
                self.script_keys(keys);
            }
            Some("pluginexportwindow") => {
                if let Some(p) = anicraft_engine::plugin_export::exporters(&self.s).first() {
                    let id = p.id().to_string();
                    crate::plugin_export_ui::open(self, &id);
                }
            }
            Some("palette") => self.palette = Some(dialogs::Palette { query: "key".into(), selected: 0 }),
            Some("shortcuts") => self.show_shortcuts = true,
            Some("lastscene") => {
                let last = self.s.doc.scenes.len().saturating_sub(1);
                self.run("scene.select", json!({"scene": last}));
            }
            #[cfg(not(target_arch = "wasm32"))]
            Some("updates") => self.updates.request(),
            Some("files") => self.show_files = true,
            Some("start") => {
                // The start screen with sample recent files (one pinned, one missing).
                crate::start::open(self);
                if self.shot.is_some() {
                    crate::recent::sample_entries(self);
                }
            }
            Some("templates") => {
                // File › New from Template: the start screen on its templates, with two sample
                // templates of the user's own.
                self.invoke("file.new_from_template");
                crate::templates::sample_user(self);
            }
            Some("savetemplate") => crate::templates::open_save(self),
            Some("droptimeline") => self.drop.preview = Some((vec![PathBuf::from("whoosh.wav")], crate::drop::PreviewAt::Timeline)), // i18n-exempt: a sample file name
            Some("drop") => self.drop.preview = Some((vec![PathBuf::from("sunset.png")], crate::drop::PreviewAt::Stage)), // i18n-exempt: a sample file name
            Some("droplibrary") => {
                self.run("library.new_folder", json!({"name": "Backgrounds"})); // i18n-exempt: a sample folder name
                self.drop.preview = Some((vec![PathBuf::from("sunset.png")], crate::drop::PreviewAt::Library)); // i18n-exempt: a sample file name
            }
            Some("recovery") => {
                // Work left behind by a session that didn't close cleanly (sample entries
                // unless ANICRAFT_RECOVERY_DIR holds real ones).
                if self.recovery.found.is_empty() {
                    self.recovery.found = autosave::sample_entries();
                }
                self.recovery.show = true;
            }
            Some("licenses") => self.show_licenses = true,
            Some("find") => {
                // Find and Replace (ADR 0058): a caption on the stage and the sample's names hold
                // "ball"; the results listed, the first one gone to.
                self.run("text.create", json!({"text": "Ball bounces (ball 1)", "x": 60, "y": 40, "size": 28}));
                self.run("edit.deselect", json!({}));
                self.find.open = true;
                self.find.text = "ball".into();
                self.find.with = "planet".into();
                crate::find_panel::step(self, true);
            }
            Some("publish") | Some("publishreport") => {
                // Publish Settings (ADR 0060) on the sample, saved in a temporary folder: the GIF
                // on and its options showing; or the report of publishing it (the HTML page and
                // a small GIF, the soundtrack skipped: the sample has no sounds).
                let dir = std::env::temp_dir().join("anicraft-publish-shot");
                let _ = std::fs::create_dir_all(&dir);
                self.s.path = Some(dir.join("walk.anicraft"));
                self.run("publish.settings_set", json!({"format": "gif", "enabled": true, "scale": 0.5, "from": 0, "to": 23}));
                self.run("publish.settings_set", json!({"format": "audio", "enabled": true}));
                self.prefs.publish_presets.insert("Web banner".into(), (*self.s.doc.publish).clone()); // i18n-exempt: a sample preset name the user typed
                self.publish.selected = anicraft_engine::doc::publish::PublishFormat::Gif;
                if which == Some("publish") {
                    crate::publish_panel::open(self);
                } else {
                    crate::publish_panel::publish_now(self);
                }
            }
            Some("publishanim") => {
                // Publish Settings with the animated PNG on (ADR 0063): played three times,
                // transparent, the WebP on beside it.
                let dir = std::env::temp_dir().join("anicraft-publish-shot");
                let _ = std::fs::create_dir_all(&dir);
                self.s.path = Some(dir.join("walk.anicraft"));
                self.run("publish.settings_set", json!({"format": "apng", "enabled": true, "loop": 3}));
                self.run("publish.settings_set", json!({"format": "webp", "enabled": true}));
                self.publish.selected = anicraft_engine::doc::publish::PublishFormat::Apng;
                crate::publish_panel::open(self);
            }
            // Dockable panels (ADR 0075): the timeline above the stage, the Output beside it,
            // Colour stacked with Properties and Align floating.
            Some("dock") => {
                use crate::dock::{Panel as P, Side, Target};
                self.dock.move_to(P::Timeline, Target::Split { side: Side::Top, before: None });
                self.dock.move_to(P::Output, Target::Split { side: Side::Top, before: None });
                self.dock.move_to(P::Colour, Target::Stack(P::Properties));
                self.dock.move_to(P::Align, Target::Float(Some([980.0, 420.0])));
                self.dock.top.size = Some(300.0);
                self.dock_changed();
            }
            // Dockable panels, second slice (ADR 0087): the right dock collapsed to icons with
            // Properties open over the stage; Colour and Swatches as two tabs of one floating
            // window (inside the editor, so the screenshot shows it); Properties at the
            // narrowest a side dock goes, with a shape selected.
            Some("dockicons") => {
                self.dock.right.collapsed = true;
                self.dock_view.flyout = Some(crate::dock::Panel::Properties);
                self.dock_changed();
            }
            Some("floattabs") => {
                use crate::dock::{Panel as P, Target};
                self.dock_view.inside = true;
                self.dock.move_to(P::Colour, Target::Float(Some([760.0, 240.0])));
                self.dock.move_to(P::Swatches, Target::Stack(P::Colour));
                self.dock_changed();
            }
            Some("docknarrow") => {
                self.dock.right.size = Some(crate::dock_ui::MIN_SIDE);
                self.dock_view.epoch += 1;
                self.run("layer.select", json!({"index": 0}));
                self.run("edit.select_all", json!({}));
                self.dock_changed();
            }
            Some("workspaces") => {
                self.save_workspace("Inking"); // i18n-exempt: a sample workspace name the user typed
                crate::workspaces_window::open(self);
            }
            Some("animator") => self.switch_workspace("animator"),
            Some("designer") => self.switch_workspace("designer"),
            Some("explorer") => {
                // The Movie Explorer tab (ADR 0058), its first rows open: scene, layers, keyframes
                // and the instances on them, opening into their symbols.
                crate::movie_explorer::open(self);
                crate::movie_explorer::expand_to(self, 3);
            }
            Some("about") => self.show_about = true,
            Some("selectzoom") => {
                // #204: a raw rectangle with a wide outline, selected and zoomed in far past the
                // window: the selection pattern covers all of it, down to the bottom.
                self.run("layer.new", json!({}));
                self.run("draw.rect", json!({"x": 60, "y": 40, "w": 840, "h": 460, "fill": "#e8594f", "stroke": "#7a1f1a", "stroke_width": 24, "object": false}));
                self.run("edit.select_all", json!({}));
                self.tool = Tool::Select;
                self.view.fit = false;
                self.view.zoom = 4.0;
                self.view.pan = egui::vec2(1820.0, 1030.0);
            }
            Some("zoom") => {
                // 300% on the ball: many tiles, edges cross shapes (seam check).
                self.view.fit = false;
                self.view.zoom = 3.0;
                self.view.pan = egui::vec2(1200.0, 300.0);
            }
            Some("pasteboard") | Some("clipped") => {
                // Artwork half off the stage (#245); `clipped`: Clip Content Outside the Stage on
                // (#251). Zoomed out so the pasteboard shows around the stage.
                let w = self.s.doc.stage.width as f32;
                self.run("draw.ellipse", json!({"x": -160.0, "y": 40.0, "w": 260.0, "h": 200.0}));
                self.run("draw.rect", json!({"x": w - 80.0, "y": 260.0, "w": 240.0, "h": 120.0}));
                self.view.fit = false;
                self.view.zoom = 0.55;
                self.view.pan = egui::Vec2::ZERO;
                self.view.clip_to_stage = which == Some("clipped");
            }
            Some("prefs") => {
                self.show_prefs = true;
                self.scan_system_fonts();
            }
            Some("confirm") => self.request("layer.delete", json!({})),
            Some("libraryfolders") => {
                // The document as it opened, every Library folder open and its first sound
                // selected (an imported file's sounds, ADR 0086).
                let folders: Vec<_> = self.s.doc.library_tree.folders.iter().map(|f| f.id).collect();
                self.library.expanded.extend(folders);
                if let Some(sound) = self.s.doc.sounds.first().map(|s| s.id) {
                    self.library_sel = Some(LibraryItem::Sound(sound));
                    self.library.sel = vec![anicraft_engine::doc::LibraryEntry::Item(LibraryItem::Sound(sound))];
                }
            }
            Some(which @ ("library" | "librarymenu")) => {
                // Library folders with an item inside, a copy, the item previewed (ADR 0050).
                let ball = self.s.doc.library.first().map(|s| s.id);
                let folder = self.run("library.new_folder", json!({"name": "Characters"})).and_then(|v| v["folder"].as_u64());
                self.run("library.new_folder", json!({"name": "Props"}));
                if let (Some(ball), Some(f)) = (ball, folder) {
                    self.run("library.duplicate", json!({"symbol": ball.0}));
                    self.run("library.move", json!({"items": [{"symbol": ball.0}], "to": f}));
                    self.library.expanded.insert(anicraft_engine::doc::FolderId(f));
                    self.library_sel = Some(LibraryItem::Symbol(ball));
                    self.library.sel = vec![anicraft_engine::doc::LibraryEntry::Item(LibraryItem::Symbol(ball))];
                }
                if which == "librarymenu" {
                    crate::library::select_unused(self);
                }
            }
            Some("convert") => {
                // Convert to Symbol with folders to choose from, the last one used offered again
                // (ADR 0053).
                let folder = self.run("library.new_folder", json!({"name": "Characters"})).and_then(|v| v["folder"].as_u64());
                self.run("library.new_folder", json!({"name": "Props"}));
                self.run("draw.rect", json!({"x": 820, "y": 120, "w": 140, "h": 90, "fill": "#e0a030", "object": true}));
                self.convert_last = crate::convert_symbol::Last { folder: folder.map(anicraft_engine::doc::FolderId), kind: anicraft_engine::doc::SymbolKind::MovieClip };
                crate::convert_symbol::open(self);
            }
            Some("mixed") => {
                // Two movie clips that differ in blending and filters, selected: those rows say
                // "mixed" (ADR 0053).
                if let Some(ball) = self.s.doc.library.first().map(|s| s.id.0) {
                    self.run("edit.deselect", json!({}));
                    self.run("symbol.set_kind", json!({"symbol": ball, "kind": "movie_clip"}));
                    let id = |v: Option<Value>| v.and_then(|v| v["element"].as_u64()).unwrap_or(0);
                    let a = id(self.run("library.place", json!({"symbol": ball, "x": 760, "y": 180})));
                    let b = id(self.run("library.place", json!({"symbol": ball, "x": 900, "y": 220})));
                    self.run("element.set_blend", json!({"mode": "multiply"}));
                    self.run("element.set_filters", json!({"filters": [{"type": "glow"}]}));
                    self.run("element.select", json!({"ids": [a, b]}));
                }
            }
            Some("colour") => {
                // A row of instances in each colour effect style (ADR 0056), the Advanced one
                // selected: Properties shows the Style menu and its channel rows.
                if let Some(ball) = self.s.doc.library.first().map(|s| s.id.0) {
                    self.run("edit.deselect", json!({}));
                    let id = |v: Option<Value>| v.and_then(|v| v["element"].as_u64()).unwrap_or(0);
                    let styles = [json!({"brightness": 45}), json!({"tint": "#2a9d8f", "tint_amount": 70}), json!({"alpha": 40}), json!({"style": "advanced", "red_multiplier": -100, "red_offset": 255, "green_multiplier": 60, "blue_offset": -40})];
                    for (i, fx) in styles.into_iter().enumerate() {
                        let e = id(self.run("library.place", json!({"symbol": ball, "x": 640 + 130 * i, "y": 470})));
                        self.run("element.select", json!({"ids": [e]}));
                        self.run("element.set_color_effect", fx);
                    }
                }
            }
            Some("bitmap") => {
                // Two placed bitmaps of different images, selected: the Bitmap row with Swap…
                // says "mixed" (ADR 0053).
                let dir = std::env::temp_dir().join("anicraft-shot-bitmaps");
                let _ = std::fs::create_dir_all(&dir);
                let mut ids = Vec::new();
                for (name, w, h, px) in [("checker.png", 8u32, 8u32, [40u8, 120, 220, 255]), ("sunset.png", 12, 6, [230, 120, 40, 255])] {
                    let path = dir.join(name);
                    let Ok(png) = anicraft_engine::render::encode_png(w, h, &px.repeat((w * h) as usize)) else { continue };
                    if std::fs::write(&path, png).is_err() {
                        continue;
                    }
                    if let Some(b) = self.run("file.import_to_library", json!({"path": path.display().to_string(), "name": name})).and_then(|v| v["bitmap"].as_u64()) {
                        let at = 700 + 160 * ids.len();
                        if let Some(e) = self.run("library.place", json!({"bitmap": b, "x": at, "y": 200})).and_then(|v| v["element"].as_u64()) {
                            ids.push(e);
                        }
                    }
                }
                self.run("element.select", json!({"ids": ids}));
            }
            Some("warp") => {
                // Asset Warp (ADR 0079): a checkered bitmap and a striped banner, each pinned at
                // three points and bent at frame 1, bent the other way at frame 24 (a classic
                // tween between), with the tool in hand and the bitmap's right pin picked.
                self.run("layer.new", json!({"name": "Warp"}));
                let dir = std::env::temp_dir().join("anicraft-shot-warp");
                let _ = std::fs::create_dir_all(&dir);
                let (w, h) = (240u32, 150u32);
                let mut rgba = Vec::with_capacity((w * h * 4) as usize);
                for y in 0..h {
                    for x in 0..w {
                        let on = ((x / 30) + (y / 30)) % 2 == 0;
                        let t = x as f32 / w as f32;
                        let c = if on { [(60.0 + 180.0 * t) as u8, 90, (230.0 - 150.0 * t) as u8, 255] } else { [250, 244, 230, 255] };
                        rgba.extend_from_slice(&c);
                    }
                }
                let path = dir.join("checker.png");
                let bitmap = anicraft_engine::render::encode_png(w, h, &rgba).ok().filter(|png| std::fs::write(&path, png).is_ok()).and_then(|_| self.run("file.import_to_library", json!({"path": path.display().to_string(), "name": "checker.png"}))).and_then(|v| v["bitmap"].as_u64());
                let id = |v: Option<Value>, k: &str| v.and_then(|v| v[k].as_u64()).unwrap_or(0);
                let img = bitmap.map_or(0, |b| id(self.run("library.place", json!({"bitmap": b, "x": 380, "y": 330})), "element"));
                let banner = id(self.run("draw.rect", json!({"x": 700, "y": 270, "w": 320, "h": 120, "fill": "#e8833a", "stroke": "#4a2c14", "stroke_width": 3, "object": true})), "element");
                self.run("timeline.insert_frame", json!({"frame": 23}));
                self.run("timeline.goto", json!({"frame": 0}));
                let mut pins = Vec::new();
                for (el, x0, x1, y) in [(img, 270.0, 490.0, 330.0), (banner, 712.0, 1008.0, 330.0)] {
                    let mid = (x0 + x1) * 0.5;
                    let p = [x0, mid, x1].map(|x| id(self.run("warp.pin_add", json!({"element": el, "x": x, "y": y})), "pin"));
                    pins.push((el, p, x1, y));
                }
                self.run("timeline.goto", json!({"frame": 23}));
                self.run("timeline.insert_keyframe", json!({}));
                self.run("timeline.goto", json!({"frame": 0}));
                self.run("timeline.create_tween", json!({}));
                for &(el, p, x1, y) in &pins {
                    self.run("warp.pin_move", json!({"element": el, "pin": p[1], "dx": 0, "dy": -30, "frame": 0}));
                    self.run("warp.pin_move", json!({"element": el, "pin": p[2], "x": x1 - 20.0, "y": y + 80.0, "frame": 0}));
                    self.run("warp.pin_move", json!({"element": el, "pin": p[2], "x": x1 - 10.0, "y": y - 70.0, "frame": 23}));
                }
                self.run("timeline.goto", json!({"frame": 0}));
                self.warp_pin = pins.first().map(|&(el, p, _, _)| (anicraft_engine::doc::ElementId(el), anicraft_engine::doc::warp::PinId(p[2])));
                self.tool = Tool::Warp;
            }
            Some(which @ ("onion" | "onionmenu")) => {
                // Onion skin ±5 on a frame with frames before and after it, the markers on the
                // ruler; with its menu open, in outlines (ADR 0047).
                self.onion.ghosts = true;
                self.onion.preset(5);
                self.run("timeline.goto", json!({"frame": 12}));
                if which == "onionmenu" {
                    self.onion.outlines = true;
                    self.onion_ui.open_menu = true;
                }
            }
            Some("editmultiple") => {
                // Edit Multiple Frames over the sample's walk, everything selected (ADR 0047).
                self.onion.preset(5);
                crate::onion::toggle_edit_multiple(self);
                self.run("timeline.goto", json!({"frame": 12}));
                crate::onion::select_all(self);
            }
            Some("edittransform") => {
                // Free Transform on Edit Multiple Frames' selection (ADR 0051): one box, one
                // pivot, over the artwork on every keyframe between the markers.
                self.onion.preset(5);
                crate::onion::toggle_edit_multiple(self);
                self.run("timeline.goto", json!({"frame": 12}));
                self.tool = Tool::Transform;
                crate::onion::select_all(self);
                // The bouncing ball on its keyframes.
                let top = self.s.timeline().layers.first().map(|l| l.id);
                self.onion_ui.sel.retain(|it| Some(it.layer) == top);
            }
            Some("layerprops") => {
                // Layer Properties (ADR 0061) on the active layer.
                crate::layer_props::open(self, None);
            }
            Some("frameview") => self.tl_frame_view_open = true,
            Some("layercolour") => self.tl_color_open = Some(self.s.layer),
            Some("timelinepolish") => {
                // Per-layer onion toggles, outlines in the layer colour and a taller layer
                // (ADR 0061): onion skin on, the top layer outlined in orange and 200 % tall,
                // the next one left out of onion skin.
                self.onion.ghosts = true;
                let ids: Vec<u64> = self.s.timeline().layers.iter().filter(|l| !matches!(l.kind, anicraft_engine::doc::LayerKind::Folder | anicraft_engine::doc::LayerKind::Camera)).map(|l| l.id.0).collect();
                if let Some(&top) = ids.first() {
                    self.run("layer.set_properties", json!({"layer": top, "outline": true, "color": "#ff8a65", "height": 200}));
                }
                if let Some(&next) = ids.get(1) {
                    self.run("layer.set_onion", json!({"layer": next, "onion": false}));
                }
                self.run("timeline.goto", json!({"frame": 12}));
            }
            Some("onionsymbol") => {
                // Onion skin inside a symbol open in place (ADR 0051): the sample's spinning star,
                // its own frames before and after its playhead around it.
                let star = self.s.timeline().layers.iter().flat_map(|l| l.keyframes.iter().flat_map(|k| k.elements.iter())).find(|e| e.name == "star").map(|e| e.id.0);
                if let Some(id) = star {
                    self.run("element.select", json!({"ids": [id]}));
                    self.run("symbol.edit", json!({}));
                    self.run("timeline.goto", json!({"frame": 12}));
                }
                self.onion.ghosts = true;
                self.onion.preset(5);
            }
            Some("selection") => {
                // A selection across layers (ADR 0044): Select All takes every unlocked layer's
                // artwork; the timeline marks the layers, Properties says how many.
                self.tool = Tool::Select;
                self.run("edit.select_all", json!({}));
            }
            Some("plugins") => {
                // The first generator's dialog, previewing (run with ANICRAFT_PLUGINS set).
                let first = self.s.plugins.list().find(|p| crate::plugins_ui::is_command(p) && !p.manifest().params.is_empty() && p.manifest().selection == anicraft_engine::plugins::SelectionNeed::Optional).map(|p| p.id().to_string());
                match first {
                    Some(id) => crate::plugins_ui::open(self, &id),
                    None => self.show_plugins = true,
                }
            }
            Some("strokes") => {
                // Stroke styles (ADR 0046): a mitred zigzag with square caps, a dotted tapering
                // line, and a dashed oval with bevelled corners, selected, so Properties shows
                // Style with its dash editor, Cap, Join and Miter.
                self.tool = Tool::Select;
                self.run("draw.path", json!({"d": "M 80 640 L 200 520 L 320 640 L 440 520 L 560 640", "fill": null, "stroke": "#1b2a4a", "stroke_width": 14, "cap": "square", "join": "miter", "miter": 4, "object": true}));
                self.run("draw.line", json!({"x0": 80, "y0": 690, "x1": 560, "y1": 690, "stroke": "#e8553b", "stroke_width": 8, "style": "dotted", "dot_spacing": 6, "profile": "taper_end", "object": true}));
                self.run("draw.rect", json!({"x": 640, "y": 500, "w": 300, "h": 180, "fill": "#f3d36b", "stroke": "#3d8bfd", "stroke_width": 6, "cap": "none", "join": "miter", "dash": [18, 8], "object": true}));
            }
            Some("filters") => {
                // A movie clip with a drop shadow switched off, a bevel and a gradient glow,
                // selected, and a filter preset of your own: Properties › Filters shows the list
                // with its grips and switches (ADR 0091), and the glow's ramp editor.
                self.run("draw.rect", json!({"x": 960, "y": 470, "w": 200, "h": 130, "fill": "#e8553b", "stroke": null}));
                self.run("modify.convert_to_symbol", json!({"name": "Badge"}));
                self.run("symbol.set_kind", json!({"kind": "movie_clip"}));
                self.run(
                    "element.set_filters",
                    json!({"filters": [
                        {"type": "drop_shadow", "blur_x": 10, "blur_y": 10, "distance": 8, "enabled": false},
                        {"type": "bevel", "blur_x": 8, "blur_y": 8, "distance": 6, "strength": 1.2, "quality": 2},
                        {"type": "gradient_glow", "blur_x": 28, "blur_y": 28, "quality": 2, "distance": 0, "stops": [{"offset": 0, "color": "#ffffff00"}, {"offset": 0.55, "color": "#ffd36b"}, {"offset": 1, "color": "#ff4d3d"}]},
                    ]}),
                );
                self.run("filter.preset_save", json!({"name": "Warm badge"})); // i18n-exempt: a sample preset name
                // Room for the whole list (this run only: screenshot runs never save the layout).
                for p in [crate::dock::Panel::Align, crate::dock::Panel::Library, crate::dock::Panel::MovieExplorer, crate::dock::Panel::History] {
                    self.dock.close(p);
                }
            }
            Some("testmovieinside") => {
                // Test Movie floating inside the editor (ADR 0062), as where there can't be a
                // window of its own; for this run only (screenshot runs never save preferences).
                self.prefs.test_movie_window = false;
                self.open_panel(Some("testmovie"));
            }
            Some(which @ ("actions" | "testmovie")) => {
                // The interactive sample: its Play button selected with the Actions panel on
                // its events, or playing in Test Movie with the pointer over the button.
                self.perform("file.new_sample", json!({"which": "interactive"}));
                self.run("layer.select", json!({"index": 1}));
                if which == "actions" {
                    if let Some(id) = self.s.visible_elements().iter().find(|e| e.name == "play_btn").map(|e| e.id.0) {
                        self.run("element.select", json!({"ids": [id]}));
                    }
                    crate::actions_panel::open(self, crate::actions_panel::Tab::Instance);
                } else {
                    self.run("layer.select", json!({"index": 0}));
                    crate::actions_panel::open(self, crate::actions_panel::Tab::Frame);
                    crate::test_movie::open(self);
                    crate::test_movie::demo_pointer(self);
                }
            }
            Some(which @ ("button" | "testhits")) => {
                // Buttons (ADR 0057): the sample's Play button open in place on its Over frame,
                // a click sound on it (made here, a short decaying tone), its states named on
                // the timeline and its Hit area over the artwork; or Test Movie with Show Hit
                // Areas on.
                self.perform("file.new_sample", json!({"which": "interactive"}));
                self.run("layer.select", json!({"index": 1}));
                if which == "testhits" {
                    crate::test_movie::open(self);
                    crate::test_movie::demo_pointer(self);
                    if let Some(tm) = self.test_movie.as_mut() {
                        tm.hit_areas = true;
                    }
                } else if let Some(id) = self.s.visible_elements().iter().find(|e| e.name == "play_btn").map(|e| e.id.0) {
                    self.run("symbol.edit", json!({"id": id}));
                    let tone: Vec<f32> = (0..4410).map(|i| (i as f32 * 0.12).sin() * 0.6 * (-(i as f32) / 900.0).exp()).collect();
                    let dir = std::env::temp_dir().join("anicraft-demo");
                    let wav = dir.join("click.wav");
                    let _ = std::fs::create_dir_all(&dir);
                    if std::fs::write(&wav, anicraft_engine::doc::audio::wav::encode(44_100, 1, &tone, anicraft_engine::doc::audio::wav::Bits::Int16)).is_ok()
                        && let Some(v) = self.run("file.import_to_library", json!({"path": wav.display().to_string()}))
                    {
                        self.run("sound.place", json!({"sound": v["sound"], "frame": 1}));
                    }
                    self.run("timeline.goto", json!({"frame": 1}));
                }
            }
            Some(which @ ("script" | "scriptproblems" | "testscript")) => {
                // The interactive sample as an imported Animate document might bring it: a
                // frame script kept on the Actions layer's first keyframe (in place of its
                // Stop action), a linkage class and a document class; the Actions panel shows
                // the Script view (ADR 0013). `script` runs (tiers 2 and 2.5: a typed variable,
                // `if`, a keyboard listener); `scriptproblems` uses what the subset doesn't, and
                // shows where; `testscript` plays the one that runs in Test Movie, its Play
                // button clicked.
                self.perform("file.new_sample", json!({"which": "interactive"}));
                self.run("layer.select", json!({"index": 0}));
                // i18n-exempt: script source, document content
                let runs = "import flash.events.*;\n\nstop();\nvar clicks:int = 0;\nplay_btn.addEventListener(MouseEvent.CLICK, onPlay);\nstage.addEventListener(KeyboardEvent.KEY_DOWN, onKey);\n\nfunction onPlay(event:MouseEvent):void {\n\tclicks++;\n\ttrace(\"Play clicked \" + clicks + (clicks == 1 ? \" time\" : \" times\"));\n\tgotoAndPlay(\"show\");\n}\n\nfunction onKey(event:KeyboardEvent):void {\n\tif (event.keyCode == Keyboard.SPACE) {\n\t\tgotoAndPlay(\"show\");\n\t}\n}\n";
                // i18n-exempt: script source, document content
                let problems = "import flash.events.MouseEvent;\n\nstop();\nvar labels:Array = [\"show\", \"menu\"];\nplay_btn.addEventListener(MouseEvent.CLICK, onPlay);\n\nfunction onPlay(event:MouseEvent):void {\n\tswitch (labels.length) {\n\t\tcase 2: gotoAndPlay(labels[0]);\n\t}\n}\n";
                let code = if which == "scriptproblems" { problems } else { runs };
                self.run("script.set", json!({"frame": 0, "text": code, "language": "actionscript3"}));
                self.run("script.set_class", json!({"class": "com.example.Menu"}));
                if let Some(button) = self.s.doc.library.iter().find(|s| s.kind == anicraft_engine::doc::SymbolKind::Button).map(|s| s.id.0) {
                    self.run("script.set_class", json!({"symbol": button, "class": "com.example.PlayButton", "base": "flash.display.SimpleButton"}));
                }
                crate::actions_panel::open(self, crate::actions_panel::Tab::Frame);
                if which == "testscript" {
                    crate::test_movie::open(self);
                    crate::test_movie::demo_pointer(self);
                }
            }
            Some(which @ ("softmask" | "pathtext")) => {
                // Soft masks and text on a path (ADR 0070): colour bands seen through a radial
                // gradient on an alpha mask layer, and a line of text along an oval. `softmask`
                // opens the mask layer's Layer Properties; `pathtext` selects the text.
                self.perform("file.new", json!({"width": 640, "height": 400}));
                self.run("draw.rect", json!({"x": 0, "y": 0, "w": 640, "h": 400, "fill": "#1d3557", "stroke": null}));
                self.run("layer.new", json!({"name": "Bands"}));
                for (i, c) in ["#e76f51", "#f4a261", "#e9c46a", "#2a9d8f", "#8ab17d"].iter().enumerate() {
                    self.run("draw.rect", json!({"x": 120 + 80 * i, "y": 40, "w": 80, "h": 320, "fill": c, "stroke": null}));
                }
                let mask = self.run("layer.new", json!({"name": "Soft mask"})).and_then(|v| v["layer"].as_u64());
                let stops = vec![anicraft_engine::doc::GradientStop { offset: 0.0, color: anicraft_engine::doc::Rgba::WHITE }, anicraft_engine::doc::GradientStop { offset: 1.0, color: anicraft_engine::doc::Rgba::WHITE.with_alpha(0.0) }];
                let swatch = self.s.style.fill_paint.replace(anicraft_engine::doc::Paint::radial(anicraft_engine::geom::Vec2::new(320.0, 200.0), 190.0, stops));
                self.run("draw.ellipse", json!({"x": 130, "y": 10, "w": 380, "h": 380, "stroke": null}));
                self.s.style.fill_paint = swatch;
                self.run("layer.toggle_mask", json!({}));
                self.run("layer.set_mask", json!({"mask": "alpha"}));
                self.run("layer.new", json!({"name": "Title"}));
                // i18n-exempt: document content
                let text = self.run("text.create", json!({"text": "Soft masks · text on a path", "x": 40, "y": 40, "size": 26, "color": "#f1faee", "align": "center"})).and_then(|v| v["element"].as_u64());
                let oval = self.run("draw.ellipse", json!({"x": 70, "y": 25, "w": 500, "h": 350, "fill": null, "stroke": "#f1faee", "stroke_width": 1})).and_then(|v| v["element"].as_u64());
                if let (Some(t), Some(o)) = (text, oval) {
                    self.run("element.select", json!({"ids": [t]}));
                    self.run("text.attach_path", json!({"path": o, "offset": 1080}));
                    self.run("element.select", json!({"ids": [o]}));
                    self.run("edit.delete", json!({}));
                    self.run("element.select", json!({"ids": [t]}));
                }
                if which == "softmask"
                    && let Some(m) = mask
                {
                    crate::layer_props::open(self, Some(anicraft_engine::doc::LayerId(m)));
                }
            }
            Some(which @ ("textfields" | "testtext")) => {
                // Dynamic and input text fields (ADR 0066): a score a script sets, a name field
                // and a multiline notes field. `textfields` selects the name field for its
                // Properties; `testtext` plays it in Test Movie with a name typed and Enter
                // pressed, the notes field focused with a word selected.
                self.perform("file.new", json!({"width": 640, "height": 400}));
                // i18n-exempt: document content
                let fields = [("Score: 0", "dynamic", "score_txt", 40.0, 40.0, 30.0, 360.0, 44.0, "single"), ("", "input", "name_txt", 40.0, 130.0, 22.0, 360.0, 34.0, "single"), ("Notes wrap at the width of the field, and viewers can type more.", "input", "notes_txt", 40.0, 200.0, 18.0, 360.0, 110.0, "multiline")];
                for (text, kind, name, x, y, size, w, h, lines) in fields {
                    self.run("text.create", json!({"text": text, "type": kind, "x": x, "y": y, "size": size, "color": "#264653"}));
                    self.run("instance.set_name", json!({"name": name}));
                    self.run("text.field", json!({"width": w, "height": h, "lines": lines, "border": kind == "input", "background": kind == "input", "max_chars": if name == "name_txt" { 24 } else { 0 }}));
                }
                // i18n-exempt: script source, document content
                let code = "stop();\nscore_txt.text = \"Type your name, then press Enter\";\nstage.addEventListener(KeyboardEvent.KEY_DOWN, onKey);\n\nfunction onKey(e:KeyboardEvent):void {\n\tif (e.keyCode == Keyboard.ENTER && name_txt.length > 0) {\n\t\tscore_txt.text = \"Hello, \" + name_txt.text + \"!\";\n\t}\n}\n";
                self.run("layer.new", json!({}));
                self.run("script.set", json!({"frame": 0, "text": code, "language": "actionscript3"}));
                if which == "textfields" {
                    self.run("layer.select", json!({"index": 1}));
                    if let Some(id) = self.s.visible_elements().iter().find(|e| e.name == "name_txt").map(|e| e.id.0) {
                        self.run("element.select", json!({"ids": [id]}));
                    }
                } else {
                    // Floating inside the editor, so the screenshot shows it (this run only).
                    self.prefs.test_movie_window = false;
                    crate::test_movie::open(self);
                    crate::test_movie::demo_typing(self);
                }
            }
            Some("exporthtml") => {
                // File › Export › Export HTML Canvas… on the interactive sample (the in-app
                // prompt: screenshots never open native dialogs).
                self.perform("file.new_sample", json!({"which": "interactive"}));
                self.invoke("file.export_html");
            }
            Some("select") => {
                // The top layer's artwork selected, for its Properties.
                self.run("layer.select", json!({"index": 0}));
                self.run("edit.select_all", json!({}));
            }
            Some("transform") => {
                self.tool = Tool::Transform;
                self.run("layer.select", json!({"index": 0}));
                self.run("edit.select_all", json!({}));
            }
            Some("points") => {
                // A pencil stroke and a pen path being edited, with one smooth point selected.
                let wave: Vec<[f32; 2]> = (0..=240).map(|i| [140.0 + i as f32 * 1.5, 330.0 + (i as f32 * 0.06).sin() * 28.0]).collect();
                self.run("draw.freehand", json!({"points": wave, "stroke_width": 3}));
                if let Some(v) = self.run("draw.path", json!({"d": "M 520 90 L 700 120 L 660 260 L 540 230 Z"})) {
                    let id = anicraft_engine::doc::ElementId(v["element"].as_u64().unwrap_or(0));
                    self.run("path.convert_point", json!({"id": id.0, "cmd": 1, "smooth": true}));
                    self.points = vec![(id, 1, anicraft_engine::geom::edit::ANCHOR)];
                }
                self.tool = Tool::Subselect;
            }
            Some("merge") => {
                // Merge drawing (ADR 0031) on a new layer: a rectangle and an oval that cut each
                // other, a line through both, and the oval's fill with its outline selected.
                self.run("layer.new", json!({}));
                for (id, p) in [
                    ("draw.rect", json!({"x": 180, "y": 120, "w": 300, "h": 200, "fill": "#f2c14e", "stroke": "#1b2a4a", "stroke_width": 3})),
                    ("draw.ellipse", json!({"x": 360, "y": 210, "w": 240, "h": 240, "fill": "#3d8bfd", "stroke": "#1b2a4a", "stroke_width": 3})),
                    ("draw.line", json!({"x0": 140, "y0": 420, "x1": 680, "y1": 140, "stroke": "#d6336c", "stroke_width": 3})),
                ] {
                    let mut p = p;
                    p["object"] = json!(false);
                    self.run(id, p);
                }
                self.run("element.select_connected", json!({"x": 520, "y": 400}));
                self.tool = Tool::Select;
            }
            Some(which @ ("rawparts" | "rawedit")) => {
                // The Selection tool on raw shapes (ADR 0041) on a new layer: a rectangle with its
                // outline, an oval and a line that cut each other.
                self.run("layer.new", json!({}));
                for (id, p) in [
                    ("draw.rect", json!({"x": 180, "y": 120, "w": 300, "h": 200, "fill": "#f2c14e", "stroke": "#1b2a4a", "stroke_width": 3})),
                    ("draw.ellipse", json!({"x": 360, "y": 210, "w": 240, "h": 240, "fill": "#3d8bfd", "stroke": "#1b2a4a", "stroke_width": 3})),
                    ("draw.line", json!({"x0": 140, "y0": 420, "x1": 680, "y1": 140, "stroke": "#d6336c", "stroke_width": 3})),
                ] {
                    let mut p = p;
                    p["object"] = json!(false);
                    self.run(id, p);
                }
                self.run("edit.deselect", json!({}));
                if which == "rawparts" {
                    // A marquee over the middle: the parts under it are selected, cut at its edges.
                    self.run("raw.select_rect", json!({"x0": 300, "y0": 170, "x1": 470, "y1": 330}));
                } else {
                    // The top edge bent upwards, the bottom-left corner pulled out, and the
                    // rectangle's top-right corner being dragged onto the line (the snap ring).
                    self.run("path.bend_segment", json!({"x": 260, "y": 120, "dx": 0, "dy": -60}));
                    self.run("raw.move_point", json!({"x": 180, "y": 320, "to_x": 130, "to_y": 350}));
                    self.s.begin_gesture(tr_noop!("Move Point"));
                    let r = self.run("raw.move_point", json!({"x": 480, "y": 120, "to_x": 520, "to_y": 214, "snap": 10, "merge": false}));
                    let f = |k: &str| r.as_ref().and_then(|v| v.get(k)).and_then(Value::as_f64).unwrap_or(0.0) as f32;
                    self.raw_points = r.as_ref().and_then(|v| v.get("points")).cloned().unwrap_or(Value::Null);
                    self.snap_ring = r.as_ref().and_then(|v| v.get("snapped")).filter(|v| v.is_object()).map(|_| anicraft_engine::geom::Vec2::new(f("x"), f("y")));
                    self.drag = Drag::RawPoint { at: egui::pos2(f("x"), f("y")), off: egui::Vec2::ZERO };
                }
                self.tool = Tool::Select;
            }
            Some("rawsub") => {
                // Subselection on raw shapes (ADR 0042) on a new layer: an outlined square, an
                // oval and a line that cut each other, merged. A click on the oval shows the
                // points of everything connected; its right-hand point is selected, so its curve
                // handles show, and one handle is being dragged (both shapes on that edge bend).
                self.run("layer.new", json!({}));
                for (id, p) in [
                    ("draw.rect", json!({"x": 180, "y": 120, "w": 300, "h": 200, "fill": "#f2c14e", "stroke": "#1b2a4a", "stroke_width": 3})),
                    ("draw.ellipse", json!({"x": 360, "y": 210, "w": 240, "h": 240, "fill": "#3d8bfd", "stroke": "#1b2a4a", "stroke_width": 3})),
                    ("draw.line", json!({"x0": 140, "y0": 420, "x1": 680, "y1": 140, "stroke": "#d6336c", "stroke_width": 3})),
                ] {
                    let mut p = p;
                    p["object"] = json!(false);
                    self.run(id, p);
                }
                self.run("edit.deselect", json!({}));
                self.tool = Tool::Subselect;
                let v = self.run("raw.subselect", json!({"x": 480, "y": 450, "slop": 4}));
                self.raw_ends = Vec::new();
                let anchor = anicraft_engine::geom::Vec2::new(600.0, 330.0);
                self.raw_anchors = vec![anchor];
                let pts = self.s.raw_points(&self.s.selection.on(self.s.layer));
                if let (Some(_), Some(h)) = (v, pts.handles.iter().find(|h| h.anchor.distance(anchor) < 0.5 && h.at.y > anchor.y)) {
                    self.s.begin_gesture(tr_noop!("Move Handle"));
                    let r = self.run("raw.move_handle", json!({"id": h.id.0, "cmd": h.cmd, "slot": h.slot, "dx": 40, "dy": 10, "merge": false}));
                    self.raw_touched = r.as_ref().map(|v| v["changed"].as_array().into_iter().flatten().filter_map(Value::as_u64).collect()).unwrap_or_default();
                    self.drag = Drag::RawHandle { id: h.id, cmd: h.cmd, slot: h.slot, last: egui::pos2(h.at.x + 40.0, h.at.y + 10.0) };
                }
            }
            Some("rawpoints") => {
                // Raw points (ADR 0045) on a new layer: an outlined square whose top-left corner
                // was deleted (the fill and its outline joined across the diagonal), and a point
                // added on its right side, pulled out and made smooth, selected with its handle
                // pair showing.
                self.run("layer.new", json!({}));
                self.run("draw.rect", json!({"x": 240, "y": 140, "w": 320, "h": 260, "fill": "#f2c14e", "stroke": "#1b2a4a", "stroke_width": 3, "object": false}));
                self.run("draw.ellipse", json!({"x": 640, "y": 170, "w": 240, "h": 200, "fill": "#3d8bfd", "stroke": "#1b2a4a", "stroke_width": 3, "object": false}));
                self.run("edit.deselect", json!({}));
                self.run("raw.delete_points", json!({"points": [[240, 140]]}));
                self.run("raw.add_point", json!({"x": 560, "y": 270}));
                self.run("raw.move_point", json!({"x": 560, "y": 270, "to_x": 620, "to_y": 270}));
                self.run("raw.convert_points", json!({"points": [[620, 270]], "smooth": true}));
                self.tool = Tool::Subselect;
                self.run("raw.subselect", json!({"x": 400, "y": 400, "slop": 4}));
                self.raw_ends = Vec::new();
                self.raw_anchors = vec![anicraft_engine::geom::Vec2::new(620.0, 270.0)];
            }
            Some("rawsnap") => {
                // Snapping while drawing (ADR 0045): a line being drawn from a corner of a raw
                // square, its end snapping onto the square's other corner (the ring).
                self.run("layer.new", json!({}));
                self.run("draw.rect", json!({"x": 240, "y": 140, "w": 320, "h": 260, "fill": "#f2c14e", "stroke": "#1b2a4a", "stroke_width": 3, "object": false}));
                self.run("edit.deselect", json!({}));
                self.tool = Tool::Line;
                self.drag = Drag::Shape { start: egui::pos2(240.0, 400.0), cur: egui::pos2(560.0, 140.0) };
                self.snap_ring = Some(anicraft_engine::geom::Vec2::new(560.0, 140.0));
            }
            Some("rulers") | Some("editguides") | Some("editgrid") => {
                // Rulers, grid and guides (ADR 0076): a rectangle being drawn from one guide
                // crossing to a grid crossing, the snap ring on its corner; `editguides` and
                // `editgrid`: their dialogs.
                self.run("view.rulers", json!({"on": true}));
                self.run("view.grid_set", json!({"spacing": 40, "show": true, "snap": true, "over_objects": true}));
                for (axis, at) in [("vertical", 320), ("vertical", 960), ("horizontal", 160), ("horizontal", 560)] {
                    self.run("view.guide_add", json!({"axis": axis, "at": at}));
                }
                self.run("layer.new", json!({}));
                self.run("draw.rect", json!({"x": 320, "y": 160, "w": 240, "h": 160, "fill": "#f2c14e", "stroke": "#1b2a4a", "stroke_width": 3, "object": true}));
                self.run("edit.deselect", json!({}));
                self.tool = Tool::Rect;
                self.drag = Drag::Shape { start: egui::pos2(960.0, 160.0), cur: egui::pos2(1120.0, 400.0) };
                self.snap_ring = Some(anicraft_engine::geom::Vec2::new(1120.0, 400.0));
                match which {
                    Some("editguides") => crate::aids_dialog::open_guides(self, true),
                    Some("editgrid") => crate::aids_dialog::open_grid(self),
                    _ => {}
                }
            }
            Some("symboledit") => {
                // A symbol open for editing in place (ADR 0045): a raw drawing inside it, merged
                // with a line drawn across it there, its connected points showing under the
                // Subselection tool; the stage around it dimmed, the way back above.
                self.run("layer.new", json!({}));
                self.run("draw.rect", json!({"x": 260, "y": 160, "w": 300, "h": 220, "fill": "#e8553b", "stroke": "#1b2a4a", "stroke_width": 3, "object": false}));
                self.run("modify.convert_to_symbol", json!({"name": "Badge"}));
                self.run("symbol.edit", json!({}));
                self.run("draw.line", json!({"x0": -190, "y0": 40, "x1": 190, "y1": -40, "stroke": "#1b2a4a", "stroke_width": 3, "object": false}));
                self.run("draw.ellipse", json!({"x": 60, "y": -150, "w": 140, "h": 140, "fill": "#f2c14e", "stroke": "#1b2a4a", "stroke_width": 3, "object": false}));
                self.run("edit.deselect", json!({}));
                self.tool = Tool::Subselect;
                self.run("raw.subselect", json!({"x": 0, "y": 60, "slop": 4}));
                self.raw_ends = Vec::new();
            }
            Some("groups") => {
                // Groups (ADR 0073): a badge of raw shapes and a drawing object grouped, the group
                // grouped again with a star, moved; the inner group open in place, a line just
                // drawn across its raw drawing (it cut only the group's own fill), the stage and the
                // outer group dimmed, the way back above (Scene 1 › Group › Group).
                self.run("layer.new", json!({"name": "Badges"}));
                self.run("draw.rect", json!({"x": 300, "y": 180, "w": 260, "h": 200, "radius": 30, "fill": "#e8553b", "stroke": "#1b2a4a", "stroke_width": 3, "object": false}));
                self.run("draw.ellipse", json!({"x": 380, "y": 220, "w": 100, "h": 100, "fill": "#f2c14e", "stroke": "#1b2a4a", "stroke_width": 3, "object": true}));
                let mine: Vec<u64> = self.s.visible_elements().iter().map(|e| e.id.0).collect();
                self.run("element.select", json!({"ids": mine}));
                self.run("modify.group", json!({}));
                let inner = self.s.selection.id_list().first().copied();
                self.run("draw.star", json!({"cx": 660, "cy": 280, "outer": 70, "inner": 32, "points": 5, "fill": "#3d8bfd", "stroke": "#1b2a4a", "stroke_width": 3, "object": true}));
                let mine: Vec<u64> = self.s.visible_elements().iter().map(|e| e.id.0).collect();
                self.run("element.select", json!({"ids": mine}));
                self.run("modify.group", json!({}));
                self.run("element.translate", json!({"dx": 40, "dy": 40}));
                self.run("symbol.edit", json!({}));
                if let Some(id) = inner {
                    self.run("symbol.edit", json!({"id": id}));
                }
                self.run("draw.line", json!({"x0": 280, "y0": 300, "x1": 580, "y1": 260, "stroke": "#1b2a4a", "stroke_width": 3, "object": false}));
                self.tool = Tool::Select;
            }
            Some("combine") => {
                // Combine Objects (ADR 0073): the same two drawing objects (a red square under a
                // blue circle) combined four ways, left to right: Union (a group of the merged
                // pieces), Intersect (the overlap, blue), Punch (the square, a bite out of it) and
                // Crop (the part of the square inside the circle). Punch's result is selected.
                self.run("layer.new", json!({"name": "Combined"}));
                let mut punched = None;
                for (i, op) in ["union", "intersect", "punch", "crop"].iter().enumerate() {
                    let x = 120.0 + 260.0 * i as f32;
                    self.run("edit.deselect", json!({}));
                    let a = self.run("draw.rect", json!({"x": x, "y": 260, "w": 140, "h": 140, "fill": "#e8553b", "stroke": null, "object": true}));
                    let b = self.run("draw.ellipse", json!({"x": x + 70.0, "y": 200, "w": 120, "h": 120, "fill": "#3d8bfd", "stroke": null, "object": true}));
                    let ids: Vec<u64> = [a, b].iter().flatten().filter_map(|v| v["element"].as_u64()).collect();
                    self.run("element.select", json!({"ids": ids}));
                    let v = self.run("modify.combine", json!({"op": op}));
                    if *op == "punch" {
                        punched = v.and_then(|v| v["created"].as_array().and_then(|a| a.first()).and_then(serde_json::Value::as_u64));
                    }
                }
                self.tool = Tool::Select;
                match punched {
                    Some(id) => {
                        self.run("element.select", json!({"ids": [id]}));
                    }
                    None => {
                        self.run("edit.deselect", json!({}));
                    }
                }
            }
            Some(which @ ("nineslice" | "nineslicedit")) => {
                // 9-slice scaling (ADR 0055): a rounded panel symbol with its guides on, one
                // instance as drawn and one stretched wide (its corners stay round), the wide one
                // selected; "nineslicedit" opens the symbol with its guides on the stage.
                self.run("layer.new", json!({"name": "Panels"}));
                self.run("draw.rect", json!({"x": 80, "y": 560, "w": 200, "h": 80, "radius": 24, "fill": "#2a9d8f", "stroke": "#264653", "stroke_width": 4}));
                self.run("modify.convert_to_symbol", json!({"name": "Panel"}));
                self.run("symbol.set_kind", json!({"kind": "movie_clip"}));
                self.run("symbol.set_scale_grid", json!({"left": -70, "right": 70, "top": -14, "bottom": 14}));
                self.run("edit.duplicate", json!({"dx": 260, "dy": 0}));
                // Stretched about its left edge (x = 340): 2.9 times as wide, 1.4 times as high.
                self.run("modify.transform", json!({"a": 2.9, "b": 0, "c": 0, "d": 1.4, "e": 340.0 * -1.9, "f": 600.0 * -0.4}));
                if which == "nineslicedit" {
                    // The instance as drawn opens in place, the scene dimmed around it.
                    self.run("element.pick", json!({"x": 180, "y": 600}));
                    self.run("symbol.edit", json!({}));
                    self.run("edit.deselect", json!({}));
                    self.tool = Tool::Select;
                }
            }
            Some(which @ ("brushlibrary" | "brushcurve" | "paintbrush")) => {
                // The brush library (ADR 0068): a page with a stroke of every built-in brush
                // (pressure swelling in the middle), each named, and one of the user's own. The
                // library open on it ("brushlibrary"), the pressure curve editor in Properties
                // ("brushcurve"), or Paint Brush lines of varying width ("paintbrush").
                self.run("layer.new", json!({"name": "Brushes"}));
                self.run("draw.object_drawing", json!({"on": true}));
                self.run("brush.preset_save", json!({"name": "Wet Ink", "from": "ink_pen", "size": 9, "curve": [[0, 0], [0.4, 0.75], [1, 1]]})); // i18n-exempt: a sample brush name
                let wave = |y: f32, x0: f32, len: f32| -> Vec<[f32; 3]> {
                    (0..=60)
                        .map(|i| {
                            let t = i as f32 / 60.0;
                            [x0 + t * len, y + (t * std::f32::consts::TAU).sin() * 14.0, 0.25 + 0.75 * (t * std::f32::consts::PI).sin()]
                        })
                        .collect()
                };
                let mut names: Vec<String> = anicraft_engine::brush_library::BUILTIN.iter().map(|(k, _)| (*k).to_string()).collect();
                names.push("Wet Ink".into()); // i18n-exempt: a sample brush name
                let as_stroke = which == "paintbrush";
                for (i, name) in names.iter().enumerate() {
                    let y = 70.0 + i as f32 * 78.0;
                    self.run("draw.brush", json!({"points": wave(y, 330.0, 560.0), "preset": name, "color": "#1b2a4a", "as_stroke": as_stroke}));
                    let label = anicraft_engine::brush_library::BUILTIN.iter().find(|(k, _)| k == name).map_or(name.as_str(), |(_, n)| n);
                    self.run("text.create", json!({"x": 60, "y": y - 14.0, "text": label, "size": 22, "color": "#3a4150"}));
                }
                self.run("edit.deselect", json!({}));
                if as_stroke {
                    self.tool = Tool::Width;
                } else {
                    self.tool = Tool::Brush;
                    if let Some(p) = anicraft_engine::brush_library::builtin("ink_pen") {
                        crate::brush_panel::choose(self, &p);
                    }
                    self.brush.curve = anicraft_engine::geom::pressure::PressureCurve::new(&[[0.0, 0.0], [0.35, 0.62], [0.7, 0.8], [1.0, 1.0]]);
                    self.brush_lib.open = which == "brushlibrary";
                    if which == "brushlibrary"
                        && let Some(p) = anicraft_engine::brush_library::builtin("chalk")
                    {
                        crate::brush_panel::choose(self, &p);
                    }
                }
            }
            Some(which @ ("brush" | "brushing")) => {
                // Brushed artwork on a new layer (ADR 0033): a pressure-tapered wave, a flat
                // calligraphic tip at 45°, square dabs, and a pink stroke painted behind a raw
                // oval; "brushing" also has a stroke in progress (the live preview).
                self.run("layer.new", json!({"name": "Paint"}));
                self.run("draw.object_drawing", json!({"on": false}));
                let wave: Vec<[f32; 3]> = (0..=120)
                    .map(|i| {
                        let t = i as f32 / 120.0;
                        [120.0 + t * 520.0, 470.0 + (t * 9.0).sin() * 46.0, (t * std::f32::consts::PI).sin()]
                    })
                    .collect();
                self.run("draw.brush", json!({"points": wave, "size": 34, "color": "#1b2a4a"}));
                let loops: Vec<[f32; 2]> = (0..=160)
                    .map(|i| {
                        let t = i as f32 / 160.0 * std::f32::consts::TAU * 2.0;
                        [190.0 + i as f32 * 2.6 + 34.0 * t.cos(), 170.0 + 40.0 * t.sin()]
                    })
                    .collect();
                self.run("draw.brush", json!({"points": loops, "size": 22, "roundness": 25, "angle": 45, "color": "#e8553b"}));
                for (k, x) in [700.0f32, 740.0, 780.0].into_iter().enumerate() {
                    self.run("draw.brush", json!({"points": [[x, 120.0 + k as f32 * 18.0]], "size": 24, "shape": "square", "angle": 15.0 * k as f32, "color": "#3d8bfd"}));
                }
                self.run("draw.ellipse", json!({"x": 820, "y": 380, "w": 220, "h": 160, "fill": "#f2c14e", "stroke": "#1b2a4a", "stroke_width": 3}));
                let behind: Vec<[f32; 2]> = (0..=60).map(|i| [760.0 + i as f32 * 6.0, 560.0 - i as f32 * 3.4]).collect();
                self.run("draw.brush", json!({"points": behind, "size": 40, "mode": "behind", "color": "#d6336c"}));
                self.run("edit.deselect", json!({}));
                self.tool = Tool::Brush;
                self.brush.size = 24.0;
                self.brush.mode = crate::brushtool::BrushOptions::default().mode;
                if which == "brushing" {
                    self.brush_samples = (0..=40).map(|i| (anicraft_engine::geom::Vec2::new(150.0 + i as f32 * 9.0, 610.0 - (i as f32 * 0.16).sin() * 30.0), Some(0.3 + i as f32 / 60.0))).collect();
                    self.drag = Drag::Brush;
                }
            }
            Some(which @ ("polystar" | "paintinside" | "drawingedit" | "shapeenvelope")) => {
                // Drawing parity (ADR 0080) on a new layer: polygons and stars of several sides
                // and point sizes; brush strokes clipped by Paint Inside and Paint Selection; a
                // drawing object open in place with a line merged across it; an envelope bending
                // a drawing object.
                self.run("layer.new", json!({"name": "Parity"}));
                self.run("draw.object_drawing", json!({"on": which != "paintinside"}));
                let ink = json!({"stroke": "#1b2a4a", "stroke_width": 3});
                let with = |mut p: Value| {
                    for (k, v) in ink.as_object().into_iter().flatten() {
                        p[k] = v.clone();
                    }
                    p
                };
                match which {
                    "polystar" => {
                        for (k, (shape, sides, size, fill)) in [("polygon", 3, 0.5, "#e8553b"), ("polygon", 6, 0.5, "#f2c14e"), ("star", 5, 0.5, "#3d8bfd"), ("star", 8, 0.2, "#7cc576"), ("star", 12, 0.8, "#d6336c")].into_iter().enumerate() {
                            self.run("draw.star", with(json!({"cx": 170 + k as i32 * 190, "cy": 300, "outer": 80, "shape": shape, "sides": sides, "point_size": size, "fill": fill})));
                        }
                        self.run("edit.deselect", json!({}));
                        self.tool = Tool::Star;
                        self.polystar = PolyStarOptions { star: true, sides: 8, point_size: 0.2 };
                    }
                    "paintinside" => {
                        self.run("draw.rect", with(json!({"x": 120, "y": 160, "w": 360, "h": 300, "fill": "#f2c14e"})));
                        self.run("draw.ellipse", with(json!({"x": 600, "y": 160, "w": 360, "h": 300, "fill": "#3d8bfd"})));
                        self.run("edit.deselect", json!({}));
                        let wave = |y: f32| -> Vec<[f32; 2]> { (0..=80).map(|i| [60.0 + i as f32 * 12.0, y + (i as f32 * 0.25).sin() * 30.0]).collect() };
                        // Started in the yellow rectangle: it paints only there.
                        self.run("draw.brush", json!({"points": wave(250.0).into_iter().skip(10).collect::<Vec<_>>(), "size": 36, "mode": "inside", "color": "#e8553b"}));
                        // The blue oval selected: Paint Selection keeps to it.
                        if let Some(oval) = self.s.visible_elements().iter().find(|e| matches!(&e.kind, anicraft_engine::doc::ElementKind::Shape(sh) if sh.fill == Some(anicraft_engine::doc::Paint::Solid(anicraft_engine::doc::Rgba::hex(0x3d8bfd))))).map(|e| e.id.0) {
                            self.run("element.select", json!({"ids": [oval]}));
                            self.run("draw.brush", json!({"points": wave(380.0), "size": 30, "mode": "selection", "color": "#7cc576"}));
                        }
                        self.run("edit.deselect", json!({}));
                        self.tool = Tool::Brush;
                        self.brush.mode = anicraft_engine::cmds_brush::PaintMode::Inside;
                    }
                    "drawingedit" => {
                        self.run("draw.rect", with(json!({"x": 140, "y": 180, "w": 300, "h": 220, "radius": 24, "fill": "#e8553b"})));
                        let v = self.run("draw.ellipse", with(json!({"x": 520, "y": 170, "w": 320, "h": 240, "fill": "#f2c14e"})));
                        if let Some(id) = v.and_then(|v| v["element"].as_u64()) {
                            self.run("symbol.edit", json!({"id": id}));
                            // Inside, in the object's own space: a line across merges with it alone.
                            self.run("draw.line", json!({"x0": -200, "y0": -40, "x1": 200, "y1": 60, "stroke": "#1b2a4a", "stroke_width": 3, "object": false}));
                            self.run("edit.deselect", json!({}));
                        }
                        self.tool = Tool::Select;
                    }
                    _ => {
                        self.run("draw.star", with(json!({"cx": 300, "cy": 320, "outer": 120, "shape": "star", "sides": 5, "point_size": 0.5, "fill": "#3d8bfd"})));
                        self.run("draw.rect", with(json!({"x": 560, "y": 200, "w": 360, "h": 220, "fill": "#e8553b"})));
                        crate::shape_envelope::open(self);
                        if let Some((_, _, mesh)) = self.s.envelope() {
                            let mut m = crate::shape_envelope::moved(&mesh, 1, anicraft_engine::geom::Vec2::new(0.0, -80.0));
                            m = crate::shape_envelope::moved(&m, 13, anicraft_engine::geom::Vec2::new(30.0, 60.0));
                            m = crate::shape_envelope::moved(&m, 3, anicraft_engine::geom::Vec2::new(40.0, -20.0));
                            let points: Vec<[f32; 2]> = m.iter().map(|p| [p.x, p.y]).collect();
                            self.run("modify.envelope_move", json!({"points": points}));
                        }
                    }
                }
            }
            Some(which @ ("eraser" | "erasing")) => {
                // Erased artwork on a new layer (ADR 0034): a band through an outlined raw
                // rectangle (Erase Normal), the outline of a raw oval erased but not its fill
                // (Erase Lines), a fill deleted by the Faucet, and a drawing object the eraser
                // passed over untouched; "erasing" also has a stroke in progress.
                self.run("layer.new", json!({"name": "Ink"}));
                self.run("draw.object_drawing", json!({"on": false}));
                self.run("draw.rect", json!({"x": 140, "y": 150, "w": 380, "h": 230, "fill": "#f2c14e", "stroke": "#1b2a4a", "stroke_width": 4}));
                let band: Vec<[f32; 2]> = (0..=80).map(|i| [100.0 + i as f32 * 5.5, 265.0 + (i as f32 * 0.12).sin() * 60.0]).collect();
                self.run("draw.erase", json!({"points": band, "size": 30}));
                self.run("draw.ellipse", json!({"x": 600, "y": 160, "w": 260, "h": 200, "fill": "#3d8bfd", "stroke": "#1b2a4a", "stroke_width": 6}));
                self.run("draw.erase", json!({"points": [[560, 260], [900, 260]], "size": 40, "mode": "lines"}));
                self.run("draw.rect", json!({"x": 940, "y": 160, "w": 200, "h": 200, "fill": "#e8553b", "stroke": "#1b2a4a", "stroke_width": 4}));
                self.run("draw.faucet", json!({"x": 1040, "y": 260}));
                self.run("draw.rect", json!({"x": 300, "y": 430, "w": 300, "h": 120, "fill": "#8a79c9", "stroke": null, "object": true}));
                self.run("draw.erase", json!({"points": [[260, 490], [640, 490]], "size": 30}));
                self.run("edit.deselect", json!({}));
                self.tool = Tool::Eraser;
                self.eraser = crate::erasertool::EraserOptions { size: 30.0, ..Default::default() };
                if which == "erasing" {
                    self.brush_samples = (0..=40).map(|i| (anicraft_engine::geom::Vec2::new(700.0 + i as f32 * 9.0, 470.0 - (i as f32 * 0.2).sin() * 40.0), None)).collect();
                    self.drag = Drag::Eraser;
                }
            }
            Some("tiltbrush") => {
                // Tilt and textured brushes (ADR 0093) on a new layer: a flat nib whose angle
                // follows the pen's azimuth as it turns along an S, a round brush widening as the
                // pen lies down, Charcoal (the paper grain) and Spatter.
                // Tilt comes in through the command's points, as no
                // desktop system reports it to the toolkit yet; so Properties shows Texture and
                // Scatter, and keeps the Tilt options hidden (ADR 0093).
                self.run("layer.new", json!({"name": "Tilt"}));
                self.run("draw.object_drawing", json!({"on": false}));
                let s_curve: Vec<[f32; 5]> = (0..=90)
                    .map(|i| {
                        let t = i as f32 / 90.0;
                        [140.0 + t * 520.0, 200.0 + (t * std::f32::consts::TAU).sin() * 70.0, 1.0, 35.0, 180.0 * t]
                    })
                    .collect();
                self.run("draw.brush", json!({"points": s_curve, "size": 30, "roundness": 15, "angle": 0, "tilt_angle": true, "color": "#1b2a4a"}));
                let lying: Vec<[f32; 5]> = (0..=60).map(|i| [140.0 + i as f32 * 8.6, 380.0, 1.0, 90.0 - i as f32 * 1.5, 0.0]).collect();
                self.run("draw.brush", json!({"points": lying, "size": 14, "tilt_width": true, "color": "#e8553b"}));
                let wave = |y: f32| -> Vec<[f32; 3]> { (0..=60).map(|i| [140.0 + i as f32 * 8.6, y + (i as f32 * 0.16).sin() * 22.0, 0.4 + 0.6 * (i as f32 / 60.0 * std::f32::consts::PI).sin()]).collect() };
                self.run("draw.brush", json!({"points": wave(500.0), "preset": "charcoal", "size": 34, "color": "#3a4150"}));
                self.run("draw.brush", json!({"points": wave(620.0), "preset": "spatter", "size": 9, "color": "#3d8bfd"}));
                // Sample artwork's labels are document text, not interface text.
                for (y, text) in [(180.0, "Tilt turns the nib"), (360.0, "Tilt widens the tip"), (470.0, "Charcoal"), (610.0, "Spatter")] { // i18n-exempt: sample text
                    self.run("text.create", json!({"x": 720, "y": y, "text": text, "size": 22, "color": "#f4f1ea"}));
                }
                self.run("edit.deselect", json!({}));
                self.tool = Tool::Brush;
                if let Some(p) = anicraft_engine::brush_library::builtin("charcoal") {
                    crate::brush_panel::choose(self, &p);
                }
            }
            Some("erasemerge") => {
                // ADR 0093 on a new layer: a raw gradient rectangle cut in two by the eraser (each
                // piece keeps its part of the gradient), a variable-width raw line erased across
                // (each piece keeps its widths), and a symbol the eraser passed over on the stage,
                // with the hint that says how to erase inside it.
                self.run("layer.new", json!({"name": "Erased"}));
                self.run("draw.object_drawing", json!({"on": false}));
                let stops = vec![anicraft_engine::doc::GradientStop { offset: 0.0, color: anicraft_engine::doc::Rgba::hex(0xe8553b) }, anicraft_engine::doc::GradientStop { offset: 1.0, color: anicraft_engine::doc::Rgba::hex(0x3d8bfd) }];
                self.s.style.fill_paint = Some(anicraft_engine::doc::Paint::linear(anicraft_engine::geom::Vec2::new(120.0, 0.0), anicraft_engine::geom::Vec2::new(600.0, 0.0), stops));
                self.s.style.lock_fill = true;
                self.run("draw.rect", json!({"x": 120, "y": 120, "w": 480, "h": 240, "stroke": null}));
                self.s.style.fill_paint = None;
                self.s.style.lock_fill = false;
                let band: Vec<[f32; 2]> = (0..=60).map(|i| [360.0 + (i as f32 * 0.2).sin() * 40.0, 90.0 + i as f32 * 5.0]).collect();
                self.run("draw.erase", json!({"points": band, "size": 36}));
                self.s.style.set_profile(anicraft_engine::doc::ProfilePreset::TaperBoth);
                self.run("draw.line", json!({"x0": 120, "y0": 480, "x1": 600, "y1": 480, "stroke": "#1b2a4a", "stroke_width": 28}));
                self.s.style.set_profile(anicraft_engine::doc::ProfilePreset::Uniform);
                for x in [260.0f32, 440.0] {
                    self.run("draw.erase", json!({"points": [[x, 440.0], [x + 10.0, 520.0]], "size": 44}));
                }
                self.run("draw.ellipse", json!({"x": 760, "y": 160, "w": 300, "h": 300, "fill": "#f2c14e", "stroke": "#1b2a4a", "stroke_width": 4}));
                self.run("edit.select_all", json!({}));
                let disc = self.s.visible_elements().iter().filter(|e| matches!(&e.kind, anicraft_engine::doc::ElementKind::Shape(sh) if sh.merge && sh.path.bounds().transformed(&e.transform).min.x >= 740.0)).map(|e| e.id.0).collect::<Vec<_>>();
                self.run("element.select", json!({"ids": disc}));
                self.run("modify.convert_to_symbol", json!({"name": "Sun"})); // i18n-exempt: a sample symbol name
                self.run("edit.deselect", json!({}));
                self.tool = Tool::Eraser;
                self.eraser = crate::erasertool::EraserOptions { size: 30.0, ..Default::default() };
                self.brush_samples = (0..=30).map(|i| (anicraft_engine::geom::Vec2::new(720.0 + i as f32 * 12.0, 310.0), None)).collect();
                crate::erasertool::commit(self, crate::pathtools::Xf { origin: egui::Pos2::ZERO, zoom: 1.0 });
            }
            Some("eyedropper") => {
                // The sample's sky gradient picked with the Eyedropper: the fill swatch holds it.
                self.run("paint.pick", json!({"x": 640, "y": 40}));
                self.tool = Tool::Eyedropper;
            }
            Some("frames") => {
                // Frames 8–20 selected on the top three layers, with a keyframe drag in progress:
                // 6 frames later and one layer down.
                let layers = &self.s.timeline().layers;
                if let (Some(a), Some(b)) = (layers.first(), layers.get(2)) {
                    self.frame_sel = Some(timeline::FrameSel { anchor: (a.id, 7), head: (b.id, 19) });
                    self.tl_drag = timeline::TlDrag::Move { grab: 10, by: 6, grab_row: 0, rows_by: 1 };
                }
                self.run("timeline.goto", json!({"frame": 10}));
            }
            Some("bucket") => {
                // A hand-drawn outline with no fill, filled by the bucket underneath its line.
                let loop_pts: Vec<[f32; 2]> = (0..=72)
                    .map(|i| {
                        let a = i as f32 / 72.0 * std::f32::consts::TAU;
                        let r = 110.0 + 18.0 * (a * 5.0).sin();
                        [640.0 + r * a.cos(), 330.0 + r * a.sin()]
                    })
                    .collect();
                self.run("draw.freehand", json!({"points": loop_pts, "closed": true, "stroke": "#14141a", "stroke_width": 4}));
                self.run("paint.bucket", json!({"x": 640, "y": 330, "color": "#ff9f43"}));
                self.tool = Tool::PaintBucket;
                self.bucket_gap = GapSize::Small;
                self.run("edit.deselect", json!({}));
            }
            Some("sound") => {
                // A two-second beat imported to the stage, stream sync, its keyframe's Sound
                // properties showing.
                if self.import_demo_beat() {
                    self.run("sound.set", json!({"frame": 0, "sync": "stream"}));
                    self.run("timeline.goto", json!({"frame": 14}));
                }
            }
            Some("plugineases") => {
                // The demo ball's tween on a plug-in's ease (Smooth Steps 4 of the sample
                // Stepped Eases), the timeline's ease menu open on it (ANICRAFT_PLUGINS set).
                let ball = self.s.timeline().layers.iter().find(|l| l.keyframes.first().is_some_and(|k| k.tween.is_some() && k.motion.is_none())).map(|l| l.id.0);
                let ease = anicraft_engine::cmds_ease::plugin_eases(&self.s).into_iter().find(|(_, c)| c.id.starts_with("smooth")).or_else(|| anicraft_engine::cmds_ease::plugin_eases(&self.s).into_iter().next());
                if let (Some(id), Some((p, c))) = (ball, ease) {
                    self.run("layer.select", json!({"layer": id}));
                    self.run("timeline.goto", json!({"frame": 4}));
                    self.run("timeline.set_easing", json!({"easing": {"plugin": p.id(), "ease": c.id}}));
                    self.s.mark_saved();
                    self.shot_ease_menu.set(Some("tl_ease"));
                }
            }
            Some(which @ ("ease" | "easepreset")) => {
                // The demo's ball: its classic tween eased by a custom curve of three segments
                // that overshoots and settles (saved as "Overshoot"), the Custom Ease window open
                // on it with a point picked; or Elastic Out on its own Position group.
                let ball = self.s.timeline().layers.iter().find(|l| l.keyframes.first().is_some_and(|k| k.tween.is_some() && k.motion.is_none())).map(|l| l.id.0);
                if let Some(id) = ball {
                    self.run("layer.select", json!({"layer": id}));
                    self.run("timeline.goto", json!({"frame": 4}));
                    let start = self.s.active_layer().and_then(|l| l.keyframe_at(4)).map_or(0, |k| k.start);
                    let curve = json!([[0, 0], [0.12, 0.6], [0.22, 1.25], [0.38, 1.25], [0.5, 1.25], [0.55, 0.85], [0.66, 0.85], [0.76, 0.85], [0.86, 1.0], [1, 1]]);
                    self.run("timeline.set_easing", json!({"frame": start, "easing": {"custom": curve}}));
                    self.run("ease.save", json!({"frame": start, "name": "Overshoot"}));
                    if which == "easepreset" {
                        self.run("ease.separate", json!({"separate": true, "frame": start}));
                        self.run("timeline.set_easing", json!({"frame": start, "group": "position", "easing": "ElasticOut"}));
                    }
                    self.s.mark_saved();
                    crate::ease_editor::open_here(self);
                    if let Some(ed) = self.ease.as_mut()
                        && which == "easepreset"
                    {
                        ed.group = Some(anicraft_engine::doc::EaseGroup::Position);
                    }
                    crate::ease_editor::select(self, if which == "ease" { 1 } else { 3 });
                }
            }
            Some("envelope") => {
                // The beat with a custom envelope (in from the left, out to the right), its
                // envelope editor open on a selected point, the layer at 80 %.
                if self.import_demo_beat() {
                    let pts = json!([[0.0, 0.0, 0.0], [0.4, 1.0, 0.25], [1.2, 0.6, 1.0], [2.0, 0.0, 0.1]]);
                    self.run("sound.set", json!({"frame": 0, "envelope": pts}));
                    self.run("layer.set_volume", json!({"volume": 0.8}));
                    self.run("timeline.goto", json!({"frame": 10}));
                    crate::envelope::open(self);
                    crate::envelope::select(self, 1);
                }
            }
            Some(which @ ("hints" | "hintsend")) => {
                // A square shape-tweening into a circle on a new layer, with three shape hints:
                // a placed on both keyframes, b on the first only, c just added (unplaced).
                self.run("layer.new", json!({"name": "Morph"}));
                self.run("timeline.goto", json!({"frame": 0}));
                self.run("draw.rect", json!({"x": 420, "y": 220, "w": 200, "h": 200, "fill": "#ff9f43"}));
                self.run("timeline.insert_blank_keyframe", json!({"frame": 15}));
                self.run("timeline.goto", json!({"frame": 15}));
                self.run("draw.ellipse", json!({"x": 750, "y": 210, "w": 220, "h": 220, "fill": "#ff9f43"}));
                self.run("timeline.create_shape_tween", json!({"frame": 0}));
                self.run("edit.deselect", json!({}));
                let d = 110.0 * std::f32::consts::FRAC_1_SQRT_2;
                for (start, end) in [(Some((420.0, 220.0)), Some((860.0 - d, 320.0 - d))), (Some((620.0, 330.0)), None), (None, None)] {
                    let Some(v) = self.run("tween.add_hint", json!({"frame": 0})) else { break };
                    let hint = v["hint"].clone();
                    if let Some((x, y)) = start {
                        self.run("tween.move_hint", json!({"hint": hint, "frame": 0, "side": "start", "x": x, "y": y}));
                    }
                    if let Some((x, y)) = end {
                        self.run("tween.move_hint", json!({"hint": hint, "frame": 0, "side": "end", "x": x, "y": y}));
                    }
                }
                self.run("timeline.goto", json!({"frame": if which == "hints" { 0 } else { 15 }}));
            }
            Some(which @ ("camera" | "cameraview")) => {
                // The sample seen by a camera that pushes in and turns over the first second,
                // the far background a layer deep; the Camera tool in hand at frame 12.
                self.run("camera.add", json!({}));
                self.run("timeline.insert_keyframe", json!({"frame": 23}));
                self.run("camera.set", json!({"frame": 23, "x": 760, "y": 380, "zoom": 160, "rotation": -8}));
                self.run("timeline.create_tween", json!({"frame": 0}));
                if let Some(back) = self.s.doc.timeline(0).layers.last().map(|l| l.id.0) {
                    self.run("layer.set_depth", json!({"layer": back, "depth": 1500}));
                }
                self.run("timeline.goto", json!({"frame": 12}));
                self.tool = Tool::Camera;
                self.last_tool = Tool::Camera;
                self.camera_preview = which == "cameraview";
            }
            Some(which @ ("layerdepth" | "layerdepthview")) => {
                // A parallax scene mid-pan (ADR 0082): the camera pans right and pushes in over
                // the first second; the sky is far back, the sun and hills between, the star
                // attached to the camera like a title; the Layer Depth panel open, frame 12.
                self.run("camera.add", json!({}));
                self.run("timeline.insert_keyframe", json!({"frame": 23}));
                self.run("camera.set", json!({"frame": 23, "dx": 100, "zoom": 125}));
                self.run("timeline.create_tween", json!({"frame": 0}));
                let id = |app: &App, name: &str| app.s.doc.timeline(0).layers.iter().find(|l| l.name.eq_ignore_ascii_case(name)).map(|l| l.id.0);
                for (name, depth) in [("sky", 900), ("sun", 450), ("hills", 200)] {
                    if let Some(l) = id(self, name) {
                        self.run("layer.set_depth", json!({"layer": l, "depth": depth}));
                    }
                }
                if let Some(l) = id(self, "star") {
                    self.run("layer.set_attached", json!({"layer": l, "attached": true}));
                }
                self.run("timeline.goto", json!({"frame": 12}));
                self.tool = Tool::Select;
                self.camera_preview = which == "layerdepthview";
                self.show_panel(crate::dock::Panel::LayerDepth);
            }
            Some("bones") => {
                // A three-limb arm on an armature (ADR 0043): bent by IK at frame 1, the elbow
                // limited, a second pose at frame 24, the elbow's bone selected.
                self.run("layer.new", json!({"name": "Puppet"}));
                self.run("draw.rect", json!({"x": 315, "y": 400, "w": 170, "h": 40, "radius": 20, "fill": "#ff9f43", "stroke": "#1b2a4a", "stroke_width": 3}));
                let id = |v: Option<Value>, k: &str| v.and_then(|v| v[k].as_u64()).unwrap_or(0);
                let made = self.run("modify.convert_to_symbol", json!({"name": "Limb"}));
                let (symbol, root) = (id(made.clone(), "symbol"), id(made, "instance"));
                let arm = id(self.run("library.place", json!({"symbol": symbol, "x": 560, "y": 420})), "element");
                let hand = id(self.run("library.place", json!({"symbol": symbol, "x": 720, "y": 420})), "element");
                let b1 = id(self.run("bone.add", json!({"from": root, "to": arm, "head": [480, 420], "tail": [640, 420]})), "bone");
                let b2 = id(self.run("bone.add", json!({"from": arm, "to": hand, "tail": [790, 420]})), "bone");
                self.run("bone.set_limits", json!({"bone": b2, "min": -30, "max": 120}));
                self.run("bone.solve", json!({"bone": b2, "x": 700, "y": 560, "frame": 0}));
                self.run("bone.pose", json!({"bone": b1, "angle": -35, "frame": 23}));
                self.run("timeline.goto", json!({"frame": 0}));
                self.bone_sel = Some(anicraft_engine::doc::BoneId(b2));
                self.tool = Tool::Select;
            }
            Some("springbones") => {
                // A tail on springs (ADR 0084): a body that bobs up and back down over half a
                // second, its three-piece tail swinging after it, mid-swing at frame 17; the tip's
                // bone picked (its joint slides along X), so Properties shows Spring and
                // Translation.
                self.run("layer.new", json!({"name": "Critter"}));
                let id = |v: Option<Value>, k: &str| v.and_then(|v| v[k].as_u64()).unwrap_or(0);
                self.run("draw.rect", json!({"x": 260, "y": 300, "w": 190, "h": 110, "radius": 36, "fill": "#5f8dd3", "stroke": "#1b2a4a", "stroke_width": 3}));
                let body = id(self.run("modify.convert_to_symbol", json!({"name": "Body"})), "instance");
                self.run("draw.rect", json!({"x": 450, "y": 341, "w": 120, "h": 28, "radius": 14, "fill": "#ff9f43", "stroke": "#1b2a4a", "stroke_width": 3}));
                let made = self.run("modify.convert_to_symbol", json!({"name": "Tail"}));
                let (symbol, t1) = (id(made.clone(), "symbol"), id(made, "instance"));
                let t2 = id(self.run("library.place", json!({"symbol": symbol, "x": 620, "y": 355})), "element");
                let t3 = id(self.run("library.place", json!({"symbol": symbol, "x": 730, "y": 355})), "element");
                let b1 = id(self.run("bone.add", json!({"from": body, "to": t1, "head": [452, 355], "tail": [562, 355]})), "bone");
                let b2 = id(self.run("bone.add", json!({"from": t1, "to": t2, "tail": [672, 355]})), "bone");
                let b3 = id(self.run("bone.add", json!({"from": t2, "to": t3, "tail": [782, 355]})), "bone");
                self.run("bone.pose", json!({"dx": 0, "dy": -50, "frame": 6}));
                self.run("bone.pose", json!({"dx": 0, "dy": 50, "frame": 12}));
                for b in [b1, b2, b3] {
                    self.run("bone.set_spring", json!({"bone": b, "strength": 45, "damping": 30}));
                }
                self.run("bone.set_translation", json!({"bone": b3, "axis": "x", "min": -20, "max": 40}));
                self.run("timeline.goto", json!({"frame": 16}));
                self.tool = Tool::Select;
                crate::bone_tool::select(self, self.s.layer, Some(anicraft_engine::doc::BoneId(b3)));
            }
            Some("shapebones") => {
                // A tail with bones inside it (ADR 0071): four bones, curled at frame 1 and the
                // other way at frame 24, the second bone picked with the Bind tool in hand.
                self.run("layer.new", json!({"name": "Tail"}));
                let d = "M220 260 C300 252 420 250 540 262 C660 274 760 280 860 296 C872 300 872 306 860 308 C760 318 660 318 540 316 C420 314 300 334 220 340 C200 340 200 260 220 260 Z";
                let id = |v: Option<Value>, k: &str| v.and_then(|v| v[k].as_u64()).unwrap_or(0);
                let shape = id(self.run("draw.path", json!({"d": d, "fill": "#e8833a", "stroke": "#4a2c14", "stroke_width": 3, "object": true})), "element");
                self.run("timeline.insert_frame", json!({"frame": 23}));
                self.run("timeline.goto", json!({"frame": 0}));
                let mut bone = id(self.run("bone.add_in_shape", json!({"shape": shape, "head": [220, 300], "tail": [380, 296]})), "bone");
                let mut bones = vec![bone];
                for x in [540, 700, 850] {
                    bone = id(self.run("bone.add_in_shape", json!({"parent": bone, "tail": [x, 298]})), "bone");
                    bones.push(bone);
                }
                for (b, (a, z)) in bones.iter().zip([(-4, 3), (-12, 10), (-16, 14), (-20, 18)]) {
                    self.run("bone.pose", json!({"bone": b, "angle": a, "frame": 0}));
                    self.run("bone.pose", json!({"bone": b, "angle": z, "frame": 23}));
                }
                self.run("timeline.goto", json!({"frame": 0}));
                self.bone_sel = bones.get(1).copied().map(anicraft_engine::doc::BoneId);
                self.tool = Tool::Bind;
            }
            Some("shapeweights") => {
                // Painted weights (ADR 0090): the tail of `shapebones` at rest, a point added to
                // its outline with Subselection, and the second bone's weights mid-stroke with
                // the Bind tool's brush, shown as a heat map.
                self.run("layer.new", json!({"name": "Tail"}));
                // A tapering tail with a point every 20 px along both edges, so the heat map reads.
                let half = |x: f32| 40.0 * (1.0 - (x - 220.0) / 700.0) + 4.0;
                let mid = |x: f32| 300.0 - 0.00004 * (x - 220.0) * (x - 220.0);
                let xs: Vec<f32> = (0..=32).map(|i| 220.0 + 20.0 * i as f32).collect();
                let mut d = String::new();
                for (i, x) in xs.iter().enumerate() {
                    d.push_str(&format!("{}{x} {} ", if i == 0 { "M" } else { "L" }, mid(*x) - half(*x)));
                }
                for x in xs.iter().rev() {
                    d.push_str(&format!("L{x} {} ", mid(*x) + half(*x)));
                }
                d.push('Z');
                let id = |v: Option<Value>, k: &str| v.and_then(|v| v[k].as_u64()).unwrap_or(0);
                let shape = id(self.run("draw.path", json!({"d": d, "fill": "#e8833a", "stroke": "#4a2c14", "stroke_width": 3, "object": true})), "element");
                self.run("timeline.insert_frame", json!({"frame": 23}));
                self.run("timeline.goto", json!({"frame": 0}));
                let mut bone = id(self.run("bone.add_in_shape", json!({"shape": shape, "head": [222.0, mid(222.0)], "tail": [380.0, mid(380.0)]})), "bone");
                let mut bones = vec![bone];
                for x in [540.0, 700.0, 850.0] {
                    bone = id(self.run("bone.add_in_shape", json!({"parent": bone, "tail": [x, mid(x)]})), "bone");
                    bones.push(bone);
                }
                for (b, z) in bones.iter().zip([3, 10, 14, 18]) {
                    self.run("bone.pose", json!({"bone": b, "angle": z, "frame": 23}));
                }
                self.run("timeline.goto", json!({"frame": 0}));
                self.run("path.add_point", json!({"id": shape, "x": 470.0, "y": mid(470.0) - half(470.0), "slop": 8}));
                let b2 = bones.get(1).copied().unwrap_or(0);
                self.bone_sel = Some(anicraft_engine::doc::BoneId(b2));
                self.tool = Tool::Bind;
                self.bind = crate::bind_tool::BindOptions { paint: true, radius: 55.0, strength: 35.0, point: Some(14), preview: Some(anicraft_engine::geom::Vec2::new(500.0, mid(500.0) - 20.0)), ..Default::default() };
                // A stroke along the top edge, still under way.
                self.s.begin_gesture(tr_noop!("Paint Weights"));
                let at: Vec<[f32; 2]> = (0..25).map(|i| 300.0 + 8.0 * i as f32).map(|x| [x, mid(x) - 20.0]).collect();
                self.run("bone.paint_weights", json!({"bone": b2, "at": at, "radius": 55, "strength": 35}));
            }
            Some("tlwide") => {
                // Zoomed in: wide frames, playhead on frame 1 and frame 4.
                self.tl_cell_w = 48.0;
                self.run("timeline.goto", json!({"frame": 3}));
            }
            Some("tlzoom") => {
                // A long timeline zoomed out: the ruler thins its labels, the scrollbar shows.
                self.run("timeline.insert_frame", json!({"frame": 47, "count": 600}));
                self.tl_cell_w = 4.0;
                self.tl_scroll = 120.0;
            }
            Some("span") => {
                // Stretching the Star layer's span by its end, with the length readout.
                if let Some(l) = self.s.timeline().layers.get(1)
                    && let Some(k) = l.keyframes.first()
                {
                    self.tl_drag = timeline::TlDrag::Span { layer: l.id, start: k.start, end: k.end(), to: k.end() + 6 };
                }
            }
            Some("swatches") => {
                // The fill swatch's colour picker open.
                self.open_fill_swatch = true;
            }
            Some("swatchpanel") => {
                // The Swatches panel (ADR 0078): a palette of the artist's own added to the
                // default set, one picked, a gradient on the selected shape.
                // i18n-exempt: swatch names are document data (a screenshot sample), not interface text
                for (c, name) in [("#264653", "Deep teal"), ("#2a9d8f", "Teal"), ("#e9c46a", "Saffron"), ("#f4a261", "Sandy"), ("#e76f51", "Burnt sienna")] {
                    self.run("swatch.add", json!({"color": c, "name": name}));
                }
                self.run("swatch.add", json!({"type": "linear", "stops": [{"offset": 0, "color": "#264653"}, {"offset": 0.5, "color": "#e9c46a"}, {"offset": 1, "color": "#e76f51"}], "name": "Dusk"}));
                self.run("edit.select_all", json!({}));
                let n = self.s.doc.swatches.len();
                self.swatch_sel = n.checked_sub(4);
                self.colour_target = crate::swatches_panel::ColourTarget::Fill;
                self.show_panel(crate::dock::Panel::Swatches);
            }
            Some("width") => {
                // A tapered brush-like stroke being shaped with the Width tool: one width point
                // pushed out on its left side and selected.
                let wave: Vec<[f32; 2]> = (0..=200).map(|i| [180.0 + i as f32 * 4.0, 420.0 - (i as f32 * 0.035).sin() * 110.0]).collect();
                if let Some(v) = self.run("draw.freehand", json!({"points": wave, "stroke": "#1b2a4a", "stroke_width": 14, "profile": "taper_both"})) {
                    let id = anicraft_engine::doc::ElementId(v["element"].as_u64().unwrap_or(0));
                    if let Some(w) = self.run("stroke.set_width_point", json!({"id": id.0, "t": 0.3, "left": 2.4, "right": 0.9})) {
                        self.width_sel = w["index"].as_u64().map(|i| (id, i as usize));
                    }
                }
                self.tool = Tool::Width;
            }
            Some(which @ ("gradient" | "gradientbucket")) => {
                // The gradient tools (ADR 0074): a reflecting linear gradient turned on a panel,
                // and an oval with a stretched radial gradient and its focal point off centre,
                // selected with the Gradient Transform tool (its handles show) while the colour
                // picker edits its stops.
                self.perform("file.new", json!({"width": 640, "height": 400}));
                self.run("draw.rect", json!({"x": 0, "y": 0, "w": 640, "h": 400, "fill": "#14213d", "stroke": null}));
                self.run("draw.rect", json!({"x": 40, "y": 60, "w": 200, "h": 280, "stroke": null}));
                self.run("paint.set_fill", json!({"type": "linear", "stops": [{"offset": 0, "color": "#e76f51"}, {"offset": 1, "color": "#f4a261"}], "spread": "reflect"}));
                self.run("paint.transform_fill", json!({"scale_x": 0.3, "rotate": 30}));
                self.run("draw.ellipse", json!({"x": 280, "y": 50, "w": 320, "h": 300, "stroke": null}));
                self.run("paint.set_fill", json!({"type": "radial", "stops": [{"offset": 0, "color": "#ffffff"}, {"offset": 0.45, "color": "#8ecae6"}, {"offset": 1, "color": "#023047"}], "focal": 0.55}));
                self.run("paint.transform_fill", json!({"scale_x": 1.1, "scale_y": 0.8, "rotate": -35}));
                self.tool = Tool::GradientTransform;
                if which == "gradient" {
                    // The colour picker open on it, in gradient mode: the stop bar, overflow and
                    // the picked stop's colour.
                    self.open_fill_swatch = true;
                }
                if which == "gradientbucket" {
                    // The Paint Bucket in hand, nothing selected: the swatch's gradient and Lock Fill.
                    self.run("edit.deselect", json!({}));
                    self.tool = Tool::PaintBucket;
                }
            }
            Some(which @ ("motion" | "motionspan")) => {
                // The Star layer's span as a motion tween: it moves, turns and fades out, with
                // an eased X and a custom Bezier on Y, shown in the Motion Editor.
                self.run("layer.select", json!({"index": 1}));
                if self.run("timeline.create_motion_tween", json!({"frame": 0})).is_some() {
                    let frames = self.s.active_layer().and_then(|l| l.keyframes.first()).map_or(24, |k| k.duration);
                    let (mid, end) = (frames / 2, frames.saturating_sub(1));
                    let x0 = self.run("motion.info", json!({"frame": 0})).and_then(|v| v["values"]["x"].as_f64()).unwrap_or(0.0);
                    let y0 = self.run("motion.info", json!({"frame": 0})).and_then(|v| v["values"]["y"].as_f64()).unwrap_or(0.0);
                    for (p, f, v) in [("x", mid, x0 + 260.0), ("x", end, x0 + 420.0), ("y", mid, y0 + 160.0), ("y", end, y0), ("rotation", end, 270.0), ("alpha", end, 20.0)] {
                        self.run("motion.set_key", json!({"property": p, "frame": f, "value": v}));
                    }
                    self.run("motion.set_ease", json!({"property": "x", "frame": 1, "easing": "CubicOut"}));
                    self.run("motion.set_ease", json!({"property": "y", "frame": 1, "easing": {"bezier": [0.2, 1.6, 0.6, 1.0]}}));
                    self.s.mark_saved();
                }
                if which == "motion" {
                    self.show_panel(crate::dock::Panel::MotionEditor);
                }
                self.me_prop = anicraft_engine::doc::Property::Y;
                self.me_key = Some(0);
                self.run("timeline.goto", json!({"frame": 6}));
            }
            Some("motionretime") => {
                // Retiming (ADR 0083): `motion`'s tween with every property's keys on the middle
                // frame picked on the lane and dragged three frames later, mid-drag.
                self.open_panel(Some("motion"));
                let Some((start, mid)) = self.s.active_layer().and_then(|l| l.keyframes.first()).map(|k| (k.start, k.duration / 2)) else { return };
                let keys: Vec<(anicraft_engine::doc::Property, u32)> = self.s.active_layer().and_then(|l| l.keyframes.first()).and_then(|k| k.motion.as_ref()).map(|m| m.tracks.iter().filter(|t| t.key_at(mid).is_some()).map(|t| (t.property, mid)).collect()).unwrap_or_default();
                self.me_sel = keys;
                self.me_sel_at = (self.s.layer.0, start);
                let by = motion_editor::retime(self, start, 3).unwrap_or(0);
                self.s.mark_saved();
                self.me_drag = Some(motion_editor::MeDrag::Key { frame: mid, lo: 0.0, hi: 1.0, axis: motion_editor::Axis::Time, by, lane: true });
                self.me_shot_drag = true;
                self.run("timeline.goto", json!({"frame": start + mid + 3}));
            }
            Some("motionfilters") => {
                // A blur tweening in the Motion Editor (ADR 0092): the Star becomes a movie clip
                // with a blur and a glow, its tween keys the blur up and down again with an ease
                // and pulses the glow, and the editor shows the Blur X curve mid-way.
                self.run("layer.select", json!({"index": 1}));
                self.run("timeline.goto", json!({"frame": 0}));
                self.run("edit.select_all", json!({}));
                self.run("symbol.set_kind", json!({"kind": "movie_clip"}));
                self.run("element.set_filters", json!({"filters": [{"type": "blur", "blur_x": 0, "blur_y": 0, "quality": 2}, {"type": "glow", "color": "#ffd54a", "blur_x": 12, "blur_y": 12, "strength": 1}]}));
                if self.run("timeline.create_motion_tween", json!({"frame": 0})).is_some() {
                    let frames = self.s.active_layer().and_then(|l| l.keyframes.first()).map_or(24, |k| k.duration);
                    let (mid, end) = (frames / 2, frames.saturating_sub(1));
                    for (p, f, v) in [("filter.0.blur_x", mid, 24.0), ("filter.0.blur_x", end, 0.0), ("filter.0.blur_y", mid, 8.0), ("filter.0.blur_y", end, 0.0), ("filter.1.strength", mid, 260.0), ("filter.1.strength", end, 100.0)] {
                        self.run("motion.set_key", json!({"property": p, "frame": f, "value": v}));
                    }
                    self.run("motion.set_ease", json!({"property": "filter.0.blur_x", "frame": 1, "easing": "CubicOut"}));
                    self.run("motion.set_ease", json!({"property": "filter.0.blur_x", "frame": mid + 1, "easing": {"bezier": [0.5, 0.0, 0.3, 1.0]}}));
                    self.s.mark_saved();
                    self.show_panel(crate::dock::Panel::MotionEditor);
                    self.me_prop = anicraft_engine::doc::Property::from_key("filter.0.blur_x").unwrap_or(anicraft_engine::doc::Property::X);
                    self.me_key = Some(0);
                    // The filter rows in view.
                    self.me_vscroll = 6.0;
                    self.run("timeline.goto", json!({"frame": mid / 2}));
                    let _ = self.s.execute("edit.deselect", &json!({}));
                }
            }
            Some("motionstretch") => {
                // A motion span stretched by its end (ADR 0083): `motionspan`'s tween made half
                // as long again, its keys in proportion, and its end being dragged further.
                self.open_panel(Some("motionspan"));
                let Some((lid, start, frames)) = self.s.active_layer().and_then(|l| l.keyframes.first().map(|k| (l.id, k.start, k.duration))) else { return };
                let longer = frames + frames / 2;
                self.run("timeline.resize_span", json!({"layer": lid.0, "frame": start, "frames": longer}));
                self.s.mark_saved();
                self.frame_sel = Some(timeline::FrameSel { anchor: (lid, start), head: (lid, start + longer - 1) });
                self.tl_drag = timeline::TlDrag::Span { layer: lid, start, end: start + longer, to: start + longer + 8 };
            }
            Some("docktabs") => {
                // The right dock at its narrowest with three long tab names in one group: they
                // shrink with an ellipsis and the strip scrolls, never under the menu button.
                if let Some(crate::dock::Place::Docked { side: crate::dock::Side::Right, group, .. }) = self.dock.place(crate::dock::Panel::Library)
                    && let Some(g) = self.dock.right.groups.get_mut(group)
                {
                    g.tabs = vec![crate::dock::Panel::Library, crate::dock::Panel::MovieExplorer, crate::dock::Panel::LayerDepth];
                    g.active = 2;
                }
                self.dock.right.size = Some(160.0);
            }
            Some(which @ ("motionpath" | "motionpoints" | "motionedit")) => {
                // The Star layer's span as a motion tween along a curved, oriented motion path,
                // the star selected half way along it (ADR 0030); with Subselection, the
                // middle anchor's handles show.
                self.run("layer.select", json!({"index": 1}));
                if self.run("timeline.create_motion_tween", json!({"frame": 0})).is_some() {
                    let info = self.run("motion.info", json!({"frame": 0}));
                    let at = |k: &str| info.as_ref().and_then(|v| v["values"][k].as_f64()).unwrap_or(0.0);
                    let (x, y) = (at("x"), at("y"));
                    let d = format!("M{x} {y} C{} {} {} {} {} {} C{} {} {} {} {} {}", x + 70.0, y - 150.0, x + 170.0, y - 150.0, x + 230.0, y - 20.0, x + 290.0, y + 110.0, x + 380.0, y + 130.0, x + 430.0, y + 10.0);
                    self.run("motion.set_path", json!({"d": d}));
                    self.run("motion.orient", json!({"orient": true}));
                    self.s.mark_saved();
                }
                self.run("timeline.goto", json!({"frame": 9}));
                self.run("edit.select_all", json!({}));
                if which != "motionpath" {
                    // The span selected instead of the star: Properties shows the tween's rows.
                    self.run("element.select", json!({"ids": []}));
                    self.frame_sel = Some(timeline::FrameSel::cell(self.s.layer, 9));
                    self.tool = Tool::Subselect;
                    self.motion_anchors = vec![1];
                }
                if which == "motionedit" {
                    // Points edited (ADR 0038): one added on the second segment, the middle
                    // anchor made a corner, both selected with their handles showing.
                    self.run("motion.add_point", json!({"segment": 2, "t": 0.45}));
                    self.run("motion.convert_point", json!({"anchor": 1, "smooth": false}));
                    self.motion_anchors = vec![1, 2];
                    self.status_ok(tr!("Added a point to the motion path. {undo} to undo.", undo = self.undo_key()));
                }
            }
            Some(which @ ("motionpins" | "motionpinsspan")) => {
                // Anchors pinned to frames (ADR 0049): `motionpath`'s curve with its middle point
                // pinned to frame 17, and the star moved up on frame 35, which pinned a new point
                // there. Subselection on that point, the span selected, and the Motion Editor on
                // Y (or the timeline, with its diamonds at the pinned frames).
                self.open_panel(Some("motionpath"));
                self.run("motion.pin_point", json!({"anchor": 1, "frame": 16}));
                self.run("timeline.goto", json!({"frame": 34}));
                self.run("element.translate", json!({"dx": 20, "dy": -110}));
                self.s.mark_saved();
                self.run("timeline.goto", json!({"frame": 24}));
                self.run("element.select", json!({"ids": []}));
                self.frame_sel = Some(timeline::FrameSel::cell(self.s.layer, 24));
                self.tool = Tool::Subselect;
                self.motion_anchors = vec![2];
                if which == "motionpins" {
                    self.show_panel(crate::dock::Panel::MotionEditor);
                }
                self.me_prop = anicraft_engine::doc::Property::Y;
                self.me_key = None;
            }
            Some("pen") => {
                // A pen path still being drawn: a corner, a smooth point with handles, a corner.
                self.tool = Tool::Pen;
                let k = |x: f32, y: f32| [anicraft_engine::geom::Vec2::new(x, y); 3];
                let mut curve = k(260.0, 140.0);
                curve[1] = anicraft_engine::geom::Vec2::new(220.0, 180.0);
                curve[2] = anicraft_engine::geom::Vec2::new(300.0, 100.0);
                self.pen = Some(pathtools::PenPath { knots: vec![k(120.0, 160.0), curve, k(380.0, 150.0)], dragging: false });
            }
            _ => {}
        }
    }

    /// Any modal (prompt, confirmation, close guard) is open: global shortcuts pause.
    pub fn modal_open(&self) -> bool {
        self.prompt.is_some() || self.convert.is_some() || self.confirm.is_some() || self.close_dialog || self.plugin_dialog.is_some() || self.templates.save.is_some()
    }

    /// The single entry point for user-initiated commands: asks first when the engine says the
    /// command needs confirmation (ADR 0004), otherwise runs it.
    pub fn request(&mut self, id: &str, params: Value) {
        if let Some(c) = self.s.confirmation(id, &params)
            && (c.irreversible || !self.skip_confirm.contains(id))
        {
            self.playing = false;
            self.confirm = Some(dialogs::PendingConfirm { id: id.to_string(), params, conf: c, dont_ask: false });
            return;
        }
        self.perform(id, params);
    }

    /// Runs a command and reports the outcome; routine destructive edits say how to undo.
    pub fn perform(&mut self, id: &str, params: Value) -> Option<Value> {
        if id == "file.publish" {
            // File › Publish runs in the background, with its own progress view (ADR 0060).
            crate::publish_panel::start(self, &params);
            return None;
        }
        let selected = self.s.selection.len();
        // A browser's autosaved copy opens as a file (ADR 0036): it is a restore all the same.
        let restored = autosave::restored_session(id, &params);
        let v = self.run(id, params)?;
        if let Some(session) = restored {
            self.view.fit = true;
            self.frame_sel = None;
            self.library_sel = crate::library::first_item(&self.s.doc);
            self.decode_sounds();
            self.recovery.restored(&session);
            let name = self.s.path.as_deref().and_then(std::path::Path::file_name).map_or_else(|| tr!("Untitled").to_string(), |n| n.to_string_lossy().into_owned());
            self.status_ok(tr!("Restored “{name}”. It isn't saved yet: Save to keep it.", name = name));
            return Some(v);
        }
        self.announce(id, &v, selected);
        Some(v)
    }

    /// What a command's result means for the window: the status line, and the view state a
    /// new document resets. `selected` is how many elements were selected before it ran.
    pub(crate) fn announce(&mut self, id: &str, v: &Value, selected: usize) {
        match id {
            "file.new" | "file.new_sample" | "file.new_from_template" | "file.open" | "file.open_recent" => {
                // The document is here: the start screen has done its job (ADR 0059).
                self.start = None;
                if !id.starts_with("file.new")
                    && let Some(path) = self.s.path.clone().or_else(|| v["imported"].as_str().map(PathBuf::from))
                {
                    crate::recent::captured_on_open(self, &path);
                }
                self.view.fit = true;
                self.library_sel = crate::library::first_item(&self.s.doc);
                self.decode_sounds();
                match v.get("report").and_then(Value::as_str) {
                    // An imported file: say what came across and what didn't, and that saving
                    // makes a new .anicraft document.
                    Some(r) => self.status_ok(tr!("{report} Save to keep it as an AniCraft document.", report = r)),
                    None => match v["template"].as_str() {
                        Some(t) => self.status_ok(tr!("New document from the template “{name}”", name = crate::i18n::t_dyn(t))),
                        None => self.status_ok(if id == "file.open" || id == "file.open_recent" { tr!("Opened") } else { tr!("New document") }),
                    },
                }
            }
            "file.recover" => {
                self.view.fit = true;
                self.frame_sel = None;
                self.library_sel = crate::library::first_item(&self.s.doc);
                self.recovery.restored(v["recovered"].as_str().unwrap_or(""));
                let name = v["name"].as_str().unwrap_or(tr!("Untitled"));
                self.status_ok(if v["original_newer"].as_bool() == Some(true) {
                    tr!("Restored “{name}” as a new document (the file on disk is newer). Save to keep it.", name = name)
                } else {
                    tr!("Restored “{name}”. It isn't saved yet: Save to keep it.", name = name)
                });
            }
            "file.discard_recovery" => {
                self.recovery.discarded(v["discarded"].as_str().unwrap_or(""));
                self.status_ok(tr!("Discarded the autosaved copy"));
            }
            "file.save" | "file.save_as" => {
                // The start screen's picture of it (ADR 0059).
                if let Some(saved) = v["saved"].as_str() {
                    crate::recent::capture(self, std::path::Path::new(saved));
                }
                // In a browser the file goes to the downloads (ADR 0036).
                let path = v["saved"].as_str().unwrap_or("");
                self.status_ok(if platform::WEB { tr!("Saved {path} to your downloads", path = path) } else { tr!("Saved {path}", path = path) });
                if self.close_after_save {
                    self.allow_close = true;
                }
            }
            "file.import" | "file.import_to_library" if v.get("sound").is_some() => {
                let id = anicraft_engine::doc::SoundId(v["sound"].as_u64().unwrap_or(0));
                self.library_sel = Some(LibraryItem::Sound(id));
                let name = v["name"].as_str().unwrap_or("");
                let secs = format!("{:.2}", v["seconds"].as_f64().unwrap_or(0.0));
                self.status_ok(if v.get("layer").is_some() {
                    tr!("{name} ({secs} s) is on a new layer from frame {frame}. Set its sync in Properties. {undo} to undo.", name = name, secs = secs, frame = self.s.frame + 1, undo = self.undo_key())
                } else {
                    tr!("{name} ({secs} s) is in the Library. Select a keyframe and pick it under Sound in Properties.", name = name, secs = secs)
                });
            }
            "file.import_to_library" if v.get("symbol").is_some() => {
                // A drawing: a graphic symbol (ADR 0065), the Library's current item.
                self.library_sel = v["symbol"].as_u64().map(|id| LibraryItem::Symbol(anicraft_engine::doc::SymbolId(id)));
                self.status_ok(tr!("Added the graphic symbol “{name}” to the Library. Drag it to the stage to place it.", name = v["name"].as_str().unwrap_or("")));
            }
            "file.import_to_library" => self.status_ok(tr!("Image added to the Library")),
            "file.save_as_template" | "file.delete_template" => crate::templates::announce(self, id, v),
            "file.export_audio" => {
                let n = v["sounds"].as_u64().unwrap_or(0);
                let (path, secs) = (v["written"].as_str().unwrap_or(""), format!("{:.2}", v["seconds"].as_f64().unwrap_or(0.0)));
                self.status_ok(if n == 0 { tr!("Exported {path} ({secs} s). No sounds play in that range, so the file is silent.", path = path, secs = secs) } else { tr!("Exported {path} ({secs} s).", path = path, secs = secs) });
            }
            "sound.delete" => self.status_ok(tr!("Deleted sound — {undo} to undo", undo = self.undo_key())),
            "find.replace" => {
                let n = v["replaced"].as_u64().unwrap_or(0);
                let skipped = v["skipped"].as_u64().unwrap_or(0);
                self.status_ok(if skipped > 0 {
                    trn!("Replaced {n} place, left {skipped} as it was — {undo} to undo", "Replaced {n} places, left {skipped} as they were — {undo} to undo", n, skipped = skipped, undo = self.undo_key())
                } else {
                    trn!("Replaced {n} place — {undo} to undo", "Replaced {n} places — {undo} to undo", n, undo = self.undo_key())
                });
            }
            "file.import" => {
                if let Some(b) = self.s.doc.bitmaps.last().filter(|b| v["bitmap"].as_u64() == Some(b.id.0)) {
                    self.library_sel = Some(LibraryItem::Bitmap(b.id));
                }
                let n = v["elements"].as_u64().unwrap_or(0);
                let report = v["report"].as_str().unwrap_or("");
                let undo = tr!("{undo} to undo.", undo = self.undo_key());
                // A drawing with clip paths or masks comes in as a folder of layers (ADR 0021).
                let placed = match (v["layers"].as_u64().unwrap_or(1), v["masks"].as_u64().unwrap_or(0)) {
                    (0 | 1, _) => trn!("{n} element on a new layer.", "{n} elements on a new layer.", n),
                    (l, 1) => trn!("{n} element in a new folder of {layers} layers with a mask.", "{n} elements in a new folder of {layers} layers with a mask.", n, layers = l),
                    (l, m) => trn!("{n} element in a new folder of {layers} layers with {masks} masks.", "{n} elements in a new folder of {layers} layers with {masks} masks.", n, layers = l, masks = m),
                };
                // Three sentences: what came in, the importer's report, how to undo.
                self.status_ok(format!("{placed} {report} {undo}"));
            }
            "file.export_sprite_sheet" => self.status_ok(trn!(
                "Exported {n} frame to {path} ({columns}×{rows} grid) and its frame map {map}",
                "Exported {n} frames to {path} ({columns}×{rows} grid) and its frame map {map}",
                v["frames"].as_u64().unwrap_or(0),
                path = v["written"].as_str().unwrap_or(""),
                columns = v["columns"],
                rows = v["rows"],
                map = v["map"].as_str().unwrap_or("")
            )),
            "file.export_html" => {
                // The exporter's report, when there is one, is a sentence of its own after.
                let report = v["report"].as_str().filter(|r| !r.is_empty()).map(|r| format!(" {r}")).unwrap_or_default();
                let kb = v["bytes"].as_u64().unwrap_or(0).div_ceil(1024);
                let done = trn!("Exported {path} ({n} frame, {kb} KB): open it in a web browser.", "Exported {path} ({n} frames, {kb} KB): open it in a web browser.", v["frames"].as_u64().unwrap_or(0), path = v["written"].as_str().unwrap_or(""), kb = kb);
                self.status_ok(format!("{done}{report}"));
            }
            "file.export_apng" | "file.export_webp" => {
                // Frames identical to the one before are kept once, with a longer delay.
                let kb = v["bytes"].as_u64().unwrap_or(0).div_ceil(1024);
                let n = v["frames"].as_u64().unwrap_or(0);
                let stored = v["stored_frames"].as_u64().unwrap_or(n);
                let path = v["written"].as_str().unwrap_or("");
                let done = trn!("Exported {path} ({n} frame, {kb} KB).", "Exported {path} ({n} frames, {kb} KB).", n, path = path, kb = kb);
                let held = if stored < n { format!(" {}", tr!("Frames that repeat the one before are stored once, shown longer.")) } else { String::new() };
                self.status_ok(format!("{done}{held}"));
            }
            "file.export_lottie" => {
                let report = v["report"].as_str().filter(|r| !r.is_empty()).map(|r| format!(" {r}")).unwrap_or_default();
                let done = trn!("Exported {path} ({n} frame, {layers} layers).", "Exported {path} ({n} frames, {layers} layers).", v["frames"].as_u64().unwrap_or(0), path = v["written"].as_str().unwrap_or(""), layers = v["layers"]);
                self.status_ok(format!("{done}{report}"));
            }
            "file.export_png" | "file.export_png_sequence" | "file.export_gif" | "file.export_svg" | "file.export_video" => self.status_ok(tr!("Exported {path}", path = v.get("written").or_else(|| v.get("dir")).and_then(Value::as_str).unwrap_or(""))),
            // Undo labels are the engine's English, translated when a catalog has them.
            "edit.undo" => self.status_ok(tr!("Undid {action}", action = crate::i18n::t_dyn(v["undone"].as_str().unwrap_or("")))),
            "edit.redo" => self.status_ok(tr!("Redid {action}", action = crate::i18n::t_dyn(v["redone"].as_str().unwrap_or("")))),
            "edit.delete" => self.status_ok(trn!("Deleted {n} element — {undo} to undo", "Deleted {n} elements — {undo} to undo", selected, undo = self.undo_key())),
            "layer.delete" => self.status_ok(tr!("Deleted layer — {undo} to undo", undo = self.undo_key())),
            "draw.erase_all" => {
                let n = v["erased"].as_u64().unwrap_or(0);
                self.status_ok(trn!("Erased everything at frame {frame} ({n} element). {undo} to undo.", "Erased everything at frame {frame} ({n} elements). {undo} to undo.", n, frame = self.s.frame + 1, undo = self.undo_key()));
            }
            "scene.delete" => {
                self.frame_sel = None;
                self.status_ok(tr!("Deleted scene — {undo} to undo", undo = self.undo_key()));
            }
            "scene.new" | "scene.duplicate" | "scene.select" | "scene.next" | "scene.prev" => {
                // Another scene's timeline: frame selections and scroll belong to the old one.
                self.frame_sel = None;
                self.tl_scroll = 0.0;
                self.playing = false;
                let (n, of) = (v["scene"].as_u64().unwrap_or(0) + 1, v["scenes"].as_u64().unwrap_or(1));
                // The scene's name is document text, as written.
                self.status_ok(tr!("{name} (scene {number} of {count})", name = v["name"].as_str().unwrap_or(""), number = n, count = of));
            }
            "timeline.clear_keyframe" => {
                let n = v["cleared"].as_u64().unwrap_or(1);
                self.status_ok(if n == 1 { tr!("Cleared keyframe — {undo} to undo", undo = self.undo_key()).to_string() } else { trn!("Cleared {n} keyframe — {undo} to undo", "Cleared {n} keyframes — {undo} to undo", n, undo = self.undo_key()) });
            }
            "timeline.copy_frames" | "timeline.cut_frames" | "timeline.paste_frames" => {
                let n = v["copied"].as_u64().or_else(|| v["cut"].as_u64()).or_else(|| v["pasted"].as_u64()).unwrap_or(0);
                self.status_ok(match id {
                    "timeline.copy_frames" => trn!("Copied {n} frame", "Copied {n} frames", n),
                    "timeline.cut_frames" => trn!("Cut {n} frame — {undo} to undo", "Cut {n} frames — {undo} to undo", n, undo = self.undo_key()),
                    _ => trn!("Pasted {n} frame — {undo} to undo", "Pasted {n} frames — {undo} to undo", n, undo = self.undo_key()),
                });
            }
            "plugin.run" | "plugin.install" | "plugin.remove" | "plugin.reload" => crate::plugins_ui::status(self, id, v),
            "timeline.remove_frame" => {
                let n = v["removed"].as_u64().unwrap_or(1);
                self.status_ok(if n == 1 { tr!("Removed frame — {undo} to undo", undo = self.undo_key()).to_string() } else { trn!("Removed {n} frame — {undo} to undo", "Removed {n} frames — {undo} to undo", n, undo = self.undo_key()) });
            }
            _ => {}
        }
    }

    /// Invokes a command from a menu, shortcut, palette or context menu, asking for a path or
    /// name first when it needs one.
    pub fn invoke(&mut self, id: &'static str) {
        // Ctrl+X/C/V act on frames when the timeline was used last and has frames to act on
        // (Animate's Ctrl+Alt+X/C/V work anywhere).
        let id: &'static str = match id {
            "edit.cut" if self.timeline_focus && self.frame_sel.is_some() => "timeline.cut_frames",
            "edit.copy" if self.timeline_focus && self.frame_sel.is_some() => "timeline.copy_frames",
            "edit.paste" if self.timeline_focus && !self.s.frame_clipboard.is_empty() => "timeline.paste_frames",
            other => other,
        };
        if let Some(why) = platform::unavailable(id) {
            self.status_err(crate::i18n::t(why));
            return;
        }
        // Edit Multiple Frames (ADR 0047, 0051) selects and edits on every keyframe between the
        // markers: the Edit menu, Align and Transform act there.
        if crate::onion::editing(self) && crate::onion::invoke(self, id) {
            return;
        }
        if id == "timeline.paste_frames"
            && let Some(p) = timeline::paste_params(self)
        {
            self.request(id, p);
            return;
        }
        if id == "file.recovery_list" {
            if !self.recovery.rescan() {
                return; // a browser answers on a later frame (autosave::tick)
            }
            self.recovery.show = !self.recovery.found.is_empty();
            if !self.recovery.show {
                self.status_ok(if self.recovery.active() { tr!("No autosaved work to recover") } else { tr!("Autosave is off on this system, so there is nothing to recover") });
            }
            return;
        }
        if matches!(id, "file.open_recent" | "file.recent_list") {
            // The recent files with their pictures (the File menu lists them too).
            crate::start::open(self);
            return;
        }
        if id == "file.new_from_template" {
            // The start screen, on its templates (ADR 0065).
            crate::start::open(self);
            if let Some(st) = self.start.as_mut() {
                st.show_templates = true;
            }
            self.status_ok(tr!("Pick a template to start a new document from. Esc goes back to the document."));
            return;
        }
        if id == "file.save_as_template" {
            crate::templates::open_save(self);
            return;
        }
        if id == "tween.add_hint" {
            crate::shape_hints::add(self);
            return;
        }
        if id == "find.search" {
            // Edit › Find and Replace (Ctrl+F): its window (headless, the command searches).
            crate::find_panel::open(self);
            return;
        }
        if id == "publish.settings_get" {
            // File › Publish Settings (Ctrl+Shift+F12): its window (headless, the command
            // returns the settings).
            crate::publish_panel::open(self);
            return;
        }
        if id == "control.test_movie" {
            // Control › Test Movie: the player in its own window (headless, the command plays
            // the movie and reports).
            crate::test_movie::open(self);
            return;
        }
        if id == "plugin.list" {
            // Commands › Manage Plug-ins (headless, the same command lists them).
            self.show_plugins = true;
            return;
        }
        if id == "layer.set_properties" {
            // Modify › Timeline › Layer Properties: the dialog for the active layer.
            crate::layer_props::open(self, None);
            return;
        }
        // View › Grid › Edit Grid and View › Guides › Edit Guides, Add Guide (ADR 0076): their
        // dialogs (headless, the commands take the settings).
        if id == "view.grid_set" {
            crate::aids_dialog::open_grid(self);
            return;
        }
        if matches!(id, "view.guides_set" | "view.guide_add") {
            crate::aids_dialog::open_guides(self, id == "view.guide_add");
            return;
        }
        if id == "view.guides_clear" && !self.s.guides().is_empty() {
            let n = self.s.guides().len();
            if self.run(id, json!({})).is_some() {
                self.status_ok(trn!("Cleared {n} guide. {undo} to undo.", "Cleared {n} guides. {undo} to undo.", n, undo = self.undo_key()));
            }
            return;
        }
        if id == "modify.envelope" {
            // The envelope's handles, with the Free Transform tool (ADR 0080).
            crate::shape_envelope::open(self);
            return;
        }
        if id == "modify.convert_to_symbol" {
            // Name, type and Library folder (ADR 0053).
            crate::convert_symbol::open(self);
            return;
        }
        if id == "edit.paste" {
            // Paste in the centre of what you are looking at.
            let c = self.view_center_stage();
            self.request(id, json!({"x": c.x, "y": c.y}));
            return;
        }
        let prompt = match id {
            "file.open" => Some((tr_noop!("Open document"), "path", "animation.anicraft".to_string())),
            "file.save" if self.s.path.is_none() => Some((tr_noop!("Save document as"), "path", "animation.anicraft".to_string())),
            "file.save_as" => Some((tr_noop!("Save document as"), "path", self.s.path.as_ref().map_or_else(|| "animation.anicraft".to_string(), |p| p.display().to_string()))),
            "layer.rename" => Some((tr_noop!("Rename layer"), "name", self.s.active_layer().map_or_else(String::new, |l| l.name.clone()))),
            "scene.rename" => Some((tr_noop!("Rename scene"), "name", self.s.doc.scenes.get(self.s.scene).map_or_else(String::new, |s| s.name.clone()))),
            "file.export_png" => Some((tr_noop!("Export frame as PNG"), "path", format!("frame_{:04}.png", self.s.frame + 1))),
            "file.export_png_sequence" => Some((tr_noop!("Export PNG sequence to folder"), "dir", "frames".to_string())),
            "file.export_gif" => Some((tr_noop!("Export animated GIF"), "path", "animation.gif".to_string())),
            "file.export_apng" => Some((tr_noop!("Export animated PNG (full colour, transparent)"), "path", "animation.png".to_string())),
            "file.export_webp" => Some((tr_noop!("Export animated WebP (lossless, transparent)"), "path", "animation.webp".to_string())),
            "file.export_svg" => Some((tr_noop!("Export frame as SVG"), "path", format!("frame_{:04}.svg", self.s.frame + 1))),
            "file.export_sprite_sheet" => Some((tr_noop!("Export sprite sheet (PNG + JSON frame map)"), "path", "sprites.png".to_string())),
            "file.export_video" => Some((tr_noop!("Export video (.webm or .mp4, made with your ffmpeg)"), "path", "animation.webm".to_string())),
            "file.export_lottie" => Some((tr_noop!("Export Lottie animation (JSON, for web and apps)"), "path", "animation.json".to_string())),
            "file.export_html" => Some((tr_noop!("Export HTML canvas (one web page)"), "path", "movie.html".to_string())),
            "file.export_audio" => Some((tr_noop!("Export audio (the movie's soundtrack as a WAV)"), "path", "soundtrack.wav".to_string())),
            "file.import_to_library" => Some((tr_noop!("Import to library (WAV, OGG or MP3 sound; PNG or JPEG image)"), "path", "sound.wav".to_string())),
            "file.import" => Some((tr_noop!("Import to stage (SVG, PNG or JPEG; WAV, OGG or MP3 onto a new layer)"), "path", "drawing.svg".to_string())),
            "plugin.install" => Some((tr_noop!("Install plug-in (.wasm file)"), "path", "plugin.wasm".to_string())),
            // A brush set (ADR 0068): your brushes as a small JSON file.
            "brush.preset_export" => Some((tr_noop!("Export your brushes (a brush set file)"), "path", "brushes.json".to_string())),
            "brush.preset_import" => Some((tr_noop!("Import brushes (a brush set file)"), "path", "brushes.json".to_string())),
            // Filter presets (ADR 0091): yours as a small JSON file.
            "filter.preset_export" => Some((tr_noop!("Export your filter presets (a preset file)"), "path", "filter-presets.json".to_string())),
            "filter.preset_import" => Some((tr_noop!("Import filter presets (a preset file)"), "path", "filter-presets.json".to_string())),
            // The document's swatches (ADR 0078): a swatch file, or a GIMP palette to import.
            "swatch.export" => Some((tr_noop!("Export swatches (a swatch file)"), "path", "swatches.json".to_string())),
            "swatch.import" => Some((tr_noop!("Import swatches (a swatch file or a GIMP palette)"), "path", "swatches.json".to_string())),
            _ => None,
        };
        if let Some((title, key, text)) = prompt {
            let is_file = matches!(key, "path" | "dir") && (id.starts_with("file.") || matches!(id, "plugin.install" | "brush.preset_export" | "brush.preset_import" | "filter.preset_export" | "filter.preset_import" | "swatch.export" | "swatch.import"));
            if is_file && platform::WEB && platform::reads_file(id) {
                // The browser's file picker (ADR 0036). The file arrives on a later frame
                // (`open_picked`); a save or export asks for the download's name below.
                self.playing = false;
                platform::pick_file(id);
                return;
            }
            if is_file && self.prefs.native_dialogs && self.shot.is_none() && !platform::WEB {
                // The platform's own dialog (it blocks, as native dialogs do). Cancel = no-op.
                self.playing = false;
                #[cfg(not(target_arch = "wasm32"))]
                match platform::file_dialog(id, crate::i18n::t(title), &text) {
                    Some(path) => self.request(id, json!({ key: path.display().to_string() })),
                    None => self.close_after_save = false,
                }
                return;
            }
            self.prompt = Some(Prompt { title, cmd: id, key, text, focus: true });
            return;
        }
        // F5, F6 and F7 work as in Animate: at the playhead with no frame selection, F5 extends
        // every layer that reaches it; on one frame the playhead moves on to what was made.
        if let Some(p) = timeline::insert_key_params(self, id) {
            self.request(id, p);
            timeline::follow_playhead(self);
            return;
        }
        // Frame commands act on the frame selection (F5 over 3 frames on 2 layers inserts 3
        // frames on each).
        if timeline::RANGE_COMMANDS.contains(&id)
            && let Some(p) = timeline::range_params(self)
        {
            self.request(id, p);
            return;
        }
        self.request(id, json!({}));
    }

    /// The stage point at the centre of the visible canvas.
    pub fn view_center_stage(&self) -> egui::Vec2 {
        let size = egui::vec2(self.s.doc.stage.width as f32, self.s.doc.stage.height as f32);
        size * 0.5 - self.view.pan / self.view.zoom.max(1e-3)
    }

    pub fn zoom_about(&mut self, canvas: egui::Rect, screen: egui::Pos2, new_zoom: f32) {
        let new_zoom = new_zoom.clamp(0.05, 64.0);
        let size = egui::vec2(self.s.doc.stage.width as f32, self.s.doc.stage.height as f32);
        let origin = canvas.center() + self.view.pan - size * self.view.zoom * 0.5;
        let stage_pt = (screen - origin) / self.view.zoom;
        let new_origin = screen - stage_pt * new_zoom;
        self.view.pan = new_origin - canvas.center() + size * new_zoom * 0.5;
        self.view.zoom = new_zoom;
    }

    pub fn toggle_play(&mut self) {
        self.playing = !self.playing;
        self.play_acc = 0.0;
        if self.playing && self.s.frame + 1 >= self.s.frame_count() {
            self.s.set_frame(0);
        }
        self.start_sound();
    }

    /// The playhead as a movie frame (scenes in order).
    fn movie_frame(&self) -> u32 {
        self.s.doc.scene_start(self.s.scene).saturating_add(self.s.frame)
    }

    /// Starts the sound from the playhead, or stops it; the playhead follows the sound's clock
    /// while it plays.
    fn start_sound(&mut self) {
        self.audio_clock = self.playing && self.shot.is_none() && self.sound.play(&self.s.doc, self.s.state, self.movie_frame());
        if !self.playing {
            self.sound.stop();
        }
        // Said once: the device could not be opened, so playback is silent.
        if let Some(e) = self.sound.error.take() {
            self.status_err(tr!("Playing without sound: {e}", e = e));
        }
    }

    /// Decodes the document's sounds on a background thread (each once; every copy of the
    /// document shares the result), so opening a file never waits on it.
    pub fn decode_sounds(&self) {
        let sounds = self.s.doc.sounds.clone();
        if !sounds.is_empty() {
            let _ = std::thread::Builder::new().name("anicraft-decode".into()).spawn(move || {
                for x in sounds {
                    let _ = x.decoded();
                }
            });
        }
    }

    /// Scrubbing: a moved playhead plays a frame's worth of the stream sounds there.
    fn scrub_sound(&mut self) {
        let here = (self.s.scene, self.s.frame);
        if !self.playing && self.shot.is_none() && self.heard.is_some_and(|h| h != here) {
            self.sound.scrub(&self.s.doc, self.s.state, self.movie_frame());
        }
        self.heard = Some(here);
    }

    fn tick_playback(&mut self, ctx: &egui::Context) {
        if !self.playing {
            if self.audio_clock {
                self.sound.stop();
                self.audio_clock = false;
            }
            return;
        }
        let n = self.s.frame_count();
        if self.audio_clock {
            // Picture follows sound: the frame is wherever the sound has got to (edits made
            // while playing are heard at once).
            self.sound.sync(&self.s.doc, self.s.state);
            if let Some(mf) = self.sound.frame(self.s.doc.stage.fps) {
                let local = mf.saturating_sub(self.s.doc.scene_start(self.s.scene));
                if local >= n {
                    if self.looping {
                        self.s.set_frame(0);
                        self.start_sound();
                    } else {
                        self.s.set_frame(n.saturating_sub(1));
                        self.playing = false;
                        self.start_sound();
                    }
                } else if local != self.s.frame {
                    self.s.set_frame(local);
                }
                self.heard = Some((self.s.scene, self.s.frame));
                ctx.request_repaint();
                return;
            }
            // The device went away: carry on silently on the wall clock.
            self.audio_clock = false;
            self.play_acc = 0.0;
        }
        let fps = f64::from(self.s.doc.stage.fps.max(1.0));
        self.play_acc += f64::from(ctx.input(|i| i.stable_dt)).min(0.25);
        let n = self.s.frame_count();
        while self.play_acc >= 1.0 / fps {
            self.play_acc -= 1.0 / fps;
            if self.s.frame + 1 >= n {
                if self.looping {
                    self.s.set_frame(0);
                } else {
                    self.playing = false;
                    break;
                }
            } else {
                self.s.set_frame(self.s.frame + 1);
            }
        }
        self.heard = Some((self.s.scene, self.s.frame));
        ctx.request_repaint();
    }

    /// Ctrl+F6 / Ctrl+Shift+F6: keyboard focus to the next or previous panel (Stage, Timeline,
    /// Properties, Library, Tools). Cut, copy and paste follow it to frames or artwork.
    pub fn focus_panel(&mut self, ctx: &egui::Context, forward: bool) {
        match crate::a11y::cycle_focus(ctx, self.last_panel, forward) {
            Some(region) => {
                self.last_panel = Some(region);
                match region {
                    crate::a11y::Region::Timeline => self.timeline_focus = true,
                    crate::a11y::Region::Stage => self.timeline_focus = false,
                    _ => {}
                }
                let next = self.keymap.primary("view.next_panel").map(platform::shortcut_label).unwrap_or_default();
                let panel = crate::i18n::t(region.label());
                self.status_ok(if next.is_empty() { panel.to_string() } else { tr!("{panel} · {next} for the next panel", panel = panel, next = next) });
            }
            None => self.status_err(tr!("No panel to move to yet")),
        }
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        // While the shortcut editor records a key, every key press belongs to it.
        if self.modal_open() || self.key_capture.is_some() {
            return;
        }
        // Alt+letter opens a menu (ADR 0064), before the tool letters: egui would match those
        // with Alt held too.
        let alt_bound = |k: Key| self.shortcuts.iter().any(|(m, key, _)| *key == k && m.alt && !m.ctrl && !m.command && !m.shift);
        let mut menu_keys = std::mem::take(&mut self.menu_keys);
        menu_keys.access_key(ctx, alt_bound);
        self.menu_keys = menu_keys;
        // Palette, shortcut sheet, preferences and quit work even while a text field has focus.
        // Preferences: Ctrl+U (the shortcut Animate users know) and Ctrl/⌘+, (platform norm).
        let (palette, sheet, prefs_key, quit) = ctx.input_mut(|i| {
            // The shortcut sheet first: Ctrl+Alt+Shift+K (Animate's Keyboard Shortcuts) must not
            // be taken by Ctrl+K, which egui would match with the extra modifiers ignored.
            let sheet = i.consume_key(Modifiers::COMMAND | Modifiers::ALT | Modifiers::SHIFT, Key::K) || i.consume_key(Modifiers::NONE, Key::F1);
            (
                i.consume_key(Modifiers::COMMAND, Key::K) || i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::P),
                sheet,
                i.consume_key(Modifiers::COMMAND, Key::U) || i.consume_key(Modifiers::COMMAND, Key::Comma),
                i.consume_key(Modifiers::COMMAND, Key::Q),
            )
        });
        if prefs_key {
            self.open_prefs();
        }
        if quit {
            // Goes through the close guard, so unsaved work is never lost.
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if palette {
            self.palette = if self.palette.is_some() { None } else { Some(dialogs::Palette::default()) };
        }
        if sheet {
            self.show_shortcuts = !self.show_shortcuts;
        }
        // Moving between panels (Ctrl+F6) works from anywhere, a text field included.
        let panel_step = ctx.input_mut(|i| {
            self.shortcuts.iter().filter(|(_, _, id)| matches!(*id, "view.next_panel" | "view.prev_panel")).find_map(|(m, k, id)| i.consume_key(*m, *k).then_some(*id == "view.next_panel"))
        });
        if let Some(forward) = panel_step {
            self.focus_panel(ctx, forward);
        }
        // Who has the keyboard (ADR 0022): nothing focused, or the stage or timeline, gets every
        // shortcut. A focused control (button, checkbox, menu) keeps Enter, Space and the arrows
        // for itself (press, move to the next control) but the other shortcuts still work. A
        // text field keeps every key.
        let keys = crate::a11y::keyboard(ctx);
        if keys == crate::a11y::Keyboard::Text || self.palette.is_some() {
            return;
        }
        // The Test Movie window's movie has the keyboard (ADR 0052): its keys are the
        // movie's; only shortcuts with Ctrl (⌘) or Alt reach the app (Restart is Ctrl+Enter).
        if self.test_movie.is_some() && crate::test_movie::has_keys(ctx) {
            let fire: Vec<&'static str> = ctx.input_mut(|i| self.shortcuts.iter().filter(|(m, _, _)| m.command || m.ctrl || m.alt).filter_map(|(m, k, id)| i.consume_key(*m, *k).then_some(*id)).collect());
            for id in fire.into_iter().filter(|id| !id.starts_with("view.") && !id.starts_with("tool.")) {
                self.invoke(id);
            }
            return;
        }
        let control = keys == crate::a11y::Keyboard::Control;
        if !control {
            crate::motion_path::consume_keys(self, ctx);
            pathtools::consume_keys(self, ctx);
            crate::widthtool::consume_keys(self, ctx);
            crate::bone_tool::consume_keys(self, ctx);
            crate::warp_tool::consume_keys(self, ctx);
        }
        crate::library::consume_keys(self, ctx);
        crate::ease_editor::consume_keys(self, ctx);
        let mut fire: Vec<&'static str> = Vec::new();
        let mut tool = None;
        let mut play = false;
        let mut nudge = egui::Vec2::ZERO;
        let surface_focused = ctx.memory(|m| m.focused().is_some());
        // On the focused timeline the arrows move the playhead (Left, Right) and the active
        // layer (Up, Down) instead of nudging artwork (ADR 0064).
        let on_timeline = surface_focused && crate::a11y::focused_region(ctx) == Some(crate::a11y::Region::Timeline);
        let (mut frame_step, mut layer_step) = (0i64, 0i64);
        ctx.input_mut(|i| {
            for (m, k, id) in &self.shortcuts {
                if i.consume_key(*m, *k) {
                    fire.push(id);
                }
            }
            if control {
                return;
            }
            if i.consume_key(Modifiers::NONE, Key::Enter) {
                play = true;
            }
            // Space pans (held, `key_down`); its press must not "click" a focused stage.
            if surface_focused {
                i.consume_key(Modifiers::NONE, Key::Space);
            }
            // Screen pixels, as in Animate: a press moves less the further you zoom in.
            let step = if i.modifiers.shift { 10.0 } else { 1.0 } / self.view.zoom.max(1e-3);
            for (k, d) in [(Key::ArrowLeft, egui::vec2(-step, 0.0)), (Key::ArrowRight, egui::vec2(step, 0.0)), (Key::ArrowUp, egui::vec2(0.0, -step)), (Key::ArrowDown, egui::vec2(0.0, step))] {
                if on_timeline && i.consume_key(Modifiers::NONE, k) {
                    match k {
                        Key::ArrowLeft => frame_step -= 1,
                        Key::ArrowRight => frame_step += 1,
                        Key::ArrowUp => layer_step -= 1,
                        _ => layer_step += 1,
                    }
                } else if i.key_pressed(k) && !i.modifiers.command {
                    nudge += d;
                }
            }
        });
        for id in fire {
            match id {
                // View-state keys (Animate's), handled here rather than in the engine.
                "view.fit" => self.view.fit = true,
                "view.onion" => crate::onion::toggle_ghosts(self),
                "view.align_to_stage" => {
                    self.align_to_stage = !self.align_to_stage;
                    self.status_ok(if self.align_to_stage { tr!("Align: to stage") } else { tr!("Align: to the selection") });
                }
                "view.select_all_frames" => {
                    let layers = &self.s.timeline().layers;
                    if let (Some(a), Some(b)) = (layers.first(), layers.last()) {
                        let last = self.s.frame_count().saturating_sub(1);
                        self.frame_sel = Some(timeline::FrameSel { anchor: (a.id, 0), head: (b.id, last) });
                        self.timeline_focus = true;
                        self.run("timeline.goto", json!({"frame": 0}));
                    }
                }
                "view.actions" => self.toggle_panel(crate::dock::Panel::Actions),
                "view.find_next" => crate::find_panel::step(self, true),
                "view.find_prev" => crate::find_panel::step(self, false),
                "view.movie_explorer" => crate::movie_explorer::open(self),
                "view.brush_smaller" => crate::brushtool::resize(self, false),
                "view.brush_larger" => crate::brushtool::resize(self, true),
                id if id.starts_with("tool.") => tool = keymap::tool_for(id).or(tool),
                _ => self.invoke(id),
            }
        }
        if let Some(t) = tool {
            self.tool = t;
        }
        if frame_step != 0 {
            self.playing = false;
            let last = i64::from(self.s.frame_count().saturating_sub(1));
            let to = (i64::from(self.s.frame) + frame_step).clamp(0, last.max(0));
            self.run("timeline.goto", json!({"frame": to}));
        }
        if layer_step != 0 {
            let layers = &self.s.timeline().layers;
            if let Some(at) = layers.iter().position(|l| l.id == self.s.layer) {
                let to = (at as i64 + layer_step).clamp(0, layers.len() as i64 - 1);
                if to != at as i64 && self.run("layer.select", json!({"index": to})).is_some() {
                    // The keyboard goes with it, to the new active row.
                    self.tl_refocus = true;
                }
            }
        }
        if play {
            self.toggle_play();
        }
        if nudge != egui::Vec2::ZERO && crate::onion::editing(self) {
            crate::onion::nudge(self, anicraft_engine::geom::Vec2::new(nudge.x, nudge.y));
        } else if nudge != egui::Vec2::ZERO && !self.s.selection.is_empty() {
            // One undo step: the move, and raw shapes merging where they land (ADR 0031).
            let own = !self.s.in_gesture();
            if own {
                self.s.begin_gesture(tr_noop!("Move")); // i18n-exempt: an undo label (the engine's English, translated where shown)
            }
            self.run("element.translate", json!({"dx": nudge.x, "dy": nudge.y}));
            self.drop_raw();
            if own {
                self.s.end_gesture();
            }
        }
    }

    fn handle_screenshot(&mut self, ctx: &egui::Context) {
        let Some(shot) = &mut self.shot else { return };
        shot.frames += 1;
        ctx.request_repaint();
        let image = ctx.input(|i| {
            i.raw.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(img) = image {
            let img = match shot.width {
                Some(w) => shrink(&img, w as usize),
                None => (*img).clone(),
            };
            match write_screenshot(&shot.path, &img) {
                Ok(()) => println!("screenshot saved to {}", shot.path.display()),
                Err(e) => eprintln!("screenshot failed: {e}"),
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        if shot.frames >= SHOT_FRAMES && !shot.requested {
            shot.requested = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        if shot.frames > SHOT_FRAMES + 600 {
            eprintln!("screenshot: the renderer never delivered a frame");
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    fn update_title(&mut self, ctx: &egui::Context) {
        let name = self.s.path.as_ref().and_then(|p| p.file_name()).map_or_else(|| tr!("Untitled").to_string(), |n| n.to_string_lossy().into_owned());
        let title = format!("AniCraft — {name}{}", if self.s.is_dirty() { " •" } else { "" }); // i18n-exempt: the product name and the file's
        if title != self.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = title;
        }
    }

    fn prompt_window(&mut self, ctx: &egui::Context) {
        let Some(p) = &mut self.prompt else { return };
        let mut submit = false;
        let mut cancel = false;
        let title = crate::i18n::t(p.title);
        egui::Window::new(title).id(egui::Id::new(("prompt", p.title))).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, -80.0]).show(ctx, |ui| {
            ui.set_min_width(360.0);
            let r = ui.add(egui::TextEdit::singleline(&mut p.text).desired_width(f32::INFINITY)).named(title);
            if p.focus {
                r.request_focus();
                p.focus = false;
            }
            if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                submit = true;
            }
            ui.horizontal(|ui| {
                if ui.button(tr!("OK")).clicked() {
                    submit = true;
                }
                if ui.button(tr!("Cancel")).clicked() || ui.input(|i| i.key_pressed(Key::Escape)) {
                    cancel = true;
                }
            });
        });
        if submit {
            if let Some(p) = self.prompt.take() {
                // Window › Workspace › Save Workspace… is the shell's (ADR 0075).
                if p.cmd == "view.save_workspace" {
                    self.save_workspace(&p.text);
                    return;
                }
                // Through `request`, so opening over unsaved work or saving over a file asks first.
                self.request(p.cmd, json!({ p.key: p.text }));
            }
        } else if cancel {
            self.prompt = None;
            self.close_after_save = false;
        }
    }

    fn about_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_about;
        let shown = egui::Window::new(tr!("About AniCraft")).id(egui::Id::new("about_window")).open(&mut open).collapsible(false).resizable(false).show(ctx, |ui| {
            ui.add(egui::Image::new((app_mark(ctx).id(), egui::vec2(64.0, 64.0))));
            ui.heading("AniCraft");
            ui.label(tr!("Version {version} · {stage}", version = env!("CARGO_PKG_VERSION"), stage = crate::i18n::t(RELEASE_STAGE)));
            ui.label(tr!("Open-source 2D vector animation studio, written in Rust."));
            ui.label(trn!("{n} engine command — all available headless via anicraft-cli.", "{n} engine commands — all available headless via anicraft-cli.", commands().len()));
            ui.colored_label(T.text_dim, "MIT OR Apache-2.0"); // i18n-exempt: licence identifiers
            ui.label(theme::text(theme::Type::Caption, tr!("Typefaces: {fonts}", fonts = theme::font_summary())));
        });
        crate::a11y::name_title_bar(&shown, tr!("About AniCraft"));
        self.show_about = open;
    }

    /// Help › Project Files: where the docs, source and saved files live, with buttons to open them.
    fn files_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_files;
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let entries: [(&str, PathBuf); 12] = [
            (tr!("Project folder"), root.clone()),
            (tr!("Requirements & technology"), root.join("docs").join("requirements.md")),
            (tr!("Architecture philosophy"), root.join("docs").join("architecture.md")),
            (tr!("Roadmap"), root.join("docs").join("roadmap.md")),
            (tr!("Performance & budgets"), root.join("docs").join("performance.md")),
            (tr!("Decision records (ADRs)"), root.join("docs").join("adr")),
            (tr!("Legal compliance"), root.join("docs").join("legal").join("compliance.md")),
            ("README", root.join("README.md")),
            (tr!("Agent / contributor guide"), root.join("AGENTS.md")),
            (tr!("Engine crates (source)"), root.join("crates")),
            (tr!("Desktop app (source)"), root.join("src")),
            (tr!("Screenshots"), root.join("shots")),
        ];
        let mut opened: Option<Result<(), String>> = None;
        let shown = egui::Window::new(tr!("Project Files")).id(egui::Id::new("files_window")).open(&mut open).collapsible(false).resizable(false).default_width(620.0).show(ctx, |ui| {
            ui.label(theme::text(theme::Type::Label, tr!("Where AniCraft's documents and files are on this computer")));
            ui.add_space(4.0);
            egui::Grid::new("files").num_columns(3).spacing([10.0, 6.0]).striped(true).show(ui, |ui| {
                for (label, path) in &entries {
                    ui.label(theme::text(theme::Type::Body, *label).strong());
                    ui.label(theme::text(theme::Type::Path, path.display().to_string()).color(if path.exists() { T.text } else { T.text_faint }));
                    if ui.add_enabled(path.exists(), egui::Button::new(tr!("Open"))).named(&tr!("Open {label}", label = label)).clicked() {
                        opened = Some(open_in_system(path));
                    }
                    ui.end_row();
                }
                let doc = self.s.path.as_ref().map_or_else(|| tr!("not saved yet (File › Save)").to_string(), |p| p.display().to_string());
                ui.label(theme::text(theme::Type::Body, tr!("Current document")).strong());
                ui.label(theme::text(theme::Type::Path, doc));
                ui.end_row();
            });
            ui.add_space(4.0);
            ui.label(theme::text(theme::Type::Caption, tr!("Saved documents and exports go where you type in the save box; a plain name lands in the folder AniCraft was started from.")));
        });
        crate::a11y::name_title_bar(&shown, tr!("Project Files"));
        match opened {
            Some(Err(e)) => self.status_err(e),
            Some(Ok(())) => self.status_ok(tr!("Opened in your system's file viewer")),
            None => {}
        }
        self.show_files = open;
    }
}

/// The app icon (`assets/app-icon/`; the PNG carries its rounded corners) as a texture, decoded on
/// first use and kept in the context. 128 px stays sharp at 64 pt on a 2× display.
pub(crate) fn app_mark(ctx: &egui::Context) -> egui::TextureHandle {
    const PNG: &[u8] = include_bytes!("../assets/app-icon/hicolor/128x128/apps/ai.storyteller.anicraft.png");
    let id = egui::Id::new("anicraft-app-mark");
    if let Some(tex) = ctx.data(|d| d.get_temp::<egui::TextureHandle>(id)) {
        return tex;
    }
    let (w, h, rgba) = anicraft_engine::doc::decode_png(PNG).map_or_else(|| (1, 1, vec![0; 4]), |p| (p.width as usize, p.height as usize, p.rgba));
    let image = egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba);
    let options = egui::TextureOptions { mipmap_mode: Some(egui::TextureFilter::Linear), ..egui::TextureOptions::LINEAR };
    let tex = ctx.load_texture("anicraft-app-mark", image, options);
    ctx.data_mut(|d| d.insert_temp(id, tex.clone()));
    tex
}

/// Writes a window's screenshot as a PNG (screenshot runs), making its folder if needed.
pub(crate) fn write_screenshot(path: &std::path::Path, img: &egui::ColorImage) -> Result<(), String> {
    let rgba: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_srgba_unmultiplied()).collect();
    let png = anicraft_render::encode_png(img.size[0] as u32, img.size[1] as u32, &rgba).map_err(|e| e.to_string())?;
    if let Some(dir) = path.parent()
        && !dir.as_os_str().is_empty()
    {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(path, png).map_err(|e| e.to_string())
}

/// A screenshot scaled down to `width` pixels across (kept when it is already narrower), each
/// pixel the area-weighted mean of the ones it covers: sharp text, no aliasing.
pub(crate) fn shrink(img: &egui::ColorImage, width: usize) -> egui::ColorImage {
    let [w, h] = img.size;
    if width == 0 || width >= w || h == 0 {
        return img.clone();
    }
    let height = ((h * width + w / 2) / w).max(1);
    // One axis at a time: per target pixel, the source pixels it covers and how much.
    let taps = |from: usize, to: usize| -> Vec<Vec<(usize, f32)>> {
        let scale = from as f32 / to as f32;
        (0..to)
            .map(|i| {
                let (a, b) = (i as f32 * scale, (i + 1) as f32 * scale);
                let mut v = Vec::new();
                let mut j = a.floor() as usize;
                while (j as f32) < b && j < from {
                    let cover = (b.min(j as f32 + 1.0) - a.max(j as f32)).max(0.0) / scale;
                    v.push((j, cover));
                    j += 1;
                }
                v
            })
            .collect()
    };
    let (tx, ty) = (taps(w, width), taps(h, height));
    let src: Vec<[f32; 4]> = img.pixels.iter().map(|c| c.to_srgba_unmultiplied().map(f32::from)).collect();
    let mix = |acc: &mut [f32; 4], p: &[f32; 4], wgt: f32| {
        for (a, v) in acc.iter_mut().zip(p) {
            *a += v * wgt;
        }
    };
    let mut rows = vec![[0.0f32; 4]; width * h];
    for y in 0..h {
        for (x, t) in tx.iter().enumerate() {
            let mut acc = [0.0f32; 4];
            for &(sx, wgt) in t {
                if let Some(p) = src.get(y * w + sx) {
                    mix(&mut acc, p, wgt);
                }
            }
            if let Some(o) = rows.get_mut(y * width + x) {
                *o = acc;
            }
        }
    }
    let mut rgba = Vec::with_capacity(width * height * 4);
    for t in &ty {
        for x in 0..width {
            let mut acc = [0.0f32; 4];
            for &(sy, wgt) in t {
                if let Some(p) = rows.get(sy * width + x) {
                    mix(&mut acc, p, wgt);
                }
            }
            rgba.extend(acc.map(|v| v.round().clamp(0.0, 255.0) as u8));
        }
    }
    egui::ColorImage::from_rgba_unmultiplied([width, height], &rgba)
}

/// Opens a file or folder with the operating system's default handler.
pub(crate) fn open_in_system(path: &std::path::Path) -> Result<(), String> {
    // Tests click every control (`i18n::tests`): they never start another program.
    if cfg!(test) {
        return Ok(());
    }
    let mut cmd = if cfg!(target_os = "windows") {
        let mut c = std::process::Command::new("explorer");
        c.arg(path);
        c
    } else if cfg!(target_os = "macos") {
        let mut c = std::process::Command::new("open");
        c.arg(path);
        c
    } else {
        let mut c = std::process::Command::new("xdg-open");
        c.arg(path);
        c
    };
    cmd.spawn().map(|_| ()).map_err(|e| tr!("could not open {path}: {e}", path = path.display(), e = e))
}

impl eframe::App for App {
    /// Screenshots only: a click where `shot_click` says, pressed and released over two frames
    /// before they begin, as a person's would be, then a scroll to the end of what it opened.
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        let Some(at) = self.shot_click.get() else { return };
        // Press, release, then scroll what opened to its end (a menu's last items).
        self.shot_step += 1;
        match self.shot_step {
            1 | 2 => {
                raw.events.push(egui::Event::PointerMoved(at));
                raw.events.push(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: self.shot_step == 1, modifiers: Modifiers::NONE });
            }
            3 => {}
            _ => {
                let over = at - egui::vec2(0.0, 120.0);
                raw.events.push(egui::Event::PointerMoved(over));
                raw.events.push(egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Point, delta: egui::vec2(0.0, -2000.0), modifiers: Modifiers::NONE, phase: egui::TouchPhase::Move });
                self.shot_click.set(None);
                self.shot_step = 0;
            }
        }
        ctx.request_repaint();
    }

    /// A clean exit (saved, or the user chose Don't Save) removes this session's recovery
    /// copy; otherwise the work is written once more and offered at the next start.
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        // The Test Movie window's place, for next time (ADR 0062).
        crate::test_movie::remember_place(self);
        // And the floating panels' windows (ADR 0075).
        crate::dock_ui::remember(self);
        let clean = !self.s.is_dirty() || self.allow_close || self.shot.is_some();
        self.recovery.finish(&self.s, clean);
        #[cfg(not(target_arch = "wasm32"))]
        self.updates.finish();
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.frame(ui);
        // A few frames drawn: the window started (#207).
        crate::startup::frame();
    }
}

impl App {
    /// One frame of the whole interface. Needs no window, so tests run it headless.
    pub fn frame(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        #[cfg(not(target_arch = "wasm32"))]
        {
            let safe = crate::updates::safe_to_install(self);
            if self.updates.show(&ctx, safe) {
                return;
            }
        }
        self.handle_screenshot(&ctx);
        self.poll_font_scan(&ctx);
        self.follow_system_look(&ctx);
        if let Some(p) = platform::picked_file() {
            self.open_picked(p);
        }
        // Documents the system handed over while running (macOS: Finder, the Dock; ADR 0062).
        for r in platform::take_requests() {
            self.system_request(&ctx, r);
        }
        if self.glyph_check {
            // Fonts installed last frame are live now: can they draw the macOS key symbols?
            let ok = ctx.fonts_mut(|f| f.has_glyphs(&theme::font(theme::Type::Key), "⌘⇧⌥"));
            platform::set_mac_glyphs(ok);
            self.glyph_check = false;
        }
        self.press_scripted_key(&ctx);
        // The colour pickers show the document's swatches (ADR 0078).
        crate::swatches_panel::publish_to_pickers(&ctx, self);
        self.handle_shortcuts(&ctx);
        timeline::announce(self, &ctx);
        // Switching away from a path tool keeps its work: a Pen path in progress is finished.
        if self.tool != Tool::Pen && self.pen.is_some() {
            pathtools::commit_pen(self, false);
        }
        if self.tool != Tool::Subselect {
            self.points.clear();
            self.raw_anchors.clear();
            self.raw_ends.clear();
        }
        pathtools::upkeep(self);
        crate::motion_path::upkeep(self);
        crate::widthtool::upkeep(self);
        crate::bone_tool::upkeep(self);
        crate::warp_tool::upkeep(self);
        // An open button shows its hit area over its other states, in the theme's colour (ADR 0057).
        self.s.hit_area = Some(theme::hit_area());
        self.tick_playback(&ctx);
        self.scrub_sound();
        self.update_title(&ctx);
        if let Some(st) = &mut self.status
            && st.at == 0.0 {
                st.at = ctx.input(|i| i.time);
            }

        let bar = |fill| egui::Frame::NONE.fill(fill).inner_margin(egui::Margin::symmetric(8, 2));
        crate::drop::begin_frame(self);
        egui::Panel::top("menu_bar").frame(bar(T.chrome)).show(ui, |ui| menus::bar(self, ui));
        if self.start.is_some() {
            // The start screen takes the workspace's place (ADR 0059); the menus stay.
            egui::Panel::bottom("status_bar").exact_size(28.0).frame(bar(T.chrome)).show(ui, |ui| panels::status_bar(self, ui));
            egui::CentralPanel::default().frame(egui::Frame::NONE.fill(T.pasteboard)).show(ui, |ui| crate::start::show(self, ui));
        } else {
            self.workspace(ui);
        }
        self.windows(&ctx);
    }

    /// The editor's panels (ADR 0075): the edit bar and status bar, then the tools, the docks
    /// and the stage.
    fn workspace(&mut self, ui: &mut egui::Ui) {
        let bar = |fill| egui::Frame::NONE.fill(fill).inner_margin(egui::Margin::symmetric(8, 2));
        egui::Panel::top("edit_bar").exact_size(theme::TARGET + 8.0).frame(bar(T.panel)).show(ui, |ui| panels::edit_bar(self, ui));
        egui::Panel::bottom("status_bar").exact_size(28.0).frame(bar(T.chrome)).show(ui, |ui| panels::status_bar(self, ui));
        crate::dock_ui::workspace(self, ui);
    }

    /// Dialogs and floating windows, the close guard and the end-of-frame upkeep.
    fn windows(&mut self, ctx: &egui::Context) {
        let ctx = ctx.clone();
        crate::drop::frame(self, &ctx);
        self.prompt_window(&ctx);
        crate::convert_symbol::window(self, &ctx);
        crate::aids_dialog::window(self, &ctx);
        crate::templates::save_window(self, &ctx);
        crate::layer_props::window(self, &ctx);
        self.about_window(&ctx);
        dialogs::show_all(self, &ctx);
        autosave::window(self, &ctx);
        crate::plugins_ui::show(self, &ctx);
        crate::plugin_export_ui::show(self, &ctx);
        crate::dock_float::floating(self, &ctx);
        crate::workspaces_window::show(self, &ctx);
        crate::find_panel::show(self, &ctx);
        crate::brush_panel::show(self, &ctx);
        crate::publish_panel::show(self, &ctx);
        crate::test_movie::show(self, &ctx);
        crate::envelope::show(self, &ctx);
        crate::ease_editor::show(self, &ctx);

        // Never lose work on close: intercept it while there are unsaved changes.
        if self.allow_close {
            if self.close_after_save || self.close_dialog {
                self.close_after_save = false;
                self.close_dialog = false;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        } else if ctx.input(|i| i.viewport().close_requested()) && self.shot.is_none() && self.s.is_dirty() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.playing = false;
            self.close_dialog = true;
        }
        self.files_window(&ctx);

        // Close a coalesced edit (property drags) once nothing is being dragged or typed.
        // (A plug-in dialog keeps its preview gesture open until OK or Cancel.)
        if self.s.in_gesture() && self.plugin_dialog.is_none() && self.drag == Drag::None && !ctx.egui_is_using_pointer() && ctx.memory(|m| m.focused().is_none()) {
            self.s.end_gesture();
        }

        // A changed arrangement of the panels, into the preferences (ADR 0075).
        crate::dock_ui::save_when_settled(self, &ctx);
        // After this frame's edits: start a recovery copy when one is due (off this thread).
        autosave::tick(self, &ctx);
        // The graphics driver's trouble: autosave at once, or close to open again (ADR 0092).
        #[cfg(not(target_arch = "wasm32"))]
        crate::gpu_watch::tick(self, &ctx);
        // Open Recent changed (an open, a save, a pin): into the preferences.
        crate::recent::sync(self);
        crate::brush_panel::sync(self);
        crate::panels::sync_filter_presets(self);
        crate::rulers::sync(self);
        // A browser asks before leaving a page with unsaved work (its close guard).
        platform::set_unsaved(self.s.is_dirty());

        // An edit late in this frame (after the stage drew) must be shown without waiting for
        // the next input event: egui only repaints on demand. This was the "drag commits after a
        // pause" bug.
        if self.s.revision != self.last_revision {
            self.last_revision = self.s.revision;
            ctx.request_repaint();
        }
    }
}

#[cfg(test)]
mod shrink_tests {
    use super::shrink;
    use egui::{Color32, ColorImage};

    #[test]
    fn shrinking_averages_the_area_and_keeps_the_aspect() {
        // 4 × 2: two black columns, two white: halved, one black pixel and one white per row.
        let px = [Color32::BLACK, Color32::BLACK, Color32::WHITE, Color32::WHITE];
        let img = ColorImage::new([4, 2], px.iter().chain(px.iter()).copied().collect());
        let out = shrink(&img, 2);
        assert_eq!(out.size, [2, 1]);
        assert_eq!(out.pixels, vec![Color32::BLACK, Color32::WHITE]);
        // A third of the way: the middle pixel mixes both halves.
        let wide = ColorImage::new([3, 3], [Color32::BLACK, Color32::from_gray(90), Color32::WHITE].repeat(3));
        let two = shrink(&wide, 2);
        assert_eq!(two.size, [2, 2]);
        assert_eq!(two.pixels[0], Color32::from_gray(30));
        // Never enlarged.
        assert_eq!(shrink(&img, 10).size, [4, 2]);
    }
}

#[cfg(test)]
mod prompt_enter_tests {
    use super::*;

    #[test]
    fn enter_confirms_layer_rename() {
        let ctx = egui::Context::default();
        let session = anicraft_engine::Session::new(anicraft_engine::doc::sample::demo());
        let mut app = App::with_context(&ctx, session, Some(std::path::PathBuf::from("unused.png")));
        app.prompt = Some(Prompt { title: "Rename layer", cmd: "layer.rename", key: "name", text: "Renamed".into(), focus: true });
        let input = |events| egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 940.0))), events, ..Default::default() };
        for _ in 0..3 {
            ctx.run_ui(input(vec![]), |_| app.prompt_window(&ctx)).textures_delta.clear();
        }
        let enter = egui::Event::Key { key: Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
        ctx.run_ui(input(vec![enter]), |_| app.prompt_window(&ctx)).textures_delta.clear();
        assert!(app.prompt.is_none(), "Enter closes the rename prompt");
        assert_eq!(app.s.active_layer().unwrap().name, "Renamed");
    }
}
