//! Comparing two versions of a series of commits, as `git range-diff` does: which commits stayed
//! the same, which changed, and which were added or removed, after a rebase, amend, or force push.

use std::path::Path;

use crate::client::GitClient;
use crate::models::Result;

/// How many reflog entries are read when looking for an earlier version of a branch.
const REFLOG_SCAN_LIMIT: &str = "50";

/// The status range-diff gives a pair of commits, shown as its one-character symbol.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RangeDiffStatus {
    /// `=`: the commits are the same, so only the message may differ.
    Same,
    /// `!`: both sides exist but the change differs.
    Changed,
    /// `<`: the commit exists only in the old series.
    Removed,
    /// `>`: the commit exists only in the new series.
    Added,
}

impl RangeDiffStatus {
    /// The one-character symbol range-diff prints for this status.
    pub fn symbol(self) -> &'static str {
        match self {
            Self::Same => "=",
            Self::Changed => "!",
            Self::Removed => "<",
            Self::Added => ">",
        }
    }

    /// The status for range-diff's symbol, or `None` for any other character.
    fn from_symbol(symbol: &str) -> Option<Self> {
        match symbol {
            "=" => Some(Self::Same),
            "!" => Some(Self::Changed),
            "<" => Some(Self::Removed),
            ">" => Some(Self::Added),
            _ => None,
        }
    }
}

/// One commit on one side of a pair, as range-diff printed it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RangeDiffCommit {
    /// The commit's position in its series, counted from 1.
    pub index: u32,
    /// The abbreviated object ID range-diff printed.
    pub short: String,
}

/// A pair of commits, or a commit with no partner, and how they compare.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RangeDiffEntry {
    /// The commit in the old series, or `None` when it was added.
    pub old: Option<RangeDiffCommit>,
    /// The commit in the new series, or `None` when it was removed.
    pub new: Option<RangeDiffCommit>,
    pub status: RangeDiffStatus,
    /// The commit's subject, as range-diff printed it.
    pub subject: String,
    /// Range-diff's comparison of the two patches, with its four-space indent removed. Only a
    /// changed pair has these lines; the outer `+` and `-` mark what the patches gained or lost.
    pub diff_lines: Vec<String>,
}

/// The comparison of two series, in range-diff's order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RangeDiff {
    pub entries: Vec<RangeDiffEntry>,
}

impl RangeDiff {
    /// How many entries have `status`.
    pub fn count(&self, status: RangeDiffStatus) -> usize {
        self.entries.iter().filter(|entry| entry.status == status).count()
    }
}

/// The four revisions a range-diff compares: each series is `base..tip`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RangeDiffInputs {
    pub old_base: String,
    pub old_tip: String,
    pub new_base: String,
    pub new_tip: String,
}

impl GitClient {
    /// Compares the series `old_base..old_tip` with `new_base..new_tip`. The revisions are resolved
    /// to object IDs first, so none of them can be read as an option.
    pub fn range_diff(&self, old_base: &str, old_tip: &str, new_base: &str, new_tip: &str, directory: &Path) -> Result<RangeDiff> {
        let old_base = self.resolve_commit(old_base, directory)?;
        let old_tip = self.resolve_commit(old_tip, directory)?;
        let new_base = self.resolve_commit(new_base, directory)?;
        let new_tip = self.resolve_commit(new_tip, directory)?;
        let old = self.series_commits(&old_base, &old_tip, directory)?;
        let new = self.series_commits(&new_base, &new_tip, directory)?;
        // Git refuses a range with no commits, so a series with none is listed by hand.
        if old.is_empty() || new.is_empty() {
            return Ok(RangeDiff { entries: one_sided_entries(&old, &new) });
        }
        let old_range = format!("{old_base}..{old_tip}");
        let new_range = format!("{new_base}..{new_tip}");
        let output =
            self.run(&["range-diff", "--no-color", "--no-dual-color", "--creation-factor=60", &old_range, &new_range], directory)?;
        Ok(RangeDiff { entries: parse_range_diff(&output) })
    }

