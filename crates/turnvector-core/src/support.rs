use crate::bounded::FixedWindowStart;
use crate::{
    Duration, FixedRecordArena, FixedStartCountBound, FixedStorageError, FixedWindowCounter,
    FutureTurnSupportEntitlementId, HotPathWorkWitness, MonotonicTime, PhysicalStartCreditId,
    SupportLedgerGeneration, SupportOperationObligationId, SupportOutstandingCreditVectorId,
    WorkBudgetError, WorkDimension, WorkMeter,
};
use std::sync::atomic::{AtomicU64, Ordering};
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
    lifecycle_batch_max: u16,
    bundles: RequestBundleStore,
    instance_nonce: u64,
}
/// Process-local one-issuance dispenser for private per-ledger instance
/// nonces. Not a domain authority, generation, public identity, or
/// caller-supplied fact: it only proves that a prepared Change belongs to one
/// exact ledger instance.
static PROCESS_INSTANCE_DISPENSER: AtomicU64 = AtomicU64::new(0);
/// Pure checked nonce issuance: issues the strictly increasing next nonzero
/// value at most once, or `None` at `u64::MAX` exhaustion without wrap or
/// reuse. Exposed as a helper so landing tests use a local atomic and never
/// exhaust the process-global dispenser.
fn issue_instance_nonce(dispenser: &AtomicU64) -> Option<u64> {
    let mut current = dispenser.load(Ordering::Relaxed);
    loop {
        if current == u64::MAX {
            return None;
        }
        let next = current + 1;
        match dispenser.compare_exchange_weak(current, next, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return Some(next),
            Err(actual) => current = actual,
        }
    }
}
impl<const R: usize, const F: usize, const H: usize> SupportChargeLedger<R, F, H> {
    #[allow(dead_code, reason = "C08 installs the Catalog adapter")]
    pub(crate) fn try_new(
        generation: SupportLedgerGeneration,
        capacities: [[u32; POOLS]; 5],
        max_claims: u16,
        starts: [[FixedStartCountBound; H]; 21],
        lifecycle_maxima: LifecycleReserveMaxima,
        bundle_records: usize,
        bundle_cells: usize,
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
        let maxima = lifecycle_maxima.0;
        // Checked constructor derivation of the exact maximum lifecycle batch
        // length: kinds 0 and 1 share one trigger, every other kind uses its
        // own trigger, and the batch never exceeds u16::MAX.
        let shared = u64::from(maxima[0]) + u64::from(maxima[1]);
        let batch = shared
            .max(u64::from(maxima[2]))
            .max(u64::from(maxima[3]))
            .max(u64::from(maxima[4]));
        let lifecycle_batch_max = u16::try_from(batch).unwrap_or(u16::MAX);
        // Checked worst-case lifecycle batch Work before construction: the
        // exact maximum batch `M_L` must fit every binary Work maximum when
        // every reciprocal legacy and C16 lookup visits its full bounded path
        // and the complete insertion envelope is charged. Per member this is
        // one loop visit, two legacy finds and two C16 finds (each at most
        // B+1 = 265 visits, and each C16 find at most B+1 = 265 checks), the
        // two per-find absence checks, plus the insertion envelope of 1,662
        // visits and 16 checks; the fixed checks are next (2), nonempty (1),
        // batch bound (1), and capacity (5).
        let batch = u64::from(lifecycle_batch_max);
        let worst_visits = batch
            .checked_mul(2_723)
            .ok_or(SupportLedgerError::InvalidInput)?;
        let copied = std::mem::size_of::<(Record, SupportFundingClaim)>() as u64 + 172;
        let worst_copied = batch
            .checked_mul(copied)
            .ok_or(SupportLedgerError::InvalidInput)?;
        let worst_checks = batch
            .checked_mul(560)
            .and_then(|value| value.checked_add(9))
            .ok_or(SupportLedgerError::InvalidInput)?;
        if worst_visits > 1_704_575 || worst_copied > 2_097_152 || worst_checks > 28_708 {
            return Err(SupportLedgerError::InvalidInput);
        }
        // After all other fallible construction, the process-global dispenser
        // issues one nonzero instance nonce; checked exhaustion is a
        // constructor error and no nonce is reused.
        let instance_nonce = issue_instance_nonce(&PROCESS_INSTANCE_DISPENSER)
            .ok_or(SupportLedgerError::Storage(FixedStorageError::Capacity))?;
        Ok(Self {
            generation,
            capacities,
            max_claims,
            records: FixedRecordArena::try_new(R, F)?,
            usage: [[0; POOLS]; 5],
            reserved: [[0; POOLS]; 3],
            starts: FixedWindowCounter::try_new(starts)?,
            lifecycle_maxima,
            lifecycle_batch_max,
            bundles: RequestBundleStore::try_new(bundle_records, bundle_cells)?,
            instance_nonce,
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
        self.reciprocal_absent(spec.id.get(), spec.physical_credit.get(), work)?;
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
        self.reciprocal_absent(spec.id.get(), spec.physical_credit.get(), work)?;
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
        check!(work, count <= self.lifecycle_batch_max, invalid)?;
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
            self.reciprocal_absent(spec.id.get(), spec.physical_credit.get(), work)?;
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
    /// Metered reciprocal absence preflight for one earlier-row insertion:
    /// both shared tagged identities must be absent from the C16
    /// request-bundle store. Live and retained-tombstone C16 leaves block
    /// later legacy reuse until pristine withdrawal or accepted C18 expiry
    /// removes them.
    fn reciprocal_absent(
        &self,
        obligation: [u8; 32],
        credit: [u8; 32],
        work: &mut WorkMeter,
    ) -> Result<(), SupportLedgerError> {
        for (tag, identity) in [(TAG_OBLIGATION, obligation), (TAG_CREDIT, credit)] {
            let absent = self.bundles.find(tag, &identity, work)?.is_none();
            check!(work, absent, FixedStorageError::Duplicate)?;
        }
        Ok(())
    }
    /// Read-only metered preparation of one complete C16 request bundle. The
    /// input binds exactly `K = 11` tagged identities (`q = 3` canonical
    /// obligations, credits, and `AdmissionInitial` claims, one Future Turn
    /// Support Entitlement, one Support Outstanding Credit Vector), all
    /// reciprocally absent from the legacy and C16 stores, with free
    /// record/cell/leaf/branch capacity and the complete reserve Work
    /// envelope preflighted before any mutation. Returns the non-forgeable
    /// instance-bound `BundleChange`; dropping it changes no state.
    pub(crate) fn prepare_bundle<'a, const V: usize>(
        &self,
        expected: SupportLedgerGeneration,
        input: &RequestBundleInput<'a, V>,
        work: &mut WorkMeter,
    ) -> Result<BundleChange<'a, V>, SupportLedgerError> {
        self.next(expected, work)?;
        let invalid = SupportLedgerError::InvalidInput;
        let obligations = input.obligations;
        let credits = input.credits;
        let claims = input.claims;
        // Canonical strict order inside each same-tag group proves internal
        // distinctness in O(q) before any index work; every identity is
        // nonzero and the borrowed vector is nonempty.
        for group in [
            obligations.map(|id| id.get()),
            credits.map(|id| id.get()),
            claims,
        ] {
            let mut prior = None;
            for identity in group {
                check!(work, identity != [0; 32], invalid)?;
                check!(work, prior.is_none_or(|prior| prior < identity), invalid)?;
                prior = Some(identity);
            }
        }
        check!(work, input.entitlement.get() != [0; 32], invalid)?;
        check!(work, input.vector.get() != [0; 32], invalid)?;
        check!(work, input.cells.len() > 0, invalid)?;
        let record = BundleRecord {
            obligations,
            credits,
            claims,
            entitlement: input.entitlement,
            vector: input.vector,
            vector_head: None,
            vector_len: input.cells.len(),
            state: BundleRecordState::Live,
        };
        // All K tagged identities absent from the C16 store and the six
        // shared obligation/credit identities absent from the legacy arena.
        let mut identities = [None; K];
        for (slot, key) in identities.iter_mut().zip(record.tagged_keys()) {
            let found = self.bundles.find(key.tag, &key.identity, work)?;
            check!(work, found.is_none(), FixedStorageError::Duplicate)?;
            *slot = found;
        }
        for identity in obligations
            .iter()
            .map(|id| key(0, id.get()))
            .chain(credits.iter().map(|id| key(1, id.get())))
        {
            let absent = self.records.find(identity, work)?.is_none();
            check!(work, absent, FixedStorageError::Duplicate)?;
        }
        // Logical and physical free capacity for one record, v cells, K
        // leaves, and the required branches.
        let branch_need = if self.bundles.is_empty() { K - 1 } else { K };
        check!(work, self.bundles.free_record_len() >= 1, CAPACITY_ERROR)?;
        check!(
            work,
            self.bundles.free_cell_len() >= record.vector_len,
            CAPACITY_ERROR
        )?;
        check!(work, self.bundles.free_leaf_len() >= K, CAPACITY_ERROR)?;
        check!(
            work,
            self.bundles.free_branch_len() >= branch_need,
            CAPACITY_ERROR
        )?;
        // Complete reserve envelope: prepare/validate lookups plus the
        // commit's copy/link/index work, charged before mutation.
        work.ensure(bundle_reserve_work(record.vector_len))?;
        Ok(BundleChange {
            nonce: self.instance_nonce,
            expected,
            capacities: self.capacities,
            usage: self.usage,
            reserved: self.reserved,
            free_records: self.bundles.free_record_len() as u32,
            free_cells: self.bundles.free_cell_len(),
            free_leaves: self.bundles.free_leaf_len(),
            free_branches: self.bundles.free_branch_len(),
            identities,
            record,
            vector: input.cells,
        })
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
/// Conservative complete-reserve Work envelope for one C16 bundle with `v`
/// validated cells, charged before any mutation. Per tagged identity it
/// budgets two absence traversals in prepare, two in validate, and one peer,
/// one insertion traversal, and a 33-byte first-difference pass in commit,
/// each bounded by `B + 1` branches plus one leaf; the six shared
/// obligation/credit identities add their reciprocal legacy traversals; each
/// cell adds five passes. The fixed checks cover the prepare preflight (next
/// 2, identity validation 18, entitlement/vector/nonempty 3, absence
/// comparisons 17, free capacity 4) and the validate preflight (nonce 1,
/// generation 1, capacities 15, usage 15, reserved 9, free counts 4, identity
/// comparison 11, legacy comparison 6, record slot 2, cell selection 1).
fn bundle_reserve_work(cells: usize) -> HotPathWorkWitness {
    let v = cells as u64;
    let bits = u64::from(IDENTITY_BITS) + 1;
    let fixed = K as u64;
    let visits = fixed * (4 * bits + 33) + 12 * bits + 1 + 5 * v;
    let checks = fixed * (4 * bits) + 109 + 5 * v;
    let copied = (std::mem::size_of::<BundleRecord>() as u64 + std::mem::size_of::<u32>() as u64)
        + fixed
            * (2 * std::mem::size_of::<Option<IdentityLeaf>>() as u64
                + 2 * std::mem::size_of::<Option<IdentityBranch>>() as u64
                + 4 * std::mem::size_of::<u32>() as u64)
        + 4 * v * (std::mem::size_of::<CellSlot>() as u64 + std::mem::size_of::<usize>() as u64);
    HotPathWorkWitness::new([visits, copied, 0, 0, checks])
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
/// Complete C16 request-bundle input: exactly three operation-specific
/// initial/release obligations, three physical credits, three distinct
/// request-owned `AdmissionInitial` claims, one Future Turn Support
/// Entitlement, one Support Outstanding Credit Vector, and its borrowed
/// validated cells. The vector view is constructed and validated first; the
/// ledger transaction never copies the cells.
pub(crate) struct RequestBundleInput<'a, const V: usize> {
    pub(crate) obligations: [SupportOperationObligationId; 3],
    pub(crate) credits: [PhysicalStartCreditId; 3],
    pub(crate) claims: [[u8; 32]; 3],
    pub(crate) entitlement: FutureTurnSupportEntitlementId,
    pub(crate) vector: SupportOutstandingCreditVectorId,
    pub(crate) cells: SupportOutstandingCreditVector<'a, V>,
}
/// Non-forgeable prepared C16 bundle change: fixed-size semantic before-image
/// facts (exact instance nonce, expected generation, complete capacity/usage
/// aggregates, free record/cell/index-node counts, and all `K = 11` tagged
/// identity lookup results) plus the borrowed validated vector and the exact
/// fixed target record. Intentionally not Clone or Copy; dropping it changes
/// no state.
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct BundleChange<'a, const V: usize> {
    nonce: u64,
    expected: SupportLedgerGeneration,
    capacities: [[u32; POOLS]; 5],
    usage: [[u32; POOLS]; 5],
    reserved: [[u32; POOLS]; 3],
    free_records: u32,
    free_cells: usize,
    free_leaves: usize,
    free_branches: usize,
    identities: [Option<u32>; K],
    record: BundleRecord,
    vector: SupportOutstandingCreditVector<'a, V>,
}
/// Private fixed entitlement-cell arena owned conceptually by the Support Charge
/// Ledger. Constructor-preallocated exact-capacity `slots` and LIFO `free` stores,
/// no later growth, no hot-path allocation, no public seam.
#[derive(Clone, Debug, Eq, PartialEq)]
struct EntitlementCellArena {
    capacity: usize,
    slots: Vec<CellSlot>,
    free: Vec<usize>,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CellSlot {
    Vacant,
    Occupied {
        owner_record: usize,
        cell: OutstandingCreditCell,
        next_owned: Option<usize>,
    },
}
impl EntitlementCellArena {
    /// Checked physical storage bytes for `capacity` cell slots plus their
    /// LIFO free stack, against the binary Storage/CopiedBytes maximum.
    fn storage_bytes(capacity: u64) -> Option<u64> {
        let slot_bytes = std::mem::size_of::<CellSlot>() as u64;
        let index_bytes = std::mem::size_of::<usize>() as u64;
        let total = capacity
            .checked_mul(slot_bytes)?
            .checked_add(capacity.checked_mul(index_bytes)?)?;
        (total <= 2_097_152).then_some(total)
    }
    fn try_new(capacity: usize) -> Result<Self, FixedStorageError> {
        if capacity == 0 {
            return Err(FixedStorageError::Capacity);
        }
        // Checked storage arithmetic before allocation.
        let slot_bytes = std::mem::size_of::<CellSlot>() as u64;
        let index_bytes = std::mem::size_of::<usize>() as u64;
        let capacity_u64 = u64::try_from(capacity).map_err(|_| FixedStorageError::Allocation)?;
        let slots_bytes = capacity_u64
            .checked_mul(slot_bytes)
            .ok_or(FixedStorageError::Allocation)?;
        let free_bytes = capacity_u64
            .checked_mul(index_bytes)
            .ok_or(FixedStorageError::Allocation)?;
        if slots_bytes > isize::MAX as u64 || free_bytes > isize::MAX as u64 {
            return Err(FixedStorageError::Allocation);
        }
        // Binary Storage/CopiedBytes maximum from the accepted HotPathWorkBudget.
        let storage_max = 2_097_152_u64;
        if slots_bytes
            .checked_add(free_bytes)
            .is_none_or(|total| total > storage_max)
        {
            return Err(FixedStorageError::Capacity);
        }
        let mut slots = Vec::new();
        slots
            .try_reserve_exact(capacity)
            .map_err(|_| FixedStorageError::Allocation)?;
        slots.resize(capacity, CellSlot::Vacant);
        let mut free = Vec::new();
        free.try_reserve_exact(capacity)
            .map_err(|_| FixedStorageError::Allocation)?;
        for index in (0..capacity).rev() {
            free.push(index);
        }
        seal_exact_capacity(&slots, &free, capacity)?;
        Ok(Self {
            capacity,
            slots,
            free,
        })
    }
    fn capacity(&self) -> usize {
        self.capacity
    }
    fn free_len(&self) -> usize {
        self.free.len()
    }
    fn validate_selection(
        &self,
        count: usize,
        work: &mut WorkMeter,
    ) -> Result<(), FixedStorageError> {
        work.record(WorkDimension::InvariantChecks, 1)?;
        if count > self.free.len() {
            return Err(FixedStorageError::Capacity);
        }
        for index in self.free.iter().rev().take(count) {
            work.record(WorkDimension::VisitedEntities, 1)?;
            work.record(WorkDimension::InvariantChecks, 1)?;
            if !matches!(self.slots[*index], CellSlot::Vacant) {
                return Err(FixedStorageError::NonCanonical);
            }
        }
        Ok(())
    }
    fn install(&mut self, owner: usize, cells: &[OutstandingCreditCell]) -> (usize, usize) {
        let count = cells.len();
        let mut head = None;
        for cell in cells.iter().rev() {
            let index = self.free.pop().expect("prevalidated free capacity");
            let next = head;
            self.slots[index] = CellSlot::Occupied {
                owner_record: owner,
                cell: *cell,
                next_owned: next,
            };
            head = Some(index);
        }
        (head.expect("nonempty chain"), count)
    }
    fn validate_chain(
        &self,
        head: usize,
        count: usize,
        owner: usize,
        cells: &[OutstandingCreditCell],
        work: &mut WorkMeter,
    ) -> Result<(), FixedStorageError> {
        work.record(WorkDimension::InvariantChecks, 1)?;
        if count != cells.len() {
            return Err(FixedStorageError::NonCanonical);
        }
        let mut slot = Some(head);
        for (i, expected) in cells.iter().enumerate() {
            work.record(WorkDimension::VisitedEntities, 1)?;
            let index = slot.ok_or(FixedStorageError::NonCanonical)?;
            check!(
                work,
                index < self.slots.len(),
                FixedStorageError::NonCanonical
            )?;
            work.record(WorkDimension::InvariantChecks, 1)?;
            let CellSlot::Occupied {
                owner_record,
                cell,
                next_owned,
            } = self.slots[index]
            else {
                return Err(FixedStorageError::NonCanonical);
            };
            check!(work, owner_record == owner, FixedStorageError::NonCanonical)?;
            check!(work, cell == *expected, FixedStorageError::NonCanonical)?;
            check!(
                work,
                next_owned.is_some() == (i + 1 != count),
                FixedStorageError::NonCanonical
            )?;
            slot = next_owned;
        }
        Ok(())
    }
    /// Metered owner-chain traversal without expected cell values: verifies
    /// every slot of the exact `count`-length chain is occupied by `owner` with
    /// a valid next link. Used by pristine withdrawal, which releases the chain
    /// after the full before-image is established.
    fn validate_owner_chain(
        &self,
        head: usize,
        count: usize,
        owner: usize,
        work: &mut WorkMeter,
    ) -> Result<(), FixedStorageError> {
        work.record(WorkDimension::InvariantChecks, 1)?;
        let mut slot = Some(head);
        for i in 0..count {
            work.record(WorkDimension::VisitedEntities, 1)?;
            let index = slot.ok_or(FixedStorageError::NonCanonical)?;
            check!(
                work,
                index < self.slots.len(),
                FixedStorageError::NonCanonical
            )?;
            work.record(WorkDimension::InvariantChecks, 1)?;
            let CellSlot::Occupied {
                owner_record,
                next_owned,
                ..
            } = self.slots[index]
            else {
                return Err(FixedStorageError::NonCanonical);
            };
            check!(work, owner_record == owner, FixedStorageError::NonCanonical)?;
            check!(
                work,
                next_owned.is_some() == (i + 1 != count),
                FixedStorageError::NonCanonical
            )?;
            slot = next_owned;
        }
        Ok(())
    }
    fn release(&mut self, head: usize, count: usize) {
        let mut slot = Some(head);
        for _ in 0..count {
            let index = slot.expect("validated chain length");
            let next_owned = match self.slots[index] {
                CellSlot::Occupied { next_owned, .. } => next_owned,
                CellSlot::Vacant => unreachable!("validated chain slot"),
            };
            self.slots[index] = CellSlot::Vacant;
            self.free.push(index);
            slot = next_owned;
        }
    }
}
/// Fail-closed exact-capacity seal: both backing Vecs must hold exactly
/// `capacity` slots, never more. `try_reserve_exact` only guarantees at least
/// the requested capacity, so a successful arena must be exactly `C`; anything
/// over-capacity is rejected deterministically, independent of allocator policy.
fn seal_exact_capacity(
    slots: &Vec<CellSlot>,
    free: &Vec<usize>,
    capacity: usize,
) -> Result<(), FixedStorageError> {
    (slots.capacity() == capacity && free.capacity() == capacity)
        .then_some(())
        .ok_or(FixedStorageError::Capacity)
}
/// One-byte canonical type tag followed by a 32-byte identity. Equal raw
/// identity bytes in distinct namespaces are distinct tagged keys.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct TaggedKey {
    tag: u8,
    identity: [u8; 32],
}
impl TaggedKey {
    fn new(tag: u8, identity: [u8; 32]) -> Self {
        Self { tag, identity }
    }
}
/// An occupied Patricia leaf: one tagged identity and its owner record index.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct IdentityLeaf {
    key: TaggedKey,
    record: u32,
}
/// An occupied compressed Patricia branch: one strictly increasing
/// discriminating bit and two child node references. A child is a leaf index
/// or `BRANCH_TAG | branch index`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct IdentityBranch {
    bit: u16,
    zero: u32,
    one: u32,
}
/// Tagged-identity compressed Patricia trie over `33 * 8 = 264` key bits:
/// one tag byte plus 32 identity bytes.
const IDENTITY_BITS: u16 = 33 * 8;
/// Node-reference tag bit distinguishing a branch reference from a leaf index.
const BRANCH_TAG: u32 = 1 << 31;
/// Empty-tree root sentinel.
const NO_NODE: u32 = u32::MAX;
/// Private fixed reusable tagged-identity Patricia index owned conceptually by
/// the Support Charge Ledger's request-bundle store. Constructor-preallocated
/// exact-capacity leaf/branch slot Vecs with LIFO free-index stacks, no
/// hot-path growth, no public seam. `I` leaf slots and `J = I - 1` branch
/// slots; every slot starts Vacant and each free stack initially holds its
/// full index domain.
#[derive(Clone, Debug, Eq, PartialEq)]
struct TaggedIdentityIndex {
    leaf_capacity: usize,
    branch_capacity: usize,
    leaf_slots: Vec<Option<IdentityLeaf>>,
    branch_slots: Vec<Option<IdentityBranch>>,
    free_leaves: Vec<u32>,
    free_branches: Vec<u32>,
    root: u32,
}
impl TaggedIdentityIndex {
    /// Checked physical storage bytes for `leaf_capacity` leaf slots and
    /// branch slots plus their LIFO free stacks, against the binary
    /// Storage/CopiedBytes maximum.
    fn storage_bytes(leaf_capacity: u64) -> Option<u64> {
        let leaf_slot_bytes = std::mem::size_of::<Option<IdentityLeaf>>() as u64;
        let branch_slot_bytes = std::mem::size_of::<Option<IdentityBranch>>() as u64;
        let index_bytes = std::mem::size_of::<u32>() as u64;
        let branches = leaf_capacity.checked_sub(1)?;
        let total = leaf_capacity
            .checked_mul(leaf_slot_bytes)?
            .checked_add(leaf_capacity.checked_mul(index_bytes)?)?
            .checked_add(branches.checked_mul(branch_slot_bytes)?)?
            .checked_add(branches.checked_mul(index_bytes)?)?;
        (total <= 2_097_152).then_some(total)
    }
    /// Creates exact-capacity storage for `I` identity leaves and `I - 1`
    /// Patricia branches. Checked storage arithmetic rejects any capacity whose
    /// physical slots and free stacks exceed the binary Storage/CopiedBytes
    /// maximum; every backing Vec must seal to its exact requested capacity.
    fn try_new(leaf_capacity: usize) -> Result<Self, FixedStorageError> {
        let branch_capacity = leaf_capacity
            .checked_sub(1)
            .ok_or(FixedStorageError::Capacity)?;
        let leaf_capacity_u64 =
            u64::try_from(leaf_capacity).map_err(|_| FixedStorageError::Allocation)?;
        let branch_capacity_u64 =
            u64::try_from(branch_capacity).map_err(|_| FixedStorageError::Allocation)?;
        let leaf_slot_bytes = std::mem::size_of::<Option<IdentityLeaf>>() as u64;
        let branch_slot_bytes = std::mem::size_of::<Option<IdentityBranch>>() as u64;
        let index_bytes = std::mem::size_of::<u32>() as u64;
        let leaf_storage = leaf_capacity_u64
            .checked_mul(leaf_slot_bytes)
            .and_then(|slots| {
                leaf_capacity_u64
                    .checked_mul(index_bytes)
                    .and_then(|free| slots.checked_add(free))
            })
            .ok_or(FixedStorageError::Allocation)?;
        let branch_storage = branch_capacity_u64
            .checked_mul(branch_slot_bytes)
            .and_then(|slots| {
                branch_capacity_u64
                    .checked_mul(index_bytes)
                    .and_then(|free| slots.checked_add(free))
            })
            .ok_or(FixedStorageError::Allocation)?;
        let storage = leaf_storage
            .checked_add(branch_storage)
            .ok_or(FixedStorageError::Allocation)?;
        // Binary Storage/CopiedBytes maximum from the accepted HotPathWorkBudget.
        let storage_max = 2_097_152_u64;
        if storage > storage_max {
            return Err(FixedStorageError::Capacity);
        }
        // Node references encode branch nodes with a tag bit, so every leaf and
        // branch index must stay below the tagged reference domain.
        if leaf_capacity >= BRANCH_TAG as usize || branch_capacity >= BRANCH_TAG as usize {
            return Err(FixedStorageError::Capacity);
        }
        if leaf_capacity > isize::MAX as usize || branch_capacity > isize::MAX as usize {
            return Err(FixedStorageError::Allocation);
        }
        let mut leaf_slots = Vec::new();
        leaf_slots
            .try_reserve_exact(leaf_capacity)
            .map_err(|_| FixedStorageError::Allocation)?;
        leaf_slots.resize(leaf_capacity, None);
        let mut branch_slots = Vec::new();
        branch_slots
            .try_reserve_exact(branch_capacity)
            .map_err(|_| FixedStorageError::Allocation)?;
        branch_slots.resize(branch_capacity, None);
        let mut free_leaves = Vec::new();
        free_leaves
            .try_reserve_exact(leaf_capacity)
            .map_err(|_| FixedStorageError::Allocation)?;
        for index in (0..leaf_capacity).rev() {
            free_leaves.push(index as u32);
        }
        let mut free_branches = Vec::new();
        free_branches
            .try_reserve_exact(branch_capacity)
            .map_err(|_| FixedStorageError::Allocation)?;
        for index in (0..branch_capacity).rev() {
            free_branches.push(index as u32);
        }
        if leaf_slots.capacity() != leaf_capacity
            || branch_slots.capacity() != branch_capacity
            || free_leaves.capacity() != leaf_capacity
            || free_branches.capacity() != branch_capacity
        {
            return Err(FixedStorageError::Capacity);
        }
        Ok(Self {
            leaf_capacity,
            branch_capacity,
            leaf_slots,
            branch_slots,
            free_leaves,
            free_branches,
            root: NO_NODE,
        })
    }
    fn leaf_capacity(&self) -> usize {
        self.leaf_capacity
    }
    fn branch_capacity(&self) -> usize {
        self.branch_capacity
    }
    fn free_leaf_len(&self) -> usize {
        self.free_leaves.len()
    }
    fn free_branch_len(&self) -> usize {
        self.free_branches.len()
    }
    fn is_empty(&self) -> bool {
        self.root == NO_NODE
    }
    /// Borrowed bounded lookup: follows at most `B = 264` branches plus one
    /// leaf, independent of `E`. No key copy and no allocation; each visited
    /// branch and the visited leaf charge one VisitedEntities and one
    /// InvariantChecks.
    fn find(
        &self,
        tag: u8,
        identity: &[u8; 32],
        work: &mut WorkMeter,
    ) -> Result<Option<u32>, FixedStorageError> {
        let node = self.locate(tag, identity, work)?;
        if node == NO_NODE {
            return Ok(None);
        }
        let leaf = self.leaf_slots[node as usize]
            .as_ref()
            .expect("validated occupied leaf slot");
        Ok((leaf.key.tag == tag && leaf.key.identity == *identity).then_some(leaf.record))
    }
    /// Walks to the terminal node for a borrowed key, charging one
    /// VisitedEntities and one InvariantChecks per visited branch and one of
    /// each for the visited leaf. Returns the leaf index, or `NO_NODE` for an
    /// empty tree.
    fn locate(
        &self,
        tag: u8,
        identity: &[u8; 32],
        work: &mut WorkMeter,
    ) -> Result<u32, FixedStorageError> {
        let mut node = self.root;
        while node != NO_NODE && is_branch(node) {
            work.record(WorkDimension::VisitedEntities, 1)?;
            work.record(WorkDimension::InvariantChecks, 1)?;
            let branch = self.branch_slots[branch_index(node)]
                .as_ref()
                .expect("validated occupied branch slot");
            node = [branch.zero, branch.one][identity_bit(tag, identity, branch.bit)];
        }
        if node == NO_NODE {
            return Ok(NO_NODE);
        }
        work.record(WorkDimension::VisitedEntities, 1)?;
        work.record(WorkDimension::InvariantChecks, 1)?;
        Ok(node)
    }
    /// Metered prevalidated insertion: proves leaf and branch capacity and
    /// key absence, meters the peer traversal, the first-difference byte pass,
    /// and the insertion-point traversal, and only then installs the leaf and
    /// branch infallibly. Rejection preserves the exact trie and free stacks.
    fn insert(
        &mut self,
        key: TaggedKey,
        record: u32,
        work: &mut WorkMeter,
    ) -> Result<(), FixedStorageError> {
        let peer = self.locate(key.tag, &key.identity, work)?;
        if peer == NO_NODE {
            work.record(WorkDimension::InvariantChecks, 1)?;
            if self.free_leaves.is_empty() {
                return Err(FixedStorageError::Capacity);
            }
            self.install_root(key, record);
            return Ok(());
        }
        let peer_key = self.leaf_slots[peer as usize]
            .as_ref()
            .expect("validated occupied leaf slot")
            .key;
        if peer_key == key {
            return Err(FixedStorageError::Duplicate);
        }
        work.record(WorkDimension::InvariantChecks, 1)?;
        if self.free_leaves.is_empty() {
            return Err(FixedStorageError::Capacity);
        }
        work.record(WorkDimension::InvariantChecks, 1)?;
        if self.free_branches.is_empty() {
            return Err(FixedStorageError::Capacity);
        }
        let (bit, bytes) = first_difference(&key, &peer_key);
        work.record(WorkDimension::VisitedEntities, bytes)?;
        let mut visits = 0u64;
        let (mut parent, mut child) = (NO_NODE, self.root);
        while is_branch(child) {
            visits += 1;
            let branch = self.branch_slots[branch_index(child)]
                .as_ref()
                .expect("validated occupied branch slot");
            if branch.bit >= bit {
                break;
            }
            parent = child;
            child = [branch.zero, branch.one][identity_bit(key.tag, &key.identity, branch.bit)];
        }
        work.record(WorkDimension::VisitedEntities, visits)?;
        work.record(WorkDimension::InvariantChecks, visits)?;
        self.install_branch(key, record, bit, parent, child);
        Ok(())
    }
    /// Infallible installation of the first leaf into an empty tree.
    fn install_root(&mut self, key: TaggedKey, record: u32) {
        let leaf = self.free_leaves.pop().expect("prevalidated leaf capacity");
        self.leaf_slots[leaf as usize] = Some(IdentityLeaf { key, record });
        self.root = leaf;
    }
    /// Infallible installation of one leaf plus one branch above `child`, whose
    /// first discriminating bit `bit` is strictly larger than every parent bit.
    fn install_branch(&mut self, key: TaggedKey, record: u32, bit: u16, parent: u32, child: u32) {
        let leaf = self.free_leaves.pop().expect("prevalidated leaf capacity");
        self.leaf_slots[leaf as usize] = Some(IdentityLeaf { key, record });
        let branch = self
            .free_branches
            .pop()
            .expect("prevalidated branch capacity");
        let children = if identity_bit(key.tag, &key.identity, bit) == 0 {
            [leaf, child]
        } else {
            [child, leaf]
        };
        let branch_node = BRANCH_TAG | branch;
        self.branch_slots[branch as usize] = Some(IdentityBranch {
            bit,
            zero: children[0],
            one: children[1],
        });
        if parent == NO_NODE {
            self.root = branch_node;
        } else {
            let parent = self.branch_slots[branch_index(parent)]
                .as_mut()
                .expect("validated occupied parent branch");
            *[&mut parent.zero, &mut parent.one]
                [identity_bit(key.tag, &key.identity, parent.bit)] = branch_node;
        }
    }
    /// Metered prevalidated removal of one tagged identity: meters the bounded
    /// leaf-locating traversal with grandparent/parent/sibling tracking and
    /// splices the sibling over the parent only after the complete before-image
    /// is established. An absent key returns `None` without mutation; every
    /// removed leaf and, when present, parent branch returns to its free stack.
    fn remove(
        &mut self,
        tag: u8,
        identity: &[u8; 32],
        work: &mut WorkMeter,
    ) -> Result<Option<u32>, FixedStorageError> {
        let mut grandparent = NO_NODE;
        let mut parent = NO_NODE;
        let mut sibling = NO_NODE;
        let mut node = self.root;
        while node != NO_NODE && is_branch(node) {
            work.record(WorkDimension::VisitedEntities, 1)?;
            work.record(WorkDimension::InvariantChecks, 1)?;
            let branch = self.branch_slots[branch_index(node)]
                .as_ref()
                .expect("validated occupied branch slot");
            let children = [branch.zero, branch.one];
            let bit = identity_bit(tag, identity, branch.bit);
            sibling = children[1 - bit];
            grandparent = parent;
            parent = node;
            node = children[bit];
        }
        if node == NO_NODE {
            return Ok(None);
        }
        work.record(WorkDimension::VisitedEntities, 1)?;
        work.record(WorkDimension::InvariantChecks, 1)?;
        let leaf = self.leaf_slots[node as usize]
            .as_ref()
            .expect("validated occupied leaf slot");
        if leaf.key.tag != tag || leaf.key.identity != *identity {
            return Ok(None);
        }
        let record = leaf.record;
        self.splice(grandparent, parent, sibling, node);
        Ok(Some(record))
    }
    /// Infallible splice: removes `leaf` by splicing `sibling` over its parent
    /// branch, returning the leaf and, when present, the parent branch to the
    /// free stacks. The last tree leaf needs no branch.
    fn splice(&mut self, grandparent: u32, parent: u32, sibling: u32, leaf: u32) {
        if parent == NO_NODE {
            self.root = NO_NODE;
        } else if grandparent == NO_NODE {
            self.root = sibling;
            let branch = branch_index(parent);
            self.branch_slots[branch] = None;
            self.free_branches.push(branch as u32);
        } else {
            let grandparent = self.branch_slots[branch_index(grandparent)]
                .as_mut()
                .expect("validated occupied grandparent branch");
            if grandparent.zero == parent {
                grandparent.zero = sibling;
            } else {
                grandparent.one = sibling;
            }
            let branch = branch_index(parent);
            self.branch_slots[branch] = None;
            self.free_branches.push(branch as u32);
        }
        self.leaf_slots[leaf as usize] = None;
        self.free_leaves.push(leaf);
    }
}
fn is_branch(node: u32) -> bool {
    node & BRANCH_TAG != 0
}
fn branch_index(node: u32) -> usize {
    (node & !BRANCH_TAG) as usize
}
fn identity_bit(tag: u8, identity: &[u8; 32], bit: u16) -> usize {
    let byte = if bit < 8 {
        tag
    } else {
        identity[(bit / 8 - 1) as usize]
    };
    ((byte >> (7 - bit % 8)) & 1) as usize
}
/// First differing bit between two distinct tagged identities, plus the number
/// of compared key bytes (the tag byte and every equal identity byte through
/// the first differing one).
fn first_difference(left: &TaggedKey, right: &TaggedKey) -> (u16, u64) {
    if left.tag != right.tag {
        let difference = left.tag ^ right.tag;
        return (difference.leading_zeros() as u16, 1);
    }
    for (index, (left, right)) in left.identity.iter().zip(right.identity).enumerate() {
        let difference = left ^ right;
        if difference != 0 {
            let bit = 8 + index as u16 * 8 + difference.leading_zeros() as u16;
            return (bit, index as u64 + 2);
        }
    }
    unreachable!("distinct tagged identities have distinct bytes")
}
/// Fixed tagged-identity count `K = 11` per complete C16 request bundle: three
/// operation obligations, three physical credits, three request-owned
/// `AdmissionInitial` claims, one Future Turn Support Entitlement, and one
/// Support Outstanding Credit Vector.
const K: usize = 11;
/// Canonical one-byte type tags for the C16 tagged identity namespaces. Tags 0
/// and 1 match the legacy obligation and physical-credit tags so reciprocal
/// collision checks share one encoding.
const TAG_OBLIGATION: u8 = 0;
const TAG_CREDIT: u8 = 1;
const TAG_ADMISSION_CLAIM: u8 = 2;
const TAG_ENTITLEMENT: u8 = 4;
const TAG_VECTOR: u8 = 5;
/// Live or retained-tombstone state of one request-bundle record. A retained
/// tombstone keeps its record, identities, cells, and claims.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BundleRecordState {
    Live,
    RetainedTombstone,
}
/// One fixed request-bundle record: three operation-specific initial/release
/// obligations, three physical credits, three distinct request-owned
/// `AdmissionInitial` claims, one Future Turn Support Entitlement, one
/// Support Outstanding Credit Vector with its occupied cell-chain head and
/// validated length, and the live/tombstone state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct BundleRecord {
    obligations: [SupportOperationObligationId; 3],
    credits: [PhysicalStartCreditId; 3],
    claims: [[u8; 32]; 3],
    entitlement: FutureTurnSupportEntitlementId,
    vector: SupportOutstandingCreditVectorId,
    vector_head: Option<usize>,
    vector_len: usize,
    state: BundleRecordState,
}
impl BundleRecord {
    fn tagged_keys(&self) -> [TaggedKey; K] {
        [
            TaggedKey::new(TAG_OBLIGATION, self.obligations[0].get()),
            TaggedKey::new(TAG_OBLIGATION, self.obligations[1].get()),
            TaggedKey::new(TAG_OBLIGATION, self.obligations[2].get()),
            TaggedKey::new(TAG_CREDIT, self.credits[0].get()),
            TaggedKey::new(TAG_CREDIT, self.credits[1].get()),
            TaggedKey::new(TAG_CREDIT, self.credits[2].get()),
            TaggedKey::new(TAG_ADMISSION_CLAIM, self.claims[0]),
            TaggedKey::new(TAG_ADMISSION_CLAIM, self.claims[1]),
            TaggedKey::new(TAG_ADMISSION_CLAIM, self.claims[2]),
            TaggedKey::new(TAG_ENTITLEMENT, self.entitlement.get()),
            TaggedKey::new(TAG_VECTOR, self.vector.get()),
        ]
    }
}
/// Private fixed reusable request-bundle store owned conceptually by the
/// Support Charge Ledger. One fixed record per C16 bundle plus its `K = 11`
/// tagged identity leaves and its entitlement cells; every slot
/// constructor-preallocated with exact-capacity free-index stacks, no hot-path
/// growth, no public seam.
#[derive(Clone, Debug, Eq, PartialEq)]
struct RequestBundleStore {
    record_capacity: usize,
    records: Vec<Option<BundleRecord>>,
    free_records: Vec<u32>,
    identities: TaggedIdentityIndex,
    cells: EntitlementCellArena,
}
impl RequestBundleStore {
    /// Creates exact-capacity storage for `E` request-bundle records, `I = 11E`
    /// tagged identity leaves, `J = I - 1` Patricia branches, and `C`
    /// entitlement cells. Every storage product and sum is checked against the
    /// binary Storage/CopiedBytes maximum and every backing Vec must seal to
    /// its exact requested capacity. Construction fails closed without complete
    /// validated production capacity facts.
    fn try_new(record_capacity: usize, cell_capacity: usize) -> Result<Self, FixedStorageError> {
        if record_capacity == 0 || cell_capacity == 0 {
            return Err(FixedStorageError::Capacity);
        }
        let leaf_capacity = record_capacity
            .checked_mul(K)
            .ok_or(FixedStorageError::Allocation)?;
        let record_bytes = std::mem::size_of::<Option<BundleRecord>>() as u64;
        let index_bytes = std::mem::size_of::<u32>() as u64;
        let record_capacity_u64 =
            u64::try_from(record_capacity).map_err(|_| FixedStorageError::Allocation)?;
        let leaf_capacity_u64 =
            u64::try_from(leaf_capacity).map_err(|_| FixedStorageError::Allocation)?;
        let cell_capacity_u64 =
            u64::try_from(cell_capacity).map_err(|_| FixedStorageError::Allocation)?;
        let record_storage = record_capacity_u64
            .checked_mul(record_bytes)
            .and_then(|slots| {
                record_capacity_u64
                    .checked_mul(index_bytes)
                    .and_then(|free| slots.checked_add(free))
            })
            .ok_or(FixedStorageError::Allocation)?;
        let identity_storage = TaggedIdentityIndex::storage_bytes(leaf_capacity_u64)
            .ok_or(FixedStorageError::Allocation)?;
        let cell_storage = EntitlementCellArena::storage_bytes(cell_capacity_u64)
            .ok_or(FixedStorageError::Allocation)?;
        let storage = record_storage
            .checked_add(identity_storage)
            .and_then(|sum| sum.checked_add(cell_storage))
            .ok_or(FixedStorageError::Allocation)?;
        // Binary Storage/CopiedBytes maximum from the accepted HotPathWorkBudget.
        let storage_max = 2_097_152_u64;
        if storage > storage_max {
            return Err(FixedStorageError::Capacity);
        }
        if record_capacity > isize::MAX as usize {
            return Err(FixedStorageError::Allocation);
        }
        let mut records = Vec::new();
        records
            .try_reserve_exact(record_capacity)
            .map_err(|_| FixedStorageError::Allocation)?;
        records.resize(record_capacity, None);
        let mut free_records = Vec::new();
        free_records
            .try_reserve_exact(record_capacity)
            .map_err(|_| FixedStorageError::Allocation)?;
        for index in (0..record_capacity).rev() {
            free_records.push(index as u32);
        }
        if records.capacity() != record_capacity || free_records.capacity() != record_capacity {
            return Err(FixedStorageError::Capacity);
        }
        Ok(Self {
            record_capacity,
            records,
            free_records,
            identities: TaggedIdentityIndex::try_new(leaf_capacity)?,
            cells: EntitlementCellArena::try_new(cell_capacity)?,
        })
    }
    fn record_capacity(&self) -> usize {
        self.record_capacity
    }
    fn record_len(&self) -> usize {
        self.records.iter().filter(|slot| slot.is_some()).count()
    }
    fn free_record_len(&self) -> usize {
        self.free_records.len()
    }
    fn free_cell_len(&self) -> usize {
        self.cells.free_len()
    }
    fn free_leaf_len(&self) -> usize {
        self.identities.free_leaf_len()
    }
    fn free_branch_len(&self) -> usize {
        self.identities.free_branch_len()
    }
    fn is_empty(&self) -> bool {
        self.record_len() == 0
    }
    fn find(
        &self,
        tag: u8,
        identity: &[u8; 32],
        work: &mut WorkMeter,
    ) -> Result<Option<u32>, FixedStorageError> {
        self.identities.find(tag, identity, work)
    }
    fn get_record(&self, index: u32) -> Option<&BundleRecord> {
        self.records.get(index as usize).and_then(Option::as_ref)
    }
    /// Metered prevalidated bundle reserve: proves all `K` tagged identities
    /// absent, checks record/cell/leaf/branch capacity, and preflights the
    /// complete insertion Work envelope before any mutation. The infallible
    /// installation then pops one record slot, `K` leaf slots, and exactly
    /// `K - 1` branches when the tree was empty or `K` branches otherwise,
    /// performs standard first-differing-bit insertion, and installs the
    /// validated cells as one owned chain.
    fn reserve_bundle(
        &mut self,
        record: &BundleRecord,
        cells: &[OutstandingCreditCell],
        work: &mut WorkMeter,
    ) -> Result<u32, FixedStorageError> {
        work.record(WorkDimension::InvariantChecks, 1)?;
        if self.free_records.is_empty() {
            return Err(FixedStorageError::Capacity);
        }
        work.record(WorkDimension::InvariantChecks, 1)?;
        if cells.len() > self.cells.free_len() {
            return Err(FixedStorageError::Capacity);
        }
        self.cells.validate_selection(cells.len(), work)?;
        work.record(WorkDimension::InvariantChecks, 1)?;
        if self.identities.free_leaf_len() < K {
            return Err(FixedStorageError::Capacity);
        }
        let branch_need = if self.identities.is_empty() { K - 1 } else { K };
        work.record(WorkDimension::InvariantChecks, 1)?;
        if self.identities.free_branch_len() < branch_need {
            return Err(FixedStorageError::Capacity);
        }
        for key in record.tagged_keys() {
            if self
                .identities
                .find(key.tag, &key.identity, work)?
                .is_some()
            {
                return Err(FixedStorageError::Duplicate);
            }
        }
        // Conservative insertion envelope: per identity one peer traversal, one
        // insertion traversal, and a 33-byte first-difference pass.
        let bits = u64::from(IDENTITY_BITS);
        work.ensure(HotPathWorkWitness::new([
            K as u64 * (2 * bits + 35),
            0,
            0,
            0,
            K as u64 * (2 * bits + 4),
        ]))?;
        let record_index = self
            .free_records
            .pop()
            .expect("prevalidated record capacity");
        for key in record.tagged_keys() {
            self.identities
                .insert(key, record_index, work)
                .expect("insertion Work fully preflighted");
        }
        let (head, len) = self.cells.install(record_index as usize, cells);
        self.records[record_index as usize] = Some(BundleRecord {
            vector_head: Some(head),
            vector_len: len,
            ..*record
        });
        Ok(record_index)
    }
    /// Metered prevalidated pristine withdrawal: proves the exact record,
    /// traverses and checks its owned cell chain, preflights the complete
    /// removal Work envelope, then removes each of the `K` identity leaves by
    /// splicing its sibling over its parent, releases the `v` cells, vacates
    /// the record, and returns every slot to its matching free stack.
    fn withdraw_bundle(
        &mut self,
        record: u32,
        work: &mut WorkMeter,
    ) -> Result<(), FixedStorageError> {
        let record_slot = self
            .records
            .get(record as usize)
            .and_then(Option::as_ref)
            .ok_or(FixedStorageError::NonCanonical)?;
        let head = record_slot
            .vector_head
            .ok_or(FixedStorageError::NonCanonical)?;
        let len = record_slot.vector_len;
        self.cells
            .validate_owner_chain(head, len, record as usize, work)?;
        let bits = u64::from(IDENTITY_BITS);
        work.ensure(HotPathWorkWitness::new([
            K as u64 * (bits + 1),
            0,
            0,
            0,
            K as u64 * (bits + 1),
        ]))?;
        for key in record_slot.tagged_keys() {
            let removed = self
                .identities
                .remove(key.tag, &key.identity, work)
                .expect("removal Work fully preflighted");
            debug_assert_eq!(removed, Some(record));
        }
        self.cells.release(head, len);
        self.records[record as usize] = None;
        self.free_records.push(record);
        Ok(())
    }
    /// Metered transition to the retained-tombstone state: the record, its
    /// identities, cells, and claims all remain occupied.
    fn retain_bundle(
        &mut self,
        record: u32,
        work: &mut WorkMeter,
    ) -> Result<(), FixedStorageError> {
        work.record(WorkDimension::InvariantChecks, 1)?;
        let slot = self
            .records
            .get_mut(record as usize)
            .and_then(Option::as_mut)
            .ok_or(FixedStorageError::NonCanonical)?;
        if slot.state != BundleRecordState::Live {
            return Err(FixedStorageError::NonCanonical);
        }
        slot.state = BundleRecordState::RetainedTombstone;
        Ok(())
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
        Ledger::try_new(generation, capacities, 2, starts, maxima, 4, 8).unwrap()
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
            HotPathWorkWitness::new([2, 288, 0, 0, 22])
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

    #[test]
    fn lifecycle_batch_constructor_rejects_unbounded_work() {
        type Wide = SupportChargeLedger<65_536, 65_536, 3>;
        let generation = SupportLedgerGeneration::new(1).unwrap();
        let capacities = [[1, 2, 1], [0, 1, 0], [1, 2, 1], [1, 4, 1], [1, 4, 1]];
        let starts = std::array::from_fn(|_| {
            [
                FixedStartCountBound(Duration::from_micros(10), 1),
                FixedStartCountBound(Duration::from_micros(20), 1),
                FixedStartCountBound(Duration::from_micros(30), 1),
            ]
        });
        let build = |maxima| Wide::try_new(generation, capacities, 2, starts, maxima, 4, 8);
        // Exact envelope boundary: M_L = max(1+50, 50, 1, 1) = 51 fits every
        // binary Work maximum, so the constructor accepts it.
        build(LifecycleReserveMaxima([1, 50, 50, 1, 1])).unwrap();
        // M_L = 52 exceeds the worst-case InvariantChecks envelope: reject.
        assert_eq!(
            build(LifecycleReserveMaxima([1, 51, 51, 1, 1])).unwrap_err(),
            SupportLedgerError::InvalidInput
        );
        // A u16::MAX-clamped batch can never fit a binary maximum: checked
        // derivation rejects without overflow or reuse.
        assert_eq!(
            build(LifecycleReserveMaxima([u16::MAX; 5])).unwrap_err(),
            SupportLedgerError::InvalidInput
        );
    }

    #[test]
    fn lifecycle_batch_bound_rejects_over_maximum_before_shape() {
        let mut ledger = new_ledger();
        for capacity in &mut ledger.capacities {
            capacity[1] = 4;
        }
        let specs: Vec<_> = (1..=4)
            .map(|n| LifecycleReserveSpec {
                id: SupportOperationObligationId::new([n; 32]).unwrap(),
                kind: if n == 1 {
                    LifecycleReserveKind::PostLoadModelDescription
                } else {
                    LifecycleReserveKind::PostLoadRequestDescription
                },
                physical_credit: PhysicalStartCreditId::new([n + 20; 32]).unwrap(),
                predecessor: SupportCausalPredecessorId([90; 32]),
                scope: SupportCallScopeId([n + 40; 32]),
                claim: SupportFundingClaim::LifecycleReserve([n; 32]),
                expires_at: None,
            })
            .collect();
        let mut measured = work();
        let before = ledger.generation();
        let result = ledger.reserve_lifecycle(
            ledger.generation(),
            MonotonicTime::from_micros(1),
            &specs,
            &mut measured,
        );
        // The batch bound (M_L = 3) rejects before any per-member shape work.
        assert_eq!(result, Err(InvalidInput));
        assert_eq!(ledger.generation(), before);
        assert_eq!(measured.witness().value(WorkDimension::VisitedEntities), 0);
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

    /// Test-only complete bundle input over `n`-derived canonical identities.
    fn bundle_input<'a, const V: usize>(
        n: u8,
        cells: &'a [OutstandingCreditCell],
    ) -> RequestBundleInput<'a, V> {
        let identity = |offset: u8| {
            let mut id = [0u8; 32];
            id[0] = n;
            id[1] = offset;
            id
        };
        let view = SupportOutstandingCreditVector::<V>::try_new(cells, &mut work()).unwrap();
        RequestBundleInput {
            obligations: [
                SupportOperationObligationId::new(identity(1)).unwrap(),
                SupportOperationObligationId::new(identity(2)).unwrap(),
                SupportOperationObligationId::new(identity(3)).unwrap(),
            ],
            credits: [
                PhysicalStartCreditId::new(identity(11)).unwrap(),
                PhysicalStartCreditId::new(identity(12)).unwrap(),
                PhysicalStartCreditId::new(identity(13)).unwrap(),
            ],
            claims: [identity(21), identity(22), identity(23)],
            entitlement: FutureTurnSupportEntitlementId::new(identity(31)).unwrap(),
            vector: SupportOutstandingCreditVectorId::new(identity(41)).unwrap(),
            cells: view,
        }
    }
    fn bundle_ledger(records: usize, cells: usize) -> Ledger {
        let generation = SupportLedgerGeneration::new(1).unwrap();
        let capacities = [[1, 2, 1], [0, 1, 0], [1, 2, 1], [1, 4, 1], [1, 4, 1]];
        let starts = [[FixedStartCountBound(Duration::from_micros(10), 1); 1]; 21];
        let maxima = LifecycleReserveMaxima([1, 2, 2, 1, 1]);
        Ledger::try_new(generation, capacities, 2, starts, maxima, records, cells).unwrap()
    }
    fn bundle_snapshot(ledger: &Ledger) -> (SupportLedgerGeneration, usize, usize, usize, usize) {
        (
            ledger.generation(),
            ledger.bundles.free_record_len(),
            ledger.bundles.free_cell_len(),
            ledger.bundles.free_leaf_len(),
            ledger.bundles.free_branch_len(),
        )
    }
    #[test]
    fn c16_instance_nonce_issuance() {
        let dispenser = AtomicU64::new(0);
        assert_eq!(issue_instance_nonce(&dispenser), Some(1));
        assert_eq!(issue_instance_nonce(&dispenser), Some(2));
        assert_eq!(dispenser.load(Ordering::Relaxed), 2);
        let dispenser = AtomicU64::new(u64::MAX - 1);
        assert_eq!(issue_instance_nonce(&dispenser), Some(u64::MAX));
        assert_eq!(issue_instance_nonce(&dispenser), None);
        assert_eq!(dispenser.load(Ordering::Relaxed), u64::MAX);
        let dispenser = AtomicU64::new(u64::MAX);
        assert_eq!(issue_instance_nonce(&dispenser), None);
        assert_eq!(dispenser.load(Ordering::Relaxed), u64::MAX);
    }
    #[test]
    fn c16_bundle_prepare_binds_exact_before_image() {
        let ledger = new_ledger();
        let before = ledger.generation();
        let cells = axis_cells(3, 1);
        let input = bundle_input::<8>(1, &cells);
        let mut measured = work();
        let change = ledger
            .prepare_bundle(before, &input, &mut measured)
            .unwrap();
        // Empty C16 and legacy indexes cost nothing: only the fixed preflight
        // (next 2, identity validation 18, extra facts 3, absence comparisons
        // 17, free capacity 4).
        assert_eq!(
            measured.witness(),
            HotPathWorkWitness::new([0, 0, 0, 0, 44])
        );
        assert_eq!(ledger.generation(), before, "prepare is read-only");
        assert_eq!(change.nonce, ledger.instance_nonce);
        assert_eq!(change.expected, before);
        assert_eq!(change.capacities, ledger.capacities);
        assert_eq!(change.usage, ledger.usage);
        assert_eq!(change.reserved, ledger.reserved);
        assert_eq!(
            (
                change.free_records,
                change.free_cells,
                change.free_leaves,
                change.free_branches
            ),
            (4, 8, 44, 43)
        );
        assert!(change.identities.iter().all(|found| found.is_none()));
        assert_eq!(change.record.vector_len, 3);
        assert_eq!(change.record.vector_head, None);
        assert_eq!(change.record.state, BundleRecordState::Live);
        assert_eq!(change.record.obligations, input.obligations);
        assert_eq!(change.record.credits, input.credits);
        assert_eq!(change.record.claims, input.claims);
        // The same input prepares again against the same before-image.
        let mut measured = work();
        ledger
            .prepare_bundle(before, &input, &mut measured)
            .unwrap();
        assert_eq!(
            measured.witness(),
            HotPathWorkWitness::new([0, 0, 0, 0, 44])
        );
    }
    #[test]
    fn c16_bundle_prepare_rejects_invalid_inputs() {
        let cells = axis_cells(3, 1);
        let mut invalid = bundle_input::<8>(1, &cells);
        invalid.claims[0] = [0; 32];
        let ledger = new_ledger();
        assert_eq!(
            ledger.prepare_bundle(ledger.generation(), &invalid, &mut work()),
            Err(InvalidInput)
        );
        invalid = bundle_input::<8>(1, &cells);
        invalid.obligations.swap(0, 1);
        assert_eq!(
            ledger.prepare_bundle(ledger.generation(), &invalid, &mut work()),
            Err(InvalidInput)
        );
        invalid = bundle_input::<8>(1, &cells);
        invalid.credits[0] = invalid.credits[1];
        assert_eq!(
            ledger.prepare_bundle(ledger.generation(), &invalid, &mut work()),
            Err(InvalidInput)
        );
        invalid = bundle_input::<8>(1, &cells);
        invalid.claims[2] = invalid.claims[1];
        assert_eq!(
            ledger.prepare_bundle(ledger.generation(), &invalid, &mut work()),
            Err(InvalidInput)
        );
    }
    #[test]
    fn c16_bundle_prepare_rejects_collisions_and_capacity() {
        let cells = axis_cells(3, 1);
        let reject = |ledger: &mut Ledger, input: &RequestBundleInput<'_, 8>| {
            let before = bundle_snapshot(ledger);
            let mut measured = work();
            let result = ledger.prepare_bundle(ledger.generation(), input, &mut measured);
            assert_eq!(
                bundle_snapshot(ledger),
                before,
                "prepare preserves exact state"
            );
            result.unwrap_err()
        };
        // A live legacy obligation blocks a matching C16 obligation.
        let mut ledger = new_ledger();
        add(&mut ledger, 1, 1).unwrap();
        let mut legacy = bundle_input::<8>(1, &cells);
        legacy.obligations[0] = SupportOperationObligationId::new([1; 32]).unwrap();
        assert_eq!(
            reject(&mut ledger, &legacy),
            SupportLedgerError::Storage(Duplicate)
        );
        // A live C16 bundle blocks every shared identity namespace.
        let mut ledger = new_ledger();
        ledger
            .bundles
            .reserve_bundle(&bundle_record(1), &axis_cells(2, 1), &mut work())
            .unwrap();
        let input = bundle_input::<8>(1, &cells);
        assert_eq!(
            reject(&mut ledger, &input),
            SupportLedgerError::Storage(Duplicate)
        );
        // A retained tombstone keeps blocking until pristine withdrawal.
        let mut ledger = new_ledger();
        ledger
            .bundles
            .reserve_bundle(&bundle_record(1), &axis_cells(2, 1), &mut work())
            .unwrap();
        ledger.bundles.retain_bundle(0, &mut work()).unwrap();
        let input = bundle_input::<8>(1, &cells);
        assert_eq!(
            reject(&mut ledger, &input),
            SupportLedgerError::Storage(Duplicate)
        );
        // Record, leaf, and branch exhaustion after E = 1 is occupied.
        let mut ledger = bundle_ledger(1, 8);
        ledger
            .bundles
            .reserve_bundle(&bundle_record(9), &axis_cells(1, 1), &mut work())
            .unwrap();
        let input = bundle_input::<8>(2, &cells);
        assert_eq!(
            reject(&mut ledger, &input),
            SupportLedgerError::Storage(FixedStorageError::Capacity)
        );
        // Cell exhaustion when the vector needs more cells than remain free.
        let mut ledger = bundle_ledger(4, 3);
        let wide = axis_cells(4, 1);
        let input = bundle_input::<8>(3, &wide);
        assert_eq!(
            reject(&mut ledger, &input),
            SupportLedgerError::Storage(FixedStorageError::Capacity)
        );
    }
    #[test]
    fn c16_bundle_prepare_work_exhaustion_rolls_back() {
        let cells = axis_cells(3, 1);
        let input = bundle_input::<8>(1, &cells);
        let ledger = new_ledger();
        let before = bundle_snapshot(&ledger);
        let mut exhausted = work();
        exhausted
            .record(WorkDimension::VisitedEntities, 1_704_575)
            .unwrap();
        let fault = ledger.prepare_bundle(ledger.generation(), &input, &mut exhausted);
        let error = WorkBudgetError::BudgetExceeded(
            WorkDimension::VisitedEntities,
            1_704_575,
            1_704_575 + 15_219,
        );
        assert_eq!(
            fault,
            Err(SupportLedgerError::Storage(FixedStorageError::Work(error)))
        );
        assert_eq!(bundle_snapshot(&ledger), before);
        let ledger = new_ledger();
        let mut exhausted = work();
        exhausted
            .record(WorkDimension::InvariantChecks, 28_707)
            .unwrap();
        let fault = ledger.prepare_bundle(ledger.generation(), &input, &mut exhausted);
        let error = WorkBudgetError::BudgetExceeded(WorkDimension::InvariantChecks, 28_708, 28_709);
        assert_eq!(
            fault,
            Err(SupportLedgerError::Storage(FixedStorageError::Work(error)))
        );
        assert_eq!(
            exhausted.witness().value(WorkDimension::InvariantChecks),
            28_708
        );
        assert_eq!(bundle_snapshot(&ledger), before);
    }

    /// Test-only oracle scanning every slot to prove the full arena partition.
    fn arena_oracle(arena: &EntitlementCellArena) {
        use std::collections::HashSet;
        let capacity = arena.capacity();
        assert!(capacity > 0);
        assert_eq!(arena.slots.len(), capacity, "slots length exact");
        assert_eq!(arena.slots.capacity(), capacity, "slots capacity sealed");
        assert_eq!(arena.free.capacity(), capacity, "free capacity sealed");
        assert!(arena.free.len() <= capacity);
        let mut free_set = HashSet::new();
        for &index in arena.free.iter() {
            assert!(index < capacity, "free index in range");
            assert!(free_set.insert(index), "free indices unique");
            assert_eq!(arena.slots[index], CellSlot::Vacant);
        }
        let mut pointed = HashSet::new();
        let mut occupied = 0usize;
        for index in 0..capacity {
            if let CellSlot::Occupied {
                next_owned: Some(next),
                ..
            } = arena.slots[index]
            {
                assert!(next < capacity, "next index in range");
                assert!(
                    pointed.insert(next),
                    "two slots share one next (not a simple chain)"
                );
            }
            if matches!(arena.slots[index], CellSlot::Occupied { .. }) {
                occupied += 1;
            }
        }
        assert_eq!(
            free_set.len() + occupied,
            capacity,
            "free disjoint_union occupied = all slots"
        );
        let mut visited = HashSet::new();
        let mut chains = 0usize;
        for index in 0..capacity {
            let CellSlot::Occupied { .. } = arena.slots[index] else {
                continue;
            };
            if pointed.contains(&index) {
                continue;
            }
            chains += 1;
            let mut slot = Some(index);
            let mut owner = None;
            let mut chain = HashSet::new();
            while let Some(current) = slot {
                assert!(current < capacity);
                assert!(chain.insert(current), "acyclic chain");
                assert!(visited.insert(current), "chain disjoint from another");
                let CellSlot::Occupied {
                    owner_record,
                    next_owned,
                    ..
                } = arena.slots[current]
                else {
                    panic!("chain reaches a Vacant slot");
                };
                assert!(
                    owner.is_none() || owner == Some(owner_record),
                    "single owner per chain"
                );
                owner = Some(owner_record);
                slot = next_owned;
            }
        }
        if occupied > 0 {
            assert!(chains >= 1, "at least one chain when occupied");
        }
        assert_eq!(
            visited.len(),
            occupied,
            "every occupied slot reachable from a head"
        );
    }
    fn witness(v: [u64; 5]) -> HotPathWorkWitness {
        HotPathWorkWitness::new(v)
    }
    fn select_ok(arena: &EntitlementCellArena, count: usize) -> HotPathWorkWitness {
        let mut meter = work();
        arena.validate_selection(count, &mut meter).unwrap();
        meter.witness()
    }
    fn select_err(arena: &EntitlementCellArena, count: usize) -> HotPathWorkWitness {
        let mut meter = work();
        let selection = arena.validate_selection(count, &mut meter);
        assert_eq!(selection, Err(FixedStorageError::Capacity));
        meter.witness()
    }
    fn chain_ok(
        arena: &EntitlementCellArena,
        head: usize,
        len: usize,
        owner: usize,
        cells: &[OutstandingCreditCell],
    ) -> HotPathWorkWitness {
        let mut meter = work();
        let chain = arena.validate_chain(head, len, owner, cells, &mut meter);
        assert_eq!(chain, Ok(()));
        meter.witness()
    }
    fn chain_err(
        arena: &EntitlementCellArena,
        head: usize,
        len: usize,
        owner: usize,
        cells: &[OutstandingCreditCell],
    ) -> HotPathWorkWitness {
        let mut meter = work();
        let chain = arena.validate_chain(head, len, owner, cells, &mut meter);
        assert_eq!(chain, Err(FixedStorageError::NonCanonical));
        meter.witness()
    }
    fn one_under(
        dim: WorkDimension,
        exact: u64,
        max: u64,
        run: impl FnOnce(&mut WorkMeter) -> std::result::Result<(), FixedStorageError>,
    ) {
        let mut meter = work();
        meter.record(dim, max - exact + 1).unwrap();
        let fault = run(&mut meter).unwrap_err();
        let expected = WorkBudgetError::BudgetExceeded(dim, max, max + 1);
        assert_eq!(fault, FixedStorageError::Work(expected));
    }

    #[test]
    fn c16_cell_arena_constructor() {
        for (capacity, error) in [
            (0, FixedStorageError::Capacity),
            (usize::MAX, FixedStorageError::Allocation),
        ] {
            assert_eq!(EntitlementCellArena::try_new(capacity).unwrap_err(), error);
        }
        // First storage-invalid boundary: slots+free bytes exceed the binary bound.
        let slot_bytes = std::mem::size_of::<CellSlot>() as u64;
        let index_bytes = std::mem::size_of::<usize>() as u64;
        let max_capacity = (2_097_152_u64 / (slot_bytes + index_bytes)) as usize;
        let boundary = EntitlementCellArena::try_new(max_capacity + 1).unwrap_err();
        assert_eq!(boundary, FixedStorageError::Capacity);
        // Deterministic fail-closed seal: an over-capacity stand-in proves
        // rejection without relying on allocator behavior. `with_capacity(16)`
        // guarantees at least 16 slots, so both backing Vecs are never exactly 8.
        let mut slots = Vec::with_capacity(16);
        slots.resize(8, CellSlot::Vacant);
        let mut free = Vec::with_capacity(16);
        free.extend((0..8).rev());
        let sealed = seal_exact_capacity(&slots, &free, 8).unwrap_err();
        assert_eq!(sealed, FixedStorageError::Capacity);
        let arena = EntitlementCellArena::try_new(8).unwrap();
        assert_eq!(arena.capacity(), 8);
        assert_eq!(arena.free_len(), 8);
        arena_oracle(&arena);
    }

    #[test]
    fn c16_cell_arena_selection_and_install() {
        let mut arena = EntitlementCellArena::try_new(8).unwrap();
        assert_eq!(select_ok(&arena, 0), witness([0, 0, 0, 0, 1]));
        assert_eq!(select_ok(&arena, 1), witness([1, 0, 0, 0, 2]));
        assert_eq!(select_err(&arena, 9), witness([0, 0, 0, 0, 1]));
        let five = axis_cells(5, 1);
        assert_eq!(select_ok(&arena, 5), witness([5, 0, 0, 0, 6]));
        let (head, len) = arena.install(7, &five);
        assert_eq!((head, len), (4, 5));
        assert_eq!(arena.free_len(), 3);
        assert_eq!(select_ok(&arena, 3), witness([3, 0, 0, 0, 4]));
        assert_eq!(select_err(&arena, 4), witness([0, 0, 0, 0, 1]));
        for (index, cell, next) in [(0, five[4], None), (4, five[0], Some(3))] {
            let expected = CellSlot::Occupied {
                owner_record: 7,
                cell,
                next_owned: next,
            };
            assert_eq!(arena.slots[index], expected);
        }
        let chain = chain_ok(&arena, head, len, 7, &five);
        assert_eq!(chain, witness([5, 0, 0, 0, 26]));
        let three = axis_cells(3, 2);
        let (tail_head, tail_len) = arena.install(8, &three);
        assert_eq!((tail_head, tail_len), (7, 3));
        assert_eq!(arena.free_len(), 0);
        assert_eq!(select_err(&arena, 1), witness([0, 0, 0, 0, 1]));
        arena_oracle(&arena);
    }

    #[test]
    fn c16_cell_arena_disjoint_owners() {
        let mut arena = EntitlementCellArena::try_new(8).unwrap();
        let first = axis_cells(3, 1);
        let second = axis_cells(2, 2);
        let (h1, l1) = arena.install(11, &first);
        let (h2, l2) = arena.install(22, &second);
        assert_eq!((h1, l1), (2, 3));
        assert_eq!((h2, l2), (4, 2));
        chain_ok(&arena, h1, l1, 11, &first);
        chain_ok(&arena, h2, l2, 22, &second);
        arena_oracle(&arena);
    }

    #[test]
    fn c16_cell_arena_chain_rejection() {
        let mut arena = EntitlementCellArena::try_new(8).unwrap();
        let cells = axis_cells(3, 1);
        let (head, len) = arena.install(7, &cells);
        let wrong_owner = chain_err(&arena, head, len, 99, &cells);
        assert_eq!(wrong_owner, witness([1, 0, 0, 0, 4]));
        chain_err(&arena, head, 2, 7, &cells[..2]);
        let reversed = [cells[0], cells[2], cells[1]];
        chain_err(&arena, head, 3, 7, &reversed);
        arena.release(head, len);
        let stale = chain_err(&arena, head, len, 7, &cells);
        assert_eq!(stale, witness([1, 0, 0, 0, 3]));
        assert_eq!(arena.free_len(), 8);
        arena_oracle(&arena);
    }

    #[test]
    fn c16_cell_arena_reuse_and_churn() {
        let mut arena = EntitlementCellArena::try_new(8).unwrap();
        let mut total_installed = 0usize;
        for cycle in 0..5 {
            let count = 3 + cycle % 2;
            let cells = axis_cells(count, 1);
            let (head, len) = arena.install(cycle + 1, &cells);
            assert_eq!(len, count);
            assert!(head < 8);
            arena_oracle(&arena);
            chain_ok(&arena, head, len, cycle + 1, &cells);
            arena.release(head, len);
            arena_oracle(&arena);
            total_installed += count;
        }
        assert!(total_installed > 8, "churn exceeds physical capacity");
        assert_eq!(arena.free_len(), 8);
        arena_oracle(&arena);
    }

    #[test]
    fn c16_cell_arena_front_middle_tail_reuse() {
        let mut arena = EntitlementCellArena::try_new(8).unwrap();
        let a = axis_cells(3, 1);
        let b = axis_cells(2, 2);
        let c = axis_cells(2, 3);
        assert_eq!(arena.install(1, &a), (2, 3));
        assert_eq!(arena.install(2, &b), (4, 2));
        assert_eq!(arena.install(3, &c), (6, 2));
        arena_oracle(&arena);
        arena.release(4, 2);
        let d = axis_cells(2, 4);
        assert_eq!(arena.install(4, &d), (4, 2), "middle slots reused");
        arena_oracle(&arena);
        arena.release(2, 3);
        let e = axis_cells(3, 5);
        assert_eq!(arena.install(5, &e), (2, 3), "front slots reused");
        arena_oracle(&arena);
        arena.release(6, 2);
        let f = axis_cells(3, 6);
        let tail = arena.install(6, &f);
        assert_eq!(tail, (7, 3), "tail slots and never-used index reused");
        assert_eq!(arena.free_len(), 0);
        arena_oracle(&arena);
    }

    #[test]
    fn c16_cell_arena_work_exact_and_one_under() {
        let mut arena = EntitlementCellArena::try_new(8).unwrap();
        let cells = axis_cells(3, 1);
        assert_eq!(select_ok(&arena, 3), witness([3, 0, 0, 0, 4]));
        one_under(WorkDimension::VisitedEntities, 3, 1_704_575, |m| {
            arena.validate_selection(3, m)
        });
        one_under(WorkDimension::InvariantChecks, 4, 28_708, |m| {
            arena.validate_selection(3, m)
        });
        let (head, len) = arena.install(7, &cells);
        let chain = chain_ok(&arena, head, len, 7, &cells);
        assert_eq!(chain, witness([3, 0, 0, 0, 16]));
        one_under(WorkDimension::VisitedEntities, 3, 1_704_575, |m| {
            arena.validate_chain(head, len, 7, &cells, m)
        });
        one_under(WorkDimension::InvariantChecks, 16, 28_708, |m| {
            arena.validate_chain(head, len, 7, &cells, m)
        });
    }

    /// Test-only tagged key over one tag byte and one repeated identity byte.
    fn tagged(tag: u8, byte: u8) -> TaggedKey {
        TaggedKey::new(tag, [byte; 32])
    }
    /// Test-only valid compressed tree over three leaves, built in the same
    /// physical slot order as three sequential inserts:
    /// `root = branch(7, branch(8, leaf(0,[0;32]), leaf(0,[0x80;32])), leaf(1,[0;32]))`.
    /// Branch slot 0 discriminates bit 8, branch slot 1 bit 7 (tag parity).
    fn three_leaf_tree() -> TaggedIdentityIndex {
        let mut index = TaggedIdentityIndex::try_new(16).unwrap();
        index.leaf_slots[0] = Some(IdentityLeaf {
            key: tagged(0, 0x00),
            record: 10,
        });
        index.leaf_slots[1] = Some(IdentityLeaf {
            key: tagged(0, 0x80),
            record: 11,
        });
        index.leaf_slots[2] = Some(IdentityLeaf {
            key: tagged(1, 0x00),
            record: 12,
        });
        index.free_leaves = (3..16).rev().map(|index| index as u32).collect();
        index.branch_slots[0] = Some(IdentityBranch {
            bit: 8,
            zero: 0,
            one: 1,
        });
        index.branch_slots[1] = Some(IdentityBranch {
            bit: 7,
            zero: BRANCH_TAG,
            one: 2,
        });
        index.free_branches = (2..15).rev().map(|index| index as u32).collect();
        index.root = BRANCH_TAG | 1;
        index
    }
    /// Test-only full trie oracle: every leaf/branch slot in range and covered
    /// exactly once, free entries unique and Vacant, branch bits strictly
    /// increasing along every route, every occupied node reachable exactly once,
    /// and every leaf key unique. Scans `Theta(I + J)` slots and is never called
    /// or charged by a production transition.
    fn trie_oracle(index: &TaggedIdentityIndex) {
        use std::collections::HashSet;
        let mut free_leaves = HashSet::new();
        for &leaf in &index.free_leaves {
            assert!(leaf < index.leaf_capacity as u32, "free leaf in range");
            assert!(free_leaves.insert(leaf), "free leaf indices unique");
            assert_eq!(index.leaf_slots[leaf as usize], None, "free leaf vacant");
        }
        let mut free_branches = HashSet::new();
        for &branch in &index.free_branches {
            assert!(
                branch < index.branch_capacity as u32,
                "free branch in range"
            );
            assert!(free_branches.insert(branch), "free branch indices unique");
            assert_eq!(
                index.branch_slots[branch as usize], None,
                "free branch vacant"
            );
        }
        let occupied_leaves = index
            .leaf_slots
            .iter()
            .filter(|slot| slot.is_some())
            .count();
        let occupied_branches = index
            .branch_slots
            .iter()
            .filter(|slot| slot.is_some())
            .count();
        assert_eq!(
            free_leaves.len() + occupied_leaves,
            index.leaf_capacity,
            "leaf partition"
        );
        assert_eq!(
            free_branches.len() + occupied_branches,
            index.branch_capacity,
            "branch partition"
        );
        assert_eq!(
            occupied_branches,
            occupied_leaves.saturating_sub(1),
            "branch cardinality"
        );
        if occupied_leaves == 0 {
            assert_eq!(index.root, NO_NODE, "empty tree root");
            return;
        }
        assert_ne!(index.root, NO_NODE, "nonempty tree root");
        let mut seen_leaves = HashSet::new();
        let mut seen_branches = HashSet::new();
        let mut keys = HashSet::new();
        let mut stack = vec![(index.root, None)];
        while let Some((node, parent_bit)) = stack.pop() {
            if is_branch(node) {
                let slot = branch_index(node);
                assert!(seen_branches.insert(slot), "branch reachable exactly once");
                let branch = index.branch_slots[slot]
                    .as_ref()
                    .expect("reachable occupied branch");
                assert!(branch.bit < IDENTITY_BITS, "branch bit in range");
                if let Some(parent) = parent_bit {
                    assert!(parent < branch.bit, "branch bits strictly increase");
                }
                assert_ne!(branch.zero, branch.one, "branch children distinct");
                stack.push((branch.zero, Some(branch.bit)));
                stack.push((branch.one, Some(branch.bit)));
            } else {
                let slot = node as usize;
                assert!(slot < index.leaf_capacity, "leaf reference in range");
                let leaf = index.leaf_slots[slot]
                    .as_ref()
                    .expect("reachable occupied leaf");
                assert!(seen_leaves.insert(slot), "leaf reachable exactly once");
                assert!(keys.insert(leaf.key), "leaf keys unique");
                assert!(
                    !free_leaves.contains(&(slot as u32)),
                    "reachable leaf not free"
                );
            }
        }
        assert_eq!(
            seen_leaves.len(),
            occupied_leaves,
            "every occupied leaf reachable"
        );
        assert_eq!(
            seen_branches.len(),
            occupied_branches,
            "every occupied branch reachable"
        );
    }
    #[test]
    fn c16_identity_index_constructor() {
        for (capacity, error) in [
            (0, FixedStorageError::Capacity),
            (usize::MAX, FixedStorageError::Allocation),
            (BRANCH_TAG as usize, FixedStorageError::Capacity),
        ] {
            assert_eq!(TaggedIdentityIndex::try_new(capacity).unwrap_err(), error);
        }
        // First storage-invalid boundary: leaf+branch slots and free stacks
        // exceed the binary Storage/CopiedBytes maximum.
        let leaf_slot_bytes = std::mem::size_of::<Option<IdentityLeaf>>() as u64;
        let branch_slot_bytes = std::mem::size_of::<Option<IdentityBranch>>() as u64;
        let index_bytes = std::mem::size_of::<u32>() as u64;
        let storage = |leaves: u64| {
            leaves * (leaf_slot_bytes + index_bytes)
                + (leaves - 1) * (branch_slot_bytes + index_bytes)
        };
        let mut maximum = 1usize;
        while storage(maximum as u64 + 1) <= 2_097_152 {
            maximum += 1;
        }
        assert_eq!(
            TaggedIdentityIndex::try_new(maximum)
                .unwrap()
                .leaf_capacity(),
            maximum
        );
        assert_eq!(
            TaggedIdentityIndex::try_new(maximum + 1).unwrap_err(),
            FixedStorageError::Capacity
        );
        let index = TaggedIdentityIndex::try_new(16).unwrap();
        assert_eq!((index.leaf_capacity(), index.branch_capacity()), (16, 15));
        assert_eq!((index.free_leaf_len(), index.free_branch_len()), (16, 15));
        assert!(index.is_empty());
        assert_eq!(index.root, NO_NODE);
    }
    #[test]
    fn c16_identity_index_empty_lookup() {
        let index = TaggedIdentityIndex::try_new(16).unwrap();
        let mut meter = work();
        assert_eq!(index.find(0, &[7; 32], &mut meter), Ok(None));
        assert_eq!(meter.witness(), witness([0, 0, 0, 0, 0]));
        let mut meter = work();
        assert_eq!(index.find(5, &[0; 32], &mut meter), Ok(None));
        assert_eq!(meter.witness(), witness([0, 0, 0, 0, 0]));
        assert_eq!(index.free_leaf_len(), 16);
        assert_eq!(index.free_branch_len(), 15);
    }
    #[test]
    fn c16_identity_index_single_leaf_lookup() {
        let mut index = TaggedIdentityIndex::try_new(16).unwrap();
        index.leaf_slots[0] = Some(IdentityLeaf {
            key: tagged(0, 0xAB),
            record: 7,
        });
        index.root = 0;
        index.free_leaves = (1..16).rev().map(|index| index as u32).collect();
        let present = |tag: u8, identity: [u8; 32]| {
            let mut meter = work();
            let found = index.find(tag, &identity, &mut meter);
            (found, meter.witness())
        };
        assert_eq!(
            present(0, [0xAB; 32]),
            (Ok(Some(7)), witness([1, 0, 0, 0, 1]))
        );
        // Same tag, different identity: same bounded path, absent.
        assert_eq!(present(0, [0xAC; 32]), (Ok(None), witness([1, 0, 0, 0, 1])));
        // Different tag, same identity bytes: a distinct key, absent.
        assert_eq!(present(1, [0xAB; 32]), (Ok(None), witness([1, 0, 0, 0, 1])));
        assert_eq!(index.free_leaf_len(), 15);
        assert_eq!(index.free_branch_len(), 15);
    }
    #[test]
    fn c16_identity_index_many_route_lookup() {
        let index = three_leaf_tree();
        let lookup = |tag: u8, identity: [u8; 32]| {
            let mut meter = work();
            let found = index.find(tag, &identity, &mut meter);
            (found, meter.witness())
        };
        // Present keys follow the two-branch path or the one-branch path.
        assert_eq!(
            lookup(0, [0x00; 32]),
            (Ok(Some(10)), witness([3, 0, 0, 0, 3]))
        );
        assert_eq!(
            lookup(0, [0x80; 32]),
            (Ok(Some(11)), witness([3, 0, 0, 0, 3]))
        );
        assert_eq!(
            lookup(1, [0x00; 32]),
            (Ok(Some(12)), witness([2, 0, 0, 0, 2]))
        );
        // Same raw identity bytes across distinct tags are distinct keys.
        assert_eq!(
            lookup(0, [0x00; 32]),
            (Ok(Some(10)), witness([3, 0, 0, 0, 3]))
        );
        assert_eq!(
            lookup(1, [0x00; 32]),
            (Ok(Some(12)), witness([2, 0, 0, 0, 2]))
        );
        // Absent key routed to leaf2 (bit 7 = 1): tag 1, identity byte 0x01.
        assert_eq!(lookup(1, [0x01; 32]), (Ok(None), witness([2, 0, 0, 0, 2])));
        // Absent key routed to leaf0 (deeper path): tag 0, identity byte 0x40.
        assert_eq!(lookup(0, [0x40; 32]), (Ok(None), witness([3, 0, 0, 0, 3])));
        // Absent key with a third tag (tag 2) is a distinct namespace.
        assert_eq!(lookup(2, [0x00; 32]), (Ok(None), witness([3, 0, 0, 0, 3])));
    }
    #[test]
    fn c16_identity_index_lookup_work_one_under() {
        let index = three_leaf_tree();
        one_under(WorkDimension::VisitedEntities, 3, 1_704_575, |meter| {
            index.find(0, &[0x00; 32], meter).map(|_| ())
        });
        one_under(WorkDimension::InvariantChecks, 3, 28_708, |meter| {
            index.find(0, &[0x00; 32], meter).map(|_| ())
        });
        one_under(WorkDimension::VisitedEntities, 2, 1_704_575, |meter| {
            index.find(1, &[0x00; 32], meter).map(|_| ())
        });
        one_under(WorkDimension::InvariantChecks, 2, 28_708, |meter| {
            index.find(1, &[0x00; 32], meter).map(|_| ())
        });
    }
    #[test]
    fn c16_identity_index_insert_first_then_second() {
        let mut index = TaggedIdentityIndex::try_new(16).unwrap();
        let mut meter = work();
        index.insert(tagged(0, 0x00), 10, &mut meter).unwrap();
        assert_eq!(meter.witness(), witness([0, 0, 0, 0, 1]));
        assert_eq!(index.root, 0);
        assert_eq!(
            index.leaf_slots[0],
            Some(IdentityLeaf {
                key: tagged(0, 0x00),
                record: 10
            })
        );
        assert_eq!((index.free_leaf_len(), index.free_branch_len()), (15, 15));
        let mut meter = work();
        index.insert(tagged(0, 0x80), 11, &mut meter).unwrap();
        assert_eq!(meter.witness(), witness([3, 0, 0, 0, 3]));
        assert_eq!(index.root, BRANCH_TAG);
        assert_eq!(
            index.branch_slots[0],
            Some(IdentityBranch {
                bit: 8,
                zero: 0,
                one: 1
            })
        );
        assert_eq!(
            index.leaf_slots[1],
            Some(IdentityLeaf {
                key: tagged(0, 0x80),
                record: 11
            })
        );
        assert_eq!((index.free_leaf_len(), index.free_branch_len()), (14, 14));
        for (tag, byte, record) in [(0u8, 0x00u8, 10u32), (0, 0x80, 11)] {
            assert_eq!(index.find(tag, &[byte; 32], &mut work()), Ok(Some(record)));
        }
    }
    #[test]
    fn c16_identity_index_insert_many_matches_fixture() {
        let mut index = TaggedIdentityIndex::try_new(16).unwrap();
        let mut meter = work();
        index.insert(tagged(0, 0x00), 10, &mut meter).unwrap();
        let mut meter = work();
        index.insert(tagged(0, 0x80), 11, &mut meter).unwrap();
        let mut meter = work();
        index.insert(tagged(1, 0x00), 12, &mut meter).unwrap();
        assert_eq!(meter.witness(), witness([4, 0, 0, 0, 5]));
        // Insert-built tree is structurally identical to the hand-built fixture.
        assert_eq!(index, three_leaf_tree());
    }
    #[test]
    fn c16_identity_index_insert_duplicate_rejects_and_rolls_back() {
        let mut index = TaggedIdentityIndex::try_new(16).unwrap();
        let mut meter = work();
        index.insert(tagged(0, 0x00), 10, &mut meter).unwrap();
        let before = index.clone();
        let mut meter = work();
        let result = index.insert(tagged(0, 0x00), 99, &mut meter);
        assert_eq!(result, Err(FixedStorageError::Duplicate));
        assert_eq!(meter.witness(), witness([1, 0, 0, 0, 1]));
        assert_eq!(index, before);
        let mut index = three_leaf_tree();
        let before = index.clone();
        let mut meter = work();
        let result = index.insert(tagged(0, 0x00), 99, &mut meter);
        assert_eq!(result, Err(FixedStorageError::Duplicate));
        assert_eq!(meter.witness(), witness([3, 0, 0, 0, 3]));
        assert_eq!(index, before);
    }
    #[test]
    fn c16_identity_index_insert_first_difference_edges() {
        // First differing bit 0: tag 0x00 versus tag 0x80.
        let mut index = TaggedIdentityIndex::try_new(16).unwrap();
        let mut meter = work();
        index.insert(tagged(0x00, 0), 1, &mut meter).unwrap();
        let mut meter = work();
        index.insert(tagged(0x80, 0), 2, &mut meter).unwrap();
        assert_eq!(meter.witness(), witness([2, 0, 0, 0, 3]));
        assert_eq!(index.root, BRANCH_TAG);
        assert_eq!(
            index.branch_slots[0],
            Some(IdentityBranch {
                bit: 0,
                zero: 0,
                one: 1
            })
        );
        assert_eq!(index.find(0x00, &[0; 32], &mut work()), Ok(Some(1)));
        assert_eq!(index.find(0x80, &[0; 32], &mut work()), Ok(Some(2)));
        // First differing bit 263: identity byte 31 LSB.
        let mut index = TaggedIdentityIndex::try_new(16).unwrap();
        let mut meter = work();
        index.insert(tagged(0, 0x00), 1, &mut meter).unwrap();
        let mut identity = [0u8; 32];
        identity[31] = 1;
        let mut meter = work();
        index
            .insert(TaggedKey::new(0, identity), 2, &mut meter)
            .unwrap();
        assert_eq!(meter.witness(), witness([34, 0, 0, 0, 3]));
        assert_eq!(index.root, BRANCH_TAG);
        assert_eq!(
            index.branch_slots[0],
            Some(IdentityBranch {
                bit: 263,
                zero: 0,
                one: 1
            })
        );
        assert_eq!(index.find(0, &[0; 32], &mut work()), Ok(Some(1)));
        assert_eq!(index.find(0, &identity, &mut work()), Ok(Some(2)));
    }
    #[test]
    fn c16_identity_index_insert_capacity_and_work() {
        // I = 4 leaves, J = 3 branches: exactly four inserts fit.
        let mut index = TaggedIdentityIndex::try_new(4).unwrap();
        for tag in [0u8, 1, 2, 3] {
            index
                .insert(tagged(tag, 0), u32::from(tag) + 1, &mut work())
                .unwrap();
        }
        assert_eq!(index.free_leaf_len(), 0);
        assert_eq!(index.free_branch_len(), 0);
        for tag in [0u8, 1, 2, 3] {
            assert_eq!(
                index.find(tag, &[0; 32], &mut work()),
                Ok(Some(u32::from(tag) + 1))
            );
        }
        let before = index.clone();
        let mut meter = work();
        let result = index.insert(tagged(4, 0), 99, &mut meter);
        assert_eq!(result, Err(FixedStorageError::Capacity));
        assert_eq!(meter.witness(), witness([3, 0, 0, 0, 4]));
        assert_eq!(index, before);
        one_under(WorkDimension::InvariantChecks, 1, 28_708, |meter| {
            TaggedIdentityIndex::try_new(16)
                .unwrap()
                .insert(tagged(0, 0), 1, meter)
        });
        let mut index = TaggedIdentityIndex::try_new(16).unwrap();
        index.insert(tagged(0, 0x00), 1, &mut work()).unwrap();
        one_under(WorkDimension::VisitedEntities, 3, 1_704_575, |meter| {
            index.insert(tagged(0, 0x80), 2, meter)
        });
        one_under(WorkDimension::InvariantChecks, 3, 28_708, |meter| {
            index.insert(tagged(0, 0x80), 2, meter)
        });
    }
    #[test]
    fn c16_identity_index_remove_leaf_and_root() {
        // Single-leaf tree: removing the root leaf empties the tree.
        let mut index = TaggedIdentityIndex::try_new(4).unwrap();
        index.insert(tagged(0, 0), 1, &mut work()).unwrap();
        let mut meter = work();
        assert_eq!(index.remove(0, &[0; 32], &mut meter), Ok(Some(1)));
        assert_eq!(meter.witness(), witness([1, 0, 0, 0, 1]));
        assert!(index.is_empty());
        assert_eq!((index.free_leaf_len(), index.free_branch_len()), (4, 3));
        trie_oracle(&index);
        // Two-leaf tree: splicing the sibling over the parent makes it the root.
        let mut index = TaggedIdentityIndex::try_new(4).unwrap();
        index.insert(tagged(0, 0x00), 1, &mut work()).unwrap();
        index.insert(tagged(0, 0x80), 2, &mut work()).unwrap();
        let mut meter = work();
        assert_eq!(index.remove(0, &[0x00; 32], &mut meter), Ok(Some(1)));
        assert_eq!(meter.witness(), witness([2, 0, 0, 0, 2]));
        assert_eq!(index.root, 1);
        assert_eq!(index.find(0, &[0x80; 32], &mut work()), Ok(Some(2)));
        assert_eq!(index.branch_slots[0], None);
        trie_oracle(&index);
    }
    #[test]
    fn c16_identity_index_remove_splices_parent_and_root() {
        // Remove the middle leaf: its sibling splices over the parent branch.
        let mut index = three_leaf_tree();
        let mut meter = work();
        assert_eq!(index.remove(0, &[0x80; 32], &mut meter), Ok(Some(11)));
        assert_eq!(meter.witness(), witness([3, 0, 0, 0, 3]));
        assert_eq!(index.root, BRANCH_TAG | 1);
        assert_eq!(
            index.branch_slots[1],
            Some(IdentityBranch {
                bit: 7,
                zero: 0,
                one: 2
            })
        );
        assert_eq!(index.branch_slots[0], None);
        assert_eq!(index.find(0, &[0; 32], &mut work()), Ok(Some(10)));
        assert_eq!(index.find(1, &[0; 32], &mut work()), Ok(Some(12)));
        trie_oracle(&index);
        // Remove the one-side leaf: the sibling branch becomes the root.
        let mut index = three_leaf_tree();
        let mut meter = work();
        assert_eq!(index.remove(1, &[0; 32], &mut meter), Ok(Some(12)));
        assert_eq!(meter.witness(), witness([2, 0, 0, 0, 2]));
        assert_eq!(index.root, BRANCH_TAG);
        assert_eq!(
            index.branch_slots[0],
            Some(IdentityBranch {
                bit: 8,
                zero: 0,
                one: 1
            })
        );
        assert_eq!(index.branch_slots[1], None);
        assert_eq!(index.find(0, &[0; 32], &mut work()), Ok(Some(10)));
        assert_eq!(index.find(0, &[0x80; 32], &mut work()), Ok(Some(11)));
        trie_oracle(&index);
        // Remove the deep zero-side leaf: the sibling splices under the root.
        let mut index = three_leaf_tree();
        let mut meter = work();
        assert_eq!(index.remove(0, &[0; 32], &mut meter), Ok(Some(10)));
        assert_eq!(meter.witness(), witness([3, 0, 0, 0, 3]));
        assert_eq!(index.root, BRANCH_TAG | 1);
        assert_eq!(
            index.branch_slots[1],
            Some(IdentityBranch {
                bit: 7,
                zero: 1,
                one: 2
            })
        );
        assert_eq!(index.branch_slots[0], None);
        assert_eq!(index.find(0, &[0x80; 32], &mut work()), Ok(Some(11)));
        assert_eq!(index.find(1, &[0; 32], &mut work()), Ok(Some(12)));
        trie_oracle(&index);
    }
    #[test]
    fn c16_identity_index_remove_absent_preserves_state() {
        let mut index = TaggedIdentityIndex::try_new(4).unwrap();
        let mut meter = work();
        assert_eq!(index.remove(0, &[1; 32], &mut meter), Ok(None));
        assert_eq!(meter.witness(), witness([0, 0, 0, 0, 0]));
        index.insert(tagged(0, 0), 1, &mut work()).unwrap();
        let before = index.clone();
        let mut meter = work();
        assert_eq!(index.remove(0, &[1; 32], &mut meter), Ok(None));
        assert_eq!(meter.witness(), witness([1, 0, 0, 0, 1]));
        assert_eq!(index, before);
        let mut index = three_leaf_tree();
        let before = index.clone();
        let mut meter = work();
        assert_eq!(index.remove(2, &[0; 32], &mut meter), Ok(None));
        assert_eq!(meter.witness(), witness([3, 0, 0, 0, 3]));
        assert_eq!(index, before);
    }
    #[test]
    fn c16_identity_index_remove_churn_and_deterministic_reuse() {
        let mut index = TaggedIdentityIndex::try_new(4).unwrap();
        let mut inserted = 0u64;
        for cycle in 0..6 {
            for offset in 0..2 {
                let tag = (cycle * 2 + offset) as u8;
                index
                    .insert(tagged(tag, 0), u32::from(tag) + 1, &mut work())
                    .unwrap();
                inserted += 1;
            }
            for offset in 0..2 {
                let tag = (cycle * 2 + offset) as u8;
                let mut meter = work();
                let removed = index.remove(tag, &[0; 32], &mut meter);
                assert_eq!(removed, Ok(Some(u32::from(tag) + 1)));
            }
            trie_oracle(&index);
        }
        assert!(inserted > 4, "churn exceeds leaf capacity");
        assert_eq!(index.free_leaf_len(), 4);
        assert_eq!(index.free_branch_len(), 3);
        assert!(index.is_empty());
        // Deterministic LIFO reuse: after removing tags 0 and 1, the next insert
        // reuses leaf slot 1 (the most recently freed leaf).
        let mut index = TaggedIdentityIndex::try_new(4).unwrap();
        for tag in [0u8, 1] {
            index
                .insert(tagged(tag, 0), u32::from(tag) + 1, &mut work())
                .unwrap();
        }
        index.remove(0, &[0; 32], &mut work()).unwrap();
        index.remove(1, &[0; 32], &mut work()).unwrap();
        index.insert(tagged(2, 0), 3, &mut work()).unwrap();
        assert_eq!(
            index.leaf_slots[1],
            Some(IdentityLeaf {
                key: tagged(2, 0),
                record: 3
            }),
            "freed leaf slot deterministically reused"
        );
        trie_oracle(&index);
    }
    #[test]
    fn c16_identity_index_remove_work_one_under() {
        let index = three_leaf_tree();
        one_under(WorkDimension::VisitedEntities, 3, 1_704_575, |meter| {
            let mut index = index.clone();
            index.remove(0, &[0x80; 32], meter).map(|_| ())
        });
        one_under(WorkDimension::InvariantChecks, 3, 28_708, |meter| {
            let mut index = index.clone();
            index.remove(0, &[0x80; 32], meter).map(|_| ())
        });
        one_under(WorkDimension::VisitedEntities, 2, 1_704_575, |meter| {
            let mut index = index.clone();
            index.remove(1, &[0; 32], meter).map(|_| ())
        });
        one_under(WorkDimension::InvariantChecks, 2, 28_708, |meter| {
            let mut index = index.clone();
            index.remove(1, &[0; 32], meter).map(|_| ())
        });
    }

    /// Test-only distinct bundle record with canonical same-tag identity groups.
    fn bundle_record(n: u8) -> BundleRecord {
        // Two-byte identities: byte 0 is the record, byte 1 the member offset,
        // so distinct records never share an identity within u8 range.
        let identity = |offset: u8| {
            let mut id = [0u8; 32];
            id[0] = n;
            id[1] = offset;
            id
        };
        BundleRecord {
            obligations: [
                SupportOperationObligationId::new(identity(1)).unwrap(),
                SupportOperationObligationId::new(identity(2)).unwrap(),
                SupportOperationObligationId::new(identity(3)).unwrap(),
            ],
            credits: [
                PhysicalStartCreditId::new(identity(11)).unwrap(),
                PhysicalStartCreditId::new(identity(12)).unwrap(),
                PhysicalStartCreditId::new(identity(13)).unwrap(),
            ],
            claims: [identity(21), identity(22), identity(23)],
            entitlement: FutureTurnSupportEntitlementId::new(identity(31)).unwrap(),
            vector: SupportOutstandingCreditVectorId::new(identity(41)).unwrap(),
            vector_head: None,
            vector_len: 0,
            state: BundleRecordState::Live,
        }
    }
    /// Test-only full request-bundle store oracle: record/cell/leaf/branch
    /// partitions, free-stack validity, trie and arena oracles, and every
    /// cross-ownership relation plus all four scalar conservation equations.
    /// Scans `Theta(C + I + J + E)` slots and is never called or charged by a
    /// production transition.
    fn bundle_store_oracle(store: &RequestBundleStore) {
        use std::collections::HashSet;
        let mut free_records = HashSet::new();
        for &record in &store.free_records {
            assert!(
                record < store.record_capacity as u32,
                "free record in range"
            );
            assert!(free_records.insert(record), "free record indices unique");
            assert_eq!(store.records[record as usize], None, "free record vacant");
        }
        let occupied: Vec<u32> = store
            .records
            .iter()
            .enumerate()
            .filter_map(|(index, slot)| slot.is_some().then_some(index as u32))
            .collect();
        assert_eq!(
            free_records.len() + occupied.len(),
            store.record_capacity,
            "record partition"
        );
        trie_oracle(&store.identities);
        arena_oracle(&store.cells);
        for &record in &occupied {
            let slot = store.records[record as usize]
                .as_ref()
                .expect("occupied record");
            for key in slot.tagged_keys() {
                assert_eq!(
                    store.identities.find(key.tag, &key.identity, &mut work()),
                    Ok(Some(record)),
                    "every record identity present and owned"
                );
            }
            let head = slot.vector_head.expect("occupied record has a cell chain");
            store
                .cells
                .validate_owner_chain(head, slot.vector_len, record as usize, &mut work())
                .unwrap();
        }
        for leaf in store.identities.leaf_slots.iter().flatten() {
            assert!(
                store.records[leaf.record as usize].is_some(),
                "every index leaf owner is an occupied record"
            );
        }
        let cells_owned: usize = occupied
            .iter()
            .map(|&record| {
                store.records[record as usize]
                    .as_ref()
                    .expect("occupied record")
                    .vector_len
            })
            .sum();
        assert_eq!(
            store.cells.free_len() + cells_owned,
            store.cells.capacity(),
            "cell scalar conservation"
        );
        assert_eq!(
            store.identities.free_leaf_len() + K * occupied.len(),
            store.identities.leaf_capacity(),
            "leaf scalar conservation"
        );
        let branches = if occupied.is_empty() {
            0
        } else {
            K * occupied.len() - 1
        };
        assert_eq!(
            store.identities.free_branch_len() + branches,
            store.identities.branch_capacity(),
            "branch scalar conservation"
        );
    }
    #[test]
    fn c16_bundle_store_constructor() {
        for (records, cells) in [(0usize, 8usize), (4, 0)] {
            assert_eq!(
                RequestBundleStore::try_new(records, cells).unwrap_err(),
                FixedStorageError::Capacity
            );
        }
        assert_eq!(
            RequestBundleStore::try_new(usize::MAX / K + 1, 8).unwrap_err(),
            FixedStorageError::Allocation
        );
        // First storage-invalid boundary: total record/identity/cell storage
        // exceeds the binary Storage/CopiedBytes maximum.
        let record_bytes =
            std::mem::size_of::<Option<BundleRecord>>() as u64 + std::mem::size_of::<u32>() as u64;
        let storage = |records: u64| {
            records * record_bytes
                + TaggedIdentityIndex::storage_bytes(records * K as u64).unwrap()
                + EntitlementCellArena::storage_bytes(1).unwrap()
        };
        let mut maximum = 1u64;
        while storage(maximum + 1) <= 2_097_152 {
            maximum += 1;
        }
        assert_eq!(
            RequestBundleStore::try_new(maximum as usize, 1)
                .unwrap()
                .record_capacity(),
            maximum as usize
        );
        assert_eq!(
            RequestBundleStore::try_new(maximum as usize + 1, 1).unwrap_err(),
            FixedStorageError::Capacity
        );
        // E = 1 boundary and exact-capacity seal.
        let store = RequestBundleStore::try_new(1, 4).unwrap();
        assert_eq!(store.record_capacity(), 1);
        assert_eq!(
            (store.records.capacity(), store.free_records.capacity()),
            (1, 1)
        );
        assert_eq!(
            (
                store.identities.leaf_capacity(),
                store.identities.branch_capacity()
            ),
            (K, K - 1)
        );
        assert_eq!(store.cells.capacity(), 4);
        assert_eq!(
            (store.free_record_len(), store.identities.free_leaf_len()),
            (1, K)
        );
        assert!(store.is_empty());
        bundle_store_oracle(&store);
        let store = RequestBundleStore::try_new(4, 8).unwrap();
        assert_eq!(
            (
                store.identities.leaf_capacity(),
                store.identities.branch_capacity()
            ),
            (44, 43)
        );
        bundle_store_oracle(&store);
    }
    #[test]
    fn c16_bundle_store_reserve_and_withdraw() {
        let mut store = RequestBundleStore::try_new(4, 8).unwrap();
        let cells = axis_cells(3, 1);
        let record = bundle_record(1);
        let mut meter = work();
        let index = store.reserve_bundle(&record, &cells, &mut meter).unwrap();
        assert_eq!(index, 0);
        assert_eq!((store.record_len(), store.free_record_len()), (1, 3));
        assert_eq!(store.cells.free_len(), 5);
        assert_eq!(store.identities.free_leaf_len(), 44 - K);
        assert_eq!(store.identities.free_branch_len(), 43 - (K - 1));
        for key in record.tagged_keys() {
            assert_eq!(store.find(key.tag, &key.identity, &mut work()), Ok(Some(0)));
        }
        bundle_store_oracle(&store);
        let mut meter = work();
        store.withdraw_bundle(0, &mut meter).unwrap();
        assert!(store.is_empty());
        assert_eq!(store.free_record_len(), 4);
        assert_eq!(store.cells.free_len(), 8);
        assert_eq!(store.identities.free_leaf_len(), 44);
        assert_eq!(store.identities.free_branch_len(), 43);
        for key in record.tagged_keys() {
            assert_eq!(store.find(key.tag, &key.identity, &mut work()), Ok(None));
        }
        bundle_store_oracle(&store);
    }
    #[test]
    fn c16_bundle_store_tombstone_retains_identities_and_cells() {
        let mut store = RequestBundleStore::try_new(4, 8).unwrap();
        let cells = axis_cells(2, 1);
        let record = bundle_record(1);
        store.reserve_bundle(&record, &cells, &mut work()).unwrap();
        let mut meter = work();
        store.retain_bundle(0, &mut meter).unwrap();
        assert_eq!(
            store.get_record(0).unwrap().state,
            BundleRecordState::RetainedTombstone
        );
        // Retained tombstone keeps every identity and cell occupied.
        for key in record.tagged_keys() {
            assert_eq!(store.find(key.tag, &key.identity, &mut work()), Ok(Some(0)));
        }
        assert_eq!(store.cells.free_len(), 6);
        assert_eq!(store.free_record_len(), 3);
        bundle_store_oracle(&store);
        // A second retain on the tombstone is rejected; the tombstone blocks no
        // slot reuse until pristine withdrawal.
        let mut meter = work();
        assert_eq!(
            store.retain_bundle(0, &mut meter),
            Err(FixedStorageError::NonCanonical)
        );
        store.withdraw_bundle(0, &mut work()).unwrap();
        assert!(store.is_empty());
        assert_eq!(store.cells.free_len(), 8);
        bundle_store_oracle(&store);
    }
    #[test]
    fn c16_bundle_store_reuse_and_churn_beyond_capacity() {
        let mut store = RequestBundleStore::try_new(4, 8).unwrap();
        let mut reserved = 0usize;
        for cycle in 0..7 {
            let record = bundle_record((cycle * 4 + 1) as u8);
            let cells = axis_cells(1 + cycle % 2, 1);
            store.reserve_bundle(&record, &cells, &mut work()).unwrap();
            reserved += 1;
            bundle_store_oracle(&store);
            for key in record.tagged_keys() {
                let owner = store
                    .find(key.tag, &key.identity, &mut work())
                    .unwrap()
                    .expect("reserved identity present");
                assert!(
                    store
                        .get_record(owner)
                        .unwrap()
                        .tagged_keys()
                        .contains(&key),
                    "owner record contains the reserved identity"
                );
            }
            if cycle % 2 == 1 {
                for index in 0..2 {
                    store.withdraw_bundle(index as u32, &mut work()).unwrap();
                }
                bundle_store_oracle(&store);
            }
        }
        assert!(reserved > 4, "record churn exceeds record capacity");
        assert!(store.record_len() > 0);
        bundle_store_oracle(&store);
        // Deterministic LIFO record reuse: after withdrawing record 0, the next
        // reserve reuses record slot 0.
        let mut store = RequestBundleStore::try_new(4, 8).unwrap();
        let first = bundle_record(1);
        let cells = axis_cells(3, 1);
        assert_eq!(
            store.reserve_bundle(&first, &cells, &mut work()).unwrap(),
            0
        );
        store.withdraw_bundle(0, &mut work()).unwrap();
        let second = bundle_record(50);
        assert_eq!(
            store.reserve_bundle(&second, &cells, &mut work()).unwrap(),
            0
        );
        bundle_store_oracle(&store);
    }
    #[test]
    fn c16_bundle_store_rollback_on_rejection() {
        // Duplicate identity rejects before mutation.
        let mut store = RequestBundleStore::try_new(4, 8).unwrap();
        let cells = axis_cells(2, 1);
        let first = bundle_record(1);
        store.reserve_bundle(&first, &cells, &mut work()).unwrap();
        let before = store.clone();
        let mut meter = work();
        let duplicate = store.reserve_bundle(&first, &cells, &mut meter);
        assert_eq!(duplicate, Err(FixedStorageError::Duplicate));
        assert_eq!(store, before);
        // Record exhaustion rejects before mutation.
        let mut store = RequestBundleStore::try_new(2, 8).unwrap();
        for n in [1u8, 2] {
            store
                .reserve_bundle(&bundle_record(n), &axis_cells(2, 1), &mut work())
                .unwrap();
        }
        let before = store.clone();
        let mut meter = work();
        let full = store.reserve_bundle(&bundle_record(3), &axis_cells(2, 1), &mut meter);
        assert_eq!(full, Err(FixedStorageError::Capacity));
        assert_eq!(store, before);
        // Cell exhaustion rejects before mutation.
        let mut store = RequestBundleStore::try_new(4, 3).unwrap();
        store
            .reserve_bundle(&bundle_record(1), &axis_cells(3, 1), &mut work())
            .unwrap();
        let before = store.clone();
        let mut meter = work();
        let full = store.reserve_bundle(&bundle_record(2), &axis_cells(1, 1), &mut meter);
        assert_eq!(full, Err(FixedStorageError::Capacity));
        assert_eq!(store, before);
        // Work exhaustion during preflight rejects with exact rollback.
        let mut store = RequestBundleStore::try_new(4, 8).unwrap();
        let before = store.clone();
        let mut meter = work();
        meter
            .record(WorkDimension::VisitedEntities, 1_704_575)
            .unwrap();
        let fault = store.reserve_bundle(&bundle_record(1), &axis_cells(2, 1), &mut meter);
        let error =
            WorkBudgetError::BudgetExceeded(WorkDimension::VisitedEntities, 1_704_575, 1_704_576);
        assert_eq!(fault, Err(FixedStorageError::Work(error)));
        assert_eq!(store, before);
        // Withdrawal of a vacant record rejects.
        let mut store = RequestBundleStore::try_new(4, 8).unwrap();
        let mut meter = work();
        assert_eq!(
            store.withdraw_bundle(0, &mut meter),
            Err(FixedStorageError::NonCanonical)
        );
    }
}
