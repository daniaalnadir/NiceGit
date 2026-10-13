use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use nicegit_core::diff::DiffRequest;
use nicegit_core::graph::{layout, layout_with_working_tree, GraphRow};
use nicegit_core::models::short;
use nicegit_core::undo::{DiscardUndo, UndoMode, UndoStep};
use nicegit_core::{Branch, GitClient, GitError, Snapshot, Stash, StatusEntry};

use crate::diff_view::DiffContent;
use crate::settings::{Draft, Settings, STORAGE_KEY};
use crate::theme;
use crate::tools::{Ctx, Request, Task, ToolWindow};
use crate::ui::dialogs::{Dialog, Pending};
use crate::worker::{ActionResult, Message, Worker};

pub const PAGE: usize = 500;
const AUTO_REFRESH: Duration = Duration::from_secs(30);
/// How often the current branch's pull request status is read again while the window is focused.
const PR_STATUS_INTERVAL: Duration = Duration::from_secs(5 * 60);
const WATCH_DEBOUNCE: Duration = Duration::from_millis(400);

pub struct Notice {
    pub text: String,
    pub is_error: bool,
    /// An action the message offers as a button, such as pulling after a rejected push.
    pub action: Option<NoticeAction>,
    /// Git's own words, when the message explains them more plainly; shown in its tooltip.
    pub detail: Option<String>,
}

/// A plainer explanation of an everyday mistake Git reports in its own terms: a name it will not
/// accept or that is taken, or a folder that is not empty. `None` for any other error.
pub fn naming_problem(error: &str) -> Option<&'static str> {
    let lower = error.to_lowercase();
    if lower.contains("is not a valid branch name") || lower.contains("is not a valid tag name") || lower.contains("not a valid ref name") {
        Some("That name cannot be used. Names cannot contain spaces, \"..\", \"~\", \"^\", \":\", \"?\", \"*\", or \"[\", or end with \".lock\" or \"/\".")
    } else if lower.contains("a branch named") && lower.contains("already exists") {
        Some("A branch with that name already exists. Choose another name, or check out the existing branch.")
    } else if lower.contains("tag") && lower.contains("already exists") {
        Some("A tag with that name already exists. Choose another name, or delete the old tag first.")
    } else if lower.contains("already exists and is not an empty directory") {
        Some("That folder already has files in it. Choose a new or empty folder.")
    } else if lower.contains("would clobber existing tag") {
        Some("A tag on the remote points somewhere else than your tag of the same name, so Git would not replace yours.")
    } else {
        None
    }
}

/// A plainer explanation of a remote failing to answer or refusing a sign-in, from Git's own
/// words; `None` for any other error.
pub fn remote_problem(error: &str) -> Option<&'static str> {
    let lower = error.to_lowercase();
    let any = |signs: &[&str]| signs.iter().any(|sign| lower.contains(sign));
    if any(&["permission denied (publickey", "host key verification failed"]) {
        Some("The remote did not accept your SSH key. Check that your key is added to your account and loaded in ssh-agent.")
    } else if any(&[
        "authentication failed",
        "could not read username",
        "could not read password",
        "invalid username or password",
        "returned error: 403",
    ]) {
        Some("The remote did not accept your sign-in. For HTTPS, sign in through a credential helper (gh auth login sets one up for GitHub); for SSH, check your key.")
    } else if any(&[
        "could not resolve host",
        "connection timed out",
        "failed to connect",
        "network is unreachable",
        "connection refused",
        "operation timed out",
    ]) {
        Some("The remote could not be reached. Check your internet connection, and that the remote's address is right.")
    } else if any(&["repository not found", "does not appear to be a git repository"]) {
        Some("The remote repository was not found, or your account cannot see it. Check its address and your access.")
    } else {
        None
    }
}

/// What a message's button does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NoticeAction {
    /// Fetch, then offer to merge the upstream's commits or rebase onto them.
    Integrate,
    /// Open Repository Settings, where the commit identity is set.
    SetIdentity,
    /// Open the stash dialog, to set uncommitted changes aside.
    Stash,
    /// Open this web address in the browser. Only `https://` addresses open.
    OpenUrl(String),
}

/// Whether Git stopped because uncommitted changes would be overwritten.
pub fn blocked_by_local_changes(error: &str) -> bool {
    error.contains("would be overwritten by") || error.contains("Please commit your changes or stash them")
}

/// Whether Git refused to continue an operation because unstaged changes remain. Continuing a
/// rebase stopped at an edit needs its changes staged, so unstaged ones block it.
pub fn blocked_by_unstaged_changes(error: &str) -> bool {
    error.contains("You have unstaged changes")
}

/// Whether Git refused to commit because it does not know who is committing.
pub fn needs_identity(error: &str) -> bool {
    ["Please tell me who you are", "empty ident name", "unable to auto-detect email address"].iter().any(|sign| error.contains(sign))
}

/// Whether a push failed because the remote has commits the branch does not, so pulling them
/// first would let it through.
pub fn push_needs_pull(error: &str) -> bool {
    ["non-fast-forward", "fetch first", "tip of your current branch is behind"].iter().any(|sign| error.contains(sign))
}

/// Whether a force push was refused by its lease: the remote branch no longer has the tip the
/// user last saw, so Git would have replaced commits nobody has reviewed. Git marks exactly this
/// refusal "stale info"; a server hook's "[remote rejected]" is a different problem.
pub fn lease_was_refused(error: &str) -> bool {
    error.contains("(stale info)")
}

/// What is selected. A commit or stash's `file` is the one file shown from it, or `None` for all.
#[derive(Clone, Debug, PartialEq)]
pub enum Selection {
    None,
    WorkingTree,
    Change { entry: StatusEntry, staged: bool },
    Commit { hash: String, file: Option<String> },
    Stash { hash: String, file: Option<String> },
}

/// An action that can be undone, with the words to describe it.
#[derive(Clone)]
pub struct HistoryEntry {
    pub title: String,
    /// What the action was done to, such as the commit's summary or the branch's name, when known.
    pub detail: Option<String>,
    pub step: UndoStep,
}

