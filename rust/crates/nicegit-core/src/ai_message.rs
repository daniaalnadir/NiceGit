//! Drafting a commit message from the staged changes with Claude. The request goes to Anthropic's
//! API through the system `curl`, as the update check does, and carries the person's own API key.
//!
//! The key is passed to curl on its standard input, as a config file, so it never appears in the
//! process arguments. The request body is written to a private temporary file that is removed
//! once curl has finished, whatever the outcome.

use std::fmt::Write as _;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::client::GitClient;
use crate::github::find_program;
use crate::json::{parse_json, Json};
use crate::runner::search_path;

/// The endpoint the message request is posted to.
pub const API_URL: &str = "https://api.anthropic.com/v1/messages";
/// The API version the request names in its `anthropic-version` header.
pub const API_VERSION: &str = "2026-10-21";
/// The beta header that lets the API fall back to another model when the first is unavailable.
pub const FALLBACK_BETA: &str = "server-side-fallback-2026-07-01";
/// The most tokens the answer may use.
pub const MAX_TOKENS: u32 = 2000;
/// The most bytes of staged diff sent in one request. A longer diff is cut short and said so.
pub const DIFF_LIMIT: usize = 200_000;
/// The longest summary line Git and most tools show in full.
pub const SUMMARY_LIMIT: usize = 72;
/// How long curl may take before the request is given up, in seconds.
const TIMEOUT_SECONDS: u32 = 120;
/// Lock files, whose diffs are large and say little, so each is replaced by a line.
pub const LOCK_FILES: &[&str] = &["Cargo.lock", "package-lock.json", "yarn.lock", "pnpm-lock.yaml"];
/// The message shown when curl could not get an answer from the API.
pub const UNREACHABLE: &str = "Could not reach api.anthropic.com";
/// The note shown when the staged diff was cut short before it was sent.
pub const SHORTENED_NOTE: &str = "The diff was long, so only part of it was sent.";
const NO_MESSAGE: &str = "Claude sent no message.";

/// The instructions the model follows. `conventional` adds the type prefix rule.
pub fn system_prompt(conventional: bool) -> String {
    let mut prompt = String::from(
        "You write Git commit messages. Write a commit message for the staged changes the user shows you. \
         The first line is a summary of at most 72 characters, in the imperative mood (such as Add or Fix), \
         with no trailing period. Then leave one blank line and, only when it helps, write a short body that \
         explains what changed and why, as wrapped plain text. Do not use Markdown headings, code fences, \
         or attribution, sign-off, or trailer lines. Reply with the commit message only.",
    );
    if conventional {
        prompt.push_str(
            " The repository uses Conventional Commits, so start the summary with its type, such as feat, fix, \
             docs, refactor, test, chore, or build, then an optional scope in parentheses, a colon, and a space, \
             as in fix(parser): keep the cursor.",
        );
    }
    prompt
}

/// Whether the model takes the `fallbacks` body field and the beta header. Haiku does not.
pub fn supports_fallbacks(model: &str) -> bool {
    matches!(model, "claude-opus-5-5" | "claude-sonnet-5-5")
}

/// Escapes `text` as a JSON string, quotes included. Non-ASCII text is written as UTF-8.
pub fn json_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            control if (control as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", control as u32);
            }
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

/// The JSON body of a message request. Sampling settings and thinking are left out on purpose.
pub fn request_body(model: &str, system: &str, prompt: &str) -> String {
    let mut body = format!(
        "{{\"model\":{},\"max_tokens\":{MAX_TOKENS},\"output_config\":{{\"effort\":\"low\"}},\"system\":{},\"messages\":[{{\"role\":\"user\",\"content\":{}}}]",
        json_string(model),
        json_string(system),
        json_string(prompt),
    );
    if supports_fallbacks(model) {
        body.push_str(",\"fallbacks\":\"default\"");
    }
    body.push('}');
    body
}

