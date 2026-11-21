//! Working-tree snapshots: quiet copies of the files a person is editing, kept in the repository
//! under `refs/nicegit/snapshots` so files lost to an editor, a script or a bad command can be
//! brought back.
//!
//! A snapshot is a commit whose tree holds the working tree, untracked files included, as it was
//! when taken. It is built from a temporary copy of the real index, so the real index and the
//! working files are never changed. Each snapshot's commit is a normal Git object, but nothing
//! lists it as a branch.
//!
//! `refs/nicegit/*` is never pushed by NiceGit, which pushes with explicit refspecs. A user's own
//! `git push --mirror` would send these refs, which the Settings text says. The history graph
//! leaves them out, so snapshots never appear as commits.

use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::client::GitClient;
use crate::diff::DiffRequest;
use crate::models::{GitError, Result, StatusKind};
use crate::rebase::{io_failure, Scratch};
use crate::runner::{self, RunOptions};

/// Files larger than this are left out of snapshots, so a large download or build output does not
/// fill the repository.
pub const DEFAULT_MAX_FILE_BYTES: u64 = 20 * 1024 * 1024;

/// The namespace every snapshot ref lives under.
pub const SNAPSHOT_PREFIX: &str = "refs/nicegit/snapshots";

/// Environment for commit-tree: snapshots are never signed, whatever the person's settings.
const SIGNING_OFF: [(&str, &str); 3] = [("GIT_CONFIG_COUNT", "1"), ("GIT_CONFIG_KEY_0", "commit.gpgSign"), ("GIT_CONFIG_VALUE_0", "false")];

/// Environment for commit-tree when Git has no identity: NiceGit's own, used for this object only.
const FALLBACK_IDENTITY: [(&str, &str); 7] = [
    ("GIT_CONFIG_COUNT", "3"),
    ("GIT_CONFIG_KEY_0", "commit.gpgSign"),
    ("GIT_CONFIG_VALUE_0", "false"),
    ("GIT_CONFIG_KEY_1", "user.name"),
    ("GIT_CONFIG_VALUE_1", "NiceGit"),
    ("GIT_CONFIG_KEY_2", "user.email"),
    ("GIT_CONFIG_VALUE_2", "snapshot@nicegit.invalid"),
];

/// Paths handed to `git rm` per call, which keeps each command line well under the system limit.
const REMOVE_CHUNK: usize = 200;

/// One saved state of the working tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotInfo {
    /// The full ref name, such as `refs/nicegit/snapshots/1763712000000`.
    pub reference: String,
    /// The snapshot commit's object ID.
    pub commit: String,
    /// The object ID of the tree of working files.
    pub tree: String,
    /// When the snapshot was taken, in Unix milliseconds. Read from the ref's name.
    pub time: u64,
}

/// A file that differs between a snapshot and the working tree now. The status describes the
/// working tree relative to the snapshot: `Added` is new since the snapshot, `Deleted` is missing
/// now, and `Modified` has changed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotChange {
    pub path: String,
    pub kind: StatusKind,
}

fn now_millis() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|elapsed| elapsed.as_millis() as u64).unwrap_or(0)
}

/// The UTC date and time of a Unix time in milliseconds, as `2026-11-21T09:30:00Z`.
pub fn iso_time(millis: u64) -> String {
    let seconds = (millis / 1000) as i64;
    let days = seconds.div_euclid(86_400);
    let secs_of_day = seconds.rem_euclid(86_400);
    // Howard Hinnant's civil-from-days conversion, for the proleptic Gregorian calendar.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era = (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 { month_index + 3 } else { month_index - 9 };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z", secs_of_day / 3600, (secs_of_day % 3600) / 60, secs_of_day % 60)
}

/// Whether a path names something inside the repository: relative, free of `..`, `.` and `.git`.
pub fn is_safe_relative_path(path: &str) -> bool {
    let parsed = Path::new(path);
    !path.is_empty()
        && parsed.components().next().is_some()
        && parsed.components().all(|part| matches!(part, Component::Normal(name) if !name.eq_ignore_ascii_case(".git")))
}

