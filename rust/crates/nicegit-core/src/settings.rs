//! Repository identity, remotes, tags on remotes, upstreams, ignore rules, and stashes of
//! selected files.

use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

use crate::client::GitClient;
use crate::models::{GitError, Result, StatusEntry, StatusKind};
use crate::runner;

/// A commit identity saved for reuse. A profile with a signing key also turns on commit signing
/// when it is applied.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IdentityProfile {
    pub name: String,
    pub email: String,
    pub signing_key: Option<String>,
    pub label: String,
}

impl IdentityProfile {
    /// A profile labelled "Name <email>".
    pub fn new(name: &str, email: &str, signing_key: Option<&str>) -> Self {
        let name = name.trim().to_string();
        let email = email.trim().to_string();
        let label = format!("{name} <{email}>");
        let signing_key = signing_key.map(str::trim).filter(|key| !key.is_empty()).map(str::to_string);
        Self { name, email, signing_key, label }
    }
}

/// Which ignore rule to write for an untracked path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IgnoreRule {
    /// Only the selected path, anchored at the repository root.
    Path,
    /// Every file with the selected file's extension, in any folder.
    FileExtension,
}

/// Where an ignore rule is written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IgnoreScope {
    /// `.gitignore` at the repository root, shared when committed.
    Shared,
    /// `info/exclude` in the Git directory, which stays on this computer.
    Local,
}

/// The addresses one remote was shown with, in the shape `require_remote_addresses` compares.
fn expected_addresses_for(remote: &str, addresses: &[String]) -> BTreeMap<String, Vec<String>> {
    BTreeMap::from([(remote.to_string(), addresses.to_vec())])
}

fn io_failure(command: &str, error: std::io::Error) -> GitError {
    GitError::failed(command, error.to_string())
}

/// The ignore pattern for `path`, with wildcard and comment characters escaped so the name is
/// matched literally. Returns `None` for a name no ignore file can express.
pub fn ignore_pattern(path: &str, rule: IgnoreRule) -> Option<String> {
    if path.is_empty() || path.contains(['\n', '\r']) {
        return None;
    }
    match rule {
        IgnoreRule::Path => {
            let is_folder = path.ends_with('/');
            let body = escape_pattern(if is_folder { &path[..path.len() - 1] } else { path });
            Some(format!("/{body}{}", if is_folder { "/" } else { "" }))
        }
        IgnoreRule::FileExtension => {
            if path.ends_with('/') {
                return None;
            }
            let name = path.rsplit('/').next().unwrap_or(path);
            let dot = name.rfind('.')?;
            if dot == 0 || dot + 1 == name.len() {
                return None;
            }
            Some(format!("*{}", escape_pattern(&name[dot..])))
        }
    }
}

