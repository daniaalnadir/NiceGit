//! Commit signatures, as the local Git configuration verifies them. Verification is local; no
//! key servers are contacted.

use std::path::Path;

use crate::client::GitClient;
use crate::models::{GitError, Result};
use crate::runner;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SignatureStatus {
    /// Valid and from a trusted key.
    Verified,
    /// Valid, but the key's trust or validity is unknown, expired, or revoked.
    Untrusted,
    /// The signature does not match the commit.
    Bad,
    /// Signed, but this computer cannot check it, for example because a key or tool is missing.
    Unverifiable,
}

impl SignatureStatus {
    pub fn title(self) -> &'static str {
        match self {
            // The Mac app's wording; "this Mac" becomes "this computer" on every system.
            SignatureStatus::Verified => "Verified signature",
            SignatureStatus::Untrusted => "Valid signature from an untrusted key",
            SignatureStatus::Bad => "Bad signature: this commit does not match it",
            SignatureStatus::Unverifiable => "Signed, but it cannot be verified on this computer",
        }
    }

    /// The wording for a tag's signature: as `title`, except that a bad one names the tag.
    pub fn tag_title(self) -> &'static str {
        match self {
            SignatureStatus::Bad => "Bad signature: this tag does not match it",
            other => other.title(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommitSignature {
    pub status: SignatureStatus,
    /// The signer as Git reports it, which may be empty.
    pub signer: String,
    /// The key that made the signature, which may be empty.
    pub key: String,
    /// Why the signature could not be checked, when Git reported a problem.
    pub problem: Option<String>,
}

impl GitClient {
    /// The signature on `commit`, or `None` when it is unsigned.
    pub fn signature(&self, commit: &str, directory: &Path) -> Result<Option<CommitSignature>> {
        let id = self.resolve_commit(commit, directory)?;
        // Read the commit's headers first, so an unsigned commit never starts a verifier.
        let object = self.run(&["cat-file", "commit", &id], directory)?;
        let headers = object.split("\n\n").next().unwrap_or("");
        let signed = headers.lines().any(|line| line.starts_with("gpgsig ") || line.starts_with("gpgsig-sha256 "));
        if !signed {
            return Ok(None);
        }
        let output = match self.run(&["log", "-1", "--no-color", "--format=%G?%x1f%GS%x1f%GK", &id, "--"], directory) {
            Ok(output) => output,
            // A broken signing setup, such as an invalid gpg.format, leaves the signature unchecked.
            Err(error) => {
                return Ok(Some(CommitSignature {
                    status: SignatureStatus::Unverifiable,
                    signer: String::new(),
                    key: String::new(),
                    problem: Some(message_of(&error)),
                }))
            }
        };
        let output = runner::strip_line_terminator(output);
        let fields: Vec<&str> = output.split('\u{1f}').collect();
        let Some(code) = fields.first().and_then(|field| field.chars().next()) else {
            return Ok(Some(CommitSignature {
                status: SignatureStatus::Unverifiable,
                signer: String::new(),
                key: String::new(),
                problem: None,
            }));
        };
        Ok(Some(CommitSignature {
            status: status_of(code),
            signer: fields.get(1).map(|field| field.to_string()).unwrap_or_default(),
            key: fields.get(2).map(|field| field.to_string()).unwrap_or_default(),
            problem: None,
        }))
    }

    /// The signature on the annotated tag `name`, or `None` when it is lightweight or unsigned.
    /// The name must already have passed `create_tag`'s check.
    pub fn tag_signature(&self, name: &str, directory: &Path) -> Result<Option<CommitSignature>> {
        self.run(&["check-ref-format", &format!("refs/tags/{name}")], directory)?;
        let id = self.tag_tip(name, directory)?;
        // Only a tag object can carry a signature. Read it first, so an unsigned tag never starts a verifier.
        if self.run_trimmed(&["cat-file", "-t", &id], directory)? != "tag" {
            return Ok(None);
        }
        let object = self.run(&["cat-file", "tag", &id], directory)?;
        if !SIGNATURE_BLOCKS.iter().any(|block| object.contains(block)) {
            return Ok(None);
        }
        let output = match runner::run_reporting(&["verify-tag", "--raw", &id], directory, &self.options()) {
            Ok(output) => output,
            // A broken signing setup, such as an invalid gpg.format, leaves the signature unchecked.
            Err(error) => {
                return Ok(Some(CommitSignature {
                    status: SignatureStatus::Unverifiable,
                    signer: String::new(),
                    key: String::new(),
                    problem: Some(message_of(&error)),
                }))
            }
        };
        let (status, signer, key, problem) = classify_tag_verification(&output);
        Ok(Some(CommitSignature { status, signer, key, problem }))
    }
}

/// The signature blocks Git can find in a tag object, for OpenPGP, SSH, and X.509 signatures.
const SIGNATURE_BLOCKS: [&str; 3] = ["-----BEGIN PGP SIGNATURE-----", "-----BEGIN SSH SIGNATURE-----", "-----BEGIN SIGNED MESSAGE-----"];

/// The status, signer, key, and problem `classify_tag_verification` reports.
type Verification = (SignatureStatus, String, String, Option<String>);

/// Classifies the output of `git verify-tag --raw`. OpenPGP output carries GnuPG status lines
/// (`[GNUPG:] ...`); anything else is Git's own SSH wording. The exit status is not used: Git
/// exits non-zero for an SSH signature from an unlisted key, though the signature is good.
fn classify_tag_verification(output: &str) -> Verification {
    if output.contains("[GNUPG:]") {
        classify_openpgp(output)
    } else {
        classify_ssh(output)
    }
}

/// OpenPGP: a bad signature is Bad; a signature from a key that is expired or revoked, or whose
/// trust is not full or ultimate, is Untrusted; a missing key or other output is Unverifiable.
fn classify_openpgp(output: &str) -> Verification {
    // `(keyword, key id, user id)` for the signature line, and `(key id, user id)` for a bad one.
    let mut signature: Option<(&str, &str, &str)> = None;
    let mut bad: Option<(&str, &str)> = None;
    let mut trust = "";
    let mut fingerprint = "";
    for line in output.lines() {
        let Some(status) = line.trim().strip_prefix("[GNUPG:] ") else { continue };
        let mut fields = status.splitn(3, ' ');
        let keyword = fields.next().unwrap_or("");
        let first = fields.next().unwrap_or("");
        let rest = fields.next().unwrap_or("");
        match keyword {
            "BADSIG" => bad = Some((first, rest)),
            "GOODSIG" | "EXPKEYSIG" | "REVKEYSIG" | "EXPSIG" => signature = Some((keyword, first, rest)),
            "VALIDSIG" => fingerprint = first,
            "TRUST_FULLY" | "TRUST_ULTIMATE" | "TRUST_UNDEFINED" | "TRUST_NEVER" | "TRUST_MARGINAL" => trust = keyword,
            _ => {}
        }
    }
    if let Some((key, signer)) = bad {
        return (SignatureStatus::Bad, signer.to_string(), key.to_string(), None);
    }
    let Some((keyword, key, signer)) = signature else { return unverifiable(output) };
    let key = if fingerprint.is_empty() { key } else { fingerprint };
    let status = match (keyword, trust) {
        ("GOODSIG", "TRUST_FULLY" | "TRUST_ULTIMATE") => SignatureStatus::Verified,
        _ => SignatureStatus::Untrusted,
    };
    (status, signer.to_string(), key.to_string(), None)
}

/// SSH: `Good "git" signature for <principal> with <type> key <fingerprint>` is Verified, and
/// without a principal (Git also prints "No principal matched.") it is Untrusted. A signature
/// that does not match is Bad.
fn classify_ssh(output: &str) -> Verification {
    for line in output.lines().map(str::trim) {
        if let Some(rest) = line.strip_prefix("Good \"git\" signature for ") {
            // The principal may contain " with ", so the key details after the last one are split off.
            let parsed = rest.rsplit_once(" with ").and_then(|(principal, details)| Some((principal, ssh_key(details)?)));
            if let Some((principal, key)) = parsed {
                return (SignatureStatus::Verified, principal.to_string(), key.to_string(), None);
            }
        } else if let Some(key) = line.strip_prefix("Good \"git\" signature with ").and_then(ssh_key) {
            return (SignatureStatus::Untrusted, String::new(), key.to_string(), None);
        }
    }
    if output.contains("Signature verification failed") || output.contains("incorrect signature") {
        return (SignatureStatus::Bad, String::new(), String::new(), None);
    }
    unverifiable(output)
}

/// The fingerprint in `<type> key <fingerprint>`, which Git prints after an SSH key's name.
fn ssh_key(details: &str) -> Option<&str> {
    details.split_once(" key ").map(|(_, fingerprint)| fingerprint)
}

/// Unverifiable, with the last line of Git's output as the reason, when there is any.
fn unverifiable(output: &str) -> Verification {
    let problem = output.lines().map(str::trim).rfind(|line| !line.is_empty()).map(str::to_string);
    (SignatureStatus::Unverifiable, String::new(), String::new(), problem)
}

/// The status for Git's `%G?` code. Expired and revoked keys (`X`, `Y`, `R`) count as untrusted,
/// as in the Mac app: the signature is genuine, but the key should not be relied on.
fn status_of(code: char) -> SignatureStatus {
    match code {
        'G' => SignatureStatus::Verified,
        'U' | 'X' | 'Y' | 'R' => SignatureStatus::Untrusted,
        'B' => SignatureStatus::Bad,
        // `N` (no signature) and `E` (cannot check) both leave the signature unchecked.
        _ => SignatureStatus::Unverifiable,
    }
}

fn message_of(error: &GitError) -> String {
    match error {
        GitError::CommandFailed { message, .. } => message.clone(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_status_reads_as_in_the_mac_app() {
        assert_eq!(SignatureStatus::Verified.title(), "Verified signature");
        assert_eq!(SignatureStatus::Untrusted.title(), "Valid signature from an untrusted key");
        assert_eq!(SignatureStatus::Bad.title(), "Bad signature: this commit does not match it");
        assert_eq!(SignatureStatus::Unverifiable.title(), "Signed, but it cannot be verified on this computer");
    }

    #[test]
    fn each_git_code_maps_to_the_status_the_mac_app_shows() {
        assert_eq!(status_of('G'), SignatureStatus::Verified);
        for code in ['U', 'X', 'Y', 'R'] {
            assert_eq!(status_of(code), SignatureStatus::Untrusted, "{code}");
        }
        assert_eq!(status_of('B'), SignatureStatus::Bad);
        for code in ['E', 'N', '?'] {
            assert_eq!(status_of(code), SignatureStatus::Unverifiable, "{code}");
        }
    }

    #[test]
    fn a_bad_tag_signature_names_the_tag_and_other_statuses_keep_their_titles() {
        assert_eq!(SignatureStatus::Bad.tag_title(), "Bad signature: this tag does not match it");
        assert_eq!(SignatureStatus::Verified.tag_title(), SignatureStatus::Verified.title());
        assert_eq!(SignatureStatus::Untrusted.tag_title(), SignatureStatus::Untrusted.title());
        assert_eq!(SignatureStatus::Unverifiable.tag_title(), SignatureStatus::Unverifiable.title());
    }

    const GOOD_SIGNATURE: &str = "[GNUPG:] NEWSIG\n\
        [GNUPG:] KEY_CONSIDERED 0123456789ABCDEF0123456789ABCDEF01234567 0\n\
        [GNUPG:] GOODSIG 4AEE18F83AFDEB23 Alice Example <alice@example.com>\n\
        [GNUPG:] VALIDSIG 0123456789ABCDEF0123456789ABCDEF01234567 2026-01-02 1767312000 0 4 0 1 8 00 0123456789ABCDEF0123456789ABCDEF01234567\n";

    #[test]
    fn a_gnupg_good_signature_from_a_fully_trusted_key_is_verified() {
        let output = format!("{GOOD_SIGNATURE}[GNUPG:] TRUST_ULTIMATE 0 pgp\n");
        assert_eq!(
            classify_tag_verification(&output),
            (
                SignatureStatus::Verified,
                "Alice Example <alice@example.com>".to_string(),
                "0123456789ABCDEF0123456789ABCDEF01234567".to_string(),
                None
            )
        );
        let fully = format!("{GOOD_SIGNATURE}[GNUPG:] TRUST_FULLY 0 pgp\n");
        assert_eq!(classify_tag_verification(&fully).0, SignatureStatus::Verified);
    }

    #[test]
    fn a_gnupg_good_signature_from_a_key_without_full_trust_is_untrusted() {
        for trust in ["TRUST_UNDEFINED", "TRUST_NEVER", "TRUST_MARGINAL"] {
            let output = format!("{GOOD_SIGNATURE}[GNUPG:] {trust} 0 pgp\n");
            let (status, signer, key, problem) = classify_tag_verification(&output);
            assert_eq!(status, SignatureStatus::Untrusted, "{trust}");
            assert_eq!(signer, "Alice Example <alice@example.com>");
            assert_eq!(key, "0123456789ABCDEF0123456789ABCDEF01234567");
            assert_eq!(problem, None);
        }
        // With no trust line at all, the key is unknown too.
        assert_eq!(classify_tag_verification(GOOD_SIGNATURE).0, SignatureStatus::Untrusted);
    }

    #[test]
    fn an_expired_or_revoked_gnupg_key_is_untrusted_even_with_full_trust() {
        for keyword in ["EXPKEYSIG", "REVKEYSIG", "EXPSIG"] {
            let output = format!(
                "[GNUPG:] NEWSIG\n[GNUPG:] {keyword} 4AEE18F83AFDEB23 Alice Example <alice@example.com>\n[GNUPG:] TRUST_ULTIMATE 0 pgp\n"
            );
            let (status, signer, key, _) = classify_tag_verification(&output);
            assert_eq!(status, SignatureStatus::Untrusted, "{keyword}");
            assert_eq!(signer, "Alice Example <alice@example.com>", "{keyword}");
            assert_eq!(key, "4AEE18F83AFDEB23", "{keyword}");
        }
    }

    #[test]
    fn a_gnupg_bad_signature_is_bad_and_names_its_signer() {
        let output = "[GNUPG:] NEWSIG\n[GNUPG:] BADSIG 4AEE18F83AFDEB23 Alice Example <alice@example.com>\n";
        assert_eq!(
            classify_tag_verification(output),
            (SignatureStatus::Bad, "Alice Example <alice@example.com>".to_string(), "4AEE18F83AFDEB23".to_string(), None)
        );
    }

    #[test]
    fn a_gnupg_signature_without_its_key_is_unverifiable_with_the_last_line_as_the_problem() {
        let no_key = "gpg: Signature made Fri 02 Jan 2026 12:00:00 UTC\n\
            gpg:                using RSA key 0123456789ABCDEF0123456789ABCDEF01234567\n\
            [GNUPG:] NEWSIG\n\
            [GNUPG:] ERRSIG 4AEE18F83AFDEB23 1 8 00 1767312000 9\n\
            [GNUPG:] NO_PUBKEY 4AEE18F83AFDEB23\n\
            gpg: Can't check signature: No public key\n";
        assert_eq!(
            classify_tag_verification(no_key),
            (SignatureStatus::Unverifiable, String::new(), String::new(), Some("gpg: Can't check signature: No public key".to_string()))
        );
        let errsig_only = "[GNUPG:] ERRSIG 4AEE18F83AFDEB23 1 8 00 1767312000 9\n";
        assert_eq!(classify_tag_verification(errsig_only).0, SignatureStatus::Unverifiable);
    }

    #[test]
    fn an_ssh_signature_with_a_matching_principal_is_verified() {
        let output = "Good \"git\" signature for test@example.com with ED25519 key SHA256:Qm2yCnq4R7dXN0L6bHpVZ3rM9wKfTg1sDoEaUj8IiXc\n";
        assert_eq!(
            classify_tag_verification(output),
            (
                SignatureStatus::Verified,
                "test@example.com".to_string(),
                "SHA256:Qm2yCnq4R7dXN0L6bHpVZ3rM9wKfTg1sDoEaUj8IiXc".to_string(),
                None
            )
        );
    }

    #[test]
    fn an_ssh_signature_without_a_principal_is_untrusted_with_its_key() {
        // Git's output for a good signature from a key no allowed signer lists; it exits non-zero.
        let output = "Good \"git\" signature with ED25519 key SHA256:Qm2yCnq4R7dXN0L6bHpVZ3rM9wKfTg1sDoEaUj8IiXc\nNo principal matched.\n";
        assert_eq!(
            classify_tag_verification(output),
            (SignatureStatus::Untrusted, String::new(), "SHA256:Qm2yCnq4R7dXN0L6bHpVZ3rM9wKfTg1sDoEaUj8IiXc".to_string(), None)
        );
    }

    #[test]
    fn an_ssh_signature_that_does_not_verify_is_bad() {
        let output = "Signature verification failed\n";
        assert_eq!(classify_tag_verification(output), (SignatureStatus::Bad, String::new(), String::new(), None));
        let incorrect = "error: incorrect signature\n";
        assert_eq!(classify_tag_verification(incorrect).0, SignatureStatus::Bad);
    }

    #[test]
    fn an_ssh_tag_without_an_allowed_signers_file_is_unverifiable_with_git_s_reason() {
        let output = "error: gpg.ssh.allowedSignersFile needs to be configured and exist for ssh signature verification\n";
        assert_eq!(
            classify_tag_verification(output),
            (
                SignatureStatus::Unverifiable,
                String::new(),
                String::new(),
                Some("error: gpg.ssh.allowedSignersFile needs to be configured and exist for ssh signature verification".to_string())
            )
        );
    }

    #[test]
    fn a_broken_signing_setup_is_unverifiable_with_the_reason_from_git() {
        let output = "error: invalid value for 'gpg.format': ''\nfatal: bad config variable 'gpg.format' in file 'config' at line 6\n";
        let (status, signer, key, problem) = classify_tag_verification(output);
        assert_eq!(status, SignatureStatus::Unverifiable);
        assert!(signer.is_empty() && key.is_empty());
        assert_eq!(problem.as_deref(), Some("fatal: bad config variable 'gpg.format' in file 'config' at line 6"));
    }

    #[test]
    fn a_verifier_that_prints_nothing_is_unverifiable_without_a_problem() {
        assert_eq!(classify_tag_verification("\n\n"), (SignatureStatus::Unverifiable, String::new(), String::new(), None));
    }
}
