//! Git LFS: which patterns store files with LFS, which files it holds, whether their content
//! is on this machine, and which files are locked on the remote's LFS server. Tracking edits the
//! root `.gitattributes`, as `git lfs track` does.

use std::io::Read;
use std::path::Path;

use crate::client::GitClient;
use crate::json::{parse_json, Json};
use crate::models::{GitError, Result};
use crate::runner::{self, RunOptions};

const ATTRIBUTES_FILE: &str = ".gitattributes";
const POINTER_PREFIX: &[u8] = b"version https://git-lfs.github.com/spec/v1";
/// Paths sent to `check-attr` per command, which keeps argument lists short.
const ATTRIBUTE_BATCH: usize = 500;
/// Lock commands receive a user-selected path, so glob characters in its name must not match others.
const LITERAL_PATHSPECS: &[(&str, &str)] = &[("GIT_LITERAL_PATHSPECS", "1")];
const NOT_INSTALLED: &str =
    "Git LFS is missing, so file locks cannot be read or set. Install it from https://git-lfs.com, then run `git lfs install` once.";
const NO_REMOTE: &str = "This repository has no remote, so its file locks cannot be read. Locks are kept by the remote's Git LFS server.";

/// A file that Git LFS stores.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LfsFile {
    pub path: String,
    /// The working file is still an LFS pointer because its content has not been downloaded.
    pub is_pointer_only: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LfsStatus {
    /// The installed git-lfs version, or None when Git LFS is not installed.
    pub version: Option<String>,
    /// Patterns in the root `.gitattributes` that store matching files with Git LFS.
    pub patterns: Vec<String>,
    pub files: Vec<LfsFile>,
}

/// A lock on a file, as the remote's Git LFS server reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LfsLock {
    /// The server's ID for the lock, which `unlock` can name exactly.
    pub id: String,
    pub path: String,
    pub owner: String,
    /// When the lock was taken, in seconds since the Unix epoch. None when the time cannot be read.
    pub locked_at: Option<i64>,
    /// Whether this user holds the lock. See [`LfsLocks::verified`].
    pub ours: bool,
}

/// The locks the remote holds for a repository, and how each lock's ownership was decided.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LfsLocks {
    pub locks: Vec<LfsLock>,
    /// True when the server said which locks are ours (`git lfs locks --verify`). Otherwise each
    /// owner is compared with `user_name`.
    pub verified: bool,
    /// The configured `user.name`, empty when it is not set.
    pub user_name: String,
}

/// The lock an unlock names: its server ID, which is exact, or the file's path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LockTarget {
    Id(String),
    Path(String),
}

impl GitClient {
    /// The installed Git LFS version, the tracked patterns, and every tracked file that uses LFS.
    /// Reading does not need Git LFS to be installed.
    pub fn lfs_status(&self, directory: &Path) -> Result<LfsStatus> {
        let root = self.repository_root(directory)?;
        let version = self.try_trimmed(&["lfs", "version"], &root);
        let tracked = self.run(&["ls-files", "-z"], &root)?;
        let paths: Vec<&str> = tracked.split('\0').filter(|path| !path.is_empty()).collect();
        let mut files = Vec::new();
        for batch in paths.chunks(ATTRIBUTE_BATCH) {
            let mut arguments = vec!["check-attr", "-z", "filter", "--"];
            arguments.extend_from_slice(batch);
            let output = self.run(&arguments, &root)?;
            for (path, value) in attribute_values(&output) {
                if value == "lfs" {
                    files.push(LfsFile { path: path.to_string(), is_pointer_only: is_lfs_pointer(&root.join(path)) });
                }
            }
        }
        Ok(LfsStatus { version, patterns: lfs_patterns(&root), files })
    }

    /// Whether `path` is stored with Git LFS by the attributes in effect, as `check-attr` reads them.
    pub fn uses_lfs(&self, path: &str, directory: &Path) -> Result<bool> {
        let root = self.repository_root(directory)?;
        let output = self.run(&["check-attr", "-z", "filter", "--", path], &root)?;
        Ok(attribute_values(&output).iter().any(|(_, value)| *value == "lfs"))
    }

