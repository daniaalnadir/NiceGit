use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

use crate::models::{GitError, Result};

/// Commands that receive user-selected paths. Literal pathspecs stop a filename containing
/// glob characters from selecting other files. `stash` is excluded because it runs Git
/// internally with its own pathspecs, and `check-ignore` rejects literal pathspec magic.
const LITERAL_PATHSPEC_COMMANDS: &[&str] =
    &["add", "clean", "diff", "show", "diff-tree", "restore", "rm", "checkout", "ls-files", "ls-tree", "log", "status"];

/// Inherited variables that would redirect Git to another repository or override options
/// NiceGit passes explicitly. `GIT_DIFF_OPTS` overrides requested diff context.
const REMOVED_VARIABLES: &[&str] = &[
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_CONFIG",
    "GIT_CONFIG_PARAMETERS",
    "GIT_CONFIG_COUNT",
    "GIT_OBJECT_DIRECTORY",
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_IMPLICIT_WORK_TREE",
    "GIT_GRAFT_FILE",
    "GIT_INDEX_FILE",
    "GIT_NO_REPLACE_OBJECTS",
    "GIT_REPLACE_REF_BASE",
    "GIT_PREFIX",
    "GIT_SHALLOW_FILE",
    "GIT_COMMON_DIR",
    "GIT_DIFF_OPTS",
];

pub fn is_removed_variable(key: &str) -> bool {
    REMOVED_VARIABLES.contains(&key) || key.starts_with("GIT_CONFIG_KEY_") || key.starts_with("GIT_CONFIG_VALUE_")
}

/// Options for a single Git invocation.
#[derive(Clone, Debug, Default)]
pub struct RunOptions<'a> {
    /// Exit statuses treated as success; empty means only 0.
    pub accepted: &'a [i32],
    /// Let `git status` refresh the index, as it normally would. Only deliberate loads use
    /// this: an index rewritten on every automatic refresh contends with the user's commands.
    pub status_updates_index: bool,
    pub env: &'a [(&'a str, &'a str)],
}

/// The search path used for Git and the programs it starts (credential helpers, hooks). Apps
/// launched from a desktop often inherit a minimal PATH, so common install locations are added.
pub fn search_path() -> &'static OsString {
    static PATH: OsLockCell = OsLockCell::new();
    PATH.get_or_init(|| {
        let inherited = std::env::var_os("PATH").unwrap_or_default();
        let mut directories: Vec<PathBuf> = Vec::new();
        if cfg!(target_os = "macos") {
            directories.extend(["/opt/homebrew/bin", "/usr/local/bin"].map(PathBuf::from));
        }
        directories.extend(std::env::split_paths(&inherited));
        if cfg!(windows) {
            for base in ["ProgramFiles", "ProgramW6432", "LOCALAPPDATA"] {
                if let Some(root) = std::env::var_os(base) {
                    let root = PathBuf::from(root);
                    directories.push(root.join("Git").join("cmd"));
                    directories.push(root.join("Programs").join("Git").join("cmd"));
                }
            }
        } else {
            directories.extend(["/usr/local/bin", "/usr/bin", "/bin", "/usr/sbin", "/sbin"].map(PathBuf::from));
        }
        let mut unique: Vec<PathBuf> = Vec::new();
        for directory in directories {
            if !directory.as_os_str().is_empty() && !unique.contains(&directory) {
                unique.push(directory);
            }
        }
        std::env::join_paths(unique).unwrap_or(inherited)
    })
}

type OsLockCell = OnceLock<OsString>;

