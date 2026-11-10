//! GitLab merge requests for a GitLab remote (gitlab.com or a GitLab host listed in Settings), read
//! and created through the `glab` command-line tool, as GitHub pull requests go through `gh`. NiceGit
//! never stores a token; `glab` keeps its own sign-in.

use std::path::{Path, PathBuf};

use crate::client::GitClient;
use crate::github::{
    self, clean_text, run_program, timestamp_seconds, Cancel, Checks, OpenPullRequest, PrState, PullRequestStatus, ReviewRequest,
};
use crate::hosting::{Host, HostedRepository};
use crate::json::{parse_json, Json};
use crate::models::{GitError, Result};

const SERVICE: &str = "GitLab";
/// The longest merge request title kept, in characters.
const TITLE_LIMIT: usize = 300;

/// The `glab` program to run. `Search` finds it on NiceGit's search path, as `gh` is found. `Given`
/// runs the named file instead, so a path that does not exist shows the "not installed" state.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum GlabProgram {
    #[default]
    Search,
    Given(PathBuf),
}

impl GlabProgram {
    fn executable(&self) -> Option<PathBuf> {
        match self {
            GlabProgram::Search => github::find_program("glab"),
            GlabProgram::Given(path) => github::is_executable(path).then(|| path.clone()),
        }
    }
}

/// Whether `glab` can be used for a host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GlabStatus {
    NotInstalled,
    /// Installed, but not signed in to the host; carries `glab`'s explanation.
    SignedOut(String),
    SignedIn,
}

impl GlabStatus {
    /// Why `glab` cannot read `host`, as one line for a list, or `None` when it is installed and
    /// signed in. The explanation from `glab` is kept, without control characters.
    pub fn reason(&self, host: &str) -> Option<String> {
        match self {
            GlabStatus::SignedIn => None,
            GlabStatus::NotInstalled => Some(not_installed_message(host)),
            GlabStatus::SignedOut(explanation) => {
                let text = format!("The GitLab CLI is not signed in to {host}. Run {} in a terminal. {explanation}", sign_in_command(host));
                Some(clean_text(&text, github::MESSAGE_LIMIT))
            }
        }
    }
}

/// The values for a new merge request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewMergeRequest {
    /// The branch the merge request comes from.
    pub source: String,
    /// The branch it merges into.
    pub target: String,
    pub title: String,
    pub description: String,
    pub draft: bool,
}

/// What to show when `glab` is missing for `host`, and how to sign in once it is installed.
pub fn not_installed_message(host: &str) -> String {
    format!("The GitLab CLI (glab) is not installed. Install it from gitlab.com/gitlab-org/cli, then run {}.", sign_in_command(host))
}

/// The command that signs `glab` in to `host`.
pub fn sign_in_command(host: &str) -> String {
    format!("glab auth login --hostname {host}")
}

impl GitClient {
    /// Whether `glab` is installed and signed in to the repository's host.
    pub fn glab_status(&self, repository: &HostedRepository, program: &GlabProgram, directory: &Path) -> GlabStatus {
        let Some(executable) = program.executable() else { return GlabStatus::NotInstalled };
        match run_glab(&executable, &["auth", "status", "--hostname", &repository.host], directory) {
            Ok(_) => GlabStatus::SignedIn,
            Err(message) => GlabStatus::SignedOut(message),
        }
    }

    /// The default branch of the project, read through `glab api`. The name is checked as a branch
    /// name before it is returned.
    pub fn glab_default_branch(&self, repository: &HostedRepository, program: &GlabProgram, directory: &Path) -> Result<String> {
        let executable = program.executable().ok_or_else(|| not_installed(repository))?;
        let endpoint = format!("projects/{}", project_path(repository));
        let output = run_glab(&executable, &["api", "--hostname", &repository.host, &endpoint], directory).map_err(failed)?;
        let name = decode_default_branch(&output)?;
        self.check_branch_name(&name, directory)?;
        Ok(name)
    }

    /// Creates a merge request through `glab mr create` and returns its address. The title is one
    /// line and must not be empty, and both branch names are checked before `glab` runs. Each value
    /// is its own argument, so nothing goes through a shell.
    pub fn create_merge_request(
        &self,
        repository: &HostedRepository,
        program: &GlabProgram,
        request: &NewMergeRequest,
        directory: &Path,
    ) -> Result<String> {
        let title = request.title.trim();
        if title.is_empty() {
            return Err(GitError::failed(SERVICE, "Enter a title for the merge request."));
        }
        if title.chars().any(char::is_control) {
            return Err(GitError::failed(SERVICE, "The title must be one line."));
        }
        if request.source == request.target {
            return Err(GitError::failed(SERVICE, "The target branch must be different from the branch."));
        }
        self.check_branch_name(&request.source, directory)?;
        self.check_branch_name(&request.target, directory)?;
        let executable = program.executable().ok_or_else(|| not_installed(repository))?;
        let request = NewMergeRequest { title: title.to_string(), ..request.clone() };
        let arguments = merge_request_arguments(repository, &request);
        let arguments: Vec<&str> = arguments.iter().map(String::as_str).collect();
        let output = run_glab(&executable, &arguments, directory).map_err(failed)?;
        parse_created_url(&output, repository).ok_or_else(|| {
            let shown: String = output.trim().chars().take(github::MESSAGE_LIMIT).collect();
            GitError::failed(SERVICE, format!("GitLab did not report the new merge request's address. {shown}"))
        })
    }

