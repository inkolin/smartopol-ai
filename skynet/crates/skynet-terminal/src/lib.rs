//! skynet-terminal — interactive PTY terminal sessions for Skynet agents.
//!
//! Provides three execution modes:
//! - `OneShot`: fire-and-forget command via `exec` (async, with timeout + safety)
//! - `Interactive`: persistent PTY session (SSH, sudo, vim, …)
//! - `Background`: long-running process tracked by `JobId`
//!
//! # Quick start
//!
//! ```rust,no_run
//! use skynet_terminal::manager::TerminalManager;
//! use skynet_terminal::types::ExecOptions;
//!
//! #[tokio::main]
//! async fn main() {
//!     let mut mgr = TerminalManager::new();
//!
//!     // Safe one-shot exec with a 30-second timeout.
//!     let result = mgr.exec("echo hello", ExecOptions::default()).await.unwrap();
//!     println!("{}", result.stdout);
//!
//!     // Interactive PTY session.
//!     let id = mgr.create_session(None, None).await.unwrap();
//!     mgr.write(&id, "echo hello\n").await.unwrap();
//!     let output = mgr.read(&id).await.unwrap();
//!     println!("{output}");
//! }
//! ```

pub mod error;
pub mod manager;
pub mod safety;
pub mod session;
pub mod truncate;
pub mod types;

pub use error::{Result, TerminalError};
pub use types::{
    BackgroundJob, ExecMode, ExecOptions, ExecResult, JobId, JobStatus, SessionId, SessionInfo,
};
