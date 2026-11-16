//! Staging and unstaging individual lines of a file. The selected lines are turned into a
//! full-file patch against the index, which `git apply --cached` checks against the index
//! before writing anything.

use std::collections::BTreeSet;
use std::fs;
use std::ops::Range;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::client::GitClient;
use crate::diff::{parse_diff, DiffLine, DiffLineKind};
use crate::models::{GitError, Result, Stash, StatusKind};
use crate::rebase::Scratch;
use crate::runner::{self, RunOptions};
use crate::undo::DiscardUndo;

const NO_NEWLINE: &str = "\\ No newline at end of file";

/// Diffs larger than this are too slow to show and rebuild line by line.
const MAX_LINE_STAGING_BYTES: usize = 4_000_000;

/// One file's diff, read for line-level staging. `staged` selects the index-to-HEAD diff;
/// otherwise the working-file-to-index diff is used.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileReview {
    pub path: String,
    pub staged: bool,
    pub untracked: bool,
    /// The full-context patch, as Git printed it.
    pub patch: String,
    /// The patch's lines, numbered and classified by `diff::parse_diff`.
    pub lines: Vec<DiffLine>,
    /// Why lines cannot be staged individually, if they cannot.
    pub line_staging_unavailable: Option<String>,
}

/// A run of changed lines with the context lines around it, as shown in a hunk view.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffHunk {
    /// The indices of the hunk's lines in the file's diff.
    pub line_indices: Range<usize>,
    /// The added and removed lines inside the hunk.
    pub changed_indices: BTreeSet<usize>,
}

impl DiffHunk {
    /// The `@@ -a,b +c,d @@` header for this hunk, numbered from the lines it covers.
    pub fn header(&self, lines: &[DiffLine]) -> String {
        let old: Vec<usize> = lines[self.line_indices.clone()].iter().filter_map(|line| line.old_number).collect();
        let new: Vec<usize> = lines[self.line_indices.clone()].iter().filter_map(|line| line.new_number).collect();
        let before = &lines[..self.line_indices.start];
        let old_start = old.first().copied().or_else(|| before.iter().rev().find_map(|line| line.old_number)).unwrap_or(0);
        let new_start = new.first().copied().or_else(|| before.iter().rev().find_map(|line| line.new_number)).unwrap_or(0);
        format!("@@ -{old_start},{} +{new_start},{} @@", old.len(), new.len())
    }

    /// Groups changed lines into hunks, keeping up to `context` unchanged lines on each side.
    /// Changes whose context overlaps merge into one hunk, and Git's missing-newline marker
    /// between two changes does not split them.
    pub fn grouped(lines: &[DiffLine], context: usize) -> Vec<DiffHunk> {
        let mut ranges: Vec<Range<usize>> = Vec::new();
        for index in 0..lines.len() {
            if !is_change(&lines[index]) {
                continue;
            }
            let mut start = index;
            let mut end = index + 1;
            let mut count = 0;
            while start > 0 && count < context && lines[start - 1].kind == DiffLineKind::Context {
                start -= 1;
                count += 1;
            }
            count = 0;
            while end < lines.len() && count < context && lines[end].kind == DiffLineKind::Context {
                end += 1;
                count += 1;
            }
            let merge = ranges
                .last()
                .is_some_and(|previous| start <= previous.end || lines[previous.end..start].iter().all(|line| line.text == NO_NEWLINE));
            if merge {
                let previous = ranges.last_mut().expect("a previous hunk exists when merging");
                previous.end = previous.end.max(end);
            } else {
                ranges.push(start..end);
            }
        }
        ranges
            .into_iter()
            .map(|range| {
                let changed_indices = range.clone().filter(|&index| is_change(&lines[index])).collect();
                DiffHunk { line_indices: range, changed_indices }
            })
            .collect()
    }
}

