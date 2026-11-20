//! Inline review comments on a pull request, read and written through the GitHub CLI (`gh`): the
//! review threads of a pull request, its diff split by file, and the replies, new line comments,
//! and resolve changes a person posts from the detail view. Text a person types is passed to `gh`
//! as a field value, never inside the GraphQL query. Nothing here posts without the caller having
//! had the person confirm it first.

use std::path::Path;

use crate::client::GitClient;
use crate::diff::{parse_diff, DiffLine, DiffLineKind};
use crate::github::{
    author_login, gh_message, object_id, optional_text, plain_text, run_gh_with, run_gh_with_input, valid_name_part, Cancel, GhProgram,
    GitHubRepository, NOT_INSTALLED,
};
use crate::json::{parse_json, Json};
use crate::models::{GitError, Result};

/// The most review threads read for one pull request. GitHub pages them, so the first page is what
/// the view shows.
const THREAD_LIMIT: u32 = 100;

/// The most comments read in one thread.
const THREAD_COMMENT_LIMIT: u32 = 50;

/// The longest comment that is posted, in characters. A longer one is refused rather than cut short.
pub const COMMENT_POST_LIMIT: usize = 20_000;

/// The largest diff read, in bytes. A larger one is refused, so the view does not hold it all.
pub const DIFF_LIMIT: usize = 20 * 1024 * 1024;

/// The longest GraphQL node ID accepted for a thread.
const NODE_ID_LIMIT: usize = 200;

/// The GraphQL query that reads the review threads of a pull request. Owner, name, and number are
/// passed as variables, so nothing a person typed is ever part of this text.
fn review_threads_query() -> String {
    format!(
        "query($owner: String!, $name: String!, $number: Int!) {{ repository(owner: $owner, name: $name) {{ \
         pullRequest(number: $number) {{ reviewThreads(first: {THREAD_LIMIT}) {{ nodes {{ \
         id isResolved isOutdated path line originalLine startLine diffSide \
         comments(first: {THREAD_COMMENT_LIMIT}) {{ nodes {{ id databaseId author {{ login }} body createdAt }} }} \
         }} }} }} }} }}"
    )
}

/// The mutation that adds a reply to a review thread. The thread ID and the body are variables.
const REPLY_MUTATION: &str = "mutation($thread: ID!, $body: String!) { addPullRequestReviewThreadReply(input: \
     {pullRequestReviewThreadId: $thread, body: $body}) { comment { id } } }";

/// The mutation that resolves a review thread.
const RESOLVE_MUTATION: &str = "mutation($thread: ID!) { resolveReviewThread(input: {threadId: $thread}) { thread { id isResolved } } }";

/// The mutation that reopens a resolved review thread.
const UNRESOLVE_MUTATION: &str =
    "mutation($thread: ID!) { unresolveReviewThread(input: {threadId: $thread}) { thread { id isResolved } } }";

/// The side of a diff a review comment is on: the old file (`Left`) or the new one (`Right`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReviewSide {
    Left,
    Right,
}

impl ReviewSide {
    /// The word the REST and GraphQL APIs use for the side.
    fn word(self) -> &'static str {
        match self {
            ReviewSide::Left => "LEFT",
            ReviewSide::Right => "RIGHT",
        }
    }
}

/// One review thread on a pull request: its anchor in the diff, its state, and its comments in
/// the order they were written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReviewThread {
    /// The GraphQL node ID, which replies and resolve changes name.
    pub id: String,
    pub resolved: bool,
    /// Whether the code under the thread has changed since it was written.
    pub outdated: bool,
    /// The file the thread is on, as the repository names it.
    pub path: String,
    /// The line the thread is on in the current head commit. `None` for an outdated thread.
    pub line: Option<u32>,
    pub side: ReviewSide,
    pub comments: Vec<ThreadComment>,
}

/// One comment in a review thread. `body` is plain text, limited like other GitHub text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThreadComment {
    pub id: String,
    pub database_id: Option<u64>,
    /// The author's login, or `None` when GitHub reports no author.
    pub author: Option<String>,
    pub body: String,
    pub created_at: String,
}

/// A line comment to add to a pull request: the commit it was written against, and the line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewLineComment {
    /// The head commit the comment is written against, as a full object ID.
    pub head_oid: String,
    pub path: String,
    /// The line in the file, numbered from 1, on `side`.
    pub line: u32,
    pub side: ReviewSide,
    pub body: String,
}

/// One file of a pull request's diff: its path, and its lines with old and new numbers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileDiff {
    pub path: String,
    pub lines: Vec<DiffLine>,
}

/// The `gh api graphql` arguments that read the review threads of pull request `number`.
pub fn review_threads_arguments(repository: &GitHubRepository, number: u64) -> Vec<String> {
    vec![
        "api".to_string(),
        "graphql".to_string(),
        "-f".to_string(),
        format!("query={}", review_threads_query()),
        "-f".to_string(),
        format!("owner={}", repository.owner),
        "-f".to_string(),
        format!("name={}", repository.name),
        "-F".to_string(),
        format!("number={number}"),
    ]
}

