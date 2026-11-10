use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};

use nicegit_core::diff::{parse_diff, DiffLine};
use nicegit_core::{GitClient, GitError, HistoryFilter, HistoryScope, Snapshot, Stash, StashFile};

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
    /// A file saved from a commit. The result is the saved file's name.
    FileSaved {
        result: Result<String, GitError>,
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

    /// `hidden` names the branches the graph leaves out, as full ref names, `solo` the one it
    /// shows alone (see `load_snapshot_solo`), and `filter` narrows the history (see
    /// `load_snapshot_with_filter`).
    #[allow(clippy::too_many_arguments)]
    pub fn load(
        &self,
        generation: u64,
        path: PathBuf,
        deliberate: bool,
        history_limit: usize,
        scope: HistoryScope,
        hidden: Vec<String>,
        solo: Option<String>,
        filter: HistoryFilter,
    ) {
        self.spawn(move || {
            let client = if deliberate { GitClient::deliberate() } else { GitClient::new() };
            let snapshot = client.load_snapshot_with_filter(&path, history_limit, scope, &hidden, solo.as_deref(), &filter);
            Message::Loaded { generation, path, action: None, snapshot: Box::new(snapshot) }
        });
    }

    /// Refreshes the working-tree status of `previous` and keeps the rest, when the checkout is
    /// unchanged (see `GitClient::quick_snapshot`). Otherwise, and when the quick read fails, it
    /// loads everything; a real failure then surfaces from that full load.
    #[allow(clippy::too_many_arguments)]
    pub fn quick_load(
        &self,
        generation: u64,
        path: PathBuf,
        previous: Snapshot,
        history_limit: usize,
        scope: HistoryScope,
        hidden: Vec<String>,
        solo: Option<String>,
        filter: HistoryFilter,
    ) {
        self.spawn(move || {
            let client = GitClient::new();
            let quick = client.quick_snapshot(&previous, &path).ok().flatten();
            let snapshot = match quick {
                Some(snapshot) => Ok(snapshot),
                None => client.load_snapshot_with_filter(&path, history_limit, scope, &hidden, solo.as_deref(), &filter),
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
        solo: Option<String>,
        filter: HistoryFilter,
        action: impl FnOnce(&GitClient, &Path) -> ActionResult + Send + 'static,
    ) {
        self.spawn(move || {
            let client = GitClient::new();
            let result = action(&client, &path);
            let snapshot = client.load_snapshot_with_filter(&path, history_limit, scope, &hidden, solo.as_deref(), &filter);
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

    /// Writes a file as it was at a commit to `destination`.
    pub fn save_copy(&self, directory: PathBuf, commit: String, file: String, destination: PathBuf) {
        self.spawn(move || Message::FileSaved { result: save_copy(&directory, &commit, &file, &destination) });
    }
}

/// Writes the contents a file had at a commit, returning the saved file's name. A file the commit
/// does not hold, such as one it deleted, has nothing to save.
fn save_copy(directory: &Path, commit: &str, file: &str, destination: &Path) -> Result<String, GitError> {
    let bytes = GitClient::new()
        .file_bytes_at(Some(commit), file, directory)?
        .ok_or_else(|| GitError::failed("save a copy", "This file is not in that commit."))?;
    std::fs::write(destination, bytes).map_err(|error| GitError::failed("save a copy", error.to_string()))?;
    Ok(destination.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default())
}
