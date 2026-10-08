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
            SignatureStatus::Verified => "Verified signature",
            SignatureStatus::Untrusted => "Signed by an untrusted key",
            SignatureStatus::Bad => "Bad signature",
            SignatureStatus::Unverifiable => "Signature cannot be checked",
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
        let status = match code {
            'G' => SignatureStatus::Verified,
            'U' | 'X' | 'Y' | 'R' => SignatureStatus::Untrusted,
            'B' => SignatureStatus::Bad,
            // `N` (no signature) and `E` (cannot check) both leave the signature unchecked.
            _ => SignatureStatus::Unverifiable,
        };
        Ok(Some(CommitSignature {
            status,
            signer: fields.get(1).map(|field| field.to_string()).unwrap_or_default(),
            key: fields.get(2).map(|field| field.to_string()).unwrap_or_default(),
            problem: None,
        }))
    }
}

fn message_of(error: &GitError) -> String {
    match error {
        GitError::CommandFailed { message, .. } => message.clone(),
        other => other.to_string(),
    }
}
