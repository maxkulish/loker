#![allow(clippy::module_inception)]

//! Verification hooks for strategy gating.
//!
//! Each hook implements [`VerifyHook`] and returns a [`VerifyResult`]
//! (Pass / Fail / reserved variants). Concrete implementations:
//!
//! - [`LLMVerifier`] — delegates to an LLM backend and parses yes/no.
//! - [`RunCommand`] — shells out and maps exit status (CLO-271).
//! - [`TestRunner`] — parses structured test output (CLO-273).

pub mod human_verifier;
pub mod llm_verifier;
pub mod run_command;
pub mod test_runner;
pub mod verify;

// Re-export the core types so `strategy::verify::FailureReason` etc. work.
pub use human_verifier::{
    HumanClock, HumanDecision, HumanResponse, HumanSeverity, HumanTimeoutAction,
    HumanTimeoutOutcome, HumanTimeoutPolicy, HumanTimeoutRule, HumanVerifier, HumanVerifierConfig,
    HumanVerifyReport, SystemHumanClock,
};
pub use verify::{
    FailureReason, SandboxViolation, VerifyContext, VerifyError, VerifyHook, VerifyResult,
};

// Re-export concrete implementations.
pub use llm_verifier::LLMVerifier;
pub use run_command::RunCommand;
pub use test_runner::{SandboxOpts, TestResult, TestRunner, TestRunnerKind};
