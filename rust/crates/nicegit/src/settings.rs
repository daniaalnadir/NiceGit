//! Preferences and per-checkout state remembered across launches. Stored by eframe in the
//! platform's app data folder; nothing is sent anywhere.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use nicegit_core::client::PullStrategy;
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

/// How Pull brings the upstream's commits into the current branch. `FollowGit` reads Git's own
/// configuration for each pull, so a repository set up for rebasing keeps rebasing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum PullSetting {
    /// Git's `pull.rebase`, `branch.<name>.rebase`, and `pull.ff`; with none set, fast-forward only.
    #[default]
    FollowGit,
    FastForwardOnly,
    Merge,
    Rebase,
}

impl PullSetting {
    pub const ALL: [PullSetting; 4] = [PullSetting::FollowGit, PullSetting::FastForwardOnly, PullSetting::Merge, PullSetting::Rebase];

    pub fn title(self) -> &'static str {
        match self {
            PullSetting::FollowGit => "Follow Git's settings",
            PullSetting::FastForwardOnly => "Fast-forward only",
            PullSetting::Merge => "Merge",
            PullSetting::Rebase => "Rebase",
        }
    }

    /// The strategy this setting names, or `None` when it follows Git's configuration.
    pub fn strategy(self) -> Option<PullStrategy> {
        match self {
            PullSetting::FollowGit => None,
            PullSetting::FastForwardOnly => Some(PullStrategy::FastForwardOnly),
            PullSetting::Merge => Some(PullStrategy::Merge),
            PullSetting::Rebase => Some(PullStrategy::Rebase),
        }
    }
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

/// A named group of repositories that open together, as GitKraken's workspaces do. The folders
/// are kept in tab order and are never changed on disk by saving, opening, or deleting one.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Workspace {
    pub name: String,
    pub repositories: Vec<PathBuf>,
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
    /// How Pull brings the upstream's commits in: fast-forward only, merge, or rebase.
    #[serde(default)]
    pub pull_setting: PullSetting,
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
    /// Self-hosted GitLab hosts, such as `gitlab.example.com`. Their remotes get Open on GitLab
    /// links, as gitlab.com remotes do. Stored lower-cased, without repeats.
    #[serde(default)]
    pub gitlab_hosts: Vec<String>,
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
    /// Whether saving a stash takes only the staged changes. The other two stash choices are
    /// unavailable while it is on, since Git's `--staged` takes neither.
    #[serde(default)]
    pub stash_staged_only: bool,
    /// Whether a cherry-pick notes the original commit in each new commit's message, as
    /// `git cherry-pick -x` does.
    #[serde(default)]
    pub cherry_pick_record_origin: bool,
    /// Branches hidden from the commit graph, keyed like `drafts`. Each entry is a full ref name,
    /// such as `refs/heads/feature`, so a local and a remote branch never collide.
    #[serde(default)]
    pub hidden_branches: BTreeMap<String, BTreeSet<String>>,
    /// The branch each checkout shows alone in the commit graph, keyed like `drafts`. The value is
    /// a full ref name, as `hidden_branches` holds them.
    #[serde(default)]
    pub solo_branch: BTreeMap<String, String>,
    /// Saved workspaces, in the order they were saved.
    #[serde(default)]
    pub workspaces: Vec<Workspace>,
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
            pull_setting: PullSetting::FollowGit,
            history_columns: Default::default(),
            open: Vec::new(),
            active: 0,
            recent: Vec::new(),
            drafts: BTreeMap::new(),
            github_enterprise_hosts: Vec::new(),
            gitlab_hosts: Vec::new(),
            history_scope: HistoryScope::All,
            diff_context: DiffContextSetting::default(),
            clone_folder: None,
            stash_keep_index: false,
            stash_include_untracked: true,
            stash_staged_only: false,
            cherry_pick_record_origin: false,
            hidden_branches: BTreeMap::new(),
            solo_branch: BTreeMap::new(),
            workspaces: Vec::new(),
        }
    }
}

