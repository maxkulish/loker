//! TDD test contract for the phase prompt template engine (CLO-289).
//!
//! Tests `crate::workflow::template::Template` — the substitution engine
//! that renders `{{ spec }}`, `{{ phase.<name>.output }}`,
//! `{{ phase.<name>.output.path }}`, and `{{ var.<name> }}` placeholders.
//!
//! Acceptance criteria:
//! 1. `{{ spec }}` substitution renders the spec content verbatim.
//! 2. `{{ phase.X.output }}` substitution renders the full phase output content.
//! 3. `{{ phase.X.output.path }}` substitution resolves to the relative path.
//! 4. `{{ var.name }}` substitution renders the var value; missing = error.
//! 5. Strict mode: `{{ undefined }}` raises `UnresolvedPlaceholder`.
//! 6. Whitespace tolerance: `{{spec}}`, `{{ spec }}`, `{{  spec  }}` all resolve.
//! 7. No template injection: `{{` in artefact body is literal.
//! 8. All four `design-doc-tdd` templates render against synthetic artefacts.

use loker::workflow::template::{PhaseOutput, Template, TemplateContext, TemplateError};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_phase_output(content: &str, path: &str) -> PhaseOutput {
    PhaseOutput {
        content: content.to_string(),
        path: path.to_string(),
    }
}

fn sample_ctx() -> TemplateContext {
    TemplateContext::new()
        .with_spec("Build a calculator CLI".to_string())
        .with_phase_output(
            "design",
            make_phase_output(
                "# Design\n\nUse Rust with clap for argument parsing.",
                "design.md",
            ),
        )
        .with_phase_output(
            "review",
            make_phase_output("{\"verdict\": \"APPROVE\", \"score\": 95}", "review.md"),
        )
        .with_phase_output(
            "implement",
            make_phase_output("Modified: src/main.rs\nAdded: src/calculator.rs", "changes"),
        )
        .with_var("branch", "feat-calculator")
}

// ---------------------------------------------------------------------------
// Test 1: `{{ spec }}` substitution
// ---------------------------------------------------------------------------

/// Render `design.md.tmpl` with a fixture spec; output contains the spec
/// content verbatim.
#[test]
fn spec_substitution() {
    let ctx = TemplateContext::new().with_spec("Build a web server in Rust.".to_string());
    let result = Template::render(
        "Read the specification: {{ spec }}\n\nDesign accordingly.",
        &ctx,
    )
    .unwrap();
    assert!(result.contains("Build a web server in Rust."));
    assert!(result.starts_with("Read the specification:"));
    assert!(result.ends_with("Design accordingly."));
}

// ---------------------------------------------------------------------------
// Test 2: `{{ phase.X.output }}` substitution
// ---------------------------------------------------------------------------

/// Simulate a manifest with a written `design.md`; rendering includes its
/// full content.
#[test]
fn phase_output_substitution() {
    let mut ctx = TemplateContext::new();
    let design_content = "# Design\n\nModule structure:\n- `src/lib.rs`\n- `src/cli.rs`";
    ctx.phase_outputs.insert(
        "design".to_string(),
        make_phase_output(design_content, "design.md"),
    );

    let result = Template::render("Design: {{ phase.design.output }}", &ctx).unwrap();
    assert_eq!(result, format!("Design: {}", design_content));
}

// ---------------------------------------------------------------------------
// Test 3: `{{ phase.X.output.path }}` substitution
// ---------------------------------------------------------------------------

/// Same as test 2 but resolves to the relative artefact path.
#[test]
fn phase_output_path_substitution() {
    let mut ctx = TemplateContext::new();
    ctx.phase_outputs.insert(
        "review".to_string(),
        make_phase_output("content", "review.md"),
    );

    let result = Template::render("Path: {{ phase.review.output.path }}", &ctx).unwrap();
    assert_eq!(result, "Path: review.md");
}

// ---------------------------------------------------------------------------
// Test 4: `{{ var.name }}` substitution
// ---------------------------------------------------------------------------

