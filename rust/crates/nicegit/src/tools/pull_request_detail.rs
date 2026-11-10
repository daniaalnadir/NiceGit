//! The read-only detail view of one pull request: its header, description, checks, conversation,
//! and files. It is read in the background through the signed-in GitHub CLI (`gh pr view`). Every
//! title, body, comment, and link shown here is display data: nothing runs, and nothing opens
//! unless the person clicks a link.

use egui::{Color32, RichText, Ui};
use egui_phosphor::regular as icon;
use nicegit_core::github::{
    relative_time, Cancel, CheckOutcome, ConversationEntry, GhStatus, GitHubRepository, PrState, PullRequestDetail, PullRequestFile,
    PullRequestReview,
};

use crate::theme;
use crate::tools::github::{label_pill, message, Refreshing};
use crate::tools::{query, widgets, Ctx, ToolWindow};

/// The sections of the detail view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tab {
    Description,
    Checks,
    Conversation,
    Files,
}

/// What the background read found: the detail, or why `gh` could not give it.
pub enum Loaded {
    NotInstalled,
    SignedOut(String),
    Read(Box<PullRequestDetail>),
}

pub struct PullRequestDetailWindow {
    repository: GitHubRepository,
    number: u64,
    tab: Tab,
    /// Bumped by Refresh so the pull request is read again.
    generation: u64,
    detail: Refreshing<u64, Loaded>,
    /// Stops the running read. Refresh replaces it, and closing the window sets it.
    cancel: Cancel,
}

impl PullRequestDetailWindow {
    /// The detail view of pull request `number` on `repository`. Nothing is read until the window is drawn.
    pub fn new(repository: GitHubRepository, number: u64) -> Self {
        Self { repository, number, tab: Tab::Description, generation: 0, detail: Refreshing::new(), cancel: Cancel::default() }
    }

    /// A window that already shows `detail`, for tests that do not reach GitHub.
    #[cfg(test)]
    pub(crate) fn loaded(repository: GitHubRepository, number: u64, detail: PullRequestDetail) -> Self {
        let mut window = Self::new(repository, number);
        window.detail = Refreshing::with_answer(0, Ok(Loaded::Read(Box::new(detail))));
        window
    }
}

impl Drop for PullRequestDetailWindow {
    /// Closing the window ends its read, so `gh` is not left running for a view nobody sees.
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

/// The clicks made in the header in one frame.
#[derive(Default)]
struct HeaderClicks {
    refresh: bool,
    open: bool,
}

impl ToolWindow for PullRequestDetailWindow {
    fn id(&self) -> String {
        format!("pull-request:{}#{}", self.repository.gh_repo(), self.number)
    }

    fn title(&self) -> String {
        format!("Pull request #{}", self.number)
    }

    fn default_size(&self) -> egui::Vec2 {
        egui::vec2(680.0, 600.0)
    }

    fn ui(&mut self, ui: &mut Ui, cx: &mut Ctx) {
        let ctx = ui.ctx().clone();
        let key = self.generation;
        if !self.detail.shown_or_pending_key(&key) {
            // A new read replaces the previous one, which is stopped rather than left running.
            self.cancel.cancel();
            self.cancel = Cancel::default();
            let token = self.cancel.clone();
            let (repository, number) = (self.repository.clone(), self.number);
            let task = query(&ctx, cx.repo, move |client, directory| match client.gh_status(&repository.host, directory) {
                GhStatus::NotInstalled => Ok(Loaded::NotInstalled),
                GhStatus::SignedOut(explanation) => Ok(Loaded::SignedOut(explanation)),
                GhStatus::SignedIn => {
                    client.pull_request_detail(&repository, number, &token, directory).map(|detail| Loaded::Read(Box::new(detail)))
                }
            });
            self.detail.start(key, task);
        }
        self.detail.settle();
        let pending = self.detail.is_pending();
        let now = now_seconds();
        let host = self.repository.host.clone();
        let number = self.number;
        let mut tab = self.tab;
        let mut refresh = false;
        let mut open_link: Option<String> = None;

        // The last answer stays on screen while a refresh runs, so the view does not flicker.
        let answer = self.detail.latest();
        let detail = match answer {
            Some(Ok(Loaded::Read(detail))) => Some(&**detail),
            _ => None,
        };
        let clicks = header(ui, number, detail, pending);
        refresh |= clicks.refresh;
        if clicks.open {
            open_link = detail.map(|detail| detail.url.clone());
        }
        ui.add_space(6.0);

        match answer {
            None => widgets::loading(ui, "Loading from GitHub"),
            Some(Err(error)) => widgets::callout(ui, &message(error), true),
            Some(Ok(Loaded::NotInstalled)) => {
                widgets::callout(
                    ui,
                    "The GitHub CLI (gh) is not installed. Install it from cli.github.com, then sign in with gh auth login and check again.",
                    true,
                );
                ui.add_space(6.0);
                refresh |= ui.button(format!("{}  Check again", icon::ARROW_CLOCKWISE)).clicked();
            }
            Some(Ok(Loaded::SignedOut(explanation))) => {
                let command = if host == "github.com" { "gh auth login".to_string() } else { format!("gh auth login --hostname {host}") };
                widgets::callout(ui, &format!("GitHub CLI is not signed in. Run {command} in a terminal, then check again."), true);
                if !explanation.is_empty() {
                    ui.add_space(4.0);
                    ui.label(RichText::new(explanation.as_str()).small().color(theme::of(ui).muted));
                }
                ui.add_space(6.0);
                refresh |= ui.button(format!("{}  Check again", icon::ARROW_CLOCKWISE)).clicked();
            }
            Some(Ok(Loaded::Read(detail))) => {
                tabs(ui, &mut tab, detail);
                ui.add_space(4.0);
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    content(ui, tab, detail, now, &mut open_link);
                });
            }
        }