    /// The locks the remote's Git LFS server holds for this repository. Ownership comes from the
    /// server when it can verify it, and otherwise from the configured `user.name`. Refused while
    /// Git LFS is not installed or the repository has no remote.
    pub fn list_locks(&self, directory: &Path) -> Result<LfsLocks> {
        let root = self.repository_root(directory)?;
        self.check_lock_server(&root)?;
        let user_name = self.try_trimmed(&["config", "user.name"], &root).unwrap_or_default();
        if let Ok(text) = self.run(&["lfs", "locks", "--verify", "--json"], &root) {
            if let Ok(locks) = parse_verified_locks(&text) {
                return Ok(LfsLocks { locks, verified: true, user_name });
            }
        }
        let text = self.run(&["lfs", "locks", "--json"], &root).map_err(|error| server_error("git lfs locks", error))?;
        let locks = parse_lock_list(&text, &user_name)?;
        Ok(LfsLocks { locks, verified: false, user_name })
    }

    /// Locks `path` on the remote's Git LFS server, so others cannot change it until it is unlocked.
    pub fn lock(&self, path: &str, directory: &Path) -> Result<LfsLock> {
        check_lock_path(path)?;
        let root = self.repository_root(directory)?;
        self.check_lock_server(&root)?;
        let text = self.run_lock_command(&lock_arguments(path), &root, "git lfs lock")?;
        let mut lock = parse_lock(&text).ok_or_else(|| {
            GitError::failed("git lfs lock", "Git LFS locked the file, but its reply could not be read. Refresh to see the lock.")
        })?;
        lock.ours = true;
        Ok(lock)
    }

    /// Releases a lock. Without `force`, Git LFS refuses a lock another person holds.
    pub fn unlock(&self, target: &LockTarget, force: bool, directory: &Path) -> Result<()> {
        target.check()?;
        let root = self.repository_root(directory)?;
        self.check_lock_server(&root)?;
        self.run_lock_command(&unlock_arguments(target, force), &root, "git lfs unlock").map(drop)
    }

    /// Lock commands need Git LFS to be installed and a remote to hold the locks. Each is checked
    /// first so the message says what is missing.
    fn check_lock_server(&self, root: &Path) -> Result<()> {
        if self.run(&["lfs", "version"], root).is_err() {
            return Err(GitError::failed("git lfs", NOT_INSTALLED));
        }
        if self.try_trimmed(&["remote"], root).is_none() {
            return Err(GitError::failed("git lfs", NO_REMOTE));
        }
        Ok(())
    }

    fn run_lock_command(&self, arguments: &[String], root: &Path, command: &str) -> Result<String> {
        let arguments: Vec<&str> = arguments.iter().map(String::as_str).collect();
        runner::run(&arguments, root, &RunOptions { env: LITERAL_PATHSPECS, ..self.options() })
            .map_err(|error| server_error(command, error))
    }

    /// Stores files matching `pattern` with Git LFS from their next commit, as `git lfs track`
    /// does. Refused while Git LFS is not installed, since Git would otherwise commit the full files.
    pub fn track_lfs(&self, pattern: &str, directory: &Path) -> Result<()> {
        let pattern = pattern.trim();
        if pattern.is_empty() || pattern.contains(['\n', '\r']) {
            return Err(GitError::failed("lfs track", "Enter a file pattern such as *.psd."));
        }
        let root = self.repository_root(directory)?;
        if self.run(&["lfs", "version"], &root).is_err() {
            return Err(GitError::failed(
                "lfs track",
                "Git LFS is not installed. Install git-lfs from https://git-lfs.com before tracking files.",
            ));
        }
        if lfs_patterns(&root).iter().any(|existing| existing == pattern) {
            return Ok(());
        }
        // Patterns with spaces, quotes, or a leading `#` are quoted the way .gitattributes expects.
        let written = if pattern.starts_with('#') || pattern.chars().any(|c| c.is_whitespace() || c == '"') {
            quote_attribute_pattern(pattern)
        } else {
            pattern.to_string()
        };
        edit_attributes(&root, |lines| lines.push(format!("{written} filter=lfs diff=lfs merge=lfs -text")))
    }

    /// Stops storing new versions of matching files with Git LFS. Files already committed stay as they are.
    pub fn untrack_lfs(&self, pattern: &str, directory: &Path) -> Result<()> {
        let root = self.repository_root(directory)?;
        edit_attributes(&root, |lines| {
            lines.retain(|line| {
                !(attribute_pattern(line).as_deref() == Some(pattern) && line.split_whitespace().any(|token| token == "filter=lfs"))
            })
        })
    }
}