/// The first executable `git` on the search path. Found once; a missing Git is looked for
/// again next time, so installing it while NiceGit runs still works.
pub fn git_executable() -> Option<PathBuf> {
    static FOUND: Mutex<Option<PathBuf>> = Mutex::new(None);
    let mut found = FOUND.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(path) = found.as_ref() {
        return Some(path.clone());
    }
    let name = if cfg!(windows) { "git.exe" } else { "git" };
    let path = std::env::split_paths(search_path()).map(|directory| directory.join(name)).find(|candidate| is_executable(candidate))?;
    *found = Some(path.clone());
    Some(path)
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

fn describe(arguments: &[&str]) -> String {
    format!("git {}", arguments.join(" "))
}

/// Runs Git with NiceGit's environment and returns its raw standard output.
pub fn run_bytes(arguments: &[&str], directory: &Path, options: &RunOptions) -> Result<Vec<u8>> {
    let git = git_executable().ok_or(GitError::GitNotFound)?;
    let mut command = Command::new(git);
    command.args(arguments).current_dir(directory).stdin(Stdio::null());
    for (key, _) in std::env::vars_os() {
        if key.to_str().is_some_and(is_removed_variable) {
            command.env_remove(&key);
        }
    }
    command.env("PATH", search_path());
    command.env("GIT_TERMINAL_PROMPT", "0");
    command.env("GIT_EDITOR", "true");
    command.env("GIT_SEQUENCE_EDITOR", "true");
    if !(options.status_updates_index && arguments.first() == Some(&"status")) {
        command.env("GIT_OPTIONAL_LOCKS", "0");
    }
    if arguments.first().is_some_and(|first| LITERAL_PATHSPEC_COMMANDS.contains(first)) {
        command.env("GIT_LITERAL_PATHSPECS", "1");
    }
    for (key, value) in options.env {
        command.env(OsStr::new(key), OsStr::new(value));
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // Without this, every Git command flashes a console window.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    // `output` reads both pipes while waiting, so a large output cannot block Git, and it waits
    // on the process itself rather than polling.
    let output = command.output().map_err(|error| GitError::failed(describe(arguments), error.to_string()))?;
    let status = output.status.code().unwrap_or(-1);
    let accepted = if options.accepted.is_empty() { &[0][..] } else { options.accepted };
    if !accepted.contains(&status) {
        let message = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let message = if message.is_empty() { format!("Exited with status {status}.") } else { message };
        return Err(GitError::failed(describe(arguments), message));
    }
    Ok(output.stdout)
}

pub fn run(arguments: &[&str], directory: &Path, options: &RunOptions) -> Result<String> {
    let bytes = run_bytes(arguments, directory, options)?;
    Ok(match String::from_utf8(bytes) {
        Ok(text) => text,
        Err(error) => String::from_utf8_lossy(error.as_bytes()).into_owned(),
    })
}

/// How many Git processes a refresh starts at once. Starting every process together makes
/// each slower; a few at a time finishes sooner.
pub fn concurrent_limit() -> usize {
    let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
    (cores / 2).clamp(2, 6)
}

/// Runs independent read-only commands at the same time and returns their results in order.
/// An empty command is skipped and yields an empty string.
pub fn run_concurrently(commands: &[Vec<String>], directory: &Path, options: &RunOptions) -> Vec<Result<String>> {
    let results: Mutex<HashMap<usize, Result<String>>> = Mutex::new(HashMap::new());
    let next = AtomicUsize::new(0);
    let workers = concurrent_limit().min(commands.len().max(1));
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| loop {
                let index = next.fetch_add(1, Ordering::SeqCst);
                let Some(command) = commands.get(index) else { break };
                let result = if command.is_empty() {
                    Ok(String::new())
                } else {
                    let arguments: Vec<&str> = command.iter().map(String::as_str).collect();
                    run(&arguments, directory, options)
                };
                results.lock().unwrap_or_else(|p| p.into_inner()).insert(index, result);
            });
        }
    });
    let mut results = results.into_inner().unwrap_or_else(|p| p.into_inner());
    (0..commands.len()).map(|index| results.remove(&index).unwrap_or_else(|| Ok(String::new()))).collect()
}

/// Removes only Git's final line terminator; paths may legitimately end in whitespace.
pub fn strip_line_terminator(mut value: String) -> String {
    if value.ends_with('\n') {
        value.pop();
        if cfg!(windows) && value.ends_with('\r') {
            value.pop();
        }
    }
    value
}
