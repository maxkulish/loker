use std::fmt;

// ---------------------------------------------------------------------------
// PhaseState
// ---------------------------------------------------------------------------

/// Valid states in the phase lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PhaseState {
    /// No phase activity has begun.
    Idle,
    /// `write_started` has been called.
    Started,
    /// The phase's artefact has been written.
    ArtefactWritten,
    /// The phase's entry has been appended to the manifest.
    ManifestAppended,
    /// `write_completed` has been called (terminal state).
    Completed,
}

impl fmt::Display for PhaseState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Idle => write!(f, "Idle"),
            Self::Started => write!(f, "Started"),
            Self::ArtefactWritten => write!(f, "ArtefactWritten"),
            Self::ManifestAppended => write!(f, "ManifestAppended"),
            Self::Completed => write!(f, "Completed"),
        }
    }
}

// ---------------------------------------------------------------------------
// PhaseOrderGuard
// ---------------------------------------------------------------------------

/// Lightweight state machine that enforces the canonical phase lifecycle
/// order at runtime:
///
/// `Idle → Started → ArtefactWritten → ManifestAppended → Completed`
///
/// # Panics
///
/// In debug builds, invalid transitions cause a panic. In release builds the
/// transition is silently ignored and an error message is emitted via
/// `eprintln!` (for now; the `tracing` crate integration is deferred to
/// T-029).
///
/// # Design note
///
/// The caller creates one `PhaseOrderGuard` per phase invocation and holds
/// it until the phase completes. T-028 (PhaseRunner) must be careful not to
/// hold the guard across `.await` points in a way that would allow re-entrant
/// calls to skip steps.
#[derive(Debug, Clone)]
pub struct PhaseOrderGuard {
    state: PhaseState,
    phase: String,
    attempt: u32,
}

impl PhaseOrderGuard {
    /// Create a new guard in the `Idle` state.
    pub fn new(phase: String, attempt: u32) -> Self {
        Self {
            state: PhaseState::Idle,
            phase,
            attempt,
        }
    }

    /// Return the current state.
    pub fn state(&self) -> &PhaseState {
        &self.state
    }

    /// Transition from `Idle` → `Started`.
    ///
    /// Must be called after the "started" marker has been written.
    pub fn mark_started(&mut self) {
        self.transition(PhaseState::Started, PhaseState::Idle);
    }

    /// Transition from `Started` → `ArtefactWritten`.
    ///
    /// Must be called after the phase's primary artefact has been written
    /// to the run directory.
    pub fn mark_artefact_written(&mut self) {
        self.transition(PhaseState::ArtefactWritten, PhaseState::Started);
    }

    /// Transition from `ArtefactWritten` → `ManifestAppended`.
    ///
    /// Must be called after the artefact entry has been appended to the
    /// run manifest.
    pub fn mark_manifest_appended(&mut self) {
        self.transition(PhaseState::ManifestAppended, PhaseState::ArtefactWritten);
    }

    /// Transition from `ManifestAppended` → `Completed`.
    ///
    /// Must be called after the "completed" marker has been written.
    pub fn mark_completed(&mut self) {
        self.transition(PhaseState::Completed, PhaseState::ManifestAppended);
    }

    /// Perform the actual state transition, enforcing order.
    fn transition(&mut self, target: PhaseState, expected_current: PhaseState) {
        if self.state != expected_current {
            let msg = format!(
                "PhaseOrderGuard[phase={}, attempt={}]: invalid transition {}-→{} (expected current={})",
                self.phase, self.attempt, self.state, target, expected_current
            );
            // In debug builds, panic to catch bugs early.
            // In release builds, log and soldier on.
            #[cfg(debug_assertions)]
            panic!("{}", msg);

            #[cfg(not(debug_assertions))]
            {
                // TODO(T-029): replace eprintln with injected log sink
                eprintln!("{}", msg);
                return;
            }
        }
        self.state = target;
    }
}
