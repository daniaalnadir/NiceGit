//! Bitbucket Cloud pull requests and issues, read and created through the Bitbucket REST API, version 2.0,
//! with the system `curl`. Only bitbucket.org is supported; Bitbucket Server is not.
//!
//! Public repositories can be read without credentials. Creating a pull request needs a username and an
//! app password or API token, sent as HTTP Basic authentication in curl's config on standard input, so
//! they never appear in the process arguments. A list follows the API's `next` links only while they stay
//! on the API's own address, so credentials are never sent anywhere else.

use std::collections::BTreeMap;
use std::path::Path;

use crate::ai_message::json_string;
use crate::client::GitClient;
use crate::forge_api::{
    basic_authorization, choose_branch_pull, encode_component, exchange, is_commit_id, item_of_pull, json_answer, name_of, open_request,
    optional_str, read_concurrently, request_json, status_of, unexpected, web_link, whole_number, Exchange, PullEntry, NO_ISSUE_TRACKER,
    TITLE_LIMIT,
};
use crate::github::{
    self, clean_text, timestamp_seconds, Cancel, Checks, GitHubItem, ItemKind, OpenPullRequest, PrState, PullRequestStatus,
};
use crate::hosting::{Host, HostedRepository};
use crate::json::Json;
use crate::models::{GitError, Result};

/// The name used in errors for this service.
const SERVICE: &str = "Bitbucket";
/// The host of every Bitbucket Cloud repository.
pub const HOST: &str = "bitbucket.org";
/// The API's address. Every request, and every `next` link followed, starts with it.
const API: &str = "https://api.bitbucket.org/2.0";
/// The most items the API returns in one page.
pub const PAGE_LIMIT: usize = 50;
/// The most pages read for one listing, so a long list costs a bounded number of requests.
pub const MAX_PAGES: usize = 4;
/// How many pages a background refresh reads of a pull request list.
pub const BACKGROUND_PAGES: usize = 2;
/// The statuses asked for in one page of a commit's statuses.
const STATUS_PAGE: usize = 100;

/// Credentials as they are saved: the username, a colon, and the app password or API token. A username
/// cannot hold a colon, and neither part can hold a space or a control character.
pub fn credentials_text(username: &str, secret: &str) -> Option<String> {
    let plain = |text: &str| !text.is_empty() && !text.chars().any(|c| c.is_whitespace() || c.is_control());
    (plain(username) && plain(secret) && !username.contains(':')).then(|| format!("{username}:{secret}"))
}

/// The Authorization header value for saved credentials, or none when there are none, or when the saved
/// text is not a username and a secret joined by a colon.
pub fn authorization(credentials: Option<&str>) -> Option<String> {
    let credentials = credentials?;
    let (username, secret) = credentials.split_once(':')?;
    (!username.is_empty() && !secret.is_empty()).then(|| basic_authorization(credentials))
}

/// The address of a repository on the API, with `path` after it, such as `/pullrequests`.
fn repository_api(repository: &HostedRepository, path: &str) -> String {
    format!("{API}/repositories/{}{path}", repository.slug())
}

/// The open pull requests of a repository, the first page.
fn open_pulls_url(repository: &HostedRepository) -> String {
    repository_api(repository, &format!("/pullrequests?state=OPEN&pagelen={PAGE_LIMIT}"))
}

/// The open pull requests whose source is `branch`, the first page. The branch name is a quoted query
/// value, so its quotes are escaped first.
fn branch_pulls_url(repository: &HostedRepository, branch: &str) -> String {
    let escaped = branch.replace('"', "\\\"");
    let query = encode_component(&format!("source.branch.name=\"{escaped}\""));
    repository_api(repository, &format!("/pullrequests?state=OPEN&pagelen={PAGE_LIMIT}&q={query}"))
}

/// The `values` of one page of a list answer.
fn page_values<'a>(page: &'a Json, what: &str) -> Result<&'a [Json]> {
    match page.get("values") {
        Some(Json::Array(values)) => Ok(values.as_slice()),
        _ => Err(unexpected(SERVICE, what)),
    }
}

/// The address of the next page, when the answer names one on the API's own address. Any other link is
/// not followed.
fn next_page(page: &Json) -> Option<String> {
    let next = page.get("next").and_then(Json::as_str)?;
    next.starts_with(&format!("{API}/")).then(|| next.to_string())
}

