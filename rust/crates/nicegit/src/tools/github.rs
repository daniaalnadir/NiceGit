#![allow(dead_code)]
//! Pull requests and issues for a GitHub remote, read through the GitHub CLI (`gh`). NiceGit
//! never stores a token; `gh` keeps its own sign-in.

use egui::{Color32, RichText, Ui};
use egui_phosphor::regular as icon;
use nicegit_core::github::{GhStatus, GitHubItem, ItemKind, ItemState};
use nicegit_core::GitError;

use crate::theme;
use crate::tools::{query, widgets, Ctx, Task, ToolWindow};

/// Items requested per page, and the most `gh` will list for one view.
const PAGE: usize = 100;
const MAX_ITEMS: usize = 1000;

/// What a loaded list was requested for. A different key means the list must be reloaded.
#[derive(Clone, Copy, PartialEq, Eq)]
struct ListKey {
    kind: ItemKind,
    state: ItemState,
    limit: usize,
    generation: u64,
}

pub struct GitHubWindow {
    /// The remote whose GitHub repository is shown.
    remote: String,
    kind: ItemKind,
    state: ItemState,
    limit: usize,
    filter: String,
    /// Bumped by Refresh so the list is read again.
    generation: u64,
    gh: Refreshing<(), GhStatus>,
    items: Refreshing<ListKey, Vec<GitHubItem>>,
    /// Whether `gh` must be checked again, after Refresh or the first open.
    check_gh: bool,
}

impl GitHubWindow {
    /// A window for the GitHub repository behind `remote`.
    pub fn new(remote: String) -> Self {
        Self {
            remote,
            kind: ItemKind::PullRequest,
            state: ItemState::Open,
            limit: PAGE,
            filter: String::new(),
            generation: 0,
            gh: Refreshing::new(),
            items: Refreshing::new(),
            check_gh: true,
        }
    }

    fn key(&self) -> ListKey {
        ListKey { kind: self.kind, state: self.state, limit: self.limit, generation: self.generation }
    }

    /// The states offered for the current kind: merged applies to pull requests only.
    fn states(&self) -> &'static [ItemState] {
        match self.kind {
            ItemKind::PullRequest => &[ItemState::Open, ItemState::Closed, ItemState::Merged, ItemState::All],
            ItemKind::Issue => &[ItemState::Open, ItemState::Closed, ItemState::All],
        }
    }
}

impl ToolWindow for GitHubWindow {
    fn id(&self) -> String {
        format!("github:{}", self.remote)
    }

    fn title(&self) -> String {
        "GitHub".to_string()
    }

    fn default_size(&self) -> egui::Vec2 {
        egui::vec2(660.0, 560.0)
    }

    fn ui(&mut self, ui: &mut Ui, cx: &mut Ctx) {
        let ctx = ui.ctx().clone();
        if self.check_gh {
            self.check_gh = false;
            let task = query(&ctx, cx.repo, |client, directory| Ok(client.gh_status(directory)));
            self.gh.start((), task);
        }
        self.gh.settle();
        let signed_in = matches!(self.gh.value(&()), Some(Ok(GhStatus::SignedIn)));

        // Start reading the list once `gh` is known to be signed in, and again whenever the
        // kind, state, page size, or refresh count changes.
        let key = self.key();
        if signed_in && !self.remote.is_empty() && !self.items.shown_or_pending_key(&key) {
            let remote = self.remote.clone();
            let task = query(&ctx, cx.repo, move |client, directory| {
                let repository = client.github_repository(&remote, directory)?;
                client.github_items(&repository, key.kind, key.state, key.limit, directory)
            });
            self.items.start(key, task);
        }
        self.items.settle();

        self.toolbar(ui);
        ui.add_space(6.0);

        if self.remote.is_empty() {
            widgets::callout(ui, "Choose a remote to see its GitHub pull requests and issues.", false);
            return;
        }
        match self.gh.value(&()) {
            None => {
                widgets::loading(ui, "Checking the GitHub CLI");
                return;
            }
            Some(Err(error)) => {
                widgets::callout(ui, &message(error), true);
                return;
            }
            Some(Ok(GhStatus::NotInstalled)) => {
                self.install_guidance(ui);
                return;
            }
            Some(Ok(GhStatus::SignedOut(explanation))) => {
                let explanation = explanation.clone();
                self.sign_in_guidance(ui, &explanation);
                return;
            }
            Some(Ok(GhStatus::SignedIn)) => {}
        }
        self.list(ui);
    }
}

