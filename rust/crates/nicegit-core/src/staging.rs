//! Staging and unstaging individual lines of a file. The selected lines are turned into a
//! full-file patch against the index, which `git apply --cached` checks against the index
//! before writing anything.

use std::collections::BTreeSet;
use std::ops::Range;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::client::GitClient;
use crate::diff::{parse_diff, DiffLine, DiffLineKind};
use crate::models::{GitError, Result, StatusKind};

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
        apply_to_index(self, &patch, directory)
    }
}

fn review_error(message: &str) -> GitError {
    GitError::failed("file review", message)
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

/// Writes the patch to a temporary file, applies it to the index, and removes the file.
fn apply_to_index(client: &GitClient, patch: &str, directory: &Path) -> Result<()> {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|time| time.as_nanos()).unwrap_or(0);
    let sequence = PATCH_COUNTER.fetch_add(1, Ordering::Relaxed);
    let file = std::env::temp_dir().join(format!("nicegit-lines-{}-{nanos}-{sequence}.patch", std::process::id()));
    std::fs::write(&file, patch.as_bytes()).map_err(|error| GitError::failed("stage lines", error.to_string()))?;
    let file_text = file.to_string_lossy().into_owned();
    let result = client.run(&["apply", "--cached", "--whitespace=nowarn", "--", &file_text], directory).map(drop);
    let _ = std::fs::remove_file(&file);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

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
