//! Submodules recorded in the superproject, and checking out the commit each one records.

use std::path::{Component, Path};

use crate::client::GitClient;
use crate::models::{GitError, Result};
use crate::runner::{self, RunOptions};

/// Why `path` cannot hold a new submodule under `root`, or `None` when it can. The path must be
/// relative, stay inside the work tree, avoid `.git`, and name nothing that already exists.
pub fn submodule_path_problem(root: &Path, path: &str) -> Option<String> {
    if path.trim().is_empty() {
        return Some("Enter the path where the submodule goes.".to_string());
    }
    let relative = Path::new(path);
    let inside = relative.components().all(|component| match component {
        Component::Normal(name) => !name.eq_ignore_ascii_case(".git"),
        _ => false,
    });
    if relative.is_absolute() || relative.has_root() {
        return Some("The path must be relative to the repository.".to_string());
    }
    if !inside {
        return Some("The path must stay inside the repository and cannot contain “..”, “.”, or .git.".to_string());
    }
    if root.join(relative).symlink_metadata().is_ok() {
        return Some(format!("Something already exists at {path}. Choose another path."));
    }
    None
}

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

    /// Initialises and checks out the recorded commit of every submodule, recursively.
    pub fn update_submodules(&self, directory: &Path) -> Result<()> {
        let root = self.repository_root(directory)?;
        self.require_finished_operation("submodule update", &root)?;
        self.run(&["submodule", "update", "--init", "--recursive"], &root).map(drop)
    }

    /// Copies each submodule's configured URL from `.gitmodules` into its checkout's config.
    pub fn sync_submodules(&self, directory: &Path) -> Result<()> {
        let root = self.repository_root(directory)?;
        self.run(&["submodule", "sync", "--recursive"], &root).map(drop)
    }

    /// Adds a submodule cloned from `url` at `path`, optionally tracking `branch`, and stages the
    /// new `.gitmodules` entry and gitlink. Git's file-transport protection applies to the clone.
    pub fn add_submodule(&self, url: &str, path: &str, branch: Option<&str>, directory: &Path) -> Result<()> {
        let root = self.repository_root(directory)?;
        if let Some(problem) = submodule_path_problem(&root, path) {
            return Err(GitError::failed("submodule add", problem));
        }
        if url.trim().is_empty() || url.starts_with('-') {
            return Err(GitError::failed("submodule add", "Enter the repository URL or path of the submodule."));
        }
        if let Some(branch) = branch {
            if branch.starts_with('-') || self.run(&["check-ref-format", &format!("refs/heads/{branch}")], &root).is_err() {
                return Err(GitError::failed("submodule add", format!("“{branch}” is not a valid branch name.")));
            }
        }
        self.require_finished_operation("submodule add", &root)?;
        let mut arguments = vec!["submodule", "add"];
        if let Some(branch) = branch {
            arguments.extend(["-b", branch]);
        }
        arguments.extend(["--", url, path]);
        // `submodule add` stages the path with its own `git add`, so keep glob characters literal.
        let options = RunOptions { env: &[("GIT_LITERAL_PATHSPECS", "1")], ..self.options() };
        runner::run(&arguments, &root, &options).map(drop)
    }

    /// Removes a registered submodule: its checkout is deleted, its `.gitmodules` entry and gitlink
    /// are removed and staged, and its Git data in `.git/modules` is kept. Refused while the
    /// checkout has uncommitted, untracked, or ignored files, which the removal would delete.
    pub fn remove_submodule(&self, path: &str, directory: &Path) -> Result<()> {
        let root = self.repository_root(directory)?;
        let Some(submodule) = self.submodules(&root)?.into_iter().find(|submodule| submodule.path == path) else {
            return Err(GitError::failed("submodule remove", "This submodule is no longer registered. Refresh the repository."));
        };
        self.require_finished_operation("submodule remove", &root)?;
        if submodule.has_local_changes {
            return Err(GitError::failed(
                "submodule remove",
                format!("{path} has uncommitted changes. Commit or discard them before removing it."),
            ));
        }
        let folder = root.join(path);
        if submodule.state != SubmoduleState::NotCheckedOut {
            let ignored = self
                .run(&["status", "--porcelain", "--ignored", "--ignore-submodules=none"], &folder)
                .map(|output| !output.trim().is_empty())
                .unwrap_or(true);
            if ignored {
                return Err(GitError::failed("submodule remove", format!("{path} has ignored files. Remove them or keep the submodule.")));
            }
            // `deinit` runs helper commands with their own pathspecs, so mark this one literal.
            let literal = format!(":(literal){path}");
            self.run(&["submodule", "deinit", "-f", "--", &literal], &root)?;
        }
        // `rm` takes literal pathspecs from the runner, so a glob-like path names only itself.
        self.run(&["rm", "-f", "--", path], &root).map(drop)
    }
}