/// Quotes a value for a curl config file: backslashes and double quotes are escaped, and line
/// breaks and tabs become their escapes. Other control characters are dropped.
pub(crate) fn config_quote(value: &str) -> String {
    let mut out = String::from("\"");
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            control if control.is_control() => {}
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

/// What curl is run with. The key is only in `config`, which goes to curl's standard input.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    pub args: Vec<String>,
    pub config: String,
}

/// The curl command for one message request. `body_path` is the file holding the request body.
pub fn curl_plan(key: &str, model: &str, body_path: &str) -> Plan {
    let timeout = TIMEOUT_SECONDS.to_string();
    let args = [
        "--silent",
        "--show-error",
        "--fail-with-body",
        "--proto",
        "=https",
        "--proto-redir",
        "=https",
        "--max-time",
        timeout.as_str(),
        "--config",
        "-",
    ]
    .map(str::to_string)
    .to_vec();
    let mut config = String::new();
    let mut line = |name: &str, value: &str| {
        let _ = writeln!(config, "{name} = {}", config_quote(value));
    };
    line("url", API_URL);
    line("request", "POST");
    line("header", "content-type: application/json");
    line("header", &format!("anthropic-version: {API_VERSION}"));
    line("header", &format!("x-api-key: {key}"));
    if supports_fallbacks(model) {
        line("header", &format!("anthropic-beta: {FALLBACK_BETA}"));
    }
    line("data-binary", &format!("@{body_path}"));
    Plan { args, config }
}

/// Git's answers for the staged changes that the prompt is built from.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StagedChanges {
    /// The subjects of the branch's most recent commits, for the style of the summary.
    pub subjects: Vec<String>,
    pub stat: String,
    pub diff: String,
}

/// Reads the staged changes of the repository in `repo`. Fails when nothing is staged.
pub fn staged_changes(repo: &Path) -> Result<StagedChanges, String> {
    let client = GitClient::new();
    let subjects = client
        .run(&["log", "-n", "10", "--format=%s"], repo)
        .unwrap_or_default()
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(str::to_string)
        .collect();
    let stat =
        client.run(&["diff", "--cached", "--no-ext-diff", "--no-textconv", "--no-color", "--stat"], repo).map_err(|e| e.to_string())?;
    let diff = client.run(&["diff", "--cached", "--no-ext-diff", "--no-textconv", "--no-color", "-U3"], repo).map_err(|e| e.to_string())?;
    if diff.trim().is_empty() {
        return Err("Nothing is staged, so there is no change to describe.".to_string());
    }
    Ok(StagedChanges { subjects, stat, diff })
}

/// The path a `diff --git` header names, from its `b/` side.
fn header_path(header: &str) -> &str {
    header.rsplit_once(" b/").map_or(header, |(_, path)| path)
}

/// The one-line stand-in for a lock file's diff, which counts its changed lines.
fn lock_file_note(section: &str) -> String {
    let header = section.lines().next().unwrap_or_default();
    let path = header_path(header);
    let changed = section
        .lines()
        .filter(|line| (line.starts_with('+') && !line.starts_with("+++")) || (line.starts_with('-') && !line.starts_with("---")))
        .count();
    format!("Changes to {path} are not shown: it is a lock file with {changed} changed lines.\n")
}

/// Replaces the diffs of lock files with a line each. Other sections are kept as they are.
pub fn replace_lock_file_diffs(diff: &str) -> String {
    let mut out = String::with_capacity(diff.len());
    let mut section = String::new();
    for line in diff.split_inclusive('\n') {
        // Content lines start with a space, plus, minus, or backslash, so a header is always a new section.
        if line.starts_with("diff --git ") && !section.is_empty() {
            out.push_str(&keep_or_replace(&section));
            section.clear();
        }
        section.push_str(line);
    }
    if !section.is_empty() {
        out.push_str(&keep_or_replace(&section));
    }
    out
}

