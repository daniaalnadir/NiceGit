//! Stacked branches: local branches built on each other, such as `feature-b` on `feature-a`. When
//! a lower branch is amended or rebased, the branches above it still hold its old commits, so
//! they are restacked: their own commits are replayed onto the new tip. Everything is done with
//! plumbing commands, so the working tree and index change only when the checked-out branch moves.

use std::collections::{BTreeMap, BTreeSet, HashSet, VecDeque};
use std::path::Path;

use crate::client::GitClient;
use crate::models::{Branch, GitError, Result};
use crate::parsers::{parse_branches, BRANCH_FORMAT};
use crate::rebase::Scratch;
use crate::undo::RefMove;

const COMMAND: &str = "restack";

/// The most local branches a stack read considers. Longer lists are cut so reads stay cheap.
const MAX_BRANCHES: usize = 50;
/// The most commits a branch may be ahead of the base for it to be part of a stack.
const MAX_DISTANCE: usize = 200;
/// How many reflog entries of a branch are read for its earlier tips.
const REFLOG_ENTRIES: usize = 20;

/// A local branch in a stack, with the branch below it when there is one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StackBranch {
    pub name: String,
    pub tip: String,
    /// The nearest branch below this one in the stack. `None` for the bottom of the stack.
    pub parent: Option<String>,
}

/// Two or more branches built on each other, bottom first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stack {
    pub branches: Vec<StackBranch>,
}

/// A local branch still based on an earlier tip of its parent, so it must be restacked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutdatedChild {
    pub name: String,
    /// The branch's tip when it was read. A restack refuses if the branch has moved since.
    pub tip: String,
    /// The earlier tip of the parent that the branch is based on.
    pub old_base: String,
    /// How many commits the branch has above `old_base`.
    pub commits: usize,
}

/// One branch of a stack push: where it goes, and whether the push rewrites the remote branch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StackPush {
    pub branch: String,
    pub remote: String,
    /// The full ref on the remote, such as `refs/heads/feature-b`.
    pub destination: String,
    /// The local tip being pushed.
    pub tip: String,
    /// The remote-tracking tip that was read, or `None` when the remote branch is new.
    pub remote_tip: Option<String>,
    /// True when the remote branch is not an ancestor of the tip, so it is replaced under a lease.
    pub forced: bool,
}

/// The local branches a stack read works on, and the base they are stacked on.
struct Layout {
    branches: Vec<Branch>,
    base: String,
}

/// Lists the stacks among the local branches. A branch is stacked on another when the other's tip
/// lies between the base branch and its own tip, and it is stacked directly on the nearest such
/// branch. Stacks have two or more branches. Without a base branch, there are none.
pub fn find_stacks(client: &GitClient, directory: &Path) -> Result<Vec<Stack>> {
    let Some(layout) = layout(client, directory)? else { return Ok(Vec::new()) };
    let base_tip = branch_tip(client, &layout.base, directory)?;
    let mut ranges: BTreeMap<String, HashSet<String>> = BTreeMap::new();
    let mut tips: BTreeMap<String, String> = BTreeMap::new();
    for branch in layout.branches.iter().filter(|branch| branch.name != layout.base).take(MAX_BRANCHES) {
        if branch.tip == base_tip {
            continue;
        }
        let between = commits_between(client, &base_tip, &branch.tip, directory)?;
        if between.is_empty() || between.len() > MAX_DISTANCE {
            continue;
        }
        ranges.insert(branch.name.clone(), between.into_iter().collect());
        tips.insert(branch.name.clone(), branch.tip.clone());
    }

    let mut parents: BTreeMap<String, Option<String>> = BTreeMap::new();
    for name in ranges.keys() {
        // The candidates lying between the base and this branch.
        let below: Vec<&String> = tips
            .iter()
            .filter(|(other, other_tip)| *other != name && ranges[name.as_str()].contains(other_tip.as_str()))
            .map(|(other, _)| other)
            .collect();
        // The nearest is one that no other candidate below this branch lies above.
        let nearest = below.iter().find(|candidate| {
            let candidate_tip = tips[candidate.as_str()].as_str();
            !below.iter().any(|other| other != *candidate && ranges[other.as_str()].contains(candidate_tip))
        });
        parents.insert(name.clone(), nearest.map(|parent| parent.to_string()));
    }

    Ok(build_stacks(&tips, &parents))
}

