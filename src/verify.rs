use async_trait::async_trait;

use crate::backend::QueryOutput;

/// Result returned by a verification hook.
///
/// v0 hooks only need `Pass` and `Fail`. `Repair` and `Score` are reserved so
/// later hook implementations can evolve without changing the public enum.
#[derive(Debug, Clone, PartialEq)]
pub enum VerifyResult {
    Pass { notes: Option<String> },
    Fail { reason: String },
    Repair { suggestion: String },
    Score(f32),
}

impl VerifyResult {
    pub fn pass() -> Self {
        Self::Pass { notes: None }
    }

    pub fn fail(reason: impl Into<String>) -> Self {
        Self::Fail {
            reason: reason.into(),
        }
    }

    pub fn is_pass(&self) -> bool {
        matches!(self, Self::Pass { .. })
    }

    pub(crate) fn phase_status(&self) -> &'static str {
        if self.is_pass() {
            "pass"
        } else {
            "fail"
        }
    }
}

#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
#[error("verify hook failed: {message}")]
pub struct VerifyError {
    pub message: String,
}

impl VerifyError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

/// Trait implemented by verification hooks that gate strategy progress.
#[async_trait]
pub trait VerifyHook: Send + Sync {
    fn name(&self) -> &str;

    async fn verify(&self, output: &QueryOutput) -> Result<VerifyResult, VerifyError>;
}
