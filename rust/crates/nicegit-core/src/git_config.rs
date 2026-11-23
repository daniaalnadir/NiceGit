//! The repository's own Git configuration, for the Git settings window: reading its local entries,
//! and setting or removing a single-valued key only while it still reads as the window showed it.

use std::collections::HashMap;
use std::path::Path;

use crate::client::GitClient;
use crate::models::{GitError, Result};
use crate::runner;

const MULTI_VALUED: &str = "This setting has several values. Edit it in a terminal.";
const CHANGED: &str = "This setting changed since it was shown; reload";

/// One key of the repository's local configuration, with every value it holds, in Git's order.
/// A key without a value (such as `[core] bare`) has one empty value, as `git config --get-all` gives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigEntry {
    /// The key as `git config --list` prints it: section and name lowercased, subsection as written.
    pub key: String,
    pub values: Vec<String>,
}

impl ConfigEntry {
    /// Whether the key holds more than one value, which the settings window shows read-only.
    pub fn is_multi_valued(&self) -> bool {
        self.values.len() > 1
    }
}

/// Checks a key before it is written. A key is `section.name` or `section.subsection.name`: the
/// section uses letters, digits, and hyphens and starts with a letter or digit, so it can never read
/// as a Git option; the subsection may hold any text but a line break (branch and remote names such
/// as `feature/x` appear there); and the name starts with a letter and uses letters, digits, and
/// hyphens. Returns the reason the key is refused.
pub fn validate_config_key(key: &str) -> std::result::Result<(), String> {
    let (Some((section, _)), Some((head, name))) = (key.split_once('.'), key.rsplit_once('.')) else {
        return Err("A setting name has a section and a name, such as core.editor.".into());
    };
    let leads_with_alphanumeric = section.chars().next().is_some_and(|c| c.is_ascii_alphanumeric());
    if !leads_with_alphanumeric || !section.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err("The section must start with a letter or digit and use only letters, digits, and hyphens.".into());
    }
    if head.len() > section.len() {
        let subsection = &head[section.len() + 1..];
        if subsection.is_empty() || subsection.contains(['\n', '\0']) {
            return Err("The part between the section and the name cannot be empty or hold a line break.".into());
        }
    }
    let mut chars = name.chars();
    let starts_with_letter = chars.next().is_some_and(|c| c.is_ascii_alphabetic());
    if !starts_with_letter || !chars.all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err("The name must start with a letter and use only letters, digits, and hyphens.".into());
    }
    Ok(())
}

/// Whether two keys name the same setting: Git compares sections and names without case, and
/// subsections exactly. Keys that are not well formed never match.
pub fn same_config_key(left: &str, right: &str) -> bool {
    fn parts(key: &str) -> Option<(String, Option<&str>, String)> {
        let (head, name) = key.rsplit_once('.')?;
        let (section, subsection) = match head.split_once('.') {
            Some((section, subsection)) => (section, Some(subsection)),
            None => (head, None),
        };
        Some((section.to_ascii_lowercase(), subsection, name.to_ascii_lowercase()))
    }
    match (parts(left), parts(right)) {
        (Some(left), Some(right)) => left == right,
        _ => false,
    }
}

/// Splits `git config --list -z` output into one entry per key, keeping Git's order. Each record
/// is `key`, or `key` and a newline and the value, since values may contain newlines.
fn parse_config_list(output: &str) -> Vec<ConfigEntry> {
    let mut entries: Vec<ConfigEntry> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    for record in output.split_terminator('\0') {
        let (key, value) = record.split_once('\n').unwrap_or((record, ""));
        match index.get(key) {
            Some(&position) => entries[position].values.push(value.to_string()),
            None => {
                index.insert(key.to_string(), entries.len());
                entries.push(ConfigEntry { key: key.to_string(), values: vec![value.to_string()] });
            }
        }
    }
    entries
}

fn stale() -> GitError {
    GitError::failed("set config", CHANGED)
}

