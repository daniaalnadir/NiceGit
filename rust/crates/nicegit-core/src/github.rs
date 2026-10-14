//! GitHub pull requests and issues for a GitHub remote (github.com or a GitHub Enterprise Server
//! host), read through the `gh` command-line tool. NiceGit never stores a token; `gh` keeps its own sign-in.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::client::GitClient;
use crate::models::{GitError, Result};
use crate::rebase::Scratch;
use crate::runner;

const NOT_INSTALLED: &str = "The GitHub CLI (gh) is not installed. Install it from cli.github.com, then sign in with gh auth login.";
const TIMEOUT: Duration = Duration::from_secs(30);
/// Longest `gh` error shown to the user.
const MESSAGE_LIMIT: usize = 2000;
/// The operation name used in errors and temporary file names for pull requests.
const PULL_REQUEST_COMMAND: &str = "create pull request";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemKind {
    PullRequest,
    Issue,
}

impl ItemKind {
    pub fn title(self) -> &'static str {
        match self {
            ItemKind::PullRequest => "Pull requests",
            ItemKind::Issue => "Issues",
        }
    }

    /// The `gh` subcommand that lists this kind.
    fn subcommand(self) -> &'static str {
        match self {
            ItemKind::PullRequest => "pr",
            ItemKind::Issue => "issue",
        }
    }

    /// The path segment GitHub uses for this kind in item links.
    fn path_segment(self) -> &'static str {
        match self {
            ItemKind::PullRequest => "pull",
            ItemKind::Issue => "issues",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemState {
    Open,
    Closed,
    /// Pull requests only.
    Merged,
    All,
}

impl ItemState {
    pub fn as_str(self) -> &'static str {
        match self {
            ItemState::Open => "open",
            ItemState::Closed => "closed",
            ItemState::Merged => "merged",
            ItemState::All => "all",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            ItemState::Open => "Open",
            ItemState::Closed => "Closed",
            ItemState::Merged => "Merged",
            ItemState::All => "All",
        }
    }
}

/// The owner and name of a repository on github.com or a GitHub Enterprise Server host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitHubRepository {
    /// The lower-cased host, such as `github.com` or `git.example.com`.
    pub host: String,
    pub owner: String,
    pub name: String,
}

impl GitHubRepository {
    pub fn slug(&self) -> String {
        format!("{}/{}", self.owner, self.name)
    }

    /// The address of the repository's web pages, such as `https://git.example.com`.
    pub fn web_base(&self) -> String {
        format!("https://{}", self.host)
    }

    /// The address of one of the repository's web pages, such as `pulls` or `actions`, or its
    /// home page for `None`. Built from the parsed host, owner, and name, so a credential or port
    /// in the remote address never appears.
    pub fn web_url(&self, page: Option<&str>) -> String {
        match page {
            Some(page) => format!("{}/{}/{page}", self.web_base(), self.slug()),
            None => format!("{}/{}", self.web_base(), self.slug()),
        }
    }

    /// The repository as `gh --repo` names it, with its host: `host/owner/name`.
    pub fn gh_repo(&self) -> String {
        format!("{}/{}", self.host, self.slug())
    }

    /// Reads a github.com repository from a remote address. See [`Self::parse_with_hosts`].
    pub fn parse(address: &str) -> Result<Self> {
        Self::parse_with_hosts(address, &[])
    }

    /// Reads the repository from a remote address. Accepts `https://` and `ssh://` URLs, and
    /// scp-like `user@host:owner/repo.git` addresses, where the host is github.com or one of
    /// `enterprise_hosts` (compared without regard to case). Other hosts, ports, passwords,
    /// queries, and fragments are refused.
    pub fn parse_with_hosts(address: &str, enterprise_hosts: &[String]) -> Result<Self> {
        let address = address.trim();
        let unsupported = || GitError::failed("GitHub", "This remote is not a supported GitHub repository address.");
        let (host, path) = if let Some((scheme, rest)) = address.split_once("://") {
            if !(scheme.eq_ignore_ascii_case("https") || scheme.eq_ignore_ascii_case("ssh")) {
                return Err(unsupported());
            }
            let (authority, path) = match rest.find('/') {
                Some(index) => (&rest[..index], &rest[index..]),
                None => (rest, ""),
            };
            if path.contains('?') || path.contains('#') || authority.contains('?') || authority.contains('#') {
                return Err(unsupported());
            }
            let host = match authority.rsplit_once('@') {
                Some((userinfo, host)) => {
                    // A user name is allowed; a password is not.
                    if userinfo.contains(':') {
                        return Err(unsupported());
                    }
                    host
                }
                None => authority,
            };
            (host, path)
        } else {
            // scp-like form: [user@]host:path. A local path such as /srv/repo has no host match.
            let Some((host_part, path)) = address.split_once(':') else { return Err(unsupported()) };
            let host = host_part.rsplit_once('@').map(|(_, host)| host).unwrap_or(host_part);
            (host, path)
        };
        // A port, or any other colon, is refused before the host is compared.
        if host.is_empty() || host.contains(':') {
            return Err(unsupported());
        }
        let host = host.to_ascii_lowercase();
        let known = host == "github.com" || enterprise_hosts.iter().any(|listed| listed.eq_ignore_ascii_case(&host));
        if !known {
            return Err(GitError::failed("GitHub", "This remote is not on github.com or a GitHub Enterprise host listed in Settings."));
        }

        let parts: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
        if parts.len() != 2 {
            return Err(GitError::failed("GitHub", "The GitHub remote must identify an owner and repository."));
        }
        let name = parts[1].strip_suffix(".git").unwrap_or(parts[1]);
        if !valid_name_part(parts[0]) || !valid_name_part(name) {
            return Err(GitError::failed("GitHub", "The GitHub repository address is invalid."));
        }
        Ok(GitHubRepository { host, owner: parts[0].to_string(), name: name.to_string() })
    }
}

/// Whether `host` can be listed as a GitHub Enterprise host, such as `git.example.com`: a DNS
/// host name of letters, digits, and hyphens in dot-separated labels, compared in lower case.
/// A port, scheme, path, or space is refused, and so is github.com, which is always supported.
pub fn valid_enterprise_host(host: &str) -> bool {
    let host = host.to_ascii_lowercase();
    if host.len() > 253 || host == "github.com" || !host.contains('.') {
        return false;
    }
    host.split('.').all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label.bytes().all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    })
}

/// A label on an item, with its colour as six hex digits and no leading `#`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitHubLabel {
    pub name: String,
    pub color: String,
}

/// A pull request or issue as `gh` reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitHubItem {
    pub number: u64,
    pub title: String,
    pub url: String,
    pub author: Option<String>,
    /// GitHub's state word, such as `OPEN`, `CLOSED`, or `MERGED`.
    pub state: Option<String>,
    pub is_draft: bool,
    pub labels: Vec<GitHubLabel>,
}

impl GitHubItem {
    /// Whether the item matches a filter: its title or author contains the text, or the text is
    /// its number, with or without a leading `#`.
    pub fn matches(&self, query: &str) -> bool {
        let query = query.trim();
        if query.is_empty() {
            return true;
        }
        let lowered = query.to_lowercase();
        self.title.to_lowercase().contains(&lowered)
            || self.author.as_ref().is_some_and(|author| author.to_lowercase().contains(&lowered))
            || self.number.to_string() == query.strip_prefix('#').unwrap_or(query)
    }
}

/// A repository in the signed-in user's list, as `gh repo list` reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitHubRepoListing {
    /// `owner/name`, as GitHub spells it.
    pub name_with_owner: String,
    /// Trimmed, without control characters, and at most 500 characters.
    pub description: String,
    /// The HTTPS address, `https://<host>/<name_with_owner>`.
    pub url: String,
    /// The SSH address of the same repository, or empty when `gh` gave none that names it.
    pub ssh_url: String,
    pub is_private: bool,
    pub is_fork: bool,
    pub updated_at: String,
}

/// The state of a pull request, as the status bar shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrState {
    Open,
    Draft,
    Merged,
    Closed,
}

/// How a pull request's checks stand: how many passed, failed, or are still pending.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Checks {
    pub passed: usize,
    pub failed: usize,
    pub pending: usize,
}

/// The pull request for a branch, with its checks, as `gh pr list` reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PullRequestStatus {
    pub number: u64,
    /// Without control characters, and at most 300 characters.
    pub title: String,
    pub state: PrState,
    /// The pull request's own page, `<web base>/<owner>/<name>/pull/<number>`.
    pub url: String,
    pub checks: Checks,
}

/// Whether `gh` can be used for the repository.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GhStatus {
    NotInstalled,
    /// Installed, but not signed in; carries `gh`'s explanation.
    SignedOut(String),
    SignedIn,
}

