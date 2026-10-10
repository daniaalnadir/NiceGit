use std::collections::BTreeMap;

use crate::models::{Branch, Commit, Stash, StatusEntry, StatusKind, Worktree};

pub const BRANCH_FORMAT: &str = "%(refname)%09%(HEAD)%09%(objectname)%09%(contents:subject)%09%(upstream)%09%(symref)";

/// Fields are separated by control characters: commas and spaces are valid in ref names.
pub const LOG_FORMAT: &str =
    "%H%x1f%h%x1f%P%x1f%(decorate:prefix=,suffix=,separator=%x1d,pointer=%x1c,tag=tag: )%x1f%s%x1f%an%x1f%ae%x1f%cr%x1f%ct%x1e";

pub const STASH_FORMAT: &str = "%H%x09%gd%x09%s";

fn entry(path: String, original: Option<String>, x: char, y: char, conflicted: bool) -> StatusEntry {
    let kind = if conflicted {
        StatusKind::Conflicted
    } else if x == '?' {
        StatusKind::Untracked
    } else if x == 'R' || y == 'R' {
        StatusKind::Renamed
    } else if x == 'C' || y == 'C' || x == 'A' || y == 'A' {
        StatusKind::Added
    } else if x == 'D' || y == 'D' {
        StatusKind::Deleted
    } else {
        StatusKind::Modified
    };
    StatusEntry { path, original_path: original, kind, index_status: x, work_tree_status: y }
}

/// Parses `git status --porcelain=v1 -z`. Records are NUL-terminated, so paths may contain
/// newlines; renames and copies carry their source path in the following record.
pub fn parse_status(output: &str) -> Vec<StatusEntry> {
    let records: Vec<&str> = output.split('\0').collect();
    let mut entries = Vec::new();
    let mut index = 0;
    while index < records.len() {
        let record = records[index];
        index += 1;
        let mut chars = record.chars();
        let (Some(x), Some(y), Some(' ')) = (chars.next(), chars.next(), chars.next()) else { continue };
        let path = chars.as_str();
        if path.is_empty() {
            continue;
        }
        let mut original = None;
        if matches!(x, 'R' | 'C') || matches!(y, 'R' | 'C') {
            let Some(source) = records.get(index) else { break };
            original = Some(source.to_string());
            index += 1;
        }
        let pair: String = [x, y].iter().collect();
        let conflicted = ["DD", "AU", "UD", "UA", "DU", "AA", "UU"].contains(&pair.as_str());
        entries.push(entry(path.to_string(), original, x, y, conflicted));
    }
    entries
}

/// The result of `git status --porcelain=v2 -z --branch`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatusWithCheckout {
    pub entries: Vec<StatusEntry>,
    pub branch: Option<String>,
    pub head_hash: Option<String>,
    /// False when any record could not be parsed; callers then fall back to a full refresh.
    pub is_complete: bool,
}

pub fn parse_status_with_checkout(output: &str) -> StatusWithCheckout {
    let records: Vec<&str> = output.split('\0').filter(|record| !record.is_empty()).collect();
    let mut result = StatusWithCheckout { entries: Vec::new(), branch: None, head_hash: None, is_complete: true };
    let mut saw_oid = false;
    let mut index = 0;
    while index < records.len() {
        let record = records[index];
        index += 1;
        if let Some(branch) = record.strip_prefix("# branch.head ") {
            result.branch = Some(branch.to_string());
            continue;
        }
        if let Some(oid) = record.strip_prefix("# branch.oid ") {
            saw_oid = true;
            result.head_hash = (oid != "(initial)").then(|| oid.to_string());
            continue;
        }
        if let Some(path) = record.strip_prefix("? ") {
            result.entries.push(entry(path.to_string(), None, '?', '?', false));
            continue;
        }
        if record.starts_with("# ") || record.starts_with("! ") {
            continue;
        }
        let is_rename = record.starts_with("2 ");
        let is_conflict = record.starts_with("u ");
        let fields = if is_conflict {
            10
        } else if is_rename {
            9
        } else {
            8
        };
        let path = (record.starts_with("1 ") || is_rename || is_conflict)
            .then(|| record.splitn(fields + 1, ' ').nth(fields))
            .flatten()
            .filter(|path| !path.is_empty());
        let Some(path) = path else {
            result.is_complete = false;
            continue;
        };
        let mut xy = record[2..].chars();
        let dot = |c: Option<char>| match c {
            Some('.') | None => ' ',
            Some(c) => c,
        };
        let (x, y) = (dot(xy.next()), dot(xy.next()));
        let original = if is_rename {
            match records.get(index) {
                Some(source) => {
                    index += 1;
                    Some(source.to_string())
                }
                None => {
                    result.is_complete = false;
                    continue;
                }
            }
        } else {
            None
        };
        result.entries.push(entry(path.to_string(), original, x, y, is_conflict));
    }
    result.is_complete = result.is_complete && saw_oid && result.branch.is_some();
    result
}