/// Converts Git's raw path bytes to a path. Git writes UTF-8 names on every platform NiceGit runs on.
fn path_from_bytes(bytes: &[u8]) -> PathBuf {
    PathBuf::from(String::from_utf8_lossy(bytes).into_owned())
}

/// The working tree's current state as a tree object, with untracked files that are not ignored,
/// and without files over `max_file_bytes`. Nothing in the real index or the working files changes.
pub fn working_tree_now(client: &GitClient, directory: &Path, max_file_bytes: u64) -> Result<String> {
    let command = "snapshot";
    let root = client.repository_root(directory)?;
    let git_directory = client.git_directory(&root)?;
    let scratch = Scratch::new(command)?;
    let index = scratch.path("index");
    // A linked worktree's index lives in its own Git directory, which `git_directory` names.
    let real_index = git_directory.join("index");
    if real_index.is_file() {
        fs::copy(&real_index, &index).map_err(|error| io_failure(command, error))?;
    }
    let index_text = index.to_string_lossy().into_owned();
    let environment = [("GIT_INDEX_FILE", index_text.as_str())];
    let options = RunOptions { env: &environment, ..client.options() };
    // Staging many files writes one loose object each, which took seconds on macOS. For this `add`
    // only, a threshold of 1 streams every new file into one pack instead, as staging does.
    let streaming = [
        ("GIT_INDEX_FILE", index_text.as_str()),
        ("GIT_CONFIG_COUNT", "1"),
        ("GIT_CONFIG_KEY_0", "core.bigFileThreshold"),
        ("GIT_CONFIG_VALUE_0", "1"),
    ];
    let adding = RunOptions { env: &streaming, ..client.options() };

    let listed = runner::run_bytes(&["ls-files", "-z", "--cached", "--others", "--exclude-standard"], &root, &client.options())?;
    let mut kept: Vec<&[u8]> = Vec::new();
    let mut large: Vec<String> = Vec::new();
    for record in listed.split(|byte| *byte == 0).filter(|record| !record.is_empty()) {
        let file = root.join(path_from_bytes(record));
        // A file that is gone is kept, so its deletion is recorded.
        let size = fs::symlink_metadata(&file).map(|meta| meta.len()).unwrap_or(0);
        if size > max_file_bytes {
            large.push(String::from_utf8_lossy(record).into_owned());
        } else {
            kept.push(record);
        }
    }

    // Without large files, a plain `add -A` is the fast path: a long list of literal pathspecs
    // makes Git match each one against the index, which took about a second for 5,000 paths.
    if large.is_empty() {
        runner::run(&["add", "-A", "--ignore-errors"], &root, &adding)?;
    } else if !kept.is_empty() {
        // Only the kept paths are added, so a large file is never hashed into the object store.
        // An empty list is skipped, since `add -A` with no pathspec would take everything.
        let list = scratch.path("paths");
        let mut bytes = Vec::new();
        for record in &kept {
            bytes.extend_from_slice(record);
            bytes.push(0);
        }
        fs::write(&list, bytes).map_err(|error| io_failure(command, error))?;
        let list_argument = format!("--pathspec-from-file={}", list.to_string_lossy());
        runner::run(&["add", "-A", "--ignore-errors", "--pathspec-file-nul", list_argument.as_str()], &root, &adding)?;
    }
    // Large files that the copied index already holds are dropped, so they are not in the tree.
    for chunk in large.chunks(REMOVE_CHUNK) {
        let mut arguments = vec!["rm", "--cached", "-q", "--ignore-unmatch", "--"];
        arguments.extend(chunk.iter().map(String::as_str));
        runner::run(&arguments, &root, &options)?;
    }
    Ok(runner::run(&["write-tree"], &root, &options)?.trim().to_string())
}

