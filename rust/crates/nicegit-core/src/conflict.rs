//! Resolving merge conflicts: reading the three versions of a conflicted file, checking an
//! edited result for leftover markers, saving it, and whole-file resolutions.

use std::collections::BTreeSet;
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

    /// Resolves a conflict by taking one side of the whole file, then stages it. When that side
    /// has no version of the file, because it deleted the file, taking it deletes the file.
    pub fn resolve_conflict_side(&self, path: &str, incoming: bool, directory: &Path) -> Result<()> {
        self.require_conflict(path, directory)?;
        // Stage 2 is ours and stage 3 is theirs. `checkout` fails for a side that has no stage.
        let stage = if incoming { 3 } else { 2 };
        if !self.conflict_has_stage(path, stage, directory)? {
            return self.run(&["rm", "--force", "--", path], directory).map(drop);
        }
        let side = if incoming { "--theirs" } else { "--ours" };
        self.run(&["checkout", side, "--", path], directory)?;
        self.stage(path, directory)
    }

    /// Resolves several conflicted files by taking one side of each, as `resolve_conflict_side`
    /// does. A file that cannot be resolved, including one no longer conflicted, is reported and
    /// the others are still resolved.
    pub fn resolve_conflicts_side(&self, paths: &[String], incoming: bool, directory: &Path) -> Result<ResolveReport> {
        let mut report = ResolveReport::default();
        for path in paths {
            match self.resolve_conflict_side(path, incoming, directory) {
                Ok(()) => report.resolved.push(path.clone()),
                Err(error) => report.failed.push((path.clone(), failure_message(&error))),
            }
        }
        Ok(report)
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

    /// Whether Git kept a version of `path` at `stage`. `ls-files -u -z` writes one record per
    /// stage as `<mode> <object> <stage>TAB<path>`, each ended by a NUL.
    fn conflict_has_stage(&self, path: &str, stage: u8, directory: &Path) -> Result<bool> {
        let output = self.run(&["ls-files", "-u", "-z", "--", path], directory)?;
        let mut stages = output.split('\0').filter_map(|record| {
            let (info, _) = record.split_once('\t')?;
            info.split_whitespace().nth(2)?.parse::<u8>().ok()
        });
        Ok(stages.any(|found| found == stage))
    }

    pub(crate) fn require_conflict(&self, path: &str, directory: &Path) -> Result<()> {
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

    /// Whether Git records and reuses conflict resolutions here (`rerere`). An explicit
    /// `rerere.enabled` wins; otherwise Git turns rerere on when the repository has an `rr-cache`
    /// directory.
    pub fn rerere_enabled(&self, directory: &Path) -> Result<bool> {
        let configured = self.run_accepting(&["config", "--get", "--bool", "--", "rerere.enabled"], directory, &[0, 1])?;
        match configured.trim() {
            "true" => Ok(true),
            "false" => Ok(false),
            _ => {
                // rr-cache lives in the common Git directory, which `--git-path` resolves for linked worktrees.
                let cache = runner::strip_line_terminator(self.run(&["rev-parse", "--git-path", "rr-cache"], directory)?);
                Ok(directory.join(cache).is_dir())
            }
        }
    }

    /// Sets `rerere.enabled` in this repository's own configuration.
    pub fn set_rerere_enabled(&self, enabled: bool, directory: &Path) -> Result<()> {
        let value = if enabled { "true" } else { "false" };
        self.run(&["config", "--local", "--replace-all", "--", "rerere.enabled", value], directory).map(drop)
    }

    /// The conflicted files that rerere resolved from a recorded resolution. Git writes such a file
    /// with its resolved text but leaves it unmerged, so these are the unmerged paths that
    /// `git rerere remaining` does not list, since that lists files that still have conflict
    /// markers. Empty while rerere is off. Pass the repository root, as the paths are relative to it.
    ///
    /// Caveat: a file whose markers were removed by hand, and not yet staged, is reported too.
    pub fn rerere_reused_paths(&self, directory: &Path) -> Result<Vec<String>> {
        if !self.rerere_enabled(directory)? {
            return Ok(Vec::new());
        }
        let unmerged = self.unmerged_paths(directory)?;
        if unmerged.is_empty() {
            return Ok(Vec::new());
        }
        // `rerere remaining` has no `-z`: it prints each path unquoted on its own line. Matching
        // whole lines, with a newline before each path, keeps a path that contains a newline intact.
        let remaining = format!("\n{}", self.run(&["rerere", "remaining"], directory)?);
        Ok(unmerged.into_iter().filter(|path| !remaining.contains(&format!("\n{path}\n"))).collect())
    }

    /// The paths Git lists as unmerged in the index, sorted and without repeats.
    fn unmerged_paths(&self, directory: &Path) -> Result<Vec<String>> {
        let output = self.run(&["ls-files", "-u", "-z"], directory)?;
        let paths: BTreeSet<String> =
            output.split('\0').filter_map(|record| record.split_once('\t')).map(|(_, path)| path.to_string()).collect();
        Ok(paths.into_iter().collect())
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

/// The outcome of resolving several conflicted files: the paths that were resolved, and each
/// path that was not with the reason.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ResolveReport {
    pub resolved: Vec<String>,
    pub failed: Vec<(String, String)>,
}

fn conflict_error(message: &str) -> GitError {
    GitError::failed("resolve conflict", message)
}

/// The reason a failed resolution gives, without the command prefix that `Display` adds.
fn failure_message(error: &GitError) -> String {
    match error {
        GitError::CommandFailed { message, .. } => message.clone(),
        other => other.to_string(),
    }
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

    /// A repository on `main` where each of `files` holds "base" and then differs on `main` and on
    /// `side`, so merging `side` conflicts in all of them. Returns the repository and `main`'s tip.
    fn conflicting_repository(files: &[&str]) -> (tempfile::TempDir, GitClient, String) {
        let dir = tempfile::tempdir().unwrap();
        let client = GitClient::new();
        let root = dir.path();
        client.run(&["init", "--initial-branch=main"], root).unwrap();
        client.run(&["config", "user.name", "Test"], root).unwrap();
        client.run(&["config", "user.email", "test@example.com"], root).unwrap();
        client.run(&["config", "commit.gpgsign", "false"], root).unwrap();
        client.set_rerere_enabled(true, root).unwrap();
        let write_all = |text: &str| {
            for file in files {
                std::fs::write(root.join(file), text).unwrap();
            }
            client.run(&["add", "--all"], root).unwrap();
            client.run(&["commit", "-q", "-m", text.trim()], root).unwrap();
        };
        write_all("base\n");
        client.run(&["checkout", "-q", "-b", "side"], root).unwrap();
        write_all("side\n");
        client.run(&["checkout", "-q", "main"], root).unwrap();
        write_all("main\n");
        let tip = client.run(&["rev-parse", "HEAD"], root).unwrap().trim().to_string();
        (dir, client, tip)
    }

    /// Merges `side` and resolves every conflict to "resolved", which records the resolution.
    fn merge_and_resolve(client: &GitClient, root: &Path, files: &[&str]) {
        assert!(client.run(&["merge", "--no-edit", "side"], root).is_err());
        for file in files {
            std::fs::write(root.join(file), "resolved\n").unwrap();
        }
        client.run(&["add", "--all"], root).unwrap();
        client.run(&["commit", "-q", "--no-edit"], root).unwrap();
    }

    #[test]
    fn rerere_reuses_a_recorded_resolution_and_reports_the_file() {
        let (dir, client, tip) = conflicting_repository(&["a.txt"]);
        let root = dir.path();
        merge_and_resolve(&client, root, &["a.txt"]);
        client.run(&["reset", "-q", "--hard", &tip], root).unwrap();

        assert!(client.run(&["merge", "--no-edit", "side"], root).is_err());
        assert_eq!(client.rerere_reused_paths(root).unwrap(), vec!["a.txt".to_string()]);
        let text = std::fs::read_to_string(root.join("a.txt")).unwrap();
        assert_eq!(text, "resolved\n");
        assert!(!has_conflict_markers(&text, DEFAULT_MARKER_SIZE));
    }

    #[test]
    fn nothing_is_reused_while_rerere_is_off() {
        let (dir, client, tip) = conflicting_repository(&["a.txt"]);
        let root = dir.path();
        merge_and_resolve(&client, root, &["a.txt"]);
        client.run(&["reset", "-q", "--hard", &tip], root).unwrap();

        client.set_rerere_enabled(false, root).unwrap();
        assert!(!client.rerere_enabled(root).unwrap());
        assert!(client.run(&["merge", "--no-edit", "side"], root).is_err());
        assert!(client.rerere_reused_paths(root).unwrap().is_empty());
        assert!(has_conflict_markers(&std::fs::read_to_string(root.join("a.txt")).unwrap(), DEFAULT_MARKER_SIZE));
    }

    #[test]
    fn rerere_setting_round_trips_in_local_configuration() {
        let dir = tempfile::tempdir().unwrap();
        let client = GitClient::new();
        let root = dir.path();
        client.run(&["init", "--initial-branch=main"], root).unwrap();
        // Without a setting, Git turns rerere on only when the rr-cache directory exists.
        assert!(!client.rerere_enabled(root).unwrap());
        client.set_rerere_enabled(true, root).unwrap();
        assert!(client.rerere_enabled(root).unwrap());
        assert_eq!(client.run(&["config", "--local", "--get", "rerere.enabled"], root).unwrap().trim(), "true");
        client.set_rerere_enabled(false, root).unwrap();
        assert!(!client.rerere_enabled(root).unwrap());
        assert_eq!(client.run(&["config", "--local", "--get", "rerere.enabled"], root).unwrap().trim(), "false");
    }

    #[test]
    fn reused_paths_keep_names_with_spaces() {
        let (dir, client, tip) = conflicting_repository(&["b c.txt"]);
        let root = dir.path();
        merge_and_resolve(&client, root, &["b c.txt"]);
        client.run(&["reset", "-q", "--hard", &tip], root).unwrap();

        assert!(client.run(&["merge", "--no-edit", "side"], root).is_err());
        assert_eq!(client.rerere_reused_paths(root).unwrap(), vec!["b c.txt".to_string()]);
        assert_eq!(std::fs::read_to_string(root.join("b c.txt")).unwrap(), "resolved\n");
    }

    #[cfg(unix)]
    #[test]
    fn reused_paths_keep_names_with_newlines() {
        let name = "x\ny.txt";
        let (dir, client, tip) = conflicting_repository(&[name, "a.txt"]);
        let root = dir.path();
        merge_and_resolve(&client, root, &[name, "a.txt"]);
        client.run(&["reset", "-q", "--hard", &tip], root).unwrap();

        assert!(client.run(&["merge", "--no-edit", "side"], root).is_err());
        assert_eq!(client.rerere_reused_paths(root).unwrap(), vec!["a.txt".to_string(), name.to_string()]);
    }
}