fn keep_or_replace(section: &str) -> String {
    let header = section.lines().next().unwrap_or_default();
    let file = header_path(header).rsplit('/').next().unwrap_or_default();
    if LOCK_FILES.contains(&file) {
        lock_file_note(section)
    } else {
        section.to_string()
    }
}

/// The diff as it is sent: lock files replaced, then cut to `limit` bytes at a character boundary.
/// The flag says whether anything was cut.
pub fn prepare_diff(diff: &str, limit: usize) -> (String, bool) {
    let diff = replace_lock_file_diffs(diff);
    if diff.len() <= limit {
        return (diff, false);
    }
    let mut end = limit;
    while !diff.is_char_boundary(end) {
        end -= 1;
    }
    (diff[..end].to_string(), true)
}

/// The user's message: the recent subjects, the stat, and the diff. When the diff was cut, the
/// prompt says so, so the model does not describe what it cannot see.
pub fn user_prompt(subjects: &[String], stat: &str, diff: &str, shortened: bool) -> String {
    let mut prompt = String::from("Recent commit subjects on this branch, for style:\n");
    if subjects.is_empty() {
        prompt.push_str("(none)\n");
    }
    for subject in subjects {
        let _ = writeln!(prompt, "- {subject}");
    }
    let _ = write!(prompt, "\nStaged files (git diff --stat):\n{stat}\nStaged diff:\n");
    if shortened {
        prompt.push_str(
            "The diff was too long to send in full, so it is cut short. Describe only the changes shown, and the file list above.\n",
        );
    }
    prompt.push_str(diff);
    prompt
}

/// Splits a model's answer into its summary and body, after removing any code fence or quotes
/// that wrap it. The summary is the first line; the body is what follows, without its blank lines.
pub fn split_message(text: &str) -> (String, String) {
    let text = strip_wrapping(text);
    let mut lines = text.lines();
    let summary = lines.next().unwrap_or_default().trim().to_string();
    let body = lines.collect::<Vec<_>>().join("\n").trim().to_string();
    (summary, body)
}

fn strip_wrapping(text: &str) -> &str {
    let mut text = text.trim();
    if text.starts_with("```") {
        let rest = text.split_once('\n').map_or("", |(_, rest)| rest);
        let rest = rest.trim_end();
        text = rest.strip_suffix("```").unwrap_or(rest).trim();
    }
    if text.len() >= 2 && text.starts_with('"') && text.ends_with('"') {
        text = text[1..text.len() - 1].trim();
    }
    text
}

/// Reads the body of an answer. `succeeded` is whether curl exited without error. A failed answer
/// with an error object says why; one without means the API could not be reached.
pub fn read_reply(succeeded: bool, body: &str, key: &str) -> Result<String, String> {
    let Ok(value) = parse_json(body) else {
        return Err(if succeeded { "Claude's answer was not in the expected form.".to_string() } else { UNREACHABLE.to_string() });
    };
    if let Some(error) = value.get("error") {
        return Err(api_error(error, key));
    }
    if !succeeded {
        return Err(UNREACHABLE.to_string());
    }
    if value.get("stop_reason").and_then(Json::as_str) == Some("refusal") {
        return Err("Claude declined to write this message.".to_string());
    }
    let text = match value.get("content") {
        Some(Json::Array(blocks)) => blocks
            .iter()
            .filter(|block| block.get("type").and_then(Json::as_str) == Some("text"))
            .filter_map(|block| block.get("text").and_then(Json::as_str))
            .collect::<String>(),
        _ => String::new(),
    };
    if text.trim().is_empty() {
        return Err(NO_MESSAGE.to_string());
    }
    Ok(text)
}

/// The short reason for an API error object. Keys and control characters never reach the person.
fn api_error(error: &Json, key: &str) -> String {
    match error.get("type").and_then(Json::as_str) {
        Some("authentication_error") => "The API key was not accepted.".to_string(),
        Some("rate_limit_error" | "overloaded_error") => "Claude is busy; try again shortly.".to_string(),
        _ => {
            let message = error.get("message").and_then(Json::as_str).unwrap_or_default();
            let short = sanitised(message, key);
            if short.is_empty() {
                "Anthropic's API refused the request.".to_string()
            } else {
                short
            }
        }
    }
}