impl GitClient {
    /// The GitHub repository behind a remote's first fetch address, on github.com or one of
    /// `enterprise_hosts`.
    pub fn github_repository(&self, remote: &str, directory: &Path, enterprise_hosts: &[String]) -> Result<GitHubRepository> {
        if remote.is_empty() || remote.starts_with('-') {
            return Err(GitError::failed("GitHub", "Choose a remote first."));
        }
        let output = self.run(&["remote", "get-url", "--all", remote], directory)?;
        let address = output.lines().next().unwrap_or("");
        GitHubRepository::parse_with_hosts(address, enterprise_hosts)
    }

    /// Whether `gh` is installed and signed in to `host`.
    pub fn gh_status(&self, host: &str, directory: &Path) -> GhStatus {
        if gh_executable().is_none() {
            return GhStatus::NotInstalled;
        }
        match run_gh(&["auth", "status", "--hostname", host], directory, &Cancel::default(), None) {
            Ok(_) => GhStatus::SignedIn,
            Err(message) => GhStatus::SignedOut(message),
        }
    }

    /// Lists pull requests or issues, newest first, through `gh`. `limit` is clamped to 1 to 1000.
    pub fn github_items(
        &self,
        repository: &GitHubRepository,
        kind: ItemKind,
        state: ItemState,
        limit: usize,
        directory: &Path,
    ) -> Result<Vec<GitHubItem>> {
        self.github_items_cancellable(repository, kind, state, limit, &Cancel::default(), directory)
    }

    /// Like [`Self::github_items`], ending the request early when `cancel` is set.
    pub fn github_items_cancellable(
        &self,
        repository: &GitHubRepository,
        kind: ItemKind,
        state: ItemState,
        limit: usize,
        cancel: &Cancel,
        directory: &Path,
    ) -> Result<Vec<GitHubItem>> {
        if kind == ItemKind::Issue && state == ItemState::Merged {
            return Err(GitError::failed("GitHub", "Merged applies to pull requests, not issues."));
        }
        let limit = limit.clamp(1, 1000).to_string();
        let repo = repository.gh_repo();
        let fields = match kind {
            ItemKind::PullRequest => "number,title,url,author,state,isDraft,labels",
            ItemKind::Issue => "number,title,url,author,state,labels",
        };
        let output = run_gh(
            &[kind.subcommand(), "list", "--repo", &repo, "--state", state.as_str(), "--limit", &limit, "--json", fields],
            directory,
            cancel,
            None,
        )
        .map_err(|message| GitError::failed("GitHub", message))?;
        decode_items(&output, repository, kind)
    }

    /// Lists the signed-in user's repositories on `host` (github.com, or an Enterprise host listed
    /// in Settings) through `gh repo list`. `limit` is clamped to 1 to 1000. No repository exists
    /// yet, so `directory` only sets where `gh` runs.
    pub fn github_repositories(&self, host: &str, limit: usize, cancel: &Cancel, directory: &Path) -> Result<Vec<GitHubRepoListing>> {
        let limit = limit.clamp(1, 1000).to_string();
        let fields = "nameWithOwner,description,url,sshUrl,isPrivate,isFork,updatedAt";
        let output = run_gh(&["repo", "list", "--limit", &limit, "--json", fields], directory, cancel, Some(host))
            .map_err(|message| GitError::failed("GitHub", message))?;
        decode_repositories(&output, host)
    }

    /// The default branch of `repository`, as GitHub reports it through `gh repo view`. The name
    /// is checked as a branch name before it is returned.
    pub fn default_branch(&self, repository: &GitHubRepository, directory: &Path) -> Result<String> {
        let repo = repository.gh_repo();
        let output = run_gh(
            &["repo", "view", &repo, "--json", "defaultBranchRef", "--jq", ".defaultBranchRef.name"],
            directory,
            &Cancel::default(),
            Some(repository.host.as_str()),
        )
        .map_err(|message| GitError::failed("GitHub", message))?;
        let name = output.trim();
        // An empty repository has no default branch, and jq then prints `null`.
        if name == "null" || name.is_empty() {
            return Err(GitError::failed("GitHub", "GitHub did not report a default branch for this repository."));
        }
        self.check_branch_name(name, directory)?;
        Ok(name.to_string())
    }

    /// Creates a pull request from `head` into `base` on `repository` through `gh pr create`, and
    /// returns the new pull request's address. The title is one line and must not be empty; the
    /// body goes to a temporary file, so its text never reaches the command line. Both branch names
    /// are checked before `gh` runs.
    // The signature is the one the app calls with; each value is a separate argument on purpose.
    #[allow(clippy::too_many_arguments)]
    pub fn create_pull_request(
        &self,
        repository: &GitHubRepository,
        head: &str,
        base: &str,
        title: &str,
        body: &str,
        draft: bool,
        directory: &Path,
    ) -> Result<String> {
        let title = title.trim();
        if title.is_empty() {
            return Err(GitError::failed("GitHub", "Enter a title for the pull request."));
        }
        if title.chars().any(char::is_control) {
            return Err(GitError::failed("GitHub", "The title must be one line."));
        }
        if head == base {
            return Err(GitError::failed("GitHub", "The base branch must be different from the branch."));
        }
        self.check_branch_name(head, directory)?;
        self.check_branch_name(base, directory)?;

        let scratch = Scratch::new(PULL_REQUEST_COMMAND)?;
        let body_file = scratch.write(PULL_REQUEST_COMMAND, "body.md", body.as_bytes())?;
        let body_path = body_file.to_string_lossy().into_owned();
        let arguments = pull_request_arguments(repository, head, base, title, &body_path, draft);
        let arguments: Vec<&str> = arguments.iter().map(String::as_str).collect();
        let output = run_gh(&arguments, directory, &Cancel::default(), Some(repository.host.as_str()))
            .map_err(|message| GitError::failed("GitHub", message))?;
        parse_created_url(&output, repository).ok_or_else(|| {
            let shown: String = output.trim().chars().take(MESSAGE_LIMIT).collect();
            GitError::failed("GitHub", format!("GitHub did not report the new pull request's address. {shown}"))
        })
    }

    /// The subjects of the commits that `head` has beyond `base`, newest first, read from the
    /// remote-tracking branches of `remote`. Both branch names are checked first.
    pub fn pull_request_commits(&self, remote: &str, base: &str, head: &str, directory: &Path) -> Result<Vec<String>> {
        self.check_branch_name(base, directory)?;
        self.check_branch_name(head, directory)?;
        let range = format!("refs/remotes/{remote}/{base}..refs/remotes/{remote}/{head}");
        let output = self.run(&["log", "--format=%s", &range, "--"], directory)?;
        Ok(output.lines().map(str::to_string).collect())
    }

    /// The pull request whose head is `head_branch` on `repository`, with its checks, read through
    /// `gh pr list --head`. A branch with no pull request gives `None`. The branch name is checked
    /// before `gh` runs.
    pub fn pull_request_for_branch(
        &self,
        repository: &GitHubRepository,
        head_branch: &str,
        directory: &Path,
    ) -> Result<Option<PullRequestStatus>> {
        self.check_branch_name(head_branch, directory)?;
        let repo = repository.gh_repo();
        let output = run_gh(
            &[
                "pr",
                "list",
                "--repo",
                &repo,
                "--head",
                head_branch,
                "--state",
                "all",
                // A few, so an open pull request is preferred over a newer closed one.
                "--limit",
                "5",
                "--json",
                "number,title,state,isDraft,url,statusCheckRollup",
            ],
            directory,
            &Cancel::default(),
            Some(repository.host.as_str()),
        )
        .map_err(|message| GitError::failed("GitHub", message))?;
        decode_pull_request_status(&output, repository)
    }

    /// Refuses `name` unless it is a usable branch name: not empty, without spaces or control
    /// characters, not starting with `-`, and accepted by `git check-ref-format --branch`. That
    /// check also expands `@{-n}`, so such names are refused before it runs.
    fn check_branch_name(&self, name: &str, directory: &Path) -> Result<()> {
        let plain = !name.is_empty()
            && !name.starts_with('-')
            && !name.contains("@{")
            && !name.chars().any(|c| c.is_whitespace() || c.is_control());
        if !plain || self.run(&["check-ref-format", "--branch", name], directory).is_err() {
            return Err(GitError::failed("GitHub", "The branch name is not valid."));
        }
        Ok(())
    }
}

/// The `gh pr create` arguments for a pull request from `head` into `base` on `repository`. Each
/// value is its own argument, and the title takes the `--title=value` form, so a title that starts
/// with `-` is never read as a flag. The body is read from `body_file`, which is not in the arguments.
pub fn pull_request_arguments(
    repository: &GitHubRepository,
    head: &str,
    base: &str,
    title: &str,
    body_file: &str,
    draft: bool,
) -> Vec<String> {
    let mut arguments = vec![
        "pr".to_string(),
        "create".to_string(),
        "--repo".to_string(),
        repository.gh_repo(),
        "--head".to_string(),
        head.to_string(),
        "--base".to_string(),
        base.to_string(),
        format!("--title={title}"),
        "--body-file".to_string(),
        body_file.to_string(),
    ];
    if draft {
        arguments.push("--draft".to_string());
    }
    arguments
}

