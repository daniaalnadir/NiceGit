//! Small adaptations of core Git operations to what the interface asks for.

use std::path::Path;

use nicegit_core::client::ForcePushPreview;
use nicegit_core::merge_preview::MergeOutcome;
use nicegit_core::rebase::Expectation;
use nicegit_core::signature::SignatureStatus;
use nicegit_core::{Branch, GitClient, Operation, Result};

/// Rebases the current branch onto `onto`, after confirming both are as displayed.
pub fn rebase_onto(client: &GitClient, onto: &Branch, branch: &str, head: Option<&str>, directory: &Path) -> Result<()> {
    let expected = Expectation { branch: Some(branch), head, source_branch: Some(onto) };
    client.start(Operation::Rebase, &onto.tip, None, expected, directory)
}

/// Reverts one commit; a merge is reverted against its first parent.
pub fn revert(client: &GitClient, commit: &str, branch: &str, head: Option<&str>, directory: &Path) -> Result<()> {
    let merge = client.commit_details(commit, directory)?.parents.len() > 1;
    let expected = Expectation { branch: Some(branch), head, source_branch: None };
    client.start(Operation::Revert, commit, merge.then_some(1), expected, directory)
}

/// A sentence describing what merging or rebasing onto `source` would do.
pub fn merge_preview_text(source: &str, rebase: bool, directory: &Path) -> Result<String> {
    let client = GitClient::new();
    let preview = if rebase { client.preview_rebase(source, directory)? } else { client.preview_merge(source, directory)? };
    let estimate = if preview.is_estimate { " This is an estimate: rebasing replays each commit, so results can differ." } else { "" };
    let ignored =
        if preview.blocked_by_ignored_files { " Ignored local files would be overwritten; move or back them up first." } else { "" };
    let text = match preview.outcome {
        MergeOutcome::UpToDate => "Already up to date: there is nothing to integrate.".to_string(),
        MergeOutcome::FastForward if rebase => "Your branch has no commits of its own, so it simply moves forward.".to_string(),
        MergeOutcome::FastForward => "Fast-forward: your branch moves forward with no merge commit.".to_string(),
        MergeOutcome::Clean => {
            let files = preview.changed_file_count;
            format!("No conflicts expected. {files} file{} would change.", if files == 1 { "" } else { "s" })
        }
        MergeOutcome::Conflicts(paths) => {
            let shown: Vec<&str> = paths.iter().take(6).map(String::as_str).collect();
            let more = if paths.len() > shown.len() { format!(" and {} more", paths.len() - shown.len()) } else { String::new() };
            format!("Expected conflicts in {}{more}. You can resolve them, or abort to return to where you started.", shown.join(", "))
        }
    };
    Ok(format!("{text}{estimate}{ignored}"))
}

/// The confirmation text for a force push: the commits on the remote that it removes, up to ten
/// of them, and that a push refused by the lease loses nothing.
pub fn force_push_message(preview: &ForcePushPreview, upstream: &str) -> String {
    const SHOWN: usize = 10;
    const SAFETY: &str = "If someone else pushed since you last fetched, the push is refused and nothing is lost.";
    let count = preview.removed_count;
    if count == 0 {
        return format!("Your branch already contains everything on {upstream}; a normal Push would also work.\n\n{SAFETY}");
    }
    let intro = if count == 1 {
        format!(
            "{upstream} will be replaced with your branch. This commit on the remote is not in your branch and will be removed from it:"
        )
    } else {
        format!("{upstream} will be replaced with your branch. These {count} commits on the remote are not in your branch and will be removed from it:")
    };
    let mut lines = vec![intro];
    lines.extend(preview.removed.iter().take(SHOWN).map(|commit| format!("• {} {}", commit.short_hash, commit.subject)));
    let more = count - preview.removed.len().min(SHOWN);
    if more > 0 {
        lines.push(format!("…and {more} more"));
    }
    format!("{}\n\n{SAFETY}", lines.join("\n"))
}

/// A commit's signature as display text, a colour level, and tooltip help.
pub fn signature_summary(hash: &str, directory: &Path) -> Option<SignatureSummary> {
    let signature = GitClient::new().signature(hash, directory).ok()??;
    Some(summarize_signature(&signature))
}

