use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use nicegit_core::graph::{layout, layout_with_working_tree, GraphRow};
use nicegit_core::models::short;
use nicegit_core::undo::{DiscardUndo, UndoMode, UndoStep};
use nicegit_core::{Branch, GitClient, Snapshot, Stash, StatusEntry};

use crate::diff_view::DiffContent;
use crate::settings::{Draft, Settings, STORAGE_KEY};
use crate::theme;
use crate::tools::{Ctx, Request, Task, ToolWindow};
use crate::ui::dialogs::{Dialog, Pending};
use crate::worker::{ActionResult, Message, Worker};

pub const PAGE: usize = 500;
const AUTO_REFRESH: Duration = Duration::from_secs(30);
const WATCH_DEBOUNCE: Duration = Duration::from_millis(400);

pub struct Notice {
    pub text: String,
    pub is_error: bool,
}

#[derive(Clone, PartialEq)]
pub enum Selection {
    None,
    WorkingTree,
    Change { entry: StatusEntry, staged: bool },
    Commit { hash: String, file: Option<String> },
    Stash { hash: String },
}

/// An action that can be undone, with the words to describe it.
#[derive(Clone)]
pub struct HistoryEntry {
    pub title: String,
    pub step: UndoStep,
}

impl HistoryEntry {
    /// Branch moves apply only while that branch is checked out; other steps always apply.
    pub fn applies_to(&self, branch: &str) -> bool {
        match &self.step {
            UndoStep::BranchMove { branch: moved, .. } => moved == branch,
            _ => true,
        }
    }
}

/// What a finished action recorded for undo, handed from the background thread.
pub enum Recorded {
    Step(HistoryEntry),
    Redo(HistoryEntry),
    Discard(DiscardUndo),
}

pub type RecordSlot = Arc<Mutex<Option<Recorded>>>;

/// One open repository tab.
pub struct Repo {
    pub path: PathBuf,
    pub snapshot: Option<Snapshot>,
    pub error: Option<String>,
    pub rows: Vec<GraphRow>,
    pub lanes: usize,
    pub history_limit: usize,
    pub generation: u64,
    pub loading: bool,
    pub last_load: Instant,
    pub selection: Selection,
    /// Commits chosen together with Command/Ctrl-click, for cherry-picking several.
    pub marked: BTreeSet<String>,
    /// A commit marked for comparison with another.
    pub compare_base: Option<String>,
    pub commit_files: Vec<(String, String)>,
    pub details: Option<Task<nicegit_core::Result<nicegit_core::client::CommitDetails>>>,
    pub diff: Option<DiffContent>,
    pub diff_loading: bool,
    pub diff_generation: u64,
    /// The selected change prepared for staging individual lines.
    pub review: Option<Task<nicegit_core::Result<nicegit_core::staging::FileReview>>>,
    pub diff_options: crate::diff_view::DiffOptions,
    pub draft: Draft,
    pub draft_key: Option<String>,
    pub amend: bool,
    pub commit_filter: String,
    pub branch_filter: String,
    pub undo: Option<HistoryEntry>,
    pub redo: Option<HistoryEntry>,
    /// A discard that can be reversed while the file is unchanged since.
    /// The last discards that can be reversed, newest last (at most 20).
    pub discard_undo: Vec<DiscardUndo>,
    pub scroll_to_selection: bool,
    /// A bisect in progress, read after each refresh.
    pub bisect: Option<Task<Option<nicegit_core::bisect::BisectStatus>>>,
    /// Submodules recorded in the index, read after each refresh.
    pub submodules: Option<Task<Vec<nicegit_core::submodule::Submodule>>>,
}

impl Repo {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            snapshot: None,
            error: None,
            rows: Vec::new(),
            lanes: 1,
            history_limit: PAGE,
            generation: 0,
            loading: false,
            last_load: Instant::now(),
            selection: Selection::None,
            marked: BTreeSet::new(),
            compare_base: None,
            commit_files: Vec::new(),
            details: None,
            diff: None,
            diff_loading: false,
            diff_generation: 0,
            review: None,
            diff_options: crate::diff_view::DiffOptions::default(),
            draft: Draft::default(),
            draft_key: None,
            amend: false,
            commit_filter: String::new(),
            branch_filter: String::new(),
            undo: None,
            redo: None,
            discard_undo: Vec::new(),
            scroll_to_selection: false,
            bisect: None,
            submodules: None,
        }
    }

    pub fn name(&self) -> String {
        self.snapshot
            .as_ref()
            .map(|s| s.name.clone())
            .or_else(|| self.path.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_else(|| self.path.display().to_string())
    }

    pub fn dirty(&self) -> bool {
        self.snapshot.as_ref().is_some_and(|s| !s.status.is_empty())
    }

    /// The commit shown in graph row `index`, or None for the working tree row.
    pub fn commit_at(&self, index: usize) -> Option<&nicegit_core::Commit> {
        let snapshot = self.snapshot.as_ref()?;
        let offset = usize::from(self.dirty());
        if index < offset {
            None
        } else {
            snapshot.commits.get(index - offset)
        }
    }
}