/// Matches each line of a diff as shown (usually with a few lines of context) to the same line
/// of `review`'s full-file diff, by its kind, its old and new line numbers, and its text, so
/// lines chosen in the shown diff can be staged. Headers and notes map to `None`. Returns
/// `None` when a shown line has no exact counterpart, as when the file changed between the two.
pub fn line_map(shown: &[DiffLine], review: &FileReview) -> Option<Vec<Option<usize>>> {
    let key = |line: &DiffLine| (line.kind as u8, line.old_number, line.new_number);
    let mut index = std::collections::HashMap::with_capacity(review.lines.len());
    for (position, line) in review.lines.iter().enumerate() {
        if matches!(line.kind, DiffLineKind::Context | DiffLineKind::Addition | DiffLineKind::Deletion) {
            index.insert(key(line), position);
        }
    }
    shown
        .iter()
        .map(|line| {
            if !matches!(line.kind, DiffLineKind::Context | DiffLineKind::Addition | DiffLineKind::Deletion) {
                return Some(None);
            }
            let position = *index.get(&key(line))?;
            (review.lines[position].text == line.text).then_some(Some(position))
        })
        .collect()
}

fn is_change(line: &DiffLine) -> bool {
    matches!(line.kind, DiffLineKind::Addition | DiffLineKind::Deletion)
}

impl GitClient {
    /// The diff of one file, with full context, for choosing lines to stage or unstage.
    pub fn file_review(&self, path: &str, staged: bool, directory: &Path) -> Result<FileReview> {
        // The full status, not one limited to this path: a rename shows its source only there.
        let entry = self.load_status(directory)?.into_iter().find(|entry| entry.path == path);
        let untracked = entry.as_ref().is_some_and(|entry| entry.kind == StatusKind::Untracked);
        // Full context, so the patch carries the whole file and every line can be chosen.
        let options = ["--no-ext-diff", "--no-textconv", "--no-color", "--no-renames", "--unified=1000000"];
        let patch = if untracked && !staged {
            // Git exits with 1 when the files differ, which is the normal case here.
            let mut arguments = vec!["diff", "--no-index"];
            arguments.extend(options);
            arguments.extend(["--", "/dev/null", path]);
            self.run_accepting(&arguments, directory, &[0, 1])?
        } else {
            let mut arguments = vec!["diff"];
            arguments.extend(options);
            if staged {
                arguments.push("--cached");
            }
            arguments.extend(["--", path]);
            self.run(&arguments, directory)?
        };
        let lines = parse_diff(&patch);
        // Only the header lines before the first hunk describe the file; content can look alike.
        let headers: Vec<&str> = lines.iter().take_while(|line| line.kind != DiffLineKind::Hunk).map(|line| line.text.as_str()).collect();
        let reason = match entry.as_ref().map(|entry| (entry.kind, entry.original_path.is_some())) {
            Some((StatusKind::Conflicted, _)) => Some("Resolve conflicts before staging individual lines."),
            Some((_, true)) => Some("Stage or unstage renamed and copied files as a whole."),
            _ if headers.iter().any(|text| is_whole_file_header(text)) => {
                Some("Binary files, symbolic links, and submodules require whole-file staging.")
            }
            _ if patch.len() > MAX_LINE_STAGING_BYTES => Some("This diff is too large for line staging."),
            _ => None,
        };
        Ok(FileReview { path: path.to_string(), staged, untracked, patch, lines, line_staging_unavailable: reason.map(str::to_string) })
    }

    /// Stages the selected lines of a file, or unstages them when `review` is the staged diff.
    /// `selected` holds indices into `review.lines`, and each must be an added or removed line.
    /// Refuses if the file or index changed since `review` was read.
    pub fn stage_lines(&self, selected: &BTreeSet<usize>, review: &FileReview, directory: &Path) -> Result<()> {
        if let Some(reason) = &review.line_staging_unavailable {
            return Err(review_error(reason));
        }
        let all_changes = selected.iter().all(|&index| review.lines.get(index).is_some_and(is_change));
        if selected.is_empty() || !all_changes {
            return Err(review_error("Select added or removed lines first."));
        }
        let fresh = self.file_review(&review.path, review.staged, directory)?;
        if fresh.patch != review.patch || fresh.line_staging_unavailable.is_some() {
            return Err(review_error("The file or index changed. Reload the diff before staging lines."));
        }

        let (baseline, target) = line_sides(review, selected);
        if baseline == target {
            return Err(review_error("The selected lines do not change the index."));
        }
        let patch = build_patch(review, &baseline, &target);
        apply_patch(self, &patch, true, directory)
    }

