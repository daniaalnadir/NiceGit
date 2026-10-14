//! Comparing a branch with the current branch: the commits that only one of them has, and the
//! merge base they share.

use std::path::Path;

use crate::client::GitClient;
use crate::models::{GitError, Result};

/// How many commits are listed for each side. A longer side sets its `more_*` flag instead.
const COMPARE_LIMIT: usize = 200;

/// Hash, subject, author, and relative date, separated by unit separators and ended by NUL.
const COMMIT_FORMAT: &str = "--format=%H%x1f%s%x1f%an%x1f%ar%x00";

/// What the current branch and another branch each have that the other does not.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BranchComparison {
    /// The merge base's object ID, or `None` when the two branches share no history.
    pub base: Option<String>,
    /// Commits on the other branch that the current branch lacks, newest first.
    pub only_in_other: Vec<ComparedCommit>,
    /// Commits on the current branch that the other branch lacks, newest first.
    pub only_in_current: Vec<ComparedCommit>,
    /// Whether the other branch has more commits than `only_in_other` lists.
    pub more_in_other: bool,
    /// Whether the current branch has more commits than `only_in_current` lists.
    pub more_in_current: bool,
}

/// One commit in a branch comparison.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComparedCommit {
    pub hash: String,
    pub subject: String,
    pub author: String,
    pub relative_date: String,
}

impl GitClient {
    /// Compares the current branch with `other_tip`, the other branch's tip as it was displayed.
    /// Fails on a repository without commits.
    pub fn compare_branch(&self, other_tip: &str, directory: &Path) -> Result<BranchComparison> {
        self.compare_branch_with_limit(other_tip, directory, COMPARE_LIMIT)
    }

    /// `compare_branch`, listing at most `limit` commits on each side.
    pub fn compare_branch_with_limit(&self, other_tip: &str, directory: &Path, limit: usize) -> Result<BranchComparison> {
        let head =
            self.head(directory).ok_or_else(|| GitError::failed("compare branches", "There are no commits on the current branch yet."))?;
        let other = self.resolve_commit(other_tip, directory)?;
        // Git exits with 1 when the two share no history, which leaves the output empty.
        let base = self.run_accepting(&["merge-base", &head, &other], directory, &[0, 1])?.trim().to_string();
        let (only_in_other, more_in_other) = self.commits_missing_from(&other, &head, limit, directory)?;
        let (only_in_current, more_in_current) = self.commits_missing_from(&head, &other, limit, directory)?;
        Ok(BranchComparison { base: (!base.is_empty()).then_some(base), only_in_other, only_in_current, more_in_other, more_in_current })
    }

    /// The commits reachable from `commit` but not from `without`, newest first, up to `limit`,
    /// and whether there were more. Both sides are object IDs, so the range cannot be an option.
    fn commits_missing_from(&self, commit: &str, without: &str, limit: usize, directory: &Path) -> Result<(Vec<ComparedCommit>, bool)> {
        let count = limit.saturating_add(1).to_string();
        let range = format!("{without}..{commit}");
        let output = self.run(&["log", "--no-color", "-n", &count, COMMIT_FORMAT, "--end-of-options", &range, "--"], directory)?;
        Ok(parse_commits(&output, limit))
    }
}

/// Parses the records written with `COMMIT_FORMAT`, keeping at most `limit` of them. The
/// returned flag says whether the output held more.
fn parse_commits(output: &str, limit: usize) -> (Vec<ComparedCommit>, bool) {
    let mut commits: Vec<ComparedCommit> = output
        .split('\0')
        .filter_map(|record| {
            let record = record.trim_matches(|c: char| c == '\n' || c == '\r');
            let fields: Vec<&str> = record.splitn(4, '\u{1f}').collect();
            if fields.len() != 4 || fields[0].is_empty() {
                return None;
            }
            Some(ComparedCommit {
                hash: fields[0].to_string(),
                subject: fields[1].to_string(),
                author: fields[2].to_string(),
                relative_date: fields[3].to_string(),
            })
        })
        .collect();
    let more = commits.len() > limit;
    commits.truncate(limit);
    (commits, more)
}