/// The address of the pull request that `gh pr create` printed, which is the last non-empty line
/// of its output. It must be `<web base>/<owner>/<name>/pull/<number>` for `repository`, compared
/// without regard to case. Any other line is refused.
pub fn parse_created_url(output: &str, repository: &GitHubRepository) -> Option<String> {
    let line = output.lines().map(str::trim).rfind(|line| !line.is_empty())?;
    let prefix = format!("{}/{}/pull/", repository.web_base(), repository.slug());
    let (start, number) = line.split_at_checked(prefix.len())?;
    let digits = !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit());
    (start.eq_ignore_ascii_case(&prefix) && digits).then(|| line.to_string())
}

/// The longest repository description kept, in characters.
const DESCRIPTION_LIMIT: usize = 500;

/// Decodes `gh repo list --json` output for `host`, which must be github.com or a valid
/// Enterprise host. The listing is refused unless it is an array of objects with the expected
/// fields. An entry is skipped when its name is not `owner/name` or its HTTPS address is not that
/// repository's own page on `host`. An SSH address that does not name the same repository on
/// `host` is dropped, leaving the entry with no SSH address.
pub fn decode_repositories(json: &str, host: &str) -> Result<Vec<GitHubRepoListing>> {
    let unexpected = || GitError::failed("GitHub", "GitHub returned unexpected repositories.");
    let host = gh_host_variable(host).ok_or_else(unexpected)?;
    let value = parse_json(json).map_err(|_| unexpected())?;
    let Json::Array(entries) = value else { return Err(unexpected()) };
    let mut repositories = Vec::with_capacity(entries.len());
    for entry in &entries {
        let Json::Object(_) = entry else { return Err(unexpected()) };
        let name_with_owner = entry.get("nameWithOwner").and_then(Json::as_str).ok_or_else(unexpected)?;
        let url = entry.get("url").and_then(Json::as_str).ok_or_else(unexpected)?;
        let ssh = optional_text(entry, "sshUrl").ok_or_else(unexpected)?;
        let description = optional_text(entry, "description").ok_or_else(unexpected)?;
        let is_private = entry.get("isPrivate").and_then(Json::as_bool).ok_or_else(unexpected)?;
        let is_fork = entry.get("isFork").and_then(Json::as_bool).ok_or_else(unexpected)?;
        let updated_at = entry.get("updatedAt").and_then(Json::as_str).ok_or_else(unexpected)?;

        let Some((owner, name)) = name_with_owner.split_once('/') else { continue };
        if !valid_name_part(owner) || !valid_name_part(name) {
            continue;
        }
        let expected = format!("https://{host}/{name_with_owner}");
        if !url.eq_ignore_ascii_case(&expected) {
            continue;
        }
        let ssh_url = match GitHubRepository::parse_with_hosts(ssh, std::slice::from_ref(&host)) {
            Ok(address) if address.host == host && address.owner.eq_ignore_ascii_case(owner) && address.name.eq_ignore_ascii_case(name) => {
                ssh.trim().to_string()
            }
            _ => String::new(),
        };
        repositories.push(GitHubRepoListing {
            name_with_owner: name_with_owner.to_string(),
            description: clean_description(description),
            url: url.to_string(),
            ssh_url,
            is_private,
            is_fork,
            updated_at: updated_at.to_string(),
        });
    }
    Ok(repositories)
}

/// The text of an optional JSON field: absent or `null` is empty, and any other type is refused.
fn optional_text<'a>(entry: &'a Json, key: &str) -> Option<&'a str> {
    match entry.get(key) {
        None | Some(Json::Null) => Some(""),
        Some(value) => value.as_str(),
    }
}

/// A description without control characters, trimmed, and at most [`DESCRIPTION_LIMIT`] characters.
fn clean_description(text: &str) -> String {
    clean_text(text, DESCRIPTION_LIMIT)
}

/// Text without control characters, trimmed, and at most `limit` characters.
fn clean_text(text: &str, limit: usize) -> String {
    // Control characters, line breaks among them, become spaces, so words they separated stay
    // apart; runs of whitespace then collapse to one space.
    let spaced: String = text.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    let words = spaced.split_whitespace().collect::<Vec<_>>().join(" ");
    words.chars().take(limit).collect::<String>().trim().to_string()
}

/// Whether one half of a repository name is valid: ASCII letters, digits, hyphens, underscores,
/// and dots, and not `.` or `..`.
fn valid_name_part(part: &str) -> bool {
    !part.is_empty() && part != "." && part != ".." && part.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
}

/// The `GH_HOST` value for a call that names `host`: github.com, or an Enterprise host that
/// passes [`valid_enterprise_host`], in lower case. Any other host yields `None`, and the call is refused.
fn gh_host_variable(host: &str) -> Option<String> {
    let host = host.to_ascii_lowercase();
    (host == "github.com" || valid_enterprise_host(&host)).then_some(host)
}

/// Decodes `gh --json` output for one kind of item, refusing links that do not point at the
/// repository's own item pages.
pub fn decode_items(json: &str, repository: &GitHubRepository, kind: ItemKind) -> Result<Vec<GitHubItem>> {
    let unexpected = || GitError::failed("GitHub", "GitHub returned unexpected items.");
    let value = parse_json(json).map_err(|_| unexpected())?;
    let Json::Array(entries) = value else { return Err(unexpected()) };
    let expected_prefix = format!("{}/{}/{}/", repository.web_base(), repository.slug(), kind.path_segment()).to_ascii_lowercase();
    let mut items = Vec::with_capacity(entries.len());
    for entry in &entries {
        let Json::Object(_) = entry else { return Err(unexpected()) };
        let number = entry.get("number").and_then(Json::as_number).filter(|n| n.fract() == 0.0 && *n >= 1.0 && *n < 9.0e15);
        let number = number.ok_or_else(unexpected)? as u64;
        let title = entry.get("title").and_then(Json::as_str).ok_or_else(unexpected)?.to_string();
        let url = entry.get("url").and_then(Json::as_str).ok_or_else(unexpected)?.to_string();
        let expected = format!("{expected_prefix}{number}").to_ascii_lowercase();
        if url.to_ascii_lowercase() != expected {
            return Err(unexpected());
        }
        let author = entry.get("author").and_then(|author| author.get("login")).and_then(Json::as_str).map(str::to_string);
        let state = entry.get("state").and_then(Json::as_str).map(str::to_string);
        let is_draft = entry.get("isDraft").and_then(Json::as_bool).unwrap_or(false);
        let mut labels = Vec::new();
        if let Some(Json::Array(values)) = entry.get("labels") {
            for label in values {
                let name = label.get("name").and_then(Json::as_str).ok_or_else(unexpected)?.to_string();
                let color = label.get("color").and_then(Json::as_str).unwrap_or("").trim_start_matches('#').to_string();
                labels.push(GitHubLabel { name, color });
            }
        }
        items.push(GitHubItem { number, title, url, author, state, is_draft, labels });
    }
    let mut numbers: Vec<u64> = items.iter().map(|item| item.number).collect();
    numbers.sort_unstable();
    numbers.dedup();
    if numbers.len() != items.len() {
        return Err(unexpected());
    }
    Ok(items)
}

/// The longest pull request title kept, in characters.
const PULL_REQUEST_TITLE_LIMIT: usize = 300;

/// Decodes `gh pr list --json number,title,state,isDraft,url,statusCheckRollup` output for
/// `repository`. Each entry is checked strictly, and its link must be the repository's own pull
/// request page. An empty list is `None`. Of several pull requests, the first open or draft one
/// is returned, or else the first listed.
pub fn decode_pull_request_status(json: &str, repository: &GitHubRepository) -> Result<Option<PullRequestStatus>> {
    let unexpected = || GitError::failed("GitHub", "GitHub returned an unexpected pull request.");
    let value = parse_json(json).map_err(|_| unexpected())?;
    let Json::Array(entries) = value else { return Err(unexpected()) };
    let statuses = entries.iter().map(|entry| decode_pull_request_entry(entry, repository)).collect::<Result<Vec<_>>>()?;
    let chosen = statuses.iter().position(|status| matches!(status.state, PrState::Open | PrState::Draft)).unwrap_or(0);
    Ok(statuses.into_iter().nth(chosen))
}

