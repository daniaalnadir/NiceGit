//! Azure DevOps Git pull requests, read and created through the Azure DevOps REST API, version 7.1, with
//! the system `curl`. The API takes a personal access token as HTTP Basic authentication with an empty user
//! name, so the token is sent in curl's config on standard input and never in the process arguments.
//!
//! Azure DevOps needs a token to read most repositories, so every listing without one is refused with the
//! notice [`NEEDS_TOKEN`] before any request is made. Azure work items are not listed: issues are not read
//! for this service. Each list is read page by page with `$skip`, and each pull request's checks come from
//! its own statuses.

use std::collections::BTreeMap;
use std::path::Path;

use crate::ai_message::json_string;
use crate::client::GitClient;
use crate::forge_api::{
    choose_branch_pull, encode_component, exchange, is_commit_id, item_of_pull, json_answer, name_of, open_request, optional_str,
    read_concurrently, status_of, unexpected, whole_number, Exchange, PullEntry, NEEDS_TOKEN, TITLE_LIMIT,
};
use crate::github::{
    self, clean_text, timestamp_seconds, Cancel, Checks, GitHubItem, ItemKind, OpenPullRequest, PrState, PullRequestStatus,
};
use crate::hosting::{Host, HostedRepository};
use crate::json::Json;
use crate::models::{GitError, Result};

/// The name used in errors for this service.
const SERVICE: &str = "Azure DevOps";
/// The version of the REST API every request asks for.
const API_VERSION: &str = "7.1";
/// The most pull requests the API returns in one page.
pub const PAGE_LIMIT: usize = 50;
/// The most pages read for one listing, so a long list costs a bounded number of requests.
pub const MAX_PAGES: usize = 4;
/// How many pages a background refresh reads of a pull request list.
pub const BACKGROUND_PAGES: usize = 2;

/// The organization of a repository: the first part of its owner, such as `acme` for `acme/Web`.
pub fn organization(repository: &HostedRepository) -> &str {
    repository.owner_path.split('/').next().unwrap_or_default()
}

/// The address of the repository on the API, such as
/// `https://dev.azure.com/acme/Web/_apis/git/repositories/shop`, with names encoded once.
fn repository_base(repository: &HostedRepository) -> String {
    let (organization, project) = repository.owner_path.split_once('/').unwrap_or((repository.owner_path.as_str(), ""));
    format!(
        "https://dev.azure.com/{}/{}/_apis/git/repositories/{}",
        encode_component(organization),
        encode_component(project),
        encode_component(&repository.name)
    )
}

/// The Authorization header for a personal access token: HTTP Basic with an empty user name. None when
/// there is no token.
pub fn authorization(token: Option<&str>) -> Option<String> {
    token.filter(|token| !token.is_empty()).map(|token| crate::forge_api::basic_authorization(&format!(":{token}")))
}

/// The token a request needs, or the notice that asks for one. Nothing is requested without it.
fn require_token(token: Option<&str>) -> Result<&str> {
    token.filter(|token| !token.is_empty()).ok_or_else(|| GitError::failed(SERVICE, NEEDS_TOKEN))
}

/// One request to the repository's API at `path`, which starts with `?` or `/` and carries the API version.
/// Azure answers a rejected token with status 203 and a sign-in page, so 203 is treated as refused.
fn api(repository: &HostedRepository, token: Option<&str>, method: &str, path: &str, body: Option<&str>, directory: &Path) -> Result<Json> {
    let url = format!("{}{path}", repository_base(repository));
    let auth = authorization(token);
    let answer = refused_sign_in(exchange(directory, method, &url, auth.as_deref(), body));
    json_answer(SERVICE, &repository.host, answer)
}

/// The path of one page of the pull requests, filtered by `criteria`, which starts with `&` or is empty.
fn pulls_path(criteria: &str, skip: usize) -> String {
    format!("/pullrequests?api-version={API_VERSION}&searchCriteria.status=active{criteria}&$top={PAGE_LIMIT}&$skip={skip}")
}