/// The inspector's line for a signature: the Mac app's wording and colour level, the signer
/// only when verified, and the key and any problem for the tooltip.
fn summarize_signature(signature: &nicegit_core::signature::CommitSignature) -> SignatureSummary {
    // The Mac app's colours: green, orange, red, and secondary for a signature it cannot check.
    let level = match signature.status {
        SignatureStatus::Verified => 0,
        SignatureStatus::Untrusted => 1,
        SignatureStatus::Bad => 2,
        SignatureStatus::Unverifiable => 3,
    };
    // As in the Mac app, only a verified signature names its signer; the key and any problem
    // are in the tooltip.
    let signer = match signature.status {
        SignatureStatus::Verified if !signature.signer.is_empty() => format!(" · {}", signature.signer),
        _ => String::new(),
    };
    let help = [(!signature.key.is_empty()).then(|| format!("Key {}", signature.key)), signature.problem.clone()]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join("\n");
    SignatureSummary { text: format!("{}{signer}", signature.status.title()), level, help }
}

/// What the commit inspector shows about a signature.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignatureSummary {
    pub text: String,
    /// 0 verified, 1 untrusted, 2 bad, 3 cannot be checked.
    pub level: u8,
    /// The key and any problem Git reported, for the tooltip; empty when there is neither.
    pub help: String,
}

/// The GitHub repository behind a remote's first fetch address, when that address is on github.com
/// or one of `enterprise_hosts`.
pub fn github_repository_for_remote(
    snapshot: &nicegit_core::Snapshot,
    remote: &str,
    enterprise_hosts: &[String],
) -> Option<nicegit_core::github::GitHubRepository> {
    let address = snapshot.remote_fetch_addresses.get(remote)?.first()?;
    nicegit_core::github::GitHubRepository::parse_with_hosts(address, enterprise_hosts).ok()
}

/// GitHub remotes, each with its repository.
fn github_remotes(snapshot: &nicegit_core::Snapshot, enterprise_hosts: &[String]) -> Vec<(String, nicegit_core::github::GitHubRepository)> {
    snapshot
        .remotes
        .iter()
        .filter_map(|remote| Some((remote.clone(), github_repository_for_remote(snapshot, remote, enterprise_hosts)?)))
        .collect()
}

/// GitHub remotes, with the web address of `commit` on each.
pub fn github_commit_links(snapshot: &nicegit_core::Snapshot, commit: &str, enterprise_hosts: &[String]) -> Vec<(String, String)> {
    github_remotes(snapshot, enterprise_hosts)
        .into_iter()
        .map(|(remote, repository)| (remote, format!("{}/{}/commit/{commit}", repository.web_base(), repository.slug())))
        .collect()
}

/// GitHub remotes, with the web address of `path` as it was in `commit` on each.
pub fn github_file_links(
    snapshot: &nicegit_core::Snapshot,
    commit: &str,
    path: &str,
    enterprise_hosts: &[String],
) -> Vec<(String, String)> {
    let path = encode_path(path);
    github_remotes(snapshot, enterprise_hosts)
        .into_iter()
        .map(|(remote, repository)| (remote, format!("{}/{}/blob/{commit}/{path}", repository.web_base(), repository.slug())))
        .collect()
}

/// Splits a branch path such as `origin/feature/x` into its remote and the branch on it. Remote
/// names can contain slashes, so the longest configured remote that prefixes the path wins.
fn split_remote<'a>(snapshot: &nicegit_core::Snapshot, path: &'a str) -> Option<(&'a str, &'a str)> {
    let remote = snapshot.remotes.iter().filter(|remote| path.starts_with(&format!("{remote}/"))).max_by_key(|remote| remote.len())?;
    Some((&path[..remote.len()], &path[remote.len() + 1..]))
}

