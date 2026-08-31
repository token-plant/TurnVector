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

#[derive(Eq, PartialEq)]
pub(crate) struct PreparedRootBatch {
    expected_c17: u64,
    expected_raw: u64,
    expected_local: u64,
    expected_lifecycle: u64,
    expected_arena_headers: [ByteArenaHeaderImage; 11],
    arena_headers_after: [ByteArenaHeaderImage; 11],
    preview: RootBatchPreview,
    formations: ArenaSelection<ROOT_BATCH_MAX>,
    funders: ArenaSelection<ROOT_BATCH_FUNDER_MAX>,
    wrappers: ArenaSelection<ROOT_BATCH_MAX>,
    wrapper_count: usize,
    mutations: ArenaSelection<ROOT_BATCH_MAX>,
    links: ArenaSelection<PLAN_MEMBERS_MAX>,
    link_count: usize,
    local_entries: [([u8; 17], [u8; 8]); ROOT_BATCH_LOCAL_MAX],
    local_count: usize,
    journals: [Option<RootJournal>; ROOT_BATCH_MAX],
    owner_records_before: [Option<BundleRecord>; PLAN_MEMBERS_MAX],
    owner_records_after: [Option<BundleRecord>; PLAN_MEMBERS_MAX],
    owner_references: [[ArenaRef; 4]; PLAN_MEMBERS_MAX],
    owner_rows_after: [[u8; OWNER_ROW_BYTES]; PLAN_MEMBERS_MAX],
    owners_after: [[u8; OWNER_BYTES]; PLAN_MEMBERS_MAX],
    retired_links: [ArenaRef; PLAN_MEMBERS_MAX],
    retired_link_before: [[u8; LINK_BYTES]; PLAN_MEMBERS_MAX],
    retired_link_after: [[u8; LINK_BYTES]; PLAN_MEMBERS_MAX],
    new_link_images: [[u8; LINK_BYTES]; PLAN_MEMBERS_MAX],
    resolution_journals: [Option<ResolutionRecordJournal>; RESOLUTION_RECORD_MAX],
    raw_updates: [Option<ResolutionRawUpdate>; RESOLUTION_RAW_MAX],
    raw_update_count: usize,
    header_after: C17HeaderImage,
    raw_plan: Option<PatriciaAssignmentPlan<ROOT_RAW_ASSIGNMENT_MAX>>,
    local_plan: PatriciaAssignmentPlan<ROOT_LOCAL_ASSIGNMENT_MAX>,
}

impl std::fmt::Debug for PreparedRootBatch {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PreparedRootBatch")
            .field("operation", &self.preview.operation)
            .field("transition_count", &self.preview.transition_count)
            .field("owner_count", &self.preview.owner_count)
            .finish_non_exhaustive()
    }
}

impl PreparedRootBatch {
    pub(crate) fn visit_assignments(
        &self,
        visitor: &mut dyn FnMut(crate::reusable::AssignmentOrderKey, crate::c17_layout::Assignment),
    ) {
        if let Some(plan) = &self.raw_plan {
            plan.visit_assignments(visitor);
        }
        self.local_plan.visit_assignments(visitor);
    }

    pub(crate) const fn aggregate_delta(&self) -> AggregateDelta {
        self.preview.aggregate
    }

    pub(crate) const fn owner_count(&self) -> usize {
        self.preview.owner_count
    }

    pub(crate) fn owner_slots(&self) -> [u32; PLAN_MEMBERS_MAX] {
        self.preview.owner_slots()
    }

    pub(crate) fn owner_branch_delta(&self, index: usize) -> Option<[i32; 4]> {
        self.preview.owner_branch_delta(index)
    }

    pub(in crate::support) const fn owner_records_after(
        &self,
    ) -> [Option<BundleRecord>; PLAN_MEMBERS_MAX] {
        self.owner_records_after
    }

    pub(in crate::support) const fn lifecycle_before(&self) -> LifecycleAggregate {
        self.preview.lifecycle_before
    }

    pub(in crate::support) const fn lifecycle_after(&self) -> LifecycleAggregate {
        self.preview.lifecycle_after
    }

    pub(in crate::support) fn retractions(&self) -> &[LifecyclePublication] {
        self.preview.retractions()
    }
}

