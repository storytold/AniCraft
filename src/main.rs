//! AniCraft desktop app.
//!
//! ```text
//! anicraft [file.anicraft] [--screenshot out.png] [--renderer wgpu|gl] [--open <panel>]
//!          [--theme graphite|high-contrast] [--scale <percent>]
//! anicraft --version
//! ```
//! `--screenshot` starts normally, draws ~120 frames, saves the window as PNG and exits.
//!
//! Built for `wasm32-unknown-unknown` the same program is the web editor (`src/web.rs`,
//! ADR 0036, docs/web.md).
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
// Release builds are Windows GUI programs, so no console window opens beside the app (the
// packaging checks the PE subsystem, ADR 0026). Debug builds keep the console for logs; the
// headless front end is anicraft-cli.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
// The browser build leaves out what only the desktop has (screenshot runs, the recovery folder,
// window chrome, `--open` panels); that code stays compiled for it but unused (ADR 0036).
#![cfg_attr(target_arch = "wasm32", allow(dead_code))]

mod a11y;
mod actions_panel;
mod app;
mod autosave;
mod brushtool;
mod envelope;
mod erasertool;
mod camera_tool;
mod dialogs;
mod gpu_stage;
mod icons;
mod keymap;
mod menus;
mod motion_editor;
mod panels;
mod motion_path;
mod pathtools;
mod platform;
mod plugins_ui;
mod prefs;
mod sound;
mod shape_hints;
mod stage;
mod test_movie;
mod theme;
mod timeline;
#[cfg(target_arch = "wasm32")]
mod web;
mod widthtool;

#[cfg(not(target_arch = "wasm32"))]
use anicraft_engine::Session;
#[cfg(not(target_arch = "wasm32"))]
use std::path::{Path, PathBuf};

/// The session to start with: the file given on the command line (through `file.open`, so
/// `.fla` / `.xfl` import works the same as from the menu), else the sample. Also the import
/// report of an Animate document, for the status bar.
#[cfg(not(target_arch = "wasm32"))]
fn open_session(file: Option<&Path>) -> (Session, Option<String>) {
    let Some(p) = file else { return (Session::new(anicraft_engine::doc::sample::demo()), None) };
    let mut s = Session::default();
    match s.execute("file.open", &serde_json::json!({"path": p.display().to_string()})) {
        Ok(v) => {
            let report = v.get("report").and_then(|r| r.as_str()).map(str::to_string);
            if let Some(r) = &report {
                println!("{r}");
            }
            (s, report)
        }
        Err(e) => {
            eprintln!("could not open {}: {e}; starting with the sample", p.display());
            (Session::new(anicraft_engine::doc::sample::demo()), None)
        }
    }
}

/// `--theme` / `--scale`: the look for this run only (screenshots of high contrast or 200 %),
/// over the saved preferences, without saving them.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Copy, Default)]
struct Look {
    theme: Option<theme::Palette>,
    scale: Option<f32>,
}

#[cfg(not(target_arch = "wasm32"))]
fn run(renderer: eframe::Renderer, file: Option<&Path>, shot: Option<PathBuf>, open: Option<String>, look: Look) -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: platform::viewport(egui::ViewportBuilder::default().with_title("AniCraft").with_inner_size([1600.0, 940.0]).with_min_inner_size([960.0, 600.0])),
        renderer,
        ..Default::default()
    };
    // The embedded interface font joins the text library before anything lays text out:
    // opening an Animate document from the command line lays its text out right here.
    anicraft_engine::text::Fonts::register(theme::embedded_fonts());
    let (session, report) = open_session(file);
    eframe::run_native(
        "AniCraft",
        options,
        Box::new(move |cc| {
            let mut a = app::App::new(cc, session, shot);
            a.preview_look(&cc.egui_ctx, look.theme, look.scale);
            a.open_panel(open.as_deref());
            // An imported document says what came across, as opening it from the menu does.
            if let Some(r) = report {
                a.status_ok(format!("{r} Save to keep it as an AniCraft document."));
            }
            Ok(Box::new(a))
        }),
    )
}