/// The `value` array of one answer, which every list carries.
fn values_of<'a>(json: &'a Json, what: &str) -> Result<&'a [Json]> {
    match json.get("value") {
        Some(Json::Array(values)) => Ok(values.as_slice()),
        _ => Err(unexpected(SERVICE, what)),
    }
}

/// The branch name of a full ref such as `refs/heads/feature/x`; any other ref is refused.
fn branch_of(full_ref: &str) -> Option<&str> {
    full_ref.strip_prefix("refs/heads/").filter(|branch| !branch.is_empty())
}

/// One pull request from a list, decoded strictly. A pull request with a fork source comes from another
/// repository. Its link is always this repository's own pull request page.
fn decode_pull_entry(entry: &Json, repository: &HostedRepository) -> Result<PullEntry> {
    let Json::Object(_) = entry else { return Err(unexpected(SERVICE, "pull requests")) };
    let number = whole_number(entry.get("pullRequestId")).ok_or_else(|| unexpected(SERVICE, "pull requests"))?;
    let title = entry.get("title").and_then(Json::as_str).ok_or_else(|| unexpected(SERVICE, "pull requests"))?;
    let is_draft = match entry.get("isDraft") {
        None | Some(Json::Null) => false,
        Some(value) => value.as_bool().ok_or_else(|| unexpected(SERVICE, "pull requests"))?,
    };
    let state = match entry.get("status").and_then(Json::as_str) {
        Some("active") if is_draft => PrState::Draft,
        Some("active") => PrState::Open,
        Some("completed") => PrState::Merged,
        Some("abandoned") => PrState::Closed,
        _ => return Err(unexpected(SERVICE, "pull requests")),
    };
    let head_branch =
        entry.get("sourceRefName").and_then(Json::as_str).and_then(branch_of).ok_or_else(|| unexpected(SERVICE, "pull requests"))?;
    // A fork source, whatever it holds, means the branch is in another repository.
    let is_cross_repository = !matches!(entry.get("forkSource"), None | Some(Json::Null));
    let head_sha = entry
        .get("lastMergeSourceCommit")
        .and_then(|commit| commit.get("commitId"))
        .and_then(Json::as_str)
        .filter(|sha| is_commit_id(sha))
        .map(str::to_string);
    let updated_at = match optional_str(entry, "creationDate").map_err(|_| unexpected(SERVICE, "pull requests"))? {
        None => 0,
        Some(text) => timestamp_seconds(text).ok_or_else(|| unexpected(SERVICE, "pull requests"))?,
    };
    Ok(PullEntry {
        number,
        title: clean_text(title, TITLE_LIMIT),
        url: format!("{}/pullrequest/{number}", repository.home_url()),
        state,
        author: name_of(entry.get("createdBy").and_then(|author| author.get("displayName"))),
        head_branch: clean_text(head_branch, TITLE_LIMIT),
        head_sha,
        is_cross_repository,
        // Reviewers are people or groups who may vote, not requests from the person, so none is listed.
        review_requests: Vec::new(),
        updated_at,
    })
}

/// The checks of a pull request from its statuses. Each context's newest status counts once, where a
/// context is its genre and name. A succeeded status passed; a failed or errored one failed; a pending or
/// unset one is pending; a status that does not apply is left out.
pub(crate) fn decode_checks(value: &Json) -> Result<Checks> {
    let statuses = values_of(value, "pull request statuses")?;
    let mut latest: BTreeMap<String, (f64, &str)> = BTreeMap::new();
    for (index, status) in statuses.iter().enumerate() {
        let context = status.get("context");
        let name = context.and_then(|context| context.get("name")).and_then(Json::as_str).unwrap_or_default();
        let genre = context.and_then(|context| context.get("genre")).and_then(Json::as_str).unwrap_or_default();
        let key = if name.is_empty() { format!("#{index}") } else { format!("{genre}/{name}") };
        let word = status.get("state").and_then(Json::as_str).unwrap_or_default();
        // The status ids grow with time, so the larger one is the newer; without one, the first listed stays.
        let order = status.get("id").and_then(Json::as_number).unwrap_or(0.0);
        match latest.get(&key) {
            Some((seen, _)) if *seen >= order => {}
            _ => {
                latest.insert(key, (order, word));
            }
        }
    }
    let mut checks = Checks::default();
    for (_, word) in latest.values() {
        match *word {
            "succeeded" => checks.passed += 1,
            "failed" | "error" => checks.failed += 1,
            "notApplicable" => {}
            _ => checks.pending += 1,
        }
    }
    Ok(checks)
}