/// Groups branches into stacks: each tree rooted at a branch with no parent that has children.
fn build_stacks(tips: &BTreeMap<String, String>, parents: &BTreeMap<String, Option<String>>) -> Vec<Stack> {
    let mut children: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (name, parent) in parents {
        if let Some(parent) = parent {
            children.entry(parent.as_str()).or_default().push(name.as_str());
        }
    }
    let mut stacks = Vec::new();
    for (name, parent) in parents {
        if parent.is_some() || !children.contains_key(name.as_str()) {
            continue;
        }
        let mut branches = Vec::new();
        let mut pending = VecDeque::from([name.as_str()]);
        while let Some(current) = pending.pop_front() {
            branches.push(StackBranch {
                name: current.to_string(),
                tip: tips[current].clone(),
                parent: parents.get(current).cloned().flatten(),
            });
            for child in children.get(current).into_iter().flatten() {
                pending.push_back(child);
            }
        }
        stacks.push(Stack { branches });
    }
    stacks
}

/// The local branches that are behind `name`'s own commits: those whose tips an earlier tip of
/// `name` lies under. `name`'s earlier tips come from its reflog; each branch found is based on
/// the newest of them it contains. A branch with no commits beyond that tip, or one that is an
/// ancestor of `name` itself (such as the base), is left out, and so is a branch built on another
/// branch in the list, which moves with that one.
pub fn outdated_children(client: &GitClient, branch: &str, directory: &Path) -> Result<Vec<OutdatedChild>> {
    let reference = format!("refs/heads/{branch}");
    let tip = client.run_trimmed(&["rev-parse", "--verify", "--end-of-options", &reference], directory)?;
    // Only a tip the branch has since left behind (an amend, rebase or reset) can leave a branch
    // on an old base; an earlier tip still in the branch's history only means it gained commits.
    let previous: Vec<String> = earlier_tips(client, &reference, &tip, directory)?
        .into_iter()
        .filter(|old| client.run(&["merge-base", "--is-ancestor", old, &tip], directory).is_err())
        .collect();
    if previous.is_empty() {
        return Ok(Vec::new());
    }
    let current_containers: HashSet<String> = refs_containing(client, &tip, directory)?.into_iter().map(|(name, _)| name).collect();
    let mut found: BTreeMap<String, OutdatedChild> = BTreeMap::new();
    for old in &previous {
        for (name, child_tip) in refs_containing(client, old, directory)? {
            let short = short_name(&name).to_string();
            if name == reference || current_containers.contains(&name) || found.contains_key(&short) || child_tip == *old {
                continue;
            }
            if client.run(&["merge-base", "--is-ancestor", &child_tip, &tip], directory).is_ok() {
                continue;
            }
            let commits = count_commits(client, &format!("{old}..{child_tip}"), directory)?;
            if commits == 0 {
                continue;
            }
            found.insert(short.clone(), OutdatedChild { name: short, tip: child_tip, old_base: old.clone(), commits });
        }
    }
    // A branch built on another outdated branch moves with that one, so only the lowest are listed.
    let mut moving_with = HashSet::new();
    for child in found.values() {
        for (name, _) in refs_containing(client, &child.tip, directory)? {
            let short = short_name(&name);
            if short != child.name && found.contains_key(short) {
                moving_with.insert(short.to_string());
            }
        }
    }
    found.retain(|name, _| !moving_with.contains(name));
    Ok(found.into_values().collect())
}