/// The address of a pull request or issue's own page, as the API's `links.html` gives it.
fn html_link(entry: &Json) -> Option<&str> {
    entry.get("links").and_then(|links| links.get("html")).and_then(|html| html.get("href")).and_then(Json::as_str)
}

/// One pull request from a list, decoded strictly. Its link is kept only when it is this repository's own
/// page; a source repository that is missing, or is another one, marks a fork.
fn decode_pull_entry(entry: &Json, repository: &HostedRepository) -> Result<PullEntry> {
    let Json::Object(_) = entry else { return Err(unexpected(SERVICE, "pull requests")) };
    let number = whole_number(entry.get("id")).ok_or_else(|| unexpected(SERVICE, "pull requests"))?;
    let title = entry.get("title").and_then(Json::as_str).ok_or_else(|| unexpected(SERVICE, "pull requests"))?;
    let state = match entry.get("state").and_then(Json::as_str) {
        Some("OPEN") => PrState::Open,
        Some("MERGED") => PrState::Merged,
        Some("DECLINED" | "SUPERSEDED") => PrState::Closed,
        _ => return Err(unexpected(SERVICE, "pull requests")),
    };
    let source =
        entry.get("source").filter(|source| matches!(source, Json::Object(_))).ok_or_else(|| unexpected(SERVICE, "pull requests"))?;
    let head_branch = source
        .get("branch")
        .and_then(|branch| branch.get("name"))
        .and_then(Json::as_str)
        .ok_or_else(|| unexpected(SERVICE, "pull requests"))?;
    let head_sha = source
        .get("commit")
        .and_then(|commit| commit.get("hash"))
        .and_then(Json::as_str)
        .filter(|sha| is_commit_id(sha))
        .map(str::to_string);
    let is_cross_repository = match source.get("repository") {
        None | Some(Json::Null) => true,
        Some(repo) => !repo.get("full_name").and_then(Json::as_str).unwrap_or_default().eq_ignore_ascii_case(&repository.slug()),
    };
    let updated_at = match optional_str(entry, "updated_on").map_err(|_| unexpected(SERVICE, "pull requests"))? {
        None => 0,
        Some(text) => timestamp_seconds(text).ok_or_else(|| unexpected(SERVICE, "pull requests"))?,
    };
    Ok(PullEntry {
        number,
        title: clean_text(title, TITLE_LIMIT),
        url: web_link(repository, "pull-requests", number, html_link(entry)),
        state,
        author: name_of(entry.get("author").and_then(|author| author.get("nickname"))),
        head_branch: clean_text(head_branch, TITLE_LIMIT),
        head_sha,
        is_cross_repository,
        // Bitbucket's reviewers are the people who may approve, not the people asked to review, so none is listed.
        review_requests: Vec::new(),
        updated_at,
    })
}

/// The pull requests of a list page, each decoded strictly. One bad entry refuses the list.
fn decode_pull_entries(page: &Json, repository: &HostedRepository) -> Result<Vec<PullEntry>> {
    page_values(page, "pull requests")?.iter().map(|entry| decode_pull_entry(entry, repository)).collect()
}

/// The checks of one commit from its statuses. Each build's newest status counts once: a successful one
/// passed; a failed or stopped one failed; anything else, such as one in progress, is pending. A status
/// with no time keeps the order it was listed in.
pub(crate) fn decode_checks(value: &Json) -> Result<Checks> {
    let statuses = page_values(value, "commit statuses")?;
    let mut latest: BTreeMap<String, (i64, &str)> = BTreeMap::new();
    for (index, status) in statuses.iter().enumerate() {
        let key = status
            .get("key")
            .or_else(|| status.get("name"))
            .and_then(Json::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| format!("#{index}"));
        let word = status.get("state").and_then(Json::as_str).unwrap_or_default();
        let stamp = optional_str(status, "updated_on").ok().flatten().and_then(timestamp_seconds).unwrap_or(0);
        match latest.get(&key) {
            Some((seen, _)) if *seen >= stamp => {}
            _ => {
                latest.insert(key, (stamp, word));
            }
        }
    }
    let mut checks = Checks::default();
    for (_, word) in latest.values() {
        match *word {
            "SUCCESSFUL" => checks.passed += 1,
            "FAILED" | "STOPPED" => checks.failed += 1,
            _ => checks.pending += 1,
        }
    }
    Ok(checks)
}

