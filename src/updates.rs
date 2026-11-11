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

enum Reply {
    Checked(Result<Option<Release>, String>, bool),
    Installed(Result<self_update::Restart, String>),
}
#[derive(Default)]
pub struct Updates {
    open: bool,
    requested: bool,
    message: String,
    release: Option<Release>,
    installable: bool,
    worker: Option<Receiver<Reply>>,
    installing: bool,
    restart: Option<self_update::Restart>,
}
impl Updates {
    /// Queue a check from the command-line panel selector.
    pub fn request(&mut self) {
        self.requested = true;
    }

    /// Start a manual check off the UI thread; repeated requests share the running check.
    pub fn check(&mut self, ctx: &egui::Context) {
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
            let result = stable_choice()
                .and_then(|()| Http::new(concat!("AniCraft/", env!("CARGO_PKG_VERSION"))))
                .and_then(|http| craft_update::check_for_update(&APP, &http, VERSION));
            let installable =
                result.as_ref().ok().and_then(|release| release.as_ref()).is_some_and(|release| self_update::release_installs(&APP, release));
            let _ = tx.send(Reply::Checked(result, installable));
            ctx.request_repaint();
        });
        if let Err(error) = result {
            self.worker = None;
            self.message = error.to_string();
        }
    }
    fn install(&mut self, ctx: &egui::Context, release: Release) {
        self.installing = true;
        self.message = tr!("Downloading and verifying the update…").into();
        let (tx, rx) = channel();
        self.worker = Some(rx);
        let ctx = ctx.clone();
        let result = std::thread::Builder::new().name("release-install".into()).spawn(move || {
            let result = (|| {
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
            })();
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
pub fn finish_helper() {
    if let Some(result) = self_update::finish(&APP, &std::env::args_os().skip(1).collect::<Vec<_>>()) {
        let failed = result.is_err();
        if let Err(error) = result {
            eprintln!("Update failed: {error}");
        }
        std::process::exit(i32::from(failed));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
