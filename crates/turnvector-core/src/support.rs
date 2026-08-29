use crate::bounded::FixedWindowStart;
use crate::{
    Duration, FixedRecordArena, FixedStartCountBound, FixedStorageError, FixedWindowCounter,
    FutureTurnSupportEntitlementId, HotPathWorkWitness, MonotonicTime, PhysicalStartCreditId,
    RequestId, RuntimeOverheadBoundSetId, SupportLedgerGeneration, SupportOperationObligationId,
    SupportOutstandingCreditVectorId, WorkBudgetError, WorkDimension, WorkMeter,
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
            // C16-only typed facts: constructible only by the complete
            // request-bundle path, never by a generic reserve.
            Self::AdmissionInitial(_) | Self::EntitlementVector(_) => return false,
            Self::LifecycleReserve(id) => (id, 0),
        };
        identity != [0; 32] && pools & (1 << pool as usize) != 0
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SupportCausalPredecessorId(pub [u8; 32]);
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SupportCallScopeId(pub(crate) [u8; 32]);
macro_rules! private_digest_identity {
    ($($name:ident),+ $(,)?) => {$(
        #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub(crate) struct $name([u8; 32]);
        impl $name {
            pub(crate) fn new(bytes: [u8; 32]) -> Result<Self, crate::DomainValueError> {
                (bytes != [0; 32])
                    .then_some(Self(bytes))
                    .ok_or(crate::DomainValueError::Zero)
            }
            pub(crate) const fn get(self) -> [u8; 32] {
                self.0
            }
        }
    )+ };
}
private_digest_identity!(
    AdmissionInitialClaimId,
    TimingCommitmentId,
    RequestClosureId,
    OwnerThreadSupportBudgetId,
);
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SupportInputBucket(u16);
impl SupportInputBucket {
    pub(crate) fn new(value: u16) -> Result<Self, crate::DomainValueError> {
        (value != 0)
            .then_some(Self(value))
            .ok_or(crate::DomainValueError::Zero)
    }
    pub(crate) const fn get(self) -> u16 {
        self.0
    }
}
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
    reserved: [[u32; POOLS]; 5],
    starts: FixedWindowCounter<21, H>,
    lifecycle_maxima: LifecycleReserveMaxima,
    lifecycle_batch_max: u16,
    bundles: RequestBundleStore,
    bundle_vector_max: u16,
    vector_capacity: [[u64; H]; 21],
    vector_usage: [[u64; H]; 21],
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
    #[allow(
        clippy::too_many_arguments,
        reason = "constructor binds the complete ledger facts"
    )]
    pub(crate) fn try_new(
        generation: SupportLedgerGeneration,
        capacities: [[u32; POOLS]; 5],
        max_claims: u16,
        starts: [[FixedStartCountBound; H]; 21],
        lifecycle_maxima: LifecycleReserveMaxima,
        bundle_records: usize,
        bundle_cells: usize,
        bundle_vector_max: usize,
    ) -> Result<Self, SupportLedgerError> {
        let records = usize::try_from(total(capacities[CREDITS]))
            .map_err(|_| SupportLedgerError::InvalidInput)?;
        let claims = usize::try_from(total(capacities[CLAIMS]))
            .map_err(|_| SupportLedgerError::InvalidInput)?;
        let capacity_consistent = (0..POOLS).all(|pool| {
            (0..=ACTIVE).all(|class| {
                capacities[class][pool] <= capacities[CREDITS][pool]
                    && capacities[class][pool] <= capacities[CLAIMS][pool]
            })
        });
        let valid = (1..=1_024).contains(&max_claims)
            && total(capacities[..3].iter().flatten().copied()) <= R as u64
            && records <= R
            && claims <= F
            && capacity_consistent
            && lifecycle_maxima
                .0
                .into_iter()
                .all(|maximum| maximum > 0 && maximum as usize <= R)
            && H > 0
            && H <= 8
            && bundle_records > 0
            && bundle_cells > 0
            && bundle_vector_max > 0
            && bundle_vector_max <= bundle_cells
            && bundle_vector_max <= 21 * H;
        valid
            .then_some(())
            .ok_or(SupportLedgerError::InvalidInput)?;
        let history_slots = starts.iter().try_fold(0u64, |total, row| {
            total.checked_add(u64::from(row[H - 1].1))
        });
        let storage = support_storage_bytes(
            H,
            records,
            claims,
            history_slots.ok_or(SupportLedgerError::InvalidInput)?,
            bundle_records,
            bundle_cells,
        )?;
        if storage > 2_097_152 {
            return Err(SupportLedgerError::Storage(FixedStorageError::Capacity));
        }
        let maxima = lifecycle_maxima.0;
        let shared = u32::from(maxima[0])
            .checked_add(u32::from(maxima[1]))
            .ok_or(SupportLedgerError::InvalidInput)?;
        let batch = shared
            .max(u32::from(maxima[2]))
            .max(u32::from(maxima[3]))
            .max(u32::from(maxima[4]))
            .min(u32::from(u16::MAX));
        let lifecycle_batch_max =
            u16::try_from(batch).map_err(|_| SupportLedgerError::InvalidInput)?;
        let height = u64::from(crate::bounded::AvlIndex::height_bound(
            records
                .checked_mul(2)
                .ok_or(SupportLedgerError::InvalidInput)?,
        )?);
        let batch = u64::from(lifecycle_batch_max);
        let worst_visits = (567u64)
            .checked_add(10 * height)
            .and_then(|per_member| per_member.checked_mul(batch))
            .ok_or(SupportLedgerError::InvalidInput)?;
        let worst_copied = 300u64
            .checked_mul(batch)
            .ok_or(SupportLedgerError::InvalidInput)?;
        let worst_checks = 16u64
            .checked_mul(batch)
            .and_then(|value| value.checked_add(13))
            .ok_or(SupportLedgerError::InvalidInput)?;
        if worst_visits > 1_704_575 || worst_copied > 2_097_152 || worst_checks > 28_708 {
            return Err(SupportLedgerError::InvalidInput);
        }
        let records = FixedRecordArena::try_new(records, claims)?;
        let vector_capacity = std::array::from_fn(|cell| {
            std::array::from_fn(|horizon| u64::from(starts[cell][horizon].1))
        });
        let starts = FixedWindowCounter::try_new(starts)?;
        let bundles = RequestBundleStore::try_new(bundle_records, bundle_cells)?;
        let bundle_vector_max =
            u16::try_from(bundle_vector_max).map_err(|_| SupportLedgerError::InvalidInput)?;
        let instance_nonce = issue_instance_nonce(&PROCESS_INSTANCE_DISPENSER)
            .ok_or(SupportLedgerError::Storage(FixedStorageError::Capacity))?;
        Ok(Self {
            generation,
            capacities,
            max_claims,
            records,
            usage: [[0; POOLS]; 5],
            reserved: [[0; POOLS]; 5],
            starts,
            lifecycle_maxima,
            lifecycle_batch_max,
            bundles,
            bundle_vector_max,
            vector_capacity,
            vector_usage: [[0; H]; 21],
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
        // The claim-to-pool pairing is the sole authority: generic reserve
        // funds OrdinaryReservation on the Ordinary pool, and the C16-only
        // AdmissionInitial/EntitlementVector claims reject above.
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
        for identity in [key(0, spec.id.get()), key(1, spec.physical_credit.get())] {
            let absent = self.records.find(identity, work)?.is_none();
            check!(work, absent, FixedStorageError::Duplicate)?;
        }
        self.reciprocal_absent(spec.id.get(), spec.physical_credit.get(), work)?;
        self.records.validate_capacity(count)?;
        let height = u64::from(self.records.maximum_identity_height()?);
        let mutation = HotPathWorkWitness::new([
            8 * height + 34,
            (std::mem::size_of::<Record>()
                + std::mem::size_of::<u32>()
                + std::mem::size_of_val(spec.claims)
                + 2 * 56) as u64,
            0,
            0,
            6,
        ]);
        work.charge(mutation)?;
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
        self.records.push_prevalidated(keys, record, spec.claims);
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
        self.records.validate_capacity(1)?;
        let height = u64::from(self.records.maximum_identity_height()?);
        work.charge(HotPathWorkWitness::new([8 * height + 34, 300, 0, 0, 6]))?;
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
        _work: &mut WorkMeter,
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
                self.records.push_prevalidated(keys, record, &[spec.claim]);
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
        self.records.validate_capacity(specs.len())?;
        let members = u64::try_from(specs.len()).map_err(|_| invalid)?;
        let height = u64::from(self.records.maximum_identity_height()?);
        work.charge(HotPathWorkWitness::new([
            members * (8 * height + 34),
            members * 300,
            0,
            0,
            4 + 2 * members,
        ]))?;
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
            self.records.push_prevalidated(keys, record, &[spec.claim]);
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
    fn bundle_logical_delta(
        &self,
        cells: impl IntoIterator<Item = OutstandingCreditCell>,
    ) -> Result<BundleLogicalDelta<H>, SupportLedgerError> {
        let invalid = SupportLedgerError::InvalidInput;
        let mut delta = BundleLogicalDelta {
            usage: [[0; POOLS]; 5],
            reserved: [[0; POOLS]; 5],
            vector: [[0; H]; 21],
        };
        let mut maxima = [[0u64; POOLS]; 7];
        let mut pool_totals = [0u64; POOLS];
        for cell in cells {
            let operation = cell.operation as usize;
            let pool = cell.pool as usize;
            let axis = operation
                .checked_mul(POOLS)
                .and_then(|value| value.checked_add(pool))
                .filter(|&value| value < 21)
                .ok_or(invalid)?;
            let horizon = self
                .starts
                .bounds(axis)
                .ok_or(invalid)?
                .iter()
                .position(|bound| bound.0 == cell.horizon)
                .ok_or(invalid)?;
            delta.vector[axis][horizon] = cell.max_outstanding;
            let prior = maxima[operation][pool];
            let maximum = prior.max(cell.max_outstanding);
            pool_totals[pool] = pool_totals[pool]
                .checked_add(maximum - prior)
                .ok_or(invalid)?;
            maxima[operation][pool] = maximum;
        }
        let mandatory = MandatoryCompletion as usize;
        for class in [CONDITIONAL, CREDITS, CLAIMS] {
            delta.usage[class][mandatory] = 3;
        }
        for class in [PENDING, ACTIVE] {
            delta.reserved[class][mandatory] = 3;
        }
        for class in 0..5 {
            for (pool, total) in pool_totals.into_iter().enumerate() {
                delta.reserved[class][pool] = u32::try_from(total).map_err(|_| invalid)?;
            }
        }
        delta.reserved[PENDING][mandatory] = delta.reserved[PENDING][mandatory]
            .checked_add(3)
            .ok_or(invalid)?;
        delta.reserved[ACTIVE][mandatory] = delta.reserved[ACTIVE][mandatory]
            .checked_add(3)
            .ok_or(invalid)?;
        Ok(delta)
    }
    fn validate_bundle_logical_delta(
        &self,
        cells: &[OutstandingCreditCell],
        work: &mut WorkMeter,
    ) -> Result<BundleLogicalDelta<H>, SupportLedgerError> {
        let delta = self.bundle_logical_delta(cells.iter().copied())?;
        for cell in cells {
            work.record(WorkDimension::VisitedEntities, H as u64 + 1)?;
            let axis = cell.operation as usize * POOLS + cell.pool as usize;
            let horizon = self
                .starts
                .bounds(axis)
                .and_then(|bounds| bounds.iter().position(|bound| bound.0 == cell.horizon));
            work.record(WorkDimension::InvariantChecks, 1)?;
            let horizon = horizon.ok_or(SupportLedgerError::InvalidInput)?;
            let updated = self.vector_usage[axis][horizon].checked_add(cell.max_outstanding);
            work.record(WorkDimension::InvariantChecks, 1)?;
            let updated = updated.ok_or(SupportLedgerError::InvalidInput)?;
            check!(
                work,
                updated <= self.vector_capacity[axis][horizon],
                CAPACITY_ERROR
            )?;
            work.record(WorkDimension::InvariantChecks, 1)?;
        }
        for class in 0..5 {
            for pool in 0..POOLS {
                let valid = self.usage[class][pool]
                    .checked_add(self.reserved[class][pool])
                    .and_then(|value| value.checked_add(delta.usage[class][pool]))
                    .and_then(|value| value.checked_add(delta.reserved[class][pool]))
                    .is_some_and(|value| value <= self.capacities[class][pool]);
                check!(work, valid, CAPACITY_ERROR)?;
            }
        }
        Ok(delta)
    }
    fn validate_bundle_logical_delta_precharged(
        &self,
        cells: &[OutstandingCreditCell],
    ) -> Result<(), SupportLedgerError> {
        let delta = self.bundle_logical_delta(cells.iter().copied())?;
        for class in 0..5 {
            for pool in 0..POOLS {
                let valid = self.usage[class][pool]
                    .checked_add(self.reserved[class][pool])
                    .and_then(|value| value.checked_add(delta.usage[class][pool]))
                    .and_then(|value| value.checked_add(delta.reserved[class][pool]))
                    .is_some_and(|value| value <= self.capacities[class][pool]);
                if !valid {
                    return Err(CAPACITY_ERROR);
                }
            }
        }
        Ok(())
    }
    fn apply_bundle_logical_delta(&mut self, cells: &[OutstandingCreditCell], add: bool) {
        let delta = self
            .bundle_logical_delta(cells.iter().copied())
            .expect("validated bundle logical delta");
        for class in 0..5 {
            for pool in 0..POOLS {
                if add {
                    self.usage[class][pool] += delta.usage[class][pool];
                    self.reserved[class][pool] += delta.reserved[class][pool];
                } else {
                    self.usage[class][pool] -= delta.usage[class][pool];
                    self.reserved[class][pool] -= delta.reserved[class][pool];
                }
            }
        }
        for axis in 0..21 {
            for horizon in 0..H {
                if add {
                    self.vector_usage[axis][horizon] += delta.vector[axis][horizon];
                } else {
                    self.vector_usage[axis][horizon] -= delta.vector[axis][horizon];
                }
            }
        }
    }
    fn remove_stored_bundle_logical_delta(&mut self, record_index: u32) {
        let record = self
            .bundles
            .get_record(record_index)
            .expect("validated occupied bundle");
        let mut next = record.vector_head;
        let cells = &self.bundles.cells.slots;
        let delta = self
            .bundle_logical_delta((0..record.vector_len).map(|_| {
                let CellSlot::Occupied {
                    cell, next_owned, ..
                } = cells[next as usize]
                else {
                    unreachable!("validated occupied bundle cell")
                };
                next = next_owned;
                cell
            }))
            .expect("validated stored bundle logical delta");
        for class in 0..5 {
            for pool in 0..POOLS {
                self.usage[class][pool] -= delta.usage[class][pool];
                self.reserved[class][pool] -= delta.reserved[class][pool];
            }
        }
        for axis in 0..21 {
            for horizon in 0..H {
                self.vector_usage[axis][horizon] -= delta.vector[axis][horizon];
            }
        }
    }
    /// Read-only metered preparation of one complete C16 request bundle. The
    /// input binds exactly `K = 11` tagged identities (`q = 3` canonical
    /// obligations, credits, and `AdmissionInitial` claims, one Future Turn
    /// Support Entitlement, one Support Outstanding Credit Vector), all
    /// reciprocally absent from the legacy and C16 stores, with free
    /// record/cell/leaf/branch capacity and the complete reserve Work
    /// envelope preflighted before any mutation. Returns the non-forgeable
    /// instance-bound `BundleChange`; dropping it changes no state.
    pub(crate) fn prepare_bundle<'input, 'work>(
        &self,
        input: &'input RequestSupportBundleInput<'input>,
        work: &'work mut WorkMeter,
    ) -> Result<BundleChange<'input, 'work, H>, SupportLedgerError> {
        let height = self.records.maximum_identity_height()?;
        work.charge(bundle_reserve_work::<H>(input.cells.len(), height)?)?;
        self.generation
            .next()
            .map_err(|_| SupportLedgerError::Generation)?;
        let invalid = SupportLedgerError::InvalidInput;
        if input.cells.is_empty()
            || input.cells.len() > usize::from(self.bundle_vector_max)
            || input.cells.len() > 168
        {
            return Err(invalid);
        }
        let mut prior_axis = None;
        for cell in input.cells {
            if cell.max_outstanding == 0 || cell.horizon.as_micros() == 0 {
                return Err(invalid);
            }
            let axis = (cell.operation, cell.pool, cell.horizon.as_micros());
            if prior_axis.is_some_and(|prior| prior >= axis) {
                return Err(invalid);
            }
            prior_axis = Some(axis);
        }
        let initial = input.initial.values();
        let obligations = initial.map(|requirement| requirement.obligation);
        let credits = initial.map(|requirement| requirement.credit);
        let claims = initial.map(|requirement| requirement.claim.get());
        for group in [
            obligations.map(|id| id.get()),
            credits.map(|id| id.get()),
            claims,
        ] {
            if !group.windows(2).all(|pair| pair[0] < pair[1]) {
                return Err(invalid);
            }
        }
        let initial_shapes = [
            (SupportOperation::MaterializeRequest, MandatoryCompletion),
            (SupportOperation::FormCandidates, MandatoryCompletion),
            (SupportOperation::ReleaseRequest, MandatoryCompletion),
        ];
        for (requirement, shape) in initial.into_iter().zip(initial_shapes) {
            if (requirement.operation, requirement.pool) != shape
                || requirement.predecessor.0 == [0; 32]
                || requirement.scope.0 == [0; 32]
                || requirement.input_bucket.get() == 0
                || requirement.prospective_bound.as_micros() == 0
            {
                return Err(invalid);
            }
        }
        let branch_shapes = [
            (SupportOperation::ObserveTurnReceipt, MandatoryCompletion),
            (SupportOperation::FormCandidates, MandatoryCompletion),
            (SupportOperation::FormCandidates, MandatoryCompletion),
            (SupportOperation::FormCandidates, MandatoryCompletion),
        ];
        for (requirement, shape) in input.branches.values().into_iter().zip(branch_shapes) {
            if (requirement.operation, requirement.pool) != shape
                || requirement.input_bucket.get() == 0
                || requirement.prospective_bound.as_micros() == 0
            {
                return Err(invalid);
            }
        }
        let vector_len = u32::try_from(input.cells.len()).map_err(|_| invalid)?;
        let record = BundleRecord::from_input(input, vector_len);
        let mut identities = [NO_NODE; 17];
        for (slot, key) in identities[..K].iter_mut().zip(record.tagged_keys()) {
            let (terminal, found) = self.bundles.route_precharged(key.tag, &key.identity)?;
            if found.is_some() {
                return Err(FixedStorageError::Duplicate.into());
            }
            *slot = terminal;
        }
        for identity in obligations
            .iter()
            .map(|id| key(0, id.get()))
            .chain(credits.iter().map(|id| key(1, id.get())))
        {
            if self.records.find_precharged(identity)?.is_some() {
                return Err(FixedStorageError::Duplicate.into());
            }
        }
        self.validate_bundle_logical_delta_precharged(input.cells)?;
        let branch_need = if self.bundles.is_empty() { K - 1 } else { K };
        if self.bundles.free_record_len() < 1
            || self.bundles.free_cell_len() < input.cells.len()
            || self.bundles.free_leaf_len() < K
            || self.bundles.free_branch_len() < branch_need
        {
            return Err(CAPACITY_ERROR);
        }
        Ok(BundleChange {
            work,
            nonce: self.instance_nonce,
            snapshot: self.capacity_snapshot(),
            identities,
            record,
            vector: input.cells,
        })
    }
    /// Metered exclusive validation of one prepared C16 bundle change. Consumes
    /// the `BundleChange`, verifies the exact non-reused instance nonce, and
    /// takes the ordinary exclusive borrow of the sole ledger. Every fixed
    /// semantic before-image and metered tagged-identity lookup is rechecked,
    /// and the current top `v` free cell indices plus the exact record slot it
    /// will use are selected after exclusivity. Returns the non-forgeable
    /// exclusive `ValidatedBundleChange`; a rejection or a drop changes no
    /// state.
    pub(crate) fn validate_bundle<'ledger, 'input, 'work>(
        &'ledger mut self,
        change: BundleChange<'input, 'work, H>,
    ) -> Result<ValidatedBundleChange<'ledger, 'input, 'work, R, F, H>, SupportLedgerError> {
        let branches = if change.snapshot.occupied_records == 0 {
            K - 1
        } else {
            K
        };
        let height = self.records.maximum_identity_height()?;
        change.work.charge(bundle_validate_commit_work::<H>(
            change.vector.len(),
            branches,
            height,
        )?)?;
        let stale = SupportLedgerError::Generation;
        if change.nonce != self.instance_nonce || change.snapshot != self.capacity_snapshot() {
            return Err(stale);
        }
        self.validate_bundle_logical_delta_precharged(change.vector)?;
        for (slot, key) in change.record.tagged_keys().into_iter().enumerate() {
            let (terminal, found) = self.bundles.route_precharged(key.tag, &key.identity)?;
            if found.is_some() || terminal != change.identities[slot] {
                return Err(stale);
            }
        }
        for identity in change
            .record
            .obligations()
            .iter()
            .map(|id| key(0, id.get()))
            .chain(change.record.credits().iter().map(|id| key(1, id.get())))
        {
            if self.records.find_precharged(identity)?.is_some() {
                return Err(stale);
            }
        }
        self.bundles
            .validate_bundle_selection_precharged(change.vector.len(), branches)?;
        Ok(ValidatedBundleChange {
            ledger: self,
            change,
        })
    }
    /// Immutable metered capacity-facts snapshot of the sole ledger and its
    /// C16 request-bundle store.
    pub(crate) fn snapshot(
        &self,
        work: &mut WorkMeter,
    ) -> Result<SupportCapacitySnapshot<H>, SupportLedgerError> {
        let copied = u64::try_from(H)
            .ok()
            .and_then(|horizon| horizon.checked_mul(336))
            .and_then(|bytes| bytes.checked_add(216))
            .ok_or(SupportLedgerError::InvalidInput)?;
        work.charge(HotPathWorkWitness::new([0, copied, 0, 0, 1]))?;
        Ok(self.capacity_snapshot())
    }
    fn capacity_snapshot(&self) -> SupportCapacitySnapshot<H> {
        SupportCapacitySnapshot {
            generation: self.generation,
            capacities: self.capacities,
            usage: self.usage,
            reserved: self.reserved,
            bundle_vector_max: self.bundle_vector_max,
            vector_capacity: self.vector_capacity,
            vector_usage: self.vector_usage,
            free_records: u32::try_from(self.bundles.free_record_len())
                .expect("constructor-bounded record count"),
            free_cells: u32::try_from(self.bundles.free_cell_len())
                .expect("constructor-bounded cell count"),
            free_leaves: u32::try_from(self.bundles.free_leaf_len())
                .expect("constructor-bounded leaf count"),
            free_branches: u32::try_from(self.bundles.free_branch_len())
                .expect("constructor-bounded branch count"),
            occupied_records: self.bundles.occupied_records,
        }
    }
    fn validate_capacity_snapshot(
        &self,
        snapshot: &SupportCapacitySnapshot<H>,
        work: &mut WorkMeter,
        stale: SupportLedgerError,
    ) -> Result<(), SupportLedgerError> {
        check!(work, snapshot.generation == self.generation, stale)?;
        check!(
            work,
            snapshot.bundle_vector_max == self.bundle_vector_max,
            stale
        )?;
        for class in 0..5 {
            for pool in 0..POOLS {
                check!(
                    work,
                    self.capacities[class][pool] == snapshot.capacities[class][pool],
                    stale
                )?;
                check!(
                    work,
                    self.usage[class][pool] == snapshot.usage[class][pool],
                    stale
                )?;
                check!(
                    work,
                    self.reserved[class][pool] == snapshot.reserved[class][pool],
                    stale
                )?;
            }
        }
        for cell in 0..21 {
            for horizon in 0..H {
                check!(
                    work,
                    self.vector_capacity[cell][horizon] == snapshot.vector_capacity[cell][horizon],
                    stale
                )?;
                check!(
                    work,
                    self.vector_usage[cell][horizon] == snapshot.vector_usage[cell][horizon],
                    stale
                )?;
            }
        }
        check!(
            work,
            self.bundles.occupied_records == snapshot.occupied_records,
            stale
        )?;
        check!(
            work,
            self.bundles.free_record_len() as u32 == snapshot.free_records,
            stale
        )?;
        check!(
            work,
            self.bundles.free_cell_len() as u32 == snapshot.free_cells,
            stale
        )?;
        check!(
            work,
            self.bundles.free_leaf_len() as u32 == snapshot.free_leaves,
            stale
        )?;
        check!(
            work,
            self.bundles.free_branch_len() as u32 == snapshot.free_branches,
            stale
        )?;
        Ok(())
    }
    /// Read-only metered preparation of one pristine C16 bundle withdrawal:
    /// locates the exact live record by its entitlement, proves the record and
    /// its complete owned cell chain, and preflights the complete removal Work
    /// envelope before any mutation. Returns the non-forgeable
    /// instance-bound `PreparedWithdrawal`; dropping it changes no state. A
    /// retained terminal tombstone is not pristine and rejects.
    pub(crate) fn prepare_withdraw<'work>(
        &self,
        entitlement: FutureTurnSupportEntitlementId,
        work: &'work mut WorkMeter,
    ) -> Result<PreparedWithdrawal<'work, H>, SupportLedgerError> {
        let expected = self.generation;
        self.next(expected, work)?;
        let invalid = SupportLedgerError::InvalidTransition;
        let record_index = self
            .bundles
            .find(TAG_ENTITLEMENT, &entitlement.get(), work)?
            .ok_or(invalid)?;
        let record = *self.bundles.get_record(record_index).ok_or(invalid)?;
        check!(
            work,
            record.state == BundleState::LivePristine
                && record.linked_claims == 0
                && record.initial.iter().all(|item| item.state == Conditional),
            invalid
        )?;
        check!(work, record.entitlement == entitlement, invalid)?;
        let head = record.vector_head;
        let len = usize::try_from(record.vector_len).map_err(|_| invalid)?;
        self.bundles
            .validate_owner_chain(head, len, record_index, work)?;
        work.ensure(HotPathWorkWitness::new([
            K as u64 * (u64::from(IDENTITY_BITS) + 1),
            0,
            0,
            0,
            K as u64 * (u64::from(IDENTITY_BITS) + 1),
        ]))?;
        Ok(PreparedWithdrawal {
            work,
            nonce: self.instance_nonce,
            snapshot: self.capacity_snapshot(),
            record_index,
            leaves: [NO_NODE; K],
            record,
        })
    }
    /// Metered exclusive validation of one prepared pristine-withdrawal
    /// change. Consumes the `PreparedWithdrawal`, verifies the exact non-reused
    /// instance nonce, and rechecks the target record before-image and its
    /// complete owner cell chain under the ordinary exclusive ledger borrow.
    /// Returns the exclusive `ValidatedWithdrawal`; a rejection or a drop
    /// changes no state.
    pub(crate) fn validate_withdraw<'ledger, 'work>(
        &'ledger mut self,
        mut change: PreparedWithdrawal<'work, H>,
    ) -> Result<ValidatedWithdrawal<'ledger, 'work, R, F, H>, SupportLedgerError> {
        let work = &mut *change.work;
        let stale = SupportLedgerError::Generation;
        check!(work, change.nonce == self.instance_nonce, stale)?;
        self.validate_capacity_snapshot(&change.snapshot, work, stale)?;
        let found = self
            .bundles
            .find(TAG_ENTITLEMENT, &change.record.entitlement.get(), work)?
            .ok_or(stale)?;
        check!(work, found == change.record_index, stale)?;
        check!(
            work,
            self.bundles.get_record(change.record_index) == Some(&change.record),
            stale
        )?;
        for (slot, key) in change.leaves.iter_mut().zip(change.record.tagged_keys()) {
            let owner = self
                .bundles
                .find(key.tag, &key.identity, work)?
                .ok_or(stale)?;
            check!(work, owner == change.record_index, stale)?;
            *slot = owner;
        }
        let head = change.record.vector_head;
        self.bundles.validate_owner_chain(
            head,
            usize::try_from(change.record.vector_len).map_err(|_| stale)?,
            change.record_index,
            work,
        )?;
        Ok(ValidatedWithdrawal {
            ledger: self,
            change,
        })
    }
    /// Read-only preparation of a live C16 bundle's retained terminal
    /// tombstone by its entitlement. The returned capability owns the complete
    /// before-image snapshot and the same Work-meter borrow.
    pub(crate) fn prepare_tombstone<'work>(
        &self,
        entitlement: FutureTurnSupportEntitlementId,
        work: &'work mut WorkMeter,
    ) -> Result<PreparedTombstone<'work, H>, SupportLedgerError> {
        self.next(self.generation, work)?;
        let invalid = SupportLedgerError::InvalidTransition;
        let record_index = self
            .bundles
            .find(TAG_ENTITLEMENT, &entitlement.get(), work)?
            .ok_or(invalid)?;
        let record = *self.bundles.get_record(record_index).ok_or(invalid)?;
        check!(
            work,
            matches!(
                record.state,
                BundleState::LivePristine | BundleState::LiveConsumed
            ),
            invalid
        )?;
        check!(work, record.entitlement == entitlement, invalid)?;
        Ok(PreparedTombstone {
            work,
            nonce: self.instance_nonce,
            snapshot: self.capacity_snapshot(),
            record_index,
            record,
        })
    }
    pub(crate) fn validate_tombstone<'ledger, 'work>(
        &'ledger mut self,
        change: PreparedTombstone<'work, H>,
    ) -> Result<ValidatedTombstone<'ledger, 'work, R, F, H>, SupportLedgerError> {
        let work = &mut *change.work;
        let stale = SupportLedgerError::Generation;
        check!(work, change.nonce == self.instance_nonce, stale)?;
        self.validate_capacity_snapshot(&change.snapshot, work, stale)?;
        for key in change.record.tagged_keys() {
            let owner = self
                .bundles
                .find(key.tag, &key.identity, work)?
                .ok_or(stale)?;
            check!(work, owner == change.record_index, stale)?;
        }
        check!(
            work,
            self.bundles.get_record(change.record_index) == Some(&change.record),
            stale
        )?;
        self.bundles.validate_owner_chain(
            change.record.vector_head,
            usize::try_from(change.record.vector_len).map_err(|_| stale)?,
            change.record_index,
            work,
        )?;
        Ok(ValidatedTombstone {
            ledger: self,
            change,
        })
    }
}
impl<'ledger, 'input, 'work, const R: usize, const F: usize, const H: usize>
    ValidatedBundleChange<'ledger, 'input, 'work, R, F, H>
{
    /// Consuming infallible commit of the validated C16 bundle: performs no
    /// new fallible lookup, check, allocation, Work call, or legal rejection
    /// branch, installs the fixed record, all `K` tagged identities, and the
    /// validated cells as one owned chain, and advances the Support Ledger
    /// Generation exactly once. Internal `expect` calls are fail-stop
    /// defenses for impossible owner corruption after validate proved
    /// capacity, absence, and local-slot selection.
    pub(crate) fn commit_bundle(self) -> SupportLedgerGeneration {
        let change = self.change;
        let ledger = self.ledger;
        ledger.bundles.commit_bundle(&change.record, change.vector);
        ledger.apply_bundle_logical_delta(change.vector, true);
        let next = change
            .snapshot
            .generation
            .next()
            .expect("prepared bundle generation");
        ledger.generation = next;
        next
    }
}
impl<'ledger, 'work, const R: usize, const F: usize, const H: usize>
    ValidatedWithdrawal<'ledger, 'work, R, F, H>
{
    /// Consuming infallible pristine withdrawal of the validated C16 bundle:
    /// performs no new fallible lookup, check, allocation, Work call, or
    /// legal rejection branch, removes every tagged identity, releases the
    /// validated cells, vacates the record, and advances the Support Ledger
    /// Generation exactly once. Internal `expect` calls are fail-stop
    /// defenses for impossible owner corruption after validate proved the
    /// exact record and chain.
    pub(crate) fn commit_withdraw(self) -> SupportLedgerGeneration {
        let change = self.change;
        let ledger = self.ledger;
        ledger.remove_stored_bundle_logical_delta(change.record_index);
        ledger
            .bundles
            .withdraw_bundle_unmetered(change.record_index);
        let next = change
            .snapshot
            .generation
            .next()
            .expect("prepared withdrawal generation");
        ledger.generation = next;
        next
    }
}
impl<'ledger, 'work, const R: usize, const F: usize, const H: usize>
    ValidatedTombstone<'ledger, 'work, R, F, H>
{
    pub(crate) fn commit_tombstone(self) -> SupportLedgerGeneration {
        let change = self.change;
        let ledger = self.ledger;
        ledger.bundles.retain_bundle_unmetered(change.record_index);
        let next = change
            .snapshot
            .generation
            .next()
            .expect("prepared tombstone generation");
        ledger.generation = next;
        next
    }
}
#[derive(Clone, Copy)]
struct BundleLogicalDelta<const H: usize> {
    usage: [[u32; POOLS]; 5],
    reserved: [[u32; POOLS]; 5],
    vector: [[u64; H]; 21],
}