    /// The abbreviated hash and subject of each non-merge commit in `base..tip`, oldest first.
    /// Both are object IDs, so the range cannot be read as an option.
    fn series_commits(&self, base: &str, tip: &str, directory: &Path) -> Result<Vec<(String, String)>> {
        let range = format!("{base}..{tip}");
        let output = self.run(&["log", "--reverse", "--no-merges", "--no-color", "--format=%h%x1f%s%x00", &range, "--"], directory)?;
        Ok(output
            .split('\0')
            .map(|record| record.trim_matches('\n'))
            .filter_map(|record| record.split_once('\u{1f}'))
            .map(|(short, subject)| (short.to_string(), subject.to_string()))
            .collect())
    }

    /// The tip a local branch had before its current one, from its reflog: the most recent
    /// earlier value that differs from the current tip. `None` when the branch has no earlier value.
    pub fn reflog_previous_tip(&self, branch: &str, directory: &Path) -> Result<Option<String>> {
        self.previous_tip(&format!("refs/heads/{branch}"), directory)
    }

    /// The tip a remote-tracking branch had before its current one, from its reflog. Works as
    /// `reflog_previous_tip` does, for `refs/remotes/<remote>/<branch>`.
    pub fn remote_reflog_previous_tip(&self, remote: &str, branch: &str, directory: &Path) -> Result<Option<String>> {
        self.previous_tip(&format!("refs/remotes/{remote}/{branch}"), directory)
    }

    /// The most recent reflog value of `full_ref` that differs from its current one.
    fn previous_tip(&self, full_ref: &str, directory: &Path) -> Result<Option<String>> {
        self.run(&["check-ref-format", full_ref], directory)?;
        let output = self.run(&["reflog", "show", "--format=%H", "-n", REFLOG_SCAN_LIMIT, full_ref, "--"], directory)?;
        let hashes: Vec<&str> = output.lines().map(str::trim).filter(|hash| !hash.is_empty()).collect();
        let Some(current) = hashes.first().copied() else { return Ok(None) };
        Ok(hashes.iter().copied().find(|hash| *hash != current).map(str::to_string))
    }

    /// The inputs that compare a branch with its previous version: its earlier reflog tip against
    /// its current tip. Each base is the merge base of a tip with the branch's upstream when it has
    /// one, or else with the other tip. `None` when there is no earlier version or the two share
    /// no history.
    pub fn default_range_diff_inputs(&self, branch: &str, directory: &Path) -> Result<Option<RangeDiffInputs>> {
        let Some(old_tip) = self.reflog_previous_tip(branch, directory)? else { return Ok(None) };
        let full_ref = format!("refs/heads/{branch}");
        let new_tip = self.resolve_commit(&full_ref, directory)?;
        let upstream = self.try_trimmed(&["rev-parse", "--verify", "--quiet", &format!("{full_ref}@{{upstream}}^{{commit}}")], directory);
        let (old_reference, new_reference) = match &upstream {
            Some(upstream) => (upstream.clone(), upstream.clone()),
            None => (new_tip.clone(), old_tip.clone()),
        };
        let Some(old_base) = self.merge_base(&old_tip, &old_reference, directory)? else { return Ok(None) };
        let Some(new_base) = self.merge_base(&new_tip, &new_reference, directory)? else { return Ok(None) };
        Ok(Some(RangeDiffInputs { old_base, old_tip, new_base, new_tip }))
    }

    /// The inputs that compare two tips, such as a remote-tracking tip and a local one, with both
    /// series starting at their merge base. `None` when the two share no history.
    pub fn tip_range_diff_inputs(&self, old_tip: &str, new_tip: &str, directory: &Path) -> Result<Option<RangeDiffInputs>> {
        let old_tip = self.resolve_commit(old_tip, directory)?;
        let new_tip = self.resolve_commit(new_tip, directory)?;
        let Some(base) = self.merge_base(&old_tip, &new_tip, directory)? else { return Ok(None) };
        Ok(Some(RangeDiffInputs { old_base: base.clone(), old_tip, new_base: base, new_tip }))
    }

