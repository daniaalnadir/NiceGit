//! Resolving merge conflicts: reading the three versions of a conflicted file, checking an
//! edited result for leftover markers, saving it, and whole-file resolutions.

use std::fs;
use std::path::{Component, Path, PathBuf};

use crate::client::GitClient;
use crate::models::{GitError, Result, StatusKind};
use crate::runner;

/// Files larger than this are too large for the built-in editor.
const EDITOR_LIMIT_BYTES: u64 = 2_000_000;

/// Git's default length for conflict marker lines.
const DEFAULT_MARKER_SIZE: usize = 7;

/// A conflicted file as the editor sees it: the working file's text and the three versions
/// Git kept for the conflict.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConflictDocument {
    pub path: String,
    /// The working file's text when it was loaded. The editor starts from it.
    pub content: String,
    /// The file's bytes when it was loaded, so a save can refuse if the file changed on disk.
    pub original: Vec<u8>,
    /// The common ancestor (stage 1), if Git kept one.
    pub base: Option<String>,
    /// Our side (stage 2), if it exists as text.
    pub current: Option<String>,
    /// Their side (stage 3), if it exists as text.
    pub incoming: Option<String>,
    /// The length of conflict marker lines, from the file's `conflict-marker-size` attribute.
    pub marker_size: usize,
}

/// Whether `content` still has a conflict marker line. A marker is a run of at least
/// `marker_size` identical `<`, `=`, `>`, or `|` characters at the start of a line.
pub fn has_conflict_markers(content: &str, marker_size: usize) -> bool {
    let size = marker_size.max(1);
    content.split('\n').any(|line| match line.chars().next() {
        Some(first) if matches!(first, '<' | '=' | '>' | '|') => line.chars().take_while(|&c| c == first).count() >= size,
        _ => false,
    })
}

impl GitClient {
    /// Reads a conflicted file for the editor. Refuses files that are too large, are not UTF-8
    /// text, or no longer have a conflict.
    pub fn load_conflict(&self, path: &str, directory: &Path) -> Result<ConflictDocument> {
        let file = self.conflict_file(path, directory)?;
        let size = fs::metadata(&file).map_err(io_failure)?.len();
        if size > EDITOR_LIMIT_BYTES {
            return Err(conflict_error("This file is too large for the built-in editor."));
        }
        let original = fs::read(&file).map_err(io_failure)?;
        if original.contains(&0) {
            return Err(not_text_error());
        }
        let content = String::from_utf8(original.clone()).map_err(|_| not_text_error())?;
        Ok(ConflictDocument {
            path: path.to_string(),
            content,
            original,
            base: self.conflict_version(path, 1, directory),
            current: self.conflict_version(path, 2, directory),
            incoming: self.conflict_version(path, 3, directory),
            marker_size: self.marker_size(path, directory)?,
        })
    }

    /// Saves an edited result and stages it. Refuses if the result still has conflict markers,
    /// or if the file changed on disk since `document` was loaded.
    pub fn resolve_conflict(&self, document: &ConflictDocument, content: &str, directory: &Path) -> Result<()> {
        let file = self.conflict_file(&document.path, directory)?;
        let on_disk = fs::read(&file).map_err(io_failure)?;
        if on_disk != document.original {
            return Err(conflict_error("The file changed outside this editor. Close and reopen it before saving."));
        }
        if has_conflict_markers(content, document.marker_size) {
            return Err(conflict_error("Remove the conflict markers before saving the resolution."));
        }
        write_keeping_permissions(&file, content.as_bytes())?;
        self.stage(&document.path, directory)
    }

    /// Resolves a conflict by taking one side of the whole file, then stages it.
    pub fn resolve_conflict_side(&self, path: &str, incoming: bool, directory: &Path) -> Result<()> {
        self.require_conflict(path, directory)?;
        let side = if incoming { "--theirs" } else { "--ours" };
        self.run(&["checkout", side, "--", path], directory)?;
        self.stage(path, directory)
    }

    /// Resolves a conflict by deleting the file.
    pub fn resolve_conflict_deletion(&self, path: &str, directory: &Path) -> Result<()> {
        self.require_conflict(path, directory)?;
        self.run(&["rm", "--force", "--", path], directory).map(drop)
    }