pub struct NiceGitApp {
    pub worker: Worker,
    pub repos: Vec<Repo>,
    pub active: usize,
    pub settings: Settings,
    pub busy: Option<String>,
    pub notice: Option<Notice>,
    pub dialog: Option<Dialog>,
    pub tools: Vec<Box<dyn ToolWindow>>,
    pub palette: Option<crate::ui::palette::PaletteState>,
    pub show_settings: bool,
    pub git_missing: bool,
    generation: u64,
    was_focused: bool,
    watcher: Option<Watch>,
    watched_change: Option<Instant>,
    applied_appearance: Option<(theme::Appearance, theme::GraphPalette)>,
    pub pending_clone: Option<Task<nicegit_core::Result<PathBuf>>>,
    recorded: RecordSlot,
    /// A debug-only light preview that does not touch saved settings.
    pub preview_light: bool,
    /// The terminal panel under the history, when shown.
    pub terminal: Option<crate::tools::terminal::TerminalWindow>,
    fitted_to_monitor: bool,
}

struct Watch {
    path: PathBuf,
    _watcher: notify::RecommendedWatcher,
    events: Receiver<notify::Result<notify::Event>>,
}

impl NiceGitApp {
    pub fn new(creation: &eframe::CreationContext<'_>, initial: Option<PathBuf>) -> Self {
        let settings: Settings = creation.storage.and_then(|storage| eframe::get_value(storage, STORAGE_KEY)).unwrap_or_default();
        theme::install_fonts(&creation.egui_ctx);
        let mut app = Self {
            worker: Worker::new(creation.egui_ctx.clone()),
            repos: Vec::new(),
            active: 0,
            settings,
            busy: None,
            notice: None,
            dialog: None,
            tools: Vec::new(),
            palette: None,
            show_settings: false,
            git_missing: nicegit_core::runner::git_executable().is_none(),
            generation: 0,
            was_focused: true,
            watcher: None,
            watched_change: None,
            applied_appearance: None,
            pending_clone: None,
            recorded: Arc::new(Mutex::new(None)),
            preview_light: false,
            terminal: None,
            fitted_to_monitor: false,
        };
        let open: Vec<PathBuf> = app.settings.open.iter().filter(|p| p.exists()).cloned().collect();
        app.active = app.settings.active.min(open.len().saturating_sub(1));
        app.repos = open.into_iter().map(Repo::new).collect();
        if let Some(path) = initial.filter(|p| p.exists()) {
            app.open(path);
        } else if !app.repos.is_empty() {
            app.load_repo(app.active, true);
        }
        app
    }

    // MARK: Tabs

    pub fn repo(&self) -> Option<&Repo> {
        self.repos.get(self.active)
    }

    pub fn repo_mut(&mut self) -> Option<&mut Repo> {
        self.repos.get_mut(self.active)
    }

    pub fn snapshot(&self) -> Option<&Snapshot> {
        self.repo().and_then(|r| r.snapshot.as_ref())
    }

    pub fn idle(&self) -> bool {
        self.busy.is_none() && self.snapshot().is_some()
    }

    /// Opens a repository in a new tab, or switches to its tab if already open.
    pub fn open(&mut self, path: PathBuf) {
        if self.busy.is_some() {
            return;
        }
        let path = strip_verbatim(path.canonicalize().unwrap_or(path));
        if let Some(index) = self.repos.iter().position(|r| r.path == path) {
            self.switch_to(index);
            return;
        }
        if self.has_unsaved_editor() {
            self.notify("Save or close the file you are editing before opening another repository.", true);
            return;
        }
        self.save_draft();
        self.repos.push(Repo::new(path));
        self.active = self.repos.len() - 1;
        self.tools.clear();
        self.load_repo(self.active, true);
    }

    pub fn switch_to(&mut self, index: usize) {
        if index >= self.repos.len() || self.busy.is_some() {
            return;
        }
        if index == self.active {
            if self.repos[index].snapshot.is_none() && !self.repos[index].loading {
                self.load_repo(index, true);
            }
            return;
        }
        if self.has_unsaved_editor() {
            self.notify("Save or close the file you are editing before switching repositories.", true);
            return;
        }
        self.save_draft();
        self.active = index;
        self.tools.clear();
        let repo = &self.repos[index];
        if repo.snapshot.is_none() || repo.last_load.elapsed() > Duration::from_secs(2) {
            self.load_repo(index, true);
        } else {
            self.watch_active();
        }
    }

