//! The stage-2 real-machine camera gate (parity plan › stage 2): Moho Pro 14.4 fixtures
//! with animated camera channels (Lua gen_* scripts, INTERP_LINEAR), rendered by Moho's
//! own FileRender onto transparency at the document resolution. This test renders the
//! *imported* documents through the CPU oracle at the same size and asks whether the
//! artwork landed in the same place. The renderers differ in AA and in gradient fills the
//! importer honestly drops, so raw pixel diffs are meaningless; instead each content mask
//! (alpha) is aligned to the other by translation (±300px, 4px steps) and the best-shift
//! F1 overlap is measured. A wrong zoom or roll cannot be fixed by any translation and
//! collapses the overlap; the best shift reads out any position error directly.
//!
//! Fixtures (all corpus-guarded, machine-local):
//! - keyframes-roll.moho / keyframes-zoom.moho: camera-only isolation (static canvas).
//! - keyframes-layer.moho: layer translation + rotation channels AND the camera moving
//!   together (the parity plan's gate fixture).

use anicraft_mohoproj_import::import_path;
use std::path::{Path, PathBuf};

fn corpus() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("MAOHOU_FIXTURES") {
        let p = PathBuf::from(dir);
        if p.is_dir() {
            return Some(p);
        }
    }
    let beside = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../maop-corpus/maop");
    beside.is_dir().then_some(beside)
}

/// Downsampled boolean content mask (alpha > 24), cell = `step` px, majority vote.
fn content_mask(px: &[u8], w: u32, h: u32, step: u32) -> (u32, u32, Vec<bool>) {
    let gw = w / step;
    let gh = h / step;
    let mut m = vec![false; (gw * gh) as usize];
    for gy in 0..gh {
        for gx in 0..gw {
            let mut on = 0u32;
            for dy in 0..step {
                for dx in 0..step {
                    let a = px[(((gy * step + dy) * w + gx * step + dx) * 4 + 3) as usize];
                    if a > 24 {
                        on += 1;
                    }
                }
            }
            // Any content lights the cell: thin strokes must survive or the stroke-only
            // side of the comparison disappears.
            m[(gy * gw + gx) as usize] = on > 0;
        }
    }
    (gw, gh, m)
}

/// Best translation (within ±`range` cells, `step`-px granularity) aligning `a` onto `b`,
/// by F1 overlap. Returns (dx_px, dy_px, f1).
fn best_shift(a: &[bool], b: &[bool], gw: u32, gh: u32, range: u32) -> (i64, i64, f64) {
    let (mut best_dx, mut best_dy) = (0i64, 0i64);
    let mut best_f1 = -1f64;
    let count = |dx: i64, dy: i64| -> f64 {
        // Overlap of `a` translated by (dx, dy) with `b`: cells outside either mask count
        // towards the denominators, so a mask can't win by shrinking out of view.
        let mut inter = 0u64;
        let mut na = 0u64;
        let mut nb = 0u64;
        for y in 0..gh as i64 {
            for x in 0..gw as i64 {
                let vb = b[(y as u32 * gw + x as u32) as usize];
                let ax = x - dx;
                let ay = y - dy;
                let va = ax >= 0 && ax < gw as i64 && ay >= 0 && ay < gh as i64 && a[(ay as u32 * gw + ax as u32) as usize];
                if va {
                    na += 1;
                }
                if vb {
                    nb += 1;
                }
                if va && vb {
                    inter += 1;
                }
            }
        }
        if na + nb == 0 {
            return 0.0;
        }
        2.0 * inter as f64 / (na + nb) as f64
    };
    for dy in -(range as i64)..=(range as i64) {
        for dx in -(range as i64)..=(range as i64) {
            let f1 = count(dx, dy);
            if f1 > best_f1 {
                best_f1 = f1;
                best_dx = dx;
                best_dy = dy;
            }
        }
    }
    (best_dx, best_dy, best_f1)
}

fn straight_rgba(ours: &anicraft_render::Pixmap) -> Vec<u8> {
    let mut rgba = Vec::with_capacity((ours.width * ours.height * 4) as usize);
    for p in &ours.data {
        let a = p[3];
        let un = |c: f32| if a > 0.0 { (c / a).clamp(0.0, 1.0) } else { 0.0 };
        rgba.extend_from_slice(&[(un(p[0]) * 255.0) as u8, (un(p[1]) * 255.0) as u8, (un(p[2]) * 255.0) as u8, (a * 255.0) as u8]);
    }
    rgba
}

#[test]
fn imported_camera_render_matches_moho_frames() {
    let Some(dir) = corpus() else {
        eprintln!("skipped: no maop-corpus beside the workspace");
        return;
    };

    for (fixture, prefix, _label) in [
        ("keyframes-roll.moho", "keyframes-roll", "roll 0..20deg only"),
        ("keyframes-zoom.moho", "keyframes-zoom", "zoom 2..4 only"),
        ("keyframes-layer.moho", "keyframes-layer", "layer channels + camera"),
    ] {
        let path = dir.join(fixture);
        if !path.is_file() {
            eprintln!("skipped {fixture}: not in the corpus");
            continue;
        }
        let (doc, _report) = import_path(&path).unwrap_or_else(|e| panic!("{fixture}: {e}"));
        let (w, h) = (doc.stage.width, doc.stage.height);
        let step = 4u32;
        for (name, frame) in [("f0", 0u32), ("f12", 12), ("f24", 24)] {
            let Ok(moho_png) = std::fs::read(dir.join(format!("inputs/{prefix}-{name}.png"))) else {
                eprintln!("skipped {fixture} {name}: no Moho render");
                continue;
            };
            let moho = anicraft_doc::decode_png(&moho_png).unwrap();
            assert_eq!((moho.width, moho.height), (w, h), "FileRender uses the document resolution");
            let (gw, gh, m_mask) = content_mask(&moho.rgba, w, h, step);

            let dl = anicraft_doc::evaluate(&doc, frame);
            let view = anicraft_render::View { transform: anicraft_geom::Affine::IDENTITY, width: w, height: h, background: None };
            let ours = anicraft_render::render(&dl, &view).unwrap().0;
            let rgba = straight_rgba(&ours);
            let (_, _, o_mask) = content_mask(&rgba, w, h, step);

            let (dx, dy, f1) = best_shift(&o_mask, &m_mask, gw, gh, 75);
            println!("{fixture} {name}: best shift ({}, {}) cells = ({}px, {}px), aligned F1 {:.3} ({}x{} grid, {}px cells)", dx, dy, dx * step as i64, dy * step as i64, f1, gw, gh, step);
            // Calibrated against these very fixtures (numbers in CASES.md): with the
            // correct camera the best shift stays within ~4 cells of the per-fixture
            // baseline (content-size bias) and F1 holds 0.23+; with a deliberately wrong
            // zoom model (2/zoom) F1 collapsed to 0.11-0.20 and shifts ran to ±296px.
            let why = format!("{fixture} {name}: content best-shift ({}, {}) cells ({}px, {}px) — the imported camera misplaced the frame", dx, dy, dx * step as i64, dy * step as i64);
            assert!(dx.abs() <= 15 && dy.abs() <= 15, "{why}");
            let why = format!("{fixture} {name}: aligned overlap F1 {f1:.3} — the imported camera does not reproduce Moho's frame");
            assert!(f1 > 0.18, "{why}");
        }
    }
}
