use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use egui::{Align, Color32, Key, KeyboardShortcut, Layout, Modifiers, RichText, Sense, Ui};
use nicegit_core::graph::{layout_with_working_tree, GraphRow, WORKING_TREE_HASH};
use nicegit_core::models::short;
use nicegit_core::{Branch, GitClient, Operation, Snapshot, Stash, StatusEntry};

use crate::diff_view::{self, DiffContent};
use crate::graph_view;
use crate::tools::{Ctx, Request, ToolWindow};
use crate::worker::{ActionResult, Message, Worker};

const PAGE: usize = 500;
const ROW_HEIGHT: f32 = 24.0;
const AUTO_REFRESH: Duration = Duration::from_secs(10);
const RECENT_KEY: &str = "recent_repositories";
const RECENT_SEPARATOR: char = '\u{1f}';

struct Notice {
    text: String,
    is_error: bool,
}

/// Something that needs confirming before it runs. Each captures what the user saw, so the
/// action can refuse if the repository changed in the meantime.
enum Pending {
    Discard(StatusEntry),
    DiscardAll(Vec<StatusEntry>),
    DeleteBranch { branch: Branch, force: bool },
    DeleteTag { name: String, tip: String },
    DropStash(Stash),
    UndoCommit { branch: String, head: String, subject: String },
    Merge { source: Branch, branch: String, head: Option<String> },
    Abort(Operation),
}

enum InputKind {
    CreateBranch { branch: String, head: Option<String> },
    CreateBranchFrom(Branch),
    RenameBranch(Branch),
    CreateTag { target: String },
    SaveStash,
    AddRemote,
    Clone,
}

enum Dialog {
    Confirm { title: String, message: String, button: String, pending: Pending },
    Input { title: String, label: String, value: String, second_label: Option<String>, second: String, kind: InputKind },
    Publish { remote: String },
}

#[derive(Clone)]
struct Redo {
    commit: String,
    branch: String,
}

#[derive(Clone, PartialEq)]
enum Selection {
    None,
    Change { entry: StatusEntry, staged: bool },
    Commit { hash: String, file: Option<String> },
    Stash(String),
}

pub struct NiceGitApp {
    worker: Worker,
    repository: Option<PathBuf>,
    snapshot: Option<Snapshot>,
    rows: Vec<GraphRow>,
    graph_width: f32,
    recent: Vec<PathBuf>,
    busy: Option<String>,
    generation: u64,
    diff_generation: u64,
    history_limit: usize,
    notice: Option<Notice>,
    selection: Selection,
    commit_files: Vec<(String, String)>,
    diff: Option<DiffContent>,
    diff_loading: bool,
    split_diff: bool,
    commit_message: String,
    amend: bool,
    dialog: Option<Dialog>,
    redo: Option<Redo>,
    was_focused: bool,
    last_load: Instant,
    git_missing: bool,
    tools: Vec<Box<dyn ToolWindow>>,
}

impl NiceGitApp {
    pub fn new(creation: &eframe::CreationContext<'_>, initial: Option<PathBuf>) -> Self {
        let recent: Vec<PathBuf> = creation
            .storage
            .and_then(|storage| storage.get_string(RECENT_KEY))
            .map(|value| value.split(RECENT_SEPARATOR).filter(|p| !p.is_empty()).map(PathBuf::from).collect())
            .unwrap_or_default();
        let mut app = Self {
            worker: Worker::new(creation.egui_ctx.clone()),
            repository: None,
            snapshot: None,
            rows: Vec::new(),
            graph_width: 40.0,
            recent,
            busy: None,
            generation: 0,
            diff_generation: 0,
            history_limit: PAGE,
            notice: None,
            selection: Selection::None,
            commit_files: Vec::new(),
            diff: None,
            diff_loading: false,
            split_diff: false,
            commit_message: String::new(),
            amend: false,
            dialog: None,
            redo: None,
            was_focused: true,
            last_load: Instant::now(),
            git_missing: nicegit_core::runner::git_executable().is_none(),
            tools: Vec::new(),
        };
        crate::theme::install_fonts(&creation.egui_ctx);
        crate::theme::apply(&creation.egui_ctx, crate::theme::Appearance::System);
        let start = initial.or_else(|| app.recent.first().cloned()).filter(|path| path.exists());
        if let Some(path) = start {
            app.open(path);
        }
        app
    }

    // MARK: Loading

    fn open(&mut self, path: PathBuf) {
        if self.busy.is_some() {
            return;
        }
        self.repository = Some(path.clone());
        self.snapshot = None;
        self.rows.clear();
        self.history_limit = PAGE;
        self.redo = None;
        self.commit_message.clear();
        self.amend = false;
        self.clear_selection();
        self.load(true);
    }

    fn load(&mut self, deliberate: bool) {
        let Some(path) = self.repository.clone() else { return };
        if self.busy.is_some() {
            return;
        }
        self.generation += 1;
        self.busy = Some(if deliberate { "Loading".into() } else { String::new() });
        self.last_load = Instant::now();
        self.worker.load(self.generation, path, deliberate, self.history_limit);
    }

    /// Runs an action in the background, then refreshes. Refused while another action runs.
    fn act(&mut self, label: &str, action: impl FnOnce(&GitClient, &Path) -> ActionResult + Send + 'static) {
        let Some(path) = self.repository.clone() else { return };
        if self.busy.is_some() {
            return;
        }
        self.generation += 1;
        self.busy = Some(label.to_string());
        self.notice = None;
        self.last_load = Instant::now();
        self.worker.act(self.generation, path, label.to_string(), self.history_limit, action);
    }

    fn receive(&mut self) {
        while let Ok(message) = self.worker.receiver.try_recv() {
            match message {
                Message::Loaded { generation, path, action, snapshot } => {
                    if generation != self.generation || Some(&path) != self.repository.as_ref() {
                        continue;
                    }
                    self.busy = None;
                    let mut action_failed = false;
                    if let Some((label, result)) = action {
                        match result {
                            Ok(Some(text)) => self.notice = Some(Notice { text, is_error: false }),
                            Ok(None) => {}
                            Err(error) => {
                                action_failed = true;
                                self.notice = Some(Notice { text: format!("{label} failed: {error}"), is_error: true });
                            }
                        }
                    }
                    match *snapshot {
                        Ok(snapshot) => self.apply(snapshot),
                        Err(error) if self.snapshot.is_none() => {
                            self.notice = Some(Notice { text: format!("Could not open {}: {error}", path.display()), is_error: true });
                            self.repository = None;
                        }
                        Err(error) if !action_failed => {
                            // The action itself completed; only the refresh failed.
                            let done = self.notice.take().map(|n| n.text + " ").unwrap_or_default();
                            self.notice = Some(Notice { text: format!("{done}Refreshing the repository failed: {error}"), is_error: true });
                        }
                        Err(_) => {}
                    }
                }
                Message::Diff { generation, title, result } => {
                    if generation != self.diff_generation {
                        continue;
                    }
                    self.diff_loading = false;
                    match result {
                        Ok(lines) => self.diff = Some(DiffContent::new(title, lines)),
                        Err(error) => {
                            self.diff = None;
                            self.notice = Some(Notice { text: error.to_string(), is_error: true });
                        }
                    }
                }
                Message::CommitFiles { generation, hash, result } => {
                    if generation != self.diff_generation || !matches!(&self.selection, Selection::Commit { hash: h, .. } if *h == hash) {
                        continue;
                    }
                    match result {
                        Ok(files) => self.commit_files = files,
                        Err(error) => self.notice = Some(Notice { text: error.to_string(), is_error: true }),
                    }
                }
            }
        }
    }