/// Restacks `children` of `parent` onto the parent's tip `parent_tip`: each child's own commits are
/// replayed onto the new tip, then the branches built on each child's old tip move onto the child's
/// new tip, and so on down the stack. Nothing is changed unless every replay succeeds, and every
/// captured tip is still current. A checked-out branch must have no staged or unstaged changes to
/// tracked files, and it moves with `reset --keep`. Returns the branches that moved.
pub fn restack(client: &GitClient, parent: &str, parent_tip: &str, children: &[OutdatedChild], directory: &Path) -> Result<Vec<RefMove>> {
    let state = client.checkout_state(directory)?;
    if state.operation.is_some() {
        return Err(GitError::failed(COMMAND, "Finish or abort the current Git operation first."));
    }
    client.require_branch_tip(parent, parent_tip, directory)?;
    let plan = plan_restack(client, parent, parent_tip, children, directory)?;
    if plan.is_empty() {
        return Ok(Vec::new());
    }
    if plan.iter().any(|moved| moved.branch == state.current_branch) {
        let changes = client.run_trimmed(&["status", "--porcelain=v1", "--untracked-files=no"], directory)?;
        if !changes.is_empty() {
            return Err(GitError::failed(
                COMMAND,
                format!("Commit or stash the changes on {} before restacking it.", state.current_branch),
            ));
        }
    }
    client.move_branch_refs(&plan, directory)?;
    Ok(plan)
}

/// The moves a restack makes, computed without changing any ref. Replays fail here, before anything moves.
fn plan_restack(client: &GitClient, parent: &str, parent_tip: &str, children: &[OutdatedChild], directory: &Path) -> Result<Vec<RefMove>> {
    struct Pending {
        name: String,
        before: String,
        old_base: String,
        new_base: String,
    }
    let mut queue: VecDeque<Pending> = children
        .iter()
        .map(|child| Pending {
            name: child.name.clone(),
            before: child.tip.clone(),
            old_base: child.old_base.clone(),
            new_base: parent_tip.to_string(),
        })
        .collect();
    let mut plan: Vec<RefMove> = Vec::new();
    let mut planned: HashSet<String> = HashSet::from([parent.to_string()]);
    while let Some(item) = queue.pop_front() {
        if !planned.insert(item.name.clone()) {
            continue;
        }
        client.require_branch_tip(&item.name, &item.before, directory)?;
        let after = replay(client, &item.name, &item.old_base, &item.before, &item.new_base, directory)?;
        for (name, tip) in refs_containing(client, &item.before, directory)? {
            let short = short_name(&name).to_string();
            if tip == item.before || planned.contains(&short) {
                continue;
            }
            queue.push_back(Pending { name: short, before: tip, old_base: item.before.clone(), new_base: after.clone() });
        }
        plan.push(RefMove { branch: item.name, before: item.before, after });
    }
    Ok(plan)
}

/// Replays the commits of `old_base..tip` onto `onto`, each keeping its author, date and message,
/// and returns the new tip. A merge commit or a conflict stops it, naming the branch and commit.
fn replay(client: &GitClient, branch: &str, old_base: &str, tip: &str, onto: &str, directory: &Path) -> Result<String> {
    let range = format!("{old_base}..{tip}");
    if !client.run_trimmed(&["rev-list", "--min-parents=2", &range, "--"], directory)?.is_empty() {
        return Err(GitError::failed(COMMAND, format!("{branch} includes a merge commit, so it cannot be restacked. Rebase it by hand.")));
    }
    let commits: Vec<String> =
        client.run_trimmed(&["rev-list", "--reverse", "--topo-order", &range, "--"], directory)?.lines().map(str::to_string).collect();
    let sign = client.signing_setup(directory)?.sign_by_default;
    let scratch = Scratch::new(COMMAND)?;
    let mut head = onto.to_string();
    for (number, commit) in commits.iter().enumerate() {
        let base = client.run_trimmed(&["rev-parse", "--verify", &format!("{commit}^")], directory)?;
        let merge_base = format!("--merge-base={base}");
        let conflict = || {
            let subject = client.try_trimmed(&["show", "-s", "--format=%s", commit, "--"], directory).unwrap_or_default();
            GitError::failed(
                COMMAND,
                format!("Restacking {branch} would conflict at {} {subject}; rebase it by hand", &commit[..commit.len().min(7)]),
            )
        };
        let tree = client
            .run(&["merge-tree", "--write-tree", &merge_base, &head, commit], directory)
            .map_err(|_| conflict())?
            .lines()
            .next()
            .unwrap_or_default()
            .trim()
            .to_string();
        let author = client.author_of(commit, directory)?;
        let message = client.raw_commit_message(commit, directory)?;
        head = client.commit_tree_as(&scratch, number, &tree, &[head.as_str()], &message, &author, sign, directory)?;
    }
    Ok(head)
}

