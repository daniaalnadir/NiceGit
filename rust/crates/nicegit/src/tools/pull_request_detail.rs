//! The detail view of one pull request: its header, description, checks, conversation, and files.
//! It is read in the background through the signed-in GitHub CLI (`gh pr view`). Every title, body,
//! comment, and link shown here is display data: nothing runs, and nothing opens unless the person
//! clicks a link. The view can also review the pull request or merge it, but only from a button
//! press followed by a confirmation that names the repository, number, and title.

use egui::{Color32, RichText, Ui};
use egui_phosphor::regular as icon;
use nicegit_core::github::{
    relative_time, Cancel, CheckOutcome, ConversationEntry, GhProgram, GhStatus, GitHubRepository, MergeMethod, PrState, PullRequestDetail,
    PullRequestFile, PullRequestReview, ReviewChoice,
};

use crate::theme;
use crate::tools::github::{label_pill, message, Refreshing};
use crate::tools::{query, widgets, Ctx, Task, ToolWindow};

/// The sections of the detail view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tab {
    Description,
    Checks,
    Conversation,
    Files,
}

/// What the background read found: the detail, or why `gh` could not give it. The login is the
/// signed-in user's, when `gh` could give it; without it, the author cannot be checked for a review.
pub enum Loaded {
    NotInstalled,
    SignedOut(String),
    Read { detail: Box<PullRequestDetail>, login: Option<String> },
}

/// What a review or merge confirmation was opened for, captured from the detail as it was shown.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Shown {
    /// The repository as `host/owner/name`.
    repository: String,
    number: u64,
    title: String,
    state: PrState,
    author: Option<String>,
    base_ref: String,
    head_ref: String,
    head_oid: String,
    login: Option<String>,
}

/// A review or merge waiting for the person's confirmation.
enum Confirm {
    Review { shown: Shown, choice: ReviewChoice, body: String },
    Merge { shown: Shown, method: MergeMethod },
}

/// What a confirmation asked to run.
enum Submit {
    Review { choice: ReviewChoice, body: String },
    Merge { method: MergeMethod, head_oid: String },
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
    /// The review or merge awaiting confirmation, if any.
    confirm: Option<Confirm>,
    /// The review or merge running in the background. It is not cancelled when the window closes.
    write: Option<Task<nicegit_core::Result<String>>>,
    /// The error from the last review or merge, shown in its confirmation.
    write_error: Option<String>,
}

impl PullRequestDetailWindow {
    /// The detail view of pull request `number` on `repository`. Nothing is read until the window is drawn.
    pub fn new(repository: GitHubRepository, number: u64) -> Self {
        Self {
            repository,
            number,
            tab: Tab::Description,
            generation: 0,
            detail: Refreshing::new(),
            cancel: Cancel::default(),
            confirm: None,
            write: None,
            write_error: None,
        }
    }

    /// A window that already shows `detail`, for tests that do not reach GitHub.
    #[cfg(test)]
    pub(crate) fn loaded(repository: GitHubRepository, number: u64, detail: PullRequestDetail) -> Self {
        let mut window = Self::new(repository, number);
        window.detail = Refreshing::with_answer(0, Ok(Loaded::Read { detail: Box::new(detail), login: None }));
        window
    }