    /// The merge request whose source is `branch` on `repository`, with its pipeline, read through
    /// `glab mr list`. A branch with no merge request gives `None`. The branch name is checked
    /// before `glab` runs.
    pub fn merge_request_for_branch(
        &self,
        repository: &HostedRepository,
        program: &GlabProgram,
        branch: &str,
        directory: &Path,
    ) -> Result<Option<PullRequestStatus>> {
        self.check_branch_name(branch, directory)?;
        let executable = program.executable().ok_or_else(|| not_installed(repository))?;
        let repo = repo_argument(repository);
        let output =
            run_glab(&executable, &["mr", "list", "--repo", &repo, "--source-branch", branch, "--all", "--output", "json"], directory)
                .map_err(failed)?;
        decode_merge_request_status(&output, repository)
    }
}

/// Runs `glab` and returns its standard output. On failure the error is the text to show. Prompts
/// are disabled by the closed standard input, and `GITLAB_HOST` is removed so the host named in each
/// call is the one used.
fn run_glab(executable: &Path, arguments: &[&str], directory: &Path) -> std::result::Result<String, String> {
    run_glab_cancellable(executable, arguments, directory, &Cancel::default())
}

/// [`run_glab`] that stops the process when `cancel` is set, as the Launchpad's reads do when closed.
fn run_glab_cancellable(executable: &Path, arguments: &[&str], directory: &Path, cancel: &Cancel) -> std::result::Result<String, String> {
    run_program(executable, arguments, directory, cancel, SERVICE, &[("NO_COLOR", "1"), ("GLAB_CHECK_UPDATE", "false")], &["GITLAB_HOST"])
}

/// The signed-in username on `host`, read through `glab api user`. The answer is checked before it
/// is returned.
pub fn glab_login(host: &str, program: &GlabProgram, directory: &Path) -> Result<String> {
    let executable = program.executable().ok_or_else(|| failed(not_installed_message(host)))?;
    let output = run_glab(&executable, &["api", "--hostname", host, "user"], directory).map_err(failed)?;
    decode_username(&output)
}

/// The open merge requests of `repository` for the Launchpad, from one read-only `glab mr list`
/// call. The list is open merge requests only, which is `glab`'s default, and `--per-page` caps it
/// at the Launchpad's limit.
pub fn gitlab_open_merge_requests(
    repository: &HostedRepository,
    program: &GlabProgram,
    cancel: &Cancel,
    directory: &Path,
) -> Result<Vec<OpenPullRequest>> {
    let executable = program.executable().ok_or_else(|| not_installed(repository))?;
    let repo = repo_argument(repository);
    let per_page = github::OPEN_PULL_REQUEST_LIMIT.to_string();
    let arguments = ["mr", "list", "--repo", &repo, "--output", "json", "--per-page", &per_page];
    let output = run_glab_cancellable(&executable, &arguments, directory, cancel).map_err(failed)?;
    decode_open_merge_requests(&output, repository)
}

fn failed(message: String) -> GitError {
    GitError::failed(SERVICE, message)
}

fn not_installed(repository: &HostedRepository) -> GitError {
    failed(not_installed_message(&repository.host))
}

/// The `--repo` value that `glab` takes: `host/group/project`.
fn repo_argument(repository: &HostedRepository) -> String {
    format!("{}/{}", repository.host, repository.slug())
}

/// The project path as the API takes it: the whole path with its slashes encoded.
fn project_path(repository: &HostedRepository) -> String {
    repository.slug().replace('/', "%2F")
}

/// Reads the default branch from `glab api projects/...` output. Its name must be a string; `null`
/// or a missing field, as for an empty project, is refused.
pub fn decode_default_branch(json: &str) -> Result<String> {
    let value = parse_json(json).map_err(|_| failed("GitLab returned an unexpected project.".to_string()))?;
    value
        .get("default_branch")
        .and_then(Json::as_str)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .ok_or_else(|| failed("GitLab did not report a default branch for this project.".to_string()))
}

/// The `glab mr create` arguments for a merge request on `repository`. The title and description
/// take the `--option=value` form, so a value that starts with `-` is never read as a flag. The
/// `--yes` flag keeps `glab` from asking for confirmation.
pub fn merge_request_arguments(repository: &HostedRepository, request: &NewMergeRequest) -> Vec<String> {
    let mut arguments = vec![
        "mr".to_string(),
        "create".to_string(),
        "--repo".to_string(),
        repo_argument(repository),
        "--source-branch".to_string(),
        request.source.clone(),
        "--target-branch".to_string(),
        request.target.clone(),
        format!("--title={}", request.title),
        format!("--description={}", request.description),
    ];
    if request.draft {
        arguments.push("--draft".to_string());
    }
    arguments.push("--yes".to_string());
    arguments
}

