//! GitHub pull requests and issues for a github.com remote, read through the `gh` command-line
//! tool. NiceGit never stores a token; `gh` keeps its own sign-in.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::client::GitClient;
use crate::models::{GitError, Result};
use crate::runner;

const NOT_INSTALLED: &str = "The GitHub CLI (gh) is not installed. Install it from cli.github.com, then sign in with gh auth login.";
const TIMEOUT: Duration = Duration::from_secs(30);
/// Longest `gh` error shown to the user.
const MESSAGE_LIMIT: usize = 2000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemKind {
    PullRequest,
    Issue,
}

impl ItemKind {
    pub fn title(self) -> &'static str {
        match self {
            ItemKind::PullRequest => "Pull requests",
            ItemKind::Issue => "Issues",
        }
    }

    /// The `gh` subcommand that lists this kind.
    fn subcommand(self) -> &'static str {
        match self {
            ItemKind::PullRequest => "pr",
            ItemKind::Issue => "issue",
        }
    }

    /// The path segment GitHub uses for this kind in item links.
    fn path_segment(self) -> &'static str {
        match self {
            ItemKind::PullRequest => "pull",
            ItemKind::Issue => "issues",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemState {
    Open,
    Closed,
    /// Pull requests only.
    Merged,
    All,
}

impl ItemState {
    pub fn as_str(self) -> &'static str {
        match self {
            ItemState::Open => "open",
            ItemState::Closed => "closed",
            ItemState::Merged => "merged",
            ItemState::All => "all",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            ItemState::Open => "Open",
            ItemState::Closed => "Closed",
            ItemState::Merged => "Merged",
            ItemState::All => "All",
        }
    }
}

/// The owner and name of a github.com repository.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitHubRepository {
    pub owner: String,
    pub name: String,
}

impl GitHubRepository {
    pub fn slug(&self) -> String {
        format!("{}/{}", self.owner, self.name)
    }

    /// Reads the repository from a remote address. Accepts `https://` and `ssh://` URLs, and
    /// scp-like `user@github.com:owner/repo.git` addresses. Other hosts, ports, passwords,
    /// queries, and fragments are refused.
    pub fn parse(address: &str) -> Result<Self> {
        let address = address.trim();
        let unsupported = || GitError::failed("GitHub", "This remote is not a supported github.com repository.");
        let path: String = if let Some((scheme, rest)) = address.split_once("://") {
            if !(scheme.eq_ignore_ascii_case("https") || scheme.eq_ignore_ascii_case("ssh")) {
                return Err(unsupported());
            }
            let (authority, path) = match rest.find('/') {
                Some(index) => (&rest[..index], &rest[index..]),
                None => (rest, ""),
            };
            if path.contains('?') || path.contains('#') || authority.contains('?') || authority.contains('#') {
                return Err(unsupported());
            }
            let host = match authority.rsplit_once('@') {
                Some((userinfo, host)) => {
                    // A user name is allowed; a password is not.
                    if userinfo.contains(':') {
                        return Err(unsupported());
                    }
                    host
                }
                None => authority,
            };
            if !host.eq_ignore_ascii_case("github.com") {
                return Err(unsupported());
            }
            path.to_string()
        } else {
            // scp-like form: [user@]host:path. A local path such as /srv/repo has no host match.
            let Some((host_part, path)) = address.split_once(':') else { return Err(unsupported()) };
            let host = host_part.rsplit_once('@').map(|(_, host)| host).unwrap_or(host_part);
            if !host.eq_ignore_ascii_case("github.com") {
                return Err(unsupported());
            }
            path.to_string()
        };

        let parts: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
        if parts.len() != 2 {
            return Err(GitError::failed("GitHub", "The GitHub remote must identify an owner and repository."));
        }
        let name = parts[1].strip_suffix(".git").unwrap_or(parts[1]);
        let valid = |part: &str| {
            !part.is_empty()
                && part != "."
                && part != ".."
                && part.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
        };
        if !valid(parts[0]) || !valid(name) {
            return Err(GitError::failed("GitHub", "The GitHub repository address is invalid."));
        }
        Ok(GitHubRepository { owner: parts[0].to_string(), name: name.to_string() })
    }
}