/// The checks of one commit. A commit whose statuses cannot be read has none, so a list still shows.
fn commit_checks(repository: &HostedRepository, credentials: Option<&str>, sha: &str, directory: &Path) -> Checks {
    if !is_commit_id(sha) {
        return Checks::default();
    }
    let url = repository_api(repository, &format!("/commit/{sha}/statuses?pagelen={STATUS_PAGE}"));
    let auth = authorization(credentials);
    request_json(SERVICE, &repository.host, "GET", &url, auth.as_deref(), None, directory)
        .ok()
        .and_then(|json| decode_checks(&json).ok())
        .unwrap_or_default()
}

/// The issues of a list page, skipping nothing: the query asks only for new and open issues.
fn decode_issues(page: &Json, repository: &HostedRepository) -> Result<Vec<GitHubItem>> {
    page_values(page, "issues")?.iter().map(|entry| decode_issue(entry, repository)).collect()
}

/// One issue, decoded strictly. Its state word is open for a new or open issue, and closed for any other.
fn decode_issue(entry: &Json, repository: &HostedRepository) -> Result<GitHubItem> {
    let Json::Object(_) = entry else { return Err(unexpected(SERVICE, "issues")) };
    let number = whole_number(entry.get("id")).ok_or_else(|| unexpected(SERVICE, "issues"))?;
    let title = entry.get("title").and_then(Json::as_str).ok_or_else(|| unexpected(SERVICE, "issues"))?;
    let state = match entry.get("state").and_then(Json::as_str).ok_or_else(|| unexpected(SERVICE, "issues"))? {
        "new" | "open" => "OPEN",
        _ => "CLOSED",
    };
    Ok(GitHubItem {
        number,
        title: clean_text(title, TITLE_LIMIT),
        url: web_link(repository, "issues", number, html_link(entry)),
        author: name_of(entry.get("reporter").and_then(|reporter| reporter.get("nickname"))),
        state: Some(state.to_string()),
        is_draft: false,
        labels: Vec::new(),
    })
}

/// The main branch of a repository from its answer. Its name must be a non-empty string.
fn decode_default_branch(value: &Json) -> Result<String> {
    value
        .get("mainbranch")
        .and_then(|branch| branch.get("name"))
        .and_then(Json::as_str)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .ok_or_else(|| GitError::failed(SERVICE, "The repository has no main branch."))
}

/// The username from `/user`: the nickname, or else the username, which must not be empty.
fn decode_login(value: &Json) -> Result<String> {
    ["nickname", "username"]
        .iter()
        .find_map(|key| name_of(value.get(key)))
        .ok_or_else(|| GitError::failed(SERVICE, "Bitbucket did not report your username."))
}

/// The body of a new pull request. Built by hand so the strings are JSON-escaped one by one.
fn pull_request_payload(title: &str, description: &str, head: &str, base: &str) -> String {
    format!(
        "{{\"title\":{},\"description\":{},\"source\":{{\"branch\":{{\"name\":{}}}}},\"destination\":{{\"branch\":{{\"name\":{}}}}}}}",
        json_string(title),
        json_string(description),
        json_string(head),
        json_string(base)
    )
}

/// The checks of the pull request's head commit, or none when it has no head or no statuses.
fn head_checks(repository: &HostedRepository, credentials: Option<&str>, entry: &PullEntry, directory: &Path) -> Checks {
    entry.head_sha.as_deref().map(|sha| commit_checks(repository, credentials, sha, directory)).unwrap_or_default()
}

/// The open pull request list of a repository, with the checks of each head commit. Pull requests that
/// share a head commit share its checks. A commit whose statuses cannot be read has none.
#[allow(clippy::too_many_arguments)]
fn read_pulls(
    repository: &HostedRepository,
    credentials: Option<&str>,
    first: String,
    max_pages: usize,
    limit: usize,
    cancel: &Cancel,
    directory: &Path,
) -> Result<Vec<PullEntry>> {
    let auth = authorization(credentials);
    collect_pages(first, max_pages, limit, |address| {
        if cancel.is_cancelled() {
            return Err(GitError::failed(SERVICE, github::CANCELLED));
        }
        let page = request_json(SERVICE, &repository.host, "GET", address, auth.as_deref(), None, directory)?;
        Ok((decode_pull_entries(&page, repository)?, next_page(&page)))
    })
}

