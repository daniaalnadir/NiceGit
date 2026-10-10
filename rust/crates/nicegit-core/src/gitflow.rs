//! GitFlow: production, development, feature, release, and hotfix branches. Settings use the same
//! `gitflow.*` keys as the git-flow command-line tool, so both can manage one repository.

use std::path::Path;

use crate::client::GitClient;
use crate::models::{GitError, Result, StatusKind};

/// The settings for a repository's GitFlow branches.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitFlowConfiguration {
    pub main_branch: String,
    pub develop_branch: String,
    pub feature_prefix: String,
    pub release_prefix: String,
    pub hotfix_prefix: String,
    pub version_tag_prefix: String,
}

impl Default for GitFlowConfiguration {
    fn default() -> Self {
        Self {
            main_branch: "main".to_string(),
            develop_branch: "develop".to_string(),
            feature_prefix: "feature/".to_string(),
            release_prefix: "release/".to_string(),
            hotfix_prefix: "hotfix/".to_string(),
            version_tag_prefix: String::new(),
        }
    }
}

impl GitFlowConfiguration {
    /// The prefix of branches of `kind`, such as `feature/`.
    pub fn prefix(&self, kind: GitFlowKind) -> &str {
        match kind {
            GitFlowKind::Feature => &self.feature_prefix,
            GitFlowKind::Release => &self.release_prefix,
            GitFlowKind::Hotfix => &self.hotfix_prefix,
        }
    }

    /// The branch a new branch of `kind` starts from: hotfixes start from production, others from development.
    pub fn start_branch(&self, kind: GitFlowKind) -> &str {
        match kind {
            GitFlowKind::Hotfix => &self.main_branch,
            _ => &self.develop_branch,
        }
    }

    /// The flow kind and version or name of a GitFlow branch, or None for any other branch.
    pub fn classify<'a>(&self, branch: &'a str) -> Option<(GitFlowKind, &'a str)> {
        GitFlowKind::ALL
            .into_iter()
            .find_map(|kind| branch.strip_prefix(self.prefix(kind)).filter(|rest| !rest.is_empty()).map(|rest| (kind, rest)))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GitFlowKind {
    Feature,
    Release,
    Hotfix,
}

impl GitFlowKind {
    pub const ALL: [GitFlowKind; 3] = [GitFlowKind::Feature, GitFlowKind::Release, GitFlowKind::Hotfix];

    /// The word git-flow uses for this kind, such as `feature`.
    pub fn name(self) -> &'static str {
        match self {
            GitFlowKind::Feature => "feature",
            GitFlowKind::Release => "release",
            GitFlowKind::Hotfix => "hotfix",
        }
    }
}

impl GitClient {
    /// The repository's GitFlow settings, or None when GitFlow is not set up.
    pub fn gitflow_configuration(&self, directory: &Path) -> Result<Option<GitFlowConfiguration>> {
        let (Some(main_branch), Some(develop_branch)) =
            (self.config("gitflow.branch.master", false, directory)?, self.config("gitflow.branch.develop", false, directory)?)
        else {
            return Ok(None);
        };
        let defaults = GitFlowConfiguration::default();
        let value = |key: &str, default: String| -> Result<String> {
            Ok(self.config(&format!("gitflow.{key}"), false, directory)?.unwrap_or(default))
        };
        Ok(Some(GitFlowConfiguration {
            main_branch,
            develop_branch,
            feature_prefix: value("prefix.feature", defaults.feature_prefix)?,
            release_prefix: value("prefix.release", defaults.release_prefix)?,
            hotfix_prefix: value("prefix.hotfix", defaults.hotfix_prefix)?,
            version_tag_prefix: value("prefix.versiontag", defaults.version_tag_prefix)?,
        }))
    }

