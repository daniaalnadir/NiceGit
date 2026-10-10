//! The reflog: every position HEAD has had. Commits left behind by resets, rebases, amends, or
//! deleted branches stay reachable here until Git expires them.

use std::collections::HashSet;
use std::path::Path;

use crate::client::GitClient;
use crate::models::{GitError, Result};

const REFLOG_FORMAT: &str = "--format=%H%x1f%gd%x1f%gs%x1f%s%x1f%ct%x1e";

/// One position HEAD has pointed to, newest first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReflogEntry {
    pub hash: String,
    /// Git's selector for this entry, such as `HEAD@{3}`.
    pub selector: String,
    /// What moved HEAD, such as `commit: Fix typo` or `reset: moving to HEAD~1`.
    pub action: String,
    /// The commit's subject.
    pub subject: String,
    /// Committer time of the commit, in seconds since the Unix epoch.
    pub time: Option<i64>,
}

impl GitClient {
    /// HEAD's reflog entries, newest first, up to `limit`. Empty before the first commit.
    pub fn reflog(&self, limit: usize, directory: &Path) -> Result<Vec<ReflogEntry>> {
        if self.head(directory).is_none() {
            return Ok(Vec::new());
        }
        let count = limit.max(1).to_string();
        let output = self.run(&["log", "--walk-reflogs", "--no-color", "-n", &count, REFLOG_FORMAT, "HEAD", "--"], directory)?;
        Ok(parse_reflog(&output))
    }

    /// The commits among `hashes` that no branch, tag, or remote-tracking branch contains. Only
    /// the reflog keeps these; Git may delete them once it expires.
    pub fn unreachable_commits(&self, hashes: &[String], directory: &Path) -> Result<HashSet<String>> {
        let mut unique: Vec<&str> = hashes
            .iter()
            .map(String::as_str)
            .filter(|hash| !hash.is_empty() && hash.bytes().all(|byte| byte.is_ascii_hexdigit()))
            .collect();
        unique.sort_unstable();
        unique.dedup();
        if unique.is_empty() {
            return Ok(HashSet::new());
        }
        let mut arguments = vec!["rev-list", "--no-walk=unsorted"];
        arguments.extend(unique);
        arguments.extend(["--not", "--all", "--"]);
        let output = self.run(&arguments, directory)?;
        Ok(output.lines().map(str::to_string).collect())
    }

    /// Creates a branch at a commit without checking it out. Git refuses a name that is taken.
    pub fn create_branch_at(&self, name: &str, commit: &str, directory: &Path) -> Result<()> {
        let name = name.trim();
        if name.is_empty() {
            return Err(GitError::EmptyBranchName);
        }
        self.run(&["check-ref-format", "--branch", name], directory)?;
        let target = self.resolve_commit(commit, directory)?;
        self.run(&["branch", "--no-track", "--", name, &target], directory).map(drop)
    }
}

/// Parses the records written with `REFLOG_FORMAT`; fields are separated by unit separators and
/// records by record separators, so subjects may contain anything except those control characters.
fn parse_reflog(output: &str) -> Vec<ReflogEntry> {
    output
        .split('\u{1e}')
        .filter_map(|record| {
            let record = record.trim_matches(|c: char| c == '\n' || c == '\r');
            let fields: Vec<&str> = record.split('\u{1f}').collect();
            if fields.len() != 5 || fields[0].is_empty() {
                return None;
            }
            Some(ReflogEntry {
                hash: fields[0].to_string(),
                selector: fields[1].to_string(),
                action: fields[2].to_string(),
                subject: fields[3].to_string(),
                time: fields[4].trim().parse().ok(),
            })
        })
        .collect()
}
