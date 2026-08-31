use super::*;
use crate::work::WorkRecorder;

const ROOT_BATCH_MAX: usize = 5;
const ROOT_BATCH_FUNDER_MAX: usize = ROOT_BATCH_MAX * PLAN_MEMBERS_MAX;
const ROOT_BATCH_LOCAL_MAX: usize = ROOT_BATCH_MAX * (PLAN_MEMBERS_MAX + 1) + PLAN_MEMBERS_MAX;
const NEXT_PLAN_CAUSAL_EVENT: usize = 56;
const CLOSE_AUTHORITY_PLAN: u8 = 1;
const CLOSE_AUTHORITY_STANDALONE: u8 = 2;
const CLOSE_AUTHORITY_CANCELLATION: u8 = 3;
const RESOLUTION_RECORD_MAX: usize = 4;
const RESOLUTION_PUBLICATION_MAX: usize = RESOLUTION_RECORD_MAX * PLAN_MEMBERS_MAX;
const RESOLUTION_RAW_MAX: usize = RESOLUTION_RECORD_MAX * 2;
const ROOT_RAW_ASSIGNMENT_MAX: usize = 9 * RESOLUTION_RAW_MAX + 1;
const ROOT_LOCAL_ASSIGNMENT_MAX: usize = 9 * ROOT_BATCH_LOCAL_MAX + 1;
const LIFECYCLE_CLOSE_ACTION: u8 = 1;
const NO_CONTINUATION_AFTER_OBSERVATION: u8 = 1;
const MEMBERSHIP_CLOSED_LIFECYCLE: u8 = 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub(crate) enum SemanticOperation {
    PlanCreateO = 1,
    PlanCreateC = 2,
    PlanCreateR = 3,
    CreateStandalone = 4,
    MergeInitial = 5,
    NewlyEligibleNotify = 6,
    ReceiptDisposition = 7,
    RejectionResult = 8,
    LocalStaleResult = 9,
    ResolveObservationDescriptions = 10,
    ResolveObservationOther = 11,
    MarkPredecessorEnded = 12,
    Begin = 13,
    Finish = 14,
    Join = 15,
    SourceFreeRebind = 16,
    NewlyEligibleJoin = 17,
    NewlyEligibleRebind = 18,
    Split = 19,
    Merge = 20,
    CancellationRemoveBound = 21,
    CancellationRemoveEligibleUnbound = 22,
    MembershipClose = 23,
    TypedCloseStandalone = 24,
    TypedCloseC = 25,
    TypedCloseR = 26,
    TypedCloseTerminal = 27,
    Tombstone = 28,
    Withdraw = 29,
    ExpiryCurrentUse = 30,
}

impl SemanticOperation {
    pub(crate) const ALL: [Self; 30] = [
        Self::PlanCreateO,
        Self::PlanCreateC,
        Self::PlanCreateR,
        Self::CreateStandalone,
        Self::MergeInitial,
        Self::NewlyEligibleNotify,
        Self::ReceiptDisposition,
        Self::RejectionResult,
        Self::LocalStaleResult,
        Self::ResolveObservationDescriptions,
        Self::ResolveObservationOther,
        Self::MarkPredecessorEnded,
        Self::Begin,
        Self::Finish,
        Self::Join,
        Self::SourceFreeRebind,
        Self::NewlyEligibleJoin,
        Self::NewlyEligibleRebind,
        Self::Split,
        Self::Merge,
        Self::CancellationRemoveBound,
        Self::CancellationRemoveEligibleUnbound,
        Self::MembershipClose,
        Self::TypedCloseStandalone,
        Self::TypedCloseC,
        Self::TypedCloseR,
        Self::TypedCloseTerminal,
        Self::Tombstone,
        Self::Withdraw,
        Self::ExpiryCurrentUse,
    ];

