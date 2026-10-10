//! Small adaptations of core Git operations to what the interface asks for.

use std::path::Path;

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

/// Remotes on github.com, with the web address of `commit` on each.
pub fn github_commit_links(snapshot: &nicegit_core::Snapshot, commit: &str) -> Vec<(String, String)> {
    snapshot
        .remotes
        .iter()
        .filter_map(|remote| {
            let address = snapshot.remote_fetch_addresses.get(remote)?.first()?;
            let repository = nicegit_core::github::GitHubRepository::parse(address).ok()?;
            Some((remote.clone(), format!("https://github.com/{}/commit/{commit}", repository.slug())))
        })
        .collect()
}

/// A submenu for copying a commit's github.com link, choosing the remote explicitly. Making a
/// link neither publishes the commit nor checks that the remote has it.
pub fn github_link_menu(ui: &mut egui::Ui, snapshot: &nicegit_core::Snapshot, commit: &str) {
    let links = github_commit_links(snapshot, commit);
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

#[cfg(test)]
mod tests {
    use nicegit_core::signature::{CommitSignature, SignatureStatus};

    use super::summarize_signature;

    fn signed(status: SignatureStatus, problem: Option<&str>) -> CommitSignature {
        CommitSignature { status, signer: "Riley <riley@example.com>".into(), key: "SHA256:abc".into(), problem: problem.map(String::from) }
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
