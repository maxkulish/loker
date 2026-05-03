//! Phase-based workflow grammar parser (FR-29, FR-30, FR-31).
//!
//! Parses `.lok/workflows/*.toml` into a typed [`Workflow`] AST. Each
//! workflow consists of a sequence of *phases*, each with a strategy,
//! backends, prompt template, inputs, output, and optional contract block.
//!
//! # API
//!
//! ```
//! use loker::workflow::grammar::Workflow;
//! let wf: Workflow = r#"
//!   name = "test"
//!   [[phases]]
//!   name = "analysis"
//!   strategy = { single = {} }
//!   backends = ["claude/"]
//!   prompt_template = "Analyze {{ spec }}"
//!   inputs = ["spec"]
//!   output = "analysis.md"
//! "#.parse().unwrap();
//! # Ok::<_, Vec<loker::workflow::grammar::WorkflowError>>(())
//! ```

use serde::Deserialize;
use std::collections::HashSet;
use std::str::FromStr;
use thiserror::Error;

// ---------------------------------------------------------------------------
// AST types
// ---------------------------------------------------------------------------

/// A phase-based workflow definition parsed from TOML (FR-29).
///
/// This is the top-level AST node. A workflow has a name, optional
/// description, an ordered list of phases, and optional defaults.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Workflow {
    pub name: String,

    #[serde(default)]
    pub description: Option<String>,

    /// Ordered list of phases. Each phase produces an output that can be
    /// referenced by name by later phases.
    pub phases: Vec<Phase>,

    /// Reserved for workflow-level default values (e.g. default strategy,
    /// default backends). Stored as raw TOML; semantics defined later.
    #[serde(default)]
    pub defaults: Option<toml::Value>,
}

/// A single phase in a workflow (FR-29).
///
/// Each phase has a name, strategy, backends, prompt template, input
/// references, an output artefact filename, and an optional contract block.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Phase {
    pub name: String,

    /// The execution strategy for this phase.
    pub strategy: Strategy,

    /// Raw backend strings (e.g. `"claude/"`, `"tensorzero/judge"`).
    /// Resolved to [`BackendRef`] during validation via [`resolve_backend_scheme`].
    #[serde(default)]
    pub backends: Vec<String>,

    /// The prompt template source string.
    #[serde(rename = "prompt_template")]
    pub prompt_template: String,

    /// Input artefact references — either `phase:<name>`, `var:<name>`,
    /// or `spec`. Parsed from raw strings.
    #[serde(default)]
    pub inputs: Vec<String>,

    /// Declared output artefact filename, written under the phase's run
    /// directory.
    pub output: String,

    /// Reserved for `phase.contract` semantics (FR-31). Parsed and stored
    /// as raw TOML; no validation is performed on the content. A lint
    /// warning is emitted when this is present.
    #[serde(default)]
    pub contract: Option<toml::Value>,
}

/// Execution strategy for a phase (FR-29).
///
/// Mirrors the runtime `Strategy` trait variants (`SingleModel`,
/// `ParallelFanOut`, `EscalatingRetry`) but as a deserialisable enum so
/// it can be parsed from TOML.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Strategy {
    /// Query a single backend once.
    Single,

    /// Query multiple backends in parallel, requiring at least
    /// `min_responses` successful responses.
    #[serde(rename = "parallel")]
    ParallelFanOut { min_responses: usize },

    /// Try backends in escalating order (cheapest/weakest first), stopping
    /// at the first successful response. Requires at least 2 backends.
    #[serde(rename = "escalating")]
    EscalatingRetry { pass_failure_context: bool },
}

/// A resolved backend reference (FR-30).
///
/// Each variant carries the information needed to instantiate the
/// corresponding `Backend` implementation at runtime. The actual
/// instantiation lives in `PhaseRunner` / T-028.
#[derive(Debug, Clone, PartialEq)]
pub enum BackendRef {
    /// TensorZero inference function (e.g. `"tensorzero/judge"`).
    TensorZero(String),

    /// Anthropic Claude.
    Claude,

    /// Codex.
    Codex,

    /// Google Gemini.
    Gemini,

    /// Ollama with a specific model (e.g. `"ollama/glm-5.1"`).
    Ollama(String),
}