    pub(crate) const fn work(self) -> [u64; 5] {
        match self {
            Self::PlanCreateO | Self::PlanCreateC | Self::PlanCreateR => WORK_PLAN_CREATE,
            Self::CreateStandalone => WORK_CREATE_STANDALONE,
            Self::MergeInitial => WORK_MERGE_INITIAL,
            Self::NewlyEligibleNotify => WORK_NEWLY_ELIGIBLE,
            Self::ReceiptDisposition | Self::RejectionResult | Self::LocalStaleResult => {
                WORK_PLAN_DISPOSITION
            }
            Self::ResolveObservationDescriptions | Self::ResolveObservationOther => {
                WORK_RESOLVE_OBSERVATION
            }
            Self::MarkPredecessorEnded | Self::Begin | Self::Finish => WORK_STATE_TRANSITION,
            Self::Join
            | Self::SourceFreeRebind
            | Self::NewlyEligibleJoin
            | Self::NewlyEligibleRebind => WORK_JOIN_REBIND,
            Self::Split => WORK_SPLIT,
            Self::Merge => WORK_MERGE,
            Self::CancellationRemoveBound => WORK_REMOVE_BOUND,
            Self::CancellationRemoveEligibleUnbound => WORK_REMOVE_ELIGIBLE,
            Self::MembershipClose
            | Self::TypedCloseStandalone
            | Self::TypedCloseC
            | Self::TypedCloseR
            | Self::TypedCloseTerminal => WORK_CLOSE,
            Self::Tombstone | Self::Withdraw | Self::ExpiryCurrentUse => WORK_TOMBSTONE,
        }
    }