    /// Discards the selected lines of a file's unstaged changes, putting them back as they are
    /// in the index, and first saves what is needed to undo it. `selected` holds indices into
    /// `review.lines`, which must be the unstaged diff. Refuses if the file or index changed
    /// since `review` was read; the change is applied with `git apply`, which refuses rather
    /// than writes when the working file does not match. Returns `None` (after discarding) when
    /// the file's versions could not be saved for undo.
    pub fn discard_lines_keeping_undo(
        &self,
        selected: &BTreeSet<usize>,
        review: &FileReview,
        directory: &Path,
    ) -> Result<Option<DiscardUndo>> {
        if review.staged {
            return Err(review_error("Only unstaged changes can be discarded line by line."));
        }
        if let Some(reason) = &review.line_staging_unavailable {
            return Err(review_error(reason));
        }
        let all_changes = selected.iter().all(|&index| review.lines.get(index).is_some_and(is_change));
        if selected.is_empty() || !all_changes {
            return Err(review_error("Select added or removed lines first."));
        }
        let fresh = self.file_review(&review.path, false, directory)?;
        if fresh.patch != review.patch || fresh.line_staging_unavailable.is_some() {
            return Err(review_error("The file or index changed. Reload the diff before discarding lines."));
        }

        // Read as the working file against the index, reverting the selected lines is what
        // unstaging them would do to the index; the patch is applied to the working file.
        let reverse = FileReview { staged: true, ..review.clone() };
        let (working, target) = line_sides(&reverse, selected);
        if working == target {
            return Err(review_error("The selected lines do not change the file."));
        }
        let before = match (self.index_version(&review.path, directory), self.working_version(&review.path, directory, true)) {
            (Ok(index), Ok(working)) => Some((index, working)),
            _ => None,
        };
        let patch = build_patch(&reverse, &working, &target);
        apply_patch(self, &patch, false, directory)?;
        let Some((index_before, working_before)) = before else { return Ok(None) };
        let after = match (self.index_version(&review.path, directory), self.working_version(&review.path, directory, false)) {
            (Ok(index), Ok(working)) => (index, working),
            _ => return Ok(None),
        };
        Ok(Some(DiscardUndo { path: review.path.clone(), index_before, working_before, index_after: after.0, working_after: after.1 }))
    }

