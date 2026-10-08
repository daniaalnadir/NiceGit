//! Undo and redo for NiceGit's own actions: branch moves (commits, resets, rebases), branch
//! deletions, and discards. Port of `GitUndo.swift` and `GitDiscardUndo.swift`.

use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::Path;

use crate::client::GitClient;
use crate::models::{Branch, GitError, ResetMode, Result, Snapshot, StatusEntry, StatusKind};
use crate::rebase::{io_failure, path_text, Scratch};
use crate::runner;

/// How a branch move is reversed. A commit undoes softly so its changes stay staged; soft and
/// mixed resets reverse in kind; everything else uses `reset --keep`, which keeps unrelated
/// local edits and refuses to overwrite any.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UndoMode {
    Soft,
    Mixed,
    Keep,
}

impl UndoMode {
    /// The mode that reverses a reset made with `mode`.
    pub fn for_reset(mode: ResetMode) -> Self {
        match mode {
            ResetMode::Soft => UndoMode::Soft,
            ResetMode::Mixed => UndoMode::Mixed,
            ResetMode::Hard => UndoMode::Keep,
        }
    }
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

/// One undoable action. Applying it with [`GitClient::undo_step`] returns the inverse, which
/// redoes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UndoStep {
    /// The branch moved from `before` to `after`.
    BranchMove { branch: String, before: String, after: String, mode: UndoMode },
    /// Local branches were deleted. Undoing restores them at their old tips.
    BranchDeletions(Vec<BranchDeletion>),
    /// Branches were restored by an undo. Redoing deletes them again, while they still point to
    /// the tips they were restored at.
    BranchRestorations(Vec<BranchDeletion>),
}

/// The index entry or working file for a path, as Git records it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileVersion {
    /// Git's mode: `100644`, `100755`, or `120000` for a symbolic link.
    pub mode: String,
    pub blob: String,
}

/// What a discarded path looked like before and after the discard, so the discard can be
/// reversed while nothing else has touched the path. File contents are kept as Git objects.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscardUndo {
    pub path: String,
    pub index_before: Option<FileVersion>,
    pub working_before: Option<FileVersion>,
    pub index_after: Option<FileVersion>,
    pub working_after: Option<FileVersion>,
}

/// The undo step for an action that moved the current branch, from the snapshots taken before
/// and after it. Returns `None` when nothing moved, the checkout is detached, or a Git operation
/// is in progress, since an action that stopped partway must not be offered for undo.
pub fn branch_move_step(before: &Snapshot, after: &Snapshot, mode: UndoMode) -> Option<UndoStep> {
    let previous = before.head_hash.as_ref()?;
    let current = after.head_hash.as_ref()?;
    let moved = before.operation.is_none()
        && after.operation.is_none()
        && before.root_path == after.root_path
        && before.current_branch == after.current_branch
        && after.is_on_branch()
        && previous != current;
    moved.then(|| UndoStep::BranchMove { branch: after.current_branch.clone(), before: previous.clone(), after: current.clone(), mode })
}

impl GitClient {
    /// Reverses `step`, returning the inverse step, which redoes it. Refuses when the repository
    /// no longer matches the state the step recorded, so newer work is never overwritten.
    pub fn undo_step(&self, step: &UndoStep, directory: &Path) -> Result<UndoStep> {
        match step {
            UndoStep::BranchMove { branch, before, after, mode } => {
                self.move_branch(branch, after, before, *mode, directory)?;
                Ok(UndoStep::BranchMove { branch: branch.clone(), before: after.clone(), after: before.clone(), mode: *mode })
            }
            UndoStep::BranchDeletions(deletions) => {
                self.restore_branches(deletions, directory)?;
                Ok(UndoStep::BranchRestorations(deletions.clone()))
            }
            UndoStep::BranchRestorations(deletions) => {
                self.delete_restored_branches(deletions, directory)?;
                Ok(UndoStep::BranchDeletions(deletions.clone()))
            }
        }
    }