/// The checks of one pull request. One whose statuses cannot be read has none, so a list still shows.
fn pull_checks(repository: &HostedRepository, token: Option<&str>, number: u64, directory: &Path) -> Checks {
    let path = format!("/pullrequests/{number}/statuses?api-version={API_VERSION}");
    api(repository, token, "GET", &path, None, directory).ok().and_then(|json| decode_checks(&json).ok()).unwrap_or_default()
}

/// The pull requests of a list, read page by page up to `max_pages`, each decoded strictly. `criteria` adds
/// filters to the active ones. Reading stops at a short page, or once `limit` pull requests are kept.
#[allow(clippy::too_many_arguments)]
fn read_pulls(
    repository: &HostedRepository,
    token: Option<&str>,
    criteria: &str,
    max_pages: usize,
    limit: usize,
    cancel: &Cancel,
    directory: &Path,
) -> Result<Vec<PullEntry>> {
    collect_skipping(max_pages, limit, |skip| {
        if cancel.is_cancelled() {
            return Err(GitError::failed(SERVICE, github::CANCELLED));
        }
        let json = api(repository, token, "GET", &pulls_path(criteria, skip), None, directory)?;
        values_of(&json, "pull requests")?.iter().map(|entry| decode_pull_entry(entry, repository)).collect()
    })
}

/// Reads a list in pages of [`PAGE_LIMIT`], each page asked for by its `$skip`, up to `max_pages`. Reading
/// stops at a short page or once `limit` items are kept. The result is cut to `limit`.
fn collect_skipping<T>(max_pages: usize, limit: usize, mut read_page: impl FnMut(usize) -> Result<Vec<T>>) -> Result<Vec<T>> {
    let mut items = Vec::new();
    for page in 0..max_pages {
        let found = read_page(page * PAGE_LIMIT)?;
        let count = found.len();
        items.extend(found);
        if count < PAGE_LIMIT || items.len() >= limit {
            break;
        }
    }
    items.truncate(limit);
    Ok(items)
}

/// The answer with a sign-in page instead of the expected one. Azure DevOps answers a rejected token with
/// status 203 and an HTML page, so that status is read as a refused token.
fn refused_sign_in(answer: Exchange) -> Exchange {
    match answer {
        Exchange::Answer { status: 203, .. } => Exchange::Answer { status: 401, body: String::new() },
        other => other,
    }
}

/// The creation body of a pull request. Built with JSON-escaped strings, one at a time.
fn creation_payload(title: &str, description: &str, head: &str, base: &str, draft: bool) -> String {
    format!(
        "{{\"sourceRefName\":{},\"targetRefName\":{},\"title\":{},\"description\":{},\"isDraft\":{}}}",
        json_string(&format!("refs/heads/{head}")),
        json_string(&format!("refs/heads/{base}")),
        json_string(title),
        json_string(description),
        draft
    )
}