/// The `gh api graphql` arguments that reply to thread `thread_id` with `body`.
pub fn reply_arguments(thread_id: &str, body: &str) -> Vec<String> {
    vec![
        "api".to_string(),
        "graphql".to_string(),
        "-f".to_string(),
        format!("query={REPLY_MUTATION}"),
        "-f".to_string(),
        format!("thread={thread_id}"),
        "-f".to_string(),
        format!("body={body}"),
    ]
}

/// The `gh api graphql` arguments that resolve or reopen thread `thread_id`.
pub fn resolve_arguments(thread_id: &str, resolved: bool) -> Vec<String> {
    let mutation = if resolved { RESOLVE_MUTATION } else { UNRESOLVE_MUTATION };
    vec![
        "api".to_string(),
        "graphql".to_string(),
        "-f".to_string(),
        format!("query={mutation}"),
        "-f".to_string(),
        format!("thread={thread_id}"),
    ]
}

/// The `gh api` arguments that add a line comment to pull request `number`. The body, path, and
/// commit are field values; the line is sent as a number.
pub fn line_comment_arguments(repository: &GitHubRepository, number: u64, comment: &NewLineComment) -> Vec<String> {
    vec![
        "api".to_string(),
        "-X".to_string(),
        "POST".to_string(),
        format!("repos/{}/{}/pulls/{number}/comments", repository.owner, repository.name),
        "-f".to_string(),
        format!("body={}", comment.body),
        "-f".to_string(),
        format!("commit_id={}", comment.head_oid),
        "-f".to_string(),
        format!("path={}", comment.path),
        "-F".to_string(),
        format!("line={}", comment.line),
        "-f".to_string(),
        format!("side={}", comment.side.word()),
    ]
}

/// The text of a comment as it is posted: line breaks kept, other control characters removed, and
/// the ends trimmed. An empty comment, or one longer than [`COMMENT_POST_LIMIT`], is refused.
pub fn comment_body(text: &str) -> std::result::Result<String, String> {
    let lines = text.replace("\r\n", "\n").replace('\r', "\n");
    let kept: String = lines.chars().filter(|c| *c == '\n' || *c == '\t' || !c.is_control()).collect();
    let trimmed = kept.trim();
    if trimmed.is_empty() {
        return Err("Write a comment first.".to_string());
    }
    if trimmed.chars().count() > COMMENT_POST_LIMIT {
        return Err(format!("A comment can be at most {COMMENT_POST_LIMIT} characters."));
    }
    Ok(trimmed.to_string())
}

/// Whether `id` looks like a GraphQL node ID: letters, digits, and the base64 signs, and not too long.
fn valid_node_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= NODE_ID_LIMIT
        && id.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'=' | b'_' | b'-' | b'+' | b'/'))
}

/// The diff line a comment on `side` at `line` is shown under: the first line of the diff with that
/// number on that side, or `None` when the diff does not show the line.
pub fn anchor_index(lines: &[DiffLine], side: ReviewSide, line: u32) -> Option<usize> {
    let line = line as usize;
    lines.iter().position(|diff| match side {
        ReviewSide::Right => diff.new_number == Some(line),
        ReviewSide::Left => diff.old_number == Some(line),
    })
}

/// The side and line number a new comment on `diff` is written to. A removed line is on the old
/// side; an added or unchanged line is on the new side. Other lines cannot take a comment.
pub fn comment_target(diff: &DiffLine) -> Option<(ReviewSide, u32)> {
    match diff.kind {
        DiffLineKind::Deletion => diff.old_number.map(|number| (ReviewSide::Left, number as u32)),
        DiffLineKind::Addition | DiffLineKind::Context => diff.new_number.map(|number| (ReviewSide::Right, number as u32)),
        DiffLineKind::Hunk | DiffLineKind::Metadata => None,
    }
}

/// Decodes the answer of the review threads query. Every thread must have the shape the query asks
/// for, and an answer without a repository or pull request is refused.
pub fn decode_review_threads(json: &str) -> Result<Vec<ReviewThread>> {
    let unexpected = || GitError::failed("GitHub", "GitHub returned unexpected review threads.");
    let value = parse_json(json).map_err(|_| unexpected())?;
    let nodes = value
        .get("data")
        .and_then(|data| data.get("repository"))
        .and_then(|repository| repository.get("pullRequest"))
        .and_then(|pull_request| pull_request.get("reviewThreads"))
        .and_then(|threads| threads.get("nodes"))
        .ok_or_else(unexpected)?;
    let Json::Array(nodes) = nodes else { return Err(unexpected()) };
    nodes.iter().map(thread).collect::<Option<Vec<_>>>().ok_or_else(unexpected)
}