/// A label on an item, with its colour as six hex digits and no leading `#`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitHubLabel {
    pub name: String,
    pub color: String,
}

/// A pull request or issue as `gh` reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitHubItem {
    pub number: u64,
    pub title: String,
    pub url: String,
    pub author: Option<String>,
    /// GitHub's state word, such as `OPEN`, `CLOSED`, or `MERGED`.
    pub state: Option<String>,
    pub is_draft: bool,
    pub labels: Vec<GitHubLabel>,
}

impl GitHubItem {
    /// Whether the item matches a filter: its title or author contains the text, or the text is
    /// its number, with or without a leading `#`.
    pub fn matches(&self, query: &str) -> bool {
        let query = query.trim();
        if query.is_empty() {
            return true;
        }
        let lowered = query.to_lowercase();
        self.title.to_lowercase().contains(&lowered)
            || self.author.as_ref().is_some_and(|author| author.to_lowercase().contains(&lowered))
            || self.number.to_string() == query.strip_prefix('#').unwrap_or(query)
    }
}

/// Whether `gh` can be used for the repository.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GhStatus {
    NotInstalled,
    /// Installed, but not signed in; carries `gh`'s explanation.
    SignedOut(String),
    SignedIn,
}

impl GitClient {
    /// The github.com repository behind a remote's first fetch address.
    pub fn github_repository(&self, remote: &str, directory: &Path) -> Result<GitHubRepository> {
        if remote.is_empty() || remote.starts_with('-') {
            return Err(GitError::failed("GitHub", "Choose a remote first."));
        }
        let output = self.run(&["remote", "get-url", "--all", remote], directory)?;
        let address = output.lines().next().unwrap_or("");
        GitHubRepository::parse(address)
    }

    /// Whether `gh` is installed and signed in to github.com.
    pub fn gh_status(&self, directory: &Path) -> GhStatus {
        if gh_executable().is_none() {
            return GhStatus::NotInstalled;
        }
        match run_gh(&["auth", "status", "--hostname", "github.com"], directory) {
            Ok(_) => GhStatus::SignedIn,
            Err(message) => GhStatus::SignedOut(message),
        }
    }

    /// Lists pull requests or issues, newest first, through `gh`. `limit` is clamped to 1 to 1000.
    pub fn github_items(
        &self,
        repository: &GitHubRepository,
        kind: ItemKind,
        state: ItemState,
        limit: usize,
        directory: &Path,
    ) -> Result<Vec<GitHubItem>> {
        if kind == ItemKind::Issue && state == ItemState::Merged {
            return Err(GitError::failed("GitHub", "Merged applies to pull requests, not issues."));
        }
        let limit = limit.clamp(1, 1000).to_string();
        let repo = format!("github.com/{}", repository.slug());
        let fields = match kind {
            ItemKind::PullRequest => "number,title,url,author,state,isDraft,labels",
            ItemKind::Issue => "number,title,url,author,state,labels",
        };
        let output = run_gh(
            &[kind.subcommand(), "list", "--repo", &repo, "--state", state.as_str(), "--limit", &limit, "--json", fields],
            directory,
        )
        .map_err(|message| GitError::failed("GitHub", message))?;
        decode_items(&output, repository, kind)
    }
}