/// Pushes the branches of a stack that have an upstream, each to its upstream branch. A branch
/// whose remote branch is not an ancestor of its tip is pushed under a lease, which the remote
/// tip read here must still satisfy. Branches without an upstream are left out.
pub fn plan_stack_push(client: &GitClient, branches: &[String], directory: &Path) -> Result<Vec<StackPush>> {
    let mut pushes = Vec::new();
    for name in branches {
        let tip = client.run_trimmed(&["rev-parse", "--verify", "--end-of-options", &format!("refs/heads/{name}")], directory)?;
        let remote = client.config(&format!("branch.{name}.remote"), false, directory)?.unwrap_or_default();
        let merge = client.config(&format!("branch.{name}.merge"), false, directory)?.unwrap_or_default();
        if remote.is_empty() || remote == "." || !merge.starts_with("refs/heads/") {
            continue;
        }
        let destination = merge;
        let short = destination.strip_prefix("refs/heads/").unwrap_or(&destination).to_string();
        let remote_tip = client
            .try_trimmed(&["rev-parse", "--verify", "--quiet", "--end-of-options", &format!("refs/remotes/{remote}/{short}")], directory)
            .filter(|value| value.len() >= 40);
        let forced = match &remote_tip {
            Some(remote_tip) => client.run(&["merge-base", "--is-ancestor", remote_tip, &tip], directory).is_err(),
            None => false,
        };
        pushes.push(StackPush { branch: name.clone(), remote, destination, tip, remote_tip, forced });
    }
    Ok(pushes)
}

/// Pushes each planned branch with an explicit refspec, in one atomic push per remote. Rewritten
/// branches are sent under a lease on the remote tip read by `plan_stack_push`. Every local tip, the
/// remote tips, and the remote's push addresses must still be as planned. If the remote does not
/// support atomic pushes, the branches are pushed one at a time.
pub fn push_stack(
    client: &GitClient,
    pushes: &[StackPush],
    expected_push_addresses: &BTreeMap<String, Vec<String>>,
    directory: &Path,
) -> Result<()> {
    if pushes.is_empty() {
        return Ok(());
    }
    for push in pushes {
        client.require_branch_tip(&push.branch, &push.tip, directory)?;
        let short = push.destination.strip_prefix("refs/heads/").unwrap_or(&push.destination);
        let current =
            client.try_trimmed(&["rev-parse", "--verify", "--quiet", &format!("refs/remotes/{}/{short}", push.remote)], directory);
        if current != push.remote_tip {
            return Err(GitError::failed(
                COMMAND,
                format!("The remote branch {} changed since it was shown. Review the push again.", push.branch),
            ));
        }
    }
    let remotes: BTreeSet<&str> = pushes.iter().map(|push| push.remote.as_str()).collect();
    for remote in remotes {
        client.require_remote_addresses(expected_push_addresses, remote, true, COMMAND, directory)?;
        let group: Vec<&StackPush> = pushes.iter().filter(|push| push.remote == remote).collect();
        push_group(client, remote, &group, directory)?;
    }
    Ok(())
}