/// The address of the merge request that `glab mr create` printed. The last line that is exactly
/// `<web base>/<group>/<project>/-/merge_requests/<number>` for `repository` is taken, compared
/// without regard to case. Other lines, and other hosts or paths, are refused.
pub fn parse_created_url(output: &str, repository: &HostedRepository) -> Option<String> {
    let prefix = format!("{}/{}/-/merge_requests/", repository.web_base(), repository.slug());
    output.lines().rev().map(str::trim).find_map(|line| {
        let (start, number) = line.split_at_checked(prefix.len())?;
        let digits = !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit());
        (start.eq_ignore_ascii_case(&prefix) && digits).then(|| line.to_string())
    })
}

/// Reads `glab mr list --output json` for `repository`. Each entry is checked strictly, and its link
/// must be the repository's own merge request page. An empty list is `None`. Of several merge
/// requests, the first open or draft one is returned, or else the first listed.
pub fn decode_merge_request_status(json: &str, repository: &HostedRepository) -> Result<Option<PullRequestStatus>> {
    let unexpected = || failed("GitLab returned an unexpected merge request.".to_string());
    let value = parse_json(json).map_err(|_| unexpected())?;
    let Json::Array(entries) = value else { return Err(unexpected()) };
    let statuses = entries.iter().map(|entry| decode_merge_request_entry(entry, repository)).collect::<Result<Vec<_>>>()?;
    let chosen = statuses.iter().position(|status| matches!(status.state, PrState::Open | PrState::Draft)).unwrap_or(0);
    Ok(statuses.into_iter().nth(chosen))
}

/// The username in `glab api user` output. It must be a plain GitLab username, or the answer is
/// refused.
pub fn decode_username(json: &str) -> Result<String> {
    let value = parse_json(json).map_err(|_| failed("GitLab did not report your username.".to_string()))?;
    value
        .get("username")
        .and_then(Json::as_str)
        .filter(|name| valid_username(name))
        .map(str::to_string)
        .ok_or_else(|| failed("GitLab did not report your username.".to_string()))
}

/// Whether `name` can be a GitLab username: 1 to 255 letters, digits, `_`, `-`, or `.`.
pub fn valid_username(name: &str) -> bool {
    !name.is_empty() && name.len() <= 255 && name.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
}

/// Decodes `glab mr list --output json` for the Launchpad, for `repository`. Every entry is checked:
/// its link must be the repository's own merge request page, and its state, date, branch, draft
/// flag, reviewers, and pipeline must have the expected shapes. A missing author is kept as none,
/// and a missing reviewer list as none. Approvals are not in the list output, so no review decision
/// is reported.
pub fn decode_open_merge_requests(json: &str, repository: &HostedRepository) -> Result<Vec<OpenPullRequest>> {
    let unexpected = || failed("GitLab returned unexpected merge requests.".to_string());
    let value = parse_json(json).map_err(|_| unexpected())?;
    let Json::Array(entries) = value else { return Err(unexpected()) };
    entries.iter().map(|entry| decode_open_merge_request(entry, repository)).collect()
}

/// One entry of [`decode_open_merge_requests`].
fn decode_open_merge_request(entry: &Json, repository: &HostedRepository) -> Result<OpenPullRequest> {
    let unexpected = || failed("GitLab returned unexpected merge requests.".to_string());
    let Json::Object(_) = entry else { return Err(unexpected()) };
    let number = entry.get("iid").and_then(Json::as_number).filter(|n| n.fract() == 0.0 && *n >= 1.0 && *n < 9.0e15);
    let number = number.ok_or_else(unexpected)? as u64;
    let title = entry.get("title").and_then(Json::as_str).ok_or_else(unexpected)?;
    let url = entry.get("web_url").and_then(Json::as_str).ok_or_else(unexpected)?;
    let expected = format!("{}/{}/-/merge_requests/{number}", repository.web_base(), repository.slug()).to_ascii_lowercase();
    if url.to_ascii_lowercase() != expected {
        return Err(unexpected());
    }
    let head_branch = entry.get("source_branch").and_then(Json::as_str).ok_or_else(unexpected)?;
    let updated_at = entry.get("updated_at").and_then(Json::as_str).and_then(timestamp_seconds).ok_or_else(unexpected)?;
    let is_draft = ["draft", "work_in_progress"]
        .iter()
        .find_map(|key| entry.get(key))
        .map(|value| value.as_bool().ok_or_else(unexpected))
        .transpose()?
        .unwrap_or(false);
    match entry.get("state").and_then(Json::as_str) {
        Some("opened" | "locked") => {}
        _ => return Err(unexpected()),
    }
    let author = match entry.get("author") {
        None | Some(Json::Null) => None,
        Some(author) => {
            let name = author.get("username").and_then(Json::as_str).filter(|name| valid_username(name)).ok_or_else(unexpected)?;
            Some(name.to_string())
        }
    };
    let review_requests = match entry.get("reviewers") {
        None | Some(Json::Null) => Vec::new(),
        Some(Json::Array(reviewers)) => reviewers
            .iter()
            .map(|reviewer| {
                reviewer
                    .get("username")
                    .and_then(Json::as_str)
                    .filter(|name| valid_username(name))
                    .map(|name| ReviewRequest::User(name.to_string()))
            })
            .collect::<Option<Vec<_>>>()
            .ok_or_else(unexpected)?,
        Some(_) => return Err(unexpected()),
    };
    let checks = pipeline_checks(entry).ok_or_else(unexpected)?;
    Ok(OpenPullRequest {
        number,
        title: clean_text(title, TITLE_LIMIT),
        url: url.to_string(),
        author,
        head_branch: clean_text(head_branch, TITLE_LIMIT),
        is_draft,
        review_decision: None,
        review_requests,
        updated_at,
        checks,
    })
}

