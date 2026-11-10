//! The Launchpad: open pull requests and merge requests across the repositories open as tabs, and
//! across a saved workspace's folders when one is chosen, grouped by what they need from you. Each
//! repository is read with one read-only call: `gh pr list` for GitHub, `glab mr list` for GitLab,
//! after checking that its `origin` is on that host and that the CLI is signed in to it. The reads
//! run in the background, a few at a time, and stop when the window closes. Nothing polls: Refresh
//! reads again.

use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use egui::{Color32, RichText, Ui};
use egui_phosphor::regular as icon;
use nicegit_core::github::{
    Cancel, Checks, GhProgram, GitHubRepository, OpenPullRequest, ReviewDecision, ReviewRequest, OPEN_PULL_REQUEST_LIMIT,
};
use nicegit_core::gitlab::{self, GlabProgram};
use nicegit_core::hosting::{Host, HostedRepository, ListedHosts};
use nicegit_core::{GitClient, GitError};

use crate::app::NiceGitApp;
use crate::settings::Workspace;
use crate::theme;
use crate::tools::{widgets, CheckOutRequest, Ctx, ToolWindow};

/// The remote whose repository is listed, as the pull request commands read it.
const REMOTE: &str = "origin";
/// The most repositories read at the same time.
const READERS: usize = 4;

/// What reading one repository found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The CLI answered for the repository: its open pull or merge requests, and the signed-in
    /// user's login. `kind` is the host service, GitHub or GitLab.
    Listed { kind: Host, slug: String, login: String, pull_requests: Vec<OpenPullRequest> },
    /// Why the repository cannot be listed, in words for the window.
    Unavailable(String),
}

/// A pull or merge request in the list, with the repository it belongs to and the user it is read for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    /// The repository's folder, which Check out switches to.
    pub path: PathBuf,
    /// `owner/name`, or `group/project` on GitLab.
    pub repository: String,
    /// The host service the request is on.
    pub kind: Host,
    /// The signed-in user's login on the repository's host.
    pub login: String,
    pub pull_request: OpenPullRequest,
}

/// The number as the row shows it: `#12` for GitHub, `!12` for GitLab, as the status bar does.
pub fn number_text(kind: Host, number: u64) -> String {
    match kind {
        Host::GitLab => format!("!{number}"),
        Host::GitHub | Host::Bitbucket => format!("#{number}"),
    }
}

/// Whether Check out is offered for a request. It is for GitHub pull requests only.
pub fn can_check_out(kind: Host) -> bool {
    kind == Host::GitHub
}

/// The programs and settings each read is given: the self-hosted hosts listed in Settings, the
/// `gh` and `glab` programs, and the logins read so far, shared by every read of the window.
#[derive(Clone)]
struct Sources {
    github_hosts: Vec<String>,
    gitlab_hosts: Vec<String>,
    gh: GhProgram,
    glab: GlabProgram,
    logins: Arc<Mutex<BTreeMap<String, String>>>,
}

/// The sections of the list, in the order they are shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    NeedsReview,
    Yours,
    Other,
}

impl Section {
    pub fn title(self) -> &'static str {
        match self {
            Section::NeedsReview => "Needs your review",
            Section::Yours => "Yours",
            Section::Other => "Other open",
        }
    }

    /// The section for a row. A pull request the user wrote is "Yours" even when a review of it
    /// was asked for; otherwise a review requested from the user, as a user and not a team, puts
    /// it in "Needs your review". Logins are compared without regard to case.
    pub fn of(row: &Row) -> Section {
        let me = row.login.as_str();
        let pull_request = &row.pull_request;
        if pull_request.author.as_deref().is_some_and(|author| author.eq_ignore_ascii_case(me)) {
            return Section::Yours;
        }
        let asked = pull_request
            .review_requests
            .iter()
            .any(|request| matches!(request, ReviewRequest::User(user) if user.eq_ignore_ascii_case(me)));
        if asked {
            Section::NeedsReview
        } else {
            Section::Other
        }
    }
}

/// The rows of each section, newest first. Rows updated at the same time keep a fixed order, by
/// repository and then number.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Groups {
    pub needs_review: Vec<Row>,
    pub yours: Vec<Row>,
    pub other: Vec<Row>,
}

