//! "From GitHub…" in the Clone dialog: a list of the repositories the signed-in GitHub CLI user can
//! see on github.com or one of the GitHub Enterprise hosts listed in Settings. Choosing one fills in
//! the dialog's address. Nothing is cloned here; the dialog's OK still does that.

use std::path::{Path, PathBuf};

use egui::RichText;
use egui_phosphor::regular as icon;
use nicegit_core::github::{Cancel, GhStatus, GitHubRepoListing, CANCELLED};
use nicegit_core::{GitClient, GitError};

use crate::theme;
use crate::tools::{widgets, Task};

/// Repositories requested from `gh`. The picker says so when the list reaches this many.
const LIMIT: usize = 500;

/// What `gh` answered for one host. A missing or signed-out `gh` is an answer, not an error.
#[derive(Clone)]
enum Loaded {
    NotInstalled,
    SignedOut(String),
    Repositories(Vec<GitHubRepoListing>),
}

#[derive(Default)]
enum Listing {
    #[default]
    Idle,
    Loading(Task<Result<Loaded, String>>),
    Ready(Result<Loaded, String>),
}

/// What the person did with the picker in one frame.
pub enum Picked {
    /// The address to put in the Clone dialog's URL field.
    Url(String),
    /// The picker was closed without a choice.
    Close,
}

pub struct ClonePicker {
    /// github.com, then each GitHub Enterprise host from Settings, as they were when the picker opened.
    hosts: Vec<String>,
    host: String,
    filter: String,
    use_ssh: bool,
    listing: Listing,
    /// Ends the `gh` call still in flight for this picker. Set when the picker is dropped.
    cancel: Cancel,
}

impl Drop for ClonePicker {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

impl ClonePicker {
    /// A picker for github.com, with the Enterprise hosts that Settings lists. Nothing is read
    /// until the picker is first shown.
    pub fn new(enterprise_hosts: &[String]) -> Self {
        let mut hosts = vec!["github.com".to_string()];
        hosts.extend(enterprise_hosts.iter().cloned());
        Self { hosts, host: "github.com".into(), filter: String::new(), use_ssh: false, listing: Listing::Idle, cancel: Cancel::default() }
    }

    /// Reads the repositories for the chosen host in the background. A request still in flight
    /// is cancelled first.
    fn start(&mut self, ctx: &egui::Context) {
        self.cancel.cancel();
        let cancel = Cancel::default();
        self.cancel = cancel.clone();
        let host = self.host.clone();
        let directory = start_directory();
        self.listing = Listing::Loading(Task::spawn(ctx, move || load(&host, &cancel, &directory)));
    }

    /// Moves a finished read into place.
    fn settle(&mut self) {
        if let Listing::Loading(task) = &mut self.listing {
            if let Some(result) = task.get() {
                let result = result.clone();
                self.listing = Listing::Ready(result);
            }
        }
    }

    /// Draws the picker above the Clone dialog, and reports what was chosen or closed this frame.
    pub fn show(&mut self, ctx: &egui::Context) -> Option<Picked> {
        self.settle();
        if matches!(self.listing, Listing::Idle) {
            self.start(ctx);
        }
        let hosts = self.hosts.clone();
        let mut host = self.host.clone();
        let mut picked = None;
        let mut check_again = false;
        let modal = egui::Modal::new(egui::Id::new("clone-picker")).show(ctx, |ui| {
            ui.set_width(440.0);
            let c = theme::of(ui);
            ui.heading("Choose a GitHub repository");
            ui.add_space(8.0);
            if hosts.len() > 1 {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Host").color(c.muted));
                    egui::ComboBox::from_id_salt("clone-picker-host").selected_text(host.as_str()).show_ui(ui, |ui| {
                        for name in &hosts {
                            ui.selectable_value(&mut host, name.clone(), name.as_str());
                        }
                    });
                });
                ui.add_space(4.0);
            }
            ui.horizontal(|ui| {
                widgets::checkbox(ui, true, &mut self.use_ssh, "Use SSH");
            });
            ui.add_space(4.0);
            widgets::search_field(ui, &mut self.filter, "Filter repositories")
                .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Filter repositories"));
            ui.add_space(8.0);

            match &self.listing {
                Listing::Idle | Listing::Loading(_) => widgets::loading(ui, "Loading your repositories from GitHub"),
                Listing::Ready(Err(message)) => {
                    widgets::error(ui, message);
                    ui.add_space(6.0);
                    check_again = ui.button(format!("{}  Check again", icon::ARROW_CLOCKWISE)).clicked();
                }
                Listing::Ready(Ok(Loaded::NotInstalled)) => {
                    widgets::callout(
                        ui,
                        "The GitHub CLI (gh) is not installed. Install it from cli.github.com, then sign in with gh auth login and check again.",
                        true,
                    );
                    ui.add_space(6.0);
                    check_again = ui.button(format!("{}  Check again", icon::ARROW_CLOCKWISE)).clicked();
                }
                Listing::Ready(Ok(Loaded::SignedOut(explanation))) => {
                    let command = sign_in_command(&self.host);
                    widgets::callout(ui, &format!("GitHub CLI is not signed in to {}. Run {command} in a terminal, then check again.", self.host), true);
                    if !explanation.is_empty() {
                        ui.add_space(4.0);
                        ui.label(RichText::new(explanation.as_str()).small().color(c.muted));
                    }
                    ui.add_space(6.0);
                    check_again = ui.button(format!("{}  Check again", icon::ARROW_CLOCKWISE)).clicked();
                }
                Listing::Ready(Ok(Loaded::Repositories(repositories))) => {
                    if let Some(url) = repository_list(ui, repositories, &self.filter, self.use_ssh) {
                        picked = Some(Picked::Url(url));
                    }
                }
            }

            ui.add_space(12.0);
            if ui.button("Cancel").clicked() {
                picked = Some(Picked::Close);
            }
        });
        if picked.is_none() && modal.should_close() {
            picked = Some(Picked::Close);
        }
        if check_again {
            self.start(ctx);
        } else if host != self.host {
            self.host = host;
            self.start(ctx);
        }
        picked
    }
}

