//! Line-by-line authorship (`git blame --porcelain`).

use std::collections::HashMap;
use std::path::Path;

use crate::client::GitClient;
use crate::models::{GitError, Result};
use crate::runner::{self, RunOptions};

/// The commit that last changed a group of blamed lines.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlameCommit {
    pub hash: String,
    pub author_name: String,
    pub author_email: String,
    /// Author time, in seconds since the Unix epoch.
    pub author_time: Option<i64>,
    pub summary: String,
    /// The path this commit's version of the lines used, which differs after a rename.
    pub path: String,
}

impl BlameCommit {
    /// Lines Git has not seen in any commit, such as unsaved working-tree edits.
    pub fn is_uncommitted(&self) -> bool {
        !self.hash.is_empty() && self.hash.bytes().all(|byte| byte == b'0')
    }

    pub fn short_hash(&self) -> &str {
        &self.hash[..self.hash.len().min(7)]
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlameLine {
    /// One-based line number in the blamed version of the file.
    pub number: usize,
    /// The line's text without its newline. A CRLF file keeps the carriage return.
    pub content: String,
    pub commit: BlameCommit,
}

/// Parses `git blame --porcelain` output. Commit details appear only on a commit's first group,
/// so later groups reuse them; every content line starts with a tab.
///
/// The output is split on bytes: splitting on `\n` as text would still work, but the content
/// keeps its own bytes and CRLF endings stay intact.
pub fn parse_blame(output: &[u8]) -> Vec<BlameLine> {
    let mut commits: HashMap<String, BlameCommit> = HashMap::new();
    let mut result = Vec::new();
    let mut lines = output.split(|byte| *byte == b'\n').peekable();
    while let Some(header) = lines.next() {
        let header = String::from_utf8_lossy(header);
        let mut fields = header.split(' ');
        let (Some(hash), Some(_original), Some(number)) = (fields.next(), fields.next(), fields.next()) else { continue };
        let (true, Ok(number)) = (hash.len() >= 40, number.parse::<usize>()) else { continue };

        let mut info: HashMap<String, String> = HashMap::new();
        while let Some(line) = lines.peek().copied() {
            if line.first() == Some(&b'\t') {
                break;
            }
            lines.next();
            let line = String::from_utf8_lossy(line).into_owned();
            let (key, value) = line.split_once(' ').unwrap_or((line.as_str(), ""));
            info.insert(key.to_string(), value.to_string());
        }
        let Some(content) = lines.next() else { break };

        let commit = commits.entry(hash.to_string()).or_insert_with(|| BlameCommit {
            hash: hash.to_string(),
            author_name: info.get("author").cloned().unwrap_or_default(),
            author_email: info.get("author-mail").map(|mail| mail.trim_matches(['<', '>'])).unwrap_or_default().to_string(),
            author_time: info.get("author-time").and_then(|time| time.parse().ok()),
            summary: info.get("summary").cloned().unwrap_or_default(),
            path: unquote(info.get("filename").map(String::as_str).unwrap_or_default()),
        });
        let content = String::from_utf8_lossy(&content[1..]).into_owned();
        result.push(BlameLine { number, content, commit: commit.clone() });
    }
    result
}

/// Reverses Git's C-style path quoting, including octal escapes for raw bytes.
pub fn unquote(value: &str) -> String {
    let bytes = value.as_bytes();
    if bytes.len() < 2 || !value.starts_with('"') || !value.ends_with('"') {
        return value.to_string();
    }
    let inner = &bytes[1..bytes.len() - 1];
    let mut output = Vec::with_capacity(inner.len());
    let mut index = 0;
    while index < inner.len() {
        let byte = inner[index];
        index += 1;
        if byte != b'\\' || index >= inner.len() {
            output.push(byte);
            continue;
        }
        let next = inner[index];
        index += 1;
        match next {
            b'n' => output.push(b'\n'),
            b't' => output.push(b'\t'),
            b'r' => output.push(b'\r'),
            b'a' => output.push(0x07),
            b'b' => output.push(0x08),
            b'f' => output.push(0x0C),
            b'v' => output.push(0x0B),
            b'0'..=b'7' => {
                let mut octal = u32::from(next - b'0');
                for _ in 0..2 {
                    match inner.get(index) {
                        Some(digit @ b'0'..=b'7') => {
                            octal = octal * 8 + u32::from(digit - b'0');
                            index += 1;
                        }
                        _ => break,
                    }
                }
                output.push(octal as u8);
            }
            other => output.push(other),
        }
    }
    String::from_utf8_lossy(&output).into_owned()
}

impl GitClient {
    /// Blames each line of `path` at `revision`, or in the working file when `revision` is `None`.
    /// With `ignore_whitespace`, lines moved past whitespace-only changes keep their earlier author.
    pub fn blame(&self, path: &str, revision: Option<&str>, ignore_whitespace: bool, directory: &Path) -> Result<Vec<BlameLine>> {
        let commit = match revision {
            // Blame parses its own arguments, so pass a resolved object ID rather than user text.
            Some(revision) => Some(self.resolve_commit(revision, directory)?),
            None => None,
        };
        let mut arguments = vec!["blame", "--porcelain", "--no-progress"];
        if ignore_whitespace {
            arguments.push("-w");
        }
        if let Some(commit) = &commit {
            arguments.push(commit);
        }
        arguments.extend(["--", path]);
        let options = RunOptions { env: &[("GIT_LITERAL_PATHSPECS", "1")], ..self.options() };
        let output = runner::run_bytes(&arguments, directory, &options)?;
        if output.contains(&0) {
            return Err(GitError::failed("blame", "This file is binary, so it has no lines to blame."));
        }
        Ok(parse_blame(&output))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unquotes_c_style_paths() {
        assert_eq!(unquote("\"a\\tb\\303\\251.txt\""), "a\tbé.txt");
        assert_eq!(unquote("plain name"), "plain name");
    }

    #[test]
    fn groups_share_commit_details() {
        let hash = "a".repeat(40);
        let other = "b".repeat(40);
        let output = format!(
            "{hash} 1 1 2\nauthor Ann\nauthor-mail <ann@example.com>\nauthor-time 1700000000\nsummary First\nfilename f.txt\n\tone\n\
             {hash} 2 2\n\ttwo\r\n{other} 3 3 1\nauthor Bob\nauthor-mail <bob@example.com>\nauthor-time 1700000100\nsummary Second\nfilename f.txt\n\tthree\n"
        );
        let lines = parse_blame(output.as_bytes());
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].commit.author_name, "Ann");
        assert_eq!(lines[0].commit.author_email, "ann@example.com");
        assert_eq!(lines[1].commit.summary, "First");
        assert_eq!(lines[1].content, "two\r");
        assert_eq!(lines[2].commit.author_time, Some(1_700_000_100));
        assert_eq!(lines[2].commit.short_hash(), "bbbbbbb");
        assert!(!lines[2].commit.is_uncommitted());
    }
}
