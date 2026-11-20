//! Applying a patch from text or a file. A patch is previewed first: the files it changes, whether
//! it applies, and for a mailbox patch from `git format-patch`, the commits it holds. It is then
//! applied to the working files, to the index as well, or as commits made by `git am`. The patch
//! is written to a private temporary file that is removed whatever the outcome.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::blame::unquote;
use crate::client::GitClient;
use crate::models::{GitError, Result};

/// The largest patch that can be previewed or applied.
pub const MAX_PATCH_BYTES: usize = 50 * 1024 * 1024;

const COMMAND: &str = "apply";

/// Whether the text is one diff, or a mailbox of commits as `git format-patch` writes them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PatchKind {
    Diff,
    Mailbox,
}

/// How a file changes. A file with no header of its own counts as modified.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileStatus {
    Added,
    Deleted,
    Modified,
    Renamed,
}

/// One file a patch changes. Counts are zero for a binary file, which `binary` marks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PatchFile {
    pub path: String,
    pub old_path: Option<String>,
    pub additions: usize,
    pub deletions: usize,
    pub binary: bool,
    pub status: FileStatus,
}

/// What a patch would do, read without changing the repository.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PatchPreview {
    pub kind: PatchKind,
    pub files: Vec<PatchFile>,
    /// The subjects of the commits in a mailbox patch, without their `[PATCH n/m]` tags.
    pub commits: Vec<String>,
    pub applies_cleanly: bool,
    /// Whether Git's three-way merge could apply the patch when it does not apply cleanly.
    pub three_way_possible: bool,
    /// Git's reason the patch does not apply cleanly.
    pub problem: Option<String>,
}

/// Where an applied patch goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApplyMode {
    /// The working files, leaving the changes unstaged.
    WorkingTree,
    /// The working files and the index, so the changes are staged.
    Index,
    /// One commit for each message in a mailbox patch, made with `git am`.
    Commits,
}

/// A patch written to a private temporary file. The file is removed when this is dropped.
struct TempPatch(PathBuf);

impl TempPatch {
    fn write(contents: &[u8]) -> Result<Self> {
        static SEQUENCE: AtomicU64 = AtomicU64::new(0);
        let directory = std::env::temp_dir().join("nicegit-patches");
        std::fs::create_dir_all(&directory).map_err(|error| GitError::failed(COMMAND, error.to_string()))?;
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|time| time.as_nanos()).unwrap_or(0);
        let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
        // Owned before writing, so a partly written file is removed too.
        let temp = Self(directory.join(format!("{}-{nanos}-{sequence}.patch", std::process::id())));
        std::fs::write(&temp.0, contents).map_err(|error| GitError::failed(COMMAND, error.to_string()))?;
        Ok(temp)
    }

    /// The file's path as Git receives it. It is absolute, so it cannot be read as an option.
    fn path_text(&self) -> String {
        self.0.to_string_lossy().into_owned()
    }
}

impl Drop for TempPatch {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn ensure_size(contents: &[u8]) -> Result<()> {
    if contents.len() > MAX_PATCH_BYTES {
        return Err(GitError::failed(COMMAND, "The patch is larger than 50 MB. Apply it in smaller parts."));
    }
    Ok(())
}

/// Git's reason for a failed command, without the command name. Git not being installed is an error.
fn failure_message(error: GitError) -> Result<String> {
    match error {
        GitError::CommandFailed { message, .. } => Ok(message),
        GitError::GitNotFound => Err(error),
        other => Ok(other.to_string()),
    }
}

impl GitClient {
    /// Reads a patch without changing the repository: its files, its commits if it is a mailbox,
    /// and whether it applies. A patch Git cannot read gives a preview with `problem` set.
    pub fn preview_patch(&self, contents: &[u8], directory: &Path) -> Result<PatchPreview> {
        ensure_size(contents)?;
        let temp = TempPatch::write(contents)?;
        self.preview_written(contents, &temp, directory)
    }

