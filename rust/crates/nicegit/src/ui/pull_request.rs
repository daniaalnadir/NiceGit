//! "Create pull request" for a local branch: a form that creates the pull request through the GitHub
//! CLI. It fills in the base branch from GitHub, and the title and body from the branch's commits.
//! The browser's compare page stays available as "Open in browser instead". Nothing here blocks the
//! interface; every `gh` and Git read runs in the background.

use std::path::{Path, PathBuf};
use std::time::Instant;

use egui::{Color32, RichText};
use egui_phosphor::regular as icon;
use nicegit_core::github::{GhStatus, GitHubRepository, PrState, PullRequestStatus};
use nicegit_core::{Branch, GitClient, GitError, Snapshot};

use crate::app::{NiceGitApp, Notice, NoticeAction};
use crate::git_ext::{self, PullRequestTarget};
use crate::theme;
use crate::tools::{widgets, Task};

/// The body lists commit subjects only when there are at least this many commits ahead of the base.
const BODY_MIN: usize = 2;
/// ...and at most this many. Longer lists are left for the person to write.
const BODY_MAX: usize = 20;

const NOT_INSTALLED: &str = "The GitHub CLI (gh) is not installed. Install it from cli.github.com, then sign in with gh auth login.";

/// What the dialog's form was closed or used for in one frame.
pub enum Outcome {
    /// The dialog was closed without creating anything.
    Close,
    /// The compare page should open in the browser instead; the dialog closes.
    OpenInBrowser(String),
    /// The pull request was created at this address; the dialog closes.
    Created(String),
}

/// What `gh` said about the repository's default branch. A missing or signed-out `gh` is an answer.
#[derive(Clone, Debug)]
enum BaseLookup {
    NotInstalled,
    SignedOut(String),
    Branch(String),
}

enum Lookup {
    Loading(Task<Result<BaseLookup, String>>),
    Done(Result<BaseLookup, String>),
}

/// A commit-subject read for one base branch, still in flight.
struct CommitRead {
    base: String,
    task: Task<Option<Vec<String>>>,
}

enum Creation {
    Idle,
    Running(Task<Result<String, String>>),
}

/// The Create pull request dialog for one local branch. Its target, the branch's unpushed count,
/// and the checkout it reads are captured when it opens; creation checks them again.
pub struct PullRequestDialog {
    target: PullRequestTarget,
    /// The GitHub compare page, opened by "Open in browser instead".
    compare_link: String,
    /// The local branch's name, as shown in the heading.
    branch: String,
    /// Commits on the branch that its upstream does not have, when the dialog opened.
    unpushed: usize,
    /// The repository folder that `gh` and Git run in.
    directory: PathBuf,
    lookup: Lookup,
    base: String,
    /// Set once the person types in the base field, so a late default branch does not overwrite it.
    base_edited: bool,
    title: String,
    title_edited: bool,
    body: String,
    body_edited: bool,
    draft: bool,
    commits: Option<CommitRead>,
    /// The base whose commit subjects were last applied to the title and body.
    commits_base: Option<String>,
    creation: Creation,
    /// gh's message when creation failed, shown until the next attempt.
    error: Option<String>,
}

impl PullRequestDialog {
    /// Whether the background reads have finished, so the dialog's layout no longer changes.
    #[cfg(test)]
    pub fn settled(&self) -> bool {
        matches!(self.lookup, Lookup::Done(_)) && self.commits.is_none()
    }
}

/// The title for a branch: its last path segment with dashes and underscores as spaces, and the first
/// letter capitalised, so `feature/fix-login` becomes "Fix login".
pub fn title_from_branch_name(branch: &str) -> String {
    let last = branch.rsplit('/').next().unwrap_or(branch);
    let words: Vec<&str> = last.split(|c: char| c == '-' || c == '_' || c.is_whitespace()).filter(|word| !word.is_empty()).collect();
    let words = if words.is_empty() { branch.to_string() } else { words.join(" ") };
    let mut characters = words.chars();
    match characters.next() {
        Some(first) => first.to_uppercase().collect::<String>() + characters.as_str(),
        None => String::new(),
    }
}

