//! Save as ZIP: writes one commit's files as a ZIP archive in the background. It is a tool window
//! rather than an app action because it only reads the repository, so it neither refreshes the
//! repository nor takes its busy state. It reports the result as a notice and then closes.

use std::path::{Path, PathBuf};

use nicegit_core::{GitClient, GitError};

use super::{query, Ctx, Task, ToolWindow};

/// The top folder name of a commit's archive, which also names the default file: the
/// repository's folder name and the commit's short hash, cleaned to one safe path component.
pub fn archive_prefix(repo: &Path, short_hash: &str) -> String {
    let folder = repo.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    let safe: String = folder.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') { c } else { '-' }).collect();
    let mut safe = safe.trim_start_matches(['-', '.']).to_string();
    // The archive refuses ".." anywhere in its folder name.
    while safe.contains("..") {
        safe = safe.replace("..", ".");
    }
    if safe.is_empty() {
        safe = "repository".into();
    }
    format!("{safe}-{short_hash}")
}

/// Writes `commit` to `destination` as a ZIP inside the folder `prefix`, then reports the result.
pub struct SaveZipWindow {
    repo: PathBuf,
    commit: String,
    prefix: String,
    destination: PathBuf,
    task: Option<Task<nicegit_core::Result<u64>>>,
    finished: bool,
}

impl SaveZipWindow {
    pub fn new(repo: PathBuf, commit: String, prefix: String, destination: PathBuf) -> Self {
        Self { repo, commit, prefix, destination, task: None, finished: false }
    }
}

impl ToolWindow for SaveZipWindow {
    fn id(&self) -> String {
        format!("save-zip:{}", self.destination.display())
    }

    fn title(&self) -> String {
        "Save as ZIP".into()
    }

    fn default_size(&self) -> egui::Vec2 {
        egui::vec2(420.0, 110.0)
    }

    fn wants_close(&self) -> bool {
        self.finished
    }

    fn ui(&mut self, ui: &mut egui::Ui, cx: &mut Ctx) {
        if self.task.is_none() {
            let (commit, prefix, destination) = (self.commit.clone(), self.prefix.clone(), self.destination.clone());
            let task = query(ui.ctx(), &self.repo, move |client: &GitClient, repo: &Path| {
                client.archive_commit(&commit, &prefix, &destination, repo)?;
                let size = std::fs::metadata(&destination).map_err(|error| GitError::failed("archive", error.to_string()))?.len();
                Ok(size)
            });
            self.task = Some(task);
        }
        let name = self.destination.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let Some(task) = self.task.as_mut() else { return };
        match task.get() {
            None => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(format!("Writing {name}…"));
                });
            }
            Some(Ok(size)) => {
                cx.notice(format!("Saved {name} ({}).", crate::tools::compare::human_size(*size as usize)), false);
                self.finished = true;
            }
            Some(Err(error)) => {
                cx.notice(error.to_string(), true);
                self.finished = true;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archive_prefix_is_one_safe_component() {
        assert_eq!(archive_prefix(Path::new("/work/my repo"), "abc1234"), "my-repo-abc1234");
        assert_eq!(archive_prefix(Path::new("/work/..hidden"), "abc1234"), "hidden-abc1234");
        assert_eq!(archive_prefix(Path::new("/work/a..b"), "abc1234"), "a.b-abc1234");
        assert_eq!(archive_prefix(Path::new("/"), "abc1234"), "repository-abc1234");
    }
}