    fn apply(&mut self, snapshot: Snapshot) {
        let path = PathBuf::from(&snapshot.root_path);
        self.recent.retain(|p| p != &path && p != self.repository.as_ref().unwrap_or(&path));
        self.recent.insert(0, path.clone());
        self.recent.truncate(12);
        self.repository = Some(path);
        let dirty = !snapshot.status.is_empty();
        self.rows = if dirty {
            layout_with_working_tree(&snapshot.commits, snapshot.head_hash.as_deref(), graph_view::PALETTE.len())
        } else {
            nicegit_core::graph::layout(&snapshot.commits, snapshot.head_hash.as_deref(), graph_view::PALETTE.len())
        };
        let lanes = self.rows.iter().map(|row| row.lane_count).max().unwrap_or(1);
        self.graph_width = graph_view::width_for(lanes).min(260.0);
        // Keep the selection only while it still exists in the new state.
        let keep = match &self.selection {
            Selection::None => true,
            Selection::Change { entry, staged } => {
                snapshot.status.iter().any(|e| e == entry && (if *staged { e.is_staged() } else { e.is_unstaged() }))
            }
            Selection::Commit { hash, .. } => snapshot.commits.iter().any(|c| &c.hash == hash),
            Selection::Stash(hash) => snapshot.stashes.iter().any(|s| &s.hash == hash),
        };
        let reload_change = matches!(self.selection, Selection::Change { .. }) && keep;
        for tool in &mut self.tools {
            tool.repository_changed(&snapshot);
        }
        self.snapshot = Some(snapshot);
        if !keep {
            self.clear_selection();
        } else if reload_change {
            // The file may have changed; show its current diff.
            if let Selection::Change { entry, staged } = self.selection.clone() {
                self.select_change(entry, staged);
            }
        }
    }

    fn clear_selection(&mut self) {
        self.selection = Selection::None;
        self.diff = None;
        self.commit_files.clear();
        self.diff_generation += 1;
        self.diff_loading = false;
    }

    fn select_change(&mut self, entry: StatusEntry, staged: bool) {
        let Some(path) = self.repository.clone() else { return };
        self.diff_generation += 1;
        self.diff_loading = true;
        self.commit_files.clear();
        self.selection = Selection::Change { entry: entry.clone(), staged };
        let title = format!("{} ({})", entry.path, if staged { "staged" } else { "unstaged" });
        self.worker.diff(self.diff_generation, title, move |client| client.diff(&entry, staged, &path));
    }

    fn select_commit(&mut self, hash: String) {
        let Some(path) = self.repository.clone() else { return };
        self.diff_generation += 1;
        self.diff_loading = true;
        self.commit_files.clear();
        self.selection = Selection::Commit { hash: hash.clone(), file: None };
        self.worker.commit_files(self.diff_generation, path.clone(), hash.clone());
        let title = format!("Commit {}", short(&hash));
        self.worker.diff(self.diff_generation, title, move |client| client.commit_diff(&hash, None, &path));
    }

    fn select_commit_file(&mut self, hash: String, file: Option<String>) {
        let Some(path) = self.repository.clone() else { return };
        self.diff_generation += 1;
        self.diff_loading = true;
        self.selection = Selection::Commit { hash: hash.clone(), file: file.clone() };
        let title = file.clone().unwrap_or_else(|| format!("Commit {}", short(&hash)));
        self.worker.diff(self.diff_generation, title, move |client| client.commit_diff(&hash, file.as_deref(), &path));
    }

    fn select_stash(&mut self, stash: Stash) {
        let Some(path) = self.repository.clone() else { return };
        self.diff_generation += 1;
        self.diff_loading = true;
        self.commit_files.clear();
        self.selection = Selection::Stash(stash.hash.clone());
        let title = format!("Stash {}", stash.reference);
        self.worker.diff(self.diff_generation, title, move |client| client.stash_diff(&stash, &path));
    }

    fn choose_folder(&mut self) {
        if self.busy.is_some() {
            return;
        }
        if let Some(folder) = rfd::FileDialog::new().set_title("Open a Git repository").pick_folder() {
            self.open(folder);
        }
    }

    // MARK: Actions

    fn idle(&self) -> bool {
        self.busy.is_none() && self.snapshot.is_some()
    }