        self.tab = tab;
        if refresh {
            self.generation += 1;
        }
        if let Some(url) = open_link {
            crate::ui::changes::open_url(&url);
        }
    }
}

/// The title row, the badges, and the header's buttons. The pull request's number and title come
/// from what is shown, or from the window until the first read arrives.
fn header(ui: &mut Ui, number: u64, detail: Option<&PullRequestDetail>, pending: bool) -> HeaderClicks {
    let c = theme::of(ui);
    let mut clicks = HeaderClicks::default();
    ui.horizontal(|ui| {
        ui.label(RichText::new(icon::GIT_PULL_REQUEST).size(16.0).color(c.muted));
        ui.label(RichText::new(format!("#{number}")).monospace().color(c.muted));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if widgets::icon_button(ui, icon::ARROWS_CLOCKWISE, "Refresh", true).clicked() {
                clicks.refresh = true;
            }
            if pending {
                ui.spinner();
            }
            if detail.is_some() && widgets::labeled_button(ui, icon::ARROW_SQUARE_OUT, "Open in browser", true).clicked() {
                clicks.open = true;
            }
        });
    });
    let title = detail.map(|detail| detail.title.clone()).unwrap_or_else(|| format!("Pull request #{number}"));
    ui.add(egui::Label::new(RichText::new(title).strong().size(16.0)).wrap());
    if let Some(detail) = detail {
        badges(ui, detail, now_seconds());
    }
    clicks
}

/// The state, author, branches, size, and dates of a pull request, as a wrapped row of small labels.
fn badges(ui: &mut Ui, detail: &PullRequestDetail, now: i64) {
    let c = theme::of(ui);
    let (word, color) = match detail.state {
        PrState::Open => ("open", c.added),
        PrState::Draft => ("draft", c.muted),
        PrState::Merged => ("merged", c.renamed),
        PrState::Closed => ("closed", c.removed),
    };
    ui.horizontal_wrapped(|ui| {
        widgets::pill(ui, word, color);
        if let Some(decision) = &detail.review_decision {
            let (text, color) = decision_label(decision, &c);
            widgets::pill(ui, text, color);
        }
        for label in &detail.labels {
            label_pill(ui, &label.name, &label.color);
        }
        let author = detail.author.as_deref().unwrap_or("no author");
        ui.label(RichText::new(format!("by {author}")).small().color(c.muted));
        ui.label(RichText::new(format!("{} \u{2190} {}", detail.base_ref, detail.head_ref)).monospace().small().color(c.muted));
        ui.label(RichText::new(format!("+{}", detail.additions)).small().color(c.added));
        ui.label(RichText::new(format!("\u{2212}{}", detail.deletions)).small().color(c.removed));
        ui.label(RichText::new(format!("{} files", detail.changed_files)).small().color(c.muted));
        ui.label(RichText::new(format!("{} commits", detail.commit_count)).small().color(c.muted));
        if let Some(opened) = relative_time(&detail.created_at, now) {
            ui.label(RichText::new(format!("opened {opened}")).small().color(c.muted));
        }
        if let Some(updated) = relative_time(&detail.updated_at, now) {
            ui.label(RichText::new(format!("updated {updated}")).small().color(c.muted));
        }
    });
}

/// The section buttons, each with its count.
fn tabs(ui: &mut Ui, tab: &mut Tab, detail: &PullRequestDetail) {
    let conversation = detail.reviews.len() + detail.comments.len();
    let sections = [
        (Tab::Description, "Description".to_string()),
        (Tab::Checks, format!("Checks ({})", detail.check_rows.len())),
        (Tab::Conversation, format!("Conversation ({conversation})")),
        (Tab::Files, format!("Files ({})", detail.changed_files)),
    ];
    ui.horizontal(|ui| {
        for (section, text) in sections {
            if ui.selectable_label(*tab == section, text).clicked() {
                *tab = section;
            }
        }
    });
}