/// The API's message as one line of at most 300 characters, with the key removed if it appears.
pub fn sanitised(message: &str, key: &str) -> String {
    const LIMIT: usize = 300;
    let spaced: String = message.chars().map(|ch| if ch.is_control() { ' ' } else { ch }).collect();
    let mut line = spaced.split_whitespace().collect::<Vec<_>>().join(" ");
    if !key.is_empty() {
        line = line.replace(key, "[key]");
    }
    if line.chars().count() > LIMIT {
        let mut short: String = line.chars().take(LIMIT - 1).collect();
        short.push('…');
        return short;
    }
    line
}

/// A drafted message and the notes to show with it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Drafted {
    pub summary: String,
    pub body: String,
    pub notes: Vec<String>,
}

/// What a draft is made with: the model, the person's key, and whether to use Conventional Commits.
#[derive(Clone, Copy, Debug)]
pub struct Options<'a> {
    pub model: &'a str,
    pub key: &'a str,
    pub conventional: bool,
}

/// Drafts a commit message for the changes staged in `repo`. Blocks until the answer arrives or
/// `cancel` stops the request, so it runs on a background thread.
pub fn draft_message(repo: &Path, options: &Options, cancel: &Cancel) -> Result<Drafted, String> {
    let staged = staged_changes(repo)?;
    let (diff, shortened) = prepare_diff(&staged.diff, DIFF_LIMIT);
    let prompt = user_prompt(&staged.subjects, &staged.stat, &diff, shortened);
    let body = request_body(options.model, &system_prompt(options.conventional), &prompt);
    let (succeeded, answer) = send(options, &body, cancel)?;
    let text = read_reply(succeeded, &answer, options.key)?;
    let (summary, body) = split_message(&text);
    let mut notes = Vec::new();
    if shortened {
        notes.push(SHORTENED_NOTE.to_string());
    }
    if summary.chars().count() > SUMMARY_LIMIT {
        notes.push(format!("The summary is over {SUMMARY_LIMIT} characters."));
    }
    Ok(Drafted { summary, body, notes })
}

/// Stops a request that is running, or stops one before it starts.
#[derive(Clone, Default)]
pub struct Cancel(Arc<Mutex<CancelState>>);

#[derive(Default)]
struct CancelState {
    cancelled: bool,
    child: Option<Child>,
}

fn lock(state: &Mutex<CancelState>) -> MutexGuard<'_, CancelState> {
    state.lock().unwrap_or_else(|poison| poison.into_inner())
}

impl Cancel {
    pub fn new() -> Self {
        Self::default()
    }

    /// Kills curl if it is running. A request that has not started yet never starts.
    pub fn cancel(&self) {
        let mut state = lock(&self.0);
        state.cancelled = true;
        if let Some(child) = state.child.as_mut() {
            let _ = child.kill();
        }
    }

    fn is_cancelled(&self) -> bool {
        lock(&self.0).cancelled
    }
}