impl SupportC17 {
    pub(crate) fn inspect_plan_disposition(
        &self,
        authority_key: [u8; 17],
        identity: [u8; PLAN_IDENTITY_BYTES],
        disposition: PlanDisposition,
        occurred_at: u64,
    ) -> Result<RootBatchPreview, SupportLedgerError> {
        if occurred_at == 0 {
            return Err(SupportLedgerError::InvalidInput);
        }
        let roots = [
            self.plan_root(authority_key, identity, 0)?,
            self.plan_root(authority_key, identity, 1)?,
            self.plan_root(authority_key, identity, 2)?,
        ];
        if roots
            .iter()
            .any(|root| root.state != RootState::Conditional)
            || roots.iter().any(|root| occurred_at <= root.occurred_at)
        {
            return Err(SupportLedgerError::InvalidTransition);
        }
        let (operation, cause, transitions, resolver) = match disposition {
            PlanDisposition::Receipt => (
                SemanticOperation::ReceiptDisposition,
                FormationCause::Receipt,
                [
                    Some((roots[0], RootState::Pending, 0)),
                    Some((roots[2], RootState::ClosedConditional, 1)),
                    None,
                    None,
                    None,
                ],
                ResolverChange::Keep,
            ),
            PlanDisposition::Rejection => (
                SemanticOperation::RejectionResult,
                FormationCause::Rejection,
                [
                    Some((roots[0], RootState::ClosedConditional, 2)),
                    Some((roots[1], RootState::ClosedConditional, 2)),
                    Some((roots[2], RootState::Pending, 0)),
                    None,
                    None,
                ],
                ResolverChange::MoveTo(2),
            ),
            PlanDisposition::LocalStale => (
                SemanticOperation::LocalStaleResult,
                FormationCause::LocalStale,
                [
                    Some((roots[0], RootState::ClosedConditional, 3)),
                    Some((roots[1], RootState::ClosedConditional, 3)),
                    Some((roots[2], RootState::Pending, 0)),
                    None,
                    None,
                ],
                ResolverChange::MoveTo(2),
            ),
        };
        self.preview_root_batch(operation, cause, transitions, resolver, occurred_at)
    }

    pub(crate) fn inspect_observation_resolution(
        &self,
        authority_key: [u8; 17],
        identity: [u8; PLAN_IDENTITY_BYTES],
        resolution: ObservationResolution,
        occurred_at: u64,
    ) -> Result<RootBatchPreview, SupportLedgerError> {
        if occurred_at == 0 {
            return Err(SupportLedgerError::InvalidInput);
        }
        let observation = self.plan_root(authority_key, identity, 0)?;
        let continuation = self.plan_root(authority_key, identity, 1)?;
        let rejection = self.plan_root(authority_key, identity, 2)?;
        if observation.state != RootState::Active
            || continuation.state != RootState::Conditional
            || rejection.state != RootState::ClosedConditional
            || occurred_at <= observation.occurred_at
            || occurred_at <= continuation.occurred_at
        {
            return Err(SupportLedgerError::InvalidTransition);
        }
        let (operation, continuation_after, reason, resolver) = match resolution {
            ObservationResolution::DescriptionsRequired => (
                SemanticOperation::ResolveObservationDescriptions,
                RootState::Pending,
                0,
                ResolverChange::MoveTo(1),
            ),
            ObservationResolution::Other => (
                SemanticOperation::ResolveObservationOther,
                RootState::ClosedConditional,
                4,
                ResolverChange::Retire,
            ),
        };
        let mut preview = self.preview_root_batch(
            operation,
            FormationCause::ObservationCompleted,
            [
                Some((observation, RootState::Retained, 0)),
                Some((continuation, continuation_after, reason)),
                None,
                None,
                None,
            ],
            resolver,
            occurred_at,
        )?;
        let (records, record_count, before, after, retractions, retraction_count) =
            self.inspect_observation_lifecycle(observation, continuation, resolution)?;
        add_lifecycle_aggregate_delta(&mut preview.aggregate, before, after)?;
        preview.resolution_records = records;
        preview.resolution_record_count = record_count;
        preview.lifecycle_before = before;
        preview.lifecycle_after = after;
        preview.retractions = retractions;
        preview.retraction_count = retraction_count;
        Ok(preview)
    }

