//! How many lines each changed file adds and removes, for a sense of each change's size.

use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::Path;

use crate::client::GitClient;
use crate::models::Result;

/// Untracked files larger than this are not read to count their lines.
const MAX_UNTRACKED_BYTES: u64 = 2_000_000;

/// The size of one file's change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineCount {
    Text {
        added: usize,
        removed: usize,
    },
    /// A binary file, which has no lines.
    Binary,
}

/// Each changed file's line counts, by path: `unstaged` compares the working files with the
/// index, and `staged` the index with HEAD. A rename is counted under its new path.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LineCounts {
    pub unstaged: HashMap<String, LineCount>,
    pub staged: HashMap<String, LineCount>,
}

impl GitClient {
    /// Counts the lines each changed file adds and removes. `untracked` lists untracked files,
    /// which Git does not diff; each is read to count its lines, unless it is large.
    pub fn line_counts(&self, untracked: &[String], directory: &Path) -> Result<LineCounts> {
        let options = ["diff", "--numstat", "-z", "--no-ext-diff", "--no-color", "--no-textconv"];
        let mut unstaged = parse_numstat(&self.run(&options, directory)?);
        let mut cached = options.to_vec();
        cached.push("--cached");
        let staged = parse_numstat(&self.run(&cached, directory)?);
        for path in untracked.iter().filter(|path| !path.ends_with('/')) {
            if let Some(count) = count_untracked(&directory.join(path)) {
                unstaged.insert(path.clone(), count);
            }
        }
        Ok(LineCounts { unstaged, staged })
    }
}

impl GitClient {
    /// Counts the lines each file of a commit adds and removes, against its first parent as the
    /// commit's file list compares them, so the two always agree.
    pub fn commit_line_counts(&self, hash: &str, directory: &Path) -> Result<HashMap<String, LineCount>> {
        let output = self.run(
            &["diff-tree", "--root", "--no-commit-id", "--first-parent", "-m", "-r", "--no-renames", "--numstat", "-z", hash, "--"],
            directory,
        )?;
        Ok(parse_numstat(&output))
    }
}

/// Parses `git diff --numstat -z`: `added<TAB>removed<TAB>path<NUL>` for each file, or for a
/// rename `added<TAB>removed<TAB><NUL>old<NUL>new<NUL>`. Binary files show `-` for both counts.
pub fn parse_numstat(output: &str) -> HashMap<String, LineCount> {
    let mut counts = HashMap::new();
    let mut tokens = output.split('\0');
    while let Some(record) = tokens.next() {
        let mut fields = record.splitn(3, '\t');
        let (Some(added), Some(removed), Some(path)) = (fields.next(), fields.next(), fields.next()) else { continue };
        let path = if path.is_empty() {
            // A rename: the old path, then the new one, each in its own token.
            let _old = tokens.next();
            match tokens.next() {
                Some(new) => new,
                None => continue,
            }
        } else {
            path
        };
        let count = match (added.parse(), removed.parse()) {
            (Ok(added), Ok(removed)) => LineCount::Text { added, removed },
            _ => LineCount::Binary,
        };
        counts.insert(path.to_string(), count);
    }
    counts
}

/// Counts an untracked file's lines, all of them added. `None` for anything but a regular
/// file, and for files too large to read quickly.
fn count_untracked(file: &Path) -> Option<LineCount> {
    let metadata = fs::symlink_metadata(file).ok()?;
    if !metadata.is_file() || metadata.len() > MAX_UNTRACKED_BYTES {
        return None;
    }
    let mut data = Vec::with_capacity(metadata.len() as usize);
    fs::File::open(file).ok()?.take(MAX_UNTRACKED_BYTES).read_to_end(&mut data).ok()?;
    Some(count_lines(&data))
}

/// The lines in a file's contents, as Git counts them; contents with a NUL byte are binary.
fn count_lines(data: &[u8]) -> LineCount {
    if data.contains(&0) {
        return LineCount::Binary;
    }
    let newlines = data.iter().filter(|&&byte| byte == b'\n').count();
    let unterminated = usize::from(data.last().is_some_and(|&byte| byte != b'\n'));
    LineCount::Text { added: newlines + unterminated, removed: 0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_changes_renames_binaries_and_paths_with_tabs() {
        let output = "3\t1\tsrc/main.rs\x002\t0\t\x00old name.txt\x00new name.txt\x00-\t-\timage.png\x001\t1\ttab\there.txt\x00";
        let counts = parse_numstat(output);
        assert_eq!(counts["src/main.rs"], LineCount::Text { added: 3, removed: 1 });
        assert_eq!(counts["new name.txt"], LineCount::Text { added: 2, removed: 0 });
        assert!(!counts.contains_key("old name.txt"), "a rename is counted under its new path");
        assert_eq!(counts["image.png"], LineCount::Binary);
        assert_eq!(counts["tab\there.txt"], LineCount::Text { added: 1, removed: 1 });
        assert_eq!(counts.len(), 4);
        assert!(parse_numstat("").is_empty());
    }

    #[test]
    fn counts_lines_with_or_without_a_final_newline_and_spots_binaries() {
        assert_eq!(count_lines(b""), LineCount::Text { added: 0, removed: 0 });
        assert_eq!(count_lines(b"a\nb\n"), LineCount::Text { added: 2, removed: 0 });
        assert_eq!(count_lines(b"a\nb"), LineCount::Text { added: 2, removed: 0 });
        assert_eq!(count_lines(b"a\0b"), LineCount::Binary);
    }
}