/// Pass `--var name=value`; render uses `value`. Missing var raises
/// `UnresolvedPlaceholder`.
#[test]
fn var_substitution_found() {
    let ctx = TemplateContext::new().with_var("language", "Rust");
    let result = Template::render("Language: {{ var.language }}", &ctx).unwrap();
    assert_eq!(result, "Language: Rust");
}

#[test]
fn var_substitution_missing() {
    let ctx = TemplateContext::new();
    let err = Template::render("{{ var.nonexistent }}", &ctx).unwrap_err();
    assert!(
        matches!(&err, TemplateError::UnresolvedPlaceholder { name } if name == "var.nonexistent"),
        "Expected UnresolvedPlaceholder for var.nonexistent, got: {:?}",
        err
    );
}

// ---------------------------------------------------------------------------
// Test 5: Strict mode — `{{ undefined }}` raises UnresolvedPlaceholder
// ---------------------------------------------------------------------------

#[test]
fn strict_mode_undefined_placeholder() {
    let ctx = TemplateContext::new().with_spec("test".to_string());
    let err = Template::render("{{ undefined }}", &ctx).unwrap_err();
    assert!(
        matches!(&err, TemplateError::UnresolvedPlaceholder { name } if name == "undefined"),
        "Expected UnresolvedPlaceholder for 'undefined', got: {:?}",
        err
    );
}

#[test]
fn strict_mode_partial_undefined() {
    // Part of the template has a valid placeholder, part has an undefined one.
    let ctx = TemplateContext::new().with_spec("test".to_string());
    let err = Template::render("Valid: {{ spec }}, Invalid: {{ missing }}", &ctx).unwrap_err();
    assert!(
        matches!(&err, TemplateError::UnresolvedPlaceholder { name } if name == "missing"),
        "Expected UnresolvedPlaceholder for 'missing', got: {:?}",
        err
    );
}

// ---------------------------------------------------------------------------
// Test 6: Whitespace tolerance
// ---------------------------------------------------------------------------

/// `{{spec}}`, `{{ spec }}`, `{{  spec  }}` all resolve identically.
#[test]
fn whitespace_tolerance_no_spaces() {
    let ctx = TemplateContext::new().with_spec("hello".to_string());
    assert_eq!(Template::render("{{spec}}", &ctx).unwrap(), "hello");
}

#[test]
fn whitespace_tolerance_one_space() {
    let ctx = TemplateContext::new().with_spec("hello".to_string());
    assert_eq!(Template::render("{{ spec }}", &ctx).unwrap(), "hello");
}

#[test]
fn whitespace_tolerance_extra_spaces() {
    let ctx = TemplateContext::new().with_spec("hello".to_string());
    assert_eq!(Template::render("{{  spec  }}", &ctx).unwrap(), "hello");
}

#[test]
fn whitespace_tolerance_mixed() {
    let ctx = TemplateContext::new()
        .with_spec("A".to_string())
        .with_var("b", "B".to_string());
    assert_eq!(
        Template::render("{{spec}}-{{ var.b }}", &ctx).unwrap(),
        "A-B"
    );
    assert_eq!(
        Template::render("{{ spec  }}-{{  var.b }}", &ctx).unwrap(),
        "A-B"
    );
}

// ---------------------------------------------------------------------------
// Test 7: No template injection
// ---------------------------------------------------------------------------

/// `{{` inside an artefact body is treated as literal text, not re-evaluated.
#[test]
fn no_template_injection_in_output() {
    let mut ctx = TemplateContext::new();
    ctx.phase_outputs.insert(
        "x".to_string(),
        make_phase_output("value is {{ secret }}", "x.md"),
    );

    let result = Template::render("{{ phase.x.output }}", &ctx).unwrap();
    assert_eq!(result, "value is {{ secret }}");
}

