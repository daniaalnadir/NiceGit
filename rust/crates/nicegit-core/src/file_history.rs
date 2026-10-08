//! Commits that changed one file, following renames, and file contents at a revision.

use std::path::Path;

use crate::client::GitClient;
use crate::models::{Commit, GitError, Result};
use crate::parsers::parse_log;
use crate::runner;

/// Commit fields, then (after NUL) the change letter and path. Only the commit part is parsed
/// by the log parser; the name-status part follows it in the same output.
const HISTORY_FORMAT: &str = "--format=%x1e%H%x1f%h%x1f%P%x1f%x1f%s%x1f%an%x1f%ae%x1f%cr%x1f%ct";

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
    pub fn commit_file_diff(&self, hash: &str, path: &str, ignore_whitespace: bool, directory: &Path) -> Result<String> {
        let mut arguments =
            vec!["show", "--first-parent", "-m", "--format=", "--patch", "--unified=3", "--no-renames", "--no-ext-diff", "--no-color"];
        if ignore_whitespace {
            arguments.push("-w");
        }
        arguments.extend([hash, "--", path]);
        self.run(&arguments, directory)
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
}
