//! Phase-level prompt template engine with upstream-artefact substitution.
//!
//! This is the substitution engine for `design-doc-tdd` (and future) phase
//! workflows. It renders plain text template files (not MiniJinja/Tera) with
//! a minimal placeholder syntax:
//!
//! - `{{ spec }}`                    → spec file content
//! - `{{ phase.<name>.output }}`     → full content of a prior phase's output
//! - `{{ phase.<name>.output.path }}`→ relative path of a prior phase's output
//! - `{{ var.<name> }}`              → CLI `--var` value
//!
//! Strict mode: every `{{ ... }}` must resolve. Unresolved placeholders
//! produce `TemplateError::UnresolvedPlaceholder`.
//!
//! No conditionals, loops, or filters — plain substitution only.

use regex::Regex;
use std::collections::HashMap;
use std::sync::LazyLock;
use thiserror::Error;

// ---------------------------------------------------------------------------
// Error types
// ---------------------------------------------------------------------------

/// Errors that can occur during template rendering.
#[derive(Debug, Error, Clone, PartialEq)]
pub enum TemplateError {
    /// A referenced placeholder was not found in the template context.
    #[error("Unresolved placeholder: `{name}`")]
    UnresolvedPlaceholder { name: String },

    /// The template string could not be parsed due to a malformed placeholder.
    #[error("Malformed placeholder: `{raw}`")]
    MalformedPlaceholder { raw: String },
}

// ---------------------------------------------------------------------------
// Context types
// ---------------------------------------------------------------------------

/// Output content and metadata from a prior phase.
#[derive(Debug, Clone)]
pub struct PhaseOutput {
    /// Full text content of the phase's output artefact.
    pub content: String,
    /// Relative path to the output artefact (e.g. `design.md`).
    pub path: String,
}

/// Runtime context passed to `Template::render`.
///
/// Every field is optional; missing values for referenced placeholders
/// produce `TemplateError::UnresolvedPlaceholder`.
#[derive(Debug, Clone, Default)]
pub struct TemplateContext {
    /// The `--spec` file content (`{{ spec }}`).
    pub spec: Option<String>,
    /// Phase outputs keyed by phase name (`{{ phase.<name>.output }}`).
    pub phase_outputs: HashMap<String, PhaseOutput>,
    /// CLI `--var` values keyed by name (`{{ var.<name> }}`).
    pub vars: HashMap<String, String>,
}

impl TemplateContext {
    /// Create an empty context.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the spec content.
    pub fn with_spec(mut self, spec: String) -> Self {
        self.spec = Some(spec);
        self
    }

    /// Add a phase output.
    pub fn with_phase_output(mut self, name: &str, output: PhaseOutput) -> Self {
        self.phase_outputs.insert(name.to_string(), output);
        self
    }

    /// Add a var value.
    pub fn with_var(mut self, name: &str, value: impl Into<String>) -> Self {
        self.vars.insert(name.to_string(), value.into());
        self
    }
}

// ---------------------------------------------------------------------------
// Regex patterns
// ---------------------------------------------------------------------------

/// Matches `{{ ... }}` placeholders, capturing the inner content with
/// leading/trailing whitespace trimmed.
///
/// Examples matched:
/// - `{{ spec }}`         → capture: `spec`
/// - `{{spec}}`           → capture: `spec`
/// - `{{  spec  }}`       → capture: `spec`
/// - `{{ phase.design.output }}`    → capture: `phase.design.output`
/// - `{{ phase.design.output.path }}` → capture: `phase.design.output.path`
/// - `{{ var.name }}`     → capture: `var.name`
static PLACEHOLDER_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\{\{\s*([^}]+?)\s*\}\}").unwrap());

// ---------------------------------------------------------------------------
// Template engine
// ---------------------------------------------------------------------------

/// The template substitution engine.
///
/// Stateless after construction — create once and reuse.
pub struct Template;

impl Template {
    /// Render a template string with the given context.
    ///
    /// Strict mode is always enabled: any `{{ ... }}` that cannot be resolved
    /// produces `TemplateError::UnresolvedPlaceholder`.
    ///
    /// The substitution is single-pass: `{{` in the body of substituted content
    /// is treated as literal text and is NOT re-evaluated (no template injection).
    ///
    /// Whitespace tolerance: `{{spec}}`, `{{ spec }}`, and `{{  spec  }}` all
    /// resolve identically.
    pub fn render(template_str: &str, ctx: &TemplateContext) -> Result<String, TemplateError> {
        let mut result = String::with_capacity(template_str.len());
        let mut last_end = 0;

        for cap in PLACEHOLDER_RE.captures_iter(template_str) {
            let m = cap.get(0).expect("match 0 always exists");
            let inner = cap.get(1).expect("capture group 1 always exists").as_str();

            // Append literal text between the last match and this match
            result.push_str(&template_str[last_end..m.start()]);

            // Resolve the placeholder
            let replacement = resolve_placeholder(inner, ctx)?;
            result.push_str(&replacement);

            last_end = m.end();
        }

        // Append remaining literal text after the last match
        result.push_str(&template_str[last_end..]);

        Ok(result)
    }
}

