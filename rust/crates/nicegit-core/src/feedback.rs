//! The GitHub addresses behind "Report a problem" and "Request a feature". Only the NiceGit version,
//! the operating system and its architecture, and the Git version go into a report; never a
//! repository path or name.

use std::fmt::Write;

/// The repository's issue page for new issues.
const NEW_ISSUE: &str = "https://github.com/daniaalnadir/NiceGit/issues/new";

/// The address that opens a feature request.
pub const FEATURE_REQUEST_URL: &str = "https://github.com/daniaalnadir/NiceGit/issues/new?template=feature_request.md";

/// The bug report's body, as `.github/ISSUE_TEMPLATE/bug_report.md` lays it out, with the
/// environment lines filled in by `bug_report_url`.
fn bug_report_body(version: &str, os: &str, arch: &str, git_version: &str) -> String {
    let mut body = String::new();
    body.push_str("## What Happened\n\n");
    body.push_str("Describe the actual behavior and what you expected.\n\n");
    body.push_str("## Reproduction\n\n");
    body.push_str("List steps using a disposable repository where possible. Do not attach a private\n");
    body.push_str("repository, tokens, credentials, shell history, or unredacted diagnostic output.\n");
    body.push_str("For security vulnerabilities, follow SECURITY.md instead of filing details here.\n\n");
    body.push_str("## Environment\n\n");
    let _ = writeln!(body, "- Operating system (macOS, Windows, or Linux) and version: {os} ({arch})");
    let _ = writeln!(body, "- NiceGit version or commit: {version}");
    let _ = writeln!(body, "- Git version: {git_version}");
    body.push_str("\n## Evidence\n\n");
    body.push_str("Redacted screenshot or error text, and whether any local changes were affected:\n");
    body
}

/// The address that opens a new bug report with the template's text and the environment filled in.
/// `git_version` is blank when NiceGit does not know it cheaply.
pub fn bug_report_url(version: &str, os: &str, arch: &str, git_version: Option<&str>) -> String {
    let body = bug_report_body(version, os, arch, git_version.unwrap_or(""));
    format!("{NEW_ISSUE}?template=bug_report.md&body={}", percent_encode(&body))
}

/// Percent-encodes `text` for a URL query: every byte except letters, digits, and `-_.~` is written
/// as `%` and its two hex digits, so spaces, line breaks, `#`, `&`, `?`, `=`, and non-ASCII text
/// (as its UTF-8 bytes) all survive.
pub fn percent_encode(text: &str) -> String {
    let mut encoded = String::with_capacity(text.len());
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => encoded.push(char::from(byte)),
            _ => {
                let _ = write!(encoded, "%{byte:02X}");
            }
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_encoding_escapes_separators_and_keeps_unreserved_characters() {
        assert_eq!(percent_encode("a b"), "a%20b");
        assert_eq!(percent_encode("line\nnext"), "line%0Anext");
        assert_eq!(percent_encode("#&?=%"), "%23%26%3F%3D%25");
        assert_eq!(percent_encode("Az09-_.~"), "Az09-_.~");
        assert_eq!(percent_encode("é"), "%C3%A9", "non-ASCII is written as its UTF-8 bytes");
        assert_eq!(percent_encode(""), "");
    }

    #[test]
    fn bug_report_url_carries_the_template_and_the_environment_without_raw_line_breaks() {
        let url = bug_report_url("2.0.0", "macos", "aarch64", None);
        assert!(url.starts_with("https://github.com/daniaalnadir/NiceGit/issues/new?template=bug_report.md&body="));
        assert!(!url.contains('\n'), "line breaks are encoded");
        assert!(!url.contains(' '), "spaces are encoded");
        assert!(url.contains("2.0.0"), "the version is in the report");
        assert!(url.contains("macos%20%28aarch64%29"), "the OS and architecture are in the report");
        assert!(url.contains("%23%23%20What%20Happened"), "the template's sections are in the report");
        assert!(url.contains("%23%23%20Evidence"), "the template's last section is encoded");
    }

    #[test]
    fn bug_report_body_names_the_git_version_only_when_known() {
        let blank = bug_report_url("2.0.0", "linux", "x86_64", None);
        assert!(blank.contains("Git%20version%3A%20%0A"), "the Git line is left blank: {blank}");
        let known = bug_report_url("2.0.0", "linux", "x86_64", Some("2.45.1"));
        assert!(known.contains("2.45.1"));
    }

    #[test]
    fn feature_requests_open_the_feature_template() {
        assert_eq!(FEATURE_REQUEST_URL, "https://github.com/daniaalnadir/NiceGit/issues/new?template=feature_request.md");
    }
}