/// The GitHub page for a branch. A remote-tracking branch links to itself on its remote; a
/// local branch links to its upstream when that is a branch on a GitHub remote.
pub fn github_branch_link(snapshot: &nicegit_core::Snapshot, branch: &nicegit_core::Branch, enterprise_hosts: &[String]) -> Option<String> {
    let path = if branch.is_remote {
        branch.name.strip_prefix("remotes/").unwrap_or(&branch.name)
    } else {
        branch.upstream.as_deref()?.strip_prefix("refs/remotes/")?
    };
    let (remote, remote_branch) = split_remote(snapshot, path)?;
    let repository = github_repository_for_remote(snapshot, remote, enterprise_hosts)?;
    Some(format!("{}/{}/tree/{}", repository.web_base(), repository.slug(), encode_path(remote_branch)))
}

/// Why Git would refuse `name` as a branch or tag name, following `git check-ref-format`, so a
/// dialog can say so while it is typed; `None` for an acceptable or empty name.
pub fn ref_name_problem(name: &str) -> Option<&'static str> {
    if name.is_empty() {
        return None;
    }
    if name.chars().any(char::is_whitespace) {
        return Some("Names cannot contain spaces.");
    }
    if name.contains("..") {
        return Some("Names cannot contain \"..\".");
    }
    if name.chars().any(|c| c.is_control() || matches!(c, '~' | '^' | ':' | '?' | '*' | '[' | '\\')) {
        return Some("Names cannot contain ~ ^ : ? * [ or \\.");
    }
    if name.starts_with('-') {
        return Some("Names cannot start with \"-\".");
    }
    if name.starts_with('/') || name.ends_with('/') || name.contains("//") {
        return Some("Names cannot start or end with \"/\", or contain \"//\".");
    }
    if name.split('/').any(|part| part.starts_with('.') || part.ends_with(".lock")) || name.ends_with('.') {
        return Some("Parts of a name cannot start with \".\" or end with \".lock\", and a name cannot end with \".\".");
    }
    if name.contains("@{") {
        return Some("Names cannot contain \"@{\".");
    }
    if name == "HEAD" {
        return Some("HEAD is reserved for Git's own use.");
    }
    None
}

/// Where a local branch's pull request goes: the GitHub repository behind its upstream's remote,
/// and the branch on that remote the pull request comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PullRequestTarget {
    pub repository: nicegit_core::github::GitHubRepository,
    /// The remote the upstream belongs to, such as `origin`.
    pub remote: String,
    /// The upstream's branch on that remote, without the remote's name.
    pub remote_branch: String,
}

/// The pull request target of a local branch whose upstream is a branch on a GitHub remote.
/// Remote names can contain slashes, so the longest one that prefixes the upstream is its remote.
pub fn github_pull_request_target(
    snapshot: &nicegit_core::Snapshot,
    branch: &nicegit_core::Branch,
    enterprise_hosts: &[String],
) -> Option<PullRequestTarget> {
    if branch.is_remote {
        return None;
    }
    // The upstream is a full ref; only a remote-tracking branch has a remote to compare on.
    let upstream = branch.upstream.as_deref()?.strip_prefix("refs/remotes/")?;
    let (remote, remote_branch) = split_remote(snapshot, upstream)?;
    let repository = github_repository_for_remote(snapshot, remote, enterprise_hosts)?;
    Some(PullRequestTarget { repository, remote: remote.to_string(), remote_branch: remote_branch.to_string() })
}

/// The GitHub compare page for a local branch whose upstream is on a GitHub remote. It opens the
/// pull request form in the browser, the fallback to Create pull request. See
/// [`github_pull_request_target`].
pub fn github_pull_request_link(
    snapshot: &nicegit_core::Snapshot,
    branch: &nicegit_core::Branch,
    enterprise_hosts: &[String],
) -> Option<String> {
    let target = github_pull_request_target(snapshot, branch, enterprise_hosts)?;
    Some(format!("{}/{}/compare/{}?expand=1", target.repository.web_base(), target.repository.slug(), encode_path(&target.remote_branch)))
}