/// In the browser the page's script calls this once the module is loaded: the editor starts on
/// the page's canvas.
#[cfg(target_arch = "wasm32")]
fn main() {
    web::start();
}

#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result {
    let mut shot: Option<PathBuf> = None;
    let mut file: Option<PathBuf> = None;
    let mut open: Option<String> = None;
    let mut renderer_arg: Option<String> = None;
    let mut look = Look::default();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--screenshot" => match args.next() {
                Some(p) => shot = Some(PathBuf::from(p)),
                None => {
                    eprintln!("--screenshot needs a file name");
                    std::process::exit(2);
                }
            },
            "--open" => open = args.next(),
            "--renderer" => renderer_arg = args.next(),
            "--theme" => match args.next().as_deref() {
                Some("graphite") => look.theme = Some(theme::Palette::Graphite),
                Some("high-contrast") => look.theme = Some(theme::Palette::HighContrast),
                other => {
                    eprintln!("--theme takes graphite or high-contrast, not {}", other.unwrap_or("nothing"));
                    std::process::exit(2);
                }
            },
            "--scale" => match args.next().and_then(|s| s.trim_end_matches('%').parse::<f32>().ok()) {
                Some(pct) if pct.is_finite() => look.scale = Some(pct / 100.0),
                _ => {
                    eprintln!("--scale takes a percentage, such as 150");
                    std::process::exit(2);
                }
            },
            // Printed by the release packaging checks and for bug reports.
            "--version" | "-V" => {
                println!("AniCraft {} ({})", env!("CARGO_PKG_VERSION"), app::RELEASE_STAGE);
                return Ok(());
            }
            "--help" | "-h" => {
                println!(
                    "anicraft [file.anicraft|file.fla|file.xfl] [--version] [--screenshot out.png] [--renderer wgpu|gl] [--theme graphite|high-contrast] [--scale <percent>] [--open palette|shortcuts|lastscene|select|about|files|confirm|transform|points|merge|brush|brushing|eraser|erasing|eyedropper|pen|frames|span|bucket|tlzoom|tlwide|motion|motionspan|motionpath|motionpoints|motionedit|hints|hintsend|camera|cameraview|filters|prefs|licenses|zoom|recovery|plugins|actions|script|testmovie|exporthtml|sound|envelope]\n\
                     --renderer   force the GPU backend (also ANICRAFT_RENDERER); default wgpu, falling back to OpenGL\n\
                     --open       shows a panel at start (handy with --screenshot for visual checks)\n\
                     ANICRAFT_STAGE_RENDERER=gpu|cpu  stage renderer (default gpu, falling back to cpu; ADR 0032)\n\
                     --theme      the interface palette for this run (not saved)\n\
                     --scale      the interface scale for this run, 75 to 200 (not saved)\n\
                     Headless use: anicraft-cli --help"
                );
                return Ok(());
            }
            other => file = Some(PathBuf::from(other)),
        }
    }

    // wgpu first (Vulkan/Metal/DX12). If it cannot start (old GPU, VM, missing drivers, common
    // on some Linux setups) retry once with OpenGL, unless the user forced a backend.
    let forced = platform::forced_renderer(renderer_arg.as_deref());
    let first = forced.unwrap_or(eframe::Renderer::Wgpu);
    match run(first, file.as_deref(), shot.clone(), open.clone(), look) {
        Err(e) if forced.is_none() && first == eframe::Renderer::Wgpu => {
            eprintln!("GPU backend (wgpu) failed to start: {e}. Retrying with OpenGL.");
            run(eframe::Renderer::Glow, file.as_deref(), shot, open, look)
        }
        other => other,
    }
}

#[cfg(test)]
mod guidelines_tests;
#[cfg(test)]
mod architecture_tests;
#[cfg(test)]
mod packaging_tests;
