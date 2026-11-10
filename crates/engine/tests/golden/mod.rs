//! Golden image checks (ADR 0035): a render is compared with a committed PNG, pixel by pixel,
//! within a tolerance.
//!
//! * `UPDATE_GOLDENS=1 cargo test …` writes the renders as the new goldens (look at them before
//!   committing: a golden is only as right as the eyes that accepted it).
//! * A missing golden fails (CI never makes its own), and says how to make it.
//! * On a mismatch the render and a diff image (the golden faded, with the pixels that differ
//!   in red, brighter the more they differ) are written under Cargo's temporary folder for
//!   integration tests, and the failure names both paths.

use std::path::{Path, PathBuf};

/// How far a render may stray from its golden: a pixel differs when any channel (RGBA, 0-255,
/// not premultiplied) is off by more than `channel`; the check fails when more than `pixels`
/// (a fraction of the image) differ. The default absorbs floating-point differences between
/// computers on anti-aliased edges, and catches a shape moved by a pixel or a colour changed
/// anywhere it covers.
#[derive(Clone, Copy, Debug)]
pub struct Tolerance {
    pub channel: u8,
    pub pixels: f32,
}

pub const DEFAULT: Tolerance = Tolerance { channel: 3, pixels: 0.002 };

/// Whether this run writes goldens instead of checking them.
pub fn updating() -> bool {
    std::env::var("UPDATE_GOLDENS").is_ok_and(|v| v.trim() == "1")
}

/// Where failed checks leave their render and diff images.
fn out_dir() -> PathBuf {
    Path::new(env!("CARGO_TARGET_TMPDIR")).join("golden-diffs")
}

fn decode(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    let mut reader = png::Decoder::new(bytes).read_info().map_err(|e| e.to_string())?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).map_err(|e| e.to_string())?;
    if info.color_type != png::ColorType::Rgba || info.bit_depth != png::BitDepth::Eight {
        return Err(format!("expected 8-bit RGBA, found {:?} {:?}", info.color_type, info.bit_depth));
    }
    buf.truncate(info.buffer_size());
    Ok((info.width, info.height, buf))
}

fn write(path: &Path, width: u32, height: u32, rgba: &[u8]) {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).unwrap();
    }
    std::fs::write(path, anicraft_render::encode_png(width, height, rgba).unwrap()).unwrap();
}

/// Checks `rgba` (`width` × `height`, straight alpha) against the golden at `golden`, or writes
/// it there when updating. `name` names the leftovers of a failure.
pub fn check(golden: &Path, name: &str, width: u32, height: u32, rgba: &[u8], tol: Tolerance) -> Result<(), String> {
    if updating() {
        assert_eq!(rgba.len(), (width * height * 4) as usize);
        write(golden, width, height, rgba);
        return Ok(());
    }
    compare(golden, name, width, height, rgba, tol)
}

/// The comparison [`check`] makes when it isn't updating.
fn compare(golden: &Path, name: &str, width: u32, height: u32, rgba: &[u8], tol: Tolerance) -> Result<(), String> {
    assert_eq!(rgba.len(), (width * height * 4) as usize);
    let keep = |why: String| -> String {
        let actual = out_dir().join(format!("{name}.actual.png"));
        write(&actual, width, height, rgba);
        format!("{name}: {why}\n  golden: {}\n  render: {}", golden.display(), actual.display())
    };
    let bytes = std::fs::read(golden).map_err(|e| keep(format!("no golden ({e}); run with UPDATE_GOLDENS=1 to make it, then look at it")))?;
    let (gw, gh, want) = decode(&bytes).map_err(|e| keep(format!("unreadable golden: {e}")))?;
    if (gw, gh) != (width, height) {
        return Err(keep(format!("the golden is {gw}×{gh}, the render {width}×{height}")));
    }
    let mut bad = 0usize;
    let mut worst = 0u8;
    let mut diff = Vec::with_capacity(rgba.len());
    for (a, b) in rgba.as_chunks::<4>().0.iter().zip(want.as_chunks::<4>().0) {
        let d = a.iter().zip(b).map(|(x, y)| x.abs_diff(*y)).max().unwrap_or(0);
        worst = worst.max(d);
        if d > tol.channel {
            bad += 1;
            diff.extend_from_slice(&[128u8.saturating_add(d / 2), 0, 0, 255]);
        } else {
            // The golden, faded towards white, for context.
            let fade = |c: u8| 192 + c / 4;
            diff.extend_from_slice(&[fade(b[0]), fade(b[1]), fade(b[2]), 255]);
        }
    }
    let allowed = (tol.pixels * (width * height) as f32).floor() as usize;
    if bad > allowed {
        let diff_path = out_dir().join(format!("{name}.diff.png"));
        write(&diff_path, width, height, &diff);
        return Err(format!("{}\n  diff:   {}", keep(format!("{bad} pixels differ (at most {allowed} may), by up to {worst}")), diff_path.display()));
    }
    Ok(())
}

#[test]
fn the_check_passes_within_the_tolerance_and_fails_past_it() {
    let dir = out_dir().join("self-test");
    let golden = dir.join("g.png");
    let (w, h) = (20, 10);
    let base: Vec<u8> = (0..w * h).flat_map(|i| [i as u8, 100, 200, 255]).collect();
    write(&golden, w, h, &base);
    assert!(compare(&golden, "self-test-same", w, h, &base, DEFAULT).is_ok());
    // Small differences everywhere are within the channel tolerance.
    let near: Vec<u8> = base.iter().map(|c| c.saturating_sub(2)).collect();
    assert!(compare(&golden, "self-test-near", w, h, &near, DEFAULT).is_ok());
    // A whole row changed is not.
    let mut off = base.clone();
    for px in off.as_chunks_mut::<4>().0.iter_mut().take(w as usize) {
        px[1] = 0;
    }
    let err = compare(&golden, "self-test-off", w, h, &off, DEFAULT).unwrap_err();
    assert!(err.contains("20 pixels differ") && err.contains("self-test-off.diff.png"), "{err}");
    assert!(out_dir().join("self-test-off.diff.png").exists() && out_dir().join("self-test-off.actual.png").exists());
    // A missing golden fails and says how to make one.
    let err = compare(&dir.join("none.png"), "self-test-missing", w, h, &base, DEFAULT).unwrap_err();
    assert!(err.contains("UPDATE_GOLDENS=1"), "{err}");
}
