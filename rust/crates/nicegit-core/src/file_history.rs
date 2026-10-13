//! Commits that changed one file, following renames, and file contents at a revision.

use std::path::Path;

use crate::client::GitClient;
use crate::diff::DiffRequest;
use crate::models::{short, Commit, GitError, Result};
use crate::parsers::parse_log;
use crate::runner;

/// Commit fields, then (after NUL) the change letter and path. Only the commit part is parsed
/// by the log parser; the name-status part follows it in the same output.
const HISTORY_FORMAT: &str = "--format=%x1e%H%x1f%h%x1f%P%x1f%x1f%s%x1f%an%x1f%ae%x1f%cr%x1f%ct";

/// The most lines one line history may cover. Longer ranges are refused.
pub const MAX_LINE_RANGE: usize = 10_000;

/// Written by `LINE_HISTORY_FORMAT` before each commit. NUL never appears in a text patch, so
/// the pair cannot occur inside one.
const LINE_RECORD_START: [u8; 2] = [0, 1];

/// Each commit's header: the record start, then NUL-separated fields. Subjects and names cannot
/// hold a newline or NUL, so the header always ends at the first newline.
const LINE_HISTORY_FORMAT: &str = "--format=%x00%x01%H%x00%h%x00%P%x00%an%x00%ae%x00%cr%x00%ct%x00%s";

/// One commit that changed a range of lines, and how it changed them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LineHistoryEntry {
    pub commit: Commit,
    /// The patch Git prints for the commit's change to the range: diff headers and hunks, with
    /// only the lines of the range that the commit changed.
    pub patch: String,
}

/// Parses `log -L` output written with `LINE_HISTORY_FORMAT`. Records are split on the record
/// start as bytes, then each header is split on NUL. Patches are decoded lossily, since a file
/// may hold bytes that are not UTF-8.
pub fn parse_line_history(output: &[u8]) -> Vec<LineHistoryEntry> {
    let starts: Vec<usize> = output
        .windows(LINE_RECORD_START.len())
        .enumerate()
        .filter(|(_, pair)| *pair == LINE_RECORD_START)
        .map(|(index, _)| index + LINE_RECORD_START.len())
        .collect();
    let mut entries = Vec::new();
    for (position, &start) in starts.iter().enumerate() {
        let end = starts.get(position + 1).map_or(output.len(), |next| next - LINE_RECORD_START.len());
        let record = &output[start..end];
        let Some(newline) = record.iter().position(|&byte| byte == b'\n') else { continue };
        let fields: Vec<String> =
            record[..newline].splitn(8, |&byte| byte == 0).map(|field| String::from_utf8_lossy(field).into_owned()).collect();
        let [hash, short_hash, parents, author_name, author_email, relative_date, commit_time, subject] = fields.as_slice() else {
            continue;
        };
        // Git ends the header with a newline and separates it from the patch with a blank line.
        let patch = &record[newline + 1..];
        let patch = &patch[patch.iter().position(|&byte| byte != b'\n').unwrap_or(patch.len())..];
        entries.push(LineHistoryEntry {
            commit: Commit {
                hash: hash.clone(),
                short_hash: short_hash.clone(),
                parents: parents.split(' ').filter(|parent| !parent.is_empty()).map(str::to_string).collect(),
                refs: Vec::new(),
                subject: subject.clone(),
                author_name: author_name.clone(),
                author_email: author_email.clone(),
                relative_date: relative_date.clone(),
                commit_time: commit_time.trim().parse().ok(),
            },
            patch: String::from_utf8_lossy(patch).into_owned(),
        });
    }
    entries
}

/// One commit that changed a file, and how it changed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileHistoryEntry {
    pub commit: Commit,
    /// The path the file had in this commit. It differs for commits from before a rename.
    pub path: String,
    /// Git's change letter for the file in this commit: A, M, D, R, or C.
    pub status: String,
}

impl FileHistoryEntry {
    pub fn deletes_file(&self) -> bool {
        self.status == "D"
    }
}

/// Where to read one version of a file from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileVersion {
    /// The file in a commit, named by any revision such as a hash or `abc123^`.
    Revision(String),
    /// The staged version.
    Index,
    /// The file on disk.
    WorkingFile,
}

