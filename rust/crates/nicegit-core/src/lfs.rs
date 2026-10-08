//! Git LFS: which patterns store files with LFS, which files it holds, and whether their content
//! is on this machine. Tracking edits the root `.gitattributes`, as `git lfs track` does.

use std::io::Read;
use std::path::Path;

use crate::client::GitClient;
use crate::models::{GitError, Result};

const ATTRIBUTES_FILE: &str = ".gitattributes";
const POINTER_PREFIX: &[u8] = b"version https://git-lfs.github.com/spec/v1";
/// Paths sent to `check-attr` per command, which keeps argument lists short.
const ATTRIBUTE_BATCH: usize = 500;

/// A file that Git LFS stores.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LfsFile {
    pub path: String,
    /// The working file is still an LFS pointer because its content has not been downloaded.
    pub is_pointer_only: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LfsStatus {
    /// The installed git-lfs version, or None when Git LFS is not installed.
    pub version: Option<String>,
    /// Patterns in the root `.gitattributes` that store matching files with Git LFS.
    pub patterns: Vec<String>,
    pub files: Vec<LfsFile>,
}

impl GitClient {
    /// The installed Git LFS version, the tracked patterns, and every tracked file that uses LFS.
    /// Reading does not need Git LFS to be installed.
    pub fn lfs_status(&self, directory: &Path) -> Result<LfsStatus> {
        let root = self.repository_root(directory)?;
        let version = self.try_trimmed(&["lfs", "version"], &root);
        let tracked = self.run(&["ls-files", "-z"], &root)?;
        let paths: Vec<&str> = tracked.split('\0').filter(|path| !path.is_empty()).collect();
        let mut files = Vec::new();
        for batch in paths.chunks(ATTRIBUTE_BATCH) {
            let mut arguments = vec!["check-attr", "-z", "filter", "--"];
            arguments.extend_from_slice(batch);
            let output = self.run(&arguments, &root)?;
            // Each record is a path, the attribute name, and its value, all NUL-terminated.
            let fields: Vec<&str> = output.split('\0').collect();
            for [path, _attribute, value] in fields.as_chunks::<3>().0 {
                if *value == "lfs" {
                    files.push(LfsFile { path: path.to_string(), is_pointer_only: is_lfs_pointer(&root.join(path)) });
                }
            }
        }
        Ok(LfsStatus { version, patterns: lfs_patterns(&root), files })
    }

    /// Stores files matching `pattern` with Git LFS from their next commit, as `git lfs track`
    /// does. Refused while Git LFS is not installed, since Git would otherwise commit the full files.
    pub fn track_lfs(&self, pattern: &str, directory: &Path) -> Result<()> {
        let pattern = pattern.trim();
        if pattern.is_empty() || pattern.contains(['\n', '\r']) {
            return Err(GitError::failed("lfs track", "Enter a file pattern such as *.psd."));
        }
        let root = self.repository_root(directory)?;
        if self.run(&["lfs", "version"], &root).is_err() {
            return Err(GitError::failed(
                "lfs track",
                "Git LFS is not installed. Install it (for example with `brew install git-lfs`) before tracking files.",
            ));
        }
        if lfs_patterns(&root).iter().any(|existing| existing == pattern) {
            return Ok(());
        }
        // Patterns with spaces, quotes, or a leading `#` are quoted the way .gitattributes expects.
        let written = if pattern.starts_with('#') || pattern.chars().any(|c| c.is_whitespace() || c == '"') {
            quote_attribute_pattern(pattern)
        } else {
            pattern.to_string()
        };
        edit_attributes(&root, |lines| lines.push(format!("{written} filter=lfs diff=lfs merge=lfs -text")))
    }

    /// Stops storing new versions of matching files with Git LFS. Files already committed stay as they are.
    pub fn untrack_lfs(&self, pattern: &str, directory: &Path) -> Result<()> {
        let root = self.repository_root(directory)?;
        edit_attributes(&root, |lines| {
            lines.retain(|line| {
                !(attribute_pattern(line).as_deref() == Some(pattern) && line.split_whitespace().any(|token| token == "filter=lfs"))
            })
        })
    }
}

/// The patterns in the root `.gitattributes` that use `filter=lfs`. A missing file has none.
fn lfs_patterns(root: &Path) -> Vec<String> {
    let text = std::fs::read_to_string(root.join(ATTRIBUTES_FILE)).unwrap_or_default();
    text.lines()
        .filter(|line| line.split_whitespace().any(|token| token == "filter=lfs"))
        .filter_map(attribute_pattern)
        .collect()
}

