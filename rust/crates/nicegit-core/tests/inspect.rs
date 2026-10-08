//! Integration tests for bisect, commit comparison, file versions, signatures, and GitHub
//! remotes and items. Repository tests run against temporary repositories; GitHub tests never
//! touch the network.

use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};

use nicegit_core::bisect::BisectMark;
use nicegit_core::client::GitClient;
use nicegit_core::github::{decode_items, GitHubRepository, ItemKind, ItemState};
use nicegit_core::signature::SignatureStatus;
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

/// Sets the identity and signing options in the repository's own config only.
fn configure(directory: &Path) {
    for (key, value) in [
        ("user.name", "Test"),
        ("user.email", "test@example.com"),
        ("commit.gpgsign", "false"),
        ("tag.gpgsign", "false"),
        // Keeps line endings byte-for-byte on Windows, where Git may default to autocrlf.
        ("core.autocrlf", "false"),
    ] {
        git(directory, &["config", key, value]);
    }
}

/// A temporary repository on branch `main`.
struct Repo {
    dir: TempDir,
}

impl Repo {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("create temp dir");
        git(dir.path(), &["init", "-b", "main"]);
        configure(dir.path());
        Repo { dir }
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }

    fn git(&self, arguments: &[&str]) -> String {
        git(self.path(), arguments)
    }

    fn write_bytes(&self, relative: &str, contents: &[u8]) {
        let file = self.path().join(relative);
        if let Some(parent) = file.parent() {
            fs::create_dir_all(parent).expect("create parent folder");
        }
        fs::write(file, contents).expect("write file");
    }

    fn write(&self, relative: &str, contents: &str) {
        self.write_bytes(relative, contents.as_bytes());
    }

    /// Stages everything and commits it, returning the new commit's full ID.
    fn commit(&self, message: &str) -> String {
        self.git(&["add", "--all"]);
        self.git(&["commit", "--quiet", "-m", message]);
        self.git(&["rev-parse", "HEAD"]).trim().to_string()
    }
}

// MARK: Bisect

#[test]
fn bisect_full_cycle_finds_first_bad_commit_and_end_restores_branch() {
    let repo = Repo::new();
    let client = GitClient::new();
    let mut commits = Vec::new();
    for index in 1..=8 {
        // Commits from the fifth onward carry the problem, so the first bad commit is the fifth.
        let marker = if index >= 5 { "bad" } else { "good" };
        repo.write("app.txt", &format!("version {index} {marker}\n"));
        commits.push(repo.commit(&format!("commit {index}")));
    }
    let head = commits[7].clone();
    let original_head = repo.git(&["rev-parse", "HEAD"]).trim().to_string();

    assert!(client.bisect_status(repo.path()).unwrap().is_none());
    client.start_bisect(&head, &commits[0], "main", Some(&original_head), repo.path()).unwrap();

    let mut rounds = 0;
    loop {
        let status = client.bisect_status(repo.path()).unwrap().expect("a bisect is running");
        assert_eq!(status.original_checkout, "main");
        if let Some(first_bad) = status.first_bad.clone() {
            assert_eq!(first_bad, commits[4], "the fifth commit introduced the problem");
            assert_eq!(status.remaining_steps, Some(0));
            break;
        }
        let testing = status.testing.clone().expect("a commit is checked out for testing");
        let contents = fs::read_to_string(repo.path().join("app.txt")).unwrap();
        let mark = if contents.contains("bad") { BisectMark::Bad } else { BisectMark::Good };
        client.mark_bisect(mark, &testing, repo.path()).unwrap();
        rounds += 1;
        assert!(rounds < 10, "bisect did not converge");
    }

    // A stale mark for a commit that is no longer checked out is refused.
    assert!(client.mark_bisect(BisectMark::Skip, &commits[0], repo.path()).is_err());

    client.end_bisect(repo.path()).unwrap();
    assert!(client.bisect_status(repo.path()).unwrap().is_none());
    let state = client.checkout_state(repo.path()).unwrap();
    assert_eq!(state.current_branch, "main");
    assert_eq!(state.head_hash.as_deref(), Some(original_head.as_str()));
}

#[test]
fn bisect_refuses_to_start_with_uncommitted_tracked_changes() {
    let repo = Repo::new();
    let client = GitClient::new();
    repo.write("app.txt", "one\n");
    let good = repo.commit("one");
    repo.write("app.txt", "two\n");
    let bad = repo.commit("two");
    repo.write("app.txt", "dirty\n");
    let head = client.checkout_state(repo.path()).unwrap().head_hash.unwrap();
    assert_eq!(head, bad);
    assert!(client.start_bisect(&bad, &good, "main", Some(&bad), repo.path()).is_err());
    assert!(client.bisect_status(repo.path()).unwrap().is_none());
}