/// The title once the commits are known: the subject of the only commit ahead of the base, else the
/// branch's name in words.
fn title_for(branch: &str, commits: Option<&[String]>) -> String {
    match commits {
        Some([only]) if !only.trim().is_empty() => only.trim().to_string(),
        _ => title_from_branch_name(branch),
    }
}

/// The body once the commits are known: a bullet per commit subject when there are a few of them.
fn body_for(commits: Option<&[String]>) -> String {
    match commits {
        Some(list) if (BODY_MIN..=BODY_MAX).contains(&list.len()) => {
            list.iter().map(|subject| format!("- {subject}")).collect::<Vec<_>>().join("\n")
        }
        _ => String::new(),
    }
}

/// The text of a `GitError` for the dialog: gh's own message when there is one.
fn message(error: &GitError) -> String {
    match error {
        GitError::CommandFailed { message, .. } => message.clone(),
        other => other.to_string(),
    }
}

/// Asks `gh` whether it is signed in to the repository's host, then reads the default branch.
fn load_base(repository: &GitHubRepository, directory: &Path) -> Result<BaseLookup, String> {
    let client = GitClient::new();
    match client.gh_status(&repository.host, directory) {
        GhStatus::NotInstalled => Ok(BaseLookup::NotInstalled),
        GhStatus::SignedOut(explanation) => Ok(BaseLookup::SignedOut(explanation)),
        GhStatus::SignedIn => client.default_branch(repository, directory).map(BaseLookup::Branch).map_err(|error| message(&error)),
    }
}

/// The command that signs `gh` in to `host`.
fn sign_in_command(host: &str) -> String {
    if host == "github.com" {
        "gh auth login".to_string()
    } else {
        format!("gh auth login --hostname {host}")
    }
}

/// What the person chose in one frame of the form.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Action {
    None,
    Create,
    Browser,
    Cancel,
}

impl PullRequestDialog {
    /// The dialog for `target`, a local branch named `branch` with `unpushed` commits not on its
    /// upstream, run in `directory`. The default branch lookup starts at once.
    pub fn open(
        ctx: &egui::Context,
        target: PullRequestTarget,
        compare_link: String,
        branch: String,
        unpushed: usize,
        directory: PathBuf,
    ) -> Self {
        let repository = target.repository.clone();
        let lookup_directory = directory.clone();
        let lookup = Lookup::Loading(Task::spawn(ctx, move || load_base(&repository, &lookup_directory)));
        Self {
            title: title_from_branch_name(&branch),
            branch,
            unpushed,
            directory,
            target,
            compare_link,
            lookup,
            base: String::new(),
            base_edited: false,
            title_edited: false,
            body: String::new(),
            body_edited: false,
            draft: false,
            commits: None,
            commits_base: None,
            creation: Creation::Idle,
            error: None,
        }
    }

    /// Moves a finished default branch lookup into place, filling the base field unless it was typed in.
    fn settle_lookup(&mut self) {
        let Lookup::Loading(task) = &mut self.lookup else { return };
        let Some(result) = task.get().cloned() else { return };
        if let Ok(BaseLookup::Branch(name)) = &result {
            if !self.base_edited {
                self.base = name.clone();
            }
        }
        self.lookup = Lookup::Done(result);
    }

    /// Starts reading the commit subjects for the base in the field, when that base has not been read.
    fn start_commits(&mut self, ctx: &egui::Context) {
        let base = self.base.trim().to_string();
        if self.commits.is_some() || base.is_empty() || self.commits_base.as_deref() == Some(base.as_str()) {
            return;
        }
        let remote = self.target.remote.clone();
        let head = self.target.remote_branch.clone();
        let directory = self.directory.clone();
        let read_base = base.clone();
        let task = Task::spawn(ctx, move || GitClient::new().pull_request_commits(&remote, &read_base, &head, &directory).ok());
        self.commits = Some(CommitRead { base, task });
    }