/// The summary line of a commit message, or `None` when the message starts blank.
pub fn summary_of(message: &str) -> Option<String> {
    let line = message.lines().next()?.trim();
    (!line.is_empty()).then(|| line.to_string())
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

/// Lines added and removed in each file, by path.
pub type FileLineCounts = std::collections::HashMap<String, nicegit_core::line_counts::LineCount>;

/// A tag's details as read for its sidebar hover text, with the tip they were read for.
pub type TagDetailsRead = (String, Task<nicegit_core::Result<Option<nicegit_core::TagDetails>>>);

/// Returns every side and bottom panel to its default width, a share of the window.
pub fn restore_panel_widths(ctx: &egui::Context) {
    ctx.data_mut(|data| {
        for id in ["repositories", "sidebar", "changes", "terminal"] {
            data.remove::<egui::containers::panel::PanelState>(egui::Id::new(id));
        }
    });
}

/// The smallest and largest interface sizes Settings offers.
pub const MIN_ZOOM: f32 = 0.6;
pub const MAX_ZOOM: f32 = 2.0;

/// The busy label of a fetch NiceGit starts on its own; its result is reported quietly.
pub const BACKGROUND_FETCH: &str = "Fetching in the background";
/// How long after a repository opens its first background fetch waits.
const FIRST_FETCH_DELAY: Duration = Duration::from_secs(30);

/// The most recorded actions Undo steps back through; older ones are dropped.
pub const MAX_UNDO_STEPS: usize = 20;

/// Added to a failed undo's notice when older undo steps were dropped with it.
const CLEARED_UNDO_NOTE: &str = "Older undo steps were cleared because the repository changed.";
/// Added to a failed redo's notice when older redo steps were dropped with it.
const CLEARED_REDO_NOTE: &str = "Older redo steps were cleared because the repository changed.";

/// What a finished action recorded for undo, handed from the background thread.
pub enum Recorded {
    /// An ordinary action: it can be undone, and ends any chain of redo steps.
    Step(HistoryEntry),
    /// The inverse of an undo, which Redo can apply.
    Redo(HistoryEntry),
    /// The inverse of a redo, which Undo can apply. The rest of the redo steps stay.
    Undo(HistoryEntry),
    /// A step that could not be applied, from Undo (`redo` false) or Redo (`redo` true).
    HistoryFailed {
        redo: bool,
    },
    Discard(DiscardUndo),
}

/// Pushes onto a history stack, dropping the oldest entries past the cap.
fn push_capped(stack: &mut Vec<HistoryEntry>, entry: HistoryEntry) {
    stack.push(entry);
    let excess = stack.len().saturating_sub(MAX_UNDO_STEPS);
    stack.drain(..excess);
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
    /// When the next background fetch is due.
    pub next_fetch: Instant,
    /// When the repository was last fetched, by hand or in the background.
    pub last_fetched: Option<Instant>,
    /// Why the last background fetch failed, shown quietly in the status bar.
    pub fetch_problem: Option<String>,
    pub selection: Selection,
    /// Commits chosen together with Command/Ctrl-click, for cherry-picking several.
    pub marked: BTreeSet<String>,
    /// A commit marked for comparison with another.
    pub compare_base: Option<String>,
    pub commit_files: Vec<(String, String)>,
    /// The files of the selected stash, with that stash's hash, once listed or when listing failed.
    pub stash_files: Option<(String, Result<Vec<nicegit_core::StashFile>, String>)>,
    pub details: Option<Task<nicegit_core::Result<nicegit_core::client::CommitDetails>>>,
    pub diff: Option<DiffContent>,
    pub diff_loading: bool,
    pub diff_generation: u64,
    /// The selected change prepared for staging individual lines. On a refresh of the same file
    /// and side, the previous review stays until its replacement arrives.
    pub review: Option<Task<nicegit_core::Result<nicegit_core::staging::FileReview>>>,
    /// Whether `review` is still waiting for its replacement.
    pub review_loading: bool,
    /// Each shown diff line's place in the review's full-file diff, once both have loaded:
    /// `Some(None)` when they do not match, so lines cannot be staged one by one.
    pub line_map: Option<Option<std::sync::Arc<Vec<Option<usize>>>>>,
    pub diff_options: crate::diff_view::DiffOptions,
    pub draft: Draft,
    /// The message of a commit being made, put back in the commit box if the commit fails.
    pub sent_draft: Option<Draft>,
    pub draft_key: Option<String>,
    pub amend: bool,
    pub commit_filter: String,
    pub branch_filter: String,
    /// Text that narrows the Add trailers menu's list of co-authors. Not saved.
    pub trailer_filter: String,
    /// The Git name and email, read while the Add trailers menu is open and cleared when it closes.
    pub trailer_identity: Option<(String, String)>,
    /// Recorded actions Undo steps back through, newest last (at most `MAX_UNDO_STEPS`).
    pub undo: Vec<HistoryEntry>,
    /// Undone actions Redo steps forward through, newest last.
    pub redo: Vec<HistoryEntry>,
    /// A discard that can be reversed while the file is unchanged since.
    /// The last discards that can be reversed, newest last (at most 20).
    pub discard_undo: Vec<DiscardUndo>,
    pub scroll_to_selection: bool,
    /// A bisect in progress, read after each refresh.
    pub bisect: Option<Task<Option<nicegit_core::bisect::BisectStatus>>>,
    /// Submodules recorded in the index, read after each refresh.
    pub submodules: Option<Task<Vec<nicegit_core::submodule::Submodule>>>,
    /// Lines added and removed in each changed file, read after each refresh.
    pub line_counts: Option<Task<Option<nicegit_core::line_counts::LineCounts>>>,
    /// Both versions of an image file whose change is open, read alongside its diff.
    pub image_preview: Option<Task<Result<crate::tools::compare::ImagePair, String>>>,
    /// The open image's versions, uploaded for drawing once read.
    pub image_textures: Option<crate::tools::compare::Textures>,
    /// Each local branch's distance from its upstream, read after each refresh.
    pub branch_tracking: Option<Task<Option<std::collections::HashMap<String, nicegit_core::parsers::Tracking>>>>,
    /// Lines added and removed in each file of the selected commit, with that commit's hash.
    pub commit_line_counts: Option<(String, Task<Option<FileLineCounts>>)>,
    /// Text that narrows the Changes panel's lists to matching paths.
    pub change_filter: String,
    /// Pull requests and issues loaded into the sidebar, in that order.
    pub github: [Option<crate::ui::github_section::GitHubList>; 2],
    /// The checkout's commit template being read in the background, with that checkout's draft
    /// key. Its message fills the draft when it arrives, if the draft is still empty.
    pub template_read: Option<(String, Task<Option<String>>)>,
    /// The draft last filled from `commit.template`. The commit box labels it while it is unchanged.
    pub draft_from_template: Option<Draft>,
    /// Each tag's details for its sidebar hover text, by tag name. A refresh drops the entries
    /// whose tip has moved or whose tag is gone.
    pub tag_details: std::collections::HashMap<String, TagDetailsRead>,
    /// The current branch's pull request and its checks, read through gh, with the branch it was
    /// read for. The status bar shows it only while the checkout is still on that branch.
    pub pr_status: Option<(String, crate::ui::pull_request::PrStatusRead)>,
    /// When the pull request status was last asked for. `None` asks again at the next snapshot.
    pub pr_checked_at: Option<Instant>,
}

impl Repo {
    /// Whether the draft is the commit template exactly as it was filled in. It is not committed
    /// or saved as a draft until it is edited, as Git refuses an unedited template.
    pub fn draft_is_untouched_template(&self) -> bool {
        self.draft_from_template.as_ref() == Some(&self.draft)
    }

    /// Records an action Undo can reverse. It ends any chain of redo steps, as they no longer
    /// follow from the repository's state.
    pub fn record_step(&mut self, entry: HistoryEntry) {
        self.redo.clear();
        push_capped(&mut self.undo, entry);
    }

    /// Records the inverse of an undo, for Redo to apply.
    pub fn push_redo(&mut self, entry: HistoryEntry) {
        push_capped(&mut self.redo, entry);
    }

    /// Records the inverse of a redo, for Undo to apply. Redo steps not yet applied stay.
    pub fn push_undo_keep_redo(&mut self, entry: HistoryEntry) {
        push_capped(&mut self.undo, entry);
    }

    /// Takes the newest step Undo would apply.
    pub fn pop_undo(&mut self) -> Option<HistoryEntry> {
        self.undo.pop()
    }

    /// Takes the newest step Redo would apply.
    pub fn pop_redo(&mut self) -> Option<HistoryEntry> {
        self.redo.pop()
    }

    /// Drops the rest of the stack a step failed on. Its older steps assume the state the failed
    /// step would have produced, so they cannot apply in order. Returns whether any were dropped.
    pub fn clear_on_failure(&mut self, redo: bool) -> bool {
        let stack = if redo { &mut self.redo } else { &mut self.undo };
        let cleared = !stack.is_empty();
        stack.clear();
        cleared
    }

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
            // Soon after opening, rather than at once, so opening stays quick.
            next_fetch: Instant::now() + FIRST_FETCH_DELAY,
            last_fetched: None,
            fetch_problem: None,
            selection: Selection::None,
            marked: BTreeSet::new(),
            compare_base: None,
            commit_files: Vec::new(),
            stash_files: None,
            details: None,
            diff: None,
            diff_loading: false,
            diff_generation: 0,
            review: None,
            review_loading: false,
            line_map: None,
            diff_options: crate::diff_view::DiffOptions::default(),
            draft: Draft::default(),
            sent_draft: None,
            draft_key: None,
            amend: false,
            commit_filter: String::new(),
            branch_filter: String::new(),
            trailer_filter: String::new(),
            trailer_identity: None,
            undo: Vec::new(),
            redo: Vec::new(),
            discard_undo: Vec::new(),
            scroll_to_selection: false,
            bisect: None,
            submodules: None,
            line_counts: None,
            change_filter: String::new(),
            commit_line_counts: None,
            branch_tracking: None,
            image_preview: None,
            image_textures: None,
            github: [None, None],
            template_read: None,
            draft_from_template: None,
            tag_details: std::collections::HashMap::new(),
            pr_status: None,
            pr_checked_at: None,
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
    /// The Go to file finder over the active repository's tracked files, while it is open.
    pub go_to_file: Option<crate::ui::go_to_file::GoToFile>,
    pub show_settings: bool,
    pub show_shortcuts: bool,
    /// Whether the fetch running now should end by offering to merge or rebase onto the upstream.
    pub integrate_after_fetch: bool,
    /// Whether the saved interface size has been applied since launch.
    pub(crate) zoom_applied: bool,
    pub git_missing: bool,
    generation: u64,
    was_focused: bool,
    /// Returning to the app asks for a refresh, made once nothing is being reviewed or run.
    pub(crate) activation_refresh: bool,
    /// How wide the diff panel was when last shown, so Settings can say whether side by side
    /// is in effect.
    pub(crate) diff_panel_width: Option<f32>,
    watcher: Option<Watch>,
    watched_change: Option<Instant>,
    applied_appearance: Option<(theme::Appearance, theme::GraphPalette)>,
    pub pending_clone: Option<Task<nicegit_core::Result<PathBuf>>>,
    /// The "From GitHub…" list over the Clone dialog, while it is open.
    pub clone_picker: Option<crate::ui::clone_picker::ClonePicker>,
    /// The Create pull request dialog, while it is open.
    pub pull_request: Option<crate::ui::pull_request::PullRequestDialog>,
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
            go_to_file: None,
            show_settings: false,
            show_shortcuts: false,
            integrate_after_fetch: false,
            zoom_applied: false,
            git_missing: nicegit_core::runner::git_executable().is_none(),
            generation: 0,
            was_focused: true,
            activation_refresh: false,
            diff_panel_width: None,
            watcher: None,
            watched_change: None,
            applied_appearance: None,
            pending_clone: None,
            clone_picker: None,
            pull_request: None,
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

    /// Applies the saved interface size at launch, then remembers changes to it, whether from
    /// Settings or from Command-plus, Command-minus, and Command-0.
    fn keep_zoom(&mut self, ctx: &egui::Context) {
        if self.zoom_applied {
            let zoom = ctx.zoom_factor();
            // The panels' widths are in points, so after a change in size they are fitted to the
            // window again; otherwise a larger interface would squeeze the history and diff.
            if (zoom - self.settings.zoom).abs() > 0.001 {
                restore_panel_widths(ctx);
            }
            self.settings.zoom = zoom;
        } else {
            ctx.set_zoom_factor(self.settings.zoom.clamp(MIN_ZOOM, MAX_ZOOM));
            self.zoom_applied = true;
        }
    }

    /// Opens folders dropped onto the window as repositories. While something is dragged over
    /// the window, it says what dropping it will do.
    fn drop_folders(&mut self, ctx: &egui::Context) {
        let (hovering, dropped) = ctx.input(|i| (!i.raw.hovered_files.is_empty(), i.raw.dropped_files.clone()));
        if hovering {
            let screen = ctx.content_rect();
            let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("drop target")));
            let c = crate::theme::colors(ctx.theme() == egui::Theme::Dark);
            painter.rect_filled(screen, 0.0, c.accent.gamma_multiply(0.12));
            painter.rect_stroke(screen.shrink(6.0), 12.0, egui::Stroke::new(2.0, c.accent), egui::StrokeKind::Inside);
            // On a solid label, so the words stay readable over whatever is beneath.
            let text =
                painter.layout_no_wrap("Drop a folder to open it as a repository".into(), egui::FontId::proportional(18.0), c.accent_text);
            let label = egui::Rect::from_center_size(screen.center(), text.size() + egui::vec2(36.0, 22.0));
            painter.rect_filled(label, 10.0, c.accent);
            painter.galley(label.center() - text.size() / 2.0, text, c.accent_text);
        }
        let paths: Vec<PathBuf> =
            dropped.iter().map(|file| file.path().to_path_buf()).filter(|path| !path.as_os_str().is_empty()).collect();
        if paths.is_empty() {
            return;
        }
        let (folders, files): (Vec<PathBuf>, Vec<PathBuf>) = paths.into_iter().partition(|path| path.is_dir());
        for folder in folders {
            self.open(folder);
        }
        if !files.is_empty() {
            self.notify("Drop a folder, not a file, to open it as a repository.", true);
        }
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

    /// Closes every repository tab but `keep`, which becomes the active one. Refused while an
    /// action runs or the editor has unsaved text, as closing a single tab is.
    pub fn close_other_tabs(&mut self, keep: usize) {
        if keep >= self.repos.len() || self.busy.is_some() {
            return;
        }
        // Checked before switching, so the refusal names closing rather than switching.
        if self.has_unsaved_editor() {
            self.notify("Save or close the file you are editing first.", true);
            return;
        }
        if keep != self.active {
            self.switch_to(keep);
            if keep != self.active {
                return;
            }
        }
        self.save_draft();
        let kept = self.repos.remove(self.active);
        self.repos.clear();
        self.repos.push(kept);
        self.active = 0;
        self.watch_active();
    }

    pub fn choose_folder(&mut self) {
        if self.busy.is_some() {
            return;
        }
        if let Some(folder) = crate::file_dialog::pick_folder("Open a Git repository") {
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
        // A deliberate load reads the pull request status again, once the snapshot arrives.
        if deliberate {
            repo.pr_checked_at = None;
        }
        let limit = repo.history_limit;
        let scope = nicegit_core::HistoryScope::from(self.settings.history_scope);
        self.worker.load(self.generation, repo.path.clone(), deliberate, limit, scope);
    }

    /// Fetches the open repository from its remotes every few minutes, as Settings chooses, so
    /// ahead and behind counts stay current. It waits while anything else runs, a dialog is
    /// open, or the editor holds unsaved text, and a failure shows quietly in the status bar.
    fn auto_fetch(&mut self, ctx: &egui::Context) {
        let minutes = self.settings.auto_fetch_minutes;
        let Some(repo) = self.repos.get(self.active) else { return };
        if minutes == 0 || repo.snapshot.as_ref().is_none_or(|s| s.remotes.is_empty()) {
            return;
        }
        let now = Instant::now();
        if now < repo.next_fetch {
            ctx.request_repaint_after(repo.next_fetch - now);
            return;
        }
        let quiet = self.busy.is_none()
            && self.dialog.is_none()
            && self.palette.is_none()
            && self.go_to_file.is_none()
            && !repo.loading
            && !self.has_unsaved_editor()
            && !self.tools.iter().any(|tool| tool.holds_refresh());
        if !quiet {
            ctx.request_repaint_after(Duration::from_secs(5));
            return;
        }
        self.repos[self.active].next_fetch = now + Duration::from_secs(u64::from(minutes) * 60);
        self.act(BACKGROUND_FETCH, |client, path| client.fetch(path).map(|_| None));
    }

    /// Reads the active checkout's pull request status every few minutes while the window is
    /// focused. It is lighter than auto-fetch: only `gh` runs, and nothing waits on Git. It waits
    /// while an action runs, a dialog is open, or the repository is loading.
    fn auto_pr_status(&mut self, ctx: &egui::Context) {
        if !self.was_focused {
            return;
        }
        let Some(repo) = self.repos.get(self.active) else { return };
        if repo.snapshot.is_none() {
            return;
        }
        let now = Instant::now();
        if let Some(checked) = repo.pr_checked_at {
            let due = checked + PR_STATUS_INTERVAL;
            if now < due {
                ctx.request_repaint_after(due - now);
                return;
            }
        }
        let quiet = self.busy.is_none() && self.dialog.is_none() && !repo.loading;
        if !quiet {
            ctx.request_repaint_after(Duration::from_secs(5));
            return;
        }
        let active = self.active;
        self.refresh_pr_status(active);
        ctx.request_repaint_after(PR_STATUS_INTERVAL);
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
        // A background fetch leaves the last message for the person to read.
        if label != BACKGROUND_FETCH {
            self.notice = None;
        }
        let limit = repo.history_limit;
        let scope = nicegit_core::HistoryScope::from(self.settings.history_scope);
        self.worker.act(self.generation, repo.path.clone(), label.to_string(), limit, scope, action);
    }

    /// Runs an action that moves the current branch, recording it so it can be undone. Only a
    /// completed move is recorded: an operation that stopped partway is not undoable.
    pub fn act_recording(&mut self, label: &str, title: &str, action: impl FnOnce(&GitClient, &Path) -> ActionResult + Send + 'static) {
        self.act_recording_detail(label, title, None, action);
    }

    /// Like `act_recording`, with the detail Undo and Redo name the step by, such as the commit's summary.
    pub fn act_recording_detail(
        &mut self,
        label: &str,
        title: &str,
        detail: Option<String>,
        action: impl FnOnce(&GitClient, &Path) -> ActionResult + Send + 'static,
    ) {
        let mode = match title {
            "Commit" | "Amend" | "Undo commit" => UndoMode::Soft,
            _ => UndoMode::Keep,
        };
        self.act_recording_mode_detail(label, title, detail, mode, action);
    }

    pub fn act_recording_mode_detail(
        &mut self,
        label: &str,
        title: &str,
        detail: Option<String>,
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
                        *slot.lock().unwrap_or_else(|p| p.into_inner()) = Some(Recorded::Step(HistoryEntry { title, detail, step }));
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
        self.notice = Some(Notice { text: text.into(), is_error, action: None, detail: None });
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
                    let history_note = if action.is_some() { self.take_record(index) } else { None };
                    if let Some((label, result)) = &action {
                        // Fetching, by hand, in the background, or as part of Pull, resets the timer.
                        if matches!(label.as_str(), BACKGROUND_FETCH | "Fetch" | "Pull") {
                            let repo = &mut self.repos[index];
                            let minutes = u64::from(self.settings.auto_fetch_minutes.max(1));
                            repo.next_fetch = Instant::now() + Duration::from_secs(minutes * 60);
                            match result {
                                Ok(_) => {
                                    repo.last_fetched = Some(Instant::now());
                                    repo.fetch_problem = None;
                                }
                                Err(error) if label == BACKGROUND_FETCH => repo.fetch_problem = Some(error.to_string()),
                                Err(_) => {}
                            }
                        }
                    }
                    // Pushing, pulling, and fetching can change the pull request or its checks, so it is read again.
                    if matches!(&action, Some((label, _)) if crate::ui::pull_request::CHANGES_PULL_REQUEST.contains(&label.as_str())) {
                        self.repos[index].pr_checked_at = None;
                    }
                    let fetched = matches!(&action, Some((label, Ok(_))) if label == "Fetch");
                    let fetch_failed = matches!(&action, Some((label, Err(_))) if label == "Fetch");
                    let action = action.filter(|(label, _)| label != BACKGROUND_FETCH);
                    // A commit that failed gives its message back; one that was made keeps it cleared,
                    // and the box starts from the commit template for the next one.
                    let mut made_commit = false;
                    if let Some((label, result)) = &action {
                        if matches!(label.as_str(), "Commit" | "Amend" | "Commit and push") {
                            let repo = &mut self.repos[index];
                            let sent = repo.sent_draft.take();
                            let committed = match result {
                                Ok(_) => true,
                                Err(error) => label == "Commit and push" && error.to_string().contains("The commit was made"),
                            };
                            if let (false, Some(sent)) = (committed, sent) {
                                if repo.draft.is_empty() {
                                    repo.draft = sent;
                                }
                            }
                            made_commit = committed;
                        }
                    }
                    if made_commit {
                        self.start_template_read(index);
                    }
                    // Git's own error, when an action failed in a way the refresh may explain better.
                    let mut stopped_with: Option<String> = None;
                    if let Some((label, result)) = action {
                        match result {
                            Ok(Some(text)) => self.record_result(index, &text),
                            Ok(None) => {}
                            // The tag was created before its push failed, so the notice must say so
                            // rather than be mapped to a generic remote explanation.
                            Err(error) if label == "Create tag" && error.to_string().contains("but could not push it to") => {
                                action_failed = true;
                                let text = match &error {
                                    GitError::CommandFailed { message, .. } => message.clone(),
                                    other => other.to_string(),
                                };
                                self.notice = Some(Notice { text, is_error: true, action: None, detail: Some(error.to_string()) });
                            }
                            // A bulk resolution that resolved no file reports the files it could not, as its
                            // whole notice rather than as a failed command.
                            Err(error) if label == "Resolve conflicts" => {
                                action_failed = true;
                                let text = match &error {
                                    GitError::CommandFailed { message, .. } => message.clone(),
                                    other => other.to_string(),
                                };
                                self.notice = Some(Notice { text, is_error: true, action: None, detail: Some(error.to_string()) });
                            }
                            Err(error) if label == "Force push" && lease_was_refused(&error.to_string()) => {
                                action_failed = true;
                                self.notice = Some(Notice {
                                    text: "The remote branch changed since you last fetched, so nothing was pushed. Fetch, review, and try again.".into(),
                                    is_error: true,
                                    action: None,
                                    detail: Some(error.to_string()),
                                });
                            }
                            Err(error) if matches!(label.as_str(), "Push" | "Commit and push") && push_needs_pull(&error.to_string()) => {
                                action_failed = true;
                                let committed = if label == "Commit and push" { "Your commit was made, but the " } else { "The " };
                                // The snapshot is still the one from before the push, so its counts show whether the
                                // branch had commits of its own as well as the remote's.
                                let diverged = self.repos[index].snapshot.as_ref().is_some_and(|s| s.ahead > Some(0) && s.behind > Some(0));
                                let rewritten = if label == "Push" && diverged {
                                    " If you rewrote commits that were already pushed, use Force push… instead."
                                } else {
                                    ""
                                };
                                self.notice = Some(Notice {
                                    text: format!(
                                        "{committed}push was refused because the remote has commits this branch does not. Merge them or rebase onto them, then push again.{rewritten}"
                                    ),
                                    is_error: true,
                                    action: Some(NoticeAction::Integrate),
                                    detail: Some(error.to_string()),
                                });
                            }
                            Err(error) if label == "Continue" && blocked_by_unstaged_changes(&error.to_string()) => {
                                action_failed = true;
                                self.notice = Some(Notice {
                                    text: "Stage or discard your unstaged changes before continuing.".into(),
                                    is_error: true,
                                    action: None,
                                    detail: Some(error.to_string()),
                                });
                            }
                            Err(error) if blocked_by_local_changes(&error.to_string()) => {
                                action_failed = true;
                                self.notice = Some(Notice {
                                    text: format!("{label} would overwrite uncommitted changes to the same files, so Git stopped. Stash or commit them, then try again."),
                                    is_error: true,
                                    action: Some(NoticeAction::Stash),
                                    detail: Some(error.to_string()),
                                });
                            }
                            Err(error) if needs_identity(&error.to_string()) => {
                                action_failed = true;
                                self.notice = Some(Notice {
                                    text: "Git needs your name and email before it can commit. Set them in Repository Settings, for this repository or as a profile.".into(),
                                    is_error: true,
                                    action: Some(NoticeAction::SetIdentity),
                                    detail: Some(error.to_string()),
                                });
                            }
                            Err(error) if remote_problem(&error.to_string()).or(naming_problem(&error.to_string())).is_some() => {
                                action_failed = true;
                                let plain = remote_problem(&error.to_string()).or(naming_problem(&error.to_string())).unwrap_or_default();
                                self.notice = Some(Notice {
                                    text: format!("{label} failed. {plain}"),
                                    is_error: true,
                                    action: None,
                                    detail: Some(error.to_string()),
                                });
                            }
                            Err(error) => {
                                action_failed = true;
                                self.notify(format!("{label} failed: {error}"), true);
                                stopped_with = Some(error.to_string());
                            }
                        }
                    }
                    // Only the fetch started for a refused push offers the choice; a failed one ends it.
                    let offer_integration = fetched && std::mem::take(&mut self.integrate_after_fetch);
                    if fetch_failed {
                        self.integrate_after_fetch = false;
                    }
                    match *snapshot {
                        Ok(snapshot) => {
                            // An action that stopped at conflicts left an operation to finish: that
                            // is a pause for the person, not a failure, so it is described as one.
                            if let (Some(detail), Some(operation)) = (stopped_with.take(), snapshot.operation) {
                                let conflicts = snapshot.status.iter().filter(|e| e.kind == nicegit_core::StatusKind::Conflicted).count();
                                if conflicts > 0 {
                                    let plural = if conflicts == 1 { "" } else { "s" };
                                    self.notice = Some(Notice {
                                        text: format!(
                                            "The {} stopped at conflicts in {conflicts} file{plural}. Resolve them from the banner above the history, then continue, or abort to go back.",
                                            operation.name()
                                        ),
                                        is_error: true,
                                        action: None,
                                        detail: Some(detail),
                                    });
                                }
                            }
                            self.apply(index, snapshot);
                            // After fetching for a refused push, the upstream's commits are offered
                            // through the usual merge or rebase choice, with its preview.
                            if offer_integration && !action_failed && index == self.active {
                                if let Some(upstream) = self.upstream_branch() {
                                    self.dialog = Some(crate::ui::dialogs::Dialog::Integrate { source: upstream });
                                }
                            }
                        }
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
                    // Said after whatever the failed step said, so it is seen without hovering.
                    if let (Some(note), Some(notice)) = (history_note, self.notice.as_mut()) {
                        notice.text = format!("{} {note}", notice.text.trim_end());
                    }
                }
                Message::Diff { generation, title, result } => {
                    let Some(repo) = self.repos.iter_mut().find(|r| r.diff_generation == generation) else { continue };
                    repo.diff_loading = false;
                    repo.line_map = None;
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
                Message::StashFiles { path, hash, result } => {
                    let Some(repo) = self.repos.iter_mut().find(|r| r.path == path) else { continue };
                    if !matches!(&repo.selection, Selection::Stash { hash: h, .. } if *h == hash) {
                        continue;
                    }
                    repo.stash_files = Some((hash, result.map_err(|error| error.to_string())));
                }
            }
        }
    }

    fn record_result(&mut self, _index: usize, text: &str) {
        self.notify(text.to_string(), false);
    }

    /// Takes whatever the finished action recorded for undo. Returns the note for the notice when
    /// a failed step dropped older steps with it.
    fn take_record(&mut self, index: usize) -> Option<&'static str> {
        let record = self.recorded.lock().unwrap_or_else(|p| p.into_inner()).take();
        let repo = &mut self.repos[index];
        match record {
            Some(Recorded::Step(entry)) => repo.record_step(entry),
            Some(Recorded::Redo(entry)) => repo.push_redo(entry),
            Some(Recorded::Undo(entry)) => repo.push_undo_keep_redo(entry),
            Some(Recorded::HistoryFailed { redo }) => {
                if repo.clear_on_failure(redo) {
                    return Some(if redo { CLEARED_REDO_NOTE } else { CLEARED_UNDO_NOTE });
                }
            }
            Some(Recorded::Discard(undo)) => {
                repo.discard_undo.retain(|older| older.path != undo.path);
                repo.discard_undo.push(undo);
                let excess = repo.discard_undo.len().saturating_sub(20);
                repo.discard_undo.drain(..excess);
            }
            None => {}
        }
        None
    }

    fn apply(&mut self, index: usize, snapshot: Snapshot) {
        self.settings.remember(PathBuf::from(&snapshot.root_path));
        let repo = &mut self.repos[index];
        repo.error = None;
        // A filter narrows the files of one branch; a different branch starts with the full list.
        if repo.snapshot.as_ref().is_some_and(|old| old.current_branch != snapshot.current_branch) {
            repo.change_filter.clear();
        }
        let branch_changed = repo.snapshot.as_ref().is_none_or(|old| old.current_branch != snapshot.current_branch);
        let dirty = !snapshot.status.is_empty();
        let colors = theme::GRAPH_COLOR_COUNT;
        repo.rows = if dirty {
            layout_with_working_tree(&snapshot.commits, snapshot.head_hash.as_deref(), colors)
        } else {
            layout(&snapshot.commits, snapshot.head_hash.as_deref(), colors)
        };
        repo.lanes = repo.rows.iter().map(|row| row.lane_count).max().unwrap_or(1);
        // A selected file stays open while it still has changes on its side, even when staging
        // part of it changed its status; it then shows its new status.
        if let Selection::Change { entry, staged } = &repo.selection {
            let same_file = |e: &&StatusEntry| e.path == entry.path && e.original_path == entry.original_path;
            let side = |e: &&StatusEntry| if *staged { e.is_staged() } else { e.is_unstaged() };
            let other_side = |e: &&StatusEntry| if *staged { e.is_unstaged() } else { e.is_staged() };
            if let Some(current) = snapshot.status.iter().filter(same_file).find(side) {
                repo.selection = Selection::Change { entry: current.clone(), staged: *staged };
            } else if let Some(moved) = snapshot.status.iter().filter(same_file).find(other_side) {
                // Staging or unstaging the whole file moved it to the other side; it stays open there.
                repo.selection = Selection::Change { entry: moved.clone(), staged: !*staged };
            }
        }
        // Keep the selection only while it still exists in the new state.
        let keep = match &repo.selection {
            Selection::None => true,
            Selection::WorkingTree => dirty,
            Selection::Change { entry, staged } => {
                snapshot.status.iter().any(|e| e == entry && (if *staged { e.is_staged() } else { e.is_unstaged() }))
            }
            Selection::Commit { hash, .. } => snapshot.commits.iter().any(|c| &c.hash == hash),
            Selection::Stash { hash, .. } => snapshot.stashes.iter().any(|s| &s.hash == hash),
        };
        repo.marked.retain(|hash| snapshot.commits.iter().any(|c| &c.hash == hash));
        // Each checkout keeps its own commit message draft.
        let key = Settings::draft_key(&snapshot.root_path);
        let mut read_template = false;
        if repo.draft_key.as_deref() != Some(key.as_str()) {
            if let Some(old) = repo.draft_key.take() {
                if repo.draft.is_empty() || repo.draft_is_untouched_template() {
                    self.settings.drafts.remove(&old);
                } else {
                    self.settings.drafts.insert(old, repo.draft.clone());
                }
            }
            repo.draft = self.settings.drafts.get(&key).cloned().unwrap_or_default();
            repo.draft_key = Some(key);
            repo.draft_from_template = None;
            repo.amend = false;
            repo.change_filter.clear();
            // An empty draft starts from the checkout's commit template.
            read_template = repo.draft.is_empty();
        }
        let reload_change = matches!(repo.selection, Selection::Change { .. }) && keep;
        if index == self.active {
            for tool in &mut self.tools {
                tool.repository_changed(&snapshot);
            }
        }
        self.repos[index]
            .tag_details
            .retain(|name, (tip, _)| snapshot.tag_tips.get(name).is_some_and(|current| current.as_str() == tip.as_str()));
        self.repos[index].snapshot = Some(snapshot);
        // The pull request is read for a new branch, and whenever a read is due.
        if branch_changed || self.repos[index].pr_checked_at.is_none() {
            self.refresh_pr_status(index);
        }
        if read_template {
            self.start_template_read(index);
        }
        // Bisect state and submodules are read alongside, without delaying the snapshot. The bar
        // keeps the last state until the new read arrives, so it does not vanish (and take its
        // buttons with it) on every refresh.
        let context = self.worker.context.clone();
        let path = self.repos[index].path.clone();
        let bisect_path = path.clone();
        let previous_bisect = self.repos[index].bisect.take().and_then(|mut task| task.get().cloned().flatten());
        self.repos[index].bisect = Some(Task::spawn_with(&context, previous_bisect.map(Some), move || {
            GitClient::new().bisect_status(&bisect_path).ok().flatten()
        }));
        let untracked: Vec<String> = self.repos[index]
            .snapshot
            .iter()
            .flat_map(|s| &s.status)
            .filter(|e| e.kind == nicegit_core::StatusKind::Untracked)
            .map(|e| e.path.clone())
            .collect();
        let changed: Vec<String> = self.repos[index].snapshot.iter().flat_map(|s| &s.status).map(|e| e.path.clone()).collect();
        let counts_path = path.clone();
        let tracking_path = path.clone();
        let previous_tracking = self.repos[index].branch_tracking.take().and_then(|mut task| task.get().cloned().flatten());
        self.repos[index].branch_tracking =
            Some(Task::spawn_with(&context, previous_tracking.map(Some), move || GitClient::new().branch_tracking(&tracking_path).ok()));
        self.repos[index].submodules = Some(Task::spawn(&context, move || GitClient::new().submodules(&path).unwrap_or_default()));
        // The previous counts stay until the new ones arrive, so the numbers do not flicker.
        let previous = self.repos[index].line_counts.take().and_then(|mut task| task.get().cloned().flatten());
        self.repos[index].line_counts = Some(Task::spawn_with(&context, previous.map(Some), move || {
            GitClient::new().line_counts(&untracked, &changed, &counts_path).ok()
        }));
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

    /// Reads the checkout's commit template in the background. Its message fills the draft when
    /// it arrives, if the draft is still empty.
    fn start_template_read(&mut self, index: usize) {
        let Some(key) = self.repos[index].draft_key.clone() else { return };
        let path = self.repos[index].path.clone();
        let context = self.worker.context.clone();
        let task = Task::spawn(&context, move || GitClient::new().commit_template(&path).ok().flatten());
        self.repos[index].template_read = Some((key, task));
    }

    /// Fills each repository's empty draft with its commit template once the template has been
    /// read. A read for a checkout the repository has since left is dropped, and so is one for an
    /// amend, which fills the draft from HEAD instead.
    fn receive_templates(&mut self) {
        for repo in &mut self.repos {
            let Some((key, task)) = repo.template_read.as_mut() else { continue };
            if task.is_loading() {
                continue;
            }
            let template = task.peek().cloned().flatten();
            let key = key.clone();
            repo.template_read = None;
            if repo.draft_key.as_deref() != Some(key.as_str()) || !repo.draft.is_empty() || repo.amend {
                continue;
            }
            repo.draft_from_template = None;
            if let Some(template) = template {
                let draft = Draft::from_message(&template);
                repo.draft = draft.clone();
                repo.draft_from_template = Some(draft);
            }
        }
    }

    pub fn save_draft(&mut self) {
        let Some(repo) = self.repos.get(self.active) else { return };
        if let Some(key) = repo.draft_key.clone() {
            if repo.draft.is_empty() || repo.draft_is_untouched_template() {
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
        repo.line_map = None;
        repo.image_preview = None;
        repo.image_textures = None;
        repo.diff_options.selected.clear();
        repo.commit_files.clear();
        repo.stash_files = None;
        repo.diff_generation = next_id();
        repo.diff_loading = false;
    }

    fn start_diff(&mut self) -> Option<(u64, PathBuf)> {
        let repo = self.repos.get_mut(self.active)?;
        repo.review = None;
        repo.line_map = None;
        repo.image_preview = None;
        repo.image_textures = None;
        repo.diff_options.selected.clear();
        repo.diff_generation = next_id();
        repo.diff_loading = true;
        Some((repo.diff_generation, repo.path.clone()))
    }

    /// Selects the working tree, listing its changes without opening a file: a file's diff
    /// takes the history's place, so it opens only when a file is chosen.
    pub fn select_working_tree(&mut self) {
        self.clear_selection();
        if let Some(repo) = self.repo_mut() {
            repo.selection = Selection::WorkingTree;
        }
    }

    /// Whether a file's diff is open. It fills the main area in place of the history.
    pub fn showing_diff(&self) -> bool {
        self.repo().is_some_and(|r| {
            matches!(r.selection, Selection::Change { .. } | Selection::Stash { .. } | Selection::Commit { file: Some(_), .. })
        })
    }

    /// The files the open diff can step through, in the order they are listed: the Changes
    /// panel's unstaged then staged files, or the selected commit's files. Also the open file's
    /// place among them.
    fn diff_files(&self) -> Option<(Vec<Selection>, usize)> {
        let repo = self.repo()?;
        let files: Vec<Selection> = match &repo.selection {
            Selection::Change { .. } => {
                let status = &repo.snapshot.as_ref()?.status;
                let filter = repo.change_filter.as_str();
                let side = |staged: bool| {
                    let mut entries: Vec<&StatusEntry> = status
                        .iter()
                        .filter(|e| if staged { e.is_staged() } else { e.is_unstaged() })
                        .filter(|e| crate::ui::changes::matches_filter(e, filter))
                        .collect();
                    entries.sort_by_key(|e| e.path.to_lowercase());
                    entries.into_iter().map(move |e| Selection::Change { entry: e.clone(), staged })
                };
                side(false).chain(side(true)).collect()
            }
            Selection::Commit { hash, file: Some(_) } => {
                repo.commit_files.iter().map(|(_, path)| Selection::Commit { hash: hash.clone(), file: Some(path.clone()) }).collect()
            }
            Selection::Stash { hash, file: Some(_) } => match &repo.stash_files {
                Some((listed, Ok(files))) if listed == hash => {
                    files.iter().map(|file| Selection::Stash { hash: hash.clone(), file: Some(file.path.clone()) }).collect()
                }
                _ => return None,
            },
            _ => return None,
        };
        let same = |candidate: &Selection| match (candidate, &repo.selection) {
            (Selection::Change { entry: a, staged: x }, Selection::Change { entry: b, staged: y }) => {
                a.path == b.path && a.original_path == b.original_path && x == y
            }
            (a, b) => a == b,
        };
        let position = files.iter().position(same)?;
        Some((files, position))
    }

    /// Where the open file sits among the files it can step through, as (position, count).
    pub fn diff_file_position(&self) -> Option<(usize, usize)> {
        self.diff_files().map(|(files, position)| (position, files.len()))
    }

    /// Opens the next file (`step` 1) or the previous one (-1) in the list the open diff came
    /// from, if there is one.
    pub fn step_diff_file(&mut self, step: isize) {
        let Some((files, position)) = self.diff_files() else { return };
        let Some(target) = position.checked_add_signed(step).and_then(|index| files.get(index)).cloned() else { return };
        match target {
            Selection::Change { entry, staged } => self.select_change(entry, staged),
            Selection::Commit { hash, file } => self.select_commit_file(hash, file),
            Selection::Stash { hash, file } => {
                if let Some(stash) = self.stash_for(&hash) {
                    self.select_stash_file(stash, file);
                }
            }
            _ => {}
        }
    }

    /// Fetches, then offers to merge the current branch's upstream or rebase onto it.
    pub fn integrate_upstream(&mut self) {
        if self.busy.is_some() {
            return;
        }
        self.fetch();
        self.integrate_after_fetch = self.busy.is_some();
    }

    /// The current branch's upstream, as a remote branch from the latest refresh.
    fn upstream_branch(&self) -> Option<Branch> {
        let snapshot = self.snapshot()?;
        let current = snapshot.branches.iter().find(|b| b.is_current && !b.is_remote)?;
        let upstream = current.upstream.as_deref()?;
        snapshot.branches.iter().find(|b| b.is_remote && format!("refs/{}", b.name) == upstream).cloned()
    }

    /// Stages the open file, or unstages it when its staged side is open. A conflicted file is
    /// left for the conflict editor, as in the Changes panel.
    pub fn toggle_stage_open_file(&mut self) {
        let Some(Selection::Change { entry, staged }) = self.repo().map(|r| r.selection.clone()) else { return };
        if entry.kind == nicegit_core::StatusKind::Conflicted {
            return;
        }
        if staged {
            self.act("Unstage", move |client, path| client.unstage(&entry, path).map(|_| None));
        } else {
            self.act("Stage", move |client, path| client.stage(&entry.path, path).map(|_| None));
        }
    }

    /// Closes the open diff and returns to the history, keeping the commit or the working tree
    /// it came from selected.
    pub fn close_diff(&mut self) {
        match self.repo().map(|r| r.selection.clone()) {
            Some(Selection::Commit { hash, file: Some(_) }) => self.select_commit(hash),
            Some(Selection::Stash { hash, file: Some(_) }) => {
                if let Some(stash) = self.stash_for(&hash) {
                    self.select_stash_file(stash, None);
                }
            }
            Some(Selection::Change { .. }) => self.select_working_tree(),
            _ => self.clear_selection(),
        }
    }

    /// The diff options the settings choose: whitespace and context lines.
    fn diff_request(&self) -> DiffRequest {
        DiffRequest { ignore_whitespace: self.settings.ignore_whitespace, context: self.settings.diff_context.context() }
    }

    pub fn select_change(&mut self, entry: StatusEntry, staged: bool) {
        let request = self.diff_request();
        // A refresh reloads the same change; its previous review stays until the new one arrives,
        // so the buttons of its lines do not vanish with each refresh. Staging re-reads the review
        // and refuses when the file has changed, so a previous review cannot act on newer content.
        let previous = self
            .repo_mut()
            .and_then(|repo| repo.review.take())
            .and_then(|mut task| task.get().cloned())
            .filter(|review| matches!(review, Ok(review) if review.path == entry.path && review.staged == staged));
        let Some((generation, path)) = self.start_diff() else { return };
        let repo = self.repo_mut().expect("active repository");
        repo.commit_files.clear();
        repo.details = None;
        repo.selection = Selection::Change { entry: entry.clone(), staged };
        let title = entry.path.clone();
        // New files can be staged line by line too; an untracked folder has no lines.
        if !request.ignore_whitespace && !entry.path.ends_with('/') {
            // A review carries the same lines as the diff, plus what staging single lines needs.
            let (review_path, file) = (path.clone(), entry.path.clone());
            let context = self.worker.context.clone();
            self.repo_mut().expect("active repository").review =
                Some(Task::spawn_with(&context, previous, move || GitClient::new().file_review(&file, staged, &review_path)));
        }
        if crate::tools::compare::is_image(&entry.path) {
            // An image's versions: the index against the working file, or HEAD (under its old
            // name, for a rename) against the index once staged.
            let (preview_path, preview_entry) = (path.clone(), entry.clone());
            self.load_image_preview(move |client| {
                let entry = preview_entry;
                if staged {
                    let old = entry.original_path.as_deref().unwrap_or(&entry.path);
                    let head = client.file_bytes_at(Some("HEAD"), old, &preview_path).unwrap_or(None);
                    Ok((head, client.index_bytes(&entry.path, &preview_path)?))
                } else {
                    Ok((client.index_bytes(&entry.path, &preview_path)?, client.file_bytes_at(None, &entry.path, &preview_path)?))
                }
            });
        }
        self.worker.diff(generation, title, move |client| client.diff_with(&entry, staged, request, &path));
    }

    /// Reads both versions of an image whose change is open, in the background, to show them
    /// in place of Git's note that the binary file differs.
    fn load_image_preview(
        &mut self,
        read: impl FnOnce(&GitClient) -> nicegit_core::Result<(Option<Vec<u8>>, Option<Vec<u8>>)> + Send + 'static,
    ) {
        let context = self.worker.context.clone();
        let Some(repo) = self.repo_mut() else { return };
        repo.image_preview = Some(Task::spawn(&context, move || {
            read(&GitClient::new()).map(|(before, after)| crate::tools::compare::ImagePair::read(before, after)).map_err(|e| e.to_string())
        }));
    }

    pub fn select_commit(&mut self, hash: String) {
        let request = self.diff_request();
        let Some((generation, path)) = self.start_diff() else { return };
        let context = self.worker.context.clone();
        let repo = self.repo_mut().expect("active repository");
        repo.commit_files.clear();
        repo.selection = Selection::Commit { hash: hash.clone(), file: None };
        repo.scroll_to_selection = true;
        let (details_path, details_hash) = (path.clone(), hash.clone());
        repo.details = Some(Task::spawn(&context, move || GitClient::new().commit_details(&details_hash, &details_path)));
        self.worker.commit_files(generation, path.clone(), hash.clone());
        // Line counts for the commit's files, read once per commit.
        let repo = self.repo_mut().expect("active repository");
        if repo.commit_line_counts.as_ref().is_none_or(|(counted, _)| *counted != hash) {
            let (counts_path, counts_hash) = (path.clone(), hash.clone());
            let task = Task::spawn(&context, move || GitClient::new().commit_line_counts(&counts_hash, &counts_path).ok());
            repo.commit_line_counts = Some((hash.clone(), task));
        }
        let title = format!("Commit {}", short(&hash));
        self.worker.diff(generation, title, move |client| client.commit_diff_with(&hash, None, request, &path));
    }

    pub fn select_commit_file(&mut self, hash: String, file: Option<String>) {
        let request = self.diff_request();
        let Some((generation, path)) = self.start_diff() else { return };
        let repo = self.repo_mut().expect("active repository");
        repo.selection = Selection::Commit { hash: hash.clone(), file: file.clone() };
        let title = file.clone().unwrap_or_else(|| format!("Commit {}", short(&hash)));
        if let Some(image) = file.clone().filter(|file| crate::tools::compare::is_image(file)) {
            // The image as its parent had it, against the commit's version.
            let (preview_path, commit) = (path.clone(), hash.clone());
            self.load_image_preview(move |client| {
                let before = client.file_bytes_at(Some(&format!("{commit}^")), &image, &preview_path).unwrap_or(None);
                Ok((before, client.file_bytes_at(Some(&commit), &image, &preview_path)?))
            });
        }
        self.worker.diff(generation, title, move |client| client.commit_diff_with(&hash, file.as_deref(), request, &path));
    }

    /// Selects a stash from the sidebar: its whole diff, with its files listed in the inspector.
    pub fn select_stash(&mut self, stash: Stash) {
        // Listed afresh for each stash chosen here, so a listing that failed is tried again.
        if let Some(repo) = self.repo_mut() {
            repo.stash_files = None;
        }
        self.select_stash_file(stash, None);
    }

    /// Shows a stash's whole diff, or one of its files, listing the files if they are not yet known.
    pub fn select_stash_file(&mut self, stash: Stash, file: Option<String>) {
        let request = self.diff_request();
        // A file's diff depends on whether it is untracked, so its listed entry is needed.
        let entry = match &file {
            None => None,
            Some(name) => {
                let listed = self.repo().and_then(|r| match &r.stash_files {
                    Some((hash, Ok(files))) if *hash == stash.hash => files.iter().find(|f| &f.path == name).cloned(),
                    _ => None,
                });
                let Some(entry) = listed else {
                    self.notify("That file is not listed for this stash. Select the stash again.", true);
                    return;
                };
                Some(entry)
            }
        };
        let Some((generation, path)) = self.start_diff() else { return };
        let repo = self.repo_mut().expect("active repository");
        repo.commit_files.clear();
        repo.details = None;
        repo.selection = Selection::Stash { hash: stash.hash.clone(), file: file.clone() };
        let listing = !matches!(&repo.stash_files, Some((hash, _)) if *hash == stash.hash);
        if listing {
            self.worker.stash_files(path.clone(), stash.clone());
        }
        let title = match &file {
            Some(name) => name.clone(),
            None => format!("{} · {}", stash.reference, stash.message),
        };
        self.worker.diff(generation, title, move |client| match entry {
            Some(entry) => client.stash_file_diff(&stash, &entry, request, &path),
            None => client.stash_diff_with(&stash, request, &path),
        });
    }

    /// The stash with this hash, as the latest refresh listed it.
    fn stash_for(&self, hash: &str) -> Option<Stash> {
        self.snapshot()?.stashes.iter().find(|stash| stash.hash == hash).cloned()
    }

    /// Reloads whatever diff is showing, for example after a diff option changes.
    pub fn reload_diff(&mut self) {
        let Some(repo) = self.repo() else { return };
        match repo.selection.clone() {
            Selection::Change { entry, staged } => self.select_change(entry, staged),
            Selection::Commit { hash, file } => self.select_commit_file(hash, file),
            Selection::Stash { hash, file } => {
                if let Some(stash) = self.stash_for(&hash) {
                    self.select_stash_file(stash, file);
                }
            }
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
            self.act_recording_detail("Amend", "Amend", summary_of(&message), move |client, path| {
                client.amend(&message, &branch, &head, path).map(|_| Some("Amended the last commit.".into()))
            });
        } else {
            self.act_recording_detail("Commit", "Commit", summary_of(&message), move |client, path| {
                client.commit(&message, path).map(|_| None)
            });
        }
        if self.busy.is_some() {
            if let Some(repo) = self.repo_mut() {
                repo.sent_draft = Some(std::mem::take(&mut repo.draft));
                repo.amend = false;
            }
            self.save_draft();
        }
    }

    /// Whether every change can be staged and committed in one step: a summary is written,
    /// nothing is staged yet, there are changes, none is a conflict, and this is not an amend.
    pub fn can_stage_all_and_commit(&self) -> bool {
        let (Some(snapshot), Some(repo)) = (self.snapshot(), self.repo()) else { return false };
        self.idle()
            && !repo.amend
            && !repo.draft.summary.trim().is_empty()
            && !repo.draft_is_untouched_template()
            && snapshot.staged_count() == 0
            && !snapshot.status.is_empty()
            && !snapshot.status.iter().any(|e| e.kind == nicegit_core::StatusKind::Conflicted)
    }

    /// Stages every change, as Stage All Changes does, and commits it with the draft message.
    pub fn stage_all_and_commit(&mut self) {
        if !self.can_stage_all_and_commit() {
            return;
        }
        let Some(message) = self.repo().map(|repo| repo.draft.message()) else { return };
        let count = self.snapshot().map_or(0, |s| s.status.len());
        self.act_recording_detail("Commit", "Commit", summary_of(&message), move |client, path| {
            client.stage_all(path)?;
            client.commit(&message, path).map(|_| Some(format!("Staged and committed {count} file{}.", if count == 1 { "" } else { "s" })))
        });
        if self.busy.is_some() {
            if let Some(repo) = self.repo_mut() {
                repo.sent_draft = Some(std::mem::take(&mut repo.draft));
            }
            self.save_draft();
        }
    }

    /// Commits the staged changes and pushes the new commit to the branch's upstream.
    pub fn commit_and_push(&mut self) {
        if !self.can_commit_and_push() {
            return;
        }
        let Some(snapshot) = self.snapshot().cloned() else { return };
        let Some(message) = self.repo().map(|repo| repo.draft.message()) else { return };
        self.act_recording_detail("Commit and push", "Commit", summary_of(&message), move |client, path| {
            client.commit_and_push(&message, &snapshot, path).map(|_| Some("Committed and pushed.".into()))
        });
        if self.busy.is_some() {
            if let Some(repo) = self.repo_mut() {
                repo.sent_draft = Some(std::mem::take(&mut repo.draft));
            }
            self.save_draft();
        }
    }

    /// Whether a commit can be made and pushed: the branch has an upstream and this is not an amend.
    pub fn can_commit_and_push(&self) -> bool {
        self.can_commit() && self.repo().is_some_and(|repo| !repo.amend) && self.snapshot().is_some_and(|s| s.upstream.is_some())
    }

    pub fn can_commit(&self) -> bool {
        let (Some(snapshot), Some(repo)) = (self.snapshot(), self.repo()) else { return false };
        self.idle()
            && !repo.draft.summary.trim().is_empty()
            && !repo.draft_is_untouched_template()
            && (snapshot.staged_count() > 0 || repo.amend)
            && !snapshot.status.iter().any(|e| e.kind == nicegit_core::StatusKind::Conflicted)
    }

    pub fn pull(&mut self) {
        let Some(snapshot) = self.snapshot().cloned() else { return };
        let branch = snapshot.is_on_branch().then(|| snapshot.current_branch.clone());
        self.act_recording_detail("Pull", "Pull", branch, move |client, path| {
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

    /// Undoes the newest recorded step; its inverse becomes the newest redo step.
    pub fn undo(&mut self) {
        self.step_history(false);
    }

    /// Redoes the newest undone step; its inverse becomes the newest undo step.
    pub fn redo(&mut self) {
        self.step_history(true);
    }

    /// Takes the newest step from the undo or redo stack and applies it. The step leaves its stack
    /// only once its action starts, so a refused action leaves both stacks as they were.
    fn step_history(&mut self, redo: bool) {
        if self.busy.is_some() {
            return;
        }
        if self.has_unsaved_editor() {
            self.notify("Save or close the file you are editing before changing the repository.", true);
            return;
        }
        let Some(repo) = self.repo_mut() else { return };
        let entry = if redo { repo.pop_redo() } else { repo.pop_undo() };
        let Some(entry) = entry else { return };
        self.apply_history(entry, redo);
    }

    fn apply_history(&mut self, entry: HistoryEntry, redo: bool) {
        let slot = self.recorded.clone();
        let verb = if redo { "Redo" } else { "Undo" };
        let label = format!("{verb} {}", entry.title.to_lowercase());
        self.act(&label, move |client, path| match client.undo_step(&entry.step, path) {
            Ok(step) => {
                let inverse = HistoryEntry { title: entry.title.clone(), detail: entry.detail.clone(), step };
                // Undoing offers the inverse as a redo; redoing offers it as an undo, and the redo steps not yet
                // applied stay.
                let record = if redo { Recorded::Undo(inverse) } else { Recorded::Redo(inverse) };
                *slot.lock().unwrap_or_else(|p| p.into_inner()) = Some(record);
                Ok(Some(format!("{verb}: {}.", entry.title.to_lowercase())))
            }
            Err(error) => {
                // Older steps in this stack assume the state this one would have produced.
                *slot.lock().unwrap_or_else(|p| p.into_inner()) = Some(Recorded::HistoryFailed { redo });
                Err(error)
            }
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
        let request = self.diff_request();
        let mut requests = Vec::new();
        let mut closed = Vec::new();
        let mut blocked_close = false;
        for (index, tool) in self.tools.iter_mut().enumerate() {
            let mut open = true;
            // Windows stay within the app window, so their buttons are never off-screen.
            let screen = ctx.content_rect();
            // Three widths keep windows consistent: forms, lists, and wide diff views.
            let wanted = tool.default_size();
            let width = if wanted.x <= 600.0 {
                540.0
            } else if wanted.x <= 840.0 {
                720.0
            } else {
                980.0
            };
            let size = egui::vec2(width, wanted.y).min(screen.size() - egui::vec2(40.0, 60.0));
            egui::Window::new(tool.title())
                .id(egui::Id::new(("tool", tool.id())))
                .pivot(egui::Align2::CENTER_CENTER)
                .default_pos(screen.center())
                .default_size(size)
                .max_size(screen.size() - egui::vec2(20.0, 40.0))
                .constrain(true)
                .collapsible(false)
                .resizable(true)
                .open(&mut open)
                .show(ctx, |ui| {
                    let mut cx = Ctx::new(&path, &snapshot, idle, &mut requests);
                    cx.ignore_whitespace = request.ignore_whitespace;
                    cx.diff_context = request.context;
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
            Request::ActRecording { label, title, detail, mode, action } => {
                self.act_recording_mode_detail(&label, &title, detail, mode, action)
            }
            Request::RecordUndo { title, detail, step } => {
                if let Some(repo) = self.repo_mut() {
                    repo.record_step(HistoryEntry { title, detail, step });
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
                // The first clone's parent folder becomes the default; an explicit choice stays.
                if self.settings.clone_folder.is_none() {
                    self.settings.clone_folder = path.parent().map(Path::to_path_buf);
                }
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
        if focused && !self.was_focused {
            self.activation_refresh = true;
        }
        self.was_focused = focused;
        if let Some(watch) = &self.watcher {
            // NiceGit's own actions refresh as they finish; skip changes from before then.
            let last_load = self.repo().map(|r| r.last_load).unwrap_or_else(Instant::now);
            while let Ok(event) = watch.events.try_recv() {
                let Ok(event) = event else { continue };
                if changes_visible_state(&event) && last_load.elapsed() > WATCH_DEBOUNCE {
                    self.watched_change.get_or_insert_with(Instant::now);
                }
            }
        }
        // As in the Mac app, nothing refreshes under an action, a dialog, or a window the user is
        // reviewing.
        let quiet = self.busy.is_none()
            && self.dialog.is_none()
            && !self.tools.iter().any(|tool| tool.holds_refresh())
            && self.repo().is_some_and(|r| !r.loading && r.snapshot.is_some());
        if !quiet {
            return;
        }
        // Returning to the app always refreshes, as in the Mac app; the setting covers changes
        // noticed while NiceGit is in use.
        let regained = std::mem::take(&mut self.activation_refresh);
        let auto = self.settings.auto_refresh;
        let watched = auto && self.watched_change.is_some_and(|at| at.elapsed() >= WATCH_DEBOUNCE);
        let periodic = auto && focused && self.repo().is_some_and(|r| r.last_load.elapsed() >= AUTO_REFRESH);
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

    /// The running bisect, once its status has been read after the last refresh.
    pub fn bisect_status(&self) -> Option<nicegit_core::bisect::BisectStatus> {
        let task = self.repos.get(self.active)?.bisect.as_ref()?;
        task.peek().cloned().flatten()
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
        self.keep_zoom(&ctx);
        self.fit_to_monitor(&ctx);
        self.receive();
        self.receive_templates();
        self.finish_clone();
        self.shortcuts(&ctx);
        self.drop_folders(&ctx);
        self.auto_refresh(&ctx);
        self.auto_fetch(&ctx);
        self.auto_pr_status(&ctx);
        self.debug_open(&ctx);
        self.layout(ui);
        self.tool_windows(&ctx);
        self.dialogs(&ctx);
        self.pull_request_window(&ctx);
        self.palette_window(&ctx);
        self.go_to_file_window(&ctx);
        self.settings_window(&ctx);
        self.shortcuts_window(&ctx);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        self.save_draft();
        self.settings.open = self.repos.iter().map(|r| r.path.clone()).collect();
        self.settings.active = self.active;
        eframe::set_value(storage, STORAGE_KEY, &self.settings);
    }
}

/// Whether a file system event can change what NiceGit shows. Reading a file is not a change:
/// on Linux every open is reported, and reacting to the reads of NiceGit's own refresh would
/// refresh again in a loop. Object writes and lock files come with every Git command; the
/// index, refs, and working files are what change the visible state.
fn changes_visible_state(event: &notify::Event) -> bool {
    use notify::event::{AccessKind, AccessMode, EventKind};
    let changed = match event.kind {
        EventKind::Access(AccessKind::Close(AccessMode::Write)) => true,
        EventKind::Access(_) => false,
        _ => true,
    };
    changed
        && event.paths.iter().any(|path| {
            let text = path.to_string_lossy().replace('\\', "/");
            !text.contains("/.git/objects/") && !text.contains("/.git/logs/") && !text.ends_with(".lock") && !text.contains("/target/")
        })
}

#[cfg(test)]
mod lease_tests {
    use super::lease_was_refused;

    #[test]
    fn a_moved_remote_branch_refuses_the_lease_and_a_plain_refusal_does_not() {
        let stale = " ! [rejected]        main -> main (stale info)\nerror: failed to push some refs to 'origin'";
        assert!(lease_was_refused(stale));
        assert!(!lease_was_refused(" ! [remote rejected] main -> main (pre-receive hook declined)"));
        assert!(!lease_was_refused("fatal: unable to access 'https://x.invalid/': Could not resolve host"));
        assert!(!lease_was_refused("Git could not run `force push`: Set an upstream branch before force pushing."));
    }
}

#[cfg(test)]
mod naming_problem_tests {
    use super::naming_problem;

    #[test]
    fn everyday_naming_mistakes_read_plainly() {
        let says = |error: &str, words: &str| naming_problem(error).is_some_and(|plain| plain.contains(words));
        assert!(says("fatal: 'my branch' is not a valid branch name", "cannot be used"));
        assert!(says("fatal: a branch named 'topic' already exists", "branch with that name"));
        assert!(says("fatal: tag 'v1.0' already exists", "tag with that name"));
        assert!(says("fatal: destination path 'app' already exists and is not an empty directory.", "already has files"));
        assert!(says(" ! [rejected]        v1.0       -> v1.0  (would clobber existing tag)", "would not replace"));
        assert_eq!(naming_problem("fatal: not a git repository"), None);
    }
}

#[cfg(test)]
mod remote_problem_tests {
    use super::remote_problem;

    #[test]
    fn common_remote_failures_read_plainly_and_others_are_left_alone() {
        let says = |error: &str, words: &str| remote_problem(error).is_some_and(|plain| plain.contains(words));
        assert!(says("git@github.com: Permission denied (publickey).\nfatal: Could not read from remote repository.", "SSH key"));
        assert!(says("fatal: Authentication failed for 'https://github.com/a/b.git/'", "sign-in"));
        assert!(says("fatal: could not read Username for 'https://github.com': terminal prompts disabled", "sign-in"));
        assert!(says("fatal: unable to access 'https://x.invalid/': Could not resolve host: x.invalid", "could not be reached"));
        assert!(says("remote: Repository not found.\nfatal: repository 'https://github.com/a/b.git/' not found", "not found"));
        assert_eq!(remote_problem("error: failed to push some refs to 'origin'\nhint: Updates were rejected"), None);
        assert_eq!(remote_problem("fatal: bad object 4038f00d"), None, "a hash containing 403 is not a sign-in failure");
    }
}

#[cfg(test)]
mod tests {
    use notify::event::{AccessKind, AccessMode, CreateKind, DataChange, EventKind, ModifyKind};
    use notify::Event;

    use super::changes_visible_state;

    fn event(kind: EventKind, path: &str) -> Event {
        Event::new(kind).add_path(path.into())
    }

    #[test]
    fn reads_never_trigger_a_refresh_but_writes_do() {
        let file = "/repo/src/main.rs";
        assert!(!changes_visible_state(&event(EventKind::Access(AccessKind::Open(AccessMode::Any)), file)));
        assert!(!changes_visible_state(&event(EventKind::Access(AccessKind::Close(AccessMode::Read)), file)));
        assert!(changes_visible_state(&event(EventKind::Access(AccessKind::Close(AccessMode::Write)), file)));
        assert!(changes_visible_state(&event(EventKind::Modify(ModifyKind::Data(DataChange::Content)), file)));
        assert!(changes_visible_state(&event(EventKind::Create(CreateKind::File), "/repo/.git/index")));
    }

    #[test]
    fn git_internals_that_change_with_every_command_are_ignored() {
        let write = EventKind::Modify(ModifyKind::Data(DataChange::Content));
        for path in ["/repo/.git/objects/ab/cdef", "/repo/.git/logs/HEAD", "/repo/.git/index.lock", "/repo/target/debug/app"] {
            assert!(!changes_visible_state(&event(write, path)), "{path}");
        }
        assert!(changes_visible_state(&event(write, "/repo/.git/refs/heads/main")));
    }
}

#[cfg(test)]
mod history_tests {
    use std::path::PathBuf;

    use nicegit_core::undo::{UndoMode, UndoStep};

    use super::{HistoryEntry, Repo, MAX_UNDO_STEPS};

    fn step(title: &str) -> HistoryEntry {
        let step = UndoStep::BranchMove { branch: "main".into(), before: "a".into(), after: "b".into(), mode: UndoMode::Soft };
        HistoryEntry { title: title.into(), detail: None, step }
    }

    fn titles(stack: &[HistoryEntry]) -> Vec<&str> {
        stack.iter().map(|entry| entry.title.as_str()).collect()
    }

    #[test]
    fn undo_steps_past_the_cap_drop_the_oldest() {
        let mut repo = Repo::new(PathBuf::new());
        for n in 0..MAX_UNDO_STEPS + 5 {
            repo.record_step(step(&format!("Step {n}")));
        }
        assert_eq!(repo.undo.len(), 20);
        assert_eq!(repo.undo.first().map(|e| e.title.as_str()), Some("Step 5"));
        assert_eq!(repo.undo.last().map(|e| e.title.as_str()), Some("Step 24"));
    }

    #[test]
    fn history_new_action_clears_redo_steps() {
        let mut repo = Repo::new(PathBuf::new());
        repo.record_step(step("A"));
        repo.record_step(step("B"));
        let undone = repo.pop_undo().expect("B");
        repo.push_redo(undone);
        assert_eq!(titles(&repo.redo), ["B"]);

        repo.record_step(step("C"));
        assert!(repo.redo.is_empty(), "a new action ends the redo chain");
        assert_eq!(titles(&repo.undo), ["A", "C"]);
    }

    #[test]
    fn history_undo_then_redo_then_undo_again() {
        let mut repo = Repo::new(PathBuf::new());
        repo.record_step(step("A"));
        repo.record_step(step("B"));

        let undone = repo.pop_undo().expect("B");
        repo.push_redo(undone);
        assert_eq!(titles(&repo.undo), ["A"]);

        let redone = repo.pop_redo().expect("B again");
        repo.push_undo_keep_redo(redone);
        assert_eq!(titles(&repo.undo), ["A", "B"]);
        assert!(repo.redo.is_empty());

        let undone = repo.pop_undo().expect("B once more");
        repo.push_redo(undone);
        assert_eq!(titles(&repo.undo), ["A"]);
        assert_eq!(titles(&repo.redo), ["B"]);
    }

    #[test]
    fn history_redo_keeps_the_remaining_redo_steps() {
        let mut repo = Repo::new(PathBuf::new());
        for title in ["A", "B", "C"] {
            repo.record_step(step(title));
        }
        for _ in 0..3 {
            let undone = repo.pop_undo().expect("a step to undo");
            repo.push_redo(undone);
        }
        assert!(repo.undo.is_empty());
        // The newest undone step is on top, so Redo starts with the one undone last.
        assert_eq!(titles(&repo.redo), ["C", "B", "A"]);

        let redone = repo.pop_redo().expect("the newest undone step");
        repo.push_undo_keep_redo(redone);
        assert_eq!(titles(&repo.undo), ["A"]);
        assert_eq!(titles(&repo.redo), ["C", "B"], "the steps not yet redone stay");
    }

    #[test]
    fn history_failure_clears_the_rest_of_its_stack() {
        let mut repo = Repo::new(PathBuf::new());
        for title in ["A", "B", "C"] {
            repo.record_step(step(title));
        }
        repo.pop_undo();
        repo.push_redo(step("C'"));
        assert!(repo.clear_on_failure(false), "older undo steps were dropped");
        assert!(repo.undo.is_empty());
        assert_eq!(titles(&repo.redo), ["C'"], "the redo stack is left alone by an undo failure");

        assert!(repo.clear_on_failure(true), "older redo steps were dropped");
        assert!(repo.redo.is_empty());
        assert!(!repo.clear_on_failure(true), "an empty stack has nothing to clear");
    }
}
