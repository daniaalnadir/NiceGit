//! What the pull request APIs of Gitea and Forgejo, Bitbucket Cloud, and Azure DevOps share: one curl
//! request per call, with the credentials passed to curl on its standard input so they never appear in
//! the process arguments; answers decoded strictly; the plain words for each failure; and the pull request
//! record that each service's list is read into before it is shown.

use std::collections::HashMap;
use std::hash::Hash;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use crate::ai_message::config_quote;
use crate::github::{self, clean_text, Checks, GitHubItem, OpenPullRequest, PrState, PullRequestStatus, ReviewRequest};
use crate::gitlab::valid_username;
use crate::hosting::{Host, HostedRepository};
use crate::json::{parse_json, Json};
use crate::models::{GitError, Result};
use crate::runner::search_path;

/// How many commit or pull request statuses are read at the same time.
pub const CONCURRENT_READS: usize = 6;
/// How long curl may spend connecting, in seconds.
const CONNECT_SECONDS: u32 = 10;
/// How long curl may take for one request, in seconds.
const TIMEOUT_SECONDS: u32 = 30;
/// Longest error message from the server shown to the person.
const MESSAGE_LIMIT: usize = 300;
/// The longest title kept, in characters.
pub const TITLE_LIMIT: usize = 300;
/// The longest team or user name kept, in characters.
const NAME_LIMIT: usize = 100;

/// The notice for a repository whose issue tracker is switched off, which is no error.
pub const NO_ISSUE_TRACKER: &str = "This repository has no issue tracker on Bitbucket.";
/// The notice for an Azure DevOps listing without a personal access token, which is no error.
pub const NEEDS_TOKEN: &str = "Add a personal access token in Settings to see pull requests.";

/// The curl command for one request. The credentials and the body are only in `config`, which goes to
/// curl's standard input; `args` never holds either.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    pub args: Vec<String>,
    pub config: String,
}

/// The curl command that makes one request to `url`. A `body` is sent as JSON, and `authorization` is
/// the whole header value, such as `token abc` or `Basic dXNlcjpwYXNz`. The write-out format ends the
/// output with the status code, so an error answer's status can be read even though `--fail-with-body`
/// also sets curl's exit code. `--globoff` keeps curl from reading brackets and braces in a URL as lists.
pub fn curl_plan(method: &str, url: &str, authorization: Option<&str>, body: Option<&str>) -> Plan {
    let connect = CONNECT_SECONDS.to_string();
    let timeout = TIMEOUT_SECONDS.to_string();
    let args = [
        "--silent",
        "--show-error",
        "--fail-with-body",
        "--globoff",
        "--proto",
        "=https",
        "--proto-redir",
        "=https",
        "--connect-timeout",
        connect.as_str(),
        "--max-time",
        timeout.as_str(),
        "--write-out",
        "\\n%{http_code}",
        "--config",
        "-",
    ]
    .map(str::to_string)
    .to_vec();
    let mut config = String::new();
    let mut line = |name: &str, value: &str| {
        config.push_str(name);
        config.push_str(" = ");
        config.push_str(&config_quote(value));
        config.push('\n');
    };
    line("url", url);
    line("request", method);
    line("header", "accept: application/json");
    if let Some(body) = body {
        line("header", "content-type: application/json");
        line("data-binary", body);
    }
    if let Some(value) = authorization {
        line("header", &format!("Authorization: {value}"));
    }
    Plan { args, config }
}

/// The outcome of one curl run: the status and the body, or why there was none.
#[derive(Debug, PartialEq, Eq)]
pub enum Exchange {
    /// curl is not installed, so no request was made.
    NoCurl,
    /// No status came back: the server could not be reached or did not answer in time.
    Unreachable,
    Answer {
        status: u16,
        body: String,
    },
}

/// Reads curl's output, which ends with the status code written by the write-out format. A missing
/// or zero status is no answer at all.
pub fn parse_exchange(output: &str) -> Exchange {
    match output.rsplit_once('\n') {
        Some((body, code)) => match code.trim().parse::<u16>() {
            Ok(status) if status != 0 => Exchange::Answer { status, body: body.to_string() },
            _ => Exchange::Unreachable,
        },
        None => Exchange::Unreachable,
    }
}

