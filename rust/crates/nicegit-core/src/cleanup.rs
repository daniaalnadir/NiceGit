//! Branch clean-up: finds local branches that are merged or stale, deletes the chosen ones in one
//! step, and returns what is needed to restore each of them.

use std::collections::HashSet;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::client::GitClient;
use crate::models::{GitError, Result};
use crate::parsers::parse_worktrees;

const CANDIDATE_FORMAT: &str = "%(refname)%09%(objectname)%09%(committerdate:unix)%09%(HEAD)%09%(contents:subject)";
const RESTORED_MESSAGE: &str = "branch: restored by NiceGit";
const SECONDS_PER_DAY: i64 = 86_400;

/// A local branch that may no longer be needed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BranchCandidate {
    pub name: String,
    /// The commit the branch pointed to when it was listed. Deletion requires it still to be there.
    pub tip: String,
    /// Committer time of the tip, in seconds since the Unix epoch.
    pub last_commit_time: Option<i64>,
    pub subject: String,
    /// Every commit on the branch is already in the current branch, so deleting it loses nothing.
    pub is_merged: bool,
}

/// A deleted local branch, with what is needed to recreate it exactly.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BranchDeletion {
    pub name: String,
    pub tip: String,
    /// The branch's upstream settings, which Git removes along with the branch.
    pub upstream_remote: Option<String>,
    pub upstream_merge: Option<String>,
}

/// The branches a clean-up deleted, and why it stopped early if it did. Branches deleted before
/// the failure are listed in `deleted`, so they can still be restored.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BranchDeletionReport {
    pub deleted: Vec<BranchDeletion>,
    pub failure: Option<GitError>,
}

impl GitClient {
    /// Local branches merged into the current branch, plus those whose tip has no commit newer than
    /// `inactive_days`. The current branch, and branches checked out in other worktrees, are never listed.
    /// Oldest first.
    pub fn branch_cleanup_candidates(&self, inactive_days: i64, directory: &Path) -> Result<Vec<BranchCandidate>> {
        let merged: HashSet<String> = if self.head(directory).is_some() {
            self.run(&["for-each-ref", "--merged", "HEAD", "--format=%(refname)", "refs/heads/"], directory)?
                .lines()
                .map(str::to_string)
                .collect()
        } else {
            HashSet::new()
        };
        let worktrees = self.run(&["worktree", "list", "--porcelain", "-z"], directory)?;
        let checked_out: HashSet<String> = parse_worktrees(&worktrees).into_iter().filter_map(|worktree| worktree.branch).collect();
        let cutoff = now_seconds() - inactive_days.max(0) * SECONDS_PER_DAY;
        let format = format!("--format={CANDIDATE_FORMAT}");
        let output = self.run(&["for-each-ref", &format, "refs/heads/"], directory)?;

        let mut candidates = Vec::new();
        for line in output.lines() {
            let fields: Vec<&str> = line.splitn(5, '\t').collect();
            if fields.len() != 5 || fields[3] == "*" {
                continue;
            }
            let Some(name) = fields[0].strip_prefix("refs/heads/") else { continue };
            if checked_out.contains(name) {
                continue;
            }
            let is_merged = merged.contains(fields[0]);
            let last_commit_time = fields[2].parse::<i64>().ok();
            let stale = last_commit_time.is_some_and(|time| time < cutoff);
            if !is_merged && !stale {
                continue;
            }
            candidates.push(BranchCandidate {
                name: name.to_string(),
                tip: fields[1].to_string(),
                last_commit_time,
                subject: fields[4].to_string(),
                is_merged,
            });
        }
        candidates.sort_by_key(|candidate| candidate.last_commit_time);
        Ok(candidates)
    }

    /// Deletes the candidates in order, saving each one for restoring. Unmerged candidates are
    /// deleted only when `include_unmerged` is set. Each deletion is atomic against the tip that
    /// was listed, so a branch that has moved since is kept and stops the clean-up.
    pub fn delete_branches_keeping_undo(
        &self,
        candidates: &[BranchCandidate],
        include_unmerged: bool,
        directory: &Path,
    ) -> BranchDeletionReport {
        let mut report = BranchDeletionReport::default();
        for candidate in candidates.iter().filter(|candidate| candidate.is_merged || include_unmerged) {
            match self.delete_listed_branch(candidate, directory) {
                Ok(deletion) => report.deleted.push(deletion),
                Err(error) => {
                    report.failure = Some(error);
                    break;
                }
            }
        }
        report
    }

    fn delete_listed_branch(&self, candidate: &BranchCandidate, directory: &Path) -> Result<BranchDeletion> {
        self.require_branch_tip(&candidate.name, &candidate.tip, directory)?;
        if candidate.is_merged && !self.is_merged_into_head(&candidate.tip, directory) {
            return Err(GitError::failed(
                "delete branch",
                format!("{} is no longer merged into the current branch. Refresh and review it again.", candidate.name),
            ));
        }
        let deletion = self.deletion_record(&candidate.name, &candidate.tip, directory);
        let reference = format!("refs/heads/{}", candidate.name);
        self.run(&["update-ref", "-d", &reference, &candidate.tip], directory)?;
        // Git keeps no configuration for a deleted branch; removing it here matches `git branch -d`.
        let _ = self.run(&["config", "--remove-section", &format!("branch.{}", candidate.name)], directory);
        Ok(deletion)
    }

    /// Recreates a deleted branch at its old tip and restores its upstream. Refuses if a branch of
    /// that name exists again, so a reused name is never replaced.
    pub fn restore_deleted_branch(&self, deletion: &BranchDeletion, directory: &Path) -> Result<()> {
        self.run(&["check-ref-format", "--branch", &deletion.name], directory)?;
        let reference = format!("refs/heads/{}", deletion.name);
        // An all-zero old value makes the update fail if the branch exists again.
        let missing = "0".repeat(deletion.tip.len());
        self.run(&["update-ref", "--create-reflog", "-m", RESTORED_MESSAGE, &reference, &deletion.tip, &missing], directory).map_err(
            |_| {
                GitError::failed(
                    "restore branch",
                    format!("A branch named {} exists again, so the deleted one was not restored.", deletion.name),
                )
            },
        )?;
        if let Some(remote) = &deletion.upstream_remote {
            self.run(&["config", &format!("branch.{}.remote", deletion.name), remote], directory)?;
        }
        if let Some(merge) = &deletion.upstream_merge {
            self.run(&["config", &format!("branch.{}.merge", deletion.name), merge], directory)?;
        }
        Ok(())
    }

    fn deletion_record(&self, name: &str, tip: &str, directory: &Path) -> BranchDeletion {
        BranchDeletion {
            name: name.to_string(),
            tip: tip.to_string(),
            upstream_remote: self.config(&format!("branch.{name}.remote"), false, directory).ok().flatten(),
            upstream_merge: self.config(&format!("branch.{name}.merge"), false, directory).ok().flatten(),
        }
    }

    fn is_merged_into_head(&self, commit: &str, directory: &Path) -> bool {
        self.run(&["merge-base", "--is-ancestor", commit, "HEAD"], directory).is_ok()
    }
}

fn now_seconds() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|duration| duration.as_secs() as i64).unwrap_or(0)
}