/// An input artefact reference.
///
/// Deserialised from strings using the prefix convention:
/// - `"phase:<name>"` → output of a prior phase
/// - `"var:<name>"` → CLI `--var` value
/// - `"spec"` → the `--spec` file
#[derive(Debug, Clone, PartialEq)]
pub enum InputRef {
    /// Reference to the output of another phase.
    PhaseRef(String),

    /// Reference to a CLI `--var` value.
    VarRef(String),

    /// Reference to the `--spec` file.
    Spec,
}

// ---------------------------------------------------------------------------
// Error types
// ---------------------------------------------------------------------------

/// Typed errors for workflow grammar parsing and validation (FR-29/30/31).
#[derive(Debug, Error, Clone, PartialEq)]
pub enum WorkflowError {
    /// TOML parse error at the top level.
    #[error("TOML parse error: {0}")]
    TomlParse(String),

    /// An unknown backend scheme was encountered.
    #[error("Unknown backend scheme: '{scheme}'")]
    UnknownBackendScheme { scheme: String },

    /// Phase A references the output of phase B, but B is declared after A.
    #[error("Phase '{from}' references phase '{to}' which is declared later")]
    ForwardInputRef { from: String, to: String },

    /// Phase references a phase that does not exist in the workflow.
    #[error("Phase '{from}' references unknown phase '{to}'")]
    UnknownPhaseRef { from: String, to: String },

    /// `ParallelFanOut.min_responses` exceeds the number of backends.
    #[error("ParallelFanOut.min_responses ({min}) exceeds number of backends ({count}) in phase '{phase}'")]
    MinResponsesExceedsBackends {
        phase: String,
        min: usize,
        count: usize,
    },

    /// `EscalatingRetry` requires at least 2 backends.
    #[error("EscalatingRetry requires at least 2 backends, got {count} in phase '{phase}'")]
    TooFewBackendsForEscalating { phase: String, count: usize },

    /// Workflow has no phases.
    #[error("Workflow has no phases")]
    NoPhases,

    /// Duplicate phase names.
    #[error("Duplicate phase name: '{name}'")]
    DuplicatePhaseName { name: String },

    /// Invalid input reference format.
    #[error("Invalid input reference: '{raw}'")]
    InvalidInputRef { raw: String },

    /// Backend scheme parser error — scheme string is malformed.
    #[error("Malformed backend reference: '{raw}'")]
    MalformedBackendRef { raw: String },
}

// ---------------------------------------------------------------------------
// String -> enum conversions (custom deserialisation)
// ---------------------------------------------------------------------------

/// Parse a backend reference string into a `BackendRef`.
///
/// Supported formats:
/// - `"tensorzero/<fn>"` → `TensorZero(fn)`
/// - `"claude/"` → `Claude`
/// - `"codex/"` → `Codex`
/// - `"gemini/"` → `Gemini`
/// - `"ollama/<model>"` → `Ollama(model)`
///
/// Unknown schemes return `Err`.
pub fn resolve_backend_scheme(s: &str) -> Result<BackendRef, WorkflowError> {
    // Require the `<scheme>/<id>` form — no bare "tensorzero" or "claudefoo".
    let Some((scheme, rest)) = s.split_once('/') else {
        return Err(WorkflowError::MalformedBackendRef { raw: s.to_string() });
    };
    match scheme {
        "tensorzero" => {
            if rest.is_empty() {
                Err(WorkflowError::MalformedBackendRef { raw: s.to_string() })
            } else {
                Ok(BackendRef::TensorZero(rest.to_string()))
            }
        }
        "claude" => {
            if !rest.is_empty() {
                Err(WorkflowError::MalformedBackendRef { raw: s.to_string() })
            } else {
                Ok(BackendRef::Claude)
            }
        }
        "codex" => {
            if !rest.is_empty() {
                Err(WorkflowError::MalformedBackendRef { raw: s.to_string() })
            } else {
                Ok(BackendRef::Codex)
            }
        }
        "gemini" => {
            if !rest.is_empty() {
                Err(WorkflowError::MalformedBackendRef { raw: s.to_string() })
            } else {
                Ok(BackendRef::Gemini)
            }
        }
        "ollama" => {
            if rest.is_empty() {
                Err(WorkflowError::MalformedBackendRef { raw: s.to_string() })
            } else {
                Ok(BackendRef::Ollama(rest.to_string()))
            }
        }
        _ => Err(WorkflowError::UnknownBackendScheme {
            scheme: s.to_string(),
        }),
    }
}

