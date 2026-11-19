//! Claude's commit message drafts: the model the person picks, where their API key is kept, and the
//! drafting itself, which runs on a background thread. Nothing here is sent anywhere unless the
//! person turns drafts on and clicks Draft.

use std::io::{self, Write};
use std::path::{Path, PathBuf};

use nicegit_core::ai_message::{self, Cancel, Drafted, Options};
use serde::{Deserialize, Serialize};

/// The environment variable that holds the key. When it is set, it is used before a saved key.
pub const KEY_ENVIRONMENT: &str = "ANTHROPIC_API_KEY";
/// The file in NiceGit's settings folder that holds the key when it is saved there.
#[cfg(not(test))]
const KEY_FILE: &str = "anthropic-key";
/// The longest text accepted as a key.
const LONGEST_KEY: usize = 512;

/// The Claude model a draft uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ClaudeModel {
    #[default]
    Opus,
    Sonnet,
    Haiku,
}

impl ClaudeModel {
    pub const ALL: [ClaudeModel; 3] = [ClaudeModel::Opus, ClaudeModel::Sonnet, ClaudeModel::Haiku];

    /// The model name the API takes.
    pub fn id(self) -> &'static str {
        match self {
            ClaudeModel::Opus => "claude-opus-5-5",
            ClaudeModel::Sonnet => "claude-sonnet-5-5",
            ClaudeModel::Haiku => "claude-haiku-5-5",
        }
    }

    /// The name Settings shows.
    pub fn title(self) -> &'static str {
        match self {
            ClaudeModel::Opus => "Claude Opus 5.5 (best)",
            ClaudeModel::Sonnet => "Claude Sonnet 5.5",
            ClaudeModel::Haiku => "Claude Haiku 5.5 (fastest)",
        }
    }
}

/// Where the saved key lives. Tests have no settings folder, so they never reach the person's.
pub fn key_file() -> Option<PathBuf> {
    #[cfg(not(test))]
    {
        eframe::storage_dir("nicegit").map(|dir| dir.join(KEY_FILE))
    }
    #[cfg(test)]
    {
        None
    }
}

/// The key from the environment, when it is set and not blank.
pub fn environment_key() -> Option<String> {
    std::env::var(KEY_ENVIRONMENT).ok().map(|key| key.trim().to_string()).filter(|key| !key.is_empty())
}

/// The key saved at `path`, when there is one.
pub fn saved_key(path: &Path) -> Option<String> {
    std::fs::read_to_string(path).ok().map(|text| text.trim().to_string()).filter(|key| !key.is_empty())
}

/// The key a draft uses: the environment's when it is set, else the one saved in `file`.
pub fn resolve_key(file: Option<&Path>) -> Option<String> {
    environment_key().or_else(|| file.and_then(saved_key))
}

/// Whether `text` could be an API key: printable ASCII with no spaces, and not too long.
pub fn is_plausible_key(text: &str) -> bool {
    !text.is_empty() && text.len() <= LONGEST_KEY && text.bytes().all(|byte| byte.is_ascii_graphic())
}

/// Saves `key` at `path`, replacing any saved key. On Unix the file is readable only by its owner.
pub fn save_key(path: &Path, key: &str) -> io::Result<()> {
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
        // A file that already existed keeps its old mode unless it is set here.
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    file.write_all(key.as_bytes())
}

