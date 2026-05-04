use std::path::Path;
use std::sync::Arc;

use crate::aggregator::{
    AggregateInput, Aggregator, BallotSchema, BranchFailure, BranchOutcome, BranchSuccess,
    TieBreak, VoteConfig,
};
use crate::strategy::verify::HumanVerifier;
use crate::strategy::{EscalatingRetry, ParallelFanOut, SingleModel, Strategy, VerifyHook};

use super::{AggregatorName, PhaseConfig, PhaseError, VerifyHookName};

pub fn resolve_aggregator(name: AggregatorName) -> Result<Aggregator, PhaseError> {
    Ok(match name {
        AggregatorName::First => Aggregator::first(),
        AggregatorName::Concat => Aggregator::concat("## {backend_id}"),
        AggregatorName::Vote => Aggregator::vote(VoteConfig {
            ballot_schema: BallotSchema::FreeText,
            tie_break: TieBreak::FirstResponder,
            abstain_threshold: usize::MAX,
        }),
        AggregatorName::AnyFail => Aggregator::any_fail(),
        AggregatorName::AllPass => Aggregator::all_pass(),
    })
}

pub fn resolve_strategy(
    cfg: &PhaseConfig,
    verify: Option<Arc<dyn VerifyHook>>,
) -> Result<Arc<dyn Strategy>, PhaseError> {
    Ok(match cfg.strategy {
        super::StrategyName::Single => Arc::new(SingleModel::new(
            cfg.backend.clone().ok_or_else(|| {
                PhaseError::InvalidConfig("single strategy requires backend".into())
            })?,
            cfg.prompt_template.clone(),
        )),
        super::StrategyName::Parallel => Arc::new(ParallelFanOut::new(
            cfg.targets.clone(),
            cfg.min_responses,
            cfg.prompt_template.clone(),
            resolve_aggregator(cfg.aggregator)?,
        )),
        super::StrategyName::EscalatingRetry => {
            let verify = verify.ok_or_else(|| {
                PhaseError::InvalidConfig("escalating_retry strategy requires verify hook".into())
            })?;
            let rungs = cfg
                .rungs
                .iter()
                .map(|r| crate::strategy::escalating_retry::Rung::new(r.tier, r.backend.clone()))
                .collect();
            Arc::new(
                EscalatingRetry::new(rungs, cfg.prompt_template.clone(), verify)
                    .with_pass_failure_context(cfg.pass_failure_context),
            )
        }
    })
}

pub fn resolve_verify_hook(
    cfg: &PhaseConfig,
    verify: Option<Arc<dyn VerifyHook>>,
) -> Result<Option<Arc<dyn VerifyHook>>, PhaseError> {
    match &cfg.verify {
        VerifyHookName::None => Ok(None),
        VerifyHookName::RunCommand | VerifyHookName::LlmVerifier => Ok(verify),
        VerifyHookName::HumanVerifier(cfg) => {
            Ok(Some(std::sync::Arc::new(HumanVerifier::new(cfg.clone()))))
        }
    }
}

pub fn canonical_bytes(
    run_dir: &Path,
    output: &crate::strategy::StrategyOutput,
    aggregator: &Aggregator,
) -> Result<(Vec<u8>, u32), PhaseError> {
    if let Some(path) = &output.aggregate_output_path {
        let path = resolve_output_path(run_dir, path);
        if path.is_file() {
            return Ok((std::fs::read(path)?, selected_attempt_index(output)));
        }
    }

    match aggregator {
        Aggregator::First => winning_success_bytes(run_dir, output),
        Aggregator::Concat { .. } | Aggregator::AllPass => {
            let input = aggregate_input_from_attempt_paths(run_dir, output)?;
            let aggregate = aggregator.aggregate(
                input,
                &[],
                &crate::strategy::PhaseContext::new(&output.phase, output.run_id),
            )?;
            Ok((aggregate.text.into_bytes(), selected_attempt_index(output)))
        }
        Aggregator::AnyFail => first_success_bytes(run_dir, output),
        Aggregator::Vote { .. } | Aggregator::LLMJudge { .. } => {
            first_success_bytes(run_dir, output)
        }
    }
}

fn first_success_bytes(
    run_dir: &Path,
    output: &crate::strategy::StrategyOutput,
) -> Result<(Vec<u8>, u32), PhaseError> {
    let (idx, attempt) = output
        .attempts
        .iter()
        .enumerate()
        .find(|(_, a)| {
            !a.finish_reasons
                .contains(&crate::strategy::FinishReason::Error)
        })
        .ok_or_else(|| PhaseError::PhaseFailed {
            phase: output.phase.clone(),
            message: "no successful attempts".into(),
        })?;
    Ok((
        std::fs::read(resolve_output_path(run_dir, &attempt.output_path))?,
        idx as u32,
    ))
}

