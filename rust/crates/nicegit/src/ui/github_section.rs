//! Pull requests and issues listed in the sidebar, loaded on demand from a chosen GitHub remote
//! (github.com or a GitHub Enterprise host) through the signed-in GitHub CLI, as in the Mac app.

use egui::{RichText, Ui};
use egui_phosphor::regular as icon;
use nicegit_core::github::{Cancel, GitHubItem, ItemKind, ItemState, CANCELLED};
use nicegit_core::{GitClient, Snapshot};

use std::path::PathBuf;

use crate::app::NiceGitApp;
use crate::theme;
use crate::tools::{self, widgets, CheckOutRequest, Task};

/// Items load in pages of this size, up to `MAXIMUM`.
const PAGE: usize = 100;
const MAXIMUM: usize = 1000;

/// A Check out from the Launchpad whose repository is still loading. The dialog opens when the
/// repository's snapshot arrives.
pub struct PendingCheckOut {
    /// The repository's path as the app stores it, which is what the tab is compared with.
    pub path: PathBuf,
    pub request: CheckOutRequest,
}

pub struct GitHubList {
    pub remote: String,
    pub limit: usize,
    pub items: Vec<GitHubItem>,
    pub error: Option<String>,
    cancel: Cancel,
    task: Option<Task<nicegit_core::Result<Vec<GitHubItem>>>>,
}

impl GitHubList {
    /// A list that is already loaded with `items`, for tests that do not reach GitHub.
    #[cfg(test)]
    pub(crate) fn loaded(remote: &str, items: Vec<GitHubItem>) -> Self {
        GitHubList { remote: remote.to_string(), limit: PAGE, items, error: None, cancel: Cancel::default(), task: None }
    }

    fn is_loading(&mut self) -> bool {
        self.task.as_mut().is_some_and(|task| task.is_pending())
    }

    /// Takes a finished request's result into the list.
    fn receive(&mut self) {
        let Some(task) = self.task.as_mut() else { return };
        let Some(result) = task.get() else { return };
        match result {
            Ok(items) => {
                self.items = items.clone();
                self.error = None;
            }
            Err(error) => {
                let text = error.to_string();
                self.error = (!text.contains(CANCELLED)).then_some(text);
            }
        }
        self.task = None;
    }
}

fn slot(kind: ItemKind) -> usize {
    match kind {
        ItemKind::PullRequest => 0,
        ItemKind::Issue => 1,
    }
}

impl NiceGitApp {
    fn load_github(&mut self, kind: ItemKind, remote: String, limit: usize) {
        let Some(repo) = self.repos.get_mut(self.active) else { return };
        if let Some(previous) = repo.github[slot(kind)].as_ref() {
            previous.cancel.cancel();
        }
        let cancel = Cancel::default();
        let (path, token, remote_name) = (repo.path.clone(), cancel.clone(), remote.clone());
        let hosts = self.settings.github_enterprise_hosts.clone();
        let task = Task::spawn(&self.worker.context, move || {
            let client = GitClient::new();
            let repository = client.github_repository(&remote_name, &path, &hosts)?;
            client.github_items_cancellable(&repository, kind, ItemState::Open, limit, &token, &path)
        });
        let items = repo.github[slot(kind)].take().filter(|list| list.remote == remote).map(|list| list.items).unwrap_or_default();
        repo.github[slot(kind)] = Some(GitHubList { remote, limit, items, error: None, cancel, task: Some(task) });
    }

    /// Opens the Check out pull request dialog for pull request `number` on `remote`. Nothing runs
    /// until the dialog's button is pressed, and that action refuses if the checkout or remote has
    /// changed.
    fn open_check_out_pull_request(&mut self, snapshot: &Snapshot, remote: String, number: u64, title: String, author: String) {
        if self.busy.is_some() {
            self.notify("Wait for the current Git action to finish before checking out a pull request.", true);
            return;
        }
        if snapshot.operation.is_some() {
            self.notify("Finish or abort the current Git operation before checking out a pull request.", true);
            return;
        }
        self.dialog = Some(crate::ui::dialogs::Dialog::check_out_pull_request(remote, number, title, author, snapshot));
    }