/// One pull request in `gh pr list` output. Its state must be a known word, its link must name
/// the same number on the repository's own page, and its checks must all be recognised shapes.
fn decode_pull_request_entry(entry: &Json, repository: &GitHubRepository) -> Result<PullRequestStatus> {
    let unexpected = || GitError::failed("GitHub", "GitHub returned an unexpected pull request.");
    let Json::Object(_) = entry else { return Err(unexpected()) };
    let number = entry.get("number").and_then(Json::as_number).filter(|n| n.fract() == 0.0 && *n >= 1.0 && *n < 9.0e15);
    let number = number.ok_or_else(unexpected)? as u64;
    let title = entry.get("title").and_then(Json::as_str).ok_or_else(unexpected)?;
    let url = entry.get("url").and_then(Json::as_str).ok_or_else(unexpected)?;
    let expected = format!("{}/{}/pull/{number}", repository.web_base(), repository.slug()).to_ascii_lowercase();
    if url.to_ascii_lowercase() != expected {
        return Err(unexpected());
    }
    let is_draft = entry.get("isDraft").and_then(Json::as_bool).ok_or_else(unexpected)?;
    let state = match entry.get("state").and_then(Json::as_str) {
        Some("OPEN") if is_draft => PrState::Draft,
        Some("OPEN") => PrState::Open,
        Some("MERGED") => PrState::Merged,
        Some("CLOSED") => PrState::Closed,
        _ => return Err(unexpected()),
    };
    let Some(Json::Array(rollup)) = entry.get("statusCheckRollup") else { return Err(unexpected()) };
    let mut checks = Checks::default();
    for check in rollup {
        match check_outcome(check).ok_or_else(unexpected)? {
            CheckOutcome::Passed => checks.passed += 1,
            CheckOutcome::Failed => checks.failed += 1,
            CheckOutcome::Pending => checks.pending += 1,
        }
    }
    Ok(PullRequestStatus { number, title: clean_text(title, PULL_REQUEST_TITLE_LIMIT), state, url: url.to_string(), checks })
}

/// How one check in a pull request's status rollup stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CheckOutcome {
    Passed,
    Failed,
    Pending,
}

/// The outcome of a check-run entry (with `status` and `conclusion`) or a commit status entry
/// (with `state`). A check that has not completed, or has no conclusion yet, is pending. `None`
/// means the entry has neither shape.
fn check_outcome(check: &Json) -> Option<CheckOutcome> {
    let Json::Object(_) = check else { return None };
    if let Some(state) = check.get("state") {
        return Some(classify_check(state.as_str()?));
    }
    if check.get("status").is_none() && check.get("conclusion").is_none() {
        return None;
    }
    let status = optional_text(check, "status")?;
    if !status.is_empty() && status != "COMPLETED" {
        return Some(CheckOutcome::Pending);
    }
    match check.get("conclusion") {
        None | Some(Json::Null) => Some(CheckOutcome::Pending),
        Some(conclusion) => Some(classify_check(conclusion.as_str()?)),
    }
}

/// The outcome for a check conclusion or commit status word. Words not listed count as pending.
fn classify_check(word: &str) -> CheckOutcome {
    match word {
        "SUCCESS" | "NEUTRAL" | "SKIPPED" => CheckOutcome::Passed,
        "FAILURE" | "CANCELLED" | "TIMED_OUT" | "ACTION_REQUIRED" | "ERROR" | "STARTUP_FAILURE" => CheckOutcome::Failed,
        _ => CheckOutcome::Pending,
    }
}

/// The first `gh` on the search path that is an executable file.
fn gh_executable() -> Option<PathBuf> {
    let name = if cfg!(windows) { "gh.exe" } else { "gh" };
    std::env::split_paths(runner::search_path()).map(|directory| directory.join(name)).find(|candidate| is_executable(candidate))
}

fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata().map(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0).unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

/// The message for a request the user cancelled.
pub const CANCELLED: &str = "Cancelled.";

/// Set from another thread to end a running GitHub request; its `gh` process is stopped.
#[derive(Clone, Debug, Default)]
pub struct Cancel(std::sync::Arc<std::sync::atomic::AtomicBool>);

impl Cancel {
    pub fn cancel(&self) {
        self.0.store(true, std::sync::atomic::Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(std::sync::atomic::Ordering::SeqCst)
    }
}

/// Runs `gh` and returns its standard output. On failure the error is the text to show.
/// `host`, when given, is set as `GH_HOST` for this call only; it must be github.com or a valid
/// Enterprise host. Prompts are disabled, and the request is abandoned after a timeout.
fn run_gh(arguments: &[&str], directory: &Path, cancel: &Cancel, host: Option<&str>) -> std::result::Result<String, String> {
    let host = match host {
        Some(host) => Some(gh_host_variable(host).ok_or_else(|| "The GitHub host is not supported.".to_string())?),
        None => None,
    };
    let executable = gh_executable().ok_or_else(|| NOT_INSTALLED.to_string())?;
    run_program(&executable, arguments, directory, cancel, host.as_deref())
}

/// Runs a program to completion, its timeout, or cancellation, returning its standard output.
fn run_program(
    executable: &Path,
    arguments: &[&str],
    directory: &Path,
    cancel: &Cancel,
    gh_host: Option<&str>,
) -> std::result::Result<String, String> {
    let mut command = Command::new(executable);
    command.args(arguments).current_dir(directory).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    command.env("PATH", runner::search_path());
    // Variables that would point gh at another repository or host. A host is set only when asked for.
    command.env_remove("GH_REPO");
    command.env_remove("GH_HOST");
    if let Some(host) = gh_host {
        command.env("GH_HOST", host);
    }
    command.env("GH_PROMPT_DISABLED", "1");
    command.env("GH_NO_UPDATE_NOTIFIER", "1");
    command.env("NO_COLOR", "1");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // Without this, every gh call flashes a console window.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = command.spawn().map_err(|error| error.to_string())?;
    // Read both pipes while waiting, so a large output cannot block the process.
    let mut stdout = child.stdout.take();
    let mut stderr = child.stderr.take();
    let out_reader = std::thread::spawn(move || {
        let mut buffer = Vec::new();
        if let Some(pipe) = stdout.as_mut() {
            let _ = pipe.read_to_end(&mut buffer);
        }
        buffer
    });
    let err_reader = std::thread::spawn(move || {
        let mut buffer = Vec::new();
        if let Some(pipe) = stderr.as_mut() {
            let _ = pipe.read_to_end(&mut buffer);
        }
        buffer
    });
    let deadline = Instant::now() + TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if cancel.is_cancelled() => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(CANCELLED.to_string());
            }
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("GitHub request timed out. Retry when the connection is available.".to_string());
            }
            Err(error) => return Err(error.to_string()),
        }
    };
    let stdout = out_reader.join().unwrap_or_default();
    let stderr = err_reader.join().unwrap_or_default();
    if status.success() {
        return Ok(String::from_utf8_lossy(&stdout).into_owned());
    }
    let message = String::from_utf8_lossy(&stderr).trim().to_string();
    let message = if message.is_empty() { "GitHub request failed.".to_string() } else { message };
    Err(message.chars().take(MESSAGE_LIMIT).collect())
}

// MARK: JSON

/// A parsed JSON value. Only what `gh` output needs is supported, which is all of the standard
/// grammar.
#[derive(Clone, Debug, PartialEq)]
enum Json {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<Json>),
    Object(Vec<(String, Json)>),
}

impl Json {
    fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Object(fields) => fields.iter().find(|(name, _)| name == key).map(|(_, value)| value),
            _ => None,
        }
    }

    fn as_str(&self) -> Option<&str> {
        match self {
            Json::String(text) => Some(text),
            _ => None,
        }
    }

    fn as_number(&self) -> Option<f64> {
        match self {
            Json::Number(number) => Some(*number),
            _ => None,
        }
    }

    fn as_bool(&self) -> Option<bool> {
        match self {
            Json::Bool(value) => Some(*value),
            _ => None,
        }
    }
}

const MAX_DEPTH: usize = 64;

fn parse_json(text: &str) -> std::result::Result<Json, String> {
    let mut parser = JsonParser { bytes: text.as_bytes(), position: 0 };
    let value = parser.value(0)?;
    parser.skip_whitespace();
    if parser.position != parser.bytes.len() {
        return Err("Unexpected text after the JSON value.".to_string());
    }
    Ok(value)
}