    pub fn close_tab(&mut self, index: usize) {
        if index >= self.repos.len() || self.busy.is_some() {
            return;
        }
        if index == self.active {
            if self.has_unsaved_editor() {
                self.notify("Save or close the file you are editing first.", true);
                return;
            }
            self.save_draft();
            self.tools.clear();
        }
        self.repos.remove(index);
        if self.active >= self.repos.len() {
            self.active = self.repos.len().saturating_sub(1);
        } else if index < self.active {
            self.active -= 1;
        }
        if self.repos.get(self.active).is_some_and(|r| r.snapshot.is_none()) {
            self.load_repo(self.active, true);
        }
    }

    pub fn choose_folder(&mut self) {
        if self.busy.is_some() {
            return;
        }
        if let Some(folder) = rfd::FileDialog::new().set_title("Open a Git repository").pick_folder() {
            self.open(folder);
        }
    }

    // MARK: Loading

    pub fn load(&mut self, deliberate: bool) {
        self.load_repo(self.active, deliberate);
    }

    fn load_repo(&mut self, index: usize, deliberate: bool) {
        if self.busy.is_some() {
            return;
        }
        let Some(repo) = self.repos.get_mut(index) else { return };
        self.generation += 1;
        repo.generation = self.generation;
        repo.loading = true;
        repo.last_load = Instant::now();
        let limit = repo.history_limit;
        self.worker.load(self.generation, repo.path.clone(), deliberate, limit);
    }

    /// Runs an action that changes the repository in the background, then refreshes it.
    /// Refused while another action runs or the editor has unsaved text.
    pub fn act(&mut self, label: &str, action: impl FnOnce(&GitClient, &Path) -> ActionResult + Send + 'static) {
        if self.busy.is_some() {
            return;
        }
        if self.has_unsaved_editor() {
            self.notify("Save or close the file you are editing before changing the repository.", true);
            return;
        }
        let Some(repo) = self.repos.get_mut(self.active) else { return };
        self.generation += 1;
        repo.generation = self.generation;
        repo.loading = true;
        repo.last_load = Instant::now();
        self.busy = Some(label.to_string());
        self.notice = None;
        let limit = repo.history_limit;
        self.worker.act(self.generation, repo.path.clone(), label.to_string(), limit, action);
    }

    /// Runs an action that moves the current branch, recording it so it can be undone. Only a
    /// completed move is recorded: an operation that stopped partway is not undoable.
    pub fn act_recording(&mut self, label: &str, title: &str, action: impl FnOnce(&GitClient, &Path) -> ActionResult + Send + 'static) {
        let mode = match title {
            "Commit" | "Amend" | "Undo commit" => UndoMode::Soft,
            _ => UndoMode::Keep,
        };
        self.act_recording_mode(label, title, mode, action);
    }

    pub fn act_recording_mode(
        &mut self,
        label: &str,
        title: &str,
        mode: UndoMode,
        action: impl FnOnce(&GitClient, &Path) -> ActionResult + Send + 'static,
    ) {
        let Some(snapshot) = self.snapshot() else { return };
        let (branch, before) = (snapshot.current_branch.clone(), snapshot.head_hash.clone());
        let on_branch = snapshot.is_on_branch();
        let title = title.to_string();
        let slot = self.recorded.clone();
        self.act(label, move |client, path| {
            let result = action(client, path)?;
            if let (true, Some(before)) = (on_branch, before) {
                let state = client.checkout_state(path)?;
                if state.operation.is_none() && state.current_branch == branch {
                    if let Some(after) = state.head_hash.filter(|after| *after != before) {
                        let step = UndoStep::BranchMove { branch, before, after, mode };
                        *slot.lock().unwrap_or_else(|p| p.into_inner()) = Some(Recorded::Step(HistoryEntry { title, step }));
                    }
                }
            }
            Ok(result)
        });
    }

    /// Runs an action that returns its own undo record, such as deleting a branch.
    pub fn act_with_record(
        &mut self,
        label: &str,
        action: impl FnOnce(&GitClient, &Path) -> nicegit_core::Result<(Option<String>, Option<Recorded>)> + Send + 'static,
    ) {
        let slot = self.recorded.clone();
        self.act(label, move |client, path| {
            let (message, record) = action(client, path)?;
            *slot.lock().unwrap_or_else(|p| p.into_inner()) = record;
            Ok(message)
        });
    }

    pub fn notify(&mut self, text: impl Into<String>, is_error: bool) {
        self.notice = Some(Notice { text: text.into(), is_error });
    }