/// Decodes `gh --json` output for one kind of item, refusing links that do not point at the
/// repository's own item pages.
pub fn decode_items(json: &str, repository: &GitHubRepository, kind: ItemKind) -> Result<Vec<GitHubItem>> {
    let unexpected = || GitError::failed("GitHub", "GitHub returned unexpected items.");
    let value = parse_json(json).map_err(|_| unexpected())?;
    let Json::Array(entries) = value else { return Err(unexpected()) };
    let expected_prefix = format!("https://github.com/{}/{}/", repository.slug(), kind.path_segment()).to_ascii_lowercase();
    let mut items = Vec::with_capacity(entries.len());
    for entry in &entries {
        let Json::Object(_) = entry else { return Err(unexpected()) };
        let number = entry.get("number").and_then(Json::as_number).filter(|n| n.fract() == 0.0 && *n >= 1.0 && *n < 9.0e15);
        let number = number.ok_or_else(unexpected)? as u64;
        let title = entry.get("title").and_then(Json::as_str).ok_or_else(unexpected)?.to_string();
        let url = entry.get("url").and_then(Json::as_str).ok_or_else(unexpected)?.to_string();
        let expected = format!("{expected_prefix}{number}").to_ascii_lowercase();
        if url.to_ascii_lowercase() != expected {
            return Err(unexpected());
        }
        let author = entry.get("author").and_then(|author| author.get("login")).and_then(Json::as_str).map(str::to_string);
        let state = entry.get("state").and_then(Json::as_str).map(str::to_string);
        let is_draft = entry.get("isDraft").and_then(Json::as_bool).unwrap_or(false);
        let mut labels = Vec::new();
        if let Some(Json::Array(values)) = entry.get("labels") {
            for label in values {
                let name = label.get("name").and_then(Json::as_str).ok_or_else(unexpected)?.to_string();
                let color = label.get("color").and_then(Json::as_str).unwrap_or("").trim_start_matches('#').to_string();
                labels.push(GitHubLabel { name, color });
            }
        }
        items.push(GitHubItem { number, title, url, author, state, is_draft, labels });
    }
    let mut numbers: Vec<u64> = items.iter().map(|item| item.number).collect();
    numbers.sort_unstable();
    numbers.dedup();
    if numbers.len() != items.len() {
        return Err(unexpected());
    }
    Ok(items)
}

/// The first `gh` on the search path that is an executable file.
fn gh_executable() -> Option<PathBuf> {
    let name = if cfg!(windows) { "gh.exe" } else { "gh" };
    std::env::split_paths(runner::search_path()).map(|directory| directory.join(name)).find(|candidate| is_executable(candidate))
}

fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata().map(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0).unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

/// Runs `gh` and returns its standard output. On failure the error is the text to show.
/// Prompts are disabled, and the request is abandoned after a timeout.
fn run_gh(arguments: &[&str], directory: &Path) -> std::result::Result<String, String> {
    let executable = gh_executable().ok_or_else(|| NOT_INSTALLED.to_string())?;
    let mut command = Command::new(executable);
    command.args(arguments).current_dir(directory).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    command.env("PATH", runner::search_path());
    // Variables that would point gh at another repository or host.
    command.env_remove("GH_REPO");
    command.env_remove("GH_HOST");
    command.env("GH_PROMPT_DISABLED", "1");
    command.env("GH_NO_UPDATE_NOTIFIER", "1");
    command.env("NO_COLOR", "1");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // Without this, every gh call flashes a console window.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = command.spawn().map_err(|error| error.to_string())?;
    // Read both pipes while waiting, so a large output cannot block the process.
    let mut stdout = child.stdout.take();
    let mut stderr = child.stderr.take();
    let out_reader = std::thread::spawn(move || {
        let mut buffer = Vec::new();
        if let Some(pipe) = stdout.as_mut() {
            let _ = pipe.read_to_end(&mut buffer);
        }
        buffer
    });
    let err_reader = std::thread::spawn(move || {
        let mut buffer = Vec::new();
        if let Some(pipe) = stderr.as_mut() {
            let _ = pipe.read_to_end(&mut buffer);
        }
        buffer
    });
    let deadline = Instant::now() + TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("GitHub request timed out. Retry when the connection is available.".to_string());
            }
            Err(error) => return Err(error.to_string()),
        }
    };
    let stdout = out_reader.join().unwrap_or_default();
    let stderr = err_reader.join().unwrap_or_default();
    if status.success() {
        return Ok(String::from_utf8_lossy(&stdout).into_owned());
    }
    let message = String::from_utf8_lossy(&stderr).trim().to_string();
    let message = if message.is_empty() { "GitHub request failed.".to_string() } else { message };
    Err(message.chars().take(MESSAGE_LIMIT).collect())
}

