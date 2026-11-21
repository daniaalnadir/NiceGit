//! Reads from the real Forgejo API on codeberg.org, without a token. Only GET requests are made, so
//! nothing is created or changed on the server. The tests are ignored by default because they need
//! the network; run them with `cargo test -p nicegit-core --test gitea_live -- --ignored --nocapture`.

use std::time::Instant;

use nicegit_core::gitea::{BACKGROUND_PAGES, MAX_PAGES};
use nicegit_core::github::{Cancel, ItemKind};
use nicegit_core::hosting::{Host, HostedRepository, ListedHosts};
use nicegit_core::GitClient;

fn repository() -> HostedRepository {
    HostedRepository::parse("https://codeberg.org/forgejo/forgejo", ListedHosts::default()).expect("a Codeberg address")
}

fn directory() -> std::path::PathBuf {
    std::env::temp_dir()
}

#[test]
#[ignore = "reads codeberg.org over the network"]
fn open_pull_requests_with_checks_decode_from_the_real_api() {
    let repository = repository();
    let started = Instant::now();
    let pulls = GitClient::new()
        .gitea_open_pull_requests(&repository, None, MAX_PAGES, &Cancel::default(), &directory())
        .expect("the open pull requests are read");
    println!("open pull requests: {} in {:?}", pulls.len(), started.elapsed());
    assert!(!pulls.is_empty(), "forgejo/forgejo has open pull requests");
    let with_checks = pulls.iter().filter(|pull| pull.checks != Default::default()).count();
    println!("  with checks: {with_checks}");
    for pull in pulls.iter().take(5) {
        println!("  #{} [{:?}] {} <- {} checks {:?}", pull.number, pull.author, pull.title, pull.head_branch, pull.checks);
    }
    let started = Instant::now();
    let background = GitClient::new()
        .gitea_open_pull_requests(&repository, None, BACKGROUND_PAGES, &Cancel::default(), &directory())
        .expect("the background listing is read");
    println!("open pull requests, background (2 pages): {} in {:?}", background.len(), started.elapsed());
    assert!(!background.is_empty());
    for pull in &pulls {
        assert!(pull.number > 0);
        assert!(!pull.title.trim().is_empty(), "#{} has a title", pull.number);
        assert!(pull.url.starts_with("https://codeberg.org/forgejo/forgejo/pulls/"), "{}", pull.url);
        assert!(pull.updated_at > 0, "#{} has a date", pull.number);
    }
}

#[test]
#[ignore = "reads codeberg.org over the network"]
fn issues_decode_from_the_real_api() {
    let repository = repository();
    let started = Instant::now();
    let issues = GitClient::new().gitea_items(&repository, ItemKind::Issue, None, 20, &directory()).expect("the open issues are read");
    println!("open issues: {} in {:?}", issues.len(), started.elapsed());
    assert!(!issues.is_empty(), "forgejo/forgejo has open issues");
    for issue in &issues {
        assert!(issue.number > 0);
        assert!(!issue.title.trim().is_empty());
        assert!(issue.url.starts_with("https://codeberg.org/forgejo/forgejo/issues/"), "{}", issue.url);
    }
}

#[test]
#[ignore = "reads codeberg.org over the network"]
fn the_default_branch_and_a_branch_pull_decode_from_the_real_api() {
    let repository = repository();
    let client = GitClient::new();
    let started = Instant::now();
    let branch = client.gitea_default_branch(&repository, None, &directory()).expect("the default branch is read");
    println!("default branch: {branch} in {:?}", started.elapsed());
    assert!(!branch.is_empty());

    // Pull requests from forks are not this repository's branches, so the first one on a branch here is used.
    let pulls = client.gitea_open_pull_requests(&repository, None, MAX_PAGES, &Cancel::default(), &directory()).expect("open pulls");
    let head = pulls
        .iter()
        .find(|pull| !pull.is_cross_repository)
        .map(|pull| pull.head_branch.clone())
        .expect("at least one open pull request from a branch of this repository");
    let started = Instant::now();
    let found = client
        .gitea_pull_request_for_branch(&repository, None, &head, &directory())
        .expect("the pull for a branch is read")
        .expect("the branch has a pull request");
    println!("pull for branch {head}: #{} {:?} in {:?}", found.number, found.state, started.elapsed());
    assert!(found.number > 0);
    assert_eq!(found.kind, Host::Gitea);
    assert!(found.url.starts_with("https://codeberg.org/forgejo/forgejo/pulls/"), "{}", found.url);

    let started = Instant::now();
    let missing = client
        .gitea_pull_request_for_branch(&repository, None, "no-such-branch-for-nicegit-live-test", &directory())
        .expect("a branch with no pull request is not an error");
    println!("pull for a branch with none: {missing:?} in {:?}", started.elapsed());
    assert!(missing.is_none());
}
