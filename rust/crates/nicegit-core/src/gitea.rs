//! Gitea and Forgejo pull requests and issues, read and created through their REST API with the
//! system `curl`, as the update check and the commit drafts reach their services. Both servers share
//! one API, so one set of requests serves codeberg.org, gitea.com, and the hosts listed in Settings.
//!
//! A token is passed to curl on its standard input, as a config file, so it never appears in the
//! process arguments. Without a token, public repositories can still be read; creating a pull request
//! needs one. Each answer is decoded strictly, and every text is cleaned before it is shown.

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use crate::ai_message::json_string;
use crate::client::GitClient;
use crate::forge_api::{
    self, choose_branch_pull, is_commit_id, item_of_pull, login_of, open_request, optional_str, status_of, web_link, whole_number,
    Exchange, PullEntry,
};
use crate::github::{
    self, clean_text, timestamp_seconds, Cancel, Checks, GitHubItem, GitHubLabel, ItemKind, OpenPullRequest, PrState, PullRequestStatus,
    ReviewRequest,
};
use crate::gitlab::valid_username;
use crate::hosting::{Host, HostedRepository, ListedHosts};
use crate::json::{parse_json, Json};
use crate::models::{GitError, Result};

/// The name used in errors for this service.
const SERVICE: &str = "Gitea / Forgejo";
/// The longest title kept, in characters.
const TITLE_LIMIT: usize = 300;
/// The longest team name kept, in characters.
const TEAM_NAME_LIMIT: usize = 100;
/// The most items the API returns in one page.
pub const PAGE_LIMIT: usize = 50;
/// The most pages read for one listing, so a long list costs a bounded number of requests.
pub const MAX_PAGES: usize = 4;
/// How many pages a background refresh reads of a pull request list, such as the status bar's and the
/// branch checks'. The Launchpad and the sidebar's explicit loads read [`MAX_PAGES`].
pub const BACKGROUND_PAGES: usize = 2;
/// How many commit statuses are read at the same time.
pub const CONCURRENT_STATUS_READS: usize = 6;
/// Longest error message from the server shown to the person.
const MESSAGE_LIMIT: usize = 300;

/// The address of the API for a repository, such as `https://codeberg.org/api/v1/repos/acme/app/pulls`.
fn repository_api(repository: &HostedRepository, path: &str) -> String {
    format!("{}/api/v1/repos/{}{path}", repository.web_base(), repository.slug())
}

/// Runs curl for one request with a token, which is sent as `Authorization: token <token>`.
fn exchange(directory: &Path, method: &str, url: &str, token: Option<&str>, body: Option<&str>) -> Exchange {
    let authorization = token.map(|token| format!("token {token}"));
    forge_api::exchange(directory, method, url, authorization.as_deref(), body)
}

/// The error for an answer that is not a success, in words for the person. The server's own message
/// is kept, cleaned, for the statuses that have no fixed wording.
fn status_failure(host: &str, status: u16, body: &str) -> GitError {
    let text = match status {
        401 => format!("The token for {host} was not accepted."),
        404 => format!("The repository was not found on {host}, or the token cannot see it."),
        _ => {
            let server = parse_json(body).ok().and_then(|value| value.get("message").and_then(Json::as_str).map(str::to_string));
            let detail = server.map(|message| format!(" {}", clean_text(&message, MESSAGE_LIMIT))).unwrap_or_default();
            format!("{host} could not complete the request (status {status}).{detail}")
        }
    };
    GitError::failed(SERVICE, text)
}

/// Makes one request and decodes its JSON answer. Transport failures and error statuses become
/// the plain words the Settings and dialogs show.
fn request_json(host: &str, method: &str, url: &str, token: Option<&str>, body: Option<&str>, directory: &Path) -> Result<Json> {
    match exchange(directory, method, url, token, body) {
        Exchange::NoCurl => Err(GitError::failed(SERVICE, "curl is needed to reach Gitea or Forgejo, and it is not installed.")),
        Exchange::Unreachable => Err(GitError::failed(SERVICE, format!("Could not reach {host}."))),
        Exchange::Answer { status, body } => answer_json(host, status, &body),
    }
}

/// The decoded body of a successful answer, or the error for any other status.
fn answer_json(host: &str, status: u16, body: &str) -> Result<Json> {
    if !(200..300).contains(&status) {
        return Err(status_failure(host, status, body));
    }
    parse_json(body).map_err(|_| GitError::failed(SERVICE, format!("{host} returned an answer that was not expected.")))
}

/// The unexpected-answer error for a list or a single item.
fn unexpected(what: &str) -> GitError {
    GitError::failed(SERVICE, format!("Gitea or Forgejo returned unexpected {what}."))
}

/// Whether a pull request is a draft. The `draft` flag is read when the server sends it; older
/// servers mark drafts with a work-in-progress prefix on the title.
fn is_draft(entry: &Json, title: &str) -> std::result::Result<bool, ()> {
    let flag = match entry.get("draft") {
        None | Some(Json::Null) => None,
        Some(value) => Some(value.as_bool().ok_or(())?),
    };
    let lowered = title.trim_start().to_ascii_lowercase();
    let prefixed = ["wip:", "[wip]", "draft:"].iter().any(|prefix| lowered.starts_with(prefix));
    Ok(flag.unwrap_or(prefixed))
}

