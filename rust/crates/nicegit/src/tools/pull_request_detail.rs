//! The detail view of one pull request: its header, description, checks, conversation, and files.
//! It is read in the background through the signed-in GitHub CLI (`gh pr view`). Every title, body,
//! comment, and link shown here is display data: nothing runs, and nothing opens unless the person
//! clicks a link. The view can also review the pull request or merge it, but only from a button
//! press followed by a confirmation that names the repository, number, and title. Review comments
//! on the diff are posted the same way: each reply, new comment, or resolve change waits for its own
//! confirmation (see `pull_request_files`).

use egui::{Color32, RichText, Ui};
use egui_phosphor::regular as icon;
use nicegit_core::github::{
    relative_time, Cancel, CheckOutcome, ConversationEntry, GhProgram, GhStatus, GitHubRepository, MergeMethod, PrState, PullRequestDetail,
    PullRequestReview, ReviewChoice,
};
use nicegit_core::github_review::{file_diffs, FileDiff, ReviewThread};

use crate::i18n::{tr, tr_fmt};
use crate::theme;
use crate::tools::github::{label_pill, message, Refreshing};
use crate::tools::pull_request_files::{self, Confirm as ThreadConfirm, FilesState, FilesView, Pull, Write as ThreadWrite};
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

/// The action a confirmation submitted, to run in the background.
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
    /// The review threads, keyed by the detail's generation and the thread generation.
    threads: Refreshing<(u64, u64), Vec<ReviewThread>>,
    /// Bumped after a reply, comment, or resolve change is posted, so the threads are read again.
    thread_generation: u64,
    /// The diff split by file, read when the Files tab is first shown.
    diff: Refreshing<u64, Vec<FileDiff>>,
    /// What the Files tab shows and the comment being written.
    files: FilesState,
    /// The reply, comment, or resolve change awaiting confirmation, if any.
    thread_confirm: Option<ThreadConfirm>,
    /// The reply, comment, or resolve change running in the background.
    thread_write: Option<Task<nicegit_core::Result<String>>>,
    /// The error from the last thread write, shown in its confirmation.
    thread_error: Option<String>,
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
            threads: Refreshing::new(),
            thread_generation: 0,
            diff: Refreshing::new(),
            files: FilesState::default(),
            thread_confirm: None,
            thread_write: None,
            thread_error: None,
        }
    }

    /// The same window, opened on its Checks tab, as a branch's check dot opens it.
    pub fn on_checks(mut self) -> Self {
        self.tab = Tab::Checks;
        self
    }

    /// A window that already shows `detail`, for tests that do not reach GitHub.
    #[cfg(test)]
    pub(crate) fn loaded(repository: GitHubRepository, number: u64, detail: PullRequestDetail) -> Self {
        Self::loaded_with_review(repository, number, detail, None, Vec::new(), Vec::new())
    }

    /// A window that already shows `detail` for the signed-in user `login`, for tests.
    #[cfg(test)]
    pub(crate) fn loaded_as(repository: GitHubRepository, number: u64, detail: PullRequestDetail, login: &str) -> Self {
        Self::loaded_with_review(repository, number, detail, Some(login), Vec::new(), Vec::new())
    }

    /// A window that already shows `detail`, with its review `threads` and `diff`, for tests that
    /// do not reach GitHub. Nothing is read for any of them.
    #[cfg(test)]
    pub(crate) fn loaded_with_review(
        repository: GitHubRepository,
        number: u64,
        detail: PullRequestDetail,
        login: Option<&str>,
        threads: Vec<ReviewThread>,
        diff: Vec<FileDiff>,
    ) -> Self {
        let mut window = Self::new(repository, number);
        window.detail = Refreshing::with_answer(0, Ok(Loaded::Read { detail: Box::new(detail), login: login.map(str::to_string) }));
        window.threads = Refreshing::with_answer((0, 0), Ok(threads));
        window.diff = Refreshing::with_answer(0, Ok(diff));
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
        tr_fmt!("Pull request #{number}", number = self.number)
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
        self.finish_thread_write(cx);
        if matches!(self.detail.latest(), Some(Ok(Loaded::Read { .. }))) {
            self.start_reviews(&ctx, cx);
        }
        self.threads.settle();
        self.diff.settle();
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
        // Reviews, merges, and thread writes wait for the window to be idle, and for the last one to finish.
        let idle = cx.idle && self.write.is_none() && self.thread_write.is_none();
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

        let threads = self.threads.latest();
        let diff = self.diff.latest();
        let mut requested: Option<ThreadConfirm> = None;
        match answer {
            None => widgets::loading(ui, tr("Loading from GitHub")),
            Some(Err(error)) => widgets::callout(ui, &message(error), true),
            Some(Ok(Loaded::NotInstalled)) => {
                widgets::callout(
                    ui,
                    tr("The GitHub CLI (gh) is not installed. Install it from cli.github.com, then sign in with gh auth login and check again."),
                    true,
                );
                ui.add_space(6.0);
                refresh |= ui.button(format!("{}  {}", icon::ARROW_CLOCKWISE, tr("Check again"))).clicked();
            }
            Some(Ok(Loaded::SignedOut(explanation))) => {
                let command = if host == "github.com" { "gh auth login".to_string() } else { format!("gh auth login --hostname {host}") };
                widgets::callout(
                    ui,
                    &tr_fmt!("GitHub CLI is not signed in. Run {command} in a terminal, then check again.", command = command),
                    true,
                );
                if !explanation.is_empty() {
                    ui.add_space(4.0);
                    ui.label(RichText::new(explanation.as_str()).small().color(theme::of(ui).muted));
                }
                ui.add_space(6.0);
                refresh |= ui.button(format!("{}  {}", icon::ARROW_CLOCKWISE, tr("Check again"))).clicked();
            }
            Some(Ok(Loaded::Read { detail, .. })) => {
                tabs(ui, &mut tab, detail);
                ui.add_space(4.0);
                let refusal = thread_refusal(detail.state, idle);
                let pull = pull_of(detail, &self.repository);
                let mut switch: Option<Tab> = None;
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    if tab == Tab::Files {
                        files_summary(ui, detail);
                        let view = FilesView { pull: &pull, files: &detail.files, diff, threads, refusal, now };
                        requested = pull_request_files::show(ui, &mut self.files, &view);
                    } else {
                        content(ui, tab, detail, now, &mut open_link, threads, &mut switch);
                    }
                });
                if let Some(next) = switch {
                    tab = next;
                }
            }
        }

        self.tab = tab;
        if refresh {
            self.generation += 1;
        }
        if let Some(url) = open_link {
            crate::ui::changes::open_url(&url);
        }
        if requested.is_some() {
            self.thread_confirm = requested;
        }
        self.confirmation(&ctx, cx);
        self.thread_confirmation(&ctx, cx);
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

    /// Takes the answer of a finished reply, comment, or resolve change. A success closes its
    /// confirmation, clears the comment, says so, and reads the threads again; a failure stays in
    /// the confirmation.
    fn finish_thread_write(&mut self, cx: &mut Ctx) {
        let Some(task) = self.thread_write.as_mut() else { return };
        if task.is_pending() {
            return;
        }
        let answer = self.thread_write.take().and_then(|mut task| task.get().cloned());
        match answer {
            Some(Ok(text)) => {
                self.thread_confirm = None;
                self.thread_error = None;
                self.files.clear_composer();
                cx.notice(text, false);
                self.thread_generation += 1;
            }
            Some(Err(error)) => self.thread_error = Some(message(&error)),
            None => {}
        }
    }

    /// Starts the review threads of the pull request, and its diff when the Files tab is shown, unless
    /// each is shown or on its way already. The threads are read for the Conversation count too.
    fn start_reviews(&mut self, ctx: &egui::Context, cx: &Ctx) {
        let threads_key = (self.generation, self.thread_generation);
        if !self.threads.shown_or_pending_key(&threads_key) {
            let token = self.cancel.clone();
            let (repository, number) = (self.repository.clone(), self.number);
            let task = query(ctx, cx.repo, move |client, directory| {
                client.review_threads(&repository, number, &GhProgram::Search, &token, directory)
            });
            self.threads.start(threads_key, task);
        }
        if self.tab == Tab::Files && !self.diff.shown_or_pending_key(&self.generation) {
            let token = self.cancel.clone();
            let (repository, number) = (self.repository.clone(), self.number);
            let task = query(ctx, cx.repo, move |client, directory| {
                let patch = client.pull_request_diff(&repository, number, &GhProgram::Search, &token, directory)?;
                Ok(file_diffs(&patch))
            });
            self.diff.start(self.generation, task);
        }
    }

    /// Draws the confirmation for a reply, comment, or resolve change, when one is waiting, and runs
    /// what the person confirmed. Cancel, Escape, and a click outside close it, but not while it runs.
    fn thread_confirmation(&mut self, ctx: &egui::Context, cx: &mut Ctx) {
        let busy = self.thread_write.is_some();
        let error = self.thread_error.clone();
        let Some(write) = pull_request_files::confirmation(ctx, &mut self.thread_confirm, busy, error.as_deref()) else { return };
        self.thread_error = None;
        let (repository, number) = (self.repository.clone(), self.number);
        let task = query(ctx, cx.repo, move |client, directory| match write {
            ThreadWrite::Reply { thread_id, body } => {
                client.reply_to_thread(&repository, &thread_id, &body, &GhProgram::Search, directory)?;
                Ok(tr_fmt!("Replied on #{number}.", number = number))
            }
            ThreadWrite::Line(comment) => {
                client.add_line_comment(&repository, number, &comment, &GhProgram::Search, directory)?;
                Ok(tr_fmt!("Commented on #{number}.", number = number))
            }
            ThreadWrite::Resolve { thread_id, resolved } => {
                client.set_thread_resolved(&repository, &thread_id, resolved, &GhProgram::Search, directory)?;
                Ok(if resolved {
                    tr_fmt!("Resolved a thread on #{number}.", number = number)
                } else {
                    tr_fmt!("Reopened a thread on #{number}.", number = number)
                })
            }
        });
        self.thread_write = Some(task);
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
                    ReviewChoice::Approve => tr_fmt!("Approved #{number}.", number = number),
                    ReviewChoice::RequestChanges => tr_fmt!("Requested changes on #{number}.", number = number),
                    ReviewChoice::Comment => tr_fmt!("Commented on #{number}.", number = number),
                })
            }
            Submit::Merge { method, head_oid } => {
                client.pull_request_merge(&repository, number, method, &head_oid, &GhProgram::Search, directory)?;
                Ok(tr_fmt!("Merged #{number}.", number = number))
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
            if widgets::icon_button(ui, icon::ARROWS_CLOCKWISE, tr("Refresh"), true).clicked() {
                clicks.refresh = true;
            }
            if pending {
                ui.spinner();
            }
            if detail.is_some() && widgets::labeled_button(ui, icon::ARROW_SQUARE_OUT, tr("Open in browser"), true).clicked() {
                clicks.open = true;
            }
            if widgets::labeled_button(ui, icon::GIT_MERGE, tr("Merge…"), merge.enabled)
                .on_hover_text(merge.hover)
                .on_disabled_hover_text(merge.hover)
                .clicked()
            {
                clicks.merge = true;
            }
            if widgets::labeled_button(ui, icon::EYE, tr("Review…"), review.enabled)
                .on_hover_text(review.hover)
                .on_disabled_hover_text(review.hover)
                .clicked()
            {
                clicks.review = true;
            }
        });
    });
    let title = detail.map(|detail| detail.title.clone()).unwrap_or_else(|| tr_fmt!("Pull request #{number}", number = number));
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
        PrState::Open => (tr("open"), c.added),
        PrState::Draft => (tr("draft"), c.muted),
        PrState::Merged => (tr("merged"), c.renamed),
        PrState::Closed => (tr("closed"), c.removed),
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
        let author = detail.author.as_deref().unwrap_or(tr("no author"));
        ui.label(RichText::new(tr_fmt!("by {author}", author = author)).small().color(c.muted));
        ui.label(RichText::new(format!("{} \u{2190} {}", detail.base_ref, detail.head_ref)).monospace().small().color(c.muted));
        ui.label(RichText::new(format!("+{}", detail.additions)).small().color(c.added));
        ui.label(RichText::new(format!("\u{2212}{}", detail.deletions)).small().color(c.removed));
        ui.label(RichText::new(tr_fmt!("{count} files", count = detail.changed_files)).small().color(c.muted));
        ui.label(RichText::new(tr_fmt!("{count} commits", count = detail.commit_count)).small().color(c.muted));
        if let Some(opened) = relative_time(&detail.created_at, now) {
            ui.label(RichText::new(tr_fmt!("opened {when}", when = opened)).small().color(c.muted));
        }
        if let Some(updated) = relative_time(&detail.updated_at, now) {
            ui.label(RichText::new(tr_fmt!("updated {when}", when = updated)).small().color(c.muted));
        }
    });
}

