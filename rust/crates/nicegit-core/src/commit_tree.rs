//! Every file in a commit's tree, and the contents of one of them at that commit, for the commit
//! inspector's All files view.

use std::path::Path;

use crate::client::GitClient;
use crate::models::{GitError, Result};

/// The most entries one listing keeps. A larger tree shows its first entries, and says so.
pub const MAX_TREE_ENTRIES: usize = 20_000;

/// A file larger than this is not read until the person asks for it anyway.
pub const LARGE_FILE_BYTES: u64 = 2 * 1024 * 1024;

/// How much of a file is searched for a NUL byte to tell binary content from text, as Git does.
const BINARY_PROBE_BYTES: usize = 8 * 1024;

/// A path in a commit's tree, with the object that holds its contents.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeEntry {
    /// The path relative to the repository root. It can contain newlines.
    pub path: String,
    /// The blob's object ID, or for a submodule, the commit it records.
    pub object: String,
    /// Whether the entry is a submodule. Its files are in another repository, so only its commit is known.
    pub submodule: bool,
}

/// A commit's tree, as `ls-tree` lists it: at most `MAX_TREE_ENTRIES` entries in Git's order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TreeListing {
    pub entries: Vec<TreeEntry>,
    /// Every entry in the tree, including any past the cap.
    pub total: usize,
}

impl TreeListing {
    /// Whether the tree has more entries than the listing keeps.
    pub fn truncated(&self) -> bool {
        self.total > self.entries.len()
    }
}

/// What a file looks like to the viewer: its lines, or why they are not shown.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileView {
    /// The file's lines, without line terminators. Bytes that are not UTF-8 are replaced.
    Text(Vec<String>),
    /// Binary content, with its size in bytes.
    Binary(u64),
    /// A file over `LARGE_FILE_BYTES` that was not read, with its size in bytes.
    TooLarge(u64),
    /// A submodule. Only the commit it records is known here, so it is named by that commit's ID.
    Submodule(String),
}

impl GitClient {
    /// Lists every entry in a commit's tree with its object ID. Paths can contain newlines, so
    /// records are split on NUL. Submodules are listed as entries of their own.
    pub fn commit_tree(&self, commit: &str, directory: &Path) -> Result<TreeListing> {
        let commit = self.resolve_commit(commit, directory)?;
        let output = self.run(&["ls-tree", "-r", "-z", "--full-tree", commit.as_str()], directory)?;
        Ok(parse_tree(&output))
    }

    /// Reads a blob's contents for display. A file over `LARGE_FILE_BYTES` is not read unless
    /// `open_anyway` is set. Binary content is recognised by a NUL byte near the start, as Git does.
    pub fn file_view(&self, object: &str, open_anyway: bool, directory: &Path) -> Result<FileView> {
        if !is_object_id(object) {
            return Err(GitError::failed("show file", "This file's object ID is not valid."));
        }
        let size = self.blob_size(object, directory)?;
        if size > LARGE_FILE_BYTES && !open_anyway {
            return Ok(FileView::TooLarge(size));
        }
        let bytes = self.blob_contents(object, directory)?;
        if is_binary(&bytes) {
            return Ok(FileView::Binary(bytes.len() as u64));
        }
        Ok(FileView::Text(split_lines(&bytes)))
    }

    /// A blob's size in bytes, read before its contents so a large file is not loaded by accident.
    fn blob_size(&self, object: &str, directory: &Path) -> Result<u64> {
        let text = self.run_trimmed(&["cat-file", "-s", object], directory)?;
        text.parse().map_err(|_| GitError::failed("cat-file -s", format!("Unexpected blob size: {text}")))
    }
}