    /// Moves `branch` from `from` to `to`, while it is checked out and `from` is its tip.
    fn move_branch(&self, branch: &str, from: &str, to: &str, mode: UndoMode, directory: &Path) -> Result<()> {
        match mode {
            UndoMode::Soft => self.reset(to, ResetMode::Soft, branch, from, directory),
            UndoMode::Mixed => self.reset(to, ResetMode::Mixed, branch, from, directory),
            UndoMode::Keep => self.move_branch_keeping_changes(to, from, branch, directory),
        }
    }

    /// Moves the current branch to `target` as `git reset --keep` does: working files follow,
    /// unrelated local edits are kept, and Git refuses if an edit would be overwritten.
    pub fn move_branch_keeping_changes(&self, target: &str, expected_head: &str, expected_branch: &str, directory: &Path) -> Result<()> {
        let state = self.checkout_state(directory)?;
        if state.operation.is_some() || state.head_hash.as_deref() != Some(expected_head) || state.current_branch != expected_branch {
            return Err(GitError::failed(
                "undo",
                "The checkout changed or a Git operation is in progress, so this can no longer be undone safely.",
            ));
        }
        let hash = self.resolve_commit(target, directory)?;
        self.run(&["reset", "--keep", &hash, "--"], directory).map(drop)
    }

    /// Deletes a local branch like `delete_branch`, first saving its tip and upstream settings.
    /// With `force`, an unmerged branch is deleted too, atomically against the tip shown.
    pub fn delete_branch_keeping_undo(&self, branch: &Branch, force: bool, directory: &Path) -> Result<BranchDeletion> {
        let deletion = BranchDeletion {
            name: branch.name.clone(),
            tip: branch.tip.clone(),
            upstream_remote: self.config(&format!("branch.{}.remote", branch.name), false, directory)?,
            upstream_merge: self.config(&format!("branch.{}.merge", branch.name), false, directory)?,
        };
        self.delete_branch(branch, force, directory)?;
        Ok(deletion)
    }

    /// Recreates a deleted branch at its old tip. The all-zero old value makes the update fail
    /// if a branch of that name exists again, so a reused name is never replaced.
    fn restore_branch(&self, deletion: &BranchDeletion, directory: &Path) -> Result<()> {
        self.run(&["check-ref-format", "--branch", &deletion.name], directory)?;
        let reference = format!("refs/heads/{}", deletion.name);
        let missing = "0".repeat(deletion.tip.len());
        if let Err(error) = self
            .run(&["update-ref", "--create-reflog", "-m", "branch: restored by NiceGit", &reference, &deletion.tip, &missing], directory)
        {
            if self.try_trimmed(&["rev-parse", "--verify", "--quiet", &reference], directory).is_some() {
                return Err(GitError::failed(
                    "restore branch",
                    format!("A branch named {} exists again, so the deleted one was not restored.", deletion.name),
                ));
            }
            return Err(error);
        }
        if let Some(remote) = &deletion.upstream_remote {
            self.run(&["config", &format!("branch.{}.remote", deletion.name), remote], directory)?;
        }
        if let Some(merge) = &deletion.upstream_merge {
            self.run(&["config", &format!("branch.{}.merge", deletion.name), merge], directory)?;
        }
        Ok(())
    }

    /// Restores every branch of an undo, or none of them: a failure removes those already restored.
    fn restore_branches(&self, deletions: &[BranchDeletion], directory: &Path) -> Result<()> {
        for (index, deletion) in deletions.iter().enumerate() {
            if let Err(error) = self.restore_branch(deletion, directory) {
                for restored in &deletions[..index] {
                    let _ = self.delete_restored_branch(restored, directory);
                }
                return Err(error);
            }
        }
        Ok(())
    }

    /// Deletes a branch again after it was restored, only while it still points to the same commit.
    fn delete_restored_branch(&self, deletion: &BranchDeletion, directory: &Path) -> Result<()> {
        self.run(&["update-ref", "-d", &format!("refs/heads/{}", deletion.name), &deletion.tip], directory).map_err(|_| {
            GitError::failed(
                "redo branch deletion",
                format!("Branch {} changed after it was restored, so it was not deleted.", deletion.name),
            )
        })?;
        let _ = self.run(&["config", "--remove-section", &format!("branch.{}", deletion.name)], directory);
        Ok(())
    }