/// The section buttons, each with its count.
fn tabs(ui: &mut Ui, tab: &mut Tab, detail: &PullRequestDetail) {
    let conversation = detail.reviews.len() + detail.comments.len();
    let sections = [
        (Tab::Description, tr("Description").to_string()),
        (Tab::Checks, tr_fmt!("Checks ({count})", count = detail.check_rows.len())),
        (Tab::Conversation, tr_fmt!("Conversation ({count})", count = conversation)),
        (Tab::Files, tr_fmt!("Files ({count})", count = detail.changed_files)),
    ];
    ui.horizontal(|ui| {
        for (section, text) in sections {
            if ui.selectable_label(*tab == section, text).clicked() {
                *tab = section;
            }
        }
    });
}

/// The Files tab's summary of the changed files, and a note when GitHub listed fewer than all of them.
fn files_summary(ui: &mut Ui, detail: &PullRequestDetail) {
    let c = theme::of(ui);
    ui.label(
        RichText::new(tr_fmt!(
            "{files} files changed, +{added} −{removed}",
            files = detail.changed_files,
            added = detail.additions,
            removed = detail.deletions
        ))
        .small()
        .color(c.muted),
    );
    if (detail.files.len() as u64) < detail.changed_files {
        ui.label(RichText::new(tr_fmt!("Showing the first {count} files.", count = detail.files.len())).small().color(c.muted));
    }
    ui.add_space(4.0);
}