impl TryFrom<String> for BackendRef {
    type Error = WorkflowError;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        resolve_backend_scheme(&s)
    }
}

/// Parse an input reference string into an `InputRef`.
///
/// - `"phase:name"` → `PhaseRef("name")`
/// - `"var:name"` → `VarRef("name")`
/// - `"spec"` → `Spec`
fn parse_input_ref(s: &str) -> Result<InputRef, WorkflowError> {
    if s == "spec" {
        return Ok(InputRef::Spec);
    }
    if let Some(name) = s.strip_prefix("phase:") {
        if name.is_empty() {
            return Err(WorkflowError::InvalidInputRef { raw: s.to_string() });
        }
        return Ok(InputRef::PhaseRef(name.to_string()));
    }
    if let Some(name) = s.strip_prefix("var:") {
        if name.is_empty() {
            return Err(WorkflowError::InvalidInputRef { raw: s.to_string() });
        }
        return Ok(InputRef::VarRef(name.to_string()));
    }
    Err(WorkflowError::InvalidInputRef { raw: s.to_string() })
}

impl TryFrom<String> for InputRef {
    type Error = WorkflowError;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        parse_input_ref(&s)
    }
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

impl FromStr for Workflow {
    type Err = Vec<WorkflowError>;

    /// Parse a TOML string into a `Workflow`, then validate the result.
    ///
    /// Returns all validation errors at once so the caller can display
    /// every problem in a single pass.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let wf: Workflow =
            toml::from_str(s).map_err(|e| vec![WorkflowError::TomlParse(e.to_string())])?;
        let errors = wf.validate();
        if errors.is_empty() {
            Ok(wf)
        } else {
            Err(errors)
        }
    }
}

impl Workflow {
    /// Resolve a phase's raw backend strings into [`BackendRef`] values.
    ///
    /// Returns an error for each unresolvable backend string.
    pub fn resolve_backends(phase: &Phase) -> Result<Vec<BackendRef>, Vec<WorkflowError>> {
        let mut refs = Vec::with_capacity(phase.backends.len());
        let mut errors = Vec::new();
        for raw in &phase.backends {
            match resolve_backend_scheme(raw) {
                Ok(r) => refs.push(r),
                Err(e) => errors.push(e),
            }
        }
        if errors.is_empty() {
            Ok(refs)
        } else {
            Err(errors)
        }
    }

    /// Resolve a phase's raw input strings into [`InputRef`] values.
    ///
    /// Returns an error for each unresolvable input string.
    pub fn resolve_inputs(phase: &Phase) -> Result<Vec<InputRef>, Vec<WorkflowError>> {
        let mut refs = Vec::with_capacity(phase.inputs.len());
        let mut errors = Vec::new();
        for raw in &phase.inputs {
            match parse_input_ref(raw) {
                Ok(r) => refs.push(r),
                Err(e) => errors.push(e),
            }
        }
        if errors.is_empty() {
            Ok(refs)
        } else {
            Err(errors)
        }
    }

