//! Branch checks: the CI status of each branch that has an open pull or merge request. A small dot
//! on the branch's label in the commit graph, and beside it in the sidebar, shows whether its checks
//! failed, are pending, or passed. The requests are read in the background through `gh`, `glab`, or
//! the Gitea, Forgejo, Bitbucket, or Azure DevOps API, for the active repository only, and only when its
//! remote is on one of those services, and the CLI is signed in where one is needed. Nothing here blocks
//! the interface.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui::{Color32, Pos2, Response, Sense, Ui};
use nicegit_core::gitea::BACKGROUND_PAGES;
use nicegit_core::github::{Cancel, Checks, GhProgram, GhStatus, OpenPullRequest, PrState, PullRequestStatus};
use nicegit_core::gitlab::{self, GlabProgram, GlabStatus};
use nicegit_core::hosting::ListedHosts;
use nicegit_core::{azure_devops, bitbucket, Branch, GitClient};

use crate::app::NiceGitApp;
use crate::forge_tokens;
use crate::git_ext::{self, Forge};
use crate::i18n::tr_fmt;
use crate::theme::Colors;
use crate::tools::Task;

/// How long a read of the open requests stands before the active repository's is read again, while
/// the window is focused.
const INTERVAL: Duration = Duration::from_secs(120);
/// How long a read waits before it is tried again while an action or dialog holds the repository.
const RETRY: Duration = Duration::from_secs(5);
/// The room a dot takes before a label's text, in points: the dot and the gap after it.
pub const DOT_SPACE: f32 = 10.0;
const DOT_RADIUS: f32 = 3.5;

/// An open request on the repository's remote, as the dot shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestCheck {
    pub number: u64,
    /// Without control characters, as the list gives it.
    pub title: String,
    /// The request's own page.
    pub url: String,
    pub checks: Checks,
    /// Whether the request is a draft, which its status bar shows as one.
    pub is_draft: bool,
}

/// The open requests of the active repository's remote, with the forge they were read from. `checks`
/// is keyed by the full ref of each branch the request comes from, such as `refs/heads/feature`.
pub struct OpenRequests {
    pub forge: Forge,
    pub checks: HashMap<String, RequestCheck>,
}

/// The read of the open requests, once it has arrived: `None` when nothing could be read.
pub type OpenRequestsRead = Task<Option<Arc<OpenRequests>>>;

/// How a request's checks stand, as the dot colours it: a failure is the news, then a pending check.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckState {
    Failed,
    Pending,
    Passed,
}

/// The dot for a branch: its state, the text a screen reader or a test finds, and its hover text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dot {
    pub state: CheckState,
    pub label: String,
    pub hover: String,
}

/// A branch that has an open request, with what its dot shows.
pub struct BranchCheck<'a> {
    pub forge: &'a Forge,
    pub request: &'a RequestCheck,
    pub dot: Dot,
}

/// How a request's checks stand, or `None` when it has no checks.
pub fn check_state(checks: Checks) -> Option<CheckState> {
    if checks.failed > 0 {
        Some(CheckState::Failed)
    } else if checks.pending > 0 {
        Some(CheckState::Pending)
    } else if checks.passed > 0 {
        Some(CheckState::Passed)
    } else {
        None
    }
}

/// The branches on `remote` that each open request comes from, as `(full ref, head branch)` pairs.
/// A local branch matches when its upstream is `refs/remotes/<remote>/<head>`, or, with no upstream,
/// when its name is the head. A remote-tracking branch matches when it is `refs/remotes/<remote>/<head>`.
/// Full refs are compared, so a branch name with slashes or ending in HEAD matches as any other.
pub fn match_branches(remote: &str, heads: &[&str], branches: &[Branch]) -> Vec<(String, String)> {
    let remote_ref = |head: &str| format!("refs/remotes/{remote}/{head}");
    let mut matches = Vec::new();
    for branch in branches {
        let head = if branch.is_remote {
            heads.iter().copied().find(|head| branch.full_ref() == remote_ref(head))
        } else {
            match branch.upstream.as_deref() {
                Some(upstream) => heads.iter().copied().find(|head| remote_ref(head) == upstream),
                None => heads.iter().copied().find(|head| *head == branch.name),
            }
        };
        if let Some(head) = head {
            matches.push((branch.full_ref(), head.to_string()));
        }
    }
    matches
}

