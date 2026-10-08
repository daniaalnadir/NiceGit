//! Searching history (messages, authors, code changes, commit IDs) and file contents (`git grep`).

use std::path::Path;

use crate::client::GitClient;
use crate::models::{Commit, Result};
use crate::parsers::{parse_log, LOG_FORMAT};

/// What a commit search looks at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommitSearchField {
    /// Commit messages, ignoring case.
    Message,
    /// Author names and emails, ignoring case.
    Author,
    /// Commits that add or remove the text in a file's contents.
    CodeChange,
    /// One commit, named by its full or abbreviated ID.
    CommitId,
}

impl CommitSearchField {
    pub const ALL: [CommitSearchField; 4] =
        [CommitSearchField::Message, CommitSearchField::Author, CommitSearchField::CodeChange, CommitSearchField::CommitId];

    pub fn title(self) -> &'static str {
        match self {
            CommitSearchField::Message => "Message",
            CommitSearchField::Author => "Author",
            CommitSearchField::CodeChange => "Code change",
            CommitSearchField::CommitId => "Commit ID",
        }
    }
}

/// One line of file contents that matched a search.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContentMatch {
    pub path: String,
    /// One-based line number.
    pub line: usize,
    /// The matching line, without its newline. A CRLF file keeps the carriage return.
    pub text: String,
}

/// Whether `query` could be a commit ID: a full or abbreviated hexadecimal object name.
fn looks_like_commit_id(query: &str) -> bool {
    (4..=64).contains(&query.len()) && query.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// Parses `git grep -n -I --null` output. Each match is "path NUL line NUL text LF". Text never
/// holds a newline but a path can, so records are split on NUL and each text ends at its first
/// newline. The prefix ("commit:") is removed from paths when searching a revision.
fn parse_grep(output: &str, prefix: &str, limit: usize) -> Vec<ContentMatch> {
    let mut tokens = output.split('\0');
    let Some(first) = tokens.next() else { return Vec::new() };
    let mut path = first.to_string();
    let mut matches = Vec::new();
    while matches.len() < limit {
        let (Some(number), Some(rest)) = (tokens.next(), tokens.next()) else { break };
        let newline = rest.find('\n').unwrap_or(rest.len());
        if let Ok(line) = number.parse::<usize>() {
            let file = path.strip_prefix(prefix).unwrap_or(path.as_str());
            matches.push(ContentMatch { path: file.to_string(), line, text: rest[..newline].to_string() });
        }
        path = rest.get(newline + 1..).unwrap_or("").to_string();
    }
    matches
}

impl GitClient {
    /// Commits in every branch, tag, and remote-tracking branch that match `text`, newest first.
    /// Messages and authors match literally and without case; a code change matches commits that
    /// add or remove the text (`-S`). A search for a commit ID also finds that commit whatever
    /// field is chosen.
    pub fn search_commits(&self, text: &str, field: CommitSearchField, limit: usize, directory: &Path) -> Result<Vec<Commit>> {
        let query = text.trim();
        if query.is_empty() {
            return Ok(Vec::new());
        }
        let selector = match field {
            CommitSearchField::CommitId => return Ok(self.commit_by_id(query, directory)?.into_iter().collect()),
            CommitSearchField::Message => format!("--grep={query}"),
            CommitSearchField::Author => format!("--author={query}"),
            CommitSearchField::CodeChange => format!("-S{query}"),
        };
        let count = limit.max(1).to_string();
        let pretty = format!("--pretty=format:{LOG_FORMAT}");
        let mut arguments = vec!["log", "--all", "--exclude=refs/stash", "--decorate=short", "--no-color", "-n", &count, &pretty];
        if matches!(field, CommitSearchField::Message | CommitSearchField::Author) {
            arguments.extend(["--fixed-strings", "--regexp-ignore-case"]);
        }
        arguments.push(&selector);
        arguments.push("--");
        let mut commits = parse_log(&self.run(&arguments, directory)?);
        if let Some(exact) = self.commit_by_id(query, directory)? {
            if !commits.iter().any(|commit| commit.hash == exact.hash) {
                commits.insert(0, exact);
            }
        }
        Ok(commits)
    }

    /// The single commit a full or abbreviated ID names, if the query looks like an ID.
    fn commit_by_id(&self, query: &str, directory: &Path) -> Result<Option<Commit>> {
        if !looks_like_commit_id(query) {
            return Ok(None);
        }
        // An ID that matches nothing, or several commits, is simply not a result.
        let Ok(hash) = self.resolve_commit(query, directory) else { return Ok(None) };
        let pretty = format!("--pretty=format:{LOG_FORMAT}");
        let output = self.run(&["log", "-1", "--decorate=short", "--no-color", &pretty, &hash, "--"], directory)?;
        Ok(parse_log(&output).into_iter().next())
    }

    /// Lines containing `text` in tracked text files, at `revision` or in the working files when
    /// `revision` is `None`. The text is matched literally, and without case when `ignore_case`.
    /// Results stop at `limit` lines.
    pub fn search_contents(
        &self,
        text: &str,
        revision: Option<&str>,
        ignore_case: bool,
        limit: usize,
        directory: &Path,
    ) -> Result<Vec<ContentMatch>> {
        if text.is_empty() || text.contains('\n') {
            return Ok(Vec::new());
        }
        let commit = match revision {
            Some(revision) => Some(self.resolve_commit(revision, directory)?),
            None => None,
        };
        let mut arguments = vec!["grep", "-n", "-I", "--null", "--full-name", "--no-color", "--fixed-strings", "--max-count=100"];
        if ignore_case {
            arguments.push("--ignore-case");
        }
        arguments.extend(["-e", text]);
        if let Some(commit) = &commit {
            arguments.push(commit);
        }
        arguments.push("--");
        // Exit status 1 means that nothing matched.
        let output = self.run_accepting(&arguments, directory, &[0, 1])?;
        let prefix = commit.map(|commit| format!("{commit}:")).unwrap_or_default();
        Ok(parse_grep(&output, &prefix, limit))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grep_output_keeps_newlines_in_paths() {
        let output = "a\nb.txt\x003\0let x = 1;\n c.txt\x007\0\tlet y = 2;\r\n";
        let matches = parse_grep(output, "", 10);
        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0].path, "a\nb.txt");
        assert_eq!(matches[0].line, 3);
        assert_eq!(matches[0].text, "let x = 1;");
        assert_eq!(matches[1].path, " c.txt");
        assert_eq!(matches[1].text, "\tlet y = 2;\r");
    }

    #[test]
    fn grep_output_strips_revision_prefix_and_honours_limit() {
        let output = "abc:one.txt\x001\x00first\nabc:two.txt\x002\x00second\n";
        let matches = parse_grep(output, "abc:", 1);
        assert_eq!(matches, vec![ContentMatch { path: "one.txt".into(), line: 1, text: "first".into() }]);
    }

    #[test]
    fn recognises_commit_ids() {
        assert!(looks_like_commit_id("abcd"));
        assert!(!looks_like_commit_id("abc"));
        assert!(!looks_like_commit_id("main"));
    }
}