/// Sorts rows into their sections.
pub fn group(rows: Vec<Row>) -> Groups {
    let mut groups = Groups::default();
    for row in rows {
        match Section::of(&row) {
            Section::NeedsReview => groups.needs_review.push(row),
            Section::Yours => groups.yours.push(row),
            Section::Other => groups.other.push(row),
        }
    }
    for list in [&mut groups.needs_review, &mut groups.yours, &mut groups.other] {
        list.sort_by(|a, b| {
            b.pull_request
                .updated_at
                .cmp(&a.pull_request.updated_at)
                .then_with(|| a.repository.cmp(&b.repository))
                .then_with(|| a.pull_request.number.cmp(&b.pull_request.number))
        });
    }
    groups
}

/// How long ago a pull request changed, as the row says it: "updated 5 minutes ago".
pub fn updated_text(updated_at: i64, now: i64) -> String {
    let elapsed = (now - updated_at).max(0);
    let (count, unit) = match elapsed {
        0..=59 => return "updated just now".to_string(),
        60..=3_599 => (elapsed / 60, "minute"),
        3_600..=86_399 => (elapsed / 3_600, "hour"),
        86_400..=2_591_999 => (elapsed / 86_400, "day"),
        _ => return "updated over a month ago".to_string(),
    };
    let unit = if count == 1 { unit.to_string() } else { format!("{unit}s") };
    format!("updated {count} {unit} ago")
}

/// The check totals as the row says them, such as "3 passing · 1 failing · 2 pending".
pub fn checks_text(checks: Checks) -> String {
    if checks == Checks::default() {
        return "No checks".to_string();
    }
    let mut parts = Vec::new();
    if checks.passed > 0 {
        parts.push(format!("{} passing", checks.passed));
    }
    if checks.failed > 0 {
        parts.push(format!("{} failing", checks.failed));
    }
    if checks.pending > 0 {
        parts.push(format!("{} pending", checks.pending));
    }
    parts.join(" · ")
}

/// Who was asked to review, as the row says it: users by login, teams by name.
pub fn requests_text(requests: &[ReviewRequest]) -> Option<String> {
    if requests.is_empty() {
        return None;
    }
    let names: Vec<String> = requests
        .iter()
        .map(|request| match request {
            ReviewRequest::User(login) => login.clone(),
            ReviewRequest::Team(name) => format!("team {name}"),
        })
        .collect();
    Some(format!("Review requested from {}", names.join(", ")))
}

/// The state of one repository in the list.
enum Reading {
    Loading,
    Done(Outcome),
}

struct Entry {
    path: PathBuf,
    reading: Reading,
}

/// One round of reads. Each repository's answer arrives on `receiver`; `cancel` stops the reads
/// still running, and `waiting` counts the answers not yet received.
struct Pass {
    receiver: Receiver<(usize, Outcome)>,
    cancel: Cancel,
    waiting: usize,
}

/// A user action on a row, run once the list has been drawn.
enum Action {
    Open(String),
    CheckOut(CheckOutRequest),
}

pub struct LaunchpadWindow {
    /// The repository tabs open when the window was made. Its list follows them.
    open: Vec<PathBuf>,
    /// The saved workspaces, as they were when the window was made.
    workspaces: Vec<Workspace>,
    /// The saved workspace whose folders are listed too, by name.
    workspace: Option<String>,
    hosts: Vec<String>,
    gitlab_hosts: Vec<String>,
    program: GhProgram,
    glab: GlabProgram,
    /// Logins read for each host, kept for the window's life and shared by the reads.
    logins: Arc<Mutex<BTreeMap<String, String>>>,
    entries: Vec<Entry>,
    pass: Option<Pass>,
    /// Set by Refresh and by a change of workspace, so the next frame reads again.
    read_again: bool,
}