/// The Description, Checks, and Conversation sections. The Files section is drawn by its own module.
fn content(
    ui: &mut Ui,
    tab: Tab,
    detail: &PullRequestDetail,
    now: i64,
    open_link: &mut Option<String>,
    threads: Option<&nicegit_core::Result<Vec<ReviewThread>>>,
    switch: &mut Option<Tab>,
) {
    let c = theme::of(ui);
    match tab {
        Tab::Description => {
            if detail.body.is_empty() {
                ui.label(RichText::new(tr("No description.")).color(c.muted));
            } else {
                ui.add(egui::Label::new(RichText::new(detail.body.as_str())).wrap());
            }
        }
        Tab::Checks => {
            let checks = detail.checks;
            ui.label(
                RichText::new(tr_fmt!(
                    "{passed} passed, {failed} failed, {pending} pending",
                    passed = checks.passed,
                    failed = checks.failed,
                    pending = checks.pending
                ))
                .small()
                .color(c.muted),
            );
            ui.add_space(4.0);
            if detail.check_rows.is_empty() {
                ui.label(RichText::new(tr("No checks reported.")).color(c.muted));
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
                            if ui.link(format!("{}  {}", icon::ARROW_SQUARE_OUT, tr("Open check"))).on_hover_text(url.as_str()).clicked() {
                                *open_link = Some(url.clone());
                            }
                        });
                    }
                });
                ui.add_space(2.0);
            }
        }
        Tab::Conversation => {
            review_thread_count(ui, threads, switch);
            let entries = detail.conversation();
            if entries.is_empty() {
                ui.label(RichText::new(tr("No reviews or comments yet.")).color(c.muted));
            }
            for (index, entry) in entries.iter().enumerate() {
                if index > 0 {
                    ui.separator();
                }
                match entry {
                    ConversationEntry::Review(review) => conversation_review(ui, review, now),
                    ConversationEntry::Comment(comment) => {
                        conversation_header(ui, comment.author.as_deref(), tr("commented"), None, &comment.created_at, now);
                        if !comment.body.is_empty() {
                            ui.add(egui::Label::new(RichText::new(comment.body.as_str())).wrap());
                        }
                    }
                }
            }
        }
        // The Files section is drawn by the window, which keeps the state it needs.
        Tab::Files => {}
    }
}

