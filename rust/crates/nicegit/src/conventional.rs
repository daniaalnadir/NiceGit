//! Conventional Commits helpers for the commit summary: reading the `type(scope)!: ` prefix,
//! rewriting it, deciding whether a repository follows the convention, and the summary length note.
//! Everything here is pure; the commit box keeps the summary as the only source of truth.

use serde::{Deserialize, Serialize};

/// Whether the commit box offers Conventional Commits help.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ConventionalCommits {
    Off,
    /// On when the repository's recent commits mostly follow the convention.
    #[default]
    Automatic,
    Always,
}

impl ConventionalCommits {
    pub const ALL: [ConventionalCommits; 3] = [ConventionalCommits::Off, ConventionalCommits::Automatic, ConventionalCommits::Always];

    pub fn title(self) -> &'static str {
        match self {
            ConventionalCommits::Off => "Off",
            ConventionalCommits::Automatic => "Automatic",
            ConventionalCommits::Always => "Always",
        }
    }
}

/// The commit types and what each is for, in the order the type menu lists them.
pub const TYPES: [(&str, &str); 11] = [
    ("feat", "A new feature"),
    ("fix", "A bug fix"),
    ("docs", "Documentation only"),
    ("style", "Formatting, no code change"),
    ("refactor", "Code change that neither fixes nor adds"),
    ("perf", "Faster code"),
    ("test", "Tests"),
    ("build", "Build system or dependencies"),
    ("ci", "CI configuration"),
    ("chore", "Other changes"),
    ("revert", "Reverts a commit"),
];

/// Recent commits this many must match, out of the last [`DETECTION_WINDOW`] non-merge commits, for
/// the repository to count as using the convention.
const DETECTION_MATCHES: usize = 5;
const DETECTION_WINDOW: usize = 20;

/// The summary guideline: the counter appears past this many characters.
pub const LENGTH_NOTE_FROM: usize = 50;
/// The limit that many tools cut summaries at; the counter turns to a warning past it.
pub const LENGTH_LIMIT: usize = 72;

/// The prefix of a summary that follows the convention, such as `fix(ui)!: `.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prefix {
    pub kind: &'static str,
    pub scope: Option<String>,
    pub breaking: bool,
    /// The summary after the prefix. Never empty for a prefix read by [`parse`].
    pub rest: String,
}

/// Splits a valid prefix off the start of `summary`: the type, the optional scope, the optional `!`,
/// and the `: ` that follows. Returns the parts and the byte length they span, whether or not any
/// text follows the colon.
fn split_prefix(summary: &str) -> Option<(&'static str, Option<&str>, bool, usize)> {
    let word_end = summary.find(|c: char| !c.is_ascii_lowercase()).unwrap_or(summary.len());
    let kind = TYPES.iter().map(|(name, _)| *name).find(|name| *name == &summary[..word_end])?;
    let mut position = word_end;
    let mut scope = None;
    if let Some(after) = summary[position..].strip_prefix('(') {
        let close = after.find(')')?;
        let text = &after[..close];
        if text.is_empty() {
            return None;
        }
        scope = Some(text);
        // The opening bracket, the scope, and the closing bracket.
        position += close + 2;
    }
    let mut breaking = false;
    if summary[position..].starts_with('!') {
        breaking = true;
        position += 1;
    }
    summary[position..].strip_prefix(": ")?;
    position += 2;
    Some((kind, scope, breaking, position))
}

/// Reads the convention prefix of `summary`, which must be followed by some text, as the pattern
/// `^(feat|fix|...)(\([^)]+\))?!?: .+` does. Unknown types give `None`.
pub fn parse(summary: &str) -> Option<Prefix> {
    let (kind, scope, breaking, end) = split_prefix(summary)?;
    let rest = &summary[end..];
    if rest.is_empty() {
        return None;
    }
    Some(Prefix { kind, scope: scope.map(str::to_string), breaking, rest: rest.to_string() })
}

