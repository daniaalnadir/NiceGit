//! Quiet snapshots of the working files, taken by the app: in the background while the active
//! repository's files change, and just before an action that rewrites them. The snapshots are
//! kept in the repository (see `nicegit_core::snapshots`), and old ones are pruned after each.

use std::path::Path;
use std::time::Duration;

use nicegit_core::snapshots::{prune_snapshots, take_snapshot, DEFAULT_MAX_FILE_BYTES};
use nicegit_core::GitClient;

/// The most snapshots kept in a repository, whatever their age.
pub const KEEP_COUNT: usize = 100;

/// How long files must stay unchanged before a snapshot is taken, so one caught mid-save is not
/// saved half-written.
pub const SETTLE: Duration = Duration::from_secs(3);

/// How often a repository whose changes are waiting is checked while the app is busy.
pub const RETRY: Duration = Duration::from_secs(5);

/// Saves the working files and prunes old snapshots, as the background timer does. A failure is
/// ignored: the next save tries again.
pub fn save_and_prune(client: &GitClient, directory: &Path, keep_days: u64) {
    let _ = take_snapshot(client, directory, DEFAULT_MAX_FILE_BYTES);
    let _ = prune_snapshots(client, directory, KEEP_COUNT, keep_days);
}

/// Wraps an action that can overwrite or remove working files, so the files are saved first. The
/// save runs on the worker thread just before `action`, and `keep_days` is `None` when snapshots
/// are off or the repository has no working files. A failed save does not stop the action.
pub fn before_rewrite<R>(
    keep_days: Option<u64>,
    action: impl FnOnce(&GitClient, &Path) -> R + Send + 'static,
) -> impl FnOnce(&GitClient, &Path) -> R + Send + 'static {
    move |client, directory| {
        if let Some(keep_days) = keep_days {
            save_and_prune(client, directory, keep_days);
        }
        action(client, directory)
    }
}