    /// Saves the settings and creates the development branch from production if it is missing.
    pub fn initialize_gitflow(&self, configuration: &GitFlowConfiguration, directory: &Path) -> Result<()> {
        let config = GitFlowConfiguration {
            main_branch: configuration.main_branch.trim().to_string(),
            develop_branch: configuration.develop_branch.trim().to_string(),
            feature_prefix: configuration.feature_prefix.trim().to_string(),
            release_prefix: configuration.release_prefix.trim().to_string(),
            hotfix_prefix: configuration.hotfix_prefix.trim().to_string(),
            version_tag_prefix: configuration.version_tag_prefix.trim().to_string(),
        };
        for branch in [&config.main_branch, &config.develop_branch] {
            self.run(&["check-ref-format", "--branch", branch], directory)?;
        }
        if config.main_branch == config.develop_branch {
            return Err(GitError::failed("gitflow init", "The main and develop branches must be different."));
        }
        let main_reference = format!("refs/heads/{}", config.main_branch);
        let develop_reference = format!("refs/heads/{}", config.develop_branch);
        self.run(&["show-ref", "--verify", "--quiet", &main_reference], directory).map_err(|_| {
            GitError::failed("gitflow init", format!("The production branch {} does not exist. Create it first.", config.main_branch))
        })?;
        if self.run(&["show-ref", "--verify", "--quiet", &develop_reference], directory).is_err() {
            self.run(&["branch", "--no-track", "--", &config.develop_branch, &main_reference], directory)?;
        }
        let settings = [
            ("branch.master", config.main_branch.as_str()),
            ("branch.develop", config.develop_branch.as_str()),
            ("prefix.feature", config.feature_prefix.as_str()),
            ("prefix.release", config.release_prefix.as_str()),
            ("prefix.hotfix", config.hotfix_prefix.as_str()),
            ("prefix.versiontag", config.version_tag_prefix.as_str()),
        ];
        for (key, value) in settings {
            self.run(&["config", "--local", &format!("gitflow.{key}"), value], directory)?;
        }
        Ok(())
    }

    /// Creates a feature, release, or hotfix branch from development (or production, for hotfixes)
    /// and checks it out. Returns the new branch's name.
    pub fn start_gitflow(
        &self,
        kind: GitFlowKind,
        name: &str,
        expected_branch: &str,
        expected_head: Option<&str>,
        directory: &Path,
    ) -> Result<String> {
        let configuration = self.flow_settings(directory)?;
        self.require_clean_flow_checkout(expected_branch, expected_head, "gitflow start", directory)?;
        let name = name.trim();
        if name.is_empty() {
            return Err(GitError::EmptyBranchName);
        }
        let branch = format!("{}{name}", configuration.prefix(kind));
        self.run(&["check-ref-format", "--branch", &branch], directory)?;
        let base = format!("refs/heads/{}", configuration.start_branch(kind));
        self.run(&["switch", "--no-overwrite-ignore", "--no-track", "--create", &branch, &base], directory)?;
        Ok(branch)
    }

