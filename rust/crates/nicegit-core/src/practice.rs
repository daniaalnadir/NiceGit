//! The practice repository the first-run tour opens: a small repository the person can stage,
//! commit and branch in without touching anything they care about.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::client::GitClient;
use crate::models::Result;

/// The folder name the practice repository is created under.
pub const PRACTICE_FOLDER: &str = "NiceGit practice";

/// The file inside `.git` that marks a folder as a practice repository NiceGit may replace.
pub const PRACTICE_MARKER: &str = "nicegit-practice";

const FALLBACK_NAME: &str = "NiceGit Tour";
const FALLBACK_EMAIL: &str = "tour@nicegit.invalid";

impl GitClient {
    /// Creates a practice repository in `parent/NiceGit practice`, with a few commits on `main`, a
    /// `idea/greeting` branch, and one uncommitted edit to `hello.txt`. A folder from an earlier
    /// practice run (one carrying the marker) is replaced; any other existing folder is left alone
    /// and the next free name is used instead. Returns the new repository's folder.
    pub fn create_practice_repository(&self, parent: &Path, name: &str, email: &str) -> Result<PathBuf> {
        let directory = self.practice_folder(parent)?;
        fs::create_dir_all(&directory)
            .map_err(|error| crate::models::GitError::failed("practice repository", format!("The folder could not be created: {error}")))?;
        self.run(&["init", "-q", "-b", "main"], &directory)?;
        let name = if name.trim().is_empty() { FALLBACK_NAME } else { name.trim() };
        let email = if email.trim().is_empty() { FALLBACK_EMAIL } else { email.trim() };
        self.run(&["config", "--local", "--", "user.name", name], &directory)?;
        self.run(&["config", "--local", "--", "user.email", email], &directory)?;
        self.run(&["config", "--local", "--", "commit.gpgsign", "false"], &directory)?;
        write_file(&directory.join(".git").join(PRACTICE_MARKER), "")?;

        write_file(&directory.join("README.md"), "# Practice repository\n\nA safe place to try NiceGit. Nothing here is shared.\n")?;
        self.commit_all(&directory, "Start the practice repository")?;

        write_file(&directory.join("hello.txt"), "Hello from Sprig.\n")?;
        self.commit_all(&directory, "Say hello")?;

        self.run(&["switch", "-q", "-c", "idea/greeting"], &directory)?;
        append_file(&directory.join("hello.txt"), "Branches let you try ideas.\n")?;
        self.commit_all(&directory, "Try a new greeting")?;
        self.run(&["switch", "-q", "main"], &directory)?;

        write_file(&directory.join("notes.md"), "Things to try\n")?;
        self.commit_all(&directory, "Add notes")?;

        append_file(&directory.join("hello.txt"), "Your first change.\n")?;
        Ok(directory)
    }

    /// The folder to create the practice repository in: the first name under `parent` that is free,
    /// or that holds an earlier practice repository (which is removed first).
    fn practice_folder(&self, parent: &Path) -> Result<PathBuf> {
        for number in 1.. {
            let name = if number == 1 { PRACTICE_FOLDER.to_string() } else { format!("{PRACTICE_FOLDER} {number}") };
            let candidate = parent.join(name);
            if !candidate.exists() {
                return Ok(candidate);
            }
            if candidate.join(".git").join(PRACTICE_MARKER).is_file() {
                fs::remove_dir_all(&candidate).map_err(|error| {
                    crate::models::GitError::failed("practice repository", format!("The old practice folder could not be removed: {error}"))
                })?;
                return Ok(candidate);
            }
        }
        unreachable!("an unbounded range of folder names always yields a free one")
    }

    /// Stages every file in the practice repository and commits it with `message`.
    fn commit_all(&self, directory: &Path, message: &str) -> Result<()> {
        self.run(&["add", "-A", "--", "."], directory)?;
        self.run(&["commit", "-q", "-m", message], directory)?;
        Ok(())
    }
}

fn write_file(path: &Path, contents: &str) -> Result<()> {
    fs::write(path, contents)
        .map_err(|error| crate::models::GitError::failed("practice repository", format!("A file could not be written: {error}")))
}

fn append_file(path: &Path, contents: &str) -> Result<()> {
    let mut file = OpenOptions::new()
        .append(true)
        .open(path)
        .map_err(|error| crate::models::GitError::failed("practice repository", format!("A file could not be changed: {error}")))?;
    file.write_all(contents.as_bytes())
        .map_err(|error| crate::models::GitError::failed("practice repository", format!("A file could not be changed: {error}")))
}