impl GitHubWindow {
    fn toolbar(&mut self, ui: &mut Ui) {
        let c = theme::of(ui);
        ui.horizontal(|ui| {
            ui.label(RichText::new(icon::GITHUB_LOGO).size(16.0).color(c.muted));
            ui.label(RichText::new(heading_for_remote(&self.remote)).strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let refresh = ui.add(egui::Button::new(icon::ARROW_CLOCKWISE)).on_hover_text("Read GitHub again");
                if refresh.clicked() {
                    self.check_gh = true;
                    self.generation += 1;
                }
            });
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            for kind in [ItemKind::PullRequest, ItemKind::Issue] {
                let selected = self.kind == kind;
                let glyph = if kind == ItemKind::PullRequest { icon::GIT_PULL_REQUEST } else { icon::CIRCLE_DASHED };
                if ui.selectable_label(selected, format!("{glyph}  {}", kind.title())).clicked() && !selected {
                    self.kind = kind;
                    if !self.states().contains(&self.state) {
                        self.state = ItemState::Open;
                    }
                    self.limit = PAGE;
                }
            }
        });
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            for state in self.states().to_vec() {
                let selected = self.state == state;
                if ui.selectable_label(selected, state.title()).clicked() && !selected {
                    self.state = state;
                    self.limit = PAGE;
                }
            }
        });
        ui.add_space(4.0);
        widgets::search_field(ui, &mut self.filter, "Filter loaded items by title, author, or number");
    }

    fn install_guidance(&mut self, ui: &mut Ui) {
        widgets::callout(
            ui,
            "The GitHub CLI (gh) is not installed. Install it from cli.github.com, then sign in with gh auth login and check again.",
            true,
        );
        ui.add_space(6.0);
        if ui.button(format!("{}  Check again", icon::ARROW_CLOCKWISE)).clicked() {
            self.check_gh = true;
        }
    }

    fn sign_in_guidance(&mut self, ui: &mut Ui, explanation: &str) {
        widgets::callout(ui, "GitHub CLI is not signed in. Run gh auth login in a terminal, then check again.", true);
        if !explanation.is_empty() {
            ui.add_space(4.0);
            let c = theme::of(ui);
            ui.label(RichText::new(explanation).small().color(c.muted));
        }
        ui.add_space(6.0);
        if ui.button(format!("{}  Check again", icon::ARROW_CLOCKWISE)).clicked() {
            self.check_gh = true;
        }
    }

    fn list(&mut self, ui: &mut Ui) {
        let c = theme::of(ui);
        let key = self.key();
        let kind = self.kind;
        let limit = self.limit;
        let mut load_more = false;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| match self.items.value(&key) {
            None => widgets::loading(ui, "Loading from GitHub"),
            Some(Err(error)) => widgets::error(ui, &message(error)),
            Some(Ok(items)) => {
                if items.is_empty() {
                    widgets::empty_state(ui, icon::CHECK_CIRCLE, &format!("No {} {}", self.state.as_str(), kind.title().to_lowercase()));
                    return;
                }
                let visible: Vec<&GitHubItem> = items.iter().filter(|item| item.matches(&self.filter)).collect();
                if visible.is_empty() {
                    ui.label(RichText::new("No matches in the loaded items").color(c.muted));
                }
                for item in visible {
                    item_row(ui, kind, item);
                    ui.add_space(6.0);
                }
                if items.len() >= limit && limit < MAX_ITEMS {
                    if ui.button("Load more").clicked() {
                        load_more = true;
                    }
                } else if items.len() >= MAX_ITEMS {
                    ui.label(RichText::new("Showing the first 1,000 items").small().color(c.muted));
                }
            }
        });
        if load_more {
            self.limit = (self.limit + PAGE).min(MAX_ITEMS);
        }
    }
}

