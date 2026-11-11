//! Native release checks and verified installation through craft-update's platform helpers.
use craft_update::pins::{Choice, Pins};
use craft_update::{App, Http, Paths, Platform, Release, Stop, self_update};
use std::sync::mpsc::{Receiver, TryRecvError, channel};

// i18n-exempt: stable release identity, not interface text.
pub const APP: App = App::new("anicraft", "AniCraft", "Animation", "storytold/anicraft");
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// A pinned, nightly, or unknown launcher choice must never be replaced by a stable update.
fn stable_choice() -> Result<(), String> {
    if Pins::load_default().choice(APP.id) == Choice::Stable {
        Ok(())
    } else {
        Err(tr!("ArtCraft Launcher manages this app's version. Change its version there.").into())
    }
}

/// Whether an update may close the editor without losing pending document work.
pub(crate) fn safe_to_install(app: &crate::app::App) -> bool {
    !app.s.is_dirty()
        && !app.s.in_gesture()
        && !app.playing
        && app.text_edit.is_none()
        && app.pen.is_none()
        && app.pencil.is_empty()
        && app.brush_samples.is_empty()
        && app.raw_touched.is_empty()
        && app.drag == crate::app::Drag::None
        && app.motion_drag.is_none()
        && app.tl_drag == crate::timeline::TlDrag::None
        && app.me_drag.is_none()
        && app.plugin_dialog.is_none()
        && app.envelope.is_none()
}

enum Reply {
    Checked(Result<Option<Release>, String>, bool),
    Installed(Result<self_update::Restart, String>),
}
#[derive(Default)]
pub struct Updates {
    open: bool,
    read_outcome_requested: bool,
    requested: bool,
    message: String,
    release: Option<Release>,
    installable: bool,
    worker: Option<Receiver<Reply>>,
    installing: bool,
    restart: Option<self_update::Restart>,
}
impl Updates {
    /// Read the previous update result when the first desktop frame can show it.
    pub fn startup(&mut self) {
        self.read_outcome_requested = true;
    }

    /// Queue a check from the command-line panel selector.
    pub fn request(&mut self) {
        self.requested = true;
    }

    /// Start a manual check off the UI thread; repeated requests share the running check.
    pub fn check(&mut self, ctx: &egui::Context) {
        self.check_using(ctx, || {
            let result = stable_choice()
                .and_then(|()| Http::new(concat!("AniCraft/", env!("CARGO_PKG_VERSION"))))
                .and_then(|http| craft_update::check_for_update(&APP, &http, VERSION));
            let installable = result
                .as_ref()
                .ok()
                .and_then(|release| release.as_ref())
                .is_some_and(|release| self_update::is_self_updating(&APP) && self_update::release_installs(&APP, release));
            (result, installable)
        });
    }

    fn check_using(&mut self, ctx: &egui::Context, check: impl FnOnce() -> (Result<Option<Release>, String>, bool) + Send + 'static) {
        self.open = true;
        if self.worker.is_some() {
            return;
        }
        self.message = tr!("Checking for updates…").into();
        self.release = None;
        self.installable = false;
        let (tx, rx) = channel();
        self.worker = Some(rx);
        let ctx = ctx.clone();
        let result = std::thread::Builder::new().name("release-check".into()).spawn(move || {
            let (result, installable) = check();
            let _ = tx.send(Reply::Checked(result, installable));
            ctx.request_repaint();
        });
        if let Err(error) = result {
            self.worker = None;
            self.message = error.to_string();
        }
    }
    fn install(&mut self, ctx: &egui::Context, release: Release) {
        self.install_using(ctx, move || {
            let install = self_update::current(&APP)
                .ok_or(tr!("This installation is managed externally. Update it with its package manager or open Releases."))?;
            let platform = Platform::current().ok_or(tr!("No installer is available for this platform."))?;
            let http = Http::new(concat!("AniCraft/", env!("CARGO_PKG_VERSION")))?;
            let paths = Paths::for_app(&APP)?;
            self_update::prepare_update(
                &APP,
                &install,
                &release,
                platform,
                &http,
                &paths,
                &Stop::default(),
                &mut |_| {},
                &stable_choice,
                Some(VERSION),
            )
        });
    }

