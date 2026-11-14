//! Reads the repository NiceGit will show first while the window is still being created.
//!
//! Creating the window and its renderer takes longer than reading a repository, so the read
//! starts at the top of `main`, before eframe runs. When the app then asks for the same load, it
//! takes the result instead of starting Git again, and the first frame can show the repository.
//!
//! What to read comes from a small hint file NiceGit writes next to its settings whenever it saves
//! them: the active repository and the graph's branch choices for it. A folder passed on the
//! command line is read instead, with the same choices. If the hint is missing or stale, the app
//! simply loads as it always has.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};
use std::sync::{Mutex, OnceLock};

use nicegit_core::{GitClient, GitError, HistoryFilter, Snapshot};
use serde::{Deserialize, Serialize};

use crate::settings::HistoryScope;

/// The file next to NiceGit's settings that names what to read at the next launch.
const HINT_FILE: &str = "startup-hint.ron";
/// eframe's application ID, which names the settings folder.
const APP_ID: &str = "nicegit";

/// Everything that decides what a first load reads. A load only uses a preload whose key matches.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Key {
    pub path: PathBuf,
    pub limit: usize,
    pub scope: HistoryScope,
    pub hidden: Vec<String>,
    pub solo: Option<String>,
}

struct Pending {
    key: Key,
    result: Receiver<Result<Snapshot, GitError>>,
}

static PENDING: OnceLock<Mutex<Option<Pending>>> = OnceLock::new();

fn hint_path() -> Option<PathBuf> {
    eframe::storage_dir(APP_ID).map(|dir| dir.join(HINT_FILE))
}

/// Starts reading the repository the next launch will show. `command_line` is a folder passed to
/// NiceGit, which opens instead of the saved active repository.
pub fn start(command_line: Option<&Path>) {
    let Some(mut key) = hint_path().and_then(|path| std::fs::read_to_string(path).ok()).and_then(|text| ron::from_str::<Key>(&text).ok())
    else {
        return;
    };
    if let Some(folder) = command_line {
        // A folder from the command line opens in a new tab with the default branch choices only
        // when it is the hinted repository; otherwise its own choices are unknown here.
        let canonical = folder.canonicalize().map(crate::app::strip_verbatim).unwrap_or_else(|_| folder.to_path_buf());
        if canonical != key.path {
            return;
        }
        key.path = canonical;
    }
    if !key.path.join(".git").exists() && !key.path.join("HEAD").exists() {
        return;
    }
    let (sender, result) = channel();
    let job = key.clone();
    std::thread::spawn(move || {
        let started = std::time::Instant::now();
        let scope = nicegit_core::HistoryScope::from(job.scope);
        let snapshot = GitClient::deliberate().load_snapshot_with_filter(
            &job.path,
            job.limit,
            scope,
            &job.hidden,
            job.solo.as_deref(),
            &HistoryFilter::default(),
        );
        crate::timing::report_since("preload: snapshot read during window creation", started);
        let _ = sender.send(snapshot);
    });
    *PENDING.get_or_init(|| Mutex::new(None)).lock().unwrap() = Some(Pending { key, result });
}

/// Takes the preload when it read exactly `key`. Any other first load discards it.
pub fn take(key: &Key) -> Option<Receiver<Result<Snapshot, GitError>>> {
    let pending = PENDING.get()?.lock().ok()?.take()?;
    (pending.key == *key).then_some(pending.result)
}

/// Remembers what the next launch should read first. Tests never call it: they must not write
/// into the person's settings folder.
#[cfg_attr(test, allow(dead_code))]
pub fn save_hint(key: Option<&Key>) {
    let Some(path) = hint_path() else { return };
    match key {
        Some(key) => {
            if let Ok(text) = ron::to_string(key) {
                let _ = std::fs::write(path, text);
            }
        }
        None => {
            let _ = std::fs::remove_file(path);
        }
    }
}
