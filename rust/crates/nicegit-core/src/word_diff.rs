//! Word-level diff rows: each run of changed lines becomes merged lines, where the words removed
//! from the old line and the words added in the new one sit together in one row. Good for prose
//! and long lines, where a whole-line diff hides which few words changed.

use crate::diff::{DiffLine, DiffLineKind};

/// Most tokens in either line of a pair that are compared word by word. Longer pairs fall back
/// to showing the whole old line, then the whole new one.
const MAX_LINE_TOKENS: usize = 2000;
/// Most tokens in one run of changed lines that are compared word by word.
const MAX_BLOCK_TOKENS: usize = 20_000;

/// One row of a word diff, referring to lines by their index in the unified list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WordRow {
    /// An unchanged line.
    Context(usize),
    /// A hunk header, file header, or Git's missing-newline note, shown as it is.
    Other(usize),
    /// Removed and added lines merged into one row. Either number is absent for a line that
    /// has no counterpart.
    Merged { old_line: Option<usize>, new_line: Option<usize>, segments: Vec<WordSegment>, source_lines: Vec<usize> },
}

/// A run of text in a merged row, and whether it is unchanged, removed or added.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WordSegment {
    pub text: String,
    pub kind: WordKind,
}

/// Whether a segment of a merged row is in both versions, only the old one, or only the new.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WordKind {
    Same,
    Removed,
    Added,
}

/// The kind of character a token is made of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CharKind {
    Word,
    Space,
    Punctuation,
}

impl CharKind {
    fn of(ch: char) -> Self {
        if ch.is_whitespace() {
            Self::Space
        } else if ch.is_alphanumeric() || ch == '_' {
            Self::Word
        } else {
            Self::Punctuation
        }
    }
}

/// Arranges a diff as word rows. Each run of removed lines pairs one-to-one, in order, with the
/// added lines that follow it; leftover lines become rows of only removed or only added words.
/// Git's missing-newline note belongs to the change around it, so it does not break a pairing.
pub fn word_rows(lines: &[DiffLine]) -> Vec<WordRow> {
    let mut rows = Vec::new();
    let is_note = |i: usize| lines[i].kind == DiffLineKind::Metadata && lines[i].text.starts_with('\\');
    let mut index = 0;
    while index < lines.len() {
        match lines[index].kind {
            DiffLineKind::Context => {
                rows.push(WordRow::Context(index));
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
                rows.extend(pair_rows(lines, &removed, &added));
                rows.extend(notes.into_iter().map(WordRow::Other));
            }
            DiffLineKind::Hunk | DiffLineKind::Metadata => {
                rows.push(WordRow::Other(index));
                index += 1;
            }
        }
    }
    rows
}

/// The merged rows for one run of removed and added lines.
fn pair_rows(lines: &[DiffLine], removed: &[usize], added: &[usize]) -> Vec<WordRow> {
    let removed_tokens: Vec<Vec<&str>> = removed.iter().map(|&i| tokenize(content(&lines[i]))).collect();
    let added_tokens: Vec<Vec<&str>> = added.iter().map(|&i| tokenize(content(&lines[i]))).collect();
    let block_tokens: usize = removed_tokens.iter().chain(&added_tokens).map(Vec::len).sum();
    let whole_block = block_tokens > MAX_BLOCK_TOKENS;
    let mut rows = Vec::new();
    for row in 0..removed.len().max(added.len()) {
        let old = removed.get(row).copied();
        let new = added.get(row).copied();
        let row = match (old, new) {
            (Some(old), Some(new)) => {
                let (old_tokens, new_tokens) = (&removed_tokens[row], &added_tokens[row]);
                let long = old_tokens.len().max(new_tokens.len()) > MAX_LINE_TOKENS;
                let segments = if whole_block || long {
                    whole_line(content(&lines[old]), content(&lines[new]))
                } else {
                    merge_tokens(old_tokens, new_tokens)
                };
                WordRow::Merged { old_line: lines[old].old_number, new_line: lines[new].new_number, segments, source_lines: vec![old, new] }
            }
            (Some(old), None) => WordRow::Merged {
                old_line: lines[old].old_number,
                new_line: None,
                segments: single(content(&lines[old]), WordKind::Removed),
                source_lines: vec![old],
            },
            (None, Some(new)) => WordRow::Merged {
                old_line: None,
                new_line: lines[new].new_number,
                segments: single(content(&lines[new]), WordKind::Added),
                source_lines: vec![new],
            },
            (None, None) => continue,
        };
        rows.push(row);
    }
    rows
}

/// A line's content without its one-character sign.
fn content(line: &DiffLine) -> &str {
    line.text.get(1..).unwrap_or("")
}

/// Splits text into tokens: a run of word characters, a run of whitespace, or one punctuation
/// character. Characters are taken whole, so multi-byte text never splits inside a character.
fn tokenize(text: &str) -> Vec<&str> {
    let mut tokens = Vec::new();
    let mut start = 0;
    let mut previous: Option<CharKind> = None;
    for (index, ch) in text.char_indices() {
        let kind = CharKind::of(ch);
        let joins = previous == Some(kind) && kind != CharKind::Punctuation;
        if !joins && index > start {
            tokens.push(&text[start..index]);
            start = index;
        }
        previous = Some(kind);
    }
    if start < text.len() {
        tokens.push(&text[start..]);
    }
    tokens
}