    /// Run the full validation pass on this workflow.
    ///
    /// Checks:
    /// 1. At least one phase exists.
    /// 2. No duplicate phase names.
    /// 3. No forward input references (phase A cannot reference phase:B
    ///    where B is declared after A).
    /// 4. Every backend string parses via the resolver.
    /// 5. Strategy-specific constraints (`parallel.min_responses <=
    ///    len(backends)`, `escalating.backends.len() >= 2`).
    /// 6. Every input reference is well-formed.
    pub fn validate(&self) -> Vec<WorkflowError> {
        let mut errors: Vec<WorkflowError> = Vec::new();

        // Check for empty workflow
        if self.phases.is_empty() {
            errors.push(WorkflowError::NoPhases);
            return errors; // No further checks possible
        }

        // Check for duplicate phase names
        let mut seen_names: HashSet<&str> = HashSet::new();
        for phase in &self.phases {
            if !seen_names.insert(&phase.name) {
                errors.push(WorkflowError::DuplicatePhaseName {
                    name: phase.name.clone(),
                });
            }
        }

        // Phase index map for forward-ref checking
        let phase_indices: std::collections::HashMap<&str, usize> = self
            .phases
            .iter()
            .enumerate()
            .map(|(i, p)| (p.name.as_str(), i))
            .collect();

        for phase in &self.phases {
            // Resolve backends (collects UnknownBackendScheme / MalformedBackendRef errors)
            let resolved_backends = match Self::resolve_backends(phase) {
                Ok(b) => b,
                Err(mut backend_errors) => {
                    errors.append(&mut backend_errors);
                    // Continue checking inputs even if backends fail —
                    // only skip strategy constraints that depend on backend count.
                    Vec::new()
                }
            };

            // Resolve inputs (collects InvalidInputRef errors)
            let resolved_inputs = match Self::resolve_inputs(phase) {
                Ok(i) => i,
                Err(mut input_errors) => {
                    errors.append(&mut input_errors);
                    // Continue checking other things even if input refs are bad
                    Vec::new()
                }
            };

            // Validate inputs for forward refs and unknown phase refs
            for input in &resolved_inputs {
                if let InputRef::PhaseRef(target) = input {
                    match phase_indices.get(target.as_str()) {
                        Some(&target_idx) => {
                            let phase_idx = phase_indices
                                .get(phase.name.as_str())
                                .copied()
                                .unwrap_or(usize::MAX);
                            if target_idx >= phase_idx {
                                errors.push(WorkflowError::ForwardInputRef {
                                    from: phase.name.clone(),
                                    to: target.clone(),
                                });
                            }
                        }
                        None => {
                            // Target phase does not exist at all
                            errors.push(WorkflowError::UnknownPhaseRef {
                                from: phase.name.clone(),
                                to: target.clone(),
                            });
                        }
                    }
                }
            }

            // Strategy-specific constraints
            match &phase.strategy {
                Strategy::ParallelFanOut { min_responses } => {
                    if *min_responses > resolved_backends.len() {
                        errors.push(WorkflowError::MinResponsesExceedsBackends {
                            phase: phase.name.clone(),
                            min: *min_responses,
                            count: resolved_backends.len(),
                        });
                    }
                }
                Strategy::EscalatingRetry { .. } => {
                    if resolved_backends.len() < 2 {
                        errors.push(WorkflowError::TooFewBackendsForEscalating {
                            phase: phase.name.clone(),
                            count: resolved_backends.len(),
                        });
                    }
                }
                Strategy::Single => {
                    // Single has no additional constraints
                }
            }
        }

        errors
    }

