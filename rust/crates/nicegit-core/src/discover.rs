//! Finds the Git repositories inside a folder, so many projects can be added at once.

use std::collections::VecDeque;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// Folder names never searched: dependency, build, and cache folders that hold other projects'
/// files rather than repositories the person keeps.
const SKIPPED: &[&str] =
    &["node_modules", "target", "vendor", "build", "dist", ".build", "Pods", "DerivedData", "venv", ".venv", "__pycache__"];

/// Finds the repositories inside `root`, searching at most `max_depth` folders below it and
/// returning at most `limit` of them, sorted.
///
/// A folder is a repository when it has a `.git` folder or file. The search does not go into a
/// repository it finds, so a repository nested in another is not listed. Hidden folders, the
/// folders in `SKIPPED`, and symbolic links are never searched, and folders that cannot be read
/// are passed over. When `root` is itself a repository, it is the only result. The search stops
/// early once `cancel` is set, and then returns what it has found so far.
pub fn find_repositories(root: &Path, max_depth: usize, limit: usize, cancel: &AtomicBool) -> Vec<PathBuf> {
    let root = strip_verbatim(root.to_path_buf());
    if is_repository(&root) {
        return vec![root];
    }
    let mut found = Vec::new();
    let mut queue = VecDeque::from([(root, 0usize)]);
    while let Some((folder, depth)) = queue.pop_front() {
        if cancel.load(Ordering::Relaxed) || found.len() >= limit {
            break;
        }
        let Ok(entries) = fs::read_dir(&folder) else { continue };
        for entry in entries.flatten() {
            // `file_type` does not follow symbolic links, so a link is never searched and cannot loop.
            if !entry.file_type().is_ok_and(|kind| kind.is_dir()) || is_skipped(&entry.file_name()) {
                continue;
            }
            let path = entry.path();
            if is_repository(&path) {
                found.push(path);
                if found.len() >= limit {
                    break;
                }
            } else if depth + 1 < max_depth {
                queue.push_back((path, depth + 1));
            }
        }
    }
    found.sort();
    found
}

fn is_repository(folder: &Path) -> bool {
    fs::symlink_metadata(folder.join(".git")).is_ok_and(|meta| meta.is_dir() || meta.is_file())
}

fn is_skipped(name: &OsStr) -> bool {
    let Some(name) = name.to_str() else { return false };
    name.starts_with('.') || SKIPPED.contains(&name)
}

/// Removes the `\\?\` prefix Windows adds to canonical paths, so the results read as the person
/// would type them. UNC paths keep theirs, as they cannot be written without it.
fn strip_verbatim(path: PathBuf) -> PathBuf {
    #[cfg(windows)]
    {
        if let Some(rest) = path.to_str().and_then(|text| text.strip_prefix(r"\\?\")) {
            if !rest.starts_with("UNC") {
                return PathBuf::from(rest);
            }
        }
    }
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    fn never() -> AtomicBool {
        AtomicBool::new(false)
    }

    /// A folder with a `.git` folder, as a repository has.
    fn repo(path: &Path) {
        fs::create_dir_all(path.join(".git")).unwrap();
    }

    fn relative(root: &Path, found: Vec<PathBuf>) -> Vec<PathBuf> {
        found.into_iter().map(|path| path.strip_prefix(root).unwrap().to_path_buf()).collect()
    }

    #[test]
    fn finds_repositories_at_several_depths_sorted() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        repo(&root.join("zeta"));
        repo(&root.join("alpha"));
        repo(&root.join("b/c/nested-two-deep"));
        let found = find_repositories(root, 5, 1000, &never());
        assert_eq!(relative(root, found), vec![PathBuf::from("alpha"), PathBuf::from("b/c/nested-two-deep"), PathBuf::from("zeta")]);
    }

    #[test]
    fn a_repository_inside_another_repository_is_not_listed() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        repo(&root.join("outer"));
        repo(&root.join("outer/inner"));
        repo(&root.join("outer/folder/deeper"));
        let found = find_repositories(root, 5, 1000, &never());
        assert_eq!(relative(root, found), vec![PathBuf::from("outer")]);
    }

    #[test]
    fn skipped_and_hidden_folders_are_not_searched() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        repo(&root.join("node_modules/x"));
        repo(&root.join("target/debug/y"));
        repo(&root.join(".hidden/z"));
        repo(&root.join("Pods/p"));
        repo(&root.join("kept"));
        let found = find_repositories(root, 5, 1000, &never());
        assert_eq!(relative(root, found), vec![PathBuf::from("kept")]);
    }

    #[test]
    fn a_git_file_counts_as_a_repository() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("worktree")).unwrap();
        fs::write(root.join("worktree/.git"), "gitdir: /elsewhere/.git/worktrees/worktree\n").unwrap();
        let found = find_repositories(root, 5, 1000, &never());
        assert_eq!(relative(root, found), vec![PathBuf::from("worktree")]);
    }

    #[test]
    fn the_depth_limit_is_respected() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        // Five folders below the root is the deepest a repository may be.
        repo(&root.join("1/2/3/4/5"));
        repo(&root.join("1/2/3/4/5/6"));
        let found = find_repositories(root, 5, 1000, &never());
        assert_eq!(relative(root, found), vec![PathBuf::from("1/2/3/4/5")]);
    }

    #[test]
    fn the_limit_on_the_number_of_repositories_is_respected() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for n in 0..5 {
            repo(&root.join(format!("repo{n}")));
        }
        let found = find_repositories(root, 5, 3, &never());
        assert_eq!(found.len(), 3);
    }

    #[test]
    fn a_chosen_repository_is_the_only_result() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        repo(root);
        repo(&root.join("inside"));
        let found = find_repositories(root, 5, 1000, &never());
        assert_eq!(found, vec![root.to_path_buf()]);
    }

    #[test]
    fn a_cancelled_search_finds_nothing_more() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        repo(&root.join("one"));
        let cancel = AtomicBool::new(true);
        assert!(find_repositories(root, 5, 1000, &cancel).is_empty());
    }

    #[test]
    fn an_empty_folder_has_no_repositories() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("plain/folder")).unwrap();
        assert!(find_repositories(dir.path(), 5, 1000, &never()).is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn symbolic_links_are_not_followed() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        repo(&root.join("real"));
        // A link back to the folder above, which would loop forever if followed.
        fs::create_dir_all(root.join("plain")).unwrap();
        std::os::unix::fs::symlink(root, root.join("plain/loop")).unwrap();
        // A link to a repository, which is listed only once, as the repository itself.
        std::os::unix::fs::symlink(root.join("real"), root.join("linked")).unwrap();
        let found = find_repositories(root, 5, 1000, &never());
        assert_eq!(relative(root, found), vec![PathBuf::from("real")]);
    }
}
