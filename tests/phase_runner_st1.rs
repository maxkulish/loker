use loker::aggregator::{AggregateInput, Aggregator, BranchFailure, BranchOutcome, BranchSuccess};

#[test]
fn aggregator_first_and_all_pass_variants_map_to_schema_labels() {
    assert_eq!(Aggregator::first().kind().as_str(), "first");
    assert_eq!(Aggregator::all_pass().kind().as_str(), "all_pass");

    let input = AggregateInput {
        branches: vec![
            BranchOutcome::Failure(BranchFailure {
                backend_id: "b0".into(),
                family: "local".into(),
                index: 1,
                reason: "nope".into(),
            }),
            BranchOutcome::Success(BranchSuccess {
                backend_id: "b1".into(),
                family: "local".into(),
                index: 2,
                output: "winner".into(),
            }),
        ],
    };
    let ctx = loker::strategy::PhaseContext::new("phase", uuid::Uuid::nil());
    let first = Aggregator::first()
        .aggregate(input, &[], &ctx)
        .expect("first aggregate");
    assert_eq!(first.text, "winner");

    let input = AggregateInput {
        branches: vec![BranchOutcome::Success(BranchSuccess {
            backend_id: "b1".into(),
            family: "local".into(),
            index: 1,
            output: "all good".into(),
        })],
    };
    let all_pass = Aggregator::all_pass()
        .aggregate(input, &[], &ctx)
        .expect("all pass aggregate");
    assert_eq!(all_pass.text, "all good");
}