/// Percent-encodes a branch name for a URL path, keeping its slashes.
fn encode_path(text: &str) -> String {
    let mut encoded = String::new();
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => encoded.push(byte as char),
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

/// A submenu for copying a commit's GitHub link, choosing the remote explicitly. Making a link
/// neither publishes the commit nor checks that the remote has it.
pub fn github_link_menu(ui: &mut egui::Ui, snapshot: &nicegit_core::Snapshot, commit: &str, enterprise_hosts: &[String]) {
    let links = github_commit_links(snapshot, commit, enterprise_hosts);
    if links.is_empty() {
        return;
    }
    ui.menu_button(format!("{}  Copy GitHub commit link", egui_phosphor::regular::GITHUB_LOGO), |ui| {
        for (remote, url) in links {
            if ui.button(remote).on_hover_text(&url).clicked() {
                ui.close();
                ui.ctx().copy_text(url);
            }
        }
    });
}

/// Hover text for an item that opens a commit's page, which exists only once the commit is pushed.
pub const PUSHED_COMMIT_NOTE: &str = "\nThe commit must be on GitHub for this page to exist.";

/// An item that opens a github.com page. With one github.com remote it is a plain button; with
/// several, a submenu lists each remote. Nothing is shown without one. `note` is added to each
/// item's hover text under its address.
pub fn github_open_menu(ui: &mut egui::Ui, label: &str, links: Vec<(String, String)>, note: &str) {
    let text = format!("{}  {label}", egui_phosphor::regular::ARROW_SQUARE_OUT);
    match links.as_slice() {
        [] => {}
        [(_, url)] => {
            if ui.button(text).on_hover_text(format!("{url}{note}")).clicked() {
                ui.close();
                crate::ui::changes::open_url(url);
            }
        }
        _ => {
            ui.menu_button(text, |ui| {
                for (remote, url) in &links {
                    if ui.button(remote.as_str()).on_hover_text(format!("{url}{note}")).clicked() {
                        ui.close();
                        crate::ui::changes::open_url(url);
                    }
                }
            });
        }
    }
}

#[cfg(test)]
mod force_push_message_tests {
    use nicegit_core::client::ForcePushPreview;
    use nicegit_core::Commit;

    use super::force_push_message;

    fn commit(index: usize) -> Commit {
        Commit {
            hash: format!("{index:040}"),
            short_hash: format!("c{index:06}"),
            parents: Vec::new(),
            refs: Vec::new(),
            subject: format!("Commit {index}"),
            author_name: String::new(),
            author_email: String::new(),
            relative_date: String::new(),
            commit_time: None,
        }
    }

    fn preview(removed_count: usize, listed: usize) -> ForcePushPreview {
        ForcePushPreview { remote_tip: "f".repeat(40), removed: (1..=listed).map(commit).collect(), removed_count, added_count: 1 }
    }

    #[test]
    fn nothing_removed_says_a_normal_push_would_work() {
        let text = force_push_message(&preview(0, 0), "origin/main");
        assert!(text.starts_with("Your branch already contains everything on origin/main; a normal Push would also work."));
        assert!(text.ends_with("the push is refused and nothing is lost."));
        assert!(!text.contains('•'));
    }

    #[test]
    fn a_single_removed_commit_is_named_in_the_singular() {
        let text = force_push_message(&preview(1, 1), "origin/main");
        assert!(text.contains("This commit on the remote is not in your branch"));
        assert!(text.contains("• c000001 Commit 1"));
        assert!(!text.contains("more"));
    }

    #[test]
    fn up_to_ten_commits_are_listed_without_a_more_line() {
        let text = force_push_message(&preview(10, 10), "origin/main");
        assert!(text.contains("These 10 commits on the remote"));
        assert_eq!(text.matches('•').count(), 10);
        assert!(!text.contains("more"));
    }

    #[test]
    fn further_commits_are_counted_in_a_more_line() {
        let text = force_push_message(&preview(12, 12), "origin/main");
        assert_eq!(text.matches('•').count(), 10);
        assert!(text.contains("…and 2 more"));
    }

    #[test]
    fn a_long_list_counts_every_commit_past_the_fifty_loaded() {
        let text = force_push_message(&preview(60, 50), "origin/main");
        assert!(text.contains("These 60 commits"));
        assert!(text.contains("…and 50 more"));
    }
}

#[cfg(test)]
mod tests {
    use nicegit_core::signature::{CommitSignature, SignatureStatus};

    use super::summarize_signature;

    fn signed(status: SignatureStatus, problem: Option<&str>) -> CommitSignature {
        CommitSignature { status, signer: "Riley <riley@example.com>".into(), key: "SHA256:abc".into(), problem: problem.map(String::from) }
    }

    #[test]
    fn ref_names_are_checked_as_git_checks_them() {
        use super::ref_name_problem;
        for good in ["main", "feature/login", "fix-42", "release/1.2.0", "user/name_with.dots", "ünïcode", "@"] {
            assert_eq!(ref_name_problem(good), None, "{good}");
        }
        for bad in [
            "my branch",
            "a..b",
            "a~1",
            "a^",
            "a:b",
            "a?",
            "a*",
            "a[b",
            "a\\b",
            "-x",
            "/x",
            "x/",
            "a//b",
            ".hidden",
            "a/.b",
            "x.lock",
            "x.",
            "a@{b",
            "HEAD",
        ] {
            assert!(ref_name_problem(bad).is_some(), "{bad} should be refused");
        }
        assert_eq!(ref_name_problem(""), None, "an empty name is left to the OK button");

        // Git itself agrees on each, checking the name as a branch is stored. (With --branch, Git
        // reads "@" as the current branch, so the stored form is the fair comparison.)
        for name in
            ["main", "feature/login", "my branch", "a..b", "a~1", "a:b", "x/", "a//b", ".hidden", "x.lock", "x.", "a@{b", "@", "ünïcode"]
        {
            let reference = format!("refs/heads/{name}");
            let git = std::process::Command::new("git").args(["check-ref-format", &reference]).output().expect("run git");
            assert_eq!(git.status.success(), ref_name_problem(name).is_none(), "{name}: Git and NiceGit disagree");
        }
        // Git's format check allows these two, but refuses them as branch names.
        for name in ["-x", "HEAD"] {
            let repo = crate::ui_tests::temp_dir();
            let run = |args: &[&str]| std::process::Command::new("git").args(args).current_dir(repo.path()).output().expect("run git");
            run(&["init", "-q"]);
            run(&[
                "-c",
                "user.name=T",
                "-c",
                "user.email=t@e.invalid",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-q",
                "--allow-empty",
                "-m",
                "x",
            ]);
            assert!(!run(&["branch", "--", name]).status.success(), "Git refuses the branch name {name}");
        }
    }

    #[test]
    fn pull_request_links_use_the_upstream_branch_on_its_github_remote() {
        use nicegit_core::{Branch, Snapshot};
        let mut snapshot = Snapshot { remotes: vec!["origin".into(), "origin/team".into(), "mirror".into()], ..Default::default() };
        snapshot.remote_fetch_addresses.insert("origin".into(), vec!["git@github.com:acme/app.git".into()]);
        snapshot.remote_fetch_addresses.insert("origin/team".into(), vec!["https://github.com/team/app".into()]);
        snapshot.remote_fetch_addresses.insert("mirror".into(), vec!["https://example.com/app.git".into()]);
        let branch = |upstream: Option<&str>| Branch {
            name: "local".into(),
            is_current: true,
            is_remote: false,
            tip: "abc".into(),
            subject: String::new(),
            upstream: upstream.map(String::from),
        };
        let link = |upstream| super::github_pull_request_link(&snapshot, &branch(upstream), &[]);

        assert_eq!(
            link(Some("refs/remotes/origin/feature/fix #2")).as_deref(),
            Some("https://github.com/acme/app/compare/feature/fix%20%232?expand=1")
        );
        assert_eq!(
            link(Some("refs/remotes/origin/team/topic")).as_deref(),
            Some("https://github.com/team/app/compare/topic?expand=1"),
            "the longest remote name wins"
        );
        assert_eq!(link(Some("refs/remotes/mirror/topic")), None, "not on github.com");
        assert_eq!(link(Some("refs/heads/main")), None, "a local upstream has no remote");
        assert_eq!(link(None), None, "an unpublished branch has nothing to compare");
    }

    /// A snapshot whose remotes have these fetch addresses, in the order given.
    fn with_remotes(addresses: &[(&str, &str)]) -> nicegit_core::Snapshot {
        let mut snapshot =
            nicegit_core::Snapshot { remotes: addresses.iter().map(|(name, _)| name.to_string()).collect(), ..Default::default() };
        for (name, address) in addresses {
            snapshot.remote_fetch_addresses.insert(name.to_string(), vec![address.to_string()]);
        }
        snapshot
    }

    fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
        list.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect()
    }

    fn remote_branch_named(name: &str) -> nicegit_core::Branch {
        nicegit_core::Branch {
            name: format!("remotes/{name}"),
            is_current: false,
            is_remote: true,
            tip: "abc".into(),
            subject: String::new(),
            upstream: None,
        }
    }

    fn local_branch_with_upstream(upstream: Option<&str>) -> nicegit_core::Branch {
        nicegit_core::Branch {
            name: "local".into(),
            is_current: false,
            is_remote: false,
            tip: "abc".into(),
            subject: String::new(),
            upstream: upstream.map(String::from),
        }
    }

    #[test]
    fn github_commit_links_accept_scp_and_https_addresses() {
        let expected = pairs(&[("origin", "https://github.com/owner/repo/commit/abc123")]);
        let scp = with_remotes(&[("origin", "git@github.com:owner/repo.git")]);
        let https = with_remotes(&[("origin", "https://github.com/owner/repo")]);
        assert_eq!(super::github_commit_links(&scp, "abc123", &[]), expected);
        assert_eq!(super::github_commit_links(&https, "abc123", &[]), expected);
    }

    #[test]
    fn github_links_skip_remotes_that_are_not_on_github() {
        let snapshot = with_remotes(&[("mirror", "https://example.com/owner/repo.git")]);
        assert!(super::github_commit_links(&snapshot, "abc123", &[]).is_empty());
        assert!(super::github_file_links(&snapshot, "abc123", "a.txt", &[]).is_empty());
        assert_eq!(super::github_branch_link(&snapshot, &remote_branch_named("mirror/main"), &[]), None);
    }

    #[test]
    fn github_commit_links_list_each_github_remote() {
        let snapshot = with_remotes(&[
            ("origin", "git@github.com:owner/repo.git"),
            ("mirror", "https://example.com/owner/repo.git"),
            ("fork", "https://github.com/me/repo"),
        ]);
        assert_eq!(
            super::github_commit_links(&snapshot, "abc123", &[]),
            pairs(&[("origin", "https://github.com/owner/repo/commit/abc123"), ("fork", "https://github.com/me/repo/commit/abc123"),])
        );
    }

    #[test]
    fn github_branch_links_use_the_remote_branch_on_its_remote() {
        let snapshot = with_remotes(&[("origin", "git@github.com:owner/repo.git")]);
        assert_eq!(
            super::github_branch_link(&snapshot, &remote_branch_named("origin/feature/x"), &[]).as_deref(),
            Some("https://github.com/owner/repo/tree/feature/x")
        );
    }

    #[test]
    fn github_branch_links_pick_the_longest_remote_name() {
        let snapshot = with_remotes(&[("team", "https://github.com/one/app"), ("team/fork", "https://github.com/two/app")]);
        assert_eq!(
            super::github_branch_link(&snapshot, &remote_branch_named("team/fork/topic"), &[]).as_deref(),
            Some("https://github.com/two/app/tree/topic")
        );
        assert_eq!(
            super::github_branch_link(&snapshot, &remote_branch_named("team/topic"), &[]).as_deref(),
            Some("https://github.com/one/app/tree/topic")
        );
    }

    #[test]
    fn github_branch_link_encodes_the_upstream_branch() {
        let snapshot = with_remotes(&[("origin", "git@github.com:owner/repo.git")]);
        let branch = local_branch_with_upstream(Some("refs/remotes/origin/my#branch"));
        assert_eq!(super::github_branch_link(&snapshot, &branch, &[]).as_deref(), Some("https://github.com/owner/repo/tree/my%23branch"));
    }

    #[test]
    fn github_branch_link_needs_a_github_upstream() {
        let snapshot = with_remotes(&[("origin", "git@github.com:owner/repo.git")]);
        assert_eq!(super::github_branch_link(&snapshot, &local_branch_with_upstream(None), &[]), None, "no upstream");
        assert_eq!(
            super::github_branch_link(&snapshot, &local_branch_with_upstream(Some("refs/heads/main")), &[]),
            None,
            "a local upstream has no remote"
        );
    }

    #[test]
    fn github_file_links_encode_the_path_and_keep_its_slashes() {
        let snapshot = with_remotes(&[("origin", "git@github.com:owner/repo.git")]);
        assert_eq!(
            super::github_file_links(&snapshot, "abc123", "dir/a b#c.txt", &[]),
            pairs(&[("origin", "https://github.com/owner/repo/blob/abc123/dir/a%20b%23c.txt")])
        );
        assert_eq!(
            super::github_file_links(&snapshot, "abc123", "ü?.txt", &[]),
            pairs(&[("origin", "https://github.com/owner/repo/blob/abc123/%C3%BC%3F.txt")])
        );
    }

    #[test]
    fn enterprise_remote_links_use_its_listed_host() {
        let hosts = vec!["git.example.com".to_string()];
        let snapshot = with_remotes(&[("origin", "https://git.example.com/acme/app.git")]);
        assert_eq!(
            super::github_commit_links(&snapshot, "abc123", &hosts),
            pairs(&[("origin", "https://git.example.com/acme/app/commit/abc123")])
        );
        assert_eq!(
            super::github_file_links(&snapshot, "abc123", "dir/a b.txt", &hosts),
            pairs(&[("origin", "https://git.example.com/acme/app/blob/abc123/dir/a%20b.txt")])
        );
        assert_eq!(
            super::github_branch_link(&snapshot, &remote_branch_named("origin/feature/x"), &hosts).as_deref(),
            Some("https://git.example.com/acme/app/tree/feature/x")
        );
        let branch = local_branch_with_upstream(Some("refs/remotes/origin/topic"));
        assert_eq!(super::github_branch_link(&snapshot, &branch, &hosts).as_deref(), Some("https://git.example.com/acme/app/tree/topic"));
        assert_eq!(
            super::github_pull_request_link(&snapshot, &branch, &hosts).as_deref(),
            Some("https://git.example.com/acme/app/compare/topic?expand=1")
        );
    }

    #[test]
    fn enterprise_remote_gets_no_links_until_its_host_is_listed() {
        let snapshot = with_remotes(&[("origin", "https://git.example.com/acme/app.git")]);
        assert!(super::github_commit_links(&snapshot, "abc123", &[]).is_empty());
        assert!(super::github_file_links(&snapshot, "abc123", "a.txt", &[]).is_empty());
        assert_eq!(super::github_branch_link(&snapshot, &remote_branch_named("origin/main"), &[]), None);
        let branch = local_branch_with_upstream(Some("refs/remotes/origin/main"));
        assert_eq!(super::github_pull_request_link(&snapshot, &branch, &[]), None);
        let other = vec!["ghe.other.com".to_string()];
        assert!(super::github_commit_links(&snapshot, "abc123", &other).is_empty(), "only the listed host counts");
    }

    #[test]
    fn each_signature_state_reads_and_colours_as_in_the_mac_app() {
        let verified = summarize_signature(&signed(SignatureStatus::Verified, None));
        assert_eq!(verified.text, "Verified signature · Riley <riley@example.com>", "only a verified signature names its signer");
        assert_eq!(verified.level, 0, "green");

        let untrusted = summarize_signature(&signed(SignatureStatus::Untrusted, None));
        assert_eq!(untrusted.text, "Valid signature from an untrusted key");
        assert_eq!(untrusted.level, 1, "orange");

        let bad = summarize_signature(&signed(SignatureStatus::Bad, None));
        assert_eq!(bad.text, "Bad signature: this commit does not match it");
        assert_eq!(bad.level, 2, "red");

        let unchecked = summarize_signature(&signed(SignatureStatus::Unverifiable, Some("gpg: no public key")));
        assert_eq!(unchecked.text, "Signed, but it cannot be verified on this computer");
        assert_eq!(unchecked.level, 3, "secondary");
        assert_eq!(unchecked.help, "Key SHA256:abc\ngpg: no public key", "the key and the problem are in the tooltip");
    }
}