fn support_storage_bytes(
    horizon_count: usize,
    records: usize,
    claims: usize,
    history_slots: u64,
    bundle_records: usize,
    bundle_cells: usize,
) -> Result<u64, SupportLedgerError> {
    let invalid = SupportLedgerError::InvalidInput;
    let h = u64::try_from(horizon_count).map_err(|_| invalid)?;
    let r = u64::try_from(records).map_err(|_| invalid)?;
    let f = u64::try_from(claims).map_err(|_| invalid)?;
    let e = u64::try_from(bundle_records).map_err(|_| invalid)?;
    let c = u64::try_from(bundle_cells).map_err(|_| invalid)?;
    let fixed = h
        .checked_mul(672)
        .and_then(|value| value.checked_add(1_232))
        .ok_or(invalid)?;
    let legacy = r
        .checked_mul(260)
        .and_then(|value| {
            f.checked_mul(40)
                .and_then(|claims| value.checked_add(claims))
        })
        .and_then(|value| {
            history_slots
                .checked_mul(8)
                .and_then(|history| value.checked_add(history))
        })
        .ok_or(invalid)?;
    let bundles = e
        .checked_mul(1_364)
        .and_then(|value| value.checked_sub(20))
        .and_then(|value| c.checked_mul(44).and_then(|cells| value.checked_add(cells)))
        .ok_or(invalid)?;
    fixed
        .checked_add(legacy)
        .and_then(|value| value.checked_add(bundles))
        .ok_or(invalid)
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
fn bundle_reserve_work<const H: usize>(
    cells: usize,
    avl_height: u8,
) -> Result<HotPathWorkWitness, SupportLedgerError> {
    let invalid = SupportLedgerError::InvalidInput;
    let v = u64::try_from(cells).map_err(|_| invalid)?;
    let horizon = u64::try_from(H).map_err(|_| invalid)?;
    let keys = u64::try_from(K).map_err(|_| invalid)?;
    let route = u64::from(IDENTITY_BITS).checked_add(2).ok_or(invalid)?;
    let visits = horizon
        .checked_add(2)
        .and_then(|factor| factor.checked_mul(v))
        .and_then(|value| {
            keys.checked_mul(route)
                .and_then(|fixed| value.checked_add(fixed))
        })
        .and_then(|value| {
            u64::from(avl_height)
                .checked_mul(6)
                .and_then(|padding| value.checked_add(padding))
        })
        .and_then(|value| value.checked_add(7))
        .ok_or(invalid)?;
    let copied = horizon
        .checked_mul(336)
        .and_then(|value| value.checked_add(1_328))
        .ok_or(invalid)?;
    let checks = v
        .checked_mul(6)
        .and_then(|value| {
            u64::from(IDENTITY_BITS)
                .checked_add(4)
                .and_then(|route| keys.checked_mul(route))
                .and_then(|fixed| value.checked_add(fixed))
        })
        .and_then(|value| value.checked_add(95))
        .ok_or(invalid)?;
    Ok(HotPathWorkWitness::new([visits, copied, 0, 0, checks]))
}
fn bundle_validate_commit_work<const H: usize>(
    cells: usize,
    branches: usize,
    avl_height: u8,
) -> Result<HotPathWorkWitness, SupportLedgerError> {
    let invalid = SupportLedgerError::InvalidInput;
    let v = u64::try_from(cells).map_err(|_| invalid)?;
    let b = u64::try_from(branches).map_err(|_| invalid)?;
    let h = u64::try_from(H).map_err(|_| invalid)?;
    let keys = u64::try_from(K).map_err(|_| invalid)?;
    let route = u64::from(IDENTITY_BITS).checked_add(2).ok_or(invalid)?;
    let sum = |terms: &[u64]| {
        terms
            .iter()
            .try_fold(0u64, |total, term| total.checked_add(*term))
            .ok_or(invalid)
    };
    let snapshot = h
        .checked_mul(42)
        .and_then(|value| value.checked_add(45))
        .ok_or(invalid)?;
    let visits = sum(&[
        snapshot,
        keys.checked_mul(route).ok_or(invalid)?,
        u64::from(avl_height).checked_mul(6).ok_or(invalid)?,
        sum(&[1, v, keys, b])?,
        keys.checked_mul(
            route
                .checked_mul(2)
                .and_then(|value| value.checked_add(5))
                .ok_or(invalid)?,
        )
        .ok_or(invalid)?,
        v.checked_mul(2)
            .and_then(|value| value.checked_add(10))
            .ok_or(invalid)?,
    ])?;
    let validated = h
        .checked_mul(336)
        .and_then(|value| value.checked_add(1_344))
        .ok_or(invalid)?;
    let mutation = sum(&[
        1_012,
        keys.checked_mul(12).ok_or(invalid)?,
        b.checked_mul(20).ok_or(invalid)?,
        v.checked_mul(52).ok_or(invalid)?,
        44,
        keys.checked_mul(4)
            .and_then(|value| value.checked_add(4))
            .ok_or(invalid)?,
    ])?;
    let copied = validated.checked_add(mutation).ok_or(invalid)?;
    let checks = sum(&[
        snapshot,
        3,
        17,
        keys.checked_mul(u64::from(IDENTITY_BITS).checked_add(4).ok_or(invalid)?)
            .ok_or(invalid)?,
        3,
        v.checked_mul(3).ok_or(invalid)?,
        keys.checked_mul(3).ok_or(invalid)?,
        b.checked_mul(3).ok_or(invalid)?,
        keys,
        10,
        v.checked_mul(2).ok_or(invalid)?,
    ])?;
    Ok(HotPathWorkWitness::new([visits, copied, 0, 0, checks]))
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
/// Immutable timing, finite closure, owner-thread budget, and runtime-bound
/// identities bound unchanged into one C16 request support entitlement.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SupportTimingFacts {
    pub(crate) timing_commitment: TimingCommitmentId,
    pub(crate) request_closure: RequestClosureId,
    pub(crate) support_budget: OwnerThreadSupportBudgetId,
    pub(crate) bound_set: RuntimeOverheadBoundSetId,
}
/// One named initial requirement. Its exact operation/pool mapping is validated
/// by bundle preparation rather than inferred from caller-selectable ordinals.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct InitialSupportRequirement {
    pub(crate) obligation: SupportOperationObligationId,
    pub(crate) credit: PhysicalStartCreditId,
    pub(crate) claim: AdmissionInitialClaimId,
    pub(crate) operation: SupportOperation,
    pub(crate) pool: SupportPool,
    pub(crate) predecessor: SupportCausalPredecessorId,
    pub(crate) scope: SupportCallScopeId,
    pub(crate) input_bucket: SupportInputBucket,
    pub(crate) prospective_bound: Duration,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct InitialSupportRequirements {
    pub(crate) materialize: InitialSupportRequirement,
    pub(crate) form_candidates: InitialSupportRequirement,
    pub(crate) release: InitialSupportRequirement,
}
impl InitialSupportRequirements {
    fn values(self) -> [InitialSupportRequirement; 3] {
        [self.materialize, self.form_candidates, self.release]
    }
}
/// One named future entitlement branch and its immutable resource axis facts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct FutureSupportBranchRequirement {
    pub(crate) operation: SupportOperation,
    pub(crate) pool: SupportPool,
    pub(crate) input_bucket: SupportInputBucket,
    pub(crate) prospective_bound: Duration,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct FutureSupportBranchRequirements {
    pub(crate) receipt_observation: FutureSupportBranchRequirement,
    pub(crate) continuation_formation: FutureSupportBranchRequirement,
    pub(crate) rejection_or_local_stale_formation: FutureSupportBranchRequirement,
    pub(crate) terminal_membership_change_formation: FutureSupportBranchRequirement,
}
impl FutureSupportBranchRequirements {
    fn values(self) -> [FutureSupportBranchRequirement; 4] {
        [
            self.receipt_observation,
            self.continuation_formation,
            self.rejection_or_local_stale_formation,
            self.terminal_membership_change_formation,
        ]
    }
}
/// Complete accepted C16 semantic input. Cells remain an immutable borrowed
/// canonical sparse vector through validation and consuming commit.
pub(crate) struct RequestSupportBundleInput<'a> {
    pub(crate) request_owner: RequestId,
    pub(crate) timing: SupportTimingFacts,
    pub(crate) initial: InitialSupportRequirements,
    pub(crate) branches: FutureSupportBranchRequirements,
    pub(crate) entitlement: FutureTurnSupportEntitlementId,
    pub(crate) vector: SupportOutstandingCreditVectorId,
    pub(crate) cells: &'a [OutstandingCreditCell],
}
/// Non-forgeable prepared C16 bundle change: fixed-size semantic before-image
/// facts (exact instance nonce, expected generation, complete capacity/usage
/// aggregates, free record/cell/index-node counts, and all `K = 11` tagged
/// identity lookup results) plus the borrowed validated vector and the exact
/// fixed target record. Intentionally not Clone or Copy; dropping it changes
/// no state.
pub(crate) struct BundleChange<'input, 'work, const H: usize> {
    work: &'work mut WorkMeter,
    nonce: u64,
    snapshot: SupportCapacitySnapshot<H>,
    identities: [u32; 17],
    record: BundleRecord,
    vector: &'input [OutstandingCreditCell],
}
impl<const H: usize> std::fmt::Debug for BundleChange<'_, '_, H> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("BundleChange")
            .finish_non_exhaustive()
    }
}
/// Exclusive non-forgeable validated C16 bundle capability: holds the sole
/// `&mut` ledger borrow, the immutable validated change, and no variable
/// destination snapshot. Not Clone or Copy; `commit_bundle` consumes it once
/// and performs no new fallible lookup, check, allocation, or Work call;
/// dropping it releases the borrow without changing any state.
pub(crate) struct ValidatedBundleChange<
    'ledger,
    'input,
    'work,
    const R: usize,
    const F: usize,
    const H: usize,