/// Parses `git branch --all --format=BRANCH_FORMAT`. Remote HEAD aliases are identified by
/// their symref field, so a real branch whose name ends in HEAD is kept.
pub fn parse_branches(output: &str) -> Vec<Branch> {
    output
        .split('\n')
        .filter(|line| !line.is_empty())
        .filter_map(|line| {
            let fields: Vec<&str> = line.split('\t').collect();
            if fields.len() < 6 {
                return None;
            }
            let name = fields[0];
            let is_remote = name.starts_with("refs/remotes/") || name.starts_with("remotes/");
            let normalized = if let Some(rest) = name.strip_prefix("refs/heads/") {
                rest.to_string()
            } else if let Some(rest) = name.strip_prefix("refs/") {
                if is_remote {
                    rest.to_string()
                } else {
                    name.to_string()
                }
            } else {
                name.to_string()
            };
            let symref = fields[fields.len() - 1];
            if is_remote && !symref.is_empty() {
                return None;
            }
            // A subject may itself contain tabs; it spans every field between the fixed ones.
            let upstream = fields[fields.len() - 2];
            Some(Branch {
                name: normalized,
                is_current: fields[1] == "*",
                is_remote,
                tip: fields[2].to_string(),
                subject: fields[3..fields.len() - 2].join("\t"),
                upstream: (!upstream.is_empty()).then(|| upstream.to_string()),
            })
        })
        .collect()
}

pub fn parse_log(output: &str) -> Vec<Commit> {
    output
        .split('\u{1e}')
        .filter_map(|record| {
            let record = record.trim_matches(|c: char| c.is_whitespace());
            if record.is_empty() {
                return None;
            }
            let fields: Vec<&str> = record.split('\u{1f}').collect();
            if fields.len() < 8 {
                return None;
            }
            let refs = fields[3]
                .split('\u{1d}')
                .map(|reference| reference.replace('\u{1c}', " -> ").trim().to_string())
                .filter(|reference| !reference.is_empty())
                .collect();
            Some(Commit {
                hash: fields[0].to_string(),
                short_hash: fields[1].to_string(),
                parents: fields[2].split(' ').filter(|p| !p.is_empty()).map(str::to_string).collect(),
                refs,
                subject: fields[4].to_string(),
                author_name: fields[5].to_string(),
                author_email: fields[6].to_string(),
                relative_date: fields[7].to_string(),
                commit_time: fields.get(8).and_then(|value| value.trim().parse().ok()),
            })
        })
        .collect()
}

/// Remote names from `git remote -v`, sorted and unique.
pub fn parse_remotes(output: &str) -> Vec<String> {
    let mut names: Vec<String> =
        output.split('\n').filter_map(|line| line.split('\t').next()).filter(|name| !name.is_empty()).map(str::to_string).collect();
    names.sort();
    names.dedup();
    names
}

/// Every address per remote for `direction` ("fetch" or "push"), from `git remote -v`.
pub fn parse_remote_addresses(output: &str, direction: &str) -> BTreeMap<String, Vec<String>> {
    let suffix = format!(" ({direction})");
    let mut result: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for line in output.split('\n') {
        let Some(line) = line.strip_suffix(&suffix) else { continue };
        if let Some((name, address)) = line.split_once('\t') {
            result.entry(name.to_string()).or_default().push(address.to_string());
        }
    }
    result
}

pub fn parse_stashes(output: &str) -> Vec<Stash> {
    output
        .split('\n')
        .filter_map(|line| {
            let mut fields = line.splitn(3, '\t');
            let (Some(hash), Some(reference), Some(message)) = (fields.next(), fields.next(), fields.next()) else {
                return None;
            };
            Some(Stash { hash: hash.to_string(), reference: reference.to_string(), message: message.to_string() })
        })
        .collect()
}

pub fn parse_worktrees(output: &str) -> Vec<Worktree> {
    let mut result = Vec::new();
    let mut current: Option<Worktree> = None;
    for field in output.split('\0') {
        if field.is_empty() {
            if let Some(worktree) = current.take() {
                result.push(worktree);
            }
        } else if let Some(path) = field.strip_prefix("worktree ") {
            current = Some(Worktree { path: path.to_string(), branch: None, is_bare: false, is_locked: false, is_prunable: false });
        } else if let Some(worktree) = current.as_mut() {
            if let Some(branch) = field.strip_prefix("branch refs/heads/") {
                worktree.branch = Some(branch.to_string());
            } else if field == "bare" {
                worktree.is_bare = true;
            } else if field == "locked" || field.starts_with("locked ") {
                worktree.is_locked = true;
            } else if field == "prunable" || field.starts_with("prunable ") {
                worktree.is_prunable = true;
            }
        }
    }
    result.extend(current);
    result
}