impl LaunchpadWindow {
    /// The Launchpad for `open` repository folders and the saved `workspaces`. `hosts` and
    /// `gitlab_hosts` are the GitHub Enterprise and GitLab hosts listed in Settings.
    pub fn new(open: Vec<PathBuf>, workspaces: Vec<Workspace>, hosts: Vec<String>, gitlab_hosts: Vec<String>) -> Self {
        Self {
            open,
            workspaces,
            workspace: None,
            hosts,
            gitlab_hosts,
            program: GhProgram::Search,
            glab: GlabProgram::Search,
            logins: Arc::default(),
            entries: Vec::new(),
            pass: None,
            read_again: true,
        }
    }

    /// The Launchpad for the repository tabs and saved workspaces of `app`.
    pub fn for_app(app: &NiceGitApp) -> Self {
        let open = app.repos.iter().map(|repo| repo.path.clone()).collect();
        Self::new(open, app.settings.workspaces.clone(), app.settings.github_enterprise_hosts.clone(), app.settings.gitlab_hosts.clone())
    }

    /// Reads through `program` instead of the GitHub CLI on the search path. Tests give a missing file.
    #[cfg(test)]
    pub fn with_program(mut self, program: GhProgram) -> Self {
        self.program = program;
        self
    }

    /// Reads through `glab` instead of the GitLab CLI on the search path. Tests give a missing file.
    #[cfg(test)]
    pub fn with_glab_program(mut self, glab: GlabProgram) -> Self {
        self.glab = glab;
        self
    }

    /// The folders to read: the open tabs, then the chosen workspace's folders that exist and are
    /// not already listed.
    fn paths(&self) -> Vec<PathBuf> {
        let mut paths = self.open.clone();
        let chosen = self.workspace.as_ref().and_then(|name| self.workspaces.iter().find(|workspace| &workspace.name == name));
        for folder in chosen.map(|workspace| workspace.repositories.clone()).unwrap_or_default() {
            if folder.is_dir() && !paths.iter().any(|known| same_folder(known, &folder)) {
                paths.push(folder);
            }
        }
        paths
    }

    /// Starts reading every repository again. The reads of an earlier round are stopped.
    fn start(&mut self, ctx: &egui::Context) {
        if let Some(pass) = self.pass.take() {
            pass.cancel.cancel();
        }
        let paths = self.paths();
        self.entries = paths.iter().map(|path| Entry { path: path.clone(), reading: Reading::Loading }).collect();
        let waiting = paths.len();
        let queue = Arc::new(Mutex::new(paths.into_iter().enumerate().collect::<VecDeque<_>>()));
        let (sender, receiver) = channel();
        let cancel = Cancel::default();
        let sources = Sources {
            github_hosts: self.hosts.clone(),
            gitlab_hosts: self.gitlab_hosts.clone(),
            gh: self.program.clone(),
            glab: self.glab.clone(),
            logins: self.logins.clone(),
        };
        for _ in 0..READERS.min(waiting) {
            let (queue, sender, cancel, ctx, sources) = (queue.clone(), sender.clone(), cancel.clone(), ctx.clone(), sources.clone());
            std::thread::spawn(move || loop {
                if cancel.is_cancelled() {
                    break;
                }
                let next = queue.lock().ok().and_then(|mut pending| pending.pop_front());
                let Some((index, path)) = next else { break };
                let outcome = read(&GitClient::new(), &path, &sources, &cancel);
                if sender.send((index, outcome)).is_err() {
                    break;
                }
                ctx.request_repaint();
            });
        }
        self.pass = (waiting > 0).then_some(Pass { receiver, cancel, waiting });
    }

    /// Takes the answers that have arrived. A round that ends early, because a reader stopped,
    /// leaves its unread repositories as unavailable rather than loading for ever.
    fn receive(&mut self) {
        let Some(pass) = self.pass.as_mut() else { return };
        let mut stopped = false;
        loop {
            match pass.receiver.try_recv() {
                Ok((index, outcome)) => {
                    if let Some(entry) = self.entries.get_mut(index) {
                        entry.reading = Reading::Done(outcome);
                    }
                    pass.waiting = pass.waiting.saturating_sub(1);
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    stopped = true;
                    break;
                }
            }
        }
        let finished = pass.waiting == 0;
        if stopped {
            for entry in &mut self.entries {
                if matches!(entry.reading, Reading::Loading) {
                    entry.reading =
                        Reading::Done(Outcome::Unavailable("The read stopped before it finished. Refresh to try again.".into()));
                }
            }
        }
        if stopped || finished {
            self.pass = None;
        }
    }