> {
    ledger: &'ledger mut SupportChargeLedger<R, F, H>,
    change: BundleChange<'input, 'work, H>,
}
impl<const R: usize, const F: usize, const H: usize> std::fmt::Debug
    for ValidatedBundleChange<'_, '_, '_, R, F, H>
{
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ValidatedBundleChange")
            .finish_non_exhaustive()
    }
}
/// Immutable capacity-facts snapshot: constructor-fixed capacities plus the
/// current usage/reserved aggregates and C16 free record/cell/index-node
/// counts. Read-only; never an authority or mutation seam.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SupportCapacitySnapshot<const H: usize> {
    pub(crate) generation: SupportLedgerGeneration,
    pub(crate) capacities: [[u32; POOLS]; 5],
    pub(crate) usage: [[u32; POOLS]; 5],
    pub(crate) reserved: [[u32; POOLS]; 5],
    pub(crate) bundle_vector_max: u16,
    pub(crate) vector_capacity: [[u64; H]; 21],
    pub(crate) vector_usage: [[u64; H]; 21],
    pub(crate) free_records: u32,
    pub(crate) free_cells: u32,
    pub(crate) free_leaves: u32,
    pub(crate) free_branches: u32,
    pub(crate) occupied_records: u32,
}
/// Non-forgeable prepared pristine-withdrawal change: the exact instance
/// nonce, expected generation, target record index, and fixed record
/// before-image. Intentionally not Clone or Copy; dropping it changes no
/// state.
pub(crate) struct PreparedWithdrawal<'work, const H: usize> {
    work: &'work mut WorkMeter,
    nonce: u64,
    snapshot: SupportCapacitySnapshot<H>,
    record_index: u32,
    leaves: [u32; K],
    record: BundleRecord,
}
impl<const H: usize> std::fmt::Debug for PreparedWithdrawal<'_, H> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PreparedWithdrawal")
            .finish_non_exhaustive()
    }
}
/// Exclusive non-forgeable validated pristine-withdrawal capability: holds the
/// sole `&mut` ledger borrow and the fixed withdrawal facts. Not Clone or
/// Copy; `commit_withdraw` consumes it once and performs no new fallible
/// lookup, check, allocation, or Work call; dropping it changes no state.
pub(crate) struct ValidatedWithdrawal<
    'ledger,
    'work,
    const R: usize,
    const F: usize,
    const H: usize,