impl GitClient {
    /// The local configuration of the checkout's repository, with each key's values.
    pub fn config_entries(&self, directory: &Path) -> Result<Vec<ConfigEntry>> {
        Ok(parse_config_list(&self.run(&["config", "--local", "-z", "--list"], directory)?))
    }

    /// The values the repository's configuration holds for `key` right now, read the way Git
    /// reads a key. Empty when the key is not set.
    fn stored_config_values(&self, key: &str, directory: &Path) -> Result<Vec<String>> {
        let output = self.run_accepting(&["config", "--local", "-z", "--get-all", "--", key], directory, &[0, 1])?;
        Ok(output.split_terminator('\0').map(str::to_string).collect())
    }

    /// Sets a single-valued key to `value`, or adds the key when `expected` is empty. Refuses unless
    /// the key's values right now equal `expected`, the values the window showed, so a stale window
    /// cannot overwrite a newer change. A key with several values is never written here.
    pub fn set_config_value(&self, directory: &Path, key: &str, expected: &[String], value: &str) -> Result<()> {
        validate_config_key(key).map_err(|reason| GitError::failed("set config", reason))?;
        let current = self.stored_config_values(key, directory)?;
        if expected.len() > 1 || current.len() > 1 {
            return Err(GitError::failed("set config", MULTI_VALUED));
        }
        if current != expected {
            return Err(stale());
        }
        // `--` keeps a value that starts with a dash from reading as an option.
        self.run(&["config", "--local", "--replace-all", "--", key, value], directory).map(drop)
    }

    /// Removes a single-valued key. Refuses unless the key is set and still reads as `expected`.
    pub fn unset_config_value(&self, directory: &Path, key: &str, expected: &[String]) -> Result<()> {
        validate_config_key(key).map_err(|reason| GitError::failed("remove config", reason))?;
        let current = self.stored_config_values(key, directory)?;
        if expected.len() > 1 || current.len() > 1 {
            return Err(GitError::failed("remove config", MULTI_VALUED));
        }
        if current.is_empty() || current != expected {
            return Err(GitError::failed("remove config", CHANGED));
        }
        self.run(&["config", "--local", "--unset", "--", key], directory).map(drop)
    }

    /// The effective global value of `key`, as `git config --global --get` reports it, or `None`
    /// when the key is not set globally. Global configuration is only ever read here.
    pub fn global_config_value(&self, key: &str, directory: &Path) -> Result<Option<String>> {
        let output = self.run_accepting(&["config", "--global", "--get", "--", key], directory, &[0, 1])?;
        let value = runner::strip_line_terminator(output);
        Ok((!value.is_empty()).then_some(value))
    }
}

/// The configuration file holding an empty `gpg.format`, when that is what Git's `error` reports.
/// Git rejects an empty `gpg.format` whenever it signs a commit or checks a signature: "invalid value
/// for 'gpg.format': ''", then "bad config variable 'gpg.format' in file '…' at line …". `None` for any other error, including a non-empty value
/// Git does not know, which only the person can correct.
pub fn empty_gpg_format_file(error: &str) -> Option<String> {
    if !error.contains("invalid value for 'gpg.format': ''") {
        return None;
    }
    let start = error.find("bad config variable 'gpg.format' in file '")? + "bad config variable 'gpg.format' in file '".len();
    let rest = &error[start..];
    let file = &rest[..rest.find("' at line ")?];
    (!file.is_empty() && !file.contains('\n')).then(|| file.to_string())
}