#[test]
fn bisect_rejects_a_good_commit_that_is_not_an_ancestor() {
    let repo = Repo::new();
    let client = GitClient::new();
    repo.write("app.txt", "base\n");
    let base = repo.commit("base");
    repo.write("app.txt", "main side\n");
    let main_tip = repo.commit("main");
    repo.git(&["switch", "-q", "-c", "side", &base]);
    repo.write("other.txt", "side\n");
    let side_tip = repo.commit("side");
    repo.git(&["switch", "-q", "main"]);
    assert!(client.start_bisect(&main_tip, &side_tip, "main", Some(&main_tip), repo.path()).is_err());
    assert!(client.bisect_status(repo.path()).unwrap().is_none());
}

// MARK: Compare

#[test]
fn compare_two_commits_and_commit_against_working_files() {
    let repo = Repo::new();
    let client = GitClient::new();
    repo.write("keep.txt", "same\n");
    repo.write("change.txt", "one\n");
    repo.write("gone.txt", "bye\n");
    let first = repo.commit("first");

    repo.write("change.txt", "two\n");
    repo.write("new.txt", "fresh\n");
    fs::remove_file(repo.path().join("gone.txt")).unwrap();
    let second = repo.commit("second");

    let files = client.compare_files(&first, Some(&second), repo.path()).unwrap();
    let listed: Vec<(&str, &str)> = files.iter().map(|file| (file.status.as_str(), file.path.as_str())).collect();
    assert_eq!(listed, [("M", "change.txt"), ("D", "gone.txt"), ("A", "new.txt")]);

    let diff = client.compare_file_diff(&first, Some(&second), "change.txt", false, repo.path()).unwrap();
    assert!(diff.contains("-one") && diff.contains("+two"), "unexpected diff: {diff}");

    // Against the working files, uncommitted edits show and untracked files do not.
    repo.write("change.txt", "three\n");
    repo.write("scratch.txt", "not tracked\n");
    let working = client.compare_files(&second, None, repo.path()).unwrap();
    let listed: Vec<&str> = working.iter().map(|file| file.path.as_str()).collect();
    assert_eq!(listed, ["change.txt"]);
    let diff = client.compare_file_diff(&second, None, "change.txt", false, repo.path()).unwrap();
    assert!(diff.contains("-two") && diff.contains("+three"), "unexpected diff: {diff}");
}

#[test]
fn file_bytes_at_returns_binary_content_from_commits_and_working_files() {
    let repo = Repo::new();
    let client = GitClient::new();
    // A PNG signature followed by bytes that are not valid UTF-8.
    let png: Vec<u8> = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0xff, 0x00, 0x80];
    repo.write_bytes("images/my pic.png", &png);
    let commit = repo.commit("add picture");

    assert_eq!(client.file_bytes_at(Some(&commit), "images/my pic.png", repo.path()).unwrap(), Some(png.clone()));
    assert_eq!(client.file_bytes_at(Some("HEAD"), "images/missing.png", repo.path()).unwrap(), None);

    let mut newer = png.clone();
    newer.push(0x42);
    repo.write_bytes("images/my pic.png", &newer);
    assert_eq!(client.file_bytes_at(None, "images/my pic.png", repo.path()).unwrap(), Some(newer));
    fs::remove_file(repo.path().join("images/my pic.png")).unwrap();
    assert_eq!(client.file_bytes_at(None, "images/my pic.png", repo.path()).unwrap(), None);

    assert!(client.file_bytes_at(None, "../outside.png", repo.path()).is_err());
}

// MARK: Signatures

#[test]
fn unsigned_commit_has_no_signature() {
    let repo = Repo::new();
    let client = GitClient::new();
    repo.write("a.txt", "a\n");
    let commit = repo.commit("unsigned");
    assert_eq!(client.signature(&commit, repo.path()).unwrap(), None);
    assert_eq!(client.signature("HEAD", repo.path()).unwrap(), None);
}

#[test]
fn commit_with_an_unchecked_signature_is_not_reported_as_verified() {
    let repo = Repo::new();
    let client = GitClient::new();
    // A commit object whose signature header cannot be verified, written directly so no key or
    // signing tool is needed.
    let body = "tree 4b825dc642cb6eb9a060e54bf8d69288fbee4904\n\
        author Test <test@example.com> 1700000000 +0000\n\
        committer Test <test@example.com> 1700000000 +0000\n\
        gpgsig -----BEGIN PGP SIGNATURE-----\n \n invalid\n -----END PGP SIGNATURE-----\n\n\
        signed message\n";
    let mut child = Command::new("git")
        .args(["hash-object", "-t", "commit", "-w", "--stdin"])
        .current_dir(repo.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start git hash-object");
    child.stdin.take().unwrap().write_all(body.as_bytes()).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "hash-object failed: {}", String::from_utf8_lossy(&output.stderr));
    let id = String::from_utf8_lossy(&output.stdout).trim().to_string();

    let signature = client.signature(&id, repo.path()).unwrap().expect("the commit is signed");
    assert_ne!(signature.status, SignatureStatus::Verified);
}