/// Reads a list page by page, following each answer's `next` link. `read_page` gives a page's items and the
/// address of the page after it. Reading stops after `max_pages` pages, once `limit` items are kept, or when
/// no next page is named. The result is cut to `limit`.
fn collect_pages<T>(
    first: String,
    max_pages: usize,
    limit: usize,
    mut read_page: impl FnMut(&str) -> Result<(Vec<T>, Option<String>)>,
) -> Result<Vec<T>> {
    let mut items = Vec::new();
    let mut next = Some(first);
    for _ in 0..max_pages {
        let Some(address) = next.take() else { break };
        let (page, following) = read_page(&address)?;
        items.extend(page);
        if items.len() >= limit {
            break;
        }
        next = following;
    }
    items.truncate(limit);
    Ok(items)
}

/// The page of issues from its answer. A 404 on the issues route means the repository's issue tracker is
/// switched off, which is a notice rather than a failure.
fn issues_answer(answer: Exchange, host: &str) -> Result<Json> {
    match answer {
        Exchange::Answer { status: 404, .. } => Err(GitError::failed(SERVICE, NO_ISSUE_TRACKER)),
        answer => json_answer(SERVICE, host, answer),
    }
}

impl GitClient {
    /// The open pull requests of a repository, with the checks of each head commit, for the Launchpad and
    /// branch checks. At most `max_pages` pages are read, and the list is capped at the open pull request limit.
    pub fn bitbucket_open_pull_requests(
        &self,
        repository: &HostedRepository,
        credentials: Option<&str>,
        max_pages: usize,
        cancel: &Cancel,
        directory: &Path,
    ) -> Result<Vec<OpenPullRequest>> {
        let limit = github::OPEN_PULL_REQUEST_LIMIT;
        let entries = read_pulls(repository, credentials, open_pulls_url(repository), max_pages, limit, cancel, directory)?;
        let heads: Vec<String> = entries.iter().filter_map(|entry| entry.head_sha.clone()).collect();
        let checks = read_concurrently(&heads, |sha: &String| commit_checks(repository, credentials, sha, directory));
        Ok(entries
            .into_iter()
            .map(|entry| {
                let found = entry.head_sha.as_deref().and_then(|sha| checks.get(sha).copied()).unwrap_or_default();
                open_request(entry, found)
            })
            .collect())
    }

    /// The open pull request whose source is `branch`, with its checks. A branch with none gives `None`.
    /// Only the first page of the listing is read, as the status bar asks each time it is shown.
    pub fn bitbucket_pull_request_for_branch(
        &self,
        repository: &HostedRepository,
        credentials: Option<&str>,
        branch: &str,
        directory: &Path,
    ) -> Result<Option<PullRequestStatus>> {
        self.check_branch_name(branch, directory)?;
        let entries =
            read_pulls(repository, credentials, branch_pulls_url(repository, branch), 1, PAGE_LIMIT, &Cancel::default(), directory)?;
        let Some(entry) = choose_branch_pull(entries, branch) else { return Ok(None) };
        let checks = head_checks(repository, credentials, &entry, directory);
        Ok(Some(status_of(Host::Bitbucket, entry, checks)))
    }

    /// Pull requests or issues of a repository, without their checks. Issues are the new and open ones.
    /// An issue tracker that is switched off is reported as a notice, not an error.
    pub fn bitbucket_items(
        &self,
        repository: &HostedRepository,
        kind: ItemKind,
        credentials: Option<&str>,
        limit: usize,
        directory: &Path,
    ) -> Result<Vec<GitHubItem>> {
        match kind {
            ItemKind::PullRequest => {
                let entries =
                    read_pulls(repository, credentials, open_pulls_url(repository), MAX_PAGES, limit, &Cancel::default(), directory)?;
                Ok(entries.into_iter().map(item_of_pull).collect())
            }
            ItemKind::Issue => self.bitbucket_issues(repository, credentials, limit, directory),
        }
    }