/// Runs curl for one request and reads its status and body.
pub fn exchange(directory: &Path, method: &str, url: &str, authorization: Option<&str>, body: Option<&str>) -> Exchange {
    let Some(curl) = github::find_program("curl") else { return Exchange::NoCurl };
    let plan = curl_plan(method, url, authorization, body);
    let mut command = Command::new(curl);
    command
        .args(&plan.args)
        .current_dir(directory)
        .env("PATH", search_path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // Without this, each request flashes a console window on Windows.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let Ok(mut child) = command.spawn() else { return Exchange::Unreachable };
    if let Some(mut stdin) = child.stdin.take() {
        // The config holds the credentials and the body, so it goes in here and never in the arguments.
        std::thread::spawn(move || {
            let _ = stdin.write_all(plan.config.as_bytes());
        });
    }
    let Ok(output) = child.wait_with_output() else { return Exchange::Unreachable };
    parse_exchange(&String::from_utf8_lossy(&output.stdout))
}

/// The header value for HTTP Basic authentication of `credentials`, which are `user:password` as text.
pub fn basic_authorization(credentials: &str) -> String {
    format!("Basic {}", base64(credentials.as_bytes()))
}

/// The standard base64 encoding of `bytes`, with padding.
pub fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let first = u32::from(chunk[0]);
        let second = u32::from(chunk.get(1).copied().unwrap_or(0));
        let third = u32::from(chunk.get(2).copied().unwrap_or(0));
        let bits = (first << 16) | (second << 8) | third;
        out.push(ALPHABET[(bits >> 18) as usize & 63] as char);
        out.push(ALPHABET[(bits >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { ALPHABET[(bits >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { ALPHABET[bits as usize & 63] as char } else { '=' });
    }
    out
}

/// A path segment or query value with every byte outside the unreserved set percent-encoded, so a
/// branch name with slashes or quotes stays one value.
pub fn encode_component(text: &str) -> String {
    let mut encoded = String::new();
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => encoded.push(byte as char),
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

/// The error for an answer that is not a success, in words for the person. The server's own message
/// is kept, cleaned, for the statuses that have no fixed wording.
pub fn failure_for_status(service: &str, host: &str, status: u16, body: &str) -> GitError {
    let text = match status {
        401 | 403 => format!("The credentials for {host} were not accepted."),
        404 => format!("The repository was not found on {host}, or the credentials cannot see it."),
        _ => {
            let detail = server_message(body).map(|message| format!(" {message}")).unwrap_or_default();
            format!("{host} could not complete the request (status {status}).{detail}")
        }
    };
    GitError::failed(service, text)
}

/// The message in a JSON error answer, cleaned: a top-level `message`, or Bitbucket's `error.message`.
fn server_message(body: &str) -> Option<String> {
    let json = parse_json(body).ok()?;
    let message = json.get("message").or_else(|| json.get("error").and_then(|error| error.get("message")))?;
    let text = clean_text(message.as_str()?, MESSAGE_LIMIT);
    (!text.is_empty()).then_some(text)
}

/// The decoded body of a successful exchange, or the error the exchange stands for.
pub(crate) fn json_answer(service: &str, host: &str, exchange: Exchange) -> Result<Json> {
    match exchange {
        Exchange::NoCurl => Err(GitError::failed(service, format!("curl is needed to reach {host}, and it is not installed."))),
        Exchange::Unreachable => Err(GitError::failed(service, format!("Could not reach {host}."))),
        Exchange::Answer { status, body } => {
            if !(200..300).contains(&status) {
                return Err(failure_for_status(service, host, status, &body));
            }
            parse_json(&body).map_err(|_| GitError::failed(service, format!("{host} returned an answer that was not expected.")))
        }
    }
}

/// Makes one request and decodes its JSON answer. Transport failures and error statuses become the
/// plain words the Settings and dialogs show.
pub(crate) fn request_json(
    service: &str,
    host: &str,
    method: &str,
    url: &str,
    authorization: Option<&str>,
    body: Option<&str>,
    directory: &Path,
) -> Result<Json> {
    json_answer(service, host, exchange(directory, method, url, authorization, body))
}

/// The error for a list or an item whose answer does not have the expected shape.
pub fn unexpected(service: &str, what: &str) -> GitError {
    GitError::failed(service, format!("{service} returned unexpected {what}."))
}

/// Whether a commit's id can be put in a request path: hexadecimal, and no longer than a SHA-256 id.
pub fn is_commit_id(text: &str) -> bool {
    !text.is_empty() && text.len() <= 64 && text.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// A whole number, from a JSON number that is at least 1 and fits the range of a number.
pub(crate) fn whole_number(value: Option<&Json>) -> Option<u64> {
    value.and_then(Json::as_number).filter(|n| n.fract() == 0.0 && *n >= 1.0 && *n < 9.0e15).map(|n| n as u64)
}

/// The text of an optional field: absent or `null` is none, and any other type is refused.
pub(crate) fn optional_str<'a>(entry: &'a Json, key: &str) -> std::result::Result<Option<&'a str>, ()> {
    match entry.get(key) {
        None | Some(Json::Null) => Ok(None),
        Some(value) => value.as_str().map(Some).ok_or(()),
    }
}

/// A login from an object's `login` field, when it is a plain username; otherwise none.
pub(crate) fn login_of(user: Option<&Json>) -> Option<String> {
    user.and_then(|user| user.get("login")).and_then(Json::as_str).filter(|login| valid_username(login)).map(str::to_string)
}

/// A name from a team or a user, cleaned and kept when it is not empty.
pub(crate) fn name_of(value: Option<&Json>) -> Option<String> {
    let name = clean_text(value.and_then(Json::as_str)?, NAME_LIMIT);
    (!name.is_empty()).then_some(name)
}

/// The address of a pull request or issue on the host's web pages. The server's own link is kept only
/// when it is the repository's page for that number, compared without regard to case; anything else is
/// replaced by the address built from the parsed repository, so a link to another host is never shown.
pub fn web_link(repository: &HostedRepository, segment: &str, number: u64, link: Option<&str>) -> String {
    let canonical = format!("{}/{}/{segment}/{number}", repository.web_base(), repository.slug());
    match link {
        Some(link) if link.eq_ignore_ascii_case(&canonical) => link.to_string(),
        _ => canonical,
    }
}

/// The values read concurrently: the value `read` gives for each distinct key, with up to
/// [`CONCURRENT_READS`] reads running at once on scoped threads. A key named twice is read once.
pub fn read_concurrently<K, V>(keys: &[K], read: impl Fn(&K) -> V + Sync) -> HashMap<K, V>
where
    K: Clone + Eq + Hash + Send + Sync,
    V: Send,
{
    let mut distinct: Vec<&K> = Vec::new();
    for key in keys {
        if !distinct.contains(&key) {
            distinct.push(key);
        }
    }
    let next = AtomicUsize::new(0);
    let results = Mutex::new(HashMap::new());
    let workers = CONCURRENT_READS.min(distinct.len());
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| loop {
                let index = next.fetch_add(1, Ordering::Relaxed);
                let Some(key) = distinct.get(index) else { break };
                let value = read(key);
                if let Ok(mut found) = results.lock() {
                    found.insert((*key).clone(), value);
                }
            });
        }
    });
    results.into_inner().unwrap_or_default()
}

