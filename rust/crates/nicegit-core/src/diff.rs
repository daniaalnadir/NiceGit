#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffLineKind {
    Metadata,
    Hunk,
    Context,
    Addition,
    Deletion,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffLine {
    pub text: String,
    pub kind: DiffLineKind,
    pub old_number: Option<usize>,
    pub new_number: Option<usize>,
}

/// One row of a side-by-side diff, referring to lines by their index in the unified list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SplitRow {
    /// A hunk header or file metadata, shown across both sides.
    Banner(usize),
    /// The old version's line on the left and the new version's on the right; either may be absent.
    Pair { left: Option<usize>, right: Option<usize> },
}

/// Lines of unchanged context shown around each change. Git's default applies to `Default`,
/// which is Git's own 3 lines or the user's `diff.context` setting.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DiffContext {
    #[default]
    Default,
    Lines(u32),
    /// Every line of the file, as the line-staging review reads it.
    WholeFile,
}

/// The context Git prints for `DiffContext::WholeFile`; the same value the line-staging review uses.
const WHOLE_FILE_CONTEXT: u32 = 1_000_000;

impl DiffContext {
    /// The Git argument that selects this context, or `None` to leave Git's default in place.
    pub fn argument(self) -> Option<String> {
        match self {
            Self::Default => None,
            Self::Lines(lines) => Some(format!("--unified={lines}")),
            Self::WholeFile => Some(format!("--unified={WHOLE_FILE_CONTEXT}")),
        }
    }
}

/// The options that change what a diff of a file, commit, or stash shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DiffRequest {
    /// Leave out whitespace-only changes.
    pub ignore_whitespace: bool,
    pub context: DiffContext,
}

/// Reads `@@ -a,b +c,d @@`, returning (old start, old count, new start, new count).
fn hunk_header(text: &str) -> Option<(usize, usize, usize, usize)> {
    let rest = text.strip_prefix("@@ -")?;
    let (ranges, _) = rest.split_once(" @@")?;
    let (old, new) = ranges.split_once(" +")?;
    let range = |value: &str| -> Option<(usize, usize)> {
        match value.split_once(',') {
            Some((start, count)) => Some((start.parse().ok()?, count.parse().ok()?)),
            None => Some((value.parse().ok()?, 1)),
        }
    };
    let (old_start, old_count) = range(old)?;
    let (new_start, new_count) = range(new)?;
    Some((old_start, old_count, new_start, new_count))
}

/// Classifies each line of a unified patch and numbers it. Line counts from the hunk header
/// decide where a hunk ends, so file content that looks like a header is not misread.
pub fn parse_diff(patch: &str) -> Vec<DiffLine> {
    if patch.is_empty() {
        return Vec::new();
    }
    let mut old: Option<usize> = None;
    let mut new: Option<usize> = None;
    let mut old_remaining = 0usize;
    let mut new_remaining = 0usize;
    let line = |text: &str, kind, old_number, new_number| DiffLine { text: text.to_string(), kind, old_number, new_number };
    patch
        .split('\n')
        .map(|text| {
            if let Some((old_start, old_count, new_start, new_count)) = hunk_header(text) {
                old = Some(old_start);
                new = Some(new_start);
                old_remaining = old_count;
                new_remaining = new_count;
                return line(text, DiffLineKind::Hunk, None, None);
            }
            if text.starts_with("diff --git ") || text.starts_with("@@@") {
                old = None;
                new = None;
            }
            let (Some(old_value), Some(new_value)) = (old, new) else {
                return line(text, DiffLineKind::Metadata, None, None);
            };
            if old_remaining == 0 && new_remaining == 0 {
                return line(text, DiffLineKind::Metadata, None, None);
            }
            if text.starts_with('+') && new_remaining > 0 {
                new = Some(new_value + 1);
                new_remaining -= 1;
                return line(text, DiffLineKind::Addition, None, Some(new_value));
            }
            if text.starts_with('-') && old_remaining > 0 {
                old = Some(old_value + 1);
                old_remaining -= 1;
                return line(text, DiffLineKind::Deletion, Some(old_value), None);
            }
            if (text.starts_with(' ') || text.is_empty()) && old_remaining > 0 && new_remaining > 0 {
                old = Some(old_value + 1);
                new = Some(new_value + 1);
                old_remaining -= 1;
                new_remaining -= 1;
                // diff.suppressBlankEmpty omits the prefix on empty context lines.
                let text = if text.is_empty() { " " } else { text };
                return line(text, DiffLineKind::Context, Some(old_value), Some(new_value));
            }
            line(text, DiffLineKind::Metadata, None, None)
        })
        .collect()
}