    /// Deletes every branch of a redo, or none of them: a failure restores those already deleted.
    fn delete_restored_branches(&self, deletions: &[BranchDeletion], directory: &Path) -> Result<()> {
        for (index, deletion) in deletions.iter().enumerate() {
            if let Err(error) = self.delete_restored_branch(deletion, directory) {
                for deleted in &deletions[..index] {
                    let _ = self.restore_branch(deleted, directory);
                }
                return Err(error);
            }
        }
        Ok(())
    }

    /// Discards `entry` like `discard`, first saving what is needed to undo it. Returns `None`
    /// (after still discarding) for paths that cannot be restored exactly: renames, copies,
    /// conflicts, submodules, and folders.
    pub fn discard_keeping_undo(&self, entry: &StatusEntry, directory: &Path) -> Result<Option<DiscardUndo>> {
        let supported = entry.original_path.is_none()
            && entry.kind != StatusKind::Conflicted
            && entry.kind != StatusKind::Renamed
            && !entry.path.ends_with('/');
        // Save both versions of the file before discarding. A failure to save means no undo.
        let before = if supported {
            match (self.index_version(&entry.path, directory), self.working_version(&entry.path, directory, true)) {
                (Ok(index), Ok(working)) => Some((index, working)),
                _ => None,
            }
        } else {
            None
        };
        self.discard(entry, directory)?;
        let Some((index_before, working_before)) = before else { return Ok(None) };
        // A submodule's index entry is a commit, not a file, so it cannot be restored here.
        if index_before.as_ref().is_some_and(|version| version.mode == "160000") || (index_before.is_none() && working_before.is_none()) {
            return Ok(None);
        }
        let after = match (self.index_version(&entry.path, directory), self.working_version(&entry.path, directory, false)) {
            (Ok(index), Ok(working)) => (index, working),
            _ => return Ok(None),
        };
        Ok(Some(DiscardUndo { path: entry.path.clone(), index_before, working_before, index_after: after.0, working_after: after.1 }))
    }

    /// Puts back the staged and working versions a discard removed. Refuses if the path has
    /// changed since the discard, so newer work is never overwritten.
    pub fn undo_discard(&self, undo: &DiscardUndo, directory: &Path) -> Result<()> {
        const COMMAND: &str = "undo discard";
        let index_now = self.index_version(&undo.path, directory)?;
        let working_now = self.working_version(&undo.path, directory, false);
        if index_now != undo.index_after || working_now.as_ref().ok() != Some(&undo.working_after) {
            return Err(GitError::failed(
                COMMAND,
                format!("{} changed after it was discarded, so the discard cannot be undone safely.", undo.path),
            ));
        }
        let file = directory.join(&undo.path);
        match &undo.working_before {
            Some(version) => {
                let data = self.blob_contents(&version.blob, directory)?;
                if let Some(parent) = file.parent() {
                    fs::create_dir_all(parent).map_err(|error| io_failure(COMMAND, error))?;
                }
                remove_existing(&file).map_err(|error| io_failure(COMMAND, error))?;
                if version.mode == "120000" {
                    create_symlink(&file, &data).map_err(|error| io_failure(COMMAND, error))?;
                } else {
                    fs::write(&file, &data).map_err(|error| io_failure(COMMAND, error))?;
                    set_executable(&file, version.mode == "100755").map_err(|error| io_failure(COMMAND, error))?;
                }
            }
            None => remove_existing(&file).map_err(|error| io_failure(COMMAND, error))?,
        }
        match (&undo.index_before, &undo.index_after) {
            (Some(index), _) => {
                self.run(&["update-index", "--add", "--cacheinfo", &index.mode, &index.blob, &undo.path], directory)?;
            }
            (None, Some(_)) => {
                self.run(&["rm", "--cached", "--quiet", "--", &undo.path], directory)?;
            }
            (None, None) => {}
        }
        let index_restored = self.index_version(&undo.path, directory)?;
        let working_restored = self.working_version(&undo.path, directory, false);
        if index_restored != undo.index_before || working_restored.as_ref().ok() != Some(&undo.working_before) {
            return Err(GitError::failed(
                COMMAND,
                format!("{} was restored, but it does not exactly match its state before the discard. Review it.", undo.path),
            ));
        }
        Ok(())
    }