impl GitClient {
    /// The organization's pull requests of a repository, with the checks of each. Each pull request's statuses
    /// are read at most six at a time. Without a token the listing is refused with [`NEEDS_TOKEN`].
    pub fn azure_open_pull_requests(
        &self,
        repository: &HostedRepository,
        token: Option<&str>,
        max_pages: usize,
        cancel: &Cancel,
        directory: &Path,
    ) -> Result<Vec<OpenPullRequest>> {
        let token = Some(require_token(token)?);
        let limit = github::OPEN_PULL_REQUEST_LIMIT;
        let entries = read_pulls(repository, token, "", max_pages, limit, cancel, directory)?;
        let numbers: Vec<u64> = entries.iter().map(|entry| entry.number).collect();
        let checks = read_concurrently(&numbers, |number: &u64| pull_checks(repository, token, *number, directory));
        Ok(entries
            .into_iter()
            .map(|entry| {
                let found = checks.get(&entry.number).copied().unwrap_or_default();
                open_request(entry, found)
            })
            .collect())
    }

    /// The active pull request whose source is `branch`, with its checks. A branch with none gives `None`.
    /// Only the first page is read. Without a token the lookup is refused with [`NEEDS_TOKEN`].
    pub fn azure_pull_request_for_branch(
        &self,
        repository: &HostedRepository,
        token: Option<&str>,
        branch: &str,
        directory: &Path,
    ) -> Result<Option<PullRequestStatus>> {
        let token = Some(require_token(token)?);
        self.check_branch_name(branch, directory)?;
        let criteria = format!("&searchCriteria.sourceRefName={}", encode_component(&format!("refs/heads/{branch}")));
        let entries = read_pulls(repository, token, &criteria, 1, PAGE_LIMIT, &Cancel::default(), directory)?;
        let Some(entry) = choose_branch_pull(entries, branch) else { return Ok(None) };
        let checks = pull_checks(repository, token, entry.number, directory);
        Ok(Some(status_of(Host::AzureDevOps, entry, checks)))
    }

    /// The pull requests of a repository, without their checks. Azure work items are not listed, so issues
    /// give none. Without a token the pull request list is refused with [`NEEDS_TOKEN`].
    pub fn azure_items(
        &self,
        repository: &HostedRepository,
        kind: ItemKind,
        token: Option<&str>,
        limit: usize,
        directory: &Path,
    ) -> Result<Vec<GitHubItem>> {
        match kind {
            ItemKind::PullRequest => {
                let token = Some(require_token(token)?);
                let entries = read_pulls(repository, token, "", MAX_PAGES, limit, &Cancel::default(), directory)?;
                Ok(entries.into_iter().map(item_of_pull).collect())
            }
            ItemKind::Issue => Ok(Vec::new()),
        }
    }

    /// The default branch of a repository, read with the token. A repository with none is an error. The
    /// name is checked as a branch name before it is returned.
    pub fn azure_default_branch(&self, repository: &HostedRepository, token: Option<&str>, directory: &Path) -> Result<String> {
        let json = api(repository, token, "GET", &format!("?api-version={API_VERSION}"), None, directory)?;
        let name = json
            .get("defaultBranch")
            .and_then(Json::as_str)
            .and_then(branch_of)
            .map(str::to_string)
            .ok_or_else(|| GitError::failed(SERVICE, "The repository has no default branch."))?;
        self.check_branch_name(&name, directory)?;
        Ok(name)
    }