    fn inspect_observation_lifecycle(
        &self,
        observation: RootSnapshot,
        continuation: RootSnapshot,
        resolution: ObservationResolution,
    ) -> Result<
        (
            [Option<ResolutionRecordSnapshot>; RESOLUTION_RECORD_MAX],
            usize,
            LifecycleAggregate,
            LifecycleAggregate,
            [LifecyclePublication; RESOLUTION_PUBLICATION_MAX],
            usize,
        ),
        SupportLedgerError,
    > {
        if self.pending_state()? != PendingState::Empty
            || observation.member_count != continuation.member_count
        {
            return Err(SupportLedgerError::InvalidTransition);
        }
        let mut resolver_links = [ArenaRef::default(); PLAN_MEMBERS_MAX];
        for (index, member) in observation.members[..observation.member_count]
            .iter()
            .copied()
            .enumerate()
        {
            let row_ref = self.owner_rows.reference_at(member.owner.slot, &[1])?;
            let row = self.owner_rows.image(row_ref, &[1])?;
            let link =
                decode_optional_arena_ref(&row[OWNER_ROW_ACTIVE_LINK..OWNER_ROW_ACTIVE_LINK + 8])?
                    .ok_or(SupportLedgerError::InvalidTransition)?;
            let link_image = self.links.image(link, &[1])?;
            if link_image[8] != 1
                || decode_arena_ref(&link_image[16..24])? != member.owner
                || decode_arena_ref(&link_image[24..32])? != observation.group
                || decode_arena_ref(&link_image[32..40])? != observation.initial_formation
            {
                return Err(noncanonical_error());
            }
            resolver_links[index] = link;
        }

        let mut records = [None; RESOLUTION_RECORD_MAX];
        let mut record_count = 0usize;
        let mut before = LifecycleAggregate::ZERO;
        let mut after = LifecycleAggregate::ZERO;
        let mut retractions = [LifecyclePublication::ZERO; RESOLUTION_PUBLICATION_MAX];
        let mut retraction_count = 0usize;
        for slot in 0..self.lifecycle.capacity() {
            let Some(reference) = self.lifecycle.reference_if_occupied(slot as u32)? else {
                continue;
            };
            let image = *self.lifecycle.image(reference, &[1])?;
            if image[488] != 0 {
                validate_closed_lifecycle_image(&image)?;
                continue;
            }
            let record = LifecycleRecordInput::decode(&image)?;
            self.validate_lifecycle_record_owner_set(record, reference)?;
            let owner_count = record
                .owners
                .iter()
                .position(|owner| *owner == LifecycleOwnerRow::ZERO)
                .unwrap_or(PLAN_MEMBERS_MAX);
            let linked = record.owners[..owner_count].iter().any(|owner| {
                decode_arena_ref(&owner.link.to_le_bytes())
                    .is_ok_and(|link| resolver_links[..observation.member_count].contains(&link))
            });
            if !linked {
                continue;
            }
            if record_count == RESOLUTION_RECORD_MAX
                || owner_count == 0
                || owner_count != observation.member_count
                || record.owners[..owner_count].iter().any(|owner| {
                    decode_arena_ref(&owner.link.to_le_bytes()).map_or(true, |link| {
                        !resolver_links[..observation.member_count].contains(&link)
                    })
                })
                || record.final_owner[..17] != observation.authority_key
                || !matches!(record.final_owner[17], 0 | 1)
            {
                return Err(SupportLedgerError::InvalidTransition);
            }
            for (kind, key) in [
                (RawOwnerKind::LifecycleObligation, record.obligation_raw),
                (RawOwnerKind::LifecycleCredit, record.credit_raw),
            ] {
                let value = self.raw.find(&key)?.ok_or_else(noncanonical_error)?;
                let (actual_kind, state, owner) = decode_raw_owner(value)?;
                if actual_kind != kind || state != RawOwnerState::Committed || owner != reference {
                    return Err(noncanonical_error());
                }
            }
            before.accrue(record)?;
            if resolution == ObservationResolution::DescriptionsRequired {
                let mut transferred = record;
                transferred.aggregate[0] = RootState::Pending as u64 - 1;
                for owner in &mut transferred.owners[..owner_count] {
                    owner.class = RootState::Pending as u64 - 1;
                }
                after.accrue(transferred)?;
            } else {
                let axis = u8::try_from(record.aggregate[2]).map_err(|_| noncanonical_error())?;
                let horizon =
                    u8::try_from(record.aggregate[3]).map_err(|_| noncanonical_error())?;
                for owner in record.owners[..owner_count].iter().copied() {
                    if retraction_count == retractions.len() {
                        return Err(capacity_error());
                    }
                    let owner_ref = decode_arena_ref(&owner.owner.to_le_bytes())?;
                    let member = decode_arena_ref(&owner.source.to_le_bytes())?;
                    let member_image = self.members.image(member, &[1])?;
                    retractions[retraction_count] = LifecyclePublication {
                        owner_slot: owner_ref.slot,
                        funder: decode_arena_ref(&member_image[24..32])?,
                        branch: record.final_owner[17],
                        axis,
                        horizon,
                        zero: 0,
                    };
                    retraction_count += 1;
                }
            }
            records[record_count] = Some(ResolutionRecordSnapshot {
                reference,
                record,
                image,
            });
            record_count += 1;
        }
        Ok((
            records,
            record_count,
            before,
            after,
            retractions,
            retraction_count,
        ))
    }