    /// Applies a finished commit read to the title and body, when it is for the base now in the
    /// field. A read for a base that has since changed is dropped, and the next one starts.
    fn settle_commits(&mut self) {
        let Some(read) = self.commits.as_mut() else { return };
        let Some(commits) = read.task.get().cloned() else { return };
        let base = read.base.clone();
        self.commits = None;
        if base != self.base.trim() {
            return;
        }
        self.commits_base = Some(base);
        if !self.title_edited {
            self.title = title_for(&self.branch, commits.as_deref());
        }
        if !self.body_edited {
            self.body = body_for(commits.as_deref());
        }
    }

    /// Moves a finished creation into place: the new address, or gh's message as the error.
    fn settle_creation(&mut self) -> Option<Outcome> {
        let Creation::Running(task) = &mut self.creation else { return None };
        let result = task.get().cloned()?;
        self.creation = Creation::Idle;
        match result {
            Ok(url) => Some(Outcome::Created(url)),
            Err(text) => {
                self.error = Some(text);
                None
            }
        }
    }

    /// Starts creating the pull request, after checking that the branch still has the GitHub target
    /// that the dialog shows. `current` is the target the snapshot gives now.
    fn create(&mut self, ctx: &egui::Context, current: Option<&PullRequestTarget>) {
        if current != Some(&self.target) {
            self.error = Some(
                "The branch's GitHub remote changed after this dialog opened, so nothing was created. Close the dialog and open it again."
                    .to_string(),
            );
            return;
        }
        let repository = self.target.repository.clone();
        let head = self.target.remote_branch.clone();
        let base = self.base.trim().to_string();
        let title = self.title.trim().to_string();
        let body = self.body.clone();
        let draft = self.draft;
        let directory = self.directory.clone();
        self.error = None;
        self.creation = Creation::Running(Task::spawn(ctx, move || {
            GitClient::new()
                .create_pull_request(&repository, &head, &base, &title, &body, draft, &directory)
                .map_err(|error| message(&error))
        }));
    }

    /// Draws the dialog and reports what was chosen. `current` is the GitHub target of the branch as
    /// the snapshot shows it now, or `None` when it no longer has one.
    pub fn show(&mut self, ctx: &egui::Context, current: Option<&PullRequestTarget>) -> Option<Outcome> {
        self.settle_lookup();
        self.settle_commits();
        self.start_commits(ctx);
        if let Some(outcome) = self.settle_creation() {
            return Some(outcome);
        }

        let creating = matches!(self.creation, Creation::Running(_));
        let loading = matches!(self.lookup, Lookup::Loading(_));
        // Creation cannot work without gh, so its button waits for a verdict that says gh is usable.
        let gh_blocked = matches!(self.lookup, Lookup::Done(Ok(BaseLookup::NotInstalled | BaseLookup::SignedOut(_))));
        let can_create = !creating && !gh_blocked && !self.title.trim().is_empty() && !self.base.trim().is_empty();
        let mut action = Action::None;
        let modal = egui::Modal::new(egui::Id::new("pull-request")).show(ctx, |ui| {
            ui.set_width(480.0);
            let c = theme::of(ui);
            let base_text = self.base.trim();
            let shown_base = if base_text.is_empty() { "…" } else { base_text };
            ui.heading(format!("{} → {shown_base}", self.branch));
            ui.label(RichText::new(format!("{} on {}", self.target.repository.slug(), self.target.repository.host)).small().color(c.muted));
            ui.add_space(8.0);

            if self.unpushed > 0 {
                let count = self.unpushed;
                let (noun, verb, pronoun) = if count == 1 { ("commit", "is", "it") } else { ("commits", "are", "them") };
                widgets::callout(
                    ui,
                    &format!("Your branch has {count} {noun} that {verb} not pushed. Push first, or the pull request will not include {pronoun}."),
                    true,
                );
                ui.add_space(8.0);
            }

            ui.horizontal(|ui| {
                ui.label(RichText::new("Base branch").color(c.muted));
                if loading {
                    ui.spinner();
                }
            });
            let response = ui.add(egui::TextEdit::singleline(&mut self.base).hint_text("main").desired_width(f32::INFINITY));
            response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Base branch"));
            if response.changed() {
                self.base_edited = true;
            }
            match &self.lookup {
                Lookup::Loading(_) | Lookup::Done(Ok(BaseLookup::Branch(_))) => {}
                Lookup::Done(Ok(BaseLookup::NotInstalled)) => widgets::callout(ui, NOT_INSTALLED, true),
                Lookup::Done(Ok(BaseLookup::SignedOut(explanation))) => {
                    let command = sign_in_command(&self.target.repository.host);
                    widgets::callout(
                        ui,
                        &format!(
                            "GitHub CLI is not signed in to {}. Run {command} in a terminal, then open this dialog again.",
                            self.target.repository.host
                        ),
                        true,
                    );
                    if !explanation.is_empty() {
                        ui.label(RichText::new(explanation.as_str()).small().color(c.muted));
                    }
                }
                Lookup::Done(Err(text)) => widgets::error(ui, text),
            }
            ui.add_space(8.0);

            ui.label(RichText::new("Title").color(c.muted));
            let response = ui.add(egui::TextEdit::singleline(&mut self.title).desired_width(f32::INFINITY));
            response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Title"));
            if response.changed() {
                self.title_edited = true;
            }
            ui.add_space(8.0);

            ui.label(RichText::new("Body").color(c.muted));
            let response = ui.add(egui::TextEdit::multiline(&mut self.body).desired_rows(8).desired_width(f32::INFINITY));
            response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Body"));
            if response.changed() {
                self.body_edited = true;
            }
            ui.add_space(6.0);

            widgets::checkbox(ui, !creating, &mut self.draft, "Create as a draft");
            ui.add_space(10.0);

            if let Some(text) = &self.error {
                widgets::error(ui, text);
                ui.add_space(6.0);
            }
            if creating {
                widgets::loading(ui, "Creating the pull request on GitHub");
                ui.add_space(6.0);
            }
            ui.horizontal(|ui| {
                if widgets::primary_button(ui, "Create pull request", can_create).clicked() {
                    action = Action::Create;
                }
                if ui.add_enabled(!creating, egui::Button::new("Open in browser instead")).clicked() {
                    action = Action::Browser;
                }
                if ui.add_enabled(!creating, egui::Button::new("Cancel")).clicked() {
                    action = Action::Cancel;
                }
            });
        });

        match action {
            Action::Browser => return Some(Outcome::OpenInBrowser(self.compare_link.clone())),
            Action::Cancel => return Some(Outcome::Close),
            Action::Create => self.create(ctx, current),
            Action::None => {}
        }
        if modal.should_close() && !creating {
            return Some(Outcome::Close);
        }
        None
    }
}