    /// A window that already shows `detail` for the signed-in user `login`, for tests.
    #[cfg(test)]
    pub(crate) fn loaded_as(repository: GitHubRepository, number: u64, detail: PullRequestDetail, login: &str) -> Self {
        let mut window = Self::new(repository, number);
        window.detail = Refreshing::with_answer(0, Ok(Loaded::Read { detail: Box::new(detail), login: Some(login.to_string()) }));
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
    review: bool,
    merge: bool,
}

/// Whether a write button is enabled, and the text it shows on hover.
struct Gate {
    enabled: bool,
    hover: &'static str,
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
                    let detail = client.pull_request_detail(&repository, number, &token, directory)?;
                    // The login only decides which reviews are offered, so a failed read leaves it out.
                    let login = client.github_login(&repository.host, &GhProgram::Search, directory).ok();
                    Ok(Loaded::Read { detail: Box::new(detail), login })
                }
            });
            self.detail.start(key, task);
        }
        self.detail.settle();
        self.finish_write(cx);
        let pending = self.detail.is_pending();
        let now = now_seconds();
        let host = self.repository.host.clone();
        let number = self.number;
        let mut tab = self.tab;
        let mut refresh = false;
        let mut open_link: Option<String> = None;

        // The last answer stays on screen while a refresh runs, so the view does not flicker.
        let answer = self.detail.latest();
        let (detail, login) = match answer {
            Some(Ok(Loaded::Read { detail, login })) => (Some(&**detail), login.as_deref()),
            _ => (None, None),
        };
        // Reviews and merges wait for the window to be idle, and for the last one to finish.
        let idle = cx.idle && self.write.is_none();
        let clicks = header(ui, number, detail, pending, idle);
        refresh |= clicks.refresh;
        if clicks.open {
            open_link = detail.map(|detail| detail.url.clone());
        }
        if let Some(detail) = detail {
            if clicks.review {
                self.confirm = Some(Confirm::Review {
                    shown: shown(detail, login, &self.repository),
                    choice: ReviewChoice::Approve,
                    body: String::new(),
                });
                self.write_error = None;
            }
            if clicks.merge {
                self.confirm = Some(Confirm::Merge { shown: shown(detail, login, &self.repository), method: MergeMethod::Merge });
                self.write_error = None;
            }
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
            Some(Ok(Loaded::Read { detail, .. })) => {
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
        self.confirmation(&ctx, cx);
    }
}

impl PullRequestDetailWindow {
    /// Takes the answer of a finished review or merge. A success closes its confirmation, says so,
    /// and reads the pull request again; a failure stays in the confirmation for the person to read.
    fn finish_write(&mut self, cx: &mut Ctx) {
        let Some(task) = self.write.as_mut() else { return };
        if task.is_pending() {
            return;
        }
        let answer = self.write.take().and_then(|mut task| task.get().cloned());
        match answer {
            Some(Ok(text)) => {
                self.confirm = None;
                self.write_error = None;
                cx.notice(text, false);
                self.generation += 1;
            }
            Some(Err(error)) => self.write_error = Some(message(&error)),
            None => {}
        }
    }

    /// Draws the confirmation for a review or merge, when one is waiting, and starts what it asks for.
    /// Cancel, Escape, and a click outside close it, but not while the action is running.
    fn confirmation(&mut self, ctx: &egui::Context, cx: &mut Ctx) {
        let Some(confirm) = self.confirm.as_mut() else { return };
        let busy = self.write.is_some();
        let error = self.write_error.clone();
        let response = egui::Modal::new(egui::Id::unique(("pull request confirmation", self.number)))
            .show(ctx, |ui| confirm_body(ui, confirm, busy, error.as_deref()));
        let closing = response.should_close();
        let (submit, cancel) = response.inner;
        if (cancel || closing) && !busy {
            self.confirm = None;
            self.write_error = None;
            return;
        }
        let Some(submit) = submit else { return };
        self.write_error = None;
        let (repository, number) = (self.repository.clone(), self.number);
        let task = query(ctx, cx.repo, move |client, directory| match submit {
            Submit::Review { choice, body } => {
                client.pull_request_review(&repository, number, choice, &body, &GhProgram::Search, directory)?;
                Ok(match choice {
                    ReviewChoice::Approve => format!("Approved #{number}."),
                    ReviewChoice::RequestChanges => format!("Requested changes on #{number}."),
                    ReviewChoice::Comment => format!("Commented on #{number}."),
                })
            }
            Submit::Merge { method, head_oid } => {
                client.pull_request_merge(&repository, number, method, &head_oid, &GhProgram::Search, directory)?;
                Ok(format!("Merged #{number}."))
            }
        });
        self.write = Some(task);
    }
}

