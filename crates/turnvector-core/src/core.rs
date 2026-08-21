use crate::model_descriptor::{
    MAX_FRAME_BYTES, ModelDescriptorError, ModelDescriptorHash, RawModelDescriptor, verify,
};
use crate::model_registry::{
    DescriptionPlan, MODEL_REGISTRY_LIMIT, ModelRegistry, ModelRevisionId, RegistrationIntent,
    RegistryError, RevisionSelection,
};
use crate::request_book::{
    AcceptanceInput, AcceptedRequest, EffectiveSamplingSeed, REQUEST_LIMIT, RequestBook,
    RequestBookGeneration, RequestError, RequestLifecycle, RequestSelector,
};
use crate::support::{
    OrdinarySupportSpec, SupportChangeInput, SupportChargeLedger, SupportLedgerError,
    SupportOperation,
};
use crate::work::WorkMeter;
use crate::{
    BoundedVec, DaemonInstanceId, EventSequence, FixedIndexError, FixedStorageError,
    GenerationVector, HotPathWorkBudget, HotPathWorkWitness, MonotonicTime, OperationId, RequestId,
    RequestStatusVersion, SupportOperationObligationId, WorkBudgetError, WorkDimension,
};
const TRANSITION_EFFECT_CAPACITY: usize = 2;
const MAX_OPERATION_ENTRIES: usize = 32_768;
#[cfg(not(test))]
const CORE_SUPPORT_RECORDS: usize = 32_768;
#[cfg(test)]
const CORE_SUPPORT_RECORDS: usize = 12;
#[cfg(not(test))]
const CORE_SUPPORT_CLAIMS: usize = 4_194_304;
#[cfg(test)]
const CORE_SUPPORT_CLAIMS: usize = 12;
const CORE_SUPPORT_HORIZONS: usize = 3;
const RAW_DESCRIPTOR_STORAGE: usize = MAX_FRAME_BYTES + 1;
type Effects = BoundedVec<Effect, TRANSITION_EFFECT_CAPACITY>;
type Operations<const N: usize> = BoundedVec<OperationId, N>;
type Positions = [usize; TRANSITION_EFFECT_CAPACITY];
type CoreSupport =
    SupportChargeLedger<CORE_SUPPORT_RECORDS, CORE_SUPPORT_CLAIMS, CORE_SUPPORT_HORIZONS>;