    /// Stashes the selected lines of a file's unstaged changes, and removes them from the working
    /// file. The stash is shaped as `git stash` makes one, a commit with HEAD and an index commit
    /// as parents, so Apply, Pop, and Drop work on it unchanged. Its tree is HEAD with only this
    /// file changed to its index version plus the selected lines, so the stash holds this file
    /// and nothing else. The stash commits are made first, in a temporary index, so a failure
    /// before the working file changes leaves everything as it was. `selected` holds indices into
    /// `review.lines`, which must be the unstaged diff. Refuses if the file or index changed since
    /// `review` was read, and checks afterwards that only this file changed.
    pub fn stash_lines(&self, message: &str, selected: &BTreeSet<usize>, review: &FileReview, directory: &Path) -> Result<()> {
        if review.staged {
            return Err(stash_error("Only unstaged changes can be stashed line by line."));
        }
        if let Some(reason) = &review.line_staging_unavailable {
            return Err(stash_error(reason));
        }
        // Added and deleted files have no index version to keep the rest of the file in.
        let whole_file = review
            .lines
            .iter()
            .take_while(|line| line.kind != DiffLineKind::Hunk)
            .any(|line| matches!(line.text.as_str(), "--- /dev/null" | "+++ /dev/null"));
        if review.untracked || whole_file {
            return Err(stash_error("Added and deleted files are stashed as a whole, not line by line."));
        }
        let all_changes = selected.iter().all(|&index| review.lines.get(index).is_some_and(is_change));
        if selected.is_empty() || !all_changes {
            return Err(stash_error("Select added or removed lines first."));
        }
        self.require_finished_operation("stash", directory)?;
        let fresh = self.file_review(&review.path, false, directory)?;
        if fresh.patch != review.patch || fresh.line_staging_unavailable.is_some() {
            return Err(stash_error("The file or index changed. Reload the diff before stashing lines."));
        }

        // The stash's own state: the index content with the selected lines, which is what staging
        // them would give. The working file is changed afterwards with the matching patch.
        let (baseline, target) = line_sides(review, selected);
        if baseline == target {
            return Err(stash_error("The selected lines do not change the index."));
        }
        let stage = build_patch(review, &baseline, &target);
        // The working file, read as an index, with the selected lines reverted: what discarding
        // them would do. Applying it to the working file is refused if the file does not match.
        let reverse = FileReview { staged: true, ..review.clone() };
        let (working, kept) = line_sides(&reverse, selected);
        if working == kept {
            return Err(stash_error("The selected lines do not change the file."));
        }
        let removal = build_patch(&reverse, &working, &kept);

        let head = self.head(directory).ok_or_else(|| stash_error("Make a first commit before stashing lines."))?;
        let others_before = status_of_other_paths(self, &review.path, directory)?;
        let branch = self.try_trimmed(&["branch", "--show-current"], directory).unwrap_or_else(|| "(no branch)".to_string());
        let subject = self.run_trimmed(&["log", "-1", "--format=%h %s", &head], directory)?;
        let label = if message.trim().is_empty() { format!("Selected lines from {}", review.path) } else { message.trim().to_string() };
        let index_message = format!("index on {branch}: {subject}");
        let stash_message = format!("On {branch}: {label}");

        // Everything below uses a temporary index, so the real index is only read.
        let scratch = Scratch::new("stash lines")?;
        let index_path = scratch.dir().join("index").to_string_lossy().into_owned();
        let index_tree = self.run_trimmed(&["write-tree"], directory)?;
        let index_commit = self.run_trimmed(&["commit-tree", &index_tree, "-p", &head, "-m", &index_message], directory)?;
        let version = self
            .index_version(&review.path, directory)?
            .ok_or_else(|| stash_error("This file is not in the index. Stash it as a whole."))?;
        run_in_index(self, &["read-tree", &head], directory, &index_path)?;
        run_in_index(self, &["update-index", "--add", "--cacheinfo", &version.mode, &version.blob, &review.path], directory, &index_path)?;
        apply_patch_in(self, &stage, true, Some(&index_path), directory)?;
        let tree = run_in_index(self, &["write-tree"], directory, &index_path)?;
        let stash = self.run_trimmed(&["commit-tree", &tree, "-p", &head, "-p", &index_commit, "-m", &stash_message], directory)?;
        self.run(&["stash", "store", "-m", &stash_message, &stash], directory)?;

        // The stash is stored, so the working file changes only now. If that fails, the stash
        // is removed again, so the selected lines are not kept twice.
        if let Err(error) = apply_patch_in(self, &removal, false, None, directory) {
            return Err(match drop_stash_entry(self, &stash, directory) {
                Ok(()) => error,
                Err(drop_error) => stash_error(&format!(
                    "The lines were stashed, but the file was not changed and the stash could not be removed. Check Stashes before continuing. {drop_error}"
                )),
            });
        }

        let file_is_expected = fs::read(directory.join(&review.path)).is_ok_and(|bytes| bytes == kept.as_bytes());
        let others_after = status_of_other_paths(self, &review.path, directory)?;
        if !file_is_expected || others_after != others_before {
            return Err(stash_error(
                "The lines were stashed, but the file is not as expected, or other files changed. Check the file and Stashes before continuing.",
            ));
        }
        Ok(())
    }