/// The title row, the badges, and the header's buttons. The pull request's number and title come
/// from what is shown, or from the window until the first read arrives.
fn header(ui: &mut Ui, number: u64, detail: Option<&PullRequestDetail>, pending: bool, idle: bool) -> HeaderClicks {
    let c = theme::of(ui);
    let mut clicks = HeaderClicks::default();
    let review = review_gate(detail, idle);
    let merge = merge_gate(detail, idle);
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
            if widgets::labeled_button(ui, icon::GIT_MERGE, "Merge…", merge.enabled)
                .on_hover_text(merge.hover)
                .on_disabled_hover_text(merge.hover)
                .clicked()
            {
                clicks.merge = true;
            }
            if widgets::labeled_button(ui, icon::EYE, "Review…", review.enabled)
                .on_hover_text(review.hover)
                .on_disabled_hover_text(review.hover)
                .clicked()
            {
                clicks.review = true;
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

/// The Review button: enabled for an open or draft pull request while the window is idle.
fn review_gate(detail: Option<&PullRequestDetail>, idle: bool) -> Gate {
    let Some(detail) = detail else { return Gate { enabled: false, hover: "The pull request is still loading." } };
    if !idle {
        return Gate { enabled: false, hover: "Wait for the current action to finish." };
    }
    match detail.state {
        PrState::Merged | PrState::Closed => Gate { enabled: false, hover: "A merged or closed pull request cannot be reviewed." },
        PrState::Open | PrState::Draft => Gate { enabled: true, hover: "Approve, request changes on, or comment on this pull request." },
    }
}

/// The Merge button: enabled only for an open pull request that is not a draft and that GitHub
/// reports as mergeable, while the window is idle.
fn merge_gate(detail: Option<&PullRequestDetail>, idle: bool) -> Gate {
    let Some(detail) = detail else { return Gate { enabled: false, hover: "The pull request is still loading." } };
    if !idle {
        return Gate { enabled: false, hover: "Wait for the current action to finish." };
    }
    match merge_refusal(detail.state, detail.mergeable.as_deref()) {
        Some(reason) => Gate { enabled: false, hover: reason },
        None => Gate { enabled: true, hover: "Merge this pull request into its base branch." },
    }
}

/// Why the pull request cannot be merged as it stands, or `None` when it can.
fn merge_refusal(state: PrState, mergeable: Option<&str>) -> Option<&'static str> {
    match state {
        PrState::Draft => Some("A draft cannot be merged. Mark it ready for review first."),
        PrState::Merged => Some("This pull request is already merged."),
        PrState::Closed => Some("A closed pull request cannot be merged."),
        PrState::Open if mergeable == Some("MERGEABLE") => None,
        PrState::Open => Some("GitHub does not report this pull request as mergeable. Refresh to check again."),
    }
}

/// Why `choice` cannot be submitted for the pull request as it was shown, or `None` when it can.
/// A merged or closed pull request takes no review. An author cannot approve or request changes on
/// their own pull request, and without a known login that cannot be checked, so those are refused
/// too. A comment is always allowed.
fn review_refusal(shown: &Shown, choice: ReviewChoice) -> Option<&'static str> {
    if matches!(shown.state, PrState::Merged | PrState::Closed) {
        return Some("A merged or closed pull request cannot be reviewed.");
    }
    if choice == ReviewChoice::Comment {
        return None;
    }
    let Some(login) = shown.login.as_deref() else {
        return Some("Your GitHub login could not be read, so the author cannot be checked. Refresh to try again.");
    };
    match shown.author.as_deref() {
        Some(author) if author.eq_ignore_ascii_case(login) => {
            Some("You wrote this pull request, so you cannot approve or request changes on it. You can still comment.")
        }
        _ => None,
    }
}

/// What a review or merge names, captured from the detail as it was shown.
fn shown(detail: &PullRequestDetail, login: Option<&str>, repository: &GitHubRepository) -> Shown {
    Shown {
        repository: format!("{}/{}", repository.host, repository.slug()),
        number: detail.number,
        title: detail.title.clone(),
        state: detail.state,
        author: detail.author.clone(),
        base_ref: detail.base_ref.clone(),
        head_ref: detail.head_ref.clone(),
        head_oid: detail.head_oid.clone(),
        login: login.map(str::to_string),
    }
}