fn escape_pattern(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        if "\\*?[".contains(character) {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    // Git trims unescaped trailing spaces from patterns, so each one is escaped instead.
    let trimmed_len = escaped.trim_end_matches(' ').len();
    let trailing = escaped.len() - trimmed_len;
    let mut result = escaped[..trimmed_len].to_string();
    for _ in 0..trailing {
        result.push_str("\\ ");
    }
    result
}

impl GitClient {
    // MARK: Identity

    /// Sets the commit name and email for this repository only.
    pub fn set_identity(&self, name: &str, email: &str, directory: &Path) -> Result<()> {
        let name = name.trim();
        let email = email.trim();
        if name.is_empty() || email.is_empty() {
            return Err(GitError::failed("config", "Name and email are required."));
        }
        self.run(&["config", "--local", "--", "user.name", name], directory)?;
        self.run(&["config", "--local", "--", "user.email", email], directory)?;
        Ok(())
    }

    /// Applies a commit identity to this repository only. With a signing key, commits are also
    /// signed with it; without one, the repository's signing settings are left unchanged.
    pub fn apply_identity(&self, name: &str, email: &str, signing_key: Option<&str>, directory: &Path) -> Result<()> {
        self.set_identity(name, email, directory)?;
        if let Some(key) = signing_key.map(str::trim).filter(|key| !key.is_empty()) {
            self.run(&["config", "--local", "--", "user.signingkey", key], directory)?;
            self.run(&["config", "--local", "--", "commit.gpgsign", "true"], directory)?;
        }
        Ok(())
    }

    // MARK: Remotes

    /// The remote's first fetch address.
    pub fn remote_address(&self, name: &str, directory: &Path) -> Result<String> {
        self.run_trimmed(&["remote", "get-url", "--", name], directory)
    }

    /// Renames a remote, if its fetch addresses still match what was shown. Git also moves its
    /// remote-tracking branches and branch upstream settings.
    pub fn rename_remote(&self, name: &str, new_name: &str, expected_addresses: &[String], directory: &Path) -> Result<()> {
        let new_name = new_name.trim();
        self.require_remote_addresses(&expected_addresses_for(name, expected_addresses), name, false, "remote rename", directory)?;
        self.run(&["remote", "rename", "--", name, new_name], directory).map(drop)
    }

    /// Removes a remote, if its fetch addresses still match what was shown. Its remote-tracking
    /// branches and the upstream settings that use it are removed too; the server is unchanged.
    pub fn remove_remote(&self, name: &str, expected_addresses: &[String], directory: &Path) -> Result<()> {
        self.require_remote_addresses(&expected_addresses_for(name, expected_addresses), name, false, "remote remove", directory)?;
        self.run(&["remote", "remove", "--", name], directory).map(drop)
    }

    /// Changes a remote's fetch address, if its addresses still match what was shown. Separately
    /// configured push addresses are left as they are.
    pub fn set_remote_address(&self, name: &str, address: &str, expected_addresses: &[String], directory: &Path) -> Result<()> {
        let address = address.trim();
        if address.is_empty() {
            return Err(GitError::failed("remote set-url", "Enter the remote's new address."));
        }
        self.require_remote_addresses(&expected_addresses_for(name, expected_addresses), name, false, "remote set-url", directory)?;
        self.run(&["remote", "set-url", "--", name, address], directory).map(drop)
    }

    // MARK: Tags on remotes

    /// Confirms a local tag still points to the object shown, returning its full ref name.
    fn require_tag_tip(&self, name: &str, expected_tip: &str, command: &str, directory: &Path) -> Result<String> {
        let reference = format!("refs/tags/{name}");
        let current = self.run_trimmed(&["rev-parse", "--verify", "--end-of-options", &reference], directory).ok();
        if current.as_deref() != Some(expected_tip) {
            return Err(GitError::failed(command, "This tag changed since it was selected. Refresh and review it again."));
        }
        Ok(reference)
    }

    /// Pushes one tag to a remote. An existing tag of the same name on the remote is not moved.
    pub fn push_tag(
        &self,
        name: &str,
        remote: &str,
        expected_tip: &str,
        expected_push_addresses: &[String],
        directory: &Path,
    ) -> Result<()> {
        let reference = self.require_tag_tip(name, expected_tip, "push tag", directory)?;
        self.require_remote_addresses(&expected_addresses_for(remote, expected_push_addresses), remote, true, "push tag", directory)?;
        let mirror = format!("remote.{remote}.mirror=false");
        let refspec = format!("{reference}:{reference}");
        self.run(&["-c", &mirror, "push", "--no-follow-tags", "--recurse-submodules=no", "--", remote, &refspec], directory).map(drop)
    }

    /// Deletes a tag from a remote, but only while the remote still has the object shown locally.
    pub fn delete_remote_tag(
        &self,
        name: &str,
        remote: &str,
        expected_tip: &str,
        expected_push_addresses: &[String],
        directory: &Path,
    ) -> Result<()> {
        let reference = self.require_tag_tip(name, expected_tip, "delete remote tag", directory)?;
        self.require_remote_addresses(
            &expected_addresses_for(remote, expected_push_addresses),
            remote,
            true,
            "delete remote tag",
            directory,
        )?;
        let mirror = format!("remote.{remote}.mirror=false");
        let lease = format!("--force-with-lease={reference}:{expected_tip}");
        let deletion = format!(":{reference}");
        self.run(&["-c", &mirror, "push", "--no-follow-tags", "--recurse-submodules=no", &lease, "--", remote, &deletion], directory)
            .map(drop)
    }

    // MARK: Upstream

    /// Sets a branch's upstream to a remote-tracking branch such as `origin/main`, or clears it
    /// when `remote_branch` is `None`. The branch must still point to `expected_tip`.
    pub fn set_upstream(&self, branch: &str, remote_branch: Option<&str>, expected_tip: &str, directory: &Path) -> Result<()> {
        self.require_branch_tip(branch, expected_tip, directory)?;
        match remote_branch {
            Some(remote_branch) => {
                let reference = format!("refs/remotes/{remote_branch}");
                self.run(&["show-ref", "--verify", "--quiet", &reference], directory)
                    .map_err(|_| GitError::failed("set upstream", "That remote branch no longer exists. Refresh and choose it again."))?;
                self.run(&["branch", &format!("--set-upstream-to={reference}"), "--", branch], directory).map(drop)
            }
            None => self.run(&["branch", "--unset-upstream", "--", branch], directory).map(drop),
        }
    }

    // MARK: Stashes of selected files

    /// Stashes only `paths`, staged and unstaged, including selected untracked files. Every other
    /// file is left exactly as it was. Git's own stash cannot take literal pathspecs from the
    /// environment, so each path is marked `:(literal)` here.
    pub fn save_stash_paths(&self, paths: &[String], message: &str, directory: &Path) -> Result<()> {
        if paths.is_empty() {
            return Err(GitError::failed("stash", "Select files to stash."));
        }
        self.require_finished_operation("stash", directory)?;
        let before = self.load_status(directory)?;
        if paths.iter().any(|path| !before.iter().any(|entry| &entry.path == path)) {
            return Err(GitError::failed("stash", "Some selected files changed since they were selected. Refresh and review them again."));
        }
        // A rename's old path must go with its new path, or the stash would split the rename.
        let mut selected: Vec<String> = paths.to_vec();
        for entry in before.iter().filter(|entry| paths.contains(&entry.path) && entry.kind == StatusKind::Renamed) {
            if let Some(original) = &entry.original_path {
                if !selected.contains(original) {
                    selected.push(original.clone());
                }
            }
        }
        let previous = self.try_trimmed(&["rev-parse", "--verify", "--quiet", "refs/stash"], directory);
        let message = message.trim();
        let message = if message.is_empty() { format!("WIP on {}", self.current_branch(directory)) } else { message.to_string() };
        let pathspecs: Vec<String> = selected.iter().map(|path| format!(":(literal){path}")).collect();
        let mut arguments = vec!["stash", "push", "--include-untracked", "-m", message.as_str(), "--"];
        arguments.extend(pathspecs.iter().map(String::as_str));
        self.run(&arguments, directory)?;

        let saved = self.try_trimmed(&["rev-parse", "--verify", "--quiet", "refs/stash"], directory);
        let Some(saved) = saved.filter(|saved| Some(saved) != previous.as_ref()) else {
            return Err(GitError::failed("stash", "Git did not save any changes for the selected files."));
        };
        let after = self.load_status(directory)?;
        if after.iter().any(|entry| selected.contains(&entry.path)) {
            return Err(GitError::failed(
                "stash",
                format!(
                    "Stash {} was saved, but some selected files still have changes. Check submodules before proceeding.",
                    short_hash(&saved)
                ),
            ));
        }
        let others = |entries: &[StatusEntry]| entries.iter().filter(|entry| !selected.contains(&entry.path)).cloned().collect::<Vec<_>>();
        if others(&after) != others(&before) {
            return Err(GitError::failed(
                "stash",
                format!(
                    "Stash {} was saved, but other files changed too. Review the working tree and the stash before continuing.",
                    short_hash(&saved)
                ),
            ));
        }
        Ok(())
    }

    // MARK: Ignore rules

    /// Whether Git currently ignores `path`.
    fn is_ignored(&self, path: &str, directory: &Path) -> bool {
        // `check-ignore` takes plain pathnames and rejects literal pathspec magic.
        self.run(&["check-ignore", "--quiet", "--no-index", "--", path], directory).is_ok()
    }

    /// Adds an ignore rule for an untracked path and confirms Git now ignores it. A path that is
    /// already ignored is left as it is.
    pub fn ignore(&self, path: &str, rule: IgnoreRule, scope: IgnoreScope, directory: &Path) -> Result<()> {
        let pattern = ignore_pattern(path, rule).ok_or_else(|| {
            GitError::failed(
                "ignore",
                match rule {
                    IgnoreRule::FileExtension => "This file has no extension to ignore.",
                    IgnoreRule::Path => "This name contains a line break, which ignore files cannot express.",
                },
            )
        })?;
        let untracked = self.load_status(directory)?.iter().any(|entry| entry.path == path && entry.kind == StatusKind::Untracked);
        if !untracked {
            let tracked = self.run(&["ls-files", "-z", "--", path], directory)?;
            if tracked.is_empty() && self.is_ignored(path, directory) {
                return Ok(());
            }
            return Err(GitError::failed("ignore", "Only untracked files can be ignored. Refresh and review this file again."));
        }
        let file = match scope {
            IgnoreScope::Shared => self.repository_root(directory)?.join(".gitignore"),
            IgnoreScope::Local => {
                let location = runner::strip_line_terminator(self.run(&["rev-parse", "--git-path", "info/exclude"], directory)?);
                let location = PathBuf::from(location);
                if location.is_absolute() {
                    location
                } else {
                    directory.join(location)
                }
            }
        };
        if let Some(parent) = file.parent() {
            std::fs::create_dir_all(parent).map_err(|error| io_failure("ignore", error))?;
        }
        let existing = match std::fs::read(&file) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == ErrorKind::NotFound => Vec::new(),
            Err(error) => return Err(io_failure("ignore", error)),
        };
        let lines = String::from_utf8_lossy(&existing);
        if !lines.split('\n').any(|line| line == pattern) {
            let mut addition = String::new();
            if !existing.is_empty() && !existing.ends_with(b"\n") {
                addition.push('\n');
            }
            addition.push_str(&pattern);
            addition.push('\n');
            let mut handle = OpenOptions::new().create(true).append(true).open(&file).map_err(|error| io_failure("ignore", error))?;
            handle.write_all(addition.as_bytes()).map_err(|error| io_failure("ignore", error))?;
        }
        if !self.is_ignored(path, directory) {
            return Err(GitError::failed(
                "ignore",
                "The rule was added, but Git still does not ignore this path. A later negated rule may re-include it.",
            ));
        }
        Ok(())
    }
}

fn short_hash(hash: &str) -> &str {
    &hash[..hash.len().min(12)]
}
