//! The command log: every Git command NiceGit has run, with its arguments, exit status, timing,
//! and the start of its error output. It is kept in memory, newest last, for the Command Log
//! window. Secrets are removed before an entry is stored, so the window can show it safely.

use std::borrow::Cow;
use std::collections::VecDeque;
use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitStatus;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime};

/// How many commands are kept. The oldest is dropped first.
pub const CAPACITY: usize = 1000;

/// How many bytes of a command's error output are kept.
pub const STDERR_LIMIT: usize = 4096;

/// What replaces the value of a configuration pair whose key is secret.
const HIDDEN: &str = "<hidden>";

/// Words in a configuration key that mark its value as secret.
const SECRET_KEY_WORDS: &[&str] = &["token", "password", "secret", "extraheader"];

/// One Git command NiceGit ran.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// Increases with each entry recorded, so a reader can ask for the ones it has not seen.
    pub id: u64,
    /// When the command started.
    pub started: SystemTime,
    /// How long the command ran.
    pub duration: Duration,
    /// The folder the command ran in.
    pub directory: PathBuf,
    /// The arguments after `git`, with secrets removed.
    pub arguments: Vec<String>,
    /// The configuration the command was given through `GIT_CONFIG_COUNT`, with secrets removed.
    pub config: Vec<(String, String)>,
    /// The exit status. `None` when the command was killed, or did not start.
    pub exit: Option<i32>,
    /// How many bytes of standard output the command wrote. Zero for output that is streamed
    /// and not counted, such as a log being read as it arrives.
    pub stdout_bytes: usize,
    /// The first `STDERR_LIMIT` bytes of the error output, decoded leniently, with secrets removed.
    pub stderr: String,
    /// Whether the command only reads the repository. Set from the arguments, see `is_read_only`.
    pub background: bool,
}

/// The entries and the id of the newest one. Kept apart from the global so its rules can be tested.
pub(crate) struct Log {
    entries: VecDeque<Entry>,
    last_id: u64,
}

impl Log {
    pub(crate) const fn new() -> Self {
        Log { entries: VecDeque::new(), last_id: 0 }
    }

    /// Stores `entry` under the next id, dropping the oldest entry when the log is full.
    fn push(&mut self, mut entry: Entry) -> u64 {
        self.last_id += 1;
        entry.id = self.last_id;
        if self.entries.len() == CAPACITY {
            self.entries.pop_front();
        }
        self.entries.push_back(entry);
        self.last_id
    }

    /// The entries with an id greater than `id`, oldest first.
    fn since(&self, id: u64) -> Vec<Entry> {
        let start = self.entries.partition_point(|entry| entry.id <= id);
        self.entries.range(start..).cloned().collect()
    }
}

static LOG: Mutex<Log> = Mutex::new(Log::new());

fn log() -> MutexGuard<'static, Log> {
    LOG.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Stores a command after removing its secrets. Returns the id it was given.
pub fn record(mut entry: Entry) -> u64 {
    redact(&mut entry);
    log().push(entry)
}

/// The entries recorded after `id`, oldest first. Pass 0 for all of them.
pub fn entries_since(id: u64) -> Vec<Entry> {
    log().since(id)
}

/// Forgets every entry. Ids keep increasing, so a reader's `entries_since` still works.
pub fn clear() {
    log().entries.clear();
}

/// A command that has started and is not yet recorded. It is recorded when `finish` is called,
/// or, if it is dropped first, as a command that did not finish.
pub struct Run {
    pending: Option<Pending>,
}

struct Pending {
    started: SystemTime,
    timer: Instant,
    directory: PathBuf,
    arguments: Vec<String>,
    config: Vec<(String, String)>,
}

impl Run {
    /// Starts timing a command with `arguments` in `directory`. `env` is the environment it is
    /// given, whose configuration pairs are kept.
    pub fn start(arguments: &[&str], directory: &Path, env: &[(&str, &str)]) -> Run {
        let pending = Pending {
            started: SystemTime::now(),
            timer: Instant::now(),
            directory: directory.to_path_buf(),
            arguments: arguments.iter().map(|argument| argument.to_string()).collect(),
            config: config_pairs(env),
        };
        Run { pending: Some(pending) }
    }

    /// Records the command's result. `stderr` is cut to `STDERR_LIMIT` bytes.
    pub fn finish(mut self, exit: Option<i32>, stdout_bytes: usize, stderr: &[u8]) {
        if let Some(pending) = self.pending.take() {
            pending.record(exit, stdout_bytes, stderr);
        }
    }