/// The body of a confirmation: what it names, its choices, and its buttons. Returns the action the
/// person asked to run, and whether Cancel was clicked.
fn confirm_body(ui: &mut Ui, confirm: &mut Confirm, busy: bool, error: Option<&str>) -> (Option<Submit>, bool) {
    let c = theme::of(ui);
    ui.set_width(460.0);
    let (heading, shown) = match confirm {
        Confirm::Review { shown, .. } => ("Review pull request", shown.clone()),
        Confirm::Merge { shown, .. } => ("Merge pull request", shown.clone()),
    };
    ui.label(RichText::new(heading).strong().size(16.0));
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new(shown.repository.as_str()).monospace().color(c.muted));
        ui.label(RichText::new(format!("#{}", shown.number)).monospace().color(c.muted));
    });
    ui.add(egui::Label::new(RichText::new(shown.title.as_str()).strong()).wrap());
    ui.add_space(8.0);
    let (submit, cancel) = match confirm {
        Confirm::Review { choice, body, .. } => review_form(ui, &shown, choice, body, busy),
        Confirm::Merge { method, .. } => merge_form(ui, &shown, method, busy),
    };
    if let Some(error) = error {
        ui.add_space(6.0);
        widgets::callout(ui, error, true);
    }
    (submit, cancel)
}

/// The review choices, the comment, and the footer. A choice that is refused for this pull request
/// is replaced by a comment, which anyone may post.
fn review_form(ui: &mut Ui, shown: &Shown, choice: &mut ReviewChoice, body: &mut String, busy: bool) -> (Option<Submit>, bool) {
    let c = theme::of(ui);
    if review_refusal(shown, *choice).is_some() {
        *choice = ReviewChoice::Comment;
    }
    widgets::section(ui, "Review");
    ui.add_space(2.0);
    for (option, text) in
        [(ReviewChoice::Approve, "Approve"), (ReviewChoice::RequestChanges, "Request changes"), (ReviewChoice::Comment, "Comment")]
    {
        let refusal = review_refusal(shown, option);
        let response = ui.add_enabled(!busy && refusal.is_none(), egui::RadioButton::new(*choice == option, text));
        let response = match refusal {
            Some(reason) => response.on_disabled_hover_text(reason),
            None => response,
        };
        if response.clicked() {
            *choice = option;
        }
    }
    ui.add_space(6.0);
    let (hint, explanation) = match choice {
        ReviewChoice::Approve => ("Optional comment", "Approving tells the author the pull request is ready."),
        ReviewChoice::RequestChanges => ("Say what needs to change", "Asks the author to make changes. A comment is required."),
        ReviewChoice::Comment => ("Write a comment", "Posts your comment without approving or requesting changes. A comment is required."),
    };
    ui.label(RichText::new(explanation).small().color(c.muted));
    ui.add_space(4.0);
    ui.add_enabled(!busy, egui::TextEdit::multiline(body).hint_text(hint).desired_rows(5).desired_width(f32::INFINITY));
    let refusal = review_refusal(shown, *choice);
    if let Some(reason) = refusal {
        ui.add_space(4.0);
        ui.label(RichText::new(reason).small().color(c.warning));
    }
    let ready = !busy && refusal.is_none() && (!choice.needs_body() || !body.trim().is_empty());
    let (send, cancel) = footer(ui, "Submit review", ready, busy);
    (send.then(|| Submit::Review { choice: *choice, body: body.clone() }), cancel)
}

