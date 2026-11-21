//! Access tokens for the pull request services: one entry per host, kept in `forge-tokens.ron` in
//! NiceGit's settings folder, readable only by its owner on Unix. Gitea and Forgejo hosts keep a token;
//! Bitbucket keeps `username:app-password`; Azure DevOps keeps a personal access token per organization.
//! Settings never holds a token, and nothing here logs one. A host with no saved token falls back to
//! the `GITEA_TOKEN` variable for Gitea and Forgejo, and `AZURE_DEVOPS_EXT_PAT` for Azure DevOps.

use std::collections::BTreeMap;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use nicegit_core::gitea::is_plausible_token;

/// The environment variable that holds a token when a host has none saved.
pub const ENVIRONMENT: &str = "GITEA_TOKEN";
/// The environment variable that holds an Azure DevOps personal access token when no organization has one saved.
pub const AZURE_ENVIRONMENT: &str = "AZURE_DEVOPS_EXT_PAT";
/// The key Bitbucket credentials are saved under. Bitbucket Cloud is only bitbucket.org.
pub const BITBUCKET_KEY: &str = "bitbucket.org";
/// The prefix of an Azure DevOps organization's key, such as `dev.azure.com/acme`.
const AZURE_PREFIX: &str = "dev.azure.com/";
/// The file in NiceGit's settings folder that holds the tokens.
#[cfg(not(test))]
const TOKEN_FILE: &str = "forge-tokens.ron";

/// The saved tokens, keyed by lower-cased host name.
pub type Tokens = BTreeMap<String, String>;

/// Where the saved tokens live. Tests have no settings folder, so they never reach the person's.
pub fn token_file() -> Option<PathBuf> {
    #[cfg(not(test))]
    {
        eframe::storage_dir("nicegit").map(|dir| dir.join(TOKEN_FILE))
    }
    #[cfg(test)]
    {
        None
    }
}

/// The tokens saved at `path`. A missing file holds none; a file that cannot be read as tokens is an
/// error, so saving never overwrites what it cannot understand.
pub fn load(path: &Path) -> io::Result<Tokens> {
    match std::fs::read_to_string(path) {
        Ok(text) if text.trim().is_empty() => Ok(Tokens::new()),
        Ok(text) => ron::from_str(&text).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string())),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Tokens::new()),
        Err(error) => Err(error),
    }
}

/// The environment's token, when it is set and not blank.
pub fn environment_token() -> Option<String> {
    environment_value(ENVIRONMENT)
}

/// The value of an environment variable, trimmed, when it is set and not blank.
fn environment_value(name: &str) -> Option<String> {
    std::env::var(name).ok().map(|token| token.trim().to_string()).filter(|token| !token.is_empty())
}

/// The key an Azure DevOps organization's token is saved under, such as `dev.azure.com/acme`.
pub fn azure_key(organization: &str) -> String {
    format!("{AZURE_PREFIX}{}", organization.to_ascii_lowercase())
}

/// The token saved for `key` exactly, with no environment fallback.
pub fn saved_for(path: Option<&Path>, key: &str) -> Option<String> {
    path.and_then(|path| load(path).ok())
        .and_then(|tokens| tokens.get(&key.to_ascii_lowercase()).cloned())
        .filter(|token| !token.is_empty())
}

/// The Bitbucket credentials saved, as `username:app-password`, when there are some.
pub fn bitbucket_credentials(path: Option<&Path>) -> Option<String> {
    saved_for(path, BITBUCKET_KEY)
}

/// The Azure DevOps token for an organization: the one saved for it, else the environment's.
pub fn azure_token(path: Option<&Path>, organization: &str) -> Option<String> {
    saved_for(path, &azure_key(organization)).or_else(|| environment_value(AZURE_ENVIRONMENT))
}

/// The organizations with a saved Azure DevOps token, in order, each without its prefix.
pub fn saved_azure_organizations(path: Option<&Path>) -> Vec<String> {
    let Some(path) = path else { return Vec::new() };
    load(path)
        .map(|tokens| tokens.keys().filter_map(|key| key.strip_prefix(AZURE_PREFIX)).map(str::to_string).collect())
        .unwrap_or_default()
}

/// The token for `host`: the saved one when there is one, else the environment's.
pub fn token_for(path: Option<&Path>, host: &str) -> Option<String> {
    let saved = path.and_then(|path| load(path).ok()).and_then(|tokens| tokens.get(&host.to_ascii_lowercase()).cloned());
    saved.filter(|token| !token.is_empty()).or_else(environment_token)
}

/// Whether a token is saved for `host`, whatever the environment holds.
pub fn has_saved(path: Option<&Path>, host: &str) -> bool {
    path.and_then(|path| load(path).ok()).is_some_and(|tokens| tokens.contains_key(&host.to_ascii_lowercase()))
}