/// Tag names and the object each points to, from `for-each-ref --format=%(refname:strip=2)%09%(objectname)`.
pub fn parse_tags(output: &str) -> Vec<(String, String)> {
    output.split('\n').filter_map(|line| line.split_once('\t')).map(|(name, tip)| (name.to_string(), tip.to_string())).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_keeps_newlines_in_paths_and_rename_sources() {
        let output = " M a\nb.txt\0R  new.txt\0old.txt\0?? spaced name.txt\0UU both.txt\0";
        let entries = parse_status(output);
        assert_eq!(entries.len(), 4);
        assert_eq!(entries[0].path, "a\nb.txt");
        assert_eq!(entries[0].kind, StatusKind::Modified);
        assert!(entries[0].is_unstaged() && !entries[0].is_staged());
        assert_eq!(entries[1].kind, StatusKind::Renamed);
        assert_eq!(entries[1].original_path.as_deref(), Some("old.txt"));
        assert!(entries[1].is_staged());
        assert_eq!(entries[2].kind, StatusKind::Untracked);
        assert_eq!(entries[3].kind, StatusKind::Conflicted);
    }

    #[test]
    fn status_v2_reports_checkout_headers() {
        let output = "# branch.oid abc123\0# branch.head main\0# branch.upstream origin/main\x001 .M N... 100644 100644 100644 1111 2222 file name.txt\x002 R. N... 100644 100644 100644 1111 1111 R100 renamed.txt\0original.txt\0? new.txt\0";
        let parsed = parse_status_with_checkout(output);
        assert!(parsed.is_complete);
        assert_eq!(parsed.branch.as_deref(), Some("main"));
        assert_eq!(parsed.head_hash.as_deref(), Some("abc123"));
        assert_eq!(parsed.entries[0].path, "file name.txt");
        assert_eq!(parsed.entries[0].index_status, ' ');
        assert_eq!(parsed.entries[0].work_tree_status, 'M');
        assert_eq!(parsed.entries[1].original_path.as_deref(), Some("original.txt"));
        assert_eq!(parsed.entries[2].kind, StatusKind::Untracked);
        assert!(!parse_status_with_checkout("# branch.oid (initial)\0").is_complete);
    }

    #[test]
    fn branches_drop_symbolic_remote_heads_but_keep_branches_named_head() {
        let output = [
            "refs/heads/main\t*\taaa\tFirst\tthe subject\trefs/remotes/origin/main\t",
            "refs/remotes/origin/HEAD\t \taaa\tFirst\t\trefs/remotes/origin/main",
            "refs/remotes/origin/feature/HEAD\t \tbbb\tSecond\t\t",
        ]
        .join("\n");
        let branches = parse_branches(&output);
        assert_eq!(branches.len(), 2);
        assert_eq!(branches[0].name, "main");
        assert_eq!(branches[0].subject, "First\tthe subject");
        assert_eq!(branches[0].upstream.as_deref(), Some("refs/remotes/origin/main"));
        assert!(branches[0].is_current);
        assert_eq!(branches[1].name, "remotes/origin/feature/HEAD");
        assert_eq!(branches[1].display_name(), "origin/feature/HEAD");
    }

    #[test]
    fn remote_name_prefers_longest_match() {
        let branch = Branch {
            name: "remotes/team/a/main".into(),
            is_current: false,
            is_remote: true,
            tip: String::new(),
            subject: String::new(),
            upstream: None,
        };
        let remotes = vec!["team".to_string(), "team/a".to_string()];
        assert_eq!(branch.remote_name(&remotes), Some("team/a"));
    }

    #[test]
    fn log_keeps_commas_in_ref_names() {
        let output = "aaaa\u{1f}aa\u{1f}bbbb cccc\u{1f}HEAD\u{1c}main\u{1d}tag: v1,2\u{1d}feature,x\u{1f}Subject, with comma\u{1f}Ann\u{1f}ann@example.com\u{1f}2 days ago\u{1f}1700000000\u{1e}\n";
        let commits = parse_log(output);
        assert_eq!(commits.len(), 1);
        assert_eq!(commits[0].parents, vec!["bbbb", "cccc"]);
        assert_eq!(commits[0].refs, vec!["HEAD -> main", "tag: v1,2", "feature,x"]);
        assert_eq!(commits[0].commit_time, Some(1_700_000_000));
    }

    #[test]
    fn remote_addresses_keep_every_url() {
        let output =
            "origin\thttps://a (fetch)\norigin\thttps://a (push)\norigin\thttps://b (push)\nup\tgit@x:y (fetch)\nup\tgit@x:y (push)\n";
        assert_eq!(parse_remotes(output), vec!["origin", "up"]);
        let push = parse_remote_addresses(output, "push");
        assert_eq!(push["origin"], vec!["https://a", "https://b"]);
        assert_eq!(parse_remote_addresses(output, "fetch")["up"], vec!["git@x:y"]);
    }

    #[test]
    fn worktrees_parse_flags() {
        let output = "worktree /a\0HEAD 1\0branch refs/heads/main\0\0worktree /b\0HEAD 2\0detached\0locked reason\0prunable gone\0\0";
        let worktrees = parse_worktrees(output);
        assert_eq!(worktrees.len(), 2);
        assert_eq!(worktrees[0].branch.as_deref(), Some("main"));
        assert!(worktrees[1].is_locked && worktrees[1].is_prunable);
    }
}