/// One merge request in `glab mr list` output.
fn decode_merge_request_entry(entry: &Json, repository: &HostedRepository) -> Result<PullRequestStatus> {
    let unexpected = || failed("GitLab returned an unexpected merge request.".to_string());
    let Json::Object(_) = entry else { return Err(unexpected()) };
    let number = entry.get("iid").and_then(Json::as_number).filter(|n| n.fract() == 0.0 && *n >= 1.0 && *n < 9.0e15);
    let number = number.ok_or_else(unexpected)? as u64;
    let title = entry.get("title").and_then(Json::as_str).ok_or_else(unexpected)?;
    let url = entry.get("web_url").and_then(Json::as_str).ok_or_else(unexpected)?;
    let expected = format!("{}/{}/-/merge_requests/{number}", repository.web_base(), repository.slug()).to_ascii_lowercase();
    if url.to_ascii_lowercase() != expected {
        return Err(unexpected());
    }
    // Older glab output names the flag `work_in_progress`; either one, when present, must be a boolean.
    let draft = ["draft", "work_in_progress"]
        .iter()
        .find_map(|key| entry.get(key))
        .map(|value| value.as_bool().ok_or_else(unexpected))
        .transpose()?
        .unwrap_or(false);
    let state = match entry.get("state").and_then(Json::as_str) {
        Some("opened" | "locked") if draft => PrState::Draft,
        Some("opened" | "locked") => PrState::Open,
        Some("merged") => PrState::Merged,
        Some("closed") => PrState::Closed,
        _ => return Err(unexpected()),
    };
    let checks = pipeline_checks(entry).ok_or_else(unexpected)?;
    Ok(PullRequestStatus { kind: Host::GitLab, number, title: clean_text(title, TITLE_LIMIT), state, url: url.to_string(), checks })
}

