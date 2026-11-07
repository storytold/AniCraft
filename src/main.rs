//! AniCraft desktop app.
//!
//! ```text
//! anicraft [file.anicraft] [--screenshot out.png] [--renderer wgpu|gl] [--open <panel>]
//! ```
//! `--screenshot` starts normally, draws ~120 frames, saves the window as PNG and exits.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod app;
mod dialogs;
mod i18n;
mod icons;
mod keymap;
mod menus;
mod panels;
mod pathtools;
mod platform;
mod prefs;
mod stage;
mod theme;
mod timeline;

use anicraft_engine::Session;
use std::path::{Path, PathBuf};

/// The session to start with: the file given on the command line (through `file.open`, so
/// `.fla` / `.xfl` import works the same as from the menu), else the sample.
fn open_session(file: Option<&Path>) -> Session {
    let Some(p) = file else { return Session::new(anicraft_engine::doc::sample::demo()) };
    let mut s = Session::default();
    match s.execute("file.open", &serde_json::json!({"path": p.display().to_string()})) {
        Ok(v) => {
            if let Some(r) = v.get("report").and_then(|r| r.as_str()) {
                println!("{r}");
            }
            s
        }
        Err(e) => {
            eprintln!("could not open {}: {e}; starting with the sample", p.display());
            Session::new(anicraft_engine::doc::sample::demo())
        }
    }
}

fn run(renderer: eframe::Renderer, file: Option<&Path>, shot: Option<PathBuf>, open: Option<String>, lang: Option<String>) -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: platform::viewport(
            egui::ViewportBuilder::default().with_title("AniCraft").with_inner_size([1600.0, 940.0]).with_min_inner_size([960.0, 600.0]),
        ),
        renderer,
        ..Default::default()
    };
    let session = open_session(file);
    eframe::run_native(
        "AniCraft",
        options,
        Box::new(move |cc| {
            let mut a = app::App::new(cc, session, shot, lang);
            a.open_panel(open.as_deref());
            Ok(Box::new(a))
        }),
    )
}

fn main() -> eframe::Result {
    let mut shot: Option<PathBuf> = None;
    let mut file: Option<PathBuf> = None;
    let mut open: Option<String> = None;
    let mut renderer_arg: Option<String> = None;
    let mut lang: Option<String> = None;
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
            "--lang" => lang = args.next(),
            "--help" | "-h" => {
                println!(
                    "anicraft [file.anicraft] [--screenshot out.png] [--renderer wgpu|gl] [--lang en|ar|auto] [--open palette|shortcuts|lastscene|select|files|confirm|transform|points|pen|frames|span|bucket|tlzoom|tlwide|prefs|licenses|zoom]\n\
                     --renderer   force the GPU backend (also ANICRAFT_RENDERER); default wgpu, falling back to OpenGL\n\
                     --lang       the interface language for this run, over Preferences (needs CRAFT_I18N_DIR at build time)\n\
                     --open       shows a panel at start (handy with --screenshot for visual checks)\n\
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
    match run(first, file.as_deref(), shot.clone(), open.clone(), lang.clone()) {
        Err(e) if forced.is_none() && first == eframe::Renderer::Wgpu => {
            eprintln!("GPU backend (wgpu) failed to start: {e}. Retrying with OpenGL.");
            run(eframe::Renderer::Glow, file.as_deref(), shot, open, lang)
        }
        other => other,
    }
}

#[cfg(test)]
mod architecture_tests;
#[cfg(test)]
mod guidelines_tests;