/// One pull request in a list, decoded strictly. Its link is kept only when it is this repository's
/// own page; the title is cleaned; and a head repository that is not this one marks a fork.
fn decode_pull_entry(entry: &Json, repository: &HostedRepository) -> Result<PullEntry> {
    let Json::Object(_) = entry else { return Err(unexpected("pull requests")) };
    let number = whole_number(entry.get("number")).ok_or_else(|| unexpected("pull requests"))?;
    let title = entry.get("title").and_then(Json::as_str).ok_or_else(|| unexpected("pull requests"))?;
    let title = clean_text(title, TITLE_LIMIT);
    let link = optional_str(entry, "html_url").map_err(|_| unexpected("pull requests"))?;
    let url = web_link(repository, "pulls", number, link);
    let merged = match entry.get("merged") {
        None | Some(Json::Null) => false,
        Some(value) => value.as_bool().ok_or_else(|| unexpected("pull requests"))?,
    };
    let state = match entry.get("state").and_then(Json::as_str) {
        Some("open") => {
            if is_draft(entry, &title).map_err(|_| unexpected("pull requests"))? {
                PrState::Draft
            } else {
                PrState::Open
            }
        }
        Some("closed") if merged => PrState::Merged,
        Some("closed") => PrState::Closed,
        _ => return Err(unexpected("pull requests")),
    };
    let head = entry.get("head").filter(|head| matches!(head, Json::Object(_))).ok_or_else(|| unexpected("pull requests"))?;
    let head_branch = head.get("ref").and_then(Json::as_str).ok_or_else(|| unexpected("pull requests"))?;
    let head_sha = optional_str(head, "sha").map_err(|_| unexpected("pull requests"))?.map(str::to_string);
    // A head repository that is missing, or named otherwise, is not this repository's branch.
    let is_cross_repository = match head.get("repo") {
        None | Some(Json::Null) => true,
        Some(repo) => {
            let full_name = repo.get("full_name").and_then(Json::as_str).unwrap_or_default();
            !full_name.eq_ignore_ascii_case(&repository.slug())
        }
    };
    let mut review_requests: Vec<ReviewRequest> = match entry.get("requested_reviewers") {
        None | Some(Json::Null) => Vec::new(),
        Some(Json::Array(users)) => users.iter().filter_map(|user| login_of(Some(user)).map(ReviewRequest::User)).collect(),
        Some(_) => return Err(unexpected("pull requests")),
    };
    match entry.get("requested_reviewers_teams") {
        None | Some(Json::Null) => {}
        Some(Json::Array(teams)) => review_requests.extend(teams.iter().filter_map(|team| {
            let name = clean_text(team.get("name").and_then(Json::as_str)?, TEAM_NAME_LIMIT);
            (!name.is_empty()).then_some(ReviewRequest::Team(name))
        })),
        Some(_) => return Err(unexpected("pull requests")),
    }
    let updated_at = match optional_str(entry, "updated_at").map_err(|_| unexpected("pull requests"))? {
        None => 0,
        Some(text) => timestamp_seconds(text).ok_or_else(|| unexpected("pull requests"))?,
    };
    Ok(PullEntry {
        number,
        title,
        url,
        state,
        author: login_of(entry.get("user")),
        head_branch: clean_text(head_branch, TITLE_LIMIT),
        head_sha,
        is_cross_repository,
        review_requests,
        updated_at,
    })
}

/// The pull requests of a list answer, each decoded strictly. One bad entry refuses the list.
fn decode_pull_entries(value: &Json, repository: &HostedRepository) -> Result<Vec<PullEntry>> {
    let Json::Array(entries) = value else { return Err(unexpected("pull requests")) };
    entries.iter().map(|entry| decode_pull_entry(entry, repository)).collect()
}

/// The checks of a commit from its combined status. Each context's latest status counts once: passed
/// for success and warning, failed for failure and error, and pending for anything else. A commit
/// with no statuses has no checks.
pub(crate) fn decode_checks(value: &Json) -> Result<Checks> {
    let statuses = match value.get("statuses") {
        None | Some(Json::Null) => return Ok(Checks::default()),
        Some(Json::Array(statuses)) => statuses,
        Some(_) => return Err(unexpected("commit statuses")),
    };
    let mut checks = Checks::default();
    for status in statuses {
        let word = status.get("status").or_else(|| status.get("state")).and_then(Json::as_str).unwrap_or_default();
        match word {
            "success" | "warning" => checks.passed += 1,
            "failure" | "error" => checks.failed += 1,
            _ => checks.pending += 1,
        }
    }
    Ok(checks)
}

/// The checks of one commit, read from its combined status. A commit whose status cannot be read has
/// none, so a list still shows. A commit id that is not hexadecimal is never put in a request path.
fn commit_checks(repository: &HostedRepository, token: Option<&str>, sha: &str, directory: &Path) -> Checks {
    if !is_commit_id(sha) {
        return Checks::default();
    }
    let url = repository_api(repository, &format!("/commits/{sha}/status"));
    request_json(&repository.host, "GET", &url, token, None, directory).ok().and_then(|json| decode_checks(&json).ok()).unwrap_or_default()
}