/// Saves the working tree as a new snapshot. Returns `None` when nothing has changed since the
/// latest snapshot or since HEAD, so there is nothing new to keep.
///
/// The snapshot commit is an internal object, not a commit on any branch. Its identity is the
/// person's own when Git has one; otherwise it is NiceGit's, and it is never signed.
pub fn take_snapshot(client: &GitClient, directory: &Path, max_file_bytes: u64) -> Result<Option<SnapshotInfo>> {
    let command = "snapshot";
    let root = client.repository_root(directory)?;
    let tree = working_tree_now(client, &root, max_file_bytes)?;
    let latest = list_snapshots(client, &root)?.into_iter().next();
    let head_tree = client.try_trimmed(&["rev-parse", "--verify", "--quiet", "HEAD^{tree}"], &root);
    if latest.as_ref().is_some_and(|snapshot| snapshot.tree == tree) || head_tree.as_deref() == Some(tree.as_str()) {
        return Ok(None);
    }
    let head = client.head(&root);
    // Times are strictly increasing, so two snapshots in one millisecond keep distinct refs.
    let time = now_millis().max(latest.as_ref().map_or(0, |snapshot| snapshot.time + 1));
    let message = format!("NiceGit snapshot {}", iso_time(time));
    let (name, email) = client.identity(&root);
    let environment: &[(&str, &str)] = if name.is_empty() || email.is_empty() { &FALLBACK_IDENTITY } else { &SIGNING_OFF };
    let mut arguments = vec!["commit-tree", tree.as_str(), "-m", message.as_str()];
    if let Some(parent) = head.as_deref() {
        arguments.extend(["-p", parent]);
    }
    let options = RunOptions { env: environment, ..client.options() };
    let commit = runner::run(&arguments, &root, &options)
        .map_err(|error| GitError::failed(command, format!("Could not write the snapshot commit: {error}")))?
        .trim()
        .to_string();
    let reference = format!("{SNAPSHOT_PREFIX}/{time}");
    let zero = "0".repeat(commit.len());
    client.run(&["update-ref", &reference, &commit, &zero], &root)?;
    Ok(Some(SnapshotInfo { reference, commit, tree, time }))
}

/// Every snapshot, newest first. A ref whose name is not a snapshot time is ignored.
pub fn list_snapshots(client: &GitClient, directory: &Path) -> Result<Vec<SnapshotInfo>> {
    let output = client.run(&["for-each-ref", "--format=%(refname) %(objectname) %(tree)", SNAPSHOT_PREFIX], directory)?;
    let mut snapshots: Vec<SnapshotInfo> = output
        .lines()
        .filter_map(|line| {
            let mut fields = line.split(' ');
            let (reference, commit, tree) = (fields.next()?, fields.next()?, fields.next()?);
            let time = reference.strip_prefix(SNAPSHOT_PREFIX)?.strip_prefix('/')?.parse::<u64>().ok()?;
            Some(SnapshotInfo { reference: reference.to_string(), commit: commit.to_string(), tree: tree.to_string(), time })
        })
        .collect();
    snapshots.sort_by_key(|snapshot| std::cmp::Reverse(snapshot.time));
    Ok(snapshots)
}

/// Deletes one snapshot. The ref is removed only while it still names the commit that was listed.
pub fn delete_snapshot(client: &GitClient, directory: &Path, snapshot: &SnapshotInfo) -> Result<()> {
    client.run(&["update-ref", "-d", &snapshot.reference, &snapshot.commit], directory).map(drop)
}

/// Deletes snapshots beyond the newest `keep_count`, and any older than `keep_days`. Each ref is
/// removed atomically against the commit that was listed. Their objects are removed by Git's own
/// garbage collection. Returns how many were deleted.
pub fn prune_snapshots(client: &GitClient, directory: &Path, keep_count: usize, keep_days: u64) -> Result<usize> {
    let cutoff = now_millis().saturating_sub(keep_days.saturating_mul(86_400_000));
    let mut removed = 0;
    for (index, snapshot) in list_snapshots(client, directory)?.iter().enumerate() {
        if index >= keep_count || snapshot.time < cutoff {
            // A ref that changed since it was listed is left alone; the next prune will see it again.
            if delete_snapshot(client, directory, snapshot).is_ok() {
                removed += 1;
            }
        }
    }
    Ok(removed)
}