#[test]
fn no_template_injection_in_spec() {
    let ctx = TemplateContext::new().with_spec("Look: {{ phase.design.output }}".to_string());
    let result = Template::render("Spec says: {{ spec }}", &ctx).unwrap();
    assert_eq!(result, "Spec says: Look: {{ phase.design.output }}");
}

// ---------------------------------------------------------------------------
// Test 8: All four templates render
// ---------------------------------------------------------------------------

/// End-to-end fixture run renders every `design-doc-tdd` template against
/// synthetic upstream artefacts; no errors.
#[test]
fn all_four_templates_render() {
    let ctx = sample_ctx();

    // design.md.tmpl
    let design_tmpl = r#"You are a senior systems architect.

Read the specification below and produce a complete design document covering:
1. Problem statement
2. Goals and non-goals
3. Architecture (modules, data flow, concrete types)
4. Public API surface (Rust trait / struct signatures)
5. Test plan (unit, integration, manual)
6. Migration / rollout
7. Open questions

Output format: Markdown with numbered sections.

--- Specification ---

{{ spec }}"#
        .trim();

    // review.md.tmpl (abbreviated for test)
    let review_tmpl = r#"Review the design document below.

--- Design Document ---

{{ phase.design.output }}

--- Specification Excerpt ---

{{ spec }}"#
        .trim();

    // implement.md.tmpl (abbreviated)
    let implement_tmpl = r#"Implement the changes described.

--- Design Document ---

{{ phase.design.output }}

--- Review ---

{{ phase.review.output }}"#
        .trim();

    // verify.md.tmpl (abbreviated)
    let verify_tmpl = r#"Verify the changes below.

--- Changes ---

{{ phase.implement.output }}

--- Design Document ---

{{ phase.design.output }}"#
        .trim();

    // Render all four — no errors
    let design_out = Template::render(design_tmpl, &ctx).unwrap();
    assert!(design_out.contains("Build a calculator CLI"));
    // design template uses {{ spec }} which is "Build a calculator CLI"

    let review_out = Template::render(review_tmpl, &ctx).unwrap();
    assert!(review_out.contains("Build a calculator CLI"));
    assert!(review_out.contains("Use Rust with clap for argument parsing."));

    let implement_out = Template::render(implement_tmpl, &ctx).unwrap();
    assert!(implement_out.contains("Use Rust with clap for argument parsing."));
    assert!(implement_out.contains("\"verdict\": \"APPROVE\""));

    let verify_out = Template::render(verify_tmpl, &ctx).unwrap();
    assert!(verify_out.contains("Modified: src/main.rs"));
    assert!(verify_out.contains("Use Rust with clap for argument parsing."));
}

// ---------------------------------------------------------------------------
// Edge cases
// ---------------------------------------------------------------------------

#[test]
fn empty_template_returns_empty() {
    let ctx = TemplateContext::new();
    assert_eq!(Template::render("", &ctx).unwrap(), "");
}

#[test]
fn template_with_no_placeholders() {
    let ctx = TemplateContext::new();
    assert_eq!(
        Template::render("Just plain text.", &ctx).unwrap(),
        "Just plain text."
    );
}

#[test]
fn literal_braces_not_placeholders() {
    let ctx = TemplateContext::new();
    // Brace pairs that don't match the placeholder pattern
    assert_eq!(
        Template::render("Not a { placeholder }", &ctx).unwrap(),
        "Not a { placeholder }"
    );
    // {{ without }} is not a valid placeholder and does not match the regex,
    // so it is treated as literal text.
    assert_eq!(Template::render("{{ broken", &ctx).unwrap(), "{{ broken");
}

#[test]
fn multiple_placeholders_in_order() {
    let ctx = sample_ctx();
    let result = Template::render(
        "S:{{ spec }}|D:{{ phase.design.output }}|R:{{ phase.review.output.path }}|V:{{ var.branch }}",
        &ctx,
    )
    .unwrap();
    assert_eq!(
        result,
        "S:Build a calculator CLI|D:# Design\n\nUse Rust with clap for argument parsing.|R:review.md|V:feat-calculator"
    );
}