/// The checks of each distinct commit in `shas`, keyed by id. Up to [`CONCURRENT_STATUS_READS`] reads
/// run at once on scoped threads, and a commit named twice is read once.
fn checks_for_commits(repository: &HostedRepository, token: Option<&str>, shas: &[&str], directory: &Path) -> HashMap<String, Checks> {
    let mut distinct: Vec<&str> = Vec::new();
    for &sha in shas {
        if !distinct.contains(&sha) {
            distinct.push(sha);
        }
    }
    let next = AtomicUsize::new(0);
    let results = Mutex::new(HashMap::new());
    let workers = CONCURRENT_STATUS_READS.min(distinct.len());
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| loop {
                let index = next.fetch_add(1, Ordering::Relaxed);
                let Some(sha) = distinct.get(index) else { break };
                let checks = commit_checks(repository, token, sha, directory);
                if let Ok(mut found) = results.lock() {
                    found.insert((*sha).to_string(), checks);
                }
            });
        }
    });
    results.into_inner().unwrap_or_default()
}

/// What a direct lookup of one pull request found.
enum Direct {
    Found(PullEntry),
    Missing,
    /// The server has no such route, so the list must be read instead.
    Unsupported,
}

/// Whether a 404 answer says the pull request is missing: a JSON message such as "The target couldn't
/// be found." A plain-text 404, such as an unknown route, says nothing of the sort.
fn is_missing_pull(body: &str) -> bool {
    parse_json(body)
        .ok()
        .and_then(|value| value.get("message").and_then(Json::as_str).map(str::to_ascii_lowercase))
        .is_some_and(|message| message.contains("couldn't be found") || message.contains("does not exist") || message.contains("not found"))
}

/// A path segment with every byte outside the unreserved set percent-encoded, so a branch name with
/// slashes stays one segment.
fn encode_segment(text: &str) -> String {
    forge_api::encode_component(text)
}

/// Decodes one issue from a list. Pull requests that the issue list might carry are skipped.
fn decode_issue(entry: &Json, repository: &HostedRepository) -> Result<Option<GitHubItem>> {
    let Json::Object(_) = entry else { return Err(unexpected("issues")) };
    if entry.get("pull_request").is_some_and(|value| *value != Json::Null) {
        return Ok(None);
    }
    let number = whole_number(entry.get("number")).ok_or_else(|| unexpected("issues"))?;
    let title = entry.get("title").and_then(Json::as_str).ok_or_else(|| unexpected("issues"))?;
    let link = optional_str(entry, "html_url").map_err(|_| unexpected("issues"))?;
    let state = match entry.get("state").and_then(Json::as_str) {
        Some("open") => Some("OPEN".to_string()),
        Some("closed") => Some("CLOSED".to_string()),
        _ => None,
    };
    let mut labels = Vec::new();
    match entry.get("labels") {
        None | Some(Json::Null) => {}
        Some(Json::Array(values)) => {
            for label in values {
                let Some(name) = label.get("name").and_then(Json::as_str) else { continue };
                let color = label.get("color").and_then(Json::as_str).unwrap_or_default().trim_start_matches('#').to_string();
                labels.push(GitHubLabel { name: clean_text(name, TITLE_LIMIT), color });
            }
        }
        Some(_) => return Err(unexpected("issues")),
    }
    Ok(Some(GitHubItem {
        number,
        title: clean_text(title, TITLE_LIMIT),
        url: web_link(repository, "issues", number, link),
        author: login_of(entry.get("user")),
        state,
        is_draft: false,
        labels,
    }))
}

/// Decodes a list of issues. Each entry is checked; the list is refused when one is not an issue.
pub(crate) fn decode_issues(value: &Json, repository: &HostedRepository) -> Result<Vec<GitHubItem>> {
    let Json::Array(entries) = value else { return Err(unexpected("issues")) };
    let mut items = Vec::with_capacity(entries.len());
    for entry in entries {
        items.extend(decode_issue(entry, repository)?);
    }
    Ok(items)
}

/// The default branch of a repository from its answer. Its name must be a non-empty string.
pub(crate) fn decode_default_branch(value: &Json) -> Result<String> {
    value
        .get("default_branch")
        .and_then(Json::as_str)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .ok_or_else(|| GitError::failed(SERVICE, "The repository has no default branch."))
}

/// The signed-in login from `/api/v1/user`. It must be a plain username.
pub(crate) fn decode_login(value: &Json) -> Result<String> {
    value
        .get("login")
        .and_then(Json::as_str)
        .filter(|login| valid_username(login))
        .map(str::to_string)
        .ok_or_else(|| GitError::failed(SERVICE, "Gitea or Forgejo did not report your login."))
}

/// The address of a new pull request from its answer, which is the pull request itself.
pub(crate) fn decode_created_url(value: &Json, repository: &HostedRepository) -> Result<String> {
    decode_pull_entry(value, repository).map(|entry| entry.url)
}

/// Whether a token can be sent in a header: printable ASCII, no spaces, and not too long.
pub fn is_plausible_token(text: &str) -> bool {
    !text.is_empty() && text.len() <= 512 && text.bytes().all(|byte| byte.is_ascii_graphic())
}

impl GitClient {
    /// The repository behind a remote's first fetch address, on Gitea, Forgejo, or a host listed in
    /// Settings as one.
    pub fn hosted_remote_repository(&self, remote: &str, directory: &Path, hosts: ListedHosts) -> Result<HostedRepository> {
        if remote.is_empty() || remote.starts_with('-') {
            return Err(GitError::failed(SERVICE, "Choose a remote first."));
        }
        let output = self.run(&["remote", "get-url", "--all", remote], directory)?;
        HostedRepository::parse(output.lines().next().unwrap_or(""), hosts)
    }