/// Arranges unified diff lines side by side. Unchanged lines appear on both sides, and each
/// run of removed lines pairs row by row with the added lines that follow it. Git's
/// missing-newline note belongs to the change around it, so it does not break the pairing.
pub fn side_by_side(lines: &[DiffLine]) -> Vec<SplitRow> {
    let mut rows = Vec::new();
    let is_note = |i: usize| lines[i].kind == DiffLineKind::Metadata && lines[i].text.starts_with('\\');
    let mut index = 0;
    while index < lines.len() {
        match lines[index].kind {
            DiffLineKind::Context => {
                rows.push(SplitRow::Pair { left: Some(index), right: Some(index) });
                index += 1;
            }
            DiffLineKind::Deletion | DiffLineKind::Addition => {
                let (mut removed, mut added, mut notes) = (Vec::new(), Vec::new(), Vec::new());
                while index < lines.len() && (lines[index].kind == DiffLineKind::Deletion || is_note(index)) {
                    if is_note(index) {
                        notes.push(index)
                    } else {
                        removed.push(index)
                    }
                    index += 1;
                }
                while index < lines.len() && (lines[index].kind == DiffLineKind::Addition || is_note(index)) {
                    if is_note(index) {
                        notes.push(index)
                    } else {
                        added.push(index)
                    }
                    index += 1;
                }
                for row in 0..removed.len().max(added.len()) {
                    rows.push(SplitRow::Pair { left: removed.get(row).copied(), right: added.get(row).copied() });
                }
                rows.extend(notes.into_iter().map(SplitRow::Banner));
            }
            DiffLineKind::Hunk | DiffLineKind::Metadata => {
                rows.push(SplitRow::Banner(index));
                index += 1;
            }
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_lines_and_stops_at_hunk_end() {
        let patch = "diff --git a/f b/f\n--- a/f\n+++ b/f\n@@ -1,2 +1,2 @@\n keep\n-old\n+new\n-- not part of hunk";
        let lines = parse_diff(patch);
        assert_eq!(lines[3].kind, DiffLineKind::Hunk);
        assert_eq!(lines[4].kind, DiffLineKind::Context);
        assert_eq!((lines[4].old_number, lines[4].new_number), (Some(1), Some(1)));
        assert_eq!(lines[5].kind, DiffLineKind::Deletion);
        assert_eq!(lines[5].old_number, Some(2));
        assert_eq!(lines[6].kind, DiffLineKind::Addition);
        assert_eq!(lines[6].new_number, Some(2));
        assert_eq!(lines[7].kind, DiffLineKind::Metadata);
        // Header lines before the hunk are metadata even though they start with - and +.
        assert_eq!(lines[1].kind, DiffLineKind::Metadata);
    }

    #[test]
    fn keeps_suppressed_blank_context_lines() {
        let lines = parse_diff("@@ -1,3 +1,3 @@\n a\n\n-b\n+c");
        assert_eq!(lines[2].kind, DiffLineKind::Context);
        assert_eq!(lines[2].text, " ");
        assert_eq!(lines[3].old_number, Some(3));
    }

    #[test]
    fn pairs_replacements_across_missing_newline_notes() {
        let lines = parse_diff("@@ -1 +1 @@\n-old\n\\ No newline at end of file\n+new\n\\ No newline at end of file");
        let rows = side_by_side(&lines);
        assert_eq!(rows[1], SplitRow::Pair { left: Some(1), right: Some(3) });
        assert_eq!(rows[2], SplitRow::Banner(2));
        assert_eq!(rows[3], SplitRow::Banner(4));
    }
}