    /// Finishes the checked-out GitFlow branch. Features merge into development; releases and
    /// hotfixes merge into production, are tagged, then merge into development. Merges never
    /// fast-forward. The branch is deleted only after every merge has succeeded.
    ///
    /// A stopped finish can be repeated: targets that already contain the branch are skipped, and
    /// a version tag is reused only if it already includes the branch. A conflicting merge stops
    /// with the merge in progress; resolve it, then finish again.
    pub fn finish_gitflow(
        &self,
        expected_branch: &str,
        expected_head: Option<&str>,
        tag_message: Option<&str>,
        directory: &Path,
    ) -> Result<String> {
        let configuration = self.flow_settings(directory)?;
        self.require_clean_flow_checkout(expected_branch, expected_head, "gitflow finish", directory)?;
        let Some((kind, name)) = configuration.classify(expected_branch) else {
            return Err(GitError::failed(
                "gitflow finish",
                format!("{expected_branch} is not a GitFlow feature, release, or hotfix branch."),
            ));
        };
        let name = name.to_string();
        let branch_reference = format!("refs/heads/{expected_branch}");
        let tip = self.run_trimmed(&["rev-parse", "--verify", "--end-of-options", &branch_reference], directory)?;
        let develop = configuration.develop_branch.clone();
        let targets: Vec<String> = match kind {
            GitFlowKind::Feature => vec![develop.clone()],
            GitFlowKind::Release | GitFlowKind::Hotfix => vec![configuration.main_branch.clone(), develop.clone()],
        };

        let tag = format!("{}{name}", configuration.version_tag_prefix);
        let tag_reference = format!("refs/tags/{tag}");
        let mut needs_tag = false;
        if kind != GitFlowKind::Feature {
            self.run(&["check-ref-format", &tag_reference], directory)?;
            if self.run(&["show-ref", "--verify", "--quiet", &tag_reference], directory).is_ok() {
                // A tag from an earlier, interrupted finish is reused only if it already includes this branch.
                if !self.flow_contains(&tip, &format!("{tag_reference}^{{commit}}"), directory) {
                    return Err(GitError::failed("gitflow finish", format!("The tag {tag} already exists for other work.")));
                }
            } else {
                needs_tag = true;
            }
        }

        for (index, target) in targets.iter().enumerate() {
            let target_reference = format!("refs/heads/{target}");
            if !self.flow_contains(&tip, &target_reference, directory) {
                self.run(&["switch", "--no-overwrite-ignore", "--", target], directory)?;
                self.require_no_ignored_merge_collisions(&tip, directory)?;
                if self.run(&["merge", "--no-ff", "--no-edit", "--no-overwrite-ignore", &tip], directory).is_err() {
                    return Err(GitError::failed(
                        "gitflow finish",
                        format!(
                            "Merging {expected_branch} into {target} stopped, usually for a conflict. Resolve it and continue the merge, then check out {expected_branch} and finish again; it has been kept."
                        ),
                    ));
                }
            }
            if needs_tag && index == 0 {
                let message = tag_message
                    .map(str::trim)
                    .filter(|message| !message.is_empty())
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("{} {name}", if kind == GitFlowKind::Release { "Release" } else { "Hotfix" }));
                self.create_tag(&tag, &target_reference, Some(message.as_str()), directory)?;
            }
        }

        let last = targets.last().map(String::as_str).unwrap_or(develop.as_str());
        self.run(&["switch", "--no-overwrite-ignore", "--", last], directory)?;
        self.require_branch_tip(expected_branch, &tip, directory)?;
        self.run(&["branch", "--delete", "--", expected_branch], directory)?;
        Ok(match kind {
            GitFlowKind::Feature => format!("Finished {expected_branch} into {develop}."),
            _ => format!("Finished {expected_branch} and tagged {tag}."),
        })
    }

    fn flow_contains(&self, commit: &str, of: &str, directory: &Path) -> bool {
        self.run(&["merge-base", "--is-ancestor", commit, of], directory).is_ok()
    }

    fn flow_settings(&self, directory: &Path) -> Result<GitFlowConfiguration> {
        self.gitflow_configuration(directory)?.ok_or_else(|| GitError::failed("gitflow", "Set up GitFlow for this repository first."))
    }

    /// The checkout must be the one shown, with no Git operation and no changes to tracked files.
    /// Untracked files are allowed, as in git-flow.
    fn require_clean_flow_checkout(
        &self,
        expected_branch: &str,
        expected_head: Option<&str>,
        command: &str,
        directory: &Path,
    ) -> Result<()> {
        let state = self.require_checkout(expected_branch, expected_head, command, directory)?;
        if state.operation.is_some() || self.load_status(directory)?.iter().any(|entry| entry.kind != StatusKind::Untracked) {
            return Err(GitError::failed(command, "Commit or stash your changes and finish any Git operation first."));
        }
        Ok(())
    }
}