    fn receive(&mut self) {
        while let Ok(message) = self.worker.receiver.try_recv() {
            match message {
                Message::Loaded { generation, path, action, snapshot } => {
                    if action.is_some() {
                        self.busy = None;
                    }
                    let Some(index) = self.repos.iter().position(|r| r.path == path && r.generation == generation) else { continue };
                    self.repos[index].loading = false;
                    let mut action_failed = false;
                    if action.is_some() {
                        self.take_record(index);
                    }
                    if let Some((label, result)) = action {
                        match result {
                            Ok(Some(text)) => self.record_result(index, &text),
                            Ok(None) => {}
                            Err(error) => {
                                action_failed = true;
                                self.notify(format!("{label} failed: {error}"), true);
                            }
                        }
                    }
                    match *snapshot {
                        Ok(snapshot) => self.apply(index, snapshot),
                        Err(error) => {
                            if self.repos[index].snapshot.is_none() {
                                self.repos[index].error = Some(error.to_string());
                            } else if !action_failed {
                                // The action itself completed; only the refresh failed.
                                let done = self.notice.take().map(|n| n.text + " ").unwrap_or_default();
                                self.notify(format!("{done}Refreshing the repository failed: {error}"), true);
                            }
                        }
                    }
                }
                Message::Diff { generation, title, result } => {
                    let Some(repo) = self.repos.iter_mut().find(|r| r.diff_generation == generation) else { continue };
                    repo.diff_loading = false;
                    match result {
                        Ok(lines) => repo.diff = Some(DiffContent::new(title, lines)),
                        Err(error) => {
                            repo.diff = None;
                            self.notify(error.to_string(), true);
                        }
                    }
                }
                Message::CommitFiles { generation, hash, result } => {
                    let Some(repo) = self.repos.iter_mut().find(|r| r.diff_generation == generation) else { continue };
                    if !matches!(&repo.selection, Selection::Commit { hash: h, .. } if *h == hash) {
                        continue;
                    }
                    match result {
                        Ok(files) => repo.commit_files = files,
                        Err(error) => self.notify(error.to_string(), true),
                    }
                }
            }
        }
    }

    fn record_result(&mut self, _index: usize, text: &str) {
        self.notify(text.to_string(), false);
    }

    /// Takes whatever the finished action recorded for undo.
    fn take_record(&mut self, index: usize) {
        let record = self.recorded.lock().unwrap_or_else(|p| p.into_inner()).take();
        let repo = &mut self.repos[index];
        match record {
            Some(Recorded::Step(entry)) => {
                repo.undo = Some(entry);
                repo.redo = None;
            }
            Some(Recorded::Redo(entry)) => repo.redo = Some(entry),
            Some(Recorded::Discard(undo)) => {
                repo.discard_undo.retain(|older| older.path != undo.path);
                repo.discard_undo.push(undo);
                let excess = repo.discard_undo.len().saturating_sub(20);
                repo.discard_undo.drain(..excess);
            }
            None => {}
        }
    }

    fn apply(&mut self, index: usize, snapshot: Snapshot) {
        self.settings.remember(PathBuf::from(&snapshot.root_path));
        let repo = &mut self.repos[index];
        repo.error = None;
        let dirty = !snapshot.status.is_empty();
        let colors = theme::GRAPH_COLOR_COUNT;
        repo.rows = if dirty {
            layout_with_working_tree(&snapshot.commits, snapshot.head_hash.as_deref(), colors)
        } else {
            layout(&snapshot.commits, snapshot.head_hash.as_deref(), colors)
        };
        repo.lanes = repo.rows.iter().map(|row| row.lane_count).max().unwrap_or(1);
        // Keep the selection only while it still exists in the new state.
        let keep = match &repo.selection {
            Selection::None => true,
            Selection::WorkingTree => dirty,
            Selection::Change { entry, staged } => {
                snapshot.status.iter().any(|e| e == entry && (if *staged { e.is_staged() } else { e.is_unstaged() }))
            }
            Selection::Commit { hash, .. } => snapshot.commits.iter().any(|c| &c.hash == hash),
            Selection::Stash { hash } => snapshot.stashes.iter().any(|s| &s.hash == hash),
        };
        repo.marked.retain(|hash| snapshot.commits.iter().any(|c| &c.hash == hash));
        // Each checkout keeps its own commit message draft.
        let key = Settings::draft_key(&snapshot.root_path, &snapshot.current_branch);
        if repo.draft_key.as_deref() != Some(key.as_str()) {
            if let Some(old) = repo.draft_key.take() {
                if repo.draft.is_empty() {
                    self.settings.drafts.remove(&old);
                } else {
                    self.settings.drafts.insert(old, repo.draft.clone());
                }
            }
            repo.draft = self.settings.drafts.get(&key).cloned().unwrap_or_default();
            repo.draft_key = Some(key);
            repo.amend = false;
        }
        let reload_change = matches!(repo.selection, Selection::Change { .. }) && keep;
        if index == self.active {
            for tool in &mut self.tools {
                tool.repository_changed(&snapshot);
            }
        }
        self.repos[index].snapshot = Some(snapshot);
        // Bisect state and submodules are read alongside, without delaying the snapshot.
        let context = self.worker.context.clone();
        let path = self.repos[index].path.clone();
        let bisect_path = path.clone();
        self.repos[index].bisect = Some(Task::spawn(&context, move || GitClient::new().bisect_status(&bisect_path).ok().flatten()));
        self.repos[index].submodules = Some(Task::spawn(&context, move || GitClient::new().submodules(&path).unwrap_or_default()));
        if index != self.active {
            return;
        }
        if !keep {
            self.clear_selection();
        } else if reload_change {
            if let Selection::Change { entry, staged } = self.repos[index].selection.clone() {
                self.select_change(entry, staged);
            }
        }
        self.watch_active();
    }

