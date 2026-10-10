use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};

use nicegit_core::diff::{parse_diff, DiffLine};
use nicegit_core::{GitClient, GitError, Snapshot};

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

    pub fn load(&self, generation: u64, path: PathBuf, deliberate: bool, history_limit: usize) {
        self.spawn(move || {
            let client = if deliberate { GitClient::deliberate() } else { GitClient::new() };
            let snapshot = client.load_snapshot(&path, history_limit);
            Message::Loaded { generation, path, action: None, snapshot: Box::new(snapshot) }
        });
    }

    /// Runs an action that changes the repository, then reloads it.
    pub fn act(
        &self,
        generation: u64,
        path: PathBuf,
        label: String,
        history_limit: usize,
        action: impl FnOnce(&GitClient, &Path) -> ActionResult + Send + 'static,
    ) {
        self.spawn(move || {
            let client = GitClient::new();
            let result = action(&client, &path);
            let snapshot = client.load_snapshot(&path, history_limit);
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
}