    /// Applies a stash's working-file changes on top of the changes already there, for when
    /// Git's own apply or pop refuses because they would be overwritten. The changes are the
    /// stash's part after its index commit (`<stash>^2`), which for a stash of selected lines is
    /// exactly those lines. Only working files change, and only if the changes fit the files as
    /// they are now; otherwise nothing changes. With `drop_after`, the stash is removed once the
    /// changes are applied. Stashes with untracked files are refused. Refused during an
    /// unfinished operation.
    /// Applies a stash's unstaged changes on top of the working files when Git's own apply refuses.
    /// Returns what happened to the stash. Only the changes against the stash's index commit are
    /// applied, so a stash that also saved staged changes keeps them: it is never removed here, even
    /// for a pop, and the caller says so.
    pub fn apply_stash_into_changes(&self, stash: &Stash, drop_after: bool, directory: &Path) -> Result<StashIntoChanges> {
        const COMMAND: &str = "apply stash";
        self.require_finished_operation(COMMAND, directory)?;
        let listed = self.require_listed_stash(stash, COMMAND, directory)?;
        let parents: Vec<String> =
            self.run_trimmed(&["log", "-1", "--format=%P", &listed.hash], directory)?.split_whitespace().map(str::to_string).collect();
        // A stash with untracked files has a third parent, which these changes do not cover.
        if parents.len() != 2 {
            return Err(GitError::failed(
                COMMAND,
                "This stash has untracked files, so it can only be applied as a whole. Apply it instead.",
            ));
        }
        let base = &parents[1];
        // The stash's staged part: what its index commit changed against the commit it was made on.
        let has_staged = !self.run_trimmed(&["diff", "--name-only", "-z", &parents[0], base, "--"], directory)?.is_empty();
        let names = self.run(&["diff", "--name-only", "-z", "--no-renames", base, &listed.hash], directory)?;
        let paths: Vec<String> = names.split('\0').filter(|name| !name.is_empty()).map(str::to_string).collect();
        let patch = runner::run_bytes(
            &["diff", "--no-ext-diff", "--no-textconv", "--no-color", "--no-renames", "--binary", base, &listed.hash],
            directory,
            &self.options(),
        )?;
        if patch.is_empty() {
            return Err(GitError::failed(COMMAND, "The stash has no changes to apply."));
        }

        let before = status_except(self, &paths, directory)?;
        let refused = || {
            GitError::failed(
                COMMAND,
                "The stashed lines no longer fit the file as it is now. Commit or stash your other changes, then apply it again.",
            )
        };
        apply_patch_bytes(self, &patch, &["--check"], None, directory).map_err(|_| refused())?;
        apply_patch_bytes(self, &patch, &[], None, directory).map_err(|_| refused())?;
        if status_except(self, &paths, directory)? != before {
            return Err(GitError::failed(
                COMMAND,
                "The stash was applied, but other files changed. Check your changes and Stashes before continuing.",
            ));
        }

        if has_staged {
            return Ok(StashIntoChanges::KeptForStagedChanges);
        }
        if drop_after {
            // Only the entry that was listed is removed, and only if it is still at its position.
            let still_listed =
                self.list_stashes(directory)?.iter().any(|entry| entry.reference == listed.reference && entry.hash == listed.hash);
            if !still_listed {
                return Err(GitError::failed(
                    COMMAND,
                    "The changes were applied, but the stash moved, so it was not removed. Check Stashes before continuing.",
                ));
            }
            self.run(&["stash", "drop", &listed.reference], directory).map(drop).map_err(|error| {
                GitError::failed(
                    COMMAND,
                    format!("The changes were applied, but the stash could not be removed. Do not apply it again. {error}"),
                )
            })?;
            return Ok(StashIntoChanges::Removed);
        }
        Ok(StashIntoChanges::Kept)
    }
}

/// What `apply_stash_into_changes` did with the stash after applying its changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StashIntoChanges {
    /// Applied, and the stash was kept, as asked.
    Kept,
    /// Applied, and the stash was removed, as a pop asks.
    Removed,
    /// Applied, but the stash also saved staged changes, which were not applied; it is kept.
    KeptForStagedChanges,
}

fn review_error(message: &str) -> GitError {
    GitError::failed("file review", message)
}

fn stash_error(message: &str) -> GitError {
    GitError::failed("stash lines", message)
}

/// Runs Git with `GIT_INDEX_FILE` set to `index`, so the real index is not touched.
fn run_in_index(client: &GitClient, arguments: &[&str], directory: &Path, index: &str) -> Result<String> {
    let environment = [("GIT_INDEX_FILE", index)];
    runner::run(arguments, directory, &RunOptions { env: &environment, ..client.options() }).map(|output| output.trim().to_string())
}