/// Pushes a group of branches to one remote, atomically when the remote supports it.
fn push_group(client: &GitClient, remote: &str, group: &[&StackPush], directory: &Path) -> Result<()> {
    let mirror = format!("remote.{remote}.mirror=false");
    let leases: Vec<String> = group
        .iter()
        .filter(|push| push.forced)
        .filter_map(|push| push.remote_tip.as_ref().map(|tip| format!("--force-with-lease={}:{tip}", push.destination)))
        .collect();
    let refspecs: Vec<String> = group.iter().map(|push| format!("{}:{}", push.tip, push.destination)).collect();
    let mut arguments: Vec<&str> = vec!["-c", &mirror, "push", "--no-follow-tags", "--recurse-submodules=no", "--atomic"];
    arguments.extend(leases.iter().map(String::as_str));
    arguments.extend(["--", remote]);
    arguments.extend(refspecs.iter().map(String::as_str));
    match client.run(&arguments, directory) {
        Ok(_) => Ok(()),
        Err(error) if error.to_string().contains("does not support") => {
            // The remote cannot push atomically, so each branch is sent on its own.
            for push in group {
                let lease = if push.forced {
                    push.remote_tip.as_ref().map(|tip| format!("--force-with-lease={}:{tip}", push.destination))
                } else {
                    None
                };
                let refspec = format!("{}:{}", push.tip, push.destination);
                let mut single: Vec<&str> = vec!["-c", &mirror, "push", "--no-follow-tags", "--recurse-submodules=no"];
                single.extend(lease.as_deref());
                single.extend(["--", remote, refspec.as_str()]);
                client.run(&single, directory)?;
            }
            Ok(())
        }
        Err(error) => Err(error),
    }
}

// MARK: Reading the repository

/// The local branches and the base they are stacked on, or none when there is no base branch.
fn layout(client: &GitClient, directory: &Path) -> Result<Option<Layout>> {
    let listing = client.run(&["for-each-ref", &format!("--format={BRANCH_FORMAT}"), "refs/heads/"], directory)?;
    let branches: Vec<Branch> = parse_branches(&listing).into_iter().filter(|branch| !branch.is_remote).collect();
    let remote_head = client
        .try_trimmed(&["symbolic-ref", "--quiet", "refs/remotes/origin/HEAD"], directory)
        .and_then(|reference| reference.strip_prefix("refs/remotes/origin/").map(str::to_string));
    let base = remote_head
        .into_iter()
        .chain(["main".to_string(), "master".to_string()])
        .find(|name| branches.iter().any(|branch| branch.name == *name));
    Ok(base.map(|base| Layout { branches, base }))
}

/// The commit a local branch points to.
fn branch_tip(client: &GitClient, branch: &str, directory: &Path) -> Result<String> {
    client.run_trimmed(&["rev-parse", "--verify", "--end-of-options", &format!("refs/heads/{branch}")], directory)
}

/// The commits in `base..tip`, newest first.
fn commits_between(client: &GitClient, base: &str, tip: &str, directory: &Path) -> Result<Vec<String>> {
    let range = format!("{base}..{tip}");
    Ok(client.run_trimmed(&["rev-list", &range, "--"], directory)?.lines().map(str::to_string).collect())
}

/// The number of commits in `range`.
fn count_commits(client: &GitClient, range: &str, directory: &Path) -> Result<usize> {
    client
        .run_trimmed(&["rev-list", "--count", range, "--"], directory)?
        .parse()
        .map_err(|_| GitError::failed(COMMAND, "Git returned a commit count in an unexpected format."))
}

/// The branches (full names and tips) whose history contains `commit`.
fn refs_containing(client: &GitClient, commit: &str, directory: &Path) -> Result<Vec<(String, String)>> {
    let listing = client.run(&["for-each-ref", "--contains", commit, "--format=%(refname)%09%(objectname)", "refs/heads/"], directory)?;
    Ok(listing.lines().filter_map(|line| line.split_once('\t')).map(|(name, tip)| (name.to_string(), tip.to_string())).collect())
}

/// A branch's name without its `refs/heads/` prefix.
fn short_name(reference: &str) -> &str {
    reference.strip_prefix("refs/heads/").unwrap_or(reference)
}

/// The distinct earlier tips of a branch, newest first, from its reflog. The current tip is left out.
fn earlier_tips(client: &GitClient, reference: &str, tip: &str, directory: &Path) -> Result<Vec<String>> {
    let count = REFLOG_ENTRIES.to_string();
    let listing = client.run(&["reflog", "show", "--format=%H", "-n", &count, reference, "--"], directory)?;
    let mut seen = HashSet::new();
    Ok(listing
        .lines()
        .map(str::trim)
        .filter(|hash| hash.len() >= 40 && *hash != tip && seen.insert(hash.to_string()))
        .map(str::to_string)
        .collect())
}