/// Deletes the saved key at `path`. A key that is not there already is not an error.
pub fn remove_key(path: &Path) -> io::Result<()> {
    match std::fs::remove_file(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

/// One drafting request, as the background thread receives it.
pub struct Job {
    pub repo: PathBuf,
    pub model: ClaudeModel,
    /// Whether the summary takes a Conventional Commits type.
    pub conventional: bool,
    pub key_file: Option<PathBuf>,
    pub cancel: Cancel,
}

/// The function that makes a draft. The app uses `draft_with_claude`; tests put in a stand-in.
pub type Drafter = fn(Job) -> Result<Drafted, String>;

/// Drafts a message with Claude, using the key from the environment or the saved file.
pub fn draft_with_claude(job: Job) -> Result<Drafted, String> {
    let key = resolve_key(job.key_file.as_deref()).ok_or_else(|| "No API key is set. Add one under Settings, in Claude.".to_string())?;
    let options = Options { model: job.model.id(), key: &key, conventional: job.conventional };
    ai_message::draft_message(&job.repo, &options, &job.cancel)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("temporary folder");
        let path = dir.path().join(name);
        (dir, path)
    }

    #[test]
    fn models_name_their_api_ids_and_default_to_opus() {
        assert_eq!(ClaudeModel::default(), ClaudeModel::Opus);
        assert_eq!(ClaudeModel::Opus.id(), "claude-opus-5-5");
        assert_eq!(ClaudeModel::Sonnet.id(), "claude-sonnet-5-5");
        assert_eq!(ClaudeModel::Haiku.id(), "claude-haiku-5-5");
    }

    #[test]
    fn a_saved_key_is_read_trimmed_and_an_empty_file_has_none() {
        let (_dir, path) = scratch("anthropic-key");
        assert_eq!(saved_key(&path), None, "no file");
        save_key(&path, "  sk-ant-test-1  \n").expect("save");
        assert_eq!(saved_key(&path).as_deref(), Some("sk-ant-test-1"));
        std::fs::write(&path, "  \n").unwrap();
        assert_eq!(saved_key(&path), None, "a blank file holds no key");
    }

    #[test]
    fn saving_replaces_the_old_key_and_removing_twice_is_fine() {
        let (_dir, path) = scratch("anthropic-key");
        save_key(&path, "sk-ant-first").unwrap();
        save_key(&path, "sk-ant-second-longer").unwrap();
        assert_eq!(saved_key(&path).as_deref(), Some("sk-ant-second-longer"));
        remove_key(&path).expect("remove");
        assert_eq!(saved_key(&path), None);
        remove_key(&path).expect("removing a missing key is not an error");
    }

    #[cfg(unix)]
    #[test]
    fn the_saved_key_is_readable_only_by_its_owner() {
        use std::os::unix::fs::PermissionsExt;
        let (_dir, path) = scratch("anthropic-key");
        save_key(&path, "sk-ant-private").unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        // A file that already existed with wider permissions is narrowed.
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        save_key(&path, "sk-ant-private-2").unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
    }

    #[test]
    fn only_printable_keys_without_spaces_are_accepted() {
        assert!(is_plausible_key("sk-ant-api03-abc_DEF-123"));
        for text in ["", "has space", "tab\there", "new\nline", "é-accented", &"k".repeat(LONGEST_KEY + 1)] {
            assert!(!is_plausible_key(text), "{text:?}");
        }
    }

    #[test]
    fn the_test_build_has_no_settings_folder_key_file() {
        assert_eq!(key_file(), None, "tests must not touch the person's settings folder");
    }

    #[test]
    fn a_file_key_is_used_when_the_environment_has_none() {
        let (_dir, path) = scratch("anthropic-key");
        save_key(&path, "sk-ant-from-file").unwrap();
        if environment_key().is_none() {
            assert_eq!(resolve_key(Some(&path)).as_deref(), Some("sk-ant-from-file"));
        }
        assert_eq!(resolve_key(None).is_some(), environment_key().is_some());
    }

    #[test]
    fn drafting_without_any_key_says_how_to_add_one() {
        let (_dir, missing) = scratch("anthropic-key");
        if environment_key().is_none() {
            let job = Job {
                repo: PathBuf::from("."),
                model: ClaudeModel::Haiku,
                conventional: false,
                key_file: Some(missing),
                cancel: Cancel::new(),
            };
            let problem = draft_with_claude(job).expect_err("no key is set");
            assert!(problem.contains("No API key is set"), "{problem}");
        }
    }
}