    fn header(&mut self, ui: &mut Ui) {
        let c = theme::of(ui);
        ui.horizontal(|ui| {
            ui.label(RichText::new(icon::GIT_PULL_REQUEST).size(16.0).color(c.muted));
            ui.label(RichText::new("Launchpad").strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if widgets::icon_button(ui, icon::ARROWS_CLOCKWISE, "Refresh", true).clicked() {
                    self.read_again = true;
                }
            });
        });
        if self.workspaces.is_empty() {
            return;
        }
        ui.horizontal(|ui| {
            ui.label(RichText::new("Include workspace").small().color(c.muted));
            let shown = self.workspace.clone().unwrap_or_else(|| "None".to_string());
            let mut chosen = self.workspace.clone();
            egui::ComboBox::from_id_salt("launchpad workspace").selected_text(shown).show_ui(ui, |ui| {
                ui.selectable_value(&mut chosen, None, "None");
                for workspace in &self.workspaces {
                    ui.selectable_value(&mut chosen, Some(workspace.name.clone()), &workspace.name);
                }
            });
            if chosen != self.workspace {
                self.workspace = chosen;
                self.read_again = true;
            }
        });
    }

    fn body(&self, ui: &mut Ui, action: &mut Option<Action>) {
        let c = theme::of(ui);
        if self.entries.is_empty() {
            widgets::empty_state(
                ui,
                icon::GIT_PULL_REQUEST,
                "Open a repository, or choose a workspace, to see its pull and merge requests.",
            );
            return;
        }
        let now = now_seconds();
        let mut rows = Vec::new();
        let mut unavailable = Vec::new();
        let mut waiting = 0;
        let mut capped = false;
        for entry in &self.entries {
            match &entry.reading {
                Reading::Loading => waiting += 1,
                Reading::Done(Outcome::Unavailable(reason)) => unavailable.push((folder_name(&entry.path), reason.clone())),
                Reading::Done(Outcome::Listed { kind, slug, login, pull_requests }) => {
                    capped |= pull_requests.len() >= OPEN_PULL_REQUEST_LIMIT;
                    rows.extend(pull_requests.iter().map(|pull_request| Row {
                        path: entry.path.clone(),
                        repository: slug.clone(),
                        kind: *kind,
                        login: login.clone(),
                        pull_request: pull_request.clone(),
                    }));
                }
            }
        }
        let groups = group(rows);
        for (section, list) in
            [(Section::NeedsReview, &groups.needs_review), (Section::Yours, &groups.yours), (Section::Other, &groups.other)]
        {
            ui.add_space(6.0);
            ui.label(RichText::new(format!("{} ({})", section.title(), list.len())).strong());
            if list.is_empty() && waiting == 0 {
                ui.label(RichText::new("None").small().color(c.muted));
            }
            for row in list {
                row_card(ui, row, now, action);
            }
        }
        if waiting > 0 {
            ui.add_space(6.0);
            widgets::loading(ui, &format!("Loading {}", plural(waiting, "repository", "repositories")));
        }
        if capped {
            ui.label(
                RichText::new(format!("Showing the first {OPEN_PULL_REQUEST_LIMIT} open pull requests of each repository."))
                    .small()
                    .color(c.muted),
            );
        }
        if !unavailable.is_empty() {
            ui.add_space(10.0);
            ui.label(RichText::new("Not available").strong().color(c.muted));
            for (name, reason) in &unavailable {
                ui.add(egui::Label::new(RichText::new(format!("{name}: {reason}")).small().color(c.muted)).wrap());
            }
        }
    }
}

impl ToolWindow for LaunchpadWindow {
    fn id(&self) -> String {
        "launchpad".to_string()
    }

    fn title(&self) -> String {
        "Launchpad".to_string()
    }

    fn default_size(&self) -> egui::Vec2 {
        egui::vec2(700.0, 600.0)
    }

