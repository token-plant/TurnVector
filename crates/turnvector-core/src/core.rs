use crate::{
    BoundedCollectionError, BoundedSet, BoundedVec, EventSequence, GenerationVector, OperationId,
};
const TRANSITION_EFFECT_CAPACITY: usize = 2;
type Effects = BoundedVec<Effect, TRANSITION_EFFECT_CAPACITY>;
type Operations<const N: usize> = BoundedSet<OperationId, N>;
/// One validated operation request presented at an exact Event Sequence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CoreEvent {
    sequence: EventSequence,
    operation: OperationId,
    follow_up: Option<OperationId>,
}
impl CoreEvent {
    #[must_use]
    pub const fn operation(
        sequence: EventSequence,
        operation: OperationId,
        follow_up: Option<OperationId>,
    ) -> Self {
        Self {
            sequence,
            operation,
            follow_up,
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Effect {
    operation: OperationId,
    generations: GenerationVector,
    depends_on: Option<OperationId>,
}
impl Effect {
    #[must_use]
    pub const fn operation(self) -> OperationId {
        self.operation
    }
    #[must_use]
    pub const fn generations(self) -> GenerationVector {
        self.generations
    }
    #[must_use]
    pub const fn depends_on(self) -> Option<OperationId> {
        self.depends_on
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DomainRejection {
    OperationIdCollision(OperationId),
    OperationCapacityExceeded,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CoreFault {
    NonContiguousEvent {
        expected: EventSequence,
        actual: EventSequence,
    },
    EventSequenceOverflow {
        current: EventSequence,
    },
    CandidateInvariant,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CoreOutcome {
    Accepted,
    Rejected(DomainRejection),
    Fault(CoreFault),
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CoreTransition {
    sequence: EventSequence,
    outcome: CoreOutcome,
    effects: Effects,
}
impl CoreTransition {
    #[must_use]
    pub const fn sequence(&self) -> EventSequence {
        self.sequence
    }
    #[must_use]
    pub const fn outcome(&self) -> &CoreOutcome {
        &self.outcome
    }
    #[must_use]
    pub const fn effects(&self) -> &Effects {
        &self.effects
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CoreState<const OPERATIONS: usize> {
    expected_sequence: EventSequence,
    generations: GenerationVector,
    operations: Operations<OPERATIONS>,
}
impl<const OPERATIONS: usize> CoreState<OPERATIONS> {
    #[must_use]
    pub const fn expected_sequence(&self) -> EventSequence {
        self.expected_sequence
    }
    #[must_use]
    pub const fn operation_count(&self) -> usize {
        self.operations.len()
    }
}
/// The deterministic Runtime Core transition coordinator.
/// ```compile_fail
/// use turnvector_core::Core;
/// fn fork<const N: usize>(core: Core<N>) { core.clone(); }
/// ```
#[derive(Debug, Eq, PartialEq)]
pub struct Core<const OPERATIONS: usize> {
    state: CoreState<OPERATIONS>,
    fault: Option<CoreFault>,
    #[cfg(test)]
    force_candidate_invariant_failure: bool,
}
impl<const OPERATIONS: usize> Core<OPERATIONS> {
    #[must_use]
    pub fn bootstrap(first_sequence: EventSequence, generations: GenerationVector) -> Self {
        Self {
            state: CoreState {
                expected_sequence: first_sequence,
                generations,
                operations: BoundedSet::new(),
            },
            fault: None,
            #[cfg(test)]
            force_candidate_invariant_failure: false,
        }
    }
    #[must_use]
    pub const fn state(&self) -> &CoreState<OPERATIONS> {
        &self.state
    }
    #[must_use]
    pub const fn fault(&self) -> Option<CoreFault> {
        self.fault
    }
    pub fn handle(&mut self, event: CoreEvent) -> CoreTransition {
        if let Some(fault) = self.fault {
            return transition(event.sequence, CoreOutcome::Fault(fault), BoundedVec::new());
        }
        if event.sequence != self.state.expected_sequence {
            return self.fail(
                event.sequence,
                CoreFault::NonContiguousEvent {
                    expected: self.state.expected_sequence,
                    actual: event.sequence,
                },
            );
        }
        let next_sequence = match event.sequence.next() {
            Ok(next) => next,
            Err(_) => {
                return self.fail(
                    event.sequence,
                    CoreFault::EventSequenceOverflow {
                        current: event.sequence,
                    },
                );
            }
        };
        match self.stage(&event) {
            Ok((operations, effects)) => {
                self.state = CoreState {
                    expected_sequence: next_sequence,
                    generations: self.state.generations,
                    operations,
                };
                transition(event.sequence, CoreOutcome::Accepted, effects)
            }
            Err(StageFailure::Rejected(rejection)) => {
                self.state.expected_sequence = next_sequence;
                transition(
                    event.sequence,
                    CoreOutcome::Rejected(rejection),
                    BoundedVec::new(),
                )
            }
            Err(StageFailure::Invariant) => {
                self.fail(event.sequence, CoreFault::CandidateInvariant)
            }
        }
    }
    fn stage(&self, event: &CoreEvent) -> Result<(Operations<OPERATIONS>, Effects), StageFailure> {
        let mut operations = self.state.operations.clone();
        let mut effects = BoundedVec::new();
        stage_operation(
            &mut operations,
            &mut effects,
            event.operation,
            None,
            self.state.generations,
        )?;
        if let Some(follow_up) = event.follow_up {
            stage_operation(
                &mut operations,
                &mut effects,
                follow_up,
                Some(event.operation),
                self.state.generations,
            )?;
        }
        if self.candidate_is_valid(&operations, &effects) {
            Ok((operations, effects))
        } else {
            Err(StageFailure::Invariant)
        }
    }
    fn candidate_is_valid(&self, ops: &Operations<OPERATIONS>, effects: &Effects) -> bool {
        #[cfg(test)]
        if self.force_candidate_invariant_failure {
            return false;
        }
        self.state.operations.len().checked_add(effects.len()) == Some(ops.len())
            && effects.iter().all(|effect| ops.contains(&effect.operation))
    }
    fn fail(&mut self, sequence: EventSequence, fault: CoreFault) -> CoreTransition {
        self.fault = Some(fault);
        transition(sequence, CoreOutcome::Fault(fault), BoundedVec::new())
    }
}
enum StageFailure {
    Rejected(DomainRejection),
    Invariant,
}
fn stage_operation<const OPERATIONS: usize>(
    operations: &mut Operations<OPERATIONS>,
    effects: &mut Effects,
    operation: OperationId,
    depends_on: Option<OperationId>,
    generations: GenerationVector,
) -> Result<(), StageFailure> {
    operations.try_insert(operation).map_err(|error| {
        StageFailure::Rejected(match error {
            BoundedCollectionError::Duplicate => DomainRejection::OperationIdCollision(operation),
            BoundedCollectionError::Full => DomainRejection::OperationCapacityExceeded,
        })
    })?;
    effects
        .try_push(Effect {
            operation,
            generations,
            depends_on,
        })
        .map_err(|_| StageFailure::Invariant)
}
fn transition(sequence: EventSequence, outcome: CoreOutcome, effects: Effects) -> CoreTransition {
    CoreTransition {
        sequence,
        outcome,
        effects,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        BackendGeneration, RuntimeOverheadGeneration, SafetyGeneration, SchedulerGeneration,
    };
    #[test]
    fn candidate_invariant_failure_preserves_state_and_latches_fault() {
        let sequence = EventSequence::new(1).unwrap();
        let generations = GenerationVector::new(
            SchedulerGeneration::new(1).unwrap(),
            BackendGeneration::new(1).unwrap(),
            SafetyGeneration::new(1).unwrap(),
            RuntimeOverheadGeneration::new(1).unwrap(),
        );
        let mut core = Core::<1>::bootstrap(sequence, generations);
        core.force_candidate_invariant_failure = true;
        let before = core.state().clone();
        let result = core.handle(CoreEvent::operation(
            sequence,
            OperationId::new(1).unwrap(),
            None,
        ));
        assert_eq!(
            result.outcome(),
            &CoreOutcome::Fault(CoreFault::CandidateInvariant)
        );
        assert!(result.effects().is_empty());
        assert_eq!(core.state(), &before);
        assert_eq!(core.fault(), Some(CoreFault::CandidateInvariant));
    }
}