    pub fn save_draft(&mut self) {
        let Some(repo) = self.repos.get(self.active) else { return };
        if let Some(key) = repo.draft_key.clone() {
            if repo.draft.is_empty() {
                self.settings.drafts.remove(&key);
            } else {
                self.settings.drafts.insert(key, repo.draft.clone());
            }
        }
    }

    // MARK: Selection

    pub fn clear_selection(&mut self) {
        let Some(repo) = self.repo_mut() else { return };
        repo.selection = Selection::None;
        repo.diff = None;
        repo.details = None;
        repo.review = None;
        repo.diff_options.selected.clear();
        repo.commit_files.clear();
        repo.diff_generation = next_id();
        repo.diff_loading = false;
    }

    fn start_diff(&mut self) -> Option<(u64, PathBuf)> {
        let repo = self.repos.get_mut(self.active)?;
        repo.review = None;
        repo.diff_options.selected.clear();
        repo.diff_generation = next_id();
        repo.diff_loading = true;
        Some((repo.diff_generation, repo.path.clone()))
    }

    pub fn select_working_tree(&mut self) {
        let first = self.snapshot().and_then(|s| s.status.first().cloned());
        match first {
            Some(entry) => {
                let staged = !entry.is_unstaged();
                self.select_change(entry, staged);
            }
            None => {
                self.clear_selection();
                if let Some(repo) = self.repo_mut() {
                    repo.selection = Selection::WorkingTree;
                }
            }
        }
    }

    pub fn select_change(&mut self, entry: StatusEntry, staged: bool) {
        let ignore_whitespace = self.settings.ignore_whitespace;
        let Some((generation, path)) = self.start_diff() else { return };
        let repo = self.repo_mut().expect("active repository");
        repo.commit_files.clear();
        repo.details = None;
        repo.selection = Selection::Change { entry: entry.clone(), staged };
        let title = entry.path.clone();
        if !ignore_whitespace && entry.kind != nicegit_core::StatusKind::Untracked {
            // A review carries the same lines as the diff, plus what staging single lines needs.
            let (review_path, file) = (path.clone(), entry.path.clone());
            let context = self.worker.context.clone();
            self.repo_mut().expect("active repository").review =
                Some(Task::spawn(&context, move || GitClient::new().file_review(&file, staged, &review_path)));
        }
        self.worker.diff(generation, title, move |client| client.diff_with(&entry, staged, ignore_whitespace, &path));
    }

    pub fn select_commit(&mut self, hash: String) {
        let ignore_whitespace = self.settings.ignore_whitespace;
        let Some((generation, path)) = self.start_diff() else { return };
        let context = self.worker.context.clone();
        let repo = self.repo_mut().expect("active repository");
        repo.commit_files.clear();
        repo.selection = Selection::Commit { hash: hash.clone(), file: None };
        repo.scroll_to_selection = true;
        let (details_path, details_hash) = (path.clone(), hash.clone());
        repo.details = Some(Task::spawn(&context, move || GitClient::new().commit_details(&details_hash, &details_path)));
        self.worker.commit_files(generation, path.clone(), hash.clone());
        let title = format!("Commit {}", short(&hash));
        self.worker.diff(generation, title, move |client| client.commit_diff_with(&hash, None, ignore_whitespace, &path));
    }

    pub fn select_commit_file(&mut self, hash: String, file: Option<String>) {
        let ignore_whitespace = self.settings.ignore_whitespace;
        let Some((generation, path)) = self.start_diff() else { return };
        let repo = self.repo_mut().expect("active repository");
        repo.selection = Selection::Commit { hash: hash.clone(), file: file.clone() };
        let title = file.clone().unwrap_or_else(|| format!("Commit {}", short(&hash)));
        self.worker.diff(generation, title, move |client| client.commit_diff_with(&hash, file.as_deref(), ignore_whitespace, &path));
    }