/// Parses `log -z --name-status` output for one file. Records are split on NUL only, because a
/// path may contain any other character, including Git's field separators. Renames and copies
/// list the old path before the new one.
pub fn parse_file_history(output: &str, path: &str) -> Vec<FileHistoryEntry> {
    let tokens: Vec<&str> = output.split('\0').collect();
    let mut entries = Vec::new();
    let mut index = 0;
    while index < tokens.len() {
        let token = tokens[index];
        index += 1;
        let Some(record) = token.strip_prefix('\u{1e}') else { continue };
        let Some(commit) = parse_log(record).into_iter().next() else { continue };
        // Git separates the commit fields from the change letter with one newline.
        let Some(status) = tokens.get(index).and_then(|token| token.strip_prefix('\n')) else {
            entries.push(FileHistoryEntry { commit, path: path.to_string(), status: "M".to_string() });
            continue;
        };
        index += 1;
        let path_count = if status.starts_with('R') || status.starts_with('C') { 2 } else { 1 };
        let Some(listed) = tokens.get(index + path_count - 1) else { break };
        entries.push(FileHistoryEntry { commit, path: listed.to_string(), status: status.chars().take(1).collect() });
        index += path_count;
    }
    entries
}

impl GitClient {
    /// Commits in the current checkout's history that changed `path`, newest first, following
    /// renames. Returns nothing when the repository has no commits yet.
    pub fn file_history(&self, path: &str, limit: usize, directory: &Path) -> Result<Vec<FileHistoryEntry>> {
        if self.head(directory).is_none() {
            return Ok(Vec::new());
        }
        let count = limit.max(1).to_string();
        let output = self
            .run(&["log", "--follow", "--no-color", "-z", "--name-status", "-n", &count, HISTORY_FORMAT, "HEAD", "--", path], directory)?;
        Ok(parse_file_history(&output, path))
    }

    /// The patch for one file's change in a commit, against its first parent. Renames are not
    /// detected, so the file shows as the path named.
    pub fn commit_file_diff(&self, hash: &str, path: &str, request: DiffRequest, directory: &Path) -> Result<String> {
        let mut arguments = vec!["show", "--first-parent", "-m", "--format=", "--patch"];
        let context = request.context.argument();
        arguments.extend(context.as_deref());
        arguments.extend(["--no-renames", "--no-ext-diff", "--no-color"]);
        if request.ignore_whitespace {
            arguments.push("-w");
        }
        arguments.extend([hash, "--", path]);
        self.run(&arguments, directory)
    }

    /// The commits that changed lines `start` to `end` (one-based, inclusive) of `path`, newest
    /// first, following those lines as they move within the file. Each entry carries the change
    /// it made to the range. The range is read at `revision`, or at HEAD, so uncommitted changes
    /// are not included.
    pub fn line_history(
        &self,
        path: &str,
        start: usize,
        end: usize,
        revision: Option<&str>,
        limit: usize,
        directory: &Path,
    ) -> Result<Vec<LineHistoryEntry>> {
        if start == 0 {
            return Err(GitError::failed("show line history", "Line numbers start at 1."));
        }
        if end < start {
            return Err(GitError::failed("show line history", "The range ends before it starts."));
        }
        if end - start + 1 > MAX_LINE_RANGE {
            return Err(GitError::failed("show line history", format!("A line history covers at most {MAX_LINE_RANGE} lines.")));
        }
        // The path follows `-L<range>:`, so a leading dash would read as an option.
        if path.is_empty() || path.starts_with('-') {
            return Err(GitError::failed("show line history", "The path must name a file and must not start with a dash."));
        }
        // Git's log parses its own arguments, so the revision is passed as an object ID.
        let commit = match revision {
            Some(revision) => self.resolve_commit(revision, directory)?,
            None => match self.head(directory) {
                Some(head) => head,
                None => return Ok(Vec::new()),
            },
        };
        let object = format!("{commit}:{path}");
        if self.try_trimmed(&["cat-file", "-t", &object], directory).as_deref() != Some("blob") {
            return Err(GitError::failed("show line history", format!("{path} is not a file in {}.", short(&commit))));
        }
        let range = format!("-L{start},{end}:{path}");
        let count = limit.max(1).to_string();
        let arguments =
            ["log", "--no-color", "--no-ext-diff", range.as_str(), LINE_HISTORY_FORMAT, "-n", count.as_str(), commit.as_str(), "--"];
        let output = runner::run_bytes(&arguments, directory, &self.options())?;
        Ok(parse_line_history(&output))
    }

