//! What merging, or rebasing onto, a commit would do, worked out without touching the working
//! tree or the index. Port of `GitMergePreview.swift`.

use std::path::Path;

use crate::client::GitClient;
use crate::models::{GitError, Result};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MergeOutcome {
    /// The current checkout already contains the source.
    UpToDate,
    /// The checkout can move forward to the source with no merge commit.
    FastForward,
    /// Git can merge automatically.
    Clean,
    /// These paths would need resolving.
    Conflicts(Vec<String>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MergePreview {
    pub outcome: MergeOutcome,
    /// Files the result would change compared with the current checkout.
    pub changed_file_count: usize,
    /// Ignored local files sit where incoming files would go, and NiceGit refuses such a merge.
    pub blocked_by_ignored_files: bool,
    /// Set for a rebase prediction. Rebasing replays commits one at a time and can conflict
    /// differently from the merge this is computed from.
    pub is_estimate: bool,
}

impl GitClient {
    /// Predicts merging `source` into HEAD, using Git's in-memory merge.
    pub fn preview_merge(&self, source: &str, directory: &Path) -> Result<MergePreview> {
        self.preview(source, directory, false)
    }

    /// Predicts rebasing HEAD onto `source`. This is an estimate: see [`MergePreview::is_estimate`].
    pub fn preview_rebase(&self, source: &str, directory: &Path) -> Result<MergePreview> {
        self.preview(source, directory, true)
    }

    fn preview(&self, source: &str, directory: &Path, is_estimate: bool) -> Result<MergePreview> {
        let head = self.run_trimmed(&["rev-parse", "--verify", "HEAD"], directory)?;
        let target = self.resolve_commit(source, directory)?;
        let is_ancestor = |older: &str, newer: &str| self.run(&["merge-base", "--is-ancestor", older, newer], directory).is_ok();
        if is_ancestor(&target, &head) {
            return Ok(MergePreview {
                outcome: MergeOutcome::UpToDate,
                changed_file_count: 0,
                blocked_by_ignored_files: false,
                is_estimate,
            });
        }
        let blocked_by_ignored_files = self.require_no_ignored_merge_collisions(&target, directory).is_err();
        if is_ancestor(&head, &target) {
            let changed_file_count = self.changed_file_count(&head, &target, directory)?;
            return Ok(MergePreview { outcome: MergeOutcome::FastForward, changed_file_count, blocked_by_ignored_files, is_estimate });
        }
        // With -z and --name-only, Git prints the merged tree, then each conflicted path, then an
        // empty field before the messages. Exit status 1 means the merge has conflicts.
        let output = self.run_accepting(&["merge-tree", "--write-tree", "--name-only", "-z", &head, &target], directory, &[0, 1])?;
        let mut fields = output.split('\0');
        let tree = fields.next().unwrap_or_default();
        if tree.is_empty() {
            return Err(GitError::failed("merge preview", "Git did not report a merge result."));
        }
        let mut conflicted: Vec<String> = Vec::new();
        for path in fields {
            if path.is_empty() {
                break;
            }
            if !conflicted.iter().any(|known| known == path) {
                conflicted.push(path.to_string());
            }
        }
        let outcome = if conflicted.is_empty() { MergeOutcome::Clean } else { MergeOutcome::Conflicts(conflicted) };
        let changed_file_count = self.changed_file_count(&head, tree, directory)?;
        Ok(MergePreview { outcome, changed_file_count, blocked_by_ignored_files, is_estimate })
    }

    fn changed_file_count(&self, older: &str, newer: &str, directory: &Path) -> Result<usize> {
        let output = self.run(&["diff-tree", "-r", "--name-only", "-z", "--no-renames", older, newer, "--"], directory)?;
        Ok(output.split('\0').filter(|path| !path.is_empty()).count())
    }
}