    pub(crate) fn plan_root_anchor(
        &self,
        authority_key: [u8; 17],
        identity: [u8; PLAN_IDENTITY_BYTES],
        branch: u8,
    ) -> Result<RootAnchor, SupportLedgerError> {
        let root = self.plan_root(authority_key, identity, branch)?;
        Ok(RootAnchor {
            authority_key,
            branch,
            group: root.group,
            root: root.group,
            version: root.version,
        })
    }

    #[cfg(test)]
    pub(in crate::support) fn root_facts_for_test(
        &self,
        anchor: RootAnchor,
    ) -> Result<(RootState, usize, ArenaRef, ArenaRef, ArenaRef), SupportLedgerError> {
        let root = self.root_from_anchor(anchor)?;
        Ok((
            root.state,
            root.member_count,
            root.formation,
            root.initial_formation,
            root.locator,
        ))
    }

    #[cfg(test)]
    pub(in crate::support) fn owner_active_link_for_test(
        &self,
        slot: u32,
    ) -> Result<(ArenaRef, ArenaRef, ArenaRef), SupportLedgerError> {
        let row_ref = self.owner_rows.reference_at(slot, &[1])?;
        let row = self.owner_rows.image(row_ref, &[1])?;
        let link =
            decode_optional_arena_ref(&row[OWNER_ROW_ACTIVE_LINK..OWNER_ROW_ACTIVE_LINK + 8])?
                .ok_or(SupportLedgerError::InvalidTransition)?;
        let image = self.links.image(link, &[1])?;
        if image[8] != 1 {
            return Err(noncanonical_error());
        }
        Ok((
            link,
            decode_arena_ref(&image[24..32])?,
            decode_arena_ref(&image[32..40])?,
        ))
    }

    #[cfg(test)]
    pub(in crate::support) fn root_formation_for_test(
        &self,
        anchor: RootAnchor,
    ) -> Result<[u8; FORMATION_BYTES], SupportLedgerError> {
        let root = self.root_from_anchor(anchor)?;
        Ok(*self.formations.image(root.formation, &[1])?)
    }

    #[cfg(test)]
    pub(in crate::support) fn owner_currents_for_test(
        &self,
        slot: u32,
    ) -> Result<(u64, [u64; 4], bool), SupportLedgerError> {
        let reference = self.owner_rows.reference_at(slot, &[1])?;
        let row = self.owner_rows.image(reference, &[1])?;
        let branches =
            std::array::from_fn(|branch| read_u64(row, OWNER_ROW_BRANCH_CURRENT + branch * 8));
        let active =
            decode_optional_arena_ref(&row[OWNER_ROW_ACTIVE_LINK..OWNER_ROW_ACTIVE_LINK + 8])?
                .is_some();
        Ok((read_u64(row, OWNER_ROW_CURRENT), branches, active))
    }

