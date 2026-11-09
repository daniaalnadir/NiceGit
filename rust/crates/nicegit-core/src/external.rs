//! The external diff and merge tools Git is configured to use (`diff.tool` and `merge.tool`), and
//! opening a file in them the way `git difftool` and `git mergetool` do.

use std::io;
use std::path::Path;
use std::process::{Child, ExitStatus};

use crate::client::GitClient;
use crate::models::{GitError, Result};
use crate::runner;

/// The tools that `diff.tool` and `merge.tool` name, when they are set to a non-empty name.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExternalTools {
    pub diff: Option<String>,
    pub merge: Option<String>,
}

impl GitClient {
    /// The diff and merge tools the configuration names. Git reads the system, global, and
    /// repository configuration here, so a tool set in any of them is found.
    pub fn configured_tools(&self, directory: &Path) -> Result<ExternalTools> {
        // With `--null`, each entry is `key`, a newline, `value`, then a NUL. Exit status 1 means
        // that no key matched. Entries come in precedence order, so the last one wins, as in Git:
        // an empty value in the repository overrides a tool set globally.
        let output = self.run_accepting(&["config", "--null", "--get-regexp", r"^(diff|merge)\.tool$"], directory, &[0, 1])?;
        let mut tools = ExternalTools::default();
        for entry in output.split('\0') {
            let (key, value) = entry.split_once('\n').unwrap_or((entry, ""));
            let value = value.trim();
            let value = (!value.is_empty()).then(|| value.to_string());
            match key {
                "diff.tool" => tools.diff = value,
                "merge.tool" => tools.merge = value,
                _ => {}
            }
        }
        Ok(tools)
    }

    /// The name in `diff.tool`, if one is set.
    pub fn configured_diff_tool(&self, directory: &Path) -> Option<String> {
        self.configured_tools(directory).ok()?.diff
    }

    /// The name in `merge.tool`, if one is set.
    pub fn configured_merge_tool(&self, directory: &Path) -> Option<String> {
        self.configured_tools(directory).ok()?.merge
    }

    /// Opens `path` in the configured diff tool, as `git difftool --no-prompt` does: `staged`
    /// compares the index with HEAD, otherwise the working file with the index. The tool is
    /// started and this returns without waiting for it; the child is reaped on a thread.
    pub fn open_in_difftool(&self, path: &str, staged: bool, directory: &Path) -> Result<()> {
        let mut arguments = vec!["difftool", "--no-prompt"];
        if staged {
            arguments.push("--cached");
        }
        arguments.extend(["--", path]);
        let child = runner::spawn(&arguments, directory, &self.options())?;
        wait_on_thread(child, |_| {});
        Ok(())
    }

    /// Opens a conflicted file in the configured merge tool, as `git mergetool --no-prompt` does.
    /// Refuses a file that is not conflicted. The tool is started and this returns without waiting;
    /// `on_exit` runs on a background thread with the tool's exit status once it has closed, so the
    /// caller can reread the repository, since the tool may have resolved the conflict.
    pub fn open_in_mergetool(
        &self,
        path: &str,
        directory: &Path,
        on_exit: impl FnOnce(io::Result<ExitStatus>) + Send + 'static,
    ) -> Result<()> {
        if self.configured_merge_tool(directory).is_none() {
            return Err(GitError::failed("mergetool", "No merge tool is set. Use `git config --global merge.tool <name>`."));
        }
        self.require_conflict(path, directory)?;
        let child = runner::spawn(&["mergetool", "--no-prompt", "--", path], directory, &self.options())?;
        wait_on_thread(child, on_exit);
        Ok(())
    }
}

/// Waits for a child on a background thread, so it is reaped without holding up the caller.
fn wait_on_thread(mut child: Child, on_exit: impl FnOnce(io::Result<ExitStatus>) + Send + 'static) {
    std::thread::spawn(move || on_exit(child.wait()));
}
