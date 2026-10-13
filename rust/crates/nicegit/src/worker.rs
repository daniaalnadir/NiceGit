use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};

use nicegit_core::diff::{parse_diff, DiffLine};
use nicegit_core::{GitClient, GitError, HistoryScope, Snapshot, Stash, StashFile};

/// What a background action reports when it finishes.
pub type ActionResult = Result<Option<String>, GitError>;

pub enum Message {
    /// A repository load, optionally after an action. The action's own result is reported
    /// separately from the refresh so a completed change is never presented as a failure.
    Loaded {
        generation: u64,
        path: PathBuf,
        action: Option<(String, ActionResult)>,
        snapshot: Box<Result<Snapshot, GitError>>,
    },
    Diff {
        generation: u64,
        title: String,
        result: Result<Vec<DiffLine>, GitError>,
    },
    CommitFiles {
        generation: u64,
        hash: String,
        result: Result<Vec<(String, String)>, GitError>,
    },
    /// A stash's files. Matched by repository and stash rather than generation, so a diff that
    /// starts meanwhile does not discard the list.
    StashFiles {
        path: PathBuf,
        hash: String,
        result: Result<Vec<StashFile>, GitError>,
    },
}

/// Runs Git work off the interface thread and wakes the interface when it is done.
pub struct Worker {
    sender: Sender<Message>,
    pub receiver: Receiver<Message>,
    pub context: egui::Context,
}

impl Worker {
    pub fn new(context: egui::Context) -> Self {
        let (sender, receiver) = channel();
        Self { sender, receiver, context }
    }

    fn spawn(&self, work: impl FnOnce() -> Message + Send + 'static) {
        let sender = self.sender.clone();
        let context = self.context.clone();
        std::thread::spawn(move || {
            let _ = sender.send(work());
            context.request_repaint();
        });
    }

    /// `hidden` names the branches the graph leaves out, as full ref names (see `load_snapshot_filtered`).
    pub fn load(&self, generation: u64, path: PathBuf, deliberate: bool, history_limit: usize, scope: HistoryScope, hidden: Vec<String>) {
        self.spawn(move || {
            let client = if deliberate { GitClient::deliberate() } else { GitClient::new() };
            let snapshot = client.load_snapshot_filtered(&path, history_limit, scope, &hidden);
            Message::Loaded { generation, path, action: None, snapshot: Box::new(snapshot) }
        });
    }

    /// Refreshes the working-tree status of `previous` and keeps the rest, when the checkout is
    /// unchanged (see `GitClient::quick_snapshot`). Otherwise, and when the quick read fails, it
    /// loads everything; a real failure then surfaces from that full load.
    pub fn quick_load(
        &self,
        generation: u64,
        path: PathBuf,
        previous: Snapshot,
        history_limit: usize,
        scope: HistoryScope,
        hidden: Vec<String>,
    ) {
        self.spawn(move || {
            let client = GitClient::new();
            let quick = client.quick_snapshot(&previous, &path).ok().flatten();
            let snapshot = match quick {
                Some(snapshot) => Ok(snapshot),
                None => client.load_snapshot_filtered(&path, history_limit, scope, &hidden),
            };
            Message::Loaded { generation, path, action: None, snapshot: Box::new(snapshot) }
        });
    }

    /// Runs an action that changes the repository, then reloads it.
    #[allow(clippy::too_many_arguments)]
    pub fn act(
        &self,
        generation: u64,
        path: PathBuf,
        label: String,
        history_limit: usize,
        scope: HistoryScope,
        hidden: Vec<String>,
        action: impl FnOnce(&GitClient, &Path) -> ActionResult + Send + 'static,
    ) {
        self.spawn(move || {
            let client = GitClient::new();
            let result = action(&client, &path);
            let snapshot = client.load_snapshot_filtered(&path, history_limit, scope, &hidden);
            Message::Loaded { generation, path, action: Some((label, result)), snapshot: Box::new(snapshot) }
        });
    }

    pub fn diff(&self, generation: u64, title: String, load: impl FnOnce(&GitClient) -> Result<String, GitError> + Send + 'static) {
        self.spawn(move || {
            let result = load(&GitClient::new()).map(|patch| parse_diff(&patch));
            Message::Diff { generation, title, result }
        });
    }

    pub fn commit_files(&self, generation: u64, path: PathBuf, hash: String) {
        self.spawn(move || {
            let result = GitClient::new().commit_files(&hash, &path);
            Message::CommitFiles { generation, hash, result }
        });
    }

    pub fn stash_files(&self, path: PathBuf, stash: Stash) {
        self.spawn(move || {
            let result = GitClient::new().stash_files(&stash, &path);
            Message::StashFiles { path, hash: stash.hash, result }
        });
    }
}