/// The checks of each branch that has an open request on `remote`. A request from a fork is skipped,
/// as its branch is not one of this repository's. When two requests share a head branch, the first
/// listed is kept.
pub fn request_checks(remote: &str, requests: &[OpenPullRequest], branches: &[Branch]) -> HashMap<String, RequestCheck> {
    let mut by_head: HashMap<&str, &OpenPullRequest> = HashMap::new();
    for request in requests.iter().filter(|request| !request.is_cross_repository) {
        by_head.entry(request.head_branch.as_str()).or_insert(request);
    }
    let heads: Vec<&str> = by_head.keys().copied().collect();
    match_branches(remote, &heads, branches)
        .into_iter()
        .filter_map(|(full_ref, head)| {
            let request = by_head.get(head.as_str())?;
            let check = RequestCheck {
                number: request.number,
                title: request.title.clone(),
                url: request.url.clone(),
                checks: request.checks,
                is_draft: request.is_draft,
            };
            Some((full_ref, check))
        })
        .collect()
}

impl RequestCheck {
    /// The dot for this request on `forge`: `None` when it has no checks. Its hover text names the
    /// request as `#12` on GitHub, or `!12` on GitLab.
    pub fn dot(&self, forge: &Forge) -> Option<Dot> {
        let state = check_state(self.checks)?;
        let checks = self.checks;
        let marker = if matches!(forge, Forge::GitLab(_)) { "!" } else { "#" };
        let counts = match (checks.passed, checks.failed, checks.pending) {
            (passed, 0, 0) => tr_fmt!("{passed} passed", passed = passed),
            (passed, failed, 0) => tr_fmt!("{passed} passed, {failed} failed", passed = passed, failed = failed),
            (passed, failed, pending) => {
                tr_fmt!("{passed} passed, {failed} failed, {pending} pending", passed = passed, failed = failed, pending = pending)
            }
        };
        let label = match state {
            CheckState::Failed => tr_fmt!("Checks: {count} failed", count = checks.failed),
            CheckState::Pending => tr_fmt!("Checks: {count} pending", count = checks.pending),
            CheckState::Passed => tr_fmt!("Checks: {count} passed", count = checks.passed),
        };
        Some(Dot {
            state,
            label,
            hover: tr_fmt!(
                "{marker}{number} {title} — {counts}\nClick to open checks",
                marker = marker,
                number = self.number,
                title = self.title,
                counts = counts
            ),
        })
    }
}

/// The status bar's pull request for `branch`, taken from the open requests read for the branch dots,
/// when that read finished within the last two minutes and lists the branch's request on `forge`.
/// Nothing is read again then. `None` means the branch has to be asked for.
pub fn status_from_listing(
    read_at: Option<Instant>,
    read: Option<&OpenRequestsRead>,
    forge: &Forge,
    branch: &str,
) -> Option<PullRequestStatus> {
    if !read_at.is_some_and(|at| at.elapsed() < INTERVAL) {
        return None;
    }
    let requests = read?.peek().cloned().flatten()?;
    if requests.forge != *forge {
        return None;
    }
    let check = requests.checks.get(&format!("refs/heads/{branch}"))?;
    Some(PullRequestStatus {
        kind: forge.kind(),
        number: check.number,
        title: check.title.clone(),
        state: if check.is_draft { PrState::Draft } else { PrState::Open },
        url: check.url.clone(),
        checks: check.checks,
    })
}

/// The colour of a dot: a failure in the danger colour, a pending check in the warning colour, and
/// all passed in the added colour.
pub fn dot_color(state: CheckState, c: &Colors) -> Color32 {
    match state {
        CheckState::Failed => c.danger,
        CheckState::Pending => c.warning,
        CheckState::Passed => c.added,
    }
}