    /// Handles a Check out from the Launchpad. Its repository is shown first, because the dialog
    /// reads the checkout it would change; when that repository's snapshot has not loaded yet, the
    /// dialog waits for it in [`Self::finish_pending_check_out`].
    pub fn check_out_from_launchpad(&mut self, request: CheckOutRequest) {
        if self.busy.is_some() {
            self.notify("Wait for the current Git action to finish before checking out a pull request.", true);
            return;
        }
        // The path is compared as `open` stores it, so a folder named by a workspace matches its tab.
        let path = crate::app::strip_verbatim(request.path.canonicalize().unwrap_or_else(|_| request.path.clone()));
        let loaded = self.repos.get(self.active).filter(|repo| repo.path == path).and_then(|repo| repo.snapshot.clone());
        if let Some(snapshot) = loaded {
            self.open_check_out_pull_request(&snapshot, request.remote, request.number, request.title, request.author);
            return;
        }
        self.pending_check_out = Some(PendingCheckOut { path: path.clone(), request });
        self.open(path.clone());
        // When the repository did not open, as when an editor has unsaved text, nothing waits.
        if !self.repos.get(self.active).is_some_and(|repo| repo.path == path) {
            self.pending_check_out = None;
        }
    }

    /// Opens a waiting Check out dialog once its repository is shown and its snapshot has loaded.
    /// A waiting request is dropped when the user shows another repository.
    pub fn finish_pending_check_out(&mut self) {
        let Some(pending) = self.pending_check_out.as_ref() else { return };
        let Some(repo) = self.repos.get(self.active) else { return };
        if repo.path != pending.path {
            self.pending_check_out = None;
            return;
        }
        let Some(snapshot) = repo.snapshot.clone() else { return };
        let Some(pending) = self.pending_check_out.take() else { return };
        let request = pending.request;
        self.open_check_out_pull_request(&snapshot, request.remote, request.number, request.title, request.author);
    }

    /// Opens the Start work dialog for issue `item` on `remote`. The remote's default branch is read
    /// in the background as the dialog opens; nothing changes until the dialog's button is pressed.
    fn open_start_issue_work(&mut self, snapshot: &Snapshot, remote: String, item: GitHubItem) {
        if self.busy.is_some() {
            self.notify("Wait for the current Git action to finish before starting work on an issue.", true);
            return;
        }
        if snapshot.operation.is_some() {
            self.notify("Finish or abort the current Git operation before starting work on an issue.", true);
            return;
        }
        if snapshot.head_hash.is_none() {
            self.notify("Make a first commit before starting work on an issue, so there is a HEAD to start from.", true);
            return;
        }
        let Some(path) = self.repo().map(|repo| repo.path.clone()) else { return };
        let hosts = self.settings.github_enterprise_hosts.clone();
        let (remote_name, directory) = (remote.clone(), path);
        let default = Task::spawn(&self.worker.context, move || {
            GitClient::new().remote_default_branch(&remote_name, &hosts, &directory).map_err(|error| error.to_string())
        });
        self.dialog = Some(crate::ui::dialogs::Dialog::start_issue_work(remote, item.number, item.title, snapshot, default));
    }