    /// Records the result of a child that has been waited for.
    pub fn finish_status(self, status: &io::Result<ExitStatus>) {
        self.finish(status.as_ref().ok().and_then(ExitStatus::code), 0, &[]);
    }
}

impl Drop for Run {
    fn drop(&mut self) {
        if let Some(pending) = self.pending.take() {
            pending.record(None, 0, &[]);
        }
    }
}

impl Pending {
    fn record(self, exit: Option<i32>, stdout_bytes: usize, stderr: &[u8]) {
        let end = stderr.len().min(STDERR_LIMIT);
        let background = is_read_only(&self.arguments);
        record(Entry {
            id: 0,
            started: self.started,
            duration: self.timer.elapsed(),
            directory: self.directory,
            arguments: self.arguments,
            config: self.config,
            exit,
            stdout_bytes,
            stderr: String::from_utf8_lossy(&stderr[..end]).into_owned(),
            background,
        });
    }
}

/// The configuration pairs in an environment: `GIT_CONFIG_KEY_n` and `GIT_CONFIG_VALUE_n` for each
/// of the `GIT_CONFIG_COUNT` pairs. The last value of a variable wins, as it does in the process.
pub fn config_pairs(env: &[(&str, &str)]) -> Vec<(String, String)> {
    let count = lookup(env, "GIT_CONFIG_COUNT").and_then(|count| count.parse::<usize>().ok()).unwrap_or(0);
    (0..count)
        .filter_map(|index| {
            let key = lookup(env, &format!("GIT_CONFIG_KEY_{index}"))?;
            let value = lookup(env, &format!("GIT_CONFIG_VALUE_{index}")).unwrap_or_default();
            Some((key.to_string(), value.to_string()))
        })
        .collect()
}

fn lookup<'a>(env: &[(&str, &'a str)], name: &str) -> Option<&'a str> {
    env.iter().rev().find(|(key, _)| *key == name).map(|(_, value)| *value)
}

/// Whether a Git command only reads the repository, so it changes nothing. Global options such as
/// `-c` are skipped to find the command. A command that is not recognised counts as a change.
pub fn is_read_only(arguments: &[String]) -> bool {
    const READS: &[&str] = &[
        "status",
        "log",
        "diff",
        "show",
        "rev-parse",
        "for-each-ref",
        "ls-files",
        "cat-file",
        "rev-list",
        "merge-base",
        "blame",
        "grep",
        "reflog",
        "show-ref",
        "symbolic-ref",
        "check-ignore",
        "ls-tree",
        "describe",
        "merge-tree",
        "ls-remote",
        "version",
        "--version",
    ];
    let Some(index) = command_index(arguments) else { return false };
    let rest = &arguments[index + 1..];
    match arguments[index].as_str() {
        "config" => {
            let reads = rest.iter().any(|argument| argument.starts_with("--get") || argument == "--list" || argument == "-l");
            let writes = rest.iter().any(|argument| {
                matches!(argument.as_str(), "--add" | "--unset" | "--unset-all" | "--replace-all" | "--rename-section" | "--remove-section")
            });
            reads && !writes
        }
        "branch" => {
            const LISTING_WITH_NAME: &[&str] = &["--contains", "--merged", "--no-merged", "--points-at"];
            rest.is_empty()
                || LISTING_WITH_NAME.contains(&rest[0].as_str())
                || rest.iter().all(|argument| {
                    argument.starts_with('-')
                        && !matches!(
                            argument.as_str(),
                            "-d" | "-D"
                                | "-m"
                                | "-M"
                                | "-c"
                                | "-C"
                                | "-f"
                                | "-u"
                                | "-t"
                                | "--delete"
                                | "--move"
                                | "--copy"
                                | "--force"
                                | "--set-upstream-to"
                                | "--unset-upstream"
                                | "--edit-description"
                                | "--track"
                        )
                })
        }
        "remote" => rest.is_empty() || matches!(rest[0].as_str(), "-v" | "--verbose" | "show" | "get-url"),
        command => READS.contains(&command),
    }
}

/// The position of the Git command name in `arguments`, after any global options.
fn command_index(arguments: &[String]) -> Option<usize> {
    let mut index = 0;
    while index < arguments.len() {
        match arguments[index].as_str() {
            "-c" | "-C" | "--git-dir" | "--work-tree" => index += 2,
            flag if flag.starts_with('-') => index += 1,
            _ => return Some(index),
        }
    }
    None
}

