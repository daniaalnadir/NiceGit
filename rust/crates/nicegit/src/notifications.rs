//! Desktop notifications for work that finishes while NiceGit is in the background.
//!
//! The decision of what to say is a pure function so it can be tested without showing anything.

use std::time::Duration;

const TITLE: &str = "NiceGit";

/// Actions that finish sooner than this are not announced, so quick ones do not spam.
#[cfg(not(test))]
pub const ACTION_NOTIFY_AFTER: Duration = Duration::from_secs(2);
/// Test actions finish in milliseconds, so the threshold is lowered for them.
#[cfg(test)]
pub const ACTION_NOTIFY_AFTER: Duration = Duration::from_millis(1);

/// Work that finished, as the app saw it.
#[derive(Clone, Debug, PartialEq)]
pub enum FinishedWork {
    /// An action the person started. `error` holds Git's message when the action failed.
    Action { label: String, repo: String, elapsed: Duration, error: Option<String> },
    /// A background fetch that left the current branch further behind its upstream.
    NewUpstreamCommits { repo: String, upstream: String, count: usize },
    /// A clone from the Clone dialog: the new folder's name, or the failure message.
    Clone(Result<String, String>),
}

/// The title and body to show for `event`, or None when nothing should be shown. Nothing is shown
/// while the window is focused or when the setting is off.
pub fn notification_for(event: &FinishedWork, focused: bool, enabled: bool) -> Option<(String, String)> {
    if focused || !enabled {
        return None;
    }
    let body = match event {
        FinishedWork::Action { label, repo, elapsed, error } => {
            if *elapsed < ACTION_NOTIFY_AFTER {
                return None;
            }
            match error {
                None => format!("{label} finished in {repo}"),
                Some(error) => match first_line(error) {
                    "" => format!("{label} failed in {repo}."),
                    line => format!("{label} failed in {repo}: {line}"),
                },
            }
        }
        FinishedWork::NewUpstreamCommits { repo, upstream, count } => {
            if *count == 0 {
                return None;
            }
            let noun = if *count == 1 { "commit" } else { "commits" };
            format!("{count} new {noun} on {upstream} in {repo}")
        }
        FinishedWork::Clone(Ok(name)) => format!("Clone finished in {name}"),
        FinishedWork::Clone(Err(error)) => match first_line(error) {
            "" => "Clone failed.".to_string(),
            line => format!("Clone failed: {line}"),
        },
    };
    Some((TITLE.to_string(), body))
}

/// The first line of `text` that has any words in it, trimmed. Empty when there is none.
fn first_line(text: &str) -> &str {
    text.lines().map(str::trim).find(|line| !line.is_empty()).unwrap_or("")
}

/// Shows a notification without waiting for it: the operating system may hold the call open.
/// Delivery errors are ignored, since a missed notification costs nothing else.
#[cfg(not(test))]
pub fn send(title: String, body: String) {
    std::thread::spawn(move || {
        // On macOS, notifications are attributed to an application bundle; without this they would
        // appear under another app's name. A development build outside the bundle keeps the default.
        #[cfg(target_os = "macos")]
        {
            static ATTRIBUTED: std::sync::Once = std::sync::Once::new();
            ATTRIBUTED.call_once(|| {
                let _ = notify_rust::set_application("app.nicegit.desktop");
            });
        }
        let _ = notify_rust::Notification::new().summary(&title).body(&body).show();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn action(elapsed: Duration, error: Option<&str>) -> FinishedWork {
        FinishedWork::Action { label: "Push".into(), repo: "notes".into(), elapsed, error: error.map(String::from) }
    }

    fn body(event: &FinishedWork) -> String {
        notification_for(event, false, true).expect("a notification").1
    }

    #[test]
    fn nothing_is_shown_while_the_window_is_focused() {
        assert_eq!(notification_for(&action(Duration::from_secs(10), None), true, true), None);
        let fetched = FinishedWork::NewUpstreamCommits { repo: "notes".into(), upstream: "origin/main".into(), count: 2 };
        assert_eq!(notification_for(&fetched, true, true), None);
        assert_eq!(notification_for(&FinishedWork::Clone(Ok("notes".into())), true, true), None);
    }

    #[test]
    fn nothing_is_shown_when_the_setting_is_off() {
        assert_eq!(notification_for(&action(Duration::from_secs(10), None), false, false), None);
        assert_eq!(notification_for(&FinishedWork::Clone(Ok("notes".into())), false, false), None);
    }

    #[test]
    fn quick_actions_are_not_announced() {
        assert_eq!(notification_for(&action(Duration::ZERO, None), false, true), None);
        assert_eq!(notification_for(&action(Duration::ZERO, Some("boom")), false, true), None);
    }

    #[test]
    fn a_finished_action_names_its_repository() {
        assert_eq!(body(&action(Duration::from_secs(10), None)), "Push finished in notes");
        let (title, _) = notification_for(&action(Duration::from_secs(10), None), false, true).unwrap();
        assert_eq!(title, "NiceGit");
    }

    #[test]
    fn a_failed_action_uses_only_the_first_error_line() {
        let error = "remote: rejected\n\nfatal: details that are not needed\n";
        assert_eq!(body(&action(Duration::from_secs(10), Some(error))), "Push failed in notes: remote: rejected");
        assert_eq!(body(&action(Duration::from_secs(10), Some("\n  \n"))), "Push failed in notes.");
    }

    #[test]
    fn background_fetch_reports_the_new_commits_on_the_upstream() {
        let one = FinishedWork::NewUpstreamCommits { repo: "notes".into(), upstream: "origin/main".into(), count: 1 };
        assert_eq!(body(&one), "1 new commit on origin/main in notes");
        let three = FinishedWork::NewUpstreamCommits { repo: "notes".into(), upstream: "origin/main".into(), count: 3 };
        assert_eq!(body(&three), "3 new commits on origin/main in notes");
    }

    #[test]
    fn background_fetch_with_no_change_is_silent() {
        let none = FinishedWork::NewUpstreamCommits { repo: "notes".into(), upstream: "origin/main".into(), count: 0 };
        assert_eq!(notification_for(&none, false, true), None);
    }

    #[test]
    fn clone_reports_the_new_folder_or_the_failure() {
        assert_eq!(body(&FinishedWork::Clone(Ok("notes".into()))), "Clone finished in notes");
        assert_eq!(body(&FinishedWork::Clone(Err("fatal: not found\nmore".into()))), "Clone failed: fatal: not found");
    }
}
