use anyhow::{anyhow, Context, Result};

use crate::workflow::grammar::{self, InputRef};
use crate::workflow::WorkflowSource;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowExplanation {
    pub name: String,
    pub description: Option<String>,
    pub source: String,
    pub phases: Vec<PhaseExplanation>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhaseExplanation {
    pub index: usize,
    pub name: String,
    pub dependencies: Vec<String>,
    pub inputs: Vec<String>,
    pub strategy: StrategyExplanation,
    pub backends: Vec<String>,
    pub prompt_template: String,
    pub output: String,
    pub verify: VerifyExplanation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StrategyExplanation {
    Single,
    Parallel { min_responses: usize },
    Escalating { pass_failure_context: bool },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerifyExplanation {
    None,
    ContractReserved,
}

pub async fn explain_workflow_source(source: WorkflowSource) -> Result<WorkflowExplanation> {
    let (display_name, text) = read_source_text(&source).await?;

    if !text.contains("[[phases]]") {
        anyhow::bail!(
            "workflow explanation supports phase-based workflows only: {}",
            display_name
        );
    }

    let workflow: grammar::Workflow =
        text.parse()
            .map_err(|errors: Vec<grammar::WorkflowError>| {
                invalid_workflow_error(&display_name, errors)
            })?;

    explain_workflow(&workflow, display_name)
}

async fn read_source_text(source: &WorkflowSource) -> Result<(String, String)> {
    match source {
        WorkflowSource::File(path) => {
            let text = tokio::fs::read_to_string(path)
                .await
                .with_context(|| format!("failed to read workflow {}", path.display()))?;
            Ok((path.display().to_string(), text))
        }
        WorkflowSource::Embedded { name, content } => {
            Ok((format!("embedded:{}", name), content.to_string()))
        }
    }
}

fn invalid_workflow_error(source: &str, errors: Vec<grammar::WorkflowError>) -> anyhow::Error {
    let mut message = format!("Invalid workflow: {source}");
    for error in errors {
        message.push_str(&format!("\n  - {error}"));
    }
    anyhow!(message)
}

pub fn explain_workflow(
    workflow: &grammar::Workflow,
    source: impl Into<String>,
) -> Result<WorkflowExplanation> {
    let errors = workflow.validate();
    if !errors.is_empty() {
        return Err(invalid_workflow_error(&source.into(), errors));
    }

    let phases = workflow
        .phases
        .iter()
        .enumerate()
        .map(|(index, phase)| {
            let dependencies = grammar::Workflow::resolve_inputs(phase)
                .unwrap_or_default()
                .into_iter()
                .filter_map(|input| match input {
                    InputRef::PhaseRef(name) => Some(name),
                    InputRef::VarRef(_) | InputRef::Spec => None,
                })
                .collect();

            PhaseExplanation {
                index: index + 1,
                name: phase.name.clone(),
                dependencies,
                inputs: phase.inputs.clone(),
                strategy: explain_strategy(&phase.strategy),
                backends: phase.backends.clone(),
                prompt_template: phase.prompt_template.clone(),
                output: phase.output.clone(),
                verify: if phase.contract.is_some() {
                    VerifyExplanation::ContractReserved
                } else {
                    VerifyExplanation::None
                },
            }
        })
        .collect();

    Ok(WorkflowExplanation {
        name: workflow.name.clone(),
        description: workflow.description.clone(),
        source: source.into(),
        phases,
        warnings: workflow.lint(),
    })
}

fn explain_strategy(strategy: &grammar::Strategy) -> StrategyExplanation {
    match strategy {
        grammar::Strategy::Single => StrategyExplanation::Single,
        grammar::Strategy::ParallelFanOut { min_responses } => StrategyExplanation::Parallel {
            min_responses: *min_responses,
        },
        grammar::Strategy::EscalatingRetry {
            pass_failure_context,
        } => StrategyExplanation::Escalating {
            pass_failure_context: *pass_failure_context,
        },
    }
}

pub fn render_text(explanation: &WorkflowExplanation) -> String {
    let mut output = String::new();

    output.push_str(&format!("Workflow: {}\n", explanation.name));
    output.push_str(&format!("Source: {}\n", explanation.source));
    if let Some(description) = &explanation.description {
        output.push_str(&format!("Description: {}\n", description));
    }

    if !explanation.warnings.is_empty() {
        output.push_str("\nWarnings:\n");
        for warning in &explanation.warnings {
            output.push_str(&format!("  - {}\n", warning));
        }
    }

    output.push_str("\nPhase order:\n");
    for phase in &explanation.phases {
        if phase.dependencies.is_empty() {
            output.push_str(&format!("  {}. {}\n", phase.index, phase.name));
        } else {
            output.push_str(&format!(
                "  {}. {} (depends on: {})\n",
                phase.index,
                phase.name,
                phase.dependencies.join(", ")
            ));
        }
    }

    output.push_str("\nPhases:\n");
    for (idx, phase) in explanation.phases.iter().enumerate() {
        if idx > 0 {
            output.push('\n');
        }
        output.push_str(&format!("  {}\n", phase.name));
        output.push_str(&format!(
            "    strategy: {}\n",
            render_strategy(&phase.strategy)
        ));
        output.push_str(&format!("    backends: {}\n", render_list(&phase.backends)));
        output.push_str(&format!("    inputs: {}\n", render_list(&phase.inputs)));
        if !phase.dependencies.is_empty() {
            output.push_str(&format!(
                "    depends_on: {}\n",
                phase.dependencies.join(", ")
            ));
        }
        output.push_str(&format!("    prompt_template: {}\n", phase.prompt_template));
        output.push_str(&format!("    output: {}\n", phase.output));
        output.push_str(&format!("    verify: {}\n", render_verify(&phase.verify)));
    }

    output
}

fn render_strategy(strategy: &StrategyExplanation) -> String {
    match strategy {
        StrategyExplanation::Single => "single".to_string(),
        StrategyExplanation::Parallel { min_responses } => {
            format!("parallel (min_responses: {min_responses})")
        }
        StrategyExplanation::Escalating {
            pass_failure_context,
        } => format!("escalating (pass_failure_context: {pass_failure_context})"),
    }
}

fn render_verify(verify: &VerifyExplanation) -> &'static str {
    match verify {
        VerifyExplanation::None => "none",
        VerifyExplanation::ContractReserved => "contract reserved",
    }
}

fn render_list(values: &[String]) -> String {
    if values.is_empty() {
        "none".to_string()
    } else {
        values.join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_workflow(toml: &str) -> grammar::Workflow {
        toml.parse().unwrap()
    }

    #[test]
    fn explain_builds_phase_dependencies_from_phase_inputs() {
        let workflow = parse_workflow(
            r#"
name = "deps"

[[phases]]
name = "design"
strategy = { single = {} }
backends = ["claude/"]
prompt_template = "design"
inputs = ["spec"]
output = "design.md"

[[phases]]
name = "review"
strategy = { single = {} }
backends = ["claude/"]
prompt_template = "review"
inputs = ["phase:design"]
output = "review.md"
"#,
        );

        let explanation = explain_workflow(&workflow, "fixture").unwrap();
        assert_eq!(explanation.phases[1].dependencies, vec!["design"]);
    }

    #[test]
    fn explain_preserves_non_phase_inputs() {
        let workflow = parse_workflow(
            r#"
name = "inputs"

[[phases]]
name = "design"
strategy = { single = {} }
backends = ["claude/"]
prompt_template = "design"
inputs = ["spec", "var:topic"]
output = "design.md"
"#,
        );

        let explanation = explain_workflow(&workflow, "fixture").unwrap();
        assert_eq!(explanation.phases[0].inputs, vec!["spec", "var:topic"]);
        assert!(explanation.phases[0].dependencies.is_empty());
    }

    #[test]
    fn render_strategy_variants() {
        assert_eq!(render_strategy(&StrategyExplanation::Single), "single");
        assert_eq!(
            render_strategy(&StrategyExplanation::Parallel { min_responses: 2 }),
            "parallel (min_responses: 2)"
        );
        assert_eq!(
            render_strategy(&StrategyExplanation::Escalating {
                pass_failure_context: true
            }),
            "escalating (pass_failure_context: true)"
        );
    }

    #[test]
    fn render_verify_contract_reserved() {
        assert_eq!(render_verify(&VerifyExplanation::None), "none");
        assert_eq!(
            render_verify(&VerifyExplanation::ContractReserved),
            "contract reserved"
        );
    }

    #[test]
    fn render_text_is_stable_for_design_doc_tdd_shape() {
        let workflow = parse_workflow(include_str!(
            "../../tests/fixtures/workflows/design-doc-tdd.toml"
        ));
        let explanation =
            explain_workflow(&workflow, "tests/fixtures/workflows/design-doc-tdd.toml").unwrap();
        let rendered = render_text(&explanation);

        assert!(rendered.contains("Workflow: design-doc-tdd"));
        assert!(rendered.contains("2. review (depends on: design)"));
        assert!(rendered.contains("strategy: parallel (min_responses: 2)"));
        assert!(rendered.contains("verify: contract reserved"));
    }
}