/// The unpushed commits of a local branch: the snapshot's count for the checkout, or the per-branch
/// tracking counts for any other branch. A branch whose upstream is gone counts as none.
fn unpushed_commits(app: &NiceGitApp, snapshot: &Snapshot, branch: &Branch) -> usize {
    if branch.is_current {
        return snapshot.ahead.unwrap_or(0);
    }
    let tracking = app.repos.get(app.active).and_then(|repo| repo.branch_tracking.as_ref()).and_then(|task| task.peek()).cloned().flatten();
    match tracking.as_ref().and_then(|map| map.get(&branch.name)) {
        Some(nicegit_core::parsers::Tracking::Counts { ahead, .. }) => *ahead,
        _ => 0,
    }
}

impl NiceGitApp {
    /// Opens Create pull request for a local branch whose upstream is on a GitHub remote. Nothing
    /// opens for any other branch.
    pub fn open_pull_request(&mut self, ctx: &egui::Context, branch: &Branch) {
        let Some(snapshot) = self.snapshot() else { return };
        let hosts = &self.settings.github_enterprise_hosts;
        let Some(target) = git_ext::github_pull_request_target(snapshot, branch, hosts) else { return };
        let Some(compare_link) = git_ext::github_pull_request_link(snapshot, branch, hosts) else { return };
        let unpushed = unpushed_commits(self, snapshot, branch);
        let Some(directory) = self.repo().map(|repo| repo.path.clone()) else { return };
        self.pull_request = Some(PullRequestDialog::open(ctx, target, compare_link, branch.name.clone(), unpushed, directory));
    }