    pub fn select_stash(&mut self, stash: Stash) {
        let Some((generation, path)) = self.start_diff() else { return };
        let repo = self.repo_mut().expect("active repository");
        repo.commit_files.clear();
        repo.details = None;
        repo.selection = Selection::Stash { hash: stash.hash.clone() };
        let title = format!("{} · {}", stash.reference, stash.message);
        self.worker.diff(generation, title, move |client| client.stash_diff(&stash, &path));
    }

    /// Reloads whatever diff is showing, for example after a diff option changes.
    pub fn reload_diff(&mut self) {
        let Some(repo) = self.repo() else { return };
        match repo.selection.clone() {
            Selection::Change { entry, staged } => self.select_change(entry, staged),
            Selection::Commit { hash, file } => self.select_commit_file(hash, file),
            _ => {}
        }
    }

    // MARK: Actions

    pub fn checkout(&mut self, branch: Branch) {
        let Some(snapshot) = self.snapshot().cloned() else { return };
        if snapshot.operation.is_some() {
            self.notify("Finish or abort the current Git operation before switching branches.", true);
            return;
        }
        let expected = snapshot.current_branch.clone();
        let head = snapshot.head_hash.clone();
        let name = branch.display_name();
        self.act(&format!("Switch to {name}"), move |client, path| {
            let stashed = if branch.is_remote {
                client.checkout_remote(&branch, &snapshot.remotes, &expected, head.as_deref(), path)?
            } else {
                client.checkout(&branch, &expected, head.as_deref(), path)?
            };
            Ok(Some(if stashed {
                format!("Switched to {name}. Your uncommitted changes were saved in a stash; apply it from Stashes when you need them.")
            } else {
                format!("Switched to {name}.")
            }))
        });
    }

    pub fn commit(&mut self) {
        let Some(snapshot) = self.snapshot().cloned() else { return };
        let Some(repo) = self.repo() else { return };
        let message = repo.draft.message();
        if repo.amend {
            let Some(head) = snapshot.head_hash.clone() else { return };
            let branch = snapshot.current_branch.clone();
            self.act_recording("Amend", "Amend", move |client, path| {
                client.amend(&message, &branch, &head, path).map(|_| Some("Amended the last commit.".into()))
            });
        } else {
            self.act_recording("Commit", "Commit", move |client, path| client.commit(&message, path).map(|_| None));
        }
        if self.busy.is_some() {
            if let Some(repo) = self.repo_mut() {
                repo.draft = Draft::default();
                repo.amend = false;
            }
            self.save_draft();
        }
    }

    pub fn can_commit(&self) -> bool {
        let (Some(snapshot), Some(repo)) = (self.snapshot(), self.repo()) else { return false };
        self.idle()
            && !repo.draft.summary.trim().is_empty()
            && (snapshot.staged_count() > 0 || repo.amend)
            && !snapshot.status.iter().any(|e| e.kind == nicegit_core::StatusKind::Conflicted)
    }

    pub fn pull(&mut self) {
        let Some(snapshot) = self.snapshot().cloned() else { return };
        self.act_recording("Pull", "Pull", move |client, path| {
            client.pull(&snapshot, path).map(|_| Some("Pulled the latest changes.".into()))
        });
    }

    pub fn push(&mut self) {
        let Some(snapshot) = self.snapshot().cloned() else { return };
        if snapshot.upstream.is_none() {
            let remote = snapshot.remotes.iter().find(|r| *r == "origin").or(snapshot.remotes.first()).cloned();
            match remote {
                Some(remote) => self.dialog = Some(Dialog::Publish { remote }),
                None => self.notify("Add a remote in Repository Settings before pushing.", true),
            }
            return;
        }
        self.act("Push", move |client, path| client.push(&snapshot, path).map(|_| Some("Pushed.".into())));
    }

    pub fn fetch(&mut self) {
        self.act("Fetch", |client, path| client.fetch(path).map(|_| Some("Fetched from all remotes.".into())));
    }

    /// Undoes the last recorded action; what it returns becomes the redo.
    pub fn undo(&mut self) {
        let Some(entry) = self.repo_mut().and_then(|r| r.undo.take()) else { return };
        self.apply_history(entry, false);
    }

    pub fn redo(&mut self) {
        let Some(entry) = self.repo_mut().and_then(|r| r.redo.take()) else { return };
        self.apply_history(entry, true);
    }

    fn apply_history(&mut self, entry: HistoryEntry, redo: bool) {
        let slot = self.recorded.clone();
        let verb = if redo { "Redo" } else { "Undo" };
        let label = format!("{verb} {}", entry.title.to_lowercase());
        self.act(&label, move |client, path| {
            let inverse = HistoryEntry { title: entry.title.clone(), step: client.undo_step(&entry.step, path)? };
            // Undoing offers the inverse as a redo; redoing offers it as an undo again.
            *slot.lock().unwrap_or_else(|p| p.into_inner()) = Some(if redo { Recorded::Step(inverse) } else { Recorded::Redo(inverse) });
            Ok(Some(format!("{verb}: {}.", entry.title.to_lowercase())))
        });
    }