/// The count of review threads and how many are unresolved, as a link to the Files tab, where the
/// threads are shown on their lines. Shown once the threads have been read.
fn review_thread_count(ui: &mut Ui, threads: Option<&nicegit_core::Result<Vec<ReviewThread>>>, switch: &mut Option<Tab>) {
    let Some(Ok(threads)) = threads else { return };
    let unresolved = threads.iter().filter(|thread| !thread.resolved).count();
    if ui.link(tr_fmt!("{count} review threads ({unresolved} unresolved)", count = threads.len(), unresolved = unresolved)).clicked() {
        *switch = Some(Tab::Files);
    }
    ui.add_space(6.0);
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
        ui.label(RichText::new(author.unwrap_or(tr("Unknown author"))).strong());
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

/// A review state, or a review decision, as a word and its colour.
fn decision_label(state: &str, c: &theme::Colors) -> (&'static str, Color32) {
    match state {
        "APPROVED" => (tr("approved"), c.added),
        "CHANGES_REQUESTED" => (tr("changes requested"), c.danger),
        "REVIEW_REQUIRED" => (tr("review required"), c.warning),
        "COMMENTED" => (tr("commented"), c.muted),
        "DISMISSED" => (tr("dismissed"), c.muted),
        "PENDING" => (tr("pending"), c.warning),
        _ => (tr("reviewed"), c.muted),
    }
}

/// The Review button: enabled for an open or draft pull request while the window is idle.
fn review_gate(detail: Option<&PullRequestDetail>, idle: bool) -> Gate {
    let Some(detail) = detail else { return Gate { enabled: false, hover: tr("The pull request is still loading.") } };
    if !idle {
        return Gate { enabled: false, hover: tr("Wait for the current action to finish.") };
    }
    match detail.state {
        PrState::Merged | PrState::Closed => Gate { enabled: false, hover: tr("A merged or closed pull request cannot be reviewed.") },
        PrState::Open | PrState::Draft => {
            Gate { enabled: true, hover: tr("Approve, request changes on, or comment on this pull request.") }
        }
    }
}

/// The Merge button: enabled only for an open pull request that is not a draft and that GitHub
/// reports as mergeable, while the window is idle.
fn merge_gate(detail: Option<&PullRequestDetail>, idle: bool) -> Gate {
    let Some(detail) = detail else { return Gate { enabled: false, hover: tr("The pull request is still loading.") } };
    if !idle {
        return Gate { enabled: false, hover: tr("Wait for the current action to finish.") };
    }
    match merge_refusal(detail.state, detail.mergeable.as_deref()) {
        Some(reason) => Gate { enabled: false, hover: reason },
        None => Gate { enabled: true, hover: tr("Merge this pull request into its base branch.") },
    }
}

/// Why the pull request cannot be merged as it stands, or `None` when it can.
fn merge_refusal(state: PrState, mergeable: Option<&str>) -> Option<&'static str> {
    match state {
        PrState::Draft => Some(tr("A draft cannot be merged. Mark it ready for review first.")),
        PrState::Merged => Some(tr("This pull request is already merged.")),
        PrState::Closed => Some(tr("A closed pull request cannot be merged.")),
        PrState::Open if mergeable == Some("MERGEABLE") => None,
        PrState::Open => Some(tr("GitHub does not report this pull request as mergeable. Refresh to check again.")),
    }
}