/// One pull request or issue: its title, number, state, author, labels, and a link.
fn item_row(ui: &mut Ui, kind: ItemKind, item: &GitHubItem) {
    let c = theme::of(ui);
    egui::Frame::new()
        .fill(c.card_bg)
        .stroke(egui::Stroke::new(1.0, c.border))
        .corner_radius(6.0)
        .inner_margin(egui::Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let glyph = if kind == ItemKind::PullRequest { icon::GIT_PULL_REQUEST } else { icon::CIRCLE_DASHED };
                ui.label(RichText::new(glyph).color(c.muted));
                ui.add(egui::Label::new(RichText::new(&item.title).strong()).wrap());
            });
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(format!("#{}", item.number)).monospace().color(c.muted));
                if let Some(state) = &item.state {
                    widgets::pill(ui, &state.to_lowercase(), state_color(state, &c));
                }
                if item.is_draft {
                    widgets::pill(ui, "draft", c.muted);
                }
                if let Some(author) = &item.author {
                    ui.label(RichText::new(format!("by {author}")).small().color(c.muted));
                }
                for label in &item.labels {
                    label_pill(ui, &label.name, &label.color);
                }
            });
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.link(format!("{}  Open in browser", icon::ARROW_SQUARE_OUT)).clicked() {
                        ui.ctx().open_url(egui::OpenUrl::new_tab(&item.url));
                    }
                });
            });
        });
}

fn state_color(state: &str, c: &theme::Colors) -> Color32 {
    match state.to_ascii_uppercase().as_str() {
        "OPEN" => c.added,
        "CLOSED" => c.removed,
        "MERGED" => c.renamed,
        _ => c.muted,
    }
}

/// A GitHub label in its own colour, with text that stays readable on it.
fn label_pill(ui: &mut Ui, name: &str, hex: &str) {
    let fill = parse_hex(hex).unwrap_or(Color32::from_rgb(0x8f, 0x99, 0xad));
    let [r, g, b, _] = fill.to_array();
    // Relative luminance decides whether dark or light text reads better on the fill.
    let luminance = 0.2126 * f32::from(r) + 0.7152 * f32::from(g) + 0.0722 * f32::from(b);
    let text = if luminance > 150.0 { Color32::from_rgb(0x1f, 0x23, 0x2b) } else { Color32::WHITE };
    egui::Frame::new()
        .fill(fill)
        .corner_radius(4.0)
        .inner_margin(egui::Margin::symmetric(6, 1))
        .show(ui, |ui| ui.label(RichText::new(name).small().color(text)));
}

fn parse_hex(hex: &str) -> Option<Color32> {
    let hex = hex.trim().trim_start_matches('#');
    if hex.len() != 6 || !hex.is_ascii() {
        return None;
    }
    let value = u32::from_str_radix(hex, 16).ok()?;
    Some(Color32::from_rgb((value >> 16) as u8, (value >> 8) as u8, value as u8))
}

/// The heading for the window, naming the remote whose GitHub repository is shown.
fn heading_for_remote(remote: &str) -> String {
    if remote.is_empty() {
        "GitHub".to_string()
    } else {
        format!("GitHub · {remote}")
    }
}

fn message(error: &GitError) -> String {
    match error {
        GitError::CommandFailed { message, .. } => message.clone(),
        other => other.to_string(),
    }
}

/// The latest answer to a background query. A refresh keeps the previous answer on screen until
/// the new one arrives, so the window does not flicker while it reloads.
struct Refreshing<K, T> {
    shown: Option<(K, Task<nicegit_core::Result<T>>)>,
    pending: Option<(K, Task<nicegit_core::Result<T>>)>,
}

impl<K: PartialEq + Copy, T: Send + 'static> Refreshing<K, T> {
    fn new() -> Self {
        Self { shown: None, pending: None }
    }

    /// Replaces any request still in flight; only the newest answer is kept.
    fn start(&mut self, key: K, task: Task<nicegit_core::Result<T>>) {
        self.pending = Some((key, task));
    }

    /// Whether the answer for `key` is already shown or on its way.
    fn shown_or_pending_key(&self, key: &K) -> bool {
        self.shown.as_ref().is_some_and(|(shown, _)| shown == key) || self.pending.as_ref().is_some_and(|(pending, _)| pending == key)
    }

    /// Moves a finished request into place. Returns true when a new answer arrived.
    fn settle(&mut self) -> bool {
        let finished = self.pending.as_mut().is_some_and(|(_, task)| !task.is_pending());
        if finished {
            self.shown = self.pending.take();
        }
        finished
    }

    /// The answer for `key`, once it has arrived.
    fn value(&mut self, key: &K) -> Option<&nicegit_core::Result<T>> {
        match &mut self.shown {
            Some((shown, task)) if shown == key => task.get(),
            _ => None,
        }
    }
}
