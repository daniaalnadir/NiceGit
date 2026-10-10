//! Git commands, output parsers, and commit graph layout for NiceGit.
//!
//! NiceGit drives the `git` command-line program rather than a Git library, so it behaves
//! exactly like the user's own Git, including their configuration, hooks, and credentials.

pub mod bisect;
pub mod blame;
pub mod cleanup;
pub mod client;
pub mod compare;
pub mod conflict;
pub mod diff;
pub mod file_history;
pub mod gitflow;
pub mod github;
pub mod graph;
pub mod inline;
pub mod lfs;
pub mod merge_preview;
pub mod models;
pub mod parsers;
pub mod rebase;
pub mod reflog;
pub mod restore;
pub mod runner;
pub mod search;
pub mod settings;
pub mod signature;
pub mod staging;
pub mod submodule;
pub mod undo;
pub mod worktree;

pub use client::GitClient;
pub use models::*;
