//! Preferences and per-checkout state remembered across launches. Stored by eframe in the
//! platform's app data folder; nothing is sent anywhere.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::theme::{Appearance, GraphPalette};

pub const STORAGE_KEY: &str = "nicegit.settings";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum FileView {
    #[default]
    Path,
    Tree,
}

/// What the toolbar's combined Fetch and Pull button does when clicked.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SyncAction {
    /// Download from all remotes without changing the current branch.
    #[default]
    Fetch,
    /// Fetch, then fast-forward the current branch.
    Pull,
}

/// Which branches the commit graph shows. Mirrors `nicegit_core::HistoryScope`, which has no
/// serialisation of its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum HistoryScope {
    /// Every branch, tag, and remote-tracking branch.
    #[default]
    All,
    /// Local branches, and HEAD even when it is detached.
    Local,
    /// HEAD and its upstream.
    Current,
}

impl From<HistoryScope> for nicegit_core::HistoryScope {
    fn from(scope: HistoryScope) -> Self {
        match scope {
            HistoryScope::All => nicegit_core::HistoryScope::All,
            HistoryScope::Local => nicegit_core::HistoryScope::Local,
            HistoryScope::Current => nicegit_core::HistoryScope::Current,
        }
    }
}

/// How many unchanged lines a diff shows around each change.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum DiffContextSetting {
    /// Three lines, as Git shows by default.
    #[default]
    Three,
    Ten,
    TwentyFive,
    WholeFile,
}

impl DiffContextSetting {
    pub const ALL: [Self; 4] = [Self::Three, Self::Ten, Self::TwentyFive, Self::WholeFile];

    pub fn label(self) -> &'static str {
        match self {
            Self::Three => "3 lines",
            Self::Ten => "10 lines",
            Self::TwentyFive => "25 lines",
            Self::WholeFile => "Whole file",
        }
    }

    /// The context Git is asked for. Three is spelled out, not left to Git, so the setting means
    /// exactly three lines whatever the user's `diff.context` says.
    pub fn context(self) -> nicegit_core::diff::DiffContext {
        use nicegit_core::diff::DiffContext;
        match self {
            Self::Three => DiffContext::Lines(3),
            Self::Ten => DiffContext::Lines(10),
            Self::TwentyFive => DiffContext::Lines(25),
            Self::WholeFile => DiffContext::WholeFile,
        }
    }
}

/// A commit message being written, kept per checkout (repository and branch).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Draft {
    pub summary: String,
    pub description: String,
}

impl Draft {
    pub fn is_empty(&self) -> bool {
        self.summary.trim().is_empty() && self.description.trim().is_empty()
    }

    /// The full message: the summary, a blank line, then the description.
    pub fn message(&self) -> String {
        let summary = self.summary.trim();
        let description = self.description.trim_end();
        if description.trim().is_empty() {
            summary.to_string()
        } else {
            format!("{summary}\n\n{description}")
        }
    }

    pub fn from_message(message: &str) -> Self {
        let message = message.trim_end();
        match message.split_once('\n') {
            Some((summary, rest)) => Draft { summary: summary.to_string(), description: rest.trim_start_matches('\n').to_string() },
            None => Draft { summary: message.to_string(), description: String::new() },
        }
    }

    /// Whether `trailer` (a whole line such as `Co-authored-by: Ana <ana@example.com>`) is one of
    /// the description's lines.
    pub fn has_trailer(&self, trailer: &str) -> bool {
        self.description.split('\n').any(|line| same_trailer(line, trailer))
    }

    /// Adds `trailer` (a whole line such as `Co-authored-by: Ana <ana@example.com>`) to the end
    /// of the description, or removes it when it is already there. Trailers go in a final
    /// paragraph of their own, after a blank line, as `git interpret-trailers` expects.
    pub fn toggle_trailer(&mut self, trailer: &str) {
        let trailer = trailer.trim_end();
        let text = self.description.trim_end();
        if self.has_trailer(trailer) {
            // Work through the paragraphs. A paragraph left empty by the removal takes one
            // neighbouring blank line with it, so no blank lines are left behind.
            let lines: Vec<&str> = text.split('\n').collect();
            let mut out: Vec<&str> = Vec::new();
            let mut index = 0;
            while index < lines.len() {
                if lines[index].trim().is_empty() {
                    out.push(lines[index]);
                    index += 1;
                    continue;
                }
                let end = lines[index..].iter().position(|line| line.trim().is_empty()).map_or(lines.len(), |offset| index + offset);
                let kept: Vec<&str> = lines[index..end].iter().copied().filter(|line| !same_trailer(line, trailer)).collect();
                if !kept.is_empty() {
                    out.extend(kept);
                } else if out.last().is_some_and(|line| line.trim().is_empty()) {
                    out.pop();
                } else if end < lines.len() {
                    // The paragraph opened the text, so the blank line after it goes instead.
                    index = end + 1;
                    continue;
                }
                index = end;
            }
            while out.last().is_some_and(|line| line.trim().is_empty()) {
                out.pop();
            }
            self.description = out.join("\n");
        } else if text.trim().is_empty() {
            self.description = trailer.to_string();
        } else {
            let mut lines: Vec<&str> = text.split('\n').collect();
            // A trailer joins a last paragraph that is already all trailers; otherwise it starts
            // a new paragraph after a blank line.
            let start = lines.iter().rposition(|line| line.trim().is_empty()).map_or(0, |position| position + 1);
            let last = &lines[start..];
            if last.is_empty() || !last.iter().all(|line| is_trailer_line(line)) {
                lines.push("");
            }
            lines.push(trailer);
            self.description = lines.join("\n");
        }
    }
}