    fn ui(&mut self, ui: &mut Ui, cx: &mut Ctx) {
        if std::mem::take(&mut self.read_again) {
            let ctx = ui.ctx().clone();
            self.start(&ctx);
        }
        self.receive();
        self.header(ui);
        let mut action = None;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| self.body(ui, &mut action));
        match action {
            Some(Action::Open(url)) => crate::ui::changes::open_url(&url),
            Some(Action::CheckOut(request)) => cx.check_out_pull_request(request),
            None => {}
        }
    }
}

impl Drop for LaunchpadWindow {
    /// Closing the window stops the reads still running.
    fn drop(&mut self) {
        if let Some(pass) = self.pass.take() {
            pass.cancel.cancel();
        }
    }
}

/// One pull request row: its host, repository and number, title, review and draft state, author,
/// age, who was asked to review, checks, and the two actions. Check out is for GitHub only.
fn row_card(ui: &mut Ui, row: &Row, now: i64, action: &mut Option<Action>) {
    let c = theme::of(ui);
    let pull_request = &row.pull_request;
    let host_name = match row.kind {
        Host::GitLab => "GitLab",
        Host::GitHub | Host::Bitbucket => "GitHub",
    };
    egui::Frame::new()
        .fill(c.card_bg)
        .stroke(egui::Stroke::new(1.0, c.border))
        .corner_radius(6.0)
        .inner_margin(egui::Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                widgets::pill(ui, host_name, c.muted);
                ui.label(RichText::new(&row.repository).small().color(c.muted));
                ui.label(RichText::new(number_text(row.kind, pull_request.number)).monospace().color(c.muted));
            });
            ui.add(egui::Label::new(RichText::new(&pull_request.title).strong()).wrap());
            ui.horizontal_wrapped(|ui| {
                if pull_request.is_draft {
                    widgets::pill(ui, "draft", c.muted);
                }
                if let Some(decision) = pull_request.review_decision {
                    widgets::pill(ui, decision_word(decision), decision_color(decision, &c));
                }
                let by = match &pull_request.author {
                    Some(author) => format!("by {author}"),
                    None => "by a deleted account".to_string(),
                };
                ui.label(RichText::new(by).small().color(c.muted));
                ui.label(RichText::new(updated_text(pull_request.updated_at, now)).small().color(c.muted));
            });
            if let Some(text) = requests_text(&pull_request.review_requests) {
                ui.label(RichText::new(text).small().color(c.muted));
            }
            ui.label(RichText::new(checks_text(pull_request.checks)).small().color(checks_color(pull_request.checks, &c)));
            ui.horizontal(|ui| {
                if can_check_out(row.kind) {
                    if ui.button(format!("{}  Check out", icon::GIT_BRANCH)).clicked() {
                        *action = Some(Action::CheckOut(CheckOutRequest {
                            path: row.path.clone(),
                            remote: REMOTE.to_string(),
                            number: pull_request.number,
                            title: pull_request.title.clone(),
                            author: pull_request.author.clone().unwrap_or_default(),
                        }));
                    }
                } else {
                    ui.add_enabled(false, egui::Button::new(format!("{}  Check out", icon::GIT_BRANCH)))
                        .on_disabled_hover_text("Check out is available for GitHub pull requests only.");
                }
                if ui.button(format!("{}  Open in browser", icon::ARROW_SQUARE_OUT)).clicked() {
                    *action = Some(Action::Open(pull_request.url.clone()));
                }
            });
        });
    ui.add_space(6.0);
}

/// The words a review decision carries on its pill.
fn decision_word(decision: ReviewDecision) -> &'static str {
    match decision {
        ReviewDecision::Approved => "approved",
        ReviewDecision::ChangesRequested => "changes requested",
        ReviewDecision::ReviewRequired => "review required",
    }
}

/// The pill's colour for a review decision.
fn decision_color(decision: ReviewDecision, c: &theme::Colors) -> Color32 {
    match decision {
        ReviewDecision::Approved => c.added,
        ReviewDecision::ChangesRequested => c.removed,
        ReviewDecision::ReviewRequired => c.renamed,
    }
}

