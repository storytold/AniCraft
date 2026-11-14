//! AniCraft desktop app.
//!
//! ```text
//! anicraft [file.anicraft] [--screenshot out.png] [--renderer wgpu|gl] [--open <panel>]
//!          [--theme dark|light|high-contrast] [--scale <percent>] [--lang <code>|auto|pseudo]
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

// First, so its `tr!` macros are in scope in every module after it.
#[macro_use]
mod i18n;
mod a11y;
mod actions_panel;
mod aids_dialog;
mod app;
mod autosave;
mod brush_panel;
mod brushtool;
mod colour;
mod colour_panel;
mod envelope;
mod find_panel;
mod publish_panel;
mod erasertool;
mod bind_tool;
mod warp_tool;
mod bone_tool;
mod camera_tool;
mod convert_symbol;
mod layer_depth;
mod layer_props;
mod dialogs;
mod dock;
mod dock_float;
mod dock_ui;
mod drop;
mod ease_editor;
mod gpu_stage;
#[cfg(not(target_arch = "wasm32"))]
mod gpu_watch;
mod gradient_tool;
mod icons;
mod keymap;
mod library;
mod menus;
#[cfg(not(target_arch = "wasm32"))]
mod updates;
mod mixed;
mod motion_editor;
mod movie_explorer;
mod panels;
mod motion_path;
mod onion;
mod pathtools;
mod platform;
mod plugin_export_ui;
#[cfg(test)]
mod plugin_fixture_tests;
#[cfg(test)]
mod plugin_ui_tests;
mod plugins_ui;
mod prefs;
mod recent;
mod rulers;
mod scale_grid;
mod shape_envelope;
mod sound;
mod shape_hints;
mod stage;
mod stage_a11y;
mod swatches_panel;
mod start;
mod startup;
mod templates;
mod test_movie;
mod theme;
mod timeline;
#[cfg(target_arch = "wasm32")]
mod web;
mod widthtool;
mod workspaces_window;

#[cfg(not(target_arch = "wasm32"))]
use anicraft_engine::Session;
#[cfg(not(target_arch = "wasm32"))]
use std::path::{Path, PathBuf};

/// What opening the command line's file said, for the status bar.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, PartialEq)]
enum Opened {
    /// Nothing to report (no file, or an AniCraft document).
    Quiet,
    /// The import report of an Animate document.
    Imported(String),
    /// It didn't open (the sample is there instead): the file's name and why.
    Failed(String, String),
}

/// The session to start with: the file given on the command line (through `file.open`, so
/// `.fla` / `.xfl` import works the same as from the menu), else the sample. A file that can't
/// be opened says why in the window (#226: a release build has no console, so the reason used
/// to go nowhere and a double-clicked document seemed to open as the sample).
#[cfg(not(target_arch = "wasm32"))]
fn open_session(file: Option<&Path>) -> (Session, Opened) {
    let Some(p) = file else { return (Session::new(anicraft_engine::doc::sample::demo()), Opened::Quiet) };
    let mut s = Session::default();
    match s.execute("file.open", &serde_json::json!({"path": p.display().to_string()})) {
        Ok(v) => match v.get("report").and_then(|r| r.as_str()) {
            Some(r) => {
                startup::line(&format!("imported {}: {r}", p.display()));
                (s, Opened::Imported(r.to_string()))
            }
            None => (s, Opened::Quiet),
        },
        Err(e) => {
            startup::line(&format!("could not open {}: {e}; starting with the sample", p.display()));
            (Session::new(anicraft_engine::doc::sample::demo()), Opened::Failed(crate::recent::display_name(p), e.to_string()))
        }
    }
}

/// `--theme` / `--scale`: the look for this run only (screenshots of high contrast or 200 %),
/// over the saved preferences, without saving them.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Debug, Default, PartialEq)]
struct Look {
    theme: Option<theme::Palette>,
    scale: Option<f32>,
    /// `--lang`: a locale code, `auto` or `pseudo` (ADR 0040).
    lang: Option<String>,
    /// `--shot-width`: the screenshot scaled down to this many pixels across (the user guide's
    /// pictures, ADR 0069).
    shot_width: Option<u32>,
}