    pub(crate) const fn retained_budget(self) -> Option<(usize, u32)> {
        match self {
            Self::CreateStandalone => Some((88, CREATE_STANDALONE_BUDGET as u32)),
            Self::MergeInitial => Some((92, MERGE_INITIAL_BUDGET as u32)),
            Self::Join
            | Self::SourceFreeRebind
            | Self::NewlyEligibleJoin
            | Self::NewlyEligibleRebind
            | Self::Split
            | Self::Merge
            | Self::CancellationRemoveBound
            | Self::CancellationRemoveEligibleUnbound
            | Self::MembershipClose => Some((96, POST_CREATE_BUDGET as u32)),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PlanDisposition {
    Receipt,
    Rejection,
    LocalStale,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ObservationResolution {
    DescriptionsRequired,
    Other,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RootAction {
    MarkPredecessorEnded,
    Begin,
    Finish,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RootAnchor {
    pub(crate) authority_key: [u8; 17],
    pub(crate) branch: u8,
    pub(crate) group: ArenaRef,
    pub(crate) root: ArenaRef,
    pub(crate) version: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct AggregateDelta {
    pub(crate) usage: [[i32; 3]; 5],
    pub(crate) reserved: [[i32; 3]; 5],
    pub(crate) attached: [[i32; 3]; 4],
}

impl AggregateDelta {
    pub(super) const ZERO: Self = Self {
        usage: [[0; 3]; 5],
        reserved: [[0; 3]; 5],
        attached: [[0; 3]; 4],
    };

    pub(super) fn add(&mut self, other: Self) -> Result<(), SupportLedgerError> {
        for class in 0..5 {
            for pool in 0..3 {
                self.usage[class][pool] = self.usage[class][pool]
                    .checked_add(other.usage[class][pool])
                    .ok_or_else(capacity_error)?;
                self.reserved[class][pool] = self.reserved[class][pool]
                    .checked_add(other.reserved[class][pool])
                    .ok_or_else(capacity_error)?;
                if class < 4 {
                    self.attached[class][pool] = self.attached[class][pool]
                        .checked_add(other.attached[class][pool])
                        .ok_or_else(capacity_error)?;
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct RootMemberSnapshot {
    pub(super) member: ArenaRef,
    pub(super) funder: ArenaRef,
    pub(super) owner: ArenaRef,
    pub(super) request_key: [u8; 40],
    pub(super) entitlement: [u8; 32],
    pub(super) vector: [u8; 32],
    pub(super) branch_limit: u64,
    pub(super) active: bool,
}

impl RootMemberSnapshot {
    pub(super) const ZERO: Self = Self {
        member: ArenaRef {
            slot: 0,
            generation: 0,
        },
        funder: ArenaRef {
            slot: 0,
            generation: 0,
        },
        owner: ArenaRef {
            slot: 0,
            generation: 0,
        },
        request_key: [0; 40],
        entitlement: [0; 32],
        vector: [0; 32],
        branch_limit: 0,
        active: false,
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct RootSnapshot {
    pub(super) authority_key: [u8; 17],
    pub(super) branch: u8,
    pub(super) group: ArenaRef,
    pub(super) formation: ArenaRef,
    pub(super) initial_formation: ArenaRef,
    pub(super) locator: ArenaRef,
    pub(super) locator_kind: u8,
    pub(super) state: RootState,
    pub(super) version: u64,
    pub(super) occurred_at: u64,
    pub(super) member_count: usize,
    pub(super) members: [RootMemberSnapshot; PLAN_MEMBERS_MAX],
    pub(super) group_image: [u8; GROUP_BYTES],
    pub(super) formation_image: [u8; FORMATION_BYTES],
    pub(super) locator_image: [u8; EXTERNAL_HEAD_BYTES],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RootTransitionSpec {
    before: RootSnapshot,
    after: RootState,
    cause: FormationCause,
    close_reason: u8,
    close_authority: [u8; 32],
    occurred_at: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct OwnerDelta {
    slot: u32,
    owner: ArenaRef,
    linked: i32,
    branch: [i32; 4],
}

impl OwnerDelta {
    const ZERO: Self = Self {
        slot: 0,
        owner: ArenaRef {
            slot: 0,
            generation: 0,
        },
        linked: 0,
        branch: [0; 4],
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ResolverChange {
    Keep,
    MoveTo(usize),
    Retire,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ResolutionRecordSnapshot {
    reference: ArenaRef,
    record: LifecycleRecordInput,
    image: [u8; LIFECYCLE_BYTES],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RootBatchPreview {
    expected_c17: u64,
    operation: SemanticOperation,
    transitions: [Option<RootTransitionSpec>; ROOT_BATCH_MAX],
    transition_count: usize,
    aggregate: AggregateDelta,
    owners: [OwnerDelta; PLAN_MEMBERS_MAX],
    owner_count: usize,
    resolver: ResolverChange,
    plan_event: Option<(u64, u64)>,
    resolution_records: [Option<ResolutionRecordSnapshot>; RESOLUTION_RECORD_MAX],
    resolution_record_count: usize,
    lifecycle_before: LifecycleAggregate,
    lifecycle_after: LifecycleAggregate,
    retractions: [LifecyclePublication; RESOLUTION_PUBLICATION_MAX],
    retraction_count: usize,
}

impl RootBatchPreview {
    pub(crate) const fn aggregate_delta(&self) -> AggregateDelta {
        self.aggregate
    }

    pub(crate) const fn owner_count(&self) -> usize {
        self.owner_count
    }

    pub(crate) fn owner_slots(&self) -> [u32; PLAN_MEMBERS_MAX] {
        let mut slots = [0; PLAN_MEMBERS_MAX];
        let mut index = 0;
        while index < self.owner_count {
            slots[index] = self.owners[index].slot;
            index += 1;
        }
        slots
    }

    pub(crate) fn owner_branch_delta(&self, index: usize) -> Option<[i32; 4]> {
        (index < self.owner_count).then_some(self.owners[index].branch)
    }

    pub(in crate::support) const fn lifecycle_before(&self) -> LifecycleAggregate {
        self.lifecycle_before
    }

    pub(in crate::support) const fn lifecycle_after(&self) -> LifecycleAggregate {
        self.lifecycle_after
    }

    pub(in crate::support) fn retractions(&self) -> &[LifecyclePublication] {
        &self.retractions[..self.retraction_count]
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RootJournal {
    before: RootSnapshot,
    locator_after_ref: ArenaRef,
    group_after: [u8; GROUP_BYTES],
    locator_after: [u8; EXTERNAL_HEAD_BYTES],
    formation_after: [u8; FORMATION_BYTES],
    funder_after: [[u8; FUNDER_BYTES]; PLAN_MEMBERS_MAX],
    member_after: [[u8; MEMBER_BYTES]; PLAN_MEMBERS_MAX],
    mutation_after: [u8; MUTATION_BYTES],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ResolutionRecordJournal {
    snapshot: ResolutionRecordSnapshot,
    after: [u8; LIFECYCLE_BYTES],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ResolutionRawUpdate {
    key: [u8; 32],
    handle: NodeHandle,
    before: [u8; 8],
    after: [u8; 8],
}