    /// The new and open issues, read page by page. A 404 on the issues route means the repository's issue
    /// tracker is switched off.
    fn bitbucket_issues(
        &self,
        repository: &HostedRepository,
        credentials: Option<&str>,
        limit: usize,
        directory: &Path,
    ) -> Result<Vec<GitHubItem>> {
        let auth = authorization(credentials);
        let query = encode_component("state=\"new\" OR state=\"open\"");
        let first = repository_api(repository, &format!("/issues?q={query}&pagelen={PAGE_LIMIT}"));
        collect_pages(first, MAX_PAGES, limit, |address| {
            let answer = exchange(directory, "GET", address, auth.as_deref(), None);
            let page = issues_answer(answer, &repository.host)?;
            Ok((decode_issues(&page, repository)?, next_page(&page)))
        })
    }

    /// The main branch of a repository, read with the credentials when there are some. Public repositories
    /// can be read without them. The name is checked as a branch name before it is returned.
    pub fn bitbucket_default_branch(&self, repository: &HostedRepository, credentials: Option<&str>, directory: &Path) -> Result<String> {
        let auth = authorization(credentials);
        let json = request_json(SERVICE, &repository.host, "GET", &repository_api(repository, ""), auth.as_deref(), None, directory)?;
        let name = decode_default_branch(&json)?;
        self.check_branch_name(&name, directory)?;
        Ok(name)
    }

    /// The username of the person whose credentials are given, read from `/user`.
    pub fn bitbucket_login(&self, credentials: &str, directory: &Path) -> Result<String> {
        let auth = authorization(Some(credentials));
        let json = request_json(SERVICE, HOST, "GET", &format!("{API}/user"), auth.as_deref(), None, directory)?;
        decode_login(&json)
    }

