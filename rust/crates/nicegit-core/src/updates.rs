//! Looking up the newest NiceGit release on GitHub, through the system `curl`. The request goes to
//! the release API and carries nothing else: no paths, no identifiers.

use std::process::{Command, Stdio};

use crate::github::find_program;
use crate::json::{parse_json, Json};
use crate::runner::search_path;

/// The address of the newest release, read for its version number.
pub const LATEST_RELEASE_API: &str = "https://api.github.com/repos/daniaalnadir/NiceGit/releases/latest";

/// Reads a version such as `2.1.0` or `v2.1.0` as its three numbers. A tag with a suffix, such as
/// `2.1.0-rc.1`, and anything else that is not `major.minor.patch` reads as `None`.
pub fn parse_version(text: &str) -> Option<(u64, u64, u64)> {
    let text = text.strip_prefix('v').unwrap_or(text);
    let mut parts = text.split('.');
    let mut number = || -> Option<u64> {
        let part = parts.next()?;
        if part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        part.parse().ok()
    };
    let version = (number()?, number()?, number()?);
    parts.next().is_none().then_some(version)
}

/// The `major.minor.patch` text of a version such as `v2.1.0`, or `None` when it has no such form.
pub fn version_text(tag: &str) -> Option<String> {
    let (major, minor, patch) = parse_version(tag)?;
    Some(format!("{major}.{minor}.{patch}"))
}

/// Whether `candidate` is a newer version than `current`. Equal versions are not newer, and
/// neither is anything that does not parse.
pub fn is_newer(candidate: &str, current: &str) -> bool {
    match (parse_version(candidate), parse_version(current)) {
        (Some(candidate), Some(current)) => candidate > current,
        _ => false,
    }
}

/// The `tag_name` of GitHub's release answer, which is JSON.
pub fn tag_name_from_release(json: &str) -> Result<String, String> {
    let value = parse_json(json).map_err(|_| "GitHub's answer was not in the expected form.".to_string())?;
    value.get("tag_name").and_then(Json::as_str).map(str::to_string).ok_or_else(|| "GitHub's answer named no release.".to_string())
}

/// Asks GitHub for the newest release's tag, through `curl`. Errors are short enough to show in
/// Settings. Blocks until curl finishes, so it runs on a background thread.
pub fn latest_release_tag() -> Result<String, String> {
    let curl = find_program("curl").ok_or_else(|| "curl is not installed.".to_string())?;
    let mut command = Command::new(curl);
    command
        .args([
            "--silent",
            "--show-error",
            "--fail",
            "--location",
            "--max-time",
            "10",
            "-H",
            "Accept: application/vnd.github+json",
            LATEST_RELEASE_API,
        ])
        .env("PATH", search_path())
        .stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // Without this, the check flashes a console window on Windows.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let output = command.output().map_err(|error| format!("curl did not start: {error}"))?;
    if !output.status.success() {
        return Err(failure_reason(&String::from_utf8_lossy(&output.stderr)));
    }
    tag_name_from_release(&String::from_utf8_lossy(&output.stdout))
}

/// The last non-empty line of curl's error output, shortened so it fits beside Settings' button.
fn failure_reason(stderr: &str) -> String {
    const LIMIT: usize = 120;
    let line = stderr.lines().rev().find(|line| !line.trim().is_empty()).unwrap_or("curl stopped without a message.").trim();
    if line.chars().count() <= LIMIT {
        line.to_string()
    } else {
        let mut short: String = line.chars().take(LIMIT - 1).collect();
        short.push('…');
        short
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_read_as_three_numbers_with_an_optional_v() {
        assert_eq!(parse_version("2.1.0"), Some((2, 1, 0)));
        assert_eq!(parse_version("v2.1.0"), Some((2, 1, 0)));
        assert_eq!(parse_version("10.0.12"), Some((10, 0, 12)));
    }

    #[test]
    fn tags_with_a_suffix_and_garbage_do_not_read() {
        for text in ["2.1.0-rc.1", "2.1", "2.1.0.4", "v", "", "latest", "2..1", "2.x.0", "2.1.0 ", "99999999999999999999.0.0"] {
            assert_eq!(parse_version(text), None, "{text:?}");
        }
    }

    #[test]
    fn newer_means_greater_numerically_not_as_text() {
        assert!(is_newer("2.0.1", "2.0.0"));
        assert!(is_newer("2.1.0", "2.0.1"));
        assert!(is_newer("v10.0.0", "2.1.0"));
        assert!(is_newer("2.0.1", "v2.0.0"));
        assert!(!is_newer("2.0.0", "2.0.0"), "an equal version is not newer");
        assert!(!is_newer("1.9.9", "2.0.0"));
        assert!(!is_newer("2.1.0-rc.1", "2.0.0"), "a suffixed tag is ignored");
        assert!(!is_newer("garbage", "2.0.0"));
    }

    #[test]
    fn version_text_drops_the_v() {
        assert_eq!(version_text("v2.1.0").as_deref(), Some("2.1.0"));
        assert_eq!(version_text("2.1.0-rc.1"), None);
    }

    #[test]
    fn the_tag_name_is_read_from_release_json() {
        let json = r#"{"url":"https://api.github.com/x","tag_name":"v2.1.0","name":"NiceGit 2.1.0","draft":false}"#;
        assert_eq!(tag_name_from_release(json).as_deref(), Ok("v2.1.0"));
        assert!(tag_name_from_release(r#"{"message":"Not Found"}"#).is_err());
        assert!(tag_name_from_release("<html>").is_err());
        assert!(tag_name_from_release(r#"{"tag_name":7}"#).is_err());
    }

    #[test]
    fn long_curl_errors_are_shortened_to_the_last_line() {
        assert_eq!(
            failure_reason("curl: (6) Could not resolve host: api.github.com\n"),
            "curl: (6) Could not resolve host: api.github.com"
        );
        assert_eq!(failure_reason("\n\n"), "curl stopped without a message.");
        let long = "x".repeat(300);
        let short = failure_reason(&long);
        assert_eq!(short.chars().count(), 120);
        assert!(short.ends_with('…'));
    }
}
