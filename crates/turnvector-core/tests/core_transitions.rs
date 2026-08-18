use turnvector_core::{
    BackendGeneration, Core, CoreEvent, CoreFault, CoreOutcome, DomainRejection, EventSequence,
    GenerationVector, OperationId, RuntimeOverheadGeneration, SafetyGeneration,
    SchedulerGeneration,
};
fn sequence(value: u64) -> EventSequence {
    EventSequence::new(value).unwrap()
}
fn operation(value: u128) -> OperationId {
    OperationId::new(value).unwrap()
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
