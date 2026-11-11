//! Checking GitHub for a newer NiceGit, once a day or when Settings' Check now asks. The check
//! runs on a background thread; its answer is handled here and shown by `ui::updates`.

use std::time::{SystemTime, UNIX_EPOCH};

use nicegit_core::updates::{is_newer, version_text};

use crate::app::NiceGitApp;
use crate::tools::Task;

/// How long after a successful check the next automatic one waits.
#[cfg(not(test))]
const CHECK_INTERVAL_SECONDS: u64 = 24 * 60 * 60;

/// The version this build is, as Cargo names it.
pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// The releases page the notice's Download button opens.
pub const RELEASES_PAGE: &str = "https://github.com/daniaalnadir/NiceGit/releases/latest";

/// A check running in the background.
pub struct UpdateCheck {
    task: Task<Result<String, String>>,
    /// Whether the person asked for it in Settings, so its outcome is shown there.
    manual: bool,
}

/// What the latest check the person asked for found, shown beside Settings' Check now button.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpdateStatus {
    Checking,
    UpToDate,
    /// A newer version, as `2.1.0`.
    Available(String),
    /// Why the check could not be made, in a few words.
    Failed(String),
}

/// The address that opens a new bug report, with this build's version and platform filled in. It
/// names no repository path or name.
pub fn bug_report_url() -> String {
    nicegit_core::feedback::bug_report_url(CURRENT_VERSION, std::env::consts::OS, std::env::consts::ARCH, None)
}

/// Seconds since the Unix epoch now; 0 if the clock is set before it.
fn now_seconds() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|elapsed| elapsed.as_secs()).unwrap_or(0)
}

/// The GitHub lookup. Tests never reach the network: they get a failure at once.
#[cfg(not(test))]
fn fetch_latest_tag() -> Result<String, String> {
    nicegit_core::updates::latest_release_tag()
}

#[cfg(test)]
fn fetch_latest_tag() -> Result<String, String> {
    Err("Update checks are not made in tests.".to_string())
}

impl NiceGitApp {
    /// Starts a check in the background. A check already running is reused; a manual request
    /// makes it report to Settings when it finishes.
    pub fn start_update_check(&mut self, manual: bool) {
        if let Some(check) = self.update_check.as_mut() {
            check.manual |= manual;
        } else {
            let context = self.worker.context.clone();
            let task = Task::spawn(&context, fetch_latest_tag);
            self.update_check = Some(UpdateCheck { task, manual });
        }
        if manual {
            self.update_status = Some(UpdateStatus::Checking);
        }
    }

    /// At launch, starts the daily check when checks are on and the last one is a day old or more.
    /// A clock set back before the last check counts as due.
    #[cfg(not(test))]
    pub fn start_due_update_check(&mut self) {
        let settings = &self.settings;
        if !settings.check_for_updates {
            return;
        }
        let now = now_seconds();
        let due = settings.last_update_check > now || now - settings.last_update_check >= CHECK_INTERVAL_SECONDS;
        if due {
            self.start_update_check(false);
        }
    }

    /// Hands a finished check to its result, once it has arrived. Failures are shown only for a
    /// check the person asked for.
    pub fn finish_update_check(&mut self) {
        let Some(check) = self.update_check.as_mut() else { return };
        let Some(result) = check.task.get().cloned() else { return };
        let manual = check.manual;
        self.update_check = None;
        match result {
            Ok(tag) => self.receive_latest_release(&tag),
            Err(reason) => {
                if manual {
                    self.update_status = Some(UpdateStatus::Failed(reason));
                }
            }
        }
    }

    /// Handles the tag of the newest release, as GitHub named it. A newer version is kept for the
    /// notice and Settings; an equal or older one clears the notice. The check's time is recorded.
    pub fn receive_latest_release(&mut self, tag: &str) {
        let Some(version) = version_text(tag) else {
            self.update_status = Some(UpdateStatus::Failed("GitHub's newest release has no plain version number.".into()));
            return;
        };
        self.settings.last_update_check = now_seconds();
        if is_newer(&version, CURRENT_VERSION) {
            self.settings.latest_version = Some(version.clone());
            self.update_status = Some(UpdateStatus::Available(version));
        } else {
            self.settings.latest_version = None;
            self.update_status = Some(UpdateStatus::UpToDate);
        }
    }

    /// The newer version the notice should offer, when it was not dismissed.
    pub fn pending_update(&self) -> Option<String> {
        let version = self.settings.latest_version.as_ref()?;
        let newer = is_newer(version, CURRENT_VERSION);
        let dismissed = self.settings.dismissed_update.as_ref() == Some(version);
        (newer && !dismissed).then(|| version.clone())
    }
}