    /// The merge base of two commits, or `None` when they share no history. Git exits with 1 for
    /// that case, which leaves the output empty.
    fn merge_base(&self, first: &str, second: &str, directory: &Path) -> Result<Option<String>> {
        let output = self.run_accepting(&["merge-base", first, second], directory, &[0, 1])?;
        let base = output.trim();
        Ok((!base.is_empty()).then(|| base.to_string()))
    }
}

/// The entries for a comparison where one series has no commits: each commit of the other is
/// removed or added, numbered within its own series.
fn one_sided_entries(old: &[(String, String)], new: &[(String, String)]) -> Vec<RangeDiffEntry> {
    let removed = old.iter().enumerate().map(|(position, (short, subject))| RangeDiffEntry {
        old: Some(RangeDiffCommit { index: position as u32 + 1, short: short.clone() }),
        new: None,
        status: RangeDiffStatus::Removed,
        subject: subject.clone(),
        diff_lines: Vec::new(),
    });
    let added = new.iter().enumerate().map(|(position, (short, subject))| RangeDiffEntry {
        old: None,
        new: Some(RangeDiffCommit { index: position as u32 + 1, short: short.clone() }),
        status: RangeDiffStatus::Added,
        subject: subject.clone(),
        diff_lines: Vec::new(),
    });
    removed.chain(added).collect()
}

/// Parses range-diff's output. A pair's header line starts with its numbers and object IDs and
/// is followed by its patch comparison, indented by four spaces. Subjects are read after the five
/// fields that precede them, so a subject may contain anything, including the status symbols.
pub fn parse_range_diff(output: &str) -> Vec<RangeDiffEntry> {
    let mut entries: Vec<RangeDiffEntry> = Vec::new();
    for line in output.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if let Some(detail) = line.strip_prefix("    ") {
            if let Some(entry) = entries.last_mut() {
                entry.diff_lines.push(detail.to_string());
            }
        } else if let Some(entry) = parse_header(line) {
            entries.push(entry);
        }
    }
    entries
}

/// Parses one pair's header, such as `2:  abc1234 ! 2:  def5678 Subject`. Returns `None` for a
/// line that is not a header, such as a blank line.
fn parse_header(line: &str) -> Option<RangeDiffEntry> {
    let (old_index, rest) = next_word(line)?;
    let (old_short, rest) = next_word(rest)?;
    let (symbol, rest) = next_word(rest)?;
    let (new_index, rest) = next_word(rest)?;
    let (new_short, rest) = next_word(rest)?;
    let status = RangeDiffStatus::from_symbol(symbol)?;
    let subject = rest.strip_prefix(' ').unwrap_or(rest).to_string();
    Some(RangeDiffEntry { old: side(old_index, old_short), new: side(new_index, new_short), status, subject, diff_lines: Vec::new() })
}

/// The first whitespace-delimited word of `text` and the text after it, with the space that
/// follows the word left in place so a subject keeps its own spacing.
fn next_word(text: &str) -> Option<(&str, &str)> {
    let text = text.trim_start_matches(' ');
    if text.is_empty() {
        return None;
    }
    let end = text.find(' ').unwrap_or(text.len());
    Some((&text[..end], &text[end..]))
}

/// One side of a pair. Range-diff prints `-:` and `-------` where a side has no commit.
fn side(index: &str, short: &str) -> Option<RangeDiffCommit> {
    if index == "-:" {
        return None;
    }
    let index = index.strip_suffix(':')?.parse::<u32>().ok()?;
    Some(RangeDiffCommit { index, short: short.to_string() })
}

#[cfg(test)]
mod tests {
    use super::{parse_range_diff, RangeDiff, RangeDiffCommit, RangeDiffStatus};

    fn commit(index: u32, short: &str) -> Option<RangeDiffCommit> {
        Some(RangeDiffCommit { index, short: short.to_string() })
    }

