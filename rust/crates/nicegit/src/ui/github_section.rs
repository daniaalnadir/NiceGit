//! Pull requests and issues listed in the sidebar, loaded on demand from a chosen github.com
//! remote through the signed-in GitHub CLI, as in the Mac app.

use egui::{RichText, Ui};
use egui_phosphor::regular as icon;
use nicegit_core::github::{Cancel, GitHubItem, ItemKind, ItemState, CANCELLED};
use nicegit_core::{GitClient, Snapshot};

use crate::app::NiceGitApp;
use crate::theme;
use crate::tools::{self, widgets, Task};

/// Items load in pages of this size, up to `MAXIMUM`.
const PAGE: usize = 100;
const MAXIMUM: usize = 1000;

pub struct GitHubList {
    pub remote: String,
    pub limit: usize,
    pub items: Vec<GitHubItem>,
    pub error: Option<String>,
    cancel: Cancel,
    task: Option<Task<nicegit_core::Result<Vec<GitHubItem>>>>,
}

impl GitHubList {
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
        let task = Task::spawn(&self.worker.context, move || {
            let client = GitClient::new();
            let repository = client.github_repository(&remote_name, &path)?;
            client.github_items_cancellable(&repository, kind, ItemState::Open, limit, &token, &path)
        });
        let items = repo.github[slot(kind)].take().filter(|list| list.remote == remote).map(|list| list.items).unwrap_or_default();
        repo.github[slot(kind)] = Some(GitHubList { remote, limit, items, error: None, cancel, task: Some(task) });
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
        crate::ui::sidebar::section(ui, id, glyph, title, count, false, |ui| {
            if snapshot.remotes.is_empty() {
                ui.label(RichText::new("   Add a github.com remote to load these").small().color(c.muted));
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
                ui.label(RichText::new(format!("{}  {}", icon::GITHUB_LOGO, list.remote)).small().color(c.muted));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let open_window = widgets::icon_button(ui, icon::ARROW_SQUARE_OUT, "Open with filters and closed items", true);
                    open_window.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Open GitHub window"));
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
        if let Some(remote) = window {
            let tool = match kind {
                ItemKind::PullRequest => tools::github::GitHubWindow::new(remote),
                ItemKind::Issue => tools::github::GitHubWindow::issues(remote),
            };
            self.open_tool(Box::new(tool));
        }
    }
}
