//! Git commands, output parsers, and commit graph layout for NiceGit.
//!
//! NiceGit drives the `git` command-line program rather than a Git library, so it behaves
//! exactly like the user's own Git, including their configuration, hooks, and credentials.

pub mod client;
pub mod diff;
pub mod graph;
pub mod models;
pub mod parsers;
pub mod runner;

pub use client::GitClient;
pub use models::*;