fn checks_color(checks: Checks, c: &theme::Colors) -> Color32 {
    if checks.failed > 0 {
        c.removed
    } else if checks.pending > 0 || checks == Checks::default() {
        c.muted
    } else {
        c.added
    }
}

/// Reads one repository. Its `origin` must be on GitHub or GitLab. The matching CLI must be installed
/// and signed in to that host, and then its open pull or merge requests are listed with the user's login.
fn read(client: &GitClient, path: &Path, sources: &Sources, cancel: &Cancel) -> Outcome {
    match read_checked(client, path, sources, cancel) {
        Ok(outcome) => outcome,
        Err(reason) => Outcome::Unavailable(reason),
    }
}

fn read_checked(client: &GitClient, path: &Path, sources: &Sources, cancel: &Cancel) -> Result<Outcome, String> {
    let remotes = client.run(&["remote"], path).map_err(|error| message(&error))?;
    if !remotes.lines().any(|line| line.trim() == REMOTE) {
        return Err(format!("This repository has no {REMOTE} remote, so its pull requests cannot be read."));
    }
    let address = client.run(&["remote", "get-url", "--all", REMOTE], path).map_err(|error| message(&error))?;
    let address = address.lines().next().unwrap_or("");
    let listed = ListedHosts { github: &sources.github_hosts, gitlab: &sources.gitlab_hosts };
    let hosted = HostedRepository::parse(address, listed).map_err(|error| message(&error))?;
    match hosted.kind {
        Host::GitHub => {
            let repository = GitHubRepository { host: hosted.host, owner: hosted.owner_path, name: hosted.name };
            read_github(client, path, &repository, sources, cancel)
        }
        Host::GitLab => read_gitlab(client, path, &hosted, sources, cancel),
        Host::Bitbucket => Err("Bitbucket pull requests are not listed here.".to_string()),
    }
}

/// Lists a GitHub repository's open pull requests through `gh`, after `gh` is checked for its host.
fn read_github(
    client: &GitClient,
    path: &Path,
    repository: &GitHubRepository,
    sources: &Sources,
    cancel: &Cancel,
) -> Result<Outcome, String> {
    if let Some(reason) = client.gh_status_with(&repository.host, &sources.gh, path).reason(&repository.host) {
        return Err(reason);
    }
    let login = cached_login(&sources.logins, &repository.host, || {
        client.github_login(&repository.host, &sources.gh, path).map_err(|error| message(&error))
    })?;
    let pull_requests = client.github_open_pull_requests(repository, &sources.gh, cancel, path).map_err(|error| message(&error))?;
    Ok(Outcome::Listed { kind: Host::GitHub, slug: repository.slug(), login, pull_requests })
}

/// Lists a GitLab repository's open merge requests through `glab`, after `glab` is checked for its host.
fn read_gitlab(
    client: &GitClient,
    path: &Path,
    repository: &HostedRepository,
    sources: &Sources,
    cancel: &Cancel,
) -> Result<Outcome, String> {
    if let Some(reason) = client.glab_status(repository, &sources.glab, path).reason(&repository.host) {
        return Err(reason);
    }
    let login = cached_login(&sources.logins, &repository.host, || {
        gitlab::glab_login(&repository.host, &sources.glab, path).map_err(|error| message(&error))
    })?;
    let pull_requests = gitlab::gitlab_open_merge_requests(repository, &sources.glab, cancel, path).map_err(|error| message(&error))?;
    Ok(Outcome::Listed { kind: Host::GitLab, slug: repository.slug(), login, pull_requests })
}

/// The signed-in login on `host`, read by `read` once and then kept in `logins`.
fn cached_login(
    logins: &Mutex<BTreeMap<String, String>>,
    host: &str,
    read: impl FnOnce() -> Result<String, String>,
) -> Result<String, String> {
    if let Some(login) = logins.lock().ok().and_then(|known| known.get(host).cloned()) {
        return Ok(login);
    }
    let login = read()?;
    if let Ok(mut known) = logins.lock() {
        known.insert(host.to_string(), login.clone());
    }
    Ok(login)
}

fn message(error: &GitError) -> String {
    match error {
        GitError::CommandFailed { message, .. } => message.clone(),
        other => other.to_string(),
    }
}

