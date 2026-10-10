//! Character ranges that changed inside a deleted line and the added line that replaces it, so
//! the diff view can highlight only the words that differ.

use std::collections::BTreeMap;

use crate::diff::{DiffLine, DiffLineKind};

/// A changed line split into the text shared with its counterpart before the change, the
/// changed text, and the shared text after it. The parts concatenate to the line's content,
/// without its leading `+` or `-`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InlineChange {
    pub prefix: String,
    pub changed: String,
    pub suffix: String,
}

/// Git's marker for a line that has no final newline. It belongs to the change around it.
const NO_NEWLINE: &str = "\\ No newline at end of file";

/// Highlights for every deletion and addition that has a counterpart, keyed by line index.
/// Deletions and additions pair by position within each run of changes, so a replacement
/// keeps its pairing even across Git's missing-newline marker.
pub fn highlights(lines: &[DiffLine]) -> BTreeMap<usize, InlineChange> {
    let mut result = BTreeMap::new();
    let mut removed: Vec<usize> = Vec::new();
    let mut added: Vec<usize> = Vec::new();

    fn flush(lines: &[DiffLine], removed: &mut Vec<usize>, added: &mut Vec<usize>, result: &mut BTreeMap<usize, InlineChange>) {
        for (&old, &new) in removed.iter().zip(added.iter()) {
            if let Some((old_change, new_change)) = changed_parts(content(&lines[old]), content(&lines[new])) {
                result.insert(old, old_change);
                result.insert(new, new_change);
            }
        }
        removed.clear();
        added.clear();
    }

    for (index, line) in lines.iter().enumerate() {
        match line.kind {
            DiffLineKind::Deletion => removed.push(index),
            DiffLineKind::Addition => added.push(index),
            DiffLineKind::Metadata if line.text == NO_NEWLINE => continue,
            _ => flush(lines, &mut removed, &mut added, &mut result),
        }
    }
    flush(lines, &mut removed, &mut added, &mut result);
    result
}

/// A line's content without its one-character sign.
fn content(line: &DiffLine) -> &str {
    line.text.get(1..).unwrap_or("")
}

fn changed_parts(old: &str, new: &str) -> Option<(InlineChange, InlineChange)> {
    let before: Vec<char> = old.chars().collect();
    let after: Vec<char> = new.chars().collect();
    let prefix_count = before.iter().zip(after.iter()).take_while(|(a, b)| a == b).count();
    let suffix_count = before[prefix_count..].iter().rev().zip(after[prefix_count..].iter().rev()).take_while(|(a, b)| a == b).count();
    let shared = before[..prefix_count].iter().chain(before[before.len() - suffix_count..].iter());
    // A single shared letter is a coincidence, not a shared word; require two visible characters.
    if shared.filter(|c| !c.is_whitespace()).count() < 2 {
        return None;
    }

    let parts = |characters: &[char]| InlineChange {
        prefix: characters[..prefix_count].iter().collect(),
        changed: characters[prefix_count..characters.len() - suffix_count].iter().collect(),
        suffix: characters[characters.len() - suffix_count..].iter().collect(),
    };
    Some((parts(&before), parts(&after)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(text: &str, kind: DiffLineKind) -> DiffLine {
        DiffLine { text: text.to_string(), kind, old_number: None, new_number: None }
    }

    #[test]
    fn finds_the_changed_middle_of_a_replacement() {
        let lines = [line("-let value = 1;", DiffLineKind::Deletion), line("+let value = 2;", DiffLineKind::Addition)];
        let result = highlights(&lines);
        assert_eq!(result[&0], InlineChange { prefix: "let value = ".into(), changed: "1".into(), suffix: ";".into() });
        assert_eq!(result[&1].changed, "2");
    }

    #[test]
    fn unpaired_lines_have_no_highlight() {
        let lines = [line("-gone", DiffLineKind::Deletion), line(" kept", DiffLineKind::Context), line("+new", DiffLineKind::Addition)];
        assert!(highlights(&lines).is_empty());
    }

    #[test]
    fn pairs_by_position_within_a_run() {
        let lines = [
            line("-alpha one", DiffLineKind::Deletion),
            line("-beta two", DiffLineKind::Deletion),
            line("+alpha uno", DiffLineKind::Addition),
            line("+beta two", DiffLineKind::Addition),
        ];
        let result = highlights(&lines);
        assert_eq!(result[&0].changed, "one");
        assert_eq!(result[&2].changed, "uno");
        // The second pair has identical content, so nothing inside it changed.
        assert!(result.contains_key(&1));
        assert_eq!(result[&1].changed, "");
    }

    #[test]
    fn keeps_pairing_across_missing_newline_marker() {
        let lines = [
            line("-value = old", DiffLineKind::Deletion),
            line("\\ No newline at end of file", DiffLineKind::Metadata),
            line("+value = new", DiffLineKind::Addition),
        ];
        let result = highlights(&lines);
        assert_eq!(result[&0].changed, "old");
        assert_eq!(result[&2].changed, "new");
    }

    #[test]
    fn single_shared_letter_is_not_a_shared_word() {
        let lines = [line("-a", DiffLineKind::Deletion), line("+b", DiffLineKind::Addition)];
        assert!(highlights(&lines).is_empty());
    }

    #[test]
    fn multibyte_characters_split_on_character_boundaries() {
        let lines = [line("-café ☕ old", DiffLineKind::Deletion), line("+café ☕ new", DiffLineKind::Addition)];
        let result = highlights(&lines);
        assert_eq!(result[&0].prefix, "café ☕ ");
        assert_eq!(result[&0].changed, "old");
        assert_eq!(result[&1].changed, "new");
    }
}
