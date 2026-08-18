use crate::work::WorkMeter;
use crate::{
    BoundedVec, EventSequence, GenerationVector, HotPathWorkBudget, HotPathWorkWitness,
    OperationId, WorkBudgetError, WorkDimension,
};
const TRANSITION_EFFECT_CAPACITY: usize = 2;
const MAX_OPERATION_ENTRIES: usize = 32_768;
type Effects = BoundedVec<Effect, TRANSITION_EFFECT_CAPACITY>;
type Operations<const N: usize> = BoundedVec<OperationId, N>;
type Positions = [usize; TRANSITION_EFFECT_CAPACITY];
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
    HotPathWorkBudget(WorkBudgetError),
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
    work: HotPathWorkWitness,
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
    #[must_use]
    pub const fn work(&self) -> HotPathWorkWitness {
        self.work
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
    work_budget: HotPathWorkBudget,
    #[cfg(test)]
    force_candidate_invariant_failure: bool,
}
impl<const OPERATIONS: usize> Core<OPERATIONS> {
    #[must_use]
    pub fn bootstrap(first_sequence: EventSequence, generations: GenerationVector) -> Self {
        Self::bootstrap_with_work_budget(
            first_sequence,
            generations,
            HotPathWorkBudget::binary_maximum(),
        )
    }
    #[must_use]
    pub fn bootstrap_with_work_budget(
        first_sequence: EventSequence,
        generations: GenerationVector,
        work_budget: HotPathWorkBudget,
    ) -> Self {
        assert!(OPERATIONS <= MAX_OPERATION_ENTRIES);
        Self {
            state: CoreState {
                expected_sequence: first_sequence,
                generations,
                operations: BoundedVec::new(),
            },
            fault: None,
            work_budget,
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
        let mut work = WorkMeter::new(self.work_budget);
        work.record(WorkDimension::VisitedEntities, 1)
            .expect("validated budget covers one Core Event");
        if let Some(fault) = self.fault {
            return transition(
                event.sequence,
                CoreOutcome::Fault(fault),
                BoundedVec::new(),
                work.witness(),
            );
        }
        if event.sequence != self.state.expected_sequence {
            return self.fail(
                event.sequence,
                CoreFault::NonContiguousEvent {
                    expected: self.state.expected_sequence,
                    actual: event.sequence,
                },
                work.witness(),
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
                    work.witness(),
                );
            }
        };
        match self.stage(&event, &mut work) {
            Ok((positions, effects)) => {
                self.commit(&positions, &effects);
                self.state.expected_sequence = next_sequence;
                transition(
                    event.sequence,
                    CoreOutcome::Accepted,
                    effects,
                    work.witness(),
                )
            }
            Err(StageFailure::Rejected(rejection)) => {
                self.state.expected_sequence = next_sequence;
                transition(
                    event.sequence,
                    CoreOutcome::Rejected(rejection),
                    BoundedVec::new(),
                    work.witness(),
                )
            }
            Err(StageFailure::Invariant) => self.fail(
                event.sequence,
                CoreFault::CandidateInvariant,
                work.witness(),
            ),
        }
    }
    fn stage(
        &self,
        event: &CoreEvent,
        work: &mut WorkMeter,
    ) -> Result<(Positions, Effects), StageFailure> {
        let mut positions = [0; TRANSITION_EFFECT_CAPACITY];
        let mut effects = BoundedVec::new();
        stage_operation(
            &self.state.operations,
            &mut positions,
            &mut effects,
            event.operation,
            None,
            self.state.generations,
            work,
        )?;
        if let Some(follow_up) = event.follow_up {
            stage_operation(
                &self.state.operations,
                &mut positions,
                &mut effects,
                follow_up,
                Some(event.operation),
                self.state.generations,
                work,
            )?;
        }
        #[cfg(test)]
        if self.force_candidate_invariant_failure {
            positions[0] = self.state.operations.len() + 1;
        }
        let checks = 1 + 3 * effects.len() as u64 + u64::from(effects.len() == 2);
        work.record(WorkDimension::InvariantChecks, checks)?;
        let distinct = effects.len() != 2
            || effects.get(0).unwrap().operation != effects.get(1).unwrap().operation;
        let valid = self.state.operations.len() + effects.len() <= OPERATIONS
            && distinct
            && effects.iter().enumerate().all(|(index, effect)| {
                self.state
                    .operations
                    .ordered_at(positions[index], &effect.operation)
            });
        if !valid {
            return Err(StageFailure::Invariant);
        }
        let copied = copied_operation_bytes(self.state.operations.len(), &positions, &effects);
        work.record(WorkDimension::CopiedBytes, copied)?;
        Ok((positions, effects))
    }
    fn commit(&mut self, positions: &Positions, effects: &Effects) {
        for index in 0..effects.len() {
            self.state.operations.insert_at(
                adjusted_position(index, positions, effects),
                effects.get(index).unwrap().operation,
            );
        }
    }
    fn fail(
        &mut self,
        sequence: EventSequence,
        fault: CoreFault,
        work: HotPathWorkWitness,
    ) -> CoreTransition {
        self.fault = Some(fault);
        transition(sequence, CoreOutcome::Fault(fault), BoundedVec::new(), work)
    }
}
enum StageFailure {
    Rejected(DomainRejection),
    Invariant,
}
impl From<WorkBudgetError> for StageFailure {
    fn from(error: WorkBudgetError) -> Self {
        Self::Rejected(DomainRejection::HotPathWorkBudget(error))
    }
}
fn stage_operation<const OPERATIONS: usize>(
    operations: &Operations<OPERATIONS>,
    positions: &mut Positions,
    effects: &mut Effects,
    operation: OperationId,
    depends_on: Option<OperationId>,
    generations: GenerationVector,
    work: &mut WorkMeter,
) -> Result<(), StageFailure> {
    work.record(WorkDimension::CandidateWork, 1)?;
    let mut comparisons = 0;
    let located = operations.as_slice().binary_search_by(|existing| {
        comparisons += 1;
        existing.as_ref().unwrap().cmp(&operation)
    });
    work.record(WorkDimension::VisitedEntities, comparisons)?;
    let Err(position) = located else {
        return reject(DomainRejection::OperationIdCollision(operation));
    };
    for effect in effects.iter() {
        work.record(WorkDimension::VisitedEntities, 1)?;
        if effect.operation == operation {
            return reject(DomainRejection::OperationIdCollision(operation));
        }
    }
    if operations.len() + effects.len() >= OPERATIONS {
        return reject(DomainRejection::OperationCapacityExceeded);
    }
    positions[effects.len()] = position;
    effects
        .try_push(Effect {
            operation,
            generations,
            depends_on,
        })
        .map_err(|_| StageFailure::Invariant)
}
fn reject(reason: DomainRejection) -> Result<(), StageFailure> {
    Err(StageFailure::Rejected(reason))
}
fn adjusted_position(index: usize, positions: &Positions, effects: &Effects) -> usize {
    let prior_is_lower =
        index == 1 && effects.get(0).unwrap().operation < effects.get(1).unwrap().operation;
    positions[index] + usize::from(prior_is_lower)
}
fn copied_operation_bytes(before: usize, positions: &Positions, effects: &Effects) -> u64 {
    (0..effects.len())
        .map(|index| before + index - adjusted_position(index, positions, effects))
        .sum::<usize>() as u64
        * std::mem::size_of::<OperationId>() as u64
}
fn transition(
    sequence: EventSequence,
    outcome: CoreOutcome,
    effects: Effects,
    work: HotPathWorkWitness,
) -> CoreTransition {
    CoreTransition {
        sequence,
        outcome,
        effects,
        work,
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