/// The command as a shell would type it, for copying: `git` and its arguments, each quoted where
/// the shell would otherwise split or change it.
pub fn command_line(entry: &Entry) -> String {
    let mut line = String::from("git");
    for argument in &entry.arguments {
        line.push(' ');
        line.push_str(&shell_quote(argument));
    }
    line
}

/// One argument quoted for a POSIX shell. Plain words are left as they are.
pub fn shell_quote(argument: &str) -> String {
    let plain = !argument.is_empty() && argument.bytes().all(|byte| byte.is_ascii_alphanumeric() || b"-_./=:,@%+".contains(&byte));
    if plain {
        argument.to_string()
    } else {
        format!("'{}'", argument.replace('\'', "'\\''"))
    }
}

/// Removes the user name and password from every web address in `text`, so
/// `https://user:token@host/repo` becomes `https://host/repo`. Text without one is borrowed.
pub fn strip_userinfo(text: &str) -> Cow<'_, str> {
    let mut stripped: Option<String> = None;
    let mut copied = 0;
    let mut search = 0;
    while let Some(found) = text[search..].find("://") {
        let authority_start = search + found + 3;
        let rest = &text[authority_start..];
        let end = rest.find(|c: char| matches!(c, '/' | '?' | '#' | '\'' | '"' | '<' | '>') || c.is_whitespace()).unwrap_or(rest.len());
        if let Some(at) = rest[..end].rfind('@') {
            let output = stripped.get_or_insert_with(String::new);
            output.push_str(&text[copied..authority_start]);
            copied = authority_start + at + 1;
        }
        search = authority_start + end;
    }
    match stripped {
        Some(mut output) => {
            output.push_str(&text[copied..]);
            Cow::Owned(output)
        }
        None => Cow::Borrowed(text),
    }
}

fn strip_in_place(text: &mut String) {
    let stripped = match strip_userinfo(text) {
        Cow::Owned(output) => Some(output),
        Cow::Borrowed(_) => None,
    };
    if let Some(output) = stripped {
        *text = output;
    }
}

fn is_secret_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    SECRET_KEY_WORDS.iter().any(|word| key.contains(word))
}

/// Hides the value of a secret key, and removes user names and passwords from either part.
fn redact_pair(key: &mut String, value: &mut String) {
    if is_secret_key(key) {
        *value = HIDDEN.to_string();
    } else {
        strip_in_place(value);
    }
    strip_in_place(key);
}

/// A `key=value` argument given after `-c`.
fn redact_config_argument(argument: &mut String) {
    match argument.split_once('=') {
        Some((key, value)) => {
            let (mut key, mut value) = (key.to_string(), value.to_string());
            redact_pair(&mut key, &mut value);
            *argument = format!("{key}={value}");
        }
        None => strip_in_place(argument),
    }
}

fn redact_arguments(arguments: &mut [String]) {
    let mut config_next = false;
    for argument in arguments.iter_mut() {
        if std::mem::take(&mut config_next) {
            redact_config_argument(argument);
        } else {
            config_next = argument.as_str() == "-c";
            strip_in_place(argument);
        }
    }
}

