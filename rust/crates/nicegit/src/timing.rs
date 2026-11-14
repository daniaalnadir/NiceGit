//! Timings for start-up, switching repositories, and actions, printed to standard error when
//! NICEGIT_TIMING is set. They measure what a person waits for, so performance work can be
//! checked against numbers (see CONTRIBUTING.md, Performance):
//!
//! ```text
//! NICEGIT_TIMING=1 ./target/release/nicegit
//! ```

use std::sync::OnceLock;
use std::time::Instant;

static START: OnceLock<Instant> = OnceLock::new();
static ENABLED: OnceLock<bool> = OnceLock::new();

/// Marks the moment the process started running NiceGit's code. Call first in `main`.
pub fn start() {
    START.get_or_init(Instant::now);
}

pub fn enabled() -> bool {
    *ENABLED.get_or_init(|| std::env::var_os("NICEGIT_TIMING").is_some())
}

/// Milliseconds since `start`.
pub fn since_start() -> f64 {
    START.get_or_init(Instant::now).elapsed().as_secs_f64() * 1000.0
}

/// Prints one timing when timings are on.
pub fn report(what: &str, milliseconds: f64) {
    if enabled() {
        eprintln!("[timing] {what}: {milliseconds:.1} ms");
    }
}

/// Prints the time since `from` when timings are on.
pub fn report_since(what: &str, from: Instant) {
    report(what, from.elapsed().as_secs_f64() * 1000.0);
}

/// What the interface is waiting to report.
#[derive(Default)]
pub struct Timings {
    first_frame: bool,
    first_repository: bool,
    /// The repository switched to, and when.
    pub switch: Option<(usize, Instant)>,
    /// The action started, and when.
    pub action: Option<(String, Instant)>,
}

impl Timings {
    /// Called at the end of each frame with whether the active repository has a snapshot to show.
    pub fn frame(&mut self, active: usize, shown: bool) {
        if !enabled() {
            return;
        }
        if !self.first_frame {
            self.first_frame = true;
            report("first frame built", since_start());
        }
        if shown && !self.first_repository {
            self.first_repository = true;
            report("first repository shown", since_start());
        }
        if let Some((index, at)) = self.switch {
            if index == active && shown {
                report_since("switch: repository shown", at);
                self.switch = None;
            }
        }
    }

    /// Called when an action's result and refreshed snapshot arrive.
    pub fn action_done(&mut self) {
        if let Some((label, at)) = self.action.take() {
            report_since(&format!("{label}: result and refresh shown"), at);
        }
    }
}