/// The rows of the loaded list, filtered by the person's text. Returns the chosen address when a
/// row is clicked.
fn repository_list(ui: &mut egui::Ui, repositories: &[GitHubRepoListing], filter: &str, use_ssh: bool) -> Option<String> {
    let c = theme::of(ui);
    if repositories.is_empty() {
        widgets::empty_state(ui, icon::CHECK_CIRCLE, "No repositories on this host");
        return None;
    }
    let query = filter.trim().to_lowercase();
    let visible: Vec<&GitHubRepoListing> = repositories
        .iter()
        .filter(|repo| {
            query.is_empty() || repo.name_with_owner.to_lowercase().contains(&query) || repo.description.to_lowercase().contains(&query)
        })
        .collect();
    if repositories.len() >= LIMIT {
        ui.label(RichText::new(format!("Showing the first {LIMIT} repositories. Filter to narrow them.")).small().color(c.muted));
    }
    if visible.is_empty() {
        ui.label(RichText::new("No repositories match the filter").color(c.muted));
        return None;
    }
    let mut picked = None;
    egui::ScrollArea::vertical().max_height(320.0).auto_shrink([false, false]).show(ui, |ui| {
        for repo in visible {
            let clicked = ui
                .vertical(|ui| {
                    let clicked = ui
                        .horizontal(|ui| {
                            // Both icon slots are always reserved, so the names line up on every row.
                            let lock = repo.is_private.then_some((icon::LOCK_SIMPLE, "Private"));
                            let fork = repo.is_fork.then_some((icon::GIT_FORK, "Fork"));
                            for mark in [lock, fork] {
                                icon_slot(ui, mark, c.muted);
                            }
                            ui.add(egui::Button::new(RichText::new(&repo.name_with_owner).strong()).frame(false)).clicked()
                        })
                        .inner;
                    if !repo.description.is_empty() {
                        ui.label(RichText::new(&repo.description).small().color(c.muted));
                    }
                    clicked
                })
                .inner;
            ui.add_space(4.0);
            if clicked {
                let ssh = !repo.ssh_url.is_empty();
                picked = Some(if use_ssh && ssh { repo.ssh_url.clone() } else { repo.url.clone() });
            }
        }
    });
    picked
}

/// A fixed-width slot for one row's icon, left empty when the row does not have that icon.
fn icon_slot(ui: &mut egui::Ui, mark: Option<(&str, &str)>, color: egui::Color32) {
    let size = egui::vec2(16.0, ui.spacing().interact_size.y);
    ui.allocate_ui_with_layout(size, egui::Layout::left_to_right(egui::Align::Center), |ui| {
        if let Some((glyph, hover)) = mark {
            ui.label(RichText::new(glyph).color(color)).on_hover_text(hover);
        }
    });
}

/// The command that signs `gh` in to `host`.
fn sign_in_command(host: &str) -> String {
    if host == "github.com" {
        "gh auth login".to_string()
    } else {
        format!("gh auth login --hostname {host}")
    }
}

/// Where `gh` runs. No repository exists yet, so the home folder is used when there is one.
fn start_directory() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .filter(|path| path.is_dir())
        .unwrap_or_else(std::env::temp_dir)
}

/// Asks `gh` whether it is signed in to `host`, then lists the repositories if it is.
fn load(host: &str, cancel: &Cancel, directory: &Path) -> Result<Loaded, String> {
    let client = GitClient::new();
    let status = client.gh_status(host, directory);
    if cancel.is_cancelled() {
        return Err(CANCELLED.to_string());
    }
    match status {
        GhStatus::NotInstalled => Ok(Loaded::NotInstalled),
        GhStatus::SignedOut(explanation) => Ok(Loaded::SignedOut(explanation)),
        GhStatus::SignedIn => {
            client.github_repositories(host, LIMIT, cancel, directory).map(Loaded::Repositories).map_err(|error| message(&error))
        }
    }
}

fn message(error: &GitError) -> String {
    match error {
        GitError::CommandFailed { message, .. } => message.clone(),
        other => other.to_string(),
    }
}