    /// The preview of a patch already written to `temp`, which holds the same `contents`.
    fn preview_written(&self, contents: &[u8], temp: &TempPatch, directory: &Path) -> Result<PatchPreview> {
        let text = String::from_utf8_lossy(contents);
        let kind = patch_kind(&text);
        let commits = if kind == PatchKind::Mailbox { mailbox_subjects(&text) } else { Vec::new() };
        if contents.iter().all(u8::is_ascii_whitespace) {
            return Ok(unreadable(kind, commits, "The patch is empty."));
        }
        let file = temp.path_text();
        let numstat = match self.run(&["apply", "--numstat", "-z", "--", &file], directory) {
            Ok(numstat) => numstat,
            Err(error) => return Ok(unreadable(kind, commits, &failure_message(error)?)),
        };
        let files = build_files(parse_numstat(&numstat), &file_headers(&text));
        if files.is_empty() {
            return Ok(unreadable(kind, commits, "The patch has no file changes."));
        }
        let (applies_cleanly, problem) = match self.run(&["apply", "--check", "--", &file], directory) {
            Ok(_) => (true, None),
            Err(error) => (false, Some(failure_message(error)?)),
        };
        let three_way_possible = !applies_cleanly && self.run(&["apply", "--check", "--3way", "--", &file], directory).is_ok();
        Ok(PatchPreview { kind, files, commits, applies_cleanly, three_way_possible, problem })
    }

    /// Applies a patch with the given mode. A patch that does not apply, and no three-way merge
    /// could fix, changes nothing. `three_way` lets Git merge where the patch does not apply
    /// cleanly; conflicts it leaves are reported as a result, and are resolved in Changes.
    /// Returns the message to show.
    pub fn apply_patch_with(&self, contents: &[u8], mode: ApplyMode, three_way: bool, directory: &Path) -> Result<String> {
        self.require_finished_operation(COMMAND, directory)?;
        ensure_size(contents)?;
        let temp = TempPatch::write(contents)?;
        let preview = self.preview_written(contents, &temp, directory)?;
        if mode == ApplyMode::Commits && preview.kind != PatchKind::Mailbox {
            return Err(GitError::failed(
                COMMAND,
                "Only a mailbox patch from format-patch can create commits. Apply it as changes instead.",
            ));
        }
        if !preview.applies_cleanly && !(three_way && preview.three_way_possible) {
            let problem = preview.problem.unwrap_or_else(|| "The patch does not apply.".to_string());
            return Err(GitError::failed(COMMAND, problem));
        }
        refuse_local_files_in_the_way(self, &preview.files, directory)?;
        let file = temp.path_text();
        match mode {
            ApplyMode::WorkingTree => self.apply_changes(&file, false, three_way, directory),
            ApplyMode::Index => self.apply_changes(&file, true, three_way, directory),
            ApplyMode::Commits => self.apply_as_commits(&file, three_way, preview.commits.len(), directory),
        }
    }

    /// Applies the patch to the working files, and to the index when `index` is set.
    fn apply_changes(&self, file: &str, index: bool, three_way: bool, directory: &Path) -> Result<String> {
        if !self.unmerged_files(directory)?.is_empty() {
            return Err(GitError::failed(COMMAND, "Resolve the conflicted files before applying a patch."));
        }
        let mut arguments = vec!["apply"];
        if index {
            arguments.push("--index");
        }
        if three_way {
            arguments.push("--3way");
        }
        arguments.extend(["--", file]);
        match self.run(&arguments, directory) {
            Ok(_) if index => Ok("Applied the patch and staged its changes.".to_string()),
            Ok(_) => Ok("Applied the patch. Its changes are unstaged.".to_string()),
            // A three-way merge that conflicts exits with an error, but leaves the conflicts to resolve.
            Err(error) if three_way => {
                let conflicted = self.unmerged_files(directory)?;
                if conflicted.is_empty() {
                    return Err(error);
                }
                Ok(format!("Applied with conflicts in {} files; resolve them in Changes.", conflicted.len()))
            }
            Err(error) => Err(error),
        }
    }

    /// Creates one commit for each message in the mailbox patch. A failed `git am` is aborted, so
    /// no half-applied sequence is left in progress.
    fn apply_as_commits(&self, file: &str, three_way: bool, count: usize, directory: &Path) -> Result<String> {
        self.require_clean_index(directory)?;
        let mut arguments = vec!["am"];
        if three_way {
            arguments.push("--3way");
        }
        arguments.push(file);
        if let Err(error) = self.run_commit(&arguments, directory) {
            // Fails harmlessly when `git am` never started, since then there is nothing to abort.
            let _ = self.run(&["am", "--abort"], directory);
            return Err(error);
        }
        self.run_due_maintenance(directory);
        Ok(match count {
            0 => "Created the commits from the patch.".to_string(),
            1 => "Created 1 commit from the patch.".to_string(),
            count => format!("Created {count} commits from the patch."),
        })
    }

