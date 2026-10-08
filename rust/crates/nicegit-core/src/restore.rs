//! Restoring one file to its version in a commit, or in the commit before it.

use std::path::Path;

use crate::client::GitClient;
use crate::models::{GitError, Result};

/// The metadata before the tab of each NUL-terminated record whose path is exactly `path`.
/// Git lists metadata first and the path after the first tab; the path itself may contain tabs.
fn records_for<'a>(listing: &'a str, path: &str) -> Vec<&'a str> {
    listing
        .split('\0')
        .filter_map(|record| {
            let (metadata, listed) = record.split_once('\t')?;
            (listed == path).then_some(metadata)
        })
        .collect()
}

impl GitClient {
    /// Restores one path's staged and working copies to its version at `source`, which is any
    /// revision such as a commit or `<commit>^`. When that version has no such file, the file is
    /// removed and the removal staged. Staged and unstaged edits to the path are replaced.
    ///
    /// Refuses unless the checkout still has `expected_branch` and `expected_head`, no Git
    /// operation is unfinished, the path has no conflict stages, and no untracked, ignored, or
    /// folder entry occupies the path. Verifies index and working file afterwards.
    pub fn restore(&self, path: &str, source: &str, expected_branch: &str, expected_head: Option<&str>, directory: &Path) -> Result<()> {
        let command = "restore file";
        let state = self.require_checkout(expected_branch, expected_head, command, directory)?;
        if state.operation.is_some() {
            return Err(GitError::failed(command, "Finish or abort the current Git operation before restoring files."));
        }
        let commit = self.resolve_commit(source, directory)?;

        // The source entry's metadata is "mode type object"; a missing file has no entry.
        let source_listing = self.run(&["ls-tree", "-z", &commit, "--", path], directory)?;
        let source_fields: Option<Vec<String>> =
            records_for(&source_listing, path).first().map(|metadata| metadata.split(' ').map(str::to_string).collect());
        let source_mode = source_fields.as_ref().and_then(|fields| fields.first()).cloned();
        let source_type = source_fields.as_ref().and_then(|fields| fields.get(1)).cloned();

        // Index entries are "mode object stage".
        let index_listing = self.run(&["ls-files", "-z", "--stage", "--", path], directory)?;
        let index_records = records_for(&index_listing, path);
        if index_records.iter().any(|metadata| metadata.split(' ').nth(2).is_some_and(|stage| stage != "0")) {
            return Err(GitError::failed(command, "This file has unresolved conflicts. Resolve them before restoring it."));
        }
        let index_is_submodule = index_records.iter().any(|metadata| metadata.starts_with("160000 "));
        if source_mode.as_deref() == Some("160000") || index_is_submodule {
            return Err(GitError::failed(
                command,
                "Submodules cannot be restored here. Check out the wanted commit inside the submodule instead.",
            ));
        }
        if source_type.as_deref().is_some_and(|kind| kind != "blob") {
            return Err(GitError::failed(command, "This path is a folder in the selected commit. Choose an individual file."));
        }

        let file = directory.join(path);
        let working = std::fs::symlink_metadata(&file).ok();
        // Git would overwrite or refuse a local file it does not track, and a folder must never be
        // replaced silently, so only a tracked file can be restored over.
        if let Some(metadata) = &working {
            if index_records.is_empty() || metadata.is_dir() {
                return Err(GitError::failed(
                    command,
                    "An untracked file or a folder is at this path in your working tree. Move or rename it before restoring.",
                ));
            }
        }
        if source_mode.is_none() && index_records.is_empty() {
            // Neither the commit nor the index has the file, and nothing is in the working tree.
            return Ok(());
        }

        if source_mode.is_some() {
            let source_argument = format!("--source={commit}");
            self.run(&["restore", &source_argument, "--staged", "--worktree", "--", path], directory)?;
        } else {
            // Forced, so an edited file is removed too; the removal is staged.
            self.run(&["rm", "--force", "--", path], directory)?;
        }

        let matches = if source_mode.is_some() {
            self.run(&["diff", "--quiet", "--no-ext-diff", &commit, "--", path], directory).is_ok()
                && self.run(&["diff", "--quiet", "--no-ext-diff", "--cached", &commit, "--", path], directory).is_ok()
        } else {
            std::fs::symlink_metadata(&file).is_err() && self.run(&["ls-files", "-z", "--", path], directory)?.is_empty()
        };
        if !matches {
            return Err(GitError::failed(
                command,
                "Git restored this path, but it still differs from the selected commit. Refresh and review it.",
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_match_the_exact_path_only() {
        let listing = "100644 blob abc\tnotes.txt\x00100644 blob def\tnotes.txt.bak\x00100644 blob 123\ttab\tname\x00";
        assert_eq!(records_for(listing, "notes.txt"), vec!["100644 blob abc"]);
        assert_eq!(records_for(listing, "tab\tname"), vec!["100644 blob 123"]);
        assert!(records_for(listing, "missing").is_empty());
    }
}