/// The check of the branch `branch` in `requests`, when it has an open request.
pub fn check_for<'a>(requests: Option<&'a OpenRequests>, branch: Option<&Branch>) -> Option<BranchCheck<'a>> {
    let requests = requests?;
    let request = requests.checks.get(&branch?.full_ref())?;
    let dot = request.dot(&requests.forge)?;
    Some(BranchCheck { forge: &requests.forge, request, dot })
}

/// Draws a dot centred on `center` and returns its response: clickable, named as the dot's label for
/// screen readers and tests, with its hover text.
pub fn show_dot(ui: &mut Ui, center: Pos2, id: egui::Id, dot: &Dot, c: &Colors) -> Response {
    let hit = egui::Rect::from_center_size(center, egui::vec2(DOT_SPACE + 2.0, 16.0));
    let response = ui.interact(hit, id, Sense::click()).on_hover_text(dot.hover.as_str());
    let label = dot.label.clone();
    response.widget_info(move || egui::WidgetInfo::labeled(egui::Role::Button, true, label.clone()));
    ui.painter().circle_filled(center, DOT_RADIUS, dot_color(dot.state, c));
    response
}

/// The open requests of `repository` read for a remote: the one remote, the forge behind it, and
/// the requests' checks for each branch. Runs `gh` or `glab`; it is called from a background task.
fn read_open_requests(
    remote: &str,
    forge: &Forge,
    branches: &[Branch],
    glab: &GlabProgram,
    path: &Path,
    token_file: Option<&Path>,
) -> Option<Arc<OpenRequests>> {
    let client = GitClient::new();
    let requests: Vec<OpenPullRequest> = match forge {
        Forge::GitHub(repository) => {
            if !matches!(client.gh_status(&repository.host, path), GhStatus::SignedIn) {
                return None;
            }
            client.github_open_pull_requests(repository, &GhProgram::Search, &Cancel::default(), path).ok()?
        }
        Forge::GitLab(repository) => {
            if !matches!(client.glab_status(repository, glab, path), GlabStatus::SignedIn) {
                return None;
            }
            gitlab::gitlab_open_merge_requests(repository, glab, &Cancel::default(), path).ok()?
        }
        Forge::Gitea(repository) => {
            // Public repositories are listed without a token; a saved or environment token is used when there is one.
            let token = forge_tokens::token_for(token_file, &repository.host);
            // A background read: at most two pages, so a long list does not hold the refresh.
            client.gitea_open_pull_requests(repository, token.as_deref(), BACKGROUND_PAGES, &Cancel::default(), path).ok()?
        }
        Forge::Bitbucket(repository) => {
            let credentials = forge_tokens::bitbucket_credentials(token_file);
            client
                .bitbucket_open_pull_requests(repository, credentials.as_deref(), bitbucket::BACKGROUND_PAGES, &Cancel::default(), path)
                .ok()?
        }
        Forge::AzureDevOps(repository) => {
            // Azure DevOps needs a token to list anything, so without one there are no dots.
            let token = forge_tokens::azure_token(token_file, azure_devops::organization(repository))?;
            client
                .azure_open_pull_requests(repository, Some(token.as_str()), azure_devops::BACKGROUND_PAGES, &Cancel::default(), path)
                .ok()?
        }
    };
    Some(Arc::new(OpenRequests { forge: forge.clone(), checks: request_checks(remote, &requests, branches) }))
}

impl NiceGitApp {
    /// The active repository's open requests, once read, when branch checks are shown. Nothing is
    /// shown while the setting is off.
    pub fn shown_requests(&self) -> Option<Arc<OpenRequests>> {
        if !self.settings.show_branch_checks {
            return None;
        }
        self.repo()?.open_requests.as_ref()?.peek().cloned().flatten()
    }