fn redact(entry: &mut Entry) {
    redact_arguments(&mut entry.arguments);
    for (key, value) in &mut entry.config {
        redact_pair(key, value);
    }
    strip_in_place(&mut entry.stderr);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(directory: &str, arguments: &[&str]) -> Entry {
        Entry {
            id: 0,
            started: SystemTime::UNIX_EPOCH,
            duration: Duration::from_millis(3),
            directory: PathBuf::from(directory),
            arguments: arguments.iter().map(|argument| argument.to_string()).collect(),
            config: Vec::new(),
            exit: Some(0),
            stdout_bytes: 0,
            stderr: String::new(),
            background: false,
        }
    }

    fn args(arguments: &[&str]) -> Vec<String> {
        arguments.iter().map(|argument| argument.to_string()).collect()
    }

    #[test]
    fn the_log_keeps_the_newest_capacity_entries_in_order() {
        let mut log = Log::new();
        for index in 0..CAPACITY + 25 {
            log.push(entry(&format!("/repo/{index}"), &["status"]));
        }
        assert_eq!(log.entries.len(), CAPACITY);
        let all = log.since(0);
        assert_eq!(all.len(), CAPACITY);
        assert_eq!(all.first().map(|entry| entry.id), Some(26), "the 25 oldest are gone");
        assert_eq!(all.last().map(|entry| entry.id), Some((CAPACITY + 25) as u64));
        assert!(all.windows(2).all(|pair| pair[0].id + 1 == pair[1].id), "ids are consecutive and ascending");
    }

    #[test]
    fn since_returns_only_entries_after_the_given_id() {
        let mut log = Log::new();
        for _ in 0..5 {
            log.push(entry("/repo", &["log"]));
        }
        let ids: Vec<u64> = log.since(3).iter().map(|entry| entry.id).collect();
        assert_eq!(ids, vec![4, 5]);
        assert!(log.since(5).is_empty());
        assert_eq!(log.since(0).len(), 5);
    }

    #[test]
    fn ids_keep_increasing_after_a_clear() {
        let mut log = Log::new();
        log.push(entry("/repo", &["log"]));
        log.entries.clear();
        let id = log.push(entry("/repo", &["log"]));
        assert_eq!(id, 2);
        assert_eq!(log.since(1).len(), 1);
    }

    #[test]
    fn userinfo_is_removed_from_web_addresses_only() {
        assert_eq!(strip_userinfo("https://user:secret@example.com/repo.git"), "https://example.com/repo.git");
        assert_eq!(strip_userinfo("https://token@host.example/x"), "https://host.example/x");
        // A password may contain an @, so the last one before the path separates the host.
        assert_eq!(strip_userinfo("https://me:p@ss@host/x"), "https://host/x");
        assert_eq!(strip_userinfo("url.https://tok@github.com/.insteadOf=gh:"), "url.https://github.com/.insteadOf=gh:");
        assert_eq!(strip_userinfo("fatal: https://a:b@h/x' failed and https://c@d/e"), "fatal: https://h/x' failed and https://d/e");
        assert_eq!(strip_userinfo("git@github.com:owner/repo.git"), "git@github.com:owner/repo.git");
        assert_eq!(strip_userinfo("https://example.com/a@b"), "https://example.com/a@b");
        assert!(matches!(strip_userinfo("plain text"), Cow::Borrowed(_)));
    }

    #[test]
    fn secret_configuration_values_are_hidden_in_arguments() {
        let mut arguments = args(&[
            "-c",
            "http.extraheader=AUTHORIZATION: bearer abc",
            "-c",
            "credential.token=xyz",
            "-c",
            "core.editor=vim",
            "-c",
            "url.https://tok@github.com/.insteadOf=gh:",
            "clone",
            "https://user:pw@example.com/repo",
        ]);
        redact_arguments(&mut arguments);
        assert_eq!(arguments[1], "http.extraheader=<hidden>");
        assert_eq!(arguments[3], "credential.token=<hidden>");
        assert_eq!(arguments[5], "core.editor=vim");
        assert_eq!(arguments[7], "url.https://github.com/.insteadOf=gh:");
        assert_eq!(arguments[9], "https://example.com/repo");
    }

    #[test]
    fn configuration_pairs_from_the_environment_are_redacted_when_recorded() {
        let env = [
            ("GIT_CONFIG_COUNT", "3"),
            ("GIT_CONFIG_KEY_0", "maintenance.auto"),
            ("GIT_CONFIG_VALUE_0", "false"),
            ("GIT_CONFIG_KEY_1", "x.password"),
            ("GIT_CONFIG_VALUE_1", "hunter2"),
            ("GIT_CONFIG_KEY_2", "remote.x.url"),
            ("GIT_CONFIG_VALUE_2", "https://me:pw@host/r"),
        ];
        let pairs = config_pairs(&env);
        assert_eq!(pairs.len(), 3);
        let mut recorded = Entry { config: pairs, ..entry("/repo", &["status"]) };
        redact(&mut recorded);
        assert_eq!(recorded.config[0], ("maintenance.auto".to_string(), "false".to_string()));
        assert_eq!(recorded.config[1], ("x.password".to_string(), HIDDEN.to_string()));
        assert_eq!(recorded.config[2], ("remote.x.url".to_string(), "https://host/r".to_string()));
    }

    #[test]
    fn stderr_is_cleaned_of_userinfo_when_recorded() {
        let mut recorded = Entry { stderr: "fatal: unable to access 'https://me:pw@host/r/'".into(), ..entry("/repo", &["fetch"]) };
        redact(&mut recorded);
        assert_eq!(recorded.stderr, "fatal: unable to access 'https://host/r/'");
    }

    #[test]
    fn read_only_commands_are_recognised_past_global_options() {
        assert!(is_read_only(&args(&["status", "--porcelain=v2"])));
        assert!(is_read_only(&args(&["-c", "core.quotepath=off", "-C", "/x", "log", "-1"])));
        assert!(is_read_only(&args(&["rev-parse", "HEAD"])));
        assert!(is_read_only(&args(&["config", "--get", "user.name"])));
        assert!(is_read_only(&args(&["config", "--local", "--list"])));
        assert!(is_read_only(&args(&["branch", "--list"])));
        assert!(is_read_only(&args(&["branch", "-a", "-v"])));
        assert!(is_read_only(&args(&["branch", "--contains", "abc"])));
        assert!(is_read_only(&args(&["remote", "-v"])));
        assert!(is_read_only(&args(&["remote", "get-url", "origin"])));
        assert!(!is_read_only(&args(&["remote", "add", "x", "url"])));
        assert!(!is_read_only(&args(&["config", "--get", "a", "--unset"])));
        assert!(!is_read_only(&args(&["branch", "feature"])));
        assert!(!is_read_only(&args(&["branch", "-D", "feature"])));
        assert!(!is_read_only(&args(&["add", "--all"])));
        assert!(!is_read_only(&args(&["commit", "-m", "x"])));
        assert!(!is_read_only(&args(&["-c", "x=y"])), "no command at all is not a read");
    }

    #[test]
    fn command_lines_quote_only_what_the_shell_needs() {
        let plain = entry("/repo", &["log", "--format=%H", "-n", "5"]);
        assert_eq!(command_line(&plain), "git log --format=%H -n 5");
        let spaced = entry("/repo", &["commit", "-m", "Fix the 'build'", "", "a b$c"]);
        assert_eq!(command_line(&spaced), "git commit -m 'Fix the '\\''build'\\''' '' 'a b$c'");
    }

    #[test]
    fn the_run_timer_measures_the_command() {
        let directory = Path::new("/nonexistent/command-log-timing-test");
        let before = entries_since(0).last().map_or(0, |entry| entry.id);
        let run = Run::start(&["status", "--porcelain"], directory, &[]);
        std::thread::sleep(Duration::from_millis(5));
        run.finish(Some(3), 12, b"fatal: something\n");
        let recorded = entries_since(before).into_iter().find(|entry| entry.directory == directory).expect("the run is recorded");
        assert!(recorded.duration >= Duration::from_millis(5));
        assert_eq!(recorded.exit, Some(3));
        assert_eq!(recorded.stdout_bytes, 12);
        assert_eq!(recorded.stderr, "fatal: something\n");
        assert!(recorded.background, "status only reads");
    }

    #[test]
    fn a_run_dropped_without_a_result_records_no_exit_status() {
        let directory = Path::new("/nonexistent/command-log-dropped-test");
        let before = entries_since(0).last().map_or(0, |entry| entry.id);
        drop(Run::start(&["commit", "-m", "x"], directory, &[]));
        let recorded = entries_since(before).into_iter().find(|entry| entry.directory == directory).expect("the run is recorded");
        assert_eq!(recorded.exit, None);
        assert!(!recorded.background);
    }

    #[test]
    fn stderr_is_cut_to_its_limit() {
        let directory = Path::new("/nonexistent/command-log-stderr-test");
        let before = entries_since(0).last().map_or(0, |entry| entry.id);
        Run::start(&["fetch"], directory, &[]).finish(Some(1), 0, &vec![b'x'; STDERR_LIMIT * 2]);
        let recorded = entries_since(before).into_iter().find(|entry| entry.directory == directory).expect("the run is recorded");
        assert_eq!(recorded.stderr.len(), STDERR_LIMIT);
    }

    #[test]
    fn ten_thousand_records_are_cheap() {
        // A private log, so other tests recording at the same time cannot be measured or evicted.
        let mut log = Log::new();
        let template = entry("/nonexistent/command-log-speed-test", &["log", "--format=%H", "-n", "100", "--", "src/main.rs"]);
        let start = Instant::now();
        for _ in 0..10_000 {
            let mut recorded = template.clone();
            redact(&mut recorded);
            log.push(recorded);
        }
        let elapsed = start.elapsed();
        assert_eq!(log.since(0).len(), CAPACITY);
        // A few milliseconds in a release build; debug builds allow more time for the same work.
        let limit = if cfg!(debug_assertions) { Duration::from_millis(250) } else { Duration::from_millis(20) };
        assert!(elapsed < limit, "10,000 records took {elapsed:?}");
    }
}