/// The porcelain status records for every path except `path`, so a change to another file shows.
fn status_of_other_paths(client: &GitClient, path: &str, directory: &Path) -> Result<Vec<String>> {
    status_except(client, &[path.to_string()], directory)
}

/// The porcelain status records for every path not in `paths`.
fn status_except(client: &GitClient, paths: &[String], directory: &Path) -> Result<Vec<String>> {
    let output = client.run(&["status", "--porcelain=v1", "-z", "--no-renames"], directory)?;
    Ok(output
        .split('\0')
        .filter(|record| !record.get(3..).is_some_and(|path| paths.iter().any(|excluded| excluded == path)))
        .map(str::to_string)
        .collect())
}

/// Removes the stash commit with this object ID, if it is still listed.
fn drop_stash_entry(client: &GitClient, hash: &str, directory: &Path) -> Result<()> {
    match client.list_stashes(directory)?.into_iter().find(|stash| stash.hash == hash) {
        Some(stash) => client.run(&["stash", "drop", &stash.reference], directory).map(drop),
        None => Ok(()),
    }
}

/// Headers that mark a file which cannot be staged by line.
fn is_whole_file_header(text: &str) -> bool {
    text.ends_with("120000") || text.ends_with("160000") || text.starts_with("Binary files ")
}

/// The index content before the change (`baseline`) and after it (`target`), with only the
/// selected lines applied. Lines are ordered as Git orders them, so the patch stays exact.
fn line_sides(review: &FileReview, selected: &BTreeSet<usize>) -> (String, String) {
    let mut baseline = String::new();
    let mut target = String::new();
    let mut removed: Vec<(usize, String)> = Vec::new();
    let mut added: Vec<(usize, String)> = Vec::new();

    fn append_target(target: &mut String, text: &str) {
        if !target.is_empty() && !target.ends_with('\n') {
            target.push('\n');
        }
        target.push_str(text);
    }

    // Pairs adjacent replacements so selecting one pair does not move it past an unselected
    // neighbouring replacement in the index.
    let flush = |target: &mut String, removed: &mut Vec<(usize, String)>, added: &mut Vec<(usize, String)>| {
        for offset in 0..removed.len().max(added.len()) {
            if let Some((index, text)) = removed.get(offset) {
                // Staging keeps a removal that is not selected; unstaging keeps one that is selected.
                let keep = if review.staged { selected.contains(index) } else { !selected.contains(index) };
                if keep {
                    append_target(target, text);
                }
            }
            if let Some((index, text)) = added.get(offset) {
                let keep = if review.staged { !selected.contains(index) } else { selected.contains(index) };
                if keep {
                    append_target(target, text);
                }
            }
        }
        removed.clear();
        added.clear();
    };

    for (index, line) in review.lines.iter().enumerate() {
        if !matches!(line.kind, DiffLineKind::Context | DiffLineKind::Addition | DiffLineKind::Deletion) {
            continue;
        }
        let no_newline = review.lines.get(index + 1).is_some_and(|next| next.text == NO_NEWLINE);
        let text = format!("{}{}", content(line), if no_newline { "" } else { "\n" });
        let in_old = matches!(line.kind, DiffLineKind::Context | DiffLineKind::Deletion);
        let in_new = matches!(line.kind, DiffLineKind::Context | DiffLineKind::Addition);
        if (review.staged && in_new) || (!review.staged && in_old) {
            baseline.push_str(&text);
        }
        match line.kind {
            DiffLineKind::Context => {
                flush(&mut target, &mut removed, &mut added);
                append_target(&mut target, &text);
            }
            DiffLineKind::Deletion => removed.push((index, text)),
            _ => added.push((index, text)),
        }
    }
    flush(&mut target, &mut removed, &mut added);
    (baseline, target)
}

/// The text after a diff line's one-character sign.
fn content(line: &DiffLine) -> &str {
    line.text.get(1..).unwrap_or("")
}