type CoreRegistry = ModelRegistry<MODEL_REGISTRY_LIMIT, MODEL_REGISTRY_LIMIT>;
type CoreRequests<const I: usize, const S: usize, const T: usize> =
    RequestBook<REQUEST_LIMIT, I, S, T>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct OwnedRawModelDescriptor {
    frame: [u8; RAW_DESCRIPTOR_STORAGE],
    frame_len: u16,
    id: [u8; 32],
    hash_schema_version: u32,
    hash: [u8; 32],
    vocabulary: u32,
}
impl OwnedRawModelDescriptor {
    #[rustfmt::skip]
    fn new(raw: RawModelDescriptor<'_>) -> Self { let length = raw.frame.len().min(RAW_DESCRIPTOR_STORAGE); let mut frame = [0; RAW_DESCRIPTOR_STORAGE]; frame[..length].copy_from_slice(&raw.frame[..length]); Self { frame, frame_len: u16::try_from(length).expect("bounded descriptor length fits u16"), id: raw.id, hash_schema_version: raw.hash_schema_version, hash: raw.hash, vocabulary: raw.vocabulary } }
    #[rustfmt::skip]
    fn as_raw(&self) -> RawModelDescriptor<'_> { RawModelDescriptor { frame: &self.frame[..usize::from(self.frame_len)], id: self.id, hash_schema_version: self.hash_schema_version, hash: self.hash, vocabulary: self.vocabulary } }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(
    clippy::large_enum_variant,
    reason = "bounded inline ownership preserves allocation-free Copy events"
)]
#[allow(
    dead_code,
    reason = "the runtime adapter constructs registration events after C10e"
)]
enum RegistrationEvent {
    Start(RegistrationIntent, OrdinarySupportSpec, MonotonicTime),
    Result(GenerationVector, OwnedRawModelDescriptor),
}
/// One validated operation request presented at an exact Event Sequence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CoreEvent<const I: usize = 0, const S: usize = 0, const T: usize = 0> {
    sequence: EventSequence,
    operation: Option<OperationId>,
    follow_up: Option<OperationId>,
    registration: Option<RegistrationEvent>,
    request: Option<AcceptanceInput<I, S, T>>,
}
impl<const I: usize, const S: usize, const T: usize> CoreEvent<I, S, T> {
    #[must_use]
    pub const fn operation(
        sequence: EventSequence,
        operation: OperationId,
        follow_up: Option<OperationId>,
    ) -> Self {
        Self {
            sequence,
            operation: Some(operation),
            follow_up,
            registration: None,
            request: None,
        }
    }
    #[allow(dead_code, reason = "the runtime adapter constructs registration events after C10e")]
    #[rustfmt::skip]
    pub(crate) const fn describe_model(sequence: EventSequence, operation: OperationId, intent: RegistrationIntent, support: OrdinarySupportSpec, at: MonotonicTime) -> Self { Self { sequence, operation: Some(operation), follow_up: None, registration: Some(RegistrationEvent::Start(intent, support, at)), request: None } }
    #[allow(dead_code, reason = "the runtime adapter constructs registration events after C10e")]
    #[rustfmt::skip]
    pub(crate) fn model_descriptor_result(sequence: EventSequence, operation: OperationId, generations: GenerationVector, result: RawModelDescriptor<'_>) -> Self { Self { sequence, operation: Some(operation), follow_up: None, registration: Some(RegistrationEvent::Result(generations, OwnedRawModelDescriptor::new(result))), request: None } }
    #[allow(dead_code, reason = "the runtime adapter constructs request events after C11c")]
    #[rustfmt::skip]
    pub(crate) const fn accept_request(sequence: EventSequence, input: AcceptanceInput<I, S, T>) -> Self { Self { sequence, operation: None, follow_up: None, registration: None, request: Some(input) } }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Effect {
    operation: OperationId,
    generations: GenerationVector,
    depends_on: Option<OperationId>,
    registration: Option<RegistrationIntent>,
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
    #[allow(
        dead_code,
        reason = "the runtime adapter reads registration effects after C10e"
    )]
    #[must_use]
    pub(crate) const fn registration(self) -> Option<RegistrationIntent> {
        self.registration
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DomainRejection {
    OperationIdCollision(OperationId),
    OperationCapacityExceeded,
    HotPathWorkBudget(WorkBudgetError),
    ModelRegistrationUnavailable,
    ModelRegistrationPending,
    ModelRegistrationResultMismatch,
    ModelRegistrationSupport,
    ModelRegistrationDescriptor,
    ModelRegistrationRegistry,
    RequestAcceptanceUnavailable,
    UnknownRequestRevision,
    UnknownRequestAlias,
    RequestRevisionUnavailable,
    RequestTopK,
    RequestContextLimit,
    RequestPreparationTimeout,
    RequestCapacityExceeded,
    RequestConnectionCapacityExceeded,
    RequestIdExhausted,
    RequestContinuity,
    RequestAcceptanceStale,
    RequestAcceptanceState,
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
    acceptance: Option<RequestAcceptance>,
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
    #[allow(
        dead_code,
        reason = "the runtime adapter reads request acceptance after C11c"
    )]
    pub(crate) const fn request_acceptance(&self) -> Option<RequestAcceptance> {
        self.acceptance
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RequestAcceptance {
    id: RequestId,
    revision: ModelRevisionId,
    seed: EffectiveSamplingSeed,
    status: RequestStatusVersion,
    lifecycle: RequestLifecycle,
}
#[allow(
    dead_code,
    reason = "the runtime adapter consumes request acceptance after C11c"
)]
impl RequestAcceptance {
    #[rustfmt::skip]
    fn from_accepted<const I: usize, const S: usize, const T: usize>(accepted: &AcceptedRequest<I, S, T>) -> Self { Self { id: accepted.id(), revision: accepted.revision_fact().revision(), seed: accepted.request().seed(), status: accepted.status(), lifecycle: accepted.lifecycle() } }
    #[rustfmt::skip]
    pub(crate) const fn id(self) -> RequestId { self.id }
    #[rustfmt::skip]
    pub(crate) const fn revision(self) -> ModelRevisionId { self.revision }
    #[rustfmt::skip]
    pub(crate) const fn seed(self) -> EffectiveSamplingSeed { self.seed }
    #[rustfmt::skip]
    pub(crate) const fn status(self) -> RequestStatusVersion { self.status }
    #[rustfmt::skip]
    pub(crate) const fn lifecycle(self) -> RequestLifecycle { self.lifecycle }
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
pub struct Core<
    const OPERATIONS: usize,
    const INPUT: usize = 0,
    const STOPS: usize = 0,
    const STOP_TOKENS: usize = 0,
> {
    state: CoreState<OPERATIONS>,
    fault: Option<CoreFault>,
    work_budget: HotPathWorkBudget,
    registration: Option<RegistrationState>,
    requests: Option<CoreRequests<INPUT, STOPS, STOP_TOKENS>>,
    #[cfg(test)]
    force_candidate_invariant_failure: bool,
    #[cfg(test)]
    request_generation_override: Option<RequestBookGeneration>,
}
#[rustfmt::skip]
#[derive(Debug, Eq, PartialEq)]
struct RegistrationState { support: CoreSupport, registry: CoreRegistry, pending: Option<PendingRegistration> }
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[rustfmt::skip]
struct PendingRegistration { operation: OperationId, generations: GenerationVector, obligation: SupportOperationObligationId, expected_hash: ModelDescriptorHash, plan: DescriptionPlan }
impl<const OPERATIONS: usize, const I: usize, const S: usize, const T: usize>
    Core<OPERATIONS, I, S, T>
{
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
            registration: None,
            requests: None,
            #[cfg(test)]
            force_candidate_invariant_failure: false,
            #[cfg(test)]
            request_generation_override: None,
        }
    }
    #[allow(dead_code, reason = "the runtime bootstrap adapter lands after the Core foundation")]
    #[rustfmt::skip]
    pub(crate) fn bootstrap_with_registration(first_sequence: EventSequence, generations: GenerationVector, support: CoreSupport, registry: CoreRegistry) -> Self { let mut core = Self::bootstrap(first_sequence, generations); core.registration = Some(RegistrationState { support, registry, pending: None }); core }
    #[allow(dead_code, reason = "the runtime bootstrap adapter lands after the Core foundation")]
    #[rustfmt::skip]
    pub(crate) fn bootstrap_with_request_acceptance(first_sequence: EventSequence, generations: GenerationVector, support: CoreSupport, registry: CoreRegistry, daemon: DaemonInstanceId, request_generation: RequestBookGeneration) -> Result<Self, RequestError> { let mut core = Self::bootstrap_with_registration(first_sequence, generations, support, registry); core.requests = Some(CoreRequests::try_new(daemon, request_generation)?); Ok(core) }
    #[must_use]
    pub const fn state(&self) -> &CoreState<OPERATIONS> {
        &self.state
    }
    #[must_use]
    pub const fn fault(&self) -> Option<CoreFault> {
        self.fault
    }
    pub fn handle(&mut self, event: CoreEvent<I, S, T>) -> CoreTransition {
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
        let applied = match event.request {
            Some(input) => self
                .accept_request(input, &mut work)
                .map(|accepted| (BoundedVec::new(), Some(accepted))),
            None => match event.registration {
                None => self
                    .stage(
                        event.operation.expect("operation event"),
                        event.follow_up,
                        None,
                        &mut work,
                    )
                    .map(|(positions, effects)| {
                        self.commit(&positions, &effects);
                        (effects, None)
                    }),
                Some(RegistrationEvent::Start(intent, support, at)) => self
                    .start_registration(
                        event.operation.expect("registration event"),
                        intent,
                        support,
                        at,
                        &mut work,
                    )
                    .map(|effects| (effects, None)),
                Some(RegistrationEvent::Result(generations, result)) => self
                    .finish_registration(
                        event.operation.expect("registration event"),
                        generations,
                        result,
                        &mut work,
                    )
                    .map(|effects| (effects, None)),
            },
        };
        match applied {
            Ok((effects, acceptance)) => {
                self.state.expected_sequence = next_sequence;
                let mut transition = transition(
                    event.sequence,
                    CoreOutcome::Accepted,
                    effects,
                    work.witness(),
                );
                transition.acceptance = acceptance;
                transition
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
        operation: OperationId,
        follow_up: Option<OperationId>,
        registration: Option<RegistrationIntent>,
        work: &mut WorkMeter,
    ) -> Result<(Positions, Effects), StageFailure> {
        let mut positions = [0; TRANSITION_EFFECT_CAPACITY];
        let mut effects = BoundedVec::new();
        stage_operation(
            &self.state.operations,
            &mut positions,
            &mut effects,
            operation,
            None,
            registration,
            self.state.generations,
            work,
        )?;
        if let Some(follow_up) = follow_up {
            stage_operation(
                &self.state.operations,
                &mut positions,
                &mut effects,
                follow_up,
                Some(operation),
                None,
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
    #[rustfmt::skip]
    fn start_registration(&mut self, operation: OperationId, intent: RegistrationIntent, support: OrdinarySupportSpec, at: MonotonicTime, work: &mut WorkMeter) -> Result<Effects, StageFailure> {
        let state = self.registration.as_ref().ok_or_else(|| registration_rejection(DomainRejection::ModelRegistrationUnavailable))?;
        if state.pending.is_some() { return Err(registration_rejection(DomainRejection::ModelRegistrationPending)); }
        if support.operation != SupportOperation::DescribeModel { return Err(registration_rejection(DomainRejection::ModelRegistrationSupport)); }
        let (positions, effects) = self.stage(operation, None, Some(intent), work)?;
        let plan = state.registry.prepare_description(state.registry.generation(), intent, work).map_err(registry_failure)?;
        let change = state.support.prepare(state.support.generation(), SupportChangeInput::BeginOrdinary(support, at), work).map_err(support_failure)?;
        state.support.validate(&change).map_err(support_failure)?;
        let state = self.registration.as_mut().expect("registration state was checked");
        state.support.commit(change, work).map_err(support_failure)?;
        state.pending = Some(PendingRegistration { operation, generations: self.state.generations, obligation: support.id, expected_hash: intent.expected_descriptor_hash, plan });
        self.commit(&positions, &effects); Ok(effects)
    }
    #[rustfmt::skip]
    fn accept_request(&mut self, input: AcceptanceInput<I, S, T>, work: &mut WorkMeter) -> Result<RequestAcceptance, StageFailure> {
        let registration = self.registration.as_ref().ok_or_else(|| registration_rejection(DomainRejection::RequestAcceptanceUnavailable))?;
        let requests = self.requests.as_ref().ok_or_else(|| registration_rejection(DomainRejection::RequestAcceptanceUnavailable))?;
        let selection = match input.request.selector() { RequestSelector::Direct(revision) => RevisionSelection::Direct(revision), RequestSelector::Alias(alias) => RevisionSelection::Alias(alias) };
        let registry_generation = registration.registry.generation();
        let revision = registration.registry.request_revision_fact(registry_generation, selection, work).map_err(request_registry_failure)?.ok_or_else(|| registration_rejection(match selection { RevisionSelection::Direct(_) => DomainRejection::UnknownRequestRevision, RevisionSelection::Alias(_) => DomainRejection::UnknownRequestAlias }))?;
        let request_generation = requests.generation();
        #[cfg(test)]
        let request_generation = self.request_generation_override.unwrap_or(request_generation);
        let change = requests.prepare(request_generation, registry_generation, input, revision, work).map_err(request_failure)?;
        registration.registry.validate_request_revision(revision).map_err(request_registry_failure)?;
        requests.validate(&change).map_err(request_failure)?;
        let accepted = RequestAcceptance::from_accepted(change.accepted());
        self.requests.as_mut().expect("request state was checked").commit(change).expect("revalidated request commit is infallible");
        Ok(accepted)
    }
    #[rustfmt::skip]
    fn finish_registration(&mut self, operation: OperationId, generations: GenerationVector, result: OwnedRawModelDescriptor, work: &mut WorkMeter) -> Result<Effects, StageFailure> {
        let state = self.registration.as_ref().ok_or_else(|| registration_rejection(DomainRejection::ModelRegistrationUnavailable))?;
        let pending = state.pending.ok_or_else(|| registration_rejection(DomainRejection::ModelRegistrationResultMismatch))?;
        if (operation, generations, self.state.generations) != (pending.operation, pending.generations, pending.generations) { return Err(registration_rejection(DomainRejection::ModelRegistrationResultMismatch)); }
        let descriptor = verify(result.as_raw(), pending.expected_hash, work).map_err(descriptor_failure)?;
        let registry_change = state.registry.prepare_registration(pending.plan, &descriptor, work).map_err(registry_failure)?;
        let support_change = state.support.prepare(state.support.generation(), SupportChangeInput::FinishActive(pending.obligation), work).map_err(support_failure)?;
        state.registry.validate(&registry_change).map_err(registry_failure)?;
        state.support.validate(&support_change).map_err(support_failure)?;
        let state = self.registration.as_mut().expect("registration state was checked");
        state.support.commit(support_change, work).expect("revalidated support finish is infallible");
        state.registry.commit(registry_change).expect("revalidated registry insertion is infallible");
        state.pending = None; Ok(BoundedVec::new())
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
#[allow(
    clippy::too_many_arguments,
    reason = "the helper stages the complete Effect contract without allocation"
)]
fn stage_operation<const OPERATIONS: usize>(
    operations: &Operations<OPERATIONS>,
    positions: &mut Positions,
    effects: &mut Effects,
    operation: OperationId,
    depends_on: Option<OperationId>,
    registration: Option<RegistrationIntent>,
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
            registration,
        })
        .map_err(|_| StageFailure::Invariant)
}
#[rustfmt::skip]
fn registration_rejection(reason: DomainRejection) -> StageFailure { StageFailure::Rejected(reason) }
#[rustfmt::skip]
fn support_failure(error: SupportLedgerError) -> StageFailure { match error { SupportLedgerError::Storage(FixedStorageError::Work(error)) => error.into(), _ => registration_rejection(DomainRejection::ModelRegistrationSupport) } }
#[rustfmt::skip]
fn descriptor_failure(error: ModelDescriptorError) -> StageFailure { match error { ModelDescriptorError::Work(error) => error.into(), _ => registration_rejection(DomainRejection::ModelRegistrationDescriptor) } }
#[rustfmt::skip]
fn registry_failure(error: RegistryError) -> StageFailure { match error { RegistryError::Work(error) | RegistryError::Index(FixedIndexError::Work(error)) => error.into(), _ => registration_rejection(DomainRejection::ModelRegistrationRegistry) } }
#[rustfmt::skip]
fn request_registry_failure(error: RegistryError) -> StageFailure { match error { RegistryError::Work(error) | RegistryError::Index(FixedIndexError::Work(error)) => error.into(), RegistryError::Generation | RegistryError::PreparedChangeStale => registration_rejection(DomainRejection::RequestAcceptanceStale), _ => registration_rejection(DomainRejection::RequestAcceptanceState) } }
#[rustfmt::skip]
fn request_failure(error: RequestError) -> StageFailure { match error { RequestError::Work(error) => error.into(), RequestError::RevisionUnavailable => registration_rejection(DomainRejection::RequestRevisionUnavailable), RequestError::TopK => registration_rejection(DomainRejection::RequestTopK), RequestError::ContextLimit => registration_rejection(DomainRejection::RequestContextLimit), RequestError::PreparationTimeout => registration_rejection(DomainRejection::RequestPreparationTimeout), RequestError::RequestCapacity => registration_rejection(DomainRejection::RequestCapacityExceeded), RequestError::ConnectionCapacity => registration_rejection(DomainRejection::RequestConnectionCapacityExceeded), RequestError::RequestIdExhausted => registration_rejection(DomainRejection::RequestIdExhausted), RequestError::Continuity => registration_rejection(DomainRejection::RequestContinuity), RequestError::RegistryGeneration | RequestError::PreparedChangeStale => registration_rejection(DomainRejection::RequestAcceptanceStale), _ => registration_rejection(DomainRejection::RequestAcceptanceState) } }
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
        acceptance: None,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::model_registry::{
        ModelAliasId, ModelManifestId, ModelRevisionId, RegistryCommand, RegistryGeneration,
    };
    use crate::request_book::{
        AcceptanceInput, EffectiveSamplingSeed, GenerationParameters, RequestBookGeneration,
        RequestLifecycle, RequestSelector, SamplingMode, SamplingSeedOrigin, TokenRequest,
    };
    use crate::support::{LifecycleReserveMaxima, SupportCallScopeId, SupportFundingClaim};
    use crate::{
        BackendGeneration, ConnectionId, DaemonInstanceId, Duration, FixedStartCountBound, ModelId,
        MonotonicTime, PhysicalStartCreditId, RequestSequence, RuntimeOverheadGeneration,
        SafetyGeneration, SchedulerGeneration, ServiceClass, SupportLedgerGeneration,
        SupportOperationObligationId, TokenCount,
    };
    const FRAME: [u8; 13] = [0, 0, 0, 1, 0, 0, 0, 7, 0, 0, 0, 1, b'x'];
    #[rustfmt::skip]
    const ID: [u8; 32] = [0xc9, 0x1c, 0x14, 0x09, 0x1c, 0xea, 0x08, 0xf4, 0x58, 0xa4, 0xe2, 0x75, 0x96, 0xc1, 0x5b, 0x2c, 0xf0, 0xc8, 0x74, 0x34, 0x2d, 0x30, 0x3e, 0xad, 0xe8, 0x9f, 0x29, 0x0e, 0xd0, 0x13, 0x38, 0x21];
    #[rustfmt::skip]
    const HASH: [u8; 32] = [0xe2, 0x24, 0x6d, 0x47, 0x7f, 0x70, 0xd3, 0xe6, 0x58, 0x8b, 0xb5, 0x45, 0xe2, 0x14, 0xc0, 0xbb, 0xa1, 0x76, 0x6e, 0xf3, 0x39, 0x7a, 0x50, 0x71, 0x89, 0x29, 0xc9, 0x4f, 0xe9, 0x62, 0x1e, 0x9b];
    #[rustfmt::skip]
    fn generations_with(scheduler: u64, backend: u64, safety: u64, overhead: u64) -> GenerationVector { GenerationVector::new(SchedulerGeneration::new(scheduler).unwrap(), BackendGeneration::new(backend).unwrap(), SafetyGeneration::new(safety).unwrap(), RuntimeOverheadGeneration::new(overhead).unwrap()) }
    fn generations() -> GenerationVector {
        generations_with(1, 1, 1, 1)
    }
    #[rustfmt::skip]
    fn registration(hash: [u8; 32]) -> RegistrationIntent { RegistrationIntent { model: ModelId::new(1).unwrap(), revision: ModelRevisionId::new([2; 32]).unwrap(), manifest: ModelManifestId::new([3; 32]).unwrap(), expected_descriptor_hash: ModelDescriptorHash::from_manifest(1, hash).unwrap(), context_limit: TokenCount::new(8) } }
    #[rustfmt::skip]
    fn ordinary(n: u8) -> OrdinarySupportSpec { OrdinarySupportSpec { id: SupportOperationObligationId([n; 32]), operation: SupportOperation::DescribeModel, physical_credit: PhysicalStartCreditId([n + 1; 32]), scope: SupportCallScopeId([n + 2; 32]), claim: SupportFundingClaim::OrdinaryReservation([n + 3; 32]) } }
    #[rustfmt::skip]
    fn raw<'a>(frame: &'a [u8], id: [u8; 32], hash: [u8; 32], vocabulary: u32) -> RawModelDescriptor<'a> { RawModelDescriptor { frame, id, hash_schema_version: 1, hash, vocabulary } }
    #[rustfmt::skip]
    fn start<const I: usize, const S: usize, const T: usize>(core: &mut Core<4, I, S, T>, sequence: u64, operation: u128, intent: RegistrationIntent, support: u8) -> CoreTransition { core.handle(CoreEvent::describe_model(EventSequence::new(sequence).unwrap(), OperationId::new(operation).unwrap(), intent, ordinary(support), MonotonicTime::from_micros(sequence))) }
    #[rustfmt::skip]
    fn result<const I: usize, const S: usize, const T: usize>(core: &mut Core<4, I, S, T>, sequence: u64, operation: u128, descriptor: RawModelDescriptor<'_>) -> CoreTransition { core.handle(CoreEvent::model_descriptor_result(EventSequence::new(sequence).unwrap(), OperationId::new(operation).unwrap(), generations(), descriptor)) }
    #[rustfmt::skip]
    fn registration_core(capacity: u32) -> Core<4> {
        let mut capacities = [[0; 3]; 5]; for class in [2, 3, 4] { capacities[class][0] = capacity; }
        let starts = std::array::from_fn(|_| [FixedStartCountBound(Duration::from_micros(10), 2), FixedStartCountBound(Duration::from_micros(20), 2), FixedStartCountBound(Duration::from_micros(30), 2)]);
        let support = CoreSupport::try_new(SupportLedgerGeneration::new(1).unwrap(), capacities, 1, starts, LifecycleReserveMaxima([1; 5])).unwrap();
        let registry = CoreRegistry::try_new(RegistryGeneration::new(1).unwrap()).unwrap();
        Core::bootstrap_with_registration(EventSequence::new(1).unwrap(), generations(), support, registry)
    }
    #[rustfmt::skip]
    fn request_core() -> Core<4, 2, 1, 2> {
        let mut capacities = [[0; 3]; 5]; for class in [2, 3, 4] { capacities[class][0] = 2; }
        let starts = std::array::from_fn(|_| [FixedStartCountBound(Duration::from_micros(10), 2), FixedStartCountBound(Duration::from_micros(20), 2), FixedStartCountBound(Duration::from_micros(30), 2)]);
        let support = CoreSupport::try_new(SupportLedgerGeneration::new(1).unwrap(), capacities, 1, starts, LifecycleReserveMaxima([1; 5])).unwrap();
        let registry = CoreRegistry::try_new(RegistryGeneration::new(1).unwrap()).unwrap();
        Core::bootstrap_with_request_acceptance(EventSequence::new(1).unwrap(), generations(), support, registry, DaemonInstanceId::new(1).unwrap(), RequestBookGeneration::new(1).unwrap()).unwrap()
    }
    #[rustfmt::skip]
    fn request_input(selector: RequestSelector, connection: u128, input: &[u32], output: u64, top_k: u32, seed: u64, timeout: u64) -> AcceptanceInput<2, 1, 2> { let parameters = if top_k == 0 { GenerationParameters::try_new(SamplingMode::Greedy, 0.0f32.to_bits(), 1.0f32.to_bits(), 0) } else { GenerationParameters::try_new(SamplingMode::Categorical, 1.0f32.to_bits(), 1.0f32.to_bits(), top_k) }.unwrap(); AcceptanceInput { connection: ConnectionId::new(connection).unwrap(), request: TokenRequest::try_new(selector, input, parameters, ServiceClass::Interactive, TokenCount::new(output), &[&[3]], EffectiveSamplingSeed::new(seed, SamplingSeedOrigin::Caller)).unwrap(), accepted_at: MonotonicTime::from_micros(3), preparation_timeout: Duration::from_micros(timeout) } }
    #[rustfmt::skip]
    fn registered_request_core() -> Core<4, 2, 1, 2> { let mut core = request_core(); assert_eq!(start(&mut core, 1, 1, registration(HASH), 5).outcome(), &CoreOutcome::Accepted); assert_eq!(result(&mut core, 2, 1, raw(&FRAME, ID, HASH, 7)).outcome(), &CoreOutcome::Accepted); core }
    #[test]
    fn candidate_invariant_failure_preserves_state_and_latches_fault() {
        let sequence = EventSequence::new(1).unwrap();
        let generations = generations();
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

    #[test]
    #[rustfmt::skip]
    fn describe_model_starts_only_after_support_charge() {
        fn public_contract<T: std::fmt::Debug + Eq + PartialEq>() {} public_contract::<Core<4>>(); let _: Option<CoreEvent> = None;
        let registration = registration(HASH); let mut core = registration_core(2); let transition = core.handle(CoreEvent::describe_model(EventSequence::new(1).unwrap(), OperationId::new(1).unwrap(), registration, ordinary(5), MonotonicTime::from_micros(1)));
        assert_eq!(transition.outcome(), &CoreOutcome::Accepted);
        assert_eq!((transition.effects().len(), transition.effects().get(0).unwrap().registration()), (1, Some(registration)));
        assert_eq!(transition.work(), HotPathWorkWitness::new([3, 448, 0, 1, 31]));
        let result = core.handle(CoreEvent::model_descriptor_result(EventSequence::new(2).unwrap(), OperationId::new(1).unwrap(), generations(), raw(&FRAME, ID, HASH, 7)));
        assert_eq!((result.outcome(), result.effects().is_empty()), (&CoreOutcome::Accepted, true));
        assert_eq!(result.work(), HotPathWorkWitness::new([833, 823, 0, 2, 29]));
        let state = core.registration.as_ref().unwrap();
        assert_eq!((state.pending.is_none(), state.registry.counts().registered, state.support.generation()), (true, 1, SupportLedgerGeneration::new(3).unwrap()));
        assert_eq!(state.registry.request_revision_fact(state.registry.generation(), crate::model_registry::RevisionSelection::Direct(ModelRevisionId::new([2; 32]).unwrap()), &mut WorkMeter::new(HotPathWorkBudget::binary_maximum())).unwrap().unwrap().context_limit(), TokenCount::new(8));
        let mut work = WorkMeter::new(HotPathWorkBudget::binary_maximum());
        let descriptor = state.registry.descriptor(ModelRevisionId::new([2; 32]).unwrap(), &mut work).unwrap().unwrap();
        let sealed = verify(raw(&FRAME, ID, HASH, 7), ModelDescriptorHash::from_manifest(1, HASH).unwrap(), &mut work).unwrap();
        assert!(descriptor.exactly_matches(&sealed, &mut work).unwrap());
    }
    #[test]
    #[rustfmt::skip]
    fn accepts_a_request_into_preparing_only_after_commit() {
        let revision = ModelRevisionId::new([2; 32]).unwrap(); let mut core = registered_request_core();
        let input = request_input(RequestSelector::Direct(revision), 2, &[1], 1, 0, 9, 5);
        let transition = core.handle(CoreEvent::accept_request(EventSequence::new(3).unwrap(), input));
        let accepted = transition.request_acceptance().unwrap();
        assert_eq!((transition.outcome(), transition.effects().len()), (&CoreOutcome::Accepted, 0));
        assert_eq!(transition.work(), HotPathWorkWitness::new([3, 720, 0, 0, 14]));
        assert_eq!((accepted.id().sequence().get(), accepted.revision(), accepted.seed().value(), accepted.seed().origin(), accepted.status().get(), accepted.lifecycle()), (1, revision, 9, SamplingSeedOrigin::Caller, 1, RequestLifecycle::Preparing));
        let alias = ModelAliasId::new([4; 32]).unwrap(); { let registry = &mut core.registration.as_mut().unwrap().registry; let mut work = WorkMeter::new(HotPathWorkBudget::binary_maximum()); let change = registry.prepare(registry.generation(), RegistryCommand::BindAlias(alias, revision), &mut work).unwrap(); registry.commit(change).unwrap(); }
        let next = core.handle(CoreEvent::accept_request(EventSequence::new(4).unwrap(), request_input(RequestSelector::Alias(alias), 2, &[1], 1, 0, 10, 5))); let accepted = next.request_acceptance().unwrap();
        assert_eq!(next.work(), HotPathWorkWitness::new([5, 720, 0, 0, 16]));
        assert_eq!((accepted.id().sequence().get(), accepted.revision(), accepted.seed().value(), core.requests.as_ref().unwrap().len()), (2, revision, 10, 2));
    }
    #[test]
    #[rustfmt::skip]
    fn request_acceptance_rejections_assign_no_id_or_success() {
        let revision = ModelRevisionId::new([2; 32]).unwrap(); let alias = ModelAliasId::new([4; 32]).unwrap();
        for (selector, rejection) in [(RequestSelector::Direct(revision), DomainRejection::UnknownRequestRevision), (RequestSelector::Alias(alias), DomainRejection::UnknownRequestAlias)] { let mut core = request_core(); let transition = core.handle(CoreEvent::accept_request(EventSequence::new(1).unwrap(), request_input(selector, 2, &[1], 1, 0, 9, 5))); assert_eq!(transition.outcome(), &CoreOutcome::Rejected(rejection)); assert!(transition.request_acceptance().is_none()); assert_eq!(core.requests.as_ref().unwrap().len(), 0); }
        for (input, rejection) in [(request_input(RequestSelector::Direct(revision), 2, &[1], 1, 7, 9, 5), DomainRejection::RequestTopK), (request_input(RequestSelector::Direct(revision), 2, &[1], 8, 0, 9, 5), DomainRejection::RequestContextLimit), (request_input(RequestSelector::Direct(revision), 2, &[1], 1, 0, 9, 0), DomainRejection::RequestPreparationTimeout)] { let mut core = registered_request_core(); let before = (core.requests.as_ref().unwrap().generation(), core.requests.as_ref().unwrap().len()); let rejected = core.handle(CoreEvent::accept_request(EventSequence::new(3).unwrap(), input)); assert_eq!((rejected.outcome(), rejected.request_acceptance()), (&CoreOutcome::Rejected(rejection), None)); assert_eq!((core.requests.as_ref().unwrap().generation(), core.requests.as_ref().unwrap().len()), before); let accepted = core.handle(CoreEvent::accept_request(EventSequence::new(4).unwrap(), request_input(RequestSelector::Direct(revision), 2, &[1], 1, 0, 9, 5))).request_acceptance().unwrap(); assert_eq!(accepted.id().sequence().get(), 1); }
        let mut unavailable = registered_request_core(); { let registry = &mut unavailable.registration.as_mut().unwrap().registry; let mut work = WorkMeter::new(HotPathWorkBudget::binary_maximum()); let change = registry.prepare(registry.generation(), RegistryCommand::BindAlias(alias, revision), &mut work).unwrap(); registry.commit(change).unwrap(); let change = registry.prepare(registry.generation(), RegistryCommand::MarkUnavailable(revision), &mut work).unwrap(); registry.commit(change).unwrap(); } for (offset, selector) in [RequestSelector::Direct(revision), RequestSelector::Alias(alias)].into_iter().enumerate() { let rejected = unavailable.handle(CoreEvent::accept_request(EventSequence::new(3 + offset as u64).unwrap(), request_input(selector, 2, &[1], 1, 0, 9, 5))); assert_eq!((rejected.outcome(), rejected.request_acceptance(), unavailable.requests.as_ref().unwrap().len()), (&CoreOutcome::Rejected(DomainRejection::RequestRevisionUnavailable), None, 0)); }
        let input = request_input(RequestSelector::Direct(revision), 2, &[1], 1, 0, 9, 5); let mut constrained = registered_request_core(); constrained.work_budget = HotPathWorkBudget::try_new(HotPathWorkWitness::new([1_000_000, 0, 0, 2, 2_100])).unwrap(); let rejected = constrained.handle(CoreEvent::accept_request(EventSequence::new(3).unwrap(), input)); assert_eq!(rejected.outcome(), &CoreOutcome::Rejected(DomainRejection::HotPathWorkBudget(WorkBudgetError::BudgetExceeded(WorkDimension::CopiedBytes, 0, 224)))); assert_eq!((rejected.work(), rejected.request_acceptance(), constrained.requests.as_ref().unwrap().len()), (HotPathWorkWitness::new([2, 0, 0, 0, 1]), None, 0)); constrained.work_budget = HotPathWorkBudget::binary_maximum(); let retried = constrained.handle(CoreEvent::accept_request(EventSequence::new(4).unwrap(), input)); assert_eq!((retried.request_acceptance().unwrap().id().sequence().get(), retried.work(), constrained.requests.as_ref().unwrap().len()), (1, HotPathWorkWitness::new([3, 720, 0, 0, 14]), 1));
    }
    #[test]
    #[rustfmt::skip]
    fn request_capacity_and_internal_rejections_are_closed() {
        let revision = ModelRevisionId::new([2; 32]).unwrap(); let mut core = registered_request_core(); for offset in 0..REQUEST_LIMIT { let transition = core.handle(CoreEvent::accept_request(EventSequence::new(3 + offset as u64).unwrap(), request_input(RequestSelector::Direct(revision), 2, &[1], 1, 0, 9, 5))); assert_eq!(transition.request_acceptance().unwrap().id().sequence().get(), offset as u64 + 1); } let rejected = core.handle(CoreEvent::accept_request(EventSequence::new(3 + REQUEST_LIMIT as u64).unwrap(), request_input(RequestSelector::Direct(revision), 2, &[1], 1, 0, 9, 5))); assert_eq!((rejected.outcome(), rejected.request_acceptance(), core.requests.as_ref().unwrap().len()), (&CoreOutcome::Rejected(DomainRejection::RequestCapacityExceeded), None, REQUEST_LIMIT));
        let mut connections = registered_request_core(); for offset in 0..64 { let accepted = connections.handle(CoreEvent::accept_request(EventSequence::new(3 + offset).unwrap(), request_input(RequestSelector::Direct(revision), u128::from(offset + 1), &[1], 1, 0, 9, 5))); assert!(accepted.request_acceptance().is_some()); } let before = connections.requests.clone(); let rejected = connections.handle(CoreEvent::accept_request(EventSequence::new(67).unwrap(), request_input(RequestSelector::Direct(revision), 65, &[1], 1, 0, 9, 5))); assert_eq!((rejected.outcome(), rejected.request_acceptance(), rejected.work(), &connections.requests), (&CoreOutcome::Rejected(DomainRejection::RequestConnectionCapacityExceeded), None, HotPathWorkWitness::new([66, 224, 0, 0, 12]), &before));
        for (last, rejection, witness) in [(u64::MAX, DomainRejection::RequestIdExhausted, [3, 224, 0, 0, 13]), (1, DomainRejection::RequestContinuity, [3, 224, 0, 0, 13])] { let mut core = registered_request_core(); core.requests.as_mut().unwrap().force_cursor(ConnectionId::new(2).unwrap(), RequestSequence::new(last).unwrap()); let before = core.requests.clone(); let rejected = core.handle(CoreEvent::accept_request(EventSequence::new(3).unwrap(), request_input(RequestSelector::Direct(revision), 2, &[1], 1, 0, 9, 5))); assert_eq!((rejected.outcome(), rejected.request_acceptance(), rejected.work(), &core.requests), (&CoreOutcome::Rejected(rejection), None, HotPathWorkWitness::new(witness), &before)); }
        let mut stale = registered_request_core(); stale.request_generation_override = Some(RequestBookGeneration::new(2).unwrap()); let before = stale.requests.clone(); let rejected = stale.handle(CoreEvent::accept_request(EventSequence::new(3).unwrap(), request_input(RequestSelector::Direct(revision), 2, &[1], 1, 0, 9, 5))); assert_eq!((rejected.outcome(), rejected.request_acceptance(), rejected.work(), &stale.requests), (&CoreOutcome::Rejected(DomainRejection::RequestAcceptanceStale), None, HotPathWorkWitness::new([2, 224, 0, 0, 2]), &before));
    }
    #[test]
    #[rustfmt::skip]
    fn registration_rejections_commit_neither_owner_and_emit_nothing() {
        let snapshot = |core: &Core<4>| { let state = core.registration.as_ref().unwrap(); (state.support.generation(), state.registry.counts(), state.pending.map(|pending| pending.operation), core.state.generations, core.state.operation_count()) };
        let assert_rejected = |transition: CoreTransition, rejection, witness| { assert_eq!(transition.outcome(), &CoreOutcome::Rejected(rejection)); assert!(transition.effects().is_empty()); assert_eq!(transition.work(), HotPathWorkWitness::new(witness)); };
        let mut unavailable = Core::<4>::bootstrap(EventSequence::new(1).unwrap(), generations()); assert_rejected(start(&mut unavailable, 1, 1, registration(HASH), 5), DomainRejection::ModelRegistrationUnavailable, [1, 0, 0, 0, 0]); assert_eq!(unavailable.state.operation_count(), 0);
        let mut exhausted = registration_core(0); let before = snapshot(&exhausted); assert_rejected(start(&mut exhausted, 1, 1, registration(HASH), 5), DomainRejection::ModelRegistrationSupport, [1, 160, 0, 1, 16]); assert_eq!(snapshot(&exhausted), before);
        let mut wrong = registration_core(2); let before = snapshot(&wrong); let mut support = ordinary(5); support.operation = SupportOperation::DescribeRequest; assert_rejected(wrong.handle(CoreEvent::describe_model(EventSequence::new(1).unwrap(), OperationId::new(1).unwrap(), registration(HASH), support, MonotonicTime::from_micros(1))), DomainRejection::ModelRegistrationSupport, [1, 0, 0, 0, 0]); assert_eq!(snapshot(&wrong), before);
        let mut core = registration_core(2); assert_eq!(start(&mut core, 1, 1, registration(HASH), 5).outcome(), &CoreOutcome::Accepted); let pending = snapshot(&core);
        assert_rejected(start(&mut core, 2, 2, registration(HASH), 9), DomainRejection::ModelRegistrationPending, [1, 0, 0, 0, 0]); assert_eq!(snapshot(&core), pending);
        assert_rejected(result(&mut core, 3, 2, raw(&FRAME, ID, HASH, 7)), DomainRejection::ModelRegistrationResultMismatch, [1, 0, 0, 0, 0]); assert_eq!(snapshot(&core), pending);
        for current in [generations_with(2, 1, 1, 1), generations_with(1, 2, 1, 1), generations_with(1, 1, 2, 1), generations_with(1, 1, 1, 2)] { let mut shifted = registration_core(2); assert_eq!(start(&mut shifted, 1, 1, registration(HASH), 5).outcome(), &CoreOutcome::Accepted); shifted.state.generations = current; let before = snapshot(&shifted); assert_rejected(result(&mut shifted, 2, 1, raw(&FRAME, ID, HASH, 7)), DomainRejection::ModelRegistrationResultMismatch, [1, 0, 0, 0, 0]); assert_eq!(snapshot(&shifted), before); }
        let oversize = [0; MAX_FRAME_BYTES + 2];
        for (sequence, descriptor, witness) in [(4, raw(&FRAME[..12], ID, HASH, 7), [2, 0, 0, 0, 6]), (5, raw(&FRAME, [0; 32], HASH, 7), [4, 108, 0, 2, 10]), (6, raw(&FRAME, ID, HASH, 8), [4, 108, 0, 2, 10]), (7, raw(&FRAME, ID, [0; 32], 7), [4, 108, 0, 2, 10]), (8, raw(&oversize, ID, HASH, 7), [2, 0, 0, 0, 1])] { assert_rejected(result(&mut core, sequence, 1, descriptor), DomainRejection::ModelRegistrationDescriptor, witness); assert_eq!(snapshot(&core), pending); }
        assert_eq!(result(&mut core, 9, 1, raw(&FRAME, ID, HASH, 7)).outcome(), &CoreOutcome::Accepted); let committed = snapshot(&core); assert_rejected(start(&mut core, 10, 2, registration(HASH), 9), DomainRejection::ModelRegistrationRegistry, [3, 0, 0, 1, 9]); assert_eq!(snapshot(&core), committed);
        let mut manifest = registration_core(2); assert_eq!(start(&mut manifest, 1, 1, registration([0; 32]), 5).outcome(), &CoreOutcome::Accepted); let before = snapshot(&manifest); assert_rejected(result(&mut manifest, 2, 1, raw(&FRAME, ID, HASH, 7)), DomainRejection::ModelRegistrationDescriptor, [4, 108, 0, 2, 10]); assert_eq!(snapshot(&manifest), before);
        let mut stale = registration_core(2); assert_eq!(start(&mut stale, 1, 1, registration(HASH), 5).outcome(), &CoreOutcome::Accepted); { let state = stale.registration.as_mut().unwrap(); let mut work = WorkMeter::new(HotPathWorkBudget::binary_maximum()); let sealed = verify(raw(&FRAME, ID, HASH, 7), ModelDescriptorHash::from_manifest(1, HASH).unwrap(), &mut work).unwrap(); let mut other = registration(HASH); other.revision = ModelRevisionId::new([9; 32]).unwrap(); let plan = state.registry.prepare_description(state.registry.generation(), other, &mut work).unwrap(); let change = state.registry.prepare_registration(plan, &sealed, &mut work).unwrap(); state.registry.commit(change).unwrap(); } let before = snapshot(&stale); assert_rejected(result(&mut stale, 2, 1, raw(&FRAME, ID, HASH, 7)), DomainRejection::ModelRegistrationRegistry, [4, 121, 0, 2, 16]); assert_eq!(snapshot(&stale), before);
    }
}