/// Reads the convention prefix of `summary` even when no text follows it, such as `fix: ` just
/// after a type is chosen. The commit box uses this to show the current type.
pub fn prefix(summary: &str) -> Option<Prefix> {
    let (kind, scope, breaking, end) = split_prefix(summary)?;
    Some(Prefix { kind, scope: scope.map(str::to_string), breaking, rest: summary[end..].to_string() })
}

/// Returns `summary` with its convention prefix set to `kind`, `scope`, and `breaking`. An existing
/// prefix is replaced, and one is added when there is none; the rest of the summary is kept. An
/// empty or blank scope leaves out the brackets. Brackets and line breaks are removed from the scope.
pub fn with_prefix(summary: &str, kind: &str, scope: Option<&str>, breaking: bool) -> String {
    let rest = match split_prefix(summary) {
        Some((_, _, _, end)) => &summary[end..],
        None => summary,
    };
    let mut out = String::from(kind);
    let scope: String = scope.unwrap_or("").chars().filter(|c| *c != ')' && *c != '(' && !c.is_control()).collect();
    let scope = scope.trim();
    if !scope.is_empty() {
        out.push('(');
        out.push_str(scope);
        out.push(')');
    }
    if breaking {
        out.push('!');
    }
    out.push_str(": ");
    out.push_str(rest);
    out
}

/// Whether the repository follows the convention: at least five of its last twenty non-merge
/// summaries match. Pass the summaries newest first, already without merge commits.
pub fn repository_uses_convention<'a>(summaries: impl Iterator<Item = &'a str>) -> bool {
    summaries.take(DETECTION_WINDOW).filter(|summary| parse(summary).is_some()).count() >= DETECTION_MATCHES
}