#[cfg(not(target_arch = "wasm32"))]
fn run(attempt: startup::Attempt, file: Option<&Path>, shot: Option<PathBuf>, open: Option<String>, look: Look) -> eframe::Result {
    let options = attempt.options(eframe::NativeOptions {
        viewport: platform::viewport(egui::ViewportBuilder::default().with_title("AniCraft").with_inner_size([1600.0, 940.0]).with_min_inner_size([960.0, 600.0])),
        ..Default::default()
    });
    // The embedded interface font joins the text library before anything lays text out:
    // opening an Animate document from the command line lays its text out right here.
    anicraft_engine::text::Fonts::register(theme::embedded_fonts());
    // `--lang` before the window exists, so its first status line is in that language too.
    i18n::force(look.lang.as_deref());
    let (session, report) = open_session(file);
    let file_given = file.map(Path::to_path_buf);
    eframe::run_native(
        "AniCraft",
        options,
        Box::new(move |cc| {
            // A document the system hands over while AniCraft is idle wakes it (ADR 0062).
            platform::wake_on_requests(&cc.egui_ctx);
            startup::log_adapter(cc.wgpu_render_state.as_ref());
            let starting_fresh = file_given.is_none() && shot.is_none();
            let mut a = app::App::new(cc, session, shot);
            if let Some(s) = a.shot.as_mut() {
                s.width = look.shot_width;
            }
            if a.shot.is_none() {
                a.show_update_outcome_on_start();
            }
            // No document to open: the start screen, unless it was switched off (ADR 0059).
            if starting_fresh && a.prefs.start_screen {
                start::open(&mut a);
            }
            a.preview_look(&cc.egui_ctx, look.theme, look.scale);
            a.open_panel(open.as_deref());
            match report {
                // An imported document says what came across, as opening it from the menu does.
                Opened::Imported(r) => a.status_ok(tr!("{report} Save to keep it as an AniCraft document.", report = r)),
                Opened::Failed(name, e) => a.status_err(tr!("Could not open {name}: {e}", name = name, e = e)),
                Opened::Quiet => {}
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

/// What the command line asks for. A plain argument is the document to open: how Windows
/// (`"%1"` in the MSI's file association) and Linux (`Exec=anicraft %f` in the desktop entry)
/// hand over a double-clicked document. macOS sends it as an Apple Event instead (ADR 0062).
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Default, PartialEq)]
struct Args {
    shot: Option<PathBuf>,
    file: Option<PathBuf>,
    open: Option<String>,
    renderer: Option<String>,
    look: Look,
    /// `--version` / `--help`: print and leave.
    print: Option<Print>,
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, PartialEq)]
enum Print {
    Version,
    Help,
}

/// Reads the arguments (without the program name). `Err` is the message for a bad one.
#[cfg(not(target_arch = "wasm32"))]
fn parse_args(args: impl IntoIterator<Item = String>) -> Result<Args, String> {
    let mut out = Args::default();
    let mut args = args.into_iter();
    while let Some(a) = args.next() {
        match a.as_str() {
            "--screenshot" => out.shot = Some(PathBuf::from(args.next().ok_or("--screenshot needs a file name")?)), // i18n-exempt: a console message
            "--open" => out.open = args.next(),
            "--renderer" => out.renderer = args.next(),
            "--shot-width" => match args.next().and_then(|s| s.parse::<u32>().ok()) {
                Some(w) if (64..=8192).contains(&w) => out.look.shot_width = Some(w),
                _ => return Err("--shot-width takes a width in pixels, from 64 to 8192".into()), // i18n-exempt: a console message
            },
            "--theme" => match args.next().as_deref() {
                Some("graphite" | "dark") => out.look.theme = Some(theme::Palette::Graphite),
                Some("light") => out.look.theme = Some(theme::Palette::Light),
                Some("high-contrast") => out.look.theme = Some(theme::Palette::HighContrast),
                other => return Err(format!("--theme takes dark (graphite), light or high-contrast, not {}", other.unwrap_or("nothing"))), // i18n-exempt: a console message
            },
            "--scale" => match args.next().and_then(|s| s.trim_end_matches('%').parse::<f32>().ok()) {
                Some(pct) if pct.is_finite() => out.look.scale = Some(pct / 100.0),
                _ => return Err("--scale takes a percentage, such as 150".into()), // i18n-exempt: a console message
            },
            "--lang" => match args.next() {
                Some(code) if i18n::known(&code) => out.look.lang = Some(code),
                other => {
                    let offered: Vec<&str> = i18n::offered().into_iter().map(|(c, _)| c).collect();
                    return Err(format!("--lang takes auto, pseudo or a language this build has ({}), not {}", offered.join(", "), other.as_deref().unwrap_or("nothing"))); // i18n-exempt: a console message
                }
            },
            // Printed by the release packaging checks and for bug reports.
            "--version" | "-V" => out.print = Some(Print::Version),
            "--help" | "-h" => out.print = Some(Print::Help),
            // Not a file: what older macOS added when Finder started an app.
            other if platform::is_system_argument(other) => {}
            other => out.file = Some(PathBuf::from(other)),
        }
    }
    Ok(out)
}

/// `--help` (the console's text, in English).
#[cfg(not(target_arch = "wasm32"))]
// i18n-exempt: the --help console text
const HELP: &str = "anicraft [file.anicraft|file.fla|file.xfl] [--version] [--screenshot out.png [--shot-width <px>]] [--renderer gl|wgpu|vulkan|dx12|metal] [--theme graphite|high-contrast] [--scale <percent>] [--lang <code>|auto|pseudo] [--open palette|shortcuts|lastscene|start|drop|droplibrary|select|selectzoom|about|files|confirm|transform|points|merge|rawparts|rawedit|rawsub|rawpoints|rawsnap|rulers|editguides|editgrid|symboledit|groups|combine|nineslice|nineslicedit|brush|brushing|polystar|paintinside|drawingedit|shapeenvelope|brushlibrary|brushcurve|paintbrush|eraser|erasing|erasemerge|tiltbrush|eyedropper|pen|frames|span|bucket|tlzoom|tlwide|motion|motionspan|motionpath|motionpoints|motionedit|motionpins|motionpinsspan|hints|hintsend|camera|cameraview|bones|springbones|shapebones|shapeweights|layerdepth|layerdepthview|filters|strokes|prefs|licenses|zoom|recovery|plugins|actions|script|scriptproblems|testmovie|testmovieinside|testscript|textfields|testtext|softmask|pathtext|button|testhits|exporthtml|publish|publishreport|publishanim|sound|envelope|ease|easepreset|plugineases|pluginexport|pluginexportwindow|library|librarymenu|libraryfolders|colour|layerprops|frameview|layercolour|timelinepolish|dock|dockicons|floattabs|docknarrow|workspaces|animator|designer|gradient|gradientbucket|swatches|swatchpanel]\n\
     --renderer   the window's graphics (also ANICRAFT_RENDERER); default automatic, falling back to OpenGL\n\
     --open       shows a panel at start (handy with --screenshot for visual checks)\n\
     --shot-width scales the screenshot down to this width (the user guide's pictures)\n\
     ANICRAFT_STAGE_RENDERER=gpu|cpu  stage renderer (default gpu, falling back to cpu; ADR 0032)\n\
     --theme      the interface palette for this run (not saved)\n\
     --scale      the interface scale for this run, 75 to 200 (not saved)\n\
     --lang       the interface language for this run (not saved); pseudo tests the layout\n\
     Headless use: anicraft-cli --help";

#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result {
    if let Some(result) = updates::finish_helper() {
        let failed = result.is_err();
        if let Err(error) = result {
            eprintln!("Update failed: {error}");
        }
        std::process::exit(i32::from(failed));
    }
    let args = match parse_args(std::env::args().skip(1)) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    match args.print {
        Some(Print::Version) => {
            println!("AniCraft {} ({})", env!("CARGO_PKG_VERSION"), app::RELEASE_STAGE);
            return Ok(());
        }
        Some(Print::Help) => {
            println!("{HELP}");
            return Ok(());
        }
        None => {}
    }
    let Args { shot, file, open, renderer, look, .. } = args;
    // macOS hands documents over as Apple Events, the one that launched the app included: listen
    // from before the event loop starts until it ends (ADR 0062).
    let _documents = platform::listen_for_documents();

    // The window: the graphics asked for, then this platform's order down to OpenGL. A way that
    // fails or panics before the window is up falls through to the next; a start that died
    // inside a driver last time tries that way last (#207, `startup`).
    let mut last = startup::begin(platform::log_dir());
    let mut file = file;
    let mut reopened = 0;
    let mut why = String::from("no attempt"); // i18n-exempt: log text
    // Once, and again each time the window closed to reopen another way.
    let mut opening = true;
    while opening {
        opening = false;
        let attempts = startup::plan(platform::forced_backend(renderer.as_deref()), prefs::Prefs::load().window_backend, &platform::window_attempts(platform::OS), &last);
        for (n, attempt) in attempts.iter().enumerate() {
            startup::trying(attempt, n + 1, attempts.len());
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(*attempt, file.as_deref(), shot.clone(), open.clone(), look.clone())));
            why = match result {
                Ok(Ok(())) => {
                    // The window's graphics device was lost mid-session (ADR 0092): it closed
                    // with the work autosaved, to open again another way, which offers it back.
                    if let Some(reopen) = startup::take_reopen()
                        && reopened < MAX_REOPENS
                    {
                        reopened += 1;
                        startup::line(&format!("opening the window again, {} last", attempt.label()));
                        last.retain(|l| *l != attempt.label());
                        last.push(attempt.label());
                        file = reopen;
                        opening = true;
                        break;
                    }
                    startup::line("closed");
                    return Ok(());
                }
                Ok(Err(e)) => e.to_string(),
                Err(p) => format!("{} panicked: {}", attempt.label(), startup::panic_text(p.as_ref())), // i18n-exempt: a log line
            };
            startup::line(&format!("{} failed: {why}", attempt.label()));
            // It ran and then stopped: not a start-up problem, and opening a second window would
            // surprise (autosave offers the work back next time). A graphics failure makes the
            // next starts try this way late (ADR 0092).
            if startup::started() {
                if startup::is_graphics_failure(&why) {
                    startup::lost_mid_session();
                }
                return Err(eframe::Error::AppCreation(why.into()));
            }
        }
    }
    startup::give_up(&why);
    Err(eframe::Error::AppCreation(why.into()))
}