    /// One of the three versions Git kept for a conflicted file: stage 1 is the base, 2 is ours,
    /// and 3 is theirs. Returns `None` when that version is missing or is not UTF-8 text.
    pub fn conflict_version(&self, path: &str, stage: u8, directory: &Path) -> Option<String> {
        // `cat-file` writes the stored bytes unchanged, without textconv or filters.
        let object = format!(":{stage}:{path}");
        let bytes = runner::run_bytes(&["cat-file", "blob", &object], directory, &self.options()).ok()?;
        if bytes.contains(&0) {
            return None;
        }
        String::from_utf8(bytes).ok()
    }

    fn require_conflict(&self, path: &str, directory: &Path) -> Result<()> {
        let conflicted =
            self.load_status_for(directory, &[path])?.iter().any(|entry| entry.path == path && entry.kind == StatusKind::Conflicted);
        if conflicted {
            Ok(())
        } else {
            Err(conflict_error("This file no longer has an unresolved conflict."))
        }
    }

    /// The working file for a conflict, after checking it is still conflicted and is a regular
    /// file inside the repository.
    fn conflict_file(&self, path: &str, directory: &Path) -> Result<PathBuf> {
        self.require_conflict(path, directory)?;
        let relative = Path::new(path);
        let inside = relative.components().all(|component| match component {
            Component::Normal(name) => !name.to_string_lossy().eq_ignore_ascii_case(".git"),
            _ => false,
        });
        if path.is_empty() || !inside {
            return Err(conflict_error("Only regular files inside the repository can be edited here."));
        }
        let root = self.repository_root(directory)?;
        let file = root.join(relative);
        // `symlink_metadata` does not follow links, so a symbolic link is refused outright.
        let metadata = fs::symlink_metadata(&file).map_err(io_failure)?;
        if !metadata.file_type().is_file() {
            return Err(conflict_error("Only regular files inside the repository can be edited here."));
        }
        let canonical_root = fs::canonicalize(&root).map_err(io_failure)?;
        let canonical = fs::canonicalize(&file).map_err(io_failure)?;
        if !canonical.starts_with(&canonical_root) {
            return Err(conflict_error("Only regular files inside the repository can be edited here."));
        }
        Ok(file)
    }

    /// The `conflict-marker-size` attribute for a path, or Git's default when it is unset.
    fn marker_size(&self, path: &str, directory: &Path) -> Result<usize> {
        let output = self.run(&["check-attr", "-z", "conflict-marker-size", "--", path], directory)?;
        // With -z, each result is three NUL-terminated fields: path, attribute, value.
        let fields: Vec<&str> = output.split('\0').collect();
        let value = fields.chunks(3).find(|chunk| chunk.len() == 3 && chunk[1] == "conflict-marker-size").map(|chunk| chunk[2]);
        Ok(value.and_then(|text| text.parse::<usize>().ok()).filter(|&size| size > 0).unwrap_or(DEFAULT_MARKER_SIZE))
    }
}

fn conflict_error(message: &str) -> GitError {
    GitError::failed("resolve conflict", message)
}

fn not_text_error() -> GitError {
    conflict_error("This file is not UTF-8 text. Choose a whole-file resolution or use an external editor.")
}

fn io_failure(error: std::io::Error) -> GitError {
    GitError::failed("resolve conflict", error.to_string())
}

/// Replaces a file's contents through a temporary file in the same folder, keeping its
/// permissions. The rename is atomic, so a failed save never leaves a half-written file.
fn write_keeping_permissions(file: &Path, bytes: &[u8]) -> Result<()> {
    let permissions = fs::metadata(file).map_err(io_failure)?.permissions();
    let name = file.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    let temporary = file.with_file_name(format!(".{name}.nicegit-{}.tmp", std::process::id()));
    let result = (|| -> std::io::Result<()> {
        fs::write(&temporary, bytes)?;
        fs::set_permissions(&temporary, permissions)?;
        fs::rename(&temporary, file)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.map_err(io_failure)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_markers_at_the_configured_length() {
        assert!(has_conflict_markers("<<<<<<< HEAD\n", 7));
        assert!(has_conflict_markers("=======\n", 7));
        assert!(!has_conflict_markers("======\n", 7));
        assert!(has_conflict_markers("===\n", 3));
        assert!(has_conflict_markers("====\n", 3));
        assert!(!has_conflict_markers("==\n", 3));
        assert!(!has_conflict_markers("a << b\n", 3));
    }
}