    #[test]
    fn each_status_symbol_parses_with_its_commits_and_subject() {
        let output = "\
1:  abc1234 = 1:  def5678 Same subject
2:  abc1235 ! 2:  def5679 Changed subject
3:  abc1236 < -:  ------- Removed subject
-:  ------- > 3:  def5680 Added subject
";
        let entries = parse_range_diff(output);
        assert_eq!(entries.len(), 4);
        assert_eq!(entries[0].status, RangeDiffStatus::Same);
        assert_eq!(entries[0].old, commit(1, "abc1234"));
        assert_eq!(entries[0].new, commit(1, "def5678"));
        assert_eq!(entries[0].subject, "Same subject");
        assert_eq!(entries[1].status, RangeDiffStatus::Changed);
        assert_eq!(entries[2].status, RangeDiffStatus::Removed);
        assert_eq!(entries[2].old, commit(3, "abc1236"));
        assert_eq!(entries[2].new, None);
        assert_eq!(entries[3].status, RangeDiffStatus::Added);
        assert_eq!(entries[3].old, None);
        assert_eq!(entries[3].new, commit(3, "def5680"));
    }

    #[test]
    fn subjects_may_contain_separators_and_status_symbols() {
        let output = "\
1:  abc1234 ! 1:  def5678 Fix: parse ! and = marks, then < > and -: too
2:  abc1235 = 2:  def5679 !
3:  abc1236 = 3:  def5680 2:  abc9999 ! 4:  x
";
        let entries = parse_range_diff(output);
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].subject, "Fix: parse ! and = marks, then < > and -: too");
        assert_eq!(entries[1].subject, "!");
        assert_eq!(entries[2].subject, "2:  abc9999 ! 4:  x");
        assert_eq!(entries[2].status, RangeDiffStatus::Same);
    }

    #[test]
    fn patch_comparison_lines_follow_their_header_and_are_not_headers() {
        // Inside a changed pair, a line such as "3:  ..." must stay a detail line, not a new entry.
        let output = "\
1:  abc1234 ! 1:  def5678 Change the file
    @@ Metadata
    -Change the file
    +Change the file
    @@ b.txt
    -+two
    ++two, changed
    3:  abc1234 ! 3:  def5678 looks like a header
     context line
2:  abc1235 = 2:  def5679 Next
";
        let entries = parse_range_diff(output);
        assert_eq!(entries.len(), 2);
        assert_eq!(
            entries[0].diff_lines,
            [
                "@@ Metadata",
                "-Change the file",
                "+Change the file",
                "@@ b.txt",
                "-+two",
                "++two, changed",
                "3:  abc1234 ! 3:  def5678 looks like a header",
                " context line",
            ]
        );
        assert!(entries[1].diff_lines.is_empty());
    }

    #[test]
    fn a_two_digit_index_and_blank_or_carriage_return_lines_parse() {
        let output = "10:  abc1234 ! 10:  def5678 Tenth\r\n\n11:  abc1235 = 11:  def5679 Eleventh\n";
        let entries = parse_range_diff(output);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].old, commit(10, "abc1234"));
        assert_eq!(entries[0].subject, "Tenth");
        assert_eq!(entries[1].new, commit(11, "def5679"));
    }

    #[test]
    fn empty_output_has_no_entries_and_other_lines_are_skipped() {
        assert!(parse_range_diff("").is_empty());
        assert!(parse_range_diff("warning: something Git printed\n").is_empty());
    }

    #[test]
    fn counts_group_entries_by_status() {
        let entries = parse_range_diff("1:  a = 1:  b S\n2:  c ! 2:  d C\n3:  e < -:  ------- R\n-:  ------- > 4:  f A\n");
        let diff = RangeDiff { entries };
        assert_eq!(diff.count(RangeDiffStatus::Same), 1);
        assert_eq!(diff.count(RangeDiffStatus::Changed), 1);
        assert_eq!(diff.count(RangeDiffStatus::Removed), 1);
        assert_eq!(diff.count(RangeDiffStatus::Added), 1);
    }
}