    /// Creates a pull request from `head` into `base` and returns its address. Creating needs a token. A draft
    /// is created when `draft` is set. The title is one line and the branch names are checked first.
    #[allow(clippy::too_many_arguments)]
    pub fn create_azure_pull_request(
        &self,
        repository: &HostedRepository,
        token: Option<&str>,
        head: &str,
        base: &str,
        title: &str,
        body: &str,
        draft: bool,
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
        let token = token.filter(|token| !token.is_empty()).ok_or_else(|| {
            GitError::failed(
                SERVICE,
                format!(
                    "Creating a pull request on {} needs a personal access token. Add one under Settings, in Azure DevOps.",
                    repository.host
                ),
            )
        })?;
        self.check_branch_name(head, directory)?;
        self.check_branch_name(base, directory)?;
        let payload = creation_payload(title, body, head, base, draft);
        let json = api(repository, Some(token), "POST", &format!("/pullrequests?api-version={API_VERSION}"), Some(&payload), directory)?;
        decode_pull_entry(&json, repository).map(|entry| entry.url)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forge_api::{basic_authorization, parse_exchange};
    use crate::hosting::ListedHosts;
    use crate::json::parse_json;

    fn repository() -> HostedRepository {
        HostedRepository::parse("https://dev.azure.com/acme/Web%20Apps/_git/shop", ListedHosts::default()).unwrap()
    }

    fn json(text: &str) -> Json {
        parse_json(text).unwrap_or_else(|error| panic!("{error}: {text}"))
    }

    /// A realistic active pull request from `feature/login`, with `extra` appended.
    fn pull(id: u64, extra: &str) -> String {
        format!(
            r#"{{"pullRequestId": {id}, "codeReviewId": {id}, "status": "active", "createdBy": {{"displayName": "Ada Lovelace",
                "uniqueName": "ada@acme.example", "id": "7d1"}},
                "creationDate": "2026-03-04T05:06:07.1234567Z", "title": "Fix login {id}",
                "sourceRefName": "refs/heads/feature/login", "targetRefName": "refs/heads/main", "isDraft": false,
                "lastMergeSourceCommit": {{"commitId": "9f2c1e4a7b3d5e6f8a9b0c1d2e3f4a5b6c7d8e9f"}},
                "repository": {{"id": "1b", "name": "shop"}}, "reviewers": [{{"displayName": "Grace"}}], "forkSource": null{extra}}}"#
        )
    }

    fn entries(text: &str) -> Result<Vec<PullEntry>> {
        let page = json(text);
        values_of(&page, "pull requests")?.iter().map(|entry| decode_pull_entry(entry, &repository())).collect()
    }

    #[test]
    fn the_repository_base_encodes_each_name_once() {
        assert_eq!(repository_base(&repository()), "https://dev.azure.com/acme/Web%20Apps/_apis/git/repositories/shop");
        assert_eq!(organization(&repository()), "acme");
        let spaced = HostedRepository::parse("git@ssh.dev.azure.com:v3/acme/Web Apps/shop", ListedHosts::default()).unwrap();
        assert_eq!(repository_base(&spaced), repository_base(&repository()), "a space and %20 are the same name");
    }

    #[test]
    fn a_pull_request_page_is_read_with_its_branch_author_and_link() {
        let decoded = entries(&format!(r#"{{"count": 2, "value": [{}, {}]}}"#, pull(3, ""), pull(4, ""))).unwrap();
        assert_eq!(decoded.len(), 2);
        assert_eq!(decoded[0].number, 3);
        assert_eq!(decoded[0].head_branch, "feature/login");
        assert_eq!(decoded[0].author.as_deref(), Some("Ada Lovelace"));
        assert_eq!(decoded[0].url, "https://dev.azure.com/acme/Web%20Apps/_git/shop/pullrequest/3", "the link is this repository's page");
        assert_eq!(decoded[0].updated_at, 1_772_600_767, "the creation date, with fractional seconds");
        assert!(!decoded[0].is_cross_repository);
        assert!(decoded[0].review_requests.is_empty());
    }

    #[test]
    fn forks_drafts_and_completed_pull_requests_are_read() {
        let fork = pull(5, "").replace(r#""forkSource": null"#, r#""forkSource": {"repository": {"id": "zz"}}"#);
        let draft = pull(6, "").replace(r#""isDraft": false"#, r#""isDraft": true"#);
        let done = pull(7, "")
            .replace(r#""status": "active""#, r#""status": "completed""#)
            .replace(r#""lastMergeSourceCommit""#, r#""closedDate": null, "lastMergeSourceCommit""#);
        let abandoned = pull(8, "").replace(r#""status": "active""#, r#""status": "abandoned""#);
        let decoded = entries(&format!(r#"{{"value": [{fork}, {draft}, {done}, {abandoned}]}}"#)).unwrap();
        assert!(decoded[0].is_cross_repository, "a fork source is another repository");
        assert_eq!(decoded[1].state, PrState::Draft);
        assert_eq!(decoded[2].state, PrState::Merged);
        assert_eq!(decoded[3].state, PrState::Closed);
    }

    #[test]
    fn null_and_missing_optional_fields_are_none_not_errors() {
        let bare = r#"{"pullRequestId": 9, "status": "active", "title": "Bare", "sourceRefName": "refs/heads/x",
            "createdBy": null, "creationDate": null, "isDraft": null, "lastMergeSourceCommit": null, "forkSource": null}"#;
        let decoded = entries(&format!(r#"{{"value": [{bare}]}}"#)).unwrap();
        assert_eq!(decoded[0].author, None);
        assert_eq!(decoded[0].head_sha, None);
        assert_eq!(decoded[0].updated_at, 0);
        assert_eq!(decoded[0].state, PrState::Open);
    }

    #[test]
    fn a_malformed_pull_request_or_ref_refuses_the_page() {
        assert!(entries(r#"{"value": [{"pullRequestId": 1}]}"#).is_err());
        assert!(entries(&format!(r#"{{"value": [{}]}}"#, pull(1, "").replace("refs/heads/feature/login", "refs/tags/v1"))).is_err());
        assert!(entries(&format!(r#"{{"value": [{}]}}"#, pull(1, "").replace("\"active\"", "\"weird\""))).is_err());
        assert!(entries(r#"{"count": 0}"#).is_err(), "no value array");
    }

    #[test]
    fn paging_skips_by_page_size_and_asks_for_active_pull_requests() {
        assert_eq!(pulls_path("", 0), "/pullrequests?api-version=7.1&searchCriteria.status=active&$top=50&$skip=0");
        assert_eq!(pulls_path("", 150), "/pullrequests?api-version=7.1&searchCriteria.status=active&$top=50&$skip=150");
        let criteria = format!("&searchCriteria.sourceRefName={}", encode_component("refs/heads/feature/x"));
        assert!(pulls_path(&criteria, 0).ends_with("&searchCriteria.sourceRefName=refs%2Fheads%2Ffeature%2Fx&$top=50&$skip=0"));
    }

    #[test]
    fn checks_count_the_latest_status_of_each_context() {
        let statuses = r#"{"count": 6, "value": [
            {"id": 11, "state": "failed", "context": {"genre": "ci", "name": "build"}},
            {"id": 12, "state": "succeeded", "context": {"genre": "ci", "name": "build"}},
            {"id": 5, "state": "pending", "context": {"genre": null, "name": "lint"}},
            {"id": 6, "state": "error", "context": {"genre": "sec", "name": "scan"}},
            {"id": 7, "state": "notApplicable", "context": {"genre": "ci", "name": "docs"}},
            {"id": 8, "state": "notSet", "context": {"genre": "ci", "name": "deploy"}}
        ]}"#;
        assert_eq!(decode_checks(&json(statuses)).unwrap(), Checks { passed: 1, failed: 1, pending: 2 });
        assert_eq!(decode_checks(&json(r#"{"value": []}"#)).unwrap(), Checks::default());
        assert!(decode_checks(&json(r#"{"value": "x"}"#)).is_err());
    }

    #[test]
    fn a_pull_request_is_refused_without_a_token_before_any_request() {
        let client = GitClient::new();
        let directory = std::env::temp_dir();
        let refused = client.azure_items(&repository(), ItemKind::PullRequest, None, 10, &directory).unwrap_err().to_string();
        assert!(refused.contains(NEEDS_TOKEN), "{refused}");
        let empty = client.azure_items(&repository(), ItemKind::PullRequest, Some(""), 10, &directory).unwrap_err().to_string();
        assert!(empty.contains(NEEDS_TOKEN), "an empty token is no token: {empty}");
        assert!(client.azure_items(&repository(), ItemKind::Issue, None, 10, &directory).unwrap().is_empty(), "work items are not listed");
        let branch = client.azure_pull_request_for_branch(&repository(), None, "main", &directory).unwrap_err().to_string();
        assert!(branch.contains(NEEDS_TOKEN), "{branch}");
        let open = client.azure_open_pull_requests(&repository(), None, MAX_PAGES, &Cancel::default(), &directory).unwrap_err();
        assert!(open.to_string().contains(NEEDS_TOKEN));
    }

    #[test]
    fn the_authorization_is_basic_with_an_empty_user_name() {
        assert_eq!(authorization(Some("pat123")), Some(basic_authorization(":pat123")));
        assert_eq!(authorization(Some("pat123")).as_deref(), Some("Basic OnBhdDEyMw=="));
        assert_eq!(authorization(Some("")), None);
        assert_eq!(authorization(None), None);
    }

    #[test]
    fn the_token_never_appears_in_the_curl_arguments() {
        let header = authorization(Some("super-secret-pat")).unwrap();
        let plan =
            crate::forge_api::curl_plan("GET", "https://dev.azure.com/acme/_apis/git/repositories/shop/pullrequests", Some(&header), None);
        assert!(plan.args.iter().all(|arg| !arg.contains("super-secret") && !arg.contains("c3VwZXI")), "{:?}", plan.args);
        assert!(plan.config.contains("Authorization: Basic "), "{}", plan.config);
    }

    #[test]
    fn the_created_pull_request_is_linked_on_its_own_page() {
        let created = json(&pull(20, ""));
        assert_eq!(
            decode_pull_entry(&created, &repository()).unwrap().url,
            "https://dev.azure.com/acme/Web%20Apps/_git/shop/pullrequest/20"
        );
        assert!(decode_pull_entry(&json("{}"), &repository()).is_err());
    }

    #[test]
    fn the_creation_body_names_both_refs_and_the_draft_flag() {
        let payload = creation_payload("Say \"hi\"", "Two\nlines", "feature/x", "main", true);
        let value = json(&payload);
        assert_eq!(value.get("sourceRefName").and_then(Json::as_str), Some("refs/heads/feature/x"));
        assert_eq!(value.get("targetRefName").and_then(Json::as_str), Some("refs/heads/main"));
        assert_eq!(value.get("title").and_then(Json::as_str), Some("Say \"hi\""));
        assert_eq!(value.get("isDraft").and_then(Json::as_bool), Some(true));
    }

    #[test]
    fn a_status_203_sign_in_page_is_a_refused_token() {
        let answer = refused_sign_in(Exchange::Answer { status: 203, body: "<html>".into() });
        let refused = json_answer(SERVICE, "dev.azure.com", answer).unwrap_err().to_string();
        assert!(refused.contains("The credentials for dev.azure.com were not accepted."), "{refused}");
        assert_eq!(parse_exchange("<html>\n203"), Exchange::Answer { status: 203, body: "<html>".into() });
        assert_eq!(
            refused_sign_in(Exchange::Answer { status: 200, body: "[]".into() }),
            Exchange::Answer { status: 200, body: "[]".into() }
        );
    }

    #[test]
    fn pages_skip_by_the_page_size_and_stop_at_a_short_page_or_the_limit() {
        let mut asked = Vec::new();
        let all = collect_skipping(4, 1000, |skip| {
            asked.push(skip);
            Ok(vec![0u8; if skip < 100 { PAGE_LIMIT } else { 20 }])
        })
        .unwrap();
        assert_eq!(all.len(), 120);
        assert_eq!(asked, vec![0, 50, 100], "each page skips the ones before it");
        let two = collect_skipping(2, 1000, |_| Ok(vec![0u8; PAGE_LIMIT])).unwrap();
        assert_eq!(two.len(), 100, "two pages at most");
        let capped = collect_skipping(4, 60, |_| Ok(vec![0u8; PAGE_LIMIT])).unwrap();
        assert_eq!(capped.len(), 60, "the result is cut to the limit");
    }
}