    /// Reverses the last discard, if the file has not changed since.
    pub fn undo_discard(&mut self) {
        let Some(undo) = self.repo_mut().and_then(|r| r.discard_undo.pop()) else { return };
        let name = undo.path.clone();
        self.act("Undo discard", move |client, path| {
            client.undo_discard(&undo, path).map(|_| Some(format!("Restored your changes to {name}.")))
        });
    }

    pub fn confirm(&mut self, title: impl Into<String>, message: impl Into<String>, button: impl Into<String>, pending: Pending) {
        self.dialog = Some(Dialog::Confirm { title: title.into(), message: message.into(), button: button.into(), pending });
    }

    // MARK: Tools

    pub fn open_tool(&mut self, tool: Box<dyn ToolWindow>) {
        let id = tool.id();
        if let Some(index) = self.tools.iter().position(|open| open.id() == id) {
            if self.tools[index].has_unsaved_changes() {
                return;
            }
            self.tools.remove(index);
        }
        self.tools.push(tool);
    }

    pub fn has_unsaved_editor(&self) -> bool {
        self.tools.iter().any(|tool| tool.has_unsaved_changes())
    }

    fn tool_windows(&mut self, ctx: &egui::Context) {
        let Some(repo) = self.repos.get(self.active) else { return };
        let (Some(snapshot), path) = (repo.snapshot.clone(), repo.path.clone()) else { return };
        let idle = self.busy.is_none();
        let mut requests = Vec::new();
        let mut closed = Vec::new();
        let mut blocked_close = false;
        for (index, tool) in self.tools.iter_mut().enumerate() {
            let mut open = true;
            // Windows stay within the app window, so their buttons are never off-screen.
            let screen = ctx.content_rect().size();
            let size = tool.default_size().min(screen - egui::vec2(40.0, 60.0));
            egui::Window::new(tool.title())
                .id(egui::Id::new(("tool", tool.id())))
                .default_size(size)
                .max_size(screen - egui::vec2(20.0, 40.0))
                .constrain(true)
                .collapsible(false)
                .resizable(true)
                .open(&mut open)
                .show(ctx, |ui| {
                    let mut cx = Ctx::new(&path, &snapshot, idle, &mut requests);
                    tool.ui(ui, &mut cx);
                });
            if tool.wants_close() || (!open && !tool.has_unsaved_changes()) {
                closed.push(index);
            } else if !open {
                blocked_close = true;
            }
        }
        for index in closed.into_iter().rev() {
            self.tools.remove(index);
        }
        if blocked_close {
            self.notify("Save or discard your edits before closing the editor.", true);
        }
        for request in requests {
            self.handle(request);
        }
    }

    pub fn handle(&mut self, request: Request) {
        match request {
            Request::Act { label, action } => self.act(&label, action),
            Request::ActRecording { label, title, mode, action } => self.act_recording_mode(&label, &title, mode, action),
            Request::RecordUndo { title, step } => {
                if let Some(repo) = self.repo_mut() {
                    repo.undo = Some(HistoryEntry { title, step });
                    repo.redo = None;
                }
            }
            Request::Notice { text, is_error } => self.notify(text, is_error),
            Request::SelectCommit(hash) => self.select_commit(hash),
            Request::OpenRepository(path) => self.open(path),
            Request::Open(tool) => self.open_tool(tool),
            Request::Refresh => self.load(true),
        }
    }

    fn finish_clone(&mut self) {
        let Some(task) = self.pending_clone.as_mut() else { return };
        let Some(result) = task.get() else { return };
        let result = result.clone();
        self.pending_clone = None;
        self.busy = None;
        match result {
            Ok(path) => {
                self.notify(format!("Cloned into {}.", path.display()), false);
                self.open(path);
            }
            Err(error) => self.notify(format!("Clone failed: {error}"), true),
        }
    }

    // MARK: Refresh

    /// Watches the active repository for changes made in other apps.
    fn watch_active(&mut self) {
        use notify::Watcher;
        let Some(path) = self.repo().map(|r| r.path.clone()) else { return };
        if !self.settings.auto_refresh {
            self.watcher = None;
            return;
        }
        if self.watcher.as_ref().is_some_and(|watch| watch.path == path) {
            return;
        }
        let (sender, events) = std::sync::mpsc::channel();
        let context = self.worker.context.clone();
        let watcher = notify::recommended_watcher(move |event| {
            let _ = sender.send(event);
            context.request_repaint_after(WATCH_DEBOUNCE);
        });
        self.watcher = None;
        if let Ok(mut watcher) = watcher {
            if watcher.watch(&path, notify::RecursiveMode::Recursive).is_ok() {
                self.watcher = Some(Watch { path, _watcher: watcher, events });
            }
        }
    }