    fn install_using(&mut self, ctx: &egui::Context, install: impl FnOnce() -> Result<self_update::Restart, String> + Send + 'static) {
        self.installing = true;
        self.message = tr!("Downloading and verifying the update…").into();
        let (tx, rx) = channel();
        self.worker = Some(rx);
        let ctx = ctx.clone();
        let result = std::thread::Builder::new().name("release-install".into()).spawn(move || {
            let result = install();
            let _ = tx.send(Reply::Installed(result));
            ctx.request_repaint();
        });
        if let Err(error) = result {
            self.worker = None;
            self.installing = false;
            self.message = error.to_string();
        }
    }
    /// Returns true while installing: the caller pauses editing and starting work until restart.
    pub fn show(&mut self, ctx: &egui::Context, safe_to_install: bool) -> bool {
        if std::mem::take(&mut self.requested) {
            self.check(ctx);
        }
        if std::mem::take(&mut self.read_outcome_requested)
            && let Ok(paths) = Paths::for_app(&APP)
        {
            self.apply_outcome(self_update::take_outcome(&paths, VERSION));
        }
        let reply = self.worker.as_ref().map(Receiver::try_recv);
        match reply {
            Some(Ok(Reply::Checked(result, installable))) => {
                self.installable = installable;
                self.worker = None;
                self.checked(result);
            }
            Some(Ok(Reply::Installed(result))) => {
                self.worker = None;
                match result {
                    Ok(restart) => {
                        self.restart = Some(restart);
                        self.message = tr!("Update ready. Restarting…").into();
                    }
                    Err(error) => {
                        self.installing = false;
                        self.message = error;
                    }
                }
            }
            Some(Err(TryRecvError::Disconnected)) => {
                self.worker = None;
                self.installing = false;
                self.message = tr!("The update worker stopped unexpectedly. Try checking again.").into();
            }
            _ => {}
        }
        if self.restart.is_some() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        } else if self.installing && ctx.input(|i| i.viewport().close_requested()) {
            // The platform helper may be replacing files. Keep this process alive until it finishes.
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        }
        if self.worker.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        if self.open {
            let mut open = true;
            let mut install = false;
            let mut again = false;
            egui::Window::new(tr!("Check for Updates"))
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .default_width(440.0)
                .max_width((ctx.content_rect().width() - 32.0).max(160.0))
                .show(ctx, |ui| {
                    ui.label(tr!("AniCraft {version}", version = VERSION));
                    ui.label(&self.message);
                    if !safe_to_install && self.release.is_some() {
                        ui.label(tr!("Save your work and stop running tasks before installing an update."));
                    }
                    if self.release.is_some() && !self.installable {
                        ui.label(tr!("Use Releases or your package manager to update this installation."));
                    }
                    ui.horizontal(|ui| {
                        if let Some(release) = &self.release {
                            ui.hyperlink_to(tr!("Releases"), &release.html_url);
                            install = ui
                                .add_enabled(
                                    safe_to_install && self.installable && self.worker.is_none() && self.restart.is_none(),
                                    egui::Button::new(tr!("Update and restart")),
                                )
                                .clicked();
                        } else {
                            ui.hyperlink_to(tr!("Releases"), APP.releases_url());
                        }
                        again = ui.add_enabled(self.worker.is_none() && !self.installing, egui::Button::new(tr!("Check again"))).clicked();
                    });
                });
            self.open = open || self.installing;
            if install && let Some(release) = self.release.clone() {
                self.install(ctx, release);
            }
            if again {
                self.check(ctx);
            }
        }
        self.installing
    }
    fn apply_outcome(&mut self, outcome: Option<self_update::Outcome>) {
        match outcome {
            Some(self_update::Outcome::Installed(version)) => {
                self.open = true;
                self.message = tr!("Updated to {version}.", version = version);
            }
            Some(self_update::Outcome::Failed(error)) => {
                self.open = true;
                self.message = tr!("The update failed: {error}", error = error);
            }
            None => {}
        }
    }

    fn checked(&mut self, result: Result<Option<Release>, String>) {
        match result {
            Ok(Some(release)) => {
                self.message = tr!("{version} is available.", version = release.version_label());
                self.release = Some(release);
            }
            Ok(None) => {
                self.message = tr!("You're up to date.").into();
                self.release = None;
            }
            Err(error) => {
                self.message = tr!("Couldn't check for updates: {error}", error = error);
                self.release = None;
            }
        }
    }
    /// Start the prepared helper only after the app's normal exit path has accepted closing.
    pub fn finish(&mut self) {
        if let Some(restart) = self.restart.take()
            && let Err(error) = restart.start()
        {
            eprintln!("Couldn't restart after updating: {error}");
        }
    }
}