/// Saves `token` for `host`, replacing any saved one. The token must be one line with no spaces.
pub fn save_token(path: &Path, host: &str, token: &str) -> io::Result<()> {
    if !is_plausible_token(token) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "the token has spaces or is empty"));
    }
    let mut tokens = load(path)?;
    tokens.insert(host.to_ascii_lowercase(), token.to_string());
    write(path, &tokens)
}

/// Deletes the saved token for `host`. A host with none saved is not an error.
pub fn remove_token(path: &Path, host: &str) -> io::Result<()> {
    let mut tokens = load(path)?;
    if tokens.remove(&host.to_ascii_lowercase()).is_none() {
        return Ok(());
    }
    write(path, &tokens)
}

/// Writes the tokens to `path`, readable only by its owner on Unix, even when the file existed with
/// wider permissions.
fn write(path: &Path, tokens: &Tokens) -> io::Result<()> {
    let text = ron::to_string(tokens).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?;
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder)?;
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    file.write_all(text.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("temporary folder");
        let path = dir.path().join("forge-tokens.ron");
        (dir, path)
    }

    #[test]
    fn tokens_are_saved_per_host_in_lower_case_and_removed() {
        let (_dir, path) = scratch();
        assert_eq!(load(&path).unwrap(), Tokens::new(), "no file holds none");
        save_token(&path, "Codeberg.org", "first-token").unwrap();
        save_token(&path, "forge.example.com", "second-token").unwrap();
        assert_eq!(token_for(Some(&path), "codeberg.org").as_deref(), Some("first-token"));
        assert!(has_saved(Some(&path), "FORGE.example.com"));
        save_token(&path, "codeberg.org", "replaced-token").unwrap();
        assert_eq!(token_for(Some(&path), "codeberg.org").as_deref(), Some("replaced-token"));
        remove_token(&path, "codeberg.org").unwrap();
        assert!(!has_saved(Some(&path), "codeberg.org"));
        assert!(has_saved(Some(&path), "forge.example.com"), "other hosts keep their tokens");
        remove_token(&path, "codeberg.org").expect("removing a host with no token is not an error");
    }

    #[test]
    fn the_token_file_holds_the_map_and_not_settings() {
        let (_dir, path) = scratch();
        save_token(&path, "codeberg.org", "abc123").unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("codeberg.org") && text.contains("abc123"), "{text}");
    }

    #[test]
    fn a_token_with_spaces_or_no_text_is_refused_and_nothing_is_written() {
        let (_dir, path) = scratch();
        assert!(save_token(&path, "codeberg.org", "has space").is_err());
        assert!(save_token(&path, "codeberg.org", "").is_err());
        assert!(!path.exists());
    }

    #[test]
    fn a_file_that_is_not_tokens_is_not_overwritten() {
        let (_dir, path) = scratch();
        std::fs::write(&path, "this is ( not ron").unwrap();
        assert!(save_token(&path, "codeberg.org", "abc123").is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "this is ( not ron", "the unreadable file is kept");
        assert_eq!(token_for(Some(&path), "codeberg.org"), None, "and it gives no token");
    }

    #[cfg(unix)]
    #[test]
    fn the_token_file_is_readable_only_by_its_owner() {
        use std::os::unix::fs::PermissionsExt;
        let (_dir, path) = scratch();
        save_token(&path, "codeberg.org", "private-token").unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        save_token(&path, "codeberg.org", "private-token-2").unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600, "a wider file is narrowed");
    }

    #[test]
    fn bitbucket_and_azure_credentials_are_kept_under_their_own_keys() {
        let (_dir, path) = scratch();
        save_token(&path, BITBUCKET_KEY, "octo:app-password").unwrap();
        save_token(&path, &azure_key("ACME"), "azure-pat").unwrap();
        assert_eq!(bitbucket_credentials(Some(&path)).as_deref(), Some("octo:app-password"));
        assert_eq!(saved_for(Some(&path), "dev.azure.com/acme").as_deref(), Some("azure-pat"), "the organization is lower-cased");
        assert_eq!(saved_azure_organizations(Some(&path)), vec!["acme".to_string()]);
        assert_eq!(bitbucket_credentials(None), None, "no settings folder, no credentials");
        remove_token(&path, &azure_key("acme")).unwrap();
        assert!(saved_azure_organizations(Some(&path)).is_empty());
        assert!(bitbucket_credentials(Some(&path)).is_some(), "Bitbucket keeps its own entry");
    }

    #[test]
    fn the_test_build_has_no_settings_folder_token_file() {
        assert_eq!(token_file(), None, "tests must not touch the person's settings folder");
    }

    #[test]
    fn a_saved_token_is_used_before_the_environment() {
        let (_dir, path) = scratch();
        save_token(&path, "codeberg.org", "saved-one").unwrap();
        assert_eq!(token_for(Some(&path), "codeberg.org").as_deref(), Some("saved-one"));
        if environment_token().is_none() {
            assert_eq!(token_for(Some(&path), "other.example.com"), None, "no saved token and no environment token");
        }
    }
}
