//! loker UI daemon (`loker ui --serve`)
//!
//! Long-running daemon that discovers all run directories under
//! `<project_root>/runs/` and serves a JSON runs list on `GET /`.

pub mod discovery;
pub mod routes;
pub mod serve;

pub use discovery::{discover_runs, RunSummary};
pub use serve::serve;