fn check_lock_path(path: &str) -> Result<()> {
    if path.trim().is_empty() || path.contains(['\n', '\r', '\0']) {
        return Err(GitError::failed("git lfs lock", "Choose a file to lock."));
    }
    Ok(())
}

impl LockTarget {
    fn check(&self) -> Result<()> {
        match self {
            LockTarget::Id(id) if id.is_empty() || id.starts_with('-') || id.contains(char::is_whitespace) => {
                Err(GitError::failed("git lfs unlock", "The lock has no valid ID. Refresh the locks and try again."))
            }
            LockTarget::Id(_) => Ok(()),
            LockTarget::Path(path) => check_lock_path(path),
        }
    }
}

/// The arguments for `git lfs lock`. The path is repository-relative, after `--` so it is never read as an option.
fn lock_arguments(path: &str) -> Vec<String> {
    vec!["lfs".into(), "lock".into(), "--json".into(), "--".into(), path.into()]
}

/// The arguments for `git lfs unlock`, naming the lock by ID or by path.
fn unlock_arguments(target: &LockTarget, force: bool) -> Vec<String> {
    let mut arguments: Vec<String> = vec!["lfs".into(), "unlock".into()];
    if force {
        arguments.push("--force".into());
    }
    match target {
        LockTarget::Id(id) => arguments.extend(["--id".to_string(), id.clone()]),
        LockTarget::Path(path) => arguments.extend(["--".to_string(), path.clone()]),
    }
    arguments.push("--json".into());
    arguments
}

/// The path and value of each record in `check-attr -z` output: a path, the attribute name, and its
/// value, each NUL-terminated.
fn attribute_values(output: &str) -> Vec<(&str, &str)> {
    let fields: Vec<&str> = output.split('\0').collect();
    fields.as_chunks::<3>().0.iter().map(|[path, _attribute, value]| (*path, *value)).collect()
}

/// A failure from the lock server, with the command that failed. Git's message is kept, except
/// where it says the server cannot lock files at all.
fn server_error(command: &str, error: GitError) -> GitError {
    match error {
        GitError::CommandFailed { message, .. } => GitError::failed(command, server_message(&message)),
        other => other,
    }
}

fn server_message(message: &str) -> String {
    let lower = message.to_lowercase();
    if lower.contains("not supported") || lower.contains("does not support") || lower.contains("unsupported") {
        "The remote's Git LFS server does not support file locking.".to_string()
    } else {
        message.to_string()
    }
}

fn unexpected_locks() -> GitError {
    GitError::failed("git lfs locks", "Git LFS returned a lock list that could not be read.")
}

/// Reads `git lfs locks --json`, a list of locks. Ownership compares each owner with `user_name`.
fn parse_lock_list(text: &str, user_name: &str) -> Result<Vec<LfsLock>> {
    let Json::Array(entries) = parse_json(text).map_err(|_| unexpected_locks())? else { return Err(unexpected_locks()) };
    entries
        .iter()
        .map(|entry| {
            let mut lock = decode_lock(entry).ok_or_else(unexpected_locks)?;
            lock.ours = !user_name.is_empty() && lock.owner == user_name;
            Ok(lock)
        })
        .collect()
}

/// Reads `git lfs locks --verify --json`, which splits the locks into `ours` and `theirs`.
fn parse_verified_locks(text: &str) -> Result<Vec<LfsLock>> {
    let value = parse_json(text).map_err(|_| unexpected_locks())?;
    let mut locks = Vec::new();
    for (key, ours) in [("ours", true), ("theirs", false)] {
        let Some(Json::Array(entries)) = value.get(key) else { return Err(unexpected_locks()) };
        for entry in entries {
            let mut lock = decode_lock(entry).ok_or_else(unexpected_locks)?;
            lock.ours = ours;
            locks.push(lock);
        }
    }
    Ok(locks)
}

/// Reads the lock that `git lfs lock --json` prints. Ownership is set by the caller.
fn parse_lock(text: &str) -> Option<LfsLock> {
    parse_json(text).ok().and_then(|value| decode_lock(&value))
}

