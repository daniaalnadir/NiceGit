//! Creating, removing, and pruning linked worktrees.

use std::path::{Path, PathBuf};

use crate::client::GitClient;
use crate::models::{GitError, Result};
use crate::parsers::parse_worktrees;
use crate::runner;

/// Whether two paths name the same folder. Folders are compared by their resolved location
/// where possible, since Git may report a path through a symbolic link.
fn same_folder(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

impl GitClient {
    /// Creates a linked worktree at `destination` with `branch` checked out. The branch must
    /// still point to `expected_tip`, checked here just before Git creates the worktree.
    pub fn create_worktree(&self, branch: &str, expected_tip: &str, destination: &Path, directory: &Path) -> Result<()> {
        self.require_branch_tip(branch, expected_tip, directory)?;
        let destination =
            destination.to_str().ok_or_else(|| GitError::failed("worktree add", "Choose a folder whose name is valid text."))?;
        self.run(&["worktree", "add", "--", destination, branch], directory).map(drop)
    }

    /// The checkout's own Git directory and, for a linked worktree, the shared one.
    pub fn git_directories(&self, directory: &Path) -> Result<Vec<PathBuf>> {
        let own = self.git_directory(directory)?;
        let common = PathBuf::from(runner::strip_line_terminator(
            self.run(&["rev-parse", "--path-format=absolute", "--git-common-dir"], directory)?,
        ));
        let mut directories = vec![own];
        if !directories.contains(&common) {
            directories.push(common);
        }
        Ok(directories)
    }

    /// Removes a linked worktree's folder and registration. Git refuses while the worktree has
    /// uncommitted or untracked files, or is locked. The main worktree and the checkout open
    /// here are never removed.
    pub fn remove_worktree(&self, path: &str, directory: &Path) -> Result<()> {
        let listing = self.run(&["worktree", "list", "--porcelain", "-z"], directory)?;
        let Some(index) = parse_worktrees(&listing).iter().position(|worktree| worktree.path == path) else {
            return Err(GitError::failed("worktree remove", "This worktree is no longer listed. Refresh the repository."));
        };
        let current = self.repository_root(directory)?;
        if index == 0 || same_folder(Path::new(path), &current) {
            return Err(GitError::failed("worktree remove", "The main worktree and the checkout you have open cannot be removed."));
        }
        self.run(&["worktree", "remove", "--", path], directory).map(drop)
    }

    /// Forgets worktrees whose folders were deleted outside Git.
    pub fn prune_worktrees(&self, directory: &Path) -> Result<()> {
        self.run(&["worktree", "prune"], directory).map(drop)
    }
}
