//! Verification hooks for strategy gating.
//!
//! Each hook implements [`VerifyHook`] and returns a [`VerifyResult`]
//! (Pass / Fail / reserved variants). Concrete implementations:
//!
//! - [`LLMVerifier`] — delegates to an LLM backend and parses yes/no.
//! - [`RunCommand`] — shells out and maps exit status (CLO-271).

pub mod llm_verifier;
pub mod run_command;
pub mod verify;

// Re-export the core types so `strategy::verify::FailureReason` etc. work.
pub use verify::{FailureReason, VerifyContext, VerifyError, VerifyHook, VerifyResult};

// Re-export concrete implementations.
pub use llm_verifier::LLMVerifier;
pub use run_command::RunCommand;