/// Applies `change` to the attribute file's lines and writes it back only if something changed.
/// A file that exists but cannot be read is an error, never overwritten with partial content.
fn edit_attributes(root: &Path, change: impl FnOnce(&mut Vec<String>)) -> Result<()> {
    let file = root.join(ATTRIBUTES_FILE);
    let text = match std::fs::read_to_string(&file) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(GitError::failed("lfs", format!("Could not read {ATTRIBUTES_FILE}: {error}"))),
    };
    let mut lines: Vec<String> = text.split('\n').map(str::to_string).collect();
    if lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    let before = lines.clone();
    change(&mut lines);
    if lines == before {
        return Ok(());
    }
    let mut updated = lines.join("\n");
    if !lines.is_empty() {
        updated.push('\n');
    }
    std::fs::write(&file, updated).map_err(|error| GitError::failed("lfs", format!("Could not write {ATTRIBUTES_FILE}: {error}")))
}

/// The pattern at the start of a `.gitattributes` line, unquoting a C-style quoted pattern.
/// Comments and blank lines have none.
fn attribute_pattern(line: &str) -> Option<String> {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return None;
    }
    if let Some(rest) = trimmed.strip_prefix('"') {
        // The pattern ends at the first closing quote that is not escaped.
        let mut escaped = false;
        for (index, character) in rest.char_indices() {
            if character == '"' && !escaped {
                return Some(unquote(&rest[..index]));
            }
            escaped = character == '\\' && !escaped;
        }
        return None;
    }
    trimmed.split_whitespace().next().map(str::to_string)
}

fn quote_attribute_pattern(pattern: &str) -> String {
    format!("\"{}\"", pattern.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Decodes the escapes inside a C-style quoted pattern: `\\`, `\"`, the usual control escapes,
/// and three-digit octal bytes. Unknown escapes keep the escaped character.
fn unquote(raw: &str) -> String {
    let mut bytes = Vec::with_capacity(raw.len());
    let mut chars = raw.bytes().peekable();
    while let Some(byte) = chars.next() {
        if byte != b'\\' {
            bytes.push(byte);
            continue;
        }
        let Some(escape) = chars.next() else { break };
        match escape {
            b'n' => bytes.push(b'\n'),
            b't' => bytes.push(b'\t'),
            b'r' => bytes.push(b'\r'),
            b'a' => bytes.push(7),
            b'b' => bytes.push(8),
            b'f' => bytes.push(12),
            b'v' => bytes.push(11),
            b'0'..=b'7' => {
                let mut value = u32::from(escape - b'0');
                for _ in 0..2 {
                    match chars.peek() {
                        Some(digit @ b'0'..=b'7') => {
                            value = value * 8 + u32::from(digit - b'0');
                            chars.next();
                        }
                        _ => break,
                    }
                }
                bytes.push(value as u8);
            }
            other => bytes.push(other),
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Whether a file is an LFS pointer rather than its content. Only the first bytes are read.
fn is_lfs_pointer(file: &Path) -> bool {
    let Ok(handle) = std::fs::File::open(file) else { return false };
    let mut prefix = Vec::with_capacity(POINTER_PREFIX.len());
    if handle.take(POINTER_PREFIX.len() as u64).read_to_end(&mut prefix).is_err() {
        return false;
    }
    prefix.starts_with(POINTER_PREFIX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attribute_patterns_are_read_plain_and_quoted() {
        assert_eq!(attribute_pattern("*.psd filter=lfs diff=lfs"), Some("*.psd".to_string()));
        assert_eq!(attribute_pattern("\"my \\\"big\\\" file.bin\" filter=lfs"), Some("my \"big\" file.bin".to_string()));
        assert_eq!(attribute_pattern("\"a\\tb\" filter=lfs"), Some("a\tb".to_string()));
        assert_eq!(attribute_pattern("\"\\303\\251.bin\" filter=lfs"), Some("é.bin".to_string()));
        assert_eq!(attribute_pattern("# comment filter=lfs"), None);
        assert_eq!(attribute_pattern("   "), None);
        assert_eq!(attribute_pattern("\"unterminated filter=lfs"), None);
    }

    #[test]
    fn quoting_round_trips() {
        let pattern = "dir with \"quotes\"\\*.bin";
        assert_eq!(attribute_pattern(&format!("{} filter=lfs", quote_attribute_pattern(pattern))), Some(pattern.to_string()));
    }
}
