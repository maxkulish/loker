//! loker UI daemon (`loker ui --serve`)
//!
//! Long-running daemon that discovers all run directories under
//! `<project_root>/runs/` and serves a JSON runs list on `GET /`.

pub mod discovery;
pub mod gate_discovery;
pub mod manifest;
pub mod routes;
pub mod serve;
pub mod sse;
pub mod templates;
pub mod trace_reader;

pub use discovery::{discover_runs, RunSummary};
pub use serve::serve;