fn content(ui: &mut Ui, tab: Tab, detail: &PullRequestDetail, now: i64, open_link: &mut Option<String>) {
    let c = theme::of(ui);
    match tab {
        Tab::Description => {
            if detail.body.is_empty() {
                ui.label(RichText::new("No description.").color(c.muted));
            } else {
                ui.add(egui::Label::new(RichText::new(detail.body.as_str())).wrap());
            }
        }
        Tab::Checks => {
            let checks = detail.checks;
            ui.label(
                RichText::new(format!("{} passed, {} failed, {} pending", checks.passed, checks.failed, checks.pending))
                    .small()
                    .color(c.muted),
            );
            ui.add_space(4.0);
            if detail.check_rows.is_empty() {
                ui.label(RichText::new("No checks reported.").color(c.muted));
            }
            for row in &detail.check_rows {
                let (glyph, color) = match row.outcome {
                    CheckOutcome::Passed => (icon::CHECK, c.added),
                    CheckOutcome::Failed => (icon::X, c.danger),
                    CheckOutcome::Pending => (icon::CIRCLE, c.warning),
                };
                ui.horizontal(|ui| {
                    ui.label(RichText::new(glyph).color(color));
                    ui.add(egui::Label::new(RichText::new(row.name.as_str()).strong()).wrap());
                    ui.label(RichText::new(row.detail.as_str()).small().color(c.muted));
                    if let Some(url) = &row.url {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.link(format!("{}  Open check", icon::ARROW_SQUARE_OUT)).on_hover_text(url.as_str()).clicked() {
                                *open_link = Some(url.clone());
                            }
                        });
                    }
                });
                ui.add_space(2.0);
            }
        }
        Tab::Conversation => {
            let entries = detail.conversation();
            if entries.is_empty() {
                ui.label(RichText::new("No reviews or comments yet.").color(c.muted));
            }
            for (index, entry) in entries.iter().enumerate() {
                if index > 0 {
                    ui.separator();
                }
                match entry {
                    ConversationEntry::Review(review) => conversation_review(ui, review, now),
                    ConversationEntry::Comment(comment) => {
                        conversation_header(ui, comment.author.as_deref(), "commented", None, &comment.created_at, now);
                        if !comment.body.is_empty() {
                            ui.add(egui::Label::new(RichText::new(comment.body.as_str())).wrap());
                        }
                    }
                }
            }
        }
        Tab::Files => {
            ui.label(
                RichText::new(format!("{} files changed, +{} \u{2212}{}", detail.changed_files, detail.additions, detail.deletions))
                    .small()
                    .color(c.muted),
            );
            if (detail.files.len() as u64) < detail.changed_files {
                ui.label(RichText::new(format!("Showing the first {} files.", detail.files.len())).small().color(c.muted));
            }
            ui.add_space(4.0);
            for file in &detail.files {
                file_row(ui, file);
            }
        }
    }
}

fn conversation_review(ui: &mut Ui, review: &PullRequestReview, now: i64) {
    let c = theme::of(ui);
    let (word, color) = decision_label(&review.state, &c);
    conversation_header(ui, review.author.as_deref(), word, Some(color), review.submitted_at.as_deref().unwrap_or(""), now);
    if !review.body.is_empty() {
        ui.add(egui::Label::new(RichText::new(review.body.as_str())).wrap());
    }
}

/// The author, a state word, and how long ago the entry was written.
fn conversation_header(ui: &mut Ui, author: Option<&str>, word: &str, color: Option<Color32>, stamp: &str, now: i64) {
    let c = theme::of(ui);
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(author.unwrap_or("Unknown author")).strong());
        if let Some(color) = color {
            widgets::pill(ui, word, color);
        } else {
            ui.label(RichText::new(word).small().color(c.muted));
        }
        if let Some(ago) = relative_time(stamp, now) {
            ui.label(RichText::new(ago).small().color(c.muted));
        }
    });
}

fn file_row(ui: &mut Ui, file: &PullRequestFile) {
    let c = theme::of(ui);
    ui.horizontal(|ui| {
        ui.add(egui::Label::new(RichText::new(file.path.as_str()).monospace()).wrap());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(RichText::new(format!("\u{2212}{}", file.deletions)).small().color(c.removed));
            ui.label(RichText::new(format!("+{}", file.additions)).small().color(c.added));
        });
    });
}

/// A review state, or a review decision, as a word and its colour.
fn decision_label(state: &str, c: &theme::Colors) -> (&'static str, Color32) {
    match state {
        "APPROVED" => ("approved", c.added),
        "CHANGES_REQUESTED" => ("changes requested", c.danger),
        "REVIEW_REQUIRED" => ("review required", c.warning),
        "COMMENTED" => ("commented", c.muted),
        "DISMISSED" => ("dismissed", c.muted),
        "PENDING" => ("pending", c.warning),
        _ => ("reviewed", c.muted),
    }
}

fn now_seconds() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|elapsed| elapsed.as_secs() as i64).unwrap_or(0)
}
