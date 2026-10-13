//! The files Git tracks in a checkout, for the Go to file finder.

use std::path::Path;

use crate::client::GitClient;
use crate::models::Result;

impl GitClient {
    /// Every tracked file's path relative to the repository root, in Git's order. Untracked and
    /// ignored files are not listed. Paths can contain newlines, so records are split on NUL.
    pub fn tracked_files(&self, directory: &Path) -> Result<Vec<String>> {
        let root = self.repository_root(directory)?;
        let output = self.run(&["ls-files", "-z", "--full-name"], &root)?;
        let mut paths: Vec<String> = output.split('\0').filter(|path| !path.is_empty()).map(str::to_string).collect();
        // A conflicted path is listed once for each of its stages, one after another.
        paths.dedup();
        Ok(paths)
    }
}
