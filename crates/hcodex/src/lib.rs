//! Rust-native Heterarchy Codex application boundaries.
pub type Error = Box<dyn std::error::Error + Send + Sync>;
pub type Result<T> = std::result::Result<T, Error>;

pub mod catalog;
pub mod context;
pub mod dag;
pub mod evidence;
pub mod gateway;
mod git;
pub mod http_runtime;
pub mod lifecycle;
pub mod recovery;
mod recovery_artifacts;
pub mod research;
pub mod research_cli;
pub mod routing;
pub mod runtime;
pub mod storage;
pub mod supervisor;
pub mod terminal_adapter;
pub mod triad;
pub mod tui;
pub(crate) mod verification;
pub mod workbench;

pub mod workspace_cli;
pub mod workspaces;

pub mod execution_policy;