// MARK: JSON

/// A parsed JSON value. Only what `gh` output needs is supported, which is all of the standard
/// grammar.
#[derive(Clone, Debug, PartialEq)]
enum Json {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<Json>),
    Object(Vec<(String, Json)>),
}

impl Json {
    fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Object(fields) => fields.iter().find(|(name, _)| name == key).map(|(_, value)| value),
            _ => None,
        }
    }

    fn as_str(&self) -> Option<&str> {
        match self {
            Json::String(text) => Some(text),
            _ => None,
        }
    }

    fn as_number(&self) -> Option<f64> {
        match self {
            Json::Number(number) => Some(*number),
            _ => None,
        }
    }

    fn as_bool(&self) -> Option<bool> {
        match self {
            Json::Bool(value) => Some(*value),
            _ => None,
        }
    }
}

const MAX_DEPTH: usize = 64;

fn parse_json(text: &str) -> std::result::Result<Json, String> {
    let mut parser = JsonParser { bytes: text.as_bytes(), position: 0 };
    let value = parser.value(0)?;
    parser.skip_whitespace();
    if parser.position != parser.bytes.len() {
        return Err("Unexpected text after the JSON value.".to_string());
    }
    Ok(value)
}

struct JsonParser<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl JsonParser<'_> {
    fn skip_whitespace(&mut self) {
        while let Some(byte) = self.bytes.get(self.position) {
            if matches!(byte, b' ' | b'\t' | b'\n' | b'\r') {
                self.position += 1;
            } else {
                break;
            }
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.position).copied()
    }

    fn expect_literal(&mut self, literal: &str) -> std::result::Result<(), String> {
        if self.bytes[self.position..].starts_with(literal.as_bytes()) {
            self.position += literal.len();
            Ok(())
        } else {
            Err(format!("Expected {literal}."))
        }
    }

    fn value(&mut self, depth: usize) -> std::result::Result<Json, String> {
        if depth > MAX_DEPTH {
            return Err("JSON is nested too deeply.".to_string());
        }
        self.skip_whitespace();
        match self.peek().ok_or("Unexpected end of JSON.")? {
            b'{' => self.object(depth),
            b'[' => self.array(depth),
            b'"' => self.string().map(Json::String),
            b't' => self.expect_literal("true").map(|_| Json::Bool(true)),
            b'f' => self.expect_literal("false").map(|_| Json::Bool(false)),
            b'n' => self.expect_literal("null").map(|_| Json::Null),
            b'-' | b'0'..=b'9' => self.number(),
            _ => Err("Unexpected character in JSON.".to_string()),
        }
    }

    fn object(&mut self, depth: usize) -> std::result::Result<Json, String> {
        self.position += 1;
        let mut fields = Vec::new();
        self.skip_whitespace();
        if self.peek() == Some(b'}') {
            self.position += 1;
            return Ok(Json::Object(fields));
        }
        loop {
            self.skip_whitespace();
            if self.peek() != Some(b'"') {
                return Err("Expected an object key.".to_string());
            }
            let key = self.string()?;
            self.skip_whitespace();
            if self.peek() != Some(b':') {
                return Err("Expected ':' in an object.".to_string());
            }
            self.position += 1;
            let value = self.value(depth + 1)?;
            fields.push((key, value));
            self.skip_whitespace();
            match self.peek() {
                Some(b',') => self.position += 1,
                Some(b'}') => {
                    self.position += 1;
                    return Ok(Json::Object(fields));
                }
                _ => return Err("Expected ',' or '}' in an object.".to_string()),
            }
        }
    }

    fn array(&mut self, depth: usize) -> std::result::Result<Json, String> {
        self.position += 1;
        let mut values = Vec::new();
        self.skip_whitespace();
        if self.peek() == Some(b']') {
            self.position += 1;
            return Ok(Json::Array(values));
        }
        loop {
            values.push(self.value(depth + 1)?);
            self.skip_whitespace();
            match self.peek() {
                Some(b',') => self.position += 1,
                Some(b']') => {
                    self.position += 1;
                    return Ok(Json::Array(values));
                }
                _ => return Err("Expected ',' or ']' in an array.".to_string()),
            }
        }
    }

    fn number(&mut self) -> std::result::Result<Json, String> {
        let start = self.position;
        while let Some(byte) = self.peek() {
            if byte.is_ascii_digit() || matches!(byte, b'-' | b'+' | b'.' | b'e' | b'E') {
                self.position += 1;
            } else {
                break;
            }
        }
        let text = std::str::from_utf8(&self.bytes[start..self.position]).map_err(|_| "Invalid number.".to_string())?;
        text.parse::<f64>().map(Json::Number).map_err(|_| "Invalid number.".to_string())
    }

    fn hex4(&mut self) -> std::result::Result<u32, String> {
        let digits = self.bytes.get(self.position..self.position + 4).ok_or("Invalid unicode escape.")?;
        let text = std::str::from_utf8(digits).map_err(|_| "Invalid unicode escape.".to_string())?;
        let value = u32::from_str_radix(text, 16).map_err(|_| "Invalid unicode escape.".to_string())?;
        self.position += 4;
        Ok(value)
    }

    fn string(&mut self) -> std::result::Result<String, String> {
        self.position += 1;
        let mut buffer: Vec<u8> = Vec::new();
        loop {
            let byte = self.peek().ok_or("Unterminated string in JSON.")?;
            self.position += 1;
            match byte {
                b'"' => break,
                b'\\' => {
                    let escape = self.peek().ok_or("Unterminated escape in JSON.")?;
                    self.position += 1;
                    let character = match escape {
                        b'"' => '"',
                        b'\\' => '\\',
                        b'/' => '/',
                        b'b' => '\u{8}',
                        b'f' => '\u{c}',
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        b'u' => {
                            let high = self.hex4()?;
                            if (0xD800..0xDC00).contains(&high) && self.bytes[self.position..].starts_with(b"\\u") {
                                self.position += 2;
                                let low = self.hex4()?;
                                if !(0xDC00..0xE000).contains(&low) {
                                    return Err("Invalid surrogate pair in JSON.".to_string());
                                }
                                let code = 0x10000 + ((high - 0xD800) << 10) + (low - 0xDC00);
                                char::from_u32(code).ok_or("Invalid surrogate pair in JSON.")?
                            } else {
                                char::from_u32(high).unwrap_or('\u{FFFD}')
                            }
                        }
                        _ => return Err("Invalid escape in JSON.".to_string()),
                    };
                    let mut encoded = [0u8; 4];
                    buffer.extend_from_slice(character.encode_utf8(&mut encoded).as_bytes());
                }
                // Control characters must be escaped in JSON strings.
                0..=0x1f => return Err("Unescaped control character in JSON.".to_string()),
                other => buffer.push(other),
            }
        }
        String::from_utf8(buffer).map_err(|_| "Invalid UTF-8 in JSON.".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nested_values_and_escapes() {
        let value = parse_json(r#"[{"n": 12, "t": "a\"bé😀", "ok": true, "x": null}]"#).unwrap();
        let Json::Array(items) = value else { panic!("expected an array") };
        assert_eq!(items[0].get("n").and_then(Json::as_number), Some(12.0));
        assert_eq!(items[0].get("t").and_then(Json::as_str), Some("a\"b\u{e9}\u{1F600}"));
        assert_eq!(items[0].get("ok").and_then(Json::as_bool), Some(true));
        assert_eq!(items[0].get("x"), Some(&Json::Null));
    }

    #[test]
    fn rejects_malformed_json() {
        assert!(parse_json("[1,").is_err());
        assert!(parse_json("[1] x").is_err());
        assert!(parse_json("\"a\nb\"").is_err());
    }
}