    /// The open pull requests of a repository, with the checks of each head commit, for the Launchpad
    /// and branch checks. The list is capped at [`github::OPEN_PULL_REQUEST_LIMIT`] and at most
    /// `max_pages` pages are read. A commit whose status cannot be read has no checks, so the list still shows.
    pub fn gitea_open_pull_requests(
        &self,
        repository: &HostedRepository,
        token: Option<&str>,
        max_pages: usize,
        cancel: &Cancel,
        directory: &Path,
    ) -> Result<Vec<OpenPullRequest>> {
        let limit = github::OPEN_PULL_REQUEST_LIMIT;
        let entries = self.read_pulls(repository, token, "open", limit, max_pages, cancel, directory, |_| false)?;
        // Each distinct head commit is read once, several at a time; pull requests sharing a head share its checks.
        let heads: Vec<&str> = entries.iter().filter_map(|entry| entry.head_sha.as_deref()).collect();
        let checks = checks_for_commits(repository, token, &heads, directory);
        Ok(entries
            .into_iter()
            .map(|entry| {
                let found = entry.head_sha.as_deref().and_then(|sha| checks.get(sha).copied()).unwrap_or_default();
                open_request(entry, found)
            })
            .collect())
    }

    /// The pull request whose head is `branch` on this repository, open or closed, with its checks. A
    /// branch with no pull request gives `None`. The repository's default branch is the base the
    /// request is looked up for. Gitea and Forgejo answer that lookup directly; only a server without
    /// the route falls back to the open list, read at most [`BACKGROUND_PAGES`] pages and stopping at
    /// the first match. The branch name is checked first.
    pub fn gitea_pull_request_for_branch(
        &self,
        repository: &HostedRepository,
        token: Option<&str>,
        branch: &str,
        directory: &Path,
    ) -> Result<Option<PullRequestStatus>> {
        self.check_branch_name(branch, directory)?;
        let base = self.gitea_default_branch(repository, token, directory)?;
        let wanted = |entry: &PullEntry| entry.head_branch == branch && !entry.is_cross_repository;
        let open_listing = |this: &GitClient| -> Result<Option<PullEntry>> {
            let entries =
                this.read_pulls(repository, token, "open", usize::MAX, BACKGROUND_PAGES, &Cancel::default(), directory, wanted)?;
            Ok(choose_branch_pull(entries, branch))
        };
        let entry = match self.direct_pull(repository, token, &base, branch, directory)? {
            Direct::Found(entry) if matches!(entry.state, PrState::Open | PrState::Draft) => Some(entry),
            // The lookup returns one pull request for the branch, which can be a closed one while an open
            // one exists; the open list settles it, and the closed one stands when there is none.
            Direct::Found(closed) => open_listing(self)?.or(Some(closed)),
            Direct::Missing => None,
            Direct::Unsupported => open_listing(self)?,
        };
        let Some(entry) = entry else { return Ok(None) };
        let checks = self.head_checks(repository, token, entry.head_sha.as_deref(), directory);
        Ok(Some(status_of(Host::Gitea, entry, checks)))
    }

    /// One pull request by base and head, from `GET /pulls/{base}/{head}`. A 404 whose answer is a JSON
    /// message is no pull request; a 404 without one means the server has no such route, so the caller
    /// falls back to the list. A head that is on a fork is not this repository's, so it is no pull request.
    fn direct_pull(&self, repository: &HostedRepository, token: Option<&str>, base: &str, head: &str, directory: &Path) -> Result<Direct> {
        let path = format!("/pulls/{}/{}", encode_segment(base), encode_segment(head));
        let url = repository_api(repository, &path);
        match exchange(directory, "GET", &url, token, None) {
            Exchange::NoCurl => Err(GitError::failed(SERVICE, "curl is needed to reach Gitea or Forgejo, and it is not installed.")),
            Exchange::Unreachable => Err(GitError::failed(SERVICE, format!("Could not reach {}.", repository.host))),
            Exchange::Answer { status: 200, body } => {
                let json = parse_json(&body).map_err(|_| unexpected("pull requests"))?;
                let entry = decode_pull_entry(&json, repository)?;
                if entry.is_cross_repository || entry.head_branch != head {
                    return Ok(Direct::Missing);
                }
                Ok(Direct::Found(entry))
            }
            Exchange::Answer { status: 404, body } => Ok(if is_missing_pull(&body) { Direct::Missing } else { Direct::Unsupported }),
            Exchange::Answer { status, body } => Err(status_failure(&repository.host, status, &body)),
        }
    }

    /// Pull requests or issues of a repository, newest first where the server sorts them, without
    /// their checks. `limit` caps the list, and at most [`MAX_PAGES`] pages are read.
    pub fn gitea_items(
        &self,
        repository: &HostedRepository,
        kind: ItemKind,
        token: Option<&str>,
        limit: usize,
        directory: &Path,
    ) -> Result<Vec<GitHubItem>> {
        match kind {
            ItemKind::PullRequest => {
                let entries = self.read_pulls(repository, token, "open", limit, MAX_PAGES, &Cancel::default(), directory, |_| false)?;
                Ok(entries.into_iter().map(item_of_pull).collect())
            }
            ItemKind::Issue => {
                let query = "type=issues&state=open";
                let mut items = Vec::new();
                for page in 1..=MAX_PAGES {
                    let json = self.api_get(repository, token, &format!("/issues?{query}&limit={PAGE_LIMIT}&page={page}"), directory)?;
                    let count = match &json {
                        Json::Array(entries) => entries.len(),
                        _ => 0,
                    };
                    items.extend(decode_issues(&json, repository)?);
                    if count < PAGE_LIMIT || items.len() >= limit {
                        break;
                    }
                }
                items.truncate(limit);
                Ok(items)
            }
        }
    }