/// Times one run opens the window again after its graphics device was lost (ADR 0092); past
/// that it closes, and the next start offers the work back.
#[cfg(not(target_arch = "wasm32"))]
const MAX_REOPENS: u32 = 2;

#[cfg(test)]
mod args_tests {
    use super::*;

    fn parse(args: &[&str]) -> Args {
        parse_args(args.iter().map(|a| a.to_string())).unwrap_or_else(|e| panic!("{args:?}: {e}"))
    }

    /// Windows (`"%1"`) and Linux (`%f`) hand a double-clicked document over as the one plain
    /// argument, spaces and all; it opens through `file.open`, as File › Open does.
    #[test]
    fn a_double_clicked_document_arrives_on_the_command_line_and_opens() {
        for path in [r"C:\Users\Ana Lima\My Films\walk cycle.anicraft", "/home/ana/Films/old scene.fla", "/home/ana/Films/Scene/DOMDocument.xfl", "relative.anicraft"] {
            assert_eq!(parse(&[path]).file, Some(PathBuf::from(path)), "{path}");
        }
        // The process serial number older macOS added is not a file.
        assert_eq!(parse(&["-psn_0_1234567"]).file, None);
        let a = parse(&["walk.anicraft", "--screenshot", "shots/x.png", "--open", "testmovie"]);
        assert_eq!((a.file, a.shot, a.open.as_deref()), (Some(PathBuf::from("walk.anicraft")), Some(PathBuf::from("shots/x.png")), Some("testmovie")));
        assert!(parse_args(["--scale".to_string(), "lots".to_string()]).is_err());
        assert!(parse_args(["--screenshot".to_string()]).is_err());
        assert_eq!(parse(&["--screenshot", "a.png", "--shot-width", "1200"]).look.shot_width, Some(1200));
        assert!(parse_args(["--shot-width".to_string(), "9".to_string()]).is_err());
        assert_eq!(parse(&["--version"]).print, Some(Print::Version));
        // And the file opens: the session starts with it, not the sample.
        let dir = std::env::temp_dir().join(format!("anicraft-shell-args-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("walk cycle.anicraft");
        let mut s = Session::new(anicraft_engine::doc::sample::interactive());
        s.execute("file.save", &serde_json::json!({"path": path.display().to_string()})).unwrap();
        let file = parse(&[&path.display().to_string()]).file;
        let (opened, _) = open_session(file.as_deref());
        assert_eq!(opened.path.as_deref(), Some(path.as_path()));
        assert!(opened.doc == s.doc, "the document on the command line, not the sample");
        // A file that isn't there: the sample, never a crash.
        let (fallback, said) = open_session(Some(&dir.join("missing.anicraft")));
        assert!(fallback.path.is_none());
        // #226: and the window says why (a release build has no console to print it on).
        assert!(matches!(&said, Opened::Failed(name, why) if name == "missing.anicraft" && !why.is_empty()), "{said:?}");
        // An Animate document double-clicked or opened with AniCraft imports, with its report.
        let xfl = Path::new(env!("CARGO_MANIFEST_DIR")).join("crates/xfl/tests/corpus/shapes/shapes.xfl");
        let (imported, said) = open_session(Some(&xfl));
        assert!(matches!(said, Opened::Imported(_)), "{said:?}");
        assert!(imported.path.is_none() && !imported.doc.timeline(0).layers.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod guide_reference_tests;
#[cfg(test)]
mod guidelines_tests;
#[cfg(test)]
mod dock_narrow_tests;
#[cfg(test)]
mod architecture_tests;
#[cfg(test)]
mod packaging_tests;
#[cfg(test)]
mod rulers_tests;
#[cfg(test)]
mod retime_ui_tests;
#[cfg(test)]
mod tilt_ui_tests;