    /// Keeps the active repository's open requests current. The first read happens when the
    /// repository is shown, then again after a fetch, push, or pull, and at most every two minutes
    /// while the window is focused. Background tabs are never read. Nothing runs when the setting is
    /// off or the remote is not GitHub or GitLab.
    pub fn auto_open_requests(&mut self, ctx: &egui::Context) {
        let active = self.active;
        let focused = self.was_focused;
        let enabled = self.settings.show_branch_checks;
        let busy = self.busy.is_some() || self.dialog.is_some();
        let Some(repo) = self.repos.get_mut(active) else { return };
        // Taking the value in each frame lets a finished read arrive.
        if let Some(task) = repo.open_requests.as_mut() {
            task.get();
        }
        if !enabled || repo.snapshot.is_none() {
            return;
        }
        let now = Instant::now();
        if let Some(read) = repo.open_requests_at {
            if !focused {
                return;
            }
            if now < read + INTERVAL {
                ctx.request_repaint_after(read + INTERVAL - now);
                return;
            }
        }
        if busy || repo.loading {
            ctx.request_repaint_after(RETRY);
            return;
        }
        self.refresh_open_requests(active);
    }

    /// Starts reading the open requests of repository `index` in the background. The previous read
    /// stays on screen until the new one arrives. With no GitHub or GitLab remote, none is shown.
    pub fn refresh_open_requests(&mut self, index: usize) {
        let github_hosts = self.settings.github_enterprise_hosts.clone();
        let gitlab_hosts = self.settings.gitlab_hosts.clone();
        let gitea_hosts = self.settings.gitea_hosts.clone();
        let glab = self.glab.clone();
        let token_file = self.forge_token_file.clone();
        let context = self.worker.context.clone();
        let Some(repo) = self.repos.get_mut(index) else { return };
        repo.open_requests_at = Some(Instant::now());
        let stale = repo.open_requests.take().and_then(|mut task| task.get().cloned());
        let target = repo.snapshot.as_ref().and_then(|snapshot| {
            let hosts = ListedHosts { github: &github_hosts, gitlab: &gitlab_hosts, gitea: &gitea_hosts };
            let (remote, forge) = git_ext::listing_forge(snapshot, hosts)?;
            Some((remote, forge, snapshot.branches.clone()))
        });
        let Some((remote, forge, branches)) = target else { return };
        let path = repo.path.clone();
        repo.open_requests = Some(Task::spawn_with(&context, stale, move || {
            read_open_requests(&remote, &forge, &branches, &glab, &path, token_file.as_deref())
        }));
    }