    /// Run lint checks on this workflow.
    ///
    /// Returns warning strings (not errors). Currently warns about
    /// `phase.contract` being present (FR-31 reservation).
    pub fn lint(&self) -> Vec<String> {
        let mut warnings = Vec::new();

        for phase in &self.phases {
            if phase.contract.is_some() {
                warnings.push(format!(
                    "WARN: phase.contract reserved for post-v0; ignored (phase '{}')",
                    phase.name
                ));
            }
        }

        warnings
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- Backend resolver tests --

    #[test]
    fn test_resolve_tensorzero() {
        let ref_ = resolve_backend_scheme("tensorzero/judge").unwrap();
        assert_eq!(ref_, BackendRef::TensorZero("judge".into()));
    }

    #[test]
    fn test_resolve_claude() {
        let ref_ = resolve_backend_scheme("claude/").unwrap();
        assert_eq!(ref_, BackendRef::Claude);
    }

    #[test]
    fn test_resolve_codex() {
        let ref_ = resolve_backend_scheme("codex/").unwrap();
        assert_eq!(ref_, BackendRef::Codex);
    }

    #[test]
    fn test_resolve_gemini() {
        let ref_ = resolve_backend_scheme("gemini/").unwrap();
        assert_eq!(ref_, BackendRef::Gemini);
    }

    #[test]
    fn test_resolve_ollama() {
        let ref_ = resolve_backend_scheme("ollama/glm-5.1").unwrap();
        assert_eq!(ref_, BackendRef::Ollama("glm-5.1".into()));
    }

    #[test]
    fn test_unknown_scheme() {
        let err = resolve_backend_scheme("unknownscheme/foo").unwrap_err();
        assert!(
            matches!(&err, WorkflowError::UnknownBackendScheme { scheme } if scheme == "unknownscheme/foo")
        );
    }

    #[test]
    fn test_ollama_missing_model() {
        let err = resolve_backend_scheme("ollama/").unwrap_err();
        assert!(matches!(&err, WorkflowError::MalformedBackendRef { raw } if raw == "ollama/"));
    }

    #[test]
    fn test_malformed_backend_no_slash() {
        let err = resolve_backend_scheme("tensorzero").unwrap_err();
        assert!(matches!(&err, WorkflowError::MalformedBackendRef { raw } if raw == "tensorzero"));
    }

    #[test]
    fn test_malformed_backend_tensorzero_empty_fn() {
        let err = resolve_backend_scheme("tensorzero/").unwrap_err();
        assert!(matches!(&err, WorkflowError::MalformedBackendRef { raw } if raw == "tensorzero/"));
    }

    #[test]
    fn test_malformed_backend_claude_extra_path() {
        let err = resolve_backend_scheme("claude/foo").unwrap_err();
        assert!(matches!(&err, WorkflowError::MalformedBackendRef { raw } if raw == "claude/foo"));
    }

    #[test]
    fn test_malformed_backend_codex_extra_path() {
        let err = resolve_backend_scheme("codex/bar").unwrap_err();
        assert!(matches!(&err, WorkflowError::MalformedBackendRef { raw } if raw == "codex/bar"));
    }

    #[test]
    fn test_malformed_backend_gemini_extra_path() {
        let err = resolve_backend_scheme("gemini/baz").unwrap_err();
        assert!(matches!(&err, WorkflowError::MalformedBackendRef { raw } if raw == "gemini/baz"));
    }

    // -- Input ref tests --

    #[test]
    fn test_parse_input_ref_phase() {
        let input = parse_input_ref("phase:analysis").unwrap();
        assert_eq!(input, InputRef::PhaseRef("analysis".into()));
    }

    #[test]
    fn test_parse_input_ref_var() {
        let input = parse_input_ref("var:branch").unwrap();
        assert_eq!(input, InputRef::VarRef("branch".into()));
    }

    #[test]
    fn test_parse_input_ref_spec() {
        let input = parse_input_ref("spec").unwrap();
        assert_eq!(input, InputRef::Spec);
    }

    #[test]
    fn test_parse_input_ref_invalid() {
        let err = parse_input_ref("invalid").unwrap_err();
        assert!(matches!(&err, WorkflowError::InvalidInputRef { raw } if raw == "invalid"));
    }

    // -- Workflow parsing tests --

    #[test]
    fn test_parse_simple_workflow() {
        let toml = r#"
name = "test"
description = "A test workflow"

[[phases]]
name = "analysis"
strategy = { single = {} }
backends = ["claude/", "gemini/"]
prompt_template = "Analyze {{ spec }}"
inputs = ["spec"]
output = "analysis.md"
"#;
        let wf = toml.parse::<Workflow>().unwrap();
        assert_eq!(wf.name, "test");
        assert_eq!(wf.description.as_deref(), Some("A test workflow"));
        assert_eq!(wf.phases.len(), 1);

        let phase = &wf.phases[0];
        assert_eq!(phase.name, "analysis");
        assert_eq!(phase.strategy, Strategy::Single);
        assert_eq!(phase.backends.len(), 2);
        assert_eq!(phase.backends[0], "claude/");
        assert_eq!(phase.backends[1], "gemini/");
        assert_eq!(phase.prompt_template, "Analyze {{ spec }}");
        assert_eq!(phase.inputs, vec!["spec".to_string()]);
        assert_eq!(phase.output, "analysis.md");
        assert!(phase.contract.is_none());
    }

    #[test]
    fn test_parse_with_parallel_strategy() {
        let toml = r#"
name = "parallel-test"

[[phases]]
name = "review"
strategy = { parallel = { min_responses = 2 } }
backends = ["claude/", "codex/", "gemini/"]
prompt_template = "Review the code"
inputs = ["spec"]
output = "review.md"
"#;
        let wf = toml.parse::<Workflow>().unwrap();
        let phase = &wf.phases[0];
        assert_eq!(
            phase.strategy,
            Strategy::ParallelFanOut { min_responses: 2 }
        );
    }

    #[test]
    fn test_parse_with_escalating_strategy() {
        let toml = r#"
name = "escalating-test"

[[phases]]
name = "fix"
strategy = { escalating = { pass_failure_context = true } }
backends = ["claude/", "codex/"]
prompt_template = "Fix the code"
inputs = ["spec"]
output = "fix.md"
"#;
        let wf = toml.parse::<Workflow>().unwrap();
        let phase = &wf.phases[0];
        assert_eq!(
            phase.strategy,
            Strategy::EscalatingRetry {
                pass_failure_context: true
            }
        );
    }

    // -- Validation tests --

    #[test]
    fn test_empty_workflow() {
        let toml = r#"
name = "empty"
phases = []
"#;
        let err = toml.parse::<Workflow>().unwrap_err();
        assert!(err.iter().any(|e| matches!(e, WorkflowError::NoPhases)));
    }

    #[test]
    fn test_duplicate_phase_name() {
        let toml = r#"
name = "dupe"

[[phases]]
name = "analysis"
strategy = { single = {} }
backends = ["claude/"]
prompt_template = "analyze"
inputs = ["spec"]
output = "a.md"

[[phases]]
name = "analysis"
strategy = { single = {} }
backends = ["gemini/"]
prompt_template = "analyze again"
inputs = ["spec"]
output = "b.md"
"#;
        let err = toml.parse::<Workflow>().unwrap_err();
        assert!(err.iter().any(
            |e| matches!(e, WorkflowError::DuplicatePhaseName { name } if name == "analysis")
        ));
    }

    #[test]
    fn test_forward_input_ref() {
        let toml = r#"
name = "forward"

[[phases]]
name = "synthesis"
strategy = { single = {} }
backends = ["claude/"]
prompt_template = "Synthesize"
inputs = ["phase:analysis"]
output = "synthesis.md"

[[phases]]
name = "analysis"
strategy = { single = {} }
backends = ["gemini/"]
prompt_template = "Analyze"
inputs = ["spec"]
output = "analysis.md"
"#;
        let err = toml.parse::<Workflow>().unwrap_err();
        assert!(err
            .iter()
            .any(|e| matches!(e, WorkflowError::ForwardInputRef { from, to }
                if from == "synthesis" && to == "analysis"
            )));
    }

    #[test]
    fn test_unknown_phase_ref() {
        // Phase references a phase that doesn't exist at all
        let toml = r#"
name = "unknown-phase"

[[phases]]
name = "synthesis"
strategy = { single = {} }
backends = ["claude/"]
prompt_template = "Synthesize"
inputs = ["phase:nonexistent"]
output = "synthesis.md"
"#;
        let err = toml.parse::<Workflow>().unwrap_err();
        assert!(err
            .iter()
            .any(|e| matches!(e, WorkflowError::UnknownPhaseRef { from, to }
                if from == "synthesis" && to == "nonexistent"
            )));
    }

    #[test]
    fn test_self_ref_input() {
        let toml = r#"
name = "self-ref"

[[phases]]
name = "analysis"
strategy = { single = {} }
backends = ["claude/"]
prompt_template = "Analyze"
inputs = ["phase:analysis"]
output = "analysis.md"
"#;
        let err = toml.parse::<Workflow>().unwrap_err();
        assert!(err
            .iter()
            .any(|e| matches!(e, WorkflowError::ForwardInputRef { from, to }
                if from == "analysis" && to == "analysis"
            )));
    }

    #[test]
    fn test_min_responses_exceeds_backends() {
        let toml = r#"
name = "min-resp"

[[phases]]
name = "review"
strategy = { parallel = { min_responses = 5 } }
backends = ["claude/", "gemini/"]
prompt_template = "Review"
inputs = ["spec"]
output = "review.md"
"#;
        let err = toml.parse::<Workflow>().unwrap_err();
        assert!(err.iter().any(
            |e| matches!(e, WorkflowError::MinResponsesExceedsBackends { phase, min, count }
                if phase == "review" && *min == 5 && *count == 2
            )
        ));
    }

    #[test]
    fn test_too_few_backends_for_escalating() {
        let toml = r#"
name = "too-few-escalating"

[[phases]]
name = "fix"
strategy = { escalating = { pass_failure_context = false } }
backends = ["claude/"]
prompt_template = "Fix"
inputs = ["spec"]
output = "fix.md"
"#;
        let err = toml.parse::<Workflow>().unwrap_err();
        assert!(err.iter().any(
            |e| matches!(e, WorkflowError::TooFewBackendsForEscalating { phase, count }
                if phase == "fix" && *count == 1
            )
        ));
    }

    #[test]
    fn test_multi_error() {
        // Two violations: forward ref AND min_responses exceeds backends
        let toml = r#"
name = "multi-error"

[[phases]]
name = "synthesis"
strategy = { parallel = { min_responses = 10 } }
backends = ["claude/"]
prompt_template = "Synthesize"
inputs = ["phase:analysis"]
output = "synthesis.md"

[[phases]]
name = "analysis"
strategy = { single = {} }
backends = ["gemini/"]
prompt_template = "Analyze"
inputs = ["spec"]
output = "analysis.md"
"#;
        let err = toml.parse::<Workflow>().unwrap_err();
        assert!(err.len() >= 2);
    }

    #[test]
    fn test_toml_parse_error() {
        let toml = r#"
name = "bad-toml"
[[phases]]
name = "test"
strategy = { single = {}
"#;
        let err = toml.parse::<Workflow>().unwrap_err();
        assert!(err.iter().any(|e| matches!(e, WorkflowError::TomlParse(_))));
    }

    // -- Contract reservation test --

    #[test]
    fn test_contract_ignored_with_warn() {
        let toml = r#"
name = "contract-test"

[[phases]]
name = "analysis"
strategy = { single = {} }
backends = ["claude/"]
prompt_template = "Analyze"
inputs = ["spec"]
output = "analysis.md"
contract = { required_capability = "file_edit", timeout_ms = 30000 }
"#;
        let wf = toml.parse::<Workflow>().unwrap();
        let phase = &wf.phases[0];
        assert!(phase.contract.is_some());

        // Lint should contain the reservation warning
        let warnings = wf.lint();
        assert!(warnings
            .iter()
            .any(|w| w.contains("phase.contract reserved for post-v0; ignored")));
    }

    // -- Input ref validation test --

    #[test]
    fn test_invalid_input_ref_format() {
        let toml = r#"
name = "bad-input"

[[phases]]
name = "analysis"
strategy = { single = {} }
backends = ["claude/"]
prompt_template = "Analyze"
inputs = ["invalid"]
output = "analysis.md"
"#;
        let err = toml.parse::<Workflow>().unwrap_err();
        assert!(err
            .iter()
            .any(|e| matches!(e, WorkflowError::InvalidInputRef { raw } if raw == "invalid")));
    }

    // -- Backend scheme deserialisation tests --

    #[test]
    fn test_backend_scheme_matrix() {
        // This test validates deserialisation with resolve_backend_scheme
        let toml = r#"
name = "scheme-matrix"

[[phases]]
name = "judge"
strategy = { single = {} }
backends = ["tensorzero/judge"]
prompt_template = "Judge"
inputs = ["spec"]
output = "judge.md"

[[phases]]
name = "code"
strategy = { single = {} }
backends = ["claude/"]
prompt_template = "Code"
inputs = ["spec"]
output = "code.md"

[[phases]]
name = "exec"
strategy = { single = {} }
backends = ["codex/"]
prompt_template = "Exec"
inputs = ["spec"]
output = "exec.md"

[[phases]]
name = "think"
strategy = { single = {} }
backends = ["gemini/"]
prompt_template = "Think"
inputs = ["spec"]
output = "think.md"

[[phases]]
name = "run"
strategy = { single = {} }
backends = ["ollama/glm-5.1"]
prompt_template = "Run"
inputs = ["spec"]
output = "run.md"
"#;
        let wf = toml.parse::<Workflow>().unwrap();
        assert_eq!(wf.phases.len(), 5);
        let resolved = Workflow::resolve_backends(&wf.phases[0]).unwrap();
        assert_eq!(resolved[0], BackendRef::TensorZero("judge".into()));
        let resolved = Workflow::resolve_backends(&wf.phases[1]).unwrap();
        assert_eq!(resolved[0], BackendRef::Claude);
        let resolved = Workflow::resolve_backends(&wf.phases[2]).unwrap();
        assert_eq!(resolved[0], BackendRef::Codex);
        let resolved = Workflow::resolve_backends(&wf.phases[3]).unwrap();
        assert_eq!(resolved[0], BackendRef::Gemini);
        let resolved = Workflow::resolve_backends(&wf.phases[4]).unwrap();
        assert_eq!(resolved[0], BackendRef::Ollama("glm-5.1".into()));
    }
}