/// One lock object: its `id`, `path`, `owner.name`, and `locked_at` time. Ownership is not read here.
fn decode_lock(entry: &Json) -> Option<LfsLock> {
    Some(LfsLock {
        id: entry.get("id")?.as_str()?.to_string(),
        path: entry.get("path")?.as_str()?.to_string(),
        owner: entry.get("owner").and_then(|owner| owner.get("name")).and_then(Json::as_str).unwrap_or_default().to_string(),
        locked_at: entry.get("locked_at").and_then(Json::as_str).and_then(rfc3339_seconds),
        ours: false,
    })
}

/// The Unix time of an RFC 3339 timestamp such as `2016-05-17T15:49:06-07:00`, with an optional
/// fraction and a `Z` or numeric offset. Anything else is None.
fn rfc3339_seconds(text: &str) -> Option<i64> {
    let bytes = text.as_bytes();
    if bytes.len() < 20 || bytes[4] != b'-' || bytes[7] != b'-' || !matches!(bytes[10], b'T' | b't' | b' ') {
        return None;
    }
    if bytes[13] != b':' || bytes[16] != b':' {
        return None;
    }
    let number = |range: std::ops::Range<usize>| text.get(range)?.parse::<i64>().ok();
    let (year, month, day) = (number(0..4)?, number(5..7)?, number(8..10)?);
    let (hour, minute, second) = (number(11..13)?, number(14..16)?, number(17..19)?);
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) || hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    let mut rest = &text[19..];
    if let Some(fraction) = rest.strip_prefix('.') {
        let digits = fraction.bytes().take_while(u8::is_ascii_digit).count();
        if digits == 0 {
            return None;
        }
        rest = &fraction[digits..];
    }
    let offset = match rest {
        "Z" | "z" => 0,
        _ => {
            let sign = match rest.as_bytes().first()? {
                b'+' => 1,
                b'-' => -1,
                _ => return None,
            };
            if rest.len() != 6 || rest.as_bytes()[3] != b':' {
                return None;
            }
            let (hours, minutes) = (rest.get(1..3)?.parse::<i64>().ok()?, rest.get(4..6)?.parse::<i64>().ok()?);
            sign * (hours * 3600 + minutes * 60)
        }
    };
    Some(days_from_civil(year, month, day) * 86_400 + hour * 3600 + minute * 60 + second - offset)
}

/// Days since 1970-01-01 for a proleptic Gregorian date (Howard Hinnant's algorithm).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let month_from_march = (month + 9) % 12;
    let day_of_year = (153 * month_from_march + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// The patterns in the root `.gitattributes` that use `filter=lfs`. A missing file has none.
fn lfs_patterns(root: &Path) -> Vec<String> {
    let text = std::fs::read_to_string(root.join(ATTRIBUTES_FILE)).unwrap_or_default();
    text.lines().filter(|line| line.split_whitespace().any(|token| token == "filter=lfs")).filter_map(attribute_pattern).collect()
}

/// Applies `change` to the attribute file's lines and writes it back only if something changed.
/// A file that exists but cannot be read is an error, never overwritten with partial content.
fn edit_attributes(root: &Path, change: impl FnOnce(&mut Vec<String>)) -> Result<()> {
    let file = root.join(ATTRIBUTES_FILE);
    let text = match std::fs::read_to_string(&file) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(GitError::failed("lfs", format!("Could not read {ATTRIBUTES_FILE}: {error}"))),
    };
    let mut lines: Vec<String> = text.split('\n').map(str::to_string).collect();
    if lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    let before = lines.clone();
    change(&mut lines);
    if lines == before {
        return Ok(());
    }
    let mut updated = lines.join("\n");
    if !lines.is_empty() {
        updated.push('\n');
    }
    std::fs::write(&file, updated).map_err(|error| GitError::failed("lfs", format!("Could not write {ATTRIBUTES_FILE}: {error}")))
}

/// The pattern at the start of a `.gitattributes` line, unquoting a C-style quoted pattern.
/// Comments and blank lines have none.
fn attribute_pattern(line: &str) -> Option<String> {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return None;
    }
    if let Some(rest) = trimmed.strip_prefix('"') {
        // The pattern ends at the first closing quote that is not escaped.
        let mut escaped = false;
        for (index, character) in rest.char_indices() {
            if character == '"' && !escaped {
                return Some(unquote(&rest[..index]));
            }
            escaped = character == '\\' && !escaped;
        }
        return None;
    }
    trimmed.split_whitespace().next().map(str::to_string)
}