/// The merge methods and the footer. The merge goes ahead only if the head commit is still the one shown.
fn merge_form(ui: &mut Ui, shown: &Shown, method: &mut MergeMethod, busy: bool) -> (Option<Submit>, bool) {
    let c = theme::of(ui);
    widgets::section(ui, "Method");
    ui.add_space(2.0);
    for (option, text) in [
        (MergeMethod::Merge, "Create a merge commit"),
        (MergeMethod::Squash, "Squash and merge"),
        (MergeMethod::Rebase, "Rebase and merge"),
    ] {
        if ui.add_enabled(!busy, egui::RadioButton::new(*method == option, text)).clicked() {
            *method = option;
        }
    }
    ui.add_space(6.0);
    ui.label(RichText::new(format!("From {} into {}", shown.head_ref, shown.base_ref)).monospace().small());
    ui.label(RichText::new(format!("Head commit {}", short_oid(&shown.head_oid))).small().color(c.muted));
    ui.label(RichText::new("The merge goes ahead only if the head commit is still this one.").small().color(c.muted));
    let (send, cancel) = footer(ui, "Merge pull request", !busy, busy);
    (send.then(|| Submit::Merge { method: *method, head_oid: shown.head_oid.clone() }), cancel)
}

/// The confirm and cancel buttons. Returns whether the confirm button was clicked, and whether Cancel was.
fn footer(ui: &mut Ui, confirm_text: &str, ready: bool, busy: bool) -> (bool, bool) {
    let c = theme::of(ui);
    let mut clicks = (false, false);
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            clicks.0 = widgets::primary_button(ui, confirm_text, ready).clicked();
            if ui.add_enabled(!busy, egui::Button::new("Cancel")).clicked() {
                clicks.1 = true;
            }
            if busy {
                ui.spinner();
                ui.label(RichText::new("Sending to GitHub").small().color(c.muted));
            }
        });
    });
    clicks
}

/// The first seven characters of a commit ID, as the confirmation shows it.
fn short_oid(oid: &str) -> String {
    oid.chars().take(7).collect()
}

fn now_seconds() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|elapsed| elapsed.as_secs() as i64).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shown(state: PrState, author: Option<&str>, login: Option<&str>) -> Shown {
        Shown {
            repository: "github.com/acme/app".into(),
            number: 12,
            title: "Add the status bar".into(),
            state,
            author: author.map(str::to_string),
            base_ref: "main".into(),
            head_ref: "status-bar".into(),
            head_oid: "0123456789abcdef0123456789abcdef01234567".into(),
            login: login.map(str::to_string),
        }
    }

    #[test]
    fn an_author_cannot_approve_or_request_changes_on_their_own_pull_request() {
        let mine = shown(PrState::Open, Some("Daniaal"), Some("daniaal"));
        assert!(review_refusal(&mine, ReviewChoice::Approve).is_some(), "logins compare without case");
        assert!(review_refusal(&mine, ReviewChoice::RequestChanges).is_some());
        assert!(review_refusal(&mine, ReviewChoice::Comment).is_none(), "a comment is always allowed");
        let theirs = shown(PrState::Open, Some("octocat"), Some("daniaal"));
        assert!(review_refusal(&theirs, ReviewChoice::Approve).is_none());
    }

    #[test]
    fn approval_needs_a_known_login_and_a_merged_or_closed_pull_request_takes_no_review() {
        let unknown = shown(PrState::Open, Some("octocat"), None);
        assert!(review_refusal(&unknown, ReviewChoice::Approve).is_some(), "the author cannot be checked");
        assert!(review_refusal(&unknown, ReviewChoice::Comment).is_none());
        let closed = shown(PrState::Closed, Some("octocat"), Some("daniaal"));
        assert!(review_refusal(&closed, ReviewChoice::Comment).is_some());
    }

    #[test]
    fn a_merge_needs_an_open_mergeable_pull_request_that_is_not_a_draft() {
        assert!(merge_refusal(PrState::Open, Some("MERGEABLE")).is_none());
        assert!(merge_refusal(PrState::Draft, Some("MERGEABLE")).is_some(), "a draft is refused");
        assert!(merge_refusal(PrState::Open, Some("CONFLICTING")).is_some());
        assert!(merge_refusal(PrState::Open, None).is_some(), "an unknown state is refused");
        assert!(merge_refusal(PrState::Merged, Some("MERGEABLE")).is_some());
        assert!(merge_refusal(PrState::Closed, Some("MERGEABLE")).is_some());
    }
}