    /// Draws the Create pull request dialog while it is open, and acts on what it reports.
    pub fn pull_request_window(&mut self, ctx: &egui::Context) {
        let Some(branch_name) = self.pull_request.as_ref().map(|dialog| dialog.branch.clone()) else { return };
        // The branch as the snapshot shows it now, so creation can refuse a changed GitHub target.
        let current = self.snapshot().and_then(|snapshot| {
            let branch = snapshot.branches.iter().find(|branch| !branch.is_remote && branch.name == branch_name)?;
            git_ext::github_pull_request_target(snapshot, branch, &self.settings.github_enterprise_hosts)
        });
        let Some(dialog) = self.pull_request.as_mut() else { return };
        match dialog.show(ctx, current.as_ref()) {
            None => {}
            Some(Outcome::Close) => self.pull_request = None,
            Some(Outcome::OpenInBrowser(link)) => {
                self.pull_request = None;
                crate::ui::changes::open_url(&link);
            }
            Some(Outcome::Created(url)) => {
                self.pull_request = None;
                let number = url.rsplit('/').next().unwrap_or_default().to_string();
                // The notice's button opens the pull request, so nothing opens before the person asks.
                self.notice = Some(Notice {
                    text: format!("Created pull request #{number}."),
                    is_error: false,
                    action: Some(NoticeAction::OpenUrl(url)),
                    detail: None,
                });
                // Creating a pull request gives no new snapshot, so its status is read here.
                self.refresh_pr_status(self.active);
            }
        }
    }
}

/// A pull request status being read through gh, or read: `None` when the branch has no pull request.
pub type PrStatusRead = Task<Result<Option<PullRequestStatus>, String>>;

/// Actions after which the current branch's pull request and its checks are read again.
pub const CHANGES_PULL_REQUEST: [&str; 7] = ["Fetch", "Pull", "Push", "Push branch", "Publish", "Force push", "Commit and push"];

impl NiceGitApp {
    /// Starts reading the current branch's pull request through `gh`, in the background, when the
    /// branch's upstream is on a GitHub remote. A branch without one has no status. The status stays
    /// on screen while it is read again, if the branch is the same. Without `gh`, or signed out of
    /// it, the status is simply none.
    pub fn refresh_pr_status(&mut self, index: usize) {
        let hosts = self.settings.github_enterprise_hosts.clone();
        let context = self.worker.context.clone();
        let Some(repo) = self.repos.get_mut(index) else { return };
        repo.pr_checked_at = Some(Instant::now());
        let target = repo.snapshot.as_ref().and_then(|snapshot| {
            let branch = snapshot.current().filter(|branch| !branch.is_remote)?;
            let target = git_ext::github_pull_request_target(snapshot, branch, &hosts)?;
            Some((snapshot.current_branch.clone(), target))
        });
        let Some((branch, target)) = target else {
            repo.pr_status = None;
            return;
        };
        let stale = repo.pr_status.take().filter(|(name, _)| *name == branch).and_then(|(_, mut task)| task.get().cloned());
        let path = repo.path.clone();
        let read_branch = branch.clone();
        let task = Task::spawn_with(&context, stale, move || {
            let client = GitClient::new();
            match client.gh_status(&target.repository.host, &path) {
                GhStatus::SignedIn => {
                    client.pull_request_for_branch(&target.repository, &read_branch, &path).map_err(|error| error.to_string())
                }
                GhStatus::NotInstalled | GhStatus::SignedOut(_) => Ok(None),
            }
        });
        repo.pr_status = Some((branch, task));
    }

    /// The active checkout's pull request, once its status has been read for the branch the checkout
    /// is on. Nothing is returned while that read is still going, or when it failed.
    pub fn pull_request_status(&mut self) -> Option<PullRequestStatus> {
        let repo = self.repos.get_mut(self.active)?;
        let snapshot = repo.snapshot.as_ref()?;
        let (name, task) = repo.pr_status.as_mut()?;
        if *name != snapshot.current_branch {
            return None;
        }
        task.get()?.clone().ok().flatten()
    }
}

/// The status bar's pull request: its label, the checks' label, and the link it opens.
pub struct PrBadge {
    label: String,
    label_color: Color32,
    checks: Option<(String, Color32)>,
    hover: String,
    /// The pull request's page, opened when the badge is clicked.
    pub url: String,
}