/// Appends a segment, joining it onto the previous one when both have the same kind.
fn push_segment(segments: &mut Vec<WordSegment>, kind: WordKind, text: &str) {
    if text.is_empty() {
        return;
    }
    match segments.last_mut() {
        Some(last) if last.kind == kind => last.text.push_str(text),
        _ => segments.push(WordSegment { text: text.to_string(), kind }),
    }
}

/// A single segment of one kind, or nothing for an empty line.
fn single(text: &str, kind: WordKind) -> Vec<WordSegment> {
    let mut segments = Vec::new();
    push_segment(&mut segments, kind, text);
    segments
}

/// The old line as removed, then the new line as added, for a pair too long to compare by word.
fn whole_line(old: &str, new: &str) -> Vec<WordSegment> {
    let mut segments = single(old, WordKind::Removed);
    push_segment(&mut segments, WordKind::Added, new);
    segments
}

/// Merges two lines' tokens into segments. Shared text at the start and end is kept as it is,
/// and the words between are compared with a longest common subsequence.
fn merge_tokens(old: &[&str], new: &[&str]) -> Vec<WordSegment> {
    let prefix = old.iter().zip(new).take_while(|(a, b)| a == b).count();
    let suffix = old[prefix..].iter().rev().zip(new[prefix..].iter().rev()).take_while(|(a, b)| a == b).count();
    let mut segments = Vec::new();
    for token in &old[..prefix] {
        push_segment(&mut segments, WordKind::Same, token);
    }
    for (kind, token) in edit_script(&old[prefix..old.len() - suffix], &new[prefix..new.len() - suffix]) {
        push_segment(&mut segments, kind, token);
    }
    for token in &old[old.len() - suffix..] {
        push_segment(&mut segments, WordKind::Same, token);
    }
    segments
}