    fn checkout(&mut self, branch: Branch) {
        let Some(snapshot) = self.snapshot.clone() else { return };
        if snapshot.operation.is_some() {
            self.notice =
                Some(Notice { text: "Finish or abort the current Git operation before switching branches.".into(), is_error: true });
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

    fn commit(&mut self) {
        let Some(snapshot) = self.snapshot.clone() else { return };
        let message = self.commit_message.clone();
        if self.amend {
            let Some(head) = snapshot.head_hash.clone() else { return };
            let branch = snapshot.current_branch.clone();
            self.act("Amend", move |client, path| {
                client.amend(&message, &branch, &head, path).map(|_| Some("Amended the last commit.".into()))
            });
        } else {
            self.act("Commit", move |client, path| client.commit(&message, path).map(|_| None));
        }
        self.commit_message.clear();
        self.amend = false;
        self.redo = None;
    }

    fn run_pending(&mut self, pending: Pending) {
        match pending {
            Pending::Discard(entry) => {
                let name = entry.path.clone();
                self.act("Discard", move |client, path| client.discard(&entry, path).map(|_| Some(format!("Discarded changes to {name}."))))
            }
            Pending::DiscardAll(entries) => self.act("Discard", move |client, path| {
                for entry in &entries {
                    client.discard(entry, path)?;
                }
                Ok(Some(format!("Discarded {} changed files.", entries.len())))
            }),
            Pending::DeleteBranch { branch, force } => {
                let name = branch.name.clone();
                self.act("Delete branch", move |client, path| {
                    client.delete_branch(&branch, force, path).map(|_| Some(format!("Deleted branch {name}.")))
                })
            }
            Pending::DeleteTag { name, tip } => self
                .act("Delete tag", move |client, path| client.delete_tag(&name, &tip, path).map(|_| Some(format!("Deleted tag {name}.")))),
            Pending::DropStash(stash) => {
                self.act("Drop stash", move |client, path| client.drop_stash(&stash, path).map(|_| Some("Deleted the stash.".into())))
            }
            Pending::UndoCommit { branch, head, subject } => {
                self.redo = Some(Redo { commit: head.clone(), branch: branch.clone() });
                if let Some(snapshot) = &self.snapshot {
                    if self.commit_message.trim().is_empty() {
                        self.commit_message =
                            snapshot.commits.iter().find(|c| c.hash == head).map(|c| c.subject.clone()).unwrap_or_default();
                    }
                }
                self.act("Undo commit", move |client, path| {
                    client.undo_last_commit(&branch, &head, path).map(|_| Some(format!("Undid “{subject}”. Its changes are staged.")))
                });
            }
            Pending::Merge { source, branch, head } => {
                let name = source.display_name();
                self.act("Merge", move |client, path| {
                    client.merge(&source, &branch, head.as_deref(), path).map(|_| Some(format!("Merged {name}.")))
                })
            }
            Pending::Abort(operation) => self.act("Abort", move |client, path| {
                client.abort_operation(operation, path).map(|_| Some(format!("Aborted the {}.", operation.name())))
            }),
        }
    }

    fn run_input(&mut self, kind: InputKind, value: String, second: String) {
        match kind {
            InputKind::CreateBranch { branch, head } => self.act("Create branch", move |client, path| {
                client
                    .create_branch(&value, &branch, head.as_deref(), path)
                    .map(|_| Some(format!("Created and switched to {}.", value.trim())))
            }),
            InputKind::CreateBranchFrom(source) => self.act("Create branch", move |client, path| {
                client.create_branch_from(&value, &source, path).map(|_| Some(format!("Created branch {}.", value.trim())))
            }),
            InputKind::RenameBranch(branch) => {
                self.act("Rename branch", move |client, path| client.rename_branch(&branch, &value, path).map(|_| None))
            }
            InputKind::CreateTag { target } => self.act("Create tag", move |client, path| {
                let message = (!second.trim().is_empty()).then_some(second.as_str());
                client.create_tag(&value, &target, message, path).map(|_| Some(format!("Created tag {}.", value.trim())))
            }),
            InputKind::SaveStash => self
                .act("Stash", move |client, path| client.save_stash(&value, path).map(|_| Some("Saved your changes in a stash.".into()))),
            InputKind::AddRemote => self.act("Add remote", move |client, path| client.add_remote(&value, &second, path).map(|_| None)),
            InputKind::Clone => {
                let destination = PathBuf::from(second.trim());
                let source = value.trim().to_string();
                if self.busy.is_some() {
                    return;
                }
                self.busy = Some("Clone".into());
                self.generation += 1;
                self.repository = Some(destination.clone());
                self.snapshot = None;
                self.worker.act(self.generation, destination.clone(), "Clone".into(), self.history_limit, move |client, _| {
                    client.clone_repository(&source, &destination).map(|_| Some(format!("Cloned into {}.", destination.display())))
                });
            }
        }
    }

    fn confirm(&mut self, title: impl Into<String>, message: impl Into<String>, button: impl Into<String>, pending: Pending) {
        self.dialog = Some(Dialog::Confirm { title: title.into(), message: message.into(), button: button.into(), pending });
    }

    fn input(&mut self, title: &str, label: &str, kind: InputKind) {
        self.dialog = Some(Dialog::Input {
            title: title.into(),
            label: label.into(),
            value: String::new(),
            second_label: None,
            second: String::new(),
            kind,
        });
    }

    fn pull(&mut self) {
        let Some(snapshot) = self.snapshot.clone() else { return };
        self.act("Pull", move |client, path| client.pull(&snapshot, path).map(|_| Some("Pulled the latest changes.".into())));
    }

    fn push(&mut self) {
        let Some(snapshot) = self.snapshot.clone() else { return };
        if snapshot.upstream.is_none() {
            let remote = snapshot.remotes.iter().find(|r| *r == "origin").or(snapshot.remotes.first()).cloned();
            match remote {
                Some(remote) => self.dialog = Some(Dialog::Publish { remote }),
                None => self.notice = Some(Notice { text: "Add a remote before pushing.".into(), is_error: true }),
            }
            return;
        }
        self.act("Push", move |client, path| client.push(&snapshot, path).map(|_| Some("Pushed.".into())));
    }

    // MARK: Interface

    fn shortcuts(&mut self, ctx: &egui::Context) {
        let command = |key| KeyboardShortcut::new(Modifiers::COMMAND, key);
        if ctx.input_mut(|i| i.consume_shortcut(&command(Key::R))) {
            self.load(true);
        }
        if ctx.input_mut(|i| i.consume_shortcut(&command(Key::O))) {
            self.choose_folder();
        }
        if ctx.input_mut(|i| i.consume_shortcut(&command(Key::Enter))) && self.can_commit() {
            self.commit();
        }
    }

    fn can_commit(&self) -> bool {
        let Some(snapshot) = &self.snapshot else { return false };
        self.idle()
            && !self.commit_message.trim().is_empty()
            && (snapshot.staged_count() > 0 || self.amend)
            && !snapshot.status.iter().any(|e| e.kind == nicegit_core::StatusKind::Conflicted)
    }

    fn toolbar(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.add_space(4.0);
            let idle_any = self.busy.is_none();
            if ui.add_enabled(idle_any, egui::Button::new("📂 Open…")).on_hover_text("Open a repository (Ctrl/Cmd-O)").clicked() {
                self.choose_folder();
            }
            ui.add_enabled_ui(idle_any, |ui| {
                ui.menu_button("Recent", |ui| {
                    if self.recent.is_empty() {
                        ui.label(RichText::new("No recent repositories").weak());
                    }
                    for path in self.recent.clone() {
                        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string());
                        if ui.button(name).on_hover_text(path.display().to_string()).clicked() {
                            ui.close();
                            self.open(path);
                        }
                    }
                    ui.separator();
                    if ui.button("Clone repository…").clicked() {
                        ui.close();
                        self.dialog = Some(Dialog::Input {
                            title: "Clone repository".into(),
                            label: "Repository URL".into(),
                            value: String::new(),
                            second_label: Some("Destination folder (full path)".into()),
                            second: String::new(),
                            kind: InputKind::Clone,
                        });
                    }
                });
            });
            ui.separator();
            let Some(snapshot) = self.snapshot.clone() else {
                if let Some(label) = &self.busy {
                    ui.spinner();
                    ui.label(label);
                }
                return;
            };
            ui.label(RichText::new(&snapshot.name).strong());
            ui.label(RichText::new(format!("on {}", snapshot.current_branch)).color(graph_view::color(0)));
            if let Some(upstream) = &snapshot.upstream {
                let counts = match (snapshot.ahead, snapshot.behind) {
                    (Some(a), Some(b)) if a + b > 0 => format!("  {a} ahead, {b} behind"),
                    _ => String::new(),
                };
                ui.label(RichText::new(format!("tracking {upstream}{counts}")).weak());
            }
            ui.separator();
            let idle = self.idle();
            let clean_operation = snapshot.operation.is_none();
            if ui.add_enabled(idle && !snapshot.remotes.is_empty(), egui::Button::new("⟳ Fetch")).clicked() {
                self.act("Fetch", |client, path| client.fetch(path).map(|_| Some("Fetched from all remotes.".into())));
            }
            if ui.add_enabled(idle && clean_operation && snapshot.upstream.is_some(), egui::Button::new("⬇ Pull")).clicked() {
                self.pull();
            }
            let push_label = if snapshot.upstream.is_some() { "⬆ Push" } else { "⬆ Publish" };
            if ui.add_enabled(idle && snapshot.is_on_branch() && !snapshot.remotes.is_empty(), egui::Button::new(push_label)).clicked() {
                self.push();
            }
            ui.separator();
            if ui.add_enabled(idle && snapshot.is_on_branch() && clean_operation, egui::Button::new("New branch")).clicked() {
                self.input(
                    "New branch",
                    "Create a branch at HEAD and switch to it",
                    InputKind::CreateBranch { branch: snapshot.current_branch.clone(), head: snapshot.head_hash.clone() },
                );
            }
            if ui.add_enabled(idle && clean_operation && !snapshot.status.is_empty(), egui::Button::new("Stash")).clicked() {
                self.input("Stash changes", "Message (optional)", InputKind::SaveStash);
            }
            let head_commit = snapshot.head_hash.as_ref().and_then(|h| snapshot.commits.iter().find(|c| &c.hash == h)).cloned();
            let can_undo =
                idle && clean_operation && snapshot.is_on_branch() && head_commit.as_ref().is_some_and(|c| !c.parents.is_empty());
            if ui
                .add_enabled(can_undo, egui::Button::new("Undo commit"))
                .on_hover_text("Undo the last commit, keeping its changes staged")
                .clicked()
            {
                if let (Some(commit), Some(head)) = (head_commit, snapshot.head_hash.clone()) {
                    self.confirm(
                        "Undo last commit?",
                        format!(
                            "“{}” will be removed from {}. Its changes stay staged so you can commit them again.",
                            commit.subject, snapshot.current_branch
                        ),
                        "Undo commit",
                        Pending::UndoCommit { branch: snapshot.current_branch.clone(), head, subject: commit.subject },
                    );
                }
            }
            if let Some(redo) = self.redo.clone() {
                let current = snapshot.head_hash.clone().unwrap_or_default();
                if ui.add_enabled(idle && redo.branch == snapshot.current_branch, egui::Button::new("Redo commit")).clicked() {
                    self.redo = None;
                    let Redo { commit, branch, .. } = redo;
                    self.act("Redo commit", move |client, path| {
                        client.redo_commit(&commit, &branch, &current, path).map(|_| Some("Restored the commit.".into()))
                    });
                }
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.add_enabled(self.busy.is_none(), egui::Button::new("↻")).on_hover_text("Refresh (Ctrl/Cmd-R)").clicked() {
                    self.load(true);
                }
                if let Some(label) = &self.busy {
                    if !label.is_empty() {
                        ui.label(RichText::new(label).weak());
                        ui.spinner();
                    }
                }
            });
        });
    }

    fn operation_banner(&mut self, ui: &mut Ui) {
        let Some(snapshot) = self.snapshot.clone() else { return };
        let Some(operation) = snapshot.operation else { return };
        let conflicts = snapshot.status.iter().filter(|e| e.kind == nicegit_core::StatusKind::Conflicted).count();
        egui::Frame::new().fill(Color32::from_rgb(0x6b, 0x4a, 0x10)).inner_margin(6.0).show(ui, |ui| {
            ui.horizontal(|ui| {
                let text = if conflicts > 0 {
                    format!(
                        "A {} is in progress with {conflicts} conflicted file(s). Resolve them in your editor, stage them, then continue.",
                        operation.name()
                    )
                } else {
                    format!("A {} is in progress. Continue when ready, or abort to return to where you started.", operation.name())
                };
                ui.label(RichText::new(text).color(Color32::WHITE));
                if ui.add_enabled(self.idle() && conflicts == 0, egui::Button::new("Continue")).clicked() {
                    self.act("Continue", move |client, path| client.continue_operation(operation, path).map(|_| None));
                }
                if ui.add_enabled(self.idle(), egui::Button::new("Abort")).clicked() {
                    self.confirm(
                        format!("Abort the {}?", operation.name()),
                        "Git returns the branch and files to the state before it started. Resolutions you made are lost.",
                        "Abort",
                        Pending::Abort(operation),
                    );
                }
            });
        });
    }

    fn sidebar(&mut self, ui: &mut Ui) {
        let Some(snapshot) = self.snapshot.clone() else {
            ui.add_space(8.0);
            ui.label(RichText::new("Recent").strong());
            for path in self.recent.clone() {
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                if ui.selectable_label(false, name).on_hover_text(path.display().to_string()).clicked() {
                    self.open(path);
                }
            }
            return;
        };
        let idle = self.idle();
        let can_switch = idle && snapshot.operation.is_none();
        let other_worktrees = GitClient::branches_in_other_worktrees(&snapshot);
        egui::ScrollArea::vertical().id_salt("sidebar").auto_shrink(false).show(ui, |ui| {
            egui::CollapsingHeader::new(RichText::new("Local branches").strong()).default_open(true).show(ui, |ui| {
                for branch in snapshot.branches.iter().filter(|b| !b.is_remote) {
                    let mut text = RichText::new(format!("{} {}", if branch.is_current { "✔" } else { "  " }, branch.display_name()));
                    if branch.is_current {
                        text = text.strong().color(graph_view::color(0));
                    }
                    let response = ui.selectable_label(false, text).on_hover_text(format!("{}\n{}", short(&branch.tip), branch.subject));
                    let in_other_worktree = other_worktrees.contains(&branch.name);
                    if response.double_clicked() && !branch.is_current && can_switch && !in_other_worktree {
                        self.checkout(branch.clone());
                    }
                    response.context_menu(|ui| self.branch_menu(ui, branch, &snapshot, can_switch && !in_other_worktree));
                }
            });
            for remote in &snapshot.remotes {
                egui::CollapsingHeader::new(RichText::new(format!("Remote: {remote}")).strong()).id_salt(("remote", remote)).show(
                    ui,
                    |ui| {
                        let remotes = snapshot.remotes.clone();
                        for branch in snapshot.branches.iter().filter(|b| b.remote_name(&remotes) == Some(remote.as_str())) {
                            let display = branch.display_name();
                            let name = display.strip_prefix(&format!("{remote}/")).unwrap_or(&display).to_string();
                            let response =
                                ui.selectable_label(false, name).on_hover_text(format!("{}\n{}", short(&branch.tip), branch.subject));
                            if response.double_clicked() && can_switch {
                                self.checkout(branch.clone());
                            }
                            response.context_menu(|ui| self.branch_menu(ui, branch, &snapshot, can_switch));
                        }
                    },
                );
            }
            if snapshot.remotes.is_empty() && ui.add_enabled(idle, egui::Button::new("Add remote…").small()).clicked() {
                self.dialog = Some(Dialog::Input {
                    title: "Add remote".into(),
                    label: "Name".into(),
                    value: "origin".into(),
                    second_label: Some("URL".into()),
                    second: String::new(),
                    kind: InputKind::AddRemote,
                });
            }
            egui::CollapsingHeader::new(RichText::new(format!("Tags ({})", snapshot.tags.len())).strong()).show(ui, |ui| {
                for tag in &snapshot.tags {
                    let response = ui.selectable_label(false, tag);
                    response.context_menu(|ui| {
                        if ui.add_enabled(idle, egui::Button::new("Delete tag…")).clicked() {
                            ui.close();
                            if let Some(tip) = snapshot.tag_tips.get(tag) {
                                self.confirm(
                                    format!("Delete tag {tag}?"),
                                    "The tag is removed from this repository only, not from remotes.",
                                    "Delete",
                                    Pending::DeleteTag { name: tag.clone(), tip: tip.clone() },
                                );
                            }
                        }
                    });
                }
            });
            egui::CollapsingHeader::new(RichText::new(format!("Stashes ({})", snapshot.stashes.len()))).default_open(true).show(ui, |ui| {
                for stash in &snapshot.stashes {
                    let selected = self.selection == Selection::Stash(stash.hash.clone());
                    let response = ui.selectable_label(selected, &stash.message).on_hover_text(&stash.reference);
                    if response.clicked() {
                        self.select_stash(stash.clone());
                    }
                    response.context_menu(|ui| {
                        let allowed = idle && snapshot.operation.is_none();
                        if ui.add_enabled(allowed, egui::Button::new("Apply")).clicked() {
                            ui.close();
                            let stash = stash.clone();
                            self.act("Apply stash", move |client, path| {
                                client.apply_stash(&stash, path).map(|_| Some("Applied the stash.".into()))
                            });
                        }
                        if ui.add_enabled(allowed, egui::Button::new("Pop (apply and delete)")).clicked() {
                            ui.close();
                            let stash = stash.clone();
                            self.act("Pop stash", move |client, path| {
                                client.pop_stash(&stash, path).map(|_| Some("Applied and deleted the stash.".into()))
                            });
                        }
                        if ui.add_enabled(idle, egui::Button::new("Delete…")).clicked() {
                            ui.close();
                            self.confirm(
                                format!("Delete {}?", stash.reference),
                                format!("“{}” will be deleted.", stash.message),
                                "Delete",
                                Pending::DropStash(stash.clone()),
                            );
                        }
                    });
                }
            });
            if snapshot.worktrees.len() > 1 {
                egui::CollapsingHeader::new(RichText::new("Worktrees")).show(ui, |ui| {
                    for worktree in &snapshot.worktrees {
                        let name = Path::new(&worktree.path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                        let label = format!("{name}  {}", worktree.branch.as_deref().unwrap_or("detached"));
                        if ui.selectable_label(worktree.path == snapshot.root_path, label).on_hover_text(&worktree.path).clicked()
                            && worktree.path != snapshot.root_path
                            && idle
                        {
                            self.open(PathBuf::from(&worktree.path));
                        }
                    }
                });
            }
        });
    }

    fn branch_menu(&mut self, ui: &mut Ui, branch: &Branch, snapshot: &Snapshot, can_switch: bool) {
        let idle = self.idle();
        if !branch.is_current && ui.add_enabled(can_switch, egui::Button::new("Check out")).clicked() {
            ui.close();
            self.checkout(branch.clone());
        }
        if !branch.is_current
            && snapshot.is_on_branch()
            && ui.add_enabled(can_switch, egui::Button::new(format!("Merge into {}…", snapshot.current_branch))).clicked()
        {
            ui.close();
            self.confirm(
                format!("Merge {} into {}?", branch.display_name(), snapshot.current_branch),
                "Git merges the branch's commits into your current branch. Conflicts stop the merge for you to resolve or abort.",
                "Merge",
                Pending::Merge { source: branch.clone(), branch: snapshot.current_branch.clone(), head: snapshot.head_hash.clone() },
            );
        }
        if ui.add_enabled(idle, egui::Button::new("Create branch from here…")).clicked() {
            ui.close();
            self.input("New branch", &format!("Create a branch at {}", branch.display_name()), InputKind::CreateBranchFrom(branch.clone()));
        }
        if ui.add_enabled(idle, egui::Button::new("Create tag here…")).clicked() {
            ui.close();
            self.dialog = Some(Dialog::Input {
                title: "New tag".into(),
                label: "Tag name".into(),
                value: String::new(),
                second_label: Some("Message (optional; makes an annotated tag)".into()),
                second: String::new(),
                kind: InputKind::CreateTag { target: branch.tip.clone() },
            });
        }
        if !branch.is_remote && !branch.is_detached() {
            if ui.add_enabled(idle, egui::Button::new("Rename…")).clicked() {
                ui.close();
                self.dialog = Some(Dialog::Input {
                    title: format!("Rename {}", branch.name),
                    label: "New name".into(),
                    value: branch.name.clone(),
                    second_label: None,
                    second: String::new(),
                    kind: InputKind::RenameBranch(branch.clone()),
                });
            }
            if !branch.is_current && ui.add_enabled(idle, egui::Button::new("Delete…")).clicked() {
                ui.close();
                let merged = snapshot.head_hash.as_deref().is_some_and(|_| self.branch_is_merged(branch, snapshot));
                let message = if merged {
                    "This branch is merged into the current branch, so no commits are lost.".to_string()
                } else {
                    format!("This branch may have commits that are not merged anywhere else. Its tip is {}; it can be recovered from Git's reflog for a while.", short(&branch.tip))
                };
                self.confirm(
                    format!("Delete branch {}?", branch.name),
                    message,
                    "Delete",
                    Pending::DeleteBranch { branch: branch.clone(), force: !merged },
                );
            }
        }
        ui.separator();
        if ui.button("Copy commit ID").clicked() {
            ui.close();
            ui.ctx().copy_text(branch.tip.clone());
        }
    }

    /// Whether the branch tip is in the visible history of the current checkout.
    fn branch_is_merged(&self, branch: &Branch, snapshot: &Snapshot) -> bool {
        let Some(head) = snapshot.head_hash.as_deref() else { return false };
        let mut stack = vec![head.to_string()];
        let mut seen = std::collections::HashSet::new();
        let by_hash: std::collections::HashMap<&str, &nicegit_core::Commit> =
            snapshot.commits.iter().map(|c| (c.hash.as_str(), c)).collect();
        while let Some(hash) = stack.pop() {
            if hash == branch.tip {
                return true;
            }
            if !seen.insert(hash.clone()) {
                continue;
            }
            if let Some(commit) = by_hash.get(hash.as_str()) {
                stack.extend(commit.parents.iter().cloned());
            }
        }
        false
    }

    fn changes_panel(&mut self, ui: &mut Ui) {
        let Some(snapshot) = self.snapshot.clone() else { return };
        let idle = self.idle();
        let unstaged: Vec<StatusEntry> = snapshot.status.iter().filter(|e| e.is_unstaged()).cloned().collect();
        let staged: Vec<StatusEntry> = snapshot.status.iter().filter(|e| e.is_staged()).cloned().collect();

        // The commit box sits at the bottom; the lists share the space above it.
        egui::Panel::bottom("commit_box").resizable(false).show(ui, |ui| {
            ui.add_space(6.0);
            ui.add(
                egui::TextEdit::multiline(&mut self.commit_message)
                    .hint_text("Commit message")
                    .desired_rows(4)
                    .desired_width(f32::INFINITY),
            );
            ui.horizontal(|ui| {
                let can_amend = snapshot.is_on_branch() && snapshot.head_hash.is_some() && snapshot.operation.is_none();
                if ui.add_enabled(can_amend, egui::Checkbox::new(&mut self.amend, "Amend last commit")).changed()
                    && self.amend
                    && self.commit_message.trim().is_empty()
                {
                    if let Some(head) = &snapshot.head_hash {
                        if let Ok(message) = GitClient::new().commit_message(head, Path::new(&snapshot.root_path)) {
                            self.commit_message = message.trim_end().to_string();
                        }
                    }
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let label = if self.amend { "Amend".to_string() } else { format!("Commit {} file(s)", staged.len()) };
                    if ui
                        .add_enabled(self.can_commit(), egui::Button::new(RichText::new(label).strong()))
                        .on_hover_text("Ctrl/Cmd-Enter")
                        .clicked()
                    {
                        self.commit();
                    }
                });
            });
            ui.add_space(6.0);
        });

        egui::ScrollArea::vertical().id_salt("changes").auto_shrink(false).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("Unstaged ({})", unstaged.len())).strong());
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.add_enabled(idle && !unstaged.is_empty(), egui::Button::new("Stage all").small()).clicked() {
                        self.act("Stage all", |client, path| client.stage_all(path).map(|_| None));
                    }
                    let discardable: Vec<StatusEntry> = unstaged.iter().filter(|e| e.kind != nicegit_core::StatusKind::Conflicted).cloned().collect();
                    if ui.add_enabled(idle && !discardable.is_empty(), egui::Button::new("Discard all…").small()).clicked() {
                        self.confirm(
                            "Discard all changes?",
                            format!("Staged and unstaged changes to {} file(s) are lost, and untracked files are deleted. This cannot be undone.", discardable.len()),
                            "Discard",
                            Pending::DiscardAll(discardable),
                        );
                    }
                });
            });
            for entry in &unstaged {
                self.change_row(ui, entry, false, idle);
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("Staged ({})", staged.len())).strong());
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.add_enabled(idle && !staged.is_empty(), egui::Button::new("Unstage all").small()).clicked() {
                        self.act("Unstage all", |client, path| client.unstage_all(path).map(|_| None));
                    }
                });
            });
            for entry in &staged {
                self.change_row(ui, entry, true, idle);
            }
        });
    }

    fn change_row(&mut self, ui: &mut Ui, entry: &StatusEntry, staged: bool, idle: bool) {
        let selected = matches!(&self.selection, Selection::Change { entry: e, staged: s } if e == entry && *s == staged);
        ui.horizontal(|ui| {
            let badge_color = match entry.kind {
                nicegit_core::StatusKind::Added | nicegit_core::StatusKind::Untracked => graph_view::color(0),
                nicegit_core::StatusKind::Deleted => graph_view::color(4),
                nicegit_core::StatusKind::Conflicted => graph_view::color(2),
                _ => graph_view::color(1),
            };
            ui.label(RichText::new(entry.kind.letter()).monospace().color(badge_color)).on_hover_text(entry.kind.title());
            let label = match &entry.original_path {
                Some(original) if entry.kind == nicegit_core::StatusKind::Renamed => format!("{original} → {}", entry.path),
                _ => entry.path.clone(),
            };
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if staged {
                    if ui.add_enabled(idle, egui::Button::new("−").small()).on_hover_text("Unstage").clicked() {
                        let entry = entry.clone();
                        self.act("Unstage", move |client, path| client.unstage(&entry, path).map(|_| None));
                    }
                } else {
                    let can_discard = entry.kind != nicegit_core::StatusKind::Conflicted;
                    if ui.add_enabled(idle && can_discard, egui::Button::new("🗑").small()).on_hover_text("Discard changes").clicked() {
                        let what = if entry.kind == nicegit_core::StatusKind::Untracked {
                            format!("{} is untracked and will be deleted.", entry.path)
                        } else {
                            format!("Staged and unstaged changes to {} are lost. This cannot be undone.", entry.path)
                        };
                        self.confirm("Discard changes?", what, "Discard", Pending::Discard(entry.clone()));
                    }
                    if ui.add_enabled(idle, egui::Button::new("+").small()).on_hover_text("Stage").clicked() {
                        let entry = entry.clone();
                        self.act("Stage", move |client, path| client.stage(&entry.path, path).map(|_| None));
                    }
                }
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    let response = ui.add(egui::Button::selectable(selected, label.as_str()).truncate()).on_hover_text(&label);
                    if response.clicked() {
                        self.select_change(entry.clone(), staged);
                    }
                });
            });
        });
    }

    fn history(&mut self, ui: &mut Ui) {
        let Some(snapshot) = self.snapshot.clone() else {
            ui.centered_and_justified(|ui| {
                if self.git_missing {
                    ui.label(RichText::new("Git was not found. Install Git, then restart NiceGit.").color(graph_view::color(4)));
                } else if self.busy.is_some() {
                    ui.spinner();
                } else {
                    ui.vertical_centered(|ui| {
                        ui.add_space(ui.available_height() / 3.0);
                        ui.heading("NiceGit");
                        ui.label(RichText::new("Open a Git repository to get started.").weak());
                        ui.add_space(8.0);
                        if ui.button("📂 Open repository…").clicked() {
                            self.choose_folder();
                        }
                    });
                }
            });
            return;
        };
        let dirty = !snapshot.status.is_empty();
        let total = self.rows.len() + usize::from(snapshot.has_more_commits);
        let background = ui.visuals().panel_fill;
        let graph_width = self.graph_width;
        egui::ScrollArea::vertical().id_salt("history").auto_shrink(false).show_rows(ui, ROW_HEIGHT, total, |ui, range| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for index in range {
                if index >= self.rows.len() {
                    ui.horizontal(|ui| {
                        if ui.add_enabled(self.idle(), egui::Button::new("Load more history")).clicked() {
                            self.history_limit += PAGE;
                            self.load(false);
                        }
                    });
                    continue;
                }
                let is_working_tree = dirty && index == 0;
                let commit = if is_working_tree { None } else { snapshot.commits.get(index - usize::from(dirty)) };
                let width = ui.available_width();
                let (rect, response) = ui.allocate_exact_size(egui::vec2(width, ROW_HEIGHT), Sense::click());
                let selected = match (&self.selection, commit) {
                    (Selection::Commit { hash, .. }, Some(commit)) => *hash == commit.hash,
                    _ => false,
                };
                if selected {
                    ui.painter().rect_filled(rect, 2.0, ui.visuals().selection.bg_fill);
                } else if response.hovered() {
                    ui.painter().rect_filled(rect, 2.0, ui.visuals().widgets.hovered.weak_bg_fill);
                }
                let graph_rect = egui::Rect::from_min_size(rect.min, egui::vec2(graph_width, ROW_HEIGHT));
                let row = &self.rows[index];
                graph_view::paint_row(&ui.painter().with_clip_rect(graph_rect), graph_rect, row, is_working_tree, background);
                let text_color = if selected { ui.visuals().selection.stroke.color } else { ui.visuals().text_color() };
                let weak = ui.visuals().weak_text_color();
                let mut x = rect.left() + graph_width + 4.0;
                let y = rect.center().y;
                let painter = ui.painter().with_clip_rect(rect);
                let font = egui::TextStyle::Body.resolve(ui.style());
                let small = egui::TextStyle::Small.resolve(ui.style());
                let right_columns = 300.0_f32.min(rect.width() * 0.4);
                let subject_limit = rect.right() - right_columns;
                if let Some(commit) = commit {
                    for reference in &commit.refs {
                        let (label, fill) = ref_style(reference);
                        let galley = painter.layout_no_wrap(label, small.clone(), Color32::WHITE);
                        let badge = egui::Rect::from_min_size(egui::pos2(x, y - 8.0), egui::vec2(galley.size().x + 10.0, 16.0));
                        if badge.right() > subject_limit {
                            break;
                        }
                        painter.rect_filled(badge, 4.0, fill);
                        painter.galley(egui::pos2(x + 5.0, y - galley.size().y / 2.0), galley, Color32::WHITE);
                        x = badge.right() + 4.0;
                    }
                    let subject = painter.layout_no_wrap(commit.subject.clone(), font.clone(), text_color);
                    painter.with_clip_rect(egui::Rect::from_min_max(rect.min, egui::pos2(subject_limit - 6.0, rect.bottom()))).galley(
                        egui::pos2(x, y - subject.size().y / 2.0),
                        subject,
                        text_color,
                    );
                    let author = painter.layout_no_wrap(commit.author_name.clone(), small.clone(), weak);
                    painter
                        .with_clip_rect(egui::Rect::from_min_max(
                            egui::pos2(subject_limit, rect.top()),
                            egui::pos2(rect.right() - 170.0, rect.bottom()),
                        ))
                        .galley(egui::pos2(subject_limit, y - author.size().y / 2.0), author, weak);
                    let date = painter.layout_no_wrap(commit.relative_date.clone(), small.clone(), weak);
                    painter.galley(egui::pos2(rect.right() - 165.0, y - date.size().y / 2.0), date, weak);
                    let hash = painter.layout_no_wrap(commit.short_hash.clone(), egui::TextStyle::Monospace.resolve(ui.style()), weak);
                    painter.galley(egui::pos2(rect.right() - 60.0, y - hash.size().y / 2.0), hash, weak);
                    if response.clicked() {
                        self.select_commit(commit.hash.clone());
                    }
                    let commit = commit.clone();
                    let can_switch = self.idle() && snapshot.operation.is_none();
                    response.context_menu(|ui| self.commit_menu(ui, &commit, &snapshot, can_switch));
                } else {
                    let text = format!(
                        "Uncommitted changes ({} file{})",
                        snapshot.status.len(),
                        if snapshot.status.len() == 1 { "" } else { "s" }
                    );
                    let galley = painter.layout_no_wrap(text, font.clone(), weak);
                    painter.galley(egui::pos2(x, y - galley.size().y / 2.0), galley, weak);
                    if response.clicked() {
                        if let Some(first) = snapshot.status.first().cloned() {
                            let staged = !first.is_unstaged();
                            self.select_change(first, staged);
                        }
                    }
                }
            }
        });
        let _ = WORKING_TREE_HASH;
    }

    fn commit_menu(&mut self, ui: &mut Ui, commit: &nicegit_core::Commit, snapshot: &Snapshot, can_switch: bool) {
        let idle = self.idle();
        if ui.add_enabled(idle, egui::Button::new("Create branch here…")).clicked() {
            ui.close();
            let source = Branch {
                name: commit.hash.clone(),
                is_current: false,
                is_remote: false,
                tip: commit.hash.clone(),
                subject: commit.subject.clone(),
                upstream: None,
            };
            self.input("New branch", &format!("Create a branch at {}", commit.short_hash), InputKind::CreateBranchFrom(source));
        }
        if ui.add_enabled(idle, egui::Button::new("Create tag here…")).clicked() {
            ui.close();
            self.dialog = Some(Dialog::Input {
                title: "New tag".into(),
                label: "Tag name".into(),
                value: String::new(),
                second_label: Some("Message (optional; makes an annotated tag)".into()),
                second: String::new(),
                kind: InputKind::CreateTag { target: commit.hash.clone() },
            });
        }
        // Branches pointing at this commit can be checked out from here.
        for branch in snapshot.branches.iter().filter(|b| b.tip == commit.hash && !b.is_current && !b.is_detached()) {
            if ui.add_enabled(can_switch, egui::Button::new(format!("Check out {}", branch.display_name()))).clicked() {
                ui.close();
                self.checkout(branch.clone());
            }
        }
        ui.separator();
        if ui.button("Copy commit ID").clicked() {
            ui.close();
            ui.ctx().copy_text(commit.hash.clone());
        }
        if ui.button("Copy subject").clicked() {
            ui.close();
            ui.ctx().copy_text(commit.subject.clone());
        }
    }

    fn diff_panel(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            let title = self.diff.as_ref().map(|d| d.title.clone()).unwrap_or_else(|| "Select a change or commit to see its diff".into());
            ui.label(RichText::new(title).strong());
            if self.diff_loading {
                ui.spinner();
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.selectable_value(&mut self.split_diff, true, "Split");
                ui.selectable_value(&mut self.split_diff, false, "Unified");
            });
        });
        ui.separator();
        if let Selection::Commit { hash, file } = self.selection.clone() {
            if !self.commit_files.is_empty() {
                egui::Panel::left("commit_files").resizable(true).default_size(220.0).show(ui, |ui| {
                    egui::ScrollArea::vertical().id_salt("commit_files_scroll").auto_shrink(false).show(ui, |ui| {
                        if ui
                            .selectable_label(file.is_none(), RichText::new(format!("All files ({})", self.commit_files.len())).strong())
                            .clicked()
                        {
                            self.select_commit_file(hash.clone(), None);
                        }
                        for (status, path) in self.commit_files.clone() {
                            let selected = file.as_deref() == Some(path.as_str());
                            if ui
                                .add(egui::Button::selectable(selected, format!("{status}  {path}")).truncate())
                                .on_hover_text(&path)
                                .clicked()
                            {
                                self.select_commit_file(hash.clone(), Some(path));
                            }
                        }
                    });
                });
            }
        }
        if let Some(diff) = &self.diff {
            diff_view::show(ui, diff, self.split_diff);
        } else {
            // Keep the panel's height while nothing is selected, so it does not collapse.
            ui.take_available_space();
        }
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.dialog.take() else { return };
        let mut close = false;
        let mut run = false;
        let modal = egui::Modal::new(egui::Id::new("dialog")).show(ctx, |ui| {
            ui.set_width(420.0);
            match &mut dialog {
                Dialog::Confirm { title, message, button, .. } => {
                    ui.heading(title.as_str());
                    ui.add_space(6.0);
                    ui.label(message.as_str());
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        if ui.button(RichText::new(button.as_str()).strong()).clicked() {
                            run = true;
                        }
                        if ui.button("Cancel").clicked() {
                            close = true;
                        }
                    });
                }
                Dialog::Input { title, label, value, second_label, second, .. } => {
                    ui.heading(title.as_str());
                    ui.add_space(6.0);
                    ui.label(label.as_str());
                    let first = ui.add(egui::TextEdit::singleline(value).desired_width(f32::INFINITY));
                    if ui.memory(|m| m.focused().is_none()) {
                        first.request_focus();
                    }
                    if let Some(second_label) = second_label {
                        ui.label(second_label.as_str());
                        ui.add(egui::TextEdit::singleline(second).desired_width(f32::INFINITY));
                    }
                    ui.add_space(12.0);
                    let enter = ui.input(|i| i.key_pressed(Key::Enter));
                    ui.horizontal(|ui| {
                        if ui.button(RichText::new("OK").strong()).clicked() || enter {
                            run = true;
                        }
                        if ui.button("Cancel").clicked() {
                            close = true;
                        }
                    });
                }
                Dialog::Publish { remote } => {
                    ui.heading("Publish branch");
                    ui.add_space(6.0);
                    let branch = self.snapshot.as_ref().map(|s| s.current_branch.clone()).unwrap_or_default();
                    ui.label(format!("Push {branch} to a remote and track it there."));
                    let remotes = self.snapshot.as_ref().map(|s| s.remotes.clone()).unwrap_or_default();
                    egui::ComboBox::from_label("Remote").selected_text(remote.as_str()).show_ui(ui, |ui| {
                        for name in remotes {
                            ui.selectable_value(remote, name.clone(), name);
                        }
                    });
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        if ui.button(RichText::new("Publish").strong()).clicked() {
                            run = true;
                        }
                        if ui.button("Cancel").clicked() {
                            close = true;
                        }
                    });
                }
            }
        });
        if modal.should_close() {
            close = true;
        }
        if run {
            match dialog {
                Dialog::Confirm { pending, .. } => self.run_pending(pending),
                Dialog::Input { kind, value, second, .. } => self.run_input(kind, value, second),
                Dialog::Publish { remote } => {
                    if let Some(snapshot) = self.snapshot.clone() {
                        self.act("Publish", move |client, path| {
                            client
                                .publish(&remote, &snapshot, path)
                                .map(|_| Some(format!("Published {} to {remote}.", snapshot.current_branch)))
                        });
                    }
                }
            }
        } else if !close {
            self.dialog = Some(dialog);
        }
    }

    fn status_bar(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            if let Some(notice) = &self.notice {
                let color = if notice.is_error { graph_view::color(4) } else { ui.visuals().text_color() };
                ui.label(RichText::new(&notice.text).color(color));
                if ui.small_button("✕").clicked() {
                    self.notice = None;
                }
            } else if let Some(snapshot) = &self.snapshot {
                ui.label(RichText::new(snapshot.root_path.as_str()).weak());
            }
        });
    }

    /// Opens a tool window, or brings forward the one already open with the same id.
    pub fn open_tool(&mut self, tool: Box<dyn ToolWindow>) {
        let id = tool.id();
        self.tools.retain(|open| open.id() != id);
        self.tools.push(tool);
    }

    fn tool_windows(&mut self, ctx: &egui::Context) {
        let (Some(snapshot), Some(repo)) = (self.snapshot.clone(), self.repository.clone()) else {
            return;
        };
        let idle = self.busy.is_none();
        let mut requests = Vec::new();
        let mut closed = Vec::new();
        for (index, tool) in self.tools.iter_mut().enumerate() {
            let mut open = true;
            egui::Window::new(tool.title())
                .id(egui::Id::new(("tool", tool.id())))
                .default_size(tool.default_size())
                .collapsible(false)
                .resizable(true)
                .open(&mut open)
                .show(ctx, |ui| {
                    let mut cx = Ctx::new(&repo, &snapshot, idle, &mut requests);
                    tool.ui(ui, &mut cx);
                });
            if !open || tool.wants_close() {
                closed.push(index);
            }
        }
        for index in closed.into_iter().rev() {
            self.tools.remove(index);
        }
        for request in requests {
            self.handle(request);
        }
    }

    fn handle(&mut self, request: Request) {
        match request {
            Request::Act { label, action } => self.act(&label, action),
            Request::Notice { text, is_error } => self.notice = Some(Notice { text, is_error }),
            Request::SelectCommit(hash) => self.select_commit(hash),
            Request::OpenRepository(path) => self.open(path),
            Request::Open(tool) => self.open_tool(tool),
            Request::Refresh => self.load(true),
        }
    }

    /// Refreshes when the window regains focus, and now and then while it has it, so changes
    /// made in other apps appear without a manual refresh.
    fn auto_refresh(&mut self, ctx: &egui::Context) {
        let focused = ctx.input(|i| i.viewport().focused.unwrap_or(true));
        let regained = focused && !self.was_focused;
        self.was_focused = focused;
        if self.snapshot.is_some()
            && self.busy.is_none()
            && self.dialog.is_none()
            && (regained || (focused && self.last_load.elapsed() >= AUTO_REFRESH))
        {
            self.load(false);
        }
        if focused {
            ctx.request_repaint_after(AUTO_REFRESH);
        }
    }
}