/// Parses `ls-tree -r -z` output: each record is `mode type object TAB path`, ending in NUL.
/// Entries past `MAX_TREE_ENTRIES` are counted but not kept.
pub fn parse_tree(output: &str) -> TreeListing {
    let mut listing = TreeListing::default();
    for record in output.split('\0').filter(|record| !record.is_empty()) {
        // The fields never contain a tab, so the first one ends them; the path keeps any others.
        let Some((fields, path)) = record.split_once('\t') else { continue };
        let mut fields = fields.split(' ');
        let (Some(mode), Some(kind), Some(object)) = (fields.next(), fields.next(), fields.next()) else { continue };
        listing.total += 1;
        if listing.entries.len() < MAX_TREE_ENTRIES {
            listing.entries.push(TreeEntry {
                path: path.to_string(),
                object: object.to_string(),
                submodule: mode == "160000" || kind == "commit",
            });
        }
    }
    listing
}

/// Whether an object ID is a full SHA-1 or SHA-256 name. Only such IDs reach Git as arguments,
/// so none can be read as an option.
fn is_object_id(object: &str) -> bool {
    matches!(object.len(), 40 | 64) && object.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// Whether content is binary: a NUL byte in its first `BINARY_PROBE_BYTES`.
pub fn is_binary(bytes: &[u8]) -> bool {
    let probe = &bytes[..bytes.len().min(BINARY_PROBE_BYTES)];
    probe.contains(&0)
}

/// Splits content into lines on the bytes that end them, before decoding, so each line keeps its
/// own text. A carriage return before a newline is dropped. A final newline ends the last line
/// rather than starting an empty one; empty content has no lines.
fn split_lines(bytes: &[u8]) -> Vec<String> {
    if bytes.is_empty() {
        return Vec::new();
    }
    let mut lines: Vec<String> = bytes
        .split(|byte| *byte == b'\n')
        .map(|line| String::from_utf8_lossy(line.strip_suffix(b"\r").unwrap_or(line)).into_owned())
        .collect();
    if bytes.ends_with(b"\n") {
        lines.pop();
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tree_records_are_split_on_nul_and_keep_newlines_in_paths() {
        let output = "100644 blob aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\tplain.txt\0\
                      100644 blob bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\ttwo\nlines.txt\0\
                      160000 commit cccccccccccccccccccccccccccccccccccccccc\tvendor/lib\0";
        let listing = parse_tree(output);
        assert_eq!(listing.total, 3);
        assert!(!listing.truncated());
        assert_eq!(listing.entries[1].path, "two\nlines.txt");
        assert!(!listing.entries[1].submodule);
        assert!(listing.entries[2].submodule, "a gitlink is a submodule");
    }

    #[test]
    fn a_tree_past_the_cap_keeps_its_first_entries_and_counts_the_rest() {
        let record = "100644 blob aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\tf\0";
        let output = record.repeat(MAX_TREE_ENTRIES + 5);
        let listing = parse_tree(&output);
        assert_eq!(listing.entries.len(), MAX_TREE_ENTRIES);
        assert_eq!(listing.total, MAX_TREE_ENTRIES + 5);
        assert!(listing.truncated());
    }

    #[test]
    fn only_full_object_ids_reach_git() {
        assert!(is_object_id(&"a".repeat(40)));
        assert!(is_object_id(&"f".repeat(64)));
        assert!(!is_object_id("--output=/tmp/x"));
        assert!(!is_object_id(&"a".repeat(39)));
    }

    #[test]
    fn a_nul_near_the_start_makes_content_binary() {
        assert!(is_binary(b"abc\0def"));
        assert!(!is_binary(b"plain text\n"));
        let mut late = vec![b'x'; BINARY_PROBE_BYTES];
        late.push(0);
        assert!(!is_binary(&late), "only the first 8 KB decide");
    }

    #[test]
    fn lines_drop_crlf_and_a_final_newline_but_keep_blank_lines() {
        assert_eq!(split_lines(b"one\r\ntwo\n\nthree"), vec!["one", "two", "", "three"]);
        assert_eq!(split_lines(b"a\n"), vec!["a"]);
        assert_eq!(split_lines(b"a\n\n"), vec!["a", ""]);
        assert!(split_lines(b"").is_empty());
    }
}