/// The changes between a snapshot's tree and the working tree now, in path order.
pub fn snapshot_changes(client: &GitClient, directory: &Path, snapshot: &SnapshotInfo, current_tree: &str) -> Result<Vec<SnapshotChange>> {
    let output = client.run(&["diff-tree", "-r", "-z", "--no-renames", "--name-status", &snapshot.tree, current_tree], directory)?;
    Ok(parse_name_status(&output))
}

/// Reads `diff-tree -z --name-status` output: a status letter, then the path, each NUL-terminated.
fn parse_name_status(output: &str) -> Vec<SnapshotChange> {
    let mut fields = output.split('\0').filter(|field| !field.is_empty());
    let mut changes = Vec::new();
    while let (Some(status), Some(path)) = (fields.next(), fields.next()) {
        let kind = match status.chars().next() {
            Some('A') => StatusKind::Added,
            Some('D') => StatusKind::Deleted,
            _ => StatusKind::Modified,
        };
        changes.push(SnapshotChange { path: path.to_string(), kind });
    }
    changes
}

/// How many files differ between each snapshot and the working tree now, in the same order as
/// `snapshots`. The working tree is read once, and the comparisons run a few at a time.
pub fn differences_from_now(client: &GitClient, directory: &Path, snapshots: &[SnapshotInfo]) -> Result<Vec<usize>> {
    let current = working_tree_now(client, directory, DEFAULT_MAX_FILE_BYTES)?;
    let commands: Vec<Vec<String>> = snapshots
        .iter()
        .map(|snapshot| {
            ["diff-tree", "-r", "-z", "--no-renames", "--name-only", snapshot.tree.as_str(), current.as_str()].map(str::to_string).to_vec()
        })
        .collect();
    let root = client.repository_root(directory)?;
    runner::run_concurrently(&commands, &root, &client.options())
        .into_iter()
        .map(|result| result.map(|text| text.split('\0').filter(|field| !field.is_empty()).count()))
        .collect()
}

/// The patch for one file between a snapshot and the working tree now, with the diff options the
/// app asks for. Empty when the file did not change.
pub fn snapshot_file_patch(
    client: &GitClient,
    directory: &Path,
    snapshot: &SnapshotInfo,
    path: &str,
    request: DiffRequest,
) -> Result<String> {
    let root = client.repository_root(directory)?;
    let current = working_tree_now(client, &root, DEFAULT_MAX_FILE_BYTES)?;
    let context = request.context.argument();
    let mut arguments = vec!["diff", "--no-ext-diff", "--no-color"];
    if request.ignore_whitespace {
        arguments.push("-w");
    }
    arguments.extend(context.as_deref());
    arguments.extend([snapshot.tree.as_str(), current.as_str(), "--", path]);
    client.run(&arguments, &root)
}

/// Writes the snapshot's version of each path into the working tree. The index is not changed.
///
/// Every path must exist in the snapshot as a file; a path that does not is refused and nothing
/// is written. Before anything is written, the working tree is saved as a snapshot of its own, so
/// the restore can be undone; that snapshot is skipped when the working tree is already saved.
pub fn restore_from_snapshot(client: &GitClient, snapshot: &SnapshotInfo, paths: &[String], directory: &Path) -> Result<()> {
    let command = "restore snapshot";
    let root = client.repository_root(directory)?;
    if paths.is_empty() {
        return Ok(());
    }
    if let Some(path) = paths.iter().find(|path| !is_safe_relative_path(path)) {
        return Err(GitError::failed(command, format!("{path} is not a file inside this repository.")));
    }
    let mut files = Vec::new();
    for path in paths {
        let (mode, bytes) = snapshot_entry(client, &root, snapshot, path)?;
        files.push((path.as_str(), mode, bytes));
    }
    take_snapshot(client, &root, DEFAULT_MAX_FILE_BYTES)
        .map_err(|error| GitError::failed(command, format!("Could not save the current files first: {error}")))?;
    for (path, mode, bytes) in files {
        write_working_file(&root.join(path), &mode, &bytes)
            .map_err(|error| GitError::failed(command, format!("Could not write {path}: {error}")))?;
    }
    Ok(())
}