struct JsonParser<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl JsonParser<'_> {
    fn skip_whitespace(&mut self) {
        while let Some(byte) = self.bytes.get(self.position) {
            if matches!(byte, b' ' | b'\t' | b'\n' | b'\r') {
                self.position += 1;
            } else {
                break;
            }
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.position).copied()
    }

    fn expect_literal(&mut self, literal: &str) -> std::result::Result<(), String> {
        if self.bytes[self.position..].starts_with(literal.as_bytes()) {
            self.position += literal.len();
            Ok(())
        } else {
            Err(format!("Expected {literal}."))
        }
    }

    fn value(&mut self, depth: usize) -> std::result::Result<Json, String> {
        if depth > MAX_DEPTH {
            return Err("JSON is nested too deeply.".to_string());
        }
        self.skip_whitespace();
        match self.peek().ok_or("Unexpected end of JSON.")? {
            b'{' => self.object(depth),
            b'[' => self.array(depth),
            b'"' => self.string().map(Json::String),
            b't' => self.expect_literal("true").map(|_| Json::Bool(true)),
            b'f' => self.expect_literal("false").map(|_| Json::Bool(false)),
            b'n' => self.expect_literal("null").map(|_| Json::Null),
            b'-' | b'0'..=b'9' => self.number(),
            _ => Err("Unexpected character in JSON.".to_string()),
        }
    }

    fn object(&mut self, depth: usize) -> std::result::Result<Json, String> {
        self.position += 1;
        let mut fields = Vec::new();
        self.skip_whitespace();
        if self.peek() == Some(b'}') {
            self.position += 1;
            return Ok(Json::Object(fields));
        }
        loop {
            self.skip_whitespace();
            if self.peek() != Some(b'"') {
                return Err("Expected an object key.".to_string());
            }
            let key = self.string()?;
            self.skip_whitespace();
            if self.peek() != Some(b':') {
                return Err("Expected ':' in an object.".to_string());
            }
            self.position += 1;
            let value = self.value(depth + 1)?;
            fields.push((key, value));
            self.skip_whitespace();
            match self.peek() {
                Some(b',') => self.position += 1,
                Some(b'}') => {
                    self.position += 1;
                    return Ok(Json::Object(fields));
                }
                _ => return Err("Expected ',' or '}' in an object.".to_string()),
            }
        }
    }

    fn array(&mut self, depth: usize) -> std::result::Result<Json, String> {
        self.position += 1;
        let mut values = Vec::new();
        self.skip_whitespace();
        if self.peek() == Some(b']') {
            self.position += 1;
            return Ok(Json::Array(values));
        }
        loop {
            values.push(self.value(depth + 1)?);
            self.skip_whitespace();
            match self.peek() {
                Some(b',') => self.position += 1,
                Some(b']') => {
                    self.position += 1;
                    return Ok(Json::Array(values));
                }
                _ => return Err("Expected ',' or ']' in an array.".to_string()),
            }
        }
    }

    fn number(&mut self) -> std::result::Result<Json, String> {
        let start = self.position;
        while let Some(byte) = self.peek() {
            if byte.is_ascii_digit() || matches!(byte, b'-' | b'+' | b'.' | b'e' | b'E') {
                self.position += 1;
            } else {
                break;
            }
        }
        let text = std::str::from_utf8(&self.bytes[start..self.position]).map_err(|_| "Invalid number.".to_string())?;
        text.parse::<f64>().map(Json::Number).map_err(|_| "Invalid number.".to_string())
    }

    fn hex4(&mut self) -> std::result::Result<u32, String> {
        let digits = self.bytes.get(self.position..self.position + 4).ok_or("Invalid unicode escape.")?;
        let text = std::str::from_utf8(digits).map_err(|_| "Invalid unicode escape.".to_string())?;
        let value = u32::from_str_radix(text, 16).map_err(|_| "Invalid unicode escape.".to_string())?;
        self.position += 4;
        Ok(value)
    }

    fn string(&mut self) -> std::result::Result<String, String> {
        self.position += 1;
        let mut buffer: Vec<u8> = Vec::new();
        loop {
            let byte = self.peek().ok_or("Unterminated string in JSON.")?;
            self.position += 1;
            match byte {
                b'"' => break,
                b'\\' => {
                    let escape = self.peek().ok_or("Unterminated escape in JSON.")?;
                    self.position += 1;
                    let character = match escape {
                        b'"' => '"',
                        b'\\' => '\\',
                        b'/' => '/',
                        b'b' => '\u{8}',
                        b'f' => '\u{c}',
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        b'u' => {
                            let high = self.hex4()?;
                            if (0xD800..0xDC00).contains(&high) && self.bytes[self.position..].starts_with(b"\\u") {
                                self.position += 2;
                                let low = self.hex4()?;
                                if !(0xDC00..0xE000).contains(&low) {
                                    return Err("Invalid surrogate pair in JSON.".to_string());
                                }
                                let code = 0x10000 + ((high - 0xD800) << 10) + (low - 0xDC00);
                                char::from_u32(code).ok_or("Invalid surrogate pair in JSON.")?
                            } else {
                                char::from_u32(high).unwrap_or('\u{FFFD}')
                            }
                        }
                        _ => return Err("Invalid escape in JSON.".to_string()),
                    };
                    let mut encoded = [0u8; 4];
                    buffer.extend_from_slice(character.encode_utf8(&mut encoded).as_bytes());
                }
                // Control characters must be escaped in JSON strings.
                0..=0x1f => return Err("Unescaped control character in JSON.".to_string()),
                other => buffer.push(other),
            }
        }
        String::from_utf8(buffer).map_err(|_| "Invalid UTF-8 in JSON.".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nested_values_and_escapes() {
        let value = parse_json(r#"[{"n": 12, "t": "a\"bé😀", "ok": true, "x": null}]"#).unwrap();
        let Json::Array(items) = value else { panic!("expected an array") };
        assert_eq!(items[0].get("n").and_then(Json::as_number), Some(12.0));
        assert_eq!(items[0].get("t").and_then(Json::as_str), Some("a\"b\u{e9}\u{1F600}"));
        assert_eq!(items[0].get("ok").and_then(Json::as_bool), Some(true));
        assert_eq!(items[0].get("x"), Some(&Json::Null));
    }

    #[cfg(unix)]
    #[test]
    fn cancelling_stops_a_running_request_promptly() {
        let cancel = Cancel::default();
        let canceller = cancel.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            canceller.cancel();
        });
        let started = Instant::now();
        let result = run_program(Path::new("/bin/sleep"), &["30"], Path::new("/"), &cancel, None);
        assert_eq!(result, Err(CANCELLED.to_string()));
        assert!(started.elapsed() < Duration::from_secs(5), "stopped after {:?}", started.elapsed());
    }

    /// One repository entry as `gh repo list` reports it, for `host`.
    fn listing_entry(host: &str, name_with_owner: &str, ssh: &str, description: &str) -> String {
        format!(
            r#"{{"nameWithOwner": "{name_with_owner}", "description": "{description}", "url": "https://{host}/{name_with_owner}", "sshUrl": "{ssh}", "isPrivate": false, "isFork": false, "updatedAt": "2026-01-02T03:04:05Z"}}"#
        )
    }

    #[test]
    fn decode_repositories_reads_a_listing() {
        let json = r#"[
            {"nameWithOwner": "acme/app", "description": "A tool", "url": "https://git.example.com/acme/app", "sshUrl": "git@git.example.com:acme/app.git", "isPrivate": true, "isFork": false, "updatedAt": "2026-01-02T03:04:05Z"},
            {"nameWithOwner": "acme/fork", "description": "", "url": "https://git.example.com/acme/fork", "sshUrl": "", "isPrivate": false, "isFork": true, "updatedAt": "2026-02-03T00:00:00Z"}
        ]"#;
        let repositories = decode_repositories(json, "git.example.com").unwrap();
        assert_eq!(
            repositories[0],
            GitHubRepoListing {
                name_with_owner: "acme/app".into(),
                description: "A tool".into(),
                url: "https://git.example.com/acme/app".into(),
                ssh_url: "git@git.example.com:acme/app.git".into(),
                is_private: true,
                is_fork: false,
                updated_at: "2026-01-02T03:04:05Z".into(),
            }
        );
        assert!(repositories[1].is_fork);
        assert!(repositories[1].ssh_url.is_empty(), "an empty SSH address stays empty");
        let public = r#"[{"nameWithOwner": "acme/app", "description": null, "url": "https://github.com/acme/app", "sshUrl": null, "isPrivate": false, "isFork": false, "updatedAt": "2026-01-02T03:04:05Z"}]"#;
        let listed = decode_repositories(public, "github.com").unwrap();
        assert_eq!(listed[0].url, "https://github.com/acme/app");
        assert!(listed[0].description.is_empty() && listed[0].ssh_url.is_empty(), "null optional fields are empty");
        assert!(decode_repositories("[]", "github.com").unwrap().is_empty(), "an empty listing is valid");
    }

    #[test]
    fn decode_repositories_skips_entries_for_other_hosts() {
        let json = format!(
            "[{}, {}, {}, {}, {}]",
            // A hostile entry whose address points at another host.
            r#"{"nameWithOwner": "acme/evil", "description": "", "url": "https://attacker.example/acme/evil", "sshUrl": "", "isPrivate": false, "isFork": false, "updatedAt": "x"}"#,
            // github.com address for an Enterprise listing.
            r#"{"nameWithOwner": "acme/public", "description": "", "url": "https://github.com/acme/public", "sshUrl": "", "isPrivate": false, "isFork": false, "updatedAt": "x"}"#,
            // Address names another repository than nameWithOwner.
            r#"{"nameWithOwner": "acme/one", "description": "", "url": "https://git.example.com/acme/two", "sshUrl": "", "isPrivate": false, "isFork": false, "updatedAt": "x"}"#,
            // A name that is not owner/name.
            r#"{"nameWithOwner": "acme/app/extra", "description": "", "url": "https://git.example.com/acme/app/extra", "sshUrl": "", "isPrivate": false, "isFork": false, "updatedAt": "x"}"#,
            listing_entry("git.example.com", "acme/kept", "", ""),
        );
        let repositories = decode_repositories(&json, "git.example.com").unwrap();
        assert_eq!(repositories.len(), 1, "only the entry on the listed host is kept");
        assert_eq!(repositories[0].name_with_owner, "acme/kept");

        let mixed_case = r#"[{"nameWithOwner": "Acme/App", "description": "", "url": "HTTPS://GIT.example.com/acme/app", "sshUrl": "", "isPrivate": false, "isFork": false, "updatedAt": "x"}]"#;
        assert_eq!(decode_repositories(mixed_case, "git.example.com").unwrap().len(), 1, "the address is compared without case");
    }

    #[test]
    fn decode_repositories_checks_the_ssh_address() {
        let host = "git.example.com";
        let cases = [
            ("git@git.example.com:acme/app.git", "git@git.example.com:acme/app.git"),
            ("ssh://git@git.example.com/acme/app", "ssh://git@git.example.com/acme/app"),
            // Another host, another owner or name, or a port: the SSH address is dropped.
            ("git@github.com:acme/app.git", ""),
            ("git@git.example.com:other/app.git", ""),
            ("git@git.example.com:acme/other.git", ""),
            ("ssh://git@git.example.com:2222/acme/app", ""),
            ("git@attacker.example:acme/app.git", ""),
            ("not an address", ""),
        ];
        for (ssh, expected) in cases {
            let json = format!("[{}]", listing_entry(host, "acme/app", ssh, ""));
            let repositories = decode_repositories(&json, host).unwrap();
            assert_eq!(repositories.len(), 1, "{ssh}: the entry itself is kept");
            assert_eq!(repositories[0].ssh_url, expected, "{ssh}");
            assert_eq!(repositories[0].url, "https://git.example.com/acme/app", "the HTTPS address is kept");
        }
    }

    #[test]
    fn decode_repositories_refuses_non_arrays() {
        for json in [
            r#"{"nameWithOwner": "acme/app"}"#,
            r#""acme/app""#,
            "[1]",
            "[null]",
            "[]x",
            "not json",
            r#"[{"nameWithOwner": 5, "url": "https://git.example.com/acme/app", "isPrivate": false, "isFork": false, "updatedAt": "x"}]"#,
            r#"[{"nameWithOwner": "acme/app", "url": "https://git.example.com/acme/app", "isFork": false, "updatedAt": "x"}]"#,
        ] {
            assert!(decode_repositories(json, "git.example.com").is_err(), "{json} should be refused");
        }
        assert!(decode_repositories("[]", "bad host").is_err(), "an unsupported host is refused");
    }

    #[test]
    fn decode_repositories_trims_and_cleans_descriptions() {
        // The descriptions are JSON text, so their control characters are escaped.
        let cases = [
            ("  Build\\u0007 tools  ", "Build tools"),
            ("\\u0000Tabs\\tand\\u001bescapes  ", "Tabs and escapes"),
            ("Two\\nlines", "Two lines"),
            ("   ", ""),
        ];
        for (description, expected) in cases {
            let json = format!("[{}]", listing_entry("git.example.com", "acme/app", "", description));
            assert_eq!(decode_repositories(&json, "git.example.com").unwrap()[0].description, expected, "{description:?}");
        }

        let long = "é".repeat(600);
        let json = format!("[{}]", listing_entry("git.example.com", "acme/app", "", &long));
        let kept = decode_repositories(&json, "git.example.com").unwrap()[0].description.clone();
        assert_eq!(kept.chars().count(), DESCRIPTION_LIMIT, "counted in characters, not bytes");
        assert!(kept.chars().all(|c| c == 'é'));

        let spaced = format!("{}{}", "a".repeat(499), " bb");
        let json = format!("[{}]", listing_entry("git.example.com", "acme/app", "", &spaced));
        let kept = decode_repositories(&json, "git.example.com").unwrap()[0].description.clone();
        assert_eq!(kept, "a".repeat(499), "the cut is trimmed again, so no trailing space is kept");
    }

    #[test]
    fn the_gh_host_is_set_only_to_github_com_or_a_valid_enterprise_host() {
        assert_eq!(gh_host_variable("github.com").as_deref(), Some("github.com"));
        assert_eq!(gh_host_variable("GitHub.com").as_deref(), Some("github.com"));
        assert_eq!(gh_host_variable("Git.Example.com").as_deref(), Some("git.example.com"));
        for refused in ["", "example.com:8443", "https://git.example.com", " git.example.com", "nodot", "git.example.com/x", "-x.com"] {
            assert_eq!(gh_host_variable(refused), None, "{refused:?} must not become GH_HOST");
        }
    }

    #[test]
    fn rejects_malformed_json() {
        assert!(parse_json("[1,").is_err());
        assert!(parse_json("[1] x").is_err());
        assert!(parse_json("\"a\nb\"").is_err());
    }

    fn hosts(list: &[&str]) -> Vec<String> {
        list.iter().map(|host| host.to_string()).collect()
    }

    fn repository(host: &str, owner: &str, name: &str) -> GitHubRepository {
        GitHubRepository { host: host.into(), owner: owner.into(), name: name.into() }
    }

    #[test]
    fn enterprise_addresses_parse_in_every_form_when_their_host_is_listed() {
        let listed = hosts(&["git.example.com"]);
        let accepted = [
            "https://git.example.com/acme/app.git",
            "https://user@git.example.com/acme/app",
            "ssh://git@git.example.com/acme/app.git",
            "git@git.example.com:acme/app.git",
            "git.example.com:acme/app",
        ];
        for address in accepted {
            let parsed = GitHubRepository::parse_with_hosts(address, &listed).unwrap_or_else(|error| panic!("{address}: {error}"));
            assert_eq!(parsed, repository("git.example.com", "acme", "app"), "{address}");
        }
    }

    #[test]
    fn enterprise_addresses_are_refused_for_an_unlisted_host() {
        let listed = hosts(&["git.example.com"]);
        for address in ["https://other.example.com/acme/app", "git@other.example.com:acme/app.git"] {
            let error = GitHubRepository::parse_with_hosts(address, &listed).unwrap_err().to_string();
            assert!(
                error.contains("This remote is not on github.com or a GitHub Enterprise host listed in Settings."),
                "{address}: {error}"
            );
        }
        assert!(GitHubRepository::parse_with_hosts("https://git.example.com/acme/app", &[]).is_err(), "nothing is listed");
    }

    #[test]
    fn a_listed_enterprise_host_with_a_port_is_still_refused() {
        let listed = hosts(&["git.example.com"]);
        for address in
            ["https://git.example.com:8443/acme/app", "ssh://git.example.com:2222/acme/app", "https://git.example.com:443/acme/app"]
        {
            assert!(GitHubRepository::parse_with_hosts(address, &listed).is_err(), "{address} should be refused");
        }
    }

    #[test]
    fn github_com_still_parses_with_no_enterprise_hosts() {
        assert_eq!(GitHubRepository::parse("https://github.com/acme/app.git").unwrap(), repository("github.com", "acme", "app"));
        assert_eq!(GitHubRepository::parse_with_hosts("git@github.com:acme/app", &[]).unwrap(), repository("github.com", "acme", "app"));
        assert!(GitHubRepository::parse("https://git.example.com/acme/app").is_err(), "parse accepts github.com only");
    }

    #[test]
    fn an_enterprise_host_is_lowercased_and_matched_without_regard_to_case() {
        let listed = hosts(&["Git.Example.COM"]);
        let parsed = GitHubRepository::parse_with_hosts("https://GIT.example.com/Acme/App", &listed).unwrap();
        assert_eq!(parsed.host, "git.example.com");
        assert_eq!(parsed.owner, "Acme", "owners keep their case");
        assert_eq!(GitHubRepository::parse_with_hosts("https://GitHub.com/acme/app", &[]).unwrap().host, "github.com");
    }

    #[test]
    fn web_base_and_gh_repo_name_the_host() {
        let enterprise = repository("git.example.com", "acme", "app");
        assert_eq!(enterprise.web_base(), "https://git.example.com");
        assert_eq!(enterprise.gh_repo(), "git.example.com/acme/app");
        assert_eq!(enterprise.slug(), "acme/app");
        let public = repository("github.com", "acme", "app");
        assert_eq!(public.web_base(), "https://github.com");
        assert_eq!(public.gh_repo(), "github.com/acme/app");
    }

    #[test]
    fn web_url_builds_home_and_section_pages_from_the_parsed_repository() {
        let public = GitHubRepository::parse("https://github.com/acme/widgets.git").unwrap();
        assert_eq!(public.web_url(None), "https://github.com/acme/widgets");
        assert_eq!(public.web_url(Some("pulls")), "https://github.com/acme/widgets/pulls");
        assert_eq!(public.web_url(Some("issues")), "https://github.com/acme/widgets/issues");
        assert_eq!(public.web_url(Some("actions")), "https://github.com/acme/widgets/actions");
        let dotted = GitHubRepository::parse("git@github.com:acme-labs/my.repo-2.git").unwrap();
        assert_eq!(dotted.web_url(None), "https://github.com/acme-labs/my.repo-2");
        assert_eq!(dotted.web_url(Some("actions")), "https://github.com/acme-labs/my.repo-2/actions");
    }

    #[test]
    fn web_url_on_an_enterprise_host_drops_credentials_and_ports_from_the_remote() {
        let listed = hosts(&["Git.Example.com"]);
        let enterprise = GitHubRepository::parse_with_hosts("https://octo@GIT.example.com/acme/app.git", &listed).unwrap();
        assert_eq!(enterprise.web_url(None), "https://git.example.com/acme/app");
        assert_eq!(enterprise.web_url(Some("pulls")), "https://git.example.com/acme/app/pulls");
        assert_eq!(enterprise.web_url(Some("issues")), "https://git.example.com/acme/app/issues");
        assert_eq!(enterprise.web_url(Some("actions")), "https://git.example.com/acme/app/actions");
        assert_eq!(repository("git.example.com", "acme", "app").web_url(None), "https://git.example.com/acme/app");
    }

    #[test]
    fn decode_items_accepts_links_on_the_repositorys_own_enterprise_host() {
        let listed = hosts(&["git.example.com"]);
        let repository = GitHubRepository::parse_with_hosts("https://git.example.com/acme/app", &listed).unwrap();
        let json = r#"[{"number": 7, "title": "Fix", "url": "https://git.example.com/acme/app/pull/7"}]"#;
        let items = decode_items(json, &repository, ItemKind::PullRequest).unwrap();
        assert_eq!(items[0].number, 7);
        let issues = r#"[{"number": 2, "title": "Bug", "url": "https://GIT.example.com/Acme/App/issues/2"}]"#;
        assert_eq!(decode_items(issues, &repository, ItemKind::Issue).unwrap().len(), 1, "case is ignored, as for github.com");
    }

    #[test]
    fn decode_items_refuses_links_on_another_host() {
        let listed = hosts(&["git.example.com"]);
        let enterprise = GitHubRepository::parse_with_hosts("https://git.example.com/acme/app", &listed).unwrap();
        let on_github = r#"[{"number": 7, "title": "Fix", "url": "https://github.com/acme/app/pull/7"}]"#;
        assert!(decode_items(on_github, &enterprise, ItemKind::PullRequest).is_err(), "github.com link for an enterprise repository");

        let public = GitHubRepository::parse("https://github.com/acme/app").unwrap();
        let on_enterprise = r#"[{"number": 7, "title": "Fix", "url": "https://git.example.com/acme/app/pull/7"}]"#;
        assert!(decode_items(on_enterprise, &public, ItemKind::PullRequest).is_err(), "enterprise link for a github.com repository");
    }

    #[test]
    fn valid_enterprise_host_accepts_dns_names_and_refuses_the_rest() {
        for good in ["git.example.com", "ghe-1.corp.internal", "GIT.Example.COM", "a.b", "10.0.0.7"] {
            assert!(valid_enterprise_host(good), "{good} should be accepted");
        }
        for bad in [
            "-bad.com",
            "bad-.com",
            "nodot",
            "host:8443",
            "https://x.com",
            "a b.com",
            "github.com",
            "x.com/path",
            "x..com",
            "x.com.",
            "x_y.com",
            "",
            " git.example.com",
        ] {
            assert!(!valid_enterprise_host(bad), "{bad:?} should be refused");
        }
    }

    #[test]
    fn valid_enterprise_host_refuses_a_host_longer_than_253_characters() {
        let at_limit = format!("{}.{}.{}.{}", "a".repeat(63), "a".repeat(63), "a".repeat(63), "a".repeat(61));
        assert_eq!(at_limit.len(), 253);
        assert!(valid_enterprise_host(&at_limit));
        let too_long = format!("{}.{}.{}.{}", "a".repeat(63), "a".repeat(63), "a".repeat(63), "a".repeat(62));
        assert_eq!(too_long.len(), 254);
        assert!(!valid_enterprise_host(&too_long));
    }

    #[test]
    fn valid_enterprise_host_refuses_a_label_longer_than_63_characters() {
        let label = format!("{}.example.com", "a".repeat(64));
        assert!(!valid_enterprise_host(&label));
    }

    #[test]
    fn pull_request_arguments_put_values_in_their_own_arguments() {
        let repository = repository("github.com", "acme", "app");
        let arguments = pull_request_arguments(&repository, "feature", "main", "-n flag-like title", "/tmp/body.md", true);
        assert_eq!(
            arguments,
            [
                "pr",
                "create",
                "--repo",
                "github.com/acme/app",
                "--head",
                "feature",
                "--base",
                "main",
                "--title=-n flag-like title",
                "--body-file",
                "/tmp/body.md",
                "--draft",
            ]
        );
        let plain = pull_request_arguments(&repository, "feature", "main", "Title", "/tmp/body.md", false);
        assert!(!plain.iter().any(|argument| argument == "--draft"), "draft is off unless asked for");
        assert_eq!(plain.last().map(String::as_str), Some("/tmp/body.md"));
    }

    #[test]
    fn pull_request_arguments_keep_a_multi_line_body_out_of_the_command_line() {
        let repository = repository("git.example.com", "acme", "app");
        let body = "- first change\n- second change\n\n--title=injected";
        // The caller writes the body to a file; the arguments name only that file.
        let arguments = pull_request_arguments(&repository, "feature", "main", "Title", "/tmp/nicegit/body.md", false);
        assert!(arguments.iter().all(|argument| !argument.contains("first change") && !argument.contains("injected")), "{body}");
        assert!(arguments.contains(&"--repo".to_string()) && arguments.contains(&"git.example.com/acme/app".to_string()));
    }

    #[test]
    fn parse_created_url_accepts_the_repositorys_pull_url() {
        let github = repository("github.com", "acme", "app");
        assert_eq!(
            parse_created_url("https://github.com/acme/app/pull/12\n", &github).as_deref(),
            Some("https://github.com/acme/app/pull/12")
        );
        assert_eq!(
            parse_created_url("Creating pull request for feature into main\n\nhttps://github.com/Acme/App/pull/12\n", &github).as_deref(),
            Some("https://github.com/Acme/App/pull/12"),
            "the last line counts, and case is ignored"
        );

        let enterprise = GitHubRepository::parse_with_hosts("https://git.example.com/acme/app", &hosts(&["git.example.com"])).unwrap();
        assert_eq!(
            parse_created_url("https://git.example.com/acme/app/pull/3", &enterprise).as_deref(),
            Some("https://git.example.com/acme/app/pull/3")
        );
    }

    #[test]
    fn parse_created_url_refuses_other_hosts_and_paths() {
        let github = repository("github.com", "acme", "app");
        let enterprise = GitHubRepository::parse_with_hosts("https://git.example.com/acme/app", &hosts(&["git.example.com"])).unwrap();
        let refused = [
            (&github, "https://git.example.com/acme/app/pull/12"),
            (&github, "https://github.com/other/app/pull/12"),
            (&github, "https://github.com/acme/other/pull/12"),
            (&github, "http://github.com/acme/app/pull/12"),
            (&github, "https://github.com/acme/app/issues/12"),
            (&github, "https://github.com/acme/app/pulls/12"),
            (&github, "https://github.com/acme/app/pull/"),
            (&github, "https://github.com/acme/app/pull/12/files"),
            (&github, "https://github.com/acme/app/pull/12?x=1"),
            (&github, "https://github.com/acme/app/pull/12 trailing"),
            (&github, "https://github.com/acme/app/pull/abc"),
            (&github, "https://github.com/acme/app/pull/12\nnot the last line"),
            (&github, "no address printed"),
            (&github, ""),
            (&enterprise, "https://github.com/acme/app/pull/12"),
        ];
        for (repository, output) in refused {
            assert_eq!(parse_created_url(output, repository), None, "{output:?} must be refused");
        }
    }

    #[test]
    fn create_pull_request_refuses_an_empty_title_and_same_branch() {
        let client = GitClient::new();
        let repository = repository("github.com", "acme", "app");
        let directory = Path::new(".");
        for title in ["", "   ", "two\nlines"] {
            let error = client.create_pull_request(&repository, "feature", "main", title, "body", false, directory).unwrap_err();
            assert!(error.to_string().contains("title"), "{title:?}: {error}");
        }
        let error = client.create_pull_request(&repository, "main", "main", "Title", "", false, directory).unwrap_err();
        assert!(error.to_string().contains("must be different"), "{error}");
    }

    #[test]
    fn pull_request_status_reads_an_open_pull_request_with_mixed_checks() {
        let github = repository("github.com", "acme", "app");
        let json = r#"[{"number": 12, "title": "Add the status bar", "state": "OPEN", "isDraft": false,
            "url": "https://github.com/acme/app/pull/12", "statusCheckRollup": [
            {"__typename": "CheckRun", "name": "build", "status": "COMPLETED", "conclusion": "SUCCESS"},
            {"__typename": "CheckRun", "name": "lint", "status": "COMPLETED", "conclusion": "NEUTRAL"},
            {"__typename": "CheckRun", "name": "docs", "status": "COMPLETED", "conclusion": "SKIPPED"},
            {"__typename": "CheckRun", "name": "test", "status": "COMPLETED", "conclusion": "FAILURE"},
            {"__typename": "CheckRun", "name": "deploy", "status": "COMPLETED", "conclusion": "TIMED_OUT"},
            {"__typename": "CheckRun", "name": "audit", "status": "IN_PROGRESS", "conclusion": null},
            {"__typename": "CheckRun", "name": "queued", "status": "QUEUED", "conclusion": ""},
            {"__typename": "CheckRun", "name": "odd", "status": "COMPLETED", "conclusion": "WHATEVER"}
        ]}]"#;
        let status = decode_pull_request_status(json, &github).unwrap().expect("a pull request");
        assert_eq!(status.number, 12);
        assert_eq!(status.title, "Add the status bar");
        assert_eq!(status.state, PrState::Open);
        assert_eq!(status.url, "https://github.com/acme/app/pull/12");
        assert_eq!(status.checks, Checks { passed: 3, failed: 2, pending: 3 });
    }

    #[test]
    fn pull_request_status_reads_a_draft_without_checks() {
        let github = repository("github.com", "acme", "app");
        let json = r#"[{"number": 4, "title": "Wip", "state": "OPEN", "isDraft": true,
            "url": "https://github.com/acme/app/pull/4", "statusCheckRollup": []}]"#;
        let status = decode_pull_request_status(json, &github).unwrap().expect("a pull request");
        assert_eq!(status.state, PrState::Draft);
        assert_eq!(status.checks, Checks::default());
    }

    #[test]
    fn pull_request_status_reads_a_merged_pull_request() {
        let github = repository("github.com", "acme", "app");
        let json = r#"[{"number": 30, "title": "Merged work", "state": "MERGED", "isDraft": false,
            "url": "https://github.com/acme/app/pull/30", "statusCheckRollup": [
            {"__typename": "CheckRun", "name": "build", "status": "COMPLETED", "conclusion": "SUCCESS"}]}]"#;
        let status = decode_pull_request_status(json, &github).unwrap().expect("a pull request");
        assert_eq!(status.state, PrState::Merged);
        assert_eq!(status.checks, Checks { passed: 1, failed: 0, pending: 0 });
    }

    #[test]
    fn pull_request_status_is_none_for_a_branch_with_no_pull_request() {
        let github = repository("github.com", "acme", "app");
        assert_eq!(decode_pull_request_status("[]", &github).unwrap(), None);
        assert!(decode_pull_request_status("{}", &github).is_err(), "an object is not a list");
    }

    #[test]
    fn pull_request_status_refuses_a_link_to_another_repository() {
        let github = repository("github.com", "acme", "app");
        let other_repository = r#"[{"number": 12, "title": "Fix", "state": "OPEN", "isDraft": false,
            "url": "https://github.com/acme/other/pull/12", "statusCheckRollup": []}]"#;
        assert!(decode_pull_request_status(other_repository, &github).is_err());
        let other_number = r#"[{"number": 12, "title": "Fix", "state": "OPEN", "isDraft": false,
            "url": "https://github.com/acme/app/pull/13", "statusCheckRollup": []}]"#;
        assert!(decode_pull_request_status(other_number, &github).is_err(), "the link must name this number");
        let enterprise = r#"[{"number": 12, "title": "Fix", "state": "OPEN", "isDraft": false,
            "url": "https://git.example.com/acme/app/pull/12", "statusCheckRollup": []}]"#;
        assert!(decode_pull_request_status(enterprise, &github).is_err(), "a link on another host");
    }

    #[test]
    fn pull_request_status_cleans_a_hostile_title() {
        let github = repository("github.com", "acme", "app");
        let json = r#"[{"number": 9, "title": "Fix\u0007 the\nbar\u0000 \u001b[31m x", "state": "OPEN", "isDraft": false,
            "url": "https://github.com/acme/app/pull/9", "statusCheckRollup": []}]"#;
        let status = decode_pull_request_status(json, &github).unwrap().expect("a pull request");
        assert_eq!(status.title, "Fix the bar [31m x");
        assert!(!status.title.chars().any(char::is_control));

        let long = "a".repeat(400);
        let json = format!(
            r#"[{{"number": 9, "title": "{long}", "state": "OPEN", "isDraft": false, "url": "https://github.com/acme/app/pull/9", "statusCheckRollup": []}}]"#
        );
        let status = decode_pull_request_status(&json, &github).unwrap().expect("a pull request");
        assert_eq!(status.title.chars().count(), 300);
    }

    #[test]
    fn pull_request_status_counts_commit_status_contexts() {
        let github = repository("github.com", "acme", "app");
        let json = r#"[{"number": 5, "title": "Statuses", "state": "OPEN", "isDraft": false,
            "url": "https://github.com/acme/app/pull/5", "statusCheckRollup": [
            {"__typename": "StatusContext", "context": "ci/a", "state": "SUCCESS"},
            {"__typename": "StatusContext", "context": "ci/b", "state": "FAILURE"},
            {"__typename": "StatusContext", "context": "ci/c", "state": "ERROR"},
            {"__typename": "StatusContext", "context": "ci/d", "state": "PENDING"},
            {"__typename": "StatusContext", "context": "ci/e", "state": "EXPECTED"},
            {"__typename": "StatusContext", "context": "ci/f", "state": "MYSTERY"}
        ]}]"#;
        let status = decode_pull_request_status(json, &github).unwrap().expect("a pull request");
        assert_eq!(status.checks, Checks { passed: 1, failed: 2, pending: 3 });
    }

    #[test]
    fn pull_request_status_prefers_an_open_pull_request() {
        let github = repository("github.com", "acme", "app");
        let json = r#"[
            {"number": 20, "title": "Old", "state": "CLOSED", "isDraft": false, "url": "https://github.com/acme/app/pull/20", "statusCheckRollup": []},
            {"number": 19, "title": "Newer", "state": "OPEN", "isDraft": false, "url": "https://github.com/acme/app/pull/19", "statusCheckRollup": []}
        ]"#;
        let status = decode_pull_request_status(json, &github).unwrap().expect("a pull request");
        assert_eq!(status.number, 19);
        assert_eq!(status.state, PrState::Open);
    }

    #[test]
    fn pull_request_status_refuses_unexpected_entries() {
        let github = repository("github.com", "acme", "app");
        let url = "https://github.com/acme/app/pull/3";
        let refused = [
            format!(r#"[{{"number": 3, "title": "T", "state": "SOMETHING", "isDraft": false, "url": "{url}", "statusCheckRollup": []}}]"#),
            format!(r#"[{{"number": 3, "title": "T", "state": "OPEN", "isDraft": false, "url": "{url}"}}]"#),
            format!(r#"[{{"number": 3, "title": "T", "state": "OPEN", "isDraft": false, "url": "{url}", "statusCheckRollup": {{}}}}]"#),
            format!(
                r#"[{{"number": 3, "title": "T", "state": "OPEN", "isDraft": false, "url": "{url}", "statusCheckRollup": [{{"name": "x"}}]}}]"#
            ),
            format!(
                r#"[{{"number": 3, "title": "T", "state": "OPEN", "isDraft": false, "url": "{url}", "statusCheckRollup": [{{"state": 5}}]}}]"#
            ),
            format!(r#"[{{"number": 3.5, "title": "T", "state": "OPEN", "isDraft": false, "url": "{url}", "statusCheckRollup": []}}]"#),
            r#"["not an object"]"#.to_string(),
        ];
        for json in refused {
            assert!(decode_pull_request_status(&json, &github).is_err(), "{json} must be refused");
        }
    }

    #[test]
    fn pull_request_status_refuses_a_branch_name_that_is_not_valid() {
        let client = GitClient::new();
        let github = repository("github.com", "acme", "app");
        let error = client.pull_request_for_branch(&github, "--state=closed", Path::new(".")).unwrap_err();
        assert!(error.to_string().contains("branch name"), "{error}");
    }
}