    /// Refuses when commits from a patch would mix with staged or conflicted changes.
    fn require_clean_index(&self, directory: &Path) -> Result<()> {
        let staged = self.run(&["diff", "--cached", "--name-only", "-z"], directory)?;
        if !staged.is_empty() || !self.unmerged_files(directory)?.is_empty() {
            return Err(GitError::failed("am", "Commit or unstage your changes before creating commits from a patch."));
        }
        Ok(())
    }

    /// The paths Git lists as unmerged in the index, sorted and without repeats.
    fn unmerged_files(&self, directory: &Path) -> Result<Vec<String>> {
        let output = self.run(&["ls-files", "-u", "-z"], directory)?;
        let mut paths: Vec<String> =
            output.split('\0').filter_map(|entry| entry.split_once('\t').map(|(_, path)| path.to_string())).collect();
        paths.sort();
        paths.dedup();
        Ok(paths)
    }

    /// Applies a patch to the working files, checking it applies cleanly first. Both steps read
    /// the same captured bytes, not a file that could change in between.
    pub fn apply_patch(&self, contents: &[u8], directory: &Path) -> Result<()> {
        self.apply_patch_with(contents, ApplyMode::WorkingTree, false, directory).map(drop)
    }
}

/// A preview for a patch Git cannot read, with its problem as the reason.
fn unreadable(kind: PatchKind, commits: Vec<String>, problem: &str) -> PatchPreview {
    PatchPreview { kind, files: Vec::new(), commits, applies_cleanly: false, three_way_possible: false, problem: Some(problem.to_string()) }
}

/// Refuses a patch that would create a file where an untracked or ignored local file already is.
/// Git would refuse the same file in a tracked folder, but an ignored file is easy to overlook.
fn refuse_local_files_in_the_way(client: &GitClient, files: &[PatchFile], directory: &Path) -> Result<()> {
    for file in files.iter().filter(|file| matches!(file.status, FileStatus::Added | FileStatus::Renamed)) {
        // Without --exclude-standard, `ls-files --others` lists ignored files as well.
        if !client.run(&["ls-files", "--others", "-z", "--", &file.path], directory)?.is_empty() {
            return Err(GitError::failed(
                COMMAND,
                format!("{} already exists as a local file that Git does not track. Move or back it up first.", file.path),
            ));
        }
    }
    Ok(())
}

/// Whether a line begins a message of a mailbox patch: `From`, a 40-digit commit ID, then a space
/// or the end of the line.
fn is_mail_start(line: &str) -> bool {
    let Some(rest) = line.strip_prefix("From ") else { return false };
    let bytes = rest.as_bytes();
    bytes.len() >= 40 && bytes[..40].iter().all(u8::is_ascii_hexdigit) && (bytes.len() == 40 || bytes[40] == b' ')
}

/// A mailbox patch has message headers, as `git format-patch` writes them. A diff has none.
fn patch_kind(text: &str) -> PatchKind {
    let mail = text.lines().any(is_mail_start)
        && text.lines().any(|line| line.starts_with("From: "))
        && text.lines().any(|line| line.starts_with("Subject: "));
    if mail {
        PatchKind::Mailbox
    } else {
        PatchKind::Diff
    }
}

/// The subjects of a mailbox patch's messages, from the headers before each message's blank line.
fn mailbox_subjects(text: &str) -> Vec<String> {
    let mut subjects = Vec::new();
    let mut in_headers = false;
    for line in text.lines() {
        if is_mail_start(line) {
            in_headers = true;
        } else if in_headers && line.is_empty() {
            in_headers = false;
        } else if in_headers {
            if let Some(subject) = line.strip_prefix("Subject: ") {
                subjects.push(strip_patch_tag(subject).to_string());
            }
        }
    }
    subjects
}

/// Removes a leading `[PATCH n/m]` tag from a subject.
fn strip_patch_tag(subject: &str) -> &str {
    let trimmed = subject.trim_start();
    if let Some(rest) = trimmed.strip_prefix('[') {
        if let Some(end) = rest.find(']') {
            if rest[..end].contains("PATCH") {
                return rest[end + 1..].trim_start();
            }
        }
    }
    trimmed
}

/// One entry of `git apply --numstat -z`. A rename may list its old path after the counts.
#[derive(Debug, PartialEq, Eq)]
struct NumstatEntry {
    path: String,
    old_path: Option<String>,
    additions: Option<usize>,
    deletions: Option<usize>,
}

/// Reads the NUL-separated `git apply --numstat -z` output. A binary file's counts are `-`.
fn parse_numstat(output: &str) -> Vec<NumstatEntry> {
    let tokens: Vec<&str> = output.split('\0').collect();
    let mut entries = Vec::new();
    let mut index = 0;
    while index < tokens.len() {
        let token = tokens[index];
        index += 1;
        let mut fields = token.splitn(3, '\t');
        let (Some(additions), Some(deletions), Some(path)) = (fields.next(), fields.next(), fields.next()) else { continue };
        let (path, old_path) = if path.is_empty() {
            // A rename gives its old and new paths as the next two NUL-terminated entries.
            let old = tokens.get(index).copied().unwrap_or_default().to_string();
            let new = tokens.get(index + 1).copied().unwrap_or_default().to_string();
            index += 2;
            (new, Some(old))
        } else {
            (path.to_string(), None)
        };
        entries.push(NumstatEntry { path, old_path, additions: additions.parse().ok(), deletions: deletions.parse().ok() });
    }
    entries
}

/// What a file's header lines say about it, read before its first hunk. File contents can look
/// like these headers, so nothing after the first hunk counts.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct FileHeader {
    status: Option<FileStatus>,
    old_path: Option<String>,
}