/// The tokens that are shared, removed or added to turn `old` into `new`, in order.
fn edit_script<'a>(old: &[&'a str], new: &[&'a str]) -> Vec<(WordKind, &'a str)> {
    let (m, n) = (old.len(), new.len());
    let width = n + 1;
    // lcs[i * width + j] is the longest common subsequence of old[i..] and new[j..].
    let mut lcs = vec![0u16; (m + 1) * width];
    for i in (0..m).rev() {
        for j in (0..n).rev() {
            lcs[i * width + j] =
                if old[i] == new[j] { lcs[(i + 1) * width + j + 1] + 1 } else { lcs[(i + 1) * width + j].max(lcs[i * width + j + 1]) };
        }
    }
    let mut script = Vec::with_capacity(m + n);
    let (mut i, mut j) = (0, 0);
    while i < m || j < n {
        if i < m && j < n && old[i] == new[j] {
            script.push((WordKind::Same, old[i]));
            i += 1;
            j += 1;
        } else if j == n || (i < m && lcs[(i + 1) * width + j] >= lcs[i * width + j + 1]) {
            script.push((WordKind::Removed, old[i]));
            i += 1;
        } else {
            script.push((WordKind::Added, new[j]));
            j += 1;
        }
    }
    script
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(text: &str, kind: DiffLineKind) -> DiffLine {
        DiffLine { text: text.to_string(), kind, old_number: None, new_number: None }
    }

    fn numbered(text: &str, kind: DiffLineKind, old: Option<usize>, new: Option<usize>) -> DiffLine {
        DiffLine { text: text.to_string(), kind, old_number: old, new_number: new }
    }

    fn seg(text: &str, kind: WordKind) -> WordSegment {
        WordSegment { text: text.to_string(), kind }
    }

    fn segments_of(row: &WordRow) -> &[WordSegment] {
        match row {
            WordRow::Merged { segments, .. } => segments,
            other => panic!("expected a merged row, got {other:?}"),
        }
    }

    #[test]
    fn replaces_only_the_changed_word_in_a_sentence() {
        let lines = [line("-the quick brown fox", DiffLineKind::Deletion), line("+the slow brown fox", DiffLineKind::Addition)];
        let rows = word_rows(&lines);
        assert_eq!(rows.len(), 1);
        assert_eq!(
            segments_of(&rows[0]),
            [seg("the ", WordKind::Same), seg("quick", WordKind::Removed), seg("slow", WordKind::Added), seg(" brown fox", WordKind::Same),]
        );
    }

    #[test]
    fn keeps_the_line_numbers_of_both_sides() {
        let lines =
            [numbered("-old text", DiffLineKind::Deletion, Some(7), None), numbered("+new text", DiffLineKind::Addition, None, Some(9))];
        let rows = word_rows(&lines);
        assert_eq!(
            rows[0],
            WordRow::Merged {
                old_line: Some(7),
                new_line: Some(9),
                segments: vec![seg("old", WordKind::Removed), seg("new", WordKind::Added), seg(" text", WordKind::Same)],
                source_lines: vec![0, 1],
            }
        );
    }

    #[test]
    fn unpaired_lines_become_rows_of_only_removed_or_added_words() {
        let lines = [
            line("-gone one", DiffLineKind::Deletion),
            line("-gone two", DiffLineKind::Deletion),
            line("+fresh", DiffLineKind::Addition),
            line(" kept", DiffLineKind::Context),
            line("+extra", DiffLineKind::Addition),
        ];
        let rows = word_rows(&lines);
        assert_eq!(rows.len(), 4);
        assert_eq!(segments_of(&rows[1]), [seg("gone two", WordKind::Removed)]);
        assert_eq!(rows[2], WordRow::Context(3));
        assert_eq!(segments_of(&rows[3]), [seg("extra", WordKind::Added)]);
        assert_eq!(
            rows[1],
            WordRow::Merged { old_line: None, new_line: None, segments: vec![seg("gone two", WordKind::Removed)], source_lines: vec![1] }
        );
    }

    #[test]
    fn headers_and_context_keep_their_own_rows() {
        let lines = [
            line("@@ -1 +1 @@", DiffLineKind::Hunk),
            line(" same", DiffLineKind::Context),
            line("-a b", DiffLineKind::Deletion),
            line("+a c", DiffLineKind::Addition),
        ];
        let rows = word_rows(&lines);
        assert_eq!(rows[0], WordRow::Other(0));
        assert_eq!(rows[1], WordRow::Context(1));
        assert!(matches!(rows[2], WordRow::Merged { .. }));
    }

    #[test]
    fn words_are_split_on_word_space_and_punctuation_runs() {
        assert_eq!(tokenize("let x_1 = f(a,  b);"), ["let", " ", "x_1", " ", "=", " ", "f", "(", "a", ",", "  ", "b", ")", ";"]);
        assert_eq!(tokenize(""), Vec::<&str>::new());
    }

    #[test]
    fn accented_and_cjk_words_stay_whole() {
        assert_eq!(tokenize("café au lait"), ["café", " ", "au", " ", "lait"]);
        assert_eq!(tokenize("日本語 です"), ["日本語", " ", "です"]);
        let lines = [line("-今日は日本", DiffLineKind::Deletion), line("+今日は中国", DiffLineKind::Addition)];
        let rows = word_rows(&lines);
        assert_eq!(segments_of(&rows[0]), [seg("今日は日本", WordKind::Removed), seg("今日は中国", WordKind::Added)]);
    }

    #[test]
    fn pairs_across_the_missing_newline_note() {
        let lines = [
            line("-value = old", DiffLineKind::Deletion),
            line("\\ No newline at end of file", DiffLineKind::Metadata),
            line("+value = new", DiffLineKind::Addition),
            line("\\ No newline at end of file", DiffLineKind::Metadata),
        ];
        let rows = word_rows(&lines);
        assert_eq!(rows.len(), 3);
        assert_eq!(segments_of(&rows[0]), [seg("value = ", WordKind::Same), seg("old", WordKind::Removed), seg("new", WordKind::Added)]);
        assert_eq!(rows[1], WordRow::Other(1));
        assert_eq!(rows[2], WordRow::Other(3));
    }

    #[test]
    fn a_pair_over_the_token_limit_shows_whole_lines() {
        let old = format!("-{}", "a ".repeat(MAX_LINE_TOKENS));
        let new = format!("+{}b", "a ".repeat(MAX_LINE_TOKENS));
        let lines = [line(&old, DiffLineKind::Deletion), line(&new, DiffLineKind::Addition)];
        let rows = word_rows(&lines);
        let segments = segments_of(&rows[0]);
        assert_eq!(segments.len(), 2);
        assert_eq!(segments[0].kind, WordKind::Removed);
        assert_eq!(segments[1].kind, WordKind::Added);
        assert_eq!(segments[1].text, format!("{}b", "a ".repeat(MAX_LINE_TOKENS)));
    }

    #[test]
    fn a_block_over_the_total_limit_shows_whole_lines_for_every_pair() {
        let text = "x ".repeat(MAX_BLOCK_TOKENS / 2 + 10);
        let lines = [line(&format!("-{text}one"), DiffLineKind::Deletion), line(&format!("+{text}two"), DiffLineKind::Addition)];
        let rows = word_rows(&lines);
        assert_eq!(segments_of(&rows[0]).len(), 2);
        assert_eq!(segments_of(&rows[0])[0].text, format!("{text}one"));
    }

    #[test]
    fn identical_content_is_all_shared() {
        let lines = [line("-same words", DiffLineKind::Deletion), line("+same words", DiffLineKind::Addition)];
        assert_eq!(segments_of(&word_rows(&lines)[0]), [seg("same words", WordKind::Same)]);
    }

    #[test]
    fn empty_lines_have_no_segments() {
        let lines = [line("-", DiffLineKind::Deletion), line("+", DiffLineKind::Addition)];
        assert!(segments_of(&word_rows(&lines)[0]).is_empty());
    }
}
