//! Comparing two commits, or a commit with the working files.

use std::path::{Component, Path};

use crate::client::GitClient;
use crate::models::{GitError, Result};
use crate::runner;

/// A file that differs in a comparison, with Git's change letter (A, M, D, T).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComparedFile {
    pub status: String,
    pub path: String,
}

impl GitClient {
    /// Files that differ between two commits, or between a commit and the working files when
    /// `to` is `None`. Untracked files are not part of a comparison. Sorted by path.
    pub fn compare_files(&self, from: &str, to: Option<&str>, directory: &Path) -> Result<Vec<ComparedFile>> {
        let revisions = self.comparison_revisions(from, to, directory)?;
        let mut arguments = vec!["diff", "--name-status", "-z", "--no-renames", "--no-ext-diff"];
        arguments.extend(revisions.iter().map(String::as_str));
        arguments.push("--");
        let output = self.run(&arguments, directory)?;
        let fields: Vec<&str> = output.split('\0').collect();
        let mut files: Vec<ComparedFile> = fields
            .chunks(2)
            .filter(|pair| pair.len() == 2 && !pair[1].is_empty())
            .map(|pair| ComparedFile { status: pair[0].to_string(), path: pair[1].to_string() })
            .collect();
        files.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(files)
    }

    /// The text change to one file between two commits, or from a commit to the working file.
    pub fn compare_file_diff(&self, from: &str, to: Option<&str>, path: &str, ignore_whitespace: bool, directory: &Path) -> Result<String> {
        let revisions = self.comparison_revisions(from, to, directory)?;
        let mut arguments = vec!["diff", "--patch", "--unified=3", "--no-renames", "--no-ext-diff", "--no-color"];
        if ignore_whitespace {
            arguments.push("-w");
        }
        arguments.extend(revisions.iter().map(String::as_str));
        arguments.extend(["--", path]);
        self.run(&arguments, directory)
    }

    /// The bytes of a file at a commit, or in the working tree when `revision` is `None`.
    /// Returns `None` when the file does not exist on that side, for example an added file's
    /// earlier version. Binary content is returned unchanged, which image comparison needs.
    pub fn file_bytes_at(&self, revision: Option<&str>, path: &str, directory: &Path) -> Result<Option<Vec<u8>>> {
        let relative = Path::new(path);
        if path.is_empty() || !relative.components().all(|component| matches!(component, Component::Normal(_))) {
            return Err(GitError::failed("read file", "This path is not inside the repository."));
        }
        match revision {
            None => match std::fs::read(directory.join(relative)) {
                Ok(bytes) => Ok(Some(bytes)),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(error) => Err(GitError::failed("read file", error.to_string())),
            },
            Some(revision) => {
                let commit = self.resolve_commit(revision, directory)?;
                let object = format!("{commit}:{path}");
                // A missing path makes `cat-file -t` fail; only a blob has content to show.
                if self.try_trimmed(&["cat-file", "-t", &object], directory).as_deref() != Some("blob") {
                    return Ok(None);
                }
                Ok(Some(runner::run_bytes(&["cat-file", "blob", &object], directory, &self.options())?))
            }
        }
    }

    /// Resolves both sides to object IDs, so neither can be read as an option or a path.
    fn comparison_revisions(&self, from: &str, to: Option<&str>, directory: &Path) -> Result<Vec<String>> {
        let mut revisions = vec![self.resolve_commit(from, directory)?];
        if let Some(to) = to {
            revisions.push(self.resolve_commit(to, directory)?);
        }
        Ok(revisions)
    }
}
