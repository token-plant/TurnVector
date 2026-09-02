pub(crate) mod c17;
pub(crate) mod c18;

use crate::bounded::FixedWindowStart;
use crate::c17_layout::{Assignment, WORK_MIGRATED_C16, WORK_TOMBSTONE, legacy_migrated};
use crate::reusable::AssignmentOrderKey;
use crate::work::{ExactWorkCensus, WorkRecorder};
use crate::{
    Duration, FixedRecordArena, FixedStartCountBound, FixedStorageError, FixedWindowCounter,
    FutureTurnSupportEntitlementId, HotPathWorkWitness, MonotonicTime, PhysicalStartCreditId,
    RequestId, RuntimeOverheadBoundSetId, SupportLedgerGeneration, SupportOperationObligationId,
    SupportOutstandingCreditVectorId, TurnPlan, WorkBudgetError, WorkDimension, WorkMeter,
};
use std::sync::atomic::{AtomicU64, Ordering};
const POOLS: usize = 3;
const CONDITIONAL: usize = 0;
const PENDING: usize = 1;
const ACTIVE: usize = 2;
const CREDITS: usize = 3;
const CLAIMS: usize = 4;

fn migrated_legacy_witness(
    work: HotPathWorkWitness,
) -> Result<HotPathWorkWitness, SupportLedgerError> {
    if work.value(WorkDimension::Allocations) != 0 || work.value(WorkDimension::CandidateWork) != 0
    {
        return Err(SupportLedgerError::Storage(FixedStorageError::NonCanonical));
    }
    legacy_migrated([
        work.value(WorkDimension::VisitedEntities),
        work.value(WorkDimension::CopiedBytes),
        0,
        0,
        work.value(WorkDimension::InvariantChecks),
    ])
    .map(HotPathWorkWitness::new)
    .ok_or(SupportLedgerError::Storage(FixedStorageError::Capacity))
}

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
pub(crate) enum SupportChangeInput { BeginOrdinary(OrdinarySupportSpec, MonotonicTime), BeginPending(SupportOperationObligationId, LifecycleReserveKind, MonotonicTime), FinishActive(SupportOperationObligationId, MonotonicTime) }
enum SupportDelta {
    BeginOrdinary(OrdinarySupportSpec, MonotonicTime, FixedWindowStart),
    BeginPending(
        usize,
        Record,
        SupportOperationObligationId,
        MonotonicTime,
        FixedWindowStart,
    ),
    FinishActive(usize, Record, SupportOperationObligationId, MonotonicTime),
    FinishInitial(
        u32,
        u8,
        InitialRequirementRecord,
        BundleState,
        MonotonicTime,
    ),
}
enum LegacyC17Change {
    Insert(c17::PreparedLegacyInsert),
    Update(c17::PreparedLegacyUpdate),
    C16Touch(c17::PreparedC16Touch),
}
pub(crate) struct SupportChange {
    expected: SupportLedgerGeneration,
    records: usize,
    delta: SupportDelta,
    charge: Option<HotPathWorkWitness>,
    c17: Option<LegacyC17Change>,
}

pub(crate) struct PreparedC17PlanCreate {
    expected: SupportLedgerGeneration,
    generation_after: SupportLedgerGeneration,
    member_count: usize,
    owner_slots: [u32; 4],
    owner_records_before: [Option<BundleRecord>; 4],
    owner_records_after: [Option<BundleRecord>; 4],
    cell_outcomes: [C17DirectCellOutcome; C17_DIRECT_CELL_MAX],
    cell_count: usize,
    usage_after: [[u32; POOLS]; 5],
    reserved_after: [[u32; POOLS]; 5],
    attached_after: [[u32; POOLS]; 4],
    c17: c17::PreparedPlanCreate,
}

impl PreparedC17PlanCreate {
    pub(crate) const fn expected_generation(&self) -> SupportLedgerGeneration {
        self.expected
    }

    pub(crate) fn visit_assignments(
        &self,
        visitor: &mut dyn FnMut(AssignmentOrderKey, Assignment),
    ) {
        self.c17.visit_assignments(visitor);
    }
}

const C17_DIRECT_CELL_MAX: usize = 4 * 21 * 3;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct C17DirectCellOutcome {
    cell_slot: u32,
    current_before: u64,
    current_after: u64,
}

impl C17DirectCellOutcome {
    const ZERO: Self = Self {
        cell_slot: 0,
        current_before: 0,
        current_after: 0,
    };
}

pub(crate) struct PreparedC17CreateStandalone {
    expected: SupportLedgerGeneration,
    generation_after: SupportLedgerGeneration,
    owner_slot: u32,
    owner_record_before: BundleRecord,
    owner_record_after: BundleRecord,
    cell_outcomes: [C17DirectCellOutcome; C17_DIRECT_CELL_MAX],
    cell_count: usize,
    usage_after: [[u32; POOLS]; 5],
    reserved_after: [[u32; POOLS]; 5],
    attached_after: [[u32; POOLS]; 4],
    c17: c17::PreparedCreateStandaloneRoot,
}

pub(crate) struct PreparedC17MergeInitial {
    expected: SupportLedgerGeneration,
    generation_after: SupportLedgerGeneration,
    owner_count: usize,
    owner_slots: [u32; 4],
    owner_records_before: [Option<BundleRecord>; 4],
    owner_records_after: [Option<BundleRecord>; 4],
    cell_outcomes: [C17DirectCellOutcome; C17_DIRECT_CELL_MAX],
    cell_count: usize,
    usage_after: [[u32; POOLS]; 5],
    reserved_after: [[u32; POOLS]; 5],
    attached_after: [[u32; POOLS]; 4],
    c17: c17::PreparedMergeInitialTopology,
}

pub(crate) struct PreparedC17MembershipTopology {
    expected: SupportLedgerGeneration,
    generation_after: SupportLedgerGeneration,
    owner_count: usize,
    owner_slots: [u32; 4],
    owner_records_before: [Option<BundleRecord>; 4],
    owner_records_after: [Option<BundleRecord>; 4],
    cell_outcomes: [C17DirectCellOutcome; C17_DIRECT_CELL_MAX],
    cell_count: usize,
    usage_after: [[u32; POOLS]; 5],
    reserved_after: [[u32; POOLS]; 5],
    attached_after: [[u32; POOLS]; 4],
    vector_after: [[u64; 3]; 21],
    c17: c17::PreparedMembershipTopology,
}

impl PreparedC17CreateStandalone {
    pub(crate) const fn generation_after(&self) -> SupportLedgerGeneration {
        self.generation_after
    }

    pub(crate) fn visit_assignments(
        &self,
        visitor: &mut dyn FnMut(AssignmentOrderKey, Assignment),
    ) {
        self.c17.visit_assignments(visitor);
    }
}

impl PreparedC17MergeInitial {
    pub(crate) const fn generation_after(&self) -> SupportLedgerGeneration {
        self.generation_after
    }

    pub(crate) fn visit_assignments(
        &self,
        visitor: &mut dyn FnMut(AssignmentOrderKey, Assignment),
    ) {
        self.c17.visit_assignments(visitor);
    }
}

impl PreparedC17MembershipTopology {
    pub(crate) const fn generation_after(&self) -> SupportLedgerGeneration {
        self.generation_after
    }

    pub(crate) fn visit_assignments(
        &self,
        visitor: &mut dyn FnMut(AssignmentOrderKey, Assignment),
    ) {
        self.c17.visit_assignments(visitor);
    }
}

pub(crate) struct PreparedC17RootBatch {
    expected: SupportLedgerGeneration,
    // The lifecycle transitions advance the C17 generation without advancing the outer
    // ledger generation, so a seal must bind and revalidate the inner one as well or a
    // stale seal could write its header after-image back over newer state.
    expected_c17: u64,
    generation_after: SupportLedgerGeneration,
    owner_count: usize,
    owner_slots: [u32; 4],
    owner_records_before: [Option<BundleRecord>; 4],
    owner_records_after: [Option<BundleRecord>; 4],
    cell_outcomes: [C17DirectCellOutcome; C17_DIRECT_CELL_MAX],
    cell_count: usize,
    usage_after: [[u32; POOLS]; 5],
    reserved_after: [[u32; POOLS]; 5],
    attached_after: [[u32; POOLS]; 4],
    vector_after: [[u64; 3]; 21],
    c17: c17::PreparedRootBatch,
}

impl PreparedC17RootBatch {
    pub(crate) const fn expected_generation(&self) -> SupportLedgerGeneration {
        self.expected
    }

    pub(crate) fn visit_assignments(
        &self,
        visitor: &mut dyn FnMut(AssignmentOrderKey, Assignment),
    ) {
        self.c17.visit_assignments(visitor);
    }
}

pub(crate) struct PreparedC17LifecycleBegin {
    expected: SupportLedgerGeneration,
    c17: c17::PreparedLifecycleBegin,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct C17LifecycleOwnerOutcome {
    owner_slot: u32,
    linked_after: u32,
}

impl C17LifecycleOwnerOutcome {
    const ZERO: Self = Self {
        owner_slot: 0,
        linked_after: 0,
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct C17LifecycleCellOutcome([u8; 12]);

impl C17LifecycleCellOutcome {
    const ZERO: Self = Self([0; 12]);

    fn new(cell_slot: u32, current_after: u64) -> Self {
        let mut image = [0; 12];
        image[..4].copy_from_slice(&cell_slot.to_le_bytes());
        image[4..].copy_from_slice(&current_after.to_le_bytes());
        Self(image)
    }

    fn cell_slot(self) -> u32 {
        u32::from_le_bytes(self.0[..4].try_into().expect("fixed cell outcome slot"))
    }

    fn current_after(self) -> u64 {
        u64::from_le_bytes(self.0[4..].try_into().expect("fixed cell outcome current"))
    }

    fn increment(&mut self) -> Option<u64> {
        let after = self.current_after().checked_add(1)?;
        self.0[4..].copy_from_slice(&after.to_le_bytes());
        Some(after)
    }
}

pub(crate) struct PreparedC17LifecycleFinalize {
    expected: SupportLedgerGeneration,
    generation_after: SupportLedgerGeneration,
    usage_after: [[u32; POOLS]; 5],
    reserved_after: [[u32; POOLS]; 5],
    attached_after: [[u32; POOLS]; 4],
    vector_after: [[u64; 3]; 21],
    owner_outcomes: [C17LifecycleOwnerOutcome; crate::c17_layout::LIFECYCLE_CAPACITY],
    owner_count: usize,
    cell_outcomes: [C17LifecycleCellOutcome; c17::LIFECYCLE_PUBLICATION_MAX],
    cell_count: usize,
    c17_owner_outcomes: [c17::LifecycleOwnerOutcome; crate::c17_layout::LIFECYCLE_CAPACITY],
    c17_owner_count: usize,
    c17_funder_outcomes: [c17::LifecycleFunderOutcome; crate::c17_layout::LIFECYCLE_CAPACITY],
    c17_funder_count: usize,
    c17: c17::PreparedLifecycleFinalize,
}

impl std::fmt::Debug for PreparedC17LifecycleFinalize {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PreparedC17LifecycleFinalize")
            .field("expected", &self.expected)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for PreparedC17LifecycleBegin {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PreparedC17LifecycleBegin")
            .field("expected", &self.expected)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for PreparedC17MergeInitial {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PreparedC17MergeInitial")
            .field("expected", &self.expected)
            .field("owner_count", &self.owner_count)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for PreparedC17MembershipTopology {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PreparedC17MembershipTopology")
            .field("expected", &self.expected)
            .field("owner_count", &self.owner_count)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for PreparedC17RootBatch {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PreparedC17RootBatch")
            .field("expected", &self.expected)
            .field("owner_count", &self.owner_count)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for PreparedC17CreateStandalone {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PreparedC17CreateStandalone")
            .field("expected", &self.expected)
            .field("owner_slot", &self.owner_slot)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for PreparedC17PlanCreate {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PreparedC17PlanCreate")
            .field("expected", &self.expected)
            .field("member_count", &self.member_count)
            .finish_non_exhaustive()
    }
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
    FinishSupport(MonotonicTime),
    CloseCausalCallImpossible(MonotonicTime),
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
    RecordMetadata,
);
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
struct RecordMetadata {
    lifecycle_kind: Option<LifecycleReserveKind>,
    physical_credit: PhysicalStartCreditId,
    lifecycle_count: u16,
    reserved: u16,
    first_record: u32,
}

impl RecordMetadata {
    fn ordinary(physical_credit: PhysicalStartCreditId) -> Self {
        Self {
            lifecycle_kind: None,
            physical_credit,
            lifecycle_count: 0,
            reserved: 0,
            first_record: 0,
        }
    }

    fn lifecycle(
        kind: LifecycleReserveKind,
        physical_credit: PhysicalStartCreditId,
        count: u16,
        first_record: usize,
    ) -> Result<Self, SupportLedgerError> {
        Ok(Self {
            lifecycle_kind: Some(kind),
            physical_credit,
            lifecycle_count: count,
            reserved: 0,
            first_record: u32::try_from(first_record)
                .map_err(|_| SupportLedgerError::Storage(FixedStorageError::Capacity))?,
        })
    }
}
#[derive(Clone, Copy)]
enum ObligationOwner {
    Legacy { index: usize, record: Record },
    InitialBundle { record: u32, ordinal: u8 },
}
#[derive(Debug, Eq, PartialEq)]
#[repr(C)]
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
    c17: c17::SupportC17,
    c18: c18::SupportC18<H>,
}

/// The exclusive validated expiry capability. It holds the sole mutable ledger
/// borrow; dropping it without committing changes nothing.
pub(crate) struct ValidatedSupportExpiry<
    'ledger,
    'work,
    const R: usize,
    const F: usize,
    const H: usize,
    const E_GROUPS: usize,
> {
    ledger: &'ledger mut SupportChargeLedger<R, F, H>,
    prepared: c18::PreparedSupportExpiry<'work, E_GROUPS>,
}

impl<const R: usize, const F: usize, const H: usize, const E_GROUPS: usize>
    ValidatedSupportExpiry<'_, '_, R, F, H, E_GROUPS>
{
    /// Consuming and infallible after validation. It frees complete groups and
    /// advances the generation exactly once when the batch is nonempty; a
    /// zero-group batch leaves the generation unchanged.
    pub(crate) fn commit(self) -> c18::ExpiryCommit {
        let Self { ledger, prepared } = self;
        // Free the authoritative group before the retry ticket is destroyed:
        // occupancy, the one physical start credit, every funding claim, both
        // raw owner keys and the record slot itself all return together, so a
        // reported release is an actual release.
        let mut units = 0u32;
        for position in 0..prepared.count {
            let ticket = prepared.selected[position];
            units += ticket.units;
            ledger.release_group(&ticket);
        }
        let (released_groups, released_units) =
            ledger.c18.commit_expiry(prepared.at, prepared.count, units);
        if released_groups > 0 {
            ledger.generation = ledger
                .generation
                .next()
                .expect("validated expiry generation");
        }
        c18::ExpiryCommit {
            generation: ledger.generation,
            released_groups,
            released_units,
            more_due: prepared.more_due,
            next_expiry_at: ledger.c18.next_release(),
        }
    }
}

pub(crate) const C17_LANDED_PREFIX_BYTES: usize =
    std::mem::offset_of!(SupportChargeLedger<16_530, 2_057, 3>, c17);

#[cfg(turnvector_c17_probe)]
pub(crate) fn b03_probe_rows() -> Vec<(&'static str, usize)> {
    use std::mem::{align_of, offset_of, size_of};
    vec![
        ("support.landed_prefix", C17_LANDED_PREFIX_BYTES),
        (
            "support.c17_offset",
            offset_of!(SupportChargeLedger<16_530, 2_057, 3>, c17),
        ),
        (
            "support.inline_size",
            size_of::<SupportChargeLedger<16_530, 2_057, 3>>(),
        ),
        (
            "support.inline_align",
            align_of::<SupportChargeLedger<16_530, 2_057, 3>>(),
        ),
        ("support.c17_inline_size", size_of::<c17::SupportC17>()),
        ("support.legacy_record", size_of::<Record>()),
        (
            "support.legacy_avl_node",
            size_of::<crate::bounded::AvlNode>(),
        ),
        ("support.funding_claim", size_of::<SupportFundingClaim>()),
        ("support.monotonic_time", size_of::<MonotonicTime>()),
        ("support.bundle_record_slot", size_of::<RecordSlot>()),
        ("support.c16_leaf", size_of::<LeafSlot>()),
        ("support.c16_branch", size_of::<BranchSlot>()),
        ("support.cell_slot", size_of::<CellSlot>()),
    ]
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
        limits: c18::SupportHistoryLimits<H>,
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
        let retained = limits.start_history_capacity;
        let history_slots = limits
            .start_history_capacity
            .iter()
            .try_fold(0u64, |total, capacity| {
                total.checked_add(u64::from(*capacity))
            });
        let storage = support_storage_bytes(
            H,
            records,
            claims,
            history_slots.ok_or(SupportLedgerError::InvalidInput)?,
            bundle_records,
            bundle_cells,
        )?;
        #[cfg(test)]
        let c17_capacities = c17::SupportC17Capacities::testing();
        #[cfg(not(test))]
        let c17_capacities = c17::SupportC17Capacities::production();
        let c17_storage = c17::SupportC17::physical_bytes(c17::SupportC17Capacities::production())
            .ok_or(SupportLedgerError::Storage(FixedStorageError::Capacity))?;
        let whole_storage = storage
            .checked_add(c17_storage)
            .ok_or(SupportLedgerError::Storage(FixedStorageError::Capacity))?;
        if whole_storage > crate::c17_generated::SUPPORT_LEDGER_CEILING_BYTES {
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
        let identity_capacity = records
            .checked_mul(2)
            .ok_or(SupportLedgerError::InvalidInput)?;
        let bundle_identity_capacity = bundle_records
            .checked_mul(K)
            .ok_or(SupportLedgerError::InvalidInput)?;
        let expected_backing = SupportBackingCapacities {
            legacy: [
                records,
                claims,
                records,
                identity_capacity,
                claims + 1,
                claims,
            ],
            history: retained.map(|capacity| capacity as usize),
            bundles: [
                bundle_records,
                bundle_records,
                bundle_identity_capacity,
                bundle_identity_capacity,
                bundle_identity_capacity - 1,
                bundle_identity_capacity - 1,
                bundle_cells,
                bundle_cells,
            ],
        };
        let records = FixedRecordArena::try_new(records, claims)?;
        let vector_capacity = std::array::from_fn(|cell| {
            std::array::from_fn(|horizon| u64::from(starts[cell][horizon].1))
        });
        let c18 = c18::SupportC18::try_new(limits, &starts)?;
        let starts = FixedWindowCounter::try_new(starts, retained)?;
        let bundles = RequestBundleStore::try_new(bundle_records, bundle_cells)?;
        let c17 = c17::SupportC17::try_new(c17_capacities)?;
        let actual_backing = SupportBackingCapacities {
            legacy: records.backing_capacities(),
            history: starts.backing_capacities(),
            bundles: bundles.backing_capacities(),
        };
        let bundle_vector_max =
            u16::try_from(bundle_vector_max).map_err(|_| SupportLedgerError::InvalidInput)?;
        let instance_nonce = seal_backing_and_issue_nonce(
            &PROCESS_INSTANCE_DISPENSER,
            H,
            storage,
            expected_backing,
            actual_backing,
        )?;
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
            c17,
            c18,
        })
    }
    pub const fn generation(&self) -> SupportLedgerGeneration {
        self.generation
    }

    /// Releases one whole retained group. Every unit of occupancy the group
    /// holds is returned in the same step, so no caller can observe a partially
    /// freed group. Validation already proved the group is present and
    /// occupied, so a violated count here is internal noncanonical state and
    /// fails stop rather than saturating.
    fn release_group(&mut self, ticket: &c18::ExpiryTicket) {
        if ticket.family == c18::OwnerFamily::Tombstone {
            // The bundle is the release group. Everything it holds returns in
            // one step: the logical occupancy and vector cells it still charges,
            // its unified raw owner rows, and only then its physical record,
            // identity leaves and owned cells. Freeing storage alone would leave
            // the capacity and the owner directory charged forever.
            let record = *self
                .bundles
                .get_record(ticket.slot_index)
                .expect("scheduled tombstone record");
            let delta = self
                .stored_bundle_logical_delta_precharged(ticket.slot_index, &record)
                .expect("validated retained bundle aggregates");
            for class in 0..5 {
                for pool in 0..POOLS {
                    self.usage[class][pool] = self.usage[class][pool]
                        .checked_sub(delta.usage[class][pool])
                        .expect("validated retained bundle occupancy");
                    self.reserved[class][pool] = self.reserved[class][pool]
                        .checked_sub(delta.reserved[class][pool])
                        .expect("validated retained bundle reserves");
                }
            }
            for axis in 0..21 {
                for horizon in 0..H {
                    self.vector_usage[axis][horizon] = self.vector_usage[axis][horizon]
                        .checked_sub(delta.vector[axis][horizon])
                        .expect("validated retained bundle vector");
                }
            }
            let c17 = self
                .c17
                .prepare_c16_tombstone_release(ticket.slot_index, &record)
                .expect("validated retained bundle owners");
            self.c17.commit_c16_withdrawal(c17);
            self.bundles.withdraw_bundle_unmetered(ticket.slot_index);
            return;
        }
        let index = ticket.slot_index as usize;
        let Some(record) = self.records.get(index).copied() else {
            return;
        };
        let pool = record.1 as usize;
        let claims = self.records.claims(index).map_or(0, <[_]>::len) as u32;
        let credit = record.6.physical_credit.get();
        // The raw owner directory releases both keys, then the arena releases
        // the record slot, its claim span and its identities.
        let change = self
            .c17
            .prepare_legacy_release(ticket.identity, credit)
            .expect("validated retained group raw owners");
        self.c17.commit_legacy_release(change);
        self.records
            .remove(index, [key(0, ticket.identity), key(1, credit)]);
        let occupied = state_class(record.3);
        for (class, released) in [(occupied, 1), (CREDITS, 1), (CLAIMS, claims)] {
            self.usage[class][pool] = self.usage[class][pool]
                .checked_sub(released)
                .expect("validated retained group occupancy");
        }
    }

    pub(crate) fn commit_c17_assignment_direct(
        &mut self,
        assignment: &crate::c17_layout::Assignment,
    ) {
        self.c17.commit_assignment_direct(assignment);
    }

    fn accumulate_c17_direct_cell_outcome(
        outcomes: &mut [C17DirectCellOutcome; C17_DIRECT_CELL_MAX],
        count: &mut usize,
        cell_slot: u32,
        current_before: u64,
        delta: i32,
        maximum: u64,
    ) -> Result<(), SupportLedgerError> {
        let index = if let Some(index) = outcomes[..*count]
            .iter()
            .position(|outcome| outcome.cell_slot == cell_slot)
        {
            if outcomes[index].current_before != current_before {
                return Err(SupportLedgerError::Storage(FixedStorageError::NonCanonical));
            }
            index
        } else {
            if *count == outcomes.len() {
                return Err(CAPACITY_ERROR);
            }
            let index = *count;
            outcomes[index] = C17DirectCellOutcome {
                cell_slot,
                current_before,
                current_after: current_before,
            };
            *count += 1;
            index
        };
        let current = outcomes[index].current_after;
        let current_after = if delta >= 0 {
            current.checked_add(delta as u64).ok_or(CAPACITY_ERROR)?
        } else {
            current
                .checked_sub(u64::from(delta.unsigned_abs()))
                .ok_or(SupportLedgerError::Storage(FixedStorageError::NonCanonical))?
        };
        if current_after > maximum {
            return Err(CAPACITY_ERROR);
        }
        outcomes[index].current_after = current_after;
        Ok(())
    }

    fn prepare_c17_direct_cell_outcomes(
        &self,
        owner_count: usize,
        owner_slots: [u32; 4],
        owner_records: [Option<BundleRecord>; 4],
        branch_deltas: [[i32; 4]; 4],
        retractions: &[c17::LifecyclePublication],
    ) -> Result<([C17DirectCellOutcome; C17_DIRECT_CELL_MAX], usize), SupportLedgerError> {
        if owner_count > owner_slots.len()
            || owner_records[owner_count..].iter().any(Option::is_some)
        {
            return Err(SupportLedgerError::InvalidInput);
        }
        self.validate_c17_lifecycle_retractions(retractions)?;
        let mut outcomes = [C17DirectCellOutcome::ZERO; C17_DIRECT_CELL_MAX];
        let mut count = 0usize;
        for publication in retractions.iter().copied() {
            let owner_slot = publication.owner_slot();
            let record = self
                .bundles
                .get_record(owner_slot)
                .ok_or(SupportLedgerError::InvalidTransition)?;
            let cell_slot = self.lifecycle_publication_cell(
                owner_slot,
                record,
                publication.axis(),
                publication.horizon(),
            )?;
            let CellSlot::Occupied { cell, current, .. } =
                self.bundles.cells.slots[cell_slot as usize]
            else {
                return Err(SupportLedgerError::Storage(FixedStorageError::NonCanonical));
            };
            Self::accumulate_c17_direct_cell_outcome(
                &mut outcomes,
                &mut count,
                cell_slot,
                current,
                -1,
                cell.max_outstanding,
            )?;
        }
        for index in 0..owner_count {
            let owner_slot = owner_slots[index];
            let record = owner_records[index].ok_or(SupportLedgerError::InvalidTransition)?;
            let len = usize::try_from(record.vector_len)
                .map_err(|_| SupportLedgerError::Storage(FixedStorageError::NonCanonical))?;
            self.bundles
                .validate_owner_chain_precharged(record.vector_head, len, owner_slot)?;
            let mut next = record.vector_head;
            for _ in 0..len {
                let cell_slot = next;
                let CellSlot::Occupied {
                    owner_record,
                    cell,
                    current,
                    next_owned,
                } = self.bundles.cells.slots[cell_slot as usize]
                else {
                    return Err(SupportLedgerError::Storage(FixedStorageError::NonCanonical));
                };
                if owner_record != owner_slot || current > cell.max_outstanding {
                    return Err(SupportLedgerError::Storage(FixedStorageError::NonCanonical));
                }
                let delta = record
                    .branches
                    .iter()
                    .zip(branch_deltas[index])
                    .filter(|(requirement, _)| {
                        requirement.operation == cell.operation && requirement.pool == cell.pool
                    })
                    .try_fold(0i32, |total, (_, delta)| total.checked_add(delta))
                    .ok_or(CAPACITY_ERROR)?;
                Self::accumulate_c17_direct_cell_outcome(
                    &mut outcomes,
                    &mut count,
                    cell_slot,
                    current,
                    delta,
                    cell.max_outstanding,
                )?;
                next = next_owned;
            }
        }
        Ok((outcomes, count))
    }

    fn commit_c17_direct_cell_outcomes(&mut self, outcomes: &[C17DirectCellOutcome]) {
        for outcome in outcomes.iter().copied() {
            let CellSlot::Occupied { current, .. } =
                &mut self.bundles.cells.slots[outcome.cell_slot as usize]
            else {
                unreachable!("sealed C17 direct cell destination")
            };
            debug_assert_eq!(*current, outcome.current_before);
            *current = outcome.current_after;
        }
    }

    fn prepare_c17_plan_owner_records_after(
        owner_records_before: [Option<BundleRecord>; 4],
        member_count: usize,
    ) -> Result<[Option<BundleRecord>; 4], SupportLedgerError> {
        if !(1..=4).contains(&member_count)
            || owner_records_before[..member_count]
                .iter()
                .any(Option::is_none)
            || owner_records_before[member_count..]
                .iter()
                .any(Option::is_some)
        {
            return Err(SupportLedgerError::InvalidInput);
        }
        let mut owner_records_after = owner_records_before;
        for record in owner_records_after[..member_count].iter_mut().flatten() {
            record.linked_claims = record.linked_claims.checked_add(3).ok_or(CAPACITY_ERROR)?;
            if record.state == BundleState::LivePristine {
                record.state = BundleState::LiveConsumed;
            }
        }
        Ok(owner_records_after)
    }

    pub(crate) fn prepare_c17_plan_create<const MEMBERS: usize, W: WorkRecorder>(
        &self,
        expected: SupportLedgerGeneration,
        plan: &TurnPlan<MEMBERS>,
        occurred_at: MonotonicTime,
        work: &mut W,
    ) -> Result<PreparedC17PlanCreate, SupportLedgerError> {
        if expected != self.generation || occurred_at.as_micros() == 0 {
            return Err(SupportLedgerError::Generation);
        }
        let generation_after = expected
            .next()
            .map_err(|_| SupportLedgerError::Generation)?;
        let member_count = plan.members().len();
        if !(1..=4).contains(&member_count) {
            return Err(SupportLedgerError::InvalidInput);
        }
        let support = plan.support();
        let obligations = [
            support.receipt_observation.id.get(),
            support.conditional_continuation_formation.id.get(),
            support.rejection_or_local_stale_formation.id.get(),
        ];
        let credits = [
            support.receipt_observation.physical_credit.get(),
            support
                .conditional_continuation_formation
                .physical_credit
                .get(),
            support
                .rejection_or_local_stale_formation
                .physical_credit
                .get(),
        ];
        let funding_sets = [
            &support.receipt_observation.funders,
            &support.conditional_continuation_formation.funders,
            &support.rejection_or_local_stale_formation.funders,
        ];
        if funding_sets
            .into_iter()
            .any(|funders| funders.as_slice() != plan.members().as_slice())
        {
            return Err(SupportLedgerError::InvalidInput);
        }

        let mut members = [c17::PlanCreateMember::ZERO; 4];
        let mut owner_slots = [0; 4];
        let mut owner_records_before = [None; 4];
        for (ordinal, funding) in plan.members().iter().copied().enumerate() {
            let (record_slot, record, branch_limits) =
                self.find_funding_owner_precharged(funding, Some(plan.identity().bound_set))?;
            let branch_limits = [branch_limits[0], branch_limits[1], branch_limits[2]];
            let owner_header = self.c17.c16_owner_header_ref(record_slot, &record)?;
            members[ordinal] = c17::PlanCreateMember {
                request: Some(funding.request_id),
                request_key: crate::request_book::c17::request_key(funding.request_id),
                record_slot,
                owner_header,
                entitlement: funding.entitlement.get(),
                vector: funding.credit_vector.get(),
                branch_limits,
            };
            owner_slots[ordinal] = record_slot;
            owner_records_before[ordinal] = Some(record);
        }
        let input = c17::PlanCreateInput {
            authority_key: plan_authority_key(plan.identity().id.get()),
            identity: encode_plan_identity(plan.identity()),
            obligations,
            credits,
            members,
            member_count,
            occurred_at: occurred_at.as_micros(),
        };
        let (usage_after, reserved_after, attached_after) =
            self.validate_plan_materialization(member_count)?;
        let branch_deltas = std::array::from_fn(|index| {
            if index < member_count {
                [1, 1, 1, 0]
            } else {
                [0; 4]
            }
        });
        let (cell_outcomes, cell_count) = self.prepare_c17_direct_cell_outcomes(
            member_count,
            owner_slots,
            owner_records_before,
            branch_deltas,
            &[],
        )?;
        let owner_records_after =
            Self::prepare_c17_plan_owner_records_after(owner_records_before, member_count)?;
        let c17 = self.c17.prepare_plan_create(input, owner_records_before)?;
        work.charge(HotPathWorkWitness::new(crate::c17_layout::WORK_PLAN_CREATE))?;
        Ok(PreparedC17PlanCreate {
            expected,
            generation_after,
            member_count,
            owner_slots,
            owner_records_before,
            owner_records_after,
            cell_outcomes,
            cell_count,
            usage_after,
            reserved_after,
            attached_after,
            c17,
        })
    }

    pub(crate) fn validate_c17_plan_create(
        &self,
        change: &PreparedC17PlanCreate,
    ) -> Result<(), SupportLedgerError> {
        if self.generation != change.expected
            || change
                .expected
                .next()
                .map_err(|_| SupportLedgerError::Generation)?
                != change.generation_after
            || !(1..=4).contains(&change.member_count)
            || self.validate_plan_materialization(change.member_count)?
                != (
                    change.usage_after,
                    change.reserved_after,
                    change.attached_after,
                )
        {
            return Err(SupportLedgerError::Generation);
        }
        let mut current = [None; 4];
        for ordinal in 0..change.member_count {
            if change.owner_slots[..ordinal].contains(&change.owner_slots[ordinal]) {
                return Err(SupportLedgerError::Generation);
            }
            current[ordinal] = Some(
                *self
                    .bundles
                    .get_record(change.owner_slots[ordinal])
                    .ok_or(SupportLedgerError::Generation)?,
            );
        }
        let branch_deltas = std::array::from_fn(|index| {
            if index < change.member_count {
                [1, 1, 1, 0]
            } else {
                [0; 4]
            }
        });
        let (cell_outcomes, cell_count) = self.prepare_c17_direct_cell_outcomes(
            change.member_count,
            change.owner_slots,
            current,
            branch_deltas,
            &[],
        )?;
        if current != change.owner_records_before
            || Self::prepare_c17_plan_owner_records_after(current, change.member_count)?
                != change.owner_records_after
            || cell_count != change.cell_count
            || cell_outcomes != change.cell_outcomes
            || change.owner_slots[change.member_count..]
                .iter()
                .any(|slot| *slot != 0)
        {
            return Err(SupportLedgerError::Generation);
        }
        self.c17
            .validate_plan_create(&change.c17, change.owner_records_before)
    }

    pub(crate) fn commit_c17_plan_create(
        &mut self,
        change: PreparedC17PlanCreate,
    ) -> SupportLedgerGeneration {
        self.commit_c17_plan_create_prevalidated(change, true)
    }

    pub(crate) fn commit_c17_plan_create_prevalidated(
        &mut self,
        change: PreparedC17PlanCreate,
        apply_index_plans: bool,
    ) -> SupportLedgerGeneration {
        assert_eq!(
            self.generation, change.expected,
            "sealed C17 Plan generation"
        );
        let PreparedC17PlanCreate {
            expected: _,
            generation_after,
            member_count,
            owner_slots,
            owner_records_before: _,
            owner_records_after,
            cell_outcomes,
            cell_count,
            usage_after,
            reserved_after,
            attached_after,
            c17,
        } = change;
        self.c17
            .commit_plan_create_prevalidated(c17, apply_index_plans);
        for ordinal in 0..member_count {
            let RecordSlot::Occupied(stored) =
                &mut self.bundles.records[owner_slots[ordinal] as usize]
            else {
                unreachable!("sealed Plan owner destination")
            };
            *stored = owner_records_after[ordinal].expect("sealed Plan owner after-image");
        }
        self.commit_c17_direct_cell_outcomes(&cell_outcomes[..cell_count]);
        self.usage = usage_after;
        self.reserved = reserved_after;
        self.c17.commit_attached_change(attached_after);
        self.generation = generation_after;
        self.generation
    }

    pub(crate) fn preview_c17_create_standalone_anchor(
        &self,
        domain: crate::FormationDomainId,
    ) -> Result<crate::request_book::c17::SupportMembershipAnchor, SupportLedgerError> {
        self.c17
            .preview_create_standalone_anchor(standalone_authority_key(domain))
    }

    pub(crate) fn prepare_c17_create_standalone(
        &self,
        expected: SupportLedgerGeneration,
        event: &crate::request_book::c17::MembershipEventRecord,
        marker: crate::request_book::c17::InitialReadyMarker,
        work: &mut impl WorkRecorder,
    ) -> Result<PreparedC17CreateStandalone, SupportLedgerError> {
        use crate::request_book::c17::{MembershipEventKind, MembershipTag};

        if expected != self.generation {
            return Err(SupportLedgerError::Generation);
        }
        let generation_after = expected
            .next()
            .map_err(|_| SupportLedgerError::Generation)?;
        marker
            .validate()
            .map_err(|_| SupportLedgerError::InvalidInput)?;
        let affected = event.affected[0].ok_or(SupportLedgerError::InvalidTransition)?;
        let before = event.before[0].ok_or(SupportLedgerError::InvalidTransition)?;
        let after = event.after[0].ok_or(SupportLedgerError::InvalidTransition)?;
        if event.kind != MembershipEventKind::CreateStandalone
            || event.source_count != 1
            || event.member_count != 1
            || !event.consumed_by_support
            || event.occurred_at != marker.occurred_at.as_micros()
            || event.cancellation_fact != 0
            || event.sources[0].is_absent()
            || event.sources[1..].iter().any(|source| !source.is_absent())
            || event.affected[1..].iter().any(Option::is_some)
            || event.before[1..].iter().any(Option::is_some)
            || event.after[1..].iter().any(Option::is_some)
            || affected.key != crate::request_book::c17::request_key(marker.request)
            || before.tag != MembershipTag::Unready
            || before.epoch != 0
            || !before.anchor.is_absent()
            || !before.initial.is_absent()
            || !before.pending.is_absent()
            || !before.cancellation.is_absent()
            || before.cancellation_fact != 0
            || before.cancellation_at != 0
            || after.tag != MembershipTag::Bound
            || after.epoch != 1
            || after.initial != event.sources[0]
            || !after.pending.is_absent()
            || !after.cancellation.is_absent()
            || after.cancellation_fact != 0
            || after.cancellation_at != 0
        {
            return Err(SupportLedgerError::InvalidTransition);
        }
        let authority_key = standalone_authority_key(
            crate::FormationDomainId::new(u128::from_be_bytes(marker.domain))
                .map_err(|_| SupportLedgerError::InvalidInput)?,
        );
        if after.anchor.authority_key() != authority_key
            || after.anchor.branch() != 3
            || after.anchor.group() != after.anchor.root()
            || after.anchor.root_version() != 1
        {
            return Err(SupportLedgerError::InvalidTransition);
        }
        let (owner_slot, owner_record, branch_limits) =
            self.find_funding_owner_precharged(marker.funding, None)?;
        let owner_header = self.c17.c16_owner_header_ref(owner_slot, &owner_record)?;
        let funding = c17::MembershipFunding {
            request: marker.funding.request_id,
            request_key: crate::request_book::c17::request_key(marker.funding.request_id),
            record_slot: owner_slot,
            owner_header,
            entitlement: marker.funding.entitlement.get(),
            vector: marker.funding.credit_vector.get(),
            branch_limit: branch_limits[3],
        };
        let input = c17::CreateStandaloneInput {
            authority_key,
            domain: marker.domain,
            source: event.sources[0],
            initial_kind: marker.kind as u8,
            event_id: event.id,
            anchor: after.anchor,
            occurred_at: marker.occurred_at.as_micros(),
            obligation: marker.obligation.get(),
            credit: marker.credit.get(),
            funding,
        };
        let delta = singleton_materialization_delta();
        let (usage_after, reserved_after, attached_after) =
            self.validate_c17_aggregate_delta(delta)?;
        let (cell_outcomes, cell_count) = self.prepare_c17_direct_cell_outcomes(
            1,
            [owner_slot, 0, 0, 0],
            [Some(owner_record), None, None, None],
            [[0, 0, 0, 1], [0; 4], [0; 4], [0; 4]],
            &[],
        )?;
        let c17 = self
            .c17
            .prepare_create_standalone_root(input, owner_record)?;
        if c17.owner_slot() != owner_slot || c17.owner_record_before() != owner_record {
            return Err(SupportLedgerError::Storage(FixedStorageError::NonCanonical));
        }
        let owner_record_after = c17.owner_record_after();
        work.charge(HotPathWorkWitness::new(
            crate::c17_layout::WORK_CREATE_STANDALONE,
        ))?;
        Ok(PreparedC17CreateStandalone {
            expected,
            generation_after,
            owner_slot,
            owner_record_before: owner_record,
            owner_record_after,
            cell_outcomes,
            cell_count,
            usage_after,
            reserved_after,
            attached_after,
            c17,
        })
    }

    pub(crate) fn validate_c17_create_standalone(
        &self,
        change: &PreparedC17CreateStandalone,
    ) -> Result<(), SupportLedgerError> {
        let current = self
            .bundles
            .get_record(change.owner_slot)
            .copied()
            .ok_or(SupportLedgerError::Generation)?;
        let (cell_outcomes, cell_count) = self.prepare_c17_direct_cell_outcomes(
            1,
            [change.owner_slot, 0, 0, 0],
            [Some(current), None, None, None],
            [[0, 0, 0, 1], [0; 4], [0; 4], [0; 4]],
            &[],
        )?;
        if self.generation != change.expected
            || change
                .expected
                .next()
                .map_err(|_| SupportLedgerError::Generation)?
                != change.generation_after
            || current != change.owner_record_before
            || change.c17.owner_slot() != change.owner_slot
            || change.c17.owner_record_before() != change.owner_record_before
            || change.c17.owner_record_after() != change.owner_record_after
            || cell_count != change.cell_count
            || cell_outcomes != change.cell_outcomes
            || self.validate_c17_aggregate_delta(singleton_materialization_delta())?
                != (
                    change.usage_after,
                    change.reserved_after,
                    change.attached_after,
                )
        {
            return Err(SupportLedgerError::Generation);
        }
        self.c17
            .validate_create_standalone_root(&change.c17, current)
    }

    pub(crate) fn commit_c17_create_standalone(
        &mut self,
        change: PreparedC17CreateStandalone,
    ) -> SupportLedgerGeneration {
        self.validate_c17_create_standalone(&change)
            .expect("validated C17 CreateStandalone transaction");
        let expected = change.expected;
        let generation_after = change.generation_after();
        self.commit_c17_create_standalone_prevalidated(change, expected, generation_after, true)
    }

    pub(crate) fn commit_c17_create_standalone_prevalidated(
        &mut self,
        change: PreparedC17CreateStandalone,
        permit_before: SupportLedgerGeneration,
        permit_after: SupportLedgerGeneration,
        apply_index_plans: bool,
    ) -> SupportLedgerGeneration {
        assert_eq!(
            self.generation, permit_before,
            "sealed CreateStandalone generation"
        );
        assert_eq!(
            change.expected, permit_before,
            "prepared CreateStandalone generation"
        );
        assert_eq!(
            change.generation_after, permit_after,
            "prepared CreateStandalone generation after"
        );
        let PreparedC17CreateStandalone {
            expected: _,
            generation_after: _,
            owner_slot,
            owner_record_before: _,
            owner_record_after,
            cell_outcomes,
            cell_count,
            usage_after,
            reserved_after,
            attached_after,
            c17,
        } = change;
        self.c17
            .commit_create_standalone_root_prevalidated(c17, apply_index_plans);
        let RecordSlot::Occupied(record) = &mut self.bundles.records[owner_slot as usize] else {
            unreachable!("validated CreateStandalone funding owner")
        };
        *record = owner_record_after;
        self.commit_c17_direct_cell_outcomes(&cell_outcomes[..cell_count]);
        self.usage = usage_after;
        self.reserved = reserved_after;
        self.c17.commit_attached_change(attached_after);
        self.generation = permit_after;
        self.generation
    }

    pub(crate) fn preview_c17_merge_initial(
        &self,
        expected: SupportLedgerGeneration,
        anchors: [crate::request_book::c17::SupportMembershipAnchor; 3],
        source_count: u8,
        domain: [u8; 16],
        occurred_at: MonotonicTime,
    ) -> Result<c17::MergeInitialPreview, SupportLedgerError> {
        if expected != self.generation {
            return Err(SupportLedgerError::Generation);
        }
        self.c17
            .inspect_merge_initial(anchors, source_count, domain, occurred_at.as_micros())
    }

    pub(crate) fn prepare_c17_merge_initial(
        &self,
        expected: SupportLedgerGeneration,
        preview: c17::MergeInitialPreview,
        event: crate::request_book::c17::MembershipEventRecord,
        work: &mut impl WorkRecorder,
    ) -> Result<PreparedC17MergeInitial, SupportLedgerError> {
        if expected != self.generation {
            return Err(SupportLedgerError::Generation);
        }
        let generation_after = expected
            .next()
            .map_err(|_| SupportLedgerError::Generation)?;
        let owner_count = preview.owner_count();
        let owner_slots = preview.owner_slots();
        if owner_count > owner_slots.len()
            || owner_slots[..owner_count]
                .iter()
                .enumerate()
                .any(|(index, slot)| owner_slots[..index].contains(slot))
            || owner_slots[owner_count..].iter().any(|slot| *slot != 0)
        {
            return Err(SupportLedgerError::InvalidInput);
        }
        let mut owner_records = [None; 4];
        for index in 0..owner_count {
            owner_records[index] = Some(
                *self
                    .bundles
                    .get_record(owner_slots[index])
                    .ok_or(SupportLedgerError::InvalidTransition)?,
            );
        }
        let (usage_after, reserved_after, attached_after) =
            self.validate_c17_aggregate_delta(preview.aggregate_delta())?;
        let mut branch_deltas = [[0; 4]; 4];
        for (index, delta) in branch_deltas.iter_mut().enumerate().take(owner_count) {
            *delta = preview
                .owner_branch_delta(index)
                .ok_or(SupportLedgerError::InvalidInput)?;
        }
        let (cell_outcomes, cell_count) = self.prepare_c17_direct_cell_outcomes(
            owner_count,
            owner_slots,
            owner_records,
            branch_deltas,
            &[],
        )?;
        let c17 = self
            .c17
            .prepare_merge_initial_topology(preview, event, owner_records, work)?;
        let owner_records_after = c17.owner_records_after();
        Ok(PreparedC17MergeInitial {
            expected,
            generation_after,
            owner_count,
            owner_slots,
            owner_records_before: owner_records,
            owner_records_after,
            cell_outcomes,
            cell_count,
            usage_after,
            reserved_after,
            attached_after,
            c17,
        })
    }

    pub(crate) fn validate_c17_merge_initial(
        &self,
        change: &PreparedC17MergeInitial,
    ) -> Result<(), SupportLedgerError> {
        if self.generation != change.expected
            || change
                .expected
                .next()
                .map_err(|_| SupportLedgerError::Generation)?
                != change.generation_after
            || change.owner_count > change.owner_slots.len()
            || self.validate_c17_aggregate_delta(change.c17.aggregate_delta())?
                != (
                    change.usage_after,
                    change.reserved_after,
                    change.attached_after,
                )
        {
            return Err(SupportLedgerError::Generation);
        }
        let mut current = [None; 4];
        let mut branch_deltas = [[0; 4]; 4];
        for (index, delta) in branch_deltas
            .iter_mut()
            .enumerate()
            .take(change.owner_count)
        {
            if change.owner_slots[..index].contains(&change.owner_slots[index]) {
                return Err(SupportLedgerError::Generation);
            }
            current[index] = Some(
                *self
                    .bundles
                    .get_record(change.owner_slots[index])
                    .ok_or(SupportLedgerError::Generation)?,
            );
            *delta = change
                .c17
                .owner_branch_delta(index)
                .ok_or(SupportLedgerError::Generation)?;
        }
        let (cell_outcomes, cell_count) = self.prepare_c17_direct_cell_outcomes(
            change.owner_count,
            change.owner_slots,
            current,
            branch_deltas,
            &[],
        )?;
        if current != change.owner_records_before
            || change.owner_records_after != change.c17.owner_records_after()
            || cell_count != change.cell_count
            || cell_outcomes != change.cell_outcomes
            || change.owner_slots[change.owner_count..]
                .iter()
                .any(|slot| *slot != 0)
        {
            return Err(SupportLedgerError::Generation);
        }
        self.c17
            .validate_merge_initial_topology(&change.c17, current)
    }

    pub(crate) fn commit_c17_merge_initial(
        &mut self,
        change: PreparedC17MergeInitial,
    ) -> SupportLedgerGeneration {
        self.validate_c17_merge_initial(&change)
            .expect("validated C17 MergeInitial transaction");
        let expected = change.expected;
        let generation_after = change.generation_after();
        self.commit_c17_merge_initial_prevalidated(change, expected, generation_after, true)
    }

    pub(crate) fn commit_c17_merge_initial_prevalidated(
        &mut self,
        change: PreparedC17MergeInitial,
        permit_before: SupportLedgerGeneration,
        permit_after: SupportLedgerGeneration,
        apply_index_plans: bool,
    ) -> SupportLedgerGeneration {
        assert_eq!(
            self.generation, permit_before,
            "sealed MergeInitial generation"
        );
        assert_eq!(
            change.expected, permit_before,
            "prepared MergeInitial generation"
        );
        assert_eq!(
            change.generation_after, permit_after,
            "prepared MergeInitial generation after"
        );
        let PreparedC17MergeInitial {
            expected: _,
            generation_after: _,
            owner_count,
            owner_slots,
            owner_records_before: _,
            owner_records_after,
            cell_outcomes,
            cell_count,
            usage_after,
            reserved_after,
            attached_after,
            c17,
        } = change;
        self.c17
            .commit_merge_initial_topology_prevalidated(c17, apply_index_plans);
        for index in 0..owner_count {
            let RecordSlot::Occupied(record) =
                &mut self.bundles.records[owner_slots[index] as usize]
            else {
                unreachable!("validated MergeInitial owner record")
            };
            *record = owner_records_after[index].expect("MergeInitial owner after-image");
        }
        self.commit_c17_direct_cell_outcomes(&cell_outcomes[..cell_count]);
        self.usage = usage_after;
        self.reserved = reserved_after;
        self.c17.commit_attached_change(attached_after);
        self.generation = permit_after;
        self.generation
    }

    pub(crate) fn preview_c17_membership_topology(
        &self,
        expected: SupportLedgerGeneration,
        intent: &crate::request_book::c17::PreparedMembershipIntent,
    ) -> Result<c17::MembershipTopologyPreview, SupportLedgerError> {
        if expected != self.generation {
            return Err(SupportLedgerError::Generation);
        }
        self.c17.inspect_membership_topology(intent)
    }

    pub(crate) fn preview_c17_cancellation_topology(
        &self,
        expected: SupportLedgerGeneration,
        cancellation: &crate::request_book::c17::PreparedCancellation,
    ) -> Result<c17::MembershipTopologyPreview, SupportLedgerError> {
        if expected != self.generation {
            return Err(SupportLedgerError::Generation);
        }
        self.c17.inspect_cancellation_topology(cancellation)
    }

    pub(crate) fn prepare_c17_membership_topology(
        &self,
        expected: SupportLedgerGeneration,
        preview: c17::MembershipTopologyPreview,
        event: crate::request_book::c17::MembershipEventRecord,
        work: &mut impl WorkRecorder,
    ) -> Result<PreparedC17MembershipTopology, SupportLedgerError> {
        if expected != self.generation {
            return Err(SupportLedgerError::Generation);
        }
        let generation_after = expected
            .next()
            .map_err(|_| SupportLedgerError::Generation)?;
        let owner_count = preview.owner_count();
        let owner_slots = preview.owner_slots();
        if owner_count > owner_slots.len()
            || owner_slots[..owner_count]
                .iter()
                .enumerate()
                .any(|(index, slot)| owner_slots[..index].contains(slot))
            || owner_slots[owner_count..].iter().any(|slot| *slot != 0)
        {
            return Err(SupportLedgerError::InvalidInput);
        }
        let mut owner_records = [None; 4];
        for index in 0..owner_count {
            owner_records[index] = Some(
                *self
                    .bundles
                    .get_record(owner_slots[index])
                    .ok_or(SupportLedgerError::InvalidTransition)?,
            );
        }
        let (usage_after, reserved_after, attached_after) =
            self.validate_c17_aggregate_delta(preview.aggregate_delta())?;
        let vector_after = self.validate_c17_lifecycle_vector_transition(
            preview.lifecycle_before(),
            preview.lifecycle_after(),
        )?;
        let mut branch_deltas = [[0; 4]; 4];
        for (index, delta) in branch_deltas.iter_mut().enumerate().take(owner_count) {
            *delta = preview
                .owner_branch_delta(index)
                .ok_or(SupportLedgerError::InvalidInput)?;
        }
        let (cell_outcomes, cell_count) = self.prepare_c17_direct_cell_outcomes(
            owner_count,
            owner_slots,
            owner_records,
            branch_deltas,
            preview.retractions(),
        )?;
        let c17 = self
            .c17
            .prepare_membership_topology(preview, event, owner_records, work)?;
        let owner_records_after = c17.owner_records_after();
        Ok(PreparedC17MembershipTopology {
            expected,
            generation_after,
            owner_count,
            owner_slots,
            owner_records_before: owner_records,
            owner_records_after,
            cell_outcomes,
            cell_count,
            usage_after,
            reserved_after,
            attached_after,
            vector_after,
            c17,
        })
    }

    pub(crate) fn validate_c17_membership_topology(
        &self,
        change: &PreparedC17MembershipTopology,
    ) -> Result<(), SupportLedgerError> {
        if self.generation != change.expected
            || change
                .expected
                .next()
                .map_err(|_| SupportLedgerError::Generation)?
                != change.generation_after
            || change.owner_count > change.owner_slots.len()
            || self.validate_c17_aggregate_delta(change.c17.aggregate_delta())?
                != (
                    change.usage_after,
                    change.reserved_after,
                    change.attached_after,
                )
            || self.validate_c17_lifecycle_vector_transition(
                change.c17.lifecycle_before(),
                change.c17.lifecycle_after(),
            )? != change.vector_after
        {
            return Err(SupportLedgerError::Generation);
        }
        let mut current = [None; 4];
        let mut branch_deltas = [[0; 4]; 4];
        for (index, delta) in branch_deltas
            .iter_mut()
            .enumerate()
            .take(change.owner_count)
        {
            if change.owner_slots[..index].contains(&change.owner_slots[index]) {
                return Err(SupportLedgerError::Generation);
            }
            current[index] = Some(
                *self
                    .bundles
                    .get_record(change.owner_slots[index])
                    .ok_or(SupportLedgerError::Generation)?,
            );
            *delta = change
                .c17
                .owner_branch_delta(index)
                .ok_or(SupportLedgerError::Generation)?;
        }
        let (cell_outcomes, cell_count) = self.prepare_c17_direct_cell_outcomes(
            change.owner_count,
            change.owner_slots,
            current,
            branch_deltas,
            change.c17.retractions(),
        )?;
        if current != change.owner_records_before
            || change.owner_records_after != change.c17.owner_records_after()
            || cell_count != change.cell_count
            || cell_outcomes != change.cell_outcomes
            || change.owner_slots[change.owner_count..]
                .iter()
                .any(|slot| *slot != 0)
        {
            return Err(SupportLedgerError::Generation);
        }
        self.c17.validate_membership_topology(&change.c17, current)
    }

    pub(crate) fn commit_c17_membership_topology(
        &mut self,
        change: PreparedC17MembershipTopology,
    ) -> SupportLedgerGeneration {
        self.validate_c17_membership_topology(&change)
            .expect("validated C17 membership topology transaction");
        let expected = change.expected;
        let generation_after = change.generation_after();
        self.commit_c17_membership_topology_prevalidated(change, expected, generation_after, true)
    }

    pub(crate) fn commit_c17_membership_topology_prevalidated(
        &mut self,
        change: PreparedC17MembershipTopology,
        permit_before: SupportLedgerGeneration,
        permit_after: SupportLedgerGeneration,
        apply_index_plans: bool,
    ) -> SupportLedgerGeneration {
        assert_eq!(
            self.generation, permit_before,
            "sealed membership-topology generation"
        );
        assert_eq!(
            change.expected, permit_before,
            "prepared membership-topology generation"
        );
        assert_eq!(
            change.generation_after, permit_after,
            "prepared membership-topology generation after"
        );
        let PreparedC17MembershipTopology {
            expected: _,
            generation_after: _,
            owner_count,
            owner_slots,
            owner_records_before: _,
            owner_records_after,
            cell_outcomes,
            cell_count,
            usage_after,
            reserved_after,
            attached_after,
            vector_after,
            c17,
        } = change;
        self.c17
            .commit_membership_topology_prevalidated(c17, apply_index_plans);
        for index in 0..owner_count {
            let RecordSlot::Occupied(record) =
                &mut self.bundles.records[owner_slots[index] as usize]
            else {
                unreachable!("validated topology owner record")
            };
            *record = owner_records_after[index].expect("topology owner after-image");
        }
        self.commit_c17_direct_cell_outcomes(&cell_outcomes[..cell_count]);
        self.usage = usage_after;
        self.reserved = reserved_after;
        self.c17.commit_attached_change(attached_after);
        for (axis, row) in vector_after.into_iter().enumerate() {
            for (horizon, value) in row.into_iter().take(H).enumerate() {
                self.vector_usage[axis][horizon] = value;
            }
        }
        self.generation = permit_after;
        self.generation
    }

    pub(crate) fn prepare_c17_plan_disposition(
        &self,
        expected: SupportLedgerGeneration,
        identity: crate::TurnPlanIdentity,
        disposition: c17::PlanDisposition,
        occurred_at: MonotonicTime,
        work: &mut impl WorkRecorder,
    ) -> Result<PreparedC17RootBatch, SupportLedgerError> {
        if expected != self.generation {
            return Err(SupportLedgerError::Generation);
        }
        let preview = self.c17.inspect_plan_disposition(
            plan_authority_key(identity.id.get()),
            encode_plan_identity(identity),
            disposition,
            occurred_at.as_micros(),
        )?;
        self.prepare_c17_root_preview(expected, preview, work)
    }

    pub(crate) fn prepare_c17_observation_resolution(
        &self,
        expected: SupportLedgerGeneration,
        identity: crate::TurnPlanIdentity,
        resolution: c17::ObservationResolution,
        occurred_at: MonotonicTime,
        work: &mut impl WorkRecorder,
    ) -> Result<PreparedC17RootBatch, SupportLedgerError> {
        if expected != self.generation {
            return Err(SupportLedgerError::Generation);
        }
        let preview = self.c17.inspect_observation_resolution(
            plan_authority_key(identity.id.get()),
            encode_plan_identity(identity),
            resolution,
            occurred_at.as_micros(),
        )?;
        self.prepare_c17_root_preview(expected, preview, work)
    }

    pub(crate) fn prepare_c17_plan_root_action(
        &self,
        expected: SupportLedgerGeneration,
        identity: crate::TurnPlanIdentity,
        branch: u8,
        action: c17::RootAction,
        occurred_at: MonotonicTime,
        work: &mut impl WorkRecorder,
    ) -> Result<PreparedC17RootBatch, SupportLedgerError> {
        if expected != self.generation {
            return Err(SupportLedgerError::Generation);
        }
        let authority = plan_authority_key(identity.id.get());
        let anchor =
            self.c17
                .plan_root_anchor(authority, encode_plan_identity(identity), branch)?;
        let preview = self
            .c17
            .inspect_root_action(anchor, action, occurred_at.as_micros())?;
        self.prepare_c17_root_preview(expected, preview, work)
    }

    pub(crate) fn prepare_c17_typed_close(
        &self,
        expected: SupportLedgerGeneration,
        input: crate::TypedCloseInput,
        work: &mut impl WorkRecorder,
    ) -> Result<PreparedC17RootBatch, SupportLedgerError> {
        if expected != self.generation {
            return Err(SupportLedgerError::Generation);
        }
        let preview = self.c17.inspect_typed_close(input)?;
        self.prepare_c17_root_preview(expected, preview, work)
    }

    pub(crate) fn prepare_c17_membership_root_action(
        &self,
        expected: SupportLedgerGeneration,
        anchor: crate::request_book::c17::SupportMembershipAnchor,
        action: c17::RootAction,
        occurred_at: MonotonicTime,
        work: &mut impl WorkRecorder,
    ) -> Result<PreparedC17RootBatch, SupportLedgerError> {
        if expected != self.generation {
            return Err(SupportLedgerError::Generation);
        }
        let preview =
            self.c17
                .inspect_membership_root_action(anchor, action, occurred_at.as_micros())?;
        self.prepare_c17_root_preview(expected, preview, work)
    }

    pub(crate) fn prepare_c17_root_action(
        &self,
        expected: SupportLedgerGeneration,
        anchor: c17::RootAnchor,
        action: c17::RootAction,
        occurred_at: MonotonicTime,
        work: &mut impl WorkRecorder,
    ) -> Result<PreparedC17RootBatch, SupportLedgerError> {
        if expected != self.generation {
            return Err(SupportLedgerError::Generation);
        }
        let preview = self
            .c17
            .inspect_root_action(anchor, action, occurred_at.as_micros())?;
        self.prepare_c17_root_preview(expected, preview, work)
    }

    fn prepare_c17_root_preview(
        &self,
        expected: SupportLedgerGeneration,
        preview: c17::RootBatchPreview,
        work: &mut impl WorkRecorder,
    ) -> Result<PreparedC17RootBatch, SupportLedgerError> {
        let generation_after = expected
            .next()
            .map_err(|_| SupportLedgerError::Generation)?;
        let owner_count = preview.owner_count();
        let owner_slots = preview.owner_slots();
        if owner_count > owner_slots.len()
            || owner_slots[..owner_count]
                .iter()
                .enumerate()
                .any(|(index, slot)| owner_slots[..index].contains(slot))
            || owner_slots[owner_count..].iter().any(|slot| *slot != 0)
        {
            return Err(SupportLedgerError::InvalidInput);
        }
        let mut owner_records = [None; 4];
        let mut branch_deltas = [[0; 4]; 4];
        for index in 0..owner_count {
            owner_records[index] = Some(
                *self
                    .bundles
                    .get_record(owner_slots[index])
                    .ok_or(SupportLedgerError::InvalidTransition)?,
            );
            branch_deltas[index] = preview
                .owner_branch_delta(index)
                .ok_or(SupportLedgerError::InvalidInput)?;
        }
        let (usage_after, reserved_after, attached_after) =
            self.validate_c17_aggregate_delta(preview.aggregate_delta())?;
        let vector_after = self.validate_c17_lifecycle_vector_transition(
            preview.lifecycle_before(),
            preview.lifecycle_after(),
        )?;
        let (cell_outcomes, cell_count) = self.prepare_c17_direct_cell_outcomes(
            owner_count,
            owner_slots,
            owner_records,
            branch_deltas,
            preview.retractions(),
        )?;
        let expected_c17 = self.c17.generation();
        let c17 = self.c17.prepare_root_batch(preview, owner_records, work)?;
        let owner_records_after = c17.owner_records_after();
        Ok(PreparedC17RootBatch {
            expected,
            expected_c17,
            generation_after,
            owner_count,
            owner_slots,
            owner_records_before: owner_records,
            owner_records_after,
            cell_outcomes,
            cell_count,
            usage_after,
            reserved_after,
            attached_after,
            vector_after,
            c17,
        })
    }

    pub(crate) fn validate_c17_root_batch(
        &self,
        change: &PreparedC17RootBatch,
    ) -> Result<(), SupportLedgerError> {
        if self.generation != change.expected
            || self.c17.generation() != change.expected_c17
            || change
                .expected
                .next()
                .map_err(|_| SupportLedgerError::Generation)?
                != change.generation_after
            || change.owner_count > change.owner_slots.len()
            || self.validate_c17_aggregate_delta(change.c17.aggregate_delta())?
                != (
                    change.usage_after,
                    change.reserved_after,
                    change.attached_after,
                )
            || self.validate_c17_lifecycle_vector_transition(
                change.c17.lifecycle_before(),
                change.c17.lifecycle_after(),
            )? != change.vector_after
        {
            return Err(SupportLedgerError::Generation);
        }
        let mut current = [None; 4];
        let mut branch_deltas = [[0; 4]; 4];
        for (index, delta) in branch_deltas
            .iter_mut()
            .enumerate()
            .take(change.owner_count)
        {
            if change.owner_slots[..index].contains(&change.owner_slots[index]) {
                return Err(SupportLedgerError::Generation);
            }
            current[index] = Some(
                *self
                    .bundles
                    .get_record(change.owner_slots[index])
                    .ok_or(SupportLedgerError::Generation)?,
            );
            *delta = change
                .c17
                .owner_branch_delta(index)
                .ok_or(SupportLedgerError::Generation)?;
        }
        let (cell_outcomes, cell_count) = self.prepare_c17_direct_cell_outcomes(
            change.owner_count,
            change.owner_slots,
            current,
            branch_deltas,
            change.c17.retractions(),
        )?;
        if current != change.owner_records_before
            || change.owner_records_after != change.c17.owner_records_after()
            || cell_count != change.cell_count
            || cell_outcomes != change.cell_outcomes
            || change.owner_slots[change.owner_count..]
                .iter()
                .any(|slot| *slot != 0)
        {
            return Err(SupportLedgerError::Generation);
        }
        self.c17.validate_root_batch(&change.c17, current)
    }

    pub(crate) fn commit_c17_root_batch(
        &mut self,
        change: PreparedC17RootBatch,
    ) -> SupportLedgerGeneration {
        self.commit_c17_root_batch_prevalidated(change, true)
    }

    pub(crate) fn commit_c17_root_batch_prevalidated(
        &mut self,
        change: PreparedC17RootBatch,
        apply_index_plans: bool,
    ) -> SupportLedgerGeneration {
        assert_eq!(
            self.generation, change.expected,
            "sealed C17 root-batch generation"
        );
        assert_eq!(
            self.c17.generation(),
            change.expected_c17,
            "sealed C17 root-batch inner generation"
        );
        let PreparedC17RootBatch {
            expected: _,
            expected_c17: _,
            generation_after,
            owner_count,
            owner_slots,
            owner_records_before: _,
            owner_records_after,
            cell_outcomes,
            cell_count,
            usage_after,
            reserved_after,
            attached_after,
            vector_after,
            c17,
        } = change;
        self.c17
            .commit_root_batch_prevalidated(c17, apply_index_plans);
        for index in 0..owner_count {
            let RecordSlot::Occupied(record) =
                &mut self.bundles.records[owner_slots[index] as usize]
            else {
                unreachable!("sealed semantic owner destination")
            };
            *record = owner_records_after[index].expect("sealed semantic owner after-image");
        }
        self.commit_c17_direct_cell_outcomes(&cell_outcomes[..cell_count]);
        self.usage = usage_after;
        self.reserved = reserved_after;
        self.c17.commit_attached_change(attached_after);
        for (axis, row) in vector_after.into_iter().enumerate() {
            for (horizon, value) in row.into_iter().take(H).enumerate() {
                self.vector_usage[axis][horizon] = value;
            }
        }
        self.generation = generation_after;
        self.generation
    }

    pub(crate) fn prepare_c17_lifecycle_begin(
        &self,
        expected: SupportLedgerGeneration,
        total: usize,
        aggregate: c17::LifecycleAggregate,
        work: &mut WorkMeter,
    ) -> Result<PreparedC17LifecycleBegin, SupportLedgerError> {
        if expected != self.generation {
            return Err(SupportLedgerError::Generation);
        }
        expected
            .next()
            .map_err(|_| SupportLedgerError::Generation)?;
        self.validate_c17_lifecycle_aggregate(aggregate)?;
        let c17 = self.c17.prepare_begin_batch(total, aggregate, expected)?;
        self.c17.validate_begin_batch(&c17)?;
        work.charge(HotPathWorkWitness::new(
            crate::c17_layout::WORK_LIFECYCLE_BEGIN,
        ))?;
        Ok(PreparedC17LifecycleBegin { expected, c17 })
    }

    pub(crate) fn validate_c17_lifecycle_begin(
        &self,
        change: &PreparedC17LifecycleBegin,
    ) -> Result<(), SupportLedgerError> {
        if self.generation != change.expected {
            return Err(SupportLedgerError::Generation);
        }
        self.c17.validate_begin_batch(&change.c17)
    }

    pub(crate) fn commit_c17_lifecycle_begin(&mut self, change: PreparedC17LifecycleBegin) {
        self.c17.commit_begin_batch(change.c17);
    }

    pub(crate) fn c17_lifecycle_plan_anchor(
        &self,
        identity: crate::TurnPlanIdentity,
        branch: crate::PlanBranch,
    ) -> Result<c17::RootAnchor, SupportLedgerError> {
        self.c17.plan_root_anchor(
            plan_authority_key(identity.id.get()),
            encode_plan_identity(identity),
            branch.ordinal(),
        )
    }

    pub(crate) fn c17_lifecycle_membership_anchor(
        &self,
        anchor: crate::request_book::c17::SupportMembershipAnchor,
    ) -> Result<c17::RootAnchor, SupportLedgerError> {
        self.c17.current_membership_root_anchor(anchor)
    }

    pub(crate) fn c17_lifecycle_stage_start(&self) -> Result<usize, SupportLedgerError> {
        self.c17.next_lifecycle_ordinal()
    }

    pub(crate) fn bind_c17_lifecycle_record_spec(
        &self,
        anchor: c17::RootAnchor,
        ordinal: usize,
        spec: crate::core::C17LifecycleRecordSpec,
    ) -> Result<c17::LifecycleRecordInput, SupportLedgerError> {
        self.c17.bind_lifecycle_record_spec(anchor, ordinal, spec)
    }

    pub(crate) fn prepare_c17_lifecycle_stage(
        &self,
        records: &[c17::LifecycleRecordInput],
        work: &mut WorkMeter,
    ) -> Result<c17::PreparedLifecycleStage, SupportLedgerError> {
        let change = self.c17.prepare_stage_chunk(records)?;
        self.c17.validate_stage_chunk(&change)?;
        work.charge(HotPathWorkWitness::new(
            crate::c17_layout::WORK_LIFECYCLE_STAGE,
        ))?;
        Ok(change)
    }

    pub(crate) fn validate_c17_lifecycle_stage(
        &self,
        change: &c17::PreparedLifecycleStage,
    ) -> Result<(), SupportLedgerError> {
        self.c17.validate_stage_chunk(change)
    }

    pub(crate) fn commit_c17_lifecycle_stage(&mut self, change: c17::PreparedLifecycleStage) {
        self.c17.commit_stage_chunk(change);
    }

    pub(crate) fn prepare_c17_lifecycle_finalize(
        &self,
        work: &mut WorkMeter,
    ) -> Result<PreparedC17LifecycleFinalize, SupportLedgerError> {
        let expected = self.generation;
        let generation_after = expected
            .next()
            .map_err(|_| SupportLedgerError::Generation)?;
        let c17 = self.c17.prepare_finalize_batch(expected)?;
        let mut c17_owner_outcomes =
            [c17::LifecycleOwnerOutcome::ZERO; crate::c17_layout::LIFECYCLE_CAPACITY];
        let mut c17_funder_outcomes =
            [c17::LifecycleFunderOutcome::ZERO; crate::c17_layout::LIFECYCLE_CAPACITY];
        let (c17_owner_count, c17_funder_count) = self.c17.prepare_finalize_owner_outcomes(
            &c17,
            &mut c17_owner_outcomes,
            &mut c17_funder_outcomes,
        )?;
        let (usage_after, reserved_after, attached_after, vector_after) =
            self.validate_c17_lifecycle_aggregate(c17.aggregate())?;
        let mut owner_outcomes =
            [C17LifecycleOwnerOutcome::ZERO; crate::c17_layout::LIFECYCLE_CAPACITY];
        let mut cell_outcomes = [C17LifecycleCellOutcome::ZERO; c17::LIFECYCLE_PUBLICATION_MAX];
        let (owner_count, cell_count) =
            self.prepare_c17_lifecycle_publications(&c17, &mut owner_outcomes, &mut cell_outcomes)?;
        work.charge(HotPathWorkWitness::new(
            crate::c17_layout::WORK_LIFECYCLE_FINALIZE,
        ))?;
        Ok(PreparedC17LifecycleFinalize {
            expected,
            generation_after,
            usage_after,
            reserved_after,
            attached_after,
            vector_after,
            owner_outcomes,
            owner_count,
            cell_outcomes,
            cell_count,
            c17_owner_outcomes,
            c17_owner_count,
            c17_funder_outcomes,
            c17_funder_count,
            c17,
        })
    }

    pub(crate) fn validate_c17_lifecycle_finalize(
        &self,
        change: &PreparedC17LifecycleFinalize,
    ) -> Result<(), SupportLedgerError> {
        self.c17.validate_finalize_batch(&change.c17)?;
        self.c17.validate_finalize_owner_outcomes(
            &change.c17,
            &change.c17_owner_outcomes,
            change.c17_owner_count,
            &change.c17_funder_outcomes,
            change.c17_funder_count,
        )?;
        let aggregate_after = self.validate_c17_lifecycle_aggregate(change.c17.aggregate())?;
        let generation_after = change
            .expected
            .next()
            .map_err(|_| SupportLedgerError::Generation)?;
        self.validate_c17_lifecycle_publications(
            &change.c17,
            &change.owner_outcomes,
            change.owner_count,
            &change.cell_outcomes,
            change.cell_count,
        )?;
        if self.generation != change.expected
            || generation_after != change.generation_after
            || aggregate_after
                != (
                    change.usage_after,
                    change.reserved_after,
                    change.attached_after,
                    change.vector_after,
                )
        {
            return Err(SupportLedgerError::Generation);
        }
        Ok(())
    }

    pub(crate) fn commit_c17_lifecycle_finalize(
        &mut self,
        change: PreparedC17LifecycleFinalize,
    ) -> SupportLedgerGeneration {
        self.c17.commit_finalize_records(&change.c17);
        self.commit_c17_lifecycle_publications(
            &change.owner_outcomes[..change.owner_count],
            &change.cell_outcomes[..change.cell_count],
        );
        self.c17.commit_finalize_owner_sets(
            &change.c17,
            &change.c17_owner_outcomes[..change.c17_owner_count],
            &change.c17_funder_outcomes[..change.c17_funder_count],
        );
        self.usage = change.usage_after;
        self.reserved = change.reserved_after;
        self.c17.commit_attached_change(change.attached_after);
        for (axis, row) in change.vector_after.into_iter().enumerate() {
            for (horizon, value) in row.into_iter().enumerate() {
                self.vector_usage[axis][horizon] = value;
            }
        }
        self.generation = change.generation_after;
        self.c17.complete_finalize_batch(change.c17);
        self.generation
    }

    pub(crate) fn prepare_c17_lifecycle_abort(
        &self,
        work: &mut WorkMeter,
    ) -> Result<c17::PreparedLifecycleAbort, SupportLedgerError> {
        let change = self.c17.prepare_abort_chunk()?;
        self.c17.validate_abort_chunk(&change)?;
        work.charge(HotPathWorkWitness::new(
            crate::c17_layout::WORK_LIFECYCLE_ABORT,
        ))?;
        Ok(change)
    }

    pub(crate) fn validate_c17_lifecycle_abort(
        &self,
        change: &c17::PreparedLifecycleAbort,
    ) -> Result<(), SupportLedgerError> {
        self.c17.validate_abort_chunk(change)
    }

    pub(crate) fn commit_c17_lifecycle_abort(
        &mut self,
        change: c17::PreparedLifecycleAbort,
    ) -> bool {
        self.c17.commit_abort_chunk(change)
    }

    fn lifecycle_publication_cell(
        &self,
        owner_slot: u32,
        record: &BundleRecord,
        axis: usize,
        horizon: usize,
    ) -> Result<u32, SupportLedgerError> {
        if axis >= 21 || horizon >= H {
            return Err(SupportLedgerError::Storage(FixedStorageError::NonCanonical));
        }
        let expected_horizon = self
            .starts
            .bounds(axis)
            .and_then(|bounds| bounds.get(horizon))
            .map(|bound| bound.0)
            .ok_or(SupportLedgerError::Storage(FixedStorageError::NonCanonical))?;
        let len = usize::try_from(record.vector_len)
            .map_err(|_| SupportLedgerError::Storage(FixedStorageError::NonCanonical))?;
        self.bundles
            .validate_owner_chain_precharged(record.vector_head, len, owner_slot)?;
        let mut next = record.vector_head;
        let mut found = None;
        for _ in 0..len {
            let index = next;
            let CellSlot::Occupied {
                owner_record,
                cell,
                current,
                next_owned,
            } = self.bundles.cells.slots[index as usize]
            else {
                return Err(SupportLedgerError::Storage(FixedStorageError::NonCanonical));
            };
            if owner_record != owner_slot || current > cell.max_outstanding {
                return Err(SupportLedgerError::Storage(FixedStorageError::NonCanonical));
            }
            let cell_axis = cell.operation as usize * POOLS + cell.pool as usize;
            if cell_axis == axis && cell.horizon == expected_horizon {
                if found.replace(index).is_some() {
                    return Err(SupportLedgerError::Storage(FixedStorageError::NonCanonical));
                }
            }
            next = next_owned;
        }
        found.ok_or(SupportLedgerError::InvalidTransition)
    }

    fn validate_c17_lifecycle_vector_transition(
        &self,
        before: c17::LifecycleAggregate,
        after: c17::LifecycleAggregate,
    ) -> Result<[[u64; 3]; 21], SupportLedgerError> {
        if (before != c17::LifecycleAggregate::ZERO || after != c17::LifecycleAggregate::ZERO)
            && H != 3
        {
            return Err(SupportLedgerError::InvalidInput);
        }
        let mut vector_after = [[0; 3]; 21];
        for axis in 0..21 {
            for horizon in 0..H.min(3) {
                vector_after[axis][horizon] = self.vector_usage[axis][horizon]
                    .checked_sub(before.vector[axis][horizon])
                    .and_then(|value| value.checked_add(after.vector[axis][horizon]))
                    .ok_or(SupportLedgerError::InvalidTransition)?;
                if vector_after[axis][horizon] > self.vector_capacity[axis][horizon] {
                    return Err(CAPACITY_ERROR);
                }
            }
        }
        Ok(vector_after)
    }

    fn validate_c17_lifecycle_retractions(
        &self,
        retractions: &[c17::LifecyclePublication],
    ) -> Result<(), SupportLedgerError> {
        for (index, publication) in retractions.iter().copied().enumerate() {
            let owner_slot = publication.owner_slot();
            let record = self
                .bundles
                .get_record(owner_slot)
                .ok_or(SupportLedgerError::InvalidTransition)?;
            if !matches!(
                record.state,
                BundleState::LivePristine | BundleState::LiveConsumed
            ) {
                return Err(SupportLedgerError::InvalidTransition);
            }
            self.c17
                .validate_lifecycle_publication_record(publication, record)?;
            let owner_delta = u32::try_from(
                retractions[..index]
                    .iter()
                    .filter(|candidate| candidate.owner_slot() == owner_slot)
                    .count()
                    + 1,
            )
            .map_err(|_| CAPACITY_ERROR)?;
            record
                .linked_claims
                .checked_sub(owner_delta)
                .ok_or(SupportLedgerError::InvalidTransition)?;
            let funder_delta = u64::try_from(
                retractions[..index]
                    .iter()
                    .filter(|candidate| candidate.funder() == publication.funder())
                    .count()
                    + 1,
            )
            .map_err(|_| CAPACITY_ERROR)?;
            let funder = self.c17.funder_image(publication.funder())?;
            u64::from_le_bytes(
                funder[112..120]
                    .try_into()
                    .expect("fixed C17 Funder current"),
            )
            .checked_sub(funder_delta)
            .ok_or(SupportLedgerError::InvalidTransition)?;
            let cell = self.lifecycle_publication_cell(
                owner_slot,
                record,
                publication.axis(),
                publication.horizon(),
            )?;
            let CellSlot::Occupied { current, .. } = self.bundles.cells.slots[cell as usize] else {
                return Err(SupportLedgerError::Storage(FixedStorageError::NonCanonical));
            };
            let cell_delta = u64::try_from(
                retractions[..index]
                    .iter()
                    .filter(|candidate| {
                        candidate.owner_slot() == owner_slot
                            && candidate.axis() == publication.axis()
                            && candidate.horizon() == publication.horizon()
                    })
                    .count()
                    + 1,
            )
            .map_err(|_| CAPACITY_ERROR)?;
            current
                .checked_sub(cell_delta)
                .ok_or(SupportLedgerError::InvalidTransition)?;
        }
        Ok(())
    }

    fn c17_lifecycle_publication_destination(
        &self,
        publication: c17::LifecyclePublication,
    ) -> Result<(u32, u32, u32, u64, u64), SupportLedgerError> {
        let owner_slot = publication.owner_slot();
        let record = self
            .bundles
            .get_record(owner_slot)
            .ok_or(SupportLedgerError::InvalidTransition)?;
        if !matches!(
            record.state,
            BundleState::LivePristine | BundleState::LiveConsumed
        ) {
            return Err(SupportLedgerError::InvalidTransition);
        }
        self.c17
            .validate_lifecycle_publication_record(publication, record)?;
        let cell_slot = self.lifecycle_publication_cell(
            owner_slot,
            record,
            publication.axis(),
            publication.horizon(),
        )?;
        let CellSlot::Occupied { cell, current, .. } = self.bundles.cells.slots[cell_slot as usize]
        else {
            unreachable!("validated lifecycle publication cell")
        };
        Ok((
            owner_slot,
            record.linked_claims,
            cell_slot,
            current,
            cell.max_outstanding,
        ))
    }

    fn prepare_c17_lifecycle_publications(
        &self,
        change: &c17::PreparedLifecycleFinalize,
        owner_outcomes: &mut [C17LifecycleOwnerOutcome; crate::c17_layout::LIFECYCLE_CAPACITY],
        cell_outcomes: &mut [C17LifecycleCellOutcome; c17::LIFECYCLE_PUBLICATION_MAX],
    ) -> Result<(usize, usize), SupportLedgerError> {
        let mut owner_count = 0usize;
        let mut cell_count = 0usize;
        self.c17
            .visit_finalize_publications(change, &mut |publication| {
                let (owner_slot, owner_before, cell_slot, cell_before, cell_max) =
                    self.c17_lifecycle_publication_destination(publication)?;
                if let Some(index) = owner_outcomes[..owner_count]
                    .iter()
                    .position(|outcome| outcome.owner_slot == owner_slot)
                {
                    owner_outcomes[index].linked_after = owner_outcomes[index]
                        .linked_after
                        .checked_add(1)
                        .ok_or(CAPACITY_ERROR)?;
                } else {
                    if owner_count == owner_outcomes.len() {
                        return Err(CAPACITY_ERROR);
                    }
                    owner_outcomes[owner_count] = C17LifecycleOwnerOutcome {
                        owner_slot,
                        linked_after: owner_before.checked_add(1).ok_or(CAPACITY_ERROR)?,
                    };
                    owner_count += 1;
                }
                if let Some(index) = cell_outcomes[..cell_count]
                    .iter()
                    .position(|outcome| outcome.cell_slot() == cell_slot)
                {
                    if cell_outcomes[index]
                        .increment()
                        .filter(|after| *after <= cell_max)
                        .is_none()
                    {
                        return Err(CAPACITY_ERROR);
                    }
                } else {
                    if cell_count == cell_outcomes.len() {
                        return Err(CAPACITY_ERROR);
                    }
                    let current_after = cell_before.checked_add(1).ok_or(CAPACITY_ERROR)?;
                    if current_after > cell_max {
                        return Err(CAPACITY_ERROR);
                    }
                    cell_outcomes[cell_count] =
                        C17LifecycleCellOutcome::new(cell_slot, current_after);
                    cell_count += 1;
                }
                Ok(())
            })?;
        Ok((owner_count, cell_count))
    }

    fn validate_c17_lifecycle_publications(
        &self,
        change: &c17::PreparedLifecycleFinalize,
        owner_outcomes: &[C17LifecycleOwnerOutcome; crate::c17_layout::LIFECYCLE_CAPACITY],
        owner_count: usize,
        cell_outcomes: &[C17LifecycleCellOutcome; c17::LIFECYCLE_PUBLICATION_MAX],
        cell_count: usize,
    ) -> Result<(), SupportLedgerError> {
        if owner_count > owner_outcomes.len()
            || cell_count > cell_outcomes.len()
            || owner_outcomes[owner_count..]
                .iter()
                .any(|outcome| *outcome != C17LifecycleOwnerOutcome::ZERO)
            || cell_outcomes[cell_count..]
                .iter()
                .any(|outcome| *outcome != C17LifecycleCellOutcome::ZERO)
            || (0..owner_count).any(|index| {
                owner_outcomes[..index]
                    .iter()
                    .any(|prior| prior.owner_slot == owner_outcomes[index].owner_slot)
            })
            || (0..cell_count).any(|index| {
                cell_outcomes[..index]
                    .iter()
                    .any(|prior| prior.cell_slot() == cell_outcomes[index].cell_slot())
            })
        {
            return Err(SupportLedgerError::Generation);
        }
        let mut owner_seen = [0u16; crate::c17_layout::LIFECYCLE_CAPACITY];
        let mut cell_seen = [0u16; c17::LIFECYCLE_PUBLICATION_MAX];
        self.c17
            .visit_finalize_publications(change, &mut |publication| {
                let (owner_slot, _, cell_slot, _, _) =
                    self.c17_lifecycle_publication_destination(publication)?;
                let owner_index = owner_outcomes[..owner_count]
                    .iter()
                    .position(|outcome| outcome.owner_slot == owner_slot)
                    .ok_or(SupportLedgerError::Generation)?;
                owner_seen[owner_index] = owner_seen[owner_index]
                    .checked_add(1)
                    .ok_or(CAPACITY_ERROR)?;
                let cell_index = cell_outcomes[..cell_count]
                    .iter()
                    .position(|outcome| outcome.cell_slot() == cell_slot)
                    .ok_or(SupportLedgerError::Generation)?;
                cell_seen[cell_index] =
                    cell_seen[cell_index].checked_add(1).ok_or(CAPACITY_ERROR)?;
                Ok(())
            })?;
        for (index, outcome) in owner_outcomes[..owner_count].iter().enumerate() {
            let record = self
                .bundles
                .get_record(outcome.owner_slot)
                .ok_or(SupportLedgerError::Generation)?;
            if owner_seen[index] == 0
                || record
                    .linked_claims
                    .checked_add(u32::from(owner_seen[index]))
                    != Some(outcome.linked_after)
            {
                return Err(SupportLedgerError::Generation);
            }
        }
        for (index, outcome) in cell_outcomes[..cell_count].iter().copied().enumerate() {
            let CellSlot::Occupied { cell, current, .. } =
                self.bundles.cells.slots[outcome.cell_slot() as usize]
            else {
                return Err(SupportLedgerError::Generation);
            };
            if cell_seen[index] == 0
                || current
                    .checked_add(u64::from(cell_seen[index]))
                    .filter(|after| *after <= cell.max_outstanding)
                    != Some(outcome.current_after())
            {
                return Err(SupportLedgerError::Generation);
            }
        }
        Ok(())
    }

    fn commit_c17_lifecycle_publications(
        &mut self,
        owner_outcomes: &[C17LifecycleOwnerOutcome],
        cell_outcomes: &[C17LifecycleCellOutcome],
    ) {
        for outcome in owner_outcomes {
            let RecordSlot::Occupied(record) =
                &mut self.bundles.records[outcome.owner_slot as usize]
            else {
                unreachable!("validated lifecycle owner record")
            };
            record.linked_claims = outcome.linked_after;
        }
        for outcome in cell_outcomes.iter().copied() {
            let CellSlot::Occupied { current, .. } =
                &mut self.bundles.cells.slots[outcome.cell_slot() as usize]
            else {
                unreachable!("validated lifecycle publication cell")
            };
            *current = outcome.current_after();
        }
    }

    fn validate_c17_lifecycle_aggregate(
        &self,
        aggregate: c17::LifecycleAggregate,
    ) -> Result<
        (
            [[u32; POOLS]; 5],
            [[u32; POOLS]; 5],
            [[u32; POOLS]; 4],
            [[u64; 3]; 21],
        ),
        SupportLedgerError,
    > {
        if H != 3 || aggregate == c17::LifecycleAggregate::ZERO {
            return Err(SupportLedgerError::InvalidInput);
        }
        if let Some(withheld) = self.c17.pending_lifecycle_aggregate()?
            && withheld != aggregate
        {
            return Err(SupportLedgerError::Generation);
        }
        let mut usage_after = self.usage;
        let mut reserved_after = self.reserved;
        let mut attached_after = [[0; POOLS]; 4];
        for (class, row) in attached_after.iter_mut().enumerate() {
            for (pool, value) in row.iter_mut().enumerate() {
                *value = self.c17.attached(class, pool)?;
            }
        }
        for pool in 0..POOLS {
            let mut state_usage = 0u32;
            let mut state_reserved = 0u32;
            let mut state_attached = 0u32;
            for class in 0..=ACTIVE {
                if aggregate.reserved[class][pool]
                    != aggregate.usage[class][pool]
                        .checked_add(aggregate.attached[class][pool])
                        .ok_or(SupportLedgerError::Storage(FixedStorageError::Capacity))?
                {
                    return Err(SupportLedgerError::InvalidInput);
                }
                state_usage = state_usage
                    .checked_add(aggregate.usage[class][pool])
                    .ok_or(SupportLedgerError::Storage(FixedStorageError::Capacity))?;
                state_reserved = state_reserved
                    .checked_add(aggregate.reserved[class][pool])
                    .ok_or(SupportLedgerError::Storage(FixedStorageError::Capacity))?;
                state_attached = state_attached
                    .checked_add(aggregate.attached[class][pool])
                    .ok_or(SupportLedgerError::Storage(FixedStorageError::Capacity))?;
            }
            if aggregate.usage[CREDITS][pool] != state_usage
                || aggregate.reserved[CREDITS][pool] != state_reserved
                || aggregate.attached[CREDITS][pool] != state_attached
                || aggregate.usage[CLAIMS][pool] != state_reserved
                || aggregate.reserved[CLAIMS][pool] != state_reserved
            {
                return Err(SupportLedgerError::InvalidInput);
            }
        }
        let claims = aggregate.usage[CLAIMS]
            .iter()
            .try_fold(0u64, |total, value| total.checked_add(u64::from(*value)))
            .ok_or(SupportLedgerError::Storage(FixedStorageError::Capacity))?;
        let vector_delta = aggregate
            .vector
            .iter()
            .flatten()
            .try_fold(0u64, |total, value| total.checked_add(*value));
        if vector_delta != Some(claims) {
            return Err(SupportLedgerError::InvalidInput);
        }
        for class in 0..5 {
            for pool in 0..POOLS {
                usage_after[class][pool] = usage_after[class][pool]
                    .checked_add(aggregate.usage[class][pool])
                    .ok_or(SupportLedgerError::Storage(FixedStorageError::Capacity))?;
                reserved_after[class][pool] = reserved_after[class][pool]
                    .checked_sub(aggregate.reserved[class][pool])
                    .ok_or(SupportLedgerError::InvalidTransition)?;
                if class < 4 {
                    attached_after[class][pool] = attached_after[class][pool]
                        .checked_add(aggregate.attached[class][pool])
                        .ok_or(SupportLedgerError::Storage(FixedStorageError::Capacity))?;
                    let occupied = usage_after[class][pool]
                        .checked_add(reserved_after[class][pool])
                        .and_then(|value| value.checked_add(attached_after[class][pool]))
                        .ok_or(SupportLedgerError::Storage(FixedStorageError::Capacity))?;
                    if occupied > self.capacities[class][pool] {
                        return Err(SupportLedgerError::Storage(FixedStorageError::Capacity));
                    }
                } else if usage_after[class][pool]
                    .checked_add(reserved_after[class][pool])
                    .is_none_or(|occupied| occupied > self.capacities[class][pool])
                {
                    return Err(SupportLedgerError::Storage(FixedStorageError::Capacity));
                }
            }
        }
        let mut vector_after = [[0; 3]; 21];
        for axis in 0..21 {
            for horizon in 0..3 {
                vector_after[axis][horizon] = self.vector_usage[axis][horizon]
                    .checked_add(aggregate.vector[axis][horizon])
                    .ok_or(SupportLedgerError::Storage(FixedStorageError::Capacity))?;
                if vector_after[axis][horizon] > self.vector_capacity[axis][horizon] {
                    return Err(SupportLedgerError::Storage(FixedStorageError::Capacity));
                }
            }
        }
        Ok((usage_after, reserved_after, attached_after, vector_after))
    }

    fn validate_c17_aggregate_delta(
        &self,
        delta: c17::AggregateDelta,
    ) -> Result<([[u32; POOLS]; 5], [[u32; POOLS]; 5], [[u32; POOLS]; 4]), SupportLedgerError> {
        let mut usage_after = self.usage;
        let mut reserved_after = self.reserved;
        let mut attached_after = [[0; POOLS]; 4];
        let withheld = self.pending_lifecycle_aggregate()?;
        for class in 0..5 {
            for pool in 0..POOLS {
                usage_after[class][pool] =
                    apply_signed_u32(self.usage[class][pool], delta.usage[class][pool])?;
                reserved_after[class][pool] =
                    apply_signed_u32(self.reserved[class][pool], delta.reserved[class][pool])?;
                let attached = if class < 4 {
                    let value = apply_signed_u32(
                        self.c17.attached(class, pool)?,
                        delta.attached[class][pool],
                    )?;
                    attached_after[class][pool] = value;
                    u64::from(value)
                } else {
                    0
                };
                if reserved_after[class][pool] < withheld.reserved[class][pool] {
                    return Err(CAPACITY_ERROR);
                }
                let occupied = u64::from(usage_after[class][pool])
                    .checked_add(u64::from(reserved_after[class][pool]))
                    .and_then(|value| value.checked_add(attached))
                    .ok_or(CAPACITY_ERROR)?;
                if occupied > u64::from(self.capacities[class][pool]) {
                    return Err(CAPACITY_ERROR);
                }
            }
        }
        Ok((usage_after, reserved_after, attached_after))
    }

    fn find_funding_owner_precharged(
        &self,
        funding: crate::PlanMemberFunding,
        expected_bound_set: Option<RuntimeOverheadBoundSetId>,
    ) -> Result<(u32, BundleRecord, [u64; 4]), SupportLedgerError> {
        let (entitlement_leaf, entitlement_owner) = self
            .bundles
            .route_precharged(TAG_ENTITLEMENT, &funding.entitlement.get())?;
        let (vector_leaf, vector_owner) = self
            .bundles
            .route_precharged(TAG_VECTOR, &funding.credit_vector.get())?;
        let owner = entitlement_owner
            .filter(|owner| Some(*owner) == vector_owner)
            .ok_or(SupportLedgerError::InvalidTransition)?;
        let (entitlement_record, entitlement_ordinal) = self
            .bundles
            .identities
            .leaf(entitlement_leaf)
            .ok_or_else(|| SupportLedgerError::Storage(FixedStorageError::NonCanonical))?;
        let (vector_record, vector_ordinal) = self
            .bundles
            .identities
            .leaf(vector_leaf)
            .ok_or_else(|| SupportLedgerError::Storage(FixedStorageError::NonCanonical))?;
        let record = *self
            .bundles
            .get_record(owner)
            .ok_or(SupportLedgerError::InvalidTransition)?;
        if entitlement_record != owner
            || vector_record != owner
            || entitlement_ordinal != 9
            || vector_ordinal != 10
            || record.request_owner != funding.request_id
            || record.entitlement != funding.entitlement
            || record.vector != funding.credit_vector
            || expected_bound_set.is_some_and(|bound_set| record.bound_set != bound_set)
            || !matches!(
                record.state,
                BundleState::LivePristine | BundleState::LiveConsumed
            )
        {
            return Err(SupportLedgerError::InvalidTransition);
        }
        self.bundles.validate_owner_chain_precharged(
            record.vector_head,
            record.vector_len as usize,
            owner,
        )?;
        let mut branch_limits = [u64::MAX; 4];
        let mut found = [false; 4];
        let mut next = record.vector_head;
        for _ in 0..record.vector_len {
            let CellSlot::Occupied {
                owner_record,
                cell,
                current,
                next_owned,
            } = self.bundles.cells.slots[next as usize]
            else {
                return Err(SupportLedgerError::Storage(FixedStorageError::NonCanonical));
            };
            if owner_record != owner || current > cell.max_outstanding {
                return Err(SupportLedgerError::Storage(FixedStorageError::NonCanonical));
            }
            for branch in 0..4 {
                let requirement = record.branches[branch];
                if cell.operation == requirement.operation && cell.pool == requirement.pool {
                    found[branch] = true;
                    branch_limits[branch] = branch_limits[branch].min(cell.max_outstanding);
                }
            }
            next = next_owned;
        }
        if !found.into_iter().all(|present| present) {
            return Err(SupportLedgerError::InvalidTransition);
        }
        Ok((owner, record, branch_limits))
    }

    fn validate_plan_materialization(
        &self,
        member_count: usize,
    ) -> Result<([[u32; POOLS]; 5], [[u32; POOLS]; 5], [[u32; POOLS]; 4]), SupportLedgerError> {
        let pool = SupportPool::MandatoryCompletion as usize;
        let members = u32::try_from(member_count).map_err(|_| SupportLedgerError::InvalidInput)?;
        let claims = members
            .checked_mul(3)
            .ok_or(SupportLedgerError::InvalidInput)?;
        for class in [CONDITIONAL, CREDITS, CLAIMS] {
            if self.spendable_reserved(class, pool)? < claims {
                return Err(CAPACITY_ERROR);
            }
        }
        let mut usage_after = self.usage;
        let mut reserved_after = self.reserved;
        usage_after[CONDITIONAL][pool] = usage_after[CONDITIONAL][pool]
            .checked_add(3)
            .ok_or(CAPACITY_ERROR)?;
        usage_after[CREDITS][pool] = usage_after[CREDITS][pool]
            .checked_add(3)
            .ok_or(CAPACITY_ERROR)?;
        usage_after[CLAIMS][pool] = usage_after[CLAIMS][pool]
            .checked_add(claims)
            .ok_or(CAPACITY_ERROR)?;
        for class in [CONDITIONAL, CREDITS, CLAIMS] {
            reserved_after[class][pool] = reserved_after[class][pool]
                .checked_sub(claims)
                .ok_or(CAPACITY_ERROR)?;
        }
        let attached = members
            .checked_sub(1)
            .and_then(|value| value.checked_mul(3))
            .ok_or(SupportLedgerError::InvalidInput)?;
        let mut delta = [[0i32; POOLS]; 4];
        delta[CONDITIONAL][pool] = attached as i32;
        delta[CREDITS][pool] = attached as i32;
        let attached_after = self.c17.validate_attached_change(delta)?;
        for (class, attached_class) in [(CONDITIONAL, CONDITIONAL), (CREDITS, CREDITS)] {
            let total = usage_after[class][pool]
                .checked_add(reserved_after[class][pool])
                .and_then(|value| value.checked_add(attached_after[attached_class][pool]))
                .ok_or(CAPACITY_ERROR)?;
            if total > self.capacities[class][pool] {
                return Err(CAPACITY_ERROR);
            }
        }
        let claims_total = usage_after[CLAIMS][pool]
            .checked_add(reserved_after[CLAIMS][pool])
            .ok_or(CAPACITY_ERROR)?;
        if claims_total > self.capacities[CLAIMS][pool] {
            return Err(CAPACITY_ERROR);
        }
        Ok((usage_after, reserved_after, attached_after))
    }

    pub fn reserve(
        &mut self,
        expected: SupportLedgerGeneration,
        spec: SupportObligationSpec<'_>,
        external_work: &mut WorkMeter,
    ) -> Result<SupportLedgerGeneration, SupportLedgerError> {
        let mut census = ExactWorkCensus::new();
        let work = &mut census;
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
            check!(work, self.available(class, pool, added)?, CAPACITY_ERROR)?;
        }
        for identity in [key(0, spec.id.get()), key(1, spec.physical_credit.get())] {
            let absent = self.records.find_with(identity, work)?.is_none();
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
        let c17 = self.c17.prepare_legacy_insert(
            self.records.next_slot(),
            spec.id.get(),
            spec.physical_credit.get(),
        )?;
        external_work.charge(migrated_legacy_witness(work.witness())?)?;
        let keys = [key(0, spec.id.get()), key(1, spec.physical_credit.get())];
        let record = (
            spec.operation,
            spec.pool,
            spec.predecessor,
            Conditional,
            Default::default(),
            SupportCallScopeId([0; 32]),
            RecordMetadata::ordinary(spec.physical_credit),
        );
        self.records.push_prevalidated(keys, record, spec.claims);
        self.c17.commit_legacy_insert(c17);
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
        _work: &mut WorkMeter,
    ) -> Result<SupportChange, SupportLedgerError> {
        let mut census = ExactWorkCensus::new();
        let prepared = match input {
            SupportChangeInput::BeginOrdinary(spec, at) => {
                self.prepare_begin(expected, spec, at, &mut census)
            }
            SupportChangeInput::BeginPending(id, kind, at) => {
                self.prepare_pending(expected, id, kind, at, &mut census)
            }
            SupportChangeInput::FinishActive(id, terminal_at) => {
                self.prepare_finish(expected, id, terminal_at, &mut census)
            }
        };
        let mut change = match prepared {
            Ok(change) => change,
            Err(error) => {
                _work.charge(census.witness())?;
                return Err(error);
            }
        };
        change.c17 = match &change.delta {
            SupportDelta::BeginOrdinary(spec, ..) => {
                Some(LegacyC17Change::Insert(self.c17.prepare_legacy_insert(
                    change.records,
                    spec.id.get(),
                    spec.physical_credit.get(),
                )?))
            }
            SupportDelta::BeginPending(index, record, id, ..) => {
                Some(LegacyC17Change::Update(self.c17.prepare_legacy_update(
                    *index,
                    id.get(),
                    record.6.physical_credit.get(),
                    false,
                )?))
            }
            SupportDelta::FinishActive(index, record, id, _) => {
                Some(LegacyC17Change::Update(self.c17.prepare_legacy_update(
                    *index,
                    id.get(),
                    record.6.physical_credit.get(),
                    true,
                )?))
            }
            SupportDelta::FinishInitial(index, ..) => {
                let record = self
                    .bundles
                    .get_record(*index)
                    .ok_or(SupportLedgerError::InvalidTransition)?;
                Some(LegacyC17Change::C16Touch(
                    self.c17.prepare_c16_touch(*index, record)?,
                ))
            }
        };
        let base = match &change.delta {
            SupportDelta::BeginOrdinary(..) => HotPathWorkWitness::new([74, 308, 0, 0, 22]),
            SupportDelta::BeginPending(..) => HotPathWorkWitness::new([34, 185, 0, 0, 9]),
            SupportDelta::FinishActive(..) => HotPathWorkWitness::new([31, 177, 0, 0, 4]),
            SupportDelta::FinishInitial(..) => census.witness(),
        };
        change.charge = Some(migrated_legacy_witness(base)?);
        Ok(change)
    }
    fn prepare_begin<W: WorkRecorder + ?Sized>(
        &self,
        expected: SupportLedgerGeneration,
        spec: OrdinarySupportSpec,
        at: MonotonicTime,
        work: &mut W,
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
            check!(work, self.available(class, pool, 1)?, CAPACITY_ERROR)?;
        }
        for identity in [key(0, spec.id.get()), key(1, spec.physical_credit.get())] {
            let absent = self.records.find_with(identity, work)?.is_none();
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
            charge: None,
            c17: None,
        })
    }
    fn prepare_finish<W: WorkRecorder + ?Sized>(
        &self,
        expected: SupportLedgerGeneration,
        id: SupportOperationObligationId,
        terminal_at: MonotonicTime,
        work: &mut W,
    ) -> Result<SupportChange, SupportLedgerError> {
        self.next(expected, work)?;
        self.c18.check_floor(terminal_at)?;
        let delta = match self.find_obligation(id, work)? {
            ObligationOwner::Legacy { index, record } => {
                check!(
                    work,
                    record.3 == Active,
                    SupportLedgerError::InvalidTransition
                )?;
                SupportDelta::FinishActive(index, record, id, terminal_at)
            }
            ObligationOwner::InitialBundle { record, ordinal } => {
                let bundle = *self
                    .bundles
                    .get_record(record)
                    .ok_or(SupportLedgerError::InvalidTransition)?;
                let item = *bundle
                    .initial
                    .get(usize::from(ordinal))
                    .ok_or(SupportLedgerError::InvalidTransition)?;
                if item.state != Active {
                    return Err(SupportLedgerError::InvalidTransition);
                }
                SupportDelta::FinishInitial(record, ordinal, item, bundle.state, terminal_at)
            }
        };
        Ok(SupportChange {
            expected,
            records: self.records.len(),
            delta,
            charge: None,
            c17: None,
        })
    }
    #[rustfmt::skip]
    fn prepare_pending<W: WorkRecorder + ?Sized>(&self, expected: SupportLedgerGeneration, id: SupportOperationObligationId, kind: LifecycleReserveKind, at: MonotonicTime, work: &mut W) -> Result<SupportChange, SupportLedgerError> { self.next(expected, work)?; let (index, record) = self.find_record(id, work)?; let reserve_kind = record.6.lifecycle_kind.ok_or(SupportLedgerError::InvalidTransition)?; check!(work, record.3 == Pending && reserve_kind == kind && at >= record.4 && self.spendable_reserved(ACTIVE, record.1 as usize)? > 0, SupportLedgerError::InvalidTransition)?; let start = self.starts.prepare_start(record.0 as usize * POOLS + record.1 as usize, at, work)?; Ok(SupportChange { expected, records: self.records.len(), delta: SupportDelta::BeginPending(index, record, id, at, start), charge: None, c17: None }) }
    pub(crate) fn validate(&self, change: &SupportChange) -> Result<(), SupportLedgerError> {
        let target = match &change.delta {
            SupportDelta::BeginOrdinary(..) => true,
            SupportDelta::BeginPending(index, record, ..) => {
                self.records.get(*index) == Some(record)
            }
            SupportDelta::FinishActive(index, record, _, _) => {
                self.records.get(*index) == Some(record)
            }
            SupportDelta::FinishInitial(index, ordinal, item, state, _) => {
                self.bundles.get_record(*index).is_some_and(|bundle| {
                    bundle.initial.get(usize::from(*ordinal)) == Some(item)
                        && bundle.state == *state
                })
            }
        };
        if self.generation != change.expected
            || self.records.len() != change.records
            || !target
            || change.charge.is_none()
        {
            return Err(SupportLedgerError::Generation);
        }
        match (&change.delta, &change.c17) {
            (SupportDelta::FinishInitial(..), Some(LegacyC17Change::C16Touch(capability))) => {
                self.c17.validate_c16_touch(capability)
            }
            (SupportDelta::FinishInitial(..), _) => Err(SupportLedgerError::Generation),
            (_, Some(LegacyC17Change::Insert(capability))) => {
                self.c17.validate_legacy_insert(capability)
            }
            (_, Some(LegacyC17Change::Update(capability))) => {
                self.c17.validate_legacy_update(capability)
            }
            _ => Err(SupportLedgerError::Generation),
        }
    }
    pub(crate) fn commit(
        &mut self,
        change: SupportChange,
        work: &mut WorkMeter,
    ) -> Result<SupportLedgerGeneration, SupportLedgerError> {
        self.validate(&change)?;
        work.charge(change.charge.expect("validated migrated Work witness"))?;
        let SupportChange {
            expected,
            delta,
            c17,
            ..
        } = change;
        match delta {
            SupportDelta::BeginOrdinary(spec, at, start) => {
                let record = (
                    spec.operation,
                    SupportPool::Ordinary,
                    SupportCausalPredecessorId([0; 32]),
                    Active,
                    at,
                    spec.scope,
                    RecordMetadata::ordinary(spec.physical_credit),
                );
                let keys = [key(0, spec.id.get()), key(1, spec.physical_credit.get())];
                self.records.push_prevalidated(keys, record, &[spec.claim]);
                self.starts.apply_start(start);
                for class in [ACTIVE, CREDITS, CLAIMS] {
                    self.usage[class][SupportPool::Ordinary as usize] += 1;
                }
            }
            SupportDelta::FinishActive(index, record, id, terminal_at) => {
                // The record keeps its start instant; the retention boundary
                // lives in the release ticket. Without this the ordinary finish
                // path retains forever.
                let release_at =
                    c18::started_release_at(record.4, terminal_at, self.c18.limits().retention())
                        .expect("validated terminal instant");
                self.c18
                    .schedule(c18::ExpiryTicket {
                        release_at,
                        family: c18::OwnerFamily::LegacyRecord,
                        slot_index: u32::try_from(index).expect("validated record index"),
                        units: 1,
                        identity: id.get(),
                    })
                    .expect("dormant ticket reserved at creation");
                self.records
                    .get_mut(index)
                    .expect("validated support record")
                    .3 = Retained;
            }
            SupportDelta::FinishInitial(index, ordinal, item, _, terminal_at) => {
                // An initial obligation is retained with its own horizon. Its
                // storage is owned by the bundle, so the ticket records the
                // retention boundary and the bundle's own tombstone releases
                // the group.
                let release_at = c18::started_release_at(
                    item.state_time,
                    terminal_at,
                    self.c18.limits().retention(),
                )
                .expect("validated terminal instant");
                self.c18
                    .schedule(c18::ExpiryTicket {
                        release_at,
                        family: c18::OwnerFamily::InitialBundle,
                        slot_index: index * 4 + u32::from(ordinal),
                        units: 1,
                        identity: [0; 32],
                    })
                    .expect("dormant ticket reserved at creation");
                let RecordSlot::Occupied(bundle) = &mut self.bundles.records[index as usize] else {
                    unreachable!("validated initial bundle")
                };
                bundle.initial[usize::from(ordinal)].state = Retained;
                if bundle.state == BundleState::LivePristine {
                    bundle.state = BundleState::LiveConsumed;
                }
            }
            SupportDelta::BeginPending(index, record, _, at, start) => {
                self.commit_pending(index, record, at, start)
            }
        }
        match c17 {
            Some(LegacyC17Change::Insert(capability)) => self.c17.commit_legacy_insert(capability),
            Some(LegacyC17Change::Update(capability)) => self.c17.commit_legacy_update(capability),
            Some(LegacyC17Change::C16Touch(capability)) => self.c17.commit_c16_touch(capability),
            None => {}
        }
        let next = expected.next().expect("prepared support generation");
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
        external_work: &mut WorkMeter,
    ) -> Result<SupportLedgerGeneration, SupportLedgerError> {
        let mut census = ExactWorkCensus::new();
        let work = &mut census;
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
                let absent = self.records.find_with(identity, work)?.is_none();
                check!(work, absent, FixedStorageError::Duplicate)?;
            }
            self.reciprocal_absent(spec.id.get(), spec.physical_credit.get(), work)?;
            prior = (Some(spec.id), Some(spec.physical_credit));
        }
        let (pool, added) = (pool as usize, u32::from(count));
        for class in [CONDITIONAL, PENDING, ACTIVE, CREDITS, CLAIMS] {
            check!(work, self.available(class, pool, added)?, CAPACITY_ERROR)?;
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
        let first_record = self.records.next_slot();
        u32::try_from(first_record)
            .map_err(|_| SupportLedgerError::Storage(FixedStorageError::Capacity))?;
        let raw_record = |offset: usize| {
            let spec = &specs[offset];
            (
                first_record + offset,
                spec.id.get(),
                spec.physical_credit.get(),
            )
        };
        let c17 = self
            .c17
            .prepare_legacy_insert_stream(specs.len(), raw_record)?;
        external_work.charge(migrated_legacy_witness(census.witness())?)?;
        for spec in specs {
            let (operation, pool, _) = lifecycle_shape(spec.kind);
            let record = (
                operation,
                pool,
                spec.predecessor,
                Conditional,
                at,
                spec.scope,
                RecordMetadata::lifecycle(spec.kind, spec.physical_credit, count, first_record)
                    .expect("validated lifecycle record address"),
            );
            let keys = [key(0, spec.id.get()), key(1, spec.physical_credit.get())];
            self.records.push_prevalidated(keys, record, &[spec.claim]);
        }
        self.c17.commit_legacy_insert_stream(c17, raw_record);
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
        external_work: &mut WorkMeter,
    ) -> Result<SupportLedgerGeneration, SupportLedgerError> {
        let mut census = ExactWorkCensus::new();
        let work = &mut census;
        let next = self.next(expected, work)?;
        let count = u16::try_from(ids.len()).map_err(|_| SupportLedgerError::InvalidInput)?;
        check!(work, count > 0, SupportLedgerError::InvalidInput)?;
        let invalid = SupportLedgerError::InvalidTransition;
        let (trigger, required) = lifecycle_result(result);
        let pool = SupportPool::MandatoryCompletion as usize + usize::from(trigger >= 2);
        let record_bytes = std::mem::size_of::<Record>() as u64;
        let mut first_index = None;
        for (offset, id) in ids.iter().enumerate() {
            let index = self
                .records
                .find_with(key(0, id.get()), work)?
                .ok_or(invalid)?;
            work.record(WorkDimension::CopiedBytes, record_bytes)?;
            let record = *self.records.get(index).expect("indexed support record");
            let reserve_kind = record.6.lifecycle_kind.ok_or(invalid)?;
            let actual_trigger = lifecycle_shape(reserve_kind).2;
            let matching = record.3 == Conditional
                && record.2 == predecessor
                && at >= record.4
                && pool == record.1 as usize
                && index == *first_index.get_or_insert(index) + offset
                && record.6.lifecycle_count == count
                && usize::try_from(record.6.first_record).ok() == first_index
                && record.6.reserved == 0
                && (trigger == actual_trigger || trigger == 4 && actual_trigger >= 2);
            check!(work, matching, invalid)?;
        }
        let added = u32::from(count);
        let held = self.spendable_reserved(PENDING, pool)? >= added
            && self.spendable_reserved(ACTIVE, pool)? >= added;
        check!(work, held, invalid)?;
        let first_index = first_index.expect("nonempty lifecycle set");
        let records = &self.records;
        let raw_record = |offset: usize| {
            let index = first_index + offset;
            let record = records.get(index).expect("validated lifecycle record");
            (
                index,
                ids[offset].get(),
                record.6.physical_credit.get(),
                !required,
            )
        };
        let c17 = self
            .c17
            .prepare_legacy_update_stream(ids.len(), raw_record)?;
        external_work.charge(migrated_legacy_witness(census.witness())?)?;
        self.c17.commit_legacy_update_stream(c17, raw_record);
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
    pub(crate) fn lifecycle_kind(&self, id: SupportOperationObligationId, work: &mut WorkMeter) -> Result<LifecycleReserveKind, SupportLedgerError> { self.find_record(id, work)?.1.6.lifecycle_kind.ok_or(SupportLedgerError::InvalidTransition) }
    pub fn transition(
        &mut self,
        expected: SupportLedgerGeneration,
        id: SupportOperationObligationId,
        transition: SupportTransition,
        external_work: &mut WorkMeter,
    ) -> Result<SupportLedgerGeneration, SupportLedgerError> {
        let mut census = ExactWorkCensus::new();
        let next = self.next(expected, &mut census)?;
        // A committed expiry advances the ledger's time floor. A time-bearing
        // transition may not move the ledger back behind it.
        if let Some(at) = transition_instant(transition) {
            self.c18.check_floor(at)?;
        }
        match self.find_obligation(id, &mut census)? {
            ObligationOwner::Legacy { index, record } => {
                census.record(WorkDimension::InvariantChecks, 1)?;
                let generic = record.6.lifecycle_kind.is_none();
                let (state, time, terminal_at, base) = match (record.3, transition) {
                    (Conditional, PredecessorEnded(predecessor, at))
                        if predecessor == record.2 && generic =>
                    {
                        (Pending, at, None, [31, 177, 0, 0, 5])
                    }
                    (Pending, BeginSupport(at)) if at >= record.4 => {
                        (Active, at, None, [34, 185, 0, 0, 10])
                    }
                    // A started record keeps its start instant: the retention
                    // boundary lives in the release ticket, not in the record.
                    (Active, FinishSupport(terminal_at)) if terminal_at >= record.4 => {
                        (Retained, record.4, Some(terminal_at), [31, 177, 0, 0, 4])
                    }
                    (Conditional, CloseCausalCallImpossible(terminal_at)) if generic => (
                        ClosedConditional,
                        record.4,
                        Some(terminal_at),
                        [31, 177, 0, 0, 4],
                    ),
                    (Pending, CloseCausalCallImpossible(terminal_at)) => (
                        ClosedPending,
                        record.4,
                        Some(terminal_at),
                        [31, 177, 0, 0, 4],
                    ),
                    _ => return Err(SupportLedgerError::InvalidTransition),
                };
                let pool = record.1 as usize;
                let (before, after) = (state_class(record.3), state_class(state));
                if self.usage[before][pool] == 0 {
                    return Err(SupportLedgerError::Storage(FixedStorageError::NonCanonical));
                }
                if before != after {
                    let held = after == ACTIVE
                        && record.6.lifecycle_kind.is_some()
                        && self.spendable_reserved(ACTIVE, pool)? > 0;
                    check!(
                        &mut census,
                        held || self.available(after, pool, 1)?,
                        CAPACITY_ERROR
                    )?;
                }
                let start = if state == Active {
                    Some(self.starts.prepare_start(
                        record.0 as usize * POOLS + pool,
                        time,
                        &mut census,
                    )?)
                } else {
                    None
                };
                let releases_active =
                    record.6.lifecycle_kind.is_some() && matches!(state, Active | ClosedPending);
                if releases_active && self.spendable_reserved(ACTIVE, pool)? == 0 {
                    return Err(SupportLedgerError::Storage(FixedStorageError::NonCanonical));
                }
                let retained = matches!(state, Retained | ClosedConditional | ClosedPending);
                // A started group releases its record, its one physical credit
                // and every linked claim together at
                // `max(terminal_at, start_at + R_cat)`. A typed-impossible
                // close consumed no start, so its whole group is due at its
                // terminal instant.
                let release_at = match terminal_at {
                    Some(terminal) if record.3 == Active => Some(c18::started_release_at(
                        record.4,
                        terminal,
                        self.c18.limits().retention(),
                    )?),
                    Some(terminal) => Some(c18::unstarted_release_at(terminal)),
                    None => None,
                };
                let c17 = self.c17.prepare_legacy_update(
                    index,
                    id.get(),
                    record.6.physical_credit.get(),
                    retained,
                )?;
                external_work.charge(migrated_legacy_witness(HotPathWorkWitness::new(base))?)?;
                // The dormant ticket was reserved for this root at creation, so
                // scheduling cannot need new capacity. It runs before any
                // mutation so a rejection leaves the ledger byte-identical.
                if let Some(release_at) = release_at {
                    self.c18.schedule(c18::ExpiryTicket {
                        release_at,
                        family: c18::OwnerFamily::LegacyRecord,
                        slot_index: u32::try_from(index).map_err(|_| {
                            SupportLedgerError::Storage(FixedStorageError::Capacity)
                        })?,
                        units: 1,
                        identity: id.get(),
                    })?;
                }

                if before != after {
                    self.usage[before][pool] -= 1;
                    self.usage[after][pool] += 1;
                }
                if releases_active {
                    self.reserved[ACTIVE][pool] -= 1;
                }
                if let Some(start) = start {
                    self.starts.apply_start(start);
                }
                let stored = self.records.get_mut(index).expect("indexed support record");
                stored.3 = state;
                stored.4 = time;
                self.c17.commit_legacy_update(c17);
            }
            ObligationOwner::InitialBundle {
                record: record_index,
                ordinal,
            } => {
                let bundle = self
                    .bundles
                    .get_record(record_index)
                    .ok_or(SupportLedgerError::InvalidTransition)?;
                let item = *bundle
                    .initial
                    .get(usize::from(ordinal))
                    .ok_or(SupportLedgerError::InvalidTransition)?;
                if !initial_semantic_envelope_is_valid(bundle.state, ordinal, item) {
                    return Err(SupportLedgerError::InvalidTransition);
                }
                let (state, time, release_pending, release_active) = match (item.state, transition)
                {
                    (Conditional, PredecessorEnded(predecessor, at))
                        if predecessor == item.predecessor && at >= item.state_time =>
                    {
                        (Pending, at, true, false)
                    }
                    (Pending, BeginSupport(at)) if at >= item.state_time => {
                        (Active, at, false, true)
                    }
                    (Active, FinishSupport(terminal)) if terminal >= item.state_time => {
                        (Retained, item.state_time, false, false)
                    }
                    (Conditional, CloseCausalCallImpossible(_)) => {
                        (ClosedConditional, item.state_time, true, true)
                    }
                    (Pending, CloseCausalCallImpossible(_)) => {
                        (ClosedPending, item.state_time, false, true)
                    }
                    _ => return Err(SupportLedgerError::InvalidTransition),
                };
                let pool = item.pool as usize;
                if (release_pending && self.spendable_reserved(PENDING, pool)? == 0)
                    || (release_active && self.spendable_reserved(ACTIVE, pool)? == 0)
                {
                    return Err(SupportLedgerError::InvalidTransition);
                }
                let start =
                    if state == Active {
                        Some(self.starts.prepare_start_precharged(
                            item.operation as usize * POOLS + pool,
                            time,
                        )?)
                    } else {
                        None
                    };
                let before = state_class(item.state);
                let after = state_class(state);
                if self.usage[before][pool] == 0 {
                    return Err(SupportLedgerError::InvalidTransition);
                }
                let c17 = self.c17.prepare_c16_touch(record_index, bundle)?;
                external_work.charge(migrated_legacy_witness(census.witness())?)?;

                if before != after {
                    self.usage[before][pool] -= 1;
                    self.usage[after][pool] += 1;
                }
                self.reserved[PENDING][pool] -= u32::from(release_pending);
                self.reserved[ACTIVE][pool] -= u32::from(release_active);
                if let Some(start) = start {
                    self.starts.apply_start(start);
                }
                let RecordSlot::Occupied(bundle) = &mut self.bundles.records[record_index as usize]
                else {
                    unreachable!("validated initial bundle owner")
                };
                let stored = &mut bundle.initial[usize::from(ordinal)];
                stored.state = state;
                stored.state_time = time;
                if bundle.state == BundleState::LivePristine {
                    bundle.state = BundleState::LiveConsumed;
                }
                self.c17.commit_c16_touch(c17);
            }
        }
        self.generation = next;
        Ok(next)
    }
    #[allow(dead_code, reason = "C17 consumes the closed C16 claim lookup")]
    fn find_initial_claim_precharged(
        &self,
        claim: AdmissionInitialClaimId,
        expected_owner: RequestId,
    ) -> Result<(u32, u8), SupportLedgerError> {
        let invalid = SupportLedgerError::InvalidTransition;
        let (leaf, owner) = self
            .bundles
            .route_precharged(TAG_ADMISSION_CLAIM, &claim.get())?;
        let record_index = owner.ok_or(invalid)?;
        let (leaf_owner, key_ordinal) = self
            .bundles
            .identities
            .leaf(leaf)
            .ok_or(FixedStorageError::NonCanonical)?;
        let ordinal = key_ordinal.checked_sub(6).filter(|ordinal| *ordinal < 3);
        let ordinal = ordinal.ok_or(FixedStorageError::NonCanonical)?;
        let record = self.bundles.get_record(record_index).ok_or(invalid)?;
        if leaf_owner != record_index
            || record.request_owner != expected_owner
            || record.initial[usize::from(ordinal)].claim != claim
        {
            return Err(invalid);
        }
        Ok((record_index, ordinal))
    }
    fn find_obligation<W: WorkRecorder + ?Sized>(
        &self,
        id: SupportOperationObligationId,
        work: &mut W,
    ) -> Result<ObligationOwner, SupportLedgerError> {
        work.record(WorkDimension::CopiedBytes, 33)?;
        let lookup = key(TAG_OBLIGATION, id.get());
        let before = work.witness().value(WorkDimension::VisitedEntities);
        if let Some(index) = self.records.find_with(lookup, work)? {
            work.record(WorkDimension::InvariantChecks, 1)?;
            work.record(
                WorkDimension::CopiedBytes,
                std::mem::size_of::<Record>() as u64,
            )?;
            let record = *self.records.get(index).expect("indexed support record");
            return Ok(ObligationOwner::Legacy { index, record });
        }
        let visited = work
            .witness()
            .value(WorkDimension::VisitedEntities)
            .checked_sub(before)
            .ok_or(SupportLedgerError::InvalidTransition)?;
        let height = u64::from(self.records.maximum_identity_height()?);
        let padding = height
            .checked_sub(visited)
            .ok_or(SupportLedgerError::InvalidTransition)?;
        work.charge(HotPathWorkWitness::new([
            padding + u64::from(IDENTITY_BITS) + 2 + H as u64,
            32,
            0,
            0,
            u64::from(IDENTITY_BITS) + 19 + H as u64,
        ]))?;
        let (leaf, owner) = self.bundles.route_precharged(TAG_OBLIGATION, &id.get())?;
        let record = owner.ok_or(SupportLedgerError::InvalidTransition)?;
        let (leaf_owner, ordinal) = self
            .bundles
            .identities
            .leaf(leaf)
            .ok_or(SupportLedgerError::InvalidTransition)?;
        if leaf_owner != record || ordinal >= 3 {
            return Err(SupportLedgerError::InvalidTransition);
        }
        Ok(ObligationOwner::InitialBundle { record, ordinal })
    }
    fn find_record<W: WorkRecorder + ?Sized>(
        &self,
        id: SupportOperationObligationId,
        work: &mut W,
    ) -> Result<(usize, Record), SupportLedgerError> {
        work.record(WorkDimension::CopiedBytes, 33)?;
        let found = self.records.find_with(key(0, id.get()), work)?;
        work.record(WorkDimension::InvariantChecks, 1)?;
        let index = found.ok_or(SupportLedgerError::InvalidTransition)?;
        let record_bytes = std::mem::size_of::<Record>() as u64;
        work.record(WorkDimension::CopiedBytes, record_bytes)?;
        let record = *self.records.get(index).expect("indexed support record");
        Ok((index, record))
    }
    fn next<W: WorkRecorder + ?Sized>(
        &self,
        expected: SupportLedgerGeneration,
        work: &mut W,
    ) -> Result<SupportLedgerGeneration, SupportLedgerError> {
        let current = expected == self.generation;
        check!(work, current, SupportLedgerError::Generation)?;
        work.record(WorkDimension::InvariantChecks, 1)?;
        self.generation
            .next()
            .map_err(|_| SupportLedgerError::Generation)
    }
    fn pending_lifecycle_aggregate(&self) -> Result<c17::LifecycleAggregate, SupportLedgerError> {
        Ok(self
            .c17
            .pending_lifecycle_aggregate()?
            .unwrap_or(c17::LifecycleAggregate::ZERO))
    }

    fn spendable_reserved(&self, class: usize, pool: usize) -> Result<u32, SupportLedgerError> {
        let withheld = self.pending_lifecycle_aggregate()?.reserved[class][pool];
        self.reserved[class][pool]
            .checked_sub(withheld)
            .ok_or(SupportLedgerError::Storage(FixedStorageError::NonCanonical))
    }

    fn available(&self, class: usize, pool: usize, added: u32) -> Result<bool, SupportLedgerError> {
        let attached = if class < 4 {
            self.c17.attached(class, pool)?
        } else {
            0
        };
        Ok(self.usage[class][pool]
            .checked_add(self.reserved[class][pool])
            .and_then(|value| value.checked_add(attached))
            .and_then(|value| value.checked_add(added))
            .is_some_and(|value| value <= self.capacities[class][pool]))
    }
    /// Metered reciprocal absence preflight for one earlier-row insertion:
    /// both shared tagged identities must be absent from the C16
    /// request-bundle store. Live and retained-tombstone C16 leaves block
    /// later legacy reuse until pristine withdrawal or accepted C18 expiry
    /// removes them.
    fn reciprocal_absent<W: WorkRecorder + ?Sized>(
        &self,
        obligation: [u8; 32],
        credit: [u8; 32],
        work: &mut W,
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
        let mut prior_axis = None;
        for cell in cells {
            let canonical_axis = (cell.operation, cell.pool, cell.horizon.as_micros());
            if cell.max_outstanding == 0
                || cell.horizon.as_micros() == 0
                || prior_axis.is_some_and(|prior| prior >= canonical_axis)
            {
                return Err(invalid);
            }
            prior_axis = Some(canonical_axis);
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
        let withheld = self.pending_lifecycle_aggregate()?;
        for cell in cells {
            work.record(WorkDimension::VisitedEntities, H as u64 + 1)?;
            let axis = cell.operation as usize * POOLS + cell.pool as usize;
            let horizon = self
                .starts
                .bounds(axis)
                .and_then(|bounds| bounds.iter().position(|bound| bound.0 == cell.horizon));
            work.record(WorkDimension::InvariantChecks, 1)?;
            let horizon = horizon.ok_or(SupportLedgerError::InvalidInput)?;
            let updated = self.vector_usage[axis][horizon]
                .checked_add(withheld.vector[axis].get(horizon).copied().unwrap_or(0))
                .and_then(|value| value.checked_add(cell.max_outstanding));
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
                let attached = if class < 4 {
                    self.c17.attached(class, pool)?
                } else {
                    0
                };
                let valid = self.usage[class][pool]
                    .checked_add(self.reserved[class][pool])
                    .and_then(|value| value.checked_add(attached))
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
        let withheld = self.pending_lifecycle_aggregate()?;
        for cell in cells {
            let axis = cell.operation as usize * POOLS + cell.pool as usize;
            let horizon = self
                .starts
                .bounds(axis)
                .and_then(|bounds| bounds.iter().position(|bound| bound.0 == cell.horizon))
                .ok_or(SupportLedgerError::InvalidInput)?;
            let valid = self.vector_usage[axis][horizon]
                .checked_add(withheld.vector[axis].get(horizon).copied().unwrap_or(0))
                .and_then(|value| value.checked_add(cell.max_outstanding))
                .is_some_and(|value| value <= self.vector_capacity[axis][horizon]);
            if !valid {
                return Err(CAPACITY_ERROR);
            }
        }
        for class in 0..5 {
            for pool in 0..POOLS {
                let attached = if class < 4 {
                    self.c17.attached(class, pool)?
                } else {
                    0
                };
                let valid = self.usage[class][pool]
                    .checked_add(self.reserved[class][pool])
                    .and_then(|value| value.checked_add(attached))
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
    fn stored_bundle_logical_delta_precharged(
        &self,
        record_index: u32,
        record: &BundleRecord,
    ) -> Result<BundleLogicalDelta<H>, SupportLedgerError> {
        let len = usize::try_from(record.vector_len)
            .map_err(|_| SupportLedgerError::InvalidTransition)?;
        self.bundles
            .validate_owner_chain_precharged(record.vector_head, len, record_index)?;
        let mut next = record.vector_head;
        let mut delta = self.bundle_logical_delta((0..len).map(|_| {
            let CellSlot::Occupied {
                cell, next_owned, ..
            } = self.bundles.cells.slots[next as usize]
            else {
                unreachable!("prevalidated occupied bundle cell")
            };
            next = next_owned;
            cell
        }))?;
        let mandatory = MandatoryCompletion as usize;
        for item in record.initial {
            let current = state_class(item.state);
            if current != CONDITIONAL {
                delta.usage[CONDITIONAL][mandatory] -= 1;
                delta.usage[current][mandatory] += 1;
            }
            match item.state {
                Conditional => {}
                Pending => delta.reserved[PENDING][mandatory] -= 1,
                Active | Retained | ClosedConditional | ClosedPending => {
                    delta.reserved[PENDING][mandatory] -= 1;
                    delta.reserved[ACTIVE][mandatory] -= 1;
                }
            }
        }
        Ok(delta)
    }
    fn validate_stored_bundle_aggregates_precharged(
        &self,
        record_index: u32,
        record: &BundleRecord,
    ) -> Result<(), SupportLedgerError> {
        let delta = self.stored_bundle_logical_delta_precharged(record_index, record)?;
        let withheld = self.pending_lifecycle_aggregate()?;
        for class in 0..5 {
            for pool in 0..POOLS {
                let usage_after = self.usage[class][pool].checked_sub(delta.usage[class][pool]);
                let reserved_after =
                    self.reserved[class][pool].checked_sub(delta.reserved[class][pool]);
                if usage_after.is_none()
                    || reserved_after.is_none_or(|value| value < withheld.reserved[class][pool])
                {
                    return Err(SupportLedgerError::Generation);
                }
            }
        }
        for axis in 0..21 {
            for horizon in 0..H {
                if self.vector_usage[axis][horizon]
                    .checked_sub(delta.vector[axis][horizon])
                    .is_none_or(|value| {
                        value < withheld.vector[axis].get(horizon).copied().unwrap_or(0)
                    })
                {
                    return Err(SupportLedgerError::Generation);
                }
            }
        }
        Ok(())
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
        let record_slot = self.bundles.selected_record_precharged()?;
        let c17 = self
            .c17
            .prepare_c16_bundle(record_slot, &change.record, change.vector)?;
        change
            .work
            .charge(HotPathWorkWitness::new(WORK_MIGRATED_C16))?;
        Ok(ValidatedBundleChange {
            ledger: self,
            change,
            c17,
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
    /// The complete immutable C18 observation Admission consumes: the C16
    /// capacity facts plus every retention, interference, expiry and carry
    /// fact. Creating it never advances the generation and returns no Effect,
    /// so repeating it on unchanged state reproduces the same value.
    pub(crate) fn ledger_snapshot(
        &self,
        expected: SupportLedgerGeneration,
        at: MonotonicTime,
        work: &mut WorkMeter,
    ) -> Result<c18::SupportLedgerSnapshot<H>, SupportLedgerError> {
        self.observe(expected, at, work)?;
        Ok(c18::SupportLedgerSnapshot {
            capacity: self.capacity_snapshot(),
            retention: self.c18.facts(self.instance_nonce, self.generation, at),
        })
    }

    /// The complete immutable carry input the later C26 work consumes. It
    /// borrows the ledger, copies no whole state, and cannot outlive that
    /// borrow; any mutation afterwards invalidates it. C18 creates no carry
    /// token and advances no generation here.
    pub(crate) fn carry_input(
        &self,
        expected: SupportLedgerGeneration,
        at: MonotonicTime,
        work: &mut WorkMeter,
    ) -> Result<c18::SupportCarryInput<'_, H>, SupportLedgerError> {
        let snapshot = self.ledger_snapshot(expected, at, work)?;
        let (scheduled, accumulator, carry) = self.c18.views();
        let history = std::array::from_fn(|cell| self.starts.len(cell).unwrap_or(0) as u64);
        Ok(c18::SupportCarryInput::new(
            snapshot,
            scheduled,
            accumulator,
            history,
            &self.vector_usage,
            &self.reserved,
            carry,
        ))
    }

    /// Shared read-only observation preflight: exact generation, a `at` that
    /// does not move the ledger backwards, and the complete metered charge.
    fn observe(
        &self,
        expected: SupportLedgerGeneration,
        at: MonotonicTime,
        work: &mut WorkMeter,
    ) -> Result<(), SupportLedgerError> {
        if expected != self.generation {
            return Err(SupportLedgerError::Generation);
        }
        self.c18.check_floor(at)?;
        let copied = u64::try_from(H)
            .ok()
            .and_then(|horizon| horizon.checked_mul(352))
            .and_then(|bytes| bytes.checked_add(400))
            .ok_or(SupportLedgerError::InvalidInput)?;
        work.charge(HotPathWorkWitness::new([0, copied, 0, 0, 2]))?;
        Ok(())
    }

    /// Read-only, non-allocating selection of the bounded due prefix in exact
    /// `(release_at, family_tag, slot_index)` order. It never splits a release
    /// group: a group whose units do not fit stops the selection and stays
    /// fully charged. The sealed quota pair must be used exactly.
    pub(crate) fn prepare_expiry<'work, const E_GROUPS: usize, const E_UNITS: usize>(
        &self,
        expected: SupportLedgerGeneration,
        at: MonotonicTime,
        work: &'work mut WorkMeter,
    ) -> Result<c18::PreparedSupportExpiry<'work, E_GROUPS>, SupportLedgerError> {
        if expected != self.generation {
            return Err(SupportLedgerError::Generation);
        }
        let (selected, count, more_due, visited) =
            self.c18.select_expiry::<E_GROUPS, E_UNITS>(at)?;
        // One preflight covers the complete envelope, because nothing after it
        // may fail: the selection walk, the identical revalidation walk, and
        // the release of every selected group. Charging only the selection
        // would leave the revalidation, the two identity deletions per group,
        // the raw owner removal and the heap extraction unmetered.
        let height = u64::from(self.records.maximum_identity_height()?);
        let heap_depth =
            u64::from(u32::BITS - self.c18.scheduled_capacity().max(1).leading_zeros());
        let per_group = (2 * (3 * height + 1))
            .checked_add(2 * u64::from(IDENTITY_BITS))
            .and_then(|value| value.checked_add(heap_depth))
            .ok_or(SupportLedgerError::InvalidInput)?;
        let release = per_group
            .checked_mul(count as u64)
            .ok_or(SupportLedgerError::InvalidInput)?;
        work.charge(HotPathWorkWitness::new([
            2 * visited + release,
            (std::mem::size_of::<c18::ExpiryTicket>() * count) as u64,
            0,
            0,
            2 + count as u64,
        ]))?;
        Ok(c18::PreparedSupportExpiry {
            work,
            nonce: self.instance_nonce,
            expected,
            at,
            before: *self.c18.accumulator(),
            selected,
            count,
            more_due,
        })
    }

    /// Takes the sole mutable borrow and rechecks every selected root against
    /// the exact instance, generation and aggregate before-image.
    pub(crate) fn validate_expiry<'ledger, 'work, const E_GROUPS: usize>(
        &'ledger mut self,
        prepared: c18::PreparedSupportExpiry<'work, E_GROUPS>,
    ) -> Result<ValidatedSupportExpiry<'ledger, 'work, R, F, H, E_GROUPS>, SupportLedgerError> {
        if prepared.nonce != self.instance_nonce || prepared.expected != self.generation {
            return Err(SupportLedgerError::Generation);
        }
        if prepared.before != *self.c18.accumulator() {
            return Err(SupportLedgerError::Generation);
        }
        self.c18.check_floor(prepared.at)?;
        // A nonempty batch must be able to publish its next generation before
        // it mutates: exhaustion rejects here, never inside the commit. The
        // selection is a fixed array, so emptiness is its count, not its length.
        if prepared.count > 0 {
            self.generation
                .next()
                .map_err(|_| SupportLedgerError::Generation)?;
        }
        // The selection must still be the heap's exact smallest prefix, which
        // is what makes releasing it `count` minimum extractions.
        let (current, count) = self.c18.reselect::<E_GROUPS>(prepared.at);
        if count != prepared.count || current[..count] != prepared.selected[..count] {
            return Err(SupportLedgerError::Storage(FixedStorageError::NonCanonical));
        }
        Ok(ValidatedSupportExpiry {
            ledger: self,
            prepared,
        })
    }

    /// Read-only metered preparation of one pristine C16 bundle withdrawal:
    /// locates the exact live record by its entitlement, proves the record and
    /// its complete owned cell chain, and preflights the complete removal Work
    /// envelope before any mutation. Returns the non-forgeable
    /// instance-bound `PreparedWithdrawal`; dropping it changes no state. A
    /// retained terminal tombstone is not pristine and rejects.
    pub(crate) fn prepare_withdraw<'work>(
        &self,
        expected_request_owner: RequestId,
        entitlement: FutureTurnSupportEntitlementId,
        work: &'work mut WorkMeter,
    ) -> Result<PreparedWithdrawal<'work, H>, SupportLedgerError> {
        self.generation
            .next()
            .map_err(|_| SupportLedgerError::Generation)?;
        let invalid = SupportLedgerError::InvalidTransition;
        let (leaf, owner) = self
            .bundles
            .route_precharged(TAG_ENTITLEMENT, &entitlement.get())?;
        let record_index = owner.ok_or(invalid)?;
        let record = *self.bundles.get_record(record_index).ok_or(invalid)?;
        let len = usize::try_from(record.vector_len).map_err(|_| invalid)?;
        if record.entitlement != entitlement
            || len == 0
            || len > usize::from(self.bundle_vector_max)
            || self.bundles.occupied_records == 0
        {
            return Err(invalid);
        }
        let mut leaves = [NO_NODE; K];
        leaves[9] = leaf;
        Ok(PreparedWithdrawal {
            work,
            nonce: self.instance_nonce,
            snapshot: self.capacity_snapshot(),
            expected_request_owner,
            record_index,
            leaves,
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
        let stale = SupportLedgerError::Generation;
        if change.nonce != self.instance_nonce
            || change.snapshot != self.capacity_snapshot()
            || self.bundles.get_record(change.record_index) != Some(&change.record)
        {
            return Err(stale);
        }
        if !terminal_semantic_envelope_is_valid(
            &change.record,
            change.expected_request_owner,
            TerminalValidationMode::WithdrawPristine,
        ) {
            return Err(SupportLedgerError::InvalidTransition);
        }
        for (slot, key) in change.leaves.iter_mut().zip(change.record.tagged_keys()) {
            let (leaf, owner) = self.bundles.route_precharged(key.tag, &key.identity)?;
            if owner != Some(change.record_index) {
                return Err(stale);
            }
            *slot = leaf;
        }
        self.validate_stored_bundle_aggregates_precharged(change.record_index, &change.record)?;
        let c17 = self
            .c17
            .prepare_c16_withdrawal(change.record_index, &change.record)?;
        change
            .work
            .charge(HotPathWorkWitness::new(WORK_TOMBSTONE))?;
        Ok(ValidatedWithdrawal {
            ledger: self,
            change,
            c17,
        })
    }
    /// Read-only preparation of a live C16 bundle's retained terminal
    /// tombstone by its entitlement. The returned capability owns the complete
    /// before-image snapshot and the same Work-meter borrow.
    pub(crate) fn prepare_tombstone<'work>(
        &self,
        expected_request_owner: RequestId,
        entitlement: FutureTurnSupportEntitlementId,
        terminal_at: MonotonicTime,
        work: &'work mut WorkMeter,
    ) -> Result<PreparedTombstone<'work, H>, SupportLedgerError> {
        self.generation
            .next()
            .map_err(|_| SupportLedgerError::Generation)?;
        let invalid = SupportLedgerError::InvalidTransition;
        let (_, owner) = self
            .bundles
            .route_precharged(TAG_ENTITLEMENT, &entitlement.get())?;
        let record_index = owner.ok_or(invalid)?;
        let record = *self.bundles.get_record(record_index).ok_or(invalid)?;
        let len = usize::try_from(record.vector_len).map_err(|_| invalid)?;
        if record.entitlement != entitlement
            || len == 0
            || len > usize::from(self.bundle_vector_max)
            || self.bundles.occupied_records == 0
        {
            return Err(invalid);
        }
        // The retained tombstone releases at its own Catalog horizon or with
        // its last linked claim, whichever is later.
        let release_at =
            c18::tombstone_release_at(terminal_at, None, self.c18.limits().retention())?;
        Ok(PreparedTombstone {
            work,
            nonce: self.instance_nonce,
            snapshot: self.capacity_snapshot(),
            expected_request_owner,
            record_index,
            record,
            release_at,
        })
    }
    pub(crate) fn validate_tombstone<'ledger, 'work>(
        &'ledger mut self,
        change: PreparedTombstone<'work, H>,
    ) -> Result<ValidatedTombstone<'ledger, 'work, R, F, H>, SupportLedgerError> {
        let stale = SupportLedgerError::Generation;
        if change.nonce != self.instance_nonce
            || change.snapshot != self.capacity_snapshot()
            || self.bundles.get_record(change.record_index) != Some(&change.record)
        {
            return Err(stale);
        }
        if !terminal_semantic_envelope_is_valid(
            &change.record,
            change.expected_request_owner,
            TerminalValidationMode::RetainTombstone,
        ) {
            return Err(SupportLedgerError::InvalidTransition);
        }
        for key in change.record.tagged_keys() {
            let (_, owner) = self.bundles.route_precharged(key.tag, &key.identity)?;
            if owner != Some(change.record_index) {
                return Err(stale);
            }
        }
        self.validate_stored_bundle_aggregates_precharged(change.record_index, &change.record)?;
        let c17 = self
            .c17
            .prepare_c16_tombstone(change.record_index, &change.record)?;
        change
            .work
            .charge(HotPathWorkWitness::new(WORK_TOMBSTONE))?;
        Ok(ValidatedTombstone {
            ledger: self,
            change,
            c17,
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
        let ValidatedBundleChange {
            ledger,
            change,
            c17,
        } = self;
        ledger.bundles.commit_bundle(&change.record, change.vector);
        ledger.c17.commit_c16_bundle(c17);
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
        let ValidatedWithdrawal {
            ledger,
            change,
            c17,
        } = self;
        ledger.remove_stored_bundle_logical_delta(change.record_index);
        ledger
            .bundles
            .withdraw_bundle_unmetered(change.record_index);
        ledger.c17.commit_c16_withdrawal(c17);
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
        let ValidatedTombstone {
            ledger,
            change,
            c17,
        } = self;
        ledger.bundles.retain_bundle_unmetered(change.record_index);
        // The retained bundle now owns a scheduled release: without this the
        // record, its identities, cells, claims and vector stay occupied for
        // the life of the process.
        ledger
            .c18
            .schedule(c18::ExpiryTicket {
                release_at: change.release_at,
                family: c18::OwnerFamily::Tombstone,
                slot_index: change.record_index,
                units: 1,
                identity: change.record.entitlement.get(),
            })
            .expect("dormant tombstone ticket reserved at creation");
        ledger.c17.commit_c16_tombstone(c17, &change.record);
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SupportBackingCapacities {
    legacy: [usize; 6],
    history: [usize; 21],
    bundles: [usize; 8],
}

fn actual_support_storage_bytes(
    horizon_count: usize,
    capacities: SupportBackingCapacities,
) -> Option<u64> {
    let bytes = |capacity: usize, element: usize| {
        u64::try_from(capacity)
            .ok()?
            .checked_mul(u64::try_from(element).ok()?)
    };
    let fixed = u64::try_from(horizon_count)
        .ok()?
        .checked_mul(672)?
        .checked_add(1_232)?;
    let legacy = bytes(capacities.legacy[0], std::mem::size_of::<Record>())?
        .checked_add(bytes(
            capacities.legacy[1],
            std::mem::size_of::<SupportFundingClaim>(),
        )?)?
        .checked_add(bytes(capacities.legacy[2], std::mem::size_of::<u32>())?)?
        .checked_add(bytes(
            capacities.legacy[3],
            std::mem::size_of::<crate::bounded::AvlNode>(),
        )?)?;
    let history = capacities
        .history
        .into_iter()
        .try_fold(0u64, |total, capacity| {
            total.checked_add(bytes(capacity, std::mem::size_of::<MonotonicTime>())?)
        })?;
    let bundle_sizes = [
        std::mem::size_of::<RecordSlot>(),
        std::mem::size_of::<u32>(),
        std::mem::size_of::<LeafSlot>(),
        std::mem::size_of::<u32>(),
        std::mem::size_of::<BranchSlot>(),
        std::mem::size_of::<u32>(),
        std::mem::size_of::<CellSlot>(),
        std::mem::size_of::<u32>(),
    ];
    let bundles = capacities
        .bundles
        .into_iter()
        .zip(bundle_sizes)
        .try_fold(0u64, |total, (capacity, size)| {
            total.checked_add(bytes(capacity, size)?)
        })?;
    fixed
        .checked_add(legacy)?
        .checked_add(history)?
        .checked_add(bundles)
}

fn seal_backing_and_issue_nonce(
    dispenser: &AtomicU64,
    horizon_count: usize,
    expected_storage: u64,
    expected: SupportBackingCapacities,
    actual: SupportBackingCapacities,
) -> Result<u64, SupportLedgerError> {
    if actual != expected
        || actual_support_storage_bytes(horizon_count, actual) != Some(expected_storage)
        || expected_storage
            .checked_add(c17::C17_PHYSICAL_BYTES)
            .is_none_or(|whole| whole > crate::c17_generated::SUPPORT_LEDGER_CEILING_BYTES)
    {
        return Err(SupportLedgerError::Storage(FixedStorageError::Capacity));
    }
    issue_instance_nonce(dispenser).ok_or(SupportLedgerError::Storage(FixedStorageError::Capacity))
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
        .and_then(|value| c.checked_mul(52).and_then(|cells| value.checked_add(cells)))
        .ok_or(invalid)?;
    fixed
        .checked_add(legacy)
        .and_then(|value| value.checked_add(bundles))
        .ok_or(invalid)
}
fn apply_signed_u32(value: u32, delta: i32) -> Result<u32, SupportLedgerError> {
    if delta >= 0 {
        value.checked_add(delta as u32).ok_or(CAPACITY_ERROR)
    } else {
        value
            .checked_sub(delta.unsigned_abs())
            .ok_or(SupportLedgerError::Storage(FixedStorageError::NonCanonical))
    }
}

#[allow(dead_code, reason = "used by the C08 adapter constructor")]
fn total(values: impl IntoIterator<Item = u32>) -> u64 {
    values.into_iter().map(u64::from).sum()
}
const STATE_CLASSES: [usize; 6] = [CONDITIONAL, PENDING, ACTIVE, ACTIVE, CONDITIONAL, PENDING];
/// The explicit instant a transition carries, when it carries one.
fn transition_instant(transition: SupportTransition) -> Option<MonotonicTime> {
    match transition {
        PredecessorEnded(_, at)
        | BeginSupport(at)
        | FinishSupport(at)
        | CloseCausalCallImpossible(at) => Some(at),
    }
}
fn state_class(state: SupportObligationState) -> usize {
    STATE_CLASSES[state as usize]
}
#[cfg(test)]
fn state_class_index(state: usize) -> usize {
    STATE_CLASSES[state]
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
fn bundle_target_work<const H: usize>(
    prepared_base: u64,
) -> Result<HotPathWorkWitness, SupportLedgerError> {
    let invalid = SupportLedgerError::InvalidInput;
    let copied = u64::try_from(H)
        .map_err(|_| invalid)?
        .checked_mul(336)
        .and_then(|value| value.checked_add(prepared_base))
        .ok_or(invalid)?;
    Ok(HotPathWorkWitness::new([
        u64::from(IDENTITY_BITS).checked_add(3).ok_or(invalid)?,
        copied,
        0,
        0,
        u64::from(IDENTITY_BITS).checked_add(10).ok_or(invalid)?,
    ]))
}
fn bundle_mutation_bytes(cells: u64, branches: u64) -> Option<u64> {
    1_012u64
        .checked_add(K as u64 * 12)?
        .checked_add(branches.checked_mul(20)?)?
        .checked_add(cells.checked_mul(52)?)?
        .checked_add(44)?
        .checked_add(K as u64 * 4 + 4)
}
fn withdraw_remainder_work<const H: usize>(
    cells: usize,
    branches: usize,
) -> Result<HotPathWorkWitness, SupportLedgerError> {
    let invalid = SupportLedgerError::InvalidInput;
    let v = u64::try_from(cells).map_err(|_| invalid)?;
    let b = u64::try_from(branches).map_err(|_| invalid)?;
    let h = u64::try_from(H).map_err(|_| invalid)?;
    let snapshot = h
        .checked_mul(42)
        .and_then(|value| value.checked_add(45))
        .ok_or(invalid)?;
    let route = u64::from(IDENTITY_BITS).checked_add(2).ok_or(invalid)?;
    let key_routes = (K as u64).checked_mul(route).ok_or(invalid)?;
    let visits = snapshot
        .checked_add(key_routes.checked_mul(2).ok_or(invalid)?)
        .and_then(|value| value.checked_add(v.checked_mul(3)?))
        .and_then(|value| value.checked_add(K as u64 + b + 17))
        .ok_or(invalid)?;
    let copied = h
        .checked_mul(336)
        .and_then(|value| value.checked_add(1_360))
        .and_then(|value| value.checked_add(bundle_mutation_bytes(v, b)?))
        .ok_or(invalid)?;
    let checks = snapshot
        .checked_add(2)
        .and_then(|value| value.checked_add((K as u64).checked_mul(u64::from(IDENTITY_BITS) + 4)?))
        .and_then(|value| value.checked_add(v.checked_mul(7)?))
        .and_then(|value| value.checked_add(3 * (K as u64 + b)))
        .and_then(|value| value.checked_add(86))
        .ok_or(invalid)?;
    Ok(HotPathWorkWitness::new([visits, copied, 0, 0, checks]))
}
fn tombstone_remainder_work<const H: usize>(
    cells: usize,
) -> Result<HotPathWorkWitness, SupportLedgerError> {
    let invalid = SupportLedgerError::InvalidInput;
    let v = u64::try_from(cells).map_err(|_| invalid)?;
    let h = u64::try_from(H).map_err(|_| invalid)?;
    let snapshot = h
        .checked_mul(42)
        .and_then(|value| value.checked_add(45))
        .ok_or(invalid)?;
    let key_routes = (K as u64)
        .checked_mul(u64::from(IDENTITY_BITS).checked_add(2).ok_or(invalid)?)
        .ok_or(invalid)?;
    let visits = snapshot
        .checked_add(key_routes)
        .and_then(|value| value.checked_add(v + 17))
        .ok_or(invalid)?;
    let copied = h
        .checked_mul(336)
        .and_then(|value| value.checked_add(1_321))
        .ok_or(invalid)?;
    let checks = snapshot
        .checked_add(2)
        .and_then(|value| value.checked_add((K as u64).checked_mul(u64::from(IDENTITY_BITS) + 4)?))
        .and_then(|value| value.checked_add(v.checked_mul(6)?))
        .and_then(|value| value.checked_add(89))
        .ok_or(invalid)?;
    Ok(HotPathWorkWitness::new([visits, copied, 0, 0, checks]))
}
fn plan_authority_key(plan: u128) -> [u8; 17] {
    let mut key = [0; 17];
    key[0] = 0x30;
    key[1..].copy_from_slice(&plan.to_be_bytes());
    key
}

fn standalone_authority_key(domain: crate::FormationDomainId) -> [u8; 17] {
    let mut key = [0; 17];
    key[0] = 0x31;
    key[1..].copy_from_slice(&domain.get().to_be_bytes());
    key
}

fn singleton_materialization_delta() -> c17::AggregateDelta {
    let pool = SupportPool::MandatoryCompletion as usize;
    let mut delta = c17::AggregateDelta {
        usage: [[0; POOLS]; 5],
        reserved: [[0; POOLS]; 5],
        attached: [[0; POOLS]; 4],
    };
    for class in [CONDITIONAL, CREDITS, CLAIMS] {
        delta.reserved[class][pool] = -1;
        delta.usage[class][pool] = 1;
    }
    delta
}

fn encode_plan_identity(identity: crate::TurnPlanIdentity) -> [u8; c17::PLAN_IDENTITY_BYTES] {
    let mut image = [0; c17::PLAN_IDENTITY_BYTES];
    image[0..16].copy_from_slice(&identity.id.get().to_le_bytes());
    image[16..32].copy_from_slice(&identity.candidate_id.get().to_le_bytes());
    image[32..48].copy_from_slice(&identity.coordinates.model_id.get().to_le_bytes());
    image[48] = match identity.coordinates.phase {
        crate::ExecutionPhase::Prefill => 1,
        crate::ExecutionPhase::Decode => 2,
    };
    image[49] = match identity.coordinates.service_class {
        crate::ServiceClass::Interactive => 1,
        crate::ServiceClass::Standard => 2,
        crate::ServiceClass::Background => 3,
    };
    image[50..52].copy_from_slice(&identity.coordinates.batch_bucket.0.to_le_bytes());
    image[52..84].copy_from_slice(&identity.capability_key.get());
    for (ordinal, generation) in identity.generations.components().into_iter().enumerate() {
        image[84 + ordinal * 8..92 + ordinal * 8].copy_from_slice(&generation.to_le_bytes());
    }
    image[116..148].copy_from_slice(&identity.bound_set.get());
    image[148..156].copy_from_slice(
        &identity
            .budget
            .target_engine_service
            .as_micros()
            .to_le_bytes(),
    );
    image[156..164].copy_from_slice(
        &identity
            .budget
            .hard_execution_bound
            .as_micros()
            .to_le_bytes(),
    );
    image[164..196].copy_from_slice(&identity.budget.stale_disposition_bound.get());
    image[196..204].copy_from_slice(
        &identity
            .budget
            .stale_successor_ceiling
            .as_micros()
            .to_le_bytes(),
    );
    image[204..212].copy_from_slice(&identity.budget.phase_work_ceiling.get().to_le_bytes());
    image
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
    c17: c17::PreparedC16Bundle,
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
    expected_request_owner: RequestId,
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
    c17: c17::PreparedC16Withdrawal,
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
    pub(crate) release_at: MonotonicTime,
    work: &'work mut WorkMeter,
    nonce: u64,
    snapshot: SupportCapacitySnapshot<H>,
    expected_request_owner: RequestId,
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
    c17: c17::PreparedC16Tombstone,
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
        current: u64,
        next_owned: u32,
    },
}
impl EntitlementCellArena {
    fn storage_bytes(capacity: u64) -> Option<u64> {
        capacity
            .checked_mul((std::mem::size_of::<CellSlot>() + std::mem::size_of::<u32>()) as u64)
            .filter(|&total| total <= 2_097_152)
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
                current: 0,
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
                current,
                next_owned,
            } = *slot
            else {
                return Err(FixedStorageError::NonCanonical);
            };
            check!(work, owner_record == owner, FixedStorageError::NonCanonical)?;
            check!(
                work,
                cell == *expected && current <= cell.max_outstanding,
                FixedStorageError::NonCanonical
            )?;
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
                cell,
                current,
                next_owned,
            } = *slot
            else {
                return Err(FixedStorageError::NonCanonical);
            };
            check!(
                work,
                owner_record == owner && current <= cell.max_outstanding,
                FixedStorageError::NonCanonical
            )?;
            check!(
                work,
                (next_owned == NO_NODE) == (position + 1 == count),
                FixedStorageError::NonCanonical
            )?;
            index = next_owned;
        }
        Ok(())
    }
    fn validate_owner_chain_precharged(
        &self,
        head: u32,
        count: usize,
        owner: u32,
    ) -> Result<(), FixedStorageError> {
        let mut index = head;
        for position in 0..count {
            if index == NO_NODE {
                return Err(FixedStorageError::NonCanonical);
            }
            let slot = self
                .slots
                .get(index as usize)
                .ok_or(FixedStorageError::NonCanonical)?;
            let CellSlot::Occupied {
                owner_record,
                cell,
                current,
                next_owned,
            } = *slot
            else {
                return Err(FixedStorageError::NonCanonical);
            };
            if owner_record != owner
                || current > cell.max_outstanding
                || (next_owned == NO_NODE) != (position + 1 == count)
            {
                return Err(FixedStorageError::NonCanonical);
            }
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
    fn find<W: WorkRecorder + ?Sized>(
        &self,
        records: &[RecordSlot],
        tag: u8,
        identity: &[u8; 32],
        work: &mut W,
    ) -> Result<Option<u32>, FixedStorageError> {
        if self.root != NO_NODE {
            let route = u64::from(IDENTITY_BITS) + 1;
            work.ensure(HotPathWorkWitness::new([route, 0, 0, 0, route]))?;
        }
        let node = self.locate(tag, identity, work)?;
        if node == NO_NODE {
            return Ok(None);
        }
        let (key, owner) = self.resolved_leaf(node, records)?;
        Ok((key.tag == tag && key.identity == *identity).then_some(owner))
    }
    fn route_precharged(
        &self,
        records: &[RecordSlot],
        tag: u8,
        identity: &[u8; 32],
    ) -> Result<(u32, Option<u32>), FixedStorageError> {
        let mut node = self.root;
        let mut prior = None;
        let mut route_masks = [0u64; 5];
        let mut route_values = [0u64; 5];
        for _ in 0..=IDENTITY_BITS {
            if node == NO_NODE {
                return if prior.is_none() {
                    Ok((NO_NODE, None))
                } else {
                    Err(FixedStorageError::NonCanonical)
                };
            }
            if !is_branch(node) {
                let (key, owner) = self.resolved_leaf(node, records)?;
                let chunks = tagged_key_chunks(key.tag, &key.identity);
                if chunks
                    .into_iter()
                    .zip(route_masks)
                    .zip(route_values)
                    .any(|((chunk, mask), value)| chunk & mask != value)
                {
                    return Err(FixedStorageError::NonCanonical);
                }
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
            let selected = identity_bit(tag, identity, branch.bit);
            let word = usize::from(branch.bit / 64);
            let mask = 1u64 << (63 - branch.bit % 64);
            route_masks[word] |= mask;
            route_values[word] |= mask * selected as u64;
            node = [branch.zero, branch.one][selected];
        }
        Err(FixedStorageError::NonCanonical)
    }
    /// Walks to the terminal node for a borrowed key, charging one
    /// VisitedEntities and one InvariantChecks per visited branch and one of
    /// each for the visited leaf. Returns the leaf index, or `NO_NODE` for an
    /// empty tree.
    fn locate<W: WorkRecorder + ?Sized>(
        &self,
        tag: u8,
        identity: &[u8; 32],
        work: &mut W,
    ) -> Result<u32, FixedStorageError> {
        let mut node = self.root;
        let mut prior = None;
        for _ in 0..=IDENTITY_BITS {
            if node == NO_NODE {
                return if prior.is_none() {
                    Ok(NO_NODE)
                } else {
                    Err(FixedStorageError::NonCanonical)
                };
            }
            if !is_branch(node) {
                work.record(WorkDimension::VisitedEntities, 1)?;
                work.record(WorkDimension::InvariantChecks, 1)?;
                return Ok(node);
            }
            work.record(WorkDimension::VisitedEntities, 1)?;
            work.record(WorkDimension::InvariantChecks, 1)?;
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
fn tagged_key_chunks(tag: u8, identity: &[u8; 32]) -> [u64; 5] {
    let chunk = |start: usize| {
        u64::from_be_bytes(
            identity[start..start + 8]
                .try_into()
                .expect("fixed tagged-key chunk"),
        )
    };
    [
        u64::from(tag) << 56 | chunk(0) >> 8,
        u64::from(identity[7]) << 56 | chunk(8) >> 8,
        u64::from(identity[15]) << 56 | chunk(16) >> 8,
        u64::from(identity[23]) << 56 | chunk(24) >> 8,
        u64::from(identity[31]) << 56,
    ]
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
fn initial_semantic_envelope_is_valid(
    bundle_state: BundleState,
    ordinal: u8,
    item: InitialRequirementRecord,
) -> bool {
    let expected_operation = match ordinal {
        0 => SupportOperation::MaterializeRequest,
        1 => SupportOperation::FormCandidates,
        2 => SupportOperation::ReleaseRequest,
        _ => return false,
    };
    let zero_time = item.state_time == MonotonicTime::from_micros(0);
    let state_time_is_valid = match item.state {
        Conditional | ClosedConditional => zero_time,
        Pending | Active | Retained | ClosedPending => true,
    };
    let bundle_state_is_valid = match bundle_state {
        BundleState::LivePristine => item.state == Conditional && zero_time,
        BundleState::LiveConsumed => true,
        BundleState::RetainedTombstone => false,
    };
    item.obligation.get() != [0; 32]
        && item.credit.get() != [0; 32]
        && item.claim.get() != [0; 32]
        && item.operation == expected_operation
        && item.pool == MandatoryCompletion
        && item.predecessor.0 != [0; 32]
        && item.scope.0 != [0; 32]
        && item.input_bucket.get() != 0
        && item.prospective_bound.as_micros() != 0
        && state_time_is_valid
        && bundle_state_is_valid
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
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TerminalValidationMode {
    WithdrawPristine,
    RetainTombstone,
}
fn terminal_semantic_envelope_is_valid(
    record: &BundleRecord,
    expected_request_owner: RequestId,
    mode: TerminalValidationMode,
) -> bool {
    if record.request_owner != expected_request_owner {
        return false;
    }
    let ordered = |identities: [[u8; 32]; 3]| identities.windows(2).all(|pair| pair[0] < pair[1]);
    let initial_is_valid = record
        .initial
        .into_iter()
        .enumerate()
        .all(|(ordinal, item)| {
            initial_semantic_envelope_is_valid(record.state, ordinal as u8, item)
        });
    let identities_are_ordered = ordered(record.initial.map(|item| item.obligation.get()))
        && ordered(record.initial.map(|item| item.credit.get()))
        && ordered(record.initial.map(|item| item.claim.get()));
    let branch_shapes = [
        SupportOperation::ObserveTurnReceipt,
        SupportOperation::FormCandidates,
        SupportOperation::FormCandidates,
        SupportOperation::FormCandidates,
    ];
    let branches_are_valid =
        record
            .branches
            .into_iter()
            .zip(branch_shapes)
            .all(|(branch, operation)| {
                branch.operation == operation
                    && branch.pool == MandatoryCompletion
                    && branch.input_bucket.get() != 0
                    && branch.prospective_bound.as_micros() != 0
            });
    let fixed_identities_are_valid = record.timing_commitment.get() != [0; 32]
        && record.request_closure.get() != [0; 32]
        && record.support_budget.get() != [0; 32]
        && record.bound_set.get() != [0; 32]
        && record.entitlement.get() != [0; 32]
        && record.vector.get() != [0; 32];
    let terminal_state_is_valid = if mode == TerminalValidationMode::WithdrawPristine {
        record.state == BundleState::LivePristine
            && record.linked_claims == 0
            && record.initial.iter().all(|item| {
                item.state == Conditional && item.state_time == MonotonicTime::from_micros(0)
            })
    } else {
        matches!(
            record.state,
            BundleState::LivePristine | BundleState::LiveConsumed
        )
    };
    initial_is_valid
        && identities_are_ordered
        && branches_are_valid
        && fixed_identities_are_valid
        && terminal_state_is_valid
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
    fn backing_capacities(&self) -> [usize; 8] {
        [
            self.records.capacity(),
            self.free_records.capacity(),
            self.identities.leaf_slots.capacity(),
            self.identities.free_leaves.capacity(),
            self.identities.branch_slots.capacity(),
            self.identities.free_branches.capacity(),
            self.cells.slots.capacity(),
            self.cells.free.capacity(),
        ]
    }

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
    fn selected_record_precharged(&self) -> Result<u32, FixedStorageError> {
        let position = self
            .free_records
            .len()
            .checked_sub(1)
            .ok_or(FixedStorageError::Capacity)?;
        let record = self.free_records[position];
        matches!(
            self.records.get(record as usize),
            Some(RecordSlot::Vacant { free_position }) if *free_position == position as u32
        )
        .then_some(record)
        .ok_or(FixedStorageError::NonCanonical)
    }
    fn is_empty(&self) -> bool {
        self.occupied_records == 0
    }
    fn find<W: WorkRecorder + ?Sized>(
        &self,
        tag: u8,
        identity: &[u8; 32],
        work: &mut W,
    ) -> Result<Option<u32>, FixedStorageError> {
        if (self.identities.root == NO_NODE) != (self.occupied_records == 0) {
            return Err(FixedStorageError::NonCanonical);
        }
        self.identities.find(&self.records, tag, identity, work)
    }
    fn route_precharged(
        &self,
        tag: u8,
        identity: &[u8; 32],
    ) -> Result<(u32, Option<u32>), FixedStorageError> {
        if (self.identities.root == NO_NODE) != (self.occupied_records == 0) {
            return Err(FixedStorageError::NonCanonical);
        }
        self.identities
            .route_precharged(&self.records, tag, identity)
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
    fn validate_owner_chain_precharged(
        &self,
        head: u32,
        len: usize,
        owner: u32,
    ) -> Result<(), FixedStorageError> {
        self.cells.validate_owner_chain_precharged(head, len, owner)
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
    use crate::{
        AuthorizedCapabilitySet, BackendGeneration, BatchBucket, CandidateCoordinates, CandidateId,
        CandidateMember, CapabilityKey, Duration, ExecutionPhase, GenerationVector,
        HotPathWorkBudget, ModelId, PlanMemberFunding, PlanSupportObligation,
        PlanSupportObligations, RuntimeOverheadBoundSetId, RuntimeOverheadGeneration,
        SafetyGeneration, SchedulerGeneration, SchedulingSnapshot, ServiceClass, TokenCount,
        TurnBudget, TurnPlanId, WorkBudgetError, WorkCandidate,
    };
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
        Ledger::try_new(
            generation,
            capacities,
            2,
            starts,
            maxima,
            4,
            8,
            6,
            c18::SupportHistoryLimits::testing(starts),
        )
        .unwrap()
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
        let mut value = [credit; 32];
        value[31] ^= 0x80;
        let claims = [Reserved([n; 32])];
        let mut input = spec(n, credit, Ordinary, &claims);
        input.physical_credit = PhysicalStartCreditId::new(value).unwrap();
        ledger.reserve(ledger.generation(), input, &mut work())
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
            Ok(488_800)
        );
        assert_eq!(
            support_storage_bytes(3, 7_211, 1_025, 21 * 1_025, 4, 8),
            Ok(2_097_160)
        );
        assert_eq!(
            support_storage_bytes(3, 7_212, 1_025, 21 * 1_025, 4, 8),
            Ok(2_097_420)
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
    fn c17_whole_ledger_storage_accepts_exact_boundary_and_rejects_first_invalid() {
        type BoundaryLedger = SupportChargeLedger<16_531, 2_057, 3>;
        let generation = SupportLedgerGeneration::new(1).unwrap();
        let capacities = |records: u32| {
            [
                [1, 0, 0],
                [1, 0, 0],
                [1, 0, 0],
                [records, 0, 0],
                [2_057, 0, 0],
            ]
        };
        let starts = std::array::from_fn(|row| {
            let history = if row < 4 { 349 } else { 348 };
            [
                FixedStartCountBound(Duration::from_micros(1_000_000), 1),
                FixedStartCountBound(Duration::from_micros(10_000_000), 1),
                FixedStartCountBound(Duration::from_micros(20_000_000), history),
            ]
        });
        let maxima = LifecycleReserveMaxima([1; 5]);
        assert_eq!(
            support_storage_bytes(3, 16_530, 2_057, 7_312, 1_152, 6_912),
            Ok(6_372_556)
        );
        let ledger = BoundaryLedger::try_new(
            generation,
            capacities(16_530),
            1_024,
            starts,
            maxima,
            1_152,
            6_912,
            63,
            c18::SupportHistoryLimits::testing(starts),
        )
        .expect("63,942,176-byte whole ledger is valid");
        assert_eq!(ledger.capacities[CREDITS], [16_530, 0, 0]);
        assert_eq!(ledger.bundles.record_capacity(), 1_152);
        drop(ledger);

        assert_eq!(
            BoundaryLedger::try_new(
                generation,
                capacities(16_531),
                1_024,
                starts,
                maxima,
                1_152,
                6_912,
                63,
                c18::SupportHistoryLimits::testing(starts),
            )
            .unwrap_err(),
            SupportLedgerError::Storage(FixedStorageError::Capacity)
        );
    }

    #[test]
    fn c16_all_backing_allocations_remain_pointer_and_capacity_stable() {
        let c16_facts = |ledger: &Ledger| {
            let store = &ledger.bundles;
            [
                (store.records.as_ptr() as usize, store.records.capacity()),
                (
                    store.free_records.as_ptr() as usize,
                    store.free_records.capacity(),
                ),
                (
                    store.identities.leaf_slots.as_ptr() as usize,
                    store.identities.leaf_slots.capacity(),
                ),
                (
                    store.identities.free_leaves.as_ptr() as usize,
                    store.identities.free_leaves.capacity(),
                ),
                (
                    store.identities.branch_slots.as_ptr() as usize,
                    store.identities.branch_slots.capacity(),
                ),
                (
                    store.identities.free_branches.as_ptr() as usize,
                    store.identities.free_branches.capacity(),
                ),
                (
                    store.cells.slots.as_ptr() as usize,
                    store.cells.slots.capacity(),
                ),
                (
                    store.cells.free.as_ptr() as usize,
                    store.cells.free.capacity(),
                ),
            ]
        };
        let mut ledger = bundle_ledger(4, 8);
        let legacy = ledger.records.allocation_facts();
        let c16 = c16_facts(&ledger);
        let stable = |ledger: &Ledger| {
            assert_eq!(ledger.records.allocation_facts(), legacy);
            assert_eq!(c16_facts(ledger), c16);
        };

        add(&mut ledger, 200, 201).unwrap();
        stable(&ledger);
        reserve_bundle(&mut ledger, 1, 3);
        stable(&ledger);
        let mut meter = work();
        let change = ledger
            .prepare_withdraw(request_owner(1), bundle_entitlement(1), &mut meter)
            .unwrap();
        ledger.validate_withdraw(change).unwrap().commit_withdraw();
        stable(&ledger);
        let obligation = reserve_bundle(&mut ledger, 2, 3);
        stable(&ledger);
        ledger
            .transition(
                ledger.generation(),
                obligation,
                CloseCausalCallImpossible(MonotonicTime::from_micros(1_000)),
                &mut work(),
            )
            .unwrap();
        stable(&ledger);
        tombstone_bundle(&mut ledger, 2);
        stable(&ledger);
    }

    #[test]
    fn c16_tombstone_recomputes_every_initial_state_contribution() {
        for terminal_state in [
            Conditional,
            Pending,
            Active,
            Retained,
            ClosedConditional,
            ClosedPending,
        ] {
            let mut ledger = bundle_ledger(4, 8);
            let cells = configured_cells(3, 1);
            let input = bundle_input(1, &cells);
            let requirement = input.initial.materialize;
            let obligation = reserve_bundle(&mut ledger, 1, 3);
            if matches!(terminal_state, Pending | Active | Retained | ClosedPending) {
                ledger
                    .transition(
                        ledger.generation(),
                        obligation,
                        PredecessorEnded(requirement.predecessor, MonotonicTime::from_micros(5)),
                        &mut work(),
                    )
                    .unwrap();
            }
            if matches!(terminal_state, Active | Retained) {
                ledger
                    .transition(
                        ledger.generation(),
                        obligation,
                        Begin(MonotonicTime::from_micros(6)),
                        &mut work(),
                    )
                    .unwrap();
            }
            if terminal_state == Retained {
                let mut meter = work();
                let change = ledger
                    .prepare(
                        ledger.generation(),
                        SupportChangeInput::FinishActive(
                            obligation,
                            MonotonicTime::from_micros(1_000),
                        ),
                        &mut meter,
                    )
                    .unwrap();
                ledger.commit(change, &mut meter).unwrap();
            }
            if matches!(terminal_state, ClosedConditional | ClosedPending) {
                ledger
                    .transition(
                        ledger.generation(),
                        obligation,
                        CloseCausalCallImpossible(MonotonicTime::from_micros(1_000)),
                        &mut work(),
                    )
                    .unwrap();
            }
            assert_eq!(
                ledger.bundles.get_record(0).unwrap().initial[0].state,
                terminal_state
            );
            let retained = (ledger.usage, ledger.reserved, ledger.vector_usage);
            tombstone_bundle(&mut ledger, 1);
            assert_eq!(
                (ledger.usage, ledger.reserved, ledger.vector_usage),
                retained,
                "tombstone preserves aggregates for {terminal_state:?}"
            );
            assert_eq!(
                ledger.bundles.get_record(0).unwrap().initial[0].state,
                terminal_state
            );
        }
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
        go(&mut ledger, 1, Finish(at(1_000))).unwrap();
        add(&mut ledger, 2, 2).unwrap();
        go(&mut ledger, 2, end(2, 5)).unwrap();
        fail(go(&mut ledger, 2, Begin(at(14))), WindowExceeded.into());
        go(&mut ledger, 2, Begin(at(15))).unwrap();
        add(&mut ledger, 3, 3).unwrap();
        go(&mut ledger, 3, end(3, 15)).unwrap();
        fail(go(&mut ledger, 3, Begin(at(25))), CAPACITY_ERROR);
        let mut ledger = generic_ledger();
        let before = ledger.generation();
        // Duplicate and reversed OrdinaryReservation claims, plus the C16-only
        // AdmissionInitial claim, reject without charging the authoritative
        // meter.
        for claims in [
            [Reserved([1; 32]); 2],
            [Reserved([2; 32]), Reserved([1; 32])],
        ] {
            let mut measured = work();
            let result = ledger.reserve(before, spec(7, 8, Ordinary, &claims), &mut measured);
            fail(result, InvalidInput);
            assert_eq!(measured.witness(), HotPathWorkWitness::new([0; 5]));
        }
        let mut measured = work();
        let result = ledger.reserve(
            before,
            spec(7, 8, Mandatory, &[Initial([7; 32])]),
            &mut measured,
        );
        fail(result, InvalidInput);
        assert_eq!(measured.witness(), HotPathWorkWitness::new([0; 5]));
        assert_eq!((ledger.generation(), ledger.records.len()), (before, 0));
        let mut measured = work();
        let valid = spec(7, 8, Ordinary, &[Reserved([7; 32])]);
        ledger.reserve(before, valid, &mut measured).unwrap();
        assert_eq!(
            measured.witness(),
            HotPathWorkWitness::new([1_134, 1_616_904, 0, 0, 1_060])
        );
        let mut ledger = generic_ledger();
        let before = ledger.generation();
        let mut exhausted = work();
        exhausted
            .record(WorkDimension::VisitedEntities, 1_704_575)
            .unwrap();
        let result = ledger.reserve(
            before,
            spec(7, 8, Ordinary, &[Reserved([7; 32])]),
            &mut exhausted,
        );
        let error =
            WorkBudgetError::BudgetExceeded(WorkDimension::VisitedEntities, 1_704_575, 1_705_709);
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
        go(&mut ledger, 1, Close(at(1_000))).unwrap();
        fail(add(&mut ledger, 2, 1), Duplicate.into());
        add(&mut ledger, 2, 2).unwrap();
        fail(add(&mut ledger, 3, 3), CAPACITY_ERROR);
        go(&mut ledger, 2, end(2, 1)).unwrap();
        go(&mut ledger, 2, Close(at(1_000))).unwrap();
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
            HotPathWorkWitness::new([1_133, 1_616_904, 0, 0, 1_062])
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
                SupportChangeInput::FinishActive(valid.id, MonotonicTime::from_micros(1_000)),
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
            HotPathWorkWitness::new([1_090, 1_616_904, 0, 0, 1_044])
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
        go(&mut ledger, 2, Close(at(1_000))).unwrap();
        assert_eq!(ledger.generation(), pending.next().unwrap().next().unwrap());
        assert_eq!(ledger.reserved[ACTIVE][1], 0);
        assert_eq!(ledger.records.get(0).unwrap().3, Active);
        assert_eq!(ledger.records.get(1).unwrap().3, ClosedPending);

        let resolved_work = HotPathWorkWitness::new([1_060, 1_616_904, 0, 0, 1_045]);
        let rejected_work = HotPathWorkWitness::new([0; 5]);
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
        let build = |maxima| {
            Wide::try_new(
                generation,
                capacities,
                2,
                starts,
                maxima,
                4,
                8,
                6,
                c18::SupportHistoryLimits::testing(starts),
            )
        };
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

    fn request_owner(n: u8) -> RequestId {
        RequestId::new(
            crate::DaemonInstanceId::new(u128::from(n)).unwrap(),
            crate::ConnectionId::new(1).unwrap(),
            crate::RequestSequence::new(1).unwrap(),
        )
    }
    /// Test-only complete bundle input over `n`-derived canonical identities.
    fn bundle_input<'a>(
        n: u8,
        cells: &'a [OutstandingCreditCell],
    ) -> RequestSupportBundleInput<'a> {
        let identity = |offset: u8| {
            let mut id = [0u8; 32];
            id[0] = n;
            id[1] = offset;
            id
        };
        let requirement = |offset: u8, operation| InitialSupportRequirement {
            obligation: SupportOperationObligationId::new(identity(offset)).unwrap(),
            credit: PhysicalStartCreditId::new(identity(offset + 10)).unwrap(),
            claim: AdmissionInitialClaimId::new(identity(offset + 20)).unwrap(),
            operation,
            pool: Mandatory,
            predecessor: SupportCausalPredecessorId(identity(offset + 50)),
            scope: SupportCallScopeId(identity(offset + 60)),
            input_bucket: SupportInputBucket::new(u16::from(offset)).unwrap(),
            prospective_bound: Duration::from_micros(u64::from(offset)),
        };
        let branch = |offset: u8, operation| FutureSupportBranchRequirement {
            operation,
            pool: Mandatory,
            input_bucket: SupportInputBucket::new(u16::from(offset)).unwrap(),
            prospective_bound: Duration::from_micros(u64::from(offset)),
        };
        RequestSupportBundleInput {
            request_owner: request_owner(n),
            timing: SupportTimingFacts {
                timing_commitment: TimingCommitmentId::new(identity(70)).unwrap(),
                request_closure: RequestClosureId::new(identity(71)).unwrap(),
                support_budget: OwnerThreadSupportBudgetId::new(identity(72)).unwrap(),
                bound_set: RuntimeOverheadBoundSetId::new(identity(73)).unwrap(),
            },
            initial: InitialSupportRequirements {
                materialize: requirement(1, SupportOperation::MaterializeRequest),
                form_candidates: requirement(2, SupportOperation::FormCandidates),
                release: requirement(3, SupportOperation::ReleaseRequest),
            },
            branches: FutureSupportBranchRequirements {
                receipt_observation: branch(1, SupportOperation::ObserveTurnReceipt),
                continuation_formation: branch(2, SupportOperation::FormCandidates),
                rejection_or_local_stale_formation: branch(3, SupportOperation::FormCandidates),
                terminal_membership_change_formation: branch(4, SupportOperation::FormCandidates),
            },
            entitlement: FutureTurnSupportEntitlementId::new(identity(31)).unwrap(),
            vector: SupportOutstandingCreditVectorId::new(identity(41)).unwrap(),
            cells,
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
            c18::SupportHistoryLimits::testing(starts),
        )
        .unwrap()
    }

    type PlanLedger = SupportChargeLedger<512, 256, 1>;
    type TopologyLedger = SupportChargeLedger<512, 256, 3>;

    fn plan_ledger() -> PlanLedger {
        let starts = [[FixedStartCountBound(Duration::from_micros(10), 64); 1]; 21];
        PlanLedger::try_new(
            SupportLedgerGeneration::new(1).unwrap(),
            [[0, 128, 0]; 5],
            4,
            starts,
            LifecycleReserveMaxima([1, 2, 2, 1, 1]),
            8,
            16,
            4,
            c18::SupportHistoryLimits::testing(starts),
        )
        .unwrap()
    }

    fn topology_ledger() -> TopologyLedger {
        let starts = std::array::from_fn(|_| {
            std::array::from_fn(|horizon| {
                FixedStartCountBound(Duration::from_micros((horizon as u64 + 1) * 10), 64)
            })
        });
        TopologyLedger::try_new(
            SupportLedgerGeneration::new(1).unwrap(),
            [[0, 128, 0]; 5],
            4,
            starts,
            LifecycleReserveMaxima([1, 2, 2, 1, 1]),
            8,
            16,
            4,
            c18::SupportHistoryLimits::testing(starts),
        )
        .unwrap()
    }

    fn plan_identity_bytes(n: u8) -> [u8; 32] {
        let mut identity = [0; 32];
        identity[0] = n;
        identity
    }

    fn reserve_plan_bundle<const RECORDS: usize, const HORIZONS: usize>(
        ledger: &mut SupportChargeLedger<RECORDS, 256, HORIZONS>,
        n: u8,
    ) -> PlanMemberFunding {
        let cells = [
            OutstandingCreditCell {
                operation: SupportOperation::FormCandidates,
                pool: Mandatory,
                horizon: Duration::from_micros(10),
                max_outstanding: 4,
            },
            OutstandingCreditCell {
                operation: SupportOperation::ObserveTurnReceipt,
                pool: Mandatory,
                horizon: Duration::from_micros(10),
                max_outstanding: 4,
            },
        ];
        let mut input = bundle_input(n, &cells);
        input.timing.bound_set = RuntimeOverheadBoundSetId::new(plan_identity_bytes(200)).unwrap();
        let funding = PlanMemberFunding {
            request_id: input.request_owner,
            entitlement: input.entitlement,
            credit_vector: input.vector,
        };
        let mut measured = work();
        let change = ledger.prepare_bundle(&input, &mut measured).unwrap();
        ledger.validate_bundle(change).unwrap().commit_bundle();
        funding
    }

    fn turn_plan(funders: &[PlanMemberFunding; 4], plan_id: u128, phase_work: u64) -> TurnPlan<4> {
        turn_plan_members(funders, plan_id, phase_work)
    }

    fn turn_plan_members(
        funders: &[PlanMemberFunding],
        plan_id: u128,
        phase_work: u64,
    ) -> TurnPlan<4> {
        let capability = CapabilityKey::new(plan_identity_bytes(201)).unwrap();
        let bound_set = RuntimeOverheadBoundSetId::new(plan_identity_bytes(200)).unwrap();
        let overhead = RuntimeOverheadGeneration::new(1).unwrap();
        let coordinates = CandidateCoordinates {
            model_id: ModelId::new(1).unwrap(),
            phase: ExecutionPhase::Decode,
            service_class: ServiceClass::Interactive,
            batch_bucket: BatchBucket(1),
        };
        let mut evidence = crate::BoundedVec::new();
        let mut eligible = crate::BoundedSet::new();
        let mut funding = crate::BoundedVec::new();
        for member in funders {
            let mut authorized = AuthorizedCapabilitySet::<1>::new();
            authorized.try_insert(capability).unwrap();
            evidence
                .try_push(CandidateMember {
                    request_id: member.request_id,
                    coordinates,
                    authorized_capabilities: authorized,
                    bound_set,
                    runtime_overhead_generation: overhead,
                })
                .unwrap();
            eligible.try_insert(member.request_id).unwrap();
            funding.try_push(*member).unwrap();
        }
        let candidate = WorkCandidate::try_new(
            CandidateId::new(1).unwrap(),
            coordinates,
            capability,
            evidence,
        )
        .unwrap();
        let mut candidates = crate::BoundedVec::new();
        candidates.try_push(candidate).unwrap();
        let generations = GenerationVector::new(
            SchedulerGeneration::new(1).unwrap(),
            BackendGeneration::new(1).unwrap(),
            SafetyGeneration::new(1).unwrap(),
            overhead,
        );
        let snapshot = SchedulingSnapshot::<4, 1, 4>::try_new(
            MonotonicTime::from_micros(1),
            generations,
            eligible,
            candidates,
            crate::BoundedVec::new(),
        )
        .unwrap();
        let base = u8::try_from(plan_id).unwrap().checked_mul(20).unwrap();
        let obligation = |n| PlanSupportObligation {
            id: SupportOperationObligationId::new(plan_identity_bytes(base + n)).unwrap(),
            physical_credit: PhysicalStartCreditId::new(plan_identity_bytes(base + n + 10))
                .unwrap(),
            funders: funding.clone(),
        };
        TurnPlan::try_new(
            TurnPlanId::new(plan_id).unwrap(),
            &snapshot,
            CandidateId::new(1).unwrap(),
            funding,
            TurnBudget {
                target_engine_service: Duration::from_micros(1),
                hard_execution_bound: Duration::from_micros(2),
                stale_disposition_bound: crate::StalePlanDispositionBoundId::new(
                    plan_identity_bytes(220),
                )
                .unwrap(),
                stale_successor_ceiling: Duration::from_micros(3),
                phase_work_ceiling: TokenCount::new(phase_work),
            },
            PlanSupportObligations {
                receipt_observation: obligation(1),
                conditional_continuation_formation: obligation(2),
                rejection_or_local_stale_formation: obligation(3),
            },
        )
        .unwrap()
    }
    type LifecyclePlanLedger = SupportChargeLedger<256, 256, 3>;

    fn lifecycle_plan_ledger() -> LifecyclePlanLedger {
        let starts = std::array::from_fn(|_| {
            [
                FixedStartCountBound(Duration::from_micros(10), 16),
                FixedStartCountBound(Duration::from_micros(20), 16),
                FixedStartCountBound(Duration::from_micros(30), 16),
            ]
        });
        LifecyclePlanLedger::try_new(
            SupportLedgerGeneration::new(1).unwrap(),
            [[4, 64, 0]; 5],
            4,
            starts,
            LifecycleReserveMaxima([4; 5]),
            8,
            16,
            4,
            c18::SupportHistoryLimits::testing(starts),
        )
        .unwrap()
    }

    fn reserve_exact_plan_bundle(ledger: &mut LifecyclePlanLedger, n: u8) -> PlanMemberFunding {
        let cells = [
            OutstandingCreditCell {
                operation: SupportOperation::FormCandidates,
                pool: Mandatory,
                horizon: Duration::from_micros(10),
                max_outstanding: 2,
            },
            OutstandingCreditCell {
                operation: SupportOperation::ObserveTurnReceipt,
                pool: Mandatory,
                horizon: Duration::from_micros(10),
                max_outstanding: 1,
            },
        ];
        let mut input = bundle_input(n, &cells);
        input.timing.bound_set = RuntimeOverheadBoundSetId::new(plan_identity_bytes(200)).unwrap();
        let funding = PlanMemberFunding {
            request_id: input.request_owner,
            entitlement: input.entitlement,
            credit_vector: input.vector,
        };
        let mut measured = work();
        let change = ledger.prepare_bundle(&input, &mut measured).unwrap();
        ledger.validate_bundle(change).unwrap().commit_bundle();
        funding
    }

    fn lifecycle_record(
        n: u8,
        class: usize,
        pool: usize,
        axis: usize,
        horizon: usize,
        amount: usize,
    ) -> c17::LifecycleRecordInput {
        assert!((1..=4).contains(&amount));
        let mut final_owner = [n; 64];
        final_owner[0] = n.max(1);
        let mut obligation_raw = [0; 32];
        obligation_raw[0] = 0xe0;
        obligation_raw[31] = n;
        let mut credit_raw = [0; 32];
        credit_raw[0] = 0xe1;
        credit_raw[31] = n;
        let mut aggregate = [0; 21];
        aggregate[..6].copy_from_slice(&[
            class as u64,
            pool as u64,
            axis as u64,
            horizon as u64,
            amount as u64,
            1,
        ]);
        let mut owners = [c17::LifecycleOwnerRow::ZERO; 4];
        for (ordinal, owner) in owners[..amount].iter_mut().enumerate() {
            let value = u64::from(n) * 8 + ordinal as u64 + 1;
            *owner = c17::LifecycleOwnerRow {
                owner: value,
                request: value,
                entitlement: value,
                vector: value,
                source: value,
                group: value,
                root: value,
                formation: value,
                link: value,
                reserve: value,
                class: class as u64,
                pool: pool as u64,
                amount: 1,
                generation: 1,
                state: 1,
                zero: 0,
            };
        }
        c17::LifecycleRecordInput {
            final_owner,
            owner_set_ref: [n.max(1); 8],
            obligation_raw,
            credit_raw,
            predecessor: [n.wrapping_add(1).max(1); 32],
            scope: [n.wrapping_add(2).max(1); 32],
            claim: [n.wrapping_add(3).max(1); 32],
            physical_credit: [n.wrapping_add(4).max(1); 32],
            kind: 1,
            occurred_at: u64::from(n.max(1)),
            expires_at: None,
            aggregate,
            owners,
        }
    }

    fn seed_lifecycle_reservation(
        ledger: &mut LifecyclePlanLedger,
        class: usize,
        pool: usize,
        amount: u32,
    ) {
        ledger.reserved[class][pool] = amount;
        ledger.reserved[CREDITS][pool] = amount;
        ledger.reserved[CLAIMS][pool] = amount;
    }

    type MaximumLifecycleLedger = SupportChargeLedger<8_192, 4_096, 3>;

    fn maximum_lifecycle_ledger() -> MaximumLifecycleLedger {
        let mut capacities = [[0; POOLS]; 5];
        for row in &mut capacities {
            row[Mandatory as usize] = 2_048;
        }
        let starts = std::array::from_fn(|_| {
            [
                FixedStartCountBound(Duration::from_micros(10), 4_096),
                FixedStartCountBound(Duration::from_micros(20), 4_096),
                FixedStartCountBound(Duration::from_micros(30), 4_096),
            ]
        });
        let mut ledger = MaximumLifecycleLedger::try_new(
            SupportLedgerGeneration::new(1).unwrap(),
            capacities,
            4,
            starts,
            LifecycleReserveMaxima([1, 1_023, 1_024, 1, 1]),
            8,
            16,
            4,
            c18::SupportHistoryLimits::testing(starts),
        )
        .unwrap();
        ledger.c17 = c17::SupportC17::try_new(c17::SupportC17Capacities::lifecycle_testing(
            crate::c17_layout::LIFECYCLE_CAPACITY,
        ))
        .unwrap();
        ledger
    }

    fn reserve_maximum_lifecycle_plan_bundle(
        ledger: &mut MaximumLifecycleLedger,
    ) -> PlanMemberFunding {
        let cells = [
            OutstandingCreditCell {
                operation: SupportOperation::FormCandidates,
                pool: Mandatory,
                horizon: Duration::from_micros(10),
                max_outstanding: 1_200,
            },
            OutstandingCreditCell {
                operation: SupportOperation::ObserveTurnReceipt,
                pool: Mandatory,
                horizon: Duration::from_micros(10),
                max_outstanding: 4,
            },
        ];
        let mut input = bundle_input(4, &cells);
        input.timing.bound_set = RuntimeOverheadBoundSetId::new(plan_identity_bytes(200)).unwrap();
        let funding = PlanMemberFunding {
            request_id: input.request_owner,
            entitlement: input.entitlement,
            credit_vector: input.vector,
        };
        let mut measured = work();
        let change = ledger.prepare_bundle(&input, &mut measured).unwrap();
        ledger.validate_bundle(change).unwrap().commit_bundle();
        funding
    }

    fn maximum_lifecycle_raw(prefix: u8, ordinal: usize) -> [u8; 32] {
        let mut raw = [0; 32];
        raw[0] = prefix;
        raw[24..].copy_from_slice(&(ordinal as u64 + 1).to_be_bytes());
        raw
    }

    fn maximum_lifecycle_spec(
        identity: crate::TurnPlanIdentity,
        ordinal: usize,
    ) -> crate::core::C17LifecycleRecordSpec {
        crate::core::C17LifecycleRecordSpec {
            root: crate::core::C17LifecycleRootSpec::Plan {
                identity,
                branch: crate::PlanBranch::Continuation,
            },
            obligation: SupportOperationObligationId::new(maximum_lifecycle_raw(0x40, ordinal))
                .unwrap(),
            credit: PhysicalStartCreditId::new(maximum_lifecycle_raw(0x80, ordinal)).unwrap(),
            predecessor: SupportCausalPredecessorId(maximum_lifecycle_raw(0x90, ordinal)),
            scope: SupportCallScopeId(maximum_lifecycle_raw(0xa0, ordinal)),
            claim: maximum_lifecycle_raw(0xb0, ordinal),
            kind: LifecycleReserveKind::PostLoadModelDescription,
            occurred_at: MonotonicTime::from_micros(ordinal as u64 + 3),
            expires_at: None,
            operation: SupportOperation::FormCandidates,
            pool: Mandatory,
            horizon: 0,
        }
    }

    fn maximum_lifecycle_aggregate(total: usize) -> c17::LifecycleAggregate {
        let axis = SupportOperation::FormCandidates as usize * POOLS + Mandatory as usize;
        let records: Vec<_> = (0..total)
            .map(|ordinal| {
                lifecycle_record(
                    (ordinal % 250 + 1) as u8,
                    CONDITIONAL,
                    Mandatory as usize,
                    axis,
                    0,
                    1,
                )
            })
            .collect();
        c17::LifecycleAggregate::from_records(&records).unwrap()
    }

    #[inline(never)]
    fn commit_maximum_lifecycle_plan(ledger: &mut MaximumLifecycleLedger, plan: &TurnPlan<4>) {
        let create = ledger
            .prepare_c17_plan_create(
                ledger.generation(),
                plan,
                MonotonicTime::from_micros(2),
                &mut work(),
            )
            .unwrap();
        ledger.commit_c17_plan_create(create);
    }

    #[inline(never)]
    fn begin_maximum_lifecycle_batch(ledger: &mut MaximumLifecycleLedger, total: usize) {
        let mut begin_work = work();
        let begin = ledger
            .prepare_c17_lifecycle_begin(
                ledger.generation(),
                total,
                maximum_lifecycle_aggregate(total),
                &mut begin_work,
            )
            .unwrap();
        assert_eq!(
            begin_work.witness(),
            HotPathWorkWitness::new(crate::c17_layout::WORK_LIFECYCLE_BEGIN)
        );
        ledger.commit_c17_lifecycle_begin(begin);
    }

    #[inline(never)]
    fn stage_maximum_lifecycle_chunk(
        ledger: &mut MaximumLifecycleLedger,
        identity: crate::TurnPlanIdentity,
        chunk_start: usize,
        len: usize,
    ) {
        let specs: Vec<_> = (chunk_start..chunk_start + len)
            .map(|ordinal| Some(maximum_lifecycle_spec(identity, ordinal)))
            .collect();
        let mut stage_work = work();
        let stage = crate::transition_coordinator::prepare_lifecycle_stage(
            None::<&crate::request_book::RequestBook<1, 1, 1, 1>>,
            ledger,
            &specs,
            &mut stage_work,
        )
        .unwrap();
        assert_eq!(
            stage_work.witness(),
            HotPathWorkWitness::new(crate::c17_layout::WORK_LIFECYCLE_STAGE)
        );
        crate::transition_coordinator::commit_lifecycle_stage(
            None::<&crate::request_book::RequestBook<1, 1, 1, 1>>,
            ledger,
            stage,
        );
    }

    #[inline(never)]
    fn stage_maximum_lifecycle_batch(
        ledger: &mut MaximumLifecycleLedger,
        identity: crate::TurnPlanIdentity,
        batch_start: usize,
        total: usize,
    ) {
        for chunk_start in (batch_start..batch_start + total).step_by(8) {
            let len = (batch_start + total - chunk_start).min(8);
            stage_maximum_lifecycle_chunk(ledger, identity, chunk_start, len);
        }
    }

    #[inline(never)]
    fn finalize_maximum_lifecycle_batch(ledger: &mut MaximumLifecycleLedger) {
        let generation_before_finalize = ledger.generation();
        let mut finalize_work = work();
        let finalize = ledger
            .prepare_c17_lifecycle_finalize(&mut finalize_work)
            .unwrap();
        assert_eq!(
            finalize_work.witness(),
            HotPathWorkWitness::new(crate::c17_layout::WORK_LIFECYCLE_FINALIZE)
        );
        let committed = ledger.commit_c17_lifecycle_finalize(finalize);
        assert_eq!(committed.get(), generation_before_finalize.get() + 1);
        assert_eq!(ledger.c17.pending_lifecycle_aggregate().unwrap(), None);
        let pending = ledger.c17.pending_header_for_test();
        assert_eq!(pending.0[0], 0);
        assert_eq!(u16::from_le_bytes(pending.0[16..18].try_into().unwrap()), 0);
        assert_eq!(u16::from_le_bytes(pending.0[18..20].try_into().unwrap()), 0);
    }

    #[inline(never)]
    fn commit_maximum_lifecycle_batch(
        ledger: &mut MaximumLifecycleLedger,
        identity: crate::TurnPlanIdentity,
        batch_start: usize,
        total: usize,
    ) {
        begin_maximum_lifecycle_batch(ledger, total);
        stage_maximum_lifecycle_batch(ledger, identity, batch_start, total);
        finalize_maximum_lifecycle_batch(ledger);
    }

    #[inline(never)]
    fn assert_maximum_lifecycle_publication(
        ledger: &MaximumLifecycleLedger,
        owner_before: u32,
        support_before: SupportLedgerGeneration,
        c17_before: u64,
    ) {
        assert_eq!(ledger.generation().get(), support_before.get() + 2);
        assert_eq!(ledger.c17.generation(), c17_before + 148);
        assert_eq!(ledger.c17.current_counts_for_test()[16], 1_152);
        assert_eq!(
            ledger.bundles.get_record(0).unwrap().linked_claims,
            owner_before + 1_152
        );
        for ordinal in [0usize, 1_023, 1_024, 1_151] {
            let image = ledger
                .c17
                .lifecycle_record_by_raw(maximum_lifecycle_raw(0x40, ordinal))
                .unwrap()
                .expect("finalized lifecycle record is directly visible");
            assert_eq!(image[0], 1);
            assert_eq!(image[96..128], maximum_lifecycle_raw(0x40, ordinal));
        }
    }

    #[inline(never)]
    fn reject_maximum_lifecycle_first_one_over(ledger: &MaximumLifecycleLedger) {
        let snapshot = (
            ledger.generation(),
            ledger.c17.generation(),
            ledger.c17.current_counts_for_test(),
            ledger.c17.pending_header_for_test(),
        );
        let mut rejected_work = work();
        assert_eq!(
            ledger
                .prepare_c17_lifecycle_begin(
                    ledger.generation(),
                    1,
                    maximum_lifecycle_aggregate(1),
                    &mut rejected_work,
                )
                .unwrap_err(),
            SupportLedgerError::Storage(FixedStorageError::Capacity)
        );
        assert_eq!(rejected_work.witness(), HotPathWorkWitness::default());
        assert_eq!(
            (
                ledger.generation(),
                ledger.c17.generation(),
                ledger.c17.current_counts_for_test(),
                ledger.c17.pending_header_for_test(),
            ),
            snapshot
        );
    }

    #[test]
    fn c17_lifecycle_1024_stage_chunks_finalize_then_fill_128_headroom() {
        let mut ledger = maximum_lifecycle_ledger();
        let funding = reserve_maximum_lifecycle_plan_bundle(&mut ledger);
        let plan = turn_plan_members(&[funding], 1, 1_200);
        commit_maximum_lifecycle_plan(&mut ledger, &plan);
        let owner_before = ledger.bundles.get_record(0).unwrap().linked_claims;
        let support_before = ledger.generation();
        let c17_before = ledger.c17.generation();
        commit_maximum_lifecycle_batch(&mut ledger, plan.identity(), 0, 1_024);
        commit_maximum_lifecycle_batch(&mut ledger, plan.identity(), 1_024, 128);
        assert_maximum_lifecycle_publication(&ledger, owner_before, support_before, c17_before);
        reject_maximum_lifecycle_first_one_over(&ledger);
    }

    #[test]
    fn c17_lifecycle_1_8_9_immediate_partial_mixed_abort_and_reuse() {
        let mut ledger = lifecycle_plan_ledger();
        let funding = reserve_plan_bundle(&mut ledger, 4);
        let plan = turn_plan_members(&[funding], 1, 16);
        let create = ledger
            .prepare_c17_plan_create(
                ledger.generation(),
                &plan,
                MonotonicTime::from_micros(2),
                &mut work(),
            )
            .unwrap();
        ledger.commit_c17_plan_create(create);
        let axis = SupportOperation::FormCandidates as usize * POOLS + Mandatory as usize;
        ledger.reserved[CONDITIONAL][Mandatory as usize] = 16;
        ledger.reserved[CREDITS][Mandatory as usize] = 16;
        ledger.reserved[CLAIMS][Mandatory as usize] = 16;
        ledger.vector_capacity[axis][0] = 16;
        let baseline_counts = ledger.c17.current_counts_for_test();

        for (total, staged, expected_abort_chunks) in [(1usize, 0usize, 1), (8, 3, 1), (9, 8, 2)] {
            let begin = ledger
                .prepare_c17_lifecycle_begin(
                    ledger.generation(),
                    total,
                    maximum_lifecycle_aggregate(total),
                    &mut work(),
                )
                .unwrap();
            ledger.commit_c17_lifecycle_begin(begin);

            for chunk_start in (0..staged).step_by(8) {
                let len = (staged - chunk_start).min(8);
                let specs: Vec<_> = (chunk_start..chunk_start + len)
                    .map(|ordinal| Some(maximum_lifecycle_spec(plan.identity(), ordinal)))
                    .collect();
                let mut stage_work = work();
                let stage = crate::transition_coordinator::prepare_lifecycle_stage(
                    None::<&crate::request_book::RequestBook<1, 1, 1, 1>>,
                    &ledger,
                    &specs,
                    &mut stage_work,
                )
                .unwrap();
                assert_eq!(
                    stage_work.witness(),
                    HotPathWorkWitness::new(crate::c17_layout::WORK_LIFECYCLE_STAGE)
                );
                crate::transition_coordinator::commit_lifecycle_stage(
                    None::<&crate::request_book::RequestBook<1, 1, 1, 1>>,
                    &mut ledger,
                    stage,
                );
            }

            let mut terminal = false;
            let mut chunks = 0;
            while !terminal {
                let mut abort_work = work();
                let abort = ledger.prepare_c17_lifecycle_abort(&mut abort_work).unwrap();
                assert_eq!(
                    abort_work.witness(),
                    HotPathWorkWitness::new(crate::c17_layout::WORK_LIFECYCLE_ABORT)
                );
                terminal = ledger.commit_c17_lifecycle_abort(abort);
                chunks += 1;
            }
            assert_eq!(chunks, expected_abort_chunks);
            assert_eq!(ledger.c17.pending_lifecycle_aggregate().unwrap(), None);
            assert_eq!(ledger.c17.current_counts_for_test(), baseline_counts);
        }
    }

    #[test]
    fn c17_lifecycle_withholding_blocks_plan_and_abort_releases_it() {
        let mut ledger = lifecycle_plan_ledger();
        let first = reserve_exact_plan_bundle(&mut ledger, 1);
        let second = reserve_exact_plan_bundle(&mut ledger, 2);
        let first_plan = turn_plan_members(&[first], 1, 1);
        let create = ledger
            .prepare_c17_plan_create(
                ledger.generation(),
                &first_plan,
                MonotonicTime::from_micros(2),
                &mut work(),
            )
            .unwrap();
        ledger.commit_c17_plan_create(create);
        let axis = SupportOperation::FormCandidates as usize * POOLS + Mandatory as usize;
        let records = [lifecycle_record(
            1,
            CONDITIONAL,
            Mandatory as usize,
            axis,
            0,
            3,
        )];
        let aggregate = c17::LifecycleAggregate::from_records(&records).unwrap();
        let begin = ledger
            .prepare_c17_lifecycle_begin(ledger.generation(), records.len(), aggregate, &mut work())
            .unwrap();
        ledger.commit_c17_lifecycle_begin(begin);

        let snapshot = (
            ledger.generation(),
            ledger.c17.generation(),
            ledger.usage,
            ledger.reserved,
            ledger.c17.current_counts_for_test(),
        );
        let second_plan = turn_plan_members(&[second], 2, 1);
        assert_eq!(
            ledger
                .prepare_c17_plan_create(
                    ledger.generation(),
                    &second_plan,
                    MonotonicTime::from_micros(3),
                    &mut work(),
                )
                .unwrap_err(),
            CAPACITY_ERROR
        );
        assert_eq!(
            (
                ledger.generation(),
                ledger.c17.generation(),
                ledger.usage,
                ledger.reserved,
                ledger.c17.current_counts_for_test(),
            ),
            snapshot
        );

        let abort =
            crate::transition_coordinator::prepare_lifecycle_abort(&ledger, &mut work()).unwrap();
        assert!(crate::transition_coordinator::commit_lifecycle_abort(
            &mut ledger,
            abort
        ));
        assert!(
            ledger
                .prepare_c17_plan_create(
                    ledger.generation(),
                    &second_plan,
                    MonotonicTime::from_micros(3),
                    &mut work(),
                )
                .is_ok()
        );
    }

    #[test]
    fn c17_lifecycle_withholding_blocks_c16_vector_and_terminal_abort_releases_it() {
        let mut ledger = lifecycle_plan_ledger();
        let axis = SupportOperation::DescribeModel as usize * POOLS + Ordinary as usize;
        ledger.vector_capacity[axis][0] = 1;
        seed_lifecycle_reservation(&mut ledger, CONDITIONAL, Ordinary as usize, 1);
        let records = [lifecycle_record(
            2,
            CONDITIONAL,
            Ordinary as usize,
            axis,
            0,
            1,
        )];
        let aggregate = c17::LifecycleAggregate::from_records(&records).unwrap();
        let begin = ledger
            .prepare_c17_lifecycle_begin(ledger.generation(), records.len(), aggregate, &mut work())
            .unwrap();
        ledger.commit_c17_lifecycle_begin(begin);

        let cells = [OutstandingCreditCell {
            operation: SupportOperation::DescribeModel,
            pool: Ordinary,
            horizon: Duration::from_micros(10),
            max_outstanding: 1,
        }];
        let input = bundle_input(3, &cells);
        assert_eq!(
            ledger.prepare_bundle(&input, &mut work()).unwrap_err(),
            CAPACITY_ERROR
        );
        let abort = ledger.prepare_c17_lifecycle_abort(&mut work()).unwrap();
        assert!(ledger.commit_c17_lifecycle_abort(abort));
        assert!(ledger.prepare_bundle(&input, &mut work()).is_ok());
    }

    #[test]
    fn c17_lifecycle_withholding_blocks_legacy_reserved_consumption() {
        let mut ledger = lifecycle_plan_ledger();
        let id = SupportOperationObligationId::new([40; 32]).unwrap();
        let predecessor = SupportCausalPredecessorId([41; 32]);
        let reserve = [LifecycleReserveSpec {
            id,
            kind: LifecycleReserveKind::PostLoadModelDescription,
            physical_credit: PhysicalStartCreditId::new([42; 32]).unwrap(),
            predecessor,
            scope: SupportCallScopeId([43; 32]),
            claim: Lifecycle([44; 32]),
            expires_at: None,
        }];
        ledger
            .reserve_lifecycle(
                ledger.generation(),
                MonotonicTime::from_micros(1),
                &reserve,
                &mut work(),
            )
            .unwrap();
        ledger
            .resolve_lifecycle(
                ledger.generation(),
                predecessor,
                MonotonicTime::from_micros(2),
                &[id],
                LifecycleTriggerResult::LoadSucceeded,
                &mut work(),
            )
            .unwrap();
        ledger.reserved[CREDITS][Mandatory as usize] = 1;
        ledger.reserved[CLAIMS][Mandatory as usize] = 1;
        let axis = SupportOperation::DescribeModel as usize * POOLS + Mandatory as usize;
        let records = [lifecycle_record(3, ACTIVE, Mandatory as usize, axis, 0, 1)];
        let aggregate = c17::LifecycleAggregate::from_records(&records).unwrap();
        let begin = ledger
            .prepare_c17_lifecycle_begin(ledger.generation(), records.len(), aggregate, &mut work())
            .unwrap();
        ledger.commit_c17_lifecycle_begin(begin);
        let snapshot = (
            ledger.generation(),
            ledger.c17.generation(),
            ledger.usage,
            ledger.reserved,
            ledger.c17.current_counts_for_test(),
        );
        assert_eq!(
            ledger
                .transition(
                    ledger.generation(),
                    id,
                    BeginSupport(MonotonicTime::from_micros(3)),
                    &mut work(),
                )
                .unwrap_err(),
            SupportLedgerError::Storage(FixedStorageError::NonCanonical)
        );
        assert_eq!(
            (
                ledger.generation(),
                ledger.c17.generation(),
                ledger.usage,
                ledger.reserved,
                ledger.c17.current_counts_for_test(),
            ),
            snapshot
        );
        let abort = ledger.prepare_c17_lifecycle_abort(&mut work()).unwrap();
        assert!(ledger.commit_c17_lifecycle_abort(abort));
        assert!(
            ledger
                .transition(
                    ledger.generation(),
                    id,
                    BeginSupport(MonotonicTime::from_micros(3)),
                    &mut work(),
                )
                .is_ok()
        );
    }

    struct NonconflictingFinalizeFixture {
        ledger: LifecyclePlanLedger,
        raw_record: c17::LifecycleRecordInput,
        axis: usize,
    }

    #[inline(never)]
    fn nonconflicting_finalize_fixture() -> NonconflictingFinalizeFixture {
        let mut ledger = lifecycle_plan_ledger();
        let funding = reserve_plan_bundle(&mut ledger, 4);
        let plan = turn_plan_members(&[funding], 1, 1);
        let create = ledger
            .prepare_c17_plan_create(
                ledger.generation(),
                &plan,
                MonotonicTime::from_micros(2),
                &mut work(),
            )
            .unwrap();
        ledger.commit_c17_plan_create(create);
        let axis = SupportOperation::FormCandidates as usize * POOLS + Mandatory as usize;
        let raw_record = lifecycle_record(4, CONDITIONAL, Mandatory as usize, axis, 0, 1);
        let aggregate = c17::LifecycleAggregate::from_records(&[raw_record]).unwrap();
        let begin = ledger
            .prepare_c17_lifecycle_begin(ledger.generation(), 1, aggregate, &mut work())
            .unwrap();
        ledger.commit_c17_lifecycle_begin(begin);
        let specs = [Some(crate::core::C17LifecycleRecordSpec {
            root: crate::core::C17LifecycleRootSpec::Plan {
                identity: plan.identity(),
                branch: crate::PlanBranch::Continuation,
            },
            obligation: SupportOperationObligationId::new(raw_record.obligation_raw).unwrap(),
            credit: PhysicalStartCreditId::new(raw_record.credit_raw).unwrap(),
            predecessor: SupportCausalPredecessorId(raw_record.predecessor),
            scope: SupportCallScopeId(raw_record.scope),
            claim: raw_record.claim,
            kind: LifecycleReserveKind::PostLoadModelDescription,
            occurred_at: MonotonicTime::from_micros(raw_record.occurred_at),
            expires_at: raw_record.expires_at.map(MonotonicTime::from_micros),
            operation: SupportOperation::FormCandidates,
            pool: Mandatory,
            horizon: 0,
        })];
        let stage = crate::transition_coordinator::prepare_lifecycle_stage(
            None::<&crate::request_book::RequestBook<1, 1, 1, 1>>,
            &ledger,
            &specs,
            &mut work(),
        )
        .unwrap();
        crate::transition_coordinator::commit_lifecycle_stage(
            None::<&crate::request_book::RequestBook<1, 1, 1, 1>>,
            &mut ledger,
            stage,
        );
        NonconflictingFinalizeFixture {
            ledger,
            raw_record,
            axis,
        }
    }

    #[inline(never)]
    fn advance_nonconflicting_support_and_raw(
        fixture: &mut NonconflictingFinalizeFixture,
    ) -> SupportLedgerGeneration {
        let support_before = fixture.ledger.generation();
        let raw_before = fixture.ledger.c17.raw_generation_for_test();
        let claims = [Reserved([90; 32])];
        fixture
            .ledger
            .reserve(
                fixture.ledger.generation(),
                spec(90, 91, Ordinary, &claims),
                &mut work(),
            )
            .unwrap();
        assert_eq!(fixture.ledger.generation().get(), support_before.get() + 1);
        assert_eq!(fixture.ledger.c17.raw_generation_for_test(), raw_before + 1);
        support_before
    }

    #[inline(never)]
    fn finalize_after_nonconflicting_advance(
        fixture: &mut NonconflictingFinalizeFixture,
        support_before: SupportLedgerGeneration,
    ) {
        let finalize = fixture
            .ledger
            .prepare_c17_lifecycle_finalize(&mut work())
            .unwrap();
        let committed = fixture.ledger.commit_c17_lifecycle_finalize(finalize);
        assert_eq!(committed.get(), support_before.get() + 2);
        assert!(
            fixture
                .ledger
                .c17
                .lifecycle_record_by_raw(fixture.raw_record.obligation_raw)
                .unwrap()
                .is_some()
        );
        assert_eq!(
            fixture.ledger.c17.pending_lifecycle_aggregate().unwrap(),
            None
        );
        let owner = fixture.ledger.bundles.get_record(0).unwrap();
        assert_eq!(owner.linked_claims, 4);
        let cell = fixture
            .ledger
            .lifecycle_publication_cell(0, owner, fixture.axis, 0)
            .unwrap();
        assert!(matches!(
            fixture.ledger.bundles.cells.slots[cell as usize],
            CellSlot::Occupied { current: 3, .. }
        ));
    }

    #[test]
    fn c17_lifecycle_finalize_rejects_corrupted_raw_header_after_image_without_burn() {
        let fixture = nonconflicting_finalize_fixture();
        let snapshot = (
            fixture.ledger.generation(),
            fixture.ledger.c17.generation(),
            fixture.ledger.c17.raw_generation_for_test(),
            fixture.ledger.c17.current_counts_for_test(),
            fixture.ledger.c17.pending_header_for_test(),
            fixture.ledger.usage,
            fixture.ledger.reserved,
            fixture.ledger.vector_usage,
        );
        let mut finalize_work = work();
        let mut finalize = fixture
            .ledger
            .prepare_c17_lifecycle_finalize(&mut finalize_work)
            .unwrap();
        assert_eq!(
            finalize_work.witness(),
            HotPathWorkWitness::new(crate::c17_layout::WORK_LIFECYCLE_FINALIZE)
        );
        let assignment = finalize.c17.raw_generation_assignment_for_test();
        assert_eq!(assignment.destination_arena, 1);
        assert_eq!(
            assignment.destination_kind,
            crate::c17_layout::DestinationKind::Header as u8
        );
        assert_eq!(assignment.image_len, 40);
        assert_eq!(assignment.destination_slot, 0);
        assert_eq!(
            assignment.expected_generation,
            fixture.ledger.c17.raw_generation_for_test()
        );
        assert_eq!(
            u64::from_le_bytes(assignment.payload[..8].try_into().unwrap()),
            assignment.expected_generation + 1
        );

        finalize.c17.corrupt_raw_generation_assignment_for_test();
        assert_eq!(
            fixture
                .ledger
                .validate_c17_lifecycle_finalize(&finalize)
                .unwrap_err(),
            SupportLedgerError::Generation
        );
        assert_eq!(
            (
                fixture.ledger.generation(),
                fixture.ledger.c17.generation(),
                fixture.ledger.c17.raw_generation_for_test(),
                fixture.ledger.c17.current_counts_for_test(),
                fixture.ledger.c17.pending_header_for_test(),
                fixture.ledger.usage,
                fixture.ledger.reserved,
                fixture.ledger.vector_usage,
            ),
            snapshot
        );
    }

    #[test]
    fn c17_lifecycle_finalize_rejects_direct_record_and_pointer_corruption_without_burn() {
        let mut record_fixture = nonconflicting_finalize_fixture();
        record_fixture
            .ledger
            .c17
            .corrupt_inactive_lifecycle_record_for_test(0);
        let record_before = record_fixture
            .ledger
            .c17
            .inactive_lifecycle_image_for_test(0);
        let record_snapshot = lifecycle_no_burn_snapshot(&record_fixture.ledger);
        let mut record_work = work();
        assert_eq!(
            record_fixture
                .ledger
                .prepare_c17_lifecycle_finalize(&mut record_work)
                .unwrap_err(),
            SupportLedgerError::Storage(FixedStorageError::NonCanonical)
        );
        assert_eq!(record_work.witness(), HotPathWorkWitness::default());
        assert_eq!(
            lifecycle_no_burn_snapshot(&record_fixture.ledger),
            record_snapshot
        );
        assert_eq!(
            record_fixture
                .ledger
                .c17
                .inactive_lifecycle_image_for_test(0),
            record_before
        );

        let mut pointer_fixture = nonconflicting_finalize_fixture();
        let raw_key = pointer_fixture.raw_record.obligation_raw;
        pointer_fixture
            .ledger
            .c17
            .corrupt_raw_owner_pointer_for_test(raw_key);
        let pointer_before = pointer_fixture.ledger.c17.raw_owner_value_for_test(raw_key);
        let pointer_snapshot = lifecycle_no_burn_snapshot(&pointer_fixture.ledger);
        let mut pointer_work = work();
        assert_eq!(
            pointer_fixture
                .ledger
                .prepare_c17_lifecycle_finalize(&mut pointer_work)
                .unwrap_err(),
            SupportLedgerError::Storage(FixedStorageError::NonCanonical)
        );
        assert_eq!(pointer_work.witness(), HotPathWorkWitness::default());
        assert_eq!(
            lifecycle_no_burn_snapshot(&pointer_fixture.ledger),
            pointer_snapshot
        );
        assert_eq!(
            pointer_fixture.ledger.c17.raw_owner_value_for_test(raw_key),
            pointer_before
        );
    }

    #[test]
    fn c17_lifecycle_finalize_survives_nonconflicting_support_and_raw_generation_advance() {
        let mut fixture = nonconflicting_finalize_fixture();
        let support_before = advance_nonconflicting_support_and_raw(&mut fixture);
        finalize_after_nonconflicting_advance(&mut fixture, support_before);
    }

    struct ObservationLifecycleFixture {
        ledger: LifecyclePlanLedger,
        identity: crate::TurnPlanIdentity,
        raw_record: c17::LifecycleRecordInput,
        axis: usize,
    }

    #[inline(never)]
    fn observation_lifecycle_fixture() -> ObservationLifecycleFixture {
        let mut ledger = lifecycle_plan_ledger();
        let funding = reserve_plan_bundle(&mut ledger, 4);
        let plan = turn_plan_members(&[funding], 1, 1);
        let identity = plan.identity();
        let create = ledger
            .prepare_c17_plan_create(
                ledger.generation(),
                &plan,
                MonotonicTime::from_micros(2),
                &mut work(),
            )
            .unwrap();
        ledger.commit_c17_plan_create(create);
        let axis = SupportOperation::FormCandidates as usize * POOLS + Mandatory as usize;
        ObservationLifecycleFixture {
            ledger,
            identity,
            raw_record: lifecycle_record(4, CONDITIONAL, Mandatory as usize, axis, 0, 1),
            axis,
        }
    }

    #[inline(never)]
    fn begin_observation_lifecycle(fixture: &mut ObservationLifecycleFixture) {
        let aggregate = c17::LifecycleAggregate::from_records(&[fixture.raw_record]).unwrap();
        let begin = fixture
            .ledger
            .prepare_c17_lifecycle_begin(fixture.ledger.generation(), 1, aggregate, &mut work())
            .unwrap();
        fixture.ledger.commit_c17_lifecycle_begin(begin);
    }

    #[inline(never)]
    fn observation_lifecycle_specs(
        fixture: &ObservationLifecycleFixture,
    ) -> [Option<crate::core::C17LifecycleRecordSpec>; 1] {
        let raw_record = fixture.raw_record;
        [Some(crate::core::C17LifecycleRecordSpec {
            root: crate::core::C17LifecycleRootSpec::Plan {
                identity: fixture.identity,
                branch: crate::PlanBranch::Continuation,
            },
            obligation: SupportOperationObligationId::new(raw_record.obligation_raw).unwrap(),
            credit: PhysicalStartCreditId::new(raw_record.credit_raw).unwrap(),
            predecessor: SupportCausalPredecessorId(raw_record.predecessor),
            scope: SupportCallScopeId(raw_record.scope),
            claim: raw_record.claim,
            kind: LifecycleReserveKind::PostLoadModelDescription,
            occurred_at: MonotonicTime::from_micros(raw_record.occurred_at),
            expires_at: None,
            operation: SupportOperation::FormCandidates,
            pool: Mandatory,
            horizon: 0,
        })]
    }

    #[inline(never)]
    fn stage_observation_lifecycle(fixture: &mut ObservationLifecycleFixture) {
        let specs = observation_lifecycle_specs(fixture);
        let stage = crate::transition_coordinator::prepare_lifecycle_stage(
            None::<&crate::request_book::RequestBook<1, 1, 1, 1>>,
            &fixture.ledger,
            &specs,
            &mut work(),
        )
        .unwrap();
        crate::transition_coordinator::commit_lifecycle_stage(
            None::<&crate::request_book::RequestBook<1, 1, 1, 1>>,
            &mut fixture.ledger,
            stage,
        );
    }

    #[inline(never)]
    fn finalize_observation_lifecycle(fixture: &mut ObservationLifecycleFixture) {
        let finalize = fixture
            .ledger
            .prepare_c17_lifecycle_finalize(&mut work())
            .unwrap();
        fixture.ledger.commit_c17_lifecycle_finalize(finalize);
    }

    #[inline(never)]
    fn commit_observation_receipt(fixture: &mut ObservationLifecycleFixture) {
        let receipt = fixture
            .ledger
            .prepare_c17_plan_disposition(
                fixture.ledger.generation(),
                fixture.identity,
                c17::PlanDisposition::Receipt,
                MonotonicTime::from_micros(5),
                &mut work(),
            )
            .unwrap();
        fixture.ledger.commit_c17_root_batch(receipt);
    }

    #[inline(never)]
    fn begin_observation_root(fixture: &mut ObservationLifecycleFixture) {
        let begin = fixture
            .ledger
            .prepare_c17_plan_root_action(
                fixture.ledger.generation(),
                fixture.identity,
                0,
                c17::RootAction::Begin,
                MonotonicTime::from_micros(6),
                &mut work(),
            )
            .unwrap();
        fixture.ledger.commit_c17_root_batch(begin);
    }

    #[inline(never)]
    fn resolve_observation(
        fixture: &mut ObservationLifecycleFixture,
        resolution: c17::ObservationResolution,
    ) {
        let vector_before = fixture.ledger.vector_usage[fixture.axis][0];
        let owner_before = fixture.ledger.bundles.get_record(0).unwrap().linked_claims;
        let change = fixture
            .ledger
            .prepare_c17_observation_resolution(
                fixture.ledger.generation(),
                fixture.identity,
                resolution,
                MonotonicTime::from_micros(7),
                &mut work(),
            )
            .unwrap();
        fixture.ledger.commit_c17_root_batch(change);
        match resolution {
            c17::ObservationResolution::DescriptionsRequired => {
                let image = fixture
                    .ledger
                    .c17
                    .lifecycle_record_by_raw(fixture.raw_record.obligation_raw)
                    .unwrap()
                    .expect("transferred lifecycle record remains committed");
                assert_eq!(image[41], crate::PlanBranch::Continuation.ordinal());
                assert_eq!(image[42], c17::RootState::Pending as u8);
                assert_eq!(fixture.ledger.vector_usage[fixture.axis][0], vector_before);
                assert_eq!(
                    fixture.ledger.bundles.get_record(0).unwrap().linked_claims,
                    owner_before
                );
            }
            c17::ObservationResolution::Other => {
                assert!(
                    fixture
                        .ledger
                        .c17
                        .lifecycle_record_by_raw(fixture.raw_record.obligation_raw)
                        .unwrap()
                        .is_none()
                );
                assert_eq!(
                    fixture.ledger.vector_usage[fixture.axis][0],
                    vector_before - 1
                );
                assert_eq!(
                    fixture.ledger.bundles.get_record(0).unwrap().linked_claims,
                    owner_before - 2
                );
            }
        }
    }

    #[inline(never)]
    fn run_observation_resolution_case(resolution: c17::ObservationResolution) {
        let mut fixture = observation_lifecycle_fixture();
        begin_observation_lifecycle(&mut fixture);
        stage_observation_lifecycle(&mut fixture);
        finalize_observation_lifecycle(&mut fixture);
        commit_observation_receipt(&mut fixture);
        begin_observation_root(&mut fixture);
        resolve_observation(&mut fixture, resolution);
    }

    #[test]
    fn c17_observation_resolution_transfers_or_closes_committed_lifecycle_record() {
        for resolution in [
            c17::ObservationResolution::DescriptionsRequired,
            c17::ObservationResolution::Other,
        ] {
            run_observation_resolution_case(resolution);
        }
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    struct LifecycleNoBurnSnapshot {
        support_generation: SupportLedgerGeneration,
        c17_generation: u64,
        raw_generation: u64,
        counts: [usize; 18],
        pending: crate::c17_layout::PendingLifecycleHeaderImage,
        usage: [[u32; POOLS]; 5],
        reserved: [[u32; POOLS]; 5],
        vector_usage: [[u64; 3]; 21],
    }

    fn lifecycle_no_burn_snapshot(ledger: &LifecyclePlanLedger) -> LifecycleNoBurnSnapshot {
        LifecycleNoBurnSnapshot {
            support_generation: ledger.generation(),
            c17_generation: ledger.c17.generation(),
            raw_generation: ledger.c17.raw_generation_for_test(),
            counts: ledger.c17.current_counts_for_test(),
            pending: ledger.c17.pending_header_for_test(),
            usage: ledger.usage,
            reserved: ledger.reserved,
            vector_usage: ledger.vector_usage,
        }
    }

    fn one_under_lifecycle_meter(row: [u64; 5], dimension: WorkDimension) -> WorkMeter {
        let mut limited = row;
        limited[dimension as usize] -= 1;
        WorkMeter::new(HotPathWorkBudget::testing(HotPathWorkWitness::new(limited)))
    }

    #[inline(never)]
    fn assert_lifecycle_stage_one_under(dimension: WorkDimension) {
        let mut fixture = observation_lifecycle_fixture();
        begin_observation_lifecycle(&mut fixture);
        let specs = observation_lifecycle_specs(&fixture);
        let snapshot = lifecycle_no_burn_snapshot(&fixture.ledger);
        let mut limited =
            one_under_lifecycle_meter(crate::c17_layout::WORK_LIFECYCLE_STAGE, dimension);
        assert!(matches!(
            crate::transition_coordinator::prepare_lifecycle_stage(
                None::<&crate::request_book::RequestBook<1, 1, 1, 1>>,
                &fixture.ledger,
                &specs,
                &mut limited,
            ),
            Err(crate::transition_coordinator::LifecycleStagePrepareError::Support(
                SupportLedgerError::Storage(FixedStorageError::Work(
                    WorkBudgetError::BudgetExceeded(actual, _, _)
                ))
            )) if actual == dimension
        ));
        assert_eq!(limited.witness(), HotPathWorkWitness::default());
        assert_eq!(lifecycle_no_burn_snapshot(&fixture.ledger), snapshot);
    }

    #[inline(never)]
    fn assert_lifecycle_finalize_one_under(dimension: WorkDimension) {
        let fixture = nonconflicting_finalize_fixture();
        let snapshot = lifecycle_no_burn_snapshot(&fixture.ledger);
        let mut limited =
            one_under_lifecycle_meter(crate::c17_layout::WORK_LIFECYCLE_FINALIZE, dimension);
        assert!(matches!(
            fixture
                .ledger
                .prepare_c17_lifecycle_finalize(&mut limited),
            Err(SupportLedgerError::Storage(FixedStorageError::Work(
                WorkBudgetError::BudgetExceeded(actual, _, _)
            ))) if actual == dimension
        ));
        assert_eq!(limited.witness(), HotPathWorkWitness::default());
        assert_eq!(lifecycle_no_burn_snapshot(&fixture.ledger), snapshot);
    }

    #[inline(never)]
    fn assert_lifecycle_abort_one_under(dimension: WorkDimension) {
        let mut fixture = observation_lifecycle_fixture();
        begin_observation_lifecycle(&mut fixture);
        let snapshot = lifecycle_no_burn_snapshot(&fixture.ledger);
        let mut limited =
            one_under_lifecycle_meter(crate::c17_layout::WORK_LIFECYCLE_ABORT, dimension);
        assert!(matches!(
            fixture.ledger.prepare_c17_lifecycle_abort(&mut limited),
            Err(SupportLedgerError::Storage(FixedStorageError::Work(
                WorkBudgetError::BudgetExceeded(actual, _, _)
            ))) if actual == dimension
        ));
        assert_eq!(limited.witness(), HotPathWorkWitness::default());
        assert_eq!(lifecycle_no_burn_snapshot(&fixture.ledger), snapshot);
    }

    #[test]
    fn c17_lifecycle_stage_finalize_and_abort_every_axis_one_under_are_byte_stable() {
        for dimension in [
            WorkDimension::VisitedEntities,
            WorkDimension::CopiedBytes,
            WorkDimension::InvariantChecks,
        ] {
            assert_lifecycle_stage_one_under(dimension);
            assert_lifecycle_finalize_one_under(dimension);
            assert_lifecycle_abort_one_under(dimension);
        }
    }

    #[test]
    fn c17_lifecycle_begin_one_under_is_byte_stable() {
        let mut ledger = lifecycle_plan_ledger();
        let axis = SupportOperation::DescribeModel as usize * POOLS + Ordinary as usize;
        seed_lifecycle_reservation(&mut ledger, CONDITIONAL, Ordinary as usize, 1);
        let records = [lifecycle_record(
            5,
            CONDITIONAL,
            Ordinary as usize,
            axis,
            0,
            1,
        )];
        let aggregate = c17::LifecycleAggregate::from_records(&records).unwrap();
        let snapshot = (
            ledger.generation(),
            ledger.c17.generation(),
            ledger.usage,
            ledger.reserved,
            ledger.vector_usage,
            ledger.c17.current_counts_for_test(),
            ledger.c17.pending_header_for_test(),
        );
        for dimension in [
            WorkDimension::VisitedEntities,
            WorkDimension::CopiedBytes,
            WorkDimension::InvariantChecks,
        ] {
            let mut row = crate::c17_layout::WORK_LIFECYCLE_BEGIN;
            row[dimension as usize] -= 1;
            let mut limited =
                WorkMeter::new(HotPathWorkBudget::testing(HotPathWorkWitness::new(row)));
            assert!(matches!(
                ledger.prepare_c17_lifecycle_begin(
                    ledger.generation(),
                    records.len(),
                    aggregate,
                    &mut limited,
                ),
                Err(SupportLedgerError::Storage(FixedStorageError::Work(
                    WorkBudgetError::BudgetExceeded(actual, _, _)
                ))) if actual == dimension
            ));
            assert_eq!(
                (
                    ledger.generation(),
                    ledger.c17.generation(),
                    ledger.usage,
                    ledger.reserved,
                    ledger.vector_usage,
                    ledger.c17.current_counts_for_test(),
                    ledger.c17.pending_header_for_test(),
                ),
                snapshot
            );
        }
    }

    #[test]
    fn c17_create_standalone_coordinates_initial_fact_and_singleton_conservation() {
        std::thread::Builder::new()
            .stack_size(32 << 20)
            .spawn(c17_create_standalone_coordinates_initial_fact_and_singleton_conservation_inner)
            .unwrap()
            .join()
            .unwrap();
    }

    fn c17_create_standalone_coordinates_initial_fact_and_singleton_conservation_inner() {
        use crate::model_descriptor::{ModelDescriptorHash, RawModelDescriptor, verify};
        use crate::model_registry::{
            ModelManifestId, ModelRegistry, ModelRevisionId, RegistrationIntent,
            RegistryGeneration, RevisionSelection,
        };
        use crate::request_book::c17::{
            CancellationKind, CancellationMarker, EligibilityMarker, InitialReadyKind,
            InitialReadyMarker, MembershipDestination, MembershipEventInput, MembershipEventKind,
            MembershipMutation, MergeInitialMarker,
        };
        use crate::request_book::{
            AcceptanceInput, EffectiveSamplingSeed, GenerationParameters, RequestBook,
            RequestBookGeneration, RequestError, RequestSelector, SamplingMode, SamplingSeedOrigin,
            TokenRequest,
        };
        use crate::{ConnectionId, DaemonInstanceId, FormationDomainId};

        const FRAME: [u8; 13] = [0, 0, 0, 1, 0, 0, 0, 7, 0, 0, 0, 1, b'x'];
        const ID: [u8; 32] = [
            0xc9, 0x1c, 0x14, 0x09, 0x1c, 0xea, 0x08, 0xf4, 0x58, 0xa4, 0xe2, 0x75, 0x96, 0xc1,
            0x5b, 0x2c, 0xf0, 0xc8, 0x74, 0x34, 0x2d, 0x30, 0x3e, 0xad, 0xe8, 0x9f, 0x29, 0x0e,
            0xd0, 0x13, 0x38, 0x21,
        ];
        const HASH: [u8; 32] = [
            0xe2, 0x24, 0x6d, 0x47, 0x7f, 0x70, 0xd3, 0xe6, 0x58, 0x8b, 0xb5, 0x45, 0xe2, 0x14,
            0xc0, 0xbb, 0xa1, 0x76, 0x6e, 0xf3, 0x39, 0x7a, 0x50, 0x71, 0x89, 0x29, 0xc9, 0x4f,
            0xe9, 0x62, 0x1e, 0x9b,
        ];

        let revision = ModelRevisionId::new([1; 32]).unwrap();
        let expected_hash = ModelDescriptorHash::from_manifest(1, HASH).unwrap();
        let descriptor = verify(
            RawModelDescriptor {
                frame: &FRAME,
                id: ID,
                hash_schema_version: 1,
                hash: HASH,
                vocabulary: 7,
            },
            expected_hash,
            &mut work(),
        )
        .unwrap();
        let mut registry =
            ModelRegistry::<2, 1, 26>::try_new(RegistryGeneration::new(1).unwrap()).unwrap();
        let registration = RegistrationIntent {
            model: ModelId::new(1).unwrap(),
            revision,
            manifest: ModelManifestId::new([2; 32]).unwrap(),
            expected_descriptor_hash: expected_hash,
            context_limit: TokenCount::new(8),
        };
        let description = registry
            .prepare_description(registry.generation(), registration, &mut work())
            .unwrap();
        let registered = registry
            .prepare_registration(description, &descriptor, &mut work())
            .unwrap();
        registry.commit(registered).unwrap();
        let revision_fact = registry
            .request_revision_fact(
                registry.generation(),
                RevisionSelection::Direct(revision),
                &mut work(),
            )
            .unwrap()
            .unwrap();
        let mut requests = RequestBook::<6, 2, 1, 2>::try_new(
            DaemonInstanceId::new(1).unwrap(),
            RequestBookGeneration::new(1).unwrap(),
        )
        .unwrap();
        let token_request = TokenRequest::try_new(
            RequestSelector::Direct(revision),
            &[],
            GenerationParameters::try_new(
                SamplingMode::Greedy,
                0.0f32.to_bits(),
                1.0f32.to_bits(),
                0,
            )
            .unwrap(),
            ServiceClass::Interactive,
            TokenCount::new(1),
            &[],
            EffectiveSamplingSeed::new(0, SamplingSeedOrigin::Caller),
        )
        .unwrap();
        let accepted = requests
            .prepare(
                requests.generation(),
                revision_fact.generation(),
                AcceptanceInput {
                    connection: ConnectionId::new(1).unwrap(),
                    request: token_request,
                    accepted_at: MonotonicTime::from_micros(1),
                    preparation_timeout: Duration::from_micros(10),
                },
                revision_fact,
                &mut work(),
            )
            .unwrap();
        let request = accepted.accepted().id();
        requests.commit(accepted).unwrap();
        assert_eq!(request, request_owner(1));

        let cells = [
            OutstandingCreditCell {
                operation: SupportOperation::FormCandidates,
                pool: Mandatory,
                horizon: Duration::from_micros(10),
                max_outstanding: 4,
            },
            OutstandingCreditCell {
                operation: SupportOperation::ObserveTurnReceipt,
                pool: Mandatory,
                horizon: Duration::from_micros(10),
                max_outstanding: 4,
            },
        ];
        let mut support = topology_ledger();
        let input = bundle_input(1, &cells);
        let funding = PlanMemberFunding {
            request_id: input.request_owner,
            entitlement: input.entitlement,
            credit_vector: input.vector,
        };
        let obligation = input.initial.materialize.obligation;
        let credit = input.initial.materialize.credit;
        let predecessor = input.initial.materialize.predecessor;
        let mut bundle_work = work();
        let reserved = support.prepare_bundle(&input, &mut bundle_work).unwrap();
        support.validate_bundle(reserved).unwrap().commit_bundle();
        support
            .transition(
                support.generation(),
                obligation,
                PredecessorEnded(predecessor, MonotonicTime::from_micros(2)),
                &mut work(),
            )
            .unwrap();
        support
            .transition(
                support.generation(),
                obligation,
                BeginSupport(MonotonicTime::from_micros(3)),
                &mut work(),
            )
            .unwrap();
        support
            .transition(
                support.generation(),
                obligation,
                FinishSupport(MonotonicTime::from_micros(1_000)),
                &mut work(),
            )
            .unwrap();

        let marker = InitialReadyMarker {
            request,
            kind: InitialReadyKind::MaterializationCompleted,
            identity: [7; 32],
            domain: FormationDomainId::new(8).unwrap().get().to_be_bytes(),
            occurred_at: MonotonicTime::from_micros(4),
            funding,
            obligation,
            credit,
        };
        support.c17.set_retained_budget_for_test(
            c17::SemanticOperation::CreateStandalone,
            crate::c17_layout::CREATE_STANDALONE_BUDGET as u32 - 1,
        );
        let before = (
            support.generation(),
            support.c17.generation(),
            support.c17.raw_generation_for_test(),
            support.c17.current_counts_for_test(),
            support.usage,
            support.reserved,
            requests.generation(),
            support.c17.retained_budgets_for_test(),
        );
        let mut one_under = WorkMeter::new(HotPathWorkBudget::testing(HotPathWorkWitness::new([
            crate::c17_layout::WORK_CREATE_STANDALONE[0] - 1,
            crate::c17_layout::WORK_CREATE_STANDALONE[1],
            0,
            0,
            crate::c17_layout::WORK_CREATE_STANDALONE[4],
        ])));
        assert!(matches!(
            crate::transition_coordinator::prepare_create_standalone(
                &requests,
                &support,
                marker,
                &mut one_under,
            ),
            Err(
                crate::transition_coordinator::CreateStandalonePrepareError::Support(
                    SupportLedgerError::Storage(FixedStorageError::Work(_))
                )
            )
        ));
        assert_eq!(
            (
                support.generation(),
                support.c17.generation(),
                support.c17.raw_generation_for_test(),
                support.c17.current_counts_for_test(),
                support.usage,
                support.reserved,
                requests.generation(),
                support.c17.retained_budgets_for_test(),
            ),
            before
        );

        let mut measured = work();
        let change = crate::transition_coordinator::prepare_create_standalone(
            &requests,
            &support,
            marker,
            &mut measured,
        )
        .unwrap();
        assert_eq!(
            measured.witness(),
            HotPathWorkWitness::new(crate::c17_layout::WORK_CREATE_STANDALONE)
        );
        crate::transition_coordinator::validate_create_standalone(&requests, &support, &change)
            .unwrap();
        crate::transition_coordinator::commit_create_standalone(
            &mut requests,
            &mut support,
            change,
        );
        let counts = support.c17.current_counts_for_test();
        assert_eq!(
            support.c17.retained_budgets_for_test(),
            [crate::c17_layout::CREATE_STANDALONE_BUDGET as u32, 0, 0]
        );
        assert_eq!(
            [
                counts[0], counts[1], counts[2], counts[3], counts[5], counts[6], counts[7],
                counts[8], counts[13], counts[14], counts[15]
            ],
            [11, 1, 8, 1, 1, 4, 4, 1, 1, 1, 1]
        );
        let pool = Mandatory as usize;
        assert_eq!(
            support.usage[CONDITIONAL][pool],
            before.4[CONDITIONAL][pool] + 1
        );
        assert_eq!(support.usage[CREDITS][pool], before.4[CREDITS][pool] + 1);
        assert_eq!(support.usage[CLAIMS][pool], before.4[CLAIMS][pool] + 1);
        assert_eq!(
            support.reserved[CONDITIONAL][pool] + 1,
            before.5[CONDITIONAL][pool]
        );
        assert_eq!(support.reserved[CREDITS][pool] + 1, before.5[CREDITS][pool]);
        assert_eq!(support.reserved[CLAIMS][pool] + 1, before.5[CLAIMS][pool]);
        let owner = support.bundles.get_record(0).unwrap();
        assert_eq!(
            (owner.linked_claims, owner.state),
            (1, BundleState::LiveConsumed)
        );
        let mut next = owner.vector_head;
        let mut currents = Vec::new();
        for _ in 0..owner.vector_len {
            let CellSlot::Occupied {
                cell,
                current,
                next_owned,
                ..
            } = support.bundles.cells.slots[next as usize]
            else {
                panic!("standalone funding cell must remain occupied")
            };
            currents.push((cell.operation, current));
            next = next_owned;
        }
        assert_eq!(
            currents,
            vec![
                (SupportOperation::FormCandidates, 1),
                (SupportOperation::ObserveTurnReceipt, 0)
            ]
        );
        assert_eq!(requests.generation().get(), before.6.get() + 1);
        assert!(matches!(
            crate::transition_coordinator::prepare_create_standalone(
                &requests,
                &support,
                marker,
                &mut work(),
            ),
            Err(
                crate::transition_coordinator::CreateStandalonePrepareError::Request(
                    RequestError::InvalidTransition
                )
            )
        ));

        let token_request = TokenRequest::try_new(
            RequestSelector::Direct(revision),
            &[],
            GenerationParameters::try_new(
                SamplingMode::Greedy,
                0.0f32.to_bits(),
                1.0f32.to_bits(),
                0,
            )
            .unwrap(),
            ServiceClass::Interactive,
            TokenCount::new(1),
            &[],
            EffectiveSamplingSeed::new(1, SamplingSeedOrigin::Caller),
        )
        .unwrap();
        let accepted = requests
            .prepare(
                requests.generation(),
                revision_fact.generation(),
                AcceptanceInput {
                    connection: ConnectionId::new(1).unwrap(),
                    request: token_request,
                    accepted_at: MonotonicTime::from_micros(5),
                    preparation_timeout: Duration::from_micros(10),
                },
                revision_fact,
                &mut work(),
            )
            .unwrap();
        let request_two = accepted.accepted().id();
        requests.commit(accepted).unwrap();

        let mut input_two = bundle_input(2, &cells);
        input_two.request_owner = request_two;
        let funding_two = PlanMemberFunding {
            request_id: request_two,
            entitlement: input_two.entitlement,
            credit_vector: input_two.vector,
        };
        let obligation_two = input_two.initial.materialize.obligation;
        let credit_two = input_two.initial.materialize.credit;
        let predecessor_two = input_two.initial.materialize.predecessor;
        let mut bundle_work_two = work();
        let reserved = support
            .prepare_bundle(&input_two, &mut bundle_work_two)
            .unwrap();
        support.validate_bundle(reserved).unwrap().commit_bundle();
        support
            .transition(
                support.generation(),
                obligation_two,
                PredecessorEnded(predecessor_two, MonotonicTime::from_micros(5)),
                &mut work(),
            )
            .unwrap();
        support
            .transition(
                support.generation(),
                obligation_two,
                BeginSupport(MonotonicTime::from_micros(6)),
                &mut work(),
            )
            .unwrap();
        support
            .transition(
                support.generation(),
                obligation_two,
                FinishSupport(MonotonicTime::from_micros(1_000)),
                &mut work(),
            )
            .unwrap();
        let marker_two = InitialReadyMarker {
            request: request_two,
            kind: InitialReadyKind::MaterializationCompleted,
            identity: [8; 32],
            domain: marker.domain,
            occurred_at: MonotonicTime::from_micros(8),
            funding: funding_two,
            obligation: obligation_two,
            credit: credit_two,
        };
        let create_limit_snapshot = (
            support.generation(),
            support.c17.generation(),
            support.c17.current_counts_for_test(),
            support.usage,
            support.reserved,
            requests.generation(),
            support.c17.retained_budgets_for_test(),
        );
        let mut create_limit_work = work();
        assert!(matches!(
            crate::transition_coordinator::prepare_create_standalone(
                &requests,
                &support,
                marker_two,
                &mut create_limit_work,
            ),
            Err(
                crate::transition_coordinator::CreateStandalonePrepareError::Support(
                    SupportLedgerError::Storage(FixedStorageError::Capacity)
                )
            )
        ));
        assert_eq!(create_limit_work.witness(), HotPathWorkWitness::default());
        assert_eq!(
            (
                support.generation(),
                support.c17.generation(),
                support.c17.current_counts_for_test(),
                support.usage,
                support.reserved,
                requests.generation(),
                support.c17.retained_budgets_for_test(),
            ),
            create_limit_snapshot
        );
        support
            .c17
            .set_retained_budget_for_test(c17::SemanticOperation::CreateStandalone, 1);
        let change = crate::transition_coordinator::prepare_create_standalone(
            &requests,
            &support,
            marker_two,
            &mut work(),
        )
        .unwrap();
        crate::transition_coordinator::commit_create_standalone(
            &mut requests,
            &mut support,
            change,
        );

        let merge = MergeInitialMarker {
            identities: [[7; 32], [8; 32], [0; 32]],
            source_count: 2,
            domain: marker.domain,
            occurred_at: MonotonicTime::from_micros(10),
        };
        let (anchors, count) = requests.merge_initial_source_anchors(merge).unwrap();
        assert_eq!(count, 2);
        for anchor in anchors[..2].iter().copied() {
            let root = c17::RootAnchor {
                authority_key: anchor.authority_key(),
                branch: anchor.branch(),
                group: anchor.group(),
                root: anchor.root(),
                version: anchor.root_version(),
            };
            let change = support
                .prepare_c17_root_action(
                    support.generation(),
                    root,
                    c17::RootAction::MarkPredecessorEnded,
                    MonotonicTime::from_micros(9),
                    &mut work(),
                )
                .unwrap();
            support.commit_c17_root_batch(change);
        }
        let links_before = [
            support.c17.owner_active_link_for_test(0).unwrap(),
            support.c17.owner_active_link_for_test(1).unwrap(),
        ];
        support.c17.set_retained_budget_for_test(
            c17::SemanticOperation::MergeInitial,
            crate::c17_layout::MERGE_INITIAL_BUDGET as u32 - 1,
        );
        let counts_before = support.c17.current_counts_for_test();
        let raw_before_merge = support.c17.raw_generation_for_test();
        let attached_before: [[u32; 3]; 4] = std::array::from_fn(|class| {
            std::array::from_fn(|pool| support.c17.attached(class, pool).unwrap())
        });
        let aggregate_before = (support.usage, support.reserved, attached_before);
        let request_generation_before = requests.generation();
        let preview = support
            .preview_c17_merge_initial(
                support.generation(),
                anchors,
                count,
                merge.domain,
                merge.occurred_at,
            )
            .unwrap();
        let destination = preview.destination();

        let mut one_under_row = crate::c17_layout::WORK_MERGE_INITIAL;
        one_under_row[WorkDimension::InvariantChecks as usize] -= 1;
        let mut one_under = WorkMeter::new(HotPathWorkBudget::testing(HotPathWorkWitness::new(
            one_under_row,
        )));
        assert!(matches!(
            crate::transition_coordinator::prepare_merge_initial(
                &requests,
                &support,
                merge,
                &mut one_under,
            ),
            Err(
                crate::transition_coordinator::MergeInitialPrepareError::Support(
                    SupportLedgerError::Storage(FixedStorageError::Work(_))
                )
            )
        ));
        assert_eq!(one_under.witness(), HotPathWorkWitness::new([0; 5]));
        assert_eq!(support.c17.current_counts_for_test(), counts_before);
        assert_eq!(
            support.c17.retained_budgets_for_test()[1],
            crate::c17_layout::MERGE_INITIAL_BUDGET as u32 - 1
        );
        assert_eq!(requests.generation(), request_generation_before);

        let mut measured = work();
        let change = crate::transition_coordinator::prepare_merge_initial(
            &requests,
            &support,
            merge,
            &mut measured,
        )
        .unwrap();
        assert_eq!(
            measured.witness(),
            HotPathWorkWitness::new(crate::c17_layout::WORK_MERGE_INITIAL)
        );
        crate::transition_coordinator::validate_merge_initial(&requests, &support, &change)
            .unwrap();
        crate::transition_coordinator::commit_merge_initial(&mut requests, &mut support, change);
        let counts_after = support.c17.current_counts_for_test();
        assert_eq!(
            support.c17.retained_budgets_for_test()[1],
            crate::c17_layout::MERGE_INITIAL_BUDGET as u32
        );
        assert_eq!(counts_after[3], counts_before[3] + 1);
        assert_eq!(counts_after[5], counts_before[5] + 3);
        assert_eq!(counts_after[6], counts_before[6] + 12);
        assert_eq!(counts_after[7], counts_before[7] + 4);
        assert_eq!(counts_after[8], counts_before[8] + 3);
        assert_eq!(counts_after[13], counts_before[13] + 2);
        assert_eq!(counts_after[14], counts_before[14] + 1);
        assert_eq!(counts_after[15], counts_before[15] + 3);
        assert_eq!(support.c17.raw_generation_for_test(), raw_before_merge);
        let mut expected_usage = aggregate_before.0;
        expected_usage[PENDING][Mandatory as usize] -= 1;
        expected_usage[CREDITS][Mandatory as usize] -= 1;
        let mut expected_attached = aggregate_before.2;
        expected_attached[PENDING][Mandatory as usize] += 1;
        expected_attached[CREDITS][Mandatory as usize] += 1;
        assert_eq!(support.usage, expected_usage);
        assert_eq!(support.reserved, aggregate_before.1);
        assert_eq!(
            std::array::from_fn::<_, 4, _>(|class| {
                std::array::from_fn::<_, 3, _>(|pool| support.c17.attached(class, pool).unwrap())
            }),
            expected_attached
        );
        for anchor in anchors[..2].iter().copied() {
            let facts = support
                .c17
                .root_facts_for_test(c17::RootAnchor {
                    authority_key: anchor.authority_key(),
                    branch: anchor.branch(),
                    group: anchor.group(),
                    root: anchor.root(),
                    version: 3,
                })
                .unwrap();
            assert_eq!((facts.0, facts.1), (c17::RootState::ClosedPending, 1));
        }
        let destination_facts = support
            .c17
            .root_facts_for_test(c17::RootAnchor {
                authority_key: destination.authority_key(),
                branch: destination.branch(),
                group: destination.group(),
                root: destination.root(),
                version: destination.root_version(),
            })
            .unwrap();
        assert_eq!(
            (destination_facts.0, destination_facts.1),
            (c17::RootState::Pending, 2)
        );
        for (index, before_link) in links_before.into_iter().enumerate() {
            let after_link = support
                .c17
                .owner_active_link_for_test(index as u32)
                .unwrap();
            assert_ne!(after_link.0, before_link.0);
            assert_eq!(after_link.1, destination.group());
            assert_eq!(after_link.2, destination_facts.3);
        }
        assert_eq!(
            requests.generation().get(),
            request_generation_before.get() + 1
        );
        assert!(matches!(
            crate::transition_coordinator::prepare_merge_initial(
                &requests,
                &support,
                merge,
                &mut work(),
            ),
            Err(
                crate::transition_coordinator::MergeInitialPrepareError::Request(
                    RequestError::InvalidTransition
                )
            )
        ));

        let axis = SupportOperation::FormCandidates as usize * POOLS + Mandatory as usize;
        let lifecycle = lifecycle_record(51, PENDING, Mandatory as usize, axis, 0, 2);
        let aggregate = c17::LifecycleAggregate::from_records(&[lifecycle]).unwrap();
        let begin = support
            .prepare_c17_lifecycle_begin(support.generation(), 1, aggregate, &mut work())
            .unwrap();
        support.commit_c17_lifecycle_begin(begin);
        let specs = [Some(crate::core::C17LifecycleRecordSpec {
            root: crate::core::C17LifecycleRootSpec::Membership {
                request,
                expected_status: crate::RequestStatusVersion::new(3).unwrap(),
            },
            obligation: SupportOperationObligationId::new(lifecycle.obligation_raw).unwrap(),
            credit: PhysicalStartCreditId::new(lifecycle.credit_raw).unwrap(),
            predecessor: SupportCausalPredecessorId(lifecycle.predecessor),
            scope: SupportCallScopeId(lifecycle.scope),
            claim: lifecycle.claim,
            kind: LifecycleReserveKind::PostLoadModelDescription,
            occurred_at: MonotonicTime::from_micros(lifecycle.occurred_at),
            expires_at: None,
            operation: SupportOperation::FormCandidates,
            pool: Mandatory,
            horizon: 0,
        })];
        let stage = crate::transition_coordinator::prepare_lifecycle_stage(
            Some(&requests),
            &support,
            &specs,
            &mut work(),
        )
        .unwrap();
        crate::transition_coordinator::commit_lifecycle_stage(Some(&requests), &mut support, stage);
        let finalize = support.prepare_c17_lifecycle_finalize(&mut work()).unwrap();
        support.commit_c17_lifecycle_finalize(finalize);
        let lifecycle_before_rebind = support
            .c17
            .lifecycle_record_by_raw(lifecycle.obligation_raw)
            .unwrap()
            .copied()
            .expect("membership lifecycle record is committed");

        let rebind = MembershipEventInput {
            kind: MembershipEventKind::Rebind,
            source_identity: None,
            member_count: 2,
            destination_count: 1,
            members: [
                Some(MembershipMutation {
                    request,
                    expected_status: crate::RequestStatusVersion::new(3).unwrap(),
                    destination: MembershipDestination::Destination(0),
                }),
                Some(MembershipMutation {
                    request: request_two,
                    expected_status: crate::RequestStatusVersion::new(3).unwrap(),
                    destination: MembershipDestination::Destination(0),
                }),
                None,
                None,
            ],
            occurred_at: MonotonicTime::from_micros(52),
        };
        support.c17.set_retained_budget_for_test(
            c17::SemanticOperation::SourceFreeRebind,
            crate::c17_layout::POST_CREATE_BUDGET as u32 - 1,
        );
        let change = crate::transition_coordinator::prepare_membership_topology(
            &requests,
            &support,
            rebind,
            &mut work(),
        )
        .unwrap();
        crate::transition_coordinator::validate_membership_topology(&requests, &support, &change)
            .unwrap();
        crate::transition_coordinator::commit_membership_topology(
            &mut requests,
            &mut support,
            change,
        );
        assert_eq!(
            support.c17.retained_budgets_for_test()[2],
            crate::c17_layout::POST_CREATE_BUDGET as u32
        );
        let rebind_anchor = requests
            .c17_membership_anchor(request, crate::RequestStatusVersion::new(4).unwrap())
            .unwrap();
        assert_eq!(
            rebind_anchor,
            requests
                .c17_membership_anchor(request_two, crate::RequestStatusVersion::new(4).unwrap(),)
                .unwrap()
        );
        let lifecycle_after_rebind = support
            .c17
            .lifecycle_record_by_raw(lifecycle.obligation_raw)
            .unwrap()
            .copied()
            .expect("rebound lifecycle record remains committed");
        assert_ne!(lifecycle_after_rebind, lifecycle_before_rebind);
        assert_eq!(
            lifecycle_after_rebind[41],
            crate::PlanBranch::Standalone.ordinal()
        );
        assert_eq!(lifecycle_after_rebind[42], c17::RootState::Pending as u8);

        crate::transition_coordinator::newly_eligible(
            &mut requests,
            EligibilityMarker {
                request,
                identity: [52; 32],
                previous_anchor: rebind_anchor,
                occurred_at: MonotonicTime::from_micros(53),
            },
            &mut work(),
        )
        .unwrap();
        let join = MembershipEventInput {
            kind: MembershipEventKind::Join,
            source_identity: Some([52; 32]),
            member_count: 2,
            destination_count: 1,
            members: [
                Some(MembershipMutation {
                    request,
                    expected_status: crate::RequestStatusVersion::new(5).unwrap(),
                    destination: MembershipDestination::Destination(0),
                }),
                Some(MembershipMutation {
                    request: request_two,
                    expected_status: crate::RequestStatusVersion::new(4).unwrap(),
                    destination: MembershipDestination::Destination(0),
                }),
                None,
                None,
            ],
            occurred_at: MonotonicTime::from_micros(54),
        };
        let post_limit_snapshot = (
            requests.generation(),
            support.generation(),
            support.c17.generation(),
            support.c17.raw_generation_for_test(),
            support.c17.current_counts_for_test(),
            support.c17.retained_budgets_for_test(),
            support.usage,
            support.reserved,
        );
        let mut post_limit_work = work();
        assert!(matches!(
            crate::transition_coordinator::prepare_membership_topology(
                &requests,
                &support,
                join,
                &mut post_limit_work,
            ),
            Err(
                crate::transition_coordinator::MembershipTopologyPrepareError::Support(
                    SupportLedgerError::Storage(FixedStorageError::Capacity)
                )
            )
        ));
        assert_eq!(post_limit_work.witness(), HotPathWorkWitness::default());
        assert_eq!(
            (
                requests.generation(),
                support.generation(),
                support.c17.generation(),
                support.c17.raw_generation_for_test(),
                support.c17.current_counts_for_test(),
                support.c17.retained_budgets_for_test(),
                support.usage,
                support.reserved,
            ),
            post_limit_snapshot
        );
        support
            .c17
            .set_retained_budget_for_test(c17::SemanticOperation::Join, 1);
        let change = crate::transition_coordinator::prepare_membership_topology(
            &requests,
            &support,
            join,
            &mut work(),
        )
        .unwrap();
        crate::transition_coordinator::commit_membership_topology(
            &mut requests,
            &mut support,
            change,
        );
        assert_eq!(support.c17.retained_budgets_for_test()[2], 2);
        let join_anchor = requests
            .c17_membership_anchor(request, crate::RequestStatusVersion::new(6).unwrap())
            .unwrap();
        assert_eq!(
            join_anchor,
            requests
                .c17_membership_anchor(request_two, crate::RequestStatusVersion::new(5).unwrap(),)
                .unwrap()
        );
        assert_ne!(join_anchor, rebind_anchor);
        let lifecycle_after_join = support
            .c17
            .lifecycle_record_by_raw(lifecycle.obligation_raw)
            .unwrap()
            .copied()
            .expect("joined lifecycle record remains committed");
        assert_ne!(lifecycle_after_join, lifecycle_after_rebind);
        assert_eq!(lifecycle_after_join[42], c17::RootState::Pending as u8);

        let mut three_identities = [[0; 32]; 3];
        let mut three_requests = [request; 3];
        for offset in 0..3u8 {
            let token_request = TokenRequest::try_new(
                RequestSelector::Direct(revision),
                &[],
                GenerationParameters::try_new(
                    SamplingMode::Greedy,
                    0.0f32.to_bits(),
                    1.0f32.to_bits(),
                    0,
                )
                .unwrap(),
                ServiceClass::Interactive,
                TokenCount::new(1),
                &[],
                EffectiveSamplingSeed::new(u64::from(offset + 2), SamplingSeedOrigin::Caller),
            )
            .unwrap();
            let accepted = requests
                .prepare(
                    requests.generation(),
                    revision_fact.generation(),
                    AcceptanceInput {
                        connection: ConnectionId::new(1).unwrap(),
                        request: token_request,
                        accepted_at: MonotonicTime::from_micros(11 + u64::from(offset)),
                        preparation_timeout: Duration::from_micros(20),
                    },
                    revision_fact,
                    &mut work(),
                )
                .unwrap();
            let request = accepted.accepted().id();
            requests.commit(accepted).unwrap();
            three_requests[usize::from(offset)] = request;
            let seed = offset + 3;
            let mut input = bundle_input(seed, &cells);
            input.request_owner = request;
            let funding = PlanMemberFunding {
                request_id: request,
                entitlement: input.entitlement,
                credit_vector: input.vector,
            };
            let obligation = input.initial.materialize.obligation;
            let credit = input.initial.materialize.credit;
            let predecessor = input.initial.materialize.predecessor;
            let mut bundle_work = work();
            let reserved = support
                .prepare_bundle(&input, &mut bundle_work)
                .unwrap_or_else(|error| panic!("bundle seed {seed}: {error:?}"));
            support.validate_bundle(reserved).unwrap().commit_bundle();
            support
                .transition(
                    support.generation(),
                    obligation,
                    PredecessorEnded(predecessor, MonotonicTime::from_micros(12)),
                    &mut work(),
                )
                .unwrap();
            support
                .transition(
                    support.generation(),
                    obligation,
                    BeginSupport(MonotonicTime::from_micros(13)),
                    &mut work(),
                )
                .unwrap();
            support
                .transition(
                    support.generation(),
                    obligation,
                    FinishSupport(MonotonicTime::from_micros(1_000)),
                    &mut work(),
                )
                .unwrap();
            let identity = [offset + 9; 32];
            three_identities[usize::from(offset)] = identity;
            let marker = InitialReadyMarker {
                request,
                kind: InitialReadyKind::MaterializationCompleted,
                identity,
                domain: merge.domain,
                occurred_at: MonotonicTime::from_micros(20 + u64::from(offset)),
                funding,
                obligation,
                credit,
            };
            let change = crate::transition_coordinator::prepare_create_standalone(
                &requests,
                &support,
                marker,
                &mut work(),
            )
            .unwrap();
            crate::transition_coordinator::commit_create_standalone(
                &mut requests,
                &mut support,
                change,
            );
        }
        let merge_three = MergeInitialMarker {
            identities: three_identities,
            source_count: 3,
            domain: merge.domain,
            occurred_at: MonotonicTime::from_micros(24),
        };
        let (anchors, count) = requests.merge_initial_source_anchors(merge_three).unwrap();
        for anchor in anchors {
            let change = support
                .prepare_c17_root_action(
                    support.generation(),
                    c17::RootAnchor {
                        authority_key: anchor.authority_key(),
                        branch: anchor.branch(),
                        group: anchor.group(),
                        root: anchor.root(),
                        version: anchor.root_version(),
                    },
                    c17::RootAction::MarkPredecessorEnded,
                    MonotonicTime::from_micros(23),
                    &mut work(),
                )
                .unwrap();
            support.commit_c17_root_batch(change);
        }
        let counts_before = support.c17.current_counts_for_test();
        let preview = support
            .preview_c17_merge_initial(
                support.generation(),
                anchors,
                count,
                merge_three.domain,
                merge_three.occurred_at,
            )
            .unwrap();
        let destination = preview.destination();
        let merge_initial_limit_snapshot = (
            requests.generation(),
            support.generation(),
            support.c17.generation(),
            support.c17.current_counts_for_test(),
            support.c17.retained_budgets_for_test(),
            support.usage,
            support.reserved,
        );
        let mut merge_initial_limit_work = work();
        assert!(matches!(
            crate::transition_coordinator::prepare_merge_initial(
                &requests,
                &support,
                merge_three,
                &mut merge_initial_limit_work,
            ),
            Err(
                crate::transition_coordinator::MergeInitialPrepareError::Support(
                    SupportLedgerError::Storage(FixedStorageError::Capacity)
                )
            )
        ));
        assert_eq!(
            merge_initial_limit_work.witness(),
            HotPathWorkWitness::default()
        );
        assert_eq!(
            (
                requests.generation(),
                support.generation(),
                support.c17.generation(),
                support.c17.current_counts_for_test(),
                support.c17.retained_budgets_for_test(),
                support.usage,
                support.reserved,
            ),
            merge_initial_limit_snapshot
        );
        support
            .c17
            .set_retained_budget_for_test(c17::SemanticOperation::MergeInitial, 1);
        let change = crate::transition_coordinator::prepare_merge_initial(
            &requests,
            &support,
            merge_three,
            &mut work(),
        )
        .unwrap();
        crate::transition_coordinator::commit_merge_initial(&mut requests, &mut support, change);
        let counts_after = support.c17.current_counts_for_test();
        assert_eq!(counts_after[3], counts_before[3] + 1);
        assert_eq!(counts_after[5], counts_before[5] + 4);
        assert_eq!(counts_after[6], counts_before[6] + 16);
        assert_eq!(counts_after[7], counts_before[7] + 4);
        assert_eq!(counts_after[8], counts_before[8] + 4);
        assert_eq!(counts_after[13], counts_before[13] + 3);
        assert_eq!(counts_after[14], counts_before[14] + 1);
        assert_eq!(counts_after[15], counts_before[15] + 4);
        for anchor in anchors {
            assert_eq!(
                support
                    .c17
                    .root_facts_for_test(c17::RootAnchor {
                        authority_key: anchor.authority_key(),
                        branch: anchor.branch(),
                        group: anchor.group(),
                        root: anchor.root(),
                        version: 3,
                    })
                    .unwrap()
                    .0,
                c17::RootState::ClosedPending
            );
        }
        assert_eq!(
            support
                .c17
                .root_facts_for_test(c17::RootAnchor {
                    authority_key: destination.authority_key(),
                    branch: destination.branch(),
                    group: destination.group(),
                    root: destination.root(),
                    version: destination.root_version(),
                })
                .unwrap()
                .1,
            3
        );

        let token_request = TokenRequest::try_new(
            RequestSelector::Direct(revision),
            &[],
            GenerationParameters::try_new(
                SamplingMode::Greedy,
                0.0f32.to_bits(),
                1.0f32.to_bits(),
                0,
            )
            .unwrap(),
            ServiceClass::Interactive,
            TokenCount::new(1),
            &[],
            EffectiveSamplingSeed::new(6, SamplingSeedOrigin::Caller),
        )
        .unwrap();
        let accepted = requests
            .prepare(
                requests.generation(),
                revision_fact.generation(),
                AcceptanceInput {
                    connection: ConnectionId::new(1).unwrap(),
                    request: token_request,
                    accepted_at: MonotonicTime::from_micros(55),
                    preparation_timeout: Duration::from_micros(20),
                },
                revision_fact,
                &mut work(),
            )
            .unwrap();
        let request_six = accepted.accepted().id();
        requests.commit(accepted).unwrap();
        let mut input_six = bundle_input(6, &cells);
        input_six.request_owner = request_six;
        let funding_six = PlanMemberFunding {
            request_id: request_six,
            entitlement: input_six.entitlement,
            credit_vector: input_six.vector,
        };
        let obligation_six = input_six.initial.materialize.obligation;
        let credit_six = input_six.initial.materialize.credit;
        let predecessor_six = input_six.initial.materialize.predecessor;
        let mut bundle_work_six = work();
        let reserved = support
            .prepare_bundle(&input_six, &mut bundle_work_six)
            .unwrap();
        support.validate_bundle(reserved).unwrap().commit_bundle();
        support
            .transition(
                support.generation(),
                obligation_six,
                PredecessorEnded(predecessor_six, MonotonicTime::from_micros(56)),
                &mut work(),
            )
            .unwrap();
        support
            .transition(
                support.generation(),
                obligation_six,
                BeginSupport(MonotonicTime::from_micros(57)),
                &mut work(),
            )
            .unwrap();
        support
            .transition(
                support.generation(),
                obligation_six,
                FinishSupport(MonotonicTime::from_micros(1_000)),
                &mut work(),
            )
            .unwrap();
        let marker_six = InitialReadyMarker {
            request: request_six,
            kind: InitialReadyKind::MaterializationCompleted,
            identity: [20; 32],
            domain: merge.domain,
            occurred_at: MonotonicTime::from_micros(58),
            funding: funding_six,
            obligation: obligation_six,
            credit: credit_six,
        };
        let change = crate::transition_coordinator::prepare_create_standalone(
            &requests,
            &support,
            marker_six,
            &mut work(),
        )
        .unwrap();
        crate::transition_coordinator::commit_create_standalone(
            &mut requests,
            &mut support,
            change,
        );
        let standalone_six = requests
            .c17_membership_anchor(request_six, crate::RequestStatusVersion::new(2).unwrap())
            .unwrap();
        let pending_six = support
            .prepare_c17_root_action(
                support.generation(),
                c17::RootAnchor {
                    authority_key: standalone_six.authority_key(),
                    branch: standalone_six.branch(),
                    group: standalone_six.group(),
                    root: standalone_six.root(),
                    version: standalone_six.root_version(),
                },
                c17::RootAction::MarkPredecessorEnded,
                MonotonicTime::from_micros(59),
                &mut work(),
            )
            .unwrap();
        support.commit_c17_root_batch(pending_six);

        let merge_four = MembershipEventInput {
            kind: MembershipEventKind::Merge,
            source_identity: None,
            member_count: 4,
            destination_count: 1,
            members: [
                Some(MembershipMutation {
                    request: three_requests[0],
                    expected_status: crate::RequestStatusVersion::new(3).unwrap(),
                    destination: MembershipDestination::Destination(0),
                }),
                Some(MembershipMutation {
                    request: three_requests[1],
                    expected_status: crate::RequestStatusVersion::new(3).unwrap(),
                    destination: MembershipDestination::Destination(0),
                }),
                Some(MembershipMutation {
                    request: three_requests[2],
                    expected_status: crate::RequestStatusVersion::new(3).unwrap(),
                    destination: MembershipDestination::Destination(0),
                }),
                Some(MembershipMutation {
                    request: request_six,
                    expected_status: crate::RequestStatusVersion::new(2).unwrap(),
                    destination: MembershipDestination::Destination(0),
                }),
            ],
            occurred_at: MonotonicTime::from_micros(60),
        };
        let mut merge_work = work();
        let change = crate::transition_coordinator::prepare_membership_topology(
            &requests,
            &support,
            merge_four,
            &mut merge_work,
        )
        .unwrap();
        assert_eq!(
            merge_work.witness(),
            HotPathWorkWitness::new(crate::c17_layout::WORK_MERGE)
        );
        crate::transition_coordinator::commit_membership_topology(
            &mut requests,
            &mut support,
            change,
        );
        let merged_four_anchor = requests
            .c17_membership_anchor(
                three_requests[0],
                crate::RequestStatusVersion::new(4).unwrap(),
            )
            .unwrap();
        assert_ne!(merged_four_anchor, standalone_six);
        for request in three_requests {
            assert_eq!(
                requests
                    .c17_membership_anchor(request, crate::RequestStatusVersion::new(4).unwrap())
                    .unwrap(),
                merged_four_anchor
            );
        }
        assert_eq!(
            requests
                .c17_membership_anchor(request_six, crate::RequestStatusVersion::new(3).unwrap(),)
                .unwrap(),
            merged_four_anchor
        );

        let split_lifecycle = lifecycle_record(59, PENDING, Mandatory as usize, axis, 0, 4);
        let split_lifecycle_aggregate =
            c17::LifecycleAggregate::from_records(&[split_lifecycle]).unwrap();
        let begin = support
            .prepare_c17_lifecycle_begin(
                support.generation(),
                1,
                split_lifecycle_aggregate,
                &mut work(),
            )
            .unwrap();
        support.commit_c17_lifecycle_begin(begin);
        let split_lifecycle_specs = [Some(crate::core::C17LifecycleRecordSpec {
            root: crate::core::C17LifecycleRootSpec::Membership {
                request: three_requests[0],
                expected_status: crate::RequestStatusVersion::new(4).unwrap(),
            },
            obligation: SupportOperationObligationId::new(split_lifecycle.obligation_raw).unwrap(),
            credit: PhysicalStartCreditId::new(split_lifecycle.credit_raw).unwrap(),
            predecessor: SupportCausalPredecessorId(split_lifecycle.predecessor),
            scope: SupportCallScopeId(split_lifecycle.scope),
            claim: split_lifecycle.claim,
            kind: LifecycleReserveKind::PostLoadModelDescription,
            occurred_at: MonotonicTime::from_micros(split_lifecycle.occurred_at),
            expires_at: None,
            operation: SupportOperation::FormCandidates,
            pool: Mandatory,
            horizon: 0,
        })];
        let stage = crate::transition_coordinator::prepare_lifecycle_stage(
            Some(&requests),
            &support,
            &split_lifecycle_specs,
            &mut work(),
        )
        .unwrap();
        crate::transition_coordinator::commit_lifecycle_stage(Some(&requests), &mut support, stage);
        let finalize = support.prepare_c17_lifecycle_finalize(&mut work()).unwrap();
        support.commit_c17_lifecycle_finalize(finalize);
        assert!(
            support
                .c17
                .lifecycle_record_by_raw(split_lifecycle.obligation_raw)
                .unwrap()
                .is_some()
        );

        let split = MembershipEventInput {
            kind: MembershipEventKind::Split,
            source_identity: None,
            member_count: 4,
            destination_count: 4,
            members: [
                Some(MembershipMutation {
                    request: three_requests[0],
                    expected_status: crate::RequestStatusVersion::new(4).unwrap(),
                    destination: MembershipDestination::Destination(0),
                }),
                Some(MembershipMutation {
                    request: three_requests[1],
                    expected_status: crate::RequestStatusVersion::new(4).unwrap(),
                    destination: MembershipDestination::Destination(1),
                }),
                Some(MembershipMutation {
                    request: three_requests[2],
                    expected_status: crate::RequestStatusVersion::new(4).unwrap(),
                    destination: MembershipDestination::Destination(2),
                }),
                Some(MembershipMutation {
                    request: request_six,
                    expected_status: crate::RequestStatusVersion::new(3).unwrap(),
                    destination: MembershipDestination::Destination(3),
                }),
            ],
            occurred_at: MonotonicTime::from_micros(61),
        };
        let split_snapshot = (
            requests.generation(),
            support.generation(),
            support.c17.generation(),
            support.c17.current_counts_for_test(),
        );
        for dimension in [
            WorkDimension::VisitedEntities,
            WorkDimension::CopiedBytes,
            WorkDimension::InvariantChecks,
        ] {
            let mut row = crate::c17_layout::WORK_SPLIT;
            row[dimension as usize] -= 1;
            let mut limited =
                WorkMeter::new(HotPathWorkBudget::testing(HotPathWorkWitness::new(row)));
            assert!(matches!(
                crate::transition_coordinator::prepare_membership_topology(
                    &requests,
                    &support,
                    split,
                    &mut limited,
                ),
                Err(
                    crate::transition_coordinator::MembershipTopologyPrepareError::Support(
                        SupportLedgerError::Storage(FixedStorageError::Work(_))
                    )
                )
            ));
            assert_eq!(limited.witness(), HotPathWorkWitness::new([0; 5]));
            assert_eq!(
                (
                    requests.generation(),
                    support.generation(),
                    support.c17.generation(),
                    support.c17.current_counts_for_test(),
                ),
                split_snapshot
            );
        }
        let mut split_work = work();
        let change = crate::transition_coordinator::prepare_membership_topology(
            &requests,
            &support,
            split,
            &mut split_work,
        )
        .unwrap();
        assert_eq!(
            change.assignment_census(),
            ([8, 28, 1, 4, 1, 0], [1, 1, 1, 1, 1, 0], 383)
        );
        assert_eq!(383 + 9 + 1, crate::c17_layout::ORDINARY_ASSIGNMENTS);
        assert_eq!(
            split_work.witness(),
            HotPathWorkWitness::new(crate::c17_layout::WORK_SPLIT)
        );
        crate::transition_coordinator::commit_membership_topology(
            &mut requests,
            &mut support,
            change,
        );
        assert!(
            support
                .c17
                .lifecycle_record_by_raw(split_lifecycle.obligation_raw)
                .unwrap()
                .is_none()
        );
        let split_anchors = [
            requests
                .c17_membership_anchor(
                    three_requests[0],
                    crate::RequestStatusVersion::new(5).unwrap(),
                )
                .unwrap(),
            requests
                .c17_membership_anchor(
                    three_requests[1],
                    crate::RequestStatusVersion::new(5).unwrap(),
                )
                .unwrap(),
            requests
                .c17_membership_anchor(
                    three_requests[2],
                    crate::RequestStatusVersion::new(5).unwrap(),
                )
                .unwrap(),
            requests
                .c17_membership_anchor(request_six, crate::RequestStatusVersion::new(4).unwrap())
                .unwrap(),
        ];
        assert!(
            split_anchors.iter().enumerate().all(|(index, anchor)| {
                split_anchors[..index].iter().all(|prior| prior != anchor)
            })
        );

        let survivor_lifecycle = lifecycle_record(60, PENDING, Mandatory as usize, axis, 0, 1);
        let removed_lifecycle = lifecycle_record(61, PENDING, Mandatory as usize, axis, 0, 1);
        let lifecycle_aggregate =
            c17::LifecycleAggregate::from_records(&[survivor_lifecycle, removed_lifecycle])
                .unwrap();
        let begin = support
            .prepare_c17_lifecycle_begin(support.generation(), 2, lifecycle_aggregate, &mut work())
            .unwrap();
        support.commit_c17_lifecycle_begin(begin);
        let lifecycle_specs = [
            Some(crate::core::C17LifecycleRecordSpec {
                root: crate::core::C17LifecycleRootSpec::Membership {
                    request: three_requests[0],
                    expected_status: crate::RequestStatusVersion::new(5).unwrap(),
                },
                obligation: SupportOperationObligationId::new(survivor_lifecycle.obligation_raw)
                    .unwrap(),
                credit: PhysicalStartCreditId::new(survivor_lifecycle.credit_raw).unwrap(),
                predecessor: SupportCausalPredecessorId(survivor_lifecycle.predecessor),
                scope: SupportCallScopeId(survivor_lifecycle.scope),
                claim: survivor_lifecycle.claim,
                kind: LifecycleReserveKind::PostLoadModelDescription,
                occurred_at: MonotonicTime::from_micros(survivor_lifecycle.occurred_at),
                expires_at: None,
                operation: SupportOperation::FormCandidates,
                pool: Mandatory,
                horizon: 0,
            }),
            Some(crate::core::C17LifecycleRecordSpec {
                root: crate::core::C17LifecycleRootSpec::Membership {
                    request: three_requests[1],
                    expected_status: crate::RequestStatusVersion::new(5).unwrap(),
                },
                obligation: SupportOperationObligationId::new(removed_lifecycle.obligation_raw)
                    .unwrap(),
                credit: PhysicalStartCreditId::new(removed_lifecycle.credit_raw).unwrap(),
                predecessor: SupportCausalPredecessorId(removed_lifecycle.predecessor),
                scope: SupportCallScopeId(removed_lifecycle.scope),
                claim: removed_lifecycle.claim,
                kind: LifecycleReserveKind::PostLoadModelDescription,
                occurred_at: MonotonicTime::from_micros(removed_lifecycle.occurred_at),
                expires_at: None,
                operation: SupportOperation::FormCandidates,
                pool: Mandatory,
                horizon: 0,
            }),
        ];
        let stage = crate::transition_coordinator::prepare_lifecycle_stage(
            Some(&requests),
            &support,
            &lifecycle_specs,
            &mut work(),
        )
        .unwrap();
        crate::transition_coordinator::commit_lifecycle_stage(Some(&requests), &mut support, stage);
        let finalize = support.prepare_c17_lifecycle_finalize(&mut work()).unwrap();
        support.commit_c17_lifecycle_finalize(finalize);
        let survivor_lifecycle_before_merge = support
            .c17
            .lifecycle_record_by_raw(survivor_lifecycle.obligation_raw)
            .unwrap()
            .copied()
            .unwrap();
        let removed_lifecycle_before_merge = support
            .c17
            .lifecycle_record_by_raw(removed_lifecycle.obligation_raw)
            .unwrap()
            .copied()
            .unwrap();

        let merge_three_destinations = MembershipEventInput {
            kind: MembershipEventKind::Merge,
            source_identity: None,
            member_count: 3,
            destination_count: 1,
            members: [
                Some(MembershipMutation {
                    request: three_requests[0],
                    expected_status: crate::RequestStatusVersion::new(5).unwrap(),
                    destination: MembershipDestination::Destination(0),
                }),
                Some(MembershipMutation {
                    request: three_requests[1],
                    expected_status: crate::RequestStatusVersion::new(5).unwrap(),
                    destination: MembershipDestination::Destination(0),
                }),
                Some(MembershipMutation {
                    request: three_requests[2],
                    expected_status: crate::RequestStatusVersion::new(5).unwrap(),
                    destination: MembershipDestination::Destination(0),
                }),
                None,
            ],
            occurred_at: MonotonicTime::from_micros(62),
        };
        let change = crate::transition_coordinator::prepare_membership_topology(
            &requests,
            &support,
            merge_three_destinations,
            &mut work(),
        )
        .unwrap();
        crate::transition_coordinator::commit_membership_topology(
            &mut requests,
            &mut support,
            change,
        );
        let remerged_anchor = requests
            .c17_membership_anchor(
                three_requests[0],
                crate::RequestStatusVersion::new(6).unwrap(),
            )
            .unwrap();
        for request in three_requests {
            assert_eq!(
                requests
                    .c17_membership_anchor(request, crate::RequestStatusVersion::new(6).unwrap())
                    .unwrap(),
                remerged_anchor
            );
        }
        assert_ne!(remerged_anchor, split_anchors[0]);
        let survivor_lifecycle_after_merge = support
            .c17
            .lifecycle_record_by_raw(survivor_lifecycle.obligation_raw)
            .unwrap()
            .copied()
            .unwrap();
        let removed_lifecycle_after_merge = support
            .c17
            .lifecycle_record_by_raw(removed_lifecycle.obligation_raw)
            .unwrap()
            .copied()
            .unwrap();
        assert_ne!(
            survivor_lifecycle_after_merge,
            survivor_lifecycle_before_merge
        );
        assert_ne!(
            removed_lifecycle_after_merge,
            removed_lifecycle_before_merge
        );

        let close_joined = MembershipEventInput {
            kind: MembershipEventKind::Close,
            source_identity: None,
            member_count: 2,
            destination_count: 0,
            members: [
                Some(MembershipMutation {
                    request,
                    expected_status: crate::RequestStatusVersion::new(6).unwrap(),
                    destination: MembershipDestination::Closed,
                }),
                Some(MembershipMutation {
                    request: request_two,
                    expected_status: crate::RequestStatusVersion::new(5).unwrap(),
                    destination: MembershipDestination::Closed,
                }),
                None,
                None,
            ],
            occurred_at: MonotonicTime::from_micros(63),
        };
        let lifecycle_vector_before_close = support.vector_usage[axis][0];
        let change = crate::transition_coordinator::prepare_membership_topology(
            &requests,
            &support,
            close_joined,
            &mut work(),
        )
        .unwrap();
        crate::transition_coordinator::commit_membership_topology(
            &mut requests,
            &mut support,
            change,
        );
        assert!(
            support
                .c17
                .lifecycle_record_by_raw(lifecycle.obligation_raw)
                .unwrap()
                .is_none()
        );
        assert_eq!(
            support.vector_usage[axis][0],
            lifecycle_vector_before_close - 2
        );
        assert!(
            requests
                .c17_membership_anchor(request, crate::RequestStatusVersion::new(7).unwrap())
                .is_err()
        );
        assert!(
            requests
                .c17_membership_anchor(request_two, crate::RequestStatusVersion::new(6).unwrap())
                .is_err()
        );
        assert!(matches!(
            crate::transition_coordinator::prepare_membership_topology(
                &requests,
                &support,
                close_joined,
                &mut work(),
            ),
            Err(
                crate::transition_coordinator::MembershipTopologyPrepareError::Request(
                    RequestError::InvalidTransition
                )
            )
        ));

        let cancellation_lifecycle_vector_before = support.vector_usage[axis][0];
        let remove_from_group = CancellationMarker {
            request: three_requests[1],
            identity: [89; 32],
            kind: CancellationKind::Deadline,
            previous_anchor: remerged_anchor,
            occurred_at: MonotonicTime::from_micros(64),
        };
        let request_preview = requests.prepare_cancellation(remove_from_group).unwrap();
        assert_eq!(request_preview.source_count(), 1);
        let support_preview = support
            .preview_c17_cancellation_topology(support.generation(), &request_preview)
            .unwrap();
        assert!(!support_preview.terminal_destination());
        let survivor_anchor = support_preview.cancellation_survivor();
        assert!(!survivor_anchor.is_absent());
        assert_eq!(support_preview.source_member_count(), 3);
        let mut remove_work = work();
        let remove = crate::transition_coordinator::prepare_cancellation_remove(
            &requests,
            &support,
            remove_from_group,
            &mut remove_work,
        )
        .unwrap();
        assert_eq!(
            remove_work.witness(),
            HotPathWorkWitness::new(crate::c17_layout::WORK_REMOVE_BOUND)
        );
        crate::transition_coordinator::validate_cancellation_remove(&requests, &support, &remove)
            .unwrap();
        crate::transition_coordinator::commit_cancellation_remove(
            &mut requests,
            &mut support,
            remove,
        );
        let survivor_lifecycle_after_remove = support
            .c17
            .lifecycle_record_by_raw(survivor_lifecycle.obligation_raw)
            .unwrap()
            .copied()
            .expect("survivor-only lifecycle record transfers");
        assert_ne!(
            survivor_lifecycle_after_remove,
            survivor_lifecycle_after_merge
        );
        assert!(
            support
                .c17
                .lifecycle_record_by_raw(removed_lifecycle.obligation_raw)
                .unwrap()
                .is_none()
        );
        assert_eq!(
            support.vector_usage[axis][0],
            cancellation_lifecycle_vector_before - 1
        );
        assert!(
            requests
                .c17_membership_anchor(
                    three_requests[1],
                    crate::RequestStatusVersion::new(7).unwrap(),
                )
                .is_err()
        );
        for survivor in [three_requests[0], three_requests[2]] {
            assert_eq!(
                requests
                    .c17_membership_anchor(survivor, crate::RequestStatusVersion::new(7).unwrap(),)
                    .unwrap(),
                survivor_anchor
            );
        }
        assert_eq!(
            support
                .c17
                .root_facts_for_test(c17::RootAnchor {
                    authority_key: survivor_anchor.authority_key(),
                    branch: survivor_anchor.branch(),
                    group: survivor_anchor.group(),
                    root: survivor_anchor.root(),
                    version: survivor_anchor.root_version(),
                })
                .unwrap()
                .1,
            2
        );
        assert_eq!(
            support.c17.owner_currents_for_test(3).unwrap(),
            (0, [0; 4], false)
        );
        for slot in [2, 4] {
            let (_, group, _) = support.c17.owner_active_link_for_test(slot).unwrap();
            assert_eq!(group, survivor_anchor.group());
        }
        assert!(matches!(
            crate::transition_coordinator::prepare_cancellation_remove(
                &requests,
                &support,
                remove_from_group,
                &mut work(),
            ),
            Err(
                crate::transition_coordinator::CancellationPrepareError::Request(
                    RequestError::InvalidTransition
                )
            )
        ));

        let remove_to_singleton = CancellationMarker {
            request: three_requests[2],
            identity: [91; 32],
            kind: CancellationKind::Client,
            previous_anchor: survivor_anchor,
            occurred_at: MonotonicTime::from_micros(65),
        };
        let remove = crate::transition_coordinator::prepare_cancellation_remove(
            &requests,
            &support,
            remove_to_singleton,
            &mut work(),
        )
        .unwrap();
        crate::transition_coordinator::commit_cancellation_remove(
            &mut requests,
            &mut support,
            remove,
        );
        let survivor_lifecycle_after_second_remove = support
            .c17
            .lifecycle_record_by_raw(survivor_lifecycle.obligation_raw)
            .unwrap()
            .copied()
            .expect("survivor lifecycle transfers to singleton");
        assert_ne!(
            survivor_lifecycle_after_second_remove,
            survivor_lifecycle_after_remove
        );
        let singleton_survivor = requests
            .c17_membership_anchor(
                three_requests[0],
                crate::RequestStatusVersion::new(8).unwrap(),
            )
            .unwrap();
        assert!(
            requests
                .c17_membership_anchor(
                    three_requests[2],
                    crate::RequestStatusVersion::new(8).unwrap(),
                )
                .is_err()
        );
        crate::transition_coordinator::newly_eligible(
            &mut requests,
            EligibilityMarker {
                request: three_requests[0],
                identity: [92; 32],
                previous_anchor: singleton_survivor,
                occurred_at: MonotonicTime::from_micros(66),
            },
            &mut work(),
        )
        .unwrap();
        let eligible_cancellation = CancellationMarker {
            request: three_requests[0],
            identity: [93; 32],
            kind: CancellationKind::DaemonShutdown,
            previous_anchor: singleton_survivor,
            occurred_at: MonotonicTime::from_micros(67),
        };
        let eligible_preview = requests
            .prepare_cancellation(eligible_cancellation)
            .unwrap();
        assert_eq!(eligible_preview.source_count(), 2);
        let eligible_support_preview = support
            .preview_c17_cancellation_topology(support.generation(), &eligible_preview)
            .unwrap();
        assert!(eligible_support_preview.terminal_destination());
        let eligible_terminal = eligible_support_preview.destination_anchors()[0];
        let eligible_lifecycle_vector_before = support.vector_usage[axis][0];
        let eligible_snapshot = (
            requests.generation(),
            support.generation(),
            support.c17.generation(),
            support.c17.current_counts_for_test(),
            support.usage,
            support.reserved,
        );
        for dimension in [
            WorkDimension::VisitedEntities,
            WorkDimension::CopiedBytes,
            WorkDimension::InvariantChecks,
        ] {
            let mut row = crate::c17_layout::WORK_REMOVE_ELIGIBLE;
            row[dimension as usize] -= 1;
            let mut limited =
                WorkMeter::new(HotPathWorkBudget::testing(HotPathWorkWitness::new(row)));
            let error = crate::transition_coordinator::prepare_cancellation_remove(
                &requests,
                &support,
                eligible_cancellation,
                &mut limited,
            )
            .err()
            .expect("one-under EligibleUnbound cancellation must fail");
            assert!(matches!(
                error,
                crate::transition_coordinator::CancellationPrepareError::Support(
                    SupportLedgerError::Storage(FixedStorageError::Work(_))
                )
            ));
            assert_eq!(limited.witness(), HotPathWorkWitness::new([0; 5]));
            assert_eq!(
                (
                    requests.generation(),
                    support.generation(),
                    support.c17.generation(),
                    support.c17.current_counts_for_test(),
                    support.usage,
                    support.reserved,
                ),
                eligible_snapshot
            );
        }
        let mut eligible_work = work();
        let remove = crate::transition_coordinator::prepare_cancellation_remove(
            &requests,
            &support,
            eligible_cancellation,
            &mut eligible_work,
        )
        .unwrap();
        assert_eq!(
            eligible_work.witness(),
            HotPathWorkWitness::new(crate::c17_layout::WORK_REMOVE_ELIGIBLE)
        );
        crate::transition_coordinator::commit_cancellation_remove(
            &mut requests,
            &mut support,
            remove,
        );
        assert!(
            support
                .c17
                .lifecycle_record_by_raw(survivor_lifecycle.obligation_raw)
                .unwrap()
                .is_none()
        );
        assert_eq!(
            support.vector_usage[axis][0],
            eligible_lifecycle_vector_before - 1
        );
        assert!(
            requests
                .c17_membership_anchor(
                    three_requests[0],
                    crate::RequestStatusVersion::new(10).unwrap(),
                )
                .is_err()
        );
        assert_eq!(
            support
                .c17
                .root_facts_for_test(c17::RootAnchor {
                    authority_key: eligible_terminal.authority_key(),
                    branch: eligible_terminal.branch(),
                    group: eligible_terminal.group(),
                    root: eligible_terminal.root(),
                    version: eligible_terminal.root_version(),
                })
                .unwrap()
                .0,
            c17::RootState::Pending
        );
        assert_eq!(
            support.c17.owner_currents_for_test(2).unwrap(),
            (1, [0, 0, 0, 1], false)
        );
        let immutable_drift = CancellationMarker {
            kind: CancellationKind::InternalFailure,
            occurred_at: MonotonicTime::from_micros(68),
            ..eligible_cancellation
        };
        assert!(matches!(
            crate::transition_coordinator::prepare_cancellation_remove(
                &requests,
                &support,
                immutable_drift,
                &mut work(),
            ),
            Err(
                crate::transition_coordinator::CancellationPrepareError::Request(
                    RequestError::Storage(FixedStorageError::NonCanonical)
                )
            )
        ));

        let cancellation = CancellationMarker {
            request: request_six,
            identity: [90; 32],
            kind: CancellationKind::Client,
            previous_anchor: split_anchors[3],
            occurred_at: MonotonicTime::from_micros(69),
        };
        let request_preview = requests.prepare_cancellation(cancellation).unwrap();
        assert_eq!(request_preview.source_count(), 1);
        let cancellation_event = request_preview.event_id();
        let cancellation_fact = request_preview.fact_id();
        let cancellation_request_generation = request_preview.event().generation_after;
        let support_preview = support
            .preview_c17_cancellation_topology(support.generation(), &request_preview)
            .unwrap();
        assert!(support_preview.terminal_destination());
        let terminal_anchor = support_preview.destination_anchors()[0];
        assert_eq!(
            terminal_anchor.branch(),
            crate::PlanBranch::Terminal.ordinal()
        );
        assert!(support_preview.cancellation_survivor().is_absent());

        let cancellation_snapshot = (
            requests.generation(),
            support.generation(),
            support.c17.generation(),
            support.c17.current_counts_for_test(),
            support.usage,
            support.reserved,
            *support.bundles.get_record(5).unwrap(),
        );
        for dimension in [
            WorkDimension::VisitedEntities,
            WorkDimension::CopiedBytes,
            WorkDimension::InvariantChecks,
        ] {
            let mut row = crate::c17_layout::WORK_REMOVE_BOUND;
            row[dimension as usize] -= 1;
            let mut limited =
                WorkMeter::new(HotPathWorkBudget::testing(HotPathWorkWitness::new(row)));
            let error = crate::transition_coordinator::prepare_cancellation_remove(
                &requests,
                &support,
                cancellation,
                &mut limited,
            )
            .err()
            .expect("one-under CancellationRemove must fail");
            assert!(
                matches!(
                    error,
                    crate::transition_coordinator::CancellationPrepareError::Support(
                        SupportLedgerError::Storage(FixedStorageError::Work(_))
                    )
                ),
                "unexpected one-under cancellation error: {error:?}; counts={:?}",
                support.c17.current_counts_for_test()
            );
            assert_eq!(limited.witness(), HotPathWorkWitness::new([0; 5]));
            assert_eq!(
                (
                    requests.generation(),
                    support.generation(),
                    support.c17.generation(),
                    support.c17.current_counts_for_test(),
                    support.usage,
                    support.reserved,
                    *support.bundles.get_record(5).unwrap(),
                ),
                cancellation_snapshot
            );
        }
        let mut cancellation_work = work();
        let change = crate::transition_coordinator::prepare_cancellation_remove(
            &requests,
            &support,
            cancellation,
            &mut cancellation_work,
        )
        .unwrap();
        assert_eq!(
            cancellation_work.witness(),
            HotPathWorkWitness::new(crate::c17_layout::WORK_REMOVE_BOUND)
        );
        crate::transition_coordinator::commit_cancellation_remove(
            &mut requests,
            &mut support,
            change,
        );
        assert_eq!(
            requests.generation().get(),
            cancellation_snapshot.0.get() + 1
        );
        assert!(
            requests
                .c17_membership_anchor(request_six, crate::RequestStatusVersion::new(5).unwrap())
                .is_err()
        );
        let terminal_root = c17::RootAnchor {
            authority_key: terminal_anchor.authority_key(),
            branch: terminal_anchor.branch(),
            group: terminal_anchor.group(),
            root: terminal_anchor.root(),
            version: terminal_anchor.root_version(),
        };
        assert_eq!(
            support.c17.root_facts_for_test(terminal_root).unwrap().0,
            c17::RootState::Pending
        );
        assert_eq!(
            support.c17.owner_currents_for_test(5).unwrap(),
            (1, [0, 0, 0, 1], false)
        );
        let terminal_formation = support.c17.root_formation_for_test(terminal_root).unwrap();
        assert_eq!(
            u64::from_le_bytes(terminal_formation[8..16].try_into().unwrap()),
            cancellation_event
        );
        assert_eq!(
            u64::from_le_bytes(terminal_formation[16..24].try_into().unwrap()),
            cancellation_fact
        );
        assert_eq!(
            u64::from_le_bytes(terminal_formation[24..32].try_into().unwrap()),
            cancellation_request_generation
        );
        assert_eq!(
            terminal_formation[222],
            c17::FormationCause::CancellationMembership as u8
        );
        assert!(matches!(
            crate::transition_coordinator::prepare_cancellation_remove(
                &requests,
                &support,
                cancellation,
                &mut work(),
            ),
            Err(
                crate::transition_coordinator::CancellationPrepareError::Request(
                    RequestError::InvalidTransition
                )
            )
        ));

        let close_terminal = crate::TypedCloseInput {
            group: terminal_anchor.group().slot,
            branch: crate::PlanBranch::Terminal,
            root: crate::RootRef::new(
                terminal_anchor.root().slot,
                terminal_anchor.root().generation,
                terminal_anchor.root_version(),
            )
            .unwrap(),
            occurred_at: MonotonicTime::from_micros(70),
            reason: crate::TypedImpossible::new(7).unwrap(),
            authority: crate::CloseAuthority::Cancellation {
                fact: crate::CancellationFactId::new(cancellation_fact).unwrap(),
                event: crate::MembershipEventId::new(cancellation_event).unwrap(),
                request_generation: RequestBookGeneration::new(cancellation_request_generation)
                    .unwrap(),
            },
        };
        let mut close_work = work();
        let close = crate::transition_coordinator::prepare_typed_close(
            Some(&requests),
            &support,
            close_terminal,
            &mut close_work,
        )
        .unwrap();
        assert_eq!(
            close_work.witness(),
            HotPathWorkWitness::new(crate::c17_layout::WORK_CLOSE)
        );
        crate::transition_coordinator::commit_typed_close(Some(&requests), &mut support, close);
        assert_eq!(
            support
                .c17
                .root_facts_for_test(c17::RootAnchor {
                    version: 2,
                    ..terminal_root
                })
                .unwrap()
                .0,
            c17::RootState::ClosedPending
        );
        assert_eq!(
            support.c17.owner_currents_for_test(5).unwrap(),
            (0, [0; 4], false)
        );
    }

    #[test]
    fn c17_plan_create_is_one_atomic_three_root_materialization() {
        let mut ledger = plan_ledger();
        let funders = std::array::from_fn(|index| {
            reserve_plan_bundle(&mut ledger, u8::try_from(index + 1).unwrap())
        });
        let plan = turn_plan(&funders, 1, 1);
        let before_generation = ledger.generation();
        let before_c17 = ledger.c17.generation();
        let mut measured = work();
        let change = ledger
            .prepare_c17_plan_create(
                before_generation,
                &plan,
                MonotonicTime::from_micros(2),
                &mut measured,
            )
            .unwrap();
        assert_eq!(
            measured.witness(),
            HotPathWorkWitness::new(crate::c17_layout::WORK_PLAN_CREATE)
        );
        ledger.validate_c17_plan_create(&change).unwrap();
        assert_eq!(
            ledger.commit_c17_plan_create(change).get(),
            before_generation.get() + 1
        );
        assert_eq!(ledger.c17.generation(), before_c17 + 1);
        let mandatory = Mandatory as usize;
        assert_eq!(ledger.usage[CONDITIONAL][mandatory], 15);
        assert_eq!(ledger.usage[CREDITS][mandatory], 15);
        assert_eq!(ledger.usage[CLAIMS][mandatory], 24);
        assert_eq!(ledger.reserved[CONDITIONAL][mandatory], 20);
        assert_eq!(ledger.reserved[CREDITS][mandatory], 20);
        assert_eq!(ledger.reserved[CLAIMS][mandatory], 20);
        assert_eq!(ledger.c17.attached(CONDITIONAL, mandatory).unwrap(), 9);
        assert_eq!(ledger.c17.attached(CREDITS, mandatory).unwrap(), 9);
        for slot in 0..4 {
            let record = ledger.bundles.get_record(slot).unwrap();
            assert_eq!(record.linked_claims, 3);
            assert_eq!(record.state, BundleState::LiveConsumed);
        }

        let snapshot = (
            ledger.generation(),
            ledger.c17.generation(),
            ledger.usage,
            ledger.reserved,
            ledger.c17.current_counts_for_test(),
        );
        let mut replay_work = work();
        assert!(matches!(
            ledger.prepare_c17_plan_create(
                ledger.generation(),
                &plan,
                MonotonicTime::from_micros(2),
                &mut replay_work,
            ),
            Err(SupportLedgerError::InvalidTransition)
        ));
        assert_eq!(replay_work.witness(), HotPathWorkWitness::new([0; 5]));
        assert_eq!(
            (
                ledger.generation(),
                ledger.c17.generation(),
                ledger.usage,
                ledger.reserved,
                ledger.c17.current_counts_for_test(),
            ),
            snapshot
        );

        let drift = turn_plan(&funders, 1, 2);
        assert!(matches!(
            ledger.prepare_c17_plan_create(
                ledger.generation(),
                &drift,
                MonotonicTime::from_micros(3),
                &mut work(),
            ),
            Err(SupportLedgerError::Storage(FixedStorageError::NonCanonical))
        ));

        let mut row = crate::c17_layout::WORK_PLAN_CREATE;
        row[WorkDimension::CopiedBytes as usize] -= 1;
        let mut limited = WorkMeter::new(
            HotPathWorkBudget::try_new(HotPathWorkWitness::new([
                1_000_000, row[1], row[2], row[3], row[4],
            ]))
            .unwrap(),
        );
        let mut limited_ledger = plan_ledger();
        let limited_funders = std::array::from_fn(|index| {
            reserve_plan_bundle(&mut limited_ledger, u8::try_from(index + 1).unwrap())
        });
        let limited_snapshot = (
            limited_ledger.generation(),
            limited_ledger.c17.generation(),
            limited_ledger.usage,
            limited_ledger.reserved,
            limited_ledger.c17.current_counts_for_test(),
        );
        let fresh_plan = turn_plan(&limited_funders, 2, 1);
        assert!(matches!(
            limited_ledger.prepare_c17_plan_create(
                limited_ledger.generation(),
                &fresh_plan,
                MonotonicTime::from_micros(4),
                &mut limited,
            ),
            Err(SupportLedgerError::Storage(FixedStorageError::Work(
                WorkBudgetError::BudgetExceeded(..)
            )))
        ));
        assert_eq!(
            (
                limited_ledger.generation(),
                limited_ledger.c17.generation(),
                limited_ledger.usage,
                limited_ledger.reserved,
                limited_ledger.c17.current_counts_for_test(),
            ),
            limited_snapshot
        );
    }

    #[inline(never)]
    fn commit_plan_create_for_root_test(ledger: &mut PlanLedger, plan: &TurnPlan<4>) {
        let create = ledger
            .prepare_c17_plan_create(
                ledger.generation(),
                plan,
                MonotonicTime::from_micros(2),
                &mut work(),
            )
            .unwrap();
        ledger.commit_c17_plan_create(create);
    }

    #[inline(never)]
    fn commit_plan_receipt_for_root_test(ledger: &mut PlanLedger, plan: &TurnPlan<4>) {
        let before = ledger.generation();
        let mut disposition_work = work();
        let disposition = ledger
            .prepare_c17_plan_disposition(
                before,
                plan.identity(),
                c17::PlanDisposition::Receipt,
                MonotonicTime::from_micros(3),
                &mut disposition_work,
            )
            .unwrap();
        assert_eq!(
            disposition_work.witness(),
            HotPathWorkWitness::new(crate::c17_layout::WORK_PLAN_DISPOSITION)
        );
        ledger.validate_c17_root_batch(&disposition).unwrap();
        assert_eq!(
            ledger.commit_c17_root_batch(disposition).get(),
            before.get() + 1
        );
        for slot in 0..4 {
            assert_eq!(ledger.bundles.get_record(slot).unwrap().linked_claims, 2);
        }
        let mandatory = Mandatory as usize;
        assert_eq!(ledger.usage[CONDITIONAL][mandatory], 13);
        assert_eq!(ledger.usage[PENDING][mandatory], 1);
        assert_eq!(ledger.reserved[CONDITIONAL][mandatory], 24);
        assert_eq!(ledger.usage[CREDITS][mandatory], 14);
        assert_eq!(ledger.reserved[CREDITS][mandatory], 24);
        assert_eq!(ledger.usage[CLAIMS][mandatory], 20);
        assert_eq!(ledger.reserved[CLAIMS][mandatory], 24);
    }

    #[inline(never)]
    fn commit_plan_begin_for_root_test(ledger: &mut PlanLedger, plan: &TurnPlan<4>) {
        let begin = ledger
            .prepare_c17_plan_root_action(
                ledger.generation(),
                plan.identity(),
                0,
                c17::RootAction::Begin,
                MonotonicTime::from_micros(4),
                &mut work(),
            )
            .unwrap();
        ledger.commit_c17_root_batch(begin);
        let mandatory = Mandatory as usize;
        assert_eq!(ledger.usage[PENDING][mandatory], 0);
        assert_eq!(ledger.usage[ACTIVE][mandatory], 1);
    }

    #[inline(never)]
    fn commit_plan_resolution_for_root_test(ledger: &mut PlanLedger, plan: &TurnPlan<4>) {
        let resolution = ledger
            .prepare_c17_observation_resolution(
                ledger.generation(),
                plan.identity(),
                c17::ObservationResolution::DescriptionsRequired,
                MonotonicTime::from_micros(5),
                &mut work(),
            )
            .unwrap();
        ledger.commit_c17_root_batch(resolution);
        let mandatory = Mandatory as usize;
        assert_eq!(ledger.usage[CONDITIONAL][mandatory], 12);
        assert_eq!(ledger.usage[PENDING][mandatory], 1);
        assert_eq!(ledger.usage[ACTIVE][mandatory], 1);
        for slot in 0..4 {
            assert_eq!(ledger.bundles.get_record(slot).unwrap().linked_claims, 2);
        }
    }

    #[inline(never)]
    fn reject_plan_resolution_replay_for_root_test(ledger: &mut PlanLedger, plan: &TurnPlan<4>) {
        let snapshot = (
            ledger.generation(),
            ledger.c17.generation(),
            ledger.usage,
            ledger.reserved,
            ledger.c17.current_counts_for_test(),
        );
        let mut replay_work = work();
        assert!(matches!(
            ledger.prepare_c17_observation_resolution(
                ledger.generation(),
                plan.identity(),
                c17::ObservationResolution::DescriptionsRequired,
                MonotonicTime::from_micros(6),
                &mut replay_work,
            ),
            Err(SupportLedgerError::InvalidTransition)
        ));
        assert_eq!(replay_work.witness(), HotPathWorkWitness::new([0; 5]));
        assert_eq!(
            (
                ledger.generation(),
                ledger.c17.generation(),
                ledger.usage,
                ledger.reserved,
                ledger.c17.current_counts_for_test(),
            ),
            snapshot
        );
    }

    #[test]
    fn c17_plan_root_transitions_apply_one_public_generation_and_conserve_aggregates() {
        let mut ledger = plan_ledger();
        let funders = std::array::from_fn(|index| {
            reserve_plan_bundle(&mut ledger, u8::try_from(index + 1).unwrap())
        });
        let plan = turn_plan(&funders, 1, 1);
        commit_plan_create_for_root_test(&mut ledger, &plan);
        commit_plan_receipt_for_root_test(&mut ledger, &plan);
        commit_plan_begin_for_root_test(&mut ledger, &plan);
        commit_plan_resolution_for_root_test(&mut ledger, &plan);
        reject_plan_resolution_replay_for_root_test(&mut ledger, &plan);
    }

    /// Each C17 plan route is driven through `transition_coordinator` itself, proving the
    /// coordinator's prepare/validate/seal/one-charge/commit phase law rather than the ledger
    /// seam the other plan tests use. A sealed coordinator value carries the fixed assignment
    /// journal, so every route gets its own fixture and only the route under test uses the
    /// coordinator; the prerequisites use the ledger seam.
    #[inline(never)]
    fn coordinator_plan_fixture() -> (Box<PlanLedger>, TurnPlan<4>) {
        let mut ledger = Box::new(plan_ledger());
        let funders = std::array::from_fn(|index| {
            reserve_plan_bundle(&mut ledger, u8::try_from(index + 1).unwrap())
        });
        let plan = turn_plan(&funders, 1, 1);
        (ledger, plan)
    }

    #[test]
    fn c17_plan_create_commits_atomically_through_the_coordinator_seam() {
        let (mut ledger, plan) = coordinator_plan_fixture();
        let before = ledger.generation();
        let mut measured = work();
        let create = crate::transition_coordinator::prepare_plan_create(
            &ledger,
            &plan,
            MonotonicTime::from_micros(2),
            &mut measured,
        )
        .unwrap();
        assert_eq!(
            measured.witness(),
            HotPathWorkWitness::new(crate::c17_layout::WORK_PLAN_CREATE)
        );
        crate::transition_coordinator::validate_plan_create(&ledger, &create).unwrap();
        crate::transition_coordinator::commit_plan_create(&mut ledger, create);
        assert_eq!(ledger.generation().get(), before.get() + 1);
    }

    type NoTypedCloseRequests = crate::request_book::RequestBook<1, 1, 1, 1>;

    struct TypedCloseFixture {
        ledger: PlanLedger,
        funders: [PlanMemberFunding; 4],
        plan: TurnPlan<4>,
        input: crate::TypedCloseInput,
    }

    fn typed_close_owner_cell_currents(ledger: &PlanLedger) -> [[u64; 2]; 4] {
        std::array::from_fn(|slot| {
            let record = ledger.bundles.get_record(slot as u32).unwrap();
            assert_eq!(record.vector_len, 2);
            let mut next = record.vector_head;
            std::array::from_fn(|_| {
                let CellSlot::Occupied {
                    current,
                    next_owned,
                    ..
                } = ledger.bundles.cells.slots[next as usize]
                else {
                    panic!("typed-close owner cell must remain occupied")
                };
                next = next_owned;
                current
            })
        })
    }

    #[inline(never)]
    fn typed_close_fixture() -> TypedCloseFixture {
        let mut ledger = plan_ledger();
        let funders = std::array::from_fn(|index| {
            reserve_plan_bundle(&mut ledger, u8::try_from(index + 1).unwrap())
        });
        let plan = turn_plan(&funders, 1, 1);
        let create = ledger
            .prepare_c17_plan_create(
                ledger.generation(),
                &plan,
                MonotonicTime::from_micros(2),
                &mut work(),
            )
            .unwrap();
        ledger.commit_c17_plan_create(create);
        let rejection = ledger
            .prepare_c17_plan_disposition(
                ledger.generation(),
                plan.identity(),
                c17::PlanDisposition::Rejection,
                MonotonicTime::from_micros(3),
                &mut work(),
            )
            .unwrap();
        ledger.commit_c17_root_batch(rejection);

        let authority_key = plan_authority_key(plan.identity().id.get());
        let identity = encode_plan_identity(plan.identity());
        let anchor = ledger
            .c17
            .plan_root_anchor(authority_key, identity, 2)
            .unwrap();
        let root =
            crate::RootRef::new(anchor.root.slot, anchor.root.generation, anchor.version).unwrap();
        let input = crate::TypedCloseInput {
            group: anchor.group.slot,
            branch: crate::PlanBranch::Rejection,
            root,
            occurred_at: MonotonicTime::from_micros(4),
            reason: crate::TypedImpossible::new(9).unwrap(),
            authority: crate::CloseAuthority::Plan {
                identity: plan.identity(),
                event: crate::PlanCausalEventId::new(1).unwrap(),
            },
        };
        TypedCloseFixture {
            ledger,
            funders,
            plan,
            input,
        }
    }

    #[inline(never)]
    fn assert_typed_close_initial_state(fixture: &TypedCloseFixture) {
        for slot in 0..4 {
            assert_eq!(
                fixture
                    .ledger
                    .bundles
                    .get_record(slot)
                    .unwrap()
                    .linked_claims,
                1
            );
            assert_eq!(
                fixture.ledger.c17.owner_currents_for_test(slot).unwrap(),
                (1, [0, 0, 1, 0], true)
            );
        }
        assert_eq!(
            typed_close_owner_cell_currents(&fixture.ledger),
            [[1, 0]; 4]
        );
    }

    #[inline(never)]
    fn reject_malformed_typed_close_authorities(fixture: &TypedCloseFixture) {
        let no_requests: Option<&NoTypedCloseRequests> = None;
        let malformed_cancellation = crate::TypedCloseInput {
            group: fixture.input.group + 1,
            authority: crate::CloseAuthority::Cancellation {
                fact: crate::CancellationFactId::new(1).unwrap(),
                event: crate::MembershipEventId::new(1).unwrap(),
                request_generation: crate::request_book::RequestBookGeneration::new(1).unwrap(),
            },
            ..fixture.input
        };
        let mut malformed_work = work();
        assert!(matches!(
            crate::transition_coordinator::prepare_typed_close(
                no_requests,
                &fixture.ledger,
                malformed_cancellation,
                &mut malformed_work,
            ),
            Err(
                crate::transition_coordinator::TypedClosePrepareError::Support(
                    SupportLedgerError::InvalidInput
                )
            )
        ));
        assert_eq!(malformed_work.witness(), HotPathWorkWitness::new([0; 5]));

        let missing_source = crate::TypedCloseInput {
            authority: crate::CloseAuthority::Standalone {
                domain: crate::FormationDomainId::new(1).unwrap(),
                source: crate::SourceRecordRef::default(),
                event: crate::MembershipEventId::new(1).unwrap(),
            },
            ..fixture.input
        };
        let mut missing_source_work = work();
        assert!(matches!(
            crate::transition_coordinator::prepare_typed_close(
                no_requests,
                &fixture.ledger,
                missing_source,
                &mut missing_source_work,
            ),
            Err(
                crate::transition_coordinator::TypedClosePrepareError::Support(
                    SupportLedgerError::InvalidInput
                )
            )
        ));
        assert_eq!(
            missing_source_work.witness(),
            HotPathWorkWitness::new([0; 5])
        );
    }

    #[inline(never)]
    fn reject_drifted_typed_close_authority(fixture: &TypedCloseFixture) {
        let no_requests: Option<&NoTypedCloseRequests> = None;
        let drift = turn_plan(&fixture.funders, 1, 2);
        let drift_input = crate::TypedCloseInput {
            authority: crate::CloseAuthority::Plan {
                identity: drift.identity(),
                event: crate::PlanCausalEventId::new(1).unwrap(),
            },
            ..fixture.input
        };
        let mut drift_work = work();
        assert!(matches!(
            crate::transition_coordinator::prepare_typed_close(
                no_requests,
                &fixture.ledger,
                drift_input,
                &mut drift_work,
            ),
            Err(
                crate::transition_coordinator::TypedClosePrepareError::Support(
                    SupportLedgerError::Storage(FixedStorageError::NonCanonical)
                )
            )
        ));
        assert_eq!(drift_work.witness(), HotPathWorkWitness::new([0; 5]));

        let wrong_event = crate::TypedCloseInput {
            authority: crate::CloseAuthority::Plan {
                identity: fixture.plan.identity(),
                event: crate::PlanCausalEventId::new(2).unwrap(),
            },
            ..fixture.input
        };
        let mut wrong_event_work = work();
        assert!(matches!(
            crate::transition_coordinator::prepare_typed_close(
                no_requests,
                &fixture.ledger,
                wrong_event,
                &mut wrong_event_work,
            ),
            Err(
                crate::transition_coordinator::TypedClosePrepareError::Support(
                    SupportLedgerError::InvalidTransition
                )
            )
        ));
        assert_eq!(wrong_event_work.witness(), HotPathWorkWitness::new([0; 5]));
    }

    #[inline(never)]
    fn reject_typed_close_work_one_under_without_burn(fixture: &TypedCloseFixture) {
        let failure_snapshot = (
            fixture.ledger.generation(),
            fixture.ledger.c17.generation(),
            fixture.ledger.c17.raw_generation_for_test(),
            fixture.ledger.c17.current_counts_for_test(),
            fixture.ledger.usage,
            fixture.ledger.reserved,
            std::array::from_fn::<_, 4, _>(|slot| {
                *fixture.ledger.bundles.get_record(slot as u32).unwrap()
            }),
            typed_close_owner_cell_currents(&fixture.ledger),
        );
        for axis in [
            WorkDimension::VisitedEntities as usize,
            WorkDimension::CopiedBytes as usize,
            WorkDimension::InvariantChecks as usize,
        ] {
            let mut limit = crate::c17_layout::WORK_CLOSE;
            limit[axis] -= 1;
            let mut one_under =
                WorkMeter::new(HotPathWorkBudget::testing(HotPathWorkWitness::new(limit)));
            assert!(matches!(
                fixture.ledger.prepare_c17_typed_close(
                    fixture.ledger.generation(),
                    fixture.input,
                    &mut one_under,
                ),
                Err(SupportLedgerError::Storage(FixedStorageError::Work(
                    WorkBudgetError::BudgetExceeded(..)
                )))
            ));
            assert_eq!(one_under.witness(), HotPathWorkWitness::new([0; 5]));
            assert_eq!(
                (
                    fixture.ledger.generation(),
                    fixture.ledger.c17.generation(),
                    fixture.ledger.c17.raw_generation_for_test(),
                    fixture.ledger.c17.current_counts_for_test(),
                    fixture.ledger.usage,
                    fixture.ledger.reserved,
                    std::array::from_fn::<_, 4, _>(|slot| {
                        *fixture.ledger.bundles.get_record(slot as u32).unwrap()
                    }),
                    typed_close_owner_cell_currents(&fixture.ledger),
                ),
                failure_snapshot
            );
        }
    }

    #[inline(never)]
    fn commit_valid_typed_close(fixture: &mut TypedCloseFixture) {
        let before_generation = fixture.ledger.generation();
        let before_c17 = fixture.ledger.c17.generation();
        let before_raw = fixture.ledger.c17.raw_generation_for_test();
        let before_usage = fixture.ledger.usage;
        let before_reserved = fixture.ledger.reserved;
        let before_attached: [[u32; 3]; 4] = std::array::from_fn(|class| {
            std::array::from_fn(|pool| fixture.ledger.c17.attached(class, pool).unwrap())
        });
        let mut measured = work();
        let change = fixture
            .ledger
            .prepare_c17_typed_close(fixture.ledger.generation(), fixture.input, &mut measured)
            .unwrap();
        assert_eq!(
            measured.witness(),
            HotPathWorkWitness::new(crate::c17_layout::WORK_CLOSE)
        );
        fixture.ledger.validate_c17_root_batch(&change).unwrap();
        fixture.ledger.commit_c17_root_batch(change);

        assert_eq!(
            fixture.ledger.generation().get(),
            before_generation.get() + 1
        );
        assert_eq!(fixture.ledger.c17.generation(), before_c17 + 1);
        assert_eq!(fixture.ledger.c17.raw_generation_for_test(), before_raw);
        let mandatory = Mandatory as usize;
        let mut expected_usage = before_usage;
        expected_usage[PENDING][mandatory] -= 1;
        expected_usage[CREDITS][mandatory] -= 1;
        expected_usage[CLAIMS][mandatory] -= 4;
        let mut expected_reserved = before_reserved;
        expected_reserved[PENDING][mandatory] += 4;
        expected_reserved[CREDITS][mandatory] += 4;
        expected_reserved[CLAIMS][mandatory] += 4;
        let mut expected_attached = before_attached;
        expected_attached[PENDING][mandatory] -= 3;
        expected_attached[CREDITS][mandatory] -= 3;
        assert_eq!(fixture.ledger.usage, expected_usage);
        assert_eq!(fixture.ledger.reserved, expected_reserved);
        assert_eq!(
            std::array::from_fn::<_, 4, _>(|class| {
                std::array::from_fn(|pool| fixture.ledger.c17.attached(class, pool).unwrap())
            }),
            expected_attached
        );
        assert_eq!(
            typed_close_owner_cell_currents(&fixture.ledger),
            [[0, 0]; 4]
        );
        for slot in 0..4 {
            assert_eq!(
                fixture
                    .ledger
                    .bundles
                    .get_record(slot)
                    .unwrap()
                    .linked_claims,
                0
            );
            assert_eq!(
                fixture.ledger.c17.owner_currents_for_test(slot).unwrap(),
                (0, [0; 4], false)
            );
        }
    }

    #[inline(never)]
    fn assert_typed_close_after_image_and_replay(fixture: &TypedCloseFixture) {
        let no_requests: Option<&NoTypedCloseRequests> = None;
        let authority_key = plan_authority_key(fixture.plan.identity().id.get());
        let identity = encode_plan_identity(fixture.plan.identity());
        let current_anchor = fixture
            .ledger
            .c17
            .plan_root_anchor(authority_key, identity, 2)
            .unwrap();
        let formation = fixture
            .ledger
            .c17
            .root_formation_for_test(current_anchor)
            .unwrap();
        assert_eq!(formation[40], fixture.input.reason.get());
        assert_eq!(formation[41], c17::SemanticOperation::TypedCloseR as u8);
        assert_eq!(formation[72], 1);
        assert!(formation[73..80].iter().all(|byte| *byte == 0));
        assert_eq!(u64::from_le_bytes(formation[80..88].try_into().unwrap()), 1);
        assert!(formation[88..104].iter().all(|byte| *byte == 0));
        assert_eq!(formation[221], c17::RootState::ClosedPending as u8);
        assert_eq!(formation[222], c17::FormationCause::TypedImpossible as u8);

        let mut stale_work = work();
        assert!(matches!(
            crate::transition_coordinator::prepare_typed_close(
                no_requests,
                &fixture.ledger,
                fixture.input,
                &mut stale_work,
            ),
            Err(
                crate::transition_coordinator::TypedClosePrepareError::Support(
                    SupportLedgerError::Generation
                )
            )
        ));
        assert_eq!(stale_work.witness(), HotPathWorkWitness::new([0; 5]));

        let current_input = crate::TypedCloseInput {
            root: crate::RootRef::new(
                current_anchor.root.slot,
                current_anchor.root.generation,
                current_anchor.version,
            )
            .unwrap(),
            authority: crate::CloseAuthority::Plan {
                identity: fixture.plan.identity(),
                event: crate::PlanCausalEventId::new(2).unwrap(),
            },
            occurred_at: MonotonicTime::from_micros(5),
            ..fixture.input
        };
        let post_close_snapshot = (
            fixture.ledger.generation(),
            fixture.ledger.c17.generation(),
            fixture.ledger.c17.current_counts_for_test(),
            fixture.ledger.usage,
            fixture.ledger.reserved,
            typed_close_owner_cell_currents(&fixture.ledger),
        );
        let mut replay_work = work();
        assert!(matches!(
            crate::transition_coordinator::prepare_typed_close(
                no_requests,
                &fixture.ledger,
                current_input,
                &mut replay_work,
            ),
            Err(
                crate::transition_coordinator::TypedClosePrepareError::Support(
                    SupportLedgerError::InvalidTransition
                )
            )
        ));
        assert_eq!(replay_work.witness(), HotPathWorkWitness::new([0; 5]));
        assert_eq!(
            (
                fixture.ledger.generation(),
                fixture.ledger.c17.generation(),
                fixture.ledger.c17.current_counts_for_test(),
                fixture.ledger.usage,
                fixture.ledger.reserved,
                typed_close_owner_cell_currents(&fixture.ledger),
            ),
            post_close_snapshot
        );
    }

    #[test]
    fn c17_typed_close_validates_plan_authority_and_exactly_dematerializes_pending_root() {
        let mut fixture = typed_close_fixture();
        assert_typed_close_initial_state(&fixture);
        reject_malformed_typed_close_authorities(&fixture);
        reject_drifted_typed_close_authority(&fixture);
        reject_typed_close_work_one_under_without_burn(&fixture);
        commit_valid_typed_close(&mut fixture);
        assert_typed_close_after_image_and_replay(&fixture);
    }

    #[inline(never)]
    fn commit_plan_other_resolution_for_root_test(ledger: &mut PlanLedger, plan: &TurnPlan<4>) {
        let before = ledger.generation();
        let mut measured = work();
        let resolution = ledger
            .prepare_c17_observation_resolution(
                before,
                plan.identity(),
                c17::ObservationResolution::Other,
                MonotonicTime::from_micros(5),
                &mut measured,
            )
            .unwrap();
        assert_eq!(
            measured.witness(),
            HotPathWorkWitness::new(crate::c17_layout::WORK_RESOLVE_OBSERVATION)
        );
        ledger.validate_c17_root_batch(&resolution).unwrap();
        assert_eq!(
            ledger.commit_c17_root_batch(resolution).get(),
            before.get() + 1
        );

        let mandatory = Mandatory as usize;
        assert_eq!(ledger.usage[CONDITIONAL][mandatory], 12);
        assert_eq!(ledger.usage[PENDING][mandatory], 0);
        assert_eq!(ledger.usage[ACTIVE][mandatory], 1);
        assert_eq!(ledger.reserved[CONDITIONAL][mandatory], 28);
        assert_eq!(ledger.usage[CREDITS][mandatory], 13);
        assert_eq!(ledger.reserved[CREDITS][mandatory], 28);
        assert_eq!(ledger.usage[CLAIMS][mandatory], 16);
        assert_eq!(ledger.reserved[CLAIMS][mandatory], 28);
        for slot in 0..4 {
            assert_eq!(ledger.bundles.get_record(slot).unwrap().linked_claims, 1);
        }
    }

    #[inline(never)]
    fn reject_plan_other_resolution_replay_for_root_test(
        ledger: &mut PlanLedger,
        plan: &TurnPlan<4>,
    ) {
        let snapshot = (
            ledger.generation(),
            ledger.c17.generation(),
            ledger.usage,
            ledger.reserved,
            ledger.c17.current_counts_for_test(),
        );
        let mut replay_work = work();
        assert!(matches!(
            ledger.prepare_c17_observation_resolution(
                ledger.generation(),
                plan.identity(),
                c17::ObservationResolution::Other,
                MonotonicTime::from_micros(6),
                &mut replay_work,
            ),
            Err(SupportLedgerError::InvalidTransition)
        ));
        assert_eq!(replay_work.witness(), HotPathWorkWitness::new([0; 5]));
        assert_eq!(
            (
                ledger.generation(),
                ledger.c17.generation(),
                ledger.usage,
                ledger.reserved,
                ledger.c17.current_counts_for_test(),
            ),
            snapshot
        );
    }

    #[test]
    fn c17_observation_other_retires_resolver_and_closes_continuation() {
        let mut ledger = plan_ledger();
        let funders = std::array::from_fn(|index| {
            reserve_plan_bundle(&mut ledger, u8::try_from(index + 1).unwrap())
        });
        let plan = turn_plan(&funders, 1, 1);
        commit_plan_create_for_root_test(&mut ledger, &plan);
        commit_plan_receipt_for_root_test(&mut ledger, &plan);
        commit_plan_begin_for_root_test(&mut ledger, &plan);
        commit_plan_other_resolution_for_root_test(&mut ledger, &plan);
        reject_plan_other_resolution_replay_for_root_test(&mut ledger, &plan);
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
    fn c16_complete_actual_backing_seals_before_nonce_issuance() {
        let expected = SupportBackingCapacities {
            legacy: [18, 18, 18, 36, 19, 18],
            history: [1; 21],
            bundles: [4, 4, 44, 44, 43, 43, 8, 8],
        };
        let storage = support_storage_bytes(1, 18, 18, 21, 4, 8).unwrap();
        assert_eq!(actual_support_storage_bytes(1, expected), Some(storage));
        for backing in 0..33 {
            let mut actual = expected;
            match backing {
                0..=3 => actual.legacy[backing] += 1,
                4..=24 => actual.history[backing - 4] += 1,
                25..=32 => actual.bundles[backing - 25] += 1,
                _ => unreachable!(),
            }
            let dispenser = AtomicU64::new(0);
            assert_eq!(
                seal_backing_and_issue_nonce(&dispenser, 1, storage, expected, actual),
                Err(SupportLedgerError::Storage(FixedStorageError::Capacity)),
                "backing {backing} must seal exactly"
            );
            assert_eq!(
                dispenser.load(Ordering::Relaxed),
                0,
                "a rejected backing cannot burn a nonce"
            );
        }
        let dispenser = AtomicU64::new(0);
        assert_eq!(
            seal_backing_and_issue_nonce(&dispenser, 1, storage, expected, expected),
            Ok(1)
        );
        assert_eq!(dispenser.load(Ordering::Relaxed), 1);
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
    fn c16_generation_exhaustion_rejects_prepare_without_state_change() {
        let mut ledger = bundle_ledger(4, 8);
        ledger.generation = SupportLedgerGeneration::new(u64::MAX).unwrap();
        let cells = configured_cells(3, 1);
        let input = bundle_input(1, &cells);
        let before = bundle_snapshot(&ledger);
        assert!(matches!(
            ledger.prepare_bundle(&input, &mut work()),
            Err(SupportLedgerError::Generation)
        ));
        assert_eq!(bundle_snapshot(&ledger), before);

        ledger.generation = SupportLedgerGeneration::new(1).unwrap();
        reserve_bundle(&mut ledger, 1, 3);
        ledger.generation = SupportLedgerGeneration::new(u64::MAX).unwrap();
        let before = bundle_snapshot(&ledger);
        assert!(matches!(
            ledger.prepare_withdraw(request_owner(1), bundle_entitlement(1), &mut work()),
            Err(SupportLedgerError::Generation)
        ));
        assert!(matches!(
            ledger.prepare_tombstone(
                request_owner(1),
                bundle_entitlement(1),
                MonotonicTime::from_micros(1_000),
                &mut work()
            ),
            Err(SupportLedgerError::Generation)
        ));
        assert_eq!(bundle_snapshot(&ledger), before);
    }

    #[test]
    fn c16_bundle_prepare_binds_exact_before_image() {
        let ledger = bundle_ledger(4, 8);
        let before = ledger.generation();
        let cells = configured_cells(3, 1);
        let input = bundle_input(1, &cells);
        let mut measured = work();
        let change = ledger.prepare_bundle(&input, &mut measured).unwrap();
        assert_eq!(change.work.witness(), HotPathWorkWitness::new([0; 5]));
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
        assert_eq!(std::mem::size_of::<PreparedTombstone<'static, 1>>(), 1_648);
        assert_eq!(
            std::mem::size_of::<ValidatedTombstone<'static, 'static, 64, 64, 1>>(),
            2_720
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
            2_688
        );
        // The same input prepares again against the same before-image.
        let mut measured = work();
        ledger.prepare_bundle(&input, &mut measured).unwrap();
        assert_eq!(measured.witness(), HotPathWorkWitness::new([0; 5]));
    }
    #[test]
    fn c16_terminal_phase_formulas_and_layouts_are_exact() {
        assert_eq!(
            bundle_target_work::<8>(1_344).unwrap(),
            witness([267, 4_032, 0, 0, 274])
        );
        assert_eq!(
            withdraw_remainder_work::<8>(168, 11).unwrap(),
            witness([6_776, 14_240, 0, 0, 4_659])
        );
        assert_eq!(
            bundle_target_work::<8>(1_296).unwrap(),
            witness([267, 3_984, 0, 0, 274])
        );
        assert_eq!(
            tombstone_remainder_work::<8>(168).unwrap(),
            witness([3_492, 4_009, 0, 0, 4_428])
        );
        assert_eq!(std::mem::size_of::<PreparedWithdrawal<'static, 8>>(), 4_032);
        assert_eq!(
            std::mem::size_of::<ValidatedWithdrawal<'static, 'static, 12, 12, 8>>(),
            4_480
        );
        assert_eq!(std::mem::size_of::<PreparedTombstone<'static, 8>>(), 4_000);
        assert_eq!(
            std::mem::size_of::<ValidatedTombstone<'static, 'static, 12, 12, 8>>(),
            5_072
        );
        assert_eq!(
            bundle_target_work::<3>(1_344).unwrap(),
            witness([267, 2_352, 0, 0, 274])
        );
        assert_eq!(
            withdraw_remainder_work::<3>(1, 10).unwrap(),
            witness([6_064, 3_856, 0, 0, 3_277])
        );
        assert_eq!(
            withdraw_remainder_work::<3>(6, 11).unwrap(),
            witness([6_080, 4_136, 0, 0, 3_315])
        );
        assert_eq!(
            bundle_target_work::<3>(1_296).unwrap(),
            witness([267, 2_304, 0, 0, 274])
        );
        assert_eq!(
            tombstone_remainder_work::<3>(1).unwrap(),
            witness([3_115, 2_329, 0, 0, 3_216])
        );
        assert_eq!(
            tombstone_remainder_work::<3>(6).unwrap(),
            witness([3_120, 2_329, 0, 0, 3_246])
        );
    }

    #[test]
    fn c16_terminal_phase_charges_are_atomic_one_under() {
        for tombstone in [false, true] {
            for (dimension, maximum) in [
                (WorkDimension::VisitedEntities, 1_704_575),
                (WorkDimension::CopiedBytes, 2_097_152),
                (WorkDimension::InvariantChecks, 28_708),
            ] {
                let mut ledger = bundle_ledger(4, 8);
                reserve_bundle(&mut ledger, 1, 3);
                let before = bundle_snapshot(&ledger);
                let before_usage = ledger.usage;
                let before_reserved = ledger.reserved;
                let before_vector_usage = ledger.vector_usage;
                let before_c17 = ledger.c17.generation();
                let required = WORK_TOMBSTONE[dimension as usize];
                let mut meter = work();
                meter.record(dimension, maximum - required + 1).unwrap();
                let before_work = meter.witness();

                let error = if tombstone {
                    let change = ledger
                        .prepare_tombstone(
                            request_owner(1),
                            bundle_entitlement(1),
                            MonotonicTime::from_micros(1_000),
                            &mut meter,
                        )
                        .unwrap();
                    assert_eq!(change.work.witness(), before_work);
                    ledger.validate_tombstone(change).unwrap_err()
                } else {
                    let change = ledger
                        .prepare_withdraw(request_owner(1), bundle_entitlement(1), &mut meter)
                        .unwrap();
                    assert_eq!(change.work.witness(), before_work);
                    ledger.validate_withdraw(change).unwrap_err()
                };
                assert_eq!(
                    error,
                    SupportLedgerError::Storage(FixedStorageError::Work(
                        WorkBudgetError::BudgetExceeded(dimension, maximum, maximum + 1)
                    ))
                );
                assert_eq!(meter.witness(), before_work, "fixed charge is all-or-none");
                assert_eq!(bundle_snapshot(&ledger), before);
                assert_eq!(ledger.usage, before_usage);
                assert_eq!(ledger.reserved, before_reserved);
                assert_eq!(ledger.vector_usage, before_vector_usage);
                assert_eq!(ledger.c17.generation(), before_c17);
            }
        }
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
    fn c17_migrated_bundle_work_exhaustion_rolls_back() {
        let cells = configured_cells(3, 1);
        let input = bundle_input(1, &cells);
        for (dimension, maximum) in [
            (WorkDimension::VisitedEntities, 1_704_575),
            (WorkDimension::CopiedBytes, 2_097_152),
            (WorkDimension::InvariantChecks, 28_708),
        ] {
            let mut ledger = bundle_ledger(4, 8);
            let before = bundle_snapshot(&ledger);
            let required = WORK_MIGRATED_C16[dimension as usize];
            let initial = maximum - required + 1;
            let mut exhausted = work();
            exhausted.record(dimension, initial).unwrap();
            let change = ledger.prepare_bundle(&input, &mut exhausted).unwrap();
            let before_charge = change.work.witness();
            assert_eq!(
                ledger.validate_bundle(change).unwrap_err(),
                SupportLedgerError::Storage(FixedStorageError::Work(
                    WorkBudgetError::BudgetExceeded(dimension, maximum, maximum + 1)
                ))
            );
            assert_eq!(exhausted.witness(), before_charge);
            assert_eq!(bundle_snapshot(&ledger), before);
        }
    }
    #[test]
    fn c17_migrated_bundle_charge_is_atomic_one_under() {
        let cells = configured_cells(3, 1);
        let input = bundle_input(1, &cells);
        for (dimension, maximum) in [
            (WorkDimension::VisitedEntities, 1_704_575),
            (WorkDimension::CopiedBytes, 2_097_152),
            (WorkDimension::InvariantChecks, 28_708),
        ] {
            let mut ledger = bundle_ledger(4, 8);
            let initial = maximum - WORK_MIGRATED_C16[dimension as usize] + 1;
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
            assert_eq!(meter.witness(), after_prepare, "charge is all-or-none");
            assert!(ledger.bundles.is_empty());
        }
    }

    #[test]
    fn c16_capability_compile_fail_contracts() {
        use std::fs;
        use std::path::Path;
        use std::process::Command;
        use std::time::{SystemTime, UNIX_EPOCH};

        fn copy_source(from: &Path, to: &Path) {
            fs::create_dir_all(to).unwrap();
            for entry in fs::read_dir(from).unwrap() {
                let entry = entry.unwrap();
                let destination = to.join(entry.file_name());
                if entry.file_type().unwrap().is_dir() {
                    copy_source(&entry.path(), &destination);
                } else {
                    fs::copy(entry.path(), destination).unwrap();
                }
            }
        }

        let root = std::env::temp_dir().join(format!(
            "turnvector-c16-compile-fail-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let source = root.join("src");
        copy_source(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &source);
        fs::create_dir(root.join("out")).unwrap();
        let original = fs::read_to_string(source.join("support.rs")).unwrap();
        let probes = [
            (
                "fresh_meter_validate",
                "let mut ledger = ledger();\nlet change = prepared_static();\nlet mut fresh = WorkMeter::new(crate::HotPathWorkBudget::binary_maximum());\nlet _ = ledger.validate_bundle(change, &mut fresh);",
                "E0061",
            ),
            (
                "two_exclusive_capabilities",
                "let mut ledger = ledger();\nlet first = prepared_static();\nlet second = prepared_static();\nlet first = ledger.validate_bundle(first).unwrap();\nlet second = ledger.validate_bundle(second).unwrap();\ndrop((first, second));",
                "E0499",
            ),
            (
                "use_after_commit",
                "let change = validated_static();\nchange.commit_bundle();\nchange.commit_bundle();",
                "E0382",
            ),
            (
                "bound_meter_while_prepared",
                "fn prepared<'work>(work: &'work mut WorkMeter) -> BundleChange<'static, 'work, 1> { let _ = work; todo!() }\nlet mut meter = WorkMeter::new(crate::HotPathWorkBudget::binary_maximum());\nlet change = prepared(&mut meter);\nlet _ = meter.record(WorkDimension::InvariantChecks, 1);\ndrop(change);",
                "E0499",
            ),
            (
                "bound_meter_while_validated",
                "fn validated<'ledger, 'work>(ledger: &'ledger mut Ledger, work: &'work mut WorkMeter) -> ValidatedBundleChange<'ledger, 'static, 'work, 64, 64, 1> { let _ = (ledger, work); todo!() }\nlet mut ledger = ledger();\nlet mut meter = WorkMeter::new(crate::HotPathWorkBudget::binary_maximum());\nlet change = validated(&mut ledger, &mut meter);\nlet _ = meter.record(WorkDimension::InvariantChecks, 1);\ndrop(change);",
                "E0499",
            ),
        ];
        let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
        for (name, body, diagnostic) in probes {
            let module = format!(
                "\nmod c16_compile_fail_probe {{\nuse super::*;\ntype Ledger = SupportChargeLedger<64, 64, 1>;\nfn ledger() -> Ledger {{ todo!() }}\nfn prepared_static() -> BundleChange<'static, 'static, 1> {{ todo!() }}\nfn validated_static() -> ValidatedBundleChange<'static, 'static, 'static, 64, 64, 1> {{ todo!() }}\nfn probe() {{\n{body}\n}}\n}}\n"
            );
            fs::write(source.join("support.rs"), format!("{original}{module}")).unwrap();
            let output = Command::new(&rustc)
                .current_dir(&root)
                .args([
                    "--crate-name",
                    "turnvector_c16_compile_probe",
                    "--edition=2024",
                    "--crate-type=lib",
                    "src/lib.rs",
                    "--out-dir",
                    "out",
                ])
                .output()
                .unwrap();
            let stderr = String::from_utf8(output.stderr).unwrap();
            assert!(
                !output.status.success(),
                "probe {name} unexpectedly compiled"
            );
            assert!(
                stderr.contains(diagnostic),
                "probe {name} missed intended {diagnostic}:\n{stderr}"
            );
            assert!(
                !stderr.contains("E0603"),
                "probe {name} failed on privacy instead of ownership:\n{stderr}"
            );
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn c16_bundle_transaction_commits_exactly_once() {
        let mut ledger = bundle_ledger(4, 8);
        let cells = configured_cells(3, 1);
        let input = bundle_input(1, &cells);
        let before = ledger.generation();
        let mut measured = work();
        let change = ledger.prepare_bundle(&input, &mut measured).unwrap();
        assert_eq!(change.work.witness(), HotPathWorkWitness::default());
        let validated = ledger
            .validate_bundle(change)
            .expect("same-instance same-state validation");
        assert_eq!(
            validated.change.work.witness(),
            HotPathWorkWitness::new(WORK_MIGRATED_C16)
        );
        let next = validated.commit_bundle();
        assert_eq!(
            measured.witness(),
            HotPathWorkWitness::new(WORK_MIGRATED_C16),
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
        for key in record.tagged_keys() {
            ledger
                .c17
                .validate_c16_raw_reciprocity(key.identity, owner.unwrap(), record)
                .unwrap();
        }
        assert_eq!(ledger.c17.generation(), 2);
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
        let starts = [bounds; 21];
        let mut ledger = H2Ledger::try_new(
            SupportLedgerGeneration::new(1).unwrap(),
            [[6; POOLS]; 5],
            2,
            starts,
            LifecycleReserveMaxima([1, 2, 2, 1, 1]),
            4,
            8,
            6,
            c18::SupportHistoryLimits::testing(starts),
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
    fn c16_bundle_logical_capacity_is_nonborrowable_across_all_classes_and_pools() {
        for class in 0..5 {
            for pool in [Ordinary, Mandatory, Safety] {
                let mut ledger = bundle_ledger(4, 8);
                let cells = [oc(SupportOperation::DescribeModel, pool, 10, 1)];
                let input = bundle_input(1, &cells);
                let delta = ledger.bundle_logical_delta(cells).unwrap();
                let pool_index = pool as usize;
                let required = delta.usage[class][pool_index] + delta.reserved[class][pool_index];
                assert!(required > 0, "matrix cell has a real delta");
                ledger.capacities[class][pool_index] = required - 1;
                for peer in 0..POOLS {
                    if peer != pool_index {
                        ledger.capacities[class][peer] = u32::MAX;
                    }
                }
                let before = bundle_snapshot(&ledger);
                assert_eq!(
                    ledger.prepare_bundle(&input, &mut work()).unwrap_err(),
                    CAPACITY_ERROR,
                    "class {class}, pool {pool_index} cannot borrow"
                );
                assert_eq!(bundle_snapshot(&ledger), before);
            }
        }
    }

    #[test]
    fn c16_bundle_logical_capacity_accumulates_across_records() {
        let mut ledger = bundle_ledger(4, 8);
        ledger.vector_capacity[SupportOperation::DescribeModel as usize * POOLS][0] = 3;
        let cells = [oc(SupportOperation::DescribeModel, Ordinary, 10, 2)];
        let first = bundle_input(1, &cells);
        let mut meter = work();
        let change = ledger.prepare_bundle(&first, &mut meter).unwrap();
        ledger.validate_bundle(change).unwrap().commit_bundle();
        let usage = ledger.vector_usage;
        let second = bundle_input(2, &cells);
        assert_eq!(
            ledger.prepare_bundle(&second, &mut work()).unwrap_err(),
            CAPACITY_ERROR
        );
        assert_eq!(
            ledger.vector_usage, usage,
            "failed aggregate reserve is read-only"
        );
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
            Ledger::try_new(
                generation,
                capacities,
                2,
                starts,
                maxima,
                4,
                4_000,
                22,
                c18::SupportHistoryLimits::testing(starts),
            )
            .unwrap_err(),
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
        let valid = spec(7, 8, Ordinary, &[Reserved([7; 32])]);
        ledger
            .reserve(ledger.generation(), valid, &mut measured)
            .unwrap();
        assert_eq!(
            measured.witness(),
            HotPathWorkWitness::new([1_134, 1_616_904, 0, 0, 1_060])
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
            HotPathWorkWitness::new([1_134, 1_616_904, 0, 0, 1_069])
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
    #[test]
    fn c16_initial_transitions_are_legacy_first_exact_and_sibling_isolated() {
        let mut ledger = bundle_ledger(4, 8);
        let cells = configured_cells(3, 1);
        let input = bundle_input(1, &cells);
        let initial = input.initial.values();
        let obligation = reserve_bundle(&mut ledger, 1, 3);
        let before = *ledger.bundles.get_record(0).unwrap();
        let mut meter = work();
        ledger
            .transition(
                ledger.generation(),
                obligation,
                PredecessorEnded(initial[0].predecessor, MonotonicTime::from_micros(5)),
                &mut meter,
            )
            .unwrap();
        assert_eq!(meter.witness(), witness([1_333, 1_616_904, 0, 0, 1_326]));
        let pending = *ledger.bundles.get_record(0).unwrap();
        assert_eq!(pending.state, BundleState::LiveConsumed);
        assert_eq!(pending.initial[0].state, Pending);
        assert_eq!(pending.initial[0].state_time, MonotonicTime::from_micros(5));
        assert_eq!(pending.initial[1..], before.initial[1..]);
        assert_eq!(ledger.reserved[PENDING][Mandatory as usize], 2);
        assert_eq!(ledger.reserved[ACTIVE][Mandatory as usize], 3);

        let mut meter = work();
        ledger
            .transition(
                ledger.generation(),
                obligation,
                Begin(MonotonicTime::from_micros(6)),
                &mut meter,
            )
            .unwrap();
        assert_eq!(meter.witness(), witness([1_333, 1_616_904, 0, 0, 1_326]));
        let active = *ledger.bundles.get_record(0).unwrap();
        assert_eq!(active.initial[0].state, Active);
        assert_eq!(active.initial[0].state_time, MonotonicTime::from_micros(6));
        assert_eq!(active.initial[1..], before.initial[1..]);
        assert_eq!(ledger.reserved[ACTIVE][Mandatory as usize], 2);

        let sibling = initial[1].obligation;
        ledger
            .transition(
                ledger.generation(),
                sibling,
                CloseCausalCallImpossible(MonotonicTime::from_micros(1_000)),
                &mut work(),
            )
            .unwrap();
        let closed = *ledger.bundles.get_record(0).unwrap();
        assert_eq!(closed.initial[1].state, ClosedConditional);
        assert_eq!(closed.initial[0], active.initial[0]);
        assert_eq!(closed.initial[2], before.initial[2]);
        assert_eq!(ledger.reserved[PENDING][Mandatory as usize], 1);
        assert_eq!(ledger.reserved[ACTIVE][Mandatory as usize], 1);

        let mut meter = work();
        let change = ledger
            .prepare(
                ledger.generation(),
                SupportChangeInput::FinishActive(obligation, MonotonicTime::from_micros(1_000)),
                &mut meter,
            )
            .unwrap();
        assert_eq!(meter.witness(), witness([0; 5]));
        ledger.validate(&change).unwrap();
        ledger.commit(change, &mut meter).unwrap();
        assert_eq!(meter.witness(), witness([1_333, 1_616_904, 0, 0, 1_326]));
        assert_eq!(
            ledger.bundles.get_record(0).unwrap().initial[0].state,
            Retained
        );

        let snapshot = bundle_snapshot(&ledger);
        let mut missing = work();
        assert_eq!(
            ledger.transition(
                ledger.generation(),
                SupportOperationObligationId::new([250; 32]).unwrap(),
                CloseCausalCallImpossible(MonotonicTime::from_micros(1_000)),
                &mut missing,
            ),
            Err(SupportLedgerError::InvalidTransition)
        );
        assert_eq!(missing.witness(), witness([0; 5]));
        assert_eq!(bundle_snapshot(&ledger), snapshot);
    }

    #[test]
    fn c16_initial_transition_revalidates_each_immutable_semantic_envelope() {
        for ordinal in 0..3 {
            for corruption in 0..8 {
                let mut ledger = bundle_ledger(4, 8);
                let cells = configured_cells(3, 1);
                let input = bundle_input(1, &cells);
                let initial = input.initial.values();
                reserve_bundle(&mut ledger, 1, 3);
                let record = match &mut ledger.bundles.records[0] {
                    RecordSlot::Occupied(record) => record,
                    RecordSlot::Vacant { .. } => unreachable!("reserved fixture record"),
                };
                let item = &mut record.initial[ordinal];
                match corruption {
                    0 => item.operation = SupportOperation::DescribeModel,
                    1 => item.pool = Ordinary,
                    2 => item.predecessor = SupportCausalPredecessorId([0; 32]),
                    3 => item.scope = SupportCallScopeId([0; 32]),
                    4 => item.prospective_bound = Duration::from_micros(0),
                    5 => item.state_time = MonotonicTime::from_micros(1),
                    6 => item.state = ClosedConditional,
                    7 => record.state = BundleState::RetainedTombstone,
                    _ => unreachable!(),
                }
                let before_store = ledger.bundles.clone();
                let before_generation = ledger.generation();
                let before_usage = ledger.usage;
                let before_reserved = ledger.reserved;
                let mut measured = work();
                assert_eq!(
                    ledger.transition(
                        before_generation,
                        initial[ordinal].obligation,
                        PredecessorEnded(
                            initial[ordinal].predecessor,
                            MonotonicTime::from_micros(5),
                        ),
                        &mut measured,
                    ),
                    Err(InvalidTransition),
                    "ordinal {ordinal}, corruption {corruption}"
                );
                assert_eq!(measured.witness(), witness([0; 5]));
                assert_eq!(ledger.generation(), before_generation);
                assert_eq!(ledger.usage, before_usage);
                assert_eq!(ledger.reserved, before_reserved);
                assert_eq!(ledger.bundles, before_store);
            }
        }
    }

    #[test]
    fn c16_initial_claim_lookup_binds_request_owner_and_compact_leaf() {
        let facts = bundle_input(1, &[]);
        let claims = facts.initial.values().map(|requirement| requirement.claim);
        let owner = facts.request_owner;
        let wrong_owner = bundle_input(2, &[]).request_owner;
        let make = || {
            let mut ledger = bundle_ledger(4, 8);
            reserve_bundle(&mut ledger, 1, 3);
            ledger
        };

        let ledger = make();
        for (ordinal, claim) in claims.into_iter().enumerate() {
            assert_eq!(
                ledger.find_initial_claim_precharged(claim, owner),
                Ok((0, ordinal as u8))
            );
        }
        let before = bundle_snapshot(&ledger);
        assert_eq!(
            ledger.find_initial_claim_precharged(claims[0], wrong_owner),
            Err(SupportLedgerError::InvalidTransition)
        );
        assert_eq!(bundle_snapshot(&ledger), before);

        let mut ledger = make();
        let (leaf, _) = ledger
            .bundles
            .route_precharged(TAG_ADMISSION_CLAIM, &claims[0].get())
            .unwrap();
        ledger.bundles.identities.leaf_slots[leaf as usize] = LeafSlot::Occupied {
            owner_record: u32::MAX,
            key_ordinal: 6,
        };
        assert_eq!(
            ledger.find_initial_claim_precharged(claims[0], owner),
            Err(SupportLedgerError::Storage(FixedStorageError::NonCanonical))
        );

        let mut ledger = make();
        let (leaf, _) = ledger
            .bundles
            .route_precharged(TAG_ADMISSION_CLAIM, &claims[0].get())
            .unwrap();
        ledger.bundles.identities.leaf_slots[leaf as usize] = LeafSlot::Occupied {
            owner_record: 0,
            key_ordinal: K as u8,
        };
        assert_eq!(
            ledger.find_initial_claim_precharged(claims[0], owner),
            Err(SupportLedgerError::Storage(FixedStorageError::NonCanonical))
        );

        let mut ledger = make();
        let RecordSlot::Occupied(record) = &mut ledger.bundles.records[0] else {
            unreachable!("fixture bundle occupies record zero")
        };
        record.initial[0].claim = AdmissionInitialClaimId::new([201; 32]).unwrap();
        assert_eq!(
            ledger.find_initial_claim_precharged(claims[0], owner),
            Err(SupportLedgerError::InvalidTransition)
        );
    }

    /// A retained tombstone must return everything the bundle held, not only
    /// its storage: releasing it while its logical occupancy, reserves, vector
    /// cells or unified owner rows stay charged leaks capacity permanently.
    #[test]
    fn a_released_tombstone_returns_capacity_vectors_and_owners() {
        let mut ledger = bundle_ledger(4, 8);
        let empty = (ledger.usage, ledger.reserved, ledger.vector_usage);
        let occupied_before = ledger.bundles.occupied_records;

        reserve_bundle(&mut ledger, 1, 3);
        assert_ne!(
            (ledger.usage, ledger.reserved, ledger.vector_usage),
            empty,
            "the bundle charges capacity"
        );
        tombstone_bundle(&mut ledger, 1);
        assert_ne!(
            (ledger.usage, ledger.reserved, ledger.vector_usage),
            empty,
            "a retained tombstone still charges capacity"
        );

        let scheduled = ledger.c18.scheduled().to_vec();
        assert_eq!(scheduled.len(), 1, "the tombstone scheduled its release");
        assert_eq!(scheduled[0].family, c18::OwnerFamily::Tombstone);

        let mut meter = work();
        let prepared = ledger
            .prepare_expiry::<1, 1>(ledger.generation(), scheduled[0].release_at, &mut meter)
            .unwrap();
        let commit = ledger.validate_expiry(prepared).unwrap().commit();
        assert_eq!(commit.released_groups, 1);

        assert_eq!(
            (ledger.usage, ledger.reserved, ledger.vector_usage),
            empty,
            "every charged unit returned"
        );
        assert_eq!(
            ledger.bundles.occupied_records, occupied_before,
            "the physical record returned"
        );
        assert_eq!(ledger.c18.scheduled(), &[], "the ticket is consumed");
    }

    fn tombstone_bundle(ledger: &mut Ledger, n: u8) -> SupportLedgerGeneration {
        let mut meter = work();
        let change = ledger
            .prepare_tombstone(
                request_owner(n),
                bundle_entitlement(n),
                MonotonicTime::from_micros(1_000),
                &mut meter,
            )
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
            .prepare_tombstone(
                request_owner(1),
                bundle_entitlement(1),
                MonotonicTime::from_micros(1_000),
                &mut meter,
            )
            .unwrap();
        ledger.validate_tombstone(change).unwrap();
        assert_eq!(bundle_snapshot(&ledger), before);

        let mut ledger = bundle_ledger(4, 8);
        reserve_bundle(&mut ledger, 1, 3);
        let mut meter = work();
        let change = ledger
            .prepare_tombstone(
                request_owner(1),
                bundle_entitlement(1),
                MonotonicTime::from_micros(1_000),
                &mut meter,
            )
            .unwrap();
        add(&mut ledger, 9, 9).unwrap();
        assert_eq!(
            ledger.validate_tombstone(change).unwrap_err(),
            SupportLedgerError::Generation
        );
    }

    #[test]
    fn c16_committed_tombstone_retains_raw_owners_and_advances_once() {
        let mut ledger = bundle_ledger(4, 8);
        let obligation = reserve_bundle(&mut ledger, 1, 3);
        let before_generation = ledger.generation();
        let before_c17 = ledger.c17.generation();
        let before_usage = ledger.usage;
        let before_reserved = ledger.reserved;
        let before_vector_usage = ledger.vector_usage;
        let before_free = (
            ledger.bundles.free_record_len(),
            ledger.bundles.free_cell_len(),
            ledger.bundles.free_leaf_len(),
            ledger.bundles.free_branch_len(),
        );

        let mut meter = work();
        let change = ledger
            .prepare_tombstone(
                request_owner(1),
                bundle_entitlement(1),
                MonotonicTime::from_micros(1_000),
                &mut meter,
            )
            .unwrap();
        assert_eq!(change.work.witness(), HotPathWorkWitness::default());
        let validated = ledger.validate_tombstone(change).unwrap();
        assert_eq!(
            validated.change.work.witness(),
            HotPathWorkWitness::new(WORK_TOMBSTONE)
        );
        let committed = validated.commit_tombstone();
        assert_eq!(meter.witness(), HotPathWorkWitness::new(WORK_TOMBSTONE));
        assert_eq!(committed, before_generation.next().unwrap());
        assert_eq!(ledger.generation(), committed);
        assert_eq!(ledger.c17.generation(), before_c17 + 1);
        assert_eq!(ledger.usage, before_usage);
        assert_eq!(ledger.reserved, before_reserved);
        assert_eq!(ledger.vector_usage, before_vector_usage);
        assert_eq!(
            (
                ledger.bundles.free_record_len(),
                ledger.bundles.free_cell_len(),
                ledger.bundles.free_leaf_len(),
                ledger.bundles.free_branch_len(),
            ),
            before_free
        );

        let owner = ledger
            .bundles
            .find(TAG_OBLIGATION, &obligation.get(), &mut work())
            .unwrap()
            .expect("retained obligation owner");
        let record = *ledger.bundles.get_record(owner).unwrap();
        assert_eq!(record.state, BundleState::RetainedTombstone);
        for key in record.tagged_keys() {
            ledger
                .c17
                .validate_c16_raw_reciprocity(key.identity, owner, &record)
                .unwrap();
        }

        let mut rejected_work = work();
        let rejected = ledger
            .prepare_withdraw(request_owner(1), bundle_entitlement(1), &mut rejected_work)
            .unwrap();
        assert_eq!(
            ledger.validate_withdraw(rejected).unwrap_err(),
            SupportLedgerError::InvalidTransition
        );
        assert_eq!(rejected_work.witness(), HotPathWorkWitness::default());
    }

    #[test]
    fn c16_bundle_pristine_withdraw_commits_once_and_releases() {
        let mut ledger = bundle_ledger(4, 8);
        let obligation = reserve_bundle(&mut ledger, 1, 3);
        let before = ledger.generation();
        let mut measured = work();
        let change = ledger
            .prepare_withdraw(request_owner(1), bundle_entitlement(1), &mut measured)
            .unwrap();
        assert_eq!(change.work.witness(), HotPathWorkWitness::default());
        let validated = ledger
            .validate_withdraw(change)
            .expect("same-instance same-state validation");
        assert_eq!(
            validated.change.work.witness(),
            HotPathWorkWitness::new(WORK_TOMBSTONE)
        );
        assert_eq!(std::mem::size_of::<PreparedWithdrawal<'static, 1>>(), 1_680);
        assert_eq!(
            std::mem::size_of::<ValidatedWithdrawal<'static, 'static, 12, 12, 1>>(),
            2_128
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
            .prepare_withdraw(request_owner(1), bundle_entitlement(1), &mut measured)
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
            .prepare_withdraw(request_owner(1), bundle_entitlement(1), &mut measured)
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
            .prepare_withdraw(request_owner(1), bundle_entitlement(1), &mut measured)
            .unwrap();
        ledger.validate_withdraw(change).unwrap();
        assert_eq!(bundle_snapshot(&ledger), before);
        // An unknown obligation rejects during prepare.
        assert_eq!(
            ledger
                .prepare_withdraw(
                    request_owner(1),
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
        let change = ledger
            .prepare_withdraw(request_owner(1), bundle_entitlement(1), &mut measured)
            .unwrap();
        assert_eq!(
            ledger.validate_withdraw(change).unwrap_err(),
            SupportLedgerError::InvalidTransition
        );
        // Double close rejects during the remainder validation phase.
        let mut tombstone_meter = work();
        let change = ledger
            .prepare_tombstone(
                request_owner(1),
                bundle_entitlement(1),
                MonotonicTime::from_micros(1_000),
                &mut tombstone_meter,
            )
            .unwrap();
        assert_eq!(
            ledger.validate_tombstone(change).unwrap_err(),
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
            .prepare_withdraw(request_owner(1), bundle_entitlement(1), &mut measured)
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
    fn c16_terminal_validation_binds_authoritative_request_owner() {
        for tombstone in [false, true] {
            let mut ledger = bundle_ledger(4, 8);
            reserve_bundle(&mut ledger, 1, 3);
            let before = bundle_snapshot(&ledger);
            let mut meter = work();
            let error = if tombstone {
                let change = ledger
                    .prepare_tombstone(
                        request_owner(2),
                        bundle_entitlement(1),
                        MonotonicTime::from_micros(1_000),
                        &mut meter,
                    )
                    .unwrap();
                ledger.validate_tombstone(change).unwrap_err()
            } else {
                let change = ledger
                    .prepare_withdraw(request_owner(2), bundle_entitlement(1), &mut meter)
                    .unwrap();
                ledger.validate_withdraw(change).unwrap_err()
            };
            assert_eq!(error, SupportLedgerError::InvalidTransition);
            assert_eq!(meter.witness(), HotPathWorkWitness::default());
            assert_eq!(bundle_snapshot(&ledger), before);
        }
    }

    #[test]
    fn c16_terminal_validation_rejects_complete_semantic_corruption_matrix() {
        let corrupt = |record: &mut BundleRecord, corruption: usize| match corruption {
            0 => record.initial[0].operation = SupportOperation::ReleaseRequest,
            1 => record.initial[1].operation = SupportOperation::MaterializeRequest,
            2 => record.initial[2].operation = SupportOperation::FormCandidates,
            3 => record.initial[0].pool = Ordinary,
            4 => record.initial[1].obligation = record.initial[0].obligation,
            5 => record.initial[1].credit = record.initial[0].credit,
            6 => record.initial[1].claim = record.initial[0].claim,
            7 => record.initial[0].predecessor = SupportCausalPredecessorId([0; 32]),
            8 => record.initial[0].scope = SupportCallScopeId([0; 32]),
            9 => record.initial[0].input_bucket = SupportInputBucket(0),
            10 => record.initial[0].prospective_bound = Duration::from_micros(0),
            11 => record.initial[0].state_time = MonotonicTime::from_micros(1),
            12 => record.initial[0].state = ClosedConditional,
            13 => record.timing_commitment = TimingCommitmentId([0; 32]),
            14 => record.request_closure = RequestClosureId([0; 32]),
            15 => record.support_budget = OwnerThreadSupportBudgetId([0; 32]),
            16 => record.branches[0].operation = SupportOperation::FormCandidates,
            17 => record.branches[1].operation = SupportOperation::ObserveTurnReceipt,
            18 => record.branches[2].operation = SupportOperation::ObserveTurnReceipt,
            19 => record.branches[3].operation = SupportOperation::ObserveTurnReceipt,
            20 => record.branches[0].pool = Ordinary,
            21 => record.branches[0].input_bucket = SupportInputBucket(0),
            22 => record.branches[0].prospective_bound = Duration::from_micros(0),
            _ => unreachable!(),
        };

        for tombstone in [false, true] {
            for corruption in 0..23 {
                let mut ledger = bundle_ledger(4, 8);
                reserve_bundle(&mut ledger, 1, 3);
                let RecordSlot::Occupied(record) = &mut ledger.bundles.records[0] else {
                    unreachable!("fixture record is occupied")
                };
                corrupt(record, corruption);
                let before_store = ledger.bundles.clone();
                let before_generation = ledger.generation();
                let before_usage = ledger.usage;
                let before_reserved = ledger.reserved;
                let before_vector_usage = ledger.vector_usage;
                let expected_error = SupportLedgerError::InvalidTransition;
                let mut meter = work();
                let error = if tombstone {
                    let change = ledger
                        .prepare_tombstone(
                            request_owner(1),
                            bundle_entitlement(1),
                            MonotonicTime::from_micros(1_000),
                            &mut meter,
                        )
                        .unwrap();
                    ledger
                        .validate_tombstone(change)
                        .err()
                        .unwrap_or_else(|| panic!("accepted tombstone corruption {corruption}"))
                } else {
                    let change = ledger
                        .prepare_withdraw(request_owner(1), bundle_entitlement(1), &mut meter)
                        .unwrap();
                    ledger
                        .validate_withdraw(change)
                        .err()
                        .unwrap_or_else(|| panic!("accepted withdrawal corruption {corruption}"))
                };
                assert_eq!(
                    error, expected_error,
                    "tombstone {tombstone}, corruption {corruption}"
                );
                assert_eq!(
                    meter.witness(),
                    HotPathWorkWitness::default(),
                    "tombstone {tombstone}, corruption {corruption} burns no Work"
                );
                assert_eq!(ledger.generation(), before_generation);
                assert_eq!(ledger.usage, before_usage);
                assert_eq!(ledger.reserved, before_reserved);
                assert_eq!(ledger.vector_usage, before_vector_usage);
                assert_eq!(ledger.bundles, before_store);
            }
        }
    }

    #[test]
    fn c16_terminal_validation_rejects_cell_record_and_aggregate_corruption() {
        let occupied =
            |ledger: &Ledger, index: u32| match ledger.bundles.cells.slots[index as usize] {
                CellSlot::Occupied {
                    owner_record,
                    cell,
                    next_owned,
                    ..
                } => (owner_record, cell, next_owned),
                CellSlot::Vacant { .. } => unreachable!("fixture chain is occupied"),
            };
        let reject = |ledger: &mut Ledger| {
            let before = bundle_snapshot(ledger);
            let mut meter = work();
            let change = ledger
                .prepare_withdraw(request_owner(1), bundle_entitlement(1), &mut meter)
                .unwrap();
            assert!(ledger.validate_withdraw(change).is_err());
            assert_eq!(
                meter.witness(),
                HotPathWorkWitness::default(),
                "semantic corruption burns no Work"
            );
            assert_eq!(bundle_snapshot(ledger), before);
        };

        for corruption in 0..13 {
            let mut ledger = bundle_ledger(4, 8);
            reserve_bundle(&mut ledger, 1, 3);
            let record = *ledger.bundles.get_record(0).unwrap();
            let first = record.vector_head;
            let (_, _, middle) = occupied(&ledger, first);
            let (_, _, tail) = occupied(&ledger, middle);
            match corruption {
                0 => {
                    let RecordSlot::Occupied(record) = &mut ledger.bundles.records[0] else {
                        unreachable!("fixture record is occupied")
                    };
                    record.vector_head = u32::MAX - 1;
                }
                1 => {
                    let RecordSlot::Occupied(record) = &mut ledger.bundles.records[0] else {
                        unreachable!("fixture record is occupied")
                    };
                    record.vector_len += 1;
                }
                2 => {
                    let (owner_record, cell, _) = occupied(&ledger, first);
                    ledger.bundles.cells.slots[first as usize] = CellSlot::Occupied {
                        owner_record,
                        cell,
                        current: 0,
                        next_owned: NO_NODE,
                    };
                }
                3 => {
                    let (owner_record, cell, _) = occupied(&ledger, tail);
                    ledger.bundles.cells.slots[tail as usize] = CellSlot::Occupied {
                        owner_record,
                        cell,
                        current: 0,
                        next_owned: first,
                    };
                }
                4 => {
                    let (_, cell, next_owned) = occupied(&ledger, middle);
                    ledger.bundles.cells.slots[middle as usize] = CellSlot::Occupied {
                        owner_record: 1,
                        cell,
                        current: 0,
                        next_owned,
                    };
                }
                5 => {
                    ledger.bundles.cells.slots[middle as usize] =
                        CellSlot::Vacant { free_position: 0 };
                }
                6 => {
                    let (owner_record, cell, _) = occupied(&ledger, middle);
                    ledger.bundles.cells.slots[middle as usize] = CellSlot::Occupied {
                        owner_record,
                        cell,
                        current: 0,
                        next_owned: middle,
                    };
                }
                7 => {
                    let (owner_record, _, next_owned) = occupied(&ledger, middle);
                    let (_, first_cell, _) = occupied(&ledger, first);
                    ledger.bundles.cells.slots[middle as usize] = CellSlot::Occupied {
                        owner_record,
                        cell: first_cell,
                        current: 0,
                        next_owned,
                    };
                }
                8 => {
                    let (owner_record, mut cell, next_owned) = occupied(&ledger, first);
                    cell.max_outstanding = 0;
                    ledger.bundles.cells.slots[first as usize] = CellSlot::Occupied {
                        owner_record,
                        cell,
                        current: 0,
                        next_owned,
                    };
                }
                9 => ledger.vector_usage[0][0] = 0,
                10 => ledger.usage[CONDITIONAL][Mandatory as usize] = 2,
                11 => ledger.reserved[PENDING][Mandatory as usize] = 2,
                12 => {
                    let RecordSlot::Occupied(record) = &mut ledger.bundles.records[0] else {
                        unreachable!("fixture record is occupied")
                    };
                    record.linked_claims = 1;
                }
                _ => unreachable!(),
            }
            reject(&mut ledger);
        }

        let mut ledger = bundle_ledger(4, 8);
        reserve_bundle(&mut ledger, 1, 3);
        let record = *ledger.bundles.get_record(0).unwrap();
        let first = record.vector_head;
        let (_, _, middle) = occupied(&ledger, first);
        let (owner_record, _, next_owned) = occupied(&ledger, middle);
        let (_, first_cell, _) = occupied(&ledger, first);
        ledger.bundles.cells.slots[middle as usize] = CellSlot::Occupied {
            owner_record,
            cell: first_cell,
            current: 0,
            next_owned,
        };
        let mut meter = work();
        let change = ledger
            .prepare_tombstone(
                request_owner(1),
                bundle_entitlement(1),
                MonotonicTime::from_micros(1_000),
                &mut meter,
            )
            .unwrap();
        assert!(ledger.validate_tombstone(change).is_err());
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
        let mut legacy_credit = [1; 32];
        legacy_credit[31] ^= 0x80;
        legacy_blocks(&mut ledger, [1; 32], legacy_credit);
        // Prepared ordinary keys block a later C16 bundle.
        let mut ledger = ordinary_ledger();
        begin(&mut ledger, ordinary((1, 2, 3, Reserved([4; 32]))), at(1)).unwrap();
        legacy_blocks(&mut ledger, [1; 32], [2; 32]);
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
        assert_eq!(std::mem::size_of::<CellSlot>(), 48);
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
                current: 0,
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
            vec![(index.root, None, [0u64; 5], [0u64; 5])]
        } else {
            Vec::new()
        };
        while let Some((node, parent_bit, route_masks, route_values)) = stack.pop() {
            if is_branch(node) {
                let slot = branch_index(node);
                assert!(seen_branches.insert(slot));
                let branch = index.branch(node).expect("reachable occupied branch");
                assert!(branch.bit < IDENTITY_BITS);
                assert!(parent_bit.is_none_or(|parent| parent < branch.bit));
                assert_ne!(branch.zero, branch.one);
                for (selected, child) in [branch.zero, branch.one].into_iter().enumerate() {
                    let mut child_masks = route_masks;
                    let mut child_values = route_values;
                    let word = usize::from(branch.bit / 64);
                    let mask = 1u64 << (63 - branch.bit % 64);
                    child_masks[word] |= mask;
                    child_values[word] |= mask * selected as u64;
                    stack.push((child, Some(branch.bit), child_masks, child_values));
                }
            } else {
                assert!(seen_leaves.insert(node));
                let (key, _) = index.resolved_leaf(node, records).unwrap();
                assert!(
                    tagged_key_chunks(key.tag, &key.identity)
                        .into_iter()
                        .zip(route_masks)
                        .zip(route_values)
                        .all(|((chunk, mask), value)| chunk & mask == value)
                );
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
        store.occupied_records = 1;
        store.identities.leaf_slots[0] = LeafSlot::Occupied {
            owner_record: 0,
            key_ordinal: 9,
        };
        store.identities.root = 0;
        assert_eq!(
            store.find(TAG_ENTITLEMENT, &record.entitlement.get(), &mut work()),
            Ok(Some(0))
        );
        assert_eq!(
            store.find(TAG_VECTOR, &record.entitlement.get(), &mut work()),
            Ok(None)
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
    fn c16_route_rejects_terminal_peer_violating_any_selected_ancestor() {
        for selected_position in 0..3 {
            let mut ledger = bundle_ledger(4, 8);
            reserve_bundle(&mut ledger, 1, 3);
            let cells = configured_cells(3, 1);
            let input = bundle_input(1, &cells);
            let key = TaggedKey::new(TAG_OBLIGATION, input.initial.materialize.obligation.get());
            let mut node = ledger.bundles.identities.root;
            let mut route = Vec::new();
            while is_branch(node) {
                let branch = ledger
                    .bundles
                    .identities
                    .branch(node)
                    .expect("fixture route branch");
                let selected = identity_bit(key.tag, &key.identity, branch.bit);
                route.push((branch_index(node), selected));
                node = [branch.zero, branch.one][selected];
            }
            assert!(
                route.len() >= 3,
                "fixture exposes first, middle, and last ancestors"
            );
            let position = [0, route.len() / 2, route.len() - 1][selected_position];
            let (branch_index, _) = route[position];
            let branch = match ledger.bundles.identities.branch_slots[branch_index] {
                BranchSlot::Occupied(branch) => branch,
                BranchSlot::Vacant { .. } => unreachable!("fixture route branch is occupied"),
            };
            ledger.bundles.identities.branch_slots[branch_index] =
                BranchSlot::Occupied(IdentityBranch {
                    zero: branch.one,
                    one: branch.zero,
                    ..branch
                });

            assert_eq!(
                ledger.bundles.route_precharged(key.tag, &key.identity),
                Err(FixedStorageError::NonCanonical),
                "ancestor position {position} cannot hide an existing identity"
            );
            let before = ledger.bundles.clone();
            let mut measured = work();
            assert_eq!(
                ledger.prepare_bundle(&input, &mut measured).unwrap_err(),
                SupportLedgerError::Storage(FixedStorageError::NonCanonical)
            );
            assert_eq!(
                measured.witness(),
                HotPathWorkWitness::default(),
                "corruption at ancestor position {position} burns no Work"
            );
            assert_eq!(ledger.bundles, before, "rejection is read-only");
        }
    }

    #[test]
    fn c16_staged_routes_reject_root_branch_and_terminal_corruption_safely() {
        let make = || {
            let mut ledger = bundle_ledger(4, 8);
            reserve_bundle(&mut ledger, 1, 3);
            ledger
        };
        let entitlement = bundle_entitlement(1);
        let reject = |ledger: &Ledger| {
            let before = bundle_snapshot(ledger);
            assert_eq!(
                ledger
                    .prepare_withdraw(request_owner(1), entitlement, &mut work())
                    .unwrap_err(),
                SupportLedgerError::Storage(FixedStorageError::NonCanonical)
            );
            assert_eq!(
                bundle_snapshot(ledger),
                before,
                "corrupt route rejection is read-only"
            );
        };

        let mut ledger = make();
        ledger.bundles.identities.root = NO_NODE;
        reject(&ledger);

        let mut ledger = make();
        ledger.bundles.identities.root = BRANCH_TAG | 1_000_000;
        reject(&ledger);

        let mut ledger = make();
        let branch = branch_index(ledger.bundles.identities.root);
        ledger.bundles.identities.branch_slots[branch] = BranchSlot::Vacant { free_position: 0 };
        reject(&ledger);

        let mut ledger = make();
        let root = ledger.bundles.identities.root;
        let branch = branch_index(root);
        let original = match ledger.bundles.identities.branch_slots[branch] {
            BranchSlot::Occupied(branch) => branch,
            BranchSlot::Vacant { .. } => unreachable!("nonempty K-leaf tree has a root branch"),
        };
        let selected = identity_bit(TAG_ENTITLEMENT, &entitlement.get(), original.bit);
        let child = [original.zero, original.one][selected];
        assert!(
            is_branch(child),
            "fixture entitlement route has a child branch"
        );
        let child_index = branch_index(child);
        let child_branch = match ledger.bundles.identities.branch_slots[child_index] {
            BranchSlot::Occupied(branch) => branch,
            BranchSlot::Vacant { .. } => unreachable!("valid route has occupied child branch"),
        };
        ledger.bundles.identities.branch_slots[child_index] =
            BranchSlot::Occupied(IdentityBranch {
                bit: original.bit,
                ..child_branch
            });
        reject(&ledger);

        let mut ledger = make();
        let root = ledger.bundles.identities.root;
        let branch = branch_index(root);
        let original = match ledger.bundles.identities.branch_slots[branch] {
            BranchSlot::Occupied(branch) => branch,
            BranchSlot::Vacant { .. } => unreachable!("nonempty K-leaf tree has a root branch"),
        };
        let selected = identity_bit(TAG_ENTITLEMENT, &entitlement.get(), original.bit);
        let corrupt = IdentityBranch {
            zero: if selected == 0 {
                NO_NODE
            } else {
                original.zero
            },
            one: if selected == 1 { NO_NODE } else { original.one },
            ..original
        };
        ledger.bundles.identities.branch_slots[branch] = BranchSlot::Occupied(corrupt);
        reject(&ledger);

        let mut ledger = make();
        let root = ledger.bundles.identities.root;
        let branch = branch_index(root);
        let original = match ledger.bundles.identities.branch_slots[branch] {
            BranchSlot::Occupied(branch) => branch,
            BranchSlot::Vacant { .. } => unreachable!("nonempty K-leaf tree has a root branch"),
        };
        ledger.bundles.identities.branch_slots[branch] = BranchSlot::Occupied(IdentityBranch {
            bit: IDENTITY_BITS,
            ..original
        });
        reject(&ledger);

        let mut ledger = make();
        let branch = branch_index(ledger.bundles.identities.root);
        let original = match ledger.bundles.identities.branch_slots[branch] {
            BranchSlot::Occupied(branch) => branch,
            BranchSlot::Vacant { .. } => unreachable!("nonempty K-leaf tree has a root branch"),
        };
        ledger.bundles.identities.branch_slots[branch] = BranchSlot::Occupied(IdentityBranch {
            one: original.zero,
            ..original
        });
        reject(&ledger);

        let mut ledger = make();
        let root = ledger.bundles.identities.root;
        let branch = branch_index(root);
        let original = match ledger.bundles.identities.branch_slots[branch] {
            BranchSlot::Occupied(branch) => branch,
            BranchSlot::Vacant { .. } => unreachable!("nonempty K-leaf tree has a root branch"),
        };
        ledger.bundles.identities.branch_slots[branch] = BranchSlot::Occupied(IdentityBranch {
            zero: root,
            ..original
        });
        reject(&ledger);

        let mut ledger = make();
        let (leaf, _) = ledger
            .bundles
            .route_precharged(TAG_ENTITLEMENT, &entitlement.get())
            .unwrap();
        ledger.bundles.identities.leaf_slots[leaf as usize] = LeafSlot::Vacant { free_position: 0 };
        reject(&ledger);

        let mut ledger = make();
        let (leaf, _) = ledger
            .bundles
            .route_precharged(TAG_ENTITLEMENT, &entitlement.get())
            .unwrap();
        ledger.bundles.identities.leaf_slots[leaf as usize] = LeafSlot::Occupied {
            owner_record: u32::MAX,
            key_ordinal: 9,
        };
        reject(&ledger);

        let mut ledger = make();
        let (leaf, _) = ledger
            .bundles
            .route_precharged(TAG_ENTITLEMENT, &entitlement.get())
            .unwrap();
        ledger.bundles.identities.leaf_slots[leaf as usize] = LeafSlot::Occupied {
            owner_record: 0,
            key_ordinal: K as u8,
        };
        reject(&ledger);
    }

    #[test]
    fn c16_legacy_reciprocal_routes_are_preflighted_and_corruption_safe() {
        let make = || {
            let mut ledger = bundle_ledger(4, 8);
            reserve_bundle(&mut ledger, 1, 3);
            ledger
        };
        let reject = |ledger: &mut Ledger| {
            let before = (bundle_snapshot(ledger), ledger.records.len());
            assert_eq!(
                ledger.reserve(
                    ledger.generation(),
                    spec(200, 201, Ordinary, &[Reserved([202; 32])]),
                    &mut work(),
                ),
                Err(SupportLedgerError::Storage(FixedStorageError::NonCanonical))
            );
            assert_eq!((bundle_snapshot(ledger), ledger.records.len()), before);
        };

        let mut ledger = make();
        let root = ledger.bundles.identities.root;
        let branch = branch_index(root);
        let BranchSlot::Occupied(original) = ledger.bundles.identities.branch_slots[branch] else {
            unreachable!("nonempty bundle index has a root branch")
        };
        ledger.bundles.identities.branch_slots[branch] = BranchSlot::Occupied(IdentityBranch {
            bit: IDENTITY_BITS,
            ..original
        });
        reject(&mut ledger);

        let mut ledger = make();
        let root = ledger.bundles.identities.root;
        let branch = branch_index(root);
        let BranchSlot::Occupied(original) = ledger.bundles.identities.branch_slots[branch] else {
            unreachable!("nonempty bundle index has a root branch")
        };
        let identity = [200; 32];
        let selected = identity_bit(TAG_OBLIGATION, &identity, original.bit);
        ledger.bundles.identities.branch_slots[branch] = BranchSlot::Occupied(IdentityBranch {
            zero: if selected == 0 {
                NO_NODE
            } else {
                original.zero
            },
            one: if selected == 1 { NO_NODE } else { original.one },
            ..original
        });
        reject(&mut ledger);

        let mut ledger = make();
        let identity = [200; 32];
        let (leaf, _) = ledger
            .bundles
            .route_precharged(TAG_OBLIGATION, &identity)
            .unwrap();
        ledger.bundles.identities.leaf_slots[leaf as usize] = LeafSlot::Occupied {
            owner_record: u32::MAX,
            key_ordinal: 0,
        };
        reject(&mut ledger);

        let ledger = make();
        let mut meter = work();
        meter
            .record(WorkDimension::VisitedEntities, 1_704_575 - 264)
            .unwrap();
        let before = meter.witness();
        assert!(matches!(
            ledger.bundles.find(TAG_OBLIGATION, &[200; 32], &mut meter),
            Err(FixedStorageError::Work(WorkBudgetError::BudgetExceeded(
                WorkDimension::VisitedEntities,
                1_704_575,
                1_704_576
            )))
        ));
        assert_eq!(meter.witness(), before, "route preflight is all-or-none");
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
    fn c16_bundle_leaf_and_branch_destinations_reject_every_selected_corruption() {
        for corruption in 0..8 {
            let mut ledger = bundle_ledger(4, 8);
            let cells = configured_cells(3, 1);
            let input = bundle_input(1, &cells);
            let mut meter = work();
            let change = ledger.prepare_bundle(&input, &mut meter).unwrap();
            match corruption {
                0 => *ledger.bundles.identities.free_leaves.last_mut().unwrap() = u32::MAX,
                1 => {
                    let free = &mut ledger.bundles.identities.free_leaves;
                    let len = free.len();
                    free[len - 1] = free[len - 2];
                }
                2 => {
                    let index = *ledger.bundles.identities.free_leaves.last().unwrap();
                    ledger.bundles.identities.leaf_slots[index as usize] = LeafSlot::Occupied {
                        owner_record: 0,
                        key_ordinal: 0,
                    };
                }
                3 => {
                    let free = &mut ledger.bundles.identities.free_leaves;
                    let len = free.len();
                    free.swap(len - 1, len - 2);
                }
                4 => *ledger.bundles.identities.free_branches.last_mut().unwrap() = u32::MAX,
                5 => {
                    let free = &mut ledger.bundles.identities.free_branches;
                    let len = free.len();
                    free[len - 1] = free[len - 2];
                }
                6 => {
                    let index = *ledger.bundles.identities.free_branches.last().unwrap();
                    ledger.bundles.identities.branch_slots[index as usize] =
                        BranchSlot::Occupied(IdentityBranch {
                            bit: 0,
                            zero: 0,
                            one: 1,
                        });
                }
                7 => {
                    let free = &mut ledger.bundles.identities.free_branches;
                    let len = free.len();
                    free.swap(len - 1, len - 2);
                }
                _ => unreachable!(),
            }
            let before = bundle_snapshot(&ledger);
            assert_eq!(
                ledger.validate_bundle(change).unwrap_err(),
                SupportLedgerError::Storage(FixedStorageError::NonCanonical),
                "selected destination corruption {corruption}"
            );
            assert_eq!(bundle_snapshot(&ledger), before);
        }
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

/// Ledger-level C18 laws: the complete observation, the bounded expiry
/// transaction, generation stability, and capability drop.
#[cfg(test)]
mod c18_ledger_tests {
    use super::*;
    use crate::{HotPathWorkBudget, SupportOperationObligationId};

    type Ledger = SupportChargeLedger<64, 64, 1>;

    fn work() -> WorkMeter {
        WorkMeter::new(HotPathWorkBudget::binary_maximum())
    }

    fn at(micros: u64) -> MonotonicTime {
        MonotonicTime::from_micros(micros)
    }

    fn ledger() -> Ledger {
        let starts = [[FixedStartCountBound(Duration::from_micros(10), 1); 1]; 21];
        Ledger::try_new(
            SupportLedgerGeneration::new(1).unwrap(),
            [[2, 1, 1], [1, 0, 1], [2, 1, 1], [4, 1, 1], [4, 1, 1]],
            2,
            starts,
            LifecycleReserveMaxima([1, 2, 2, 1, 1]),
            4,
            8,
            6,
            c18::SupportHistoryLimits::testing(starts),
        )
        .unwrap()
    }

    fn obligation(n: u8) -> SupportOperationObligationId {
        SupportOperationObligationId::new([n; 32]).unwrap()
    }

    /// T02 — a newly constructed ledger reports zero use, no due expiry, a
    /// vacant carry slot, and running ordinary reservations.
    #[test]
    fn an_empty_ledger_reports_complete_zero_facts() {
        let ledger = ledger();
        let snapshot = ledger
            .ledger_snapshot(ledger.generation(), at(0), &mut work())
            .unwrap();
        assert_eq!(snapshot.retention.generation, ledger.generation());
        assert_eq!(snapshot.retention.at, at(0));
        assert_eq!(snapshot.retention.expiry_due, 0);
        assert_eq!(snapshot.retention.expiry_scheduled, 0);
        assert_eq!(snapshot.retention.next_expiry_at, None);
        assert_eq!(snapshot.retention.carry_slot, c18::CarrySlot::Vacant);
        assert_eq!(snapshot.retention.carry_capacity, 1);
        assert_eq!(
            snapshot.retention.ordinary_reservations,
            c18::OrdinaryReservations::Running
        );
        assert_eq!(
            snapshot.retention.retention_horizon,
            Duration::from_micros(10)
        );
        assert_eq!(snapshot.capacity.usage, [[0; POOLS]; 5]);
    }

    /// T16 — repeating the same observation on unchanged state reproduces the
    /// same value and keeps the generation.
    #[test]
    fn repeating_an_observation_is_stable_and_advances_nothing() {
        let ledger = ledger();
        let generation = ledger.generation();
        let first = ledger
            .ledger_snapshot(generation, at(3), &mut work())
            .unwrap();
        let second = ledger
            .ledger_snapshot(generation, at(3), &mut work())
            .unwrap();
        assert_eq!(first, second);
        assert_eq!(ledger.generation(), generation);
    }

    /// T17 — a stale generation is rejected even though the counts are equal,
    /// and a backward `at` cannot observe the ledger.
    #[test]
    fn a_stale_generation_or_backward_time_is_rejected() {
        let mut ledger = ledger();
        let stale = ledger.generation();
        let newer = stale.next().unwrap();
        assert_eq!(
            ledger
                .ledger_snapshot(newer, at(0), &mut work())
                .unwrap_err(),
            SupportLedgerError::Generation
        );
        // An expiry commit advances the floor; an earlier observation then
        // fails closed on time rather than reporting a stale view.
        let mut meter = work();
        let prepared = ledger
            .prepare_expiry::<1, 1>(stale, at(50), &mut meter)
            .unwrap();
        ledger.validate_expiry(prepared).unwrap().commit();
        assert_eq!(
            ledger
                .ledger_snapshot(ledger.generation(), at(49), &mut work())
                .unwrap_err(),
            SupportLedgerError::Storage(FixedStorageError::InvalidTime)
        );
    }

    /// T16 — an empty batch commits successfully, releases nothing, and leaves
    /// the generation unchanged.
    #[test]
    fn a_no_op_expiry_keeps_the_generation_stable() {
        let mut ledger = ledger();
        let generation = ledger.generation();
        let mut meter = work();
        let prepared = ledger
            .prepare_expiry::<1, 1>(generation, at(0), &mut meter)
            .unwrap();
        let commit = ledger.validate_expiry(prepared).unwrap().commit();
        assert_eq!(commit.released_groups, 0);
        assert_eq!(commit.released_units, 0);
        assert!(!commit.more_due);
        assert_eq!(commit.next_expiry_at, None);
        assert_eq!(commit.generation, generation);
        assert_eq!(ledger.generation(), generation);
    }

    /// T21 — dropping a prepared or validated capability without committing
    /// leaves the ledger byte-identical.
    #[test]
    fn dropping_a_prepared_expiry_changes_nothing() {
        let mut ledger = ledger();
        let generation = ledger.generation();
        let before = ledger.c18.scheduled().to_vec();
        let mut first = work();
        drop(
            ledger
                .prepare_expiry::<1, 1>(generation, at(0), &mut first)
                .unwrap(),
        );
        assert_eq!(ledger.generation(), generation);
        assert_eq!(ledger.c18.scheduled(), before.as_slice());

        let mut second = work();
        let prepared = ledger
            .prepare_expiry::<1, 1>(generation, at(0), &mut second)
            .unwrap();
        drop(ledger.validate_expiry(prepared).unwrap());
        assert_eq!(ledger.generation(), generation);
        assert_eq!(ledger.c18.scheduled(), before.as_slice());
    }

    /// A prepared selection cannot be replayed after the state it bound has
    /// moved on.
    #[test]
    fn a_replayed_prepared_expiry_is_rejected() {
        let mut ledger = ledger();
        let generation = ledger.generation();
        let mut stale_meter = work();
        let mut fresh_meter = work();
        let stale = ledger
            .prepare_expiry::<1, 1>(generation, at(0), &mut stale_meter)
            .unwrap();
        let fresh = ledger
            .prepare_expiry::<1, 1>(generation, at(0), &mut fresh_meter)
            .unwrap();
        ledger.validate_expiry(fresh).unwrap().commit();
        // The no-op commit kept the generation, so advance it through a real
        // ordinary reservation before replaying the stale capability.
        ledger.generation = ledger.generation().next().unwrap();
        assert!(matches!(
            ledger.validate_expiry(stale),
            Err(SupportLedgerError::Generation)
        ));
    }

    /// The carry input exposes the exact snapshot, the canonical scheduled
    /// view, the accumulator, and the vacant slot, without copying the state.
    #[test]
    fn the_carry_input_exposes_the_complete_canonical_view() {
        let ledger = ledger();
        let input = ledger
            .carry_input(ledger.generation(), at(0), &mut work())
            .unwrap();
        assert_eq!(input.scheduled(), &[]);
        assert_eq!(input.carry_slot(), &c18::CarrySlot::Vacant);
        assert_eq!(input.accumulator(), &c18::Accumulator::default());
        assert_eq!(input.snapshot().retention.carry_capacity, c18::CARRY_SLOTS);
        // The inventory is complete on every axis the ledger actually owns; it
        // does not invent a tensor the ledger does not track.
        assert_eq!(input.history(), &[0; c18::CELLS]);
        assert_eq!(input.vectors(), &[[0; 1]; c18::CELLS]);
        assert_eq!(input.reserved(), &[[0; POOLS]; 5]);
    }

    /// D2/D3 end to end — a started record stays fully charged until its
    /// Catalog Retention Horizon, and one bounded transition then releases the
    /// whole group and advances the generation exactly once.
    #[test]
    fn a_started_record_is_retained_until_its_horizon_then_released_once() {
        let mut ledger = ledger();
        let id = obligation(1);
        let claims = [SupportFundingClaim::OrdinaryReservation([1; 32])];
        let mut credit = [1; 32];
        credit[31] ^= 0x80;
        let spec = SupportObligationSpec {
            id,
            operation: SupportOperation::MaterializeRequest,
            pool: SupportPool::Ordinary,
            physical_credit: PhysicalStartCreditId::new(credit).unwrap(),
            predecessor: SupportCausalPredecessorId([1; 32]),
            claims: &claims,
        };
        ledger
            .reserve(ledger.generation(), spec, &mut work())
            .unwrap();
        ledger
            .transition(
                ledger.generation(),
                id,
                PredecessorEnded(SupportCausalPredecessorId([1; 32]), at(2)),
                &mut work(),
            )
            .unwrap();
        ledger
            .transition(ledger.generation(), id, BeginSupport(at(5)), &mut work())
            .unwrap();
        // Finishing early must not shorten retention: the release instant is
        // `max(terminal_at, start_at + R_cat) = max(6, 5 + 10) = 15`.
        ledger
            .transition(ledger.generation(), id, FinishSupport(at(6)), &mut work())
            .unwrap();

        let scheduled = ledger.c18.scheduled();
        assert_eq!(scheduled.len(), 1);
        assert_eq!(scheduled[0].release_at, at(15));

        // Before the horizon the group is neither due nor releasable.
        let early = ledger
            .ledger_snapshot(ledger.generation(), at(14), &mut work())
            .unwrap();
        assert_eq!(early.retention.expiry_due, 0);
        assert_eq!(early.retention.expiry_scheduled, 1);
        assert_eq!(early.retention.next_expiry_at, Some(at(15)));

        // Equality is eligible.
        let due = ledger
            .ledger_snapshot(ledger.generation(), at(15), &mut work())
            .unwrap();
        assert_eq!(due.retention.expiry_due, 1);

        let generation = ledger.generation();
        let mut meter = work();
        let prepared = ledger
            .prepare_expiry::<1, 1>(generation, at(15), &mut meter)
            .unwrap();
        let commit = ledger.validate_expiry(prepared).unwrap().commit();
        assert_eq!(commit.released_groups, 1);
        assert_eq!(commit.released_units, 1);
        assert!(!commit.more_due);
        assert_eq!(commit.next_expiry_at, None);
        assert_eq!(commit.generation, generation.next().unwrap());
        assert_eq!(ledger.generation(), generation.next().unwrap());
        assert_eq!(ledger.c18.scheduled(), &[]);

        // A reported release is an actual release: the whole group's
        // occupancy, its one physical start credit and its funding claim all
        // return, and the record slot becomes reusable.
        assert_eq!(ledger.usage[ACTIVE][0], 0, "retained occupancy returned");
        assert_eq!(ledger.usage[CREDITS][0], 0, "physical credit returned");
        assert_eq!(ledger.usage[CLAIMS][0], 0, "funding claim returned");
        assert_eq!(ledger.records.live(), 0, "record slot reclaimed");
        assert_eq!(ledger.records.free_record_len(), 1, "slot is reusable");
        // The identity left the index, so the same obligation may be created
        // again rather than colliding forever with a released group.
        assert_eq!(
            ledger.records.find(key(0, id.get()), &mut work()).unwrap(),
            None
        );
    }

    /// One preflight covers the whole envelope: after prepare returns, neither
    /// validation nor commit may charge anything more, and the charge must
    /// cover the release itself rather than only the selection walk.
    #[test]
    fn the_expiry_preflight_covers_validation_and_release() {
        let mut ledger = ledger();
        let id = obligation(1);
        let claims = [SupportFundingClaim::OrdinaryReservation([1; 32])];
        let mut credit = [1; 32];
        credit[31] ^= 0x80;
        let spec = SupportObligationSpec {
            id,
            operation: SupportOperation::MaterializeRequest,
            pool: SupportPool::Ordinary,
            physical_credit: PhysicalStartCreditId::new(credit).unwrap(),
            predecessor: SupportCausalPredecessorId([1; 32]),
            claims: &claims,
        };
        ledger
            .reserve(ledger.generation(), spec, &mut work())
            .unwrap();
        ledger
            .transition(
                ledger.generation(),
                id,
                PredecessorEnded(SupportCausalPredecessorId([1; 32]), at(2)),
                &mut work(),
            )
            .unwrap();
        ledger
            .transition(ledger.generation(), id, BeginSupport(at(5)), &mut work())
            .unwrap();
        ledger
            .transition(ledger.generation(), id, FinishSupport(at(6)), &mut work())
            .unwrap();

        let mut meter = work();
        let prepared = ledger
            .prepare_expiry::<1, 1>(ledger.generation(), at(15), &mut meter)
            .unwrap();
        let preflight = prepared.work.witness();
        // A one-group batch must be charged strictly more than the walk that
        // found it: the release deletes two identities, removes the raw owner
        // and extracts from the heap.
        assert!(
            preflight.value(WorkDimension::VisitedEntities) > 4,
            "the preflight covers the release, saw {preflight:?}"
        );
        assert_eq!(preflight.value(WorkDimension::Allocations), 0);
        assert_eq!(preflight.value(WorkDimension::CandidateWork), 0);

        let validated = ledger.validate_expiry(prepared).unwrap();
        let commit = validated.commit();
        assert_eq!(commit.released_groups, 1);
        // The meter is the same one the prepare charged; nothing after the
        // preflight may add to it.
        assert_eq!(
            meter.witness(),
            preflight,
            "validation and commit charged nothing further"
        );
    }

    /// A committed expiry advances the ledger's time floor, and no later
    /// time-bearing transition may move the ledger back behind it.
    #[test]
    fn a_transition_cannot_move_the_ledger_behind_its_floor() {
        let mut ledger = ledger();
        let generation = ledger.generation();
        let mut meter = work();
        let prepared = ledger
            .prepare_expiry::<1, 1>(generation, at(50), &mut meter)
            .unwrap();
        ledger.validate_expiry(prepared).unwrap().commit();

        let id = obligation(1);
        let claims = [SupportFundingClaim::OrdinaryReservation([1; 32])];
        let mut credit = [1; 32];
        credit[31] ^= 0x80;
        let spec = SupportObligationSpec {
            id,
            operation: SupportOperation::MaterializeRequest,
            pool: SupportPool::Ordinary,
            physical_credit: PhysicalStartCreditId::new(credit).unwrap(),
            predecessor: SupportCausalPredecessorId([1; 32]),
            claims: &claims,
        };
        ledger
            .reserve(ledger.generation(), spec, &mut work())
            .unwrap();
        let before = ledger.generation();
        assert_eq!(
            ledger.transition(
                before,
                id,
                PredecessorEnded(SupportCausalPredecessorId([1; 32]), at(2)),
                &mut work()
            ),
            Err(SupportLedgerError::Storage(FixedStorageError::InvalidTime)),
            "a transition behind the floor"
        );
        assert_eq!(ledger.generation(), before, "state unchanged");
    }

    /// The ordinary prepare/commit finish path is the production route, and it
    /// must enter expiry exactly like the transition route does.
    #[test]
    fn the_ordinary_finish_path_schedules_its_release() {
        let mut ledger = ledger();
        let id = obligation(1);
        let claims = [SupportFundingClaim::OrdinaryReservation([1; 32])];
        let mut credit = [1; 32];
        credit[31] ^= 0x80;
        let spec = SupportObligationSpec {
            id,
            operation: SupportOperation::MaterializeRequest,
            pool: SupportPool::Ordinary,
            physical_credit: PhysicalStartCreditId::new(credit).unwrap(),
            predecessor: SupportCausalPredecessorId([1; 32]),
            claims: &claims,
        };
        ledger
            .reserve(ledger.generation(), spec, &mut work())
            .unwrap();
        ledger
            .transition(
                ledger.generation(),
                id,
                PredecessorEnded(SupportCausalPredecessorId([1; 32]), at(2)),
                &mut work(),
            )
            .unwrap();
        ledger
            .transition(ledger.generation(), id, BeginSupport(at(5)), &mut work())
            .unwrap();
        assert_eq!(ledger.c18.scheduled(), &[], "nothing retained yet");

        // Finish through prepare/commit, not through transition.
        let mut meter = work();
        let change = ledger
            .prepare(
                ledger.generation(),
                SupportChangeInput::FinishActive(id, at(6)),
                &mut meter,
            )
            .unwrap();
        ledger.commit(change, &mut meter).unwrap();

        let scheduled = ledger.c18.scheduled();
        assert_eq!(
            scheduled.len(),
            1,
            "the ordinary finish scheduled a release"
        );
        assert_eq!(
            scheduled[0].release_at,
            at(15),
            "max(terminal, start + R_cat)"
        );
        assert_eq!(scheduled[0].identity, id.get());
    }

    /// T28 — churn beyond physical capacity succeeds when expiry runs between
    /// generations, which is only true if a release actually frees the slot.
    #[test]
    fn churn_past_capacity_succeeds_when_expiry_runs_between() {
        let mut ledger = ledger();
        for round in 1..=6u8 {
            let id = obligation(round);
            let claims = [SupportFundingClaim::OrdinaryReservation([round; 32])];
            let mut credit = [round; 32];
            credit[31] ^= 0x80;
            let spec = SupportObligationSpec {
                id,
                operation: SupportOperation::MaterializeRequest,
                pool: SupportPool::Ordinary,
                physical_credit: PhysicalStartCreditId::new(credit).unwrap(),
                predecessor: SupportCausalPredecessorId([round; 32]),
                claims: &claims,
            };
            let base = u64::from(round) * 100;
            ledger
                .reserve(ledger.generation(), spec, &mut work())
                .unwrap();
            ledger
                .transition(
                    ledger.generation(),
                    id,
                    PredecessorEnded(SupportCausalPredecessorId([round; 32]), at(base + 2)),
                    &mut work(),
                )
                .unwrap();
            ledger
                .transition(
                    ledger.generation(),
                    id,
                    BeginSupport(at(base + 5)),
                    &mut work(),
                )
                .unwrap();
            ledger
                .transition(
                    ledger.generation(),
                    id,
                    FinishSupport(at(base + 6)),
                    &mut work(),
                )
                .unwrap();
            let mut meter = work();
            let prepared = ledger
                .prepare_expiry::<1, 1>(ledger.generation(), at(base + 20), &mut meter)
                .unwrap();
            let commit = ledger.validate_expiry(prepared).unwrap().commit();
            assert_eq!(
                commit.released_groups, 1,
                "round {round} released its group"
            );
            assert_eq!(
                ledger.records.live(),
                0,
                "round {round} left no live record"
            );
            assert_eq!(
                ledger.usage[CLAIMS][0], 0,
                "round {round} returned its claim"
            );
        }
    }
}