/// Whether two folders are the same, comparing their real locations when both exist.
fn same_folder(a: &Path, b: &Path) -> bool {
    a == b || matches!((a.canonicalize(), b.canonicalize()), (Ok(a), Ok(b)) if a == b)
}

fn folder_name(path: &Path) -> String {
    path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string())
}

fn now_seconds() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|elapsed| elapsed.as_secs() as i64).unwrap_or(0)
}

/// "1 repository" or "3 repositories".
fn plural(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pull_request(number: u64, author: Option<&str>, requests: Vec<ReviewRequest>, updated_at: i64) -> OpenPullRequest {
        OpenPullRequest {
            number,
            title: format!("Pull request {number}"),
            url: format!("https://github.com/acme/app/pull/{number}"),
            author: author.map(str::to_string),
            head_branch: "feature".into(),
            is_draft: false,
            review_decision: None,
            review_requests: requests,
            updated_at,
            checks: Checks::default(),
        }
    }

    fn row(login: &str, pull_request: OpenPullRequest) -> Row {
        Row { path: PathBuf::from("/work/app"), repository: "acme/app".into(), kind: Host::GitHub, login: login.into(), pull_request }
    }

    #[test]
    fn numbers_and_check_out_depend_on_the_host() {
        assert_eq!(number_text(Host::GitHub, 12), "#12");
        assert_eq!(number_text(Host::GitLab, 12), "!12", "a GitLab merge request is shown with !");
        assert!(can_check_out(Host::GitHub));
        assert!(!can_check_out(Host::GitLab), "Check out is not offered for GitLab");
    }

    #[test]
    fn a_gitlab_merge_request_is_grouped_by_its_reviewers_like_a_pull_request() {
        let asked = Row {
            path: PathBuf::from("/work/app"),
            repository: "group/app".into(),
            kind: Host::GitLab,
            login: "daniaal".into(),
            pull_request: pull_request(21, Some("alice"), vec![ReviewRequest::User("Daniaal".into())], 0),
        };
        assert_eq!(Section::of(&asked), Section::NeedsReview);
        let own = Row { pull_request: pull_request(22, Some("daniaal"), Vec::new(), 0), ..asked.clone() };
        assert_eq!(Section::of(&own), Section::Yours);
    }

    #[test]
    fn a_review_requested_from_the_user_puts_a_pull_request_in_needs_your_review() {
        let asked = pull_request(1, Some("alice"), vec![ReviewRequest::User("Daniaal".into())], 0);
        assert_eq!(Section::of(&row("daniaal", asked)), Section::NeedsReview, "logins compare without case");
    }

    #[test]
    fn a_review_requested_from_a_team_does_not_count_as_asked_of_the_user() {
        let team = pull_request(2, Some("alice"), vec![ReviewRequest::Team("Core".into())], 0);
        assert_eq!(Section::of(&row("daniaal", team)), Section::Other);
        let other_user = pull_request(3, Some("alice"), vec![ReviewRequest::User("bob".into())], 0);
        assert_eq!(Section::of(&row("daniaal", other_user)), Section::Other, "a review of someone else's request is not yours");
    }

    #[test]
    fn a_pull_request_the_user_wrote_is_yours_even_with_a_review_asked_of_them() {
        let own = pull_request(4, Some("Daniaal"), vec![ReviewRequest::User("daniaal".into())], 0);
        assert_eq!(Section::of(&row("daniaal", own)), Section::Yours);
        let own_plain = pull_request(5, Some("daniaal"), Vec::new(), 0);
        assert_eq!(Section::of(&row("daniaal", own_plain)), Section::Yours);
        let deleted = pull_request(6, None, Vec::new(), 0);
        assert_eq!(Section::of(&row("daniaal", deleted)), Section::Other, "a deleted author is nobody's");
    }

    #[test]
    fn rows_are_grouped_and_each_group_is_newest_first() {
        let rows = vec![
            row("daniaal", pull_request(10, Some("alice"), Vec::new(), 100)),
            row("daniaal", pull_request(11, Some("daniaal"), Vec::new(), 300)),
            row("daniaal", pull_request(12, Some("bob"), vec![ReviewRequest::User("daniaal".into())], 200)),
            row("daniaal", pull_request(13, Some("carol"), vec![ReviewRequest::User("daniaal".into())], 400)),
            row("daniaal", pull_request(14, Some("daniaal"), Vec::new(), 50)),
            row("daniaal", pull_request(15, Some("dave"), Vec::new(), 500)),
        ];
        let groups = group(rows);
        let numbers = |list: &[Row]| list.iter().map(|row| row.pull_request.number).collect::<Vec<_>>();
        assert_eq!(numbers(&groups.needs_review), [13, 12]);
        assert_eq!(numbers(&groups.yours), [11, 14]);
        assert_eq!(numbers(&groups.other), [15, 10]);
    }

    #[test]
    fn rows_updated_at_the_same_time_keep_a_fixed_order() {
        let rows = vec![
            Row {
                path: PathBuf::from("/b"),
                repository: "acme/b".into(),
                kind: Host::GitHub,
                login: "me".into(),
                pull_request: pull_request(2, None, Vec::new(), 7),
            },
            Row {
                path: PathBuf::from("/a"),
                repository: "acme/a".into(),
                kind: Host::GitHub,
                login: "me".into(),
                pull_request: pull_request(9, None, Vec::new(), 7),
            },
            Row {
                path: PathBuf::from("/a"),
                repository: "acme/a".into(),
                kind: Host::GitHub,
                login: "me".into(),
                pull_request: pull_request(3, None, Vec::new(), 7),
            },
        ];
        let groups = group(rows);
        let order: Vec<(String, u64)> = groups.other.iter().map(|row| (row.repository.clone(), row.pull_request.number)).collect();
        assert_eq!(order, [("acme/a".to_string(), 3), ("acme/a".to_string(), 9), ("acme/b".to_string(), 2)]);
    }

    #[test]
    fn updated_times_read_in_the_largest_whole_unit() {
        let now = 1_000_000;
        assert_eq!(updated_text(now, now), "updated just now");
        assert_eq!(updated_text(now - 59, now), "updated just now");
        assert_eq!(updated_text(now - 60, now), "updated 1 minute ago");
        assert_eq!(updated_text(now - 5 * 60, now), "updated 5 minutes ago");
        assert_eq!(updated_text(now - 3_600, now), "updated 1 hour ago");
        assert_eq!(updated_text(now - 26 * 3_600, now), "updated 1 day ago");
        assert_eq!(updated_text(now - 3 * 86_400, now), "updated 3 days ago");
        assert_eq!(updated_text(now - 40 * 86_400, now), "updated over a month ago");
        assert_eq!(updated_text(now + 500, now), "updated just now", "a clock behind the update is not negative");
    }

    #[test]
    fn check_totals_read_as_counts_and_say_when_there_are_none() {
        assert_eq!(checks_text(Checks::default()), "No checks");
        assert_eq!(checks_text(Checks { passed: 3, failed: 1, pending: 2 }), "3 passing · 1 failing · 2 pending");
        assert_eq!(checks_text(Checks { passed: 0, failed: 0, pending: 4 }), "4 pending");
    }

    #[test]
    fn review_requests_name_users_and_teams() {
        assert_eq!(requests_text(&[]), None);
        let both = [ReviewRequest::User("alice".into()), ReviewRequest::Team("Core".into())];
        assert_eq!(requests_text(&both).as_deref(), Some("Review requested from alice, team Core"));
    }

    #[test]
    fn a_review_decision_has_its_own_words() {
        assert_eq!(decision_word(ReviewDecision::Approved), "approved");
        assert_eq!(decision_word(ReviewDecision::ChangesRequested), "changes requested");
        assert_eq!(decision_word(ReviewDecision::ReviewRequired), "review required");
    }

    #[test]
    fn folders_are_the_same_only_when_they_are_the_same_place() {
        assert!(same_folder(Path::new("/work/app"), Path::new("/work/app")));
        assert!(!same_folder(Path::new("/work/app"), Path::new("/work/other")));
        assert!(!same_folder(Path::new("/nonexistent/nicegit-test/a"), Path::new("/nonexistent/nicegit-test/b")));
    }
}
