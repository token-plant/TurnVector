#![allow(dead_code, reason = "later C11 rows consume bounded token requests")]

use crate::WorkDimension::{CopiedBytes, InvariantChecks, VisitedEntities};
use crate::model_registry::{
    MODEL_REGISTRY_LIMIT, ModelAliasId, ModelRevisionId, RegistryGeneration, RequestRevision,
    RevisionLifecycle, RevisionSelection,
};
use crate::{
    BackendGeneration, BatchBucket, BoundedVec, ByteCount, ConnectionId, DaemonInstanceId,
    Duration, ExecutionPhase, MonotonicTime, RequestId, RequestSequence, RequestStatusVersion,
    ServiceClass, TokenCount, WorkBudgetError, WorkMeter,
};
use std::mem::size_of;

pub(crate) const REQUEST_LIMIT: usize = 1_024;
pub(crate) const CONNECTION_LIMIT: usize = 64;
pub(crate) const REQUEST_REQUIREMENT_LIMIT: usize = 256;
type RequestResult<T> = Result<T, RequestError>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RequestSelector {
    Direct(ModelRevisionId),
    Alias(ModelAliasId),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SamplingMode {
    Greedy,
    Categorical,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SamplingSeedOrigin {
    Caller,
    Daemon,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct EffectiveSamplingSeed {
    value: u64,
    origin: SamplingSeedOrigin,
}

impl EffectiveSamplingSeed {
    #[rustfmt::skip]
    pub(crate) const fn new(value: u64, origin: SamplingSeedOrigin) -> Self { Self { value, origin } }
    #[rustfmt::skip]
    pub(crate) const fn value(self) -> u64 { self.value }
    #[rustfmt::skip]
    pub(crate) const fn origin(self) -> SamplingSeedOrigin { self.origin }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct GenerationParameters {
    mode: SamplingMode,
    temperature_bits: u32,
    top_p_bits: u32,
    top_k: u32,
}

impl GenerationParameters {
    pub(crate) fn try_new(
        mode: SamplingMode,
        temperature_bits: u32,
        top_p_bits: u32,
        top_k: u32,
    ) -> Result<Self, RequestError> {
        let (temperature, top_p) = (f32::from_bits(temperature_bits), f32::from_bits(top_p_bits));
        let valid = match mode {
            SamplingMode::Greedy => {
                temperature_bits == 0 && top_p_bits == 1.0f32.to_bits() && top_k == 0
            }
            SamplingMode::Categorical => {
                temperature.is_finite()
                    && temperature > 0.0
                    && temperature <= 2.0
                    && top_p.is_finite()
                    && top_p > 0.0
                    && top_p <= 1.0
            }
        };
        valid
            .then_some(Self {
                mode,
                temperature_bits,
                top_p_bits,
                top_k,
            })
            .ok_or(RequestError::GenerationParameters)
    }

    #[rustfmt::skip]
    pub(crate) const fn mode(self) -> SamplingMode { self.mode }
    #[rustfmt::skip]
    pub(crate) const fn temperature_bits(self) -> u32 { self.temperature_bits }
    #[rustfmt::skip]
    pub(crate) const fn top_p_bits(self) -> u32 { self.top_p_bits }
    #[rustfmt::skip]
    pub(crate) const fn top_k(self) -> u32 { self.top_k }
}

#[rustfmt::skip]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RequestError {
    Allocation, InvalidGeneration, GenerationOverflow, RequestCapacity, ConnectionCapacity,
    RegistryGeneration, SelectorMismatch, RevisionUnavailable, TopK, ContextLimit,
    PreparationTimeout, RequestIdExhausted, Continuity, PreparedChangeStale, Work(WorkBudgetError),
    GenerationParameters,
    InputTokenCapacity,
    MaxOutputTokens,
    StopSequenceCapacity,
    EmptyStopTokenSequence,
    StopTokenCapacity,
    UnknownRequest,
    DescriptionState,
}
impl From<WorkBudgetError> for RequestError {
    fn from(error: WorkBudgetError) -> Self {
        Self::Work(error)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RequestBookGeneration(u64);
#[rustfmt::skip]
impl RequestBookGeneration {
    pub(crate) fn new(value: u64) -> RequestResult<Self> { (value != 0).then_some(Self(value)).ok_or(RequestError::InvalidGeneration) }
    fn next(self) -> RequestResult<Self> { self.0.checked_add(1).map(Self).ok_or(RequestError::GenerationOverflow) }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TokenRequest<const INPUT: usize, const STOPS: usize, const STOP_TOKENS: usize> {
    selector: RequestSelector,
    input: BoundedVec<u32, INPUT>,
    parameters: GenerationParameters,
    service: ServiceClass,
    max_output: TokenCount,
    stops: BoundedVec<BoundedVec<u32, STOP_TOKENS>, STOPS>,
    seed: EffectiveSamplingSeed,
}

impl<const I: usize, const S: usize, const T: usize> TokenRequest<I, S, T> {
    pub(crate) fn try_new(
        selector: RequestSelector,
        input: &[u32],
        parameters: GenerationParameters,
        service: ServiceClass,
        max_output: TokenCount,
        stops: &[&[u32]],
        seed: EffectiveSamplingSeed,
    ) -> Result<Self, RequestError> {
        if max_output.get() == 0 {
            return Err(RequestError::MaxOutputTokens);
        }
        let input = bounded(input, RequestError::InputTokenCapacity)?;
        let mut retained_stops = BoundedVec::new();
        for &stop in stops {
            if stop.is_empty() {
                return Err(RequestError::EmptyStopTokenSequence);
            }
            retained_stops
                .try_push(bounded(stop, RequestError::StopTokenCapacity)?)
                .map_err(|_| RequestError::StopSequenceCapacity)?;
        }
        Ok(Self {
            selector,
            input,
            parameters,
            service,
            max_output,
            stops: retained_stops,
            seed,
        })
    }

    #[rustfmt::skip]
    pub(crate) const fn selector(&self) -> RequestSelector { self.selector }
    #[rustfmt::skip]
    pub(crate) const fn input(&self) -> &BoundedVec<u32, I> { &self.input }
    #[rustfmt::skip]
    pub(crate) const fn parameters(&self) -> GenerationParameters { self.parameters }
    #[rustfmt::skip]
    pub(crate) const fn service(&self) -> ServiceClass { self.service }
    #[rustfmt::skip]
    pub(crate) const fn max_output(&self) -> TokenCount { self.max_output }
    #[rustfmt::skip]
    pub(crate) const fn stops(&self) -> &BoundedVec<BoundedVec<u32, T>, S> { &self.stops }
    #[rustfmt::skip]
    pub(crate) const fn seed(&self) -> EffectiveSamplingSeed { self.seed }
    #[rustfmt::skip]
    pub(crate) fn exactly_matches(&self, other: &Self, work: &mut WorkMeter) -> Result<bool, WorkBudgetError> { work.record(InvariantChecks, 7)?; if (self.selector, self.parameters, self.service, self.max_output, self.seed, self.input.len(), self.stops.len()) != (other.selector, other.parameters, other.service, other.max_output, other.seed, other.input.len(), other.stops.len()) { return Ok(false); } for (left, right) in self.input.iter().zip(other.input.iter()) { work.record(VisitedEntities, 1)?; work.record(InvariantChecks, 1)?; if left != right { return Ok(false); } } for (left, right) in self.stops.iter().zip(other.stops.iter()) { work.record(VisitedEntities, 1)?; work.record(InvariantChecks, 1)?; if left.len() != right.len() { return Ok(false); } for (left, right) in left.iter().zip(right.iter()) { work.record(VisitedEntities, 1)?; work.record(InvariantChecks, 1)?; if left != right { return Ok(false); } } } Ok(true) }
}

#[rustfmt::skip]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RequestLifecycle { Preparing, Warming }
#[rustfmt::skip] #[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CapabilityRequirement { pub(crate) phase: ExecutionPhase, pub(crate) batch: BatchBucket, pub(crate) shape: u16, pub(crate) route: [u8; 32], pub(crate) adapter_build: [u8; 32], pub(crate) mlx_build: [u8; 32], pub(crate) backend_interface: u32 }
#[rustfmt::skip] #[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RequestDescriptionFacts { pub(crate) requirements: BoundedVec<CapabilityRequirement, REQUEST_REQUIREMENT_LIMIT>, pub(crate) backend_capabilities: [u8; 32], pub(crate) ordinary_estimate: Duration, pub(crate) conservative_time: Duration, pub(crate) resource_bytes: ByteCount, pub(crate) output_bytes: ByteCount, pub(crate) residency_bytes: ByteCount }
#[rustfmt::skip]
impl RequestDescriptionFacts { pub(crate) fn valid(&self, work: &mut WorkMeter) -> Result<bool, WorkBudgetError> { work.record(InvariantChecks, 7)?; if self.requirements.is_empty() || self.backend_capabilities == [0; 32] || self.ordinary_estimate.as_micros() == 0 || self.conservative_time < self.ordinary_estimate || [self.resource_bytes, self.output_bytes, self.residency_bytes].contains(&ByteCount::new(0)) { return Ok(false); } for requirement in self.requirements.iter() { work.record(VisitedEntities, 1)?; work.record(InvariantChecks, 5)?; if requirement.shape == 0 || requirement.route == [0; 32] || requirement.adapter_build == [0; 32] || requirement.mlx_build == [0; 32] || requirement.backend_interface == 0 { return Ok(false); } } Ok(true) } }
#[rustfmt::skip] #[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DescriptionState { Missing, InFlight(BackendGeneration), Ready(BackendGeneration) }
#[rustfmt::skip] #[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DescriptionRefreshScope { Observation, Loaded(ModelRevisionId) }
#[rustfmt::skip]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct AcceptanceInput<const I: usize, const S: usize, const T: usize> { pub(crate) connection: ConnectionId, pub(crate) request: TokenRequest<I, S, T>, pub(crate) accepted_at: MonotonicTime, pub(crate) preparation_timeout: Duration }
#[rustfmt::skip]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AcceptedRequest<const I: usize, const S: usize, const T: usize> { id: RequestId, revision: RequestRevision, request: TokenRequest<I, S, T>, status: RequestStatusVersion, lifecycle: RequestLifecycle, deadline: MonotonicTime, description: DescriptionState }
#[rustfmt::skip]
impl<const I: usize, const S: usize, const T: usize> AcceptedRequest<I, S, T> {
    pub(crate) const fn id(&self) -> RequestId { self.id }
    pub(crate) const fn revision_fact(&self) -> RequestRevision { self.revision }
    pub(crate) const fn request(&self) -> &TokenRequest<I, S, T> { &self.request }
    pub(crate) const fn status(&self) -> RequestStatusVersion { self.status }
    pub(crate) const fn lifecycle(&self) -> RequestLifecycle { self.lifecycle }
    pub(crate) const fn deadline(&self) -> MonotonicTime { self.deadline }
    pub(crate) const fn description(&self) -> DescriptionState { self.description }
}
#[rustfmt::skip]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Cursor { connection: ConnectionId, last: RequestSequence }
#[rustfmt::skip]
#[derive(Debug)]
pub(crate) struct RequestChange<const I: usize, const S: usize, const T: usize> { expected: RequestBookGeneration, before: usize, cursor_slot: usize, cursor_before: Option<Cursor>, accepted: AcceptedRequest<I, S, T> }
#[rustfmt::skip]
impl<const I: usize, const S: usize, const T: usize> RequestChange<I, S, T> {
    pub(crate) const fn accepted(&self) -> &AcceptedRequest<I, S, T> { &self.accepted }
    pub(crate) const fn revision(&self) -> RequestRevision { self.accepted.revision }
}
#[rustfmt::skip] #[derive(Debug)] #[allow(clippy::type_complexity, reason = "the prepared change retains one bounded warming-slot transition")]
pub(crate) struct DescriptionChange<'a> { expected: RequestBookGeneration, index: usize, before_lifecycle: RequestLifecycle, before: DescriptionState, after_lifecycle: RequestLifecycle, after: DescriptionState, before_facts: bool, after_facts: Option<&'a RequestDescriptionFacts>, described_before: u16, described_after: u16, warming: Option<(usize, Option<(ModelRevisionId, u16)>, Option<(ModelRevisionId, u16)>)> }
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RequestBook<const R: usize, const I: usize, const S: usize, const T: usize> {
    daemon: DaemonInstanceId,
    generation: RequestBookGeneration,
    requests: Vec<AcceptedRequest<I, S, T>>,
    description_facts: Vec<Option<RequestDescriptionFacts>>,
    connections: [Option<Cursor>; CONNECTION_LIMIT],
    described: u16,
    warming: [Option<(ModelRevisionId, u16)>; MODEL_REGISTRY_LIMIT],
}
impl<const R: usize, const I: usize, const S: usize, const T: usize> RequestBook<R, I, S, T> {
    #[cfg(test)]
    pub(crate) fn try_new(
        daemon: DaemonInstanceId,
        generation: RequestBookGeneration,
    ) -> RequestResult<Self> {
        Self::try_new_with_limits(daemon, generation)
    }
    fn try_new_with_limits(
        daemon: DaemonInstanceId,
        generation: RequestBookGeneration,
    ) -> RequestResult<Self> {
        if R == 0 || R > REQUEST_LIMIT {
            return Err(RequestError::RequestCapacity);
        }
        let mut requests = Vec::new();
        requests
            .try_reserve_exact(R)
            .map_err(|_| RequestError::Allocation)?;
        let mut description_facts = Vec::new();
        description_facts
            .try_reserve_exact(R)
            .map_err(|_| RequestError::Allocation)?;
        Ok(Self {
            daemon,
            generation,
            requests,
            description_facts,
            connections: [None; CONNECTION_LIMIT],
            described: 0,
            warming: [None; MODEL_REGISTRY_LIMIT],
        })
    }
    #[rustfmt::skip]
    pub(crate) const fn generation(&self) -> RequestBookGeneration { self.generation }
    #[rustfmt::skip]
    pub(crate) fn len(&self) -> usize { self.requests.len() }
    #[cfg(test)]
    #[rustfmt::skip]
    pub(crate) fn force_cursor(&mut self, connection: ConnectionId, last: RequestSequence) { self.connections[0] = Some(Cursor { connection, last }); }
    pub(crate) fn get(
        &self,
        id: RequestId,
        work: &mut WorkMeter,
    ) -> RequestResult<Option<&AcceptedRequest<I, S, T>>> {
        if id.daemon_instance() != self.daemon {
            return Ok(None);
        }
        Ok(self.find(id, work)?.map(|index| &self.requests[index]))
    }
    #[rustfmt::skip]
    pub(crate) fn prepare(&self, expected: RequestBookGeneration, registry: RegistryGeneration, input: AcceptanceInput<I, S, T>, revision: RequestRevision, work: &mut WorkMeter) -> RequestResult<RequestChange<I, S, T>> {
        require(work, expected == self.generation, RequestError::PreparedChangeStale)?;
        work.record(InvariantChecks, 1)?;
        expected.next()?;
        require(work, registry == revision.generation(), RequestError::RegistryGeneration)?;
        let selector = match (input.request.selector(), revision.selection()) {
            (RequestSelector::Direct(selected), RevisionSelection::Direct(resolved)) => selected == resolved,
            (RequestSelector::Alias(selected), RevisionSelection::Alias(resolved)) => selected == resolved,
            _ => false,
        };
        require(work, selector, RequestError::SelectorMismatch)?;
        require(work, revision.lifecycle() == RevisionLifecycle::Available, RequestError::RevisionUnavailable)?;
        let top_k = input.request.parameters().top_k();
        require(work, top_k == 0 || top_k < revision.vocabulary(), RequestError::TopK)?;
        work.record(InvariantChecks, 1)?;
        let input_tokens = u64::try_from(input.request.input().len()).map_err(|_| RequestError::ContextLimit)?;
        let total = TokenCount::new(input_tokens).checked_add(input.request.max_output()).map_err(|_| RequestError::ContextLimit)?;
        require(work, total <= revision.context_limit(), RequestError::ContextLimit)?;
        require(work, input.preparation_timeout.as_micros() != 0, RequestError::PreparationTimeout)?;
        work.record(InvariantChecks, 1)?;
        let deadline = input.accepted_at.checked_add(input.preparation_timeout).map_err(|_| RequestError::PreparationTimeout)?;
        require(work, self.requests.len() < R, RequestError::RequestCapacity)?;
        let (cursor_slot, cursor_before, sequence) = self.prepare_cursor(input.connection, work)?;
        let id = RequestId::new(self.daemon, input.connection, sequence);
        let unused = self.find(id, work)?.is_none();
        require(work, unused, RequestError::Continuity)?;
        let accepted = AcceptedRequest { id, revision, request: input.request, status: RequestStatusVersion::new(1).expect("one is nonzero"), lifecycle: RequestLifecycle::Preparing, deadline, description: DescriptionState::Missing };
        let copied = size_of::<RequestChange<I, S, T>>() as u64;
        work.ensure(crate::HotPathWorkWitness::new([0, copied, 0, 0, 1]))?;
        work.record(CopiedBytes, copied)?;
        work.record(InvariantChecks, 1)?;
        Ok(RequestChange { expected, before: self.requests.len(), cursor_slot, cursor_before, accepted })
    }
    pub(crate) fn validate(&self, change: &RequestChange<I, S, T>) -> RequestResult<()> {
        (self.generation == change.expected
            && self.requests.len() == change.before
            && self.connections.get(change.cursor_slot).copied() == Some(change.cursor_before))
        .then_some(())
        .ok_or(RequestError::PreparedChangeStale)
    }
    pub(crate) fn commit(
        &mut self,
        change: RequestChange<I, S, T>,
    ) -> RequestResult<RequestBookGeneration> {
        self.validate(&change)?;
        let next = change.expected.next()?;
        self.connections[change.cursor_slot] = Some(Cursor {
            connection: change.accepted.id.connection(),
            last: change.accepted.id.sequence(),
        });
        self.requests.push(change.accepted);
        self.description_facts.push(None);
        self.generation = next;
        Ok(self.generation)
    }
    #[rustfmt::skip]
    pub(crate) fn prepare_description(&self, expected: RequestBookGeneration, id: RequestId, target: BackendGeneration, at: MonotonicTime, work: &mut WorkMeter) -> RequestResult<DescriptionChange<'static>> { require(work, expected == self.generation, RequestError::PreparedChangeStale)?; expected.next()?; let index = self.find(id, work)?.ok_or(RequestError::UnknownRequest)?; let request = &self.requests[index]; require(work, at < request.deadline, RequestError::PreparationTimeout)?; require(work, request.lifecycle == RequestLifecycle::Preparing && request.description == DescriptionState::Missing, RequestError::DescriptionState)?; self.description_change(expected, index, request.lifecycle, DescriptionState::InFlight(target), None, work) }
    #[rustfmt::skip]
    pub(crate) fn prepare_warming(&self, expected: RequestBookGeneration, id: RequestId, work: &mut WorkMeter) -> RequestResult<DescriptionChange<'static>> { require(work, expected == self.generation, RequestError::PreparedChangeStale)?; expected.next()?; let index = self.find(id, work)?.ok_or(RequestError::UnknownRequest)?; let request = &self.requests[index]; require(work, request.lifecycle == RequestLifecycle::Preparing && request.description == DescriptionState::Missing, RequestError::DescriptionState)?; self.description_change(expected, index, RequestLifecycle::Warming, DescriptionState::Missing, None, work) }
    #[rustfmt::skip]
    pub(crate) fn prepare_description_result<'a>(&self, expected: RequestBookGeneration, id: RequestId, target: BackendGeneration, facts: &'a RequestDescriptionFacts, work: &mut WorkMeter) -> RequestResult<DescriptionChange<'a>> { require(work, expected == self.generation, RequestError::PreparedChangeStale)?; expected.next()?; let index = self.find(id, work)?.ok_or(RequestError::UnknownRequest)?; let request = &self.requests[index]; require(work, request.description == DescriptionState::InFlight(target), RequestError::DescriptionState)?; self.description_change(expected, index, RequestLifecycle::Preparing, DescriptionState::Ready(target), Some(facts), work) }
    #[rustfmt::skip]
    pub(crate) fn refresh_count(&self, scope: DescriptionRefreshScope, work: &mut WorkMeter) -> RequestResult<usize> { let warming = match scope { DescriptionRefreshScope::Observation => 0, DescriptionRefreshScope::Loaded(revision) => { let mut count = 0; for entry in &self.warming { work.record(VisitedEntities, 1)?; if let Some((candidate, value)) = entry && *candidate == revision { count = usize::from(*value); break; } } count } }; Ok(usize::from(self.described) + warming) }
    #[rustfmt::skip]
    pub(crate) fn prepare_refresh(&self, expected: RequestBookGeneration, scope: DescriptionRefreshScope, target: BackendGeneration, after: Option<RequestId>, at: MonotonicTime, work: &mut WorkMeter) -> RequestResult<(RequestId, Option<DescriptionChange<'static>>)> {
        require(work, expected == self.generation, RequestError::PreparedChangeStale)?; expected.next()?; let mut selected = None; for (index, request) in self.requests.iter().enumerate() { work.record(VisitedEntities, 1)?; if description_candidate(request, scope, target) && after.is_none_or(|cursor| request.id > cursor) && selected.is_none_or(|prior: usize| request.id < self.requests[prior].id) { selected = Some(index); } } let index = selected.ok_or(RequestError::DescriptionState)?; let request = &self.requests[index]; work.record(InvariantChecks, 1)?; if at >= request.deadline { return Ok((request.id, None)); } Ok((request.id, Some(self.description_change(expected, index, RequestLifecycle::Preparing, DescriptionState::InFlight(target), None, work)?)))
    }
    #[rustfmt::skip]
    pub(crate) fn validate_description(&self, change: &DescriptionChange<'_>) -> RequestResult<()> { let warming = change.warming.is_none_or(|(slot, before, _)| self.warming.get(slot) == Some(&before)); (self.generation == change.expected && self.described == change.described_before && warming && self.description_facts.get(change.index).is_some_and(|facts| facts.is_some() == change.before_facts) && self.requests.get(change.index).is_some_and(|request| request.lifecycle == change.before_lifecycle && request.description == change.before)).then_some(()).ok_or(RequestError::PreparedChangeStale) }
    #[rustfmt::skip]
    pub(crate) fn description_request(&self, change: &DescriptionChange<'_>) -> RequestResult<&AcceptedRequest<I, S, T>> { self.validate_description(change)?; Ok(&self.requests[change.index]) }
    #[rustfmt::skip]
    pub(crate) fn description_facts(&self, id: RequestId, work: &mut WorkMeter) -> RequestResult<Option<&RequestDescriptionFacts>> { Ok(match self.find(id, work)? { Some(index) => self.description_facts[index].as_ref(), None => None }) }
    #[rustfmt::skip]
    pub(crate) fn commit_description(&mut self, change: DescriptionChange<'_>) -> RequestResult<RequestBookGeneration> { self.validate_description(&change)?; self.requests[change.index].lifecycle = change.after_lifecycle; self.requests[change.index].description = change.after; self.description_facts[change.index] = change.after_facts.copied(); self.described = change.described_after; if let Some((slot, _, after)) = change.warming { self.warming[slot] = after; } self.generation = change.expected.next()?; Ok(self.generation) }
    #[rustfmt::skip]
    fn description_change<'a>(&self, expected: RequestBookGeneration, index: usize, after_lifecycle: RequestLifecycle, after: DescriptionState, after_facts: Option<&'a RequestDescriptionFacts>, work: &mut WorkMeter) -> RequestResult<DescriptionChange<'a>> { let request = &self.requests[index]; let was_described = request.lifecycle == RequestLifecycle::Preparing && request.description != DescriptionState::Missing; let is_described = after_lifecycle == RequestLifecycle::Preparing && after != DescriptionState::Missing; let described_after = match (was_described, is_described) { (false, true) => self.described.checked_add(1), (true, false) => self.described.checked_sub(1), _ => Some(self.described) }.ok_or(RequestError::DescriptionState)?; let entering = request.lifecycle != RequestLifecycle::Warming && after_lifecycle == RequestLifecycle::Warming; let leaving = request.lifecycle == RequestLifecycle::Warming && after_lifecycle != RequestLifecycle::Warming; let warming = if entering || leaving { let revision = request.revision.revision(); let mut slot = None; for (index, entry) in self.warming.iter().enumerate() { work.record(VisitedEntities, 1)?; if entry.is_some_and(|(candidate, _)| candidate == revision) || entering && entry.is_none() && slot.is_none() { slot = Some(index); if entry.is_some() { break; } } } let slot = slot.ok_or(RequestError::DescriptionState)?; let before = self.warming[slot]; let count = before.map_or(0, |(_, count)| count); let after_count = if entering { count.checked_add(1) } else { count.checked_sub(1) }.ok_or(RequestError::DescriptionState)?; Some((slot, before, (after_count != 0).then_some((revision, after_count)))) } else { None }; let copied = size_of::<DescriptionChange<'_>>() as u64 + after_facts.map_or(0, |_| size_of::<RequestDescriptionFacts>() as u64); work.ensure(crate::HotPathWorkWitness::new([0, copied, 0, 0, 1]))?; work.record(CopiedBytes, copied)?; work.record(InvariantChecks, 1)?; Ok(DescriptionChange { expected, index, before_lifecycle: request.lifecycle, before: request.description, after_lifecycle, after, before_facts: self.description_facts[index].is_some(), after_facts, described_before: self.described, described_after, warming }) }
    fn prepare_cursor(
        &self,
        connection: ConnectionId,
        work: &mut WorkMeter,
    ) -> RequestResult<(usize, Option<Cursor>, RequestSequence)> {
        for (slot, entry) in self.connections.iter().enumerate() {
            work.record(VisitedEntities, 1)?;
            let Some(cursor) = entry else {
                return Ok((slot, None, RequestSequence::new(1).expect("one is nonzero")));
            };
            if cursor.connection == connection {
                work.record(InvariantChecks, 1)?;
                let next = cursor
                    .last
                    .next()
                    .map_err(|_| RequestError::RequestIdExhausted)?;
                let prior = RequestId::new(self.daemon, connection, cursor.last);
                let Some(request_index) = self.find(prior, work)? else {
                    return Err(RequestError::Continuity);
                };
                work.record(InvariantChecks, 1)?;
                if self
                    .requests
                    .get(request_index)
                    .is_none_or(|request| request.id != prior)
                {
                    return Err(RequestError::Continuity);
                }
                return Ok((slot, Some(*cursor), next));
            }
        }
        Err(RequestError::ConnectionCapacity)
    }
    fn find(&self, id: RequestId, work: &mut WorkMeter) -> RequestResult<Option<usize>> {
        for (index, request) in self.requests.iter().enumerate() {
            work.record(VisitedEntities, 1)?;
            if request.id == id {
                return Ok(Some(index));
            }
        }
        Ok(None)
    }
}
#[rustfmt::skip]
fn description_candidate<const I: usize, const S: usize, const T: usize>(request: &AcceptedRequest<I, S, T>, scope: DescriptionRefreshScope, target: BackendGeneration) -> bool { let stale = match request.description { DescriptionState::Missing => true, DescriptionState::InFlight(generation) | DescriptionState::Ready(generation) => generation != target }; stale && match (request.lifecycle, scope) { (RequestLifecycle::Preparing, _) => request.description != DescriptionState::Missing, (RequestLifecycle::Warming, DescriptionRefreshScope::Loaded(revision)) => request.revision.revision() == revision, (RequestLifecycle::Warming, DescriptionRefreshScope::Observation) => false } }
#[cfg(not(test))]
impl<const I: usize, const S: usize, const T: usize> RequestBook<REQUEST_LIMIT, I, S, T> {
    #[rustfmt::skip]
    pub(crate) fn try_new(daemon: DaemonInstanceId, generation: RequestBookGeneration) -> RequestResult<Self> { Self::try_new_with_limits(daemon, generation) }
}

#[rustfmt::skip]
fn require(work: &mut WorkMeter, valid: bool, error: RequestError) -> RequestResult<()> { work.record(InvariantChecks, 1)?; valid.then_some(()).ok_or(error) }

#[rustfmt::skip]
fn bounded<const N: usize>(values: &[u32], error: RequestError) -> Result<BoundedVec<u32, N>, RequestError> {
    let mut bounded = BoundedVec::new();
    for &value in values {
        bounded.try_push(value).map_err(|_| error)?;
    }
    Ok(bounded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model_descriptor::{ModelDescriptorHash, RawModelDescriptor, verify};
    use crate::model_registry::{
        ModelAliasId, ModelManifestId, ModelRegistry, ModelRevisionId, RegistrationIntent,
        RegistryCommand, RegistryGeneration,
    };
    use crate::{
        ConnectionId, Duration, HotPathWorkBudget, HotPathWorkWitness, ModelId, MonotonicTime,
        ServiceClass, TokenCount, WorkMeter,
    };

    const FRAME: [u8; 13] = [0, 0, 0, 1, 0, 0, 0, 7, 0, 0, 0, 1, b'x'];
    #[rustfmt::skip]
    const ID: [u8; 32] = [0xc9, 0x1c, 0x14, 0x09, 0x1c, 0xea, 0x08, 0xf4, 0x58, 0xa4, 0xe2, 0x75, 0x96, 0xc1, 0x5b, 0x2c, 0xf0, 0xc8, 0x74, 0x34, 0x2d, 0x30, 0x3e, 0xad, 0xe8, 0x9f, 0x29, 0x0e, 0xd0, 0x13, 0x38, 0x21];
    #[rustfmt::skip]
    const HASH: [u8; 32] = [0xe2, 0x24, 0x6d, 0x47, 0x7f, 0x70, 0xd3, 0xe6, 0x58, 0x8b, 0xb5, 0x45, 0xe2, 0x14, 0xc0, 0xbb, 0xa1, 0x76, 0x6e, 0xf3, 0x39, 0x7a, 0x50, 0x71, 0x89, 0x29, 0xc9, 0x4f, 0xe9, 0x62, 0x1e, 0x9b];

    #[rustfmt::skip]
    fn greedy() -> GenerationParameters { GenerationParameters::try_new(SamplingMode::Greedy, 0.0f32.to_bits(), 1.0f32.to_bits(), 0).unwrap() }
    #[rustfmt::skip]
    fn meter() -> WorkMeter { WorkMeter::new(HotPathWorkBudget::binary_maximum()) }
    #[rustfmt::skip]
    fn categorical(top_k: u32) -> GenerationParameters { GenerationParameters::try_new(SamplingMode::Categorical, 1.0f32.to_bits(), 0.9f32.to_bits(), top_k).unwrap() }
    #[rustfmt::skip]
    fn revision_fact(selection: RevisionSelection, lifecycle: RevisionLifecycle) -> crate::model_registry::RequestRevision {
        let expected = ModelDescriptorHash::from_manifest(1, HASH).unwrap(); let descriptor = verify(RawModelDescriptor { frame: &FRAME, id: ID, hash_schema_version: 1, hash: HASH, vocabulary: 7 }, expected, &mut meter()).unwrap(); let revision = ModelRevisionId::new([1; 32]).unwrap(); let intent = RegistrationIntent { model: ModelId::new(1).unwrap(), revision, manifest: ModelManifestId::new([2; 32]).unwrap(), expected_descriptor_hash: expected, context_limit: TokenCount::new(8) }; let mut registry = ModelRegistry::<2, 1, 26>::try_new(RegistryGeneration::new(1).unwrap()).unwrap(); let plan = registry.prepare_description(registry.generation(), intent, &mut meter()).unwrap(); let change = registry.prepare_registration(plan, &descriptor, &mut meter()).unwrap(); registry.commit(change).unwrap();
        if let RevisionSelection::Alias(alias) = selection { let change = registry.prepare(registry.generation(), RegistryCommand::BindAlias(alias, revision), &mut meter()).unwrap(); registry.commit(change).unwrap(); }
        let command = match lifecycle { RevisionLifecycle::Available => None, RevisionLifecycle::Retiring => Some(RegistryCommand::Retire(revision)), RevisionLifecycle::Unavailable => Some(RegistryCommand::MarkUnavailable(revision)) }; if let Some(command) = command { let change = registry.prepare(registry.generation(), command, &mut meter()).unwrap(); registry.commit(change).unwrap(); }
        registry.request_revision_fact(registry.generation(), selection, &mut meter()).unwrap().unwrap()
    }
    #[rustfmt::skip]
    fn acceptance(selector: RequestSelector, input: &[u32], output: u64, top_k: u32, connection: u128, at: u64, timeout: u64) -> AcceptanceInput<2, 1, 2> { AcceptanceInput { connection: ConnectionId::new(connection).unwrap(), request: TokenRequest::try_new(selector, input, categorical(top_k), ServiceClass::Interactive, TokenCount::new(output), &[&[3]], EffectiveSamplingSeed::new(0, SamplingSeedOrigin::Caller)).unwrap(), accepted_at: MonotonicTime::from_micros(at), preparation_timeout: Duration::from_micros(timeout) } }
    type Book<const R: usize> = RequestBook<R, 2, 1, 2>;
    #[rustfmt::skip]
    fn book<const R: usize>() -> Book<R> { Book::try_new(DaemonInstanceId::new(1).unwrap(), RequestBookGeneration::new(1).unwrap()).unwrap() }
    #[rustfmt::skip]
    #[allow(clippy::type_complexity, reason = "the rollback witness captures the complete bounded state")] fn snapshot<const R: usize>(book: &Book<R>) -> (RequestBookGeneration, Vec<AcceptedRequest<2, 1, 2>>, Vec<Option<RequestDescriptionFacts>>, [Option<Cursor>; CONNECTION_LIMIT], u16, [Option<(ModelRevisionId, u16)>; MODEL_REGISTRY_LIMIT]) { (book.generation, book.requests.clone(), book.description_facts.clone(), book.connections, book.described, book.warming) }
    #[rustfmt::skip]
    fn rejected<const R: usize>(book: &Book<R>, expected: RequestBookGeneration, registry: RegistryGeneration, input: AcceptanceInput<2, 1, 2>, fact: crate::model_registry::RequestRevision, mut work: WorkMeter) -> (RequestError, HotPathWorkWitness) { let before = snapshot(book); let error = book.prepare(expected, registry, input, fact, &mut work).unwrap_err(); assert_eq!(snapshot(book), before); (error, work.witness()) }

    #[test]
    #[rustfmt::skip]
    fn selectors_and_values_are_closed_and_retained() {
        let revision = ModelRevisionId::new([1; 32]).unwrap();
        let direct = TokenRequest::<2, 1, 2>::try_new(RequestSelector::Direct(revision), &[11, 12], greedy(), ServiceClass::Interactive, TokenCount::new(3), &[], EffectiveSamplingSeed::new(0, SamplingSeedOrigin::Caller)).unwrap();
        assert_eq!((direct.selector(), direct.input().len(), direct.seed()), (RequestSelector::Direct(revision), 2, EffectiveSamplingSeed::new(0, SamplingSeedOrigin::Caller)));
        let alias = ModelAliasId::new([2; 32]).unwrap();
        let parameters = GenerationParameters::try_new(SamplingMode::Categorical, 0.5f32.to_bits(), 0.9f32.to_bits(), 7).unwrap();
        let request = TokenRequest::<3, 2, 3>::try_new(RequestSelector::Alias(alias), &[1, 2, 3], parameters, ServiceClass::Background, TokenCount::new(4), &[&[5], &[6, 7]], EffectiveSamplingSeed::new(u64::MAX, SamplingSeedOrigin::Daemon)).unwrap();
        assert_eq!((request.selector(), request.parameters(), request.service(), request.max_output().get(), request.seed()), (RequestSelector::Alias(alias), parameters, ServiceClass::Background, 4, EffectiveSamplingSeed::new(u64::MAX, SamplingSeedOrigin::Daemon)));
        assert_eq!(request.stops().get(1).unwrap().iter().copied().collect::<Vec<_>>(), vec![6, 7]);
    }

    #[test]
    #[rustfmt::skip]
    fn generation_parameters_enforce_exact_binary32_domains() {
        let minimum = GenerationParameters::try_new(SamplingMode::Categorical, 1, 1, u32::MAX).unwrap();
        assert_eq!((minimum.mode(), minimum.temperature_bits(), minimum.top_p_bits(), minimum.top_k()), (SamplingMode::Categorical, 1, 1, u32::MAX));
        for (mode, temperature, top_p, top_k) in [
            (SamplingMode::Greedy, (-0.0f32).to_bits(), 1.0f32.to_bits(), 0),
            (SamplingMode::Greedy, 0.0f32.to_bits(), 0.5f32.to_bits(), 0),
            (SamplingMode::Greedy, 0.0f32.to_bits(), 1.0f32.to_bits(), 1),
            (SamplingMode::Categorical, 0.0f32.to_bits(), 1.0f32.to_bits(), 0),
            (SamplingMode::Categorical, (-0.0f32).to_bits(), 1.0f32.to_bits(), 0),
            (SamplingMode::Categorical, f32::NAN.to_bits(), 1.0f32.to_bits(), 0),
            (SamplingMode::Categorical, f32::INFINITY.to_bits(), 1.0f32.to_bits(), 0),
            (SamplingMode::Categorical, f32::NEG_INFINITY.to_bits(), 1.0f32.to_bits(), 0),
            (SamplingMode::Categorical, 2.0f32.to_bits() + 1, 1.0f32.to_bits(), 0),
            (SamplingMode::Categorical, 1.0f32.to_bits(), (-0.0f32).to_bits(), 0),
            (SamplingMode::Categorical, 1.0f32.to_bits(), f32::NAN.to_bits(), 0),
            (SamplingMode::Categorical, 1.0f32.to_bits(), f32::INFINITY.to_bits(), 0),
            (SamplingMode::Categorical, 1.0f32.to_bits(), f32::NEG_INFINITY.to_bits(), 0),
            (SamplingMode::Categorical, 1.0f32.to_bits(), 1.0f32.to_bits() + 1, 0),
        ] {
            assert_eq!(GenerationParameters::try_new(mode, temperature, top_p, top_k), Err(RequestError::GenerationParameters));
        }
        GenerationParameters::try_new(SamplingMode::Categorical, 2.0f32.to_bits(), 1.0f32.to_bits(), 0).unwrap();
    }

    #[rustfmt::skip]
    fn bounded_request(input: &[u32], output: u64, stops: &[&[u32]]) -> Result<TokenRequest<2, 2, 2>, RequestError> { TokenRequest::try_new(RequestSelector::Direct(ModelRevisionId::new([3; 32]).unwrap()), input, greedy(), ServiceClass::Standard, TokenCount::new(output), stops, EffectiveSamplingSeed::new(u64::MAX, SamplingSeedOrigin::Caller)) }

    #[test]
    #[rustfmt::skip]
    fn token_and_stop_bounds_reject_without_truncation() {
        let exact = bounded_request(&[1, 2], 1, &[&[3, 4], &[5]]).unwrap();
        assert_eq!((exact.input().len(), exact.stops().len(), exact.seed().value()), (2, 2, u64::MAX));
        assert!(bounded_request(&[], 1, &[]).is_ok());
        assert_eq!(bounded_request(&[1, 2, 3], 1, &[]), Err(RequestError::InputTokenCapacity));
        assert_eq!(bounded_request(&[], 0, &[]), Err(RequestError::MaxOutputTokens));
        assert_eq!(bounded_request(&[], 1, &[&[1], &[2], &[3]]), Err(RequestError::StopSequenceCapacity));
        assert_eq!(bounded_request(&[], 1, &[&[]]), Err(RequestError::EmptyStopTokenSequence));
        assert_eq!(bounded_request(&[], 1, &[&[1, 2, 3]]), Err(RequestError::StopTokenCapacity));
    }

    #[test]
    #[rustfmt::skip]
    fn description_counts_do_not_scan_requests() { let revision = ModelRevisionId::new([1; 32]).unwrap(); let mut small = book::<1>(); small.described = 1; small.warming[0] = Some((revision, 1)); let mut large = book::<1>(); large.described = u16::MAX; large.warming[0] = Some((revision, u16::MAX)); for (scope, counts) in [(DescriptionRefreshScope::Observation, (1, usize::from(u16::MAX))), (DescriptionRefreshScope::Loaded(revision), (2, 2 * usize::from(u16::MAX)))] { let mut small_work = meter(); let mut large_work = meter(); assert_eq!((small.refresh_count(scope, &mut small_work).unwrap(), large.refresh_count(scope, &mut large_work).unwrap(), small_work.witness()), (counts.0, counts.1, large_work.witness())); } }
    #[test]
    #[rustfmt::skip]
    fn acceptance_state_is_prepared_before_commit() {
        let mut requests = book::<2>();
        let before = (requests.generation(), requests.len());
        let revision = revision_fact(RevisionSelection::Direct(ModelRevisionId::new([1; 32]).unwrap()), RevisionLifecycle::Available); let mut work = meter(); let change = requests.prepare(requests.generation(), revision.generation(), acceptance(RequestSelector::Direct(ModelRevisionId::new([1; 32]).unwrap()), &[1, 2], 6, 6, 2, 10, 5), revision, &mut work).unwrap();
        assert_eq!((requests.generation(), requests.len()), before);
        let id = change.accepted().id();
        assert_eq!((id.daemon_instance(), id.connection(), id.sequence().get()), (DaemonInstanceId::new(1).unwrap(), ConnectionId::new(2).unwrap(), 1));
        requests.validate(&change).unwrap(); requests.commit(change).unwrap();
        let accepted = requests.get(id, &mut meter()).unwrap().unwrap();
        assert_eq!((accepted.id(), accepted.revision_fact().revision(), accepted.status().get(), accepted.lifecycle(), accepted.deadline().as_micros()), (id, ModelRevisionId::new([1; 32]).unwrap(), 1, RequestLifecycle::Preparing, 15));
        assert_eq!(work.witness(), HotPathWorkWitness::new([1, 512, 0, 0, 13]));
        let alias = ModelAliasId::new([3; 32]).unwrap(); let fact = revision_fact(RevisionSelection::Alias(alias), RevisionLifecycle::Available); let mut alias_book = book::<1>(); let mut alias_work = meter(); let change = alias_book.prepare(alias_book.generation(), fact.generation(), acceptance(RequestSelector::Alias(alias), &[], 8, 0, 3, 20, 1), fact, &mut alias_work).unwrap(); alias_book.commit(change).unwrap(); assert_eq!((alias_book.requests[0].revision_fact().selection(), alias_work.witness()), (RevisionSelection::Alias(alias), HotPathWorkWitness::new([1, 512, 0, 0, 13])));
    }

    #[test]
    #[rustfmt::skip]
    fn domain_rejections_preserve_exact_state_and_work() {
        let revision = ModelRevisionId::new([1; 32]).unwrap(); let alias = ModelAliasId::new([3; 32]).unwrap(); let selector = RequestSelector::Direct(revision); let direct = revision_fact(RevisionSelection::Direct(revision), RevisionLifecycle::Available); let book = book::<2>(); let g = book.generation();
        let case = |input: AcceptanceInput<2, 1, 2>, fact: crate::model_registry::RequestRevision| rejected(&book, g, fact.generation(), input, fact, meter());
        assert_eq!(rejected(&book, RequestBookGeneration::new(2).unwrap(), direct.generation(), acceptance(selector, &[], 1, 0, 2, 1, 1), direct, meter()), (RequestError::PreparedChangeStale, HotPathWorkWitness::new([0, 0, 0, 0, 1])));
        assert_eq!(rejected(&book, g, RegistryGeneration::new(1).unwrap(), acceptance(selector, &[], 1, 0, 2, 1, 1), direct, meter()), (RequestError::RegistryGeneration, HotPathWorkWitness::new([0, 0, 0, 0, 3])));
        let alias_fact = revision_fact(RevisionSelection::Alias(alias), RevisionLifecycle::Available); for (selected, fact) in [(RequestSelector::Alias(alias), direct), (RequestSelector::Direct(ModelRevisionId::new([2; 32]).unwrap()), direct), (RequestSelector::Alias(ModelAliasId::new([4; 32]).unwrap()), alias_fact)] { assert_eq!(case(acceptance(selected, &[], 1, 0, 2, 1, 1), fact), (RequestError::SelectorMismatch, HotPathWorkWitness::new([0, 0, 0, 0, 4]))); }
        for selection in [RevisionSelection::Direct(revision), RevisionSelection::Alias(alias)] { let selected = match selection { RevisionSelection::Direct(value) => RequestSelector::Direct(value), RevisionSelection::Alias(value) => RequestSelector::Alias(value) }; for lifecycle in [RevisionLifecycle::Retiring, RevisionLifecycle::Unavailable] { let fact = revision_fact(selection, lifecycle); assert_eq!(case(acceptance(selected, &[], 1, 0, 2, 1, 1), fact), (RequestError::RevisionUnavailable, HotPathWorkWitness::new([0, 0, 0, 0, 5]))); } }
        assert_eq!(case(acceptance(selector, &[], 1, 7, 2, 1, 1), direct), (RequestError::TopK, HotPathWorkWitness::new([0, 0, 0, 0, 6])));
        assert_eq!(case(acceptance(selector, &[1, 2], 7, 0, 2, 1, 1), direct), (RequestError::ContextLimit, HotPathWorkWitness::new([0, 0, 0, 0, 8])));
        assert_eq!(case(acceptance(selector, &[1, 2], u64::MAX, 0, 2, 1, 1), direct), (RequestError::ContextLimit, HotPathWorkWitness::new([0, 0, 0, 0, 7])));
        assert_eq!(case(acceptance(selector, &[], 1, 0, 2, 1, 0), direct), (RequestError::PreparationTimeout, HotPathWorkWitness::new([0, 0, 0, 0, 9])));
        assert_eq!(case(acceptance(selector, &[], 1, 0, 2, u64::MAX, 1), direct), (RequestError::PreparationTimeout, HotPathWorkWitness::new([0, 0, 0, 0, 10])));
    }

    #[test]
    #[rustfmt::skip]
    fn capacities_generations_ids_and_work_are_atomic() {
        let revision = ModelRevisionId::new([1; 32]).unwrap(); let selector = RequestSelector::Direct(revision); let fact = revision_fact(RevisionSelection::Direct(revision), RevisionLifecycle::Available);
        assert_eq!(RequestBookGeneration::new(0), Err(RequestError::InvalidGeneration)); assert!(matches!(Book::<0>::try_new(DaemonInstanceId::new(1).unwrap(), RequestBookGeneration::new(1).unwrap()), Err(RequestError::RequestCapacity))); assert!(matches!(Book::<1025>::try_new(DaemonInstanceId::new(1).unwrap(), RequestBookGeneration::new(1).unwrap()), Err(RequestError::RequestCapacity)));
        let mut full = book::<1024>(); for accepted in 1..=1024 { let change = full.prepare(full.generation(), fact.generation(), acceptance(selector, &[], 1, 0, 2, accepted, 1), fact, &mut meter()).unwrap(); full.commit(change).unwrap(); } assert_eq!((full.len(), full.requests.last().unwrap().id().sequence().get()), (1024, 1024)); assert_eq!(rejected(&full, full.generation(), fact.generation(), acceptance(selector, &[], 1, 0, 2, 1025, 1), fact, meter()), (RequestError::RequestCapacity, HotPathWorkWitness::new([0, 0, 0, 0, 11])));
        let mut connections = book::<65>(); for connection in 1..=64 { let change = connections.prepare(connections.generation(), fact.generation(), acceptance(selector, &[], 1, 0, connection, 1, 1), fact, &mut meter()).unwrap(); connections.commit(change).unwrap(); } assert_eq!(rejected(&connections, connections.generation(), fact.generation(), acceptance(selector, &[], 1, 0, 65, 1, 1), fact, meter()), (RequestError::ConnectionCapacity, HotPathWorkWitness::new([64, 0, 0, 0, 11])));
        let mut exhausted = book::<2>(); exhausted.connections[0] = Some(Cursor { connection: ConnectionId::new(2).unwrap(), last: RequestSequence::new(u64::MAX).unwrap() }); assert_eq!(rejected(&exhausted, exhausted.generation(), fact.generation(), acceptance(selector, &[], 1, 0, 2, 1, 1), fact, meter()), (RequestError::RequestIdExhausted, HotPathWorkWitness::new([1, 0, 0, 0, 12])));
        let mut corrupt = book::<2>(); corrupt.connections[0] = Some(Cursor { connection: ConnectionId::new(2).unwrap(), last: RequestSequence::new(1).unwrap() }); assert_eq!(rejected(&corrupt, corrupt.generation(), fact.generation(), acceptance(selector, &[], 1, 0, 2, 1, 1), fact, meter()), (RequestError::Continuity, HotPathWorkWitness::new([1, 0, 0, 0, 12])));
        let mut overflow = book::<2>(); overflow.generation = RequestBookGeneration(u64::MAX); assert_eq!(rejected(&overflow, overflow.generation(), fact.generation(), acceptance(selector, &[], 1, 0, 2, 1, 1), fact, meter()), (RequestError::GenerationOverflow, HotPathWorkWitness::new([0, 0, 0, 0, 2])));
        let mut stale = book::<2>(); let first = stale.prepare(stale.generation(), fact.generation(), acceptance(selector, &[], 1, 0, 2, 1, 1), fact, &mut meter()).unwrap(); let old = stale.prepare(stale.generation(), fact.generation(), acceptance(selector, &[], 1, 0, 2, 1, 1), fact, &mut meter()).unwrap(); stale.commit(first).unwrap(); let before = snapshot(&stale); assert_eq!(stale.validate(&old), Err(RequestError::PreparedChangeStale)); assert_eq!(stale.commit(old), Err(RequestError::PreparedChangeStale)); assert_eq!(snapshot(&stale), before);
        let constrained = HotPathWorkBudget::try_new(HotPathWorkWitness::new([1_000_000, 0, 0, 2, 2_100])).unwrap(); assert_eq!(rejected(&book::<2>(), RequestBookGeneration::new(1).unwrap(), fact.generation(), acceptance(selector, &[], 1, 0, 2, 1, 1), fact, WorkMeter::new(constrained)), (RequestError::Work(WorkBudgetError::BudgetExceeded(CopiedBytes, 0, 512)), HotPathWorkWitness::new([1, 0, 0, 0, 12])));
    }
}