impl GitClient {
    /// Removes an empty `gpg.format` from `file`, as `empty_gpg_format_file` names it; a relative
    /// file is read from `directory`, where Git reported it. An empty value means the same as no
    /// value to a Git that accepts it, so nothing about signing changes. Refuses when the file holds
    /// a value that is not empty; succeeds without writing when the setting is already gone.
    pub fn remove_empty_gpg_format(&self, file: &str, directory: &Path) -> Result<()> {
        let path = directory.join(file);
        let path = path.to_string_lossy();
        let output = self.run_accepting(&["config", "--file", &path, "-z", "--get-all", "gpg.format"], directory, &[0, 1])?;
        let values: Vec<&str> = output.split_terminator('\0').collect();
        if values.is_empty() {
            return Ok(());
        }
        if values.iter().any(|value| !value.trim().is_empty()) {
            return Err(GitError::failed("remove gpg.format", "gpg.format is no longer empty in that file; it was left as it is."));
        }
        self.run(&["config", "--file", &path, "--unset-all", "gpg.format"], directory).map(drop)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_gpg_format_names_its_file() {
        let windows = "error: invalid value for 'gpg.format': ''\nfatal: bad config variable 'gpg.format' in file 'C:/Users/Daniaal Nadir/.gitconfig' at line 8\n";
        assert_eq!(empty_gpg_format_file(windows).as_deref(), Some("C:/Users/Daniaal Nadir/.gitconfig"));
        let quoted = "error: invalid value for 'gpg.format': ''\nfatal: bad config variable 'gpg.format' in file '/home/o'neil/.gitconfig' at line 3";
        assert_eq!(empty_gpg_format_file(quoted).as_deref(), Some("/home/o'neil/.gitconfig"));
        let unknown =
            "error: invalid value for 'gpg.format': 'pgp'\nfatal: bad config variable 'gpg.format' in file '.git/config' at line 3";
        assert_eq!(empty_gpg_format_file(unknown), None);
        assert_eq!(empty_gpg_format_file("fatal: not a git repository"), None);
    }

    #[test]
    fn removing_an_empty_gpg_format_leaves_other_settings() {
        let folder = std::env::temp_dir().join(format!("nicegit-gpg-format-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let file = folder.join("config");
        std::fs::write(&file, "[user]\n\tname = Someone\n[gpg]\n\tformat =\n").unwrap();
        let client = GitClient::new();
        client.remove_empty_gpg_format("config", &folder).unwrap();
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.contains("name = Someone"), "{text}");
        assert!(!text.contains("format"), "{text}");
        client.remove_empty_gpg_format("config", &folder).unwrap();
        std::fs::write(&file, "[gpg]\n\tformat = ssh\n").unwrap();
        assert!(client.remove_empty_gpg_format("config", &folder).is_err());
        assert!(std::fs::read_to_string(&file).unwrap().contains("format = ssh"));
        std::fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn keys_must_name_a_section_and_a_name() {
        for key in [
            "user.name",
            "core.editor",
            "remote.origin.url",
            "branch.feature-x.rebase",
            "a.b.c.d1",
            "core.name-",
            "branch.feature/x y.rebase",
        ] {
            assert!(validate_config_key(key).is_ok(), "{key} should be accepted");
        }
        for key in
            ["", "nodot", ".name", "section.", "a..b", "-x.name", "core.1name", "core.na me", "a.b.c_d", "a_b.name", "branch.x\ny.rebase"]
        {
            assert!(validate_config_key(key).is_err(), "{key:?} should be refused");
        }
    }

    #[test]
    fn same_key_ignores_case_except_in_subsections() {
        assert!(same_config_key("User.Name", "user.name"));
        assert!(same_config_key("remote.Origin.url", "REMOTE.Origin.URL"));
        assert!(!same_config_key("remote.Origin.url", "remote.origin.url"));
        assert!(!same_config_key("user.name", "user.email"));
        assert!(!same_config_key("nodot", "nodot"));
    }

    #[test]
    fn list_output_keeps_newlines_in_values_and_groups_repeated_keys() {
        let output = "core.bare\0user.name\nTest\0remote.origin.fetch\n+a:b\0remote.origin.fetch\n+c:d\0nicegit.note\nfirst\nsecond\0";
        let entries = parse_config_list(output);
        assert_eq!(entries.len(), 4);
        assert_eq!(entries[0], ConfigEntry { key: "core.bare".into(), values: vec![String::new()] });
        assert_eq!(entries[1].values, vec!["Test".to_string()]);
        assert!(entries[2].is_multi_valued());
        assert_eq!(entries[2].values, vec!["+a:b".to_string(), "+c:d".to_string()]);
        assert_eq!(entries[3].values, vec!["first\nsecond".to_string()]);
        assert!(parse_config_list("").is_empty());
    }
}