/// Whether `line` is a trailer, as `^[A-Za-z0-9-]+: \S` matches: a key of letters, digits and
/// hyphens, then a colon, a space and a value.
fn is_trailer_line(line: &str) -> bool {
    let line = line.trim_end();
    let Some((key, value)) = line.split_once(':') else { return false };
    !key.is_empty()
        && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        && value.strip_prefix(' ').and_then(|value| value.chars().next()).is_some_and(|c| !c.is_whitespace())
}

/// Whether `line` is `trailer`, ignoring trailing whitespace and the case of the key.
fn same_trailer(line: &str, trailer: &str) -> bool {
    let (line, trailer) = (line.trim_end(), trailer.trim_end());
    match (line.split_once(": "), trailer.split_once(": ")) {
        (Some((key, value)), Some((wanted_key, wanted_value))) => key.eq_ignore_ascii_case(wanted_key) && value == wanted_value,
        _ => line == trailer,
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub appearance: Appearance,
    pub graph_palette: GraphPalette,
    pub split_diff: bool,
    pub ignore_whitespace: bool,
    pub auto_refresh: bool,
    /// How often the open repository is fetched from its remotes in the background, in
    /// minutes; 0 turns it off.
    pub auto_fetch_minutes: u32,
    /// How large the whole interface is drawn: 1.0 is normal size.
    pub zoom: f32,
    /// The command that opens a file in the person's own editor, such as `code` or `zed`; the
    /// file's path is added after it. Empty when none is set.
    pub external_editor: String,
    pub file_view: FileView,
    pub show_repositories: bool,
    /// Whether the toolbar's Fetch and Pull button fetches or pulls.
    pub sync_action: SyncAction,
    /// The history's columns: which are shown and how wide.
    pub history_columns: crate::ui::history_columns::HistoryColumns,
    /// Repositories open as tabs, in order.
    pub open: Vec<PathBuf>,
    pub active: usize,
    /// Recently opened repositories, newest first.
    pub recent: Vec<PathBuf>,
    /// Commit message drafts, keyed by checkout folder.
    pub drafts: BTreeMap<String, Draft>,
    /// GitHub Enterprise Server hosts, such as `git.example.com`. Their remotes work like
    /// github.com remotes. Stored lower-cased, without repeats.
    #[serde(default)]
    pub github_enterprise_hosts: Vec<String>,
    /// Which branches the commit graph shows, in every repository.
    #[serde(default)]
    pub history_scope: HistoryScope,
    /// How many unchanged lines each diff shows around its changes.
    #[serde(default)]
    pub diff_context: DiffContextSetting,
    /// The folder clones go into by default. Set in Settings, or remembered from the parent folder
    /// of the first clone; the Clone dialog fills its destination from it.
    #[serde(default)]
    pub clone_folder: Option<PathBuf>,
    /// Whether saving a stash leaves staged changes in the index and the working files.
    #[serde(default)]
    pub stash_keep_index: bool,
    /// Whether saving a stash also takes untracked files. On by default, as it always was.
    #[serde(default = "default_true")]
    pub stash_include_untracked: bool,
}

fn default_true() -> bool {
    true
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            appearance: Appearance::System,
            graph_palette: GraphPalette::Standard,
            split_diff: false,
            ignore_whitespace: false,
            auto_refresh: true,
            auto_fetch_minutes: 15,
            zoom: 1.0,
            external_editor: String::new(),
            file_view: FileView::Path,
            show_repositories: true,
            sync_action: SyncAction::Fetch,
            history_columns: Default::default(),
            open: Vec::new(),
            active: 0,
            recent: Vec::new(),
            drafts: BTreeMap::new(),
            github_enterprise_hosts: Vec::new(),
            history_scope: HistoryScope::All,
            diff_context: DiffContextSetting::default(),
            clone_folder: None,
            stash_keep_index: false,
            stash_include_untracked: true,
        }
    }
}

/// Reads the GitHub Enterprise box, which has one hostname per line. Returns the valid hostnames,
/// trimmed, lower-cased, and without repeats, and the lines that are not valid hostnames. A line
/// for github.com is left out silently, since github.com is always supported.
pub fn parse_enterprise_hosts(text: &str) -> (Vec<String>, Vec<String>) {
    let mut valid: Vec<String> = Vec::new();
    let mut invalid = Vec::new();
    for line in text.lines().map(str::trim).filter(|line| !line.is_empty()) {
        if line.eq_ignore_ascii_case("github.com") {
            continue;
        }
        if nicegit_core::github::valid_enterprise_host(line) {
            let host = line.to_ascii_lowercase();
            if !valid.contains(&host) {
                valid.push(host);
            }
        } else {
            invalid.push(line.to_string());
        }
    }
    (valid, invalid)
}

