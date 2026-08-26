use crate::bounded::FixedWindowStart;
use crate::{
    Duration, FixedRecordArena, FixedStartCountBound, FixedStorageError, FixedWindowCounter,
    HotPathWorkWitness, MonotonicTime, PhysicalStartCreditId, SupportLedgerGeneration,
    SupportOperationObligationId, WorkBudgetError, WorkDimension, WorkMeter,
};
const POOLS: usize = 3;
const CONDITIONAL: usize = 0;
const PENDING: usize = 1;
const ACTIVE: usize = 2;
const CREDITS: usize = 3;
const CLAIMS: usize = 4;
macro_rules! values {
    ($($name:ident { $($variant:ident $(($value:ty))?),+ $(,)? })+) => {$(
        #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        #[repr(usize)]
        pub enum $name { $($variant $(($value))?),+ }
    )+};
}
values! {
    SupportOperation {
        DescribeModel, DescribeRequest, MaterializeRequest, ReleaseRequest,
        FormCandidates, ObserveTurnReceipt, SampleBackendResources,
    }
    SupportPool { Ordinary, MandatoryCompletion, SafetySampling }
    SupportFundingClaim {
        OrdinaryReservation([u8; 32]), AdmissionInitial([u8; 32]),
        EntitlementVector([u8; 32]), LifecycleReserve([u8; 32]),
    }
    SupportObligationState {
        Conditional, Pending, Active, Retained, ClosedConditional, ClosedPending,
    }
}
use SupportObligationState::*;
use SupportTransition::{BeginSupport, CloseCausalCallImpossible, FinishSupport, PredecessorEnded};
impl SupportFundingClaim {
    fn valid_for(self, pool: SupportPool) -> bool {
        let (identity, pools) = match self {
            Self::OrdinaryReservation(id) => (id, 0b001),
            Self::AdmissionInitial(id) | Self::EntitlementVector(id) => (id, 0b010),
            Self::LifecycleReserve(id) => (id, 0),
        };
        identity != [0; 32] && pools & (1 << pool as usize) != 0
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SupportCausalPredecessorId(pub [u8; 32]);
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SupportCallScopeId(pub(crate) [u8; 32]);
pub struct SupportObligationSpec<'a> {
    pub id: SupportOperationObligationId,
    pub operation: SupportOperation,
    pub pool: SupportPool,
    pub physical_credit: PhysicalStartCreditId,
    pub predecessor: SupportCausalPredecessorId,
    pub claims: &'a [SupportFundingClaim],
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct OrdinarySupportSpec {
    pub(crate) id: SupportOperationObligationId,
    pub(crate) operation: SupportOperation,
    pub(crate) physical_credit: PhysicalStartCreditId,
    pub(crate) scope: SupportCallScopeId,
    pub(crate) claim: SupportFundingClaim,
}
#[rustfmt::skip]
pub(crate) enum SupportChangeInput { BeginOrdinary(OrdinarySupportSpec, MonotonicTime), BeginPending(SupportOperationObligationId, LifecycleReserveKind, MonotonicTime), FinishActive(SupportOperationObligationId) }
#[rustfmt::skip]
enum SupportDelta { BeginOrdinary(OrdinarySupportSpec, MonotonicTime, FixedWindowStart), BeginPending(usize, Record, MonotonicTime, FixedWindowStart), FinishActive(usize, Record) }
pub(crate) struct SupportChange {
    expected: SupportLedgerGeneration,
    records: usize,
    delta: SupportDelta,
}
#[allow(dead_code, reason = "C12, G01, and G09 construct lifecycle reserves")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(usize)]
pub(crate) enum LifecycleReserveKind {
    PostLoadModelDescription,
    PostLoadRequestDescription,
    PostObservationRequestDescription,
    FirstSafetySample,
    NextSafetySample,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct LifecycleReserveSpec {
    pub(crate) id: SupportOperationObligationId,
    pub(crate) kind: LifecycleReserveKind,
    pub(crate) physical_credit: PhysicalStartCreditId,
    pub(crate) predecessor: SupportCausalPredecessorId,
    pub(crate) scope: SupportCallScopeId,
    pub(crate) claim: SupportFundingClaim,
    pub(crate) expires_at: Option<MonotonicTime>,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct LifecycleReserveMaxima(pub(crate) [u16; 5]);
#[allow(dead_code, reason = "C12, G01, and G09 report lifecycle results")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(usize)]
pub(crate) enum LifecycleTriggerResult {
    LoadSucceeded,
    LoadFailed,
    LoadCancelled,
    ObservationDescriptionsRequired,
    ObservationUnchanged,
    ObservationFailed,
    ObservationCancelled,
    QualificationActivated,
    QualificationFailed,
    QualificationCancelled,
    SampleSucceeded,
    SampleFailed,
    SampleCancelled,
    Shutdown,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SupportTransition {
    PredecessorEnded(SupportCausalPredecessorId, MonotonicTime),
    BeginSupport(MonotonicTime),
    FinishSupport,
    CloseCausalCallImpossible,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SupportLedgerError {
    InvalidInput,
    Generation,
    InvalidTransition,
    Storage(FixedStorageError),
}
const CAPACITY_ERROR: SupportLedgerError = SupportLedgerError::Storage(FixedStorageError::Capacity);
macro_rules! check {
    ($work:expr, $condition:expr, $error:expr) => {{
        $work.record(WorkDimension::InvariantChecks, 1)?;
        $condition.then_some(()).ok_or($error)
    }};
}
impl<T: Into<FixedStorageError>> From<T> for SupportLedgerError {
    fn from(error: T) -> Self {
        Self::Storage(error.into())
    }
}
type Record = (
    SupportOperation,
    SupportPool,
    SupportCausalPredecessorId,
    SupportObligationState,
    MonotonicTime,
    SupportCallScopeId,
    Option<LifecycleReservation>,
);
type LifecycleReservation = (LifecycleReserveKind, SupportOperationObligationId, u16);
#[derive(Debug, Eq, PartialEq)]
pub struct SupportChargeLedger<const R: usize, const F: usize, const H: usize> {
    generation: SupportLedgerGeneration,
    capacities: [[u32; POOLS]; 5],
    max_claims: u16,
    records: FixedRecordArena<Record, SupportFundingClaim, 2>,
    usage: [[u32; POOLS]; 5],
    reserved: [[u32; POOLS]; 3],
    starts: FixedWindowCounter<21, H>,
    lifecycle_maxima: LifecycleReserveMaxima,
}
impl<const R: usize, const F: usize, const H: usize> SupportChargeLedger<R, F, H> {
    #[allow(dead_code, reason = "C08 installs the Catalog adapter")]
    pub(crate) fn try_new(
        generation: SupportLedgerGeneration,
        capacities: [[u32; POOLS]; 5],
        max_claims: u16,
        starts: [[FixedStartCountBound; H]; 21],
        lifecycle_maxima: LifecycleReserveMaxima,
    ) -> Result<Self, SupportLedgerError> {
        let valid = (1..=1_024).contains(&max_claims)
            && total(capacities[..3].iter().flatten().copied()) <= R as u64
            && total(capacities[CREDITS]) <= R as u64
            && total(capacities[CLAIMS]) <= F as u64
            && lifecycle_maxima
                .0
                .into_iter()
                .all(|maximum| maximum > 0 && maximum as usize <= R);
        valid
            .then_some(())
            .ok_or(SupportLedgerError::InvalidInput)?;
        Ok(Self {
            generation,
            capacities,
            max_claims,
            records: FixedRecordArena::try_new(R, F)?,
            usage: [[0; POOLS]; 5],
            reserved: [[0; POOLS]; 3],
            starts: FixedWindowCounter::try_new(starts)?,
            lifecycle_maxima,
        })
    }
    pub const fn generation(&self) -> SupportLedgerGeneration {
        self.generation
    }
    pub fn reserve(
        &mut self,
        expected: SupportLedgerGeneration,
        spec: SupportObligationSpec<'_>,
        work: &mut WorkMeter,
    ) -> Result<SupportLedgerGeneration, SupportLedgerError> {
        let next = self.next(expected, work)?;
        let count = spec.claims.len();
        let pool = spec.pool as usize;
        let invalid = SupportLedgerError::InvalidInput;
        // id and physical_credit are constructor-validated to reject zero; this
        // comparison/accounting shape is intentionally retained so fixed HotPath
        // Work witnesses remain stable.
        for identity in [
            spec.id.get(),
            spec.physical_credit.get(),
            spec.predecessor.0,
        ] {
            check!(work, identity != [0; 32], invalid)?;
        }
        let valid_count = (1..=self.max_claims as usize).contains(&count);
        check!(work, valid_count, invalid)?;
        let remaining = HotPathWorkWitness::new([count as u64, 66, 0, 0, (2 * count + 3) as u64]);
        work.ensure(remaining)?;
        check!(work, spec.pool != SupportPool::Ordinary, invalid)?;
        let mut previous = None;
        for claim in spec.claims {
            work.record(WorkDimension::VisitedEntities, 1)?;
            if let Some(prior) = previous {
                check!(work, prior < claim, invalid)?;
            }
            check!(work, claim.valid_for(spec.pool), invalid)?;
            previous = Some(claim);
        }
        let claims = count as u32;
        for (class, added) in [(CONDITIONAL, 1), (CREDITS, 1), (CLAIMS, claims)] {
            check!(work, self.available(class, pool, added), CAPACITY_ERROR)?;
        }
        work.record(WorkDimension::CopiedBytes, 66)?;
        let keys = [key(0, spec.id.get()), key(1, spec.physical_credit.get())];
        let record = (
            spec.operation,
            spec.pool,
            spec.predecessor,
            Conditional,
            Default::default(),
            SupportCallScopeId([0; 32]),
            None,
        );
        self.records.try_push(keys, record, spec.claims, work)?;
        self.usage[CONDITIONAL][pool] += 1;
        self.usage[CREDITS][pool] += 1;
        self.usage[CLAIMS][pool] += claims;
        self.generation = next;
        Ok(next)
    }
    #[allow(dead_code, reason = "C10 and C12 install the ordinary support callers")]
    pub(crate) fn begin_ordinary(
        &mut self,
        expected: SupportLedgerGeneration,
        spec: OrdinarySupportSpec,
        at: MonotonicTime,
        work: &mut WorkMeter,
    ) -> Result<SupportLedgerGeneration, SupportLedgerError> {
        let change = self.prepare(expected, SupportChangeInput::BeginOrdinary(spec, at), work)?;
        self.commit(change, work)
    }
    pub(crate) fn prepare(
        &self,
        expected: SupportLedgerGeneration,
        input: SupportChangeInput,
        work: &mut WorkMeter,
    ) -> Result<SupportChange, SupportLedgerError> {
        match input {
            SupportChangeInput::BeginOrdinary(spec, at) => {
                self.prepare_begin(expected, spec, at, work)
            }
            SupportChangeInput::BeginPending(id, kind, at) => {
                self.prepare_pending(expected, id, kind, at, work)
            }
            SupportChangeInput::FinishActive(id) => self.prepare_finish(expected, id, work),
        }
    }
    fn prepare_begin(
        &self,
        expected: SupportLedgerGeneration,
        spec: OrdinarySupportSpec,
        at: MonotonicTime,
        work: &mut WorkMeter,
    ) -> Result<SupportChange, SupportLedgerError> {
        self.next(expected, work)?;
        work.record(WorkDimension::InvariantChecks, 3)?;
        // id and physical_credit are constructor-validated to reject zero; this
        // comparison/accounting shape is intentionally retained so fixed HotPath
        // Work witnesses remain stable.
        let valid = [spec.id.get(), spec.physical_credit.get(), spec.scope.0]
            .into_iter()
            .all(|id| id != [0; 32])
            && matches!(spec.claim, SupportFundingClaim::OrdinaryReservation(id) if id != [0; 32]);
        check!(work, valid, SupportLedgerError::InvalidInput)?;
        let pool = SupportPool::Ordinary as usize;
        for class in [ACTIVE, CREDITS, CLAIMS] {
            check!(work, self.available(class, pool, 1), CAPACITY_ERROR)?;
        }
        for identity in [key(0, spec.id.get()), key(1, spec.physical_credit.get())] {
            let absent = self.records.find(identity, work)?.is_none();
            check!(work, absent, FixedStorageError::Duplicate)?;
        }
        work.ensure(insertion_work(1))?;
        let start = self
            .starts
            .prepare_start(spec.operation as usize * POOLS + pool, at, work)?;
        Ok(SupportChange {
            expected,
            records: self.records.len(),
            delta: SupportDelta::BeginOrdinary(spec, at, start),
        })
    }
    fn prepare_finish(
        &self,
        expected: SupportLedgerGeneration,
        id: SupportOperationObligationId,
        work: &mut WorkMeter,
    ) -> Result<SupportChange, SupportLedgerError> {
        self.next(expected, work)?;
        let (index, record) = self.find_record(id, work)?;
        check!(
            work,
            record.3 == Active,
            SupportLedgerError::InvalidTransition
        )?;
        Ok(SupportChange {
            expected,
            records: self.records.len(),
            delta: SupportDelta::FinishActive(index, record),
        })
    }
    #[rustfmt::skip]
    fn prepare_pending(&self, expected: SupportLedgerGeneration, id: SupportOperationObligationId, kind: LifecycleReserveKind, at: MonotonicTime, work: &mut WorkMeter) -> Result<SupportChange, SupportLedgerError> { self.next(expected, work)?; let (index, record) = self.find_record(id, work)?; let reserve = record.6.ok_or(SupportLedgerError::InvalidTransition)?; check!(work, record.3 == Pending && reserve.0 == kind && at >= record.4 && self.reserved[ACTIVE][record.1 as usize] > 0, SupportLedgerError::InvalidTransition)?; let start = self.starts.prepare_start(record.0 as usize * POOLS + record.1 as usize, at, work)?; Ok(SupportChange { expected, records: self.records.len(), delta: SupportDelta::BeginPending(index, record, at, start) }) }
    #[rustfmt::skip]
    pub(crate) fn validate(&self, change: &SupportChange) -> Result<(), SupportLedgerError> {
        let target = match &change.delta {
            SupportDelta::BeginOrdinary(..) => true,
            SupportDelta::BeginPending(index, record, ..) => self.records.get(*index) == Some(record),
            SupportDelta::FinishActive(index, record) => self.records.get(*index) == Some(record),
        };
        (self.generation == change.expected && self.records.len() == change.records && target)
            .then_some(())
            .ok_or(SupportLedgerError::Generation)
    }
    #[rustfmt::skip]
    pub(crate) fn commit(
        &mut self,
        change: SupportChange,
        work: &mut WorkMeter,
    ) -> Result<SupportLedgerGeneration, SupportLedgerError> {
        self.validate(&change)?;
        match change.delta {
            SupportDelta::BeginOrdinary(spec, at, start) => {
                let record = (
                    spec.operation,
                    SupportPool::Ordinary,
                    SupportCausalPredecessorId([0; 32]),
                    Active,
                    at,
                    spec.scope,
                    None,
                );
                let keys = [key(0, spec.id.get()), key(1, spec.physical_credit.get())];
                self.records.try_push(keys, record, &[spec.claim], work)?;
                self.starts.apply_start(start);
                for class in [ACTIVE, CREDITS, CLAIMS] {
                    self.usage[class][SupportPool::Ordinary as usize] += 1;
                }
            }
            SupportDelta::FinishActive(index, _) => {
                self.records
                    .get_mut(index)
                    .expect("validated support record")
                    .3 = Retained;
            }
            SupportDelta::BeginPending(index, record, at, start) => self.commit_pending(index, record, at, start),
        }
        let next = change.expected.next().expect("prepared support generation");
        self.generation = next;
        Ok(next)
    }
    #[rustfmt::skip]
    fn commit_pending(&mut self, index: usize, record: Record, at: MonotonicTime, start: FixedWindowStart) { let pool = record.1 as usize; let entry = self.records.get_mut(index).expect("validated support record"); entry.3 = Active; entry.4 = at; self.starts.apply_start(start); self.usage[PENDING][pool] -= 1; self.usage[ACTIVE][pool] += 1; self.reserved[ACTIVE][pool] -= 1; }
    #[allow(dead_code, reason = "C12, G01, and G09 install the lifecycle callers")]
    pub(crate) fn reserve_lifecycle(
        &mut self,
        expected: SupportLedgerGeneration,
        at: MonotonicTime,
        specs: &[LifecycleReserveSpec],
        work: &mut WorkMeter,
    ) -> Result<SupportLedgerGeneration, SupportLedgerError> {
        let next = self.next(expected, work)?;
        let invalid = SupportLedgerError::InvalidInput;
        let count = u16::try_from(specs.len()).map_err(|_| invalid)?;
        check!(work, count > 0, invalid)?;
        let first = specs[0];
        let (_, pool, trigger) = lifecycle_shape(first.kind);
        let mut maxima = [0u16; 5];
        let mut prior = (None, None);
        for spec in specs {
            work.record(WorkDimension::VisitedEntities, 1)?;
            let candidate = lifecycle_shape(spec.kind);
            maxima[spec.kind as usize] += 1;
            // id and physical_credit are constructor-validated to reject zero; this
            // comparison/accounting shape is intentionally retained so fixed HotPath
            // Work witnesses remain stable.
            let identities = [
                spec.id.get(),
                spec.physical_credit.get(),
                spec.predecessor.0,
                spec.scope.0,
            ];
            let valid = identities.into_iter().all(|id| id != [0; 32])
                && prior.0.is_none_or(|id| id < spec.id)
                && prior.1.is_none_or(|id| id < spec.physical_credit)
                && spec.predecessor == first.predecessor
                && candidate.2 == trigger
                && match spec.kind {
                    LifecycleReserveKind::NextSafetySample => {
                        spec.expires_at.is_some_and(|end| at < end)
                    }
                    _ => spec.expires_at.is_none(),
                }
                && matches!(spec.claim, SupportFundingClaim::LifecycleReserve(id) if id != [0; 32])
                && maxima[spec.kind as usize] <= self.lifecycle_maxima.0[spec.kind as usize];
            work.record(WorkDimension::InvariantChecks, 10)?;
            valid.then_some(()).ok_or(invalid)?;
            for identity in [key(0, spec.id.get()), key(1, spec.physical_credit.get())] {
                let absent = self.records.find(identity, work)?.is_none();
                check!(work, absent, FixedStorageError::Duplicate)?;
            }
            prior = (Some(spec.id), Some(spec.physical_credit));
        }
        let (pool, added) = (pool as usize, u32::from(count));
        for class in [CONDITIONAL, PENDING, ACTIVE, CREDITS, CLAIMS] {
            check!(work, self.available(class, pool, added), CAPACITY_ERROR)?;
        }
        work.ensure(insertion_work(specs.len()))?;
        for spec in specs {
            let (operation, pool, _) = lifecycle_shape(spec.kind);
            let record = (
                operation,
                pool,
                spec.predecessor,
                Conditional,
                at,
                spec.scope,
                Some((spec.kind, first.id, count)),
            );
            let keys = [key(0, spec.id.get()), key(1, spec.physical_credit.get())];
            self.records
                .try_push(keys, record, &[spec.claim], work)
                .expect("lifecycle insertion was fully prevalidated");
        }
        for class in [CONDITIONAL, CREDITS, CLAIMS] {
            self.usage[class][pool] += added;
        }
        self.reserved[PENDING][pool] += added;
        self.reserved[ACTIVE][pool] += added;
        self.generation = next;
        Ok(next)
    }
    #[allow(dead_code, reason = "C12, G01, and G09 install the lifecycle callers")]
    pub(crate) fn resolve_lifecycle(
        &mut self,
        expected: SupportLedgerGeneration,
        predecessor: SupportCausalPredecessorId,
        at: MonotonicTime,
        ids: &[SupportOperationObligationId],
        result: LifecycleTriggerResult,
        work: &mut WorkMeter,
    ) -> Result<SupportLedgerGeneration, SupportLedgerError> {
        let next = self.next(expected, work)?;
        let count = u16::try_from(ids.len()).map_err(|_| SupportLedgerError::InvalidInput)?;
        check!(work, count > 0, SupportLedgerError::InvalidInput)?;
        let invalid = SupportLedgerError::InvalidTransition;
        let (trigger, required) = lifecycle_result(result);
        let pool = SupportPool::MandatoryCompletion as usize + usize::from(trigger >= 2);
        let record_bytes = std::mem::size_of::<Record>() as u64;
        let mut first_index = None;
        for (offset, id) in ids.iter().enumerate() {
            let index = self.records.find(key(0, id.get()), work)?.ok_or(invalid)?;
            work.record(WorkDimension::CopiedBytes, record_bytes)?;
            let record = *self.records.get(index).expect("indexed support record");
            let reserve = record.6.ok_or(invalid)?;
            let actual_trigger = lifecycle_shape(reserve.0).2;
            let matching = record.3 == Conditional
                && record.2 == predecessor
                && at >= record.4
                && pool == record.1 as usize
                && index == *first_index.get_or_insert(index) + offset
                && reserve.1 == ids[0]
                && reserve.2 == count
                && (trigger == actual_trigger || trigger == 4 && actual_trigger >= 2);
            check!(work, matching, invalid)?;
        }
        let added = u32::from(count);
        let held = self.reserved[PENDING][pool] >= added && self.reserved[ACTIVE][pool] >= added;
        check!(work, held, invalid)?;
        let first_index = first_index.expect("nonempty lifecycle set");
        for index in first_index..first_index + ids.len() {
            let record = self.records.get_mut(index).expect("indexed support record");
            record.3 = if required { Pending } else { ClosedConditional };
            record.4 = at;
        }
        self.reserved[PENDING][pool] -= added;
        let pending = added * u32::from(required);
        self.usage[CONDITIONAL][pool] -= pending;
        self.usage[PENDING][pool] += pending;
        self.reserved[ACTIVE][pool] -= added - pending;
        self.generation = next;
        Ok(next)
    }
    #[rustfmt::skip]
    pub(crate) fn lifecycle_kind(&self, id: SupportOperationObligationId, work: &mut WorkMeter) -> Result<LifecycleReserveKind, SupportLedgerError> { self.find_record(id, work)?.1.6.map(|reserve| reserve.0).ok_or(SupportLedgerError::InvalidTransition) }
    pub fn transition(
        &mut self,
        expected: SupportLedgerGeneration,
        id: SupportOperationObligationId,
        transition: SupportTransition,
        work: &mut WorkMeter,
    ) -> Result<SupportLedgerGeneration, SupportLedgerError> {
        if transition == FinishSupport {
            let change = self.prepare(expected, SupportChangeInput::FinishActive(id), work)?;
            return self.commit(change, work);
        }
        let next = self.next(expected, work)?;
        let (index, record) = self.find_record(id, work)?;
        work.record(WorkDimension::InvariantChecks, 1)?;
        let generic = record.6.is_none();
        let (state, time) = match (record.3, transition) {
            (Conditional, PredecessorEnded(id, at)) if id == record.2 && generic => (Pending, at),
            (Pending, BeginSupport(at)) if at >= record.4 => (Active, at),
            (Conditional, CloseCausalCallImpossible) if generic => (ClosedConditional, record.4),
            (Pending, CloseCausalCallImpossible) => (ClosedPending, record.4),
            _ => return Err(SupportLedgerError::InvalidTransition),
        };
        let pool = record.1 as usize;
        let (before, after) = (state_class(record.3), state_class(state));
        if before != after {
            let held = after == ACTIVE && record.6.is_some() && self.reserved[ACTIVE][pool] > 0;
            check!(work, held || self.available(after, pool, 1), CAPACITY_ERROR)?;
        }
        if state == Active {
            self.starts
                .try_start(record.0 as usize * POOLS + pool, time, work)?;
        }
        if before != after {
            self.usage[before][pool] -= 1;
            self.usage[after][pool] += 1;
        }
        if record.6.is_some() && (state == Active || state == ClosedPending) {
            self.reserved[ACTIVE][pool] -= 1;
        }
        let record = self.records.get_mut(index).expect("indexed support record");
        record.3 = state;
        record.4 = time;
        self.generation = next;
        Ok(next)
    }
    fn find_record(
        &self,
        id: SupportOperationObligationId,
        work: &mut WorkMeter,
    ) -> Result<(usize, Record), SupportLedgerError> {
        work.record(WorkDimension::CopiedBytes, 33)?;
        let found = self.records.find(key(0, id.get()), work)?;
        work.record(WorkDimension::InvariantChecks, 1)?;
        let index = found.ok_or(SupportLedgerError::InvalidTransition)?;
        let record_bytes = std::mem::size_of::<Record>() as u64;
        work.record(WorkDimension::CopiedBytes, record_bytes)?;
        let record = *self.records.get(index).expect("indexed support record");
        Ok((index, record))
    }
    fn next(
        &self,
        expected: SupportLedgerGeneration,
        work: &mut WorkMeter,
    ) -> Result<SupportLedgerGeneration, SupportLedgerError> {
        let current = expected == self.generation;
        check!(work, current, SupportLedgerError::Generation)?;
        work.record(WorkDimension::InvariantChecks, 1)?;
        self.generation
            .next()
            .map_err(|_| SupportLedgerError::Generation)
    }
    fn available(&self, class: usize, pool: usize, added: u32) -> bool {
        let reserved = self.reserved.get(class).map_or(0, |held| held[pool]);
        self.usage[class][pool]
            .checked_add(reserved)
            .and_then(|value| value.checked_add(added))
            .is_some_and(|value| value <= self.capacities[class][pool])
    }
}
#[allow(dead_code, reason = "used by the C08 adapter constructor")]
fn total(values: impl IntoIterator<Item = u32>) -> u64 {
    values.into_iter().map(u64::from).sum()
}
fn state_class(state: SupportObligationState) -> usize {
    [CONDITIONAL, PENDING, ACTIVE, ACTIVE, CONDITIONAL, PENDING][state as usize]
}
use SupportOperation::{DescribeModel, DescribeRequest, SampleBackendResources};
use SupportPool::{MandatoryCompletion, SafetySampling};
const LIFECYCLE_SHAPES: [(SupportOperation, SupportPool, u8); 5] = [
    (DescribeModel, MandatoryCompletion, 0),
    (DescribeRequest, MandatoryCompletion, 0),
    (DescribeRequest, MandatoryCompletion, 1),
    (SampleBackendResources, SafetySampling, 2),
    (SampleBackendResources, SafetySampling, 3),
];
fn lifecycle_shape(kind: LifecycleReserveKind) -> (SupportOperation, SupportPool, u8) {
    LIFECYCLE_SHAPES[kind as usize]
}
const LIFECYCLE_RESULTS: [u8; 14] = [1, 0, 0, 3, 2, 2, 2, 5, 4, 4, 7, 7, 7, 8];
fn lifecycle_result(result: LifecycleTriggerResult) -> (u8, bool) {
    let encoded = LIFECYCLE_RESULTS[result as usize];
    (encoded >> 1, encoded & 1 != 0)
}
fn insertion_work(count: usize) -> HotPathWorkWitness {
    let count = count as u64;
    let copied = std::mem::size_of::<(Record, SupportFundingClaim)>() as u64 + 172;
    HotPathWorkWitness::new([1_662 * count, copied * count, 0, 0, 16 * count])
}
fn key(tag: u8, id: [u8; 32]) -> [u8; 33] {
    let mut key = [0; 33];
    key[0] = tag;
    key[1..].copy_from_slice(&id);
    key
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct OutstandingCreditCell {
    pub(crate) operation: SupportOperation,
    pub(crate) pool: SupportPool,
    pub(crate) horizon: Duration,
    pub(crate) max_outstanding: u64,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SupportOutstandingCreditVectorError {
    Empty,
    TooLarge,
    ZeroOutstanding,
    ZeroHorizon,
    DuplicateAxis,
    ReverseOrder,
    Work(WorkBudgetError),
}
impl From<WorkBudgetError> for SupportOutstandingCreditVectorError {
    fn from(error: WorkBudgetError) -> Self {
        Self::Work(error)
    }
}
use SupportOutstandingCreditVectorError::{
    DuplicateAxis, Empty, ReverseOrder, TooLarge, ZeroHorizon, ZeroOutstanding,
};
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SupportOutstandingCreditVector<'a, const V: usize> {
    cells: &'a [OutstandingCreditCell],
}
impl<'a, const V: usize> SupportOutstandingCreditVector<'a, V> {
    pub(crate) fn try_new(
        cells: &'a [OutstandingCreditCell],
        work: &mut WorkMeter,
    ) -> Result<Self, SupportOutstandingCreditVectorError> {
        let count = cells.len() as u64;
        // Preflight the complete work requirement before validation.
        work.ensure(HotPathWorkWitness::new([count, 0, 0, 0, 2 + 2 * count]))?;
        check!(work, count > 0, Empty)?;
        check!(work, count <= V as u64, TooLarge)?;
        let mut previous = None;
        for cell in cells {
            work.record(WorkDimension::VisitedEntities, 1)?;
            check!(
                work,
                cell.max_outstanding > 0 && cell.horizon.as_micros() > 0,
                if cell.max_outstanding == 0 {
                    ZeroOutstanding
                } else {
                    ZeroHorizon
                }
            )?;
            let axis = (cell.operation, cell.pool, cell.horizon.as_micros());
            check!(
                work,
                previous.is_none_or(|prev| prev < axis),
                if previous.is_some_and(|prev| prev == axis) {
                    DuplicateAxis
                } else {
                    ReverseOrder
                }
            )?;
            previous = Some(axis);
        }
        Ok(Self { cells })
    }
    pub(crate) const fn len(&self) -> usize {
        self.cells.len()
    }
    pub(crate) fn iter(&self) -> std::slice::Iter<'a, OutstandingCreditCell> {
        self.cells.iter()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Duration, HotPathWorkBudget, WorkBudgetError};
    use FixedStorageError::{Duplicate, WindowExceeded};
    use SupportFundingClaim::{
        AdmissionInitial as Initial, LifecycleReserve as Lifecycle, OrdinaryReservation as Reserved,
    };
    use SupportLedgerError::{Generation as Stale, InvalidInput, InvalidTransition};
    use SupportPool::{MandatoryCompletion as Mandatory, Ordinary, SafetySampling as Safety};
    use SupportTransition::{
        BeginSupport as Begin, CloseCausalCallImpossible as Close, FinishSupport as Finish,
    };
    type Ledger = SupportChargeLedger<12, 12, 1>;
    type Result = std::result::Result<SupportLedgerGeneration, SupportLedgerError>;
    type Claim = SupportFundingClaim;
    fn new_ledger() -> Ledger {
        let generation = SupportLedgerGeneration::new(1).unwrap();
        let capacities = [[1, 2, 1], [0, 1, 0], [1, 2, 1], [1, 4, 1], [1, 4, 1]];
        let starts = [[FixedStartCountBound(Duration::from_micros(10), 1); 1]; 21];
        let maxima = LifecycleReserveMaxima([1, 2, 2, 1, 1]);
        Ledger::try_new(generation, capacities, 2, starts, maxima).unwrap()
    }
    fn ordinary_ledger() -> Ledger {
        let mut ledger = new_ledger();
        for class in [ACTIVE, CREDITS, CLAIMS] {
            ledger.capacities[class][0] = 2;
        }
        ledger
    }
    fn work() -> WorkMeter {
        WorkMeter::new(HotPathWorkBudget::binary_maximum())
    }
    fn spec(n: u8, credit: u8, pool: SupportPool, claims: &[Claim]) -> SupportObligationSpec<'_> {
        SupportObligationSpec {
            id: SupportOperationObligationId::new([n; 32]).unwrap(),
            operation: SupportOperation::MaterializeRequest,
            pool,
            physical_credit: PhysicalStartCreditId::new([credit; 32]).unwrap(),
            predecessor: SupportCausalPredecessorId([n; 32]),
            claims,
        }
    }
    fn put(ledger: &mut Ledger, n: u8, c: u8, pool: SupportPool, claims: &[Claim]) -> Result {
        ledger.reserve(ledger.generation(), spec(n, c, pool, claims), &mut work())
    }
    fn add(ledger: &mut Ledger, n: u8, credit: u8) -> Result {
        put(ledger, n, credit, Mandatory, &[Initial([n; 32])])
    }
    fn go(ledger: &mut Ledger, n: u8, transition: SupportTransition) -> Result {
        let id = SupportOperationObligationId::new([n; 32]).unwrap();
        ledger.transition(ledger.generation(), id, transition, &mut work())
    }
    #[test]
    fn support_ledger_contract() {
        let fail = |result: Result, error| assert_eq!(result, Err(error));
        let at = MonotonicTime::from_micros;
        let end = |n: u8, value| PredecessorEnded(SupportCausalPredecessorId([n; 32]), at(value));
        let mut ledger = new_ledger();
        add(&mut ledger, 1, 1).unwrap();
        fail(go(&mut ledger, 1, Begin(at(1))), InvalidTransition);
        fail(go(&mut ledger, 1, end(2, 1)), InvalidTransition);
        go(&mut ledger, 1, end(1, 5)).unwrap();
        go(&mut ledger, 1, Begin(at(5))).unwrap();
        go(&mut ledger, 1, Finish).unwrap();
        add(&mut ledger, 2, 2).unwrap();
        go(&mut ledger, 2, end(2, 5)).unwrap();
        fail(go(&mut ledger, 2, Begin(at(14))), WindowExceeded.into());
        go(&mut ledger, 2, Begin(at(15))).unwrap();
        add(&mut ledger, 3, 3).unwrap();
        go(&mut ledger, 3, end(3, 15)).unwrap();
        fail(go(&mut ledger, 3, Begin(at(25))), CAPACITY_ERROR);
        let mut ledger = new_ledger();
        let before = ledger.generation();
        for claims in [[Initial([1; 32]); 2], [Initial([2; 32]), Initial([1; 32])]] {
            let mut measured = work();
            let result = ledger.reserve(before, spec(7, 7, Mandatory, &claims), &mut measured);
            fail(result, InvalidInput);
            assert_eq!(measured.witness().value(WorkDimension::VisitedEntities), 2);
            assert_eq!(measured.witness().value(WorkDimension::InvariantChecks), 9);
        }
        assert_eq!((ledger.generation(), ledger.records.len()), (before, 0));
        let mut measured = work();
        let valid = spec(7, 7, Mandatory, &[Initial([7; 32])]);
        ledger.reserve(before, valid, &mut measured).unwrap();
        assert_eq!(measured.witness().value(WorkDimension::VisitedEntities), 3);
        let mut ledger = new_ledger();
        let before = ledger.generation();
        let mut exhausted = work();
        exhausted
            .record(WorkDimension::VisitedEntities, 1_704_575)
            .unwrap();
        let result = ledger.reserve(
            before,
            spec(7, 7, Mandatory, &[Initial([7; 32])]),
            &mut exhausted,
        );
        let error =
            WorkBudgetError::BudgetExceeded(WorkDimension::VisitedEntities, 1_704_575, 1_704_576);
        fail(result, error.into());
        assert_eq!((ledger.generation(), ledger.records.len()), (before, 0));
        fail(
            put(&mut ledger, 1, 1, Ordinary, &[Reserved([1; 32])]),
            InvalidInput,
        );
        add(&mut ledger, 1, 1).unwrap();
        let lifecycle = [Lifecycle([9; 32])];
        fail(put(&mut ledger, 9, 9, Safety, &lifecycle), InvalidInput);
        assert_eq!(ledger.records.get(0).unwrap().5.0, [0; 32]);
        go(&mut ledger, 1, Close).unwrap();
        fail(add(&mut ledger, 2, 1), Duplicate.into());
        add(&mut ledger, 2, 2).unwrap();
        fail(add(&mut ledger, 3, 3), CAPACITY_ERROR);
        go(&mut ledger, 2, end(2, 1)).unwrap();
        go(&mut ledger, 2, Close).unwrap();
        add(&mut ledger, 3, 3).unwrap();
        fail(go(&mut ledger, 3, end(3, 1)), CAPACITY_ERROR);
        fail(put(&mut ledger, 7, 7, Safety, &[]), InvalidInput);
        let stale = SupportLedgerGeneration::new(1).unwrap();
        let id = SupportOperationObligationId::new([2; 32]).unwrap();
        let result = ledger.transition(stale, id, end(2, 1), &mut work());
        fail(result, Stale);
    }

    fn ordinary(parts: (u8, u8, u8, Claim)) -> OrdinarySupportSpec {
        let (id, credit, scope, claim) = parts;
        OrdinarySupportSpec {
            id: SupportOperationObligationId::new([id; 32]).unwrap(),
            operation: SupportOperation::DescribeRequest,
            physical_credit: PhysicalStartCreditId::new([credit; 32]).unwrap(),
            scope: SupportCallScopeId([scope; 32]),
            claim,
        }
    }
    fn begin(ledger: &mut Ledger, spec: OrdinarySupportSpec, at: MonotonicTime) -> Result {
        ledger.begin_ordinary(ledger.generation(), spec, at, &mut work())
    }
    #[test]
    fn c08_reservation_contracts() {
        let snapshot = |ledger: &Ledger| {
            let claims = |index| ledger.records.claims(index).map(<[_]>::to_vec);
            (
                ledger.generation(),
                std::array::from_fn::<_, 12, _>(|index| ledger.records.get(index).copied()),
                std::array::from_fn::<_, 12, _>(claims),
                std::array::from_fn::<_, 21, _>(|cell| ledger.starts.len(cell)),
                ledger.usage,
                ledger.reserved,
            )
        };
        macro_rules! rejected {
            ($ledger:ident, $action:expr, $error:expr) => {{
                let before = snapshot(&$ledger);
                assert_eq!($action, Err($error));
                assert_eq!(snapshot(&$ledger), before);
            }};
        }

        let at = MonotonicTime::from_micros;
        let valid = ordinary((1, 21, 41, Reserved([1; 32])));
        let second = ordinary((2, 22, 42, Reserved([2; 32])));
        let mut ledger = ordinary_ledger();
        for invalid in [
            ordinary((1, 21, 0, Reserved([1; 32]))),
            ordinary((1, 21, 41, Reserved([0; 32]))),
            ordinary((1, 21, 41, Initial([1; 32]))),
        ] {
            rejected!(ledger, begin(&mut ledger, invalid, at(1)), InvalidInput);
        }
        let initial = ledger.generation();
        let before = snapshot(&ledger);
        let mut measured = work();
        let change = ledger
            .prepare(
                initial,
                SupportChangeInput::BeginOrdinary(valid, at(5)),
                &mut measured,
            )
            .unwrap();
        let replay = ledger
            .prepare(
                initial,
                SupportChangeInput::BeginOrdinary(second, at(15)),
                &mut work(),
            )
            .unwrap();
        assert_eq!(snapshot(&ledger), before);
        assert_eq!(ledger.validate(&change), Ok(()));
        let next = ledger.commit(change, &mut measured).unwrap();
        assert_eq!(
            measured.witness(),
            HotPathWorkWitness::new([2, 288, 0, 0, 20])
        );
        let mut rejected_work = work();
        rejected!(ledger, ledger.commit(replay, &mut rejected_work), Stale);
        assert_eq!(rejected_work.witness(), HotPathWorkWitness::default());
        let record = ledger.records.get(0).unwrap();
        assert_eq!(next, initial.next().unwrap());
        let predecessor = SupportCausalPredecessorId([0; 32]);
        assert_eq!(
            (record.0, record.1, record.2),
            (valid.operation, Ordinary, predecessor)
        );
        assert_eq!((record.3, record.4, record.5), (Active, at(5), valid.scope));
        assert_eq!(ledger.records.claims(0), Some(&[valid.claim][..]));
        for class in [ACTIVE, CREDITS, CLAIMS] {
            assert_eq!(ledger.usage[class][0], 1);
        }
        assert_eq!(ledger.starts.len(valid.operation as usize * POOLS), Some(1));
        let before = snapshot(&ledger);
        let mut finished = work();
        let change = ledger
            .prepare(
                next,
                SupportChangeInput::FinishActive(valid.id),
                &mut finished,
            )
            .unwrap();
        assert_eq!(snapshot(&ledger), before);
        assert_eq!(ledger.validate(&change), Ok(()));
        assert_eq!(
            ledger.commit(change, &mut finished).unwrap(),
            next.next().unwrap()
        );
        assert_eq!(
            finished.witness(),
            HotPathWorkWitness::new([2, 177, 0, 0, 4])
        );
        assert_eq!(ledger.records.get(0).unwrap().3, Retained);

        let error = SupportLedgerError::Storage(Duplicate);
        for (id, credit) in [(1, 22), (2, 21)] {
            let duplicate = ordinary((id, credit, 42, Reserved([2; 32])));
            rejected!(ledger, begin(&mut ledger, duplicate, at(6)), error);
        }
        let error = SupportLedgerError::Storage(WindowExceeded);
        rejected!(ledger, begin(&mut ledger, second, at(6)), error);
        rejected!(
            ledger,
            ledger.begin_ordinary(initial, second, at(15), &mut work()),
            Stale
        );

        let mut ledger = new_ledger();
        begin(&mut ledger, valid, at(5)).unwrap();
        rejected!(ledger, begin(&mut ledger, second, at(15)), CAPACITY_ERROR);

        let mut ledger = new_ledger();
        let mut exhausted = work();
        exhausted
            .record(WorkDimension::VisitedEntities, 1_704_575)
            .unwrap();
        let before = snapshot(&ledger);
        let fault = ledger.begin_ordinary(ledger.generation(), valid, at(1), &mut exhausted);
        assert!(matches!(
            fault,
            Err(SupportLedgerError::Storage(FixedStorageError::Work(_)))
        ));
        assert_eq!(snapshot(&ledger), before);
        type Life = LifecycleReserveSpec;
        use LifecycleTriggerResult::*;
        fn lifecycle_ledger() -> Ledger {
            let mut ledger = new_ledger();
            ledger.capacities = [[0, 3, 2], [0, 2, 1], [0, 2, 1], [0, 4, 2], [0, 4, 2]];
            ledger
        }
        fn life(n: u8, kind: LifecycleReserveKind) -> Life {
            Life {
                id: SupportOperationObligationId::new([n; 32]).unwrap(),
                kind,
                physical_credit: PhysicalStartCreditId::new([n + 20; 32]).unwrap(),
                predecessor: SupportCausalPredecessorId([90; 32]),
                scope: SupportCallScopeId([n + 40; 32]),
                claim: Lifecycle([n; 32]),
                expires_at: None,
            }
        }
        fn reserve(ledger: &mut Ledger, at: MonotonicTime, specs: &[Life]) -> Result {
            ledger.reserve_lifecycle(ledger.generation(), at, specs, &mut work())
        }
        fn trigger(
            ledger: &mut Ledger,
            at: MonotonicTime,
            specs: &[Life],
            result: LifecycleTriggerResult,
            measured: &mut WorkMeter,
        ) -> Result {
            let ids = specs.iter().map(|spec| spec.id).collect::<Vec<_>>();
            let (generation, predecessor) = (ledger.generation(), specs[0].predecessor);
            ledger.resolve_lifecycle(generation, predecessor, at, &ids, result, measured)
        }
        use LifecycleReserveKind::{
            FirstSafetySample as First, NextSafetySample as Next,
            PostLoadModelDescription as Model, PostLoadRequestDescription as Request,
            PostObservationRequestDescription as Observe,
        };
        let specs = [life(1, Model), life(2, Request)];
        let mut ledger = lifecycle_ledger();
        let mut exhausted = work();
        exhausted
            .record(WorkDimension::VisitedEntities, 1_704_575)
            .unwrap();
        let before = snapshot(&ledger);
        let fault = ledger.reserve_lifecycle(ledger.generation(), at(1), &specs, &mut exhausted);
        assert!(matches!(
            fault,
            Err(SupportLedgerError::Storage(FixedStorageError::Work(_)))
        ));
        assert_eq!(snapshot(&ledger), before);
        reserve(&mut ledger, at(1), &specs).unwrap();
        add(&mut ledger, 3, 3).unwrap();
        let end = PredecessorEnded(SupportCausalPredecessorId([3; 32]), at(2));
        rejected!(ledger, go(&mut ledger, 3, end), CAPACITY_ERROR);
        rejected!(
            ledger,
            trigger(&mut ledger, at(2), &specs, ObservationFailed, &mut work()),
            InvalidTransition
        );
        let pending = trigger(&mut ledger, at(4), &specs, LoadSucceeded, &mut work()).unwrap();
        let cell = DescribeModel as usize * POOLS + Mandatory as usize;
        assert_eq!(
            (ledger.reserved[PENDING][1], ledger.starts.len(cell)),
            (0, Some(0))
        );
        go(&mut ledger, 1, Begin(at(5))).unwrap();
        go(&mut ledger, 2, Close).unwrap();
        assert_eq!(ledger.generation(), pending.next().unwrap().next().unwrap());
        assert_eq!(ledger.reserved[ACTIVE][1], 0);
        assert_eq!(ledger.records.get(0).unwrap().3, Active);
        assert_eq!(ledger.records.get(1).unwrap().3, ClosedPending);

        let resolved_work = HotPathWorkWitness::new([2, 144, 0, 0, 5]);
        let rejected_work = HotPathWorkWitness::new([2, 144, 0, 0, 4]);
        let run = |kind, result, state| {
            let mut spec = life(result as u8 + 10, kind);
            spec.expires_at = (kind == Next).then_some(at(3));
            let (mut ledger, specs) = (lifecycle_ledger(), [spec]);
            reserve(&mut ledger, at(1), &specs).unwrap();
            let mut measured = work();
            trigger(&mut ledger, at(2), &specs, result, &mut measured).unwrap();
            assert_eq!(ledger.records.get(0).unwrap().3, state);
            assert_eq!(measured.witness(), resolved_work);
            let before = snapshot(&ledger);
            measured = work();
            let result = trigger(&mut ledger, at(2), &specs, result, &mut measured);
            assert_eq!(result, Err(InvalidTransition));
            assert_eq!(snapshot(&ledger), before);
            assert_eq!(measured.witness(), rejected_work);
        };
        macro_rules! cases {
            ($($kind:ident, $state:ident => [$($result:ident),+]);+ $(;)?) => {
                $($(run($kind, $result, $state);)+)+
            };
        }
        cases! {
            Model, Pending => [LoadSucceeded];
            Model, ClosedConditional => [LoadFailed, LoadCancelled];
            Observe, Pending => [ObservationDescriptionsRequired];
            Observe, ClosedConditional => [ObservationUnchanged, ObservationFailed, ObservationCancelled];
            First, Pending => [QualificationActivated];
            First, ClosedConditional => [QualificationFailed, QualificationCancelled];
            Next, Pending => [SampleSucceeded, SampleFailed, SampleCancelled];
            Next, ClosedConditional => [Shutdown];
        }

        let mut ledger = lifecycle_ledger();
        for kind in [Model, First] {
            let over = [life(4, kind), life(5, kind)];
            rejected!(ledger, reserve(&mut ledger, at(1), &over), InvalidInput);
        }
        let mut next = life(6, Next);
        rejected!(ledger, reserve(&mut ledger, at(1), &[next]), InvalidInput);
        next.expires_at = Some(at(2));
        rejected!(ledger, reserve(&mut ledger, at(2), &[next]), InvalidInput);
        let first = [life(7, First)];
        reserve(&mut ledger, at(1), &first).unwrap();
        next = life(8, Next);
        next.expires_at = Some(at(4));
        rejected!(ledger, reserve(&mut ledger, at(2), &[next]), CAPACITY_ERROR);
        trigger(&mut ledger, at(2), &first, QualificationFailed, &mut work()).unwrap();
        reserve(&mut ledger, at(2), &[next]).unwrap();
    }

    use SupportOutstandingCreditVectorError::{
        DuplicateAxis as DupAxis, Empty, ReverseOrder as Reversed, TooLarge, Work as WorkFault,
        ZeroHorizon, ZeroOutstanding,
    };
    fn cell(horizon: u64, outstanding: u64) -> OutstandingCreditCell {
        OutstandingCreditCell {
            operation: SupportOperation::DescribeModel,
            pool: SupportPool::Ordinary,
            horizon: Duration::from_micros(horizon),
            max_outstanding: outstanding,
        }
    }
    fn axis_cells(count: usize, outstanding: u64) -> Vec<OutstandingCreditCell> {
        (1..=count)
            .map(|horizon| cell(horizon as u64, outstanding))
            .collect()
    }
    fn oc(
        operation: SupportOperation,
        pool: SupportPool,
        horizon: u64,
        outstanding: u64,
    ) -> OutstandingCreditCell {
        OutstandingCreditCell {
            operation,
            pool,
            horizon: Duration::from_micros(horizon),
            max_outstanding: outstanding,
        }
    }
    fn make_view<'a, const V: usize>(
        cells: &'a [OutstandingCreditCell],
        work: &mut WorkMeter,
    ) -> std::result::Result<
        SupportOutstandingCreditVector<'a, V>,
        SupportOutstandingCreditVectorError,
    > {
        SupportOutstandingCreditVector::<'a, V>::try_new(cells, work)
    }
    fn reject<const V: usize>(
        cells: &[OutstandingCreditCell],
        expected: SupportOutstandingCreditVectorError,
    ) -> HotPathWorkWitness {
        let mut meter = work();
        let result = make_view::<V>(cells, &mut meter);
        assert_eq!(result, Err(expected));
        assert_eq!(meter.witness().value(WorkDimension::CopiedBytes), 0);
        assert_eq!(meter.witness().value(WorkDimension::Allocations), 0);
        assert_eq!(meter.witness().value(WorkDimension::CandidateWork), 0);
        meter.witness()
    }
    #[test]
    fn c16_outstanding_credit_vector_contract() {
        let cells = axis_cells(1, 1);
        let mut meter = work();
        let view = make_view::<168>(&cells, &mut meter).unwrap();
        assert_eq!(meter.witness(), HotPathWorkWitness::new([1, 0, 0, 0, 4]));
        assert_eq!(view.len(), 1);
        let collected: Vec<_> = view.iter().copied().collect();
        assert_eq!(collected, cells);

        let cells = axis_cells(1, 1);
        let mut meter = work();
        meter
            .record(WorkDimension::VisitedEntities, 1_704_575)
            .unwrap();
        let fault = make_view::<168>(&cells, &mut meter);
        let error =
            WorkBudgetError::BudgetExceeded(WorkDimension::VisitedEntities, 1_704_575, 1_704_576);
        assert_eq!(fault, Err(WorkFault(error)));
        assert_eq!(
            meter.witness(),
            HotPathWorkWitness::new([1_704_575, 0, 0, 0, 0])
        );
        let mut meter = work();
        meter
            .record(WorkDimension::InvariantChecks, 28_705)
            .unwrap();
        let fault = make_view::<168>(&cells, &mut meter);
        let error = WorkBudgetError::BudgetExceeded(WorkDimension::InvariantChecks, 28_708, 28_709);
        assert_eq!(fault, Err(WorkFault(error)));
        assert_eq!(
            meter.witness(),
            HotPathWorkWitness::new([0, 0, 0, 0, 28_705])
        );

        let cells = axis_cells(6, 1);
        let mut meter = work();
        let view = make_view::<168>(&cells, &mut meter).unwrap();
        assert_eq!(meter.witness(), HotPathWorkWitness::new([6, 0, 0, 0, 14]));
        assert_eq!(view.len(), 6);

        let cells = axis_cells(168, 1);
        let mut meter = work();
        let view = make_view::<168>(&cells, &mut meter).unwrap();
        assert_eq!(
            meter.witness(),
            HotPathWorkWitness::new([168, 0, 0, 0, 338])
        );
        assert_eq!(view.len(), 168);

        let tail = [cell(1, 1), cell(1, 1)];
        let witness = reject::<168>(&tail, DupAxis);
        assert_eq!(witness, HotPathWorkWitness::new([2, 0, 0, 0, 6]));

        assert_eq!(
            reject::<168>(&[cell(1, 1), cell(1, 2)], DupAxis),
            HotPathWorkWitness::new([2, 0, 0, 0, 6])
        );
        assert_eq!(
            reject::<168>(&[cell(2, 1), cell(1, 1)], Reversed),
            HotPathWorkWitness::new([2, 0, 0, 0, 6])
        );
        assert_eq!(
            reject::<168>(&[cell(1, 0)], ZeroOutstanding),
            HotPathWorkWitness::new([1, 0, 0, 0, 3])
        );
        assert_eq!(
            reject::<168>(&[cell(0, 1)], ZeroHorizon),
            HotPathWorkWitness::new([1, 0, 0, 0, 3])
        );
        assert_eq!(
            reject::<168>(&[], Empty),
            HotPathWorkWitness::new([0, 0, 0, 0, 1])
        );
        assert_eq!(
            reject::<168>(&axis_cells(169, 1), TooLarge),
            HotPathWorkWitness::new([0, 0, 0, 0, 2])
        );
    }
    #[test]
    fn c16_outstanding_credit_vector_axis_ordering() {
        // Same horizon, increasing SupportOperation succeeds.
        let cells = [
            oc(SupportOperation::DescribeModel, Ordinary, 1, 1),
            oc(SupportOperation::DescribeRequest, Ordinary, 1, 1),
        ];
        let mut meter = work();
        let view = make_view::<168>(&cells, &mut meter).unwrap();
        assert_eq!(meter.witness(), HotPathWorkWitness::new([2, 0, 0, 0, 6]));
        assert_eq!(view.len(), 2);

        // Same operation/horizon, increasing SupportPool succeeds.
        let cells = [
            oc(SupportOperation::DescribeModel, Ordinary, 1, 1),
            oc(SupportOperation::DescribeModel, Mandatory, 1, 1),
        ];
        let mut meter = work();
        let view = make_view::<168>(&cells, &mut meter).unwrap();
        assert_eq!(meter.witness(), HotPathWorkWitness::new([2, 0, 0, 0, 6]));
        assert_eq!(view.len(), 2);

        // Decreasing SupportOperation rejects ReverseOrder.
        assert_eq!(
            reject::<168>(
                &[
                    oc(SupportOperation::DescribeRequest, Ordinary, 1, 1),
                    oc(SupportOperation::DescribeModel, Ordinary, 1, 1),
                ],
                Reversed,
            ),
            HotPathWorkWitness::new([2, 0, 0, 0, 6])
        );

        // Decreasing SupportPool rejects ReverseOrder.
        assert_eq!(
            reject::<168>(
                &[
                    oc(SupportOperation::DescribeModel, Mandatory, 1, 1),
                    oc(SupportOperation::DescribeModel, Ordinary, 1, 1),
                ],
                Reversed,
            ),
            HotPathWorkWitness::new([2, 0, 0, 0, 6])
        );
    }
}