    fn auto_refresh(&mut self, ctx: &egui::Context) {
        let focused = ctx.input(|i| i.viewport().focused.unwrap_or(true));
        let regained = focused && !self.was_focused;
        self.was_focused = focused;
        if let Some(watch) = &self.watcher {
            // NiceGit's own actions refresh as they finish; skip changes from before then.
            let last_load = self.repo().map(|r| r.last_load).unwrap_or_else(Instant::now);
            while let Ok(event) = watch.events.try_recv() {
                let Ok(event) = event else { continue };
                // Object writes and lock files come with every Git command; the index, refs,
                // and working files are what change the visible state.
                let relevant = event.paths.iter().any(|p| {
                    let text = p.to_string_lossy().replace('\\', "/");
                    !text.contains("/.git/objects/") && !text.ends_with(".lock") && !text.contains("/target/")
                });
                if relevant && last_load.elapsed() > WATCH_DEBOUNCE {
                    self.watched_change.get_or_insert_with(Instant::now);
                }
            }
        }
        let quiet = self.busy.is_none() && self.dialog.is_none() && self.repo().is_some_and(|r| !r.loading && r.snapshot.is_some());
        if !quiet || !self.settings.auto_refresh {
            return;
        }
        let watched = self.watched_change.is_some_and(|at| at.elapsed() >= WATCH_DEBOUNCE);
        let periodic = focused && self.repo().is_some_and(|r| r.last_load.elapsed() >= AUTO_REFRESH);
        if regained || watched || periodic {
            self.watched_change = None;
            self.load(false);
        }
        if focused {
            ctx.request_repaint_after(AUTO_REFRESH);
        }
    }

    /// On first launch the window may be larger than a small screen; shrink it to fit.
    fn fit_to_monitor(&mut self, ctx: &egui::Context) {
        if self.fitted_to_monitor {
            return;
        }
        let (monitor, inner) = ctx.input(|i| (i.viewport().monitor_size, i.viewport().inner_rect));
        let (Some(monitor), Some(inner)) = (monitor, inner) else { return };
        self.fitted_to_monitor = true;
        let fits = egui::vec2(monitor.x - 40.0, monitor.y - 80.0);
        if inner.width() > fits.x || inner.height() > fits.y {
            let size = inner.size().min(fits).max(egui::vec2(900.0, 560.0));
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
            ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(20.0, 20.0)));
        }
    }

    fn apply_theme(&mut self, ctx: &egui::Context) {
        let appearance = if self.preview_light { theme::Appearance::Light } else { self.settings.appearance };
        let wanted = (appearance, self.settings.graph_palette);
        if self.applied_appearance != Some(wanted) {
            theme::apply(ctx, wanted.0);
            theme::set_graph_palette(wanted.1);
            self.applied_appearance = Some(wanted);
        }
    }

    /// Whether the branch tip is in the loaded history of the current checkout.
    pub fn branch_is_merged(branch: &Branch, snapshot: &Snapshot) -> bool {
        let Some(head) = snapshot.head_hash.as_deref() else { return false };
        let by_hash: std::collections::HashMap<&str, &nicegit_core::Commit> =
            snapshot.commits.iter().map(|c| (c.hash.as_str(), c)).collect();
        let mut stack = vec![head];
        let mut seen = std::collections::HashSet::new();
        while let Some(hash) = stack.pop() {
            if hash == branch.tip {
                return true;
            }
            if !seen.insert(hash) {
                continue;
            }
            if let Some(commit) = by_hash.get(hash) {
                stack.extend(commit.parents.iter().map(String::as_str));
            }
        }
        false
    }
}

/// Windows' canonical paths start with `\\?\`, which Git and users do not expect.
fn strip_verbatim(path: PathBuf) -> PathBuf {
    let text = path.to_string_lossy();
    match text.strip_prefix(r"\\?\") {
        Some(rest) if !rest.starts_with("UNC") => PathBuf::from(rest),
        _ => path,
    }
}

fn next_id() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

impl eframe::App for NiceGitApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.apply_theme(&ctx);
        self.fit_to_monitor(&ctx);
        self.receive();
        self.finish_clone();
        self.shortcuts(&ctx);
        self.auto_refresh(&ctx);
        self.debug_open(&ctx);
        self.layout(ui);
        self.tool_windows(&ctx);
        self.dialogs(&ctx);
        self.palette_window(&ctx);
        self.settings_window(&ctx);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        self.save_draft();
        self.settings.open = self.repos.iter().map(|r| r.path.clone()).collect();
        self.settings.active = self.active;
        eframe::set_value(storage, STORAGE_KEY, &self.settings);
    }
}