impl Settings {
    /// The self-hosted hosts listed in Settings, for the links that name a host's pages.
    pub fn listed_hosts(&self) -> nicegit_core::hosting::ListedHosts<'_> {
        nicegit_core::hosting::ListedHosts { github: &self.github_enterprise_hosts, gitlab: &self.gitlab_hosts }
    }
}

/// Reads the GitHub Enterprise box, which has one hostname per line. Returns the valid hostnames,
/// trimmed, lower-cased, and without repeats, and the lines that are not valid hostnames. A line
/// for github.com is left out silently, since github.com is always supported.
pub fn parse_enterprise_hosts(text: &str) -> (Vec<String>, Vec<String>) {
    parse_host_lines(text, "github.com")
}

/// Reads the GitLab box, which has one hostname per line, as [`parse_enterprise_hosts`] does. A
/// line for gitlab.com is left out silently, since gitlab.com is always supported.
pub fn parse_gitlab_hosts(text: &str) -> (Vec<String>, Vec<String>) {
    parse_host_lines(text, "gitlab.com")
}

/// The hostnames in a box with one per line: the valid ones, lower-cased and without repeats, and
/// the lines that are not hostnames. A line for `always_supported` is left out silently.
fn parse_host_lines(text: &str, always_supported: &str) -> (Vec<String>, Vec<String>) {
    let mut valid: Vec<String> = Vec::new();
    let mut invalid = Vec::new();
    for line in text.lines().map(str::trim).filter(|line| !line.is_empty()) {
        if line.eq_ignore_ascii_case(always_supported) {
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

    /// The full ref names of the branches hidden in the checkout at `folder`. Keyed by the folder the
    /// repository was opened at, as a snapshot's root is not known before the first load.
    pub fn hidden_branches_of(&self, folder: &Path) -> Vec<String> {
        self.hidden_branches.get(&Self::draft_key(&folder.to_string_lossy())).map(|set| set.iter().cloned().collect()).unwrap_or_default()
    }

    /// Changes the hidden branches of the checkout at `folder`, dropping its entry once none are left.
    pub fn change_hidden_branches(&mut self, folder: &Path, change: impl FnOnce(&mut BTreeSet<String>)) {
        let key = Self::draft_key(&folder.to_string_lossy());
        let mut hidden = self.hidden_branches.remove(&key).unwrap_or_default();
        change(&mut hidden);
        if !hidden.is_empty() {
            self.hidden_branches.insert(key, hidden);
        }
    }

    /// The full ref name of the branch the commit graph of the checkout at `folder` shows alone, if any.
    pub fn solo_branch_of(&self, folder: &Path) -> Option<String> {
        self.solo_branch.get(&Self::draft_key(&folder.to_string_lossy())).cloned()
    }

    /// Sets or clears the branch shown alone in the commit graph of the checkout at `folder`.
    pub fn set_solo_branch(&mut self, folder: &Path, reference: Option<String>) {
        let key = Self::draft_key(&folder.to_string_lossy());
        match reference {
            Some(reference) => {
                self.solo_branch.insert(key, reference);
            }
            None => {
                self.solo_branch.remove(&key);
            }
        }
    }

    pub fn remember(&mut self, path: PathBuf) {
        self.recent.retain(|recent| recent != &path);
        self.recent.insert(0, path);
        self.recent.truncate(15);
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_enterprise_hosts, parse_gitlab_hosts, Draft, Settings, Workspace, STORAGE_KEY};
    use eframe::Storage;
    use std::path::{Path, PathBuf};

    /// Saved state held in memory, as the platform's storage holds it between launches.
    #[derive(Default)]
    struct MemoryStorage(std::collections::HashMap<String, String>);

    impl eframe::Storage for MemoryStorage {
        fn get_string(&self, key: &str) -> Option<String> {
            self.0.get(key).cloned()
        }

        fn set_string(&mut self, key: &str, value: String) {
            self.0.insert(key.to_string(), value);
        }

        fn remove_string(&mut self, key: &str) {
            self.0.remove(key);
        }

        fn flush(&mut self) {}
    }

    #[test]
    fn hidden_branches_round_trip_for_their_checkout_only() {
        let folder = Path::new("/work/repo");
        let mut settings = Settings::default();
        settings.change_hidden_branches(folder, |hidden| {
            hidden.insert("refs/heads/feature".into());
            hidden.insert("refs/remotes/origin/feature".into());
        });
        let mut storage = MemoryStorage::default();
        eframe::set_value(&mut storage, STORAGE_KEY, &settings);
        let loaded: Settings = eframe::get_value(&storage, STORAGE_KEY).expect("saved settings");
        assert_eq!(loaded.hidden_branches_of(folder), ["refs/heads/feature", "refs/remotes/origin/feature"]);
        assert!(loaded.hidden_branches_of(Path::new("/work/other")).is_empty());
    }

    #[test]
    fn showing_every_branch_again_drops_the_checkout_entry() {
        let folder = Path::new("/work/repo");
        let mut settings = Settings::default();
        settings.change_hidden_branches(folder, |hidden| {
            hidden.insert("refs/heads/feature".into());
        });
        assert_eq!(settings.hidden_branches.len(), 1);
        settings.change_hidden_branches(folder, |hidden| hidden.clear());
        assert!(settings.hidden_branches.is_empty());
    }

    #[test]
    fn settings_saved_before_hidden_branches_still_load() {
        let mut storage = MemoryStorage::default();
        eframe::set_value(&mut storage, STORAGE_KEY, &Settings::default());
        let saved = storage.get_string(STORAGE_KEY).expect("saved settings");
        // An empty map is written as `hidden_branches:{}`; an older file has no such field.
        let older = saved.replacen(",hidden_branches:{}", "", 1);
        assert_ne!(older, saved, "the field is in the saved settings");
        storage.set_string(STORAGE_KEY, older);
        let loaded: Settings = eframe::get_value(&storage, STORAGE_KEY).expect("older settings still load");
        assert!(loaded.hidden_branches.is_empty());
    }

    #[test]
    fn solo_branch_round_trips_for_its_checkout_only() {
        let folder = Path::new("/work/repo");
        let mut settings = Settings::default();
        settings.set_solo_branch(folder, Some("refs/remotes/origin/feature".into()));
        let mut storage = MemoryStorage::default();
        eframe::set_value(&mut storage, STORAGE_KEY, &settings);
        let loaded: Settings = eframe::get_value(&storage, STORAGE_KEY).expect("saved settings");
        assert_eq!(loaded.solo_branch_of(folder).as_deref(), Some("refs/remotes/origin/feature"));
        assert_eq!(loaded.solo_branch_of(Path::new("/work/other")), None);
    }

    #[test]
    fn clearing_the_solo_branch_drops_the_checkout_entry() {
        let folder = Path::new("/work/repo");
        let mut settings = Settings::default();
        settings.set_solo_branch(folder, Some("refs/heads/feature".into()));
        assert_eq!(settings.solo_branch.len(), 1);
        settings.set_solo_branch(folder, None);
        assert!(settings.solo_branch.is_empty());
        assert_eq!(settings.solo_branch_of(folder), None);
    }

    #[test]
    fn settings_saved_before_solo_branch_still_load() {
        let mut storage = MemoryStorage::default();
        eframe::set_value(&mut storage, STORAGE_KEY, &Settings::default());
        let saved = storage.get_string(STORAGE_KEY).expect("saved settings");
        // An empty map is written as `solo_branch:{}`; an older file has no such field.
        let older = saved.replacen(",solo_branch:{}", "", 1);
        assert_ne!(older, saved, "the field is in the saved settings");
        storage.set_string(STORAGE_KEY, older);
        let loaded: Settings = eframe::get_value(&storage, STORAGE_KEY).expect("older settings still load");
        assert!(loaded.solo_branch.is_empty());
    }

    #[test]
    fn cherry_pick_origin_choice_round_trips_and_starts_off() {
        assert!(!Settings::default().cherry_pick_record_origin);
        let settings = Settings { cherry_pick_record_origin: true, ..Settings::default() };
        let mut storage = MemoryStorage::default();
        eframe::set_value(&mut storage, STORAGE_KEY, &settings);
        let loaded: Settings = eframe::get_value(&storage, STORAGE_KEY).expect("saved settings");
        assert!(loaded.cherry_pick_record_origin);
    }

    #[test]
    fn workspaces_round_trip_with_their_repositories_in_tab_order() {
        let settings = Settings {
            workspaces: vec![
                Workspace { name: "Client work".into(), repositories: vec!["/work/api".into(), "/work/web".into()] },
                Workspace { name: "Scratch".into(), repositories: vec!["/tmp/play".into()] },
            ],
            ..Settings::default()
        };
        let mut storage = MemoryStorage::default();
        eframe::set_value(&mut storage, STORAGE_KEY, &settings);
        let loaded: Settings = eframe::get_value(&storage, STORAGE_KEY).expect("saved settings");
        assert_eq!(loaded.workspaces, settings.workspaces);
        assert_eq!(loaded.workspaces[0].repositories, [PathBuf::from("/work/api"), PathBuf::from("/work/web")]);
    }

    #[test]
    fn settings_saved_before_workspaces_still_load() {
        let mut storage = MemoryStorage::default();
        eframe::set_value(&mut storage, STORAGE_KEY, &Settings::default());
        let saved = storage.get_string(STORAGE_KEY).expect("saved settings");
        // An empty list is written as `workspaces:[]`; an older file has no such field.
        let older = saved.replacen(",workspaces:[]", "", 1);
        assert_ne!(older, saved, "the field is in the saved settings");
        storage.set_string(STORAGE_KEY, older);
        let loaded: Settings = eframe::get_value(&storage, STORAGE_KEY).expect("older settings still load");
        assert!(loaded.workspaces.is_empty());
    }

    #[test]
    fn settings_saved_before_cherry_pick_origin_still_load() {
        let mut storage = MemoryStorage::default();
        eframe::set_value(&mut storage, STORAGE_KEY, &Settings::default());
        let saved = storage.get_string(STORAGE_KEY).expect("saved settings");
        // The field is written as `cherry_pick_record_origin:false`; an older file has no such field.
        let older = saved.replacen(",cherry_pick_record_origin:false", "", 1);
        assert_ne!(older, saved, "the field is in the saved settings");
        storage.set_string(STORAGE_KEY, older);
        let loaded: Settings = eframe::get_value(&storage, STORAGE_KEY).expect("older settings still load");
        assert!(!loaded.cherry_pick_record_origin);
    }

    #[test]
    fn enterprise_hosts_are_trimmed_lowercased_and_deduplicated_and_invalid_lines_kept_apart() {
        let (valid, invalid) =
            parse_enterprise_hosts("  Git.Example.com \n\ngit.example.com\nnot a host\nhost:8443\ngithub.com\nghe.corp.internal");
        assert_eq!(valid, vec!["git.example.com", "ghe.corp.internal"]);
        assert_eq!(invalid, vec!["not a host", "host:8443"]);
        assert_eq!(parse_enterprise_hosts(""), (Vec::new(), Vec::new()));
    }

    #[test]
    fn gitlab_hosts_are_read_like_enterprise_hosts_and_skip_gitlab_com() {
        let (valid, invalid) = parse_gitlab_hosts("GitLab.Example.com\ngitlab.com\nnot a host\ngitlab.example.com");
        assert_eq!(valid, vec!["gitlab.example.com"]);
        assert_eq!(invalid, vec!["not a host"]);
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
