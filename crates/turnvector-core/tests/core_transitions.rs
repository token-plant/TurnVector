use turnvector_core::{
    BackendGeneration, Core, CoreEvent, CoreFault, CoreOutcome, DomainRejection, EventSequence,
    GenerationVector, HotPathWorkBudget, HotPathWorkWitness, OperationId,
    RuntimeOverheadGeneration, SafetyGeneration, SchedulerGeneration, WorkBudgetError,
    WorkDimension,
};
fn sequence(value: u64) -> EventSequence {
    EventSequence::new(value).unwrap()
}
fn operation(value: u128) -> OperationId {
    OperationId::new(value).unwrap()
}
fn work(values: [u64; 5]) -> HotPathWorkWitness {
    HotPathWorkWitness::new(values)
}
fn generations() -> GenerationVector {
    GenerationVector::new(
        SchedulerGeneration::new(1).unwrap(),
        BackendGeneration::new(2).unwrap(),
        SafetyGeneration::new(3).unwrap(),
        RuntimeOverheadGeneration::new(4).unwrap(),
    )
}
#[test]
fn contiguous_event_commits_ordered_effects_atomically() {
    let mut core = Core::<4>::bootstrap(sequence(1), generations());
    let transition = core.handle(CoreEvent::operation(
        sequence(1),
        operation(10),
        Some(operation(11)),
    ));
    assert_eq!(transition.outcome(), &CoreOutcome::Accepted);
    let effects = transition.effects().iter().collect::<Vec<_>>();
    assert_eq!(effects.len(), 2);
    assert_eq!(effects[0].operation(), operation(10));
    assert_eq!(effects[0].depends_on(), None);
    assert_eq!(effects[0].generations(), generations());
    assert_eq!(effects[1].operation(), operation(11));
    assert_eq!(effects[1].depends_on(), Some(operation(10)));
    assert_eq!(effects[1].generations(), generations());
    assert_eq!(core.state().expected_sequence(), sequence(2));
    assert_eq!(core.state().operation_count(), 2);
}
#[test]
fn domain_rejection_consumes_sequence_without_applying_requested_state() {
    let mut core = Core::<4>::bootstrap(sequence(1), generations());
    let accepted = core.handle(CoreEvent::operation(sequence(1), operation(10), None));
    assert_eq!(accepted.outcome(), &CoreOutcome::Accepted);
    assert_eq!(accepted.work(), work([1, 0, 0, 1, 4]));
    let rejected = core.handle(CoreEvent::operation(sequence(2), operation(10), None));
    assert_eq!(
        rejected.outcome(),
        &CoreOutcome::Rejected(DomainRejection::OperationIdCollision(operation(10)))
    );
    assert!(rejected.effects().is_empty());
    assert_eq!(core.state().expected_sequence(), sequence(3));
    assert_eq!(core.state().operation_count(), 1);
}
#[test]
fn noncontiguous_event_faults_and_preserves_committed_state() {
    let mut core = Core::<4>::bootstrap(sequence(1), generations());
    core.handle(CoreEvent::operation(sequence(1), operation(10), None));
    let before = core.state().clone();
    let fault = CoreFault::NonContiguousEvent {
        expected: sequence(2),
        actual: sequence(3),
    };
    let faulted = core.handle(CoreEvent::operation(sequence(3), operation(11), None));
    assert_eq!(faulted.outcome(), &CoreOutcome::Fault(fault));
    assert!(faulted.effects().is_empty());
    assert_eq!(core.state(), &before);
    assert_eq!(core.fault(), Some(fault));
    let after_fault = core.handle(CoreEvent::operation(sequence(2), operation(12), None));
    assert_eq!(after_fault.outcome(), faulted.outcome());
    assert!(after_fault.effects().is_empty());
    assert_eq!(after_fault.work(), work([1, 0, 0, 0, 0]));
    assert_eq!(core.state(), &before);
}
#[test]
fn successor_overflow_discards_the_candidate_transition() {
    let maximum = sequence(u64::MAX);
    let mut core = Core::<2>::bootstrap(maximum, generations());
    let before = core.state().clone();
    let faulted = core.handle(CoreEvent::operation(
        maximum,
        operation(10),
        Some(operation(11)),
    ));
    assert_eq!(
        faulted.outcome(),
        &CoreOutcome::Fault(CoreFault::EventSequenceOverflow { current: maximum })
    );
    assert!(faulted.effects().is_empty());
    assert_eq!(core.state(), &before);
}

#[test]
fn budget_exhaustion_rejects_without_partial_state_or_effects() {
    let budget = HotPathWorkBudget::try_new(work([34, 1_048_528, 0, 0, 8])).unwrap();
    let mut core = Core::<4>::bootstrap_with_work_budget(sequence(1), generations(), budget);
    let transition = core.handle(CoreEvent::operation(sequence(1), operation(10), None));
    assert_eq!(
        transition.outcome(),
        &CoreOutcome::Rejected(DomainRejection::HotPathWorkBudget(
            WorkBudgetError::BudgetExceeded(WorkDimension::CandidateWork, 0, 1)
        ))
    );
    assert!(transition.effects().is_empty());
    assert_eq!(
        (
            core.state().operation_count(),
            core.state().expected_sequence()
        ),
        (0, sequence(2))
    );
}
#[test]
fn operation_lookup_uses_counted_binary_work_without_a_full_state_scan() {
    let mut core = Core::<16>::bootstrap(sequence(1), generations());
    for offset in 0..8 {
        let result = core.handle(CoreEvent::operation(
            sequence(offset + 1),
            operation(u128::from(offset + 1) * 10),
            None,
        ));
        assert_eq!(result.outcome(), &CoreOutcome::Accepted);
    }
    let transition = core.handle(CoreEvent::operation(sequence(9), operation(5), None));
    assert_eq!(transition.outcome(), &CoreOutcome::Accepted);
    assert_eq!(transition.work(), work([5, 128, 0, 1, 4]));
}
#[test]
fn work_budgets_reject_overflow_and_truncation() {
    assert!(HotPathWorkBudget::try_new(work([35, 1_048_528, 0, 2, 8])).is_err());
    assert!(matches!(
        work([u64::MAX, 0, 0, 0, 0]).checked_add(work([1, 0, 0, 0, 0])),
        Err(WorkBudgetError::CounterOverflow(
            WorkDimension::VisitedEntities
        ))
    ));
}