    /// Creates a pull request from `head` into `base` and returns its address. Creating needs credentials,
    /// and Bitbucket pull requests have no drafts, so none is made. The title is one line and the branch
    /// names are checked before the request is made.
    #[allow(clippy::too_many_arguments)]
    pub fn create_bitbucket_pull_request(
        &self,
        repository: &HostedRepository,
        credentials: Option<&str>,
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
        let Some(auth) = authorization(credentials) else {
            return Err(GitError::failed(
                SERVICE,
                format!(
                    "Creating a pull request on {} needs a username and an app password. Add them under Settings, in Bitbucket.",
                    repository.host
                ),
            ));
        };
        self.check_branch_name(head, directory)?;
        self.check_branch_name(base, directory)?;
        let payload = pull_request_payload(title, body, head, base);
        let url = repository_api(repository, "/pullrequests");
        let json = request_json(SERVICE, &repository.host, "POST", &url, Some(&auth), Some(&payload), directory)?;
        decode_pull_entry(&json, repository).map(|entry| entry.url)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forge_api::{basic_authorization as basic, curl_plan, parse_exchange};
    use crate::hosting::ListedHosts;
    use crate::json::parse_json;

    fn repository() -> HostedRepository {
        HostedRepository::parse("git@bitbucket.org:team/tool.git", ListedHosts::default()).unwrap()
    }

    fn json(text: &str) -> Json {
        parse_json(text).unwrap_or_else(|error| panic!("{error}: {text}"))
    }

    /// A realistic open pull request from the team's own branch `feature/login`, with `extra` appended.
    fn pull(id: u64, extra: &str) -> String {
        format!(
            r#"{{"id": {id}, "title": "Fix login {id}", "state": "OPEN", "type": "pullrequest",
                "author": {{"display_name": "Ada Lovelace", "nickname": "ada", "type": "user", "account_id": "5f1"}},
                "source": {{"branch": {{"name": "feature/login"}}, "commit": {{"hash": "9f2c1e4a7b3d5e6f8a9b0c1d2e3f4a5b6c7d8e9f"}},
                            "repository": {{"full_name": "team/tool", "name": "tool"}}}},
                "destination": {{"branch": {{"name": "main"}}, "repository": {{"full_name": "team/tool"}}}},
                "links": {{"html": {{"href": "https://bitbucket.org/team/tool/pull-requests/{id}"}}}},
                "updated_on": "2026-03-04T05:06:07.123456+00:00"{extra}}}"#
        )
    }

    fn entries(text: &str) -> Result<Vec<PullEntry>> {
        decode_pull_entries(&json(text), &repository())
    }

    #[test]
    fn a_bitbucket_pull_request_page_is_read_with_its_source_and_author() {
        let page = format!(r#"{{"pagelen": 50, "size": 2, "page": 1, "values": [{}, {}]}}"#, pull(3, ""), pull(4, ""));
        let decoded = entries(&page).unwrap();
        assert_eq!(decoded.len(), 2);
        assert_eq!(decoded[0].number, 3);
        assert_eq!(decoded[0].head_branch, "feature/login", "a branch name with a slash is kept whole");
        assert_eq!(decoded[0].head_sha.as_deref(), Some("9f2c1e4a7b3d5e6f8a9b0c1d2e3f4a5b6c7d8e9f"));
        assert_eq!(decoded[0].author.as_deref(), Some("ada"));
        assert_eq!(decoded[0].url, "https://bitbucket.org/team/tool/pull-requests/3");
        assert_eq!(decoded[0].updated_at, 1_772_600_767, "2026-03-04T05:06:07 with fractional seconds");
        assert!(!decoded[0].is_cross_repository);
    }

    #[test]
    fn forks_deleted_sources_and_null_fields_are_read_without_failing() {
        let fork = pull(5, "").replace(r#""full_name": "team/tool", "name": "tool""#, r#""full_name": "ada/tool", "name": "tool""#);
        let deleted_fork = pull(6, "").replace(r#""repository": {"full_name": "team/tool", "name": "tool"}"#, r#""repository": null"#);
        let bare = r#"{"id": 7, "title": "Bare", "state": "MERGED", "author": null,
            "source": {"branch": {"name": "old"}, "commit": null, "repository": {"full_name": "team/tool"}},
            "links": null, "updated_on": null}"#;
        let decoded = entries(&format!(r#"{{"values": [{fork}, {deleted_fork}, {bare}]}}"#)).unwrap();
        assert!(decoded[0].is_cross_repository, "a fork is another repository's branch");
        assert!(decoded[1].is_cross_repository, "a deleted fork is not this repository's branch");
        assert_eq!(decoded[2].state, PrState::Merged);
        assert_eq!(decoded[2].author, None);
        assert_eq!(decoded[2].head_sha, None);
        assert_eq!(decoded[2].url, "https://bitbucket.org/team/tool/pull-requests/7", "a missing link is built");
        assert_eq!(decoded[2].updated_at, 0);
    }

    #[test]
    fn a_link_to_another_host_or_repository_is_replaced() {
        let elsewhere =
            pull(8, "").replace("https://bitbucket.org/team/tool/pull-requests/8", "https://evil.example/team/tool/pull-requests/8");
        assert_eq!(entries(&format!(r#"{{"values": [{elsewhere}]}}"#)).unwrap()[0].url, "https://bitbucket.org/team/tool/pull-requests/8");
    }

    #[test]
    fn a_malformed_pull_request_refuses_the_page() {
        assert!(entries(r#"{"values": [{"id": 1}]}"#).is_err());
        assert!(entries(&format!(r#"{{"values": [{}]}}"#, pull(9, "").replace("\"OPEN\"", "\"WEIRD\""))).is_err());
        assert!(entries(r#"{"items": []}"#).is_err(), "no values");
        assert!(entries("[]").is_err(), "not a page");
    }

    #[test]
    fn the_next_link_is_followed_only_on_the_api_address() {
        let same = json(r#"{"values": [], "next": "https://api.bitbucket.org/2.0/repositories/team/tool/pullrequests?page=2"}"#);
        assert_eq!(next_page(&same).as_deref(), Some("https://api.bitbucket.org/2.0/repositories/team/tool/pullrequests?page=2"));
        let other = json(r#"{"values": [], "next": "https://attacker.example/2.0/steal"}"#);
        assert_eq!(next_page(&other), None, "credentials never go to another host");
        assert_eq!(next_page(&json(r#"{"values": []}"#)), None);
    }

    #[test]
    fn the_branch_query_quotes_and_escapes_the_name() {
        let url = branch_pulls_url(&repository(), "feature/login");
        assert!(url.contains("state=OPEN"), "{url}");
        assert!(url.contains("q=source.branch.name%3D%22feature%2Flogin%22"), "{url}");
        let quoted = branch_pulls_url(&repository(), "a\"b");
        assert!(quoted.contains("source.branch.name%3D%22a%5C%22b%22"), "a quote is escaped: {quoted}");
    }

    #[test]
    fn checks_count_the_newest_status_of_each_build() {
        let statuses = r#"{"values": [
            {"key": "build", "state": "FAILED", "updated_on": "2026-03-04T05:00:00.000000+00:00"},
            {"key": "build", "state": "SUCCESSFUL", "updated_on": "2026-03-04T06:00:00.000000+00:00"},
            {"key": "lint", "state": "INPROGRESS", "updated_on": "2026-03-04T05:00:00.000000+00:00"},
            {"key": "deploy", "state": "STOPPED", "updated_on": "2026-03-04T05:00:00.000000+00:00"},
            {"name": "docs", "state": "SUCCESSFUL"},
            {"key": "odd", "state": "MYSTERY"}
        ]}"#;
        assert_eq!(decode_checks(&json(statuses)).unwrap(), Checks { passed: 2, failed: 1, pending: 2 });
        assert_eq!(decode_checks(&json(r#"{"values": []}"#)).unwrap(), Checks::default());
        assert!(decode_checks(&json(r#"{"values": 3}"#)).is_err());
    }

    #[test]
    fn issues_are_read_and_a_missing_state_is_refused() {
        let page = json(
            r#"{"values": [
                {"id": 12, "title": "Crash\u0007 on <b>start</b>", "state": "new",
                 "reporter": {"nickname": "ada"}, "links": {"html": {"href": "https://bitbucket.org/team/tool/issues/12"}}},
                {"id": 13, "title": "Old", "state": "resolved", "reporter": null, "links": null}
            ]}"#,
        );
        let items = decode_issues(&page, &repository()).unwrap();
        assert_eq!(items[0].title, "Crash on <b>start</b>");
        assert_eq!(items[0].state.as_deref(), Some("OPEN"));
        assert_eq!(items[0].author.as_deref(), Some("ada"));
        assert_eq!(items[1].state.as_deref(), Some("CLOSED"));
        assert_eq!(items[1].url, "https://bitbucket.org/team/tool/issues/13");
        assert!(decode_issues(&json(r#"{"values": [{"id": 1, "title": "x"}]}"#), &repository()).is_err());
    }

    #[test]
    fn the_main_branch_login_and_created_address_are_decoded() {
        assert_eq!(decode_default_branch(&json(r#"{"mainbranch": {"name": "trunk", "type": "branch"}}"#)).unwrap(), "trunk");
        assert!(decode_default_branch(&json(r#"{"mainbranch": null}"#)).is_err(), "an empty repository has none");
        assert_eq!(decode_login(&json(r#"{"nickname": "ada", "username": "ada_l"}"#)).unwrap(), "ada");
        assert_eq!(decode_login(&json(r#"{"nickname": "", "username": "ada_l"}"#)).unwrap(), "ada_l");
        assert!(decode_login(&json("{}")).is_err());
        let created = json(&pull(20, ""));
        assert_eq!(decode_pull_entry(&created, &repository()).unwrap().url, "https://bitbucket.org/team/tool/pull-requests/20");
    }

    #[test]
    fn credentials_are_joined_and_refused_when_plain_text_is_unsafe() {
        assert_eq!(credentials_text("octo", "app-password").as_deref(), Some("octo:app-password"));
        assert_eq!(credentials_text("octo:x", "secret"), None, "a username cannot hold a colon");
        assert_eq!(credentials_text("octo", "has space"), None);
        assert_eq!(credentials_text("", "secret"), None);
    }

    #[test]
    fn the_authorization_header_is_basic_over_user_and_secret() {
        assert_eq!(authorization(Some("octo:pw")).as_deref(), Some(basic("octo:pw").as_str()));
        assert_eq!(authorization(Some("octo:pw")).as_deref(), Some("Basic b2N0bzpwdw=="));
        assert_eq!(authorization(Some("nosecret")), None);
        assert_eq!(authorization(Some(":pw")), None);
        assert_eq!(authorization(None), None);
    }

    #[test]
    fn the_credentials_never_appear_in_the_curl_arguments() {
        let header = authorization(Some("octo:super-secret-password")).unwrap();
        let plan = curl_plan("POST", "https://api.bitbucket.org/2.0/repositories/team/tool/pullrequests", Some(&header), Some("{}"));
        assert!(plan.args.iter().all(|arg| !arg.contains("super-secret") && !arg.contains("b2N0")), "{:?}", plan.args);
        assert!(plan.config.contains("Authorization: Basic "), "{}", plan.config);
    }

    #[test]
    fn the_payload_of_a_new_pull_request_is_valid_json_with_both_branches() {
        let payload = pull_request_payload("Say \"hi\"", "Line one\nTwo", "feature/x", "main");
        let value = json(&payload);
        assert_eq!(value.get("title").and_then(Json::as_str), Some("Say \"hi\""));
        assert_eq!(value.get("description").and_then(Json::as_str), Some("Line one\nTwo"));
        let source = value.get("source").and_then(|s| s.get("branch")).and_then(|b| b.get("name")).and_then(Json::as_str);
        let destination = value.get("destination").and_then(|s| s.get("branch")).and_then(|b| b.get("name")).and_then(Json::as_str);
        assert_eq!((source, destination), (Some("feature/x"), Some("main")));
    }

    #[test]
    fn pages_are_followed_until_the_page_count_the_limit_or_the_last_page() {
        let page = |address: &str| -> Result<(Vec<u32>, Option<String>)> {
            Ok(match address {
                "p1" => (vec![1, 2], Some("p2".to_string())),
                "p2" => (vec![3], Some("p3".to_string())),
                "p3" => (vec![4], None),
                other => panic!("an unknown page {other}"),
            })
        };
        let mut asked = Vec::new();
        let all = collect_pages("p1".to_string(), 4, 10, |address| {
            asked.push(address.to_string());
            page(address)
        })
        .unwrap();
        assert_eq!(all, vec![1, 2, 3, 4]);
        assert_eq!(asked, vec!["p1", "p2", "p3"], "the last page names no next page, so none is asked for");

        let two = collect_pages("p1".to_string(), 2, 10, &page).unwrap();
        assert_eq!(two, vec![1, 2, 3], "two pages at most");
        let capped = collect_pages("p1".to_string(), 4, 2, &page).unwrap();
        assert_eq!(capped, vec![1, 2], "the first page fills the limit, so no second is asked for");
    }

    #[test]
    fn a_next_page_on_another_host_is_never_followed() {
        let elsewhere = json(r#"{"values": [], "next": "https://evil.example/2.0/repositories/team/tool/pullrequests?page=2"}"#);
        let mut asked = Vec::new();
        let items = collect_pages("first".to_string(), 4, 10, |address| {
            asked.push(address.to_string());
            Ok((vec![1], next_page(&elsewhere)))
        })
        .unwrap();
        assert_eq!(items, vec![1]);
        assert_eq!(asked, vec!["first"], "the credentials stay with the API");
    }

    #[test]
    fn a_switched_off_issue_tracker_is_the_notice_and_other_answers_decode() {
        let missing = issues_answer(Exchange::Answer { status: 404, body: String::new() }, HOST).unwrap_err().to_string();
        assert!(missing.contains(NO_ISSUE_TRACKER), "{missing}");
        let page = issues_answer(Exchange::Answer { status: 200, body: r#"{"values": []}"#.into() }, HOST).unwrap();
        assert!(page.get("values").is_some());
        let refused = issues_answer(Exchange::Answer { status: 401, body: String::new() }, HOST).unwrap_err().to_string();
        assert!(refused.contains("The credentials for bitbucket.org were not accepted."), "{refused}");
    }

    #[test]
    fn answers_map_to_the_credential_words() {
        let refused = failure_text(401, "");
        assert!(refused.contains("The credentials for bitbucket.org were not accepted."), "{refused}");
        assert!(failure_text(404, "{}").contains("The repository was not found on bitbucket.org, or the credentials cannot see it."));
        assert_eq!(parse_exchange("{}\n404"), Exchange::Answer { status: 404, body: "{}".into() });
    }

    fn failure_text(status: u16, body: &str) -> String {
        crate::forge_api::failure_for_status(SERVICE, HOST, status, body).to_string()
    }

    #[test]
    fn a_pull_request_becomes_a_row_and_a_status_for_its_branch() {
        let entry = entries(&format!(r#"{{"values": [{}]}}"#, pull(30, ""))).unwrap().remove(0);
        let request = open_request(entry.clone(), Checks { passed: 1, failed: 0, pending: 0 });
        assert_eq!(request.head_branch, "feature/login");
        let status = status_of(Host::Bitbucket, entry, Checks::default());
        assert_eq!((status.kind, status.number, status.state), (Host::Bitbucket, 30, PrState::Open));
    }
}