/// Why replies, comments, and resolve changes are refused for a pull request in `state`, or `None`
/// when they can be confirmed. Only an open or draft pull request takes them, and only while the
/// window is idle.
fn thread_refusal(state: PrState, idle: bool) -> Option<&'static str> {
    if !idle {
        return Some(tr("Wait for the current action to finish."));
    }
    match state {
        PrState::Open | PrState::Draft => None,
        PrState::Merged | PrState::Closed => Some(tr("A merged or closed pull request cannot take review comments.")),
    }
}

/// Why `choice` cannot be submitted for the pull request as it was shown, or `None` when it can.
/// A merged or closed pull request takes no review. An author cannot approve or request changes on
/// their own pull request, and without a known login that cannot be checked, so those are refused
/// too. A comment is always allowed.
fn review_refusal(shown: &Shown, choice: ReviewChoice) -> Option<&'static str> {
    if matches!(shown.state, PrState::Merged | PrState::Closed) {
        return Some(tr("A merged or closed pull request cannot be reviewed."));
    }
    if choice == ReviewChoice::Comment {
        return None;
    }
    let Some(login) = shown.login.as_deref() else {
        return Some(tr("Your GitHub login could not be read, so the author cannot be checked. Refresh to try again."));
    };
    match shown.author.as_deref() {
        Some(author) if author.eq_ignore_ascii_case(login) => {
            Some(tr("You wrote this pull request, so you cannot approve or request changes on it. You can still comment."))
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

/// The pull request a thread write names, as the Files tab shows it.
fn pull_of(detail: &PullRequestDetail, repository: &GitHubRepository) -> Pull {
    Pull {
        repository: format!("{}/{}", repository.host, repository.slug()),
        number: detail.number,
        title: detail.title.clone(),
        head_oid: detail.head_oid.clone(),
    }
}

/// The body of a confirmation: what it names, its choices, and its buttons. Returns the action the
/// person asked to run, and whether Cancel was clicked.
fn confirm_body(ui: &mut Ui, confirm: &mut Confirm, busy: bool, error: Option<&str>) -> (Option<Submit>, bool) {
    let c = theme::of(ui);
    ui.set_width(460.0);
    let (heading, shown) = match confirm {
        Confirm::Review { shown, .. } => (tr("Review pull request"), shown.clone()),
        Confirm::Merge { shown, .. } => (tr("Merge pull request"), shown.clone()),
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
    widgets::section(ui, tr("Review"));
    ui.add_space(2.0);
    for (option, text) in [
        (ReviewChoice::Approve, tr("Approve")),
        (ReviewChoice::RequestChanges, tr("Request changes")),
        (ReviewChoice::Comment, tr("Comment")),
    ] {
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
        ReviewChoice::Approve => (tr("Optional comment"), tr("Approving tells the author the pull request is ready.")),
        ReviewChoice::RequestChanges => (tr("Say what needs to change"), tr("Asks the author to make changes. A comment is required.")),
        ReviewChoice::Comment => {
            (tr("Write a comment"), tr("Posts your comment without approving or requesting changes. A comment is required."))
        }
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
    let (send, cancel) = footer(ui, tr("Submit review"), ready, busy);
    (send.then(|| Submit::Review { choice: *choice, body: body.clone() }), cancel)
}

/// The merge methods and the footer. The merge goes ahead only if the head commit is still the one shown.
fn merge_form(ui: &mut Ui, shown: &Shown, method: &mut MergeMethod, busy: bool) -> (Option<Submit>, bool) {
    let c = theme::of(ui);
    widgets::section(ui, tr("Method"));
    ui.add_space(2.0);
    for (option, text) in [
        (MergeMethod::Merge, tr("Create a merge commit")),
        (MergeMethod::Squash, tr("Squash and merge")),
        (MergeMethod::Rebase, tr("Rebase and merge")),
    ] {
        if ui.add_enabled(!busy, egui::RadioButton::new(*method == option, text)).clicked() {
            *method = option;
        }
    }
    ui.add_space(6.0);
    ui.label(RichText::new(tr_fmt!("From {head} into {base}", head = shown.head_ref, base = shown.base_ref)).monospace().small());
    ui.label(RichText::new(tr_fmt!("Head commit {oid}", oid = short_oid(&shown.head_oid))).small().color(c.muted));
    ui.label(RichText::new(tr("The merge goes ahead only if the head commit is still this one.")).small().color(c.muted));
    let (send, cancel) = footer(ui, tr("Merge pull request"), !busy, busy);
    (send.then(|| Submit::Merge { method: *method, head_oid: shown.head_oid.clone() }), cancel)
}

/// The confirm and cancel buttons. Returns whether the confirm button was clicked, and whether Cancel was.
pub(crate) fn footer(ui: &mut Ui, confirm_text: &str, ready: bool, busy: bool) -> (bool, bool) {
    let c = theme::of(ui);
    let mut clicks = (false, false);
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            clicks.0 = widgets::primary_button(ui, confirm_text, ready).clicked();
            if ui.add_enabled(!busy, egui::Button::new(tr("Cancel"))).clicked() {
                clicks.1 = true;
            }
            if busy {
                ui.spinner();
                ui.label(RichText::new(tr("Sending to GitHub")).small().color(c.muted));
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

    #[test]
    fn review_comments_are_written_only_on_an_open_pull_request_while_idle() {
        assert!(thread_refusal(PrState::Open, true).is_none());
        assert!(thread_refusal(PrState::Draft, true).is_none(), "a draft takes review comments");
        assert!(thread_refusal(PrState::Merged, true).is_some());
        assert!(thread_refusal(PrState::Closed, true).is_some());
        assert!(thread_refusal(PrState::Open, false).is_some(), "an action in progress refuses them");
    }
}