fn quote_attribute_pattern(pattern: &str) -> String {
    format!("\"{}\"", pattern.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Decodes the escapes inside a C-style quoted pattern: `\\`, `\"`, the usual control escapes,
/// and three-digit octal bytes. Unknown escapes keep the escaped character.
fn unquote(raw: &str) -> String {
    let mut bytes = Vec::with_capacity(raw.len());
    let mut chars = raw.bytes().peekable();
    while let Some(byte) = chars.next() {
        if byte != b'\\' {
            bytes.push(byte);
            continue;
        }
        let Some(escape) = chars.next() else { break };
        match escape {
            b'n' => bytes.push(b'\n'),
            b't' => bytes.push(b'\t'),
            b'r' => bytes.push(b'\r'),
            b'a' => bytes.push(7),
            b'b' => bytes.push(8),
            b'f' => bytes.push(12),
            b'v' => bytes.push(11),
            b'0'..=b'7' => {
                let mut value = u32::from(escape - b'0');
                for _ in 0..2 {
                    match chars.peek() {
                        Some(digit @ b'0'..=b'7') => {
                            value = value * 8 + u32::from(digit - b'0');
                            chars.next();
                        }
                        _ => break,
                    }
                }
                bytes.push(value as u8);
            }
            other => bytes.push(other),
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Whether a file is an LFS pointer rather than its content. Only the first bytes are read.
fn is_lfs_pointer(file: &Path) -> bool {
    let Ok(handle) = std::fs::File::open(file) else { return false };
    let mut prefix = Vec::with_capacity(POINTER_PREFIX.len());
    if handle.take(POINTER_PREFIX.len() as u64).read_to_end(&mut prefix).is_err() {
        return false;
    }
    prefix.starts_with(POINTER_PREFIX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attribute_patterns_are_read_plain_and_quoted() {
        assert_eq!(attribute_pattern("*.psd filter=lfs diff=lfs"), Some("*.psd".to_string()));
        assert_eq!(attribute_pattern("\"my \\\"big\\\" file.bin\" filter=lfs"), Some("my \"big\" file.bin".to_string()));
        assert_eq!(attribute_pattern("\"a\\tb\" filter=lfs"), Some("a\tb".to_string()));
        assert_eq!(attribute_pattern("\"\\303\\251.bin\" filter=lfs"), Some("é.bin".to_string()));
        assert_eq!(attribute_pattern("# comment filter=lfs"), None);
        assert_eq!(attribute_pattern("   "), None);
        assert_eq!(attribute_pattern("\"unterminated filter=lfs"), None);
    }

    #[test]
    fn quoting_round_trips() {
        let pattern = "dir with \"quotes\"\\*.bin";
        assert_eq!(attribute_pattern(&format!("{} filter=lfs", quote_attribute_pattern(pattern))), Some(pattern.to_string()));
    }

    #[test]
    fn lock_list_names_each_owner_and_marks_this_users_locks() {
        let text = r#"[{"id":"1234","path":"docs/big.psd","locked_at":"2016-05-17T15:49:06-07:00","owner":{"name":"Alice"}},
            {"id":"1235","path":"art/cover.png","locked_at":"2016-05-17T22:49:06Z","owner":{"name":"Bob"}}]"#;
        let locks = parse_lock_list(text, "Alice").unwrap();
        assert_eq!(locks.len(), 2);
        assert_eq!(locks[0].id, "1234");
        assert_eq!(locks[0].path, "docs/big.psd");
        assert_eq!(locks[0].owner, "Alice");
        assert_eq!(locks[0].locked_at, Some(1_463_525_346));
        assert!(locks[0].ours);
        assert!(!locks[1].ours);
        // Without a user name, no lock is shown as this user's.
        assert!(parse_lock_list(text, "").unwrap().iter().all(|lock| !lock.ours));
        assert_eq!(parse_lock_list("[]", "Alice").unwrap(), Vec::new());
    }

    #[test]
    fn verified_locks_split_into_ours_and_theirs() {
        let text = r#"{"ours":[{"id":"7","path":"a.bin","locked_at":"2024-01-02T03:04:05+05:30","owner":{"name":"me"}}],
            "theirs":[{"id":"8","path":"b.bin","owner":{"name":"you"}}]}"#;
        let locks = parse_verified_locks(text).unwrap();
        assert_eq!(locks.len(), 2);
        assert_eq!((locks[0].id.as_str(), locks[0].ours), ("7", true));
        assert_eq!(locks[0].locked_at, Some(1_704_144_845));
        assert_eq!((locks[1].id.as_str(), locks[1].ours), ("8", false));
        assert_eq!(locks[1].locked_at, None, "a missing time is not an error");
        // A reply without both lists is not a verification, so the caller falls back to the plain list.
        assert!(parse_verified_locks(r#"{"ours":[]}"#).is_err());
        assert!(parse_verified_locks("[]").is_err());
    }

    #[test]
    fn lock_reply_is_one_lock_object() {
        let lock = parse_lock(r#"{"id":"7","path":"c.psd","locked_at":"2024-01-02T03:04:05+05:30","owner":{"name":"me"}}"#).unwrap();
        assert_eq!((lock.id.as_str(), lock.path.as_str(), lock.owner.as_str()), ("7", "c.psd", "me"));
        assert!(parse_lock("Locked c.psd").is_none());
        assert!(parse_lock("{}").is_none());
    }

    #[test]
    fn malformed_lock_output_is_refused() {
        assert!(parse_lock_list("{}", "").is_err());
        assert!(parse_lock_list(r#"[{"id":1,"path":"a","owner":{"name":"x"}}]"#, "").is_err(), "numeric IDs are not lock IDs");
        assert!(parse_lock_list(r#"[{"id":"1","owner":{"name":"x"}}]"#, "").is_err(), "a lock needs its path");
        assert!(parse_lock_list("[", "").is_err());
    }

    #[test]
    fn rfc3339_times_read_offsets_and_fractions() {
        assert_eq!(rfc3339_seconds("2016-05-17T22:49:06Z"), Some(1_463_525_346));
        assert_eq!(rfc3339_seconds("2016-05-17T15:49:06.250-07:00"), Some(1_463_525_346));
        assert_eq!(rfc3339_seconds("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(rfc3339_seconds("2024-01-01T00:00:00+00:00"), Some(1_704_067_200));
        assert_eq!(rfc3339_seconds("yesterday"), None);
        assert_eq!(rfc3339_seconds("2016-13-17T22:49:06Z"), None);
        assert_eq!(rfc3339_seconds("2016-05-17T22:49:06"), None, "a time without an offset is not guessed");
    }

    #[test]
    fn lock_commands_are_built_with_ids_and_separators() {
        assert_eq!(lock_arguments("docs/a b.psd"), ["lfs", "lock", "--json", "--", "docs/a b.psd"]);
        assert_eq!(unlock_arguments(&LockTarget::Id("42".into()), true), ["lfs", "unlock", "--force", "--id", "42", "--json"]);
        assert_eq!(unlock_arguments(&LockTarget::Id("42".into()), false), ["lfs", "unlock", "--id", "42", "--json"]);
        assert_eq!(unlock_arguments(&LockTarget::Path("-odd.bin".into()), false), ["lfs", "unlock", "--", "-odd.bin", "--json"]);
        assert!(LockTarget::Id("--force".into()).check().is_err());
        assert!(LockTarget::Path("a\nb".into()).check().is_err());
        assert!(check_lock_path("  ").is_err());
    }

    #[test]
    fn server_refusals_are_named_and_other_messages_pass_through() {
        assert_eq!(server_message("Locking is not supported by this server"), "The remote's Git LFS server does not support file locking.");
        assert_eq!(server_message("permission denied"), "permission denied");
        let failure = GitError::failed("git lfs lock", "Locking is unsupported");
        assert_eq!(
            server_error("git lfs lock", failure),
            GitError::failed("git lfs lock", "The remote's Git LFS server does not support file locking.")
        );
    }

    #[test]
    fn check_attr_records_are_read_in_threes() {
        let output = "a.psd\0filter\0lfs\0notes.txt\0filter\0unspecified\0";
        assert_eq!(attribute_values(output), [("a.psd", "lfs"), ("notes.txt", "unspecified")]);
    }
}