    /// The bytes of `path` in `version`, or `None` when that version has no such file. Files
    /// larger than `limit` bytes are refused rather than loaded.
    pub fn file_bytes(&self, path: &str, version: &FileVersion, limit: u64, directory: &Path) -> Result<Option<Vec<u8>>> {
        let object = match version {
            FileVersion::WorkingFile => return read_working_file(&directory.join(path), limit),
            FileVersion::Index => format!(":0:{path}"),
            FileVersion::Revision(revision) => match self.resolve_commit(revision, directory) {
                Ok(commit) => format!("{commit}:{path}"),
                Err(_) => return Ok(None),
            },
        };
        if self.try_trimmed(&["cat-file", "-t", &object], directory).as_deref() != Some("blob") {
            return Ok(None);
        }
        let size = self.run_trimmed(&["cat-file", "-s", &object], directory)?;
        let size: u64 = size.parse().map_err(|_| GitError::failed("show file", "Git reported an unreadable file size."))?;
        if size > limit {
            return Err(too_large());
        }
        Ok(Some(runner::run_bytes(&["cat-file", "blob", &object], directory, &self.options())?))
    }
}

fn too_large() -> GitError {
    GitError::failed("show file", "This file is too large to preview.")
}

fn read_working_file(file: &Path, limit: u64) -> Result<Option<Vec<u8>>> {
    // Symbolic links are not followed: only a regular file has contents to show.
    let Ok(metadata) = std::fs::symlink_metadata(file) else { return Ok(None) };
    if !metadata.is_file() {
        return Ok(None);
    }
    if metadata.len() > limit {
        return Err(too_large());
    }
    std::fs::read(file).map(Some).map_err(|error| GitError::failed("show file", error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_status_letters_and_rename_sources() {
        let commit =
            |hash: &str| format!("\u{1e}{hash}\u{1f}{}\u{1f}\u{1f}\u{1f}subject\u{1f}Ann\u{1f}ann@x\u{1f}now\u{1f}1700000000", &hash[..7]);
        let output =
            format!("{}\0\nR100\0old name.txt\0new name.txt\0{}\0\nM\0new name.txt\0", commit(&"a".repeat(40)), commit(&"b".repeat(40)));
        let entries = parse_file_history(&output, "new name.txt");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].status, "R");
        assert_eq!(entries[0].path, "new name.txt");
        assert_eq!(entries[1].status, "M");
        assert_eq!(entries[1].commit.hash, "b".repeat(40));
    }

    #[test]
    fn parses_line_history_headers_and_patches() {
        let (first, second) = ("a".repeat(40), "b".repeat(40));
        let output = format!(
            "\0\u{1}{first}\0aaaaaaa\0\0Ann\0ann@x\0now\01700000000\0First line\n\n\
             diff --git a/n b/n\n@@ -1 +1 @@\n-x\n+y\n\
             \0\u{1}{second}\0bbbbbbb\0{first}\0Bob Ö\0bob@x\02 days ago\0\0Second\n\n\
             diff --git a/n b/n\n@@ -1 +1 @@\n+caf"
        );
        let mut bytes = output.into_bytes();
        // A patch may hold bytes that are not UTF-8; they are decoded lossily.
        bytes.extend_from_slice(b"\xff\n");

        let entries = parse_line_history(&bytes);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].commit.hash, first);
        assert_eq!(entries[0].commit.short_hash, "aaaaaaa");
        assert!(entries[0].commit.parents.is_empty(), "a root commit has no parents");
        assert_eq!(entries[0].commit.subject, "First line");
        assert_eq!(entries[0].commit.commit_time, Some(1_700_000_000));
        assert!(entries[0].patch.starts_with("diff --git a/n b/n\n"), "the blank line before the patch is dropped");
        assert!(entries[0].patch.contains("+y\n"));
        assert_eq!(entries[1].commit.parents, vec![first.clone()]);
        assert_eq!(entries[1].commit.author_name, "Bob Ö");
        assert_eq!(entries[1].commit.relative_date, "2 days ago");
        assert_eq!(entries[1].commit.commit_time, None, "an empty time is not a number");
        assert!(entries[1].patch.ends_with("+caf\u{fffd}\n"), "invalid UTF-8 is replaced: {:?}", entries[1].patch);
        assert!(parse_line_history(b"").is_empty());
    }
}