/// The label and colour of a ref decoration such as "HEAD -> main", "origin/main", or "tag: v1".
fn ref_style(reference: &str) -> (String, Color32) {
    if let Some(tag) = reference.strip_prefix("tag: ") {
        (tag.to_string(), Color32::from_rgb(0x8a, 0x6d, 0x1e))
    } else if let Some(branch) = reference.strip_prefix("HEAD -> ") {
        (branch.to_string(), Color32::from_rgb(0x2f, 0x8a, 0x58))
    } else if reference == "HEAD" {
        ("HEAD".to_string(), Color32::from_rgb(0x2f, 0x8a, 0x58))
    } else if reference.contains('/') {
        (reference.to_string(), Color32::from_rgb(0x3a, 0x6e, 0xb0))
    } else {
        (reference.to_string(), Color32::from_rgb(0x55, 0x60, 0x70))
    }
}

impl eframe::App for NiceGitApp {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.receive();
        self.shortcuts(&ctx);
        self.auto_refresh(&ctx);
        egui::Panel::top("toolbar").show(ui, |ui| {
            ui.add_space(4.0);
            self.toolbar(ui);
            ui.add_space(4.0);
        });
        egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui));
        if self.snapshot.is_some() {
            egui::Panel::left("sidebar").resizable(true).default_size(240.0).size_range(160.0..=420.0).show(ui, |ui| self.sidebar(ui));
            egui::Panel::right("changes")
                .resizable(true)
                .default_size(340.0)
                .size_range(240.0..=560.0)
                .show(ui, |ui| self.changes_panel(ui));
        }
        egui::CentralPanel::default().show(ui, |ui| {
            self.operation_banner(ui);
            if self.snapshot.is_some() {
                egui::Panel::bottom("diff")
                    .resizable(true)
                    .default_size(320.0)
                    .size_range(120.0..=900.0)
                    .show(ui, |ui| self.diff_panel(ui));
            }
            self.history(ui);
        });
        self.tool_windows(&ctx);
        self.dialogs(&ctx);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        let joined = self.recent.iter().map(|p| p.to_string_lossy().into_owned()).collect::<Vec<_>>().join(&RECENT_SEPARATOR.to_string());
        storage.set_string(RECENT_KEY, joined);
    }
}