    /// Opens what a request's dot leads to: the pull request's Checks tab on GitHub, and its page on
    /// GitLab and Gitea, where the pipeline or the statuses are.
    pub fn open_request_checks(&mut self, forge: &Forge, request: &RequestCheck) {
        match forge {
            Forge::GitHub(repository) => {
                let window =
                    crate::tools::pull_request_detail::PullRequestDetailWindow::new(repository.clone(), request.number).on_checks();
                self.open_tool(Box::new(window));
            }
            Forge::GitLab(_) | Forge::Gitea(_) | Forge::Bitbucket(_) | Forge::AzureDevOps(_) => crate::ui::changes::open_url(&request.url),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn branch(name: &str, is_remote: bool, upstream: Option<&str>) -> Branch {
        Branch {
            name: name.to_string(),
            is_current: false,
            is_remote,
            tip: "0123456".to_string(),
            subject: String::new(),
            upstream: upstream.map(str::to_string),
        }
    }

    fn remote_branch(name: &str) -> Branch {
        branch(&format!("remotes/{name}"), true, None)
    }

    #[test]
    fn a_local_branch_matches_its_upstream_on_the_same_remote() {
        let branches = [branch("feature", false, Some("refs/remotes/origin/feature"))];
        assert_eq!(match_branches("origin", &["feature"], &branches), [("refs/heads/feature".into(), "feature".into())]);
        // The name is not what matches: a branch tracking another branch on the remote is that branch's.
        let renamed = [branch("local", false, Some("refs/remotes/origin/feature"))];
        assert_eq!(match_branches("origin", &["feature"], &renamed), [("refs/heads/local".into(), "feature".into())]);
        assert!(match_branches("origin", &["local"], &renamed).is_empty(), "the name alone does not match a tracked branch");
    }

    #[test]
    fn a_local_branch_without_an_upstream_matches_by_name() {
        let branches = [branch("feature", false, None), branch("other", false, None)];
        assert_eq!(match_branches("origin", &["feature"], &branches), [("refs/heads/feature".into(), "feature".into())]);
        assert!(match_branches("origin", &["missing"], &branches).is_empty());
    }

    #[test]
    fn a_branch_tracking_another_remote_does_not_match() {
        let branches = [
            branch("feature", false, Some("refs/remotes/upstream/feature")),
            branch("sidebar", false, Some("refs/remotes/origin/sidebar")),
            remote_branch("upstream/feature"),
        ];
        assert!(match_branches("origin", &["feature"], &branches).is_empty(), "upstream/feature is another remote's");
        assert_eq!(match_branches("upstream", &["feature"], &branches).len(), 2, "the same branch on its own remote matches");
        assert_eq!(match_branches("origin", &["sidebar"], &branches), [("refs/heads/sidebar".into(), "sidebar".into())]);
    }

    #[test]
    fn a_remote_branch_matches_its_own_name_on_the_remote() {
        let branches = [remote_branch("origin/feature"), remote_branch("upstream/feature")];
        assert_eq!(match_branches("origin", &["feature"], &branches), [("refs/remotes/origin/feature".into(), "feature".into())]);
        assert_eq!(match_branches("upstream", &["feature"], &branches), [("refs/remotes/upstream/feature".into(), "feature".into())]);
    }

    #[test]
    fn branch_names_with_slashes_match_the_whole_name() {
        let branches = [
            branch("user/alice/fix", false, Some("refs/remotes/origin/user/alice/fix")),
            remote_branch("origin/user/alice/fix"),
            branch("fix", false, None),
        ];
        let heads = ["user/alice/fix", "fix"];
        let matched = match_branches("origin", &heads, &branches);
        assert_eq!(matched.len(), 3);
        assert!(matched.contains(&("refs/heads/user/alice/fix".into(), "user/alice/fix".into())));
        assert!(matched.contains(&("refs/remotes/origin/user/alice/fix".into(), "user/alice/fix".into())));
        assert!(matched.contains(&("refs/heads/fix".into(), "fix".into())));
        // A head that is only a prefix of a branch's name does not match it.
        assert!(match_branches("origin", &["user/alice"], &branches).is_empty());
    }

    #[test]
    fn branch_names_ending_in_head_match_like_other_names() {
        let branches = [
            branch("fix-HEAD", false, Some("refs/remotes/origin/fix-HEAD")),
            remote_branch("origin/feature/HEAD"),
            branch("HEAD", false, None),
        ];
        let matched = match_branches("origin", &["fix-HEAD", "feature/HEAD", "HEAD"], &branches);
        assert_eq!(matched.len(), 3, "each branch ending in HEAD matches its own head");
        assert!(matched.contains(&("refs/remotes/origin/feature/HEAD".into(), "feature/HEAD".into())));
        assert!(matched.contains(&("refs/heads/HEAD".into(), "HEAD".into())));
    }

    #[test]
    fn requests_from_forks_are_skipped_and_the_rest_keep_their_checks() {
        let request = |number: u64, head: &str, fork: bool, checks: Checks| OpenPullRequest {
            number,
            title: format!("Title {number}"),
            url: format!("https://github.com/acme/app/pull/{number}"),
            author: None,
            head_branch: head.to_string(),
            is_cross_repository: fork,
            is_draft: false,
            review_decision: None,
            review_requests: Vec::new(),
            updated_at: 0,
            checks,
        };
        let failing = Checks { passed: 2, failed: 1, pending: 0 };
        let requests = [request(12, "feature", false, failing), request(13, "fork-branch", true, failing)];
        let branches = [branch("feature", false, None), branch("fork-branch", false, None)];
        let checks = request_checks("origin", &requests, &branches);
        assert_eq!(checks.len(), 1, "the fork's branch is not this repository's");
        assert_eq!(checks["refs/heads/feature"].number, 12);
        assert_eq!(checks["refs/heads/feature"].checks, failing);
    }

    #[test]
    fn a_dot_names_the_worst_state_and_says_nothing_without_checks() {
        let forge =
            Forge::GitHub(nicegit_core::github::GitHubRepository { host: "github.com".into(), owner: "acme".into(), name: "app".into() });
        let check = |checks: Checks| RequestCheck { number: 12, title: "Add it".into(), url: String::new(), checks, is_draft: false };
        let failed = check(Checks { passed: 3, failed: 1, pending: 0 }).dot(&forge).unwrap();
        assert_eq!(failed.state, CheckState::Failed);
        assert_eq!(failed.label, "Checks: 1 failed");
        assert_eq!(failed.hover, "#12 Add it — 3 passed, 1 failed\nClick to open checks");
        let pending = check(Checks { passed: 1, failed: 0, pending: 2 }).dot(&forge).unwrap();
        assert_eq!((pending.state, pending.label.as_str()), (CheckState::Pending, "Checks: 2 pending"));
        let passed = check(Checks { passed: 4, failed: 0, pending: 0 }).dot(&forge).unwrap();
        assert_eq!((passed.state, passed.label.as_str()), (CheckState::Passed, "Checks: 4 passed"));
        assert!(check(Checks::default()).dot(&forge).is_none(), "no checks, no dot");
    }

    #[test]
    fn a_fresh_listing_with_the_branch_answers_the_status_bar_and_a_stale_one_does_not() {
        let forge = Forge::Gitea(nicegit_core::hosting::HostedRepository {
            host: "codeberg.org".into(),
            kind: nicegit_core::hosting::Host::Gitea,
            owner_path: "acme".into(),
            name: "app".into(),
        });
        let mut checks = HashMap::new();
        checks.insert(
            "refs/heads/feature".to_string(),
            RequestCheck {
                number: 9,
                title: "Feature".into(),
                url: "https://codeberg.org/acme/app/pulls/9".into(),
                checks: Checks { passed: 2, failed: 0, pending: 0 },
                is_draft: true,
            },
        );
        let read = Task::ready(Some(Arc::new(OpenRequests { forge: forge.clone(), checks })));
        let fresh = Some(Instant::now());
        let status = status_from_listing(fresh, Some(&read), &forge, "feature").expect("the listing has the branch");
        assert_eq!((status.number, status.state), (9, PrState::Draft));
        assert!(status_from_listing(fresh, Some(&read), &forge, "other").is_none(), "a branch not listed is asked for");
        let stale = Some(Instant::now() - Duration::from_secs(300));
        assert!(status_from_listing(stale, Some(&read), &forge, "feature").is_none(), "an old read is not used");
        let other = Forge::Gitea(nicegit_core::hosting::HostedRepository {
            host: "gitea.com".into(),
            kind: nicegit_core::hosting::Host::Gitea,
            owner_path: "acme".into(),
            name: "app".into(),
        });
        assert!(status_from_listing(fresh, Some(&read), &other, "feature").is_none(), "another repository's listing is not used");
    }

    #[test]
    fn a_gitlab_request_is_named_by_its_bang_number() {
        let repository = nicegit_core::hosting::HostedRepository {
            host: "gitlab.com".into(),
            kind: nicegit_core::hosting::Host::GitLab,
            owner_path: "group".into(),
            name: "proj".into(),
        };
        let forge = Forge::GitLab(repository);
        let check = RequestCheck {
            number: 7,
            title: "Merge".into(),
            url: String::new(),
            checks: Checks { passed: 1, failed: 0, pending: 0 },
            is_draft: false,
        };
        assert!(check.dot(&forge).unwrap().hover.starts_with("!7 Merge"));
    }
}