// ---------------------------------------------------------------------------
// Placeholder resolution
// ---------------------------------------------------------------------------

/// Resolve a single trimmed placeholder string against the context.
///
/// Returns the replacement text or `TemplateError::UnresolvedPlaceholder`.
fn resolve_placeholder(inner: &str, ctx: &TemplateContext) -> Result<String, TemplateError> {
    match inner {
        "spec" => ctx
            .spec
            .clone()
            .ok_or_else(|| TemplateError::UnresolvedPlaceholder {
                name: inner.to_string(),
            }),

        _ if inner.starts_with("phase.") => {
            // pattern: phase.<name>.output[.path]
            let rest = &inner["phase.".len()..];

            // Split into segments: [name, "output"] or [name, "output", "path"]
            let segments: Vec<&str> = rest.splitn(3, '.').collect();
            if segments.len() < 2 {
                return Err(TemplateError::MalformedPlaceholder {
                    raw: inner.to_string(),
                });
            }

            let phase_name = segments[0];
            let field = segments[1];
            let subfield = segments.get(2).copied();

            if field != "output" {
                return Err(TemplateError::MalformedPlaceholder {
                    raw: inner.to_string(),
                });
            }

            let phase_output = ctx.phase_outputs.get(phase_name).ok_or_else(|| {
                TemplateError::UnresolvedPlaceholder {
                    name: format!("phase.{}.output", phase_name),
                }
            })?;

            match subfield {
                Some("path") => Ok(phase_output.path.clone()),
                Some(other) => Err(TemplateError::MalformedPlaceholder {
                    raw: format!("phase.{}.output.{}", phase_name, other),
                }),
                None => Ok(phase_output.content.clone()),
            }
        }

        _ if inner.starts_with("var.") => {
            let var_name = &inner["var.".len()..];
            if var_name.is_empty() {
                return Err(TemplateError::MalformedPlaceholder {
                    raw: inner.to_string(),
                });
            }
            ctx.vars
                .get(var_name)
                .cloned()
                .ok_or_else(|| TemplateError::UnresolvedPlaceholder {
                    name: format!("var.{}", var_name),
                })
        }

        other => Err(TemplateError::UnresolvedPlaceholder {
            name: other.to_string(),
        }),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- Helpers --

    fn make_phase_output(content: &str, path: &str) -> PhaseOutput {
        PhaseOutput {
            content: content.to_string(),
            path: path.to_string(),
        }
    }

    fn sample_ctx() -> TemplateContext {
        TemplateContext::new()
            .with_spec("Create a calculator CLI".to_string())
            .with_phase_output(
                "design",
                make_phase_output("# Design\n\nUse Rust with clap.", "design.md"),
            )
            .with_phase_output("review", make_phase_output("APPROVED", "review.md"))
            .with_var("branch", "feat-calculator")
    }

    #[test]
    fn test_spec_substitution() {
        let ctx = TemplateContext::new().with_spec("Build a web server".to_string());
        let result = Template::render("Read {{ spec }} and produce a design.", &ctx).unwrap();
        assert_eq!(result, "Read Build a web server and produce a design.");
    }

    #[test]
    fn test_phase_output_substitution() {
        let ctx = sample_ctx();
        let result = Template::render("Design: {{ phase.design.output }}", &ctx).unwrap();
        assert_eq!(result, "Design: # Design\n\nUse Rust with clap.");
    }

    #[test]
    fn test_phase_output_path_substitution() {
        let ctx = sample_ctx();
        let result = Template::render("See {{ phase.design.output.path }}", &ctx).unwrap();
        assert_eq!(result, "See design.md");
    }

    #[test]
    fn test_var_substitution() {
        let ctx = sample_ctx();
        let result = Template::render("Branch: {{ var.branch }}", &ctx).unwrap();
        assert_eq!(result, "Branch: feat-calculator");
    }

    #[test]
    fn test_unresolved_placeholder() {
        let ctx = TemplateContext::new();
        let err = Template::render("{{ undefined }}", &ctx).unwrap_err();
        assert!(matches!(
            &err,
            TemplateError::UnresolvedPlaceholder { name } if name == "undefined"
        ));
    }

    #[test]
    fn test_unresolved_var() {
        let ctx = TemplateContext::new();
        let err = Template::render("{{ var.missing }}", &ctx).unwrap_err();
        assert!(matches!(
            &err,
            TemplateError::UnresolvedPlaceholder { name } if name == "var.missing"
        ));
    }

    #[test]
    fn test_unresolved_phase() {
        let ctx = TemplateContext::new().with_spec("test".to_string());
        let err = Template::render("{{ phase.unknown.output }}", &ctx).unwrap_err();
        assert!(matches!(
            &err,
            TemplateError::UnresolvedPlaceholder { name } if name == "phase.unknown.output"
        ));
    }

    #[test]
    fn test_whitespace_tolerance_no_spaces() {
        let ctx = TemplateContext::new().with_spec("hello".to_string());
        let result = Template::render("{{spec}}", &ctx).unwrap();
        assert_eq!(result, "hello");
    }

    #[test]
    fn test_whitespace_tolerance_extra_spaces() {
        let ctx = TemplateContext::new().with_spec("hello".to_string());
        let result = Template::render("{{  spec  }}", &ctx).unwrap();
        assert_eq!(result, "hello");
    }

    #[test]
    fn test_no_template_injection() {
        // Brace sequences inside substituted content must not be re-evaluated.
        let ctx = TemplateContext::new()
            .with_phase_output("x", make_phase_output("value is {{ secret }}", "x.md"));
        let result = Template::render("{{ phase.x.output }}", &ctx).unwrap();
        assert_eq!(result, "value is {{ secret }}");
    }

    #[test]
    fn test_multiple_placeholders() {
        let ctx = sample_ctx();
        let result = Template::render(
            "Spec: {{ spec }}\nDesign: {{ phase.design.output }}\nRev: {{ phase.review.output }}",
            &ctx,
        )
        .unwrap();
        assert_eq!(
            result,
            "Spec: Create a calculator CLI\nDesign: # Design\n\nUse Rust with clap.\nRev: APPROVED"
        );
    }

    #[test]
    fn test_mixed_whitespace_placeholders() {
        let ctx = TemplateContext::new()
            .with_spec("A".to_string())
            .with_var("b", "B".to_string());
        let result = Template::render("{{spec}}-{{ var.b }}", &ctx).unwrap();
        assert_eq!(result, "A-B");
    }

    #[test]
    fn test_all_four_templates_render() {
        // Simulate rendering all four design-doc-tdd templates against
        // synthetic upstream artefacts; no errors.
        let ctx = TemplateContext::new()
            .with_spec("Build a calculator".to_string())
            .with_phase_output(
                "design",
                make_phase_output(
                    "# Design\n\nUse Rust with clap and serde_json.",
                    "design.md",
                ),
            )
            .with_phase_output(
                "review",
                make_phase_output("APPROVED with suggestions", "review.md"),
            )
            .with_phase_output(
                "implement",
                make_phase_output("src/main.rs changed", "changes"),
            );

        // design.md.tmpl
        let design_tmpl = "Read {{ spec }} and produce a design document.";
        let design_out = Template::render(design_tmpl, &ctx).unwrap();
        assert!(design_out.contains("Build a calculator"));

        // review.md.tmpl
        let review_tmpl = "Review the design: {{ phase.design.output }}";
        let review_out = Template::render(review_tmpl, &ctx).unwrap();
        assert!(review_out.contains("Use Rust with clap and serde_json."));

        // implement.md.tmpl
        let implement_tmpl = "Design: {{ phase.design.output }}\nReview: {{ phase.review.output }}";
        let implement_out = Template::render(implement_tmpl, &ctx).unwrap();
        assert!(implement_out.contains("APPROVED with suggestions"));
        assert!(implement_out.contains("Use Rust with clap and serde_json."));

        // verify.md.tmpl
        let verify_tmpl = "Does {{ phase.implement.output }} match {{ phase.design.output }}?";
        let verify_out = Template::render(verify_tmpl, &ctx).unwrap();
        assert!(verify_out.contains("src/main.rs changed"));
        assert!(verify_out.contains("Use Rust with clap and serde_json."));
    }

    #[test]
    fn test_malformed_placeholder_no_output_field() {
        let ctx = sample_ctx();
        let err = Template::render("{{ phase.design }}", &ctx).unwrap_err();
        assert!(
            matches!(&err, TemplateError::MalformedPlaceholder { raw } if raw == "phase.design")
        );
    }

    #[test]
    fn test_malformed_placeholder_unknown_subfield() {
        let ctx = sample_ctx();
        let err = Template::render("{{ phase.design.output.size }}", &ctx).unwrap_err();
        assert!(matches!(
            &err,
            TemplateError::MalformedPlaceholder { raw } if raw == "phase.design.output.size"
        ));
    }

    #[test]
    fn test_spec_missing_in_context() {
        // spec referenced but not provided
        let ctx = TemplateContext::new();
        let err = Template::render("{{ spec }}", &ctx).unwrap_err();
        assert!(matches!(
            &err,
            TemplateError::UnresolvedPlaceholder { name } if name == "spec"
        ));
    }

    #[test]
    fn test_literal_text_preserved() {
        let ctx = TemplateContext::new().with_spec("hello".to_string());
        let result = Template::render("before {{ spec }} middle {{ spec }} after", &ctx).unwrap();
        assert_eq!(result, "before hello middle hello after");
    }

    #[test]
    fn test_empty_template() {
        let ctx = TemplateContext::new();
        let result = Template::render("", &ctx).unwrap();
        assert_eq!(result, "");
    }

    #[test]
    fn test_template_with_no_placeholders() {
        let ctx = TemplateContext::new();
        let result = Template::render("Just plain text.", &ctx).unwrap();
        assert_eq!(result, "Just plain text.");
    }
}