// MARK: GitHub

#[test]
fn github_remote_addresses_are_parsed_in_every_supported_form() {
    let accepted = [
        ("https://github.com/owner/repo.git", "owner", "repo"),
        ("https://github.com/owner/repo", "owner", "repo"),
        ("https://github.com/owner/repo/", "owner", "repo"),
        ("https://user@github.com/owner/repo.git", "owner", "repo"),
        ("HTTPS://GitHub.com/Owner/Repo", "Owner", "Repo"),
        ("ssh://git@github.com/owner/repo.git", "owner", "repo"),
        ("ssh://github.com/owner/repo", "owner", "repo"),
        ("git@github.com:owner/repo.git", "owner", "repo"),
        ("git@github.com:owner/repo", "owner", "repo"),
        ("github.com:owner/repo.git", "owner", "repo"),
        ("  https://github.com/owner/my-repo.name_1.git\n", "owner", "my-repo.name_1"),
    ];
    for (address, owner, name) in accepted {
        let repository = GitHubRepository::parse(address).unwrap_or_else(|error| panic!("{address}: {error}"));
        assert_eq!(repository.owner, owner, "{address}");
        assert_eq!(repository.name, name, "{address}");
        assert_eq!(repository.slug(), format!("{owner}/{name}"));
    }

    let refused = [
        "https://gitlab.com/owner/repo.git",
        "git@gitlab.com:owner/repo.git",
        "http://github.com/owner/repo",
        "git://github.com/owner/repo",
        "https://github.com:443/owner/repo",
        "https://user:secret@github.com/owner/repo",
        "https://github.com/owner/repo?tab=readme",
        "https://github.com/owner/repo#readme",
        "https://github.com/owner",
        "https://github.com/owner/repo/tree/main",
        "https://github.com/own er/repo",
        "https://github.com/owner/..",
        "https://github.com/owner/.git",
        "https://github.com/",
        "/srv/github.com/owner/repo",
        "",
    ];
    for address in refused {
        assert!(GitHubRepository::parse(address).is_err(), "{address} should be refused");
    }
}

#[test]
fn github_items_decode_and_refuse_foreign_links() {
    let repository = GitHubRepository::parse("https://github.com/acme/widgets").unwrap();
    let json = r#"[
        {"number": 12, "title": "Fix \"quoted\" thing", "url": "https://github.com/acme/widgets/pull/12",
         "author": {"login": "ann", "is_bot": false}, "state": "OPEN", "isDraft": true,
         "labels": [{"name": "bug", "color": "d73a4a", "id": "x"}]},
        {"number": 3, "title": "Second", "url": "https://github.com/acme/widgets/pull/3",
         "author": null, "state": "MERGED", "isDraft": false, "labels": []}
    ]"#;
    let items = decode_items(json, &repository, ItemKind::PullRequest).unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].title, "Fix \"quoted\" thing");
    assert!(items[0].is_draft);
    assert_eq!(items[0].labels[0].name, "bug");
    assert_eq!(items[0].labels[0].color, "d73a4a");
    assert_eq!(items[1].author, None);
    assert!(items[0].matches("#12"));
    assert!(items[0].matches("ANN"));
    assert!(items[0].matches("quoted"));
    assert!(!items[0].matches("nothing like this"));

    // Links must point at this repository's item pages of the requested kind.
    let foreign = r#"[{"number": 1, "title": "x", "url": "https://github.com/other/widgets/pull/1"}]"#;
    assert!(decode_items(foreign, &repository, ItemKind::PullRequest).is_err());
    let wrong_kind = r#"[{"number": 1, "title": "x", "url": "https://github.com/acme/widgets/pull/1"}]"#;
    assert!(decode_items(wrong_kind, &repository, ItemKind::Issue).is_err());
    let duplicate = r#"[
        {"number": 4, "title": "a", "url": "https://github.com/acme/widgets/issues/4"},
        {"number": 4, "title": "b", "url": "https://github.com/acme/widgets/issues/4"}
    ]"#;
    assert!(decode_items(duplicate, &repository, ItemKind::Issue).is_err());
    assert!(decode_items("not json", &repository, ItemKind::Issue).is_err());
    assert_eq!(decode_items("[]", &repository, ItemKind::Issue).unwrap(), Vec::new());
}

#[test]
fn github_merged_state_is_refused_for_issues_before_running_gh() {
    let repository = GitHubRepository::parse("https://github.com/acme/widgets").unwrap();
    let directory = tempfile::tempdir().unwrap();
    let result = GitClient::new().github_items(&repository, ItemKind::Issue, ItemState::Merged, 10, directory.path());
    assert!(result.is_err());
}