/// The header of each `diff --git` section, in patch order.
fn file_headers(text: &str) -> Vec<FileHeader> {
    let mut headers: Vec<FileHeader> = Vec::new();
    let mut in_headers = false;
    for line in text.lines() {
        if line.starts_with("diff --git ") {
            headers.push(FileHeader::default());
            in_headers = true;
            continue;
        }
        if line.starts_with("@@") || line.starts_with("GIT binary patch") || line.starts_with("Binary files ") {
            in_headers = false;
            continue;
        }
        let Some(header) = headers.last_mut().filter(|_| in_headers) else { continue };
        if line.starts_with("new file mode ") {
            header.status = Some(FileStatus::Added);
        } else if line.starts_with("deleted file mode ") {
            header.status = Some(FileStatus::Deleted);
        } else if let Some(old) = line.strip_prefix("rename from ") {
            header.status = Some(FileStatus::Renamed);
            header.old_path = Some(unquote(old));
        }
    }
    headers
}

/// Joins the counts with the headers. Git lists files in patch order, so they pair by position;
/// if the counts differ, the headers cannot be trusted and every file counts as modified.
fn build_files(entries: Vec<NumstatEntry>, headers: &[FileHeader]) -> Vec<PatchFile> {
    let paired = headers.len() == entries.len();
    entries
        .into_iter()
        .enumerate()
        .map(|(index, entry)| {
            let header = if paired { headers.get(index).cloned().unwrap_or_default() } else { FileHeader::default() };
            let status = if entry.old_path.is_some() { FileStatus::Renamed } else { header.status.unwrap_or(FileStatus::Modified) };
            PatchFile {
                path: entry.path,
                old_path: entry.old_path.or(header.old_path),
                additions: entry.additions.unwrap_or(0),
                deletions: entry.deletions.unwrap_or(0),
                binary: entry.additions.is_none(),
                status,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_a_format_patch_message() {
        let mail = "From 0123456789abcdef0123456789abcdef01234567 Mon Sep 17 00:00:00 2001\nFrom: A <a@x>\nSubject: [PATCH] Fix\n\n";
        assert_eq!(patch_kind(mail), PatchKind::Mailbox);
        assert_eq!(patch_kind("diff --git a/f b/f\n"), PatchKind::Diff);
        assert_eq!(patch_kind("From: someone\nSubject: x\n"), PatchKind::Diff);
    }

    #[test]
    fn strips_patch_tags_from_subjects() {
        assert_eq!(strip_patch_tag("[PATCH 2/3] Add the file"), "Add the file");
        assert_eq!(strip_patch_tag("[PATCH] Plain"), "Plain");
        assert_eq!(strip_patch_tag("[Draft] Not a patch tag"), "[Draft] Not a patch tag");
    }

    #[test]
    fn a_header_inside_a_hunk_does_not_set_the_status() {
        // The removed line "-- /dev/null" reads as "--- /dev/null" once the marker is added.
        let patch = "diff --git a/f b/f\n@@ -1 +1 @@\n-new file mode 100644\n+x\n";
        assert_eq!(file_headers(patch), vec![FileHeader::default()]);
    }

    #[test]
    fn numstat_pairs_rename_paths() {
        let entries = parse_numstat("1\t0\t\0old.txt\0new name.txt\0-\t-\tbin.dat\0");
        assert_eq!(entries[0].old_path.as_deref(), Some("old.txt"));
        assert_eq!(entries[0].path, "new name.txt");
        assert_eq!(entries[1].additions, None);
        assert_eq!(entries[1].path, "bin.dat");
    }
}
