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
            file_view: FileView::Path,
            show_repositories: true,
            sync_action: SyncAction::Fetch,
            history_columns: Default::default(),
            open: Vec::new(),
            active: 0,
            recent: Vec::new(),
            drafts: BTreeMap::new(),
        }
    }
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
    use super::Draft;

    #[test]
    fn drafts_round_trip_summary_and_description() {
        let draft = Draft::from_message("Fix the graph\n\nLines keep their lane.\nSecond line.\n");
        assert_eq!(draft.summary, "Fix the graph");
        assert_eq!(draft.description, "Lines keep their lane.\nSecond line.");
        assert_eq!(draft.message(), "Fix the graph\n\nLines keep their lane.\nSecond line.");
        assert_eq!(Draft::from_message("Only a summary").message(), "Only a summary");
    }
}