/// Waits for curl to exit while keeping it where `cancel` can reach it. Returns its status, or
/// `None` when it was cancelled or could not be waited for.
///
/// Polls every 25 ms, which is nothing against a request that takes seconds, and lets a cancel
/// kill curl at once. A single blocking wait would leave nothing to kill it with.
fn wait_for_curl(child: Child, cancel: &Cancel) -> Option<ExitStatus> {
    {
        let mut state = lock(&cancel.0);
        if state.cancelled {
            let mut child = child;
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        state.child = Some(child);
    }
    loop {
        {
            let mut state = lock(&cancel.0);
            let child = state.child.as_mut()?;
            match child.try_wait() {
                Ok(Some(status)) => {
                    state.child = None;
                    return Some(status);
                }
                Ok(None) => {}
                Err(_) => {
                    state.child = None;
                    return None;
                }
            }
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

/// A folder only this process can read, holding the request body. It is removed when dropped.
struct PrivateDirectory(PathBuf);

impl PrivateDirectory {
    fn create() -> std::io::Result<Self> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |elapsed| elapsed.as_nanos());
        let name = format!("nicegit-draft-{}-{nanos}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed));
        let path = std::env::temp_dir().join(name);
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(&path)?;
        Ok(Self(path))
    }
}

impl Drop for PrivateDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Writes `bytes` to a new file that only its owner can read.
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(bytes)
}

/// Posts the body through curl. Returns whether curl succeeded and what it printed.
fn send(options: &Options, body: &str, cancel: &Cancel) -> Result<(bool, String), String> {
    let curl = find_program("curl").ok_or_else(|| "curl is not installed.".to_string())?;
    let prepared = |error: std::io::Error| format!("The request could not be prepared: {error}");
    let directory = PrivateDirectory::create().map_err(prepared)?;
    let body_path: PathBuf = directory.0.join("request.json");
    write_private(&body_path, body.as_bytes()).map_err(prepared)?;
    let plan = curl_plan(options.key, options.model, &body_path.to_string_lossy());

    let mut command = Command::new(curl);
    command.args(&plan.args).env("PATH", search_path()).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // Without this, the request flashes a console window on Windows.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = command.spawn().map_err(|error| format!("curl did not start: {error}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        // The config holds the key, so it goes in here and never in the arguments.
        let _ = stdin.write_all(plan.config.as_bytes());
    }
    let stdout = child.stdout.take();
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        if let Some(mut stdout) = stdout {
            let _ = stdout.read_to_end(&mut bytes);
        }
        bytes
    });
    let status = wait_for_curl(child, cancel);
    let bytes = reader.join().unwrap_or_default();
    // The body file is needed until curl has exited, so the folder goes only now.
    drop(directory);
    if cancel.is_cancelled() {
        return Err("Drafting was cancelled.".to_string());
    }
    let Some(status) = status else {
        return Err(UNREACHABLE.to_string());
    };
    Ok((status.success(), String::from_utf8_lossy(&bytes).into_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field<'a>(value: &'a Json, key: &str) -> &'a Json {
        value.get(key).unwrap_or_else(|| panic!("no field {key}"))
    }

    #[test]
    fn json_strings_escape_quotes_backslashes_and_control_characters() {
        assert_eq!(json_string("say \"hi\""), r#""say \"hi\"""#);
        assert_eq!(json_string(r"a\b"), r#""a\\b""#);
        assert_eq!(json_string("line\nnext\ttab\r"), r#""line\nnext\ttab\r""#);
        assert_eq!(json_string("\u{1}\u{8}\u{c}\u{1f}"), r#""\u0001\b\f\u001f""#);
    }

    #[test]
    fn json_strings_read_back_to_the_same_text() {
        for text in ["plain", "quotes \" and \\ slashes", "control \u{0} \u{7} \n\r\t", "accents é, emoji 🦀, Greek λ"] {
            let parsed = parse_json(&json_string(text)).expect("valid JSON");
            assert_eq!(parsed.as_str(), Some(text));
        }
    }

    #[test]
    fn the_body_names_the_model_and_limits_and_leaves_out_sampling_settings() {
        let body = request_body("claude-haiku-5-5", "Be brief.", "The prompt");
        let value = parse_json(&body).expect("valid JSON");
        assert_eq!(field(&value, "model").as_str(), Some("claude-haiku-5-5"));
        assert_eq!(field(&value, "max_tokens").as_number(), Some(2000.0));
        assert_eq!(field(&value, "output_config").get("effort").and_then(Json::as_str), Some("low"));
        assert_eq!(field(&value, "system").as_str(), Some("Be brief."));
        for banned in ["temperature", "top_p", "top_k", "thinking"] {
            assert!(value.get(banned).is_none(), "{banned} is not sent");
        }
    }

    #[test]
    fn fallbacks_are_sent_for_opus_and_sonnet_only() {
        for model in ["claude-opus-5-5", "claude-sonnet-5-5"] {
            let value = parse_json(&request_body(model, "s", "p")).unwrap();
            assert_eq!(field(&value, "fallbacks").as_str(), Some("default"), "{model}");
            assert!(supports_fallbacks(model));
        }
        let haiku = parse_json(&request_body("claude-haiku-5-5", "s", "p")).unwrap();
        assert!(haiku.get("fallbacks").is_none());
        assert!(!supports_fallbacks("claude-haiku-5-5"));
    }

    #[test]
    fn the_prompt_round_trips_through_the_body() {
        let prompt = "Recent:\n- \"quoted\"\nDiff:\n+let x = \"é\";\n";
        let value = parse_json(&request_body("claude-opus-5-5", "s", prompt)).unwrap();
        let message = match field(&value, "messages") {
            Json::Array(messages) => messages[0].clone(),
            other => panic!("messages is {other:?}"),
        };
        assert_eq!(message.get("role").and_then(Json::as_str), Some("user"));
        assert_eq!(message.get("content").and_then(Json::as_str), Some(prompt));
    }

    #[test]
    fn config_values_escape_quotes_backslashes_and_line_breaks() {
        assert_eq!(config_quote(r#"a"b\c"#), r#""a\"b\\c""#);
        assert_eq!(config_quote("one\ntwo\r\tthree"), r#""one\ntwo\r\tthree""#);
        assert_eq!(config_quote("bell\u{7}"), r#""bell""#, "other control characters are dropped");
    }

    #[test]
    fn the_key_is_never_in_the_arguments() {
        let key = "sk-ant-test-secret-123";
        let plan = curl_plan(key, "claude-opus-5-5", "/tmp/private/request.json");
        assert!(plan.args.iter().all(|arg| !arg.contains(key)), "{:?}", plan.args);
        assert!(plan.config.contains(&format!("x-api-key: {key}")), "the key goes to curl's config");
        assert!(plan.args.contains(&"--config".to_string()) && plan.args.contains(&"-".to_string()));
    }

    #[test]
    fn the_plan_posts_to_the_api_over_https_with_the_body_file() {
        let plan = curl_plan("k", "claude-sonnet-5-5", "/tmp/x/request.json");
        for expected in [
            "url = \"https://api.anthropic.com/v1/messages\"",
            "request = \"POST\"",
            "header = \"content-type: application/json\"",
            "header = \"anthropic-version: 2026-10-21\"",
            "data-binary = \"@/tmp/x/request.json\"",
        ] {
            assert!(plan.config.lines().any(|line| line == expected), "{expected} in {}", plan.config);
        }
        assert!(plan.args.windows(2).any(|pair| pair == ["--proto", "=https"]));
        assert!(plan.args.contains(&"--fail-with-body".to_string()));
    }

    #[test]
    fn the_beta_header_is_sent_for_opus_and_sonnet_only() {
        let beta = format!("anthropic-beta: {FALLBACK_BETA}");
        assert!(curl_plan("k", "claude-opus-5-5", "p").config.contains(&beta));
        assert!(curl_plan("k", "claude-sonnet-5-5", "p").config.contains(&beta));
        assert!(!curl_plan("k", "claude-haiku-5-5", "p").config.contains("anthropic-beta"));
    }

    #[test]
    fn a_key_with_quotes_stays_inside_its_config_value() {
        let plan = curl_plan("a\"b\\c", "claude-haiku-5-5", "p");
        assert!(plan.config.contains(r#"header = "x-api-key: a\"b\\c""#), "{}", plan.config);
    }

    #[test]
    fn text_blocks_are_joined_for_the_answer() {
        let reply = r#"{"id":"m","stop_reason":"end_turn","content":[{"type":"thinking","thinking":"no"},{"type":"text","text":"Add x"},{"type":"text","text":"\n\nBody."}]}"#;
        assert_eq!(read_reply(true, reply, "k").as_deref(), Ok("Add x\n\nBody."));
    }

    #[test]
    fn a_refusal_is_reported_as_a_refusal() {
        let reply = r#"{"stop_reason":"refusal","content":[{"type":"text","text":"no"}]}"#;
        assert_eq!(read_reply(true, reply, "k"), Err("Claude declined to write this message.".to_string()));
    }

    #[test]
    fn a_long_answer_is_still_used() {
        let reply = r#"{"stop_reason":"max_tokens","content":[{"type":"text","text":"Add the parser\n\nPartial bo"}]}"#;
        assert_eq!(read_reply(true, reply, "k").as_deref(), Ok("Add the parser\n\nPartial bo"));
    }

    #[test]
    fn an_answer_without_text_is_an_error() {
        let reply = r#"{"stop_reason":"end_turn","content":[{"type":"thinking","thinking":"x"}]}"#;
        assert_eq!(read_reply(true, reply, "k"), Err(NO_MESSAGE.to_string()));
        assert!(read_reply(true, "not json", "k").is_err());
    }

    #[test]
    fn api_errors_are_mapped_to_short_reasons() {
        let auth = r#"{"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key"}}"#;
        assert_eq!(read_reply(false, auth, "k"), Err("The API key was not accepted.".to_string()));
        for kind in ["rate_limit_error", "overloaded_error"] {
            let body = format!(r#"{{"type":"error","error":{{"type":"{kind}","message":"slow down"}}}}"#);
            assert_eq!(read_reply(false, &body, "k"), Err("Claude is busy; try again shortly.".to_string()), "{kind}");
        }
        let other = r#"{"type":"error","error":{"type":"not_found_error","message":"model:  claude-x\nnot found"}}"#;
        assert_eq!(read_reply(false, other, "k"), Err("model: claude-x not found".to_string()));
    }

    #[test]
    fn a_failed_request_without_an_error_object_is_an_unreachable_api() {
        assert_eq!(read_reply(false, "", "k"), Err(UNREACHABLE.to_string()));
        assert_eq!(read_reply(false, "<html>", "k"), Err(UNREACHABLE.to_string()));
    }

    #[test]
    fn api_messages_are_shortened_and_lose_the_key() {
        let long = format!("{} sk-secret", "word ".repeat(100));
        let short = sanitised(&long, "sk-secret");
        assert!(short.chars().count() <= 300, "{}", short.chars().count());
        assert!(short.ends_with('…'));
        assert!(!short.contains("sk-secret"));
        assert_eq!(sanitised("", "k"), "");
    }

    #[test]
    fn fences_and_quotes_are_removed_before_the_summary_is_read() {
        assert_eq!(split_message("```\nAdd parser\n\nBody text.\n```"), ("Add parser".to_string(), "Body text.".to_string()));
        assert_eq!(split_message("```text\nFix cursor\n```"), ("Fix cursor".to_string(), String::new()));
        assert_eq!(split_message("\"Add parser\""), ("Add parser".to_string(), String::new()));
        assert_eq!(
            split_message("  Add parser  \n\n\n  First line of body\nSecond\n"),
            ("Add parser".to_string(), "First line of body\nSecond".to_string())
        );
    }

    #[test]
    fn a_summary_alone_has_an_empty_body() {
        assert_eq!(split_message("Add parser"), ("Add parser".to_string(), String::new()));
    }

    #[test]
    fn lock_file_diffs_become_one_line_each() {
        let diff = "diff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1 +1 @@\n-old\n+new\n\
                    diff --git a/Cargo.lock b/Cargo.lock\n--- a/Cargo.lock\n+++ b/Cargo.lock\n@@ -1,3 +1,3 @@\n-a\n-b\n+c\n \n\
                    diff --git a/web/yarn.lock b/web/yarn.lock\n@@ -1 +1 @@\n-x\n+y\n";
        let replaced = replace_lock_file_diffs(diff);
        assert!(replaced.contains("diff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs"), "ordinary files keep their diffs");
        assert!(replaced.contains("Changes to Cargo.lock are not shown: it is a lock file with 3 changed lines.\n"), "{replaced}");
        assert!(replaced.contains("Changes to web/yarn.lock are not shown: it is a lock file with 2 changed lines.\n"), "{replaced}");
        assert!(!replaced.contains("+c"), "lock file lines are gone");
    }

    #[test]
    fn a_content_line_that_looks_like_a_header_does_not_end_a_section() {
        let diff = "diff --git a/notes.txt b/notes.txt\n@@ -1 +1 @@\n-x\n+++ b/Cargo.lock\n";
        assert_eq!(replace_lock_file_diffs(diff), diff, "the plus line is content, and notes.txt is not a lock file");
    }

    #[test]
    fn a_long_diff_is_cut_at_the_limit_and_flagged() {
        let (cut, flagged) = prepare_diff(&"x".repeat(50), 20);
        assert_eq!(cut.len(), 20);
        assert!(flagged);
        let (whole, flagged) = prepare_diff("short", 20);
        assert_eq!(whole, "short");
        assert!(!flagged);
    }

    #[test]
    fn the_cut_keeps_a_character_whole() {
        let (cut, flagged) = prepare_diff("aé", 2); // é takes two bytes, so byte 2 splits it
        assert!(flagged);
        assert_eq!(cut, "a");
    }

    #[test]
    fn a_cut_diff_is_said_so_in_the_prompt() {
        let full = user_prompt(&["Fix x".to_string()], " a.rs | 1 +", "+y", false);
        assert!(!full.contains("cut short"));
        assert!(full.contains("- Fix x"));
        let cut = user_prompt(&[], "stat", "+y", true);
        assert!(cut.contains("cut short"));
        assert!(cut.contains("(none)"));
    }

    #[test]
    fn the_system_prompt_adds_the_convention_only_when_asked() {
        assert!(!system_prompt(false).contains("Conventional Commits"));
        assert!(system_prompt(true).contains("fix(parser): keep the cursor"));
    }

    #[test]
    fn cancelling_before_a_request_starts_stops_it() {
        let cancel = Cancel::new();
        cancel.cancel();
        let mut command = Command::new(if cfg!(windows) { "cmd" } else { "sleep" });
        command.args(if cfg!(windows) { &["/C", "timeout 5 >NUL"][..] } else { &["5"][..] }).stdout(Stdio::null());
        let child = command.spawn().expect("spawn a stand-in process");
        assert!(wait_for_curl(child, &cancel).is_none(), "a cancelled request is not waited for");
    }

    #[test]
    fn staged_changes_read_the_staged_diff_and_recent_subjects() {
        let dir = tempfile::tempdir().expect("temporary folder");
        let path = dir.path();
        let run = |args: &[&str]| {
            let output = Command::new("git").args(args).current_dir(path).output().expect("run git");
            assert!(output.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&output.stderr));
        };
        run(&["init", "-q", "-b", "main"]);
        run(&["config", "user.name", "Test"]);
        run(&["config", "user.email", "test@example.invalid"]);
        run(&["config", "commit.gpgsign", "false"]);
        std::fs::write(path.join("a.txt"), "one\n").unwrap();
        run(&["add", "a.txt"]);
        run(&["commit", "-q", "-m", "Start a file"]);
        std::fs::write(path.join("a.txt"), "one\ntwo\n").unwrap();
        run(&["add", "a.txt"]);
        let staged = staged_changes(path).expect("something is staged");
        assert_eq!(staged.subjects, vec!["Start a file".to_string()]);
        assert!(staged.stat.contains("a.txt"));
        assert!(staged.diff.contains("+two"));
        run(&["commit", "-q", "-m", "Add a line"]);
        assert!(staged_changes(path).is_err(), "nothing staged is an error");
    }
}
