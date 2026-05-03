// Most modules are private to this library and are surfaced only because
// they sit in the dependency closure of the public ones (`strategy` →
// `template` → `workflow` → ...). The binary (src/main.rs) re-declares
// them via its own private `mod` tree, so this file's job is just to
// satisfy the lib-side compile and expose the integration-test surface.
#![allow(dead_code)]

pub mod aggregator;
pub mod backend;
pub mod family;
pub mod manifest;
pub mod phase_runner;
pub mod run_state;
pub mod strategy;
pub mod template;
pub mod trace;

mod apply_verify;
mod cache;
mod config;
mod consensus;
mod context;
mod git_agent;
mod role;
mod utils;
pub mod workflow;
mod workflows;

pub use phase_runner::{
    AggregatorName, PhaseConfig, PhaseError, PhaseInputs, PhaseOutcome, PhaseRung, PhaseRunner,
    StrategyName, VerifyHookName,
};
pub use trace::{
    AttemptSpanContext, BackendSpanResult, InMemorySink, PhaseSpanContext, TraceSink, TraceWriter,
    VerifySpanResult,
};