fn winning_success_bytes(
    run_dir: &Path,
    output: &crate::strategy::StrategyOutput,
) -> Result<(Vec<u8>, u32), PhaseError> {
    let (idx, attempt) = output
        .attempts
        .iter()
        .enumerate()
        .rev()
        .find(|(_, a)| {
            !a.finish_reasons
                .contains(&crate::strategy::FinishReason::Error)
                && a.verify.status != crate::strategy::VerifyStatus::Fail
        })
        .ok_or_else(|| PhaseError::PhaseFailed {
            phase: output.phase.clone(),
            message: "no successful attempts".into(),
        })?;
    Ok((
        std::fs::read(resolve_output_path(run_dir, &attempt.output_path))?,
        idx as u32,
    ))
}

fn selected_attempt_index(output: &crate::strategy::StrategyOutput) -> u32 {
    output
        .attempts
        .iter()
        .enumerate()
        .rev()
        .find(|(_, a)| {
            !a.finish_reasons
                .contains(&crate::strategy::FinishReason::Error)
                && a.verify.status != crate::strategy::VerifyStatus::Fail
        })
        .map(|(idx, _)| idx as u32)
        .unwrap_or(0)
}

fn aggregate_input_from_attempt_paths(
    run_dir: &Path,
    output: &crate::strategy::StrategyOutput,
) -> Result<AggregateInput, PhaseError> {
    let branches = output
        .attempts
        .iter()
        .enumerate()
        .map(|(idx, attempt)| {
            let path = resolve_output_path(run_dir, &attempt.output_path);
            if attempt
                .finish_reasons
                .contains(&crate::strategy::FinishReason::Error)
            {
                Ok(BranchOutcome::Failure(BranchFailure {
                    backend_id: attempt.backend.clone(),
                    family: attempt.family.clone().unwrap_or_else(|| "local".into()),
                    index: idx + 1,
                    reason: "backend error".into(),
                }))
            } else {
                Ok(BranchOutcome::Success(BranchSuccess {
                    backend_id: attempt.backend.clone(),
                    family: attempt.family.clone().unwrap_or_else(|| "local".into()),
                    index: idx + 1,
                    output: std::fs::read_to_string(path)?,
                }))
            }
        })
        .collect::<Result<Vec<_>, std::io::Error>>()?;
    Ok(AggregateInput { branches })
}

fn resolve_output_path(run_dir: &Path, path: &str) -> std::path::PathBuf {
    let path = Path::new(path);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        run_dir.join(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::Kind;
    use crate::phase_runner::{AggregatorName, PhaseConfig, StrategyName};
    use crate::strategy::verify::{HumanDecision, HumanSeverity, HumanVerifierConfig};
    use crate::strategy::TargetSpec;
    use std::path::PathBuf;

    #[test]
    fn name_dispatch_resolves_known() {
        assert!(matches!(
            resolve_aggregator(AggregatorName::First).unwrap(),
            Aggregator::First
        ));
        assert!(matches!(
            resolve_aggregator(AggregatorName::AllPass).unwrap(),
            Aggregator::AllPass
        ));

        let single = PhaseConfig::single("p", "mock", "prompt", "out.md");
        assert!(resolve_strategy(&single, None).is_ok());

        let mut parallel = single.clone();
        parallel.strategy = StrategyName::Parallel;
        parallel.aggregator = AggregatorName::Concat;
        parallel.targets = vec![TargetSpec::new("mock")];
        assert!(resolve_strategy(&parallel, None).is_ok());

        assert!(resolve_verify_hook(&single, None).unwrap().is_none());
    }

    #[test]
    fn resolve_verify_hook_returns_human_verifier() {
        let mut single = PhaseConfig::single("p", "mock", "prompt", "out.md");
        single.verify = VerifyHookName::HumanVerifier(HumanVerifierConfig {
            run_dir: PathBuf::from("/tmp/run"),
            run_id: "run-1".into(),
            workflow: "wf".into(),
            phase: "p".into(),
            artefact_name: "out.md".into(),
            artefact_kind: Kind::DesignMd,
            severity: HumanSeverity::Medium,
            decision_options: vec![HumanDecision::Approve],
        });

        let hook = resolve_verify_hook(&single, None)
            .unwrap()
            .expect("hook should exist");
        assert_eq!(hook.name(), "HumanVerifier");
    }
}