/// One pull request from a list, decoded, before its checks are read. Each service's list is read into
/// this shape, so the sidebar, the status bar, and the Launchpad treat them alike.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PullEntry {
    pub number: u64,
    pub title: String,
    pub url: String,
    pub state: PrState,
    pub author: Option<String>,
    pub head_branch: String,
    /// The head commit, whose status gives the checks. Absent when the answer has none.
    pub head_sha: Option<String>,
    /// Whether the branch is on a fork, or the fork is gone, so it is not this repository's branch.
    pub is_cross_repository: bool,
    pub review_requests: Vec<ReviewRequest>,
    pub updated_at: i64,
}

/// The pull request for `branch` among the listed pull requests: one whose head is `branch` on this
/// repository, preferring an open or draft one over a closed one. The list order decides the rest.
pub fn choose_branch_pull(entries: Vec<PullEntry>, branch: &str) -> Option<PullEntry> {
    let mut matching: Vec<PullEntry> =
        entries.into_iter().filter(|entry| entry.head_branch == branch && !entry.is_cross_repository).collect();
    matching.sort_by_key(|entry| !matches!(entry.state, PrState::Open | PrState::Draft));
    matching.into_iter().next()
}

/// The status of a pull request as the status bar shows it, once its checks are read.
pub fn status_of(kind: Host, entry: PullEntry, checks: Checks) -> PullRequestStatus {
    PullRequestStatus { kind, number: entry.number, title: entry.title, state: entry.state, url: entry.url, checks }
}

/// The open pull request as the Launchpad and branch checks list it.
pub fn open_request(entry: PullEntry, checks: Checks) -> OpenPullRequest {
    OpenPullRequest {
        number: entry.number,
        title: entry.title,
        url: entry.url,
        author: entry.author,
        head_branch: entry.head_branch,
        is_cross_repository: entry.is_cross_repository,
        is_draft: entry.state == PrState::Draft,
        review_decision: None,
        review_requests: entry.review_requests,
        updated_at: entry.updated_at,
        checks,
    }
}

