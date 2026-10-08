use std::collections::BTreeMap;
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StatusKind {
    Added,
    Modified,
    Deleted,
    Renamed,
    Conflicted,
    Untracked,
}

impl StatusKind {
    pub fn title(self) -> &'static str {
        match self {
            StatusKind::Added => "Added",
            StatusKind::Modified => "Modified",
            StatusKind::Deleted => "Deleted",
            StatusKind::Renamed => "Renamed",
            StatusKind::Conflicted => "Conflict",
            StatusKind::Untracked => "Untracked",
        }
    }

    /// A one-letter badge for lists.
    pub fn letter(self) -> &'static str {
        match self {
            StatusKind::Added => "A",
            StatusKind::Modified => "M",
            StatusKind::Deleted => "D",
            StatusKind::Renamed => "R",
            StatusKind::Conflicted => "!",
            StatusKind::Untracked => "U",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct StatusEntry {
    pub path: String,
    pub original_path: Option<String>,
    pub kind: StatusKind,
    pub index_status: char,
    pub work_tree_status: char,
}

impl StatusEntry {
    /// Unambiguous identity: Git paths can contain any separator used in display text.
    pub fn id(&self) -> String {
        format!("{}\0{}\0{}{}", self.original_path.as_deref().unwrap_or(""), self.path, self.index_status, self.work_tree_status)
    }

    pub fn file_name(&self) -> &str {
        self.path.rsplit('/').next().unwrap_or(&self.path)
    }

    pub fn is_staged(&self) -> bool {
        self.kind != StatusKind::Conflicted && self.index_status != ' ' && self.index_status != '?'
    }

    pub fn is_unstaged(&self) -> bool {
        self.kind == StatusKind::Conflicted || self.work_tree_status != ' '
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Branch {
    pub name: String,
    pub is_current: bool,
    pub is_remote: bool,
    pub tip: String,
    pub subject: String,
    pub upstream: Option<String>,
}

impl Branch {
    pub fn id(&self) -> String {
        format!("{}\0{}", if self.is_remote { "remote" } else { "local" }, self.name)
    }

    pub fn display_name(&self) -> String {
        // Git lists a detached HEAD as a parenthesised description rather than a branch name.
        if !self.is_remote && self.name.starts_with('(') && self.name.ends_with(')') {
            if let Some(start) = self.name.find("bisect started on ") {
                let rest = &self.name[start + "bisect started on ".len()..self.name.len() - 1];
                return format!("Bisecting from {rest}");
            }
            return format!("Detached HEAD at {}", short(&self.tip));
        }
        self.name.strip_prefix("remotes/").unwrap_or(&self.name).to_string()
    }

    /// The longest configured remote whose name prefixes this branch; remote names can contain slashes.
    pub fn remote_name<'a>(&self, remotes: &'a [String]) -> Option<&'a str> {
        if !self.is_remote {
            return None;
        }
        remotes
            .iter()
            .filter(|remote| self.name.starts_with(&format!("remotes/{remote}/")))
            .max_by_key(|remote| remote.len())
            .map(String::as_str)
    }

    pub fn is_detached(&self) -> bool {
        !self.is_remote && self.name.starts_with('(')
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Commit {
    pub hash: String,
    pub short_hash: String,
    pub parents: Vec<String>,
    pub refs: Vec<String>,
    pub subject: String,
    pub author_name: String,
    pub author_email: String,
    pub relative_date: String,
    pub commit_time: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stash {
    pub hash: String,
    pub reference: String,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Worktree {
    pub path: String,
    pub branch: Option<String>,
    pub is_bare: bool,
    pub is_locked: bool,
    pub is_prunable: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    Merge,
    Rebase,
    CherryPick,
    Revert,
}

impl Operation {
    pub fn name(self) -> &'static str {
        match self {
            Operation::Merge => "merge",
            Operation::Rebase => "rebase",
            Operation::CherryPick => "cherry-pick",
            Operation::Revert => "revert",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResetMode {
    Soft,
    Mixed,
    Hard,
}

impl ResetMode {
    pub fn flag(self) -> &'static str {
        match self {
            ResetMode::Soft => "--soft",
            ResetMode::Mixed => "--mixed",
            ResetMode::Hard => "--hard",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Snapshot {
    pub root_path: String,
    pub name: String,
    pub current_branch: String,
    pub status: Vec<StatusEntry>,
    pub branches: Vec<Branch>,
    pub commits: Vec<Commit>,
    pub remotes: Vec<String>,
    pub remote_fetch_addresses: BTreeMap<String, Vec<String>>,
    pub remote_push_addresses: BTreeMap<String, Vec<String>>,
    pub stashes: Vec<Stash>,
    pub operation: Option<Operation>,
    pub has_more_commits: bool,
    pub tags: Vec<String>,
    pub tag_tips: BTreeMap<String, String>,
    pub upstream: Option<String>,
    pub ahead: Option<usize>,
    pub behind: Option<usize>,
    pub head_hash: Option<String>,
    pub worktrees: Vec<Worktree>,
}

impl Snapshot {
    pub fn staged_count(&self) -> usize {
        self.status.iter().filter(|entry| entry.is_staged()).count()
    }

    pub fn unstaged_count(&self) -> usize {
        self.status.iter().filter(|entry| entry.is_unstaged()).count()
    }

    pub fn current(&self) -> Option<&Branch> {
        self.branches.iter().find(|branch| branch.is_current)
    }

    /// Whether HEAD is on a named branch, rather than detached or unborn.
    pub fn is_on_branch(&self) -> bool {
        self.current().is_some_and(|branch| !branch.is_detached())
    }
}

/// The checkout's branch, HEAD, and unfinished operation, read together so an action can
/// confirm it still applies to the checkout it was chosen for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckoutState {
    pub current_branch: String,
    pub head_hash: Option<String>,
    pub operation: Option<Operation>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GitError {
    CommandFailed { command: String, message: String },
    EmptyBranchName,
    EmptyCommitMessage,
    GitNotFound,
}

impl GitError {
    pub fn failed(command: impl Into<String>, message: impl Into<String>) -> Self {
        GitError::CommandFailed { command: command.into(), message: message.into() }
    }
}

impl fmt::Display for GitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GitError::CommandFailed { command, message } => write!(f, "Git could not run `{command}`: {message}"),
            GitError::EmptyBranchName => write!(f, "Branch name cannot be empty."),
            GitError::EmptyCommitMessage => write!(f, "Commit message cannot be empty."),
            GitError::GitNotFound => write!(f, "Git was not found. Install Git and make sure it is on your PATH."),
        }
    }
}

impl std::error::Error for GitError {}

pub type Result<T> = std::result::Result<T, GitError>;

pub fn short(hash: &str) -> &str {
    &hash[..hash.len().min(7)]
}