    pub(crate) fn inspect_membership_root_action(
        &self,
        anchor: crate::request_book::c17::SupportMembershipAnchor,
        action: RootAction,
        occurred_at: u64,
    ) -> Result<RootBatchPreview, SupportLedgerError> {
        if anchor.is_absent()
            || anchor.group() != anchor.root()
            || !matches!(anchor.branch(), 0..=3)
        {
            return Err(SupportLedgerError::InvalidInput);
        }
        let current =
            self.root_at_group(anchor.group(), anchor.authority_key(), anchor.branch())?;
        self.inspect_root_action(
            RootAnchor {
                authority_key: current.authority_key,
                branch: current.branch,
                group: current.group,
                root: current.group,
                version: current.version,
            },
            action,
            occurred_at,
        )
    }

    pub(crate) fn inspect_root_action(
        &self,
        anchor: RootAnchor,
        action: RootAction,
        occurred_at: u64,
    ) -> Result<RootBatchPreview, SupportLedgerError> {
        if occurred_at == 0 || anchor.group != anchor.root {
            return Err(SupportLedgerError::InvalidInput);
        }
        let root = self.root_from_anchor(anchor)?;
        if occurred_at <= root.occurred_at {
            return Err(SupportLedgerError::InvalidTransition);
        }
        let (operation, after, cause, reason, resolver) = match action {
            RootAction::MarkPredecessorEnded => {
                if root.authority_key[0] != 0x31
                    || root.branch != 3
                    || root.state != RootState::Conditional
                {
                    return Err(SupportLedgerError::InvalidTransition);
                }
                (
                    SemanticOperation::MarkPredecessorEnded,
                    RootState::Pending,
                    FormationCause::PredecessorEnded,
                    0,
                    ResolverChange::Keep,
                )
            }
            RootAction::Begin => {
                if root.state != RootState::Pending {
                    return Err(SupportLedgerError::InvalidTransition);
                }
                (
                    SemanticOperation::Begin,
                    RootState::Active,
                    FormationCause::BeganSupport,
                    0,
                    ResolverChange::Keep,
                )
            }
            RootAction::Finish => {
                if root.state != RootState::Active
                    || (root.authority_key[0] == 0x30 && root.branch == 0)
                {
                    return Err(SupportLedgerError::InvalidTransition);
                }
                (
                    SemanticOperation::Finish,
                    RootState::Retained,
                    FormationCause::FinishedSupport,
                    0,
                    ResolverChange::Retire,
                )
            }
        };
        self.preview_root_batch(
            operation,
            cause,
            [Some((root, after, reason)), None, None, None, None],
            resolver,
            occurred_at,
        )
    }