/// One thread node. A thread on a side other than LEFT or RIGHT, or with a line that is not a
/// positive whole number, is refused.
fn thread(entry: &Json) -> Option<ReviewThread> {
    let Json::Object(_) = entry else { return None };
    let id = entry.get("id")?.as_str()?;
    if !valid_node_id(id) {
        return None;
    }
    let path = entry.get("path")?.as_str()?;
    if path.is_empty() {
        return None;
    }
    let side = match entry.get("diffSide")?.as_str()? {
        "LEFT" => ReviewSide::Left,
        "RIGHT" => ReviewSide::Right,
        _ => return None,
    };
    let comments = match entry.get("comments")?.get("nodes")? {
        Json::Array(nodes) => nodes.iter().map(comment).collect::<Option<Vec<_>>>()?,
        _ => return None,
    };
    Some(ReviewThread {
        id: id.to_string(),
        resolved: entry.get("isResolved")?.as_bool()?,
        outdated: entry.get("isOutdated")?.as_bool()?,
        path: path.to_string(),
        line: optional_line(entry, "line")?,
        side,
        comments,
    })
}

/// A line number that is absent or `null` (`Some(None)`), or a positive whole number. Anything else
/// is `None`, which the decoders refuse.
fn optional_line(entry: &Json, key: &str) -> Option<Option<u32>> {
    match entry.get(key) {
        None | Some(Json::Null) => Some(None),
        Some(value) => {
            let number = value.as_number()?;
            (number.fract() == 0.0 && (1.0..=f64::from(u32::MAX)).contains(&number)).then_some(Some(number as u32))
        }
    }
}

/// One comment of a thread. Its ID and time are required; its author and body may be absent.
fn comment(entry: &Json) -> Option<ThreadComment> {
    let Json::Object(_) = entry else { return None };
    let id = entry.get("id")?.as_str()?;
    if !valid_node_id(id) {
        return None;
    }
    let created_at = entry.get("createdAt")?.as_str()?;
    crate::github::timestamp_seconds(created_at)?;
    let database_id = match entry.get("databaseId") {
        None | Some(Json::Null) => None,
        Some(value) => {
            let number = value.as_number()?;
            if number.fract() != 0.0 || !(0.0..9.0e15).contains(&number) {
                return None;
            }
            Some(number as u64)
        }
    };
    Some(ThreadComment {
        id: id.to_string(),
        database_id,
        author: author_login(entry)?,
        body: plain_text(optional_text(entry, "body")?),
        created_at: created_at.to_string(),
    })
}

/// Splits a unified diff by file and numbers each file's lines. Each file's path is read from its
/// header, before its first hunk, so file content that looks like a header is not taken for one.
pub fn file_diffs(patch: &str) -> Vec<FileDiff> {
    let mut starts: Vec<usize> = Vec::new();
    if patch.starts_with("diff --git ") {
        starts.push(0);
    }
    starts.extend(patch.match_indices("\ndiff --git ").map(|(index, _)| index + 1));
    starts
        .iter()
        .enumerate()
        .map(|(position, &start)| {
            let end = starts.get(position + 1).copied().unwrap_or(patch.len());
            let section = patch[start..end].strip_suffix('\n').unwrap_or(&patch[start..end]);
            FileDiff { path: header_path(section).unwrap_or_default(), lines: parse_diff(section) }
        })
        .collect()
}

/// The path a file section is about, from the header lines before its first hunk. The new path is
/// preferred, then a rename or copy target, then the old path, then the `diff --git` line.
fn header_path(section: &str) -> Option<String> {
    let header: Vec<&str> = section.split('\n').take_while(|line| !line.starts_with("@@")).collect();
    let find = |prefix: &str| header.iter().find_map(|line| line.strip_prefix(prefix)).map(|rest| rest.trim_end_matches('\t').to_string());
    let side = |value: String, side_prefix: &str| {
        let unquoted = unquote(&value);
        if value == "/dev/null" {
            return None;
        }
        Some(unquoted.strip_prefix(side_prefix).unwrap_or(&unquoted).to_string())
    };
    if let Some(path) = find("+++ ").and_then(|value| side(value, "b/")) {
        return Some(path);
    }
    if let Some(path) = find("rename to ").or_else(|| find("copy to ")) {
        return Some(unquote(&path));
    }
    if let Some(path) = find("--- ").and_then(|value| side(value, "a/")) {
        return Some(path);
    }
    let git_line = section.lines().next()?.strip_prefix("diff --git ")?;
    git_line.rsplit_once(" b/").map(|(_, path)| path.to_string())
}