impl PrBadge {
    pub fn new(status: &PullRequestStatus, c: &crate::theme::Colors) -> Self {
        let (glyph, color) = match status.state {
            PrState::Open => (icon::GIT_PULL_REQUEST, c.accent),
            PrState::Draft => (icon::GIT_PULL_REQUEST, c.muted),
            PrState::Merged => (icon::GIT_MERGE, c.renamed),
            PrState::Closed => (icon::X, c.muted),
        };
        let checks = status.checks;
        // A failing check is the news, then a pending one; when neither, the passed count.
        let checks_label = if checks.failed > 0 {
            Some((format!("{} {}", icon::X, checks.failed), c.danger))
        } else if checks.pending > 0 {
            Some((format!("{} {}", icon::CIRCLE, checks.pending), c.warning))
        } else if checks.passed > 0 {
            Some((format!("{} {}", icon::CHECK, checks.passed), c.added))
        } else {
            None
        };
        Self {
            label: format!("{glyph}  PR #{}", status.number),
            label_color: color,
            checks: checks_label,
            hover: format!("{}\n{} passed, {} failed, {} pending", status.title, checks.passed, checks.failed, checks.pending),
            url: status.url.clone(),
        }
    }

    /// The width the badge takes in the status bar, in points, including button padding.
    pub fn width(&self, ui: &egui::Ui) -> f32 {
        let font = egui::TextStyle::Small.resolve(ui.style());
        let padding = ui.spacing().button_padding.x * 2.0;
        let measure = |text: &str, color: Color32| ui.painter().layout_no_wrap(text.to_string(), font.clone(), color).size().x + padding;
        let mut width = measure(&self.label, self.label_color);
        if let Some((text, color)) = &self.checks {
            width += ui.spacing().item_spacing.x + measure(text, *color);
        }
        width
    }
}

/// Draws the badge as two buttons, the pull request and its checks. Returns whether either was clicked.
pub fn show_pull_request_badge(ui: &mut egui::Ui, badge: &PrBadge) -> bool {
    let mut clicked = ui
        .add(egui::Button::new(RichText::new(&badge.label).small().color(badge.label_color)).frame(false))
        .on_hover_text(badge.hover.as_str())
        .clicked();
    if let Some((text, color)) = &badge.checks {
        clicked |=
            ui.add(egui::Button::new(RichText::new(text).small().color(*color)).frame(false)).on_hover_text(badge.hover.as_str()).clicked();
    }
    clicked
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_from_branch_name_turns_the_last_segment_into_words() {
        assert_eq!(title_from_branch_name("feature/fix-login"), "Fix login");
        assert_eq!(title_from_branch_name("user/alice/update_docs"), "Update docs");
        assert_eq!(title_from_branch_name("plain"), "Plain");
        assert_eq!(title_from_branch_name("--"), "--", "a name with no words keeps its text");
    }

    #[test]
    fn title_uses_the_only_commit_and_otherwise_the_branch() {
        let one = vec!["Fix the login form".to_string()];
        assert_eq!(title_for("feature/fix-login", Some(&one)), "Fix the login form");
        let two = vec!["Second".to_string(), "First".to_string()];
        assert_eq!(title_for("feature/fix-login", Some(&two)), "Fix login");
        assert_eq!(title_for("feature/fix-login", None), "Fix login");
        let blank = vec!["  ".to_string()];
        assert_eq!(title_for("feature/fix-login", Some(&blank)), "Fix login", "a blank subject is not a title");
    }

    #[test]
    fn body_lists_two_to_twenty_commits() {
        assert_eq!(body_for(None), "");
        assert_eq!(body_for(Some(&["Only".to_string()])), "", "one commit leaves the body for the person");
        let two = vec!["Second".to_string(), "First".to_string()];
        assert_eq!(body_for(Some(&two)), "- Second\n- First");
        let twenty: Vec<String> = (0..20).map(|n| format!("c{n}")).collect();
        assert!(body_for(Some(&twenty)).lines().count() == 20);
        let too_many: Vec<String> = (0..21).map(|n| format!("c{n}")).collect();
        assert_eq!(body_for(Some(&too_many)), "", "more than twenty commits leaves the body empty");
    }
}
