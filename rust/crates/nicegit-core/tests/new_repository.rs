//! Integration tests for `GitClient::create_repository`: the first branch's name, the README, and
//! the first commit, run in temporary folders.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use std::sync::{Once, OnceLock};

use nicegit_core::client::GitClient;
use nicegit_core::NewRepository;
use tempfile::TempDir;

// MARK: Helpers

/// Runs raw Git in `directory`, returning its output without checking the status.
fn git_output(directory: &Path, arguments: &[&str]) -> Output {
    let mut command = Command::new("git");
    command.args(arguments).current_dir(directory);
    // Inherited repository variables would redirect these commands to another repository.
    for key in ["GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE", "GIT_OBJECT_DIRECTORY", "GIT_COMMON_DIR"] {
        command.env_remove(key);
    }
    command.output().unwrap_or_else(|error| panic!("could not run git {arguments:?}: {error}"))
}

/// Runs raw Git in `directory` and returns its standard output, panicking on failure.
fn git(directory: &Path, arguments: &[&str]) -> String {
    let output = git_output(directory, arguments);
    assert!(output.status.success(), "git {arguments:?} failed: {}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// Gives this test process a global identity. A new repository has no settings of its own, and
/// its first commit needs a name and email. Only the global settings are replaced, so a
/// repository's own settings still take precedence.
fn use_test_identity() {
    static CONFIG: OnceLock<TempDir> = OnceLock::new();
    static ENVIRONMENT: Once = Once::new();
    let config = CONFIG.get_or_init(|| tempfile::tempdir().expect("create temp dir"));
    ENVIRONMENT.call_once(|| {
        let file = config.path().join("gitconfig");
        fs::write(&file, "[user]\n\tname = Test\n\temail = test@example.com\n[commit]\n\tgpgsign = false\n").expect("write config");
        std::env::set_var("GIT_CONFIG_GLOBAL", file);
    });
}

fn client() -> GitClient {
    use_test_identity();
    GitClient::new()
}

fn options(initial_branch: &str, readme: bool, first_commit: bool) -> NewRepository {
    NewRepository { initial_branch: initial_branch.to_string(), readme, first_commit }
}

/// Commit subjects reachable from HEAD, newest first; empty when there is no commit.
fn subjects(directory: &Path) -> Vec<String> {
    if git_output(directory, &["rev-parse", "--verify", "--quiet", "HEAD"]).status.success() {
        git(directory, &["log", "--format=%s"]).lines().map(str::to_string).collect()
    } else {
        Vec::new()
    }
}

// MARK: Creating

#[test]
fn creates_the_branch_a_readme_and_the_first_commit() {
    let parent = tempfile::tempdir().expect("create temp dir");
    let folder = parent.path().join("notes");

    client().create_repository(&folder, &options("trunk", true, true)).expect("create repository");

    assert_eq!(git(&folder, &["symbolic-ref", "HEAD"]).trim(), "refs/heads/trunk");
    assert_eq!(subjects(&folder), vec!["Initial commit"]);
    assert_eq!(git(&folder, &["ls-tree", "--name-only", "HEAD"]).trim(), "README.md");
    assert_eq!(git(&folder, &["show", "HEAD:README.md"]), "# notes\n", "the README is named after the folder");
    assert!(git(&folder, &["status", "--porcelain"]).trim().is_empty(), "the README is committed, so nothing is left over");
}

#[test]
fn a_first_commit_without_a_readme_is_empty() {
    let parent = tempfile::tempdir().expect("create temp dir");
    let folder = parent.path().join("empty");

    client().create_repository(&folder, &options("main", false, true)).expect("create repository");

    assert_eq!(subjects(&folder), vec!["Initial commit"]);
    assert!(git(&folder, &["ls-tree", "--name-only", "-r", "HEAD"]).trim().is_empty(), "the commit holds no files");
    assert!(!folder.join("README.md").exists());
}

#[test]
fn a_readme_without_a_first_commit_stays_uncommitted() {
    let parent = tempfile::tempdir().expect("create temp dir");
    let folder = parent.path().join("draft");

    client().create_repository(&folder, &options("main", true, false)).expect("create repository");

    assert!(subjects(&folder).is_empty(), "there is no commit yet");
    assert_eq!(fs::read_to_string(folder.join("README.md")).expect("read README"), "# draft\n");
    assert_eq!(git(&folder, &["status", "--porcelain"]), "?? README.md\n");
}

#[test]
fn an_existing_readme_is_kept_and_committed() {
    let parent = tempfile::tempdir().expect("create temp dir");
    let folder = parent.path().join("mine");
    fs::create_dir(&folder).expect("create folder");
    fs::write(folder.join("README.md"), "Written before\n").expect("write README");

    client().create_repository(&folder, &options("main", true, true)).expect("create repository");

    assert_eq!(fs::read_to_string(folder.join("README.md")).expect("read README"), "Written before\n");
    assert_eq!(git(&folder, &["show", "HEAD:README.md"]), "Written before\n");
    assert_eq!(subjects(&folder), vec!["Initial commit"]);
}

#[test]
fn an_existing_folder_without_a_repository_is_initialized() {
    let parent = tempfile::tempdir().expect("create temp dir");
    let folder = parent.path().join("existing");
    fs::create_dir(&folder).expect("create folder");
    fs::write(folder.join("notes.txt"), "kept\n").expect("write file");

    client().create_repository(&folder, &options("main", false, false)).expect("create repository");

    assert!(folder.join(".git").is_dir());
    assert_eq!(fs::read_to_string(folder.join("notes.txt")).expect("read file"), "kept\n");
}

#[test]
fn the_default_first_branch_follows_git_or_is_main() {
    let parent = tempfile::tempdir().expect("create temp dir");
    // Git for Windows sets init.defaultBranch in its system settings, so the expected name is
    // whatever Git reports, and "main" only when nothing sets it.
    let configured = git_output(parent.path(), &["config", "--get", "init.defaultBranch"]);
    let configured = String::from_utf8_lossy(&configured.stdout).trim().to_string();
    let expected = if configured.is_empty() { "main".to_string() } else { configured };
    assert_eq!(client().default_initial_branch(), expected);

    let folder = parent.path().join("plain");
    client().initialize(&folder).expect("initialize");

    assert_eq!(git(&folder, &["symbolic-ref", "HEAD"]).trim(), format!("refs/heads/{expected}"));
    assert!(subjects(&folder).is_empty());
}

// MARK: Refusals

#[test]
fn an_existing_repository_is_refused_and_left_unchanged() {
    let parent = tempfile::tempdir().expect("create temp dir");
    let folder = parent.path().join("repo");
    fs::create_dir(&folder).expect("create folder");
    git(&folder, &["init", "-q", "-b", "main"]);

    let error = client().create_repository(&folder, &options("trunk", true, true)).expect_err("an existing repository is refused");

    assert!(error.to_string().contains("already in a Git repository. Open it instead."), "{error}");
    assert_eq!(git(&folder, &["symbolic-ref", "HEAD"]).trim(), "refs/heads/main", "the branch is not renamed");
    assert!(!folder.join("README.md").exists(), "no README is written");
    assert!(subjects(&folder).is_empty());
}

#[test]
fn a_folder_inside_a_repository_is_refused_without_being_created() {
    let parent = tempfile::tempdir().expect("create temp dir");
    git(parent.path(), &["init", "-q", "-b", "main"]);
    let folder = parent.path().join("sub").join("new");

    let error = client().create_repository(&folder, &options("trunk", true, true)).expect_err("a folder inside a repository is refused");

    assert!(error.to_string().contains("already in a Git repository"), "{error}");
    assert!(!parent.path().join("sub").exists(), "nothing is created for a refused folder");
}

#[test]
fn an_invalid_branch_name_is_refused_before_anything_is_created() {
    let parent = tempfile::tempdir().expect("create temp dir");
    let folder = parent.path().join("named");

    assert!(client().create_repository(&folder, &options("bad name", true, true)).is_err(), "a name with a space is refused");
    assert!(client().create_repository(&folder, &options("  ", true, true)).is_err(), "an empty name is refused");
    assert!(!folder.exists(), "no folder is made for a refused name");
}
