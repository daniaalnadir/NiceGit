//! Reads from the real Bitbucket Cloud API for atlassian/atlassian-refapp, a public repository with open pull
//! requests, without credentials. Only GET requests are made, so nothing is created or changed on the server.
//! The tests are ignored by default because they need the network; run them with
//! `cargo test -p nicegit-core --test bitbucket_live -- --ignored --nocapture`.

use std::time::Instant;

use nicegit_core::bitbucket::{BACKGROUND_PAGES, MAX_PAGES};
use nicegit_core::forge_api::NO_ISSUE_TRACKER;
use nicegit_core::github::{Cancel, ItemKind};
use nicegit_core::hosting::{Host, HostedRepository, ListedHosts};
use nicegit_core::GitClient;

fn repository() -> HostedRepository {
    HostedRepository::parse("https://bitbucket.org/atlassian/atlassian-refapp", ListedHosts::default()).expect("a Bitbucket address")
}

fn directory() -> std::path::PathBuf {
    std::env::temp_dir()
}

#[test]
#[ignore = "reads bitbucket.org over the network"]
fn open_pull_requests_with_checks_decode_from_the_real_api() {
    let repository = repository();
    let started = Instant::now();
    let pulls = GitClient::new()
        .bitbucket_open_pull_requests(&repository, None, MAX_PAGES, &Cancel::default(), &directory())
        .expect("the open pull requests are read");
    println!("open pull requests: {} in {:?}", pulls.len(), started.elapsed());
    // refapp has 79 open pull requests; one page of 50 fills the cap, so no second page is asked for.
    assert_eq!(pulls.len(), 50, "the list is capped at the open pull request limit");
    let with_checks = pulls.iter().filter(|pull| pull.checks != Default::default()).count();
    println!("  with checks: {with_checks}");
    for pull in pulls.iter().take(5) {
        println!("  #{} [{:?}] {} <- {} checks {:?}", pull.number, pull.author, pull.title, pull.head_branch, pull.checks);
    }

    let started = Instant::now();
    let background = GitClient::new()
        .bitbucket_open_pull_requests(&repository, None, BACKGROUND_PAGES, &Cancel::default(), &directory())
        .expect("the background listing is read");
    println!("open pull requests, background (2 pages): {} in {:?}", background.len(), started.elapsed());
    assert_eq!(background.len(), 50, "the background read gives the same capped list");
    for pull in &pulls {
        assert!(pull.number > 0);
        assert!(!pull.title.trim().is_empty(), "#{} has a title", pull.number);
        assert!(pull.url.starts_with("https://bitbucket.org/atlassian/atlassian-refapp/pull-requests/"), "{}", pull.url);
        assert!(pull.updated_at > 0, "#{} has a date", pull.number);
    }
}

#[test]
#[ignore = "reads bitbucket.org over the network"]
fn a_branch_pull_request_is_found_by_its_source_branch() {
    let repository = repository();
    let client = GitClient::new();
    let pulls =
        client.bitbucket_open_pull_requests(&repository, None, BACKGROUND_PAGES, &Cancel::default(), &directory()).expect("open pulls");
    let head = pulls
        .iter()
        .find(|pull| !pull.is_cross_repository)
        .map(|pull| pull.head_branch.clone())
        .expect("at least one open pull request from a branch of this repository");
    let started = Instant::now();
    let found = client
        .bitbucket_pull_request_for_branch(&repository, None, &head, &directory())
        .expect("the pull request for a branch is read")
        .expect("the branch has a pull request");
    println!("pull request for branch {head}: #{} {:?} in {:?}", found.number, found.state, started.elapsed());
    assert_eq!(found.kind, Host::Bitbucket);
    assert!(found.url.starts_with("https://bitbucket.org/atlassian/atlassian-refapp/pull-requests/"), "{}", found.url);

    let started = Instant::now();
    let missing = client
        .bitbucket_pull_request_for_branch(&repository, None, "no-such-branch-for-nicegit-live-test", &directory())
        .expect("a branch with no pull request is not an error");
    println!("pull request for a branch with none: {missing:?} in {:?}", started.elapsed());
    assert!(missing.is_none());
}

#[test]
#[ignore = "reads bitbucket.org over the network"]
fn issues_decode_or_report_the_switched_off_tracker() {
    let repository = repository();
    let started = Instant::now();
    match GitClient::new().bitbucket_items(&repository, ItemKind::Issue, None, 20, &directory()) {
        Ok(issues) => {
            println!("open issues: {} in {:?}", issues.len(), started.elapsed());
            for issue in &issues {
                assert!(issue.url.starts_with("https://bitbucket.org/atlassian/atlassian-refapp/issues/"), "{}", issue.url);
            }
        }
        Err(error) => {
            println!("issues: {error} in {:?}", started.elapsed());
            assert!(error.to_string().contains(NO_ISSUE_TRACKER), "an unexpected failure: {error}");
        }
    }
}
