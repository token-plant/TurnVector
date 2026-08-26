use crate::model_descriptor::{
    MAX_FRAME_BYTES, ModelDescriptorError, ModelDescriptorHash, ModelDescriptorId,
    RawModelDescriptor, verify,
};
use crate::model_registry::{
    DescriptionPlan, MODEL_REGISTRY_LIMIT, ModelRegistry, ModelRevisionId, RegisteredDescriptor,
    RegistrationIntent, RegistryError, RevisionSelection,
};
use crate::request_book::{
    AcceptanceInput, AcceptedRequest, DescriptionRefreshScope, EffectiveSamplingSeed,
    REQUEST_LIMIT, RequestBook, RequestBookGeneration, RequestDescriptionFacts, RequestError,
    RequestLifecycle, RequestSelector, TokenRequest,
};
use crate::support::{
    LifecycleReserveKind, LifecycleTriggerResult, OrdinarySupportSpec, SupportCausalPredecessorId,
    SupportChangeInput, SupportChargeLedger, SupportLedgerError, SupportOperation,
};
use crate::work::WorkMeter;
use crate::{
    BackendGeneration, BoundedVec, DaemonInstanceId, EventSequence, FixedIndexError,
    FixedStorageError, GenerationVector, HotPathWorkBudget, HotPathWorkWitness, MonotonicTime,
    OperationId, RequestId, RequestStatusVersion, SupportOperationObligationId, WorkBudgetError,
    WorkDimension,
};
const TRANSITION_EFFECT_CAPACITY: usize = 2;
const MAX_OPERATION_ENTRIES: usize = 32_768;
#[cfg(not(test))]
const CORE_SUPPORT_RECORDS: usize = 32_768;
#[cfg(test)]
const CORE_SUPPORT_RECORDS: usize = 20;
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
type DescriptionObligations = BoundedVec<SupportOperationObligationId, { REQUEST_LIMIT + 1 }>;

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
enum CoreAction<const I: usize, const S: usize, const T: usize> {
    Register(RegistrationIntent, OrdinarySupportSpec, MonotonicTime),
    RegistrationResult(GenerationVector, OwnedRawModelDescriptor),
    Warm(RequestId),
    Describe(OperationId, RequestId, OrdinarySupportSpec, MonotonicTime),
    DescriptionResult(OperationId, RawRequestDescription<I, S, T>),
    PostLoadDescriptorResult(OperationId, OwnedRawModelDescriptor),
    Refresh(DescriptionRefresh),
    DriveDescription(OperationId, MonotonicTime),
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DescriptionRefresh {
    predecessor: SupportCausalPredecessorId,
    at: MonotonicTime,
    obligations: DescriptionObligations,
    result: LifecycleTriggerResult,
    next_backend: Option<BackendGeneration>,
    loaded_revision: Option<ModelRevisionId>,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RawRequestDescription<const I: usize, const S: usize, const T: usize> {
    request: RequestId,
    token_request: TokenRequest<I, S, T>,
    descriptor_id: [u8; 32],
    descriptor_hash_schema: u32,
    descriptor_hash: [u8; 32],
    vocabulary: u32,
    backend: BackendGeneration,
    facts: RequestDescriptionFacts,
}
/// One validated operation request presented at an exact Event Sequence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CoreEvent<const I: usize = 0, const S: usize = 0, const T: usize = 0> {
    sequence: EventSequence,
    operation: Option<OperationId>,
    follow_up: Option<OperationId>,
    action: Option<CoreAction<I, S, T>>,
    request: Option<AcceptanceInput<I, S, T>>,
}
#[rustfmt::skip] #[allow(dead_code, reason = "the runtime adapter consumes the C12 request-description seam later")]
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
            action: None,
            request: None,
        }
    }
    #[allow(dead_code, reason = "the runtime adapter constructs registration events after C10e")]
    #[rustfmt::skip]
    pub(crate) const fn describe_model(sequence: EventSequence, operation: OperationId, intent: RegistrationIntent, support: OrdinarySupportSpec, at: MonotonicTime) -> Self { Self { sequence, operation: Some(operation), follow_up: None, action: Some(CoreAction::Register(intent, support, at)), request: None } }
    #[allow(dead_code, reason = "the runtime adapter constructs registration events after C10e")]
    #[rustfmt::skip]
    pub(crate) fn model_descriptor_result(sequence: EventSequence, operation: OperationId, generations: GenerationVector, result: RawModelDescriptor<'_>) -> Self { Self { sequence, operation: Some(operation), follow_up: None, action: Some(CoreAction::RegistrationResult(generations, OwnedRawModelDescriptor::new(result))), request: None } }
    #[allow(dead_code, reason = "the runtime adapter constructs request events after C11c")]
    #[rustfmt::skip]
    pub(crate) const fn accept_request(sequence: EventSequence, input: AcceptanceInput<I, S, T>) -> Self { Self { sequence, operation: None, follow_up: None, action: None, request: Some(input) } }
    #[rustfmt::skip]
    pub(crate) const fn warm_request(sequence: EventSequence, request: RequestId) -> Self { Self { sequence, operation: None, follow_up: None, action: Some(CoreAction::Warm(request)), request: None } }
    #[rustfmt::skip]
    pub(crate) const fn describe_request(sequence: EventSequence, operation: OperationId, request: RequestId, support: OrdinarySupportSpec, at: MonotonicTime) -> Self { Self { sequence, operation: None, follow_up: None, action: Some(CoreAction::Describe(operation, request, support, at)), request: None } }
    #[rustfmt::skip]
    pub(crate) const fn request_description_result(sequence: EventSequence, operation: OperationId, result: RawRequestDescription<I, S, T>) -> Self { Self { sequence, operation: None, follow_up: None, action: Some(CoreAction::DescriptionResult(operation, result)), request: None } }
    #[rustfmt::skip]
    pub(crate) fn post_load_model_descriptor_result(sequence: EventSequence, operation: OperationId, result: RawModelDescriptor<'_>) -> Self { Self { sequence, operation: None, follow_up: None, action: Some(CoreAction::PostLoadDescriptorResult(operation, OwnedRawModelDescriptor::new(result))), request: None } }
    #[rustfmt::skip]
    pub(crate) const fn refresh_request_descriptions(sequence: EventSequence, predecessor: SupportCausalPredecessorId, at: MonotonicTime, obligations: DescriptionObligations, result: LifecycleTriggerResult, next_backend: Option<BackendGeneration>, loaded_revision: Option<ModelRevisionId>) -> Self { Self { sequence, operation: None, follow_up: None, action: Some(CoreAction::Refresh(DescriptionRefresh { predecessor, at, obligations, result, next_backend, loaded_revision })), request: None } }
    #[rustfmt::skip]
    pub(crate) const fn drive_request_description(sequence: EventSequence, operation: OperationId, at: MonotonicTime) -> Self { Self { sequence, operation: None, follow_up: None, action: Some(CoreAction::DriveDescription(operation, at)), request: None } }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Effect {
    operation: OperationId,
    generations: GenerationVector,
    depends_on: Option<OperationId>,
    registration: Option<RegistrationIntent>,
    request_description: Option<RequestId>,
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
    #[rustfmt::skip] #[allow(dead_code, reason = "the runtime adapter consumes the C12 request-description seam later")]
    pub(crate) const fn request_description(&self) -> Option<RequestId> { self.request_description }
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
    RequestDescriptionUnavailable,
    RequestDescriptionPending,
    RequestDescriptionSupport,
    RequestDescriptionResultMismatch,
    RequestDescriptionState,
    RequestDescriptionDescriptor,
    RequestDescriptionRefreshSet,
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
    pending_description: Option<PendingDescription<INPUT, STOPS, STOP_TOKENS>>,
    description_refresh: Option<DescriptionRefreshState>,
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
#[derive(Clone, Debug, Eq, PartialEq)]
struct OwnedRequestDescriptionInput<const I: usize, const S: usize, const T: usize> {
    request: RequestId,
    token_request: TokenRequest<I, S, T>,
    descriptor_frame: [u8; MAX_FRAME_BYTES],
    descriptor_len: u16,
    descriptor_id: ModelDescriptorId,
    descriptor_hash: ModelDescriptorHash,
    vocabulary: u32,
    backend: BackendGeneration,
}
impl<const I: usize, const S: usize, const T: usize> OwnedRequestDescriptionInput<I, S, T> {
    fn new(
        request: &AcceptedRequest<I, S, T>,
        descriptor: RegisteredDescriptor<'_>,
        backend: BackendGeneration,
        work: &mut WorkMeter,
    ) -> Result<Self, StageFailure> {
        let (frame, descriptor_id, descriptor_hash, vocabulary) = descriptor.values();
        let copied = frame.len() as u64 + std::mem::size_of::<TokenRequest<I, S, T>>() as u64;
        work.ensure(HotPathWorkWitness::new([0, copied, 0, 0, 1]))?;
        work.record(WorkDimension::CopiedBytes, copied)?;
        work.record(WorkDimension::InvariantChecks, 1)?;
        let mut descriptor_frame = [0; MAX_FRAME_BYTES];
        descriptor_frame[..frame.len()].copy_from_slice(frame);
        Ok(Self {
            request: request.id(),
            token_request: *request.request(),
            descriptor_frame,
            descriptor_len: u16::try_from(frame.len()).expect("registered descriptor is bounded"),
            descriptor_id,
            descriptor_hash,
            vocabulary,
            backend,
        })
    }
    fn matches(
        &self,
        result: &RawRequestDescription<I, S, T>,
        work: &mut WorkMeter,
    ) -> Result<bool, WorkBudgetError> {
        work.record(WorkDimension::InvariantChecks, 6)?;
        Ok(result.facts.valid(work)?
            && self.request == result.request
            && self
                .token_request
                .exactly_matches(&result.token_request, work)?
            && self.descriptor_id.bytes() == result.descriptor_id
            && self.descriptor_hash.schema_version() == result.descriptor_hash_schema
            && self.descriptor_hash.digest() == result.descriptor_hash
            && self.vocabulary == result.vocabulary
            && self.backend == result.backend)
    }
}
#[rustfmt::skip] #[derive(Clone, Debug, Eq, PartialEq)] #[allow(clippy::large_enum_variant, reason = "the bounded request input remains allocation-free")]
enum PendingDescription<const I: usize, const S: usize, const T: usize> { Model { operation: OperationId, obligation: SupportOperationObligationId, revision: ModelRevisionId }, Request { operation: OperationId, obligation: SupportOperationObligationId, input: OwnedRequestDescriptionInput<I, S, T> } }
#[rustfmt::skip] #[derive(Clone, Debug, Eq, PartialEq)]
struct DescriptionRefreshState { scope: DescriptionRefreshScope, target: BackendGeneration, model: Option<(SupportOperationObligationId, ModelRevisionId)>, obligations: DescriptionObligations, cursor: Option<RequestId>, next: usize }
#[rustfmt::skip]
pub(crate) struct RequestDescriptionInput<'a, const I: usize, const S: usize, const T: usize> { input: &'a OwnedRequestDescriptionInput<I, S, T> }
#[rustfmt::skip] #[allow(dead_code, reason = "the runtime adapter consumes the C12 request-description seam later")]
impl<const I: usize, const S: usize, const T: usize> RequestDescriptionInput<'_, I, S, T> {
    #[rustfmt::skip]
    pub(crate) fn frame(&self) -> &[u8] { &self.input.descriptor_frame[..usize::from(self.input.descriptor_len)] }
    #[rustfmt::skip]
    pub(crate) const fn token_request(&self) -> &TokenRequest<I, S, T> { &self.input.token_request }
    #[rustfmt::skip]
    pub(crate) const fn backend(&self) -> BackendGeneration { self.input.backend }
    #[rustfmt::skip]
    pub(crate) const fn bound_result(&self, facts: RequestDescriptionFacts) -> RawRequestDescription<I, S, T> { RawRequestDescription { request: self.input.request, token_request: self.input.token_request, descriptor_id: self.input.descriptor_id.bytes(), descriptor_hash_schema: self.input.descriptor_hash.schema_version(), descriptor_hash: self.input.descriptor_hash.digest(), vocabulary: self.input.vocabulary, backend: self.input.backend, facts } }
}
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
            pending_description: None,
            description_refresh: None,
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
    #[rustfmt::skip] #[allow(dead_code, reason = "the runtime adapter consumes the C12 request-description seam later")]
    pub(crate) const fn request_description_input(&self) -> Option<RequestDescriptionInput<'_, I, S, T>> { match &self.pending_description { Some(PendingDescription::Request { input, .. }) => Some(RequestDescriptionInput { input }), _ => None } }
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
            None => match event.action.as_ref() {
                Some(action) => self.apply_action(action, event.operation, &mut work),
                None => self
                    .stage(
                        event.operation.expect("operation event"),
                        event.follow_up,
                        None,
                        None,
                        &mut work,
                    )
                    .map(|(positions, effects)| {
                        self.commit(&positions, &effects);
                        (effects, None)
                    }),
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
    fn apply_action(
        &mut self,
        action: &CoreAction<I, S, T>,
        operation: Option<OperationId>,
        work: &mut WorkMeter,
    ) -> Result<(Effects, Option<RequestAcceptance>), StageFailure> {
        let none = |effects| (effects, None);
        match action {
            CoreAction::Register(intent, support, at) => self
                .start_registration(
                    operation.expect("registration event"),
                    *intent,
                    *support,
                    *at,
                    work,
                )
                .map(none),
            CoreAction::RegistrationResult(generations, result) => self
                .finish_registration(
                    operation.expect("registration event"),
                    *generations,
                    result,
                    work,
                )
                .map(none),
            CoreAction::Warm(request) => self.warm_request(*request, work).map(none),
            CoreAction::Describe(operation, request, support, at) => self
                .start_request_description(*operation, *request, *support, *at, work)
                .map(none),
            CoreAction::DescriptionResult(operation, result) => self
                .finish_request_description(*operation, result, work)
                .map(none),
            CoreAction::PostLoadDescriptorResult(operation, result) => self
                .finish_post_load_model_description(*operation, result, work)
                .map(none),
            CoreAction::Refresh(refresh) => {
                self.resolve_description_refresh(refresh, work).map(none)
            }
            CoreAction::DriveDescription(operation, at) => self
                .drive_request_description(*operation, *at, work)
                .map(none),
        }
    }
    fn stage(
        &self,
        operation: OperationId,
        follow_up: Option<OperationId>,
        registration: Option<RegistrationIntent>,
        request_description: Option<RequestId>,
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
            request_description,
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
        let (positions, effects) = self.stage(operation, None, Some(intent), None, work)?;
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
    fn warm_request(
        &mut self,
        request: RequestId,
        work: &mut WorkMeter,
    ) -> Result<Effects, StageFailure> {
        if self.pending_description.is_some() || self.description_refresh.is_some() {
            return Err(registration_rejection(
                DomainRejection::RequestDescriptionPending,
            ));
        }
        let requests = self.requests.as_ref().ok_or_else(|| {
            registration_rejection(DomainRejection::RequestDescriptionUnavailable)
        })?;
        let change = requests
            .prepare_warming(requests.generation(), request, work)
            .map_err(request_description_failure)?;
        requests
            .validate_description(&change)
            .map_err(request_description_failure)?;
        self.requests
            .as_mut()
            .unwrap()
            .commit_description(change)
            .expect("revalidated Warming transition is infallible");
        Ok(BoundedVec::new())
    }
    fn start_request_description(
        &mut self,
        operation: OperationId,
        request: RequestId,
        support: OrdinarySupportSpec,
        at: MonotonicTime,
        work: &mut WorkMeter,
    ) -> Result<Effects, StageFailure> {
        if self.pending_description.is_some() || self.description_refresh.is_some() {
            return Err(registration_rejection(
                DomainRejection::RequestDescriptionPending,
            ));
        } else if support.operation != SupportOperation::DescribeRequest {
            return Err(registration_rejection(
                DomainRejection::RequestDescriptionSupport,
            ));
        }
        let registration = self.registration.as_ref().ok_or_else(|| {
            registration_rejection(DomainRejection::RequestDescriptionUnavailable)
        })?;
        let requests = self.requests.as_ref().ok_or_else(|| {
            registration_rejection(DomainRejection::RequestDescriptionUnavailable)
        })?;
        let accepted = requests
            .get(request, work)
            .map_err(request_description_failure)?
            .ok_or_else(|| registration_rejection(DomainRejection::RequestDescriptionState))?;
        let revision = accepted.revision_fact();
        let descriptor = registration
            .registry
            .descriptor(revision.revision(), work)
            .map_err(request_registry_failure)?
            .ok_or_else(|| registration_rejection(DomainRejection::RequestDescriptionState))?;
        let input = OwnedRequestDescriptionInput::new(
            accepted,
            descriptor,
            self.state.generations.backend,
            work,
        )?;
        let change = requests
            .prepare_description(
                requests.generation(),
                request,
                self.state.generations.backend,
                at,
                work,
            )
            .map_err(request_description_failure)?;
        let (positions, effects) = self.stage(operation, None, None, Some(request), work)?;
        let support_change = registration
            .support
            .prepare(
                registration.support.generation(),
                SupportChangeInput::BeginOrdinary(support, at),
                work,
            )
            .map_err(request_support_failure)?;
        requests
            .validate_description(&change)
            .map_err(request_description_failure)?;
        registration
            .support
            .validate(&support_change)
            .map_err(request_support_failure)?;
        self.registration
            .as_mut()
            .unwrap()
            .support
            .commit(support_change, work)
            .expect("revalidated support begin is infallible");
        self.requests
            .as_mut()
            .unwrap()
            .commit_description(change)
            .expect("revalidated description start is infallible");
        self.pending_description = Some(PendingDescription::Request {
            operation,
            obligation: support.id,
            input,
        });
        self.commit(&positions, &effects);
        Ok(effects)
    }
    fn resolve_description_refresh(
        &mut self,
        refresh: &DescriptionRefresh,
        work: &mut WorkMeter,
    ) -> Result<Effects, StageFailure> {
        if self.description_refresh.is_some() {
            return Err(registration_rejection(
                DomainRejection::RequestDescriptionPending,
            ));
        }
        let (scope, required) = match (
            refresh.result,
            refresh.next_backend,
            refresh.loaded_revision,
        ) {
            (LifecycleTriggerResult::LoadSucceeded, Some(_), Some(revision)) => {
                (DescriptionRefreshScope::Loaded(revision), true)
            }
            (
                LifecycleTriggerResult::LoadFailed | LifecycleTriggerResult::LoadCancelled,
                None,
                Some(revision),
            ) => (DescriptionRefreshScope::Loaded(revision), false),
            (LifecycleTriggerResult::ObservationDescriptionsRequired, Some(_), None) => {
                (DescriptionRefreshScope::Observation, true)
            }
            (
                LifecycleTriggerResult::ObservationUnchanged
                | LifecycleTriggerResult::ObservationFailed
                | LifecycleTriggerResult::ObservationCancelled,
                None,
                None,
            ) => (DescriptionRefreshScope::Observation, false),
            _ => {
                return Err(registration_rejection(
                    DomainRejection::RequestDescriptionState,
                ));
            }
        };
        let registration = self.registration.as_ref().ok_or_else(|| {
            registration_rejection(DomainRejection::RequestDescriptionUnavailable)
        })?;
        let requests = self.requests.as_ref().ok_or_else(|| {
            registration_rejection(DomainRejection::RequestDescriptionUnavailable)
        })?;
        let mut model = None;
        let mut obligations = DescriptionObligations::new();
        for id in refresh.obligations.iter() {
            let kind = registration
                .support
                .lifecycle_kind(*id, work)
                .map_err(request_support_failure)?;
            match (scope, kind) {
                (
                    DescriptionRefreshScope::Loaded(revision),
                    LifecycleReserveKind::PostLoadModelDescription,
                ) if model.is_none() => model = Some((*id, revision)),
                (
                    DescriptionRefreshScope::Loaded(_),
                    LifecycleReserveKind::PostLoadRequestDescription,
                )
                | (
                    DescriptionRefreshScope::Observation,
                    LifecycleReserveKind::PostObservationRequestDescription,
                ) => obligations.try_push(*id).map_err(|_| {
                    registration_rejection(DomainRejection::RequestDescriptionRefreshSet)
                })?,
                _ => {
                    return Err(registration_rejection(
                        DomainRejection::RequestDescriptionRefreshSet,
                    ));
                }
            }
        }
        if refresh.obligations.is_empty()
            || matches!(scope, DescriptionRefreshScope::Loaded(_)) && model.is_none()
        {
            return Err(registration_rejection(
                DomainRejection::RequestDescriptionSupport,
            ));
        }
        if required {
            let target = refresh
                .next_backend
                .expect("required refresh has generation");
            if self.state.generations.backend.next().ok() != Some(target) {
                return Err(registration_rejection(
                    DomainRejection::RequestDescriptionState,
                ));
            } else if requests
                .refresh_count(scope, work)
                .map_err(request_description_failure)?
                != obligations.len()
            {
                return Err(registration_rejection(
                    DomainRejection::RequestDescriptionRefreshSet,
                ));
            }
        }
        let support_generation = registration.support.generation();
        let copied = (refresh.obligations.len()
            * std::mem::size_of::<SupportOperationObligationId>()) as u64;
        work.ensure(HotPathWorkWitness::new([0, copied, 0, 0, 0]))?;
        let mut ids = [*refresh
            .obligations
            .iter()
            .next()
            .expect("validated nonempty refresh obligations");
            REQUEST_LIMIT + 1];
        for (index, id) in refresh.obligations.iter().enumerate() {
            ids[index] = *id;
        }
        work.record(WorkDimension::CopiedBytes, copied)?;
        self.registration
            .as_mut()
            .unwrap()
            .support
            .resolve_lifecycle(
                support_generation,
                refresh.predecessor,
                refresh.at,
                &ids[..refresh.obligations.len()],
                refresh.result,
                work,
            )
            .map_err(request_support_failure)?;
        if required {
            let target = refresh.next_backend.unwrap();
            self.state.generations.backend = target;
            self.description_refresh = Some(DescriptionRefreshState {
                scope,
                target,
                model,
                obligations,
                cursor: None,
                next: 0,
            });
        }
        Ok(BoundedVec::new())
    }
    fn drive_request_description(
        &mut self,
        operation: OperationId,
        at: MonotonicTime,
        work: &mut WorkMeter,
    ) -> Result<Effects, StageFailure> {
        if self.pending_description.is_some() {
            return Err(registration_rejection(
                DomainRejection::RequestDescriptionPending,
            ));
        }
        let refresh = self
            .description_refresh
            .as_ref()
            .ok_or_else(|| registration_rejection(DomainRejection::RequestDescriptionRefreshSet))?;
        if refresh.target != self.state.generations.backend {
            return Err(registration_rejection(
                DomainRejection::RequestDescriptionState,
            ));
        } else if let Some((obligation, revision)) = refresh.model {
            return self
                .start_post_load_model_description(operation, obligation, revision, at, work);
        }
        let obligation = *refresh
            .obligations
            .get(refresh.next)
            .ok_or_else(|| registration_rejection(DomainRejection::RequestDescriptionRefreshSet))?;
        let (scope, target, cursor) = (refresh.scope, refresh.target, refresh.cursor);
        let requests = self.requests.as_ref().expect("refresh has request state");
        let change = match requests.prepare_refresh(
            requests.generation(),
            scope,
            target,
            cursor,
            at,
            work,
        ) {
            Ok((_, Some(change))) => change,
            Ok((selected, None)) => {
                let generation = self.registration.as_ref().unwrap().support.generation();
                self.registration
                    .as_mut()
                    .unwrap()
                    .support
                    .transition(
                        generation,
                        obligation,
                        crate::support::SupportTransition::CloseCausalCallImpossible,
                        work,
                    )
                    .map_err(request_support_failure)?;
                let refresh = self.description_refresh.as_mut().unwrap();
                refresh.cursor = Some(selected);
                refresh.next += 1;
                if refresh.next == refresh.obligations.len() {
                    self.description_refresh = None;
                }
                return Ok(BoundedVec::new());
            }
            Err(error) => return Err(request_description_failure(error)),
        };
        let accepted = requests
            .description_request(&change)
            .map_err(request_description_failure)?;
        let registration = self
            .registration
            .as_ref()
            .expect("refresh has registration state");
        let descriptor = registration
            .registry
            .descriptor(accepted.revision_fact().revision(), work)
            .map_err(request_registry_failure)?
            .ok_or_else(|| registration_rejection(DomainRejection::RequestDescriptionState))?;
        let request = accepted.id();
        let input = OwnedRequestDescriptionInput::new(accepted, descriptor, target, work)?;
        let (positions, effects) = self.stage(operation, None, None, Some(request), work)?;
        let kind = match scope {
            DescriptionRefreshScope::Loaded(_) => LifecycleReserveKind::PostLoadRequestDescription,
            DescriptionRefreshScope::Observation => {
                LifecycleReserveKind::PostObservationRequestDescription
            }
        };
        let support_change = registration
            .support
            .prepare(
                registration.support.generation(),
                SupportChangeInput::BeginPending(obligation, kind, at),
                work,
            )
            .map_err(request_support_failure)?;
        requests
            .validate_description(&change)
            .map_err(request_description_failure)?;
        registration
            .support
            .validate(&support_change)
            .map_err(request_support_failure)?;
        self.registration
            .as_mut()
            .unwrap()
            .support
            .commit(support_change, work)
            .expect("revalidated pending support begin is infallible");
        self.requests
            .as_mut()
            .unwrap()
            .commit_description(change)
            .expect("revalidated refresh start is infallible");
        let refresh = self.description_refresh.as_mut().unwrap();
        refresh.cursor = Some(request);
        refresh.next += 1;
        self.pending_description = Some(PendingDescription::Request {
            operation,
            obligation,
            input,
        });
        self.commit(&positions, &effects);
        Ok(effects)
    }
    fn start_post_load_model_description(
        &mut self,
        operation: OperationId,
        obligation: SupportOperationObligationId,
        revision: ModelRevisionId,
        at: MonotonicTime,
        work: &mut WorkMeter,
    ) -> Result<Effects, StageFailure> {
        let registration = self
            .registration
            .as_ref()
            .expect("refresh has registration state");
        let record = registration
            .registry
            .revision(revision, work)
            .map_err(request_registry_failure)?
            .ok_or_else(|| registration_rejection(DomainRejection::RequestDescriptionState))?;
        let descriptor = registration
            .registry
            .descriptor(revision, work)
            .map_err(request_registry_failure)?
            .ok_or_else(|| registration_rejection(DomainRejection::RequestDescriptionState))?;
        let intent = RegistrationIntent {
            model: record.model,
            revision,
            manifest: record.manifest,
            expected_descriptor_hash: descriptor.values().2,
            context_limit: record.context_limit,
        };
        let (positions, effects) = self.stage(operation, None, Some(intent), None, work)?;
        let support_change = registration
            .support
            .prepare(
                registration.support.generation(),
                SupportChangeInput::BeginPending(
                    obligation,
                    LifecycleReserveKind::PostLoadModelDescription,
                    at,
                ),
                work,
            )
            .map_err(request_support_failure)?;
        registration
            .support
            .validate(&support_change)
            .map_err(request_support_failure)?;
        self.registration
            .as_mut()
            .unwrap()
            .support
            .commit(support_change, work)
            .expect("revalidated post-load model begin is infallible");
        self.pending_description = Some(PendingDescription::Model {
            operation,
            obligation,
            revision,
        });
        self.commit(&positions, &effects);
        Ok(effects)
    }
    fn finish_request_description(
        &mut self,
        operation: OperationId,
        result: &RawRequestDescription<I, S, T>,
        work: &mut WorkMeter,
    ) -> Result<Effects, StageFailure> {
        let pending = self.pending_description.as_ref().ok_or_else(|| {
            registration_rejection(DomainRejection::RequestDescriptionResultMismatch)
        })?;
        let PendingDescription::Request {
            operation: pending_operation,
            obligation,
            input,
        } = pending
        else {
            return Err(registration_rejection(
                DomainRejection::RequestDescriptionResultMismatch,
            ));
        };
        if *pending_operation != operation || !input.matches(result, work)? {
            return Err(registration_rejection(
                DomainRejection::RequestDescriptionResultMismatch,
            ));
        }
        let (request, target, obligation) = (input.request, input.backend, *obligation);
        let registration = self
            .registration
            .as_ref()
            .expect("pending description has registration state");
        let requests = self
            .requests
            .as_ref()
            .expect("pending description has request state");
        let request_change = requests
            .prepare_description_result(requests.generation(), request, target, &result.facts, work)
            .map_err(request_description_failure)?;
        let support_change = registration
            .support
            .prepare(
                registration.support.generation(),
                SupportChangeInput::FinishActive(obligation),
                work,
            )
            .map_err(request_support_failure)?;
        requests
            .validate_description(&request_change)
            .map_err(request_description_failure)?;
        registration
            .support
            .validate(&support_change)
            .map_err(request_support_failure)?;
        self.registration
            .as_mut()
            .unwrap()
            .support
            .commit(support_change, work)
            .expect("revalidated support finish is infallible");
        self.requests
            .as_mut()
            .unwrap()
            .commit_description(request_change)
            .expect("revalidated description result is infallible");
        self.pending_description = None;
        if self.description_refresh.as_ref().is_some_and(|refresh| {
            refresh.model.is_none() && refresh.next == refresh.obligations.len()
        }) {
            self.description_refresh = None;
        }
        Ok(BoundedVec::new())
    }
    fn finish_post_load_model_description(
        &mut self,
        operation: OperationId,
        result: &OwnedRawModelDescriptor,
        work: &mut WorkMeter,
    ) -> Result<Effects, StageFailure> {
        let pending = self.pending_description.as_ref().ok_or_else(|| {
            registration_rejection(DomainRejection::RequestDescriptionResultMismatch)
        })?;
        let PendingDescription::Model {
            operation: pending_operation,
            obligation,
            revision,
        } = pending
        else {
            return Err(registration_rejection(
                DomainRejection::RequestDescriptionResultMismatch,
            ));
        };
        if *pending_operation != operation {
            return Err(registration_rejection(
                DomainRejection::RequestDescriptionResultMismatch,
            ));
        }
        let (obligation, revision) = (*obligation, *revision);
        let registration = self
            .registration
            .as_ref()
            .expect("pending description has registration state");
        let registered = registration
            .registry
            .descriptor(revision, work)
            .map_err(request_registry_failure)?
            .ok_or_else(|| registration_rejection(DomainRejection::RequestDescriptionState))?;
        let verified = verify(result.as_raw(), registered.values().2, work)
            .map_err(request_descriptor_failure)?;
        if !registered
            .exactly_matches(&verified, work)
            .map_err(request_registry_failure)?
        {
            return Err(registration_rejection(
                DomainRejection::RequestDescriptionDescriptor,
            ));
        }
        let support_change = registration
            .support
            .prepare(
                registration.support.generation(),
                SupportChangeInput::FinishActive(obligation),
                work,
            )
            .map_err(request_support_failure)?;
        registration
            .support
            .validate(&support_change)
            .map_err(request_support_failure)?;
        self.registration
            .as_mut()
            .unwrap()
            .support
            .commit(support_change, work)
            .expect("revalidated post-load model finish is infallible");
        self.pending_description = None;
        self.description_refresh
            .as_mut()
            .expect("post-load model has refresh state")
            .model = None;
        if self
            .description_refresh
            .as_ref()
            .is_some_and(|refresh| refresh.obligations.is_empty())
        {
            self.description_refresh = None;
        }
        Ok(BoundedVec::new())
    }
    fn finish_registration(
        &mut self,
        operation: OperationId,
        generations: GenerationVector,
        result: &OwnedRawModelDescriptor,
        work: &mut WorkMeter,
    ) -> Result<Effects, StageFailure> {
        let state = self
            .registration
            .as_ref()
            .ok_or_else(|| registration_rejection(DomainRejection::ModelRegistrationUnavailable))?;
        let pending = state.pending.ok_or_else(|| {
            registration_rejection(DomainRejection::ModelRegistrationResultMismatch)
        })?;
        if (operation, generations, self.state.generations)
            != (pending.operation, pending.generations, pending.generations)
        {
            return Err(registration_rejection(
                DomainRejection::ModelRegistrationResultMismatch,
            ));
        }
        let descriptor =
            verify(result.as_raw(), pending.expected_hash, work).map_err(descriptor_failure)?;
        let registry_change = state
            .registry
            .prepare_registration(pending.plan, &descriptor, work)
            .map_err(registry_failure)?;
        let support_change = state
            .support
            .prepare(
                state.support.generation(),
                SupportChangeInput::FinishActive(pending.obligation),
                work,
            )
            .map_err(support_failure)?;
        state
            .registry
            .validate(&registry_change)
            .map_err(registry_failure)?;
        state
            .support
            .validate(&support_change)
            .map_err(support_failure)?;
        let state = self
            .registration
            .as_mut()
            .expect("registration state was checked");
        state
            .support
            .commit(support_change, work)
            .expect("revalidated support finish is infallible");
        state
            .registry
            .commit(registry_change)
            .expect("revalidated registry insertion is infallible");
        state.pending = None;
        Ok(BoundedVec::new())
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
    request_description: Option<RequestId>,
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
            request_description,
        })
        .map_err(|_| StageFailure::Invariant)
}
#[rustfmt::skip]
fn registration_rejection(reason: DomainRejection) -> StageFailure { StageFailure::Rejected(reason) }
#[rustfmt::skip]
fn support_failure(error: SupportLedgerError) -> StageFailure { match error { SupportLedgerError::Storage(FixedStorageError::Work(error)) => error.into(), _ => registration_rejection(DomainRejection::ModelRegistrationSupport) } }
#[rustfmt::skip]
fn request_support_failure(error: SupportLedgerError) -> StageFailure { match error { SupportLedgerError::Storage(FixedStorageError::Work(error)) => error.into(), _ => registration_rejection(DomainRejection::RequestDescriptionSupport) } }
#[rustfmt::skip]
fn descriptor_failure(error: ModelDescriptorError) -> StageFailure { match error { ModelDescriptorError::Work(error) => error.into(), _ => registration_rejection(DomainRejection::ModelRegistrationDescriptor) } }
#[rustfmt::skip]
fn registry_failure(error: RegistryError) -> StageFailure { match error { RegistryError::Work(error) | RegistryError::Index(FixedIndexError::Work(error)) => error.into(), _ => registration_rejection(DomainRejection::ModelRegistrationRegistry) } }
#[rustfmt::skip]
fn request_registry_failure(error: RegistryError) -> StageFailure { match error { RegistryError::Work(error) | RegistryError::Index(FixedIndexError::Work(error)) => error.into(), RegistryError::Generation | RegistryError::PreparedChangeStale => registration_rejection(DomainRejection::RequestAcceptanceStale), _ => registration_rejection(DomainRejection::RequestAcceptanceState) } }
#[rustfmt::skip]
fn request_failure(error: RequestError) -> StageFailure { match error { RequestError::Work(error) => error.into(), RequestError::RevisionUnavailable => registration_rejection(DomainRejection::RequestRevisionUnavailable), RequestError::TopK => registration_rejection(DomainRejection::RequestTopK), RequestError::ContextLimit => registration_rejection(DomainRejection::RequestContextLimit), RequestError::PreparationTimeout => registration_rejection(DomainRejection::RequestPreparationTimeout), RequestError::RequestCapacity => registration_rejection(DomainRejection::RequestCapacityExceeded), RequestError::ConnectionCapacity => registration_rejection(DomainRejection::RequestConnectionCapacityExceeded), RequestError::RequestIdExhausted => registration_rejection(DomainRejection::RequestIdExhausted), RequestError::Continuity => registration_rejection(DomainRejection::RequestContinuity), RequestError::RegistryGeneration | RequestError::PreparedChangeStale => registration_rejection(DomainRejection::RequestAcceptanceStale), _ => registration_rejection(DomainRejection::RequestAcceptanceState) } }
#[rustfmt::skip]
fn request_description_failure(error: RequestError) -> StageFailure { match error { RequestError::Work(error) => error.into(), RequestError::PreparationTimeout => registration_rejection(DomainRejection::RequestPreparationTimeout), _ => registration_rejection(DomainRejection::RequestDescriptionState) } }
#[rustfmt::skip]
fn request_descriptor_failure(error: ModelDescriptorError) -> StageFailure { match error { ModelDescriptorError::Work(error) => error.into(), _ => registration_rejection(DomainRejection::RequestDescriptionDescriptor) } }
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
        AcceptanceInput, CapabilityRequirement, DescriptionState, EffectiveSamplingSeed,
        GenerationParameters, RequestBookGeneration, RequestDescriptionFacts, RequestLifecycle,
        RequestSelector, SamplingMode, SamplingSeedOrigin, TokenRequest,
    };
    use crate::support::{
        LifecycleReserveKind, LifecycleReserveMaxima, LifecycleReserveSpec, LifecycleTriggerResult,
        SupportCallScopeId, SupportCausalPredecessorId, SupportFundingClaim,
    };
    use crate::{
        BackendGeneration, BatchBucket, ByteCount, ConnectionId, DaemonInstanceId, Duration,
        ExecutionPhase, FixedStartCountBound, ModelId, MonotonicTime, PhysicalStartCreditId,
        RequestSequence, RuntimeOverheadGeneration, SafetyGeneration, SchedulerGeneration,
        ServiceClass, SupportLedgerGeneration, SupportOperationObligationId, TokenCount,
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
    fn ordinary(n: u8) -> OrdinarySupportSpec { OrdinarySupportSpec { id: SupportOperationObligationId::new([n; 32]).unwrap(), operation: SupportOperation::DescribeModel, physical_credit: PhysicalStartCreditId::new([n + 1; 32]).unwrap(), scope: SupportCallScopeId([n + 2; 32]), claim: SupportFundingClaim::OrdinaryReservation([n + 3; 32]) } }
    #[rustfmt::skip]
    fn ordinary_request(n: u8) -> OrdinarySupportSpec { OrdinarySupportSpec { operation: SupportOperation::DescribeRequest, ..ordinary(n) } }
    #[rustfmt::skip]
    fn reserve_descriptions<const O: usize, const N: usize>(core: &mut Core<O, 2, 1, 2>, n: u8, kinds: [LifecycleReserveKind; N]) -> (SupportCausalPredecessorId, DescriptionObligations) { let predecessor = SupportCausalPredecessorId([n + 1; 32]); let specs: [LifecycleReserveSpec; N] = std::array::from_fn(|offset| { let value = n + offset as u8; LifecycleReserveSpec { id: SupportOperationObligationId::new([value; 32]).unwrap(), kind: kinds[offset], physical_credit: PhysicalStartCreditId::new([value + 2; 32]).unwrap(), predecessor, scope: SupportCallScopeId([value + 3; 32]), claim: SupportFundingClaim::LifecycleReserve([value + 4; 32]), expires_at: None } }); let state = core.registration.as_mut().unwrap(); state.support.reserve_lifecycle(state.support.generation(), MonotonicTime::from_micros(5), &specs, &mut WorkMeter::new(HotPathWorkBudget::binary_maximum())).unwrap(); let mut ids = BoundedVec::new(); for spec in specs { ids.try_push(spec.id).unwrap(); } (predecessor, ids) }
    #[rustfmt::skip]
    fn raw<'a>(frame: &'a [u8], id: [u8; 32], hash: [u8; 32], vocabulary: u32) -> RawModelDescriptor<'a> { RawModelDescriptor { frame, id, hash_schema_version: 1, hash, vocabulary } }
    #[rustfmt::skip]
    fn start<const O: usize, const I: usize, const S: usize, const T: usize>(core: &mut Core<O, I, S, T>, sequence: u64, operation: u128, intent: RegistrationIntent, support: u8) -> CoreTransition { core.handle(CoreEvent::describe_model(EventSequence::new(sequence).unwrap(), OperationId::new(operation).unwrap(), intent, ordinary(support), MonotonicTime::from_micros(sequence))) }
    #[rustfmt::skip]
    fn result<const O: usize, const I: usize, const S: usize, const T: usize>(core: &mut Core<O, I, S, T>, sequence: u64, operation: u128, descriptor: RawModelDescriptor<'_>) -> CoreTransition { core.handle(CoreEvent::model_descriptor_result(EventSequence::new(sequence).unwrap(), OperationId::new(operation).unwrap(), generations(), descriptor)) }
    #[rustfmt::skip]
    fn registration_core(capacity: u32) -> Core<4> {
        let mut capacities = [[0; 3]; 5]; for class in [2, 3, 4] { capacities[class][0] = capacity; }
        let starts = std::array::from_fn(|_| [FixedStartCountBound(Duration::from_micros(10), 2), FixedStartCountBound(Duration::from_micros(20), 2), FixedStartCountBound(Duration::from_micros(30), 2)]);
        let support = CoreSupport::try_new(SupportLedgerGeneration::new(1).unwrap(), capacities, 1, starts, LifecycleReserveMaxima([1; 5])).unwrap();
        let registry = CoreRegistry::try_new(RegistryGeneration::new(1).unwrap()).unwrap();
        Core::bootstrap_with_registration(EventSequence::new(1).unwrap(), generations(), support, registry)
    }
    #[rustfmt::skip]
    fn request_core() -> Core<4, 2, 1, 2> { request_core_with(2) }
    #[rustfmt::skip]
    fn request_core_with<const O: usize>(ordinary: u32) -> Core<O, 2, 1, 2> { let mut capacities = [[0; 3]; 5]; for class in [2, 3, 4] { capacities[class][0] = ordinary; } for capacity in &mut capacities { capacity[1] = 4; } let starts = std::array::from_fn(|_| [FixedStartCountBound(Duration::from_micros(10), 4), FixedStartCountBound(Duration::from_micros(20), 4), FixedStartCountBound(Duration::from_micros(30), 4)]); let support = CoreSupport::try_new(SupportLedgerGeneration::new(1).unwrap(), capacities, 1, starts, LifecycleReserveMaxima([4; 5])).unwrap(); let registry = CoreRegistry::try_new(RegistryGeneration::new(1).unwrap()).unwrap(); Core::bootstrap_with_request_acceptance(EventSequence::new(1).unwrap(), generations(), support, registry, DaemonInstanceId::new(1).unwrap(), RequestBookGeneration::new(1).unwrap()).unwrap() }
    #[rustfmt::skip]
    fn request_input(selector: RequestSelector, connection: u128, input: &[u32], output: u64, top_k: u32, seed: u64, timeout: u64) -> AcceptanceInput<2, 1, 2> { let parameters = if top_k == 0 { GenerationParameters::try_new(SamplingMode::Greedy, 0.0f32.to_bits(), 1.0f32.to_bits(), 0) } else { GenerationParameters::try_new(SamplingMode::Categorical, 1.0f32.to_bits(), 1.0f32.to_bits(), top_k) }.unwrap(); AcceptanceInput { connection: ConnectionId::new(connection).unwrap(), request: TokenRequest::try_new(selector, input, parameters, ServiceClass::Interactive, TokenCount::new(output), &[&[3]], EffectiveSamplingSeed::new(seed, SamplingSeedOrigin::Caller)).unwrap(), accepted_at: MonotonicTime::from_micros(3), preparation_timeout: Duration::from_micros(timeout) } }
    #[rustfmt::skip]
    fn description_facts() -> RequestDescriptionFacts { let mut requirements = BoundedVec::new(); requirements.try_push(CapabilityRequirement { phase: ExecutionPhase::Prefill, batch: BatchBucket(1), shape: 1, route: [1; 32], adapter_build: [2; 32], mlx_build: [3; 32], backend_interface: 1 }).unwrap(); RequestDescriptionFacts { requirements, backend_capabilities: [4; 32], ordinary_estimate: Duration::from_micros(1), conservative_time: Duration::from_micros(2), resource_bytes: ByteCount::new(3), output_bytes: ByteCount::new(4), residency_bytes: ByteCount::new(5) } }
    #[rustfmt::skip]
    fn registered_request_core() -> Core<4, 2, 1, 2> { registered_request_core_with(2) }
    #[rustfmt::skip]
    fn registered_request_core_with<const O: usize>(ordinary: u32) -> Core<O, 2, 1, 2> { let mut core = request_core_with(ordinary); assert_eq!(start(&mut core, 1, 1, registration(HASH), 5).outcome(), &CoreOutcome::Accepted); assert_eq!(result(&mut core, 2, 1, raw(&FRAME, ID, HASH, 7)).outcome(), &CoreOutcome::Accepted); core }
    #[rustfmt::skip]
    fn install_revision<const O: usize>(core: &mut Core<O, 2, 1, 2>, intent: RegistrationIntent) { let mut work = WorkMeter::new(HotPathWorkBudget::binary_maximum()); let descriptor = verify(raw(&FRAME, ID, HASH, 7), intent.expected_descriptor_hash, &mut work).unwrap(); let registry = &mut core.registration.as_mut().unwrap().registry; let plan = registry.prepare_description(registry.generation(), intent, &mut work).unwrap(); let change = registry.prepare_registration(plan, &descriptor, &mut work).unwrap(); registry.commit(change).unwrap(); }
    #[rustfmt::skip]
    fn accepted<const O: usize>(core: &Core<O, 2, 1, 2>, id: RequestId) -> AcceptedRequest<2, 1, 2> { core.requests.as_ref().unwrap().get(id, &mut WorkMeter::new(HotPathWorkBudget::binary_maximum())).unwrap().unwrap().clone() }
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
        assert_eq!(transition.work(), HotPathWorkWitness::new([3, 736, 0, 0, 14]));
        assert_eq!((accepted.id().sequence().get(), accepted.revision(), accepted.seed().value(), accepted.seed().origin(), accepted.status().get(), accepted.lifecycle()), (1, revision, 9, SamplingSeedOrigin::Caller, 1, RequestLifecycle::Preparing));
        let alias = ModelAliasId::new([4; 32]).unwrap(); { let registry = &mut core.registration.as_mut().unwrap().registry; let mut work = WorkMeter::new(HotPathWorkBudget::binary_maximum()); let change = registry.prepare(registry.generation(), RegistryCommand::BindAlias(alias, revision), &mut work).unwrap(); registry.commit(change).unwrap(); }
        let next = core.handle(CoreEvent::accept_request(EventSequence::new(4).unwrap(), request_input(RequestSelector::Alias(alias), 2, &[1], 1, 0, 10, 5))); let accepted = next.request_acceptance().unwrap();
        assert_eq!(next.work(), HotPathWorkWitness::new([5, 736, 0, 0, 16]));
        assert_eq!((accepted.id().sequence().get(), accepted.revision(), accepted.seed().value(), core.requests.as_ref().unwrap().len()), (2, revision, 10, 2));
    }
    #[rustfmt::skip] #[test]
    fn drives_request_descriptions_without_admission() {
        let revision = ModelRevisionId::new([2; 32]).unwrap(); let mut core = registered_request_core(); let accepted = core.handle(CoreEvent::accept_request(EventSequence::new(3).unwrap(), request_input(RequestSelector::Direct(revision), 2, &[1], 1, 0, 9, 5))).request_acceptance().unwrap(); let started = core.handle(CoreEvent::describe_request(EventSequence::new(4).unwrap(), OperationId::new(2).unwrap(), accepted.id(), ordinary_request(6), MonotonicTime::from_micros(4))); assert_eq!((started.outcome(), started.effects().len(), started.effects().get(0).unwrap().request_description(), started.work()), (&CoreOutcome::Accepted, 1, Some(accepted.id()), HotPathWorkWitness::new([23, 681, 0, 1, 31])));
        let facts = description_facts(); let result = core.request_description_input().unwrap().bound_result(facts); let mut wrong = result; wrong.backend = BackendGeneration::new(2).unwrap(); let rejected = core.handle(CoreEvent::request_description_result(EventSequence::new(5).unwrap(), OperationId::new(2).unwrap(), wrong)); assert_eq!((rejected.outcome(), core.pending_description.is_some()), (&CoreOutcome::Rejected(DomainRejection::RequestDescriptionResultMismatch), true)); let mut incomplete = result; incomplete.facts.requirements = BoundedVec::new(); let rejected = core.handle(CoreEvent::request_description_result(EventSequence::new(6).unwrap(), OperationId::new(2).unwrap(), incomplete)); assert_eq!((rejected.outcome(), core.pending_description.is_some()), (&CoreOutcome::Rejected(DomainRejection::RequestDescriptionResultMismatch), true)); let other = RegistrationIntent { model: ModelId::new(2).unwrap(), revision: ModelRevisionId::new([4; 32]).unwrap(), manifest: ModelManifestId::new([5; 32]).unwrap(), ..registration(HASH) }; install_revision(&mut core, other); let (predecessor, ids) = reserve_descriptions::<4, 1>(&mut core, 10, [LifecycleReserveKind::PostObservationRequestDescription]); let invalidated = core.handle(CoreEvent::refresh_request_descriptions(EventSequence::new(7).unwrap(), predecessor, MonotonicTime::from_micros(6), ids, LifecycleTriggerResult::ObservationDescriptionsRequired, Some(BackendGeneration::new(2).unwrap()), None)); assert_eq!((invalidated.outcome(), invalidated.effects().len()), (&CoreOutcome::Accepted, 0));
        core.work_budget = HotPathWorkBudget::try_new(HotPathWorkWitness::new([1_000_000, 321, 0, 2, 2_100])).unwrap(); let underfunded = core.handle(CoreEvent::request_description_result(EventSequence::new(8).unwrap(), OperationId::new(2).unwrap(), result)); assert_eq!((underfunded.outcome(), underfunded.work(), core.pending_description.is_some(), core.requests.as_ref().unwrap().description_facts(accepted.id(), &mut WorkMeter::new(HotPathWorkBudget::binary_maximum())).unwrap()), (&CoreOutcome::Rejected(DomainRejection::HotPathWorkBudget(WorkBudgetError::BudgetExceeded(WorkDimension::CopiedBytes, 321, 27_872))), HotPathWorkWitness::new([6, 0, 0, 0, 30]), true, None)); core.work_budget = HotPathWorkBudget::binary_maximum(); let initial_completed = core.handle(CoreEvent::request_description_result(EventSequence::new(9).unwrap(), OperationId::new(2).unwrap(), result)); assert_eq!((initial_completed.outcome(), initial_completed.effects().len(), initial_completed.request_acceptance(), initial_completed.work()), (&CoreOutcome::Accepted, 0, None, HotPathWorkWitness::new([10, 28_049, 0, 0, 35]))); let refreshed = core.handle(CoreEvent::drive_request_description(EventSequence::new(10).unwrap(), OperationId::new(3).unwrap(), MonotonicTime::from_micros(7))); assert_eq!((invalidated.work(), refreshed.outcome(), refreshed.effects().get(0).unwrap().request_description(), refreshed.work()), (HotPathWorkWitness::new([7, 353, 0, 0, 6]), &CoreOutcome::Accepted, Some(accepted.id()), HotPathWorkWitness::new([9, 566, 0, 1, 17]))); let result = core.request_description_input().unwrap().bound_result(facts); let completed = core.handle(CoreEvent::request_description_result(EventSequence::new(11).unwrap(), OperationId::new(3).unwrap(), result)); assert_eq!((completed.outcome(), completed.effects().len(), completed.request_acceptance(), completed.work()), (&CoreOutcome::Accepted, 0, None, HotPathWorkWitness::new([9, 28_049, 0, 0, 35])));
    }
    #[rustfmt::skip] #[test]
    fn post_load_refresh_is_stable_and_defers_unrelated_warming() {
        std::thread::Builder::new().stack_size(8 << 20).spawn(|| { let first = ModelRevisionId::new([2; 32]).unwrap(); let second = ModelRevisionId::new([4; 32]).unwrap(); let mut core = registered_request_core_with::<8>(2); let intent = RegistrationIntent { model: ModelId::new(2).unwrap(), revision: second, manifest: ModelManifestId::new([5; 32]).unwrap(), expected_descriptor_hash: ModelDescriptorHash::from_manifest(1, HASH).unwrap(), context_limit: TokenCount::new(8) }; install_revision(&mut core, intent); let resident = core.handle(CoreEvent::accept_request(EventSequence::new(3).unwrap(), request_input(RequestSelector::Direct(first), 3, &[1], 1, 0, 1, 5))).request_acceptance().unwrap().id(); let warming = core.handle(CoreEvent::accept_request(EventSequence::new(4).unwrap(), request_input(RequestSelector::Direct(first), 2, &[1], 1, 0, 2, 5))).request_acceptance().unwrap().id(); let unrelated = core.handle(CoreEvent::accept_request(EventSequence::new(5).unwrap(), request_input(RequestSelector::Direct(second), 4, &[1], 1, 0, 3, 5))).request_acceptance().unwrap().id(); assert_eq!(core.handle(CoreEvent::warm_request(EventSequence::new(6).unwrap(), warming)).outcome(), &CoreOutcome::Accepted); assert_eq!(core.handle(CoreEvent::warm_request(EventSequence::new(7).unwrap(), unrelated)).outcome(), &CoreOutcome::Accepted); assert_eq!(core.handle(CoreEvent::describe_request(EventSequence::new(8).unwrap(), OperationId::new(2).unwrap(), resident, ordinary_request(6), MonotonicTime::from_micros(4))).outcome(), &CoreOutcome::Accepted); let initial = core.request_description_input().unwrap().bound_result(description_facts()); assert_eq!(core.handle(CoreEvent::request_description_result(EventSequence::new(9).unwrap(), OperationId::new(2).unwrap(), initial)).outcome(), &CoreOutcome::Accepted); let kinds = [LifecycleReserveKind::PostLoadModelDescription, LifecycleReserveKind::PostLoadRequestDescription, LifecycleReserveKind::PostLoadRequestDescription]; let (predecessor, ids) = reserve_descriptions::<8, 3>(&mut core, 20, kinds); let refreshed = core.handle(CoreEvent::refresh_request_descriptions(EventSequence::new(10).unwrap(), predecessor, MonotonicTime::from_micros(6), ids, LifecycleTriggerResult::LoadSucceeded, Some(BackendGeneration::new(2).unwrap()), Some(first))); assert_eq!((refreshed.outcome(), core.state.generations.backend), (&CoreOutcome::Accepted, BackendGeneration::new(2).unwrap()));
        let model = core.handle(CoreEvent::drive_request_description(EventSequence::new(11).unwrap(), OperationId::new(3).unwrap(), MonotonicTime::from_micros(7))); assert_eq!((model.effects().get(0).unwrap().registration(), model.effects().get(0).unwrap().request_description()), (Some(registration(HASH)), None)); let drift = core.handle(CoreEvent::post_load_model_descriptor_result(EventSequence::new(12).unwrap(), OperationId::new(3).unwrap(), raw(&FRAME, ID, HASH, 8))); assert_eq!((drift.outcome(), core.pending_description.is_some()), (&CoreOutcome::Rejected(DomainRejection::RequestDescriptionDescriptor), true)); let checked = core.handle(CoreEvent::post_load_model_descriptor_result(EventSequence::new(13).unwrap(), OperationId::new(3).unwrap(), raw(&FRAME, ID, HASH, 7))); assert_eq!(checked.outcome(), &CoreOutcome::Accepted); for (sequence, operation, expected) in [(14, 4, warming), (16, 5, resident)] { let driven = core.handle(CoreEvent::drive_request_description(EventSequence::new(sequence).unwrap(), OperationId::new(operation).unwrap(), MonotonicTime::from_micros(7))); assert_eq!(driven.effects().get(0).unwrap().request_description(), Some(expected)); let result = core.request_description_input().unwrap().bound_result(description_facts()); assert_eq!(core.handle(CoreEvent::request_description_result(EventSequence::new(sequence + 1).unwrap(), OperationId::new(operation).unwrap(), result)).outcome(), &CoreOutcome::Accepted); } assert_eq!((accepted(&core, warming).lifecycle(), accepted(&core, resident).description(), core.requests.as_ref().unwrap().description_facts(resident, &mut WorkMeter::new(HotPathWorkBudget::binary_maximum())).unwrap(), accepted(&core, unrelated).lifecycle(), accepted(&core, unrelated).description(), accepted(&core, unrelated).deadline().as_micros()), (RequestLifecycle::Preparing, DescriptionState::Ready(BackendGeneration::new(2).unwrap()), Some(&description_facts()), RequestLifecycle::Warming, DescriptionState::Missing, 8)); let unreserved = core.handle(CoreEvent::drive_request_description(EventSequence::new(18).unwrap(), OperationId::new(6).unwrap(), MonotonicTime::from_micros(8))); assert_eq!((unreserved.outcome(), unreserved.effects().len()), (&CoreOutcome::Rejected(DomainRejection::RequestDescriptionRefreshSet), 0)); }).unwrap().join().unwrap();
    }
    #[rustfmt::skip] #[test]
    fn description_rejections_and_impossible_triggers_emit_nothing() {
        let revision = ModelRevisionId::new([2; 32]).unwrap(); let mut exhausted = registered_request_core_with::<4>(1); let id = exhausted.handle(CoreEvent::accept_request(EventSequence::new(3).unwrap(), request_input(RequestSelector::Direct(revision), 2, &[1], 1, 0, 9, 5))).request_acceptance().unwrap().id(); let support = exhausted.registration.as_ref().unwrap().support.generation(); let rejected = exhausted.handle(CoreEvent::describe_request(EventSequence::new(4).unwrap(), OperationId::new(2).unwrap(), id, ordinary_request(6), MonotonicTime::from_micros(4))); assert_eq!((rejected.outcome(), rejected.effects().len(), rejected.work(), exhausted.pending_description.is_none(), accepted(&exhausted, id).description(), exhausted.registration.as_ref().unwrap().support.generation()), (&CoreOutcome::Rejected(DomainRejection::RequestDescriptionSupport), 0, HotPathWorkWitness::new([5, 381, 0, 1, 16]), true, DescriptionState::Missing, support)); for result in [LifecycleTriggerResult::LoadFailed, LifecycleTriggerResult::LoadCancelled, LifecycleTriggerResult::ObservationUnchanged, LifecycleTriggerResult::ObservationFailed, LifecycleTriggerResult::ObservationCancelled] { let load = matches!(result, LifecycleTriggerResult::LoadFailed | LifecycleTriggerResult::LoadCancelled); let mut core = registered_request_core(); let id = core.handle(CoreEvent::accept_request(EventSequence::new(3).unwrap(), request_input(RequestSelector::Direct(revision), 2, &[1], 1, 0, 9, 5))).request_acceptance().unwrap().id(); if load { assert_eq!(core.handle(CoreEvent::warm_request(EventSequence::new(4).unwrap(), id)).outcome(), &CoreOutcome::Accepted); let blocked = core.handle(CoreEvent::describe_request(EventSequence::new(5).unwrap(), OperationId::new(2).unwrap(), id, ordinary_request(6), MonotonicTime::from_micros(5))); assert_eq!((blocked.outcome(), blocked.effects().len()), (&CoreOutcome::Rejected(DomainRejection::RequestDescriptionState), 0)); } let kinds = if load { [LifecycleReserveKind::PostLoadModelDescription, LifecycleReserveKind::PostLoadRequestDescription] } else { [LifecycleReserveKind::PostObservationRequestDescription; 2] }; let (predecessor, ids) = reserve_descriptions::<4, 2>(&mut core, 20, kinds); let sequence = 4 + 2 * u64::from(load); let closed = core.handle(CoreEvent::refresh_request_descriptions(EventSequence::new(sequence).unwrap(), predecessor, MonotonicTime::from_micros(6), ids, result, None, if load { Some(revision) } else { None })); assert_eq!((closed.outcome(), closed.effects().len(), core.state.generations.backend, core.description_refresh.is_none()), (&CoreOutcome::Accepted, 0, BackendGeneration::new(1).unwrap(), true)); }
        let mut empty = registered_request_core(); let before = empty.registration.as_ref().unwrap().support.generation(); let transition = empty.handle(CoreEvent::refresh_request_descriptions(EventSequence::new(3).unwrap(), SupportCausalPredecessorId([30; 32]), MonotonicTime::from_micros(6), BoundedVec::new(), LifecycleTriggerResult::ObservationDescriptionsRequired, Some(BackendGeneration::new(2).unwrap()), None)); assert_eq!((transition.outcome(), transition.effects().len(), empty.state.generations.backend, empty.registration.as_ref().unwrap().support.generation()), (&CoreOutcome::Rejected(DomainRejection::RequestDescriptionSupport), 0, BackendGeneration::new(1).unwrap(), before)); let mut expired = registered_request_core(); let id = expired.handle(CoreEvent::accept_request(EventSequence::new(3).unwrap(), request_input(RequestSelector::Direct(revision), 2, &[1], 1, 0, 9, 5))).request_acceptance().unwrap().id(); let transition = expired.handle(CoreEvent::describe_request(EventSequence::new(4).unwrap(), OperationId::new(2).unwrap(), id, ordinary_request(6), MonotonicTime::from_micros(8))); assert_eq!((transition.outcome(), accepted(&expired, id).description()), (&CoreOutcome::Rejected(DomainRejection::RequestPreparationTimeout), DescriptionState::Missing)); std::thread::Builder::new().stack_size(8 << 20).spawn(move || { let mut refresh = registered_request_core_with::<4>(3); let expired = refresh.handle(CoreEvent::accept_request(EventSequence::new(3).unwrap(), request_input(RequestSelector::Direct(revision), 2, &[1], 1, 0, 9, 5))).request_acceptance().unwrap().id(); assert_eq!(refresh.handle(CoreEvent::describe_request(EventSequence::new(4).unwrap(), OperationId::new(2).unwrap(), expired, ordinary_request(6), MonotonicTime::from_micros(4))).outcome(), &CoreOutcome::Accepted); let result = refresh.request_description_input().unwrap().bound_result(description_facts()); assert_eq!(refresh.handle(CoreEvent::request_description_result(EventSequence::new(5).unwrap(), OperationId::new(2).unwrap(), result)).outcome(), &CoreOutcome::Accepted); let fresh = refresh.handle(CoreEvent::accept_request(EventSequence::new(6).unwrap(), request_input(RequestSelector::Direct(revision), 3, &[1], 1, 0, 10, 100))).request_acceptance().unwrap().id(); assert_eq!(refresh.handle(CoreEvent::describe_request(EventSequence::new(7).unwrap(), OperationId::new(3).unwrap(), fresh, ordinary_request(10), MonotonicTime::from_micros(4))).outcome(), &CoreOutcome::Accepted); let result = refresh.request_description_input().unwrap().bound_result(description_facts()); assert_eq!(refresh.handle(CoreEvent::request_description_result(EventSequence::new(8).unwrap(), OperationId::new(3).unwrap(), result)).outcome(), &CoreOutcome::Accepted); let (predecessor, ids) = reserve_descriptions::<4, 2>(&mut refresh, 50, [LifecycleReserveKind::PostObservationRequestDescription; 2]); assert_eq!(refresh.handle(CoreEvent::refresh_request_descriptions(EventSequence::new(9).unwrap(), predecessor, MonotonicTime::from_micros(8), ids, LifecycleTriggerResult::ObservationDescriptionsRequired, Some(BackendGeneration::new(2).unwrap()), None)).outcome(), &CoreOutcome::Accepted); let closed = refresh.handle(CoreEvent::drive_request_description(EventSequence::new(10).unwrap(), OperationId::new(4).unwrap(), MonotonicTime::from_micros(8))); assert_eq!((closed.outcome(), closed.effects().len(), refresh.description_refresh.as_ref().unwrap().next), (&CoreOutcome::Accepted, 0, 1)); let driven = refresh.handle(CoreEvent::drive_request_description(EventSequence::new(11).unwrap(), OperationId::new(5).unwrap(), MonotonicTime::from_micros(8))); assert_eq!((driven.outcome(), driven.effects().get(0).unwrap().request_description()), (&CoreOutcome::Accepted, Some(fresh))); }).unwrap().join().unwrap();
    }
    #[rustfmt::skip] #[test]
    fn refresh_rejects_the_wrong_lifecycle_kind_before_mutation() { let mut core = registered_request_core(); let (predecessor, ids) = reserve_descriptions::<4, 1>(&mut core, 40, [LifecycleReserveKind::PostLoadRequestDescription]); let before = core.registration.as_ref().unwrap().support.generation(); let transition = core.handle(CoreEvent::refresh_request_descriptions(EventSequence::new(3).unwrap(), predecessor, MonotonicTime::from_micros(6), ids, LifecycleTriggerResult::ObservationDescriptionsRequired, Some(BackendGeneration::new(2).unwrap()), None)); assert_eq!((transition.outcome(), core.state.generations.backend, core.registration.as_ref().unwrap().support.generation()), (&CoreOutcome::Rejected(DomainRejection::RequestDescriptionRefreshSet), BackendGeneration::new(1).unwrap(), before)); }
    #[test]
    #[rustfmt::skip]
    fn request_acceptance_rejections_assign_no_id_or_success() {
        let revision = ModelRevisionId::new([2; 32]).unwrap(); let alias = ModelAliasId::new([4; 32]).unwrap();
        for (selector, rejection) in [(RequestSelector::Direct(revision), DomainRejection::UnknownRequestRevision), (RequestSelector::Alias(alias), DomainRejection::UnknownRequestAlias)] { let mut core = request_core(); let transition = core.handle(CoreEvent::accept_request(EventSequence::new(1).unwrap(), request_input(selector, 2, &[1], 1, 0, 9, 5))); assert_eq!(transition.outcome(), &CoreOutcome::Rejected(rejection)); assert!(transition.request_acceptance().is_none()); assert_eq!(core.requests.as_ref().unwrap().len(), 0); }
        for (input, rejection) in [(request_input(RequestSelector::Direct(revision), 2, &[1], 1, 7, 9, 5), DomainRejection::RequestTopK), (request_input(RequestSelector::Direct(revision), 2, &[1], 8, 0, 9, 5), DomainRejection::RequestContextLimit), (request_input(RequestSelector::Direct(revision), 2, &[1], 1, 0, 9, 0), DomainRejection::RequestPreparationTimeout)] { let mut core = registered_request_core(); let before = (core.requests.as_ref().unwrap().generation(), core.requests.as_ref().unwrap().len()); let rejected = core.handle(CoreEvent::accept_request(EventSequence::new(3).unwrap(), input)); assert_eq!((rejected.outcome(), rejected.request_acceptance()), (&CoreOutcome::Rejected(rejection), None)); assert_eq!((core.requests.as_ref().unwrap().generation(), core.requests.as_ref().unwrap().len()), before); let accepted = core.handle(CoreEvent::accept_request(EventSequence::new(4).unwrap(), request_input(RequestSelector::Direct(revision), 2, &[1], 1, 0, 9, 5))).request_acceptance().unwrap(); assert_eq!(accepted.id().sequence().get(), 1); }
        let mut unavailable = registered_request_core(); { let registry = &mut unavailable.registration.as_mut().unwrap().registry; let mut work = WorkMeter::new(HotPathWorkBudget::binary_maximum()); let change = registry.prepare(registry.generation(), RegistryCommand::BindAlias(alias, revision), &mut work).unwrap(); registry.commit(change).unwrap(); let change = registry.prepare(registry.generation(), RegistryCommand::MarkUnavailable(revision), &mut work).unwrap(); registry.commit(change).unwrap(); } for (offset, selector) in [RequestSelector::Direct(revision), RequestSelector::Alias(alias)].into_iter().enumerate() { let rejected = unavailable.handle(CoreEvent::accept_request(EventSequence::new(3 + offset as u64).unwrap(), request_input(selector, 2, &[1], 1, 0, 9, 5))); assert_eq!((rejected.outcome(), rejected.request_acceptance(), unavailable.requests.as_ref().unwrap().len()), (&CoreOutcome::Rejected(DomainRejection::RequestRevisionUnavailable), None, 0)); }
        let input = request_input(RequestSelector::Direct(revision), 2, &[1], 1, 0, 9, 5); let mut constrained = registered_request_core(); constrained.work_budget = HotPathWorkBudget::try_new(HotPathWorkWitness::new([1_000_000, 0, 0, 2, 2_100])).unwrap(); let rejected = constrained.handle(CoreEvent::accept_request(EventSequence::new(3).unwrap(), input)); assert_eq!(rejected.outcome(), &CoreOutcome::Rejected(DomainRejection::HotPathWorkBudget(WorkBudgetError::BudgetExceeded(WorkDimension::CopiedBytes, 0, 224)))); assert_eq!((rejected.work(), rejected.request_acceptance(), constrained.requests.as_ref().unwrap().len()), (HotPathWorkWitness::new([2, 0, 0, 0, 1]), None, 0)); constrained.work_budget = HotPathWorkBudget::binary_maximum(); let retried = constrained.handle(CoreEvent::accept_request(EventSequence::new(4).unwrap(), input)); assert_eq!((retried.request_acceptance().unwrap().id().sequence().get(), retried.work(), constrained.requests.as_ref().unwrap().len()), (1, HotPathWorkWitness::new([3, 736, 0, 0, 14]), 1));
    }
    #[test]
    #[rustfmt::skip]
    fn request_capacity_and_internal_rejections_are_closed() {
        let mut obligations = DescriptionObligations::new(); for _ in 0..=REQUEST_LIMIT { obligations.try_push(SupportOperationObligationId::new([1; 32]).unwrap()).unwrap(); } assert_eq!(obligations.len(), REQUEST_LIMIT + 1); std::thread::Builder::new().stack_size(8 << 20).spawn(|| { let mut capacities = [[0; 3]; 5]; for capacity in &mut capacities { capacity[1] = 1_025; } let starts = std::array::from_fn(|_| [FixedStartCountBound(Duration::from_micros(10), 1_025), FixedStartCountBound(Duration::from_micros(20), 1_025), FixedStartCountBound(Duration::from_micros(30), 1_025)]); let mut ledger = SupportChargeLedger::<4_096, 1_100, 3>::try_new(SupportLedgerGeneration::new(1).unwrap(), capacities, 1, starts, LifecycleReserveMaxima([1, 1_024, 1_024, 1, 1])).unwrap(); let identity = |tag: u8, value: usize| { let mut id = [0; 32]; id[0] = tag; id[28..].copy_from_slice(&u32::try_from(value).unwrap().to_be_bytes()); id }; let predecessor = SupportCausalPredecessorId([9; 32]); let specs: Vec<_> = (1..=1_025).map(|value| LifecycleReserveSpec { id: SupportOperationObligationId::new(identity(1, value)).unwrap(), kind: if value == 1 { LifecycleReserveKind::PostLoadModelDescription } else { LifecycleReserveKind::PostLoadRequestDescription }, physical_credit: PhysicalStartCreditId::new(identity(2, value)).unwrap(), predecessor, scope: SupportCallScopeId(identity(3, value)), claim: SupportFundingClaim::LifecycleReserve(identity(4, value)), expires_at: None }).collect(); let mut work = WorkMeter::new(HotPathWorkBudget::binary_maximum()); ledger.reserve_lifecycle(ledger.generation(), MonotonicTime::from_micros(5), &specs, &mut work).unwrap(); assert_eq!(work.witness(), HotPathWorkWitness::new([108_505, 299_288, 0, 0, 18_458])); }).unwrap().join().unwrap(); let revision = ModelRevisionId::new([2; 32]).unwrap(); let mut core = registered_request_core(); for offset in 0..REQUEST_LIMIT { let transition = core.handle(CoreEvent::accept_request(EventSequence::new(3 + offset as u64).unwrap(), request_input(RequestSelector::Direct(revision), 2, &[1], 1, 0, 9, 5))); assert_eq!(transition.request_acceptance().unwrap().id().sequence().get(), offset as u64 + 1); } let rejected = core.handle(CoreEvent::accept_request(EventSequence::new(3 + REQUEST_LIMIT as u64).unwrap(), request_input(RequestSelector::Direct(revision), 2, &[1], 1, 0, 9, 5))); assert_eq!((rejected.outcome(), rejected.request_acceptance(), core.requests.as_ref().unwrap().len()), (&CoreOutcome::Rejected(DomainRejection::RequestCapacityExceeded), None, REQUEST_LIMIT));
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