    /// The repository's default branch, read without a token when the repository is public. The
    /// name is checked as a branch name before it is returned.
    pub fn gitea_default_branch(&self, repository: &HostedRepository, token: Option<&str>, directory: &Path) -> Result<String> {
        let json = self.api_get(repository, token, "", directory)?;
        let name = decode_default_branch(&json)?;
        self.check_branch_name(&name, directory)?;
        Ok(name)
    }

    /// The login of the user whose token is given, read from `/api/v1/user` on `host`.
    pub fn gitea_login(&self, host: &str, token: &str, directory: &Path) -> Result<String> {
        let url = format!("https://{host}/api/v1/user");
        let json = request_json(host, "GET", &url, Some(token), None, directory)?;
        decode_login(&json)
    }

    /// Creates a pull request from `head` into `base` and returns its address. Creating needs a token,
    /// and Gitea and Forgejo have no draft pull requests, so none is made. The title is one line and
    /// the branch names are checked before the request is made.
    #[allow(clippy::too_many_arguments)]
    pub fn create_gitea_pull_request(
        &self,
        repository: &HostedRepository,
        token: Option<&str>,
        head: &str,
        base: &str,
        title: &str,
        body: &str,
        directory: &Path,
    ) -> Result<String> {
        let title = title.trim();
        if title.is_empty() {
            return Err(GitError::failed(SERVICE, "Enter a title for the pull request."));
        }
        if title.chars().any(char::is_control) {
            return Err(GitError::failed(SERVICE, "The title must be one line."));
        }
        if head == base {
            return Err(GitError::failed(SERVICE, "The base branch must be different from the branch."));
        }
        let Some(token) = token else {
            return Err(GitError::failed(
                SERVICE,
                format!("Creating a pull request on {} needs a token. Add one under Settings, in Gitea and Forgejo.", repository.host),
            ));
        };
        self.check_branch_name(head, directory)?;
        self.check_branch_name(base, directory)?;
        let payload = format!(
            "{{\"title\":{},\"body\":{},\"head\":{},\"base\":{}}}",
            json_string(title),
            json_string(body),
            json_string(head),
            json_string(base)
        );
        let url = repository_api(repository, "/pulls");
        let json = request_json(&repository.host, "POST", &url, Some(token), Some(&payload), directory)?;
        decode_created_url(&json, repository)
    }

    /// One pull request list, up to `limit` entries, read page by page with the given state. Each
    /// request checks for cancellation first.
    #[allow(clippy::too_many_arguments)]
    fn read_pulls(
        &self,
        repository: &HostedRepository,
        token: Option<&str>,
        state: &str,
        limit: usize,
        max_pages: usize,
        cancel: &Cancel,
        directory: &Path,
        found: impl Fn(&PullEntry) -> bool,
    ) -> Result<Vec<PullEntry>> {
        let mut entries = Vec::new();
        for page in 1..=max_pages {
            if cancel.is_cancelled() {
                return Err(GitError::failed(SERVICE, github::CANCELLED));
            }
            let path = format!("/pulls?state={state}&sort=recentupdate&limit={PAGE_LIMIT}&page={page}");
            let json = self.api_get(repository, token, &path, directory)?;
            let count = match &json {
                Json::Array(items) => items.len(),
                _ => 0,
            };
            let decoded = decode_pull_entries(&json, repository)?;
            let matched = decoded.iter().any(&found);
            entries.extend(decoded);
            // A wanted pull request found on this page ends the reading, so no further page is asked for.
            if matched || count < PAGE_LIMIT || entries.len() >= limit {
                break;
            }
        }
        entries.truncate(limit);
        Ok(entries)
    }

    /// The combined status of a head commit as its checks. A missing or unreadable status is none.
    fn head_checks(&self, repository: &HostedRepository, token: Option<&str>, sha: Option<&str>, directory: &Path) -> Checks {
        sha.map(|sha| commit_checks(repository, token, sha, directory)).unwrap_or_default()
    }