> {
    ledger: &'ledger mut SupportChargeLedger<R, F, H>,
    change: PreparedWithdrawal<'work, H>,
}
impl<const R: usize, const F: usize, const H: usize> std::fmt::Debug
    for ValidatedWithdrawal<'_, '_, R, F, H>
{
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ValidatedWithdrawal")
            .finish_non_exhaustive()
    }
}
pub(crate) struct PreparedTombstone<'work, const H: usize> {
    work: &'work mut WorkMeter,
    nonce: u64,
    snapshot: SupportCapacitySnapshot<H>,
    record_index: u32,
    record: BundleRecord,
}
impl<const H: usize> std::fmt::Debug for PreparedTombstone<'_, H> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PreparedTombstone")
            .finish_non_exhaustive()
    }
}
pub(crate) struct ValidatedTombstone<'ledger, 'work, const R: usize, const F: usize, const H: usize>
{
    ledger: &'ledger mut SupportChargeLedger<R, F, H>,
    change: PreparedTombstone<'work, H>,
}
impl<const R: usize, const F: usize, const H: usize> std::fmt::Debug
    for ValidatedTombstone<'_, '_, R, F, H>
{
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ValidatedTombstone")
            .finish_non_exhaustive()
    }
}
/// Private fixed entitlement-cell arena owned conceptually by the Support Charge
/// Ledger. Constructor-preallocated exact-capacity `slots` and LIFO `free` stores,
/// no later growth, no hot-path allocation, no public seam.
#[derive(Clone, Debug, Eq, PartialEq)]
struct EntitlementCellArena {
    slots: Vec<CellSlot>,
    free: Vec<u32>,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CellSlot {
    Vacant {
        free_position: u32,
    },
    Occupied {
        owner_record: u32,
        cell: OutstandingCreditCell,
        next_owned: u32,
    },
}
impl EntitlementCellArena {
    fn storage_bytes(capacity: u64) -> Option<u64> {
        capacity.checked_mul(44).filter(|&total| total <= 2_097_152)
    }
    fn try_new(capacity: usize) -> Result<Self, FixedStorageError> {
        if capacity == 0 || capacity >= NO_NODE as usize {
            return Err(FixedStorageError::Capacity);
        }
        let capacity_u64 = u64::try_from(capacity).map_err(|_| FixedStorageError::Allocation)?;
        let slots_bytes = capacity_u64
            .checked_mul(std::mem::size_of::<CellSlot>() as u64)
            .ok_or(FixedStorageError::Allocation)?;
        let free_bytes = capacity_u64
            .checked_mul(std::mem::size_of::<u32>() as u64)
            .ok_or(FixedStorageError::Allocation)?;
        if slots_bytes > isize::MAX as u64
            || free_bytes > isize::MAX as u64
            || slots_bytes
                .checked_add(free_bytes)
                .is_none_or(|total| total > 2_097_152)
        {
            return Err(FixedStorageError::Capacity);
        }
        let mut slots = Vec::new();
        slots
            .try_reserve_exact(capacity)
            .map_err(|_| FixedStorageError::Allocation)?;
        for position in (0..capacity).rev() {
            slots.push(CellSlot::Vacant {
                free_position: u32::try_from(position).map_err(|_| FixedStorageError::Capacity)?,
            });
        }
        let mut free = Vec::new();
        free.try_reserve_exact(capacity)
            .map_err(|_| FixedStorageError::Allocation)?;
        for index in (0..capacity).rev() {
            free.push(u32::try_from(index).map_err(|_| FixedStorageError::Capacity)?);
        }
        seal_exact_capacity(&slots, &free, capacity)?;
        Ok(Self { slots, free })
    }
    fn capacity(&self) -> usize {
        self.slots.len()
    }
    fn free_len(&self) -> usize {
        self.free.len()
    }
    fn validate_selection(
        &self,
        count: usize,
        work: &mut WorkMeter,
    ) -> Result<(), FixedStorageError> {
        check!(work, count <= self.free.len(), FixedStorageError::Capacity)?;
        let start = self.free.len() - count;
        for position in start..self.free.len() {
            work.record(WorkDimension::VisitedEntities, 1)?;
            let index = *self
                .free
                .get(position)
                .ok_or(FixedStorageError::NonCanonical)?;
            check!(
                work,
                (index as usize) < self.slots.len(),
                FixedStorageError::NonCanonical
            )?;
            let slot = &self.slots[index as usize];
            let CellSlot::Vacant { free_position } = *slot else {
                work.record(WorkDimension::InvariantChecks, 1)?;
                return Err(FixedStorageError::NonCanonical);
            };
            work.record(WorkDimension::InvariantChecks, 1)?;
            check!(
                work,
                free_position == position as u32,
                FixedStorageError::NonCanonical
            )?;
        }
        Ok(())
    }
    fn validate_selection_precharged(&self, count: usize) -> Result<(), FixedStorageError> {
        if count > self.free.len() {
            return Err(FixedStorageError::Capacity);
        }
        let start = self.free.len() - count;
        for position in start..self.free.len() {
            let index = *self
                .free
                .get(position)
                .ok_or(FixedStorageError::NonCanonical)?;
            let slot = self
                .slots
                .get(index as usize)
                .ok_or(FixedStorageError::NonCanonical)?;
            if !matches!(slot, CellSlot::Vacant { free_position } if *free_position == position as u32)
            {
                return Err(FixedStorageError::NonCanonical);
            }
        }
        Ok(())
    }
    fn install(&mut self, owner: u32, cells: &[OutstandingCreditCell]) -> (u32, u32) {
        let mut head = NO_NODE;
        for cell in cells.iter().rev() {
            let index = self.free.pop().expect("prevalidated free capacity");
            self.slots[index as usize] = CellSlot::Occupied {
                owner_record: owner,
                cell: *cell,
                next_owned: head,
            };
            head = index;
        }
        (
            head,
            u32::try_from(cells.len()).expect("constructor-bounded cell count"),
        )
    }
    fn validate_chain(
        &self,
        head: u32,
        count: usize,
        owner: u32,
        cells: &[OutstandingCreditCell],
        work: &mut WorkMeter,
    ) -> Result<(), FixedStorageError> {
        check!(work, count == cells.len(), FixedStorageError::NonCanonical)?;
        let mut index = head;
        for (position, expected) in cells.iter().enumerate() {
            work.record(WorkDimension::VisitedEntities, 1)?;
            check!(work, index != NO_NODE, FixedStorageError::NonCanonical)?;
            let slot = self
                .slots
                .get(index as usize)
                .ok_or(FixedStorageError::NonCanonical)?;
            work.record(WorkDimension::InvariantChecks, 1)?;
            let CellSlot::Occupied {
                owner_record,
                cell,
                next_owned,
            } = *slot
            else {
                return Err(FixedStorageError::NonCanonical);
            };
            check!(work, owner_record == owner, FixedStorageError::NonCanonical)?;
            check!(work, cell == *expected, FixedStorageError::NonCanonical)?;
            check!(
                work,
                (next_owned == NO_NODE) == (position + 1 == count),
                FixedStorageError::NonCanonical
            )?;
            index = next_owned;
        }
        Ok(())
    }
    fn validate_owner_chain(
        &self,
        head: u32,
        count: usize,
        owner: u32,
        work: &mut WorkMeter,
    ) -> Result<(), FixedStorageError> {
        let mut index = head;
        for position in 0..count {
            work.record(WorkDimension::VisitedEntities, 1)?;
            check!(work, index != NO_NODE, FixedStorageError::NonCanonical)?;
            let slot = self
                .slots
                .get(index as usize)
                .ok_or(FixedStorageError::NonCanonical)?;
            work.record(WorkDimension::InvariantChecks, 1)?;
            let CellSlot::Occupied {
                owner_record,
                next_owned,
                ..
            } = *slot
            else {
                return Err(FixedStorageError::NonCanonical);
            };
            check!(work, owner_record == owner, FixedStorageError::NonCanonical)?;
            check!(
                work,
                (next_owned == NO_NODE) == (position + 1 == count),
                FixedStorageError::NonCanonical
            )?;
            index = next_owned;
        }
        Ok(())
    }
    fn release(&mut self, head: u32, count: usize) {
        let mut index = head;
        for _ in 0..count {
            let next = match self.slots[index as usize] {
                CellSlot::Occupied { next_owned, .. } => next_owned,
                CellSlot::Vacant { .. } => unreachable!("validated chain slot"),
            };
            let free_position = u32::try_from(self.free.len()).expect("bounded free stack");
            self.slots[index as usize] = CellSlot::Vacant { free_position };
            self.free.push(index);
            index = next;
        }
    }
}
/// Fail-closed exact-capacity seal: both backing Vecs must hold exactly
/// `capacity` slots, never more. `try_reserve_exact` only guarantees at least
/// the requested capacity, so a successful arena must be exactly `C`; anything
/// over-capacity is rejected deterministically, independent of allocator policy.
fn seal_exact_capacity<T, U>(
    slots: &Vec<T>,
    free: &Vec<U>,
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
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LeafSlot {
    Vacant { free_position: u32 },
    Occupied { owner_record: u32, key_ordinal: u8 },
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BranchSlot {
    Vacant { free_position: u32 },
    Occupied(IdentityBranch),
}
#[derive(Clone, Debug, Eq, PartialEq)]
struct TaggedIdentityIndex {
    leaf_slots: Vec<LeafSlot>,
    branch_slots: Vec<BranchSlot>,
    free_leaves: Vec<u32>,
    free_branches: Vec<u32>,
    root: u32,
}
impl TaggedIdentityIndex {
    /// Checked physical storage bytes for `leaf_capacity` leaf slots and
    /// branch slots plus their LIFO free stacks, against the binary
    /// Storage/CopiedBytes maximum.
    fn storage_bytes(leaf_capacity: u64) -> Option<u64> {
        let leaf_slot_bytes = std::mem::size_of::<LeafSlot>() as u64;
        let branch_slot_bytes = std::mem::size_of::<BranchSlot>() as u64;
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
        let leaf_slot_bytes = std::mem::size_of::<LeafSlot>() as u64;
        let branch_slot_bytes = std::mem::size_of::<BranchSlot>() as u64;
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
        for position in (0..leaf_capacity).rev() {
            leaf_slots.push(LeafSlot::Vacant {
                free_position: u32::try_from(position).map_err(|_| FixedStorageError::Capacity)?,
            });
        }
        let mut branch_slots = Vec::new();
        branch_slots
            .try_reserve_exact(branch_capacity)
            .map_err(|_| FixedStorageError::Allocation)?;
        for position in (0..branch_capacity).rev() {
            branch_slots.push(BranchSlot::Vacant {
                free_position: u32::try_from(position).map_err(|_| FixedStorageError::Capacity)?,
            });
        }
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
            leaf_slots,
            branch_slots,
            free_leaves,
            free_branches,
            root: NO_NODE,
        })
    }
    fn leaf_capacity(&self) -> usize {
        self.leaf_slots.len()
    }
    fn branch_capacity(&self) -> usize {
        self.branch_slots.len()
    }
    fn leaf(&self, index: u32) -> Option<(u32, u8)> {
        match self.leaf_slots.get(index as usize)? {
            LeafSlot::Occupied {
                owner_record,
                key_ordinal,
            } => Some((*owner_record, *key_ordinal)),
            LeafSlot::Vacant { .. } => None,
        }
    }
    fn resolved_leaf(
        &self,
        index: u32,
        records: &[RecordSlot],
    ) -> Result<(TaggedKey, u32), FixedStorageError> {
        let (owner_record, key_ordinal) =
            self.leaf(index).ok_or(FixedStorageError::NonCanonical)?;
        let record = match records.get(owner_record as usize) {
            Some(RecordSlot::Occupied(record)) => record,
            _ => return Err(FixedStorageError::NonCanonical),
        };
        let key = record
            .tagged_key(key_ordinal)
            .ok_or(FixedStorageError::NonCanonical)?;
        Ok((key, owner_record))
    }
    fn branch(&self, node: u32) -> Option<&IdentityBranch> {
        match self.branch_slots.get(branch_index(node))? {
            BranchSlot::Occupied(branch) => Some(branch),
            BranchSlot::Vacant { .. } => None,
        }
    }
    fn branch_mut(&mut self, node: u32) -> Option<&mut IdentityBranch> {
        match self.branch_slots.get_mut(branch_index(node))? {
            BranchSlot::Occupied(branch) => Some(branch),
            BranchSlot::Vacant { .. } => None,
        }
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
    fn validate_selection(
        &self,
        leaves: usize,
        branches: usize,
        work: &mut WorkMeter,
    ) -> Result<(), FixedStorageError> {
        check!(
            work,
            leaves <= self.free_leaves.len(),
            FixedStorageError::Capacity
        )?;
        check!(
            work,
            branches <= self.free_branches.len(),
            FixedStorageError::Capacity
        )?;
        let leaf_start = self.free_leaves.len() - leaves;
        for position in leaf_start..self.free_leaves.len() {
            work.record(WorkDimension::VisitedEntities, 1)?;
            let index = self.free_leaves[position];
            check!(
                work,
                (index as usize) < self.leaf_slots.len(),
                FixedStorageError::NonCanonical
            )?;
            work.record(WorkDimension::InvariantChecks, 1)?;
            let LeafSlot::Vacant { free_position } = self.leaf_slots[index as usize] else {
                return Err(FixedStorageError::NonCanonical);
            };
            check!(
                work,
                free_position == position as u32,
                FixedStorageError::NonCanonical
            )?;
        }
        let branch_start = self.free_branches.len() - branches;
        for position in branch_start..self.free_branches.len() {
            work.record(WorkDimension::VisitedEntities, 1)?;
            let index = self.free_branches[position];
            check!(
                work,
                (index as usize) < self.branch_slots.len(),
                FixedStorageError::NonCanonical
            )?;
            work.record(WorkDimension::InvariantChecks, 1)?;
            let BranchSlot::Vacant { free_position } = self.branch_slots[index as usize] else {
                return Err(FixedStorageError::NonCanonical);
            };
            check!(
                work,
                free_position == position as u32,
                FixedStorageError::NonCanonical
            )?;
        }
        Ok(())
    }
    fn validate_selection_precharged(
        &self,
        leaves: usize,
        branches: usize,
    ) -> Result<(), FixedStorageError> {
        if leaves > self.free_leaves.len() || branches > self.free_branches.len() {
            return Err(FixedStorageError::Capacity);
        }
        let leaf_start = self.free_leaves.len() - leaves;
        for position in leaf_start..self.free_leaves.len() {
            let index = self.free_leaves[position];
            if !matches!(
                self.leaf_slots.get(index as usize),
                Some(LeafSlot::Vacant { free_position }) if *free_position == position as u32
            ) {
                return Err(FixedStorageError::NonCanonical);
            }
        }
        let branch_start = self.free_branches.len() - branches;
        for position in branch_start..self.free_branches.len() {
            let index = self.free_branches[position];
            if !matches!(
                self.branch_slots.get(index as usize),
                Some(BranchSlot::Vacant { free_position }) if *free_position == position as u32
            ) {
                return Err(FixedStorageError::NonCanonical);
            }
        }
        Ok(())
    }
    /// Borrowed bounded lookup: follows at most `B = 264` branches plus one
    /// leaf, independent of `E`. No key copy and no allocation; each visited
    /// branch and the visited leaf charge one VisitedEntities and one
    /// InvariantChecks.
    fn find(
        &self,
        records: &[RecordSlot],
        tag: u8,
        identity: &[u8; 32],
        work: &mut WorkMeter,
    ) -> Result<Option<u32>, FixedStorageError> {
        let node = self.locate(tag, identity, work)?;
        if node == NO_NODE {
            return Ok(None);
        }
        let (key, owner) = self.resolved_leaf(node, records)?;
        Ok((key.tag == tag && key.identity == *identity).then_some(owner))
    }
    fn find_precharged(
        &self,
        records: &[RecordSlot],
        tag: u8,
        identity: &[u8; 32],
    ) -> Result<Option<u32>, FixedStorageError> {
        self.route_precharged(records, tag, identity)
            .map(|(_, owner)| owner)
    }
    fn route_precharged(
        &self,
        records: &[RecordSlot],
        tag: u8,
        identity: &[u8; 32],
    ) -> Result<(u32, Option<u32>), FixedStorageError> {
        let mut node = self.root;
        let mut prior = None;
        for _ in 0..=IDENTITY_BITS {
            if node == NO_NODE {
                return Ok((NO_NODE, None));
            }
            if !is_branch(node) {
                let (key, owner) = self.resolved_leaf(node, records)?;
                let found = (key.tag == tag && key.identity == *identity).then_some(owner);
                return Ok((node, found));
            }
            let branch = self.branch(node).ok_or(FixedStorageError::NonCanonical)?;
            if branch.bit >= IDENTITY_BITS
                || prior.is_some_and(|bit| bit >= branch.bit)
                || branch.zero == branch.one
                || branch.zero == node
                || branch.one == node
            {
                return Err(FixedStorageError::NonCanonical);
            }
            prior = Some(branch.bit);
            node = [branch.zero, branch.one][identity_bit(tag, identity, branch.bit)];
        }
        Err(FixedStorageError::NonCanonical)
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
            let branch = self.branch(node).expect("validated occupied branch slot");
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
        records: &[RecordSlot],
        key: TaggedKey,
        record: u32,
        ordinal: u8,
        work: &mut WorkMeter,
    ) -> Result<(), FixedStorageError> {
        let peer = self.locate(key.tag, &key.identity, work)?;
        if peer == NO_NODE {
            work.record(WorkDimension::InvariantChecks, 1)?;
            if self.free_leaves.is_empty() {
                return Err(FixedStorageError::Capacity);
            }
            self.install_root(record, ordinal);
            return Ok(());
        }
        let (peer_key, _) = self.resolved_leaf(peer, records)?;
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
            let branch = self.branch(child).expect("validated occupied branch slot");
            if branch.bit >= bit {
                break;
            }
            parent = child;
            child = [branch.zero, branch.one][identity_bit(key.tag, &key.identity, branch.bit)];
        }
        work.record(WorkDimension::VisitedEntities, visits)?;
        work.record(WorkDimension::InvariantChecks, visits)?;
        self.install_branch(key, record, ordinal, bit, parent, child);
        Ok(())
    }
    /// Infallible installation used only by the consuming C16 bundle commit
    /// under the validated trie invariant and preflighted leaf/branch
    /// capacity: locates the peer, finds the first differing bit, and splices
    /// one leaf plus one branch over the exact path with no Work call, no
    /// fallible branch, and no allocation. Internal `expect` and
    /// `unreachable!` are fail-stop defenses for impossible owner corruption.
    fn install(&mut self, records: &[RecordSlot], key: TaggedKey, record: u32, ordinal: u8) {
        let peer = self.locate_unmetered(key.tag, &key.identity);
        if peer == NO_NODE {
            self.install_root(record, ordinal);
            return;
        }
        let (peer_key, _) = self
            .resolved_leaf(peer, records)
            .expect("validated occupied leaf owner and ordinal");
        debug_assert_ne!(peer_key, key, "validated absent bundle identity");
        let (bit, _) = first_difference(&key, &peer_key);
        let (mut parent, mut child) = (NO_NODE, self.root);
        while is_branch(child) {
            let branch = self.branch(child).expect("validated occupied branch slot");
            if branch.bit >= bit {
                break;
            }
            parent = child;
            child = [branch.zero, branch.one][identity_bit(key.tag, &key.identity, branch.bit)];
        }
        self.install_branch(key, record, ordinal, bit, parent, child);
    }
    /// Unmetered leaf-locating traversal for the infallible commit install.
    fn locate_unmetered(&self, tag: u8, identity: &[u8; 32]) -> u32 {
        let mut node = self.root;
        while node != NO_NODE && is_branch(node) {
            let branch = self.branch(node).expect("validated occupied branch slot");
            node = [branch.zero, branch.one][identity_bit(tag, identity, branch.bit)];
        }
        node
    }
    /// Infallible installation of the first leaf into an empty tree.
    fn install_root(&mut self, owner_record: u32, key_ordinal: u8) {
        let leaf = self.free_leaves.pop().expect("prevalidated leaf capacity");
        self.leaf_slots[leaf as usize] = LeafSlot::Occupied {
            owner_record,
            key_ordinal,
        };
        self.root = leaf;
    }
    /// Infallible installation of one leaf plus one branch above `child`, whose
    /// first discriminating bit `bit` is strictly larger than every parent bit.
    fn install_branch(
        &mut self,
        key: TaggedKey,
        owner_record: u32,
        key_ordinal: u8,
        bit: u16,
        parent: u32,
        child: u32,
    ) {
        let leaf = self.free_leaves.pop().expect("prevalidated leaf capacity");
        self.leaf_slots[leaf as usize] = LeafSlot::Occupied {
            owner_record,
            key_ordinal,
        };
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
        self.branch_slots[branch as usize] = BranchSlot::Occupied(IdentityBranch {
            bit,
            zero: children[0],
            one: children[1],
        });
        if parent == NO_NODE {
            self.root = branch_node;
        } else {
            let parent = self
                .branch_mut(parent)
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
        records: &[RecordSlot],
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
            let branch = self.branch(node).expect("validated occupied branch slot");
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
        let (key, record) = self.resolved_leaf(node, records)?;
        if key.tag != tag || key.identity != *identity {
            return Ok(None);
        }
        self.splice(grandparent, parent, sibling, node);
        Ok(Some(record))
    }
    /// Infallible removal used only by the consuming pristine-withdrawal
    /// commit: walks to the exact leaf for the tagged identity and splices
    /// its sibling over its parent, returning every released leaf and branch
    /// slot to its matching free stack, with no Work call, no fallible
    /// branch, and no allocation. Internal `expect` is a fail-stop defense
    /// for impossible owner corruption after validate proved the exact record
    /// and its cell chain.
    fn remove_unmetered(&mut self, tag: u8, identity: &[u8; 32]) {
        let mut grandparent = NO_NODE;
        let mut parent = NO_NODE;
        let mut sibling = NO_NODE;
        let mut node = self.root;
        while node != NO_NODE && is_branch(node) {
            let branch = self.branch(node).expect("validated occupied branch slot");
            let children = [branch.zero, branch.one];
            let bit = identity_bit(tag, identity, branch.bit);
            sibling = children[1 - bit];
            grandparent = parent;
            parent = node;
            node = children[bit];
        }
        debug_assert_ne!(node, NO_NODE, "validated present bundle identity");
        self.splice(grandparent, parent, sibling, node);
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
            let free_position = u32::try_from(self.free_branches.len())
                .expect("constructor-bounded branch free stack");
            self.branch_slots[branch] = BranchSlot::Vacant { free_position };
            self.free_branches.push(branch as u32);
        } else {
            let grandparent = self
                .branch_mut(grandparent)
                .expect("validated occupied grandparent branch");
            if grandparent.zero == parent {
                grandparent.zero = sibling;
            } else {
                grandparent.one = sibling;
            }
            let branch = branch_index(parent);
            let free_position = u32::try_from(self.free_branches.len())
                .expect("constructor-bounded branch free stack");
            self.branch_slots[branch] = BranchSlot::Vacant { free_position };
            self.free_branches.push(branch as u32);
        }
        let free_position =
            u32::try_from(self.free_leaves.len()).expect("constructor-bounded leaf free stack");
        self.leaf_slots[leaf as usize] = LeafSlot::Vacant { free_position };
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
const TAG_ENTITLEMENT: u8 = 3;
const TAG_VECTOR: u8 = 4;
/// Stored independent state/time and immutable facts for one named initial
/// request-support requirement.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct InitialRequirementRecord {
    obligation: SupportOperationObligationId,
    credit: PhysicalStartCreditId,
    claim: AdmissionInitialClaimId,
    operation: SupportOperation,
    pool: SupportPool,
    predecessor: SupportCausalPredecessorId,
    scope: SupportCallScopeId,
    input_bucket: SupportInputBucket,
    prospective_bound: Duration,
    state: SupportObligationState,
    state_time: MonotonicTime,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FutureBranchRequirementRecord {
    operation: SupportOperation,
    pool: SupportPool,
    input_bucket: SupportInputBucket,
    prospective_bound: Duration,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
enum BundleState {
    LivePristine,
    LiveConsumed,
    RetainedTombstone,
}
/// One fixed complete semantic request-bundle record. Initial requirement state
/// and time remain independent; retained tombstones preserve every identity,
/// cell, claim, timing fact, branch fact, and logical reservation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct BundleRecord {
    initial: [InitialRequirementRecord; 3],
    request_owner: RequestId,
    timing_commitment: TimingCommitmentId,
    request_closure: RequestClosureId,
    support_budget: OwnerThreadSupportBudgetId,
    bound_set: RuntimeOverheadBoundSetId,
    branches: [FutureBranchRequirementRecord; 4],
    entitlement: FutureTurnSupportEntitlementId,
    vector: SupportOutstandingCreditVectorId,
    vector_head: u32,
    vector_len: u32,
    linked_claims: u32,
    state: BundleState,
}
impl BundleRecord {
    fn from_input(input: &RequestSupportBundleInput<'_>, vector_len: u32) -> Self {
        Self {
            initial: input
                .initial
                .values()
                .map(|requirement| InitialRequirementRecord {
                    obligation: requirement.obligation,
                    credit: requirement.credit,
                    claim: requirement.claim,
                    operation: requirement.operation,
                    pool: requirement.pool,
                    predecessor: requirement.predecessor,
                    scope: requirement.scope,
                    input_bucket: requirement.input_bucket,
                    prospective_bound: requirement.prospective_bound,
                    state: Conditional,
                    state_time: MonotonicTime::from_micros(0),
                }),
            request_owner: input.request_owner,
            timing_commitment: input.timing.timing_commitment,
            request_closure: input.timing.request_closure,
            support_budget: input.timing.support_budget,
            bound_set: input.timing.bound_set,
            branches: input
                .branches
                .values()
                .map(|requirement| FutureBranchRequirementRecord {
                    operation: requirement.operation,
                    pool: requirement.pool,
                    input_bucket: requirement.input_bucket,
                    prospective_bound: requirement.prospective_bound,
                }),
            entitlement: input.entitlement,
            vector: input.vector,
            vector_head: NO_NODE,
            vector_len,
            linked_claims: 0,
            state: BundleState::LivePristine,
        }
    }
    fn obligations(&self) -> [SupportOperationObligationId; 3] {
        self.initial.map(|requirement| requirement.obligation)
    }
    fn credits(&self) -> [PhysicalStartCreditId; 3] {
        self.initial.map(|requirement| requirement.credit)
    }
    fn tagged_key(&self, ordinal: u8) -> Option<TaggedKey> {
        self.tagged_keys().get(usize::from(ordinal)).copied()
    }
    fn tagged_keys(&self) -> [TaggedKey; K] {
        let initial = self.initial;
        [
            TaggedKey::new(TAG_OBLIGATION, initial[0].obligation.get()),
            TaggedKey::new(TAG_OBLIGATION, initial[1].obligation.get()),
            TaggedKey::new(TAG_OBLIGATION, initial[2].obligation.get()),
            TaggedKey::new(TAG_CREDIT, initial[0].credit.get()),
            TaggedKey::new(TAG_CREDIT, initial[1].credit.get()),
            TaggedKey::new(TAG_CREDIT, initial[2].credit.get()),
            TaggedKey::new(TAG_ADMISSION_CLAIM, initial[0].claim.get()),
            TaggedKey::new(TAG_ADMISSION_CLAIM, initial[1].claim.get()),
            TaggedKey::new(TAG_ADMISSION_CLAIM, initial[2].claim.get()),
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
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(clippy::large_enum_variant)]
enum RecordSlot {
    Vacant { free_position: u32 },
    Occupied(BundleRecord),
}
#[derive(Clone, Debug, Eq, PartialEq)]
struct RequestBundleStore {
    records: Vec<RecordSlot>,
    free_records: Vec<u32>,
    occupied_records: u32,
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
        let record_bytes = std::mem::size_of::<RecordSlot>() as u64;
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
        for position in (0..record_capacity).rev() {
            records.push(RecordSlot::Vacant {
                free_position: u32::try_from(position).map_err(|_| FixedStorageError::Capacity)?,
            });
        }
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
            records,
            free_records,
            occupied_records: 0,
            identities: TaggedIdentityIndex::try_new(leaf_capacity)?,
            cells: EntitlementCellArena::try_new(cell_capacity)?,
        })
    }
    fn record_capacity(&self) -> usize {
        self.records.len()
    }
    fn record_len(&self) -> usize {
        usize::try_from(self.occupied_records).expect("u32 record count fits usize")
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
        self.occupied_records == 0
    }
    fn find(
        &self,
        tag: u8,
        identity: &[u8; 32],
        work: &mut WorkMeter,
    ) -> Result<Option<u32>, FixedStorageError> {
        self.identities.find(&self.records, tag, identity, work)
    }
    fn route_precharged(
        &self,
        tag: u8,
        identity: &[u8; 32],
    ) -> Result<(u32, Option<u32>), FixedStorageError> {
        self.identities
            .route_precharged(&self.records, tag, identity)
    }
    fn find_precharged(
        &self,
        tag: u8,
        identity: &[u8; 32],
    ) -> Result<Option<u32>, FixedStorageError> {
        self.identities
            .find_precharged(&self.records, tag, identity)
    }
    fn get_record(&self, index: u32) -> Option<&BundleRecord> {
        match self.records.get(index as usize)? {
            RecordSlot::Occupied(record) => Some(record),
            RecordSlot::Vacant { .. } => None,
        }
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
                .find(&self.records, key.tag, &key.identity, work)?
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
        let occupied_records = self
            .occupied_records
            .checked_add(1)
            .ok_or(FixedStorageError::Capacity)?;
        let record_index = self
            .free_records
            .pop()
            .expect("prevalidated record capacity");
        self.records[record_index as usize] = RecordSlot::Occupied(*record);
        for (ordinal, key) in record.tagged_keys().into_iter().enumerate() {
            self.identities
                .insert(&self.records, key, record_index, ordinal as u8, work)
                .expect("insertion Work fully preflighted");
        }
        let (head, len) = self.cells.install(record_index, cells);
        let RecordSlot::Occupied(installed) = &mut self.records[record_index as usize] else {
            unreachable!("record was installed before its compact leaves")
        };
        installed.vector_head = head;
        installed.vector_len = len;
        self.occupied_records = occupied_records;
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
        let record_slot = *self
            .get_record(record)
            .ok_or(FixedStorageError::NonCanonical)?;
        let head = record_slot.vector_head;
        let len =
            usize::try_from(record_slot.vector_len).map_err(|_| FixedStorageError::NonCanonical)?;
        self.cells.validate_owner_chain(head, len, record, work)?;
        let bits = u64::from(IDENTITY_BITS);
        work.ensure(HotPathWorkWitness::new([
            K as u64 * (bits + 1),
            0,
            0,
            0,
            K as u64 * (bits + 1),
        ]))?;
        let occupied_records = self
            .occupied_records
            .checked_sub(1)
            .ok_or(FixedStorageError::NonCanonical)?;
        for key in record_slot.tagged_keys() {
            let removed = self
                .identities
                .remove(&self.records, key.tag, &key.identity, work)
                .expect("removal Work fully preflighted");
            debug_assert_eq!(removed, Some(record));
        }
        self.cells.release(head, len);
        let free_position =
            u32::try_from(self.free_records.len()).expect("constructor-bounded record free stack");
        self.records[record as usize] = RecordSlot::Vacant { free_position };
        self.free_records.push(record);
        self.occupied_records = occupied_records;
        Ok(())
    }
    /// Metered validation of the exact record slot the consuming commit will
    /// pop: the top free record index names a Vacant slot.
    fn validate_record_slot(&self, work: &mut WorkMeter) -> Result<(), FixedStorageError> {
        work.record(WorkDimension::InvariantChecks, 1)?;
        let index = self
            .free_records
            .last()
            .copied()
            .ok_or(FixedStorageError::Capacity)?;
        work.record(WorkDimension::VisitedEntities, 1)?;
        work.record(WorkDimension::InvariantChecks, 1)?;
        check!(
            work,
            matches!(
                self.records.get(index as usize),
                Some(RecordSlot::Vacant { free_position })
                    if *free_position == (self.free_records.len() - 1) as u32
            ),
            FixedStorageError::NonCanonical
        )
    }
    /// Metered validation that the exact `len`-length owner cell chain of one
    /// record is in range, occupied by that owner, and acyclic, the same
    /// before-image the consuming withdrawal commit will release.
    fn validate_owner_chain(
        &self,
        head: u32,
        len: usize,
        owner: u32,
        work: &mut WorkMeter,
    ) -> Result<(), FixedStorageError> {
        self.cells.validate_owner_chain(head, len, owner, work)
    }
    /// Metered validation that the current top `count` free cell indices name
    /// Vacant slots, the exact destinations the consuming commit will pop.
    fn validate_cell_selection(
        &self,
        count: usize,
        work: &mut WorkMeter,
    ) -> Result<(), FixedStorageError> {
        self.cells.validate_selection(count, work)
    }
    fn validate_index_selection(
        &self,
        branches: usize,
        work: &mut WorkMeter,
    ) -> Result<(), FixedStorageError> {
        self.identities.validate_selection(K, branches, work)
    }
    fn validate_bundle_selection_precharged(
        &self,
        cells: usize,
        branches: usize,
    ) -> Result<(), FixedStorageError> {
        let position = self
            .free_records
            .len()
            .checked_sub(1)
            .ok_or(FixedStorageError::Capacity)?;
        let record = self.free_records[position];
        if !matches!(
            self.records.get(record as usize),
            Some(RecordSlot::Vacant { free_position }) if *free_position == position as u32
        ) {
            return Err(FixedStorageError::NonCanonical);
        }
        self.cells.validate_selection_precharged(cells)?;
        self.identities.validate_selection_precharged(K, branches)
    }
    /// Infallible consuming installation under the validated selection: pops
    /// one record slot, `K` leaf slots, the required branches, and exactly `v`
    /// cell slots, installs every tagged identity, and links the validated
    /// cells as one owned chain. No Work calls, no fallible branch, no
    /// allocation; internal `expect` is a fail-stop defense for impossible
    /// owner corruption after validate proved capacity, absence, and
    /// local-slot selection.
    fn commit_bundle(&mut self, record: &BundleRecord, cells: &[OutstandingCreditCell]) {
        let occupied_records = self
            .occupied_records
            .checked_add(1)
            .expect("validated occupied record count");
        let record_index = self.free_records.pop().expect("validated record capacity");
        self.records[record_index as usize] = RecordSlot::Occupied(*record);
        for (ordinal, key) in record.tagged_keys().into_iter().enumerate() {
            self.identities
                .install(&self.records, key, record_index, ordinal as u8);
        }
        let (head, len) = self.cells.install(record_index, cells);
        let RecordSlot::Occupied(installed) = &mut self.records[record_index as usize] else {
            unreachable!("record was installed before its compact leaves")
        };
        installed.vector_head = head;
        installed.vector_len = len;
        self.occupied_records = occupied_records;
    }
    /// Infallible consuming pristine withdrawal under the validated selection:
    /// removes each of the `K` identity leaves by splicing its sibling over
    /// its parent, releases the validated `v` cells, vacates the record, and
    /// returns every slot to its matching free stack. No Work calls, no
    /// fallible branch, no allocation; internal `expect` is a fail-stop
    /// defense for impossible owner corruption after validate proved the
    /// exact record and chain.
    fn withdraw_bundle_unmetered(&mut self, record: u32) {
        let occupied_records = self
            .occupied_records
            .checked_sub(1)
            .expect("validated occupied record count");
        let record_slot = *self.get_record(record).expect("validated occupied record");
        let head = record_slot.vector_head;
        let len = usize::try_from(record_slot.vector_len).expect("validated vector length");
        for key in record_slot.tagged_keys() {
            self.identities.remove_unmetered(key.tag, &key.identity);
        }
        self.cells.release(head, len);
        let free_position =
            u32::try_from(self.free_records.len()).expect("constructor-bounded record free stack");
        self.records[record as usize] = RecordSlot::Vacant { free_position };
        self.free_records.push(record);
        self.occupied_records = occupied_records;
    }
    fn retain_bundle_unmetered(&mut self, record: u32) {
        let RecordSlot::Occupied(record) = &mut self.records[record as usize] else {
            unreachable!("validated occupied tombstone record")
        };
        debug_assert_ne!(record.state, BundleState::RetainedTombstone);
        record.state = BundleState::RetainedTombstone;
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
            .ok_or(FixedStorageError::NonCanonical)?;
        let RecordSlot::Occupied(record) = slot else {
            return Err(FixedStorageError::NonCanonical);
        };
        if record.state == BundleState::RetainedTombstone {
            return Err(FixedStorageError::NonCanonical);
        }
        record.state = BundleState::RetainedTombstone;
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
    type Ledger = SupportChargeLedger<64, 64, 1>;
    type Result = std::result::Result<SupportLedgerGeneration, SupportLedgerError>;
    type Claim = SupportFundingClaim;
    fn new_ledger() -> Ledger {
        let generation = SupportLedgerGeneration::new(1).unwrap();
        let capacities = [[1, 2, 1], [0, 1, 0], [1, 2, 1], [1, 4, 1], [1, 4, 1]];
        let starts = [[FixedStartCountBound(Duration::from_micros(10), 1); 1]; 21];
        let maxima = LifecycleReserveMaxima([1, 2, 2, 1, 1]);
        Ledger::try_new(generation, capacities, 2, starts, maxima, 4, 8, 6).unwrap()
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
        put(ledger, n, credit, Ordinary, &[Reserved([n; 32])])
    }
    /// Generic-reserve fixture with the Ordinary pool carrying the same
    /// capacities the generic tests previously used on the Mandatory pool:
    /// the claim-to-pool pairing is now the sole generic authority.
    fn generic_ledger() -> Ledger {
        let mut ledger = new_ledger();
        ledger.capacities = [[2, 1, 1], [1, 0, 1], [2, 1, 1], [4, 1, 1], [4, 1, 1]];
        ledger
    }
    fn go(ledger: &mut Ledger, n: u8, transition: SupportTransition) -> Result {
        let id = SupportOperationObligationId::new([n; 32]).unwrap();
        ledger.transition(ledger.generation(), id, transition, &mut work())
    }
    #[test]
    fn c16_semantic_identity_and_record_layout_contract() {
        macro_rules! identity_contract {
            ($kind:ident) => {{
                assert_eq!($kind::new([0; 32]), Err(crate::DomainValueError::Zero));
                assert_eq!($kind::new([1; 32]).unwrap().get(), [1; 32]);
            }};
        }
        identity_contract!(AdmissionInitialClaimId);
        identity_contract!(TimingCommitmentId);
        identity_contract!(RequestClosureId);
        identity_contract!(OwnerThreadSupportBudgetId);
        assert_eq!(
            SupportInputBucket::new(0),
            Err(crate::DomainValueError::Zero)
        );
        assert_eq!(SupportInputBucket::new(1).unwrap().get(), 1);
        assert_eq!(std::mem::size_of::<InitialRequirementRecord>(), 208);
        assert_eq!(std::mem::size_of::<FutureBranchRequirementRecord>(), 32);
        assert_eq!(std::mem::size_of::<BundleState>(), 1);
        assert_eq!(
            support_storage_bytes(3, 1_025, 1_025, 21 * 1_025, 4, 8),
            Ok(488_736)
        );
        assert_eq!(
            support_storage_bytes(3, 7_211, 1_025, 21 * 1_025, 4, 8),
            Ok(2_097_096)
        );
        assert_eq!(
            support_storage_bytes(3, 7_212, 1_025, 21 * 1_025, 4, 8),
            Ok(2_097_356)
        );
        assert_eq!(
            bundle_reserve_work::<8>(168, 19),
            Ok(HotPathWorkWitness::new([4_727, 4_016, 0, 0, 4_051]))
        );
        assert_eq!(
            bundle_validate_commit_work::<8>(168, 11, 19),
            Ok(HotPathWorkWitness::new([9_865, 14_224, 0, 0, 4_279]))
        );
        assert_eq!(
            bundle_reserve_work::<{ usize::MAX }>(usize::MAX, u8::MAX),
            Err(SupportLedgerError::InvalidInput)
        );
    }
    #[test]
    fn support_ledger_contract() {
        let fail = |result: Result, error| assert_eq!(result, Err(error));
        let at = MonotonicTime::from_micros;
        let end = |n: u8, value| PredecessorEnded(SupportCausalPredecessorId([n; 32]), at(value));
        let mut ledger = generic_ledger();
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
        let mut ledger = generic_ledger();
        let before = ledger.generation();
        // Duplicate and reversed OrdinaryReservation claims reject with exact
        // per-claim Work; the C16-only AdmissionInitial claim rejects at its
        // first validity check.
        for claims in [
            [Reserved([1; 32]); 2],
            [Reserved([2; 32]), Reserved([1; 32])],
        ] {
            let mut measured = work();
            let result = ledger.reserve(before, spec(7, 7, Ordinary, &claims), &mut measured);
            fail(result, InvalidInput);
            assert_eq!(measured.witness().value(WorkDimension::VisitedEntities), 2);
            assert_eq!(measured.witness().value(WorkDimension::InvariantChecks), 8);
        }
        let mut measured = work();
        let result = ledger.reserve(
            before,
            spec(7, 7, Mandatory, &[Initial([7; 32])]),
            &mut measured,
        );
        fail(result, InvalidInput);
        assert_eq!(measured.witness().value(WorkDimension::VisitedEntities), 1);
        assert_eq!(measured.witness().value(WorkDimension::InvariantChecks), 7);
        assert_eq!((ledger.generation(), ledger.records.len()), (before, 0));
        let mut measured = work();
        let valid = spec(7, 7, Ordinary, &[Reserved([7; 32])]);
        ledger.reserve(before, valid, &mut measured).unwrap();
        assert_eq!(
            measured.witness(),
            HotPathWorkWitness::new([75, 366, 0, 0, 20])
        );
        let mut ledger = generic_ledger();
        let before = ledger.generation();
        let mut exhausted = work();
        exhausted
            .record(WorkDimension::VisitedEntities, 1_704_575)
            .unwrap();
        let result = ledger.reserve(
            before,
            spec(7, 7, Ordinary, &[Reserved([7; 32])]),
            &mut exhausted,
        );
        let error =
            WorkBudgetError::BudgetExceeded(WorkDimension::VisitedEntities, 1_704_575, 1_704_576);
        fail(result, error.into());
        assert_eq!((ledger.generation(), ledger.records.len()), (before, 0));
        // Generic reserve rejects the C16-only AdmissionInitial claim even on
        // the Ordinary pool.
        fail(
            put(&mut ledger, 1, 1, Ordinary, &[Initial([1; 32])]),
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
            HotPathWorkWitness::new([74, 308, 0, 0, 22])
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
            HotPathWorkWitness::new([1, 177, 0, 0, 4])
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
            // The Ordinary pool carries the generic capacities so the generic
            // domain coexists with the Mandatory lifecycle reservations.
            ledger.capacities = [[2, 3, 2], [1, 2, 1], [2, 2, 1], [4, 4, 2], [4, 4, 2]];
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
        // The Ordinary generic domain is independent of the Mandatory
        // lifecycle reservations: the generic record reserves and advances to
        // Pending without contending for lifecycle capacity.
        add(&mut ledger, 3, 3).unwrap();
        let end = PredecessorEnded(SupportCausalPredecessorId([3; 32]), at(2));
        go(&mut ledger, 3, end).unwrap();
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

        let resolved_work = HotPathWorkWitness::new([1, 144, 0, 0, 5]);
        let rejected_work = HotPathWorkWitness::new([1, 144, 0, 0, 4]);
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
    fn lifecycle_batch_constructor_accepts_the_immutable_boundary() {
        type Wide = SupportChargeLedger<65_536, 65_536, 3>;
        let generation = SupportLedgerGeneration::new(1).unwrap();
        let capacities = [[1, 2, 1], [0, 1, 0], [1, 2, 1], [1, 4, 1], [1, 4, 1]];
        let starts = [[
            FixedStartCountBound(Duration::from_micros(10), 1),
            FixedStartCountBound(Duration::from_micros(20), 1),
            FixedStartCountBound(Duration::from_micros(30), 1),
        ]; 21];
        let build = |maxima| Wide::try_new(generation, capacities, 2, starts, maxima, 4, 8, 6);
        build(LifecycleReserveMaxima([1, 1_024, 1_024, 1, 1])).unwrap();
        build(LifecycleReserveMaxima([1, 1_792, 1_792, 1, 1])).unwrap();
        assert_eq!(
            build(LifecycleReserveMaxima([1, 1_793, 1_793, 1, 1])).unwrap_err(),
            InvalidInput
        );
        assert_eq!(
            build(LifecycleReserveMaxima([u16::MAX; 5])).unwrap_err(),
            InvalidInput
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
    #[rustfmt::skip]
    fn bundle_input<'a>(n: u8, cells: &'a [OutstandingCreditCell]) -> RequestSupportBundleInput<'a> {
        let identity = |offset: u8| { let mut id = [0u8; 32]; id[0] = n; id[1] = offset; id };
        let requirement = |offset: u8, operation| InitialSupportRequirement {
            obligation: SupportOperationObligationId::new(identity(offset)).unwrap(), credit: PhysicalStartCreditId::new(identity(offset + 10)).unwrap(), claim: AdmissionInitialClaimId::new(identity(offset + 20)).unwrap(), operation, pool: Mandatory,
            predecessor: SupportCausalPredecessorId(identity(offset + 50)), scope: SupportCallScopeId(identity(offset + 60)), input_bucket: SupportInputBucket::new(u16::from(offset)).unwrap(), prospective_bound: Duration::from_micros(u64::from(offset)),
        };
        let branch = |offset: u8, operation| FutureSupportBranchRequirement { operation, pool: Mandatory, input_bucket: SupportInputBucket::new(u16::from(offset)).unwrap(), prospective_bound: Duration::from_micros(u64::from(offset)) };
        RequestSupportBundleInput {
            request_owner: RequestId::new(crate::DaemonInstanceId::new(u128::from(n)).unwrap(), crate::ConnectionId::new(1).unwrap(), crate::RequestSequence::new(1).unwrap()),
            timing: SupportTimingFacts { timing_commitment: TimingCommitmentId::new(identity(70)).unwrap(), request_closure: RequestClosureId::new(identity(71)).unwrap(), support_budget: OwnerThreadSupportBudgetId::new(identity(72)).unwrap(), bound_set: RuntimeOverheadBoundSetId::new(identity(73)).unwrap() },
            initial: InitialSupportRequirements { materialize: requirement(1, SupportOperation::MaterializeRequest), form_candidates: requirement(2, SupportOperation::FormCandidates), release: requirement(3, SupportOperation::ReleaseRequest) },
            branches: FutureSupportBranchRequirements { receipt_observation: branch(1, SupportOperation::ObserveTurnReceipt), continuation_formation: branch(2, SupportOperation::FormCandidates), rejection_or_local_stale_formation: branch(3, SupportOperation::FormCandidates), terminal_membership_change_formation: branch(4, SupportOperation::FormCandidates) },
            entitlement: FutureTurnSupportEntitlementId::new(identity(31)).unwrap(), vector: SupportOutstandingCreditVectorId::new(identity(41)).unwrap(), cells,
        }
    }
    fn configured_cells(count: usize, outstanding: u64) -> Vec<OutstandingCreditCell> {
        let operations = [
            SupportOperation::DescribeModel,
            SupportOperation::DescribeRequest,
            SupportOperation::MaterializeRequest,
            SupportOperation::ReleaseRequest,
            SupportOperation::FormCandidates,
            SupportOperation::ObserveTurnReceipt,
            SupportOperation::SampleBackendResources,
        ];
        operations
            .into_iter()
            .take(count)
            .map(|operation| oc(operation, Ordinary, 10, outstanding))
            .collect()
    }
    fn bundle_entitlement(n: u8) -> FutureTurnSupportEntitlementId {
        let mut identity = [0; 32];
        identity[0] = n;
        identity[1] = 31;
        FutureTurnSupportEntitlementId::new(identity).unwrap()
    }
    fn bundle_ledger(records: usize, cells: usize) -> Ledger {
        let generation = SupportLedgerGeneration::new(1).unwrap();
        let capacities = [[6; POOLS]; 5];
        let starts = [[FixedStartCountBound(Duration::from_micros(10), 1); 1]; 21];
        let maxima = LifecycleReserveMaxima([1, 2, 2, 1, 1]);
        Ledger::try_new(
            generation,
            capacities,
            2,
            starts,
            maxima,
            records,
            cells,
            cells.clamp(1, 6),
        )
        .unwrap()
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
        let ledger = bundle_ledger(4, 8);
        let before = ledger.generation();
        let cells = configured_cells(3, 1);
        let input = bundle_input(1, &cells);
        let mut measured = work();
        let change = ledger.prepare_bundle(&input, &mut measured).unwrap();
        // Empty C16 and legacy indexes cost nothing: only the fixed preflight
        // (next 2, identity validation 18, extra facts 3, absence comparisons
        // 17, free capacity 4).
        assert_eq!(
            change.work.witness(),
            HotPathWorkWitness::new([2_984, 1_664, 0, 0, 3_061])
        );
        assert_eq!(ledger.generation(), before, "prepare is read-only");
        assert_eq!(change.nonce, ledger.instance_nonce);
        assert_eq!(change.snapshot.generation, before);
        assert_eq!(change.snapshot.capacities, ledger.capacities);
        assert_eq!(change.snapshot.usage, ledger.usage);
        assert_eq!(change.snapshot.reserved, ledger.reserved);
        assert_eq!(
            (
                change.snapshot.free_records,
                change.snapshot.free_cells,
                change.snapshot.free_leaves,
                change.snapshot.free_branches
            ),
            (4, 8, 44, 43)
        );
        assert!(change.identities.iter().all(|&found| found == NO_NODE));
        assert_eq!(change.record.vector_len, 3);
        assert_eq!(change.record.vector_head, NO_NODE);
        assert_eq!(change.record.state, BundleState::LivePristine);
        assert_eq!(
            change.record.obligations(),
            input.initial.values().map(|item| item.obligation)
        );
        assert_eq!(
            change.record.credits(),
            input.initial.values().map(|item| item.credit)
        );
        assert_eq!(change.record.request_owner, input.request_owner);
        assert_eq!(
            change.record.timing_commitment,
            input.timing.timing_commitment
        );
        assert_eq!(change.record.request_closure, input.timing.request_closure);
        assert_eq!(change.record.support_budget, input.timing.support_budget);
        assert_eq!(change.record.bound_set, input.timing.bound_set);
        assert_eq!(change.record.linked_claims, 0);
        assert_eq!(std::mem::size_of::<PreparedTombstone<'static, 1>>(), 1_584);
        assert_eq!(
            std::mem::size_of::<ValidatedTombstone<'static, 'static, 64, 64, 1>>(),
            1_600
        );
        assert_eq!(std::mem::size_of::<BundleRecord>(), 1_008);
        assert_eq!(
            std::mem::size_of::<BundleChange<'static, 'static, 1>>(),
            1_664
        );
        assert_eq!(
            std::mem::size_of::<BundleChange<'static, 'static, 8>>(),
            4_016
        );
        assert_eq!(
            std::mem::size_of::<ValidatedBundleChange<'static, 'static, 'static, 12, 12, 1>>(),
            1_680
        );
        // The same input prepares again against the same before-image.
        let mut measured = work();
        ledger.prepare_bundle(&input, &mut measured).unwrap();
        assert_eq!(
            measured.witness(),
            HotPathWorkWitness::new([2_984, 1_664, 0, 0, 3_061])
        );
    }
    #[test]
    fn c16_bundle_prepare_rejects_invalid_inputs() {
        let cells = configured_cells(3, 1);
        let mut invalid = bundle_input(1, &cells);
        invalid.initial.materialize.predecessor = SupportCausalPredecessorId([0; 32]);
        let ledger = bundle_ledger(4, 8);
        assert!(matches!(
            ledger.prepare_bundle(&invalid, &mut work()),
            Err(InvalidInput)
        ));
        invalid = bundle_input(1, &cells);
        std::mem::swap(
            &mut invalid.initial.materialize,
            &mut invalid.initial.form_candidates,
        );
        assert!(matches!(
            ledger.prepare_bundle(&invalid, &mut work()),
            Err(InvalidInput)
        ));
        invalid = bundle_input(1, &cells);
        invalid.initial.materialize.credit = invalid.initial.form_candidates.credit;
        assert!(matches!(
            ledger.prepare_bundle(&invalid, &mut work()),
            Err(InvalidInput)
        ));
        invalid = bundle_input(1, &cells);
        invalid.initial.release.claim = invalid.initial.form_candidates.claim;
        assert!(matches!(
            ledger.prepare_bundle(&invalid, &mut work()),
            Err(InvalidInput)
        ));
    }
    #[test]
    fn c16_bundle_prepare_rejects_collisions_and_capacity() {
        let cells = configured_cells(3, 1);
        let reject = |ledger: &mut Ledger, input: &RequestSupportBundleInput<'_>| {
            let before = bundle_snapshot(ledger);
            let mut measured = work();
            let result = ledger.prepare_bundle(input, &mut measured);
            assert_eq!(
                bundle_snapshot(ledger),
                before,
                "prepare preserves exact state"
            );
            result.unwrap_err()
        };
        // A live legacy obligation blocks a matching C16 obligation.
        let mut ledger = bundle_ledger(4, 8);
        add(&mut ledger, 1, 1).unwrap();
        assert_eq!(ledger.records.get(0).unwrap().1, Ordinary);
        let mut legacy = bundle_input(1, &cells);
        legacy.initial.materialize.obligation = SupportOperationObligationId::new([1; 32]).unwrap();
        assert_eq!(
            reject(&mut ledger, &legacy),
            SupportLedgerError::Storage(Duplicate)
        );
        // A live C16 bundle blocks every shared identity namespace.
        let mut ledger = bundle_ledger(4, 8);
        ledger
            .bundles
            .reserve_bundle(&bundle_record(1), &configured_cells(2, 1), &mut work())
            .unwrap();
        let input = bundle_input(1, &cells);
        assert_eq!(
            reject(&mut ledger, &input),
            SupportLedgerError::Storage(Duplicate)
        );
        // A retained tombstone keeps blocking until pristine withdrawal.
        let mut ledger = bundle_ledger(4, 8);
        ledger
            .bundles
            .reserve_bundle(&bundle_record(1), &configured_cells(2, 1), &mut work())
            .unwrap();
        ledger.bundles.retain_bundle(0, &mut work()).unwrap();
        let input = bundle_input(1, &cells);
        assert_eq!(
            reject(&mut ledger, &input),
            SupportLedgerError::Storage(Duplicate)
        );
        // Record, leaf, and branch exhaustion after E = 1 is occupied.
        let mut ledger = bundle_ledger(1, 8);
        ledger
            .bundles
            .reserve_bundle(&bundle_record(9), &configured_cells(1, 1), &mut work())
            .unwrap();
        let input = bundle_input(2, &cells);
        assert_eq!(
            reject(&mut ledger, &input),
            SupportLedgerError::Storage(FixedStorageError::Capacity)
        );
        // Cell exhaustion when the vector needs more cells than remain free.
        let mut ledger = bundle_ledger(4, 4);
        ledger
            .bundles
            .reserve_bundle(&bundle_record(9), &configured_cells(1, 1), &mut work())
            .unwrap();
        let wide = configured_cells(4, 1);
        let input = bundle_input(3, &wide);
        assert_eq!(
            reject(&mut ledger, &input),
            SupportLedgerError::Storage(FixedStorageError::Capacity)
        );
    }
    #[test]
    fn c16_bundle_prepare_work_exhaustion_rolls_back() {
        let cells = configured_cells(3, 1);
        let input = bundle_input(1, &cells);
        let ledger = bundle_ledger(4, 8);
        let before = bundle_snapshot(&ledger);
        let mut exhausted = work();
        exhausted
            .record(WorkDimension::VisitedEntities, 1_704_575)
            .unwrap();
        let fault = ledger.prepare_bundle(&input, &mut exhausted);
        let error = WorkBudgetError::BudgetExceeded(
            WorkDimension::VisitedEntities,
            1_704_575,
            1_704_575 + 2_984,
        );
        assert_eq!(
            fault.err(),
            Some(SupportLedgerError::Storage(FixedStorageError::Work(error)))
        );
        assert_eq!(bundle_snapshot(&ledger), before);
        let ledger = bundle_ledger(4, 8);
        let mut exhausted = work();
        exhausted
            .record(WorkDimension::CopiedBytes, 2_095_489)
            .unwrap();
        let fault = ledger.prepare_bundle(&input, &mut exhausted);
        let error =
            WorkBudgetError::BudgetExceeded(WorkDimension::CopiedBytes, 2_097_152, 2_097_153);
        assert_eq!(
            fault.err(),
            Some(SupportLedgerError::Storage(FixedStorageError::Work(error)))
        );
        assert_eq!(
            exhausted.witness().value(WorkDimension::CopiedBytes),
            2_095_489
        );
        assert_eq!(bundle_snapshot(&ledger), before);
        let ledger = bundle_ledger(4, 8);
        let mut exhausted = work();
        exhausted
            .record(WorkDimension::InvariantChecks, 25_648)
            .unwrap();
        let fault = ledger.prepare_bundle(&input, &mut exhausted);
        let error = WorkBudgetError::BudgetExceeded(WorkDimension::InvariantChecks, 28_708, 28_709);
        assert_eq!(
            fault.err(),
            Some(SupportLedgerError::Storage(FixedStorageError::Work(error)))
        );
        assert_eq!(
            exhausted.witness().value(WorkDimension::InvariantChecks),
            25_648
        );
        assert_eq!(bundle_snapshot(&ledger), before);
    }
    #[test]
    fn c16_bundle_validate_phase_charge_is_atomic_one_under() {
        let cells = configured_cells(3, 1);
        let input = bundle_input(1, &cells);
        let prepare = bundle_reserve_work::<1>(3, 7).unwrap();
        let validate = bundle_validate_commit_work::<1>(3, 10, 7).unwrap();
        for (dimension, maximum) in [
            (WorkDimension::VisitedEntities, 1_704_575),
            (WorkDimension::CopiedBytes, 2_097_152),
            (WorkDimension::InvariantChecks, 28_708),
        ] {
            let mut ledger = bundle_ledger(4, 8);
            let initial = maximum - prepare.value(dimension) - validate.value(dimension) + 1;
            let mut meter = work();
            meter.record(dimension, initial).unwrap();
            let change = ledger.prepare_bundle(&input, &mut meter).unwrap();
            let after_prepare = change.work.witness();
            let error = ledger.validate_bundle(change).unwrap_err();
            assert_eq!(
                error,
                SupportLedgerError::Storage(FixedStorageError::Work(
                    WorkBudgetError::BudgetExceeded(dimension, maximum, maximum + 1)
                ))
            );
            assert_eq!(
                meter.witness(),
                after_prepare,
                "phase charge is all-or-none"
            );
            assert!(ledger.bundles.is_empty());
        }
    }

    #[test]
    fn c16_bundle_transaction_commits_exactly_once() {
        let mut ledger = bundle_ledger(4, 8);
        let cells = configured_cells(3, 1);
        let input = bundle_input(1, &cells);
        let before = ledger.generation();
        let mut measured = work();
        let change = ledger.prepare_bundle(&input, &mut measured).unwrap();
        assert_eq!(
            change.work.witness(),
            HotPathWorkWitness::new([2_984, 1_664, 0, 0, 3_061])
        );
        let validated = ledger
            .validate_bundle(change)
            .expect("same-instance same-state validation");
        assert_eq!(
            validated.change.work.witness(),
            HotPathWorkWitness::new([11_987, 4_936, 0, 0, 6_218])
        );
        let next = validated.commit_bundle();
        assert_eq!(
            measured.witness(),
            HotPathWorkWitness::new([11_987, 4_936, 0, 0, 6_218]),
            "commit performs no Work call"
        );
        assert_eq!(next, before.next().unwrap());
        assert_eq!(ledger.generation(), next);
        assert_eq!(
            ledger.usage,
            [[0, 3, 0], [0, 0, 0], [0, 0, 0], [0, 3, 0], [0, 3, 0]]
        );
        assert_eq!(
            ledger.reserved,
            [[3, 0, 0], [3, 3, 0], [3, 3, 0], [3, 0, 0], [3, 0, 0]]
        );
        let mut expected_vector = [[0; 1]; 21];
        for axis in [0, 3, 6] {
            expected_vector[axis][0] = 1;
        }
        assert_eq!(ledger.vector_usage, expected_vector);
        // Every K identity now resolves to the committed record.
        let owner = ledger
            .bundles
            .find(
                TAG_OBLIGATION,
                &[
                    1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                    0, 0, 0, 0, 0, 0,
                ],
                &mut work(),
            )
            .unwrap();
        assert!(owner.is_some());
        let record = ledger
            .bundles
            .get_record(owner.expect("committed bundle"))
            .expect("occupied bundle record");
        assert_eq!(record.state, BundleState::LivePristine);
        assert_eq!(record.vector_len, 3);
        assert_ne!(record.vector_head, NO_NODE);
        assert_eq!(
            (
                ledger.bundles.free_record_len(),
                ledger.bundles.free_cell_len()
            ),
            (3, 5)
        );
        assert_eq!(
            (
                ledger.bundles.free_leaf_len(),
                ledger.bundles.free_branch_len()
            ),
            (33, 33)
        );
        bundle_store_oracle(&ledger.bundles);
        // The committed bundle now blocks a second prepare of the same facts.
        let mut measured = work();
        let second = ledger.prepare_bundle(&input, &mut measured);
        assert_eq!(
            second.unwrap_err(),
            SupportLedgerError::Storage(FixedStorageError::Duplicate)
        );
        // And blocks a later legacy generic reserve on the same obligation.
        let mut legacy = spec(1, 9, Ordinary, &[Reserved([9; 32])]);
        legacy.id = SupportOperationObligationId::new([
            1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0,
        ])
        .unwrap();
        assert_eq!(
            ledger.reserve(ledger.generation(), legacy, &mut work()),
            Err(SupportLedgerError::Storage(FixedStorageError::Duplicate))
        );
    }
    #[test]
    fn c16_bundle_logical_backing_uses_horizon_max_not_sum() {
        type H2Ledger = SupportChargeLedger<64, 64, 2>;
        let bounds = [
            FixedStartCountBound(Duration::from_micros(10), 10),
            FixedStartCountBound(Duration::from_micros(20), 10),
        ];
        let mut ledger = H2Ledger::try_new(
            SupportLedgerGeneration::new(1).unwrap(),
            [[6; POOLS]; 5],
            2,
            [bounds; 21],
            LifecycleReserveMaxima([1, 2, 2, 1, 1]),
            4,
            8,
            6,
        )
        .unwrap();
        let cells = [
            oc(SupportOperation::DescribeModel, Ordinary, 10, 2),
            oc(SupportOperation::DescribeModel, Ordinary, 20, 3),
        ];
        let input = bundle_input(1, &cells);
        let mut meter = work();
        let change = ledger.prepare_bundle(&input, &mut meter).unwrap();
        ledger.validate_bundle(change).unwrap().commit_bundle();
        assert_eq!(
            ledger.reserved,
            [[3, 0, 0], [3, 3, 0], [3, 3, 0], [3, 0, 0], [3, 0, 0]],
            "future backing is max(2,3), never 2+3"
        );
        assert_eq!(ledger.vector_usage[0], [2, 3]);
    }

    #[test]
    fn c16_bundle_transaction_drift_drops_and_moves() {
        let cells = configured_cells(3, 1);
        // Dropping the validated capability changes no state.
        let mut ledger = bundle_ledger(4, 8);
        let before = bundle_snapshot(&ledger);
        let input = bundle_input(1, &cells);
        let mut meter = work();
        let change = ledger.prepare_bundle(&input, &mut meter).unwrap();
        // The exclusive validated capability drops at the statement end
        // without committing; the ledger and store remain unchanged.
        ledger.validate_bundle(change).unwrap();
        assert_eq!(bundle_snapshot(&ledger), before);
        // An intervening legal mutation makes the prepared change stale.
        let mut ledger = bundle_ledger(4, 8);
        let input = bundle_input(1, &cells);
        let mut meter = work();
        let change = ledger.prepare_bundle(&input, &mut meter).unwrap();
        add(&mut ledger, 9, 9).unwrap();
        assert_eq!(
            ledger.validate_bundle(change).unwrap_err(),
            SupportLedgerError::Generation
        );
        assert_eq!(
            bundle_snapshot(&ledger),
            (SupportLedgerGeneration::new(2).unwrap(), 4, 8, 44, 43)
        );
        // Moving the ledger preserves the instance nonce: validation succeeds.
        let ledger = bundle_ledger(4, 8);
        let input = bundle_input(1, &cells);
        let mut meter = work();
        let change = ledger.prepare_bundle(&input, &mut meter).unwrap();
        let mut moved = ledger;
        let validated = moved.validate_bundle(change).unwrap();
        validated.commit_bundle();
        assert_eq!(moved.bundles.free_record_len(), 3);
        // A same-state different-instance ledger rejects the change.
        let first = bundle_ledger(4, 8);
        let input = bundle_input(1, &cells);
        let mut meter = work();
        let change = first.prepare_bundle(&input, &mut meter).unwrap();
        let mut second = bundle_ledger(4, 8);
        assert_eq!(
            second.validate_bundle(change).unwrap_err(),
            SupportLedgerError::Generation
        );
    }
    #[test]
    fn c16_bundle_constructor_rejects_vector_above_axis_domain() {
        let generation = SupportLedgerGeneration::new(1).unwrap();
        let capacities = [[1, 2, 1], [0, 1, 0], [1, 2, 1], [1, 4, 1], [1, 4, 1]];
        let starts = [[FixedStartCountBound(Duration::from_micros(10), 1); 1]; 21];
        let maxima = LifecycleReserveMaxima([1, 2, 2, 1, 1]);
        assert_eq!(
            Ledger::try_new(generation, capacities, 2, starts, maxima, 4, 4_000, 22).unwrap_err(),
            SupportLedgerError::InvalidInput
        );
    }
    #[test]
    fn c16_bundle_snapshot_reports_capacity_facts() {
        let mut ledger = bundle_ledger(4, 8);
        let mut measured = work();
        let snapshot = ledger.snapshot(&mut measured).unwrap();
        assert_eq!(
            measured.witness(),
            HotPathWorkWitness::new([0, 552, 0, 0, 1])
        );
        assert_eq!(snapshot.generation, ledger.generation);
        assert_eq!(snapshot.capacities, ledger.capacities);
        assert_eq!(snapshot.usage, ledger.usage);
        assert_eq!(snapshot.reserved, ledger.reserved);
        assert_eq!(snapshot.bundle_vector_max, ledger.bundle_vector_max);
        assert_eq!(snapshot.vector_capacity, ledger.vector_capacity);
        assert_eq!(snapshot.vector_usage, ledger.vector_usage);
        assert_eq!(std::mem::size_of_val(&snapshot), 552);
        assert_eq!(std::mem::size_of::<SupportCapacitySnapshot<8>>(), 2_904);
        let copied_one_under =
            HotPathWorkBudget::try_new(HotPathWorkWitness::new([1_704_575, 551, 0, 2, 28_708]))
                .unwrap();
        let mut rejected = WorkMeter::new(copied_one_under);
        assert!(matches!(
            ledger.snapshot(&mut rejected),
            Err(SupportLedgerError::Storage(FixedStorageError::Work(
                WorkBudgetError::BudgetExceeded(WorkDimension::CopiedBytes, 551, 552)
            )))
        ));
        assert_eq!(rejected.witness(), HotPathWorkWitness::default());
        assert_eq!(
            (
                snapshot.occupied_records,
                snapshot.free_records,
                snapshot.free_cells,
                snapshot.free_leaves,
                snapshot.free_branches
            ),
            (0, 4, 8, 44, 43)
        );
        let cells = configured_cells(2, 1);
        let input = bundle_input(1, &cells);
        let mut meter = work();
        let change = ledger.prepare_bundle(&input, &mut meter).unwrap();
        let validated = ledger.validate_bundle(change).unwrap();
        validated.commit_bundle();
        let snapshot = ledger.snapshot(&mut work()).unwrap();
        assert_eq!(
            (
                snapshot.occupied_records,
                snapshot.free_records,
                snapshot.free_cells,
                snapshot.free_leaves,
                snapshot.free_branches
            ),
            (1, 3, 6, 33, 33)
        );
    }
    #[test]
    fn generic_reserve_rejects_c16_only_claims() {
        let mut ledger = bundle_ledger(4, 8);
        let mut reject = |claims: &[Claim]| {
            let before = (ledger.generation(), ledger.records.len());
            let mut measured = work();
            let result = ledger.reserve(
                ledger.generation(),
                spec(7, 7, Mandatory, claims),
                &mut measured,
            );
            assert_eq!(result, Err(InvalidInput));
            assert_eq!((ledger.generation(), ledger.records.len()), before);
        };
        reject(&[Initial([7; 32])]);
        reject(&[SupportFundingClaim::EntitlementVector([7; 32])]);
        // C16-only facts remain constructible only by the complete bundle path.
        let mut ledger = bundle_ledger(4, 8);
        reserve_bundle(&mut ledger, 1, 3);
        assert_eq!(ledger.bundles.free_record_len(), 3);
        assert_eq!(
            ledger.records.len(),
            0,
            "C16 bundles insert nothing into the legacy arena"
        );
    }
    /// Test-only helper: reserve one complete C16 bundle with `n`-derived
    /// identities and `v` cells, returning the first obligation handle.
    #[test]
    fn c16_reciprocal_work_witnesses() {
        // Generic reserve with one OrdinaryReservation claim: the reciprocal
        // C16 absence preflight adds exactly two InvariantChecks on an empty
        // store, and the complete witness is frozen.
        let mut ledger = generic_ledger();
        let mut measured = work();
        let valid = spec(7, 7, Ordinary, &[Reserved([7; 32])]);
        ledger
            .reserve(ledger.generation(), valid, &mut measured)
            .unwrap();
        assert_eq!(
            measured.witness(),
            HotPathWorkWitness::new([75, 366, 0, 0, 20])
        );
        // Lifecycle reserve m = 1 with the reciprocal preflight frozen.
        let mut ledger = new_ledger();
        for capacity in &mut ledger.capacities {
            capacity[1] = 4;
        }
        let lifecycle = LifecycleReserveSpec {
            id: SupportOperationObligationId::new([1; 32]).unwrap(),
            kind: LifecycleReserveKind::PostLoadModelDescription,
            physical_credit: PhysicalStartCreditId::new([2; 32]).unwrap(),
            predecessor: SupportCausalPredecessorId([90; 32]),
            scope: SupportCallScopeId([91; 32]),
            claim: SupportFundingClaim::LifecycleReserve([92; 32]),
            expires_at: None,
        };
        let mut measured = work();
        ledger
            .reserve_lifecycle(
                ledger.generation(),
                MonotonicTime::from_micros(1),
                &[lifecycle],
                &mut measured,
            )
            .unwrap();
        assert_eq!(
            measured.witness(),
            HotPathWorkWitness::new([75, 300, 0, 0, 29])
        );
    }
    fn reserve_bundle(ledger: &mut Ledger, n: u8, v: usize) -> SupportOperationObligationId {
        let cells = configured_cells(v, 1);
        let input = bundle_input(n, &cells);
        let mut meter = work();
        let change = ledger.prepare_bundle(&input, &mut meter).unwrap();
        let validated = ledger.validate_bundle(change).unwrap();
        validated.commit_bundle();
        input.initial.materialize.obligation
    }
    fn tombstone_bundle(ledger: &mut Ledger, n: u8) -> SupportLedgerGeneration {
        let mut meter = work();
        let change = ledger
            .prepare_tombstone(bundle_entitlement(n), &mut meter)
            .unwrap();
        ledger
            .validate_tombstone(change)
            .unwrap()
            .commit_tombstone()
    }
    #[test]
    fn c16_bundle_tombstone_drift_and_drop_are_read_only() {
        let mut ledger = bundle_ledger(4, 8);
        reserve_bundle(&mut ledger, 1, 3);
        let before = bundle_snapshot(&ledger);
        let mut meter = work();
        let change = ledger
            .prepare_tombstone(bundle_entitlement(1), &mut meter)
            .unwrap();
        ledger.validate_tombstone(change).unwrap();
        assert_eq!(bundle_snapshot(&ledger), before);

        let mut ledger = bundle_ledger(4, 8);
        reserve_bundle(&mut ledger, 1, 3);
        let mut meter = work();
        let change = ledger
            .prepare_tombstone(bundle_entitlement(1), &mut meter)
            .unwrap();
        add(&mut ledger, 9, 9).unwrap();
        assert_eq!(
            ledger.validate_tombstone(change).unwrap_err(),
            SupportLedgerError::Generation
        );
    }

    #[test]
    fn c16_bundle_pristine_withdraw_commits_once_and_releases() {
        let mut ledger = bundle_ledger(4, 8);
        let obligation = reserve_bundle(&mut ledger, 1, 3);
        let before = ledger.generation();
        let mut measured = work();
        let change = ledger
            .prepare_withdraw(bundle_entitlement(1), &mut measured)
            .unwrap();
        assert_eq!(
            change.work.witness(),
            HotPathWorkWitness::new([7, 0, 0, 0, 20])
        );
        let validated = ledger
            .validate_withdraw(change)
            .expect("same-instance same-state validation");
        assert_eq!(
            validated.change.work.witness(),
            HotPathWorkWitness::new([71, 0, 0, 0, 201])
        );
        assert_eq!(std::mem::size_of::<PreparedWithdrawal<'static, 1>>(), 1_632);
        assert_eq!(
            std::mem::size_of::<ValidatedWithdrawal<'static, 'static, 12, 12, 1>>(),
            1_648
        );
        let next = validated.commit_withdraw();
        assert_eq!(next, before.next().unwrap());
        assert_eq!(ledger.generation(), next);
        assert_eq!(ledger.usage, [[0; POOLS]; 5]);
        assert_eq!(ledger.reserved, [[0; POOLS]; 5]);
        assert_eq!(ledger.vector_usage, [[0; 1]; 21]);
        // The store is pristine again: every identity is released and the
        // exact later reuse of the same facts succeeds.
        assert!(ledger.bundles.is_empty());
        assert_eq!(
            (
                ledger.bundles.free_record_len(),
                ledger.bundles.free_cell_len(),
                ledger.bundles.free_leaf_len(),
                ledger.bundles.free_branch_len()
            ),
            (4, 8, 44, 43)
        );
        bundle_store_oracle(&ledger.bundles);
        // The released obligation is again usable by a legacy generic reserve.
        let mut legacy = spec(1, 9, Ordinary, &[Reserved([9; 32])]);
        legacy.id = obligation;
        ledger
            .reserve(ledger.generation(), legacy, &mut work())
            .unwrap();
        // Exact later reuse of the same C16 facts succeeds after pristine
        // withdrawal on a fresh ledger.
        let mut ledger = bundle_ledger(4, 8);
        let first = reserve_bundle(&mut ledger, 1, 3);
        let mut measured = work();
        let change = ledger
            .prepare_withdraw(bundle_entitlement(1), &mut measured)
            .unwrap();
        ledger.validate_withdraw(change).unwrap().commit_withdraw();
        let reused = reserve_bundle(&mut ledger, 1, 3);
        assert_eq!(ledger.bundles.free_record_len(), 3);
        assert_eq!(reused, first);
    }
    #[test]
    fn c16_bundle_withdraw_drift_and_non_pristine_reject() {
        let cells = configured_cells(3, 1);
        // An intervening legal mutation makes the prepared withdrawal stale.
        let mut ledger = bundle_ledger(4, 8);
        reserve_bundle(&mut ledger, 1, 3);
        let mut measured = work();
        let change = ledger
            .prepare_withdraw(bundle_entitlement(1), &mut measured)
            .unwrap();
        add(&mut ledger, 9, 9).unwrap();
        assert_eq!(
            ledger.validate_withdraw(change).unwrap_err(),
            SupportLedgerError::Generation
        );
        // Dropping the validated capability changes no state.
        let mut ledger = bundle_ledger(4, 8);
        reserve_bundle(&mut ledger, 1, 3);
        let before = bundle_snapshot(&ledger);
        let change = ledger
            .prepare_withdraw(bundle_entitlement(1), &mut measured)
            .unwrap();
        ledger.validate_withdraw(change).unwrap();
        assert_eq!(bundle_snapshot(&ledger), before);
        // An unknown obligation rejects during prepare.
        assert_eq!(
            ledger
                .prepare_withdraw(
                    FutureTurnSupportEntitlementId::new([200; 32]).unwrap(),
                    &mut measured,
                )
                .unwrap_err(),
            SupportLedgerError::InvalidTransition
        );
        // A retained terminal tombstone is not pristine: withdrawal rejects.
        let mut ledger = bundle_ledger(4, 8);
        let obligation = reserve_bundle(&mut ledger, 1, 3);
        let retained = (ledger.usage, ledger.reserved, ledger.vector_usage);
        tombstone_bundle(&mut ledger, 1);
        assert_eq!(
            (ledger.usage, ledger.reserved, ledger.vector_usage),
            retained,
            "tombstone retains every logical and vector delta"
        );
        assert_eq!(
            ledger
                .prepare_withdraw(bundle_entitlement(1), &mut measured)
                .unwrap_err(),
            SupportLedgerError::InvalidTransition
        );
        // Double close rejects.
        let mut tombstone_meter = work();
        assert_eq!(
            ledger
                .prepare_tombstone(bundle_entitlement(1), &mut tombstone_meter)
                .unwrap_err(),
            SupportLedgerError::InvalidTransition
        );
        // The tombstone keeps every identity and cell occupied.
        assert_eq!(ledger.bundles.free_record_len(), 3);
        assert_eq!(ledger.bundles.free_cell_len(), 5);
        assert_eq!(ledger.bundles.free_leaf_len(), 33);
        assert_eq!(ledger.bundles.free_branch_len(), 33);
        let input = bundle_input(1, &cells);
        let second = ledger.prepare_bundle(&input, &mut work()).unwrap_err();
        assert_eq!(
            second,
            SupportLedgerError::Storage(FixedStorageError::Duplicate)
        );
        // A legacy generic reserve on a tombstoned obligation still rejects.
        let mut legacy = spec(1, 9, Ordinary, &[Reserved([9; 32])]);
        legacy.id = obligation;
        assert_eq!(
            ledger.reserve(ledger.generation(), legacy, &mut work()),
            Err(SupportLedgerError::Storage(FixedStorageError::Duplicate))
        );
        // Post-removal reuse: close then pristine-withdraw is impossible, so
        // reuse is tested on a fresh Live bundle below.
        let mut ledger = bundle_ledger(4, 8);
        reserve_bundle(&mut ledger, 1, 3);
        let mut measured = work();
        let change = ledger
            .prepare_withdraw(bundle_entitlement(1), &mut measured)
            .unwrap();
        ledger.validate_withdraw(change).unwrap().commit_withdraw();
        let tombstone = reserve_bundle(&mut ledger, 2, 3);
        tombstone_bundle(&mut ledger, 2);
        assert_eq!(ledger.bundles.free_record_len(), 3);
        // The live bundle's identities were released; the tombstone's remain.
        let owner = ledger
            .bundles
            .find(TAG_OBLIGATION, &tombstone.get(), &mut work())
            .unwrap()
            .expect("tombstone identity retained");
        assert_eq!(
            ledger.bundles.get_record(owner).unwrap().state,
            BundleState::RetainedTombstone
        );
    }
    #[test]
    fn c16_reciprocal_uniqueness_across_legacy_paths() {
        let cells = configured_cells(3, 1);
        let at = MonotonicTime::from_micros;
        let duplicate = SupportLedgerError::Storage(Duplicate);
        let legacy_blocks = |ledger: &mut Ledger, obligation: [u8; 32], credit: [u8; 32]| {
            let mut input = bundle_input(2, &cells);
            input.initial.materialize.obligation =
                SupportOperationObligationId::new(obligation).unwrap();
            assert_eq!(
                ledger.prepare_bundle(&input, &mut work()).unwrap_err(),
                duplicate
            );
            let mut input = bundle_input(3, &cells);
            input.initial.materialize.credit = PhysicalStartCreditId::new(credit).unwrap();
            assert_eq!(
                ledger.prepare_bundle(&input, &mut work()).unwrap_err(),
                duplicate
            );
        };
        // Generic reserve keys block a later C16 bundle in both namespaces.
        let mut ledger = bundle_ledger(4, 8);
        add(&mut ledger, 1, 1).unwrap();
        legacy_blocks(&mut ledger, [1; 32], [1; 32]);
        // Prepared ordinary keys block a later C16 bundle.
        let mut ledger = ordinary_ledger();
        begin(&mut ledger, ordinary((1, 1, 3, Reserved([4; 32]))), at(1)).unwrap();
        legacy_blocks(&mut ledger, [1; 32], [1; 32]);
        // Lifecycle reserve keys block a later C16 bundle.
        let mut ledger = bundle_ledger(4, 8);
        for capacity in &mut ledger.capacities {
            capacity[1] = 4;
        }
        let lifecycle = LifecycleReserveSpec {
            id: SupportOperationObligationId::new([1; 32]).unwrap(),
            kind: LifecycleReserveKind::PostLoadModelDescription,
            physical_credit: PhysicalStartCreditId::new([2; 32]).unwrap(),
            predecessor: SupportCausalPredecessorId([90; 32]),
            scope: SupportCallScopeId([91; 32]),
            claim: SupportFundingClaim::LifecycleReserve([92; 32]),
            expires_at: None,
        };
        ledger
            .reserve_lifecycle(ledger.generation(), at(1), &[lifecycle], &mut work())
            .unwrap();
        legacy_blocks(&mut ledger, [1; 32], [2; 32]);
        // A live C16 bundle blocks every earlier-row legacy path in both
        // shared namespaces.
        let mut ledger = bundle_ledger(4, 8);
        let obligation = reserve_bundle(&mut ledger, 1, 3);
        let credit = PhysicalStartCreditId::new([
            1, 11, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0,
        ])
        .unwrap();
        let mut generic = spec(9, 9, Ordinary, &[Reserved([9; 32])]);
        generic.id = obligation;
        assert_eq!(
            ledger
                .reserve(ledger.generation(), generic, &mut work())
                .unwrap_err(),
            duplicate
        );
        let mut generic = spec(9, 9, Ordinary, &[Reserved([9; 32])]);
        generic.physical_credit = credit;
        assert_eq!(
            ledger
                .reserve(ledger.generation(), generic, &mut work())
                .unwrap_err(),
            duplicate
        );
        let mut ordinary_spec = ordinary((9, 21, 41, Reserved([9; 32])));
        ordinary_spec.id = obligation;
        assert_eq!(
            ledger
                .begin_ordinary(ledger.generation(), ordinary_spec, at(2), &mut work())
                .unwrap_err(),
            duplicate
        );
        let mut ordinary_spec = ordinary((9, 21, 41, Reserved([9; 32])));
        ordinary_spec.physical_credit = credit;
        assert_eq!(
            ledger
                .begin_ordinary(ledger.generation(), ordinary_spec, at(2), &mut work())
                .unwrap_err(),
            duplicate
        );
        let lifecycle = LifecycleReserveSpec {
            id: obligation,
            kind: LifecycleReserveKind::PostLoadModelDescription,
            physical_credit: PhysicalStartCreditId::new([21; 32]).unwrap(),
            predecessor: SupportCausalPredecessorId([90; 32]),
            scope: SupportCallScopeId([91; 32]),
            claim: SupportFundingClaim::LifecycleReserve([92; 32]),
            expires_at: None,
        };
        assert_eq!(
            ledger
                .reserve_lifecycle(ledger.generation(), at(2), &[lifecycle], &mut work())
                .unwrap_err(),
            duplicate
        );
        let lifecycle = LifecycleReserveSpec {
            id: SupportOperationObligationId::new([8; 32]).unwrap(),
            kind: LifecycleReserveKind::PostLoadModelDescription,
            physical_credit: credit,
            predecessor: SupportCausalPredecessorId([90; 32]),
            scope: SupportCallScopeId([91; 32]),
            claim: SupportFundingClaim::LifecycleReserve([92; 32]),
            expires_at: None,
        };
        assert_eq!(
            ledger
                .reserve_lifecycle(ledger.generation(), at(2), &[lifecycle], &mut work())
                .unwrap_err(),
            duplicate
        );
        // A retained terminal tombstone keeps blocking every earlier-row path.
        let mut ledger = bundle_ledger(4, 8);
        let obligation = reserve_bundle(&mut ledger, 1, 3);
        tombstone_bundle(&mut ledger, 1);
        let mut generic = spec(9, 9, Ordinary, &[Reserved([9; 32])]);
        generic.id = obligation;
        assert_eq!(
            ledger
                .reserve(ledger.generation(), generic, &mut work())
                .unwrap_err(),
            duplicate
        );
        let mut ordinary_spec = ordinary((9, 21, 41, Reserved([9; 32])));
        ordinary_spec.id = obligation;
        assert_eq!(
            ledger
                .begin_ordinary(ledger.generation(), ordinary_spec, at(2), &mut work())
                .unwrap_err(),
            duplicate
        );
        let lifecycle = LifecycleReserveSpec {
            id: obligation,
            kind: LifecycleReserveKind::PostLoadModelDescription,
            physical_credit: PhysicalStartCreditId::new([21; 32]).unwrap(),
            predecessor: SupportCausalPredecessorId([90; 32]),
            scope: SupportCallScopeId([91; 32]),
            claim: SupportFundingClaim::LifecycleReserve([92; 32]),
            expires_at: None,
        };
        assert_eq!(
            ledger
                .reserve_lifecycle(ledger.generation(), at(2), &[lifecycle], &mut work())
                .unwrap_err(),
            duplicate
        );
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
        for (position, &index) in arena.free.iter().enumerate() {
            assert!((index as usize) < capacity, "free index in range");
            assert!(free_set.insert(index), "free indices unique");
            assert_eq!(
                arena.slots[index as usize],
                CellSlot::Vacant {
                    free_position: position as u32,
                }
            );
        }
        let mut pointed = HashSet::new();
        let mut occupied = 0usize;
        for index in 0..capacity {
            if let CellSlot::Occupied { next_owned, .. } = arena.slots[index]
                && next_owned != NO_NODE
            {
                assert!((next_owned as usize) < capacity, "next index in range");
                assert!(
                    pointed.insert(next_owned),
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
            if pointed.contains(&(index as u32)) {
                continue;
            }
            chains += 1;
            let mut slot = index as u32;
            let mut owner = None;
            let mut chain = HashSet::new();
            while slot != NO_NODE {
                let current = slot as usize;
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
        head: u32,
        len: u32,
        owner: u32,
        cells: &[OutstandingCreditCell],
    ) -> HotPathWorkWitness {
        let mut meter = work();
        let chain = arena.validate_chain(head, len as usize, owner, cells, &mut meter);
        assert_eq!(chain, Ok(()));
        meter.witness()
    }
    fn chain_err(
        arena: &EntitlementCellArena,
        head: u32,
        len: u32,
        owner: u32,
        cells: &[OutstandingCreditCell],
    ) -> HotPathWorkWitness {
        let mut meter = work();
        let chain = arena.validate_chain(head, len as usize, owner, cells, &mut meter);
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
            (usize::MAX, FixedStorageError::Capacity),
        ] {
            assert_eq!(EntitlementCellArena::try_new(capacity).unwrap_err(), error);
        }
        // First storage-invalid boundary: slots+free bytes exceed the binary bound.
        let slot_bytes = std::mem::size_of::<CellSlot>() as u64;
        let index_bytes = std::mem::size_of::<u32>() as u64;
        let max_capacity = (2_097_152_u64 / (slot_bytes + index_bytes)) as usize;
        let boundary = EntitlementCellArena::try_new(max_capacity + 1).unwrap_err();
        assert_eq!(boundary, FixedStorageError::Capacity);
        // Deterministic fail-closed seal: an over-capacity stand-in proves
        // rejection without relying on allocator behavior. `with_capacity(16)`
        // guarantees at least 16 slots, so both backing Vecs are never exactly 8.
        let mut slots = Vec::with_capacity(16);
        slots.extend((0..8).map(|free_position| CellSlot::Vacant { free_position }));
        let mut free = Vec::with_capacity(16);
        free.extend((0u32..8).rev());
        let sealed = seal_exact_capacity(&slots, &free, 8).unwrap_err();
        assert_eq!(sealed, FixedStorageError::Capacity);
        let arena = EntitlementCellArena::try_new(8).unwrap();
        assert_eq!(std::mem::size_of::<CellSlot>(), 40);
        assert_eq!(std::mem::size_of::<EntitlementCellArena>(), 48);
        assert_eq!(arena.capacity(), 8);
        assert_eq!(arena.free_len(), 8);
        arena_oracle(&arena);
    }

    #[test]
    fn c16_cell_arena_selection_and_install() {
        let mut arena = EntitlementCellArena::try_new(8).unwrap();
        assert_eq!(select_ok(&arena, 0), witness([0, 0, 0, 0, 1]));
        assert_eq!(select_ok(&arena, 1), witness([1, 0, 0, 0, 4]));
        assert_eq!(select_err(&arena, 9), witness([0, 0, 0, 0, 1]));
        let five = axis_cells(5, 1);
        assert_eq!(select_ok(&arena, 5), witness([5, 0, 0, 0, 16]));
        let (head, len) = arena.install(7, &five);
        assert_eq!((head, len), (4, 5));
        assert_eq!(arena.free_len(), 3);
        assert_eq!(select_ok(&arena, 3), witness([3, 0, 0, 0, 10]));
        assert_eq!(select_err(&arena, 4), witness([0, 0, 0, 0, 1]));
        for (index, cell, next) in [(0, five[4], None), (4, five[0], Some(3))] {
            let expected = CellSlot::Occupied {
                owner_record: 7,
                cell,
                next_owned: next.unwrap_or(NO_NODE),
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
    fn c16_cell_arena_selected_slot_corruption_rejects_safely() {
        let arena = EntitlementCellArena::try_new(8).unwrap();
        let mut out_of_range = arena.clone();
        *out_of_range.free.last_mut().unwrap() = u32::MAX;
        assert_eq!(
            out_of_range.validate_selection(1, &mut work()),
            Err(FixedStorageError::NonCanonical)
        );
        let mut wrong_position = arena.clone();
        wrong_position.slots[0] = CellSlot::Vacant { free_position: 0 };
        assert_eq!(
            wrong_position.validate_selection(1, &mut work()),
            Err(FixedStorageError::NonCanonical)
        );
        let mut duplicate = arena;
        let last = duplicate.free.len() - 1;
        duplicate.free[last - 1] = duplicate.free[last];
        assert_eq!(
            duplicate.validate_selection(2, &mut work()),
            Err(FixedStorageError::NonCanonical)
        );
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
        arena.release(head, len as usize);
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
            let (head, len) = arena.install((cycle + 1) as u32, &cells);
            assert_eq!(len, count as u32);
            assert!(head < 8);
            arena_oracle(&arena);
            chain_ok(&arena, head, len, (cycle + 1) as u32, &cells);
            arena.release(head, len as usize);
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
        assert_eq!(select_ok(&arena, 3), witness([3, 0, 0, 0, 10]));
        one_under(WorkDimension::VisitedEntities, 3, 1_704_575, |m| {
            arena.validate_selection(3, m)
        });
        one_under(WorkDimension::InvariantChecks, 10, 28_708, |m| {
            arena.validate_selection(3, m)
        });
        let (head, len) = arena.install(7, &cells);
        let chain = chain_ok(&arena, head, len, 7, &cells);
        assert_eq!(chain, witness([3, 0, 0, 0, 16]));
        one_under(WorkDimension::VisitedEntities, 3, 1_704_575, |m| {
            arena.validate_chain(head, len as usize, 7, &cells, m)
        });
        one_under(WorkDimension::InvariantChecks, 16, 28_708, |m| {
            arena.validate_chain(head, len as usize, 7, &cells, m)
        });
    }

    #[cfg(any())]
    #[rustfmt::skip]
    mod legacy_identity_index_mutation_tests {
        use super::*;

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
            BranchSlot::Occupied(IdentityBranch {
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
            BranchSlot::Occupied(IdentityBranch {
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
        assert!(matches!(index.branch_slots[0], BranchSlot::Vacant { .. }));
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
            BranchSlot::Occupied(IdentityBranch {
                bit: 7,
                zero: 0,
                one: 2
            })
        );
        assert!(matches!(index.branch_slots[0], BranchSlot::Vacant { .. }));
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
            BranchSlot::Occupied(IdentityBranch {
                bit: 8,
                zero: 0,
                one: 1
            })
        );
        assert!(matches!(index.branch_slots[1], BranchSlot::Vacant { .. }));
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
            BranchSlot::Occupied(IdentityBranch {
                bit: 7,
                zero: 1,
                one: 2
            })
        );
        assert!(matches!(index.branch_slots[0], BranchSlot::Vacant { .. }));
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
            LeafSlot::Occupied(IdentityLeaf {
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

    }

    fn trie_oracle(index: &TaggedIdentityIndex, records: &[RecordSlot]) {
        use std::collections::HashSet;
        assert_eq!(std::mem::size_of::<LeafSlot>(), 8);
        assert_eq!(std::mem::size_of::<BranchSlot>(), 16);
        assert_eq!(std::mem::size_of::<TaggedIdentityIndex>(), 104);
        let mut free_leaves = HashSet::new();
        for (position, &leaf) in index.free_leaves.iter().enumerate() {
            assert!(free_leaves.insert(leaf));
            assert!(matches!(
                index.leaf_slots.get(leaf as usize),
                Some(LeafSlot::Vacant { free_position }) if *free_position == position as u32
            ));
        }
        let occupied_leaves = index
            .leaf_slots
            .iter()
            .filter(|slot| matches!(slot, LeafSlot::Occupied { .. }))
            .count();
        assert_eq!(free_leaves.len() + occupied_leaves, index.leaf_capacity());
        let mut seen_leaves = HashSet::new();
        let mut seen_branches = HashSet::new();
        let mut keys = HashSet::new();
        let mut stack = if index.root != NO_NODE {
            vec![(index.root, None)]
        } else {
            Vec::new()
        };
        while let Some((node, parent_bit)) = stack.pop() {
            if is_branch(node) {
                let slot = branch_index(node);
                assert!(seen_branches.insert(slot));
                let branch = index.branch(node).expect("reachable occupied branch");
                assert!(branch.bit < IDENTITY_BITS);
                assert!(parent_bit.is_none_or(|parent| parent < branch.bit));
                assert_ne!(branch.zero, branch.one);
                stack.push((branch.zero, Some(branch.bit)));
                stack.push((branch.one, Some(branch.bit)));
            } else {
                assert!(seen_leaves.insert(node));
                let (key, _) = index.resolved_leaf(node, records).unwrap();
                assert!(keys.insert(key));
            }
        }
        assert_eq!(seen_leaves.len(), occupied_leaves);
        assert_eq!(seen_branches.len(), occupied_leaves.saturating_sub(1));
    }

    #[test]
    fn c16_compact_leaf_layout_tags_and_corruption_are_closed() {
        assert_eq!(std::mem::size_of::<LeafSlot>(), 8);
        assert_eq!(
            [
                TAG_OBLIGATION,
                TAG_CREDIT,
                TAG_ADMISSION_CLAIM,
                TAG_ENTITLEMENT,
                TAG_VECTOR
            ],
            [0, 1, 2, 3, 4]
        );
        let mut store = RequestBundleStore::try_new(1, 1).unwrap();
        assert_eq!(store.record_capacity(), 1);
        assert_eq!(store.identities.leaf_slots.len(), K);
        assert_eq!(store.identities.branch_slots.len(), K - 1);
        assert_eq!(store.find(TAG_ENTITLEMENT, &[1; 32], &mut work()), Ok(None));
        let record = bundle_record(1);
        store.records[0] = RecordSlot::Occupied(record);
        store.identities.leaf_slots[0] = LeafSlot::Occupied {
            owner_record: 0,
            key_ordinal: 9,
        };
        store.identities.root = 0;
        assert_eq!(
            store.find(TAG_ENTITLEMENT, &record.entitlement.get(), &mut work()),
            Ok(Some(0))
        );
        let before = store.clone();
        store.identities.leaf_slots[0] = LeafSlot::Occupied {
            owner_record: 1,
            key_ordinal: 9,
        };
        assert_eq!(
            store.find(TAG_ENTITLEMENT, &record.entitlement.get(), &mut work()),
            Err(FixedStorageError::NonCanonical)
        );
        store = before;
        store.identities.leaf_slots[0] = LeafSlot::Occupied {
            owner_record: 0,
            key_ordinal: K as u8,
        };
        assert_eq!(
            store.find(TAG_ENTITLEMENT, &record.entitlement.get(), &mut work()),
            Err(FixedStorageError::NonCanonical)
        );
    }

    /// Test-only distinct bundle record with canonical same-tag identity groups.
    fn bundle_record(n: u8) -> BundleRecord {
        BundleRecord::from_input(&bundle_input(n, &[]), 0)
    }
    /// Test-only full request-bundle store oracle: record/cell/leaf/branch
    /// partitions, free-stack validity, trie and arena oracles, and every
    /// cross-ownership relation plus all four scalar conservation equations.
    /// Scans `Theta(C + I + J + E)` slots and is never called or charged by a
    /// production transition.
    fn bundle_store_oracle(store: &RequestBundleStore) {
        use std::collections::HashSet;
        let mut free_records = HashSet::new();
        for (position, &record) in store.free_records.iter().enumerate() {
            assert!(
                record < store.record_capacity() as u32,
                "free record in range"
            );
            assert!(free_records.insert(record), "free record indices unique");
            assert_eq!(
                store.records[record as usize],
                RecordSlot::Vacant {
                    free_position: position as u32,
                },
                "free record vacant"
            );
        }
        let occupied: Vec<u32> = store
            .records
            .iter()
            .enumerate()
            .filter_map(|(index, slot)| {
                matches!(slot, RecordSlot::Occupied(_)).then_some(index as u32)
            })
            .collect();
        assert_eq!(
            store.record_len(),
            occupied.len(),
            "constant-time occupied record count"
        );
        assert_eq!(
            free_records.len() + occupied.len(),
            store.record_capacity(),
            "record partition"
        );
        trie_oracle(&store.identities, &store.records);
        arena_oracle(&store.cells);
        for &record in &occupied {
            let slot = store.get_record(record).expect("occupied record");
            for key in slot.tagged_keys() {
                assert_eq!(
                    store
                        .identities
                        .find(&store.records, key.tag, &key.identity, &mut work(),),
                    Ok(Some(record)),
                    "every record identity present and owned"
                );
            }
            let head = slot.vector_head;
            store
                .cells
                .validate_owner_chain(
                    head,
                    usize::try_from(slot.vector_len).unwrap(),
                    record,
                    &mut work(),
                )
                .unwrap();
        }
        for (owner_record, key_ordinal) in
            store
                .identities
                .leaf_slots
                .iter()
                .filter_map(|slot| match slot {
                    LeafSlot::Occupied {
                        owner_record,
                        key_ordinal,
                    } => Some((*owner_record, *key_ordinal)),
                    LeafSlot::Vacant { .. } => None,
                })
        {
            assert!(
                matches!(
                    store.records[owner_record as usize],
                    RecordSlot::Occupied(_)
                ) && key_ordinal < K as u8,
                "every index leaf owner is an occupied record"
            );
        }
        let cells_owned: usize = occupied
            .iter()
            .map(|&record| {
                usize::try_from(
                    store
                        .get_record(record)
                        .expect("occupied record")
                        .vector_len,
                )
                .unwrap()
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
            std::mem::size_of::<RecordSlot>() as u64 + std::mem::size_of::<u32>() as u64;
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
        assert_eq!(std::mem::size_of::<RecordSlot>(), 1_008);
        assert_eq!(std::mem::size_of::<RequestBundleStore>(), 208);
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
    fn c16_bundle_record_destination_corruption_rejects_safely() {
        let store = RequestBundleStore::try_new(4, 8).unwrap();
        let mut out_of_range = store.clone();
        *out_of_range.free_records.last_mut().unwrap() = u32::MAX;
        assert_eq!(
            out_of_range.validate_record_slot(&mut work()),
            Err(FixedStorageError::NonCanonical)
        );
        let mut wrong_position = store.clone();
        wrong_position.records[0] = RecordSlot::Vacant { free_position: 0 };
        assert_eq!(
            wrong_position.validate_record_slot(&mut work()),
            Err(FixedStorageError::NonCanonical)
        );
        let mut occupied = store;
        occupied.records[0] = RecordSlot::Occupied(bundle_record(1));
        assert_eq!(
            occupied.validate_record_slot(&mut work()),
            Err(FixedStorageError::NonCanonical)
        );
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
            BundleState::RetainedTombstone
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
