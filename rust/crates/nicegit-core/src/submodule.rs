//! Submodules recorded in the superproject, and checking out the commit each one records.

use std::path::Path;

use crate::client::GitClient;
use crate::models::{GitError, Result};

/// Whether a submodule is checked out, and where its HEAD is compared with the recorded commit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SubmoduleState {
    /// Registered in the superproject but not checked out yet.
    NotCheckedOut,
    /// Checked out at the commit the superproject records.
    AtRecordedCommit,
    /// Checked out at a different commit; holds the commit that is checked out.
    OnAnotherCommit(String),
}

/// One submodule as the superproject records it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Submodule {
    /// The path relative to the superproject's root.
    pub path: String,
    /// The commit the superproject records for this submodule.
    pub recorded_commit: String,
    pub state: SubmoduleState,
    /// Uncommitted changes inside the submodule's own working tree.
    pub has_local_changes: bool,
}

impl GitClient {
    /// Lists submodules from the index's gitlinks (mode 160000). Unlike `submodule status`, the
    /// NUL-separated output keeps paths with spaces or newlines unambiguous.
    pub fn submodules(&self, directory: &Path) -> Result<Vec<Submodule>> {
        let root = self.repository_root(directory)?;
        let listing = self.run(&["ls-files", "-z", "--stage"], &root)?;
        let gitlinks: Vec<(String, String)> = listing
            .split('\0')
            .filter_map(|record| {
                let (metadata, path) = record.split_once('\t')?;
                let fields: Vec<&str> = metadata.split_whitespace().collect();
                match fields[..] {
                    ["160000", commit, "0"] => Some((path.to_string(), commit.to_string())),
                    _ => None,
                }
            })
            .collect();

        let mut submodules = Vec::with_capacity(gitlinks.len());
        for (path, recorded_commit) in gitlinks {
            let folder = root.join(&path);
            // Only a folder with its own `.git` is checked out; anything else would resolve to
            // the superproject.
            let head =
                if folder.join(".git").exists() { self.try_trimmed(&["rev-parse", "--verify", "--quiet", "HEAD"], &folder) } else { None };
            let Some(head) = head else {
                submodules.push(Submodule { path, recorded_commit, state: SubmoduleState::NotCheckedOut, has_local_changes: false });
                continue;
            };
            let has_local_changes = self
                .run(&["status", "--porcelain", "--ignore-submodules=none"], &folder)
                .map(|output| !output.trim().is_empty())
                .unwrap_or(false);
            let state = if head == recorded_commit { SubmoduleState::AtRecordedCommit } else { SubmoduleState::OnAnotherCommit(head) };
            submodules.push(Submodule { path, recorded_commit, state, has_local_changes });
        }
        Ok(submodules)
    }

    /// Checks out the commit the superproject records in one submodule, initialising it first if
    /// needed. Git refuses when the checkout would overwrite the submodule's uncommitted changes.
    /// Git's file-transport protection stays in force for any clone this starts.
    pub fn update_submodule(&self, path: &str, directory: &Path) -> Result<()> {
        let root = self.repository_root(directory)?;
        if !self.submodules(&root)?.iter().any(|submodule| submodule.path == path) {
            return Err(GitError::failed("submodule update", "This submodule is no longer registered. Refresh the repository."));
        }
        self.require_finished_operation("submodule update", &root)?;
        // `submodule` runs helper commands with their own pathspecs, so mark this one literal.
        let literal = format!(":(literal){path}");
        self.run(&["submodule", "update", "--init", "--", &literal], &root).map(drop)
    }
}