/// Quotes a path the way Git writes it in patch headers: C-style, with octal byte escapes for
/// anything outside printable ASCII. JSON-style escapes do not preserve control characters.
fn quote_path(path: &str) -> String {
    let mut quoted = String::from("\"");
    for byte in path.bytes() {
        match byte {
            b'"' | b'\\' => {
                quoted.push('\\');
                quoted.push(byte as char);
            }
            32..=126 => quoted.push(byte as char),
            _ => quoted.push_str(&format!("\\{byte:03o}")),
        }
    }
    quoted.push('"');
    quoted
}

/// The `-`/`+` lines for one side, and the number of lines they cover.
fn hunk_body(text: &str, sign: char) -> (usize, String) {
    if text.is_empty() {
        return (0, String::new());
    }
    let mut parts: Vec<&str> = text.split('\n').collect();
    if text.ends_with('\n') {
        parts.pop();
    }
    let mut body = String::new();
    for part in &parts {
        body.push(sign);
        body.push_str(part);
        body.push('\n');
    }
    if !text.ends_with('\n') {
        body.push_str(NO_NEWLINE);
        body.push('\n');
    }
    (parts.len(), body)
}

/// Builds a single-file patch that turns `baseline` into `target`, with headers that create or
/// delete the file only when the index itself has no such file.
fn build_patch(review: &FileReview, baseline: &str, target: &str) -> String {
    let (old_count, old_body) = hunk_body(baseline, '-');
    let (new_count, new_body) = hunk_body(target, '+');
    // Headers come only from before the first hunk, so content lines cannot imitate them.
    let headers: Vec<&str> =
        review.lines.iter().take_while(|line| line.kind != DiffLineKind::Hunk).map(|line| line.text.as_str()).collect();
    // Staging a file the index lacks creates it; unstaging a file the index holds nothing of removes it.
    let creates = headers.contains(&if review.staged { "+++ /dev/null" } else { "--- /dev/null" });
    let removes = target.is_empty() && headers.contains(&if review.staged { "--- /dev/null" } else { "+++ /dev/null" });
    let mode_of = |prefix: &str| {
        headers.iter().find(|text| text.starts_with(prefix)).and_then(|text| text.rsplit(' ').next()).unwrap_or("100644").to_string()
    };

    let a = quote_path(&format!("a/{}", review.path));
    let b = quote_path(&format!("b/{}", review.path));
    let mut patch = format!("diff --git {a} {b}\n");
    if creates {
        let mode = mode_of(if review.staged { "deleted file mode " } else { "new file mode " });
        patch.push_str(&format!("new file mode {mode}\n"));
    }
    if removes {
        let mode = mode_of(if review.staged { "new file mode " } else { "deleted file mode " });
        patch.push_str(&format!("deleted file mode {mode}\n"));
    }
    let old_name = if creates { "/dev/null".to_string() } else { a };
    let new_name = if removes { "/dev/null".to_string() } else { b };
    patch.push_str(&format!("--- {old_name}\n+++ {new_name}\n"));
    let old_start = if old_count == 0 { 0 } else { 1 };
    let new_start = if new_count == 0 { 0 } else { 1 };
    patch.push_str(&format!("@@ -{old_start},{old_count} +{new_start},{new_count} @@\n"));
    patch.push_str(&old_body);
    patch.push_str(&new_body);
    patch
}

static PATCH_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Writes the patch to a temporary file, applies it to the index (`cached`) or the working
/// files, and removes the file.
fn apply_patch(client: &GitClient, patch: &str, cached: bool, directory: &Path) -> Result<()> {
    apply_patch_in(client, patch, cached, None, directory)
}

/// Like `apply_patch`, and with `index` set, applies to the index at that path instead of the
/// repository's own index.
fn apply_patch_in(client: &GitClient, patch: &str, cached: bool, index: Option<&str>, directory: &Path) -> Result<()> {
    let flags: &[&str] = if cached { &["--cached"] } else { &[] };
    apply_patch_bytes(client, patch.as_bytes(), flags, index, directory)
}

