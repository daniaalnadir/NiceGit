//! The branches and tags that contain a commit, as the commit inspector lists them.

use std::path::Path;

use crate::client::GitClient;
use crate::models::Result;

/// The refs that contain one commit.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ContainingRefs {
    /// Local branch names (short, e.g. "main"), current branch first, then by name.
    pub local: Vec<String>,
    /// Remote-tracking branch names for display, e.g. "origin/main", by name; symbolic remote HEAD aliases left out.
    pub remote: Vec<String>,
    /// The tag on the oldest commit among the tags that contain the commit, if any: the first
    /// release that shipped it.
    pub first_tag: Option<String>,
    /// How many tags contain the commit in total (including first_tag).
    pub tag_count: usize,
}

/// Each ref's name, symbolic target, current-branch marker, and the committer date and ID of the
/// commit it names: the `*` fields for an annotated tag, which names a tag object, and the plain
/// ones otherwise. A tag is ordered by its commit, not by when it was made, since a lightweight
/// tag records no date of its own. Fields are NUL-separated, and records are newline-terminated;
/// a ref name cannot contain either.
const CONTAINING_FORMAT: &str =
    "%(refname)%00%(symref)%00%(HEAD)%00%(*committerdate:unix)%00%(committerdate:unix)%00%(*objectname)%00%(objectname)";

/// The most tags sharing the earliest commit date whose ancestry is compared.
const TIED_TAGS_COMPARED: usize = 8;

/// `ContainingRefs` as read, with the tags on the earliest commit date and their commits.
struct Parsed {
    refs: ContainingRefs,
    /// Sorted by name. Commits made in the same second, as a rebase makes them, share a date.
    earliest: Vec<(String, String)>,
}

impl GitClient {
    /// The local and remote-tracking branches that contain `commit`, and the tags that contain
    /// it, read with one Git process.
    pub fn containing_refs(&self, commit: &str, directory: &Path) -> Result<ContainingRefs> {
        let id = self.resolve_commit(commit, directory)?;
        let contains = format!("--contains={id}");
        let format = format!("--format={CONTAINING_FORMAT}");
        let output = self.run(&["for-each-ref", &contains, &format, "refs/heads", "refs/remotes", "refs/tags"], directory)?;
        let Parsed { mut refs, earliest } = parse_containing(&output);
        // Among tags whose commits share the earliest date, the first release is the one whose
        // commit the others' commits contain.
        if (2..=TIED_TAGS_COMPARED).contains(&earliest.len()) {
            let is_ancestor = |older: &str, newer: &str| self.run(&["merge-base", "--is-ancestor", older, newer], directory).is_ok();
            let first = earliest.iter().find(|(_, commit)| earliest.iter().all(|(_, other)| other == commit || is_ancestor(commit, other)));
            if let Some((tag, _)) = first {
                refs.first_tag = Some(tag.clone());
            }
        }
        Ok(refs)
    }
}

/// Reads `for-each-ref` output in `CONTAINING_FORMAT`.
fn parse_containing(output: &str) -> Parsed {
    // The current branch is kept apart so it sorts first.
    let mut local: Vec<(bool, String)> = Vec::new();
    let mut remote = Vec::new();
    let mut tags: Vec<(i64, String, String)> = Vec::new();
    for record in output.split('\n').filter(|record| !record.is_empty()) {
        let fields: Vec<&str> = record.split('\0').collect();
        let [name, symref, head, peeled_date, date, peeled_id, id, ..] = fields.as_slice() else { continue };
        if let Some(branch) = name.strip_prefix("refs/heads/") {
            local.push((*head == "*", branch.to_string()));
        } else if let Some(rest) = name.strip_prefix("refs/remotes/") {
            // A symbolic ref such as origin/HEAD points at another remote branch; it is not one itself.
            if symref.is_empty() {
                remote.push(rest.to_string());
            }
        } else if let Some(tag) = name.strip_prefix("refs/tags/") {
            let (date, commit) = if peeled_id.is_empty() { (date, id) } else { (peeled_date, peeled_id) };
            tags.push((date.parse().unwrap_or(0), tag.to_string(), commit.to_string()));
        }
    }
    local.sort_by(|(current_a, name_a), (current_b, name_b)| current_b.cmp(current_a).then_with(|| name_a.cmp(name_b)));
    remote.sort();
    // Tags sort by their commit's date, then by name, so the first one is the earliest release.
    tags.sort();
    let earliest_date = tags.first().map(|(date, ..)| *date);
    let earliest =
        tags.iter().filter(|(date, ..)| Some(*date) == earliest_date).map(|(_, name, commit)| (name.clone(), commit.clone())).collect();
    let refs = ContainingRefs {
        local: local.into_iter().map(|(_, name)| name).collect(),
        remote,
        first_tag: tags.first().map(|(_, name, _)| name.clone()),
        tag_count: tags.len(),
    };
    Parsed { refs, earliest }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(name: &str, symref: &str, date: &str, head: &str) -> String {
        format!("{name}\0{symref}\0{head}\0\0{date}\0\0{name}-commit\n")
    }

    /// An annotated tag: its own committer date is empty, and its commit's date is the peeled one.
    fn annotated(name: &str, commit_date: &str) -> String {
        format!("{name}\0\0 \0{commit_date}\0\0{name}-commit\0{name}-object\n")
    }

    #[test]
    fn current_branch_comes_first_then_branches_by_name() {
        let output =
            [record("refs/heads/feature", "", "0", " "), record("refs/heads/main", "", "0", "*"), record("refs/heads/alpha", "", "0", " ")]
                .concat();
        let refs = parse_containing(&output).refs;
        assert_eq!(refs.local, ["main", "alpha", "feature"]);
    }

    #[test]
    fn symbolic_remote_heads_are_left_out_and_slashed_remotes_are_kept_whole() {
        let output = [
            record("refs/remotes/origin/main", "", "0", " "),
            record("refs/remotes/origin/HEAD", "refs/remotes/origin/main", "0", " "),
            record("refs/remotes/upstream/team/main", "", "0", " "),
        ]
        .concat();
        let refs = parse_containing(&output).refs;
        assert_eq!(refs.remote, ["origin/main", "upstream/team/main"]);
        assert!(refs.local.is_empty());
    }

    #[test]
    fn a_branch_whose_name_ends_in_head_is_kept() {
        let output = record("refs/heads/release/HEAD", "", "0", " ");
        assert_eq!(parse_containing(&output).refs.local, ["release/HEAD"]);
    }

    #[test]
    fn first_tag_is_the_earliest_by_date_then_name_and_all_tags_are_counted() {
        let output =
            [record("refs/tags/v2", "", "2000", " "), record("refs/tags/v1-rc", "", "1000", " "), record("refs/tags/v1", "", "1000", " ")]
                .concat();
        let refs = parse_containing(&output).refs;
        assert_eq!(refs.first_tag.as_deref(), Some("v1"), "equal dates fall back to the name");
        assert_eq!(refs.tag_count, 3);
    }

    #[test]
    fn an_annotated_tag_is_ordered_by_its_commits_date() {
        // The annotated tag was made later, but on an older commit than the lightweight one.
        let output = [record("refs/tags/beta", "", "2000", " "), annotated("refs/tags/v1.0", "1000")].concat();
        assert_eq!(parse_containing(&output).refs.first_tag.as_deref(), Some("v1.0"));
    }

    #[test]
    fn no_refs_gives_empty_lists_and_no_tag() {
        assert_eq!(parse_containing("").refs, ContainingRefs::default());
    }
}
