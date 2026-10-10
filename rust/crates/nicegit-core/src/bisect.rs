//! Guided bisect: finding the commit that introduced a problem, by testing commits between a
//! known good and a known bad one.

use std::collections::{BTreeSet, HashMap};
use std::path::Path;

use crate::client::GitClient;
use crate::models::{GitError, Result, StatusKind};
use crate::runner;

/// A bisect in progress, as Git's own bisect files and refs describe it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BisectStatus {
    /// The branch or commit Git returns to when the bisect ends.
    pub original_checkout: String,
    pub bad: Option<String>,
    pub good: Vec<String>,
    pub skipped: Vec<String>,
    /// The commit checked out for testing.
    pub testing: Option<String>,
    /// Roughly how many more marks are needed once both a good and a bad commit are known.
    pub remaining_steps: Option<usize>,
    /// The first bad commit, once Git has narrowed the search to one commit.
    pub first_bad: Option<String>,
}

/// A verdict for the commit being tested.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BisectMark {
    Good,
    Bad,
    Skip,
}

impl BisectMark {
    pub fn as_str(self) -> &'static str {
        match self {
            BisectMark::Good => "good",
            BisectMark::Bad => "bad",
            BisectMark::Skip => "skip",
        }
    }
}

/// Paths sent to one `ls-files` call, so a long bisect range cannot exceed the argument limit.
const PATH_CHUNK: usize = 500;

impl GitClient {
    /// The bisect in progress in this checkout, or `None` when no bisect is running.
    pub fn bisect_status(&self, directory: &Path) -> Result<Option<BisectStatus>> {
        let git_directory = self.git_directory(directory)?;
        let start = git_directory.join("BISECT_START");
        if !start.exists() {
            return Ok(None);
        }
        let original = runner::strip_line_terminator(std::fs::read_to_string(&start).unwrap_or_default());

        let mut bad = None;
        let mut good = Vec::new();
        let mut skipped = Vec::new();
        let refs = self.run(&["for-each-ref", "--format=%(refname) %(objectname)", "refs/bisect/"], directory)?;
        for line in refs.lines() {
            // Ref names cannot contain spaces, so the first space separates name from object ID.
            let Some((name, id)) = line.split_once(' ') else { continue };
            if name == "refs/bisect/bad" {
                bad = Some(id.to_string());
            } else if name.starts_with("refs/bisect/good-") {
                good.push(id.to_string());
            } else if name.starts_with("refs/bisect/skip-") {
                skipped.push(id.to_string());
            }
        }

        let testing = self.head(directory);
        let mut remaining_steps = None;
        let mut first_bad = None;
        if let (Some(bad_id), false) = (&bad, good.is_empty()) {
            let mut arguments = vec!["rev-list", "--bisect-vars", bad_id.as_str(), "--not"];
            arguments.extend(good.iter().map(String::as_str));
            arguments.push("--");
            let mut values: HashMap<String, String> = HashMap::new();
            for line in self.run(&arguments, directory)?.lines() {
                if let Some((key, value)) = line.split_once('=') {
                    values.insert(key.to_string(), value.trim_matches('\'').to_string());
                }
            }
            remaining_steps = values.get("bisect_steps").and_then(|steps| steps.parse().ok());
            // A single remaining candidate is the bad commit itself, so it is the first bad one.
            if values.get("bisect_all").map(String::as_str) == Some("1") {
                first_bad = Some(bad_id.clone());
                remaining_steps = Some(0);
            }
        }

        Ok(Some(BisectStatus { original_checkout: original, bad, good, skipped, testing, remaining_steps, first_bad }))
    }

    /// Starts a bisect between a bad and a good commit, checking out the first commit to test.
    /// Refuses unless the checkout is still as shown, no other operation or bisect is running,
    /// and the working tree holds only untracked files, since bisect checks out other commits.
    pub fn start_bisect(&self, bad: &str, good: &str, expected_branch: &str, expected_head: Option<&str>, directory: &Path) -> Result<()> {
        self.require_checkout(expected_branch, expected_head, "bisect", directory)?;
        if self.current_operation(directory)?.is_some() || self.bisect_status(directory)?.is_some() {
            return Err(GitError::failed("bisect", "Finish the current Git operation or bisect first."));
        }
        if self.load_status(directory)?.iter().any(|entry| entry.kind != StatusKind::Untracked) {
            return Err(GitError::failed("bisect", "Commit or stash your changes first; bisect checks out other commits."));
        }
        let bad_id = self.resolve_commit(bad, directory)?;
        let good_id = self.resolve_commit(good, directory)?;
        if bad_id == good_id || self.run(&["merge-base", "--is-ancestor", &good_id, &bad_id], directory).is_err() {
            return Err(GitError::failed("bisect", "The good commit must be an older ancestor of the bad one."));
        }
        self.require_no_ignored_bisect_collisions(&bad_id, &good_id, directory)?;
        self.run(&["bisect", "start", &bad_id, &good_id, "--"], directory).map(drop)
    }

    /// Bisect's checkouts overwrite ignored local files at paths the range changes, and Git
    /// cannot be told otherwise, so refuse while such files exist.
    fn require_no_ignored_bisect_collisions(&self, bad: &str, good: &str, directory: &Path) -> Result<()> {
        let range = format!("{good}..{bad}");
        let mut candidates: BTreeSet<String> = BTreeSet::new();
        for path in self.run(&["log", "--format=", "--name-only", "--no-renames", "-z", &range, "--"], directory)?.split('\0') {
            if !path.is_empty() {
                candidates.insert(path.to_string());
            }
        }
        if let Some(head) = self.head(directory) {
            for path in self.run(&["diff", "--name-only", "--no-renames", "-z", &head, bad, "--"], directory)?.split('\0') {
                if !path.is_empty() {
                    candidates.insert(path.to_string());
                }
            }
        }
        let candidates: Vec<String> = candidates.into_iter().collect();
        for chunk in candidates.chunks(PATH_CHUNK) {
            let mut arguments = vec!["ls-files", "--others", "--ignored", "--exclude-standard", "-z", "--"];
            arguments.extend(chunk.iter().map(String::as_str));
            if !self.run(&arguments, directory)?.is_empty() {
                return Err(GitError::failed(
                    "bisect",
                    "Ignored local files would be overwritten while bisecting. Move or back them up before starting.",
                ));
            }
        }
        Ok(())
    }

    /// Marks the commit being tested. Refuses when the checkout no longer shows `expected_testing`,
    /// so a stale window cannot mark a different commit. Returns Git's progress message.
    pub fn mark_bisect(&self, mark: BisectMark, expected_testing: &str, directory: &Path) -> Result<String> {
        if self.bisect_status(directory)?.is_none() {
            return Err(GitError::failed("bisect", "No bisect is in progress."));
        }
        if self.head(directory).as_deref() != Some(expected_testing) {
            return Err(GitError::failed("bisect", "The tested commit changed since it was shown. Refresh and review it again."));
        }
        let commit = self.resolve_commit(expected_testing, directory)?;
        self.run(&["bisect", mark.as_str(), &commit], directory).map(|output| output.trim().to_string())
    }

    /// Ends the bisect and returns to the checkout it started from.
    pub fn end_bisect(&self, directory: &Path) -> Result<()> {
        if self.bisect_status(directory)?.is_none() {
            return Ok(());
        }
        self.run(&["bisect", "reset"], directory).map(drop)
    }
}