    /// The stage-zero index entry for a path, if any.
    fn index_version(&self, path: &str, directory: &Path) -> Result<Option<FileVersion>> {
        let output = self.run(&["ls-files", "-z", "--stage", "--", path], directory)?;
        for record in output.split('\0') {
            let Some((fields, name)) = record.split_once('\t') else { continue };
            let parts: Vec<&str> = fields.split(' ').collect();
            if name == path && parts.len() == 3 && parts[2] == "0" {
                return Ok(Some(FileVersion { mode: parts[0].to_string(), blob: parts[1].to_string() }));
            }
        }
        Ok(None)
    }

    /// The file on disk as Git would record it. `store` also writes its contents to the object
    /// database. A missing file is `None`; a folder or other kind of entry is an error.
    fn working_version(&self, path: &str, directory: &Path, store: bool) -> Result<Option<FileVersion>> {
        const COMMAND: &str = "discard";
        let file = directory.join(path);
        let metadata = match fs::symlink_metadata(&file) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(io_failure(COMMAND, error)),
        };
        let file_type = metadata.file_type();
        if file_type.is_symlink() {
            let target = fs::read_link(&file).map_err(|error| io_failure(COMMAND, error))?;
            let scratch = Scratch::new(COMMAND)?;
            let temporary = scratch.write(COMMAND, "link-target", &os_bytes(target.as_os_str()))?;
            let blob = self.hash_file(&temporary, store, directory)?;
            Ok(Some(FileVersion { mode: "120000".to_string(), blob }))
        } else if file_type.is_file() {
            let mode = if is_executable(&metadata) { "100755" } else { "100644" };
            let blob = self.hash_file(&file, store, directory)?;
            Ok(Some(FileVersion { mode: mode.to_string(), blob }))
        } else {
            Err(GitError::failed(COMMAND, "Only files can be restored after a discard."))
        }
    }

    fn hash_file(&self, file: &Path, store: bool, directory: &Path) -> Result<String> {
        let file = path_text(file);
        let mut arguments = vec!["hash-object", "--no-filters"];
        if store {
            arguments.push("-w");
        }
        arguments.extend(["--", file.as_str()]);
        self.run_trimmed(&arguments, directory)
    }

    /// The raw bytes of a blob. Binary contents are kept exactly, unlike `run`, which requires UTF-8.
    fn blob_contents(&self, blob: &str, directory: &Path) -> Result<Vec<u8>> {
        runner::run_bytes(&["cat-file", "blob", blob], directory, &self.options())
    }
}

/// A path's raw bytes. Unix paths are arbitrary bytes, so they are kept exactly.
fn os_bytes(value: &OsStr) -> Vec<u8> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        value.as_bytes().to_vec()
    }
    #[cfg(not(unix))]
    {
        value.to_string_lossy().into_owned().into_bytes()
    }
}

#[cfg(unix)]
fn is_executable(metadata: &fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable(_metadata: &fs::Metadata) -> bool {
    false
}

#[cfg(unix)]
fn set_executable(file: &Path, executable: bool) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(file, fs::Permissions::from_mode(if executable { 0o755 } else { 0o644 }))
}

#[cfg(not(unix))]
fn set_executable(_file: &Path, _executable: bool) -> io::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn create_symlink(file: &Path, target: &[u8]) -> io::Result<()> {
    use std::os::unix::ffi::OsStrExt;
    std::os::unix::fs::symlink(OsStr::from_bytes(target), file)
}

#[cfg(not(unix))]
fn create_symlink(file: &Path, target: &[u8]) -> io::Result<()> {
    std::os::windows::fs::symlink_file(String::from_utf8_lossy(target).as_ref(), file)
}

/// Removes a file or symbolic link at `file`, if there is one. Folders are never removed.
fn remove_existing(file: &Path) -> io::Result<()> {
    match fs::symlink_metadata(file) {
        Ok(metadata) if metadata.is_dir() => Err(io::Error::other("a folder is in the way")),
        Ok(_) => fs::remove_file(file),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}