/// The pipeline of a merge request's head commit as one check: passed, failed, or pending. A
/// merge request with no pipeline has no checks. `None` means the pipeline has no status word.
fn pipeline_checks(entry: &Json) -> Option<Checks> {
    let pipeline = ["head_pipeline", "pipeline"].iter().filter_map(|key| entry.get(key)).find(|value| !matches!(value, Json::Null));
    let mut checks = Checks::default();
    let Some(pipeline) = pipeline else { return Some(checks) };
    match pipeline.get("status").and_then(Json::as_str)? {
        "success" | "skipped" => checks.passed = 1,
        "failed" | "canceled" => checks.failed = 1,
        _ => checks.pending = 1,
    }
    Some(checks)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hosting::ListedHosts;

    fn repository() -> HostedRepository {
        HostedRepository::parse("https://gitlab.com/group/sub/proj.git", ListedHosts::default()).unwrap()
    }

    fn request(title: &str) -> NewMergeRequest {
        NewMergeRequest {
            source: "feature/x".into(),
            target: "main".into(),
            title: title.into(),
            description: "- one\n- two".into(),
            draft: false,
        }
    }

    fn has(arguments: &[String], value: &str) -> bool {
        arguments.iter().any(|argument| argument == value)
    }

    fn message_of<T: std::fmt::Debug>(result: Result<T>) -> String {
        match result {
            Err(GitError::CommandFailed { message, .. }) => message,
            other => panic!("expected a failure, got {other:?}"),
        }
    }

    #[test]
    fn merge_request_arguments_put_every_value_in_its_own_argument() {
        let mut draft = request("--looks-like-a-flag");
        draft.draft = true;
        let arguments = merge_request_arguments(&repository(), &draft);
        let head: Vec<&str> = arguments[..4].iter().map(String::as_str).collect();
        assert_eq!(head, ["mr", "create", "--repo", "gitlab.com/group/sub/proj"]);
        assert!(arguments.windows(2).any(|pair| pair[0] == "--source-branch" && pair[1] == "feature/x"));
        assert!(arguments.windows(2).any(|pair| pair[0] == "--target-branch" && pair[1] == "main"));
        assert!(has(&arguments, "--title=--looks-like-a-flag"), "a title that starts with - stays a value");
        assert!(has(&arguments, "--description=- one\n- two"), "a multi-line description stays one argument");
        assert!(has(&arguments, "--draft"));
        assert_eq!(arguments.last().map(String::as_str), Some("--yes"));
        let plain = merge_request_arguments(&repository(), &request("Plain"));
        assert!(!has(&plain, "--draft"), "a draft is only asked for");
    }

    #[test]
    fn created_url_is_accepted_only_for_the_repositorys_own_merge_request_page() {
        let repository = repository();
        let output = "Creating merge request for feature/x\n!12 Plain\nhttps://gitlab.com/group/sub/proj/-/merge_requests/12\n";
        assert_eq!(parse_created_url(output, &repository).as_deref(), Some("https://gitlab.com/group/sub/proj/-/merge_requests/12"));
        let upper = "https://GitLab.com/Group/Sub/proj/-/merge_requests/12";
        assert_eq!(parse_created_url(upper, &repository).as_deref(), Some(upper), "compared without regard to case");
        for refused in [
            "https://gitlab.example.com/group/sub/proj/-/merge_requests/12",
            "http://gitlab.com/group/sub/proj/-/merge_requests/12",
            "https://gitlab.com/group/other/-/merge_requests/12",
            "https://gitlab.com/group/sub/proj/-/merge_requests/12x",
            "https://gitlab.com/group/sub/proj/-/merge_requests/",
            "https://gitlab.com/group/sub/proj/-/merge_requests/12/diffs",
            "https://gitlab.com/group/sub/proj/issues/12",
        ] {
            assert_eq!(parse_created_url(refused, &repository), None, "{refused}");
        }
    }

    #[test]
    fn default_branch_must_be_a_name() {
        assert_eq!(decode_default_branch(r#"{"id": 4, "default_branch": "trunk"}"#).unwrap(), "trunk");
        assert!(decode_default_branch(r#"{"default_branch": null}"#).is_err(), "an empty project has none");
        assert!(decode_default_branch(r#"{"default_branch": ""}"#).is_err());
        assert!(decode_default_branch(r#"{"name": "proj"}"#).is_err());
        assert!(decode_default_branch("not json").is_err());
    }

    fn merge_request(iid: u64, state: &str, extra: &str) -> String {
        format!(
            r#"{{"iid": {iid}, "title": "Title {iid}", "web_url": "https://gitlab.com/group/sub/proj/-/merge_requests/{iid}", "state": "{state}"{extra}}}"#
        )
    }

    #[test]
    fn status_reads_the_open_merge_request_with_its_pipeline() {
        let json = format!("[{}]", merge_request(12, "opened", r#", "draft": false, "head_pipeline": {"status": "failed"}"#));
        let status = decode_merge_request_status(&json, &repository()).unwrap().expect("a merge request");
        assert_eq!(status.kind, Host::GitLab);
        assert_eq!((status.number, status.state), (12, PrState::Open));
        assert_eq!(status.checks, Checks { passed: 0, failed: 1, pending: 0 });
        assert_eq!(status.url, "https://gitlab.com/group/sub/proj/-/merge_requests/12");
    }

    #[test]
    fn status_reads_drafts_merged_and_closed_states_and_pipelines() {
        let repository = repository();
        let draft = format!("[{}]", merge_request(3, "opened", r#", "work_in_progress": true, "head_pipeline": {"status": "running"}"#));
        let status = decode_merge_request_status(&draft, &repository).unwrap().unwrap();
        assert_eq!((status.state, status.checks), (PrState::Draft, Checks { passed: 0, failed: 0, pending: 1 }));
        let merged = format!("[{}]", merge_request(4, "merged", r#", "draft": false, "pipeline": {"status": "success"}"#));
        let status = decode_merge_request_status(&merged, &repository).unwrap().unwrap();
        assert_eq!((status.state, status.checks.passed), (PrState::Merged, 1), "an older pipeline field is read too");
        let closed = format!("[{}]", merge_request(5, "closed", r#", "draft": false, "head_pipeline": null"#));
        let status = decode_merge_request_status(&closed, &repository).unwrap().unwrap();
        assert_eq!((status.state, status.checks), (PrState::Closed, Checks::default()));
    }

    #[test]
    fn status_is_none_for_no_merge_request_and_prefers_an_open_one() {
        let repository = repository();
        assert_eq!(decode_merge_request_status("[]", &repository).unwrap(), None);
        let json =
            format!("[{}, {}]", merge_request(9, "closed", r#", "draft": false"#), merge_request(7, "opened", r#", "draft": false"#));
        let status = decode_merge_request_status(&json, &repository).unwrap().unwrap();
        assert_eq!(status.number, 7, "the open one wins over a newer closed one");
    }

    #[test]
    fn status_refuses_links_to_other_repositories_and_unknown_shapes() {
        let repository = repository();
        let other = r#"[{"iid": 1, "title": "x", "web_url": "https://gitlab.com/other/proj/-/merge_requests/1", "state": "opened", "draft": false}]"#;
        assert!(decode_merge_request_status(other, &repository).is_err(), "a link to another project");
        let unknown = format!("[{}]", merge_request(2, "weird", r#", "draft": false"#));
        assert!(decode_merge_request_status(&unknown, &repository).is_err(), "an unknown state");
        let bad_draft = format!("[{}]", merge_request(2, "opened", r#", "draft": "yes""#));
        assert!(decode_merge_request_status(&bad_draft, &repository).is_err(), "a draft flag that is not a boolean");
        let bad_pipeline = format!("[{}]", merge_request(2, "opened", r#", "draft": false, "head_pipeline": {"id": 1}"#));
        assert!(decode_merge_request_status(&bad_pipeline, &repository).is_err(), "a pipeline with no status");
        assert!(decode_merge_request_status(r#"{"iid": 1}"#, &repository).is_err(), "not a list");
    }

    #[test]
    fn a_title_with_control_characters_is_cleaned_for_display() {
        let json = r#"[{"iid": 6, "title": "Add\u0007 the\nbadge", "web_url": "https://gitlab.com/group/sub/proj/-/merge_requests/6", "state": "opened", "draft": false}]"#;
        let status = decode_merge_request_status(json, &repository()).unwrap().unwrap();
        assert_eq!(status.title, "Add the badge");
    }

    #[test]
    fn a_missing_glab_is_reported_with_the_sign_in_command() {
        let repository = repository();
        let missing = GlabProgram::Given(PathBuf::from("/nonexistent/nicegit-test/glab"));
        let client = GitClient::new();
        assert_eq!(client.glab_status(&repository, &missing, Path::new("/")), GlabStatus::NotInstalled);
        let message = message_of(client.glab_default_branch(&repository, &missing, Path::new("/")));
        assert!(message.contains("not installed"), "{message}");
        assert!(message.contains("glab auth login --hostname gitlab.com"), "{message}");
        assert_eq!(sign_in_command("git.example.com"), "glab auth login --hostname git.example.com");
    }

    #[test]
    fn creation_refuses_a_bad_title_and_the_same_branch_before_glab_runs() {
        let repository = repository();
        let client = GitClient::new();
        let missing = GlabProgram::Given(PathBuf::from("/nonexistent/nicegit-test/glab"));
        let directory = std::env::temp_dir();
        let message = message_of(client.create_merge_request(&repository, &missing, &request("  "), &directory));
        assert_eq!(message, "Enter a title for the merge request.");
        let message = message_of(client.create_merge_request(&repository, &missing, &request("two\nlines"), &directory));
        assert_eq!(message, "The title must be one line.");
        let same = NewMergeRequest { target: "feature/x".into(), ..request("Title") };
        let message = message_of(client.create_merge_request(&repository, &missing, &same, &directory));
        assert_eq!(message, "The target branch must be different from the branch.");
    }

    /// A stand-in `glab` that answers the calls NiceGit makes, and appends each call's arguments to
    /// `calls.txt` beside itself, one argument per line. Only the test uses it; it is never installed.
    #[cfg(unix)]
    const FAKE_GLAB: &str = r#"#!/bin/sh
{ printf '%s\n' "$@"; echo '--end-of-call--'; } >> "$(dirname "$0")/calls.txt"
case "$1" in
  api) echo '{"default_branch": "trunk"}' ;;
  auth) echo "Logged in to gitlab.com" ;;
  mr)
    if [ "$2" = "list" ]; then
      echo '[{"iid": 12, "title": "Add it", "web_url": "https://gitlab.com/group/sub/proj/-/merge_requests/12", "state": "opened", "draft": false, "head_pipeline": {"status": "success"}}]'
    else
      echo "Creating merge request for feature/x"
      echo "https://gitlab.com/group/sub/proj/-/merge_requests/12"
    fi ;;
esac
"#;

    #[cfg(unix)]
    #[test]
    fn the_glab_calls_read_and_create_through_the_program() {
        use std::os::unix::fs::PermissionsExt;
        let scratch = crate::rebase::Scratch::new("gitlab test").unwrap();
        let script = scratch.write("gitlab test", "glab", FAKE_GLAB.as_bytes()).unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let folder = script.parent().unwrap().to_path_buf();
        let program = GlabProgram::Given(script);
        let repository = repository();
        let client = GitClient::new();
        let directory = std::env::temp_dir();

        assert_eq!(client.glab_status(&repository, &program, &directory), GlabStatus::SignedIn);
        assert_eq!(client.glab_default_branch(&repository, &program, &directory).unwrap(), "trunk");
        let status = client.merge_request_for_branch(&repository, &program, "feature/x", &directory).unwrap().unwrap();
        assert_eq!((status.number, status.checks.passed), (12, 1));
        let url = client.create_merge_request(&repository, &program, &request("Add it"), &directory).unwrap();
        assert_eq!(url, "https://gitlab.com/group/sub/proj/-/merge_requests/12");

        let calls = std::fs::read_to_string(folder.join("calls.txt")).unwrap();
        let lines: Vec<&str> = calls.lines().collect();
        assert!(lines.contains(&"projects/group%2Fsub%2Fproj"), "the project is read by its encoded path: {calls}");
        assert!(lines.contains(&"gitlab.com/group/sub/proj"), "{calls}");
        assert!(lines.contains(&"--title=Add it"), "{calls}");
        let create_end = lines.iter().enumerate().filter(|(_, line)| **line == "--end-of-call--").map(|(index, _)| index).nth(3).unwrap();
        assert_eq!(lines[create_end - 1], "--yes", "the merge request is created without a prompt: {calls}");
    }

    /// One merge request of the Launchpad's list, with every field, and `changes` applied: a value
    /// replaces its field, an empty value leaves the field out.
    fn listed_entry(changes: &[(&str, &str)]) -> String {
        let mut fields: Vec<(&str, String)> = vec![
            ("iid", "7".into()),
            ("title", "\"Title\"".into()),
            ("web_url", "\"https://gitlab.com/group/sub/proj/-/merge_requests/7\"".into()),
            ("state", "\"opened\"".into()),
            ("draft", "false".into()),
            ("source_branch", "\"feature/x\"".into()),
            ("updated_at", "\"2026-01-02T03:04:05Z\"".into()),
            ("author", "{\"username\": \"alice\"}".into()),
            ("reviewers", "[]".into()),
            ("head_pipeline", "null".into()),
        ];
        for (key, value) in changes {
            match fields.iter().position(|(name, _)| name == key) {
                Some(index) => fields[index].1 = value.to_string(),
                None => fields.push((*key, value.to_string())),
            }
        }
        let body: Vec<String> =
            fields.iter().filter(|(_, value)| !value.is_empty()).map(|(name, value)| format!("\"{name}\": {value}")).collect();
        format!("[{{{}}}]", body.join(", "))
    }

    /// A list of three merge requests as `glab mr list --output json` shows them.
    const LISTED: &str = r#"[
        {"iid": 4, "title": "Draft work", "web_url": "https://gitlab.com/group/sub/proj/-/merge_requests/4", "state": "opened",
         "draft": true, "source_branch": "wip", "updated_at": "2026-11-09T10:30:00.123Z", "author": {"username": "alice"},
         "reviewers": [], "head_pipeline": {"status": "running"}},
        {"iid": 7, "title": "Needs you", "web_url": "https://gitlab.com/group/sub/proj/-/merge_requests/7", "state": "opened",
         "draft": false, "source_branch": "feature/x", "updated_at": "2026-11-08T09:00:00Z", "author": {"username": "bob"},
         "reviewers": [{"username": "daniaal"}], "head_pipeline": {"status": "success"}},
        {"iid": 9, "title": "Orphan", "web_url": "https://gitlab.com/group/sub/proj/-/merge_requests/9", "state": "opened",
         "draft": false, "source_branch": "old", "updated_at": "2026-11-01T00:00:00Z", "reviewers": [], "head_pipeline": null}
    ]"#;

    #[test]
    fn open_merge_requests_decode_from_a_captured_list() {
        let repository = repository();
        let list = decode_open_merge_requests(LISTED, &repository).unwrap();
        assert_eq!(list.len(), 3);

        let draft = &list[0];
        assert_eq!(draft.number, 4);
        assert!(draft.is_draft, "a draft flag is read");
        assert_eq!(draft.author.as_deref(), Some("alice"));
        assert_eq!(draft.checks, Checks { passed: 0, failed: 0, pending: 1 }, "a running pipeline is pending");
        assert_eq!(draft.updated_at, timestamp_seconds("2026-11-09T10:30:00Z").unwrap(), "fractional seconds are ignored");

        let asked = &list[1];
        assert_eq!(asked.review_requests, [ReviewRequest::User("daniaal".into())], "a reviewer is a user request");
        assert_eq!(asked.head_branch, "feature/x");
        assert_eq!(asked.url, "https://gitlab.com/group/sub/proj/-/merge_requests/7");
        assert_eq!(asked.review_decision, None, "approvals are not in the list output");
        assert_eq!(asked.checks, Checks { passed: 1, failed: 0, pending: 0 });

        let orphan = &list[2];
        assert_eq!(orphan.author, None, "a missing author is kept as none");
        assert!(orphan.review_requests.is_empty());
        assert_eq!(orphan.checks, Checks::default(), "no pipeline, no checks");
        assert!(decode_open_merge_requests("[]", &repository).unwrap().is_empty());
    }

    #[test]
    fn open_merge_requests_refuse_a_foreign_link_and_unexpected_entries() {
        let repository = repository();
        assert!(decode_open_merge_requests(&listed_entry(&[]), &repository).is_ok());
        let refused = [
            listed_entry(&[("web_url", "\"https://gitlab.com/other/proj/-/merge_requests/7\"")]),
            listed_entry(&[("web_url", "\"https://gitlab.example.com/group/sub/proj/-/merge_requests/7\"")]),
            listed_entry(&[("web_url", "\"https://gitlab.com/group/sub/proj/-/merge_requests/8\"")]),
            listed_entry(&[("state", "\"merged\"")]),
            listed_entry(&[("state", "\"weird\"")]),
            listed_entry(&[("draft", "\"yes\"")]),
            listed_entry(&[("author", "{\"username\": \"bad name\"}")]),
            listed_entry(&[("reviewers", "[{\"name\": \"x\"}]")]),
            listed_entry(&[("reviewers", "\"daniaal\"")]),
            listed_entry(&[("head_pipeline", "{\"id\": 1}")]),
            listed_entry(&[("updated_at", "\"last Tuesday\"")]),
            listed_entry(&[("iid", "")]),
            listed_entry(&[("title", "")]),
            listed_entry(&[("source_branch", "")]),
            "[1]".to_string(),
            "{}".to_string(),
        ];
        for json in refused {
            assert!(decode_open_merge_requests(&json, &repository).is_err(), "{json} must be refused");
        }
        let missing_field = |field: &str| listed_entry(&[(field, "")]);
        for field in ["web_url", "state", "updated_at", "title", "iid", "source_branch"] {
            assert!(decode_open_merge_requests(&missing_field(field), &repository).is_err(), "without {field} the list is refused");
        }
    }

    #[test]
    fn a_reviewer_who_is_you_is_read_as_a_user_request() {
        let json = listed_entry(&[("reviewers", "[{\"username\": \"daniaal\", \"name\": \"Daniaal\"}]"), ("draft", "true")]);
        let list = decode_open_merge_requests(&json, &repository()).unwrap();
        assert_eq!(list[0].review_requests, [ReviewRequest::User("daniaal".into())]);
        assert!(list[0].is_draft);
    }

    #[test]
    fn usernames_are_checked_before_they_are_used() {
        assert_eq!(decode_username(r#"{"id": 3, "username": "daniaal"}"#).unwrap(), "daniaal");
        assert_eq!(decode_username(r#"{"username": "a.b-c_d"}"#).unwrap(), "a.b-c_d");
        assert!(decode_username(r#"{"username": ""}"#).is_err());
        assert!(decode_username(r#"{"username": "two words"}"#).is_err());
        assert!(decode_username(r#"{"name": "Daniaal"}"#).is_err());
        assert!(decode_username("not json").is_err());
        assert!(valid_username(&"a".repeat(255)));
        assert!(!valid_username(&"a".repeat(256)));
    }

    #[test]
    fn a_missing_or_signed_out_glab_gives_a_reason_for_the_host() {
        assert_eq!(GlabStatus::SignedIn.reason("gitlab.com"), None);
        let missing = GlabStatus::NotInstalled.reason("gitlab.com").unwrap();
        assert!(missing.contains("not installed"), "{missing}");
        assert!(missing.contains("glab auth login --hostname gitlab.com"), "{missing}");
        let signed_out = GlabStatus::SignedOut("To sign in,\nuse a token.\n".into()).reason("git.example.com").unwrap();
        assert!(signed_out.contains("glab auth login --hostname git.example.com"), "{signed_out}");
        assert!(!signed_out.contains('\n'), "the explanation is one line");
    }

    /// A stand-in `glab` for the Launchpad's reads. It answers the calls NiceGit makes and appends
    /// each call's arguments to `calls.txt` beside itself, one per line. Only the test uses it.
    #[cfg(unix)]
    const FAKE_GLAB_READS: &str = r#"#!/bin/sh
{ printf '%s\n' "$@"; echo '--end-of-call--'; } >> "$(dirname "$0")/calls.txt"
case "$1" in
  auth) echo "Logged in to gitlab.com" ;;
  api) echo '{"id": 3, "username": "daniaal"}' ;;
  mr) cat "$(dirname "$0")/merge_requests.json" ;;
esac
"#;

    #[cfg(unix)]
    #[test]
    fn the_launchpad_gitlab_reads_run_only_read_only_glab_commands() {
        use std::os::unix::fs::PermissionsExt;
        let scratch = crate::rebase::Scratch::new("launchpad gitlab test").unwrap();
        let script = scratch.write("launchpad gitlab test", "glab", FAKE_GLAB_READS.as_bytes()).unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let folder = script.parent().unwrap().to_path_buf();
        std::fs::write(folder.join("merge_requests.json"), LISTED).unwrap();
        let program = GlabProgram::Given(script);
        let repository = repository();
        let client = GitClient::new();
        let directory = std::env::temp_dir();

        assert_eq!(client.glab_status(&repository, &program, &directory), GlabStatus::SignedIn);
        assert_eq!(glab_login("gitlab.com", &program, &directory).unwrap(), "daniaal");
        let list = gitlab_open_merge_requests(&repository, &program, &Cancel::default(), &directory).unwrap();
        assert_eq!(list.len(), 3);

        let calls = std::fs::read_to_string(folder.join("calls.txt")).unwrap();
        let calls: Vec<Vec<&str>> =
            calls.split("--end-of-call--\n").filter(|call| !call.is_empty()).map(|call| call.lines().collect()).collect();
        assert_eq!(calls[0], ["auth", "status", "--hostname", "gitlab.com"]);
        assert_eq!(calls[1], ["api", "--hostname", "gitlab.com", "user"]);
        assert_eq!(calls[2], ["mr", "list", "--repo", "gitlab.com/group/sub/proj", "--output", "json", "--per-page", "50"]);
        for call in &calls {
            let verb = call.first().copied().unwrap_or_default();
            assert!(matches!(verb, "auth" | "api" | "mr"), "{call:?} is not a read");
            assert!(
                !call.iter().any(|word| matches!(*word, "create" | "merge" | "close" | "update" | "POST" | "PUT" | "PATCH" | "DELETE")),
                "{call:?}"
            );
            if verb == "api" {
                assert!(!call.iter().any(|word| word.starts_with("-X") || *word == "--method"), "{call:?} is a GET");
            }
        }
    }
}