/// The length note under the summary: `(text, over the limit)`, or `None` up to the guideline.
/// Length counts characters, not bytes.
pub fn length_note(summary: &str) -> Option<(String, bool)> {
    let length = summary.chars().count();
    if length <= LENGTH_NOTE_FROM {
        return None;
    }
    if length > LENGTH_LIMIT {
        Some((format!("{length}/{LENGTH_LIMIT} · long summaries are cut off in many tools"), true))
    } else {
        Some((format!("{length}/{LENGTH_LIMIT}"), false))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_type_alone() {
        let parsed = parse("fix: correct the count").unwrap();
        assert_eq!(parsed.kind, "fix");
        assert_eq!(parsed.scope, None);
        assert!(!parsed.breaking);
        assert_eq!(parsed.rest, "correct the count");
    }

    #[test]
    fn parses_a_scope_and_breaking_marker() {
        let parsed = parse("feat(api)!: drop the v1 routes").unwrap();
        assert_eq!(parsed.kind, "feat");
        assert_eq!(parsed.scope.as_deref(), Some("api"));
        assert!(parsed.breaking);
        assert_eq!(parsed.rest, "drop the v1 routes");
    }

    #[test]
    fn parses_a_breaking_marker_without_a_scope() {
        let parsed = parse("refactor!: rename things").unwrap();
        assert_eq!(parsed.scope, None);
        assert!(parsed.breaking);
    }

    #[test]
    fn unknown_types_and_malformed_prefixes_are_not_parsed() {
        assert_eq!(parse("feature: new thing"), None);
        assert_eq!(parse("Fix: capital letters do not count"), None);
        assert_eq!(parse("fix(): empty scope"), None);
        assert_eq!(parse("fix(ui: unclosed scope"), None);
        assert_eq!(parse("fix:no space"), None);
        assert_eq!(parse("fix: "), None, "a prefix with no text after it is not a summary");
        assert_eq!(parse("Add a thing"), None);
        assert_eq!(parse(""), None);
    }

    #[test]
    fn prefix_reads_a_type_that_has_no_text_yet() {
        let parsed = prefix("fix(ui): ").unwrap();
        assert_eq!(parsed.kind, "fix");
        assert_eq!(parsed.scope.as_deref(), Some("ui"));
        assert_eq!(parsed.rest, "");
        assert_eq!(prefix("Add a thing"), None);
    }

    #[test]
    fn with_prefix_adds_a_type_when_there_is_none() {
        assert_eq!(with_prefix("Add a thing", "fix", None, false), "fix: Add a thing");
        assert_eq!(with_prefix("", "docs", None, false), "docs: ");
        assert_eq!(with_prefix("feature: not a type", "fix", None, false), "fix: feature: not a type");
    }

    #[test]
    fn with_prefix_replaces_the_type_and_keeps_the_rest() {
        assert_eq!(with_prefix("feat: add the menu", "fix", None, false), "fix: add the menu");
        assert_eq!(with_prefix("feat(ui)!: add the menu", "fix", Some("ui"), false), "fix(ui): add the menu");
    }

    #[test]
    fn with_prefix_adds_and_removes_scope_and_breaking() {
        assert_eq!(with_prefix("fix: keep it", "fix", Some("ui"), false), "fix(ui): keep it");
        assert_eq!(with_prefix("fix(ui): keep it", "fix", None, false), "fix: keep it");
        assert_eq!(with_prefix("fix(ui): keep it", "fix", Some("ui"), true), "fix(ui)!: keep it");
        assert_eq!(with_prefix("fix(ui)!: keep it", "fix", Some("ui"), false), "fix(ui): keep it");
        assert_eq!(with_prefix("fix: keep it", "fix", None, true), "fix!: keep it");
    }

    #[test]
    fn with_prefix_drops_blank_scopes_and_strips_brackets() {
        assert_eq!(with_prefix("fix: a", "fix", Some("   "), false), "fix: a");
        assert_eq!(with_prefix("fix: a", "fix", Some("a) b"), false), "fix(a b): a");
        assert_eq!(with_prefix("fix: a", "fix", Some(" ui "), false), "fix(ui): a");
    }

    #[test]
    fn with_prefix_round_trips_through_parse() {
        let summary = with_prefix("Keep the panel open", "perf", Some("graph"), true);
        let parsed = parse(&summary).unwrap();
        assert_eq!(parsed.kind, "perf");
        assert_eq!(parsed.scope.as_deref(), Some("graph"));
        assert!(parsed.breaking);
        assert_eq!(parsed.rest, "Keep the panel open");
    }

    #[test]
    fn detection_needs_five_of_the_last_twenty_summaries() {
        let four = ["feat: a", "fix: b", "fix: c", "fix: d", "Plain", "Plain", "Plain"];
        assert!(!repository_uses_convention(four.iter().copied()));
        let five = ["feat: a", "fix: b", "fix: c", "fix: d", "docs: e", "Plain"];
        assert!(repository_uses_convention(five.iter().copied()));
    }

    #[test]
    fn detection_looks_only_at_the_newest_twenty() {
        let mut summaries = vec!["Plain"; 20];
        summaries.extend(["fix: old"; 10]);
        assert!(!repository_uses_convention(summaries.iter().copied()));
    }

    #[test]
    fn length_counts_characters_not_bytes() {
        // Fifty accented letters are 100 bytes but 50 characters: no note yet.
        assert_eq!(length_note(&"é".repeat(50)), None);
        let (text, over) = length_note(&"é".repeat(51)).unwrap();
        assert_eq!(text, "51/72");
        assert!(!over);
        let (text, over) = length_note(&"😀".repeat(78)).unwrap();
        assert!(text.starts_with("78/72"));
        assert!(over);
    }

    #[test]
    fn length_note_warns_past_seventy_two() {
        assert_eq!(length_note(&"a".repeat(72)), Some(("72/72".to_string(), false)));
        assert_eq!(length_note(&"a".repeat(78)).unwrap().0, "78/72 · long summaries are cut off in many tools");
        assert!(length_note(&"a".repeat(78)).unwrap().1);
    }
}