    pub(crate) fn inspect_typed_close(
        &self,
        input: crate::TypedCloseInput,
    ) -> Result<RootBatchPreview, SupportLedgerError> {
        let occurred_at = input.occurred_at.as_micros();
        if occurred_at == 0 || input.group != input.root.slot() {
            return Err(SupportLedgerError::InvalidInput);
        }
        if matches!(
            input.authority,
            crate::CloseAuthority::Standalone { source, .. }
                if source.reserved != 0 || source.generation() == 0
        ) {
            return Err(SupportLedgerError::InvalidInput);
        }
        let branch = input.branch.ordinal();
        let group = ArenaRef {
            slot: input.root.slot(),
            generation: input.root.generation(),
        };
        let (root, operation, authority_image, plan_event) = match input.authority {
            crate::CloseAuthority::Plan { identity, event } => {
                let operation = match input.branch {
                    crate::PlanBranch::Continuation => SemanticOperation::TypedCloseC,
                    crate::PlanBranch::Rejection => SemanticOperation::TypedCloseR,
                    _ => return Err(SupportLedgerError::InvalidTransition),
                };
                let authority_key = plan_authority_key_for_semantic(identity.id.get());
                let root = self.plan_root(
                    authority_key,
                    crate::support::encode_plan_identity(identity),
                    branch,
                )?;
                validate_typed_close_root(input.root, group, root)?;
                if root.state != RootState::Pending {
                    return Err(SupportLedgerError::InvalidTransition);
                }
                let expected_event = read_u64(&self.header.0, NEXT_PLAN_CAUSAL_EVENT);
                if event.get() != expected_event {
                    return Err(SupportLedgerError::InvalidTransition);
                }
                let next_event = expected_event.checked_add(1).ok_or_else(capacity_error)?;
                (
                    root,
                    operation,
                    encode_close_authority(CLOSE_AUTHORITY_PLAN, event.get(), 0, 0),
                    Some((expected_event, next_event)),
                )
            }
            crate::CloseAuthority::Standalone {
                domain,
                source,
                event,
            } => {
                if input.branch != crate::PlanBranch::Standalone {
                    return Err(SupportLedgerError::InvalidTransition);
                }
                let authority_key = standalone_authority_key_for_semantic(domain.get());
                let group_image = self.groups.image(group, &[1])?;
                let mut stored_authority_key = [0; 17];
                stored_authority_key.copy_from_slice(&group_image[40..57]);
                if stored_authority_key == [0; 17] {
                    return Err(noncanonical_error());
                }
                let root = self.root_from_anchor(RootAnchor {
                    authority_key: stored_authority_key,
                    branch,
                    group,
                    root: group,
                    version: input.root.version(),
                })?;
                validate_typed_close_root(input.root, group, root)?;
                if stored_authority_key != authority_key {
                    return Err(SupportLedgerError::InvalidTransition);
                }
                if !matches!(root.state, RootState::Conditional | RootState::Pending) {
                    return Err(SupportLedgerError::InvalidTransition);
                }
                let initial = self.formations.image(root.initial_formation, &[1])?;
                if initial[220] != branch
                    || initial[221] != RootState::Conditional as u8
                    || initial[222] != FormationCause::InitialReady as u8
                    || initial[8..16] != encode_source_record_ref(source)
                    || read_u64(initial, 16) != event.get()
                    || initial[24..40] != domain.get().to_be_bytes()
                    || initial[104..121] != authority_key
                {
                    return Err(SupportLedgerError::InvalidTransition);
                }
                (
                    root,
                    SemanticOperation::TypedCloseStandalone,
                    encode_close_authority(
                        CLOSE_AUTHORITY_STANDALONE,
                        event.get(),
                        u64::from_le_bytes(encode_source_record_ref(source)),
                        0,
                    ),
                    None,
                )
            }
            crate::CloseAuthority::Cancellation {
                fact,
                event,
                request_generation,
            } => {
                if input.branch != crate::PlanBranch::Terminal {
                    return Err(SupportLedgerError::InvalidTransition);
                }
                let group_image = self.groups.image(group, &[1])?;
                let mut authority_key = [0; 17];
                authority_key.copy_from_slice(&group_image[40..57]);
                if authority_key == [0; 17] {
                    return Err(noncanonical_error());
                }
                let root = self.root_from_anchor(RootAnchor {
                    authority_key,
                    branch,
                    group,
                    root: group,
                    version: input.root.version(),
                })?;
                validate_typed_close_root(input.root, group, root)?;
                if root.state != RootState::Pending {
                    return Err(SupportLedgerError::InvalidTransition);
                }
                let initial = self.formations.image(root.initial_formation, &[1])?;
                if initial[220] != branch
                    || initial[221] != RootState::Pending as u8
                    || initial[222] != FormationCause::CancellationMembership as u8
                    || read_u64(initial, 8) != event.get()
                    || read_u64(initial, 16) != fact.get()
                    || read_u64(initial, 24) != request_generation.get()
                {
                    return Err(SupportLedgerError::InvalidTransition);
                }
                (
                    root,
                    SemanticOperation::TypedCloseTerminal,
                    encode_close_authority(
                        CLOSE_AUTHORITY_CANCELLATION,
                        event.get(),
                        fact.get(),
                        request_generation.get(),
                    ),
                    None,
                )
            }
        };
        if occurred_at <= root.occurred_at {
            return Err(SupportLedgerError::InvalidTransition);
        }
        let after = match root.state {
            RootState::Conditional => RootState::ClosedConditional,
            RootState::Pending => RootState::ClosedPending,
            _ => return Err(SupportLedgerError::InvalidTransition),
        };
        let resolver = if operation == SemanticOperation::TypedCloseTerminal {
            ResolverChange::Keep
        } else {
            ResolverChange::Retire
        };
        let mut preview = self.preview_root_batch(
            operation,
            FormationCause::TypedImpossible,
            [
                Some((root, after, input.reason.get())),
                None,
                None,
                None,
                None,
            ],
            resolver,
            occurred_at,
        )?;
        preview.transitions[0]
            .as_mut()
            .expect("typed close transition")
            .close_authority = authority_image;
        preview.plan_event = plan_event;
        Ok(preview)
    }
}