/// Removes Git's quotes and C-style escapes from a path. Git quotes a path with unusual characters;
/// an escape is a letter, or a three-digit octal byte. A value that is not quoted is returned as it is.
fn unquote(text: &str) -> String {
    let Some(inner) = text.strip_prefix('"').and_then(|rest| rest.strip_suffix('"')) else {
        return text.to_string();
    };
    let bytes = inner.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'\\' || index + 1 >= bytes.len() {
            out.push(bytes[index]);
            index += 1;
            continue;
        }
        let octal = bytes.get(index + 1..index + 4).filter(|digits| digits.iter().all(|digit| (b'0'..=b'7').contains(digit)));
        if let Some(digits) = octal {
            out.push(digits.iter().fold(0u32, |value, digit| value * 8 + u32::from(digit - b'0')) as u8);
            index += 4;
        } else {
            out.push(match bytes[index + 1] {
                b'a' => 7,
                b'b' => 8,
                b't' => b'\t',
                b'n' => b'\n',
                b'v' => 11,
                b'f' => 12,
                b'r' => b'\r',
                other => other,
            });
            index += 2;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

impl GitClient {
    /// The review threads of pull request `number`, read through `gh api graphql`.
    pub fn review_threads(
        &self,
        repository: &GitHubRepository,
        number: u64,
        program: &GhProgram,
        cancel: &Cancel,
        directory: &Path,
    ) -> Result<Vec<ReviewThread>> {
        check_pull_request(repository, number)?;
        let executable = program.executable().ok_or_else(|| GitError::failed("GitHub", NOT_INSTALLED))?;
        let arguments = review_threads_arguments(repository, number);
        let arguments: Vec<&str> = arguments.iter().map(String::as_str).collect();
        let output = run_gh_with(&executable, &arguments, directory, cancel, Some(repository.host.as_str()))
            .map_err(|message| GitError::failed("GitHub", message))?;
        decode_review_threads(&output)
    }

    /// The diff of pull request `number` as a plain unified diff, read through `gh pr diff`. A diff
    /// over [`DIFF_LIMIT`] bytes is refused.
    pub fn pull_request_diff(
        &self,
        repository: &GitHubRepository,
        number: u64,
        program: &GhProgram,
        cancel: &Cancel,
        directory: &Path,
    ) -> Result<String> {
        check_pull_request(repository, number)?;
        let executable = program.executable().ok_or_else(|| GitError::failed("GitHub", NOT_INSTALLED))?;
        let number_text = number.to_string();
        let repo = repository.gh_repo();
        let output = run_gh_with(
            &executable,
            &["pr", "diff", &number_text, "--repo", &repo, "--patch=false"],
            directory,
            cancel,
            Some(repository.host.as_str()),
        )
        .map_err(|message| GitError::failed("GitHub", message))?;
        if output.len() > DIFF_LIMIT {
            return Err(GitError::failed("GitHub", "This pull request's diff is too large to show."));
        }
        Ok(output)
    }

    /// Adds a reply to review thread `thread_id`. The body is checked before `gh` runs.
    pub fn reply_to_thread(
        &self,
        repository: &GitHubRepository,
        thread_id: &str,
        body: &str,
        program: &GhProgram,
        directory: &Path,
    ) -> Result<()> {
        checked_repository(repository)?;
        if !valid_node_id(thread_id) {
            return Err(GitError::failed("GitHub", "The review thread is not valid."));
        }
        let body = comment_body(body).map_err(|message| GitError::failed("GitHub", message))?;
        self.run_write(reply_arguments(thread_id, &body), repository, program, directory)
    }

    /// Adds a line comment to pull request `number`, written against the head commit it names. The
    /// comment is checked before `gh` runs.
    pub fn add_line_comment(
        &self,
        repository: &GitHubRepository,
        number: u64,
        comment: &NewLineComment,
        program: &GhProgram,
        directory: &Path,
    ) -> Result<()> {
        check_pull_request(repository, number)?;
        if object_id(&comment.head_oid).is_none() {
            return Err(GitError::failed("GitHub", "The head commit of the pull request is not valid."));
        }
        if comment.path.is_empty() || comment.line == 0 {
            return Err(GitError::failed("GitHub", "Choose a line of the diff to comment on."));
        }
        let body = comment_body(&comment.body).map_err(|message| GitError::failed("GitHub", message))?;
        let comment = NewLineComment { body, ..comment.clone() };
        self.run_write(line_comment_arguments(repository, number, &comment), repository, program, directory)
    }

    /// Resolves or reopens review thread `thread_id`.
    pub fn set_thread_resolved(
        &self,
        repository: &GitHubRepository,
        thread_id: &str,
        resolved: bool,
        program: &GhProgram,
        directory: &Path,
    ) -> Result<()> {
        checked_repository(repository)?;
        if !valid_node_id(thread_id) {
            return Err(GitError::failed("GitHub", "The review thread is not valid."));
        }
        self.run_write(resolve_arguments(thread_id, resolved), repository, program, directory)
    }

    /// Runs one `gh` call that changes the pull request. Its failure is shown as `gh` printed it.
    fn run_write(&self, arguments: Vec<String>, repository: &GitHubRepository, program: &GhProgram, directory: &Path) -> Result<()> {
        let executable = program.executable().ok_or_else(|| GitError::failed("GitHub", NOT_INSTALLED))?;
        let arguments: Vec<&str> = arguments.iter().map(String::as_str).collect();
        run_gh_with_input(&executable, &arguments, directory, &Cancel::default(), Some(repository.host.as_str()), None)
            .map(|_| ())
            .map_err(|message| GitError::failed("GitHub", gh_message(&message)))
    }
}

/// Refuses a repository whose owner or name would not be a valid path part.
fn checked_repository(repository: &GitHubRepository) -> Result<()> {
    if valid_name_part(&repository.owner) && valid_name_part(&repository.name) {
        Ok(())
    } else {
        Err(GitError::failed("GitHub", "The repository is not valid."))
    }
}

/// Refuses a repository or a pull request number that cannot be read.
fn check_pull_request(repository: &GitHubRepository, number: u64) -> Result<()> {
    checked_repository(repository)?;
    if number == 0 {
        return Err(GitError::failed("GitHub", "The pull request number is not valid."));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repository() -> GitHubRepository {
        GitHubRepository { host: "github.com".into(), owner: "acme".into(), name: "app".into() }
    }

    /// Threads as `gh api graphql` prints them: an unresolved thread with a comment whose author was
    /// deleted, a resolved thread with two comments, and an outdated thread with no line.
    const THREADS_JSON: &str = r#"{"data": {"repository": {"pullRequest": {"reviewThreads": {"nodes": [
        {"id": "PRRT_kwDOAbc123", "isResolved": false, "isOutdated": false, "path": "src/status_bar.rs",
         "line": 42, "originalLine": 42, "startLine": null, "diffSide": "RIGHT",
         "comments": {"nodes": [
            {"id": "PRRC_kwDOAbc001", "databaseId": 1001, "author": null,
             "body": "Please rename\u0007 the helper.\r\n\r\nThanks", "createdAt": "2026-01-02T04:00:00Z"}
         ]}},
        {"id": "PRRT_kwDOAbc456", "isResolved": true, "isOutdated": false, "path": "src/status_bar.rs",
         "line": 7, "originalLine": 7, "startLine": 5, "diffSide": "LEFT",
         "comments": {"nodes": [
            {"id": "PRRC_kwDOAbc002", "databaseId": 1002, "author": {"login": "carol"},
             "body": "Why was this removed?", "createdAt": "2026-01-02T05:00:00Z"},
            {"id": "PRRC_kwDOAbc003", "databaseId": 1003, "author": {"login": "octocat"},
             "body": "It is unused now.", "createdAt": "2026-01-02T06:00:00Z"}
         ]}},
        {"id": "PRRT_kwDOAbc789", "isResolved": false, "isOutdated": true, "path": "src/status_bar.rs",
         "line": null, "originalLine": 19, "startLine": null, "diffSide": "RIGHT",
         "comments": {"nodes": [
            {"id": "PRRC_kwDOAbc004", "databaseId": null, "author": {"login": "bob"},
             "body": "", "createdAt": "2026-01-03T09:00:00Z"}
         ]}}
    ]}}}}}"#;

    #[test]
    fn review_threads_decode_from_a_captured_answer() {
        let threads = decode_review_threads(THREADS_JSON).unwrap();
        assert_eq!(threads.len(), 3);

        let first = &threads[0];
        assert_eq!((first.id.as_str(), first.resolved, first.outdated), ("PRRT_kwDOAbc123", false, false));
        assert_eq!((first.path.as_str(), first.line, first.side), ("src/status_bar.rs", Some(42), ReviewSide::Right));
        assert_eq!(first.comments[0].author, None, "a null author is no author");
        assert_eq!(first.comments[0].database_id, Some(1001));
        assert_eq!(first.comments[0].body, "Please rename the helper.\n\nThanks", "control characters go and line breaks stay");

        let second = &threads[1];
        assert!(second.resolved);
        assert_eq!((second.line, second.side), (Some(7), ReviewSide::Left));
        let authors: Vec<Option<&str>> = second.comments.iter().map(|c| c.author.as_deref()).collect();
        assert_eq!(authors, [Some("carol"), Some("octocat")], "comments keep their order");

        let outdated = &threads[2];
        assert!(outdated.outdated);
        assert_eq!(outdated.line, None, "an outdated thread has no line");
        assert_eq!(outdated.comments[0].database_id, None);
        assert_eq!(outdated.comments[0].body, "", "an empty body stays empty");
    }

    #[test]
    fn review_threads_decode_an_empty_list() {
        let json = r#"{"data": {"repository": {"pullRequest": {"reviewThreads": {"nodes": []}}}}}"#;
        assert!(decode_review_threads(json).unwrap().is_empty());
    }

    #[test]
    fn review_threads_refuse_unexpected_answers() {
        let refused = [
            // No repository or pull request: the number or the repository does not exist.
            r#"{"data": {"repository": null}}"#.to_string(),
            r#"{"data": {"repository": {"pullRequest": null}}}"#.to_string(),
            r#"{"errors": [{"message": "Could not resolve"}]}"#.to_string(),
            // A side that is neither LEFT nor RIGHT, a line that is zero, and a bad thread ID.
            THREADS_JSON.replace(r#""diffSide": "LEFT""#, r#""diffSide": "MIDDLE""#),
            THREADS_JSON.replace(r#""line": 42"#, r#""line": 0"#),
            THREADS_JSON.replace(r#""line": 42"#, r#""line": 4.5"#),
            THREADS_JSON.replace("PRRT_kwDOAbc123", "PRRT kwDOAbc123"),
            // A comment without a creation time.
            THREADS_JSON.replace(r#""createdAt": "2026-01-02T04:00:00Z""#, r#""createdAt": "yesterday""#),
            // A thread with no path.
            THREADS_JSON.replace(
                r#""path": "src/status_bar.rs",
         "line": 42"#,
                r#""path": "",
         "line": 42"#,
            ),
        ];
        for json in refused {
            assert!(decode_review_threads(&json).is_err(), "refused: {}", &json[..json.len().min(80)]);
        }
        assert!(decode_review_threads("not json").is_err());
    }

    #[test]
    fn anchors_match_the_side_and_number_of_a_diff_line() {
        let patch = "diff --git a/f b/f\n--- a/f\n+++ b/f\n@@ -5,3 +5,3 @@\n keep\n-old\n+new\n keep two";
        let lines = file_diffs(patch).remove(0).lines;
        // Line 6 is the removed line on the left and the added line on the right.
        assert_eq!(anchor_index(&lines, ReviewSide::Left, 6), Some(5));
        assert_eq!(anchor_index(&lines, ReviewSide::Right, 6), Some(6));
        assert_eq!(anchor_index(&lines, ReviewSide::Right, 7), Some(7), "an unchanged line has a new number");
        assert_eq!(anchor_index(&lines, ReviewSide::Right, 99), None, "a line the diff does not show has no anchor");
        assert_eq!(comment_target(&lines[5]), Some((ReviewSide::Left, 6)));
        assert_eq!(comment_target(&lines[6]), Some((ReviewSide::Right, 6)));
        assert_eq!(comment_target(&lines[0]), None, "a header takes no comment");
    }

    #[test]
    fn file_diffs_split_files_and_read_their_paths_from_the_headers() {
        let patch = concat!(
            "diff --git a/src/a.rs b/src/a.rs\n",
            "index 111..222 100644\n",
            "--- a/src/a.rs\n",
            "+++ b/src/a.rs\n",
            "@@ -1,2 +1,4 @@\n",
            " keep\n",
            "-old\n",
            "+new\n",
            // Added content that looks like file headers. It is inside a hunk, so it is not a header.
            "+++ b/not/a/file\n",
            "+--- a/also/content\n",
            "diff --git a/gone.txt b/gone.txt\n",
            "deleted file mode 100644\n",
            "--- a/gone.txt\n",
            "+++ /dev/null\n",
            "@@ -1 +0,0 @@\n",
            "-bye\n",
            "diff --git a/old name.txt b/new name.txt\n",
            "similarity index 90%\n",
            "rename from old name.txt\n",
            "rename to new name.txt\n",
            "diff --git \"a/tab\\there.txt\" \"b/tab\\there.txt\"\n",
            "new file mode 100644\n",
            "--- /dev/null\n",
            "+++ \"b/tab\\there.txt\"\n",
            "@@ -0,0 +1 @@\n",
            "+hello\n",
            "diff --git a/logo.png b/logo.png\n",
            "Binary files a/logo.png and b/logo.png differ\n",
        );
        let files = file_diffs(patch);
        let paths: Vec<&str> = files.iter().map(|file| file.path.as_str()).collect();
        assert_eq!(paths, ["src/a.rs", "gone.txt", "new name.txt", "tab\there.txt", "logo.png"]);

        let first = &files[0];
        // Added lines that look like headers stay in the hunk, with their numbers.
        assert_eq!(first.lines[8].text, "+++ b/not/a/file");
        assert_eq!(first.lines[8].kind, DiffLineKind::Addition);
        assert_eq!(first.lines[8].new_number, Some(3));
        assert_eq!(first.lines[9].new_number, Some(4));
        assert_eq!(first.lines.iter().filter(|line| line.kind == DiffLineKind::Addition).count(), 3);
        assert_eq!(files[1].lines.last().unwrap().old_number, Some(1));
        assert!(files[4].lines.iter().all(|line| line.kind == DiffLineKind::Metadata), "a binary file has no hunks");
    }

    #[test]
    fn file_diffs_of_an_empty_patch_are_empty() {
        assert!(file_diffs("").is_empty());
        assert!(file_diffs("no header here\n").is_empty());
    }

    #[test]
    fn quoted_paths_lose_their_quotes_and_escapes() {
        assert_eq!(unquote("\"a\\tb\\\"c\\\\d\""), "a\tb\"c\\d");
        assert_eq!(unquote("\"caf\\303\\251.txt\""), "café.txt", "octal bytes are UTF-8");
        assert_eq!(unquote("plain/name.txt"), "plain/name.txt");
    }

    /// Whether `arguments` holds `flag` directly followed by `value`.
    fn has_pair(arguments: &[String], flag: &str, value: &str) -> bool {
        arguments.windows(2).any(|pair| pair[0] == flag && pair[1] == value)
    }

    #[test]
    fn review_threads_arguments_pass_names_as_fields_and_never_in_the_query() {
        let arguments = review_threads_arguments(&repository(), 21);
        assert_eq!(&arguments[..2], ["api", "graphql"]);
        let query = arguments.iter().find_map(|a| a.strip_prefix("query=")).unwrap();
        assert!(!query.contains("acme") && !query.contains("21"), "the query names variables only");
        assert!(query.contains("$owner") && query.contains("$name") && query.contains("$number"));
        assert!(has_pair(&arguments, "-f", "owner=acme"));
        assert!(has_pair(&arguments, "-f", "name=app"));
        assert!(has_pair(&arguments, "-F", "number=21"));
    }

    #[test]
    fn the_read_query_is_balanced_and_asks_for_each_field_the_decoder_needs() {
        let query = review_threads_query();
        assert_eq!(query.matches('{').count(), query.matches('}').count(), "braces balance");
        for field in ["isResolved", "isOutdated", "path", "line", "diffSide", "databaseId", "createdAt"] {
            assert!(query.contains(field), "{field}");
        }
    }

    #[test]
    fn reply_and_comment_arguments_carry_the_body_as_a_value_only() {
        let body = "Use \"quotes\" and $thread, then --admin\n@octocat";
        let reply = reply_arguments("PRRT_kwDOAbc123", body);
        let query = reply.iter().find_map(|a| a.strip_prefix("query=")).unwrap();
        assert!(!query.contains(body) && !query.contains("--admin") && !query.contains("PRRT_"), "text stays out of the query");
        assert!(has_pair(&reply, "-f", &format!("body={body}")));
        assert!(has_pair(&reply, "-f", "thread=PRRT_kwDOAbc123"));

        let comment = NewLineComment {
            head_oid: "0123456789abcdef0123456789abcdef01234567".into(),
            path: "src/a b.rs".into(),
            line: 12,
            side: ReviewSide::Left,
            body: body.into(),
        };
        let arguments = line_comment_arguments(&repository(), 21, &comment);
        assert_eq!(&arguments[..4], ["api", "-X", "POST", "repos/acme/app/pulls/21/comments"]);
        assert!(has_pair(&arguments, "-F", "line=12"));
        assert!(has_pair(&arguments, "-f", "side=LEFT"));
        assert!(has_pair(&arguments, "-f", "path=src/a b.rs"));
        assert!(has_pair(&arguments, "-f", "commit_id=0123456789abcdef0123456789abcdef01234567"));
        assert!(has_pair(&arguments, "-f", &format!("body={body}")));
    }

    #[test]
    fn resolve_arguments_name_the_mutation_for_each_direction() {
        let resolve = resolve_arguments("PRRT_kwDOAbc123", true);
        assert!(resolve.iter().any(|a| a.contains("{ resolveReviewThread(input: {threadId: $thread})")));
        assert!(!resolve.iter().any(|a| a.contains("unresolve")));
        let reopen = resolve_arguments("PRRT_kwDOAbc123", false);
        assert!(reopen.iter().any(|a| a.contains("{ unresolveReviewThread(input: {threadId: $thread})")));
        assert!(has_pair(&reopen, "-f", "thread=PRRT_kwDOAbc123"));
    }

    #[test]
    fn comment_bodies_keep_lines_and_refuse_empty_or_long_text() {
        assert_eq!(comment_body("  one\r\ntwo\u{7}\n\n").unwrap(), "one\ntwo");
        assert!(comment_body(" \n\t ").is_err());
        assert!(comment_body(&"x".repeat(COMMENT_POST_LIMIT + 1)).is_err(), "a long comment is refused, not cut");
        assert_eq!(comment_body(&"x".repeat(COMMENT_POST_LIMIT)).unwrap().chars().count(), COMMENT_POST_LIMIT);
    }

    #[test]
    fn node_ids_are_refused_when_they_could_read_as_flags_or_paths() {
        assert!(valid_node_id("PRRT_kwDOAbc123"));
        assert!(valid_node_id("MDI0/abc+def=="));
        assert!(!valid_node_id(""));
        assert!(!valid_node_id("PRRT kwDO"));
        assert!(!valid_node_id("PRRT\n-f"));
        assert!(!valid_node_id(&"a".repeat(NODE_ID_LIMIT + 1)));
    }

    #[test]
    fn writes_are_refused_before_gh_runs_for_bad_input() {
        let client = GitClient::new();
        let directory = std::env::temp_dir();
        let program = GhProgram::Given(std::path::PathBuf::from("/nonexistent/gh"));
        let repo = repository();
        let refused = client.reply_to_thread(&repo, "PRRT_kwDOAbc123", "  ", &program, &directory).unwrap_err().to_string();
        assert!(refused.contains("Write a comment"), "{refused}");
        let refused = client.reply_to_thread(&repo, "bad id", "Thanks", &program, &directory).unwrap_err().to_string();
        assert!(refused.contains("not valid"), "{refused}");
        let mut bad_repo = repo.clone();
        bad_repo.name = "../app".into();
        assert!(client.set_thread_resolved(&bad_repo, "PRRT_kwDOAbc123", true, &program, &directory).is_err());
        let comment =
            NewLineComment { head_oid: "not an oid".into(), path: "a".into(), line: 1, side: ReviewSide::Right, body: "x".into() };
        assert!(client.add_line_comment(&repo, 21, &comment, &program, &directory).is_err());
        assert!(client.review_threads(&repo, 0, &program, &Cancel::default(), &directory).is_err());
    }

    #[cfg(unix)]
    mod with_stand_in {
        use super::*;
        use std::os::unix::fs::PermissionsExt;
        use std::path::PathBuf;

        /// A stand-in `gh` that records each call's arguments in `calls.txt` and prints `response.json`.
        const STAND_IN: &str = "#!/bin/sh\ndir=\"$(dirname \"$0\")\"\n{ printf '%s\\n' \"$@\"; echo '--end-of-call--'; } >> \"$dir/calls.txt\"\ncat \"$dir/response.json\"\n";

        fn stand_in(response: &str) -> (crate::rebase::Scratch, GhProgram, PathBuf) {
            let scratch = crate::rebase::Scratch::new("review thread test").unwrap();
            let script = scratch.write("review thread test", "gh", STAND_IN.as_bytes()).unwrap();
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
            scratch.write("review thread test", "response.json", response.as_bytes()).unwrap();
            let folder = script.parent().unwrap().to_path_buf();
            (scratch, GhProgram::Given(script), folder)
        }

        fn calls(folder: &Path) -> Vec<Vec<String>> {
            let text = std::fs::read_to_string(folder.join("calls.txt")).unwrap_or_default();
            text.split("--end-of-call--\n").filter(|call| !call.is_empty()).map(|call| call.lines().map(str::to_string).collect()).collect()
        }

        /// Whether `arguments` holds `flag` directly followed by `value`.
        fn has_pair(arguments: &[String], flag: &str, value: &str) -> bool {
            arguments.windows(2).any(|pair| pair[0] == flag && pair[1] == value)
        }

        #[test]
        fn reading_threads_runs_one_graphql_query_with_the_names_as_fields() {
            let (_scratch, program, folder) = stand_in(THREADS_JSON);
            let threads = GitClient::new().review_threads(&repository(), 21, &program, &Cancel::default(), &std::env::temp_dir()).unwrap();
            assert_eq!(threads.len(), 3);
            let recorded = calls(&folder);
            assert_eq!(recorded.len(), 1);
            assert_eq!(recorded[0][..2], ["api", "graphql"]);
            assert!(has_pair(&recorded[0], "-F", "number=21"));
        }

        #[test]
        fn a_reply_sends_the_body_as_one_argument_and_a_resolve_names_the_thread() {
            let (_scratch, program, folder) = stand_in("{}");
            let client = GitClient::new();
            let directory = std::env::temp_dir();
            // One line, because the stand-in logs one argument per line.
            let body = "Done, thanks. --admin";
            client.reply_to_thread(&repository(), "PRRT_kwDOAbc123", body, &program, &directory).unwrap();
            client.set_thread_resolved(&repository(), "PRRT_kwDOAbc123", true, &program, &directory).unwrap();
            let recorded = calls(&folder);
            assert_eq!(recorded.len(), 2);
            assert!(has_pair(&recorded[0], "-f", &format!("body={body}")), "the body is one field value");
            assert!(recorded[1].iter().any(|word| word.contains("{ resolveReviewThread(input")));
            assert!(has_pair(&recorded[1], "-f", "thread=PRRT_kwDOAbc123"));
        }

        #[test]
        fn a_line_comment_is_posted_once_with_its_commit_and_side() {
            let (_scratch, program, folder) = stand_in("{}");
            let comment = NewLineComment {
                head_oid: "0123456789abcdef0123456789abcdef01234567".into(),
                path: "src/status_bar.rs".into(),
                line: 42,
                side: ReviewSide::Right,
                body: "Rename this.".into(),
            };
            GitClient::new().add_line_comment(&repository(), 21, &comment, &program, &std::env::temp_dir()).unwrap();
            let recorded = calls(&folder);
            assert_eq!(recorded.len(), 1);
            assert_eq!(recorded[0][..4], ["api", "-X", "POST", "repos/acme/app/pulls/21/comments"]);
            assert!(has_pair(&recorded[0], "-f", "side=RIGHT"));
            assert!(has_pair(&recorded[0], "-F", "line=42"));
        }
    }
}
