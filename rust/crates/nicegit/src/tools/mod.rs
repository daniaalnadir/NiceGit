//! Feature windows. Each tool is a self-contained window that reads the repository through
//! its own background tasks and changes it only through [`Ctx::act`], so every change runs
//! through the app's busy state and refreshes the repository afterwards.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, TryRecvError};

use nicegit_core::{GitClient, Snapshot};

use crate::worker::ActionResult;

pub mod bisect;
pub mod blame;
pub mod branch_cleanup;
pub mod commit_search;
pub mod compare;
pub mod conflict;
pub mod content_search;
pub mod editor;
pub mod file_history;
pub mod gitflow;
pub mod github;
pub mod interactive_rebase;
pub mod lfs;
pub mod reflog;
pub mod repository_settings;
pub mod reset;
pub mod stash;
pub mod submodules;
pub mod terminal;
pub mod widgets;
pub mod worktrees;

/// A repository-changing action, run in the background by the app.
pub type Action = Box<dyn FnOnce(&GitClient, &Path) -> ActionResult + Send>;

/// What a tool asks the app to do once the current frame is drawn.
pub enum Request {
    /// Run an action that changes the repository, then refresh. Ignored while another runs.
    Act { label: String, action: Action },
    /// Like `Act`, but the branch move it makes is recorded so Undo can reverse it.
    ActRecording { label: String, title: String, mode: nicegit_core::undo::UndoMode, action: Action },
    /// Offer `step` through the toolbar's Undo, described by `title`.
    RecordUndo { title: String, step: nicegit_core::undo::UndoStep },
    /// Show a message in the status bar.
    Notice { text: String, is_error: bool },
    /// Select a commit in the graph and show it in the inspector.
    SelectCommit(String),
    /// Open (or switch to) another repository or worktree.
    OpenRepository(PathBuf),
    /// Open another tool window.
    Open(Box<dyn ToolWindow>),
    /// Reload the repository.
    Refresh,
}

/// What a tool sees of the app each frame.
pub struct Ctx<'a> {
    /// The repository root.
    pub repo: &'a Path,
    /// The latest repository state. Tools capture what they need from it when an action is
    /// chosen, so the action can refuse if the repository changed in the meantime.
    pub snapshot: &'a Snapshot,
    /// No action is running; tools disable controls that change the repository otherwise.
    pub idle: bool,
    /// Diffs leave out whitespace-only changes, following the app setting.
    pub ignore_whitespace: bool,
    requests: &'a mut Vec<Request>,
}

impl<'a> Ctx<'a> {
    pub fn new(repo: &'a Path, snapshot: &'a Snapshot, idle: bool, requests: &'a mut Vec<Request>) -> Self {
        Self { repo, snapshot, idle, ignore_whitespace: false, requests }
    }

    pub fn act(&mut self, label: impl Into<String>, action: impl FnOnce(&GitClient, &Path) -> ActionResult + Send + 'static) {
        self.requests.push(Request::Act { label: label.into(), action: Box::new(action) });
    }

    /// Runs an action that moves the current branch and records it for Undo, reversed with `mode`.
    pub fn act_recording(
        &mut self,
        label: impl Into<String>,
        title: impl Into<String>,
        mode: nicegit_core::undo::UndoMode,
        action: impl FnOnce(&GitClient, &Path) -> ActionResult + Send + 'static,
    ) {
        self.requests.push(Request::ActRecording { label: label.into(), title: title.into(), mode, action: Box::new(action) });
    }

    pub fn record_undo(&mut self, title: impl Into<String>, step: nicegit_core::undo::UndoStep) {
        self.requests.push(Request::RecordUndo { title: title.into(), step });
    }

    pub fn notice(&mut self, text: impl Into<String>, is_error: bool) {
        self.requests.push(Request::Notice { text: text.into(), is_error });
    }

    pub fn select_commit(&mut self, hash: impl Into<String>) {
        self.requests.push(Request::SelectCommit(hash.into()));
    }

    pub fn open_repository(&mut self, path: PathBuf) {
        self.requests.push(Request::OpenRepository(path));
    }

    pub fn open(&mut self, tool: Box<dyn ToolWindow>) {
        self.requests.push(Request::Open(tool));
    }

    pub fn refresh(&mut self) {
        self.requests.push(Request::Refresh);
    }
}

/// A feature window. The app draws it inside an `egui::Window` titled [`ToolWindow::title`].
pub trait ToolWindow {
    /// A stable identity; opening a tool whose id is already open brings that one forward.
    fn id(&self) -> String;
    fn title(&self) -> String;
    fn default_size(&self) -> egui::Vec2 {
        egui::vec2(760.0, 520.0)
    }
    fn ui(&mut self, ui: &mut egui::Ui, cx: &mut Ctx);
    /// Return true to close the window, for example after a confirmed action.
    fn wants_close(&self) -> bool {
        false
    }
    /// True while the tool holds edits that would be lost, such as unsaved text in an editor.
    /// The app then refuses actions that could rewrite working files.
    fn has_unsaved_changes(&self) -> bool {
        false
    }
    /// True while the tool shows something the user is reviewing, as the Mac app's sheets for
    /// stashes, repository settings, and conflicts did; automatic refreshes and the refresh for
    /// returning to the app wait until it closes.
    fn holds_refresh(&self) -> bool {
        self.has_unsaved_changes()
    }
    /// Called after the repository is reloaded, so the tool can reread its data.
    fn repository_changed(&mut self, _snapshot: &Snapshot) {}
}

/// A value computed on a background thread. Poll it each frame; it asks egui to repaint
/// when the value arrives.
pub struct Task<T> {
    receiver: Option<Receiver<T>>,
    value: Option<T>,
}

impl<T: Send + 'static> Task<T> {
    pub fn spawn(ctx: &egui::Context, work: impl FnOnce() -> T + Send + 'static) -> Self {
        let (sender, receiver) = channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = sender.send(work());
            ctx.request_repaint();
        });
        Self { receiver: Some(receiver), value: None }
    }

    /// A task that already has its value.
    #[allow(dead_code)]
    pub fn ready(value: T) -> Self {
        Self { receiver: None, value: Some(value) }
    }

    /// The value, once it has arrived.
    pub fn get(&mut self) -> Option<&T> {
        self.poll();
        self.value.as_ref()
    }

    #[allow(dead_code)]
    pub fn get_mut(&mut self) -> Option<&mut T> {
        self.poll();
        self.value.as_mut()
    }

    /// The value if it has already arrived, without polling for it.
    pub fn peek(&self) -> Option<&T> {
        self.value.as_ref()
    }

    pub fn is_pending(&mut self) -> bool {
        self.poll();
        self.value.is_none()
    }

    fn poll(&mut self) {
        if let Some(receiver) = &self.receiver {
            match receiver.try_recv() {
                Ok(value) => {
                    self.value = Some(value);
                    self.receiver = None;
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => self.receiver = None,
            }
        }
    }
}

/// Runs a read-only Git query in the background for a tool.
pub fn query<T: Send + 'static>(
    ctx: &egui::Context,
    repo: &Path,
    work: impl FnOnce(&GitClient, &Path) -> nicegit_core::Result<T> + Send + 'static,
) -> Task<nicegit_core::Result<T>> {
    let repo = repo.to_path_buf();
    Task::spawn(ctx, move || work(&GitClient::new(), &repo))
}