/// Applies a patch given as bytes with extra `git apply` flags, such as `--check` or `--cached`.
/// Without `index`, the working files are changed.
fn apply_patch_bytes(client: &GitClient, patch: &[u8], flags: &[&str], index: Option<&str>, directory: &Path) -> Result<()> {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|time| time.as_nanos()).unwrap_or(0);
    let sequence = PATCH_COUNTER.fetch_add(1, Ordering::Relaxed);
    let file = std::env::temp_dir().join(format!("nicegit-lines-{}-{nanos}-{sequence}.patch", std::process::id()));
    std::fs::write(&file, patch).map_err(|error| GitError::failed("stage lines", error.to_string()))?;
    let file_text = file.to_string_lossy().into_owned();
    let mut arguments = vec!["apply"];
    arguments.extend(flags);
    arguments.extend(["--whitespace=nowarn", "--", &file_text]);
    let environment: Vec<(&str, &str)> = index.map(|path| ("GIT_INDEX_FILE", path)).into_iter().collect();
    let result = runner::run(&arguments, directory, &RunOptions { env: &environment, ..client.options() }).map(drop);
    let _ = std::fs::remove_file(&file);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn review_of(patch: &str) -> FileReview {
        FileReview {
            path: "f".into(),
            staged: false,
            untracked: false,
            patch: patch.into(),
            lines: parse_diff(patch),
            line_staging_unavailable: None,
        }
    }

    #[test]
    fn shown_lines_map_to_the_full_file_diff_by_number_and_text() {
        let review = review_of("diff --git a/f b/f\n@@ -1,8 +1,8 @@\n a\n b\n c\n-d\n+D\n e\n f\n g\n h\n");
        let shown = parse_diff("diff --git a/f b/f\n@@ -2,5 +2,5 @@\n b\n c\n-d\n+D\n e\n f\n");
        let map = line_map(&shown, &review).expect("every shown line has a counterpart");
        let texts: Vec<Option<&str>> = map.iter().map(|m| m.map(|i| review.lines[i].text.as_str())).collect();
        // The header, the hunk line, and the empty line after the last newline map to nothing.
        assert_eq!(texts, [None, None, Some(" b"), Some(" c"), Some("-d"), Some("+D"), Some(" e"), Some(" f"), None]);
        // A shown line that differs from the review's, as after an edit, has no counterpart.
        let edited = parse_diff("@@ -4 +4 @@\n-d\n+E\n");
        assert_eq!(line_map(&edited, &review), None);
    }

    #[test]
    fn quotes_control_characters_and_quotes_as_c_escapes() {
        assert_eq!(quote_path("a b"), "\"a b\"");
        assert_eq!(quote_path("tab\there"), "\"tab\\011here\"");
        assert_eq!(quote_path("say \"hi\"\\"), "\"say \\\"hi\\\"\\\\\"");
        assert_eq!(quote_path("é"), "\"\\303\\251\"");
    }

    #[test]
    fn hunk_body_marks_a_missing_final_newline() {
        assert_eq!(hunk_body("a\nb", '-'), (2, "-a\n-b\n\\ No newline at end of file\n".to_string()));
        assert_eq!(hunk_body("a\n", '+'), (1, "+a\n".to_string()));
        assert_eq!(hunk_body("", '+'), (0, String::new()));
    }

    #[test]
    fn groups_changes_into_separate_hunks_when_context_does_not_overlap() {
        let lines = parse_diff("@@ -1,8 +1,8 @@\n a\n b\n-c\n+C\n d\n e\n f\n-g\n+G\n h\n");
        let hunks = DiffHunk::grouped(&lines, 1);
        assert_eq!(hunks.len(), 2);
        assert_eq!(hunks[0].header(&lines), "@@ -2,3 +2,3 @@");
        assert_eq!(hunks[1].changed_indices, BTreeSet::from([8, 9]));
    }

    #[test]
    fn merges_changes_whose_context_overlaps() {
        let lines = parse_diff("@@ -1,8 +1,8 @@\n a\n b\n-c\n+C\n d\n e\n f\n-g\n+G\n h\n");
        let hunks = DiffHunk::grouped(&lines, 3);
        assert_eq!(hunks.len(), 1);
        assert_eq!(hunks[0].changed_indices, BTreeSet::from([3, 4, 8, 9]));
        assert_eq!(hunks[0].header(&lines), "@@ -1,8 +1,8 @@");
    }
}