impl Settings {
    /// Drafts belong to a checkout folder, as in the Mac app, so switching branches keeps the
    /// message being written.
    pub fn draft_key(checkout: &str) -> String {
        checkout.to_string()
    }

    pub fn remember(&mut self, path: PathBuf) {
        self.recent.retain(|recent| recent != &path);
        self.recent.insert(0, path);
        self.recent.truncate(15);
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_enterprise_hosts, Draft};

    #[test]
    fn enterprise_hosts_are_trimmed_lowercased_and_deduplicated_and_invalid_lines_kept_apart() {
        let (valid, invalid) =
            parse_enterprise_hosts("  Git.Example.com \n\ngit.example.com\nnot a host\nhost:8443\ngithub.com\nghe.corp.internal");
        assert_eq!(valid, vec!["git.example.com", "ghe.corp.internal"]);
        assert_eq!(invalid, vec!["not a host", "host:8443"]);
        assert_eq!(parse_enterprise_hosts(""), (Vec::new(), Vec::new()));
    }

    #[test]
    fn drafts_round_trip_summary_and_description() {
        let draft = Draft::from_message("Fix the graph\n\nLines keep their lane.\nSecond line.\n");
        assert_eq!(draft.summary, "Fix the graph");
        assert_eq!(draft.description, "Lines keep their lane.\nSecond line.");
        assert_eq!(draft.message(), "Fix the graph\n\nLines keep their lane.\nSecond line.");
        assert_eq!(Draft::from_message("Only a summary").message(), "Only a summary");
    }

    const ANA: &str = "Co-authored-by: Ana <ana@example.com>";
    const DANA: &str = "Signed-off-by: Dana <dana@example.com>";

    fn described(description: &str) -> Draft {
        Draft { summary: "Fix the graph".into(), description: description.into() }
    }

    #[test]
    fn trailers_go_into_an_empty_description_alone() {
        let mut draft = Draft::default();
        draft.toggle_trailer(ANA);
        assert_eq!(draft.description, ANA);
        assert!(draft.has_trailer(ANA));
    }

    #[test]
    fn trailers_follow_prose_after_a_blank_line() {
        let mut draft = described("Keep the lanes still.\nThen check the tests.");
        draft.toggle_trailer(ANA);
        assert_eq!(draft.description, format!("Keep the lanes still.\nThen check the tests.\n\n{ANA}"));
    }

    #[test]
    fn trailers_add_a_second_line_to_the_trailer_paragraph() {
        let mut draft = described(&format!("Keep the lanes still.\n\n{DANA}"));
        draft.toggle_trailer(ANA);
        assert_eq!(draft.description, format!("Keep the lanes still.\n\n{DANA}\n{ANA}"));
    }

    #[test]
    fn trailers_removing_the_only_one_leaves_the_prose() {
        let mut draft = described(&format!("Keep the lanes still.\n\n{ANA}\n"));
        assert!(draft.has_trailer(ANA));
        draft.toggle_trailer(ANA);
        assert_eq!(draft.description, "Keep the lanes still.");
        assert!(!draft.has_trailer(ANA));
    }

    #[test]
    fn trailers_removing_one_of_two_keeps_the_other() {
        let mut draft = described(&format!("Keep the lanes still.\n\n{DANA}\n{ANA}"));
        draft.toggle_trailer(ANA);
        assert_eq!(draft.description, format!("Keep the lanes still.\n\n{DANA}"));
        assert!(draft.has_trailer(DANA));
        assert!(!draft.has_trailer(ANA));
    }

    #[test]
    fn trailers_match_the_key_regardless_of_case() {
        let draft = described("Keep the lanes still.\n\nco-authored-by: Ana <ana@example.com>");
        assert!(draft.has_trailer(ANA));
        // The value is compared exactly, so a changed address is a different trailer.
        assert!(!draft.has_trailer("Co-authored-by: Ana <ANA@example.com>"));
        let mut draft = draft;
        draft.toggle_trailer(ANA);
        assert_eq!(draft.description, "Keep the lanes still.");
    }

    #[test]
    fn trailers_after_prose_that_looks_like_one_still_get_a_blank_line() {
        let mut draft = described("Fixes: the graph\nLanes stay put.");
        draft.toggle_trailer(ANA);
        assert_eq!(draft.description, format!("Fixes: the graph\nLanes stay put.\n\n{ANA}"));
    }

    #[test]
    fn trailers_toggled_twice_restore_the_original() {
        let original = "Keep the lanes still.\n\nNotes: none";
        let mut draft = described(original);
        draft.toggle_trailer(ANA);
        draft.toggle_trailer(ANA);
        assert_eq!(draft.description, original);
    }
}
