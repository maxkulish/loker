//! Summary writer: emits `runs/<id>/summary.json` at run completion.
//!
//! Follows the same pattern as `trace` — a `SummarySink` trait with
//! file-backed and in-memory implementations.

pub mod prices;
pub mod reader;

pub use prices::PriceTable;
pub use reader::{BackendUsage, TraceReader, TraceReaderError};