    /// A GET of one path under the repository's API. An empty path is the repository itself.
    fn api_get(&self, repository: &HostedRepository, token: Option<&str>, path: &str, directory: &Path) -> Result<Json> {
        let url = repository_api(repository, path);
        request_json(&repository.host, "GET", &url, token, None, directory)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forge_api::{curl_plan, parse_exchange};

    fn repository() -> HostedRepository {
        HostedRepository::parse("https://codeberg.org/acme/app.git", ListedHosts::default()).unwrap()
    }

    /// A realistic open pull request from this repository's own branch `feature`, with `extra`
    /// fields appended. Callers do not repeat a field that is already here.
    fn pull(number: u64, extra: &str) -> String {
        format!(
            r#"{{"number": {number}, "title": "Title {number}", "html_url": "https://codeberg.org/acme/app/pulls/{number}", "state": "open", "user": {{"login": "ada"}}, "head": {{"ref": "feature", "sha": "abc123", "repo": {{"full_name": "acme/app"}}}}, "updated_at": "2026-03-04T05:06:07Z"{extra}}}"#
        )
    }

    /// The parsed JSON of a test's text.
    fn json(text: &str) -> Json {
        parse_json(text).unwrap_or_else(|error| panic!("{error}: {text}"))
    }

    /// The pull requests of a list answer, decoded.
    fn entries(text: &str) -> Result<Vec<PullEntry>> {
        decode_pull_entries(&json(text), &repository())
    }

    #[test]
    fn the_token_is_never_in_the_arguments() {
        let token = "0123456789abcdef-secret";
        let plan = curl_plan("GET", "https://codeberg.org/api/v1/repos/acme/app/pulls?state=open", Some(&format!("token {token}")), None);
        assert!(plan.args.iter().all(|arg| !arg.contains(token)), "{:?}", plan.args);
        assert!(plan.config.contains("Authorization: token 0123456789abcdef-secret"), "{}", plan.config);
        assert!(plan.args.contains(&"--config".to_string()) && plan.args.contains(&"-".to_string()));
    }

    #[test]
    fn the_request_is_https_only_and_bounded_in_time() {
        let plan = curl_plan("GET", "https://codeberg.org/api/v1/user", None, None);
        assert!(plan.args.windows(2).any(|pair| pair == ["--proto", "=https"]));
        assert!(plan.args.windows(2).any(|pair| pair == ["--proto-redir", "=https"]));
        assert!(plan.args.contains(&"--fail-with-body".to_string()));
        assert!(plan.args.iter().any(|arg| arg == "--max-time"));
        assert!(plan.args.iter().any(|arg| arg == "--connect-timeout"));
        assert!(plan.config.contains("url = \"https://codeberg.org/api/v1/user\""));
        assert!(!plan.config.contains("Authorization"), "no token, no header");
    }

    #[test]
    fn a_body_goes_in_the_config_with_its_content_type() {
        let body = r#"{"title":"Say \"hi\""}"#;
        let plan = curl_plan("POST", "https://codeberg.org/api/v1/repos/acme/app/pulls", Some("token t0k"), Some(body));
        assert!(plan.config.contains("request = \"POST\""));
        assert!(plan.config.contains("header = \"content-type: application/json\""));
        assert!(plan.config.contains(r#"data-binary = "{\"title\":\"Say \\\"hi\\\"\"}""#), "{}", plan.config);
        assert!(plan.args.iter().all(|arg| !arg.contains("hi")), "the body is never in the arguments");
    }

    #[test]
    fn curls_output_ends_with_the_status_or_there_is_no_answer() {
        assert_eq!(parse_exchange("[]\n200"), Exchange::Answer { status: 200, body: "[]".into() });
        assert_eq!(parse_exchange("\n401"), Exchange::Answer { status: 401, body: String::new() });
        assert_eq!(parse_exchange("\n000"), Exchange::Unreachable, "curl wrote no status");
        assert_eq!(parse_exchange(""), Exchange::Unreachable);
        assert_eq!(parse_exchange("no newline 200"), Exchange::Unreachable);
    }

    #[test]
    fn pulls_are_read_with_their_fields_and_forks_are_marked() {
        let fork = r#"{"number": 4, "title": "From a fork", "html_url": "https://codeberg.org/acme/app/pulls/4", "state": "open", "head": {"ref": "fix", "sha": "def456", "repo": {"full_name": "ada/app"}}}"#;
        let decoded = entries(&format!("[{}, {fork}]", pull(3, ""))).unwrap();
        assert_eq!(decoded.len(), 2);
        assert_eq!(decoded[0].number, 3);
        assert_eq!(decoded[0].head_branch, "feature");
        assert_eq!(decoded[0].head_sha.as_deref(), Some("abc123"));
        assert!(!decoded[0].is_cross_repository);
        assert_eq!(decoded[0].author.as_deref(), Some("ada"));
        assert_eq!(decoded[0].updated_at, 1_772_600_767, "2026-03-04T05:06:07Z");
        assert!(decoded[1].is_cross_repository, "the head repository is another one");
        assert_eq!(decoded[1].head_branch, "fix");
    }

    #[test]
    fn null_and_missing_fields_are_none_not_errors() {
        let text = r#"[{"number": 5, "title": "Bare", "html_url": null, "state": "open", "user": null, "merged": null,
            "head": {"ref": "topic", "sha": null, "repo": null}, "requested_reviewers": null,
            "requested_reviewers_teams": null, "updated_at": null}]"#;
        let entry = &entries(text).unwrap()[0];
        assert_eq!(entry.url, "https://codeberg.org/acme/app/pulls/5", "a missing link is built from the repository");
        assert_eq!(entry.author, None);
        assert_eq!(entry.head_sha, None);
        assert!(entry.is_cross_repository, "a head with no repository is not this repository's branch");
        assert!(entry.review_requests.is_empty());
        assert_eq!(entry.updated_at, 0);
    }

    #[test]
    fn hostile_titles_and_links_are_cleaned_or_replaced() {
        let text = r#"[{"number": 9, "title": "Add\u0007 the\n\u001b[31mbadge\t\u202e  now", "html_url": "https://evil.example/acme/app/pulls/9",
            "state": "open", "user": {"login": "bad login<script>"}, "head": {"ref": "x", "repo": {"full_name": "acme/app"}}}]"#;
        let entry = &entries(text).unwrap()[0];
        assert!(!entry.title.chars().any(char::is_control), "{:?}", entry.title);
        assert!(entry.title.starts_with("Add the"), "{:?}", entry.title);
        assert_eq!(entry.url, "https://codeberg.org/acme/app/pulls/9", "a link to another host is never kept");
        assert_eq!(entry.author, None, "a login that is not a username is not shown");
        let long: String = "t".repeat(1000);
        let text = format!(r#"[{{"number": 2, "title": "{long}", "state": "open", "head": {{"ref": "y"}}}}]"#);
        assert_eq!(entries(&text).unwrap()[0].title.chars().count(), TITLE_LIMIT);
    }