/// The sidebar's row for a pull request. Its checks are not read for a list of rows.
pub fn item_of_pull(entry: PullEntry) -> GitHubItem {
    GitHubItem {
        number: entry.number,
        title: entry.title,
        url: entry.url,
        author: entry.author,
        state: Some("OPEN".to_string()),
        is_draft: entry.state == PrState::Draft,
        labels: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_credentials_are_never_in_the_arguments() {
        let token = "0123456789abcdef-secret";
        let plan = curl_plan("GET", "https://api.example.test/x", Some(&format!("token {token}")), None);
        assert!(plan.args.iter().all(|arg| !arg.contains(token)), "{:?}", plan.args);
        assert!(plan.config.contains(&format!("Authorization: token {token}")), "{}", plan.config);
        let basic = basic_authorization("octo:app-password-secret");
        let plan = curl_plan("POST", "https://api.example.test/y", Some(&basic), Some("{}"));
        assert!(plan.args.iter().all(|arg| !arg.contains("app-password-secret") && !arg.contains("dG9j")), "{:?}", plan.args);
        assert!(plan.config.contains(&format!("Authorization: {basic}")));
    }

    #[test]
    fn globbing_is_off_and_no_header_is_sent_without_credentials() {
        let plan = curl_plan("GET", "https://api.example.test/x?q=[a]{b}", None, None);
        assert!(plan.args.contains(&"--globoff".to_string()));
        assert!(!plan.config.contains("Authorization"));
    }

    #[test]
    fn base64_matches_the_standard_vectors() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foob"), "Zm9vYg==");
        assert_eq!(base64(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64(b":"), "Og==", "an Azure token is sent after an empty user name");
    }

    #[test]
    fn basic_authorization_is_the_base64_of_user_and_password() {
        assert_eq!(basic_authorization("user:pass"), "Basic dXNlcjpwYXNz");
        assert_eq!(basic_authorization(":pat"), format!("Basic {}", base64(b":pat")));
    }

    #[test]
    fn exchanges_end_with_their_status_or_are_no_answer() {
        assert_eq!(parse_exchange("[]\n200"), Exchange::Answer { status: 200, body: "[]".into() });
        assert_eq!(parse_exchange("\n401"), Exchange::Answer { status: 401, body: String::new() });
        assert_eq!(parse_exchange("\n000"), Exchange::Unreachable);
        assert_eq!(parse_exchange("no newline 200"), Exchange::Unreachable);
    }

    #[test]
    fn components_keep_one_segment_and_encode_quotes_and_slashes() {
        assert_eq!(encode_component("feature/x"), "feature%2Fx");
        assert_eq!(encode_component("source.branch.name=\"a\""), "source.branch.name%3D%22a%22");
        assert_eq!(encode_component("a{b}[c]"), "a%7Bb%7D%5Bc%5D");
    }

    #[test]
    fn failures_use_the_credential_wording_and_the_server_message() {
        let refused = failure_for_status("Bitbucket", "bitbucket.org", 401, "").to_string();
        assert!(refused.contains("The credentials for bitbucket.org were not accepted."), "{refused}");
        let forbidden = failure_for_status("Bitbucket", "bitbucket.org", 403, "").to_string();
        assert!(forbidden.contains("The credentials for bitbucket.org were not accepted."), "{forbidden}");
        let missing = failure_for_status("Azure DevOps", "dev.azure.com", 404, "{}").to_string();
        assert!(missing.contains("The repository was not found on dev.azure.com, or the credentials cannot see it."), "{missing}");
        let bitbucket = r#"{"type": "error", "error": {"message": "Source branch\u0007 not found"}}"#;
        let other = failure_for_status("Bitbucket", "bitbucket.org", 400, bitbucket).to_string();
        assert!(other.contains("bitbucket.org could not complete the request (status 400). Source branch not found"), "{other}");
        let azure = failure_for_status("Azure DevOps", "dev.azure.com", 500, r#"{"message": "TF401: boom"}"#).to_string();
        assert!(azure.ends_with("TF401: boom"), "{azure}");
    }

    #[test]
    fn json_answers_are_decoded_or_refused_with_words() {
        assert!(json_answer("S", "h", Exchange::Answer { status: 200, body: "[]".into() }).is_ok());
        assert!(json_answer("S", "h", Exchange::Answer { status: 200, body: "<html>".into() }).is_err());
        assert!(json_answer("S", "h", Exchange::Unreachable).unwrap_err().to_string().contains("Could not reach h."));
    }

    #[test]
    fn concurrent_reads_give_each_distinct_key_one_value() {
        let keys = ["a", "b", "a", "c"];
        let values = read_concurrently(&keys, |key| key.to_uppercase());
        assert_eq!(values.len(), 3);
        assert_eq!(values["b"], "B");
        assert_eq!(values["c"], "C");
    }

    #[test]
    fn commit_ids_are_hexadecimal_only() {
        assert!(is_commit_id("abc123"));
        assert!(!is_commit_id("abc/../x"));
        assert!(!is_commit_id(""));
    }
}