/// The mode and content of one path in the snapshot. A folder, a submodule, or a missing path is
/// refused, so nothing is ever deleted or guessed at.
fn snapshot_entry(client: &GitClient, root: &Path, snapshot: &SnapshotInfo, path: &str) -> Result<(String, Vec<u8>)> {
    let command = "restore snapshot";
    let listing = client.run(&["ls-tree", "-z", &snapshot.tree, "--", path], root)?;
    let entry = listing.split('\0').find_map(|record| {
        let (metadata, listed) = record.split_once('\t')?;
        (listed == path).then_some(metadata)
    });
    let Some(metadata) = entry else {
        return Err(GitError::failed(command, format!("{path} is not in the snapshot, so it cannot be restored.")));
    };
    let fields: Vec<&str> = metadata.split(' ').collect();
    let (mode, kind, oid) = (fields[0], fields.get(1).copied().unwrap_or_default(), fields.get(2).copied().unwrap_or_default());
    if kind != "blob" {
        return Err(GitError::failed(command, format!("{path} is a folder or submodule in the snapshot. Choose individual files.")));
    }
    let bytes = runner::run_bytes(&["cat-file", "blob", oid], root, &client.options())?;
    Ok((mode.to_string(), bytes))
}

/// Writes one file, creating its folders, with the executable bit or symbolic link the snapshot
/// recorded. An existing folder at the path is refused.
fn write_working_file(file: &Path, mode: &str, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = file.parent() {
        fs::create_dir_all(parent)?;
    }
    if file.is_dir() && !file.is_symlink() {
        return Err(std::io::Error::other("a folder is in the way"));
    }
    if mode == "120000" {
        return write_symlink(file, bytes);
    }
    fs::write(file, bytes)?;
    set_executable(file, mode == "100755")
}

#[cfg(unix)]
fn write_symlink(file: &Path, target: &[u8]) -> std::io::Result<()> {
    use std::os::unix::ffi::OsStrExt;
    if file.symlink_metadata().is_ok() {
        fs::remove_file(file)?;
    }
    std::os::unix::fs::symlink(std::ffi::OsStr::from_bytes(target), file)
}

#[cfg(not(unix))]
fn write_symlink(_file: &Path, _target: &[u8]) -> std::io::Result<()> {
    Err(std::io::Error::other("symbolic links cannot be restored on this system"))
}

#[cfg(unix)]
fn set_executable(file: &Path, executable: bool) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mode = if executable { 0o755 } else { 0o644 };
    fs::set_permissions(file, fs::Permissions::from_mode(mode))
}

#[cfg(not(unix))]
fn set_executable(_file: &Path, _executable: bool) -> std::io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_time_formats_utc() {
        assert_eq!(iso_time(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso_time(1_763_712_000_000), "2025-11-21T08:00:00Z");
        assert_eq!(iso_time(951_782_400_000), "2000-02-29T00:00:00Z");
    }

    #[test]
    fn safe_paths_stay_inside_the_repository() {
        assert!(is_safe_relative_path("src/main.rs"));
        assert!(is_safe_relative_path("a b/c\nd"));
        assert!(!is_safe_relative_path(""));
        assert!(!is_safe_relative_path("../outside"));
        assert!(!is_safe_relative_path("a/../b"));
        assert!(!is_safe_relative_path("/etc/passwd"));
        assert!(!is_safe_relative_path(".git/config"));
        assert!(!is_safe_relative_path("sub/.GIT/HEAD"));
        assert!(!is_safe_relative_path("./file"));
    }

    #[test]
    fn name_status_pairs_map_to_kinds() {
        let changes = parse_name_status("M\0a.txt\0A\0new file.txt\0D\0gone\0");
        assert_eq!(
            changes,
            vec![
                SnapshotChange { path: "a.txt".into(), kind: StatusKind::Modified },
                SnapshotChange { path: "new file.txt".into(), kind: StatusKind::Added },
                SnapshotChange { path: "gone".into(), kind: StatusKind::Deleted },
            ]
        );
    }
}