    #[test]
    fn a_link_on_the_same_repository_in_another_case_is_kept() {
        let text = r#"[{"number": 1, "title": "Case", "html_url": "https://CODEBERG.org/Acme/App/pulls/1", "state": "open", "head": {"ref": "z"}}]"#;
        assert_eq!(entries(text).unwrap()[0].url, "https://CODEBERG.org/Acme/App/pulls/1");
    }

    #[test]
    fn merged_closed_and_draft_states_are_read() {
        let text = r#"[
            {"number": 1, "title": "Done", "state": "closed", "merged": true, "head": {"ref": "a"}},
            {"number": 2, "title": "Dropped", "state": "closed", "merged": false, "head": {"ref": "b"}},
            {"number": 3, "title": "Flagged", "state": "open", "draft": true, "head": {"ref": "c"}},
            {"number": 4, "title": "WIP: Older server", "state": "open", "draft": null, "head": {"ref": "d"}},
            {"number": 5, "title": "Ready", "state": "open", "draft": false, "head": {"ref": "e"}}
        ]"#;
        let states: Vec<PrState> = entries(text).unwrap().iter().map(|entry| entry.state).collect();
        assert_eq!(states, [PrState::Merged, PrState::Closed, PrState::Draft, PrState::Draft, PrState::Open]);
    }

    #[test]
    fn requested_reviewers_are_users_and_teams() {
        let extra = r#", "requested_reviewers": [{"login": "grace"}, {"login": ""}, null], "requested_reviewers_teams": [{"name": "Core"}, {"name": "  "}]"#;
        let entry = &entries(&format!("[{}]", pull(6, extra))).unwrap()[0];
        assert_eq!(entry.review_requests, vec![ReviewRequest::User("grace".into()), ReviewRequest::Team("Core".into())]);
    }

    #[test]
    fn a_malformed_entry_refuses_the_whole_list() {
        assert!(entries(r#"[{"number": 1}]"#).is_err(), "no title, state, or head");
        assert!(entries(r#"[{"number": 1, "title": "x", "state": "weird", "head": {"ref": "a"}}]"#).is_err());
        assert!(entries(r#"[{"number": 1.5, "title": "x", "state": "open", "head": {"ref": "a"}}]"#).is_err());
        assert!(entries(r#"[{"number": 1, "title": "x", "state": "open", "head": null}]"#).is_err());
        assert!(entries(r#"{"number": 1}"#).is_err(), "not a list");
    }

    #[test]
    fn the_branch_pull_is_this_repositorys_and_prefers_an_open_one() {
        let text = format!(
            "[{}, {}, {}]",
            r#"{"number": 8, "title": "Old", "state": "closed", "merged": true, "head": {"ref": "feature", "repo": {"full_name": "acme/app"}}}"#,
            pull(9, ""),
            r#"{"number": 10, "title": "Fork", "state": "open", "head": {"ref": "feature", "repo": {"full_name": "bob/app"}}}"#,
        );
        let listed = entries(&text).unwrap();
        let chosen = choose_branch_pull(listed.clone(), "feature").expect("a pull request");
        assert_eq!(chosen.number, 9, "the open one wins, and the fork's is not this repository's");
        assert!(choose_branch_pull(listed, "other").is_none());
    }

    #[test]
    fn a_pull_request_with_no_open_one_is_still_shown() {
        let text = r#"[{"number": 4, "title": "Merged", "state": "closed", "merged": true, "head": {"ref": "done", "repo": {"full_name": "acme/app"}}}]"#;
        let chosen = choose_branch_pull(entries(text).unwrap(), "done").unwrap();
        assert_eq!((chosen.number, chosen.state), (4, PrState::Merged));
    }

    #[test]
    fn checks_count_each_context_once_by_its_status() {
        let text = r#"{"state": "failure", "statuses": [
            {"status": "success", "context": "ci/build"},
            {"status": "warning", "context": "ci/lint"},
            {"status": "failure", "context": "ci/test"},
            {"status": "error", "context": "ci/deploy"},
            {"status": "pending", "context": "ci/docs"},
            {"status": "mystery", "context": "ci/other"}
        ]}"#;
        assert_eq!(decode_checks(&json(text)).unwrap(), Checks { passed: 2, failed: 2, pending: 2 });
    }

    #[test]
    fn a_commit_with_no_statuses_has_no_checks() {
        assert_eq!(decode_checks(&json(r#"{"state": "pending", "statuses": []}"#)).unwrap(), Checks::default());
        assert_eq!(decode_checks(&json(r#"{"state": "pending", "statuses": null}"#)).unwrap(), Checks::default());
        assert_eq!(decode_checks(&json(r#"{"state": "pending"}"#)).unwrap(), Checks::default());
        assert!(decode_checks(&json(r#"{"statuses": "many"}"#)).is_err());
    }

    #[test]
    fn issues_skip_pull_requests_and_clean_their_text() {
        let text = r##"[
            {"number": 2, "title": "Crash\u0000 on <b>start</b>", "html_url": "https://codeberg.org/acme/app/issues/2", "state": "open",
             "user": {"login": "ada"}, "labels": [{"name": "bug", "color": "#ee0701"}, {"color": "00ff00"}], "pull_request": null},
            {"number": 3, "title": "A pull", "html_url": "https://codeberg.org/acme/app/pulls/3", "state": "open", "pull_request": {"merged": false}},
            {"number": 4, "title": "Closed", "html_url": "https://codeberg.org/acme/app/issues/4", "state": "closed", "user": null, "labels": null}
        ]"##;
        let items = decode_issues(&json(text), &repository()).unwrap();
        assert_eq!(items.len(), 2, "the pull request is skipped");
        assert_eq!(items[0].number, 2);
        assert_eq!(items[0].title, "Crash on <b>start</b>");
        assert_eq!(items[0].author.as_deref(), Some("ada"));
        assert_eq!(items[0].state.as_deref(), Some("OPEN"));
        assert_eq!(items[0].labels, vec![GitHubLabel { name: "bug".into(), color: "ee0701".into() }]);
        assert_eq!(items[1].state.as_deref(), Some("CLOSED"));
        assert_eq!(items[1].author, None);
    }

    #[test]
    fn a_non_list_of_issues_is_refused() {
        assert!(decode_issues(&json(r#"{"number": 1}"#), &repository()).is_err());
        assert!(decode_issues(&json(r#"[{"title": "no number"}]"#), &repository()).is_err());
    }

    #[test]
    fn the_default_branch_and_the_login_must_be_names() {
        assert_eq!(decode_default_branch(&json(r#"{"default_branch": "trunk"}"#)).unwrap(), "trunk");
        assert!(decode_default_branch(&json(r#"{"default_branch": null}"#)).is_err());
        assert!(decode_default_branch(&json(r#"{"default_branch": ""}"#)).is_err());
        assert_eq!(decode_login(&json(r#"{"login": "ada.l_1"}"#)).unwrap(), "ada.l_1");
        assert!(decode_login(&json(r#"{"login": "a b"}"#)).is_err());
        assert!(decode_login(&json(r#"{"login": null}"#)).is_err());
    }

    #[test]
    fn the_created_address_is_the_pull_request_page() {
        let created = json(&pull(12, ""));
        assert_eq!(decode_created_url(&created, &repository()).unwrap(), "https://codeberg.org/acme/app/pulls/12");
        assert!(decode_created_url(&json("{}"), &repository()).is_err());
    }

    #[test]
    fn answers_map_to_plain_words() {
        let unauthorized = status_failure("codeberg.org", 401, "").to_string();
        assert!(unauthorized.contains("The token for codeberg.org was not accepted."), "{unauthorized}");
        let missing = status_failure("codeberg.org", 404, "{}").to_string();
        assert!(missing.contains("The repository was not found on codeberg.org, or the token cannot see it."), "{missing}");
        let other = status_failure("gitea.com", 422, r#"{"message": "pull request already\u0007 exists"}"#).to_string();
        assert!(other.contains("gitea.com could not complete the request (status 422). pull request already exists"), "{other}");
        assert!(answer_json("gitea.com", 200, "[]").is_ok());
        assert!(answer_json("gitea.com", 200, "not json").is_err());
    }

    #[test]
    fn commit_ids_and_tokens_are_checked_before_use() {
        assert!(is_commit_id("abc123"));
        assert!(!is_commit_id("abc/../x"));
        assert!(!is_commit_id(""));
        assert!(is_plausible_token("0f1e2d3c"));
        assert!(!is_plausible_token("has space"));
        assert!(!is_plausible_token(""));
    }

    #[test]
    fn repository_addresses_are_built_from_the_parsed_host() {
        let listed = vec!["forge.example.com".to_string()];
        let hosts = ListedHosts { github: &[], gitlab: &[], gitea: &listed };
        let repository = HostedRepository::parse("https://forge.example.com/acme/app", hosts).unwrap();
        assert_eq!(repository_api(&repository, "/pulls"), "https://forge.example.com/api/v1/repos/acme/app/pulls");
    }

    #[test]
    fn a_404_with_a_message_is_a_missing_pull_request_and_a_plain_one_is_a_missing_route() {
        assert!(is_missing_pull(r#"{"message":"The target couldn't be found.","url":"x","errors":[]}"#));
        assert!(!is_missing_pull("404 page not found"), "an unknown route is not a missing pull request");
        assert!(!is_missing_pull(""));
    }

    #[test]
    fn a_branch_name_with_slashes_is_one_path_segment() {
        assert_eq!(encode_segment("renovate/forgejo-1.x"), "renovate%2Fforgejo-1.x");
        assert_eq!(encode_segment("main"), "main");
    }

    #[test]
    fn a_pull_request_becomes_a_sidebar_row_and_an_open_request() {
        let entry = entries(&format!("[{}]", pull(7, r#", "draft": true"#))).unwrap().remove(0);
        let item = item_of_pull(entry.clone());
        assert_eq!((item.number, item.state.as_deref(), item.is_draft), (7, Some("OPEN"), true));
        let request = open_request(entry, Checks { passed: 1, failed: 0, pending: 0 });
        assert!(request.is_draft && !request.is_cross_repository);
        assert_eq!(request.checks.passed, 1);
    }
}