    pub fn github_section(&mut self, ui: &mut Ui, snapshot: &Snapshot, kind: ItemKind) {
        let c = theme::of(ui);
        let (id, glyph, title) = match kind {
            ItemKind::PullRequest => ("pulls", icon::GIT_PULL_REQUEST, "Pull requests"),
            ItemKind::Issue => ("issues", icon::CIRCLE_DASHED, "Issues"),
        };
        if let Some(list) = self.repos.get_mut(self.active).and_then(|r| r.github[slot(kind)].as_mut()) {
            list.receive();
        }
        let count = self.repo().and_then(|r| r.github[slot(kind)].as_ref()).map(|l| l.items.len()).unwrap_or(0);
        let mut load: Option<(String, usize)> = None;
        let mut cancel = false;
        let mut window: Option<String> = None;
        // A pull request's "Check out locally…" asks for the dialog, which opens once the list is drawn.
        let mut check_out: Option<(String, GitHubItem)> = None;
        // An issue's "Start work…" asks for its dialog the same way.
        let mut start_work: Option<(String, GitHubItem)> = None;
        let hosts = self.settings.github_enterprise_hosts.clone();
        crate::ui::sidebar::section(ui, id, glyph, title, count, false, |ui| {
            if snapshot.remotes.is_empty() {
                ui.label(RichText::new("   Add a GitHub remote to load these").small().color(c.muted));
                return;
            }
            let repo = &mut self.repos[self.active];
            let Some(list) = repo.github[slot(kind)].as_mut() else {
                ui.horizontal(|ui| {
                    ui.add_space(14.0);
                    ui.menu_button(format!("{}  Load from GitHub {}", icon::GITHUB_LOGO, icon::CARET_DOWN), |ui| {
                        for remote in &snapshot.remotes {
                            if ui.button(remote).clicked() {
                                ui.close();
                                load = Some((remote.clone(), PAGE));
                            }
                        }
                    });
                });
                return;
            };
            let loading = list.is_loading();
            ui.horizontal(|ui| {
                ui.add_space(14.0);
                // A GitHub Enterprise remote names its host, so the list says which server it reads.
                let header = match crate::git_ext::github_repository_for_remote(snapshot, &list.remote, &hosts) {
                    Some(repository) if repository.host != "github.com" => {
                        format!("{}  {} · {}", icon::GITHUB_LOGO, list.remote, repository.host)
                    }
                    _ => format!("{}  {}", icon::GITHUB_LOGO, list.remote),
                };
                ui.label(RichText::new(header).small().color(c.muted));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let open_window = widgets::icon_button(ui, icon::ARROW_SQUARE_OUT, "Open with filters and closed items", true);
                    open_window.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "Open GitHub window"));
                    if open_window.clicked() {
                        window = Some(list.remote.clone());
                    }
                    if loading {
                        if ui.small_button("Cancel").clicked() {
                            cancel = true;
                        }
                        ui.spinner();
                    } else if widgets::icon_button(ui, icon::ARROWS_CLOCKWISE, "Refresh", true).clicked() {
                        load = Some((list.remote.clone(), list.limit));
                    }
                });
            });
            if let Some(error) = &list.error {
                ui.horizontal(|ui| {
                    ui.add_space(14.0);
                    ui.add(egui::Label::new(RichText::new(error).small().color(c.danger)).wrap());
                });
            }
            if list.items.is_empty() && !loading && list.error.is_none() {
                ui.label(RichText::new(format!("   No open {}", title.to_lowercase())).small().color(c.muted));
            }
            for item in &list.items {
                let text = format!("#{}  {}", item.number, item.title);
                let response = ui
                    .horizontal(|ui| {
                        ui.add_space(14.0);
                        ui.add(egui::Button::new(RichText::new(text)).frame(false).truncate())
                    })
                    .inner
                    .on_hover_text(format!("{}\n{}", item.author.clone().unwrap_or_default(), item.url));
                if response.clicked() {
                    ui.ctx().open_url(egui::OpenUrl::new_tab(&item.url));
                }
                response.context_menu(|ui| {
                    if ui.button(format!("{}  Open in browser", icon::ARROW_SQUARE_OUT)).clicked() {
                        ui.close();
                        ui.ctx().open_url(egui::OpenUrl::new_tab(&item.url));
                    }
                    if ui.button(format!("{}  Copy link", icon::COPY)).clicked() {
                        ui.close();
                        ui.ctx().copy_text(item.url.clone());
                    }
                    if kind == ItemKind::PullRequest && ui.button(format!("{}  Check out locally…", icon::GIT_BRANCH)).clicked() {
                        ui.close();
                        check_out = Some((list.remote.clone(), item.clone()));
                    }
                    if kind == ItemKind::Issue && ui.button(format!("{}  Start work…", icon::GIT_BRANCH)).clicked() {
                        ui.close();
                        start_work = Some((list.remote.clone(), item.clone()));
                    }
                });
            }
            if !loading && list.items.len() >= list.limit && list.limit < MAXIMUM {
                ui.horizontal(|ui| {
                    ui.add_space(14.0);
                    if ui.small_button(format!("Load {} more", PAGE)).clicked() {
                        load = Some((list.remote.clone(), (list.limit + PAGE).min(MAXIMUM)));
                    }
                });
            }
        });
        if cancel {
            if let Some(list) = self.repos.get_mut(self.active).and_then(|r| r.github[slot(kind)].as_mut()) {
                list.cancel.cancel();
            }
        }
        if let Some((remote, limit)) = load {
            self.load_github(kind, remote, limit);
        }
        if let Some((remote, item)) = check_out {
            let author = item.author.unwrap_or_default();
            self.open_check_out_pull_request(snapshot, remote, item.number, item.title, author);
        }
        if let Some((remote, item)) = start_work {
            self.open_start_issue_work(snapshot, remote, item);
        }
        if let Some(remote) = window {
            let tool = match kind {
                ItemKind::PullRequest => tools::github::GitHubWindow::new(remote, hosts),
                ItemKind::Issue => tools::github::GitHubWindow::issues(remote, hosts),
            };
            self.open_tool(Box::new(tool));
        }
    }
}