/// Runs the Windows installation helper before opening a normal application window.
pub fn finish_helper() -> Option<Result<(), String>> {
    self_update::finish(&APP, &std::env::args_os().skip(1).collect::<Vec<_>>())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draw(updates: &mut Updates, safe: bool, close: bool) -> egui::FullOutput {
        let ctx = egui::Context::default();
        let mut input =
            egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(640.0, 400.0))), ..Default::default() };
        if close {
            input.viewports.entry(egui::ViewportId::ROOT).or_default().events.push(egui::ViewportEvent::Close);
        }
        let mut output = ctx.run_ui(input, |ui| {
            updates.show(ui.ctx(), safe);
        });
        output.textures_delta.clear();
        output
    }

    #[test]
    fn a_launcher_pin_stops_the_real_check_before_network_access() {
        const CHILD: &str = "CRAFT_UPDATE_PIN_TEST_CHILD";
        if std::env::var_os(CHILD).is_some() {
            assert!(stable_choice().is_err());
            let mut updates = Updates::default();
            updates.check(&egui::Context::default());
            let reply = updates
                .worker
                .as_ref()
                .expect("check worker")
                .recv_timeout(std::time::Duration::from_secs(5))
                .expect("pin check finishes without network");
            let (tx, rx) = channel();
            tx.send(reply).expect("receiver");
            updates.worker = Some(rx);
            draw(&mut updates, true, false);
            assert!(updates.message.contains("ArtCraft Launcher"));
            assert!(updates.release.is_none());
            assert!(!updates.installable);
            assert!(finish_helper().is_none());
            #[cfg(target_os = "linux")]
            {
                // This child has an isolated XDG data folder; never consume a user's update record.
                let paths = Paths::for_app(&APP).expect("isolated app folders");
                std::fs::create_dir_all(paths.logs()).expect("test logs");
                let failure = paths.logs().join("self-update-error.txt");
                std::fs::write(&failure, "test installer failure").expect("test failure record");
                let mut restarted = Updates::default();
                restarted.startup();
                draw(&mut restarted, true, false);
                assert!(restarted.open);
                assert!(restarted.message.contains("test installer failure"));
                assert!(!failure.exists(), "the helper result is consumed exactly once");
                restarted.open = false;
                restarted.startup();
                draw(&mut restarted, true, false);
                assert!(!restarted.open, "the old failure must not reappear on the next start");
            }

            return;
        }
        // Keep the process environment and the user's actual version choices untouched.
        let dir = std::env::temp_dir().join(format!("{}-updater-pin-test-{}", APP.id, std::process::id()));
        std::fs::create_dir_all(&dir).expect("test folder");
        let path = dir.join("versions.json");
        std::fs::write(&path, format!(r#"{{"apps":{{"{}":{{"Pinned":"0.1.0"}}}}}}"#, APP.id)).expect("test pin");
        let status = std::process::Command::new(std::env::current_exe().expect("test binary"))
            .args(["--exact", "updates::tests::a_launcher_pin_stops_the_real_check_before_network_access"])
            .env(CHILD, "1")
            .env(craft_update::pins::PATH_ENV, &path)
            .env("XDG_DATA_HOME", dir.join("data"))
            .status()
            .expect("isolated pin test");
        std::fs::remove_dir_all(dir).expect("remove test folder");
        assert!(status.success());
    }
    #[test]
    fn available_release_is_shown_but_never_installed_without_confirmation() {
        let release = Release { tag: "v99.0.0".into(), html_url: APP.releases_url(), ..Release::default() };
        let (tx, rx) = channel();
        tx.send(Reply::Checked(Ok(Some(release)), false)).expect("receiver is alive");
        let mut updates = Updates { open: true, worker: Some(rx), ..Updates::default() };
        let output = draw(&mut updates, false, false);
        assert!(updates.message.contains("99.0.0"));
        assert!(!output.shapes.is_empty(), "the result window must be rendered");
        assert!(updates.worker.is_none());
        assert!(!updates.installing);
        assert!(!updates.installable, "externally managed installs must only offer Releases");
        updates.installable = true;
        draw(&mut updates, true, false);
        assert!(!updates.installing, "an available update alone must never install");
    }

    #[test]
    fn checks_run_off_thread_and_repeated_requests_share_the_worker() {
        let ctx = egui::Context::default();
        let (started, wait) = channel();
        let mut updates = Updates::default();
        updates.check_using(&ctx, move || {
            wait.recv().expect("release test worker");
            (Ok(None), false)
        });
        assert!(updates.worker.is_some());
        updates.check_using(&ctx, || panic!("must not start a second check"));
        draw(&mut updates, false, false);
        started.send(()).expect("worker waiting");
        let reply = updates.worker.as_ref().expect("worker").recv_timeout(std::time::Duration::from_secs(5)).expect("worker finishes");
        let (tx, rx) = channel();
        tx.send(reply).expect("receiver");
        updates.worker = Some(rx);
        draw(&mut updates, true, false);
        assert_eq!(updates.message, "You're up to date.");
        assert!(updates.worker.is_none());
    }

    #[test]
    fn an_install_failure_unlocks_editing_and_close_is_cancelled_until_it_finishes() {
        let ctx = egui::Context::default();
        let (finish, wait) = channel();
        let mut updates = Updates { open: true, ..Updates::default() };
        updates.install_using(&ctx, move || {
            wait.recv().expect("release test worker");
            Err("signature verification failed".into())
        });
        assert!(updates.installing);
        let output = draw(&mut updates, false, true);
        assert!(output.viewport_output.values().any(|v| v.commands.contains(&egui::ViewportCommand::CancelClose)));
        finish.send(()).expect("worker waiting");
        let reply = updates.worker.as_ref().expect("worker").recv_timeout(std::time::Duration::from_secs(5)).expect("worker finishes");
        let (tx, rx) = channel();
        tx.send(reply).expect("receiver");
        updates.worker = Some(rx);
        draw(&mut updates, false, false);
        assert!(!updates.installing);
        assert!(updates.worker.is_none());
        assert!(updates.restart.is_none(), "failed verification must never schedule a restart");
        assert!(updates.message.contains("signature verification failed"));
        updates.finish();
        updates.apply_outcome(None);
    }
    #[test]
    fn uncommitted_stage_text_and_pen_paths_prevent_installation() {
        let ctx = egui::Context::default();
        let session = anicraft_engine::Session::new(anicraft_engine::doc::sample::demo());
        let mut app = crate::app::App::with_context(&ctx, session, Some(std::path::PathBuf::from("unused.png")));
        assert!(!app.s.is_dirty());
        assert!(safe_to_install(&app));
        app.text_edit = Some(crate::app::TextEditing {
            at: egui::Pos2::ZERO,
            element: None,
            text: "Uncommitted stage text".into(),
            original: String::new(),
            focus: false,
        });
        assert!(!app.s.is_dirty(), "the edit has not reached Session yet");
        assert!(!safe_to_install(&app), "restarting would discard the text buffer");
        app.text_edit = None;
        app.pen = Some(crate::pathtools::PenPath { knots: vec![[anicraft_engine::geom::Vec2::ZERO; 3]], dragging: false });
        assert!(!app.s.is_dirty());
        assert!(!safe_to_install(&app), "an unfinished Pen path is also pending work");
        app.pen = None;
        assert!(safe_to_install(&app));
        app.s.begin_gesture("pending edit");
        assert!(!safe_to_install(&app), "an engine gesture must finish first");
    }

    #[test]
    fn a_failed_helper_is_reported_when_the_old_app_restarts() {
        let mut updates = Updates::default();
        updates.apply_outcome(Some(self_update::Outcome::Failed("installer refused the update".into())));
        assert!(updates.open);
        assert!(updates.message.contains("installer refused the update"));
        assert!(updates.worker.is_none());
        assert!(!updates.installing);
    }

    #[test]
    fn a_successful_restart_reports_the_installed_version() {
        let mut updates = Updates::default();
        let version = craft_update::version::parse("1.2.3").expect("valid test version");
        updates.apply_outcome(Some(self_update::Outcome::Installed(version)));
        assert!(updates.open);
        assert!(updates.message.contains("1.2.3"));
    }

    #[test]
    fn a_manual_check_reports_success_and_failure() {
        let mut updates = Updates::default();
        updates.checked(Ok(None));
        assert_eq!(updates.message, "You're up to date.");
        updates.checked(Err("network unavailable".into()));
        assert!(updates.message.contains("network unavailable"));
        assert!(updates.release.is_none());
    }
    #[test]
    fn a_dropped_worker_reports_failure_and_unblocks_the_app() {
        let (tx, rx) = channel();
        drop(tx);
        let mut updates = Updates { worker: Some(rx), installing: true, ..Updates::default() };
        assert!(!updates.show(&egui::Context::default(), false));
        assert!(updates.message.contains("stopped unexpectedly"));
    }
}
