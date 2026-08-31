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

    pub(in crate::support) fn prepare_root_batch(
        &self,
        preview: RootBatchPreview,
        owner_records: [Option<BundleRecord>; PLAN_MEMBERS_MAX],
        work: &mut impl WorkRecorder,
    ) -> Result<PreparedRootBatch, SupportLedgerError> {
        self.validate_preview(&preview)?;
        let generation_after = self
            .generation()
            .checked_add(1)
            .ok_or_else(capacity_error)?;
        let expected_arena_headers = self.semantic_arena_headers();
        if let Some((offset, maximum)) = preview.operation.retained_budget()
            && read_u32(&self.header.0, offset) >= maximum
        {
            return Err(capacity_error());
        }
        let transition_count = preview.transition_count;
        let formations = self
            .formations
            .prepare_reserve::<ROOT_BATCH_MAX>(transition_count)?;
        let funders = self
            .funders
            .prepare_reserve::<ROOT_BATCH_FUNDER_MAX>(transition_count * PLAN_MEMBERS_MAX)?;
        let wrapper_count = preview.transitions[..transition_count]
            .iter()
            .flatten()
            .filter(|transition| transition.before.locator_kind == 2)
            .count();
        let wrappers = if wrapper_count == 0 {
            ArenaSelection::empty()
        } else {
            self.wrappers
                .prepare_reserve::<ROOT_BATCH_MAX>(wrapper_count)?
        };
        let mutations = self
            .mutations
            .prepare_reserve::<ROOT_BATCH_MAX>(transition_count)?;
        let link_count = if matches!(preview.resolver, ResolverChange::MoveTo(_)) {
            preview.owner_count
        } else {
            0
        };
        let links = if link_count == 0 {
            ArenaSelection::empty()
        } else {
            self.links.prepare_reserve::<PLAN_MEMBERS_MAX>(link_count)?
        };
        self.groups.validate_advance_generation()?;
        self.members.validate_advance_generation()?;
        let touches_external = preview.transitions[..transition_count]
            .iter()
            .flatten()
            .any(|transition| transition.before.locator_kind == 1);
        if touches_external {
            self.external_heads.validate_advance_generation()?;
        }
        if preview.owner_count > 0 {
            self.owner_rows.validate_advance_generation()?;
            self.owners.validate_advance_generation()?;
        }
        if !matches!(preview.resolver, ResolverChange::Keep) {
            self.links.validate_advance_generation()?;
        }

        let mut journals = [None; ROOT_BATCH_MAX];
        let mut local_entries = [([0; 17], [0; 8]); ROOT_BATCH_LOCAL_MAX];
        let mut local_count = 0;
        let mut wrapper_index = 0;
        for transition_index in 0..transition_count {
            let spec = preview.transitions[transition_index].expect("active root transition");
            let formation = formations[transition_index];
            let locator_after_ref = if spec.before.locator_kind == 1 {
                spec.before.locator
            } else {
                let reference = wrappers[wrapper_index];
                wrapper_index += 1;
                reference
            };
            let mut group_after = spec.before.group_image;
            group_after[9] = spec.after as u8;
            encode_arena_ref(&mut group_after[16..24], formation);
            encode_arena_ref(&mut group_after[24..32], locator_after_ref);
            write_u64(&mut group_after, 32, spec.before.version + 1);
            let mut locator_after = spec.before.locator_image;
            if spec.before.locator_kind == 2 {
                locator_after[..8].fill(0);
            }
            locator_after[9] = spec.after as u8;
            encode_arena_ref(&mut locator_after[24..32], formation);
            let locator_version = if spec.before.locator_kind == 1 {
                120
            } else {
                56
            };
            write_u64(&mut locator_after, locator_version, spec.before.version + 1);
            if spec.before.locator_kind == 2 {
                locator_after = self.wrappers.prepare_reserved_image_after(
                    locator_after_ref,
                    locator_after,
                    1,
                )?;
            }
            let formation_after = self.formations.prepare_reserved_image_after(
                formation,
                encode_successor_formation(
                    spec,
                    formation,
                    locator_after_ref,
                    preview.operation,
                    transition_index,
                ),
                1,
            )?;
            let mut funder_after = [[0; FUNDER_BYTES]; PLAN_MEMBERS_MAX];
            let mut member_after = [[0; MEMBER_BYTES]; PLAN_MEMBERS_MAX];
            for ordinal in 0..PLAN_MEMBERS_MAX {
                let member = spec.before.members[ordinal];
                let next_funder = funders[transition_index * PLAN_MEMBERS_MAX + ordinal];
                let mut funder = *self.funders.image(member.funder, &[1])?;
                funder[..8].fill(0);
                funder[10] = u8::try_from(spec.before.version + 1).map_err(|_| capacity_error())?;
                encode_arena_ref(&mut funder[24..32], formation);
                let mut member_image = *self.members.image(member.member, &[1])?;
                encode_arena_ref(&mut member_image[24..32], next_funder);
                funder_after[ordinal] =
                    self.funders
                        .prepare_reserved_image_after(next_funder, funder, 1)?;
                member_after[ordinal] = member_image;
                local_entries[local_count] = (
                    local_key(LocalKind::Funder, next_funder),
                    encode_arena_ref_value(next_funder),
                );
                local_count += 1;
            }
            let mutation = mutations[transition_index];
            let mutation_after = self.mutations.prepare_reserved_image_after(
                mutation,
                encode_root_mutation(
                    preview.operation,
                    transition_index,
                    spec,
                    formation,
                    self.generation(),
                ),
                1,
            )?;
            local_entries[local_count] = (
                local_key(LocalKind::Mutation, mutation),
                encode_arena_ref_value(mutation),
            );
            local_count += 1;
            journals[transition_index] = Some(RootJournal {
                before: spec.before,
                locator_after_ref,
                group_after,
                locator_after,
                formation_after,
                funder_after,
                member_after,
                mutation_after,
            });
        }

        let mut owner_records_after = owner_records;
        let mut owner_references = [[ArenaRef::default(); 4]; PLAN_MEMBERS_MAX];
        let mut owner_rows_after = [[0; OWNER_ROW_BYTES]; PLAN_MEMBERS_MAX];
        let mut owners_after = [[0; OWNER_BYTES]; PLAN_MEMBERS_MAX];
        let mut retired_links = [ArenaRef::default(); PLAN_MEMBERS_MAX];
        let mut retired_link_before = [[0; LINK_BYTES]; PLAN_MEMBERS_MAX];
        let mut retired_link_after = [[0; LINK_BYTES]; PLAN_MEMBERS_MAX];
        let mut new_link_images = [[0; LINK_BYTES]; PLAN_MEMBERS_MAX];
        let resolver_before = preview.transitions[0]
            .expect("root batch has one transition")
            .before;
        let resolver_target = match preview.resolver {
            ResolverChange::MoveTo(index) => Some((
                preview.transitions[index]
                    .ok_or(SupportLedgerError::InvalidInput)?
                    .before
                    .group,
                preview.transitions[index]
                    .ok_or(SupportLedgerError::InvalidInput)?
                    .before
                    .initial_formation,
            )),
            _ => None,
        };
        for ordinal in 0..preview.owner_count {
            let delta = preview.owners[ordinal];
            let before_record =
                owner_records[ordinal].ok_or(SupportLedgerError::InvalidTransition)?;
            if before_record.request_owner
                != request_id_from_key_for_support(resolver_before.members[ordinal].request_key)?
                || delta.slot != delta.owner.slot
            {
                return Err(SupportLedgerError::InvalidTransition);
            }
            owner_references[ordinal] = [
                self.owner_headers.reference_at(delta.slot, &[1])?,
                self.owner_rows.reference_at(delta.slot, &[1])?,
                self.owner_indices.reference_at(delta.slot, &[1])?,
                self.owners.reference_at(delta.slot, &[1])?,
            ];
            if owner_references[ordinal][0] != delta.owner {
                return Err(noncanonical_error());
            }
            validate_c16_owner_set(
                [
                    self.owner_headers
                        .image(owner_references[ordinal][0], &[1])?
                        .as_slice(),
                    self.owner_rows
                        .image(owner_references[ordinal][1], &[1])?
                        .as_slice(),
                    self.owner_indices
                        .image(owner_references[ordinal][2], &[1])?
                        .as_slice(),
                    self.owners
                        .image(owner_references[ordinal][3], &[1])?
                        .as_slice(),
                ],
                owner_references[ordinal],
                delta.slot,
                &before_record,
                OWNER_STATE_LIVE,
            )?;
            let mut record_after = before_record;
            record_after.linked_claims = apply_i32_u32(record_after.linked_claims, delta.linked)?;
            let mut row = *self.owner_rows.image(owner_references[ordinal][1], &[1])?;
            let mut owner = *self.owners.image(owner_references[ordinal][3], &[1])?;
            write_u32(
                &mut row,
                OWNER_ROW_LINKED_CLAIMS,
                record_after.linked_claims,
            );
            let current_after = apply_i32_u64(read_u64(&row, OWNER_ROW_CURRENT), delta.linked)?;
            write_u64(&mut row, OWNER_ROW_CURRENT, current_after);
            for branch in 0..4 {
                let offset = OWNER_ROW_BRANCH_CURRENT + branch * 8;
                let branch_after = apply_i32_u64(read_u64(&row, offset), delta.branch[branch])?;
                write_u64(&mut row, offset, branch_after);
            }
            write_u32(
                &mut owner,
                OWNER_IMAGE_LINKED_CLAIMS,
                record_after.linked_claims,
            );
            if !matches!(preview.resolver, ResolverChange::Keep) {
                let active = decode_optional_arena_ref(
                    &row[OWNER_ROW_ACTIVE_LINK..OWNER_ROW_ACTIVE_LINK + 8],
                )?
                .ok_or(SupportLedgerError::InvalidTransition)?;
                let before_link = *self.links.image(active, &[1])?;
                if before_link[8] != 1
                    || decode_arena_ref(&before_link[16..24])? != delta.owner
                    || decode_arena_ref(&before_link[24..32])? != resolver_before.group
                    || decode_arena_ref(&before_link[32..40])? != resolver_before.initial_formation
                {
                    return Err(noncanonical_error());
                }
                let mut after_link = before_link;
                after_link[8] = 0;
                write_u64(&mut after_link, 80, self.generation() + 1);
                write_u64(
                    &mut after_link,
                    88,
                    preview.transitions[0].expect("transition").occurred_at,
                );
                retired_links[ordinal] = active;
                retired_link_before[ordinal] = before_link;
                retired_link_after[ordinal] = after_link;
                match resolver_target {
                    Some((group, formation)) => {
                        let next = links[ordinal];
                        encode_arena_ref(
                            &mut row[OWNER_ROW_ACTIVE_LINK..OWNER_ROW_ACTIVE_LINK + 8],
                            next,
                        );
                        new_link_images[ordinal] = self.links.prepare_reserved_image_after(
                            next,
                            encode_plan_link(
                                delta.owner,
                                group,
                                formation,
                                resolver_before.authority_key,
                                self.generation() + 1,
                            ),
                            1,
                        )?;
                        local_entries[local_count] = (
                            local_key(LocalKind::Link, next),
                            encode_arena_ref_value(next),
                        );
                        local_count += 1;
                    }
                    None => row[OWNER_ROW_ACTIVE_LINK..OWNER_ROW_ACTIVE_LINK + 8].fill(0),
                }
            }
            owner_records_after[ordinal] = Some(record_after);
            owner_rows_after[ordinal] = row;
            owners_after[ordinal] = owner;
        }
        let (resolution_journals, raw_updates, raw_update_count) = self
            .prepare_resolution_journals(
                &preview,
                &formations,
                &funders,
                &links,
                &mut journals,
                &owner_references,
                &mut owner_records_after,
                &mut owner_rows_after,
                &mut owners_after,
            )?;
        if owner_records[preview.owner_count..]
            .iter()
            .any(Option::is_some)
        {
            return Err(SupportLedgerError::InvalidInput);
        }
        local_entries[..local_count].sort_unstable_by_key(|entry| entry.0);
        if local_entries[..local_count]
            .windows(2)
            .any(|pair| pair[0].0 >= pair[1].0)
        {
            return Err(SupportLedgerError::InvalidInput);
        }
        self.local
            .validate_insert_batch(&local_entries[..local_count])?;
        let local_plan = self.local.prepare_insert_assignment_plan(
            LOCAL_INDEX_ASSIGNMENT_ARENA,
            &local_entries[..local_count],
        )?;
        let raw_plan = if raw_update_count == 0 {
            None
        } else {
            let mut updates = [([0; 32], NodeHandle::SENTINEL, [0; 8]); RESOLUTION_RAW_MAX];
            for (index, update) in raw_updates[..raw_update_count]
                .iter()
                .copied()
                .flatten()
                .enumerate()
            {
                updates[index] = (update.key, update.handle, update.after);
            }
            Some(self.raw.prepare_update_assignment_plan(
                RAW_INDEX_ASSIGNMENT_ARENA,
                &updates[..raw_update_count],
            )?)
        };
        let arena_headers_after = self.prepare_semantic_arena_headers_after(
            &preview,
            &formations,
            &funders,
            &wrappers,
            &mutations,
            &links,
        )?;
        let mut header_after = self.header;
        if let Some((offset, _)) = preview.operation.retained_budget() {
            let next = read_u32(&header_after.0, offset)
                .checked_add(1)
                .ok_or_else(capacity_error)?;
            write_u32(&mut header_after.0, offset, next);
        }
        if let Some((_, after)) = preview.plan_event {
            write_u64(&mut header_after.0, NEXT_PLAN_CAUSAL_EVENT, after);
        }
        write_u64(&mut header_after.0, 48, generation_after);
        work.charge(HotPathWorkWitness::new(preview.operation.work()))?;
        Ok(PreparedRootBatch {
            expected_c17: self.generation(),
            expected_raw: self.raw.generation(),
            expected_local: self.local.generation(),
            expected_lifecycle: self.lifecycle.generation(),
            expected_arena_headers,
            arena_headers_after,
            preview,
            formations,
            funders,
            wrappers,
            wrapper_count,
            mutations,
            links,
            link_count,
            local_entries,
            local_count,
            journals,
            owner_records_before: owner_records,
            owner_records_after,
            owner_references,
            owner_rows_after,
            owners_after,
            retired_links,
            retired_link_before,
            retired_link_after,
            new_link_images,
            resolution_journals,
            raw_updates,
            raw_update_count,
            header_after,
            raw_plan,
            local_plan,
        })
    }

    fn prepare_resolution_journals(
        &self,
        preview: &RootBatchPreview,
        formations: &ArenaSelection<ROOT_BATCH_MAX>,
        funders: &ArenaSelection<ROOT_BATCH_FUNDER_MAX>,
        links: &ArenaSelection<PLAN_MEMBERS_MAX>,
        root_journals: &mut [Option<RootJournal>; ROOT_BATCH_MAX],
        owner_references: &[[ArenaRef; 4]; PLAN_MEMBERS_MAX],
        owner_records_after: &mut [Option<BundleRecord>; PLAN_MEMBERS_MAX],
        owner_rows_after: &mut [[u8; OWNER_ROW_BYTES]; PLAN_MEMBERS_MAX],
        owners_after: &mut [[u8; OWNER_BYTES]; PLAN_MEMBERS_MAX],
    ) -> Result<
        (
            [Option<ResolutionRecordJournal>; RESOLUTION_RECORD_MAX],
            [Option<ResolutionRawUpdate>; RESOLUTION_RAW_MAX],
            usize,
        ),
        SupportLedgerError,
    > {
        let mut resolution_journals = [None; RESOLUTION_RECORD_MAX];
        let mut raw_updates = [None; RESOLUTION_RAW_MAX];
        let mut raw_update_count = 0usize;
        if preview.resolution_record_count == 0 {
            return Ok((resolution_journals, raw_updates, raw_update_count));
        }
        if !matches!(
            preview.operation,
            SemanticOperation::ResolveObservationDescriptions
                | SemanticOperation::ResolveObservationOther
        ) || preview.resolution_records[..preview.resolution_record_count]
            .iter()
            .any(Option::is_none)
            || preview.resolution_records[preview.resolution_record_count..]
                .iter()
                .any(Option::is_some)
            || preview.transition_count != 2
        {
            return Err(SupportLedgerError::InvalidInput);
        }
        self.lifecycle.validate_advance_generation()?;

        let target_spec = preview.transitions[1].ok_or(SupportLedgerError::InvalidInput)?;
        let target_journal = root_journals[1].ok_or(SupportLedgerError::InvalidInput)?;
        let mut target = target_spec.before;
        target.state = target_spec.after;
        target.version = target.version.checked_add(1).ok_or_else(capacity_error)?;
        target.occurred_at = target_spec.occurred_at;
        target.formation = formations[1];
        target.locator = target_journal.locator_after_ref;
        target.group_image = target_journal.group_after;
        target.formation_image = target_journal.formation_after;
        target.locator_image = target_journal.locator_after;
        for ordinal in 0..target.member_count {
            target.members[ordinal].funder = funders[PLAN_MEMBERS_MAX + ordinal];
        }

        for record_index in 0..preview.resolution_record_count {
            let snapshot =
                preview.resolution_records[record_index].ok_or(SupportLedgerError::InvalidInput)?;
            if self.lifecycle.image(snapshot.reference, &[1])? != &snapshot.image
                || snapshot.image[488] != 0
            {
                return Err(SupportLedgerError::Generation);
            }
            let owner_count = snapshot
                .record
                .owners
                .iter()
                .position(|owner| *owner == LifecycleOwnerRow::ZERO)
                .unwrap_or(PLAN_MEMBERS_MAX);
            let mut after_image = snapshot.image;
            if preview.operation == SemanticOperation::ResolveObservationDescriptions {
                let mut transferred = snapshot.record;
                transferred.final_owner = encode_lifecycle_final_owner(target);
                transferred.owner_set_ref = encode_arena_ref_value(target.members[0].owner);
                transferred.aggregate[0] = 1;
                for owner in &mut transferred.owners[..owner_count] {
                    let owner_ref = decode_arena_ref(&owner.owner.to_le_bytes())?;
                    let owner_index = preview.owners[..preview.owner_count]
                        .iter()
                        .position(|candidate| candidate.owner == owner_ref)
                        .ok_or_else(noncanonical_error)?;
                    if owner_references[owner_index][0] != owner_ref {
                        return Err(noncanonical_error());
                    }
                    let old_group = decode_arena_ref(&owner.group.to_le_bytes())?;
                    let old_transition = preview.transitions[..preview.transition_count]
                        .iter()
                        .position(|transition| {
                            transition
                                .is_some_and(|transition| transition.before.group == old_group)
                        })
                        .ok_or_else(noncanonical_error)?;
                    let target_ordinal = target.members[..target.member_count]
                        .iter()
                        .position(|member| member.owner == owner_ref)
                        .ok_or_else(noncanonical_error)?;
                    if old_transition != 1 {
                        adjust_resolution_funder(root_journals, old_transition, owner_ref, -1)?;
                        adjust_resolution_funder(root_journals, 1, owner_ref, 1)?;
                    }
                    let old_branch = usize::from(snapshot.record.final_owner[17]);
                    let target_branch = usize::from(target.branch);
                    let row = &mut owner_rows_after[owner_index];
                    if old_branch != target_branch {
                        let old_offset = OWNER_ROW_BRANCH_CURRENT + old_branch * 8;
                        let new_offset = OWNER_ROW_BRANCH_CURRENT + target_branch * 8;
                        let old_after = read_u64(row, old_offset)
                            .checked_sub(1)
                            .ok_or_else(noncanonical_error)?;
                        let new_after = read_u64(row, new_offset)
                            .checked_add(1)
                            .ok_or_else(capacity_error)?;
                        write_u64(row, old_offset, old_after);
                        write_u64(row, new_offset, new_after);
                    }
                    owner.source = arena_ref_word(target.members[target_ordinal].member);
                    owner.group = arena_ref_word(target.group);
                    owner.root = arena_ref_word(target.locator);
                    owner.formation = arena_ref_word(target.formation);
                    owner.link = arena_ref_word(links[owner_index]);
                    owner.class = 1;
                }
                let batch = read_u64(&snapshot.image, 8);
                let ordinal = usize::from(read_u16(&snapshot.image, 16));
                after_image = transferred.encode(snapshot.reference, batch, ordinal)?;
                after_image[..24].copy_from_slice(&snapshot.image[..24]);
                after_image[480..488].copy_from_slice(&snapshot.image[480..488]);
            } else {
                after_image[488..512].fill(0);
                after_image[488] = LIFECYCLE_CLOSE_ACTION;
                after_image[489] = NO_CONTINUATION_AFTER_OBSERVATION;
                write_u64(
                    &mut after_image,
                    496,
                    preview.transitions[0]
                        .ok_or(SupportLedgerError::InvalidInput)?
                        .occurred_at,
                );
                write_u64(&mut after_image, 504, self.generation() + 1);
                validate_closed_lifecycle_image(&after_image)?;
                for owner in snapshot.record.owners[..owner_count].iter().copied() {
                    let owner_ref = decode_arena_ref(&owner.owner.to_le_bytes())?;
                    let owner_index = preview.owners[..preview.owner_count]
                        .iter()
                        .position(|candidate| candidate.owner == owner_ref)
                        .ok_or_else(noncanonical_error)?;
                    if owner_references[owner_index][0] != owner_ref {
                        return Err(noncanonical_error());
                    }
                    let old_group = decode_arena_ref(&owner.group.to_le_bytes())?;
                    let old_transition = preview.transitions[..preview.transition_count]
                        .iter()
                        .position(|transition| {
                            transition
                                .is_some_and(|transition| transition.before.group == old_group)
                        })
                        .ok_or_else(noncanonical_error)?;
                    adjust_resolution_funder(root_journals, old_transition, owner_ref, -1)?;
                    let record = owner_records_after[owner_index]
                        .as_mut()
                        .ok_or(SupportLedgerError::InvalidTransition)?;
                    record.linked_claims = record
                        .linked_claims
                        .checked_sub(1)
                        .ok_or_else(noncanonical_error)?;
                    let row = &mut owner_rows_after[owner_index];
                    let linked = read_u32(row, OWNER_ROW_LINKED_CLAIMS)
                        .checked_sub(1)
                        .ok_or_else(noncanonical_error)?;
                    let current = read_u64(row, OWNER_ROW_CURRENT)
                        .checked_sub(1)
                        .ok_or_else(noncanonical_error)?;
                    let branch_offset =
                        OWNER_ROW_BRANCH_CURRENT + usize::from(snapshot.record.final_owner[17]) * 8;
                    let branch = read_u64(row, branch_offset)
                        .checked_sub(1)
                        .ok_or_else(noncanonical_error)?;
                    write_u32(row, OWNER_ROW_LINKED_CLAIMS, linked);
                    write_u64(row, OWNER_ROW_CURRENT, current);
                    write_u64(row, branch_offset, branch);
                    write_u32(
                        &mut owners_after[owner_index],
                        OWNER_IMAGE_LINKED_CLAIMS,
                        linked,
                    );
                }
                for (kind, key) in [
                    (
                        RawOwnerKind::LifecycleObligation,
                        snapshot.record.obligation_raw,
                    ),
                    (RawOwnerKind::LifecycleCredit, snapshot.record.credit_raw),
                ] {
                    let handle = self.raw.find_handle(&key)?.ok_or_else(noncanonical_error)?;
                    let before = self.raw.value_at(handle)?;
                    let (actual_kind, state, owner) = decode_raw_owner(before)?;
                    if actual_kind != kind
                        || state != RawOwnerState::Committed
                        || owner != snapshot.reference
                        || raw_update_count == RESOLUTION_RAW_MAX
                    {
                        return Err(noncanonical_error());
                    }
                    raw_updates[raw_update_count] = Some(ResolutionRawUpdate {
                        key,
                        handle,
                        before,
                        after: encode_raw_owner(kind, RawOwnerState::Retained, snapshot.reference)?,
                    });
                    raw_update_count += 1;
                }
            }
            resolution_journals[record_index] = Some(ResolutionRecordJournal {
                snapshot,
                after: after_image,
            });
        }
        raw_updates[..raw_update_count]
            .sort_unstable_by_key(|update| update.expect("active Raw update").key);
        if raw_updates[..raw_update_count].windows(2).any(|pair| {
            pair[0].expect("active Raw update").key >= pair[1].expect("active Raw update").key
        }) {
            return Err(SupportLedgerError::InvalidInput);
        }
        if raw_update_count > 0 {
            let mut updates = [([0; 32], NodeHandle::SENTINEL, [0; 8]); RESOLUTION_RAW_MAX];
            for (index, update) in raw_updates[..raw_update_count]
                .iter()
                .copied()
                .flatten()
                .enumerate()
            {
                updates[index] = (update.key, update.handle, update.after);
            }
            self.raw
                .validate_update_batch(&updates[..raw_update_count])?;
        }
        Ok((resolution_journals, raw_updates, raw_update_count))
    }

    pub(in crate::support) fn validate_root_batch(
        &self,
        change: &PreparedRootBatch,
        owner_records: [Option<BundleRecord>; PLAN_MEMBERS_MAX],
    ) -> Result<(), SupportLedgerError> {
        self.validate_preview(&change.preview)?;
        if let Some((offset, maximum)) = change.preview.operation.retained_budget()
            && read_u32(&self.header.0, offset) >= maximum
        {
            return Err(capacity_error());
        }
        if change.local_count == 0
            || change.local_count > ROOT_BATCH_LOCAL_MAX
            || change.raw_update_count > RESOLUTION_RAW_MAX
        {
            return Err(SupportLedgerError::Generation);
        }
        let generation_after = self
            .generation()
            .checked_add(1)
            .ok_or_else(capacity_error)?;
        let expected_arena_headers = self.semantic_arena_headers();
        let arena_headers_after = self.prepare_semantic_arena_headers_after(
            &change.preview,
            &change.formations,
            &change.funders,
            &change.wrappers,
            &change.mutations,
            &change.links,
        )?;
        let mut header_after = self.header;
        if let Some((offset, _)) = change.preview.operation.retained_budget() {
            let next = read_u32(&header_after.0, offset)
                .checked_add(1)
                .ok_or_else(capacity_error)?;
            write_u32(&mut header_after.0, offset, next);
        }
        if let Some((_, after)) = change.preview.plan_event {
            write_u64(&mut header_after.0, NEXT_PLAN_CAUSAL_EVENT, after);
        }
        write_u64(&mut header_after.0, 48, generation_after);
        if self.generation() != change.expected_c17
            || self.raw.generation() != change.expected_raw
            || self.local.generation() != change.expected_local
            || self.lifecycle.generation() != change.expected_lifecycle
            || expected_arena_headers != change.expected_arena_headers
            || arena_headers_after != change.arena_headers_after
            || header_after != change.header_after
            || !self.local.validates_assignment_plan(&change.local_plan)
            || owner_records != change.owner_records_before
            || self
                .formations
                .prepare_reserve::<ROOT_BATCH_MAX>(change.preview.transition_count)?
                .as_slice()
                != change.formations.as_slice()
            || self
                .funders
                .prepare_reserve::<ROOT_BATCH_FUNDER_MAX>(
                    change.preview.transition_count * PLAN_MEMBERS_MAX,
                )?
                .as_slice()
                != change.funders.as_slice()
            || change.wrapper_count
                != change.preview.transitions[..change.preview.transition_count]
                    .iter()
                    .flatten()
                    .filter(|transition| transition.before.locator_kind == 2)
                    .count()
            || change.wrappers.len() != change.wrapper_count
            || (change.wrapper_count > 0
                && self
                    .wrappers
                    .prepare_reserve::<ROOT_BATCH_MAX>(change.wrapper_count)?
                    .as_slice()
                    != change.wrappers.as_slice())
            || self
                .mutations
                .prepare_reserve::<ROOT_BATCH_MAX>(change.preview.transition_count)?
                .as_slice()
                != change.mutations.as_slice()
            || change.links.len() != change.link_count
            || (change.link_count > 0
                && self
                    .links
                    .prepare_reserve::<PLAN_MEMBERS_MAX>(change.link_count)?
                    .as_slice()
                    != change.links.as_slice())
            || change.local_entries[change.local_count..]
                .iter()
                .any(|entry| *entry != ([0; 17], [0; 8]))
            || change.resolution_journals[..change.preview.resolution_record_count]
                .iter()
                .any(Option::is_none)
            || change.resolution_journals[change.preview.resolution_record_count..]
                .iter()
                .any(Option::is_some)
            || change.raw_update_count > RESOLUTION_RAW_MAX
            || change.raw_updates[..change.raw_update_count]
                .iter()
                .any(Option::is_none)
            || change.raw_updates[change.raw_update_count..]
                .iter()
                .any(Option::is_some)
        {
            return Err(SupportLedgerError::Generation);
        }
        for index in 0..change.preview.owner_count {
            let references = change.owner_references[index];
            validate_c16_owner_set(
                [
                    self.owner_headers.image(references[0], &[1])?.as_slice(),
                    self.owner_rows.image(references[1], &[1])?.as_slice(),
                    self.owner_indices.image(references[2], &[1])?.as_slice(),
                    self.owners.image(references[3], &[1])?.as_slice(),
                ],
                references,
                change.preview.owners[index].slot,
                &owner_records[index].ok_or(SupportLedgerError::Generation)?,
                OWNER_STATE_LIVE,
            )?;
            if !matches!(change.preview.resolver, ResolverChange::Keep) {
                let reference = change.retired_links[index];
                if self.links.image(reference, &[1])? != &change.retired_link_before[index] {
                    return Err(SupportLedgerError::Generation);
                }
            }
        }
        for journal in change.resolution_journals[..change.preview.resolution_record_count]
            .iter()
            .copied()
            .flatten()
        {
            if self.lifecycle.image(journal.snapshot.reference, &[1])? != &journal.snapshot.image {
                return Err(SupportLedgerError::Generation);
            }
        }
        if change.preview.resolution_record_count > 0 {
            self.lifecycle.validate_advance_generation()?;
        }
        let expected_raw_plan = if change.raw_update_count > 0 {
            let mut updates = [([0; 32], NodeHandle::SENTINEL, [0; 8]); RESOLUTION_RAW_MAX];
            for (index, update) in change.raw_updates[..change.raw_update_count]
                .iter()
                .copied()
                .flatten()
                .enumerate()
            {
                if self.raw.value_at(update.handle)? != update.before {
                    return Err(SupportLedgerError::Generation);
                }
                updates[index] = (update.key, update.handle, update.after);
            }
            self.raw
                .validate_update_batch(&updates[..change.raw_update_count])?;
            Some(self.raw.prepare_update_assignment_plan(
                RAW_INDEX_ASSIGNMENT_ARENA,
                &updates[..change.raw_update_count],
            )?)
        } else {
            None
        };
        self.local
            .validate_insert_batch(&change.local_entries[..change.local_count])?;
        let expected_local_plan = self.local.prepare_insert_assignment_plan(
            LOCAL_INDEX_ASSIGNMENT_ARENA,
            &change.local_entries[..change.local_count],
        )?;
        if expected_raw_plan != change.raw_plan || expected_local_plan != change.local_plan {
            return Err(SupportLedgerError::Generation);
        }
        let mut census = crate::work::ExactWorkCensus::new();
        let reconstructed = self.prepare_root_batch(change.preview, owner_records, &mut census)?;
        if &reconstructed != change {
            return Err(SupportLedgerError::Generation);
        }
        Ok(())
    }

    pub(in crate::support) fn commit_root_batch_prevalidated(
        &mut self,
        change: PreparedRootBatch,
        apply_index_plans: bool,
    ) {
        for index in 0..change.preview.transition_count {
            let journal = change.journals[index].expect("sealed semantic journal");
            self.formations
                .install_reserved_image_direct(change.formations[index], journal.formation_after);
            self.mutations
                .install_reserved_image_direct(change.mutations[index], journal.mutation_after);
            self.groups
                .replace_image_prevalidated(journal.before.group, journal.group_after);
            if journal.before.locator_kind == 1 {
                self.external_heads
                    .replace_image_prevalidated(journal.before.locator, journal.locator_after);
            } else {
                self.wrappers.install_reserved_image_direct(
                    journal.locator_after_ref,
                    journal.locator_after,
                );
            }
            for ordinal in 0..PLAN_MEMBERS_MAX {
                let funder = change.funders[index * PLAN_MEMBERS_MAX + ordinal];
                self.funders
                    .install_reserved_image_direct(funder, journal.funder_after[ordinal]);
                self.members.replace_image_prevalidated(
                    journal.before.members[ordinal].member,
                    journal.member_after[ordinal],
                );
            }
        }
        if change.link_count > 0 {
            for index in 0..change.preview.owner_count {
                self.links.install_reserved_image_direct(
                    change.links[index],
                    change.new_link_images[index],
                );
            }
        }
        if apply_index_plans {
            if let Some(raw_plan) = change.raw_plan {
                self.raw.commit_assignment_plan_prevalidated(raw_plan);
            }
        }
        for journal in change.resolution_journals[..change.preview.resolution_record_count]
            .iter()
            .copied()
            .flatten()
        {
            self.lifecycle
                .replace_image_prevalidated(journal.snapshot.reference, journal.after);
        }
        for index in 0..change.preview.owner_count {
            let references = change.owner_references[index];
            self.owner_rows
                .replace_image_prevalidated(references[1], change.owner_rows_after[index]);
            self.owners
                .replace_image_prevalidated(references[3], change.owners_after[index]);
            if !matches!(change.preview.resolver, ResolverChange::Keep) {
                self.links.replace_image_prevalidated(
                    change.retired_links[index],
                    change.retired_link_after[index],
                );
            }
        }
        if apply_index_plans {
            self.local
                .commit_assignment_plan_prevalidated(change.local_plan);
        }
        self.assign_semantic_arena_headers(change.arena_headers_after);
        self.header = change.header_after;
    }

    fn preview_root_batch(
        &self,
        operation: SemanticOperation,
        cause: FormationCause,
        transitions: [Option<(RootSnapshot, RootState, u8)>; ROOT_BATCH_MAX],
        resolver: ResolverChange,
        occurred_at: u64,
    ) -> Result<RootBatchPreview, SupportLedgerError> {
        let transition_count = transitions.iter().flatten().count();
        if transition_count == 0
            || transitions[..transition_count].iter().any(Option::is_none)
            || transitions[transition_count..].iter().any(Option::is_some)
        {
            return Err(SupportLedgerError::InvalidInput);
        }
        let mut specs = [None; ROOT_BATCH_MAX];
        let mut aggregate = AggregateDelta::ZERO;
        let mut owners = [OwnerDelta::ZERO; PLAN_MEMBERS_MAX];
        let mut owner_count = 0;
        for index in 0..transition_count {
            let (before, after, reason) = transitions[index].expect("contiguous transition");
            if before.version >= 4 || occurred_at <= before.occurred_at {
                return Err(SupportLedgerError::InvalidTransition);
            }
            let delta = transition_aggregate(before.state, after, before.member_count)?;
            aggregate.add(delta)?;
            if matches!(
                after,
                RootState::ClosedConditional | RootState::ClosedPending
            ) {
                for member in before.members[..before.member_count].iter().copied() {
                    let position = owners[..owner_count]
                        .iter()
                        .position(|owner| owner.owner == member.owner)
                        .unwrap_or_else(|| {
                            let position = owner_count;
                            owners[position].owner = member.owner;
                            owners[position].slot = member.owner.slot;
                            owner_count += 1;
                            position
                        });
                    owners[position].linked = owners[position]
                        .linked
                        .checked_sub(1)
                        .ok_or_else(capacity_error)?;
                    let branch = usize::from(funding_branch(before.branch)?);
                    owners[position].branch[branch] = owners[position].branch[branch]
                        .checked_sub(1)
                        .ok_or_else(capacity_error)?;
                }
            }
            specs[index] = Some(RootTransitionSpec {
                before,
                after,
                cause,
                close_reason: reason,
                close_authority: [0; 32],
                occurred_at,
            });
        }
        if owner_count == 0 {
            let resolver_root = specs[0].expect("one transition").before;
            for member in resolver_root.members[..resolver_root.member_count]
                .iter()
                .copied()
            {
                owners[owner_count] = OwnerDelta {
                    slot: member.owner.slot,
                    owner: member.owner,
                    linked: 0,
                    branch: [0; 4],
                };
                owner_count += 1;
            }
        }
        for transition in specs[..transition_count].iter().flatten() {
            if transition.before.member_count != owner_count
                || transition.before.members[..owner_count]
                    .iter()
                    .zip(owners[..owner_count].iter())
                    .any(|(member, owner)| member.owner != owner.owner)
            {
                return Err(SupportLedgerError::InvalidTransition);
            }
        }
        if let ResolverChange::MoveTo(index) = resolver
            && index >= transition_count
        {
            return Err(SupportLedgerError::InvalidInput);
        }
        Ok(RootBatchPreview {
            expected_c17: self.generation(),
            operation,
            transitions: specs,
            transition_count,
            aggregate,
            owners,
            owner_count,
            resolver,
            plan_event: None,
            resolution_records: [None; RESOLUTION_RECORD_MAX],
            resolution_record_count: 0,
            lifecycle_before: LifecycleAggregate::ZERO,
            lifecycle_after: LifecycleAggregate::ZERO,
            retractions: [LifecyclePublication::ZERO; RESOLUTION_PUBLICATION_MAX],
            retraction_count: 0,
        })
    }

    fn validate_preview(&self, preview: &RootBatchPreview) -> Result<(), SupportLedgerError> {
        if self.generation() != preview.expected_c17
            || preview.transition_count == 0
            || preview.transition_count > ROOT_BATCH_MAX
            || preview.transitions[..preview.transition_count]
                .iter()
                .any(Option::is_none)
            || preview.transitions[preview.transition_count..]
                .iter()
                .any(Option::is_some)
        {
            return Err(SupportLedgerError::Generation);
        }
        match preview.plan_event {
            Some((before, after)) => {
                if !matches!(
                    preview.operation,
                    SemanticOperation::TypedCloseC | SemanticOperation::TypedCloseR
                ) || read_u64(&self.header.0, NEXT_PLAN_CAUSAL_EVENT) != before
                    || before.checked_add(1) != Some(after)
                {
                    return Err(SupportLedgerError::Generation);
                }
            }
            None if matches!(
                preview.operation,
                SemanticOperation::TypedCloseC | SemanticOperation::TypedCloseR
            ) =>
            {
                return Err(SupportLedgerError::InvalidInput);
            }
            None => {}
        }
        let resolution = match preview.operation {
            SemanticOperation::ResolveObservationDescriptions => {
                Some(ObservationResolution::DescriptionsRequired)
            }
            SemanticOperation::ResolveObservationOther => Some(ObservationResolution::Other),
            _ => None,
        };
        if let Some(resolution) = resolution {
            let observation = preview.transitions[0]
                .ok_or(SupportLedgerError::InvalidInput)?
                .before;
            let continuation = preview.transitions[1]
                .ok_or(SupportLedgerError::InvalidInput)?
                .before;
            let (records, count, before, after, retractions, retraction_count) =
                self.inspect_observation_lifecycle(observation, continuation, resolution)?;
            if preview.resolution_records != records
                || preview.resolution_record_count != count
                || preview.lifecycle_before != before
                || preview.lifecycle_after != after
                || preview.retractions != retractions
                || preview.retraction_count != retraction_count
            {
                return Err(SupportLedgerError::Generation);
            }
        } else if preview.resolution_record_count != 0
            || preview.resolution_records.iter().any(Option::is_some)
            || preview.lifecycle_before != LifecycleAggregate::ZERO
            || preview.lifecycle_after != LifecycleAggregate::ZERO
            || preview.retraction_count != 0
            || preview
                .retractions
                .iter()
                .any(|publication| *publication != LifecyclePublication::ZERO)
        {
            return Err(SupportLedgerError::InvalidInput);
        }
        for (index, transition) in preview.transitions[..preview.transition_count]
            .iter()
            .flatten()
            .enumerate()
        {
            if !validate_close_authority_image(preview.operation, index, transition.close_authority)
            {
                return Err(SupportLedgerError::InvalidInput);
            }
            let current = self.root_at_group(
                transition.before.group,
                transition.before.authority_key,
                transition.before.branch,
            )?;
            if current != transition.before {
                return Err(SupportLedgerError::Generation);
            }
        }
        Ok(())
    }

    fn plan_root(
        &self,
        authority_key: [u8; 17],
        identity: [u8; PLAN_IDENTITY_BYTES],
        branch: u8,
    ) -> Result<RootSnapshot, SupportLedgerError> {
        if authority_key[0] != 0x30 || branch >= PLAN_BRANCHES as u8 {
            return Err(SupportLedgerError::InvalidInput);
        }
        let value = self
            .authority
            .find(&authority_key)?
            .ok_or(SupportLedgerError::InvalidTransition)?;
        let first = decode_arena_ref(&value)?;
        let first_image = self.groups.image(first, &[1])?;
        if first_image[8] != 0 || first_image[40..57] != authority_key {
            return Err(noncanonical_error());
        }
        let group = decode_arena_ref(
            &first_image[96 + usize::from(branch) * 8..104 + usize::from(branch) * 8],
        )?;
        let snapshot = self.root_at_group(group, authority_key, branch)?;
        let initial = self.formations.image(snapshot.initial_formation, &[1])?;
        if read_u64(initial, 224) != 1
            || initial[222] != FormationCause::Plan as u8
            || initial[8..8 + PLAN_IDENTITY_BYTES] != identity
        {
            return Err(noncanonical_error());
        }
        Ok(snapshot)
    }

    pub(super) fn root_from_anchor(
        &self,
        anchor: RootAnchor,
    ) -> Result<RootSnapshot, SupportLedgerError> {
        if anchor.authority_key == [0; 17]
            || anchor.branch > 4
            || anchor.group.generation == 0
            || anchor.root.generation == 0
            || anchor.version == 0
        {
            return Err(SupportLedgerError::InvalidInput);
        }
        let snapshot = self.root_at_group(anchor.group, anchor.authority_key, anchor.branch)?;
        if anchor.root != anchor.group || snapshot.version != anchor.version {
            return Err(SupportLedgerError::Generation);
        }
        Ok(snapshot)
    }

    pub(super) fn root_at_group(
        &self,
        group: ArenaRef,
        authority_key: [u8; 17],
        branch: u8,
    ) -> Result<RootSnapshot, SupportLedgerError> {
        let group_image = *self.groups.image(group, &[1])?;
        let state = decode_root_state(group_image[9])?;
        let member_count = usize::from(group_image[10]);
        let locator_kind = group_image[11];
        if group_image[8] != branch
            || !(1..=PLAN_MEMBERS_MAX).contains(&member_count)
            || !matches!(locator_kind, 1 | 2)
            || group_image[12..16].iter().any(|byte| *byte != 0)
            || group_image[40..57] != authority_key
            || group_image[57..64].iter().any(|byte| *byte != 0)
        {
            return Err(noncanonical_error());
        }
        let formation = decode_arena_ref(&group_image[16..24])?;
        let locator = decode_arena_ref(&group_image[24..32])?;
        let version = read_u64(&group_image, 32);
        if version == 0 || version > 4 {
            return Err(noncanonical_error());
        }
        let formation_image = *self.formations.image(formation, &[1])?;
        if formation_image[220] != branch
            || decode_root_state(formation_image[221])? != state
            || read_u64(&formation_image, 224) != version
            || decode_arena_ref(&formation_image[240..248])? != group
            || decode_arena_ref(&formation_image[248..256])? != locator
        {
            return Err(noncanonical_error());
        }
        let occurred_at = read_u64(&formation_image, 232);
        if occurred_at == 0 {
            return Err(noncanonical_error());
        }
        let initial_formation = if version == 1 {
            formation
        } else {
            decode_arena_ref(&formation_image[8..16])?
        };
        let locator_image = if locator_kind == 1 {
            *self.external_heads.image(locator, &[1])?
        } else {
            *self.wrappers.image(locator, &[1])?
        };
        let locator_version = if locator_kind == 1 { 120 } else { 56 };
        if locator_image[8] != branch
            || decode_root_state(locator_image[9])? != state
            || decode_arena_ref(&locator_image[16..24])? != group
            || decode_arena_ref(&locator_image[24..32])? != formation
            || read_u64(&locator_image, locator_version) != version
        {
            return Err(noncanonical_error());
        }
        let mut members = [RootMemberSnapshot::ZERO; PLAN_MEMBERS_MAX];
        for ordinal in 0..PLAN_MEMBERS_MAX {
            let member = decode_arena_ref(&group_image[64 + ordinal * 8..72 + ordinal * 8])?;
            let member_image = self.members.image(member, &[1])?;
            let active = ordinal < member_count;
            if member_image[8] != u8::from(active)
                || member_image[9] != branch
                || usize::from(member_image[10]) != ordinal
                || member_image[11..16].iter().any(|byte| *byte != 0)
                || decode_arena_ref(&member_image[16..24])? != group
            {
                return Err(noncanonical_error());
            }
            let funder = decode_arena_ref(&member_image[24..32])?;
            let funder_image = self.funders.image(funder, &[1])?;
            if funder_image[8] != u8::from(active)
                || funder_image[9] != branch
                || u64::from(funder_image[10]) != version
                || usize::from(funder_image[11]) != ordinal
                || decode_arena_ref(&funder_image[16..24])? != group
                || decode_arena_ref(&funder_image[24..32])? != formation
                || decode_arena_ref(&funder_image[32..40])? != member
            {
                return Err(noncanonical_error());
            }
            let mut request_key = [0; 40];
            request_key.copy_from_slice(&member_image[32..72]);
            let owner = if active {
                decode_arena_ref(&member_image[72..80])?
            } else {
                ArenaRef::default()
            };
            let mut entitlement = [0; 32];
            entitlement.copy_from_slice(&member_image[80..112]);
            let mut vector = [0; 32];
            vector.copy_from_slice(&funder_image[80..112]);
            if active {
                if request_key == [0; 40]
                    || entitlement == [0; 32]
                    || vector == [0; 32]
                    || decode_arena_ref(&funder_image[40..48])? != owner
                    || funder_image[48..80] != entitlement
                    || read_u64(funder_image, 112) == 0
                    || read_u64(funder_image, 120) == 0
                {
                    return Err(noncanonical_error());
                }
            } else if request_key != [0; 40]
                || owner != ArenaRef::default()
                || entitlement != [0; 32]
                || vector != [0; 32]
                || funder_image[40..].iter().any(|byte| *byte != 0)
            {
                return Err(noncanonical_error());
            }
            members[ordinal] = RootMemberSnapshot {
                member,
                funder,
                owner,
                request_key,
                entitlement,
                vector,
                branch_limit: read_u64(funder_image, 120),
                active,
            };
        }
        Ok(RootSnapshot {
            authority_key,
            branch,
            group,
            formation,
            initial_formation,
            locator,
            locator_kind,
            state,
            version,
            occurred_at,
            member_count,
            members,
            group_image,
            formation_image,
            locator_image,
        })
    }

    pub(crate) fn current_membership_root_anchor(
        &self,
        anchor: crate::request_book::c17::SupportMembershipAnchor,
    ) -> Result<RootAnchor, SupportLedgerError> {
        if anchor.is_absent() || anchor.group() != anchor.root() {
            return Err(SupportLedgerError::InvalidInput);
        }
        let root = self.root_at_group(anchor.group(), anchor.authority_key(), anchor.branch())?;
        Ok(RootAnchor {
            authority_key: root.authority_key,
            branch: root.branch,
            group: root.group,
            root: root.group,
            version: root.version,
        })
    }

    pub(crate) fn bind_lifecycle_record_spec(
        &self,
        anchor: RootAnchor,
        ordinal: usize,
        spec: crate::core::C17LifecycleRecordSpec,
    ) -> Result<LifecycleRecordInput, SupportLedgerError> {
        let root = self.root_from_anchor(anchor)?;
        let class = match root.state {
            RootState::Conditional => 0,
            RootState::Pending => 1,
            RootState::Active => 2,
            _ => return Err(SupportLedgerError::InvalidTransition),
        };
        let pool = spec.pool as usize;
        let horizon = usize::from(spec.horizon);
        if horizon >= 3
            || spec.claim == [0; 32]
            || spec.occurred_at.as_micros() == 0
            || spec
                .expires_at
                .is_some_and(|expiry| expiry <= spec.occurred_at)
            || spec.obligation.get() >= spec.credit.get()
        {
            return Err(SupportLedgerError::InvalidInput);
        }
        let axis = spec.operation as usize * 3 + pool;
        let mut aggregate = [0; 21];
        aggregate[..6].copy_from_slice(&[
            class as u64,
            pool as u64,
            axis as u64,
            horizon as u64,
            root.member_count as u64,
            1,
        ]);
        self.bind_lifecycle_record(
            anchor,
            ordinal,
            LifecycleRecordInput {
                final_owner: [0; 64],
                owner_set_ref: [0; 8],
                obligation_raw: spec.obligation.get(),
                credit_raw: spec.credit.get(),
                predecessor: spec.predecessor.0,
                scope: spec.scope.0,
                claim: spec.claim,
                physical_credit: spec.credit.get(),
                kind: spec.kind as u8 + 1,
                occurred_at: spec.occurred_at.as_micros(),
                expires_at: spec.expires_at.map(|time| time.as_micros()),
                aggregate,
                owners: [LifecycleOwnerRow::ZERO; PLAN_MEMBERS_MAX],
            },
        )
    }

    pub(crate) fn bind_lifecycle_record(
        &self,
        anchor: RootAnchor,
        ordinal: usize,
        mut record: LifecycleRecordInput,
    ) -> Result<LifecycleRecordInput, SupportLedgerError> {
        let root = self.root_from_anchor(anchor)?;
        let reserve = self.reserved_reference(ordinal)?;
        let amount = usize::try_from(record.aggregate[4]).map_err(|_| capacity_error())?;
        if amount != root.member_count || !(1..=PLAN_MEMBERS_MAX).contains(&amount) {
            return Err(SupportLedgerError::InvalidInput);
        }
        record.final_owner = encode_lifecycle_final_owner(root);
        record.owner_set_ref = encode_arena_ref_value(root.members[0].owner);
        record.owners = [LifecycleOwnerRow::ZERO; PLAN_MEMBERS_MAX];
        for index in 0..root.member_count {
            record.owners[index] =
                self.canonical_lifecycle_owner_row(root, root.members[index], reserve, record)?;
        }
        record.validate()?;
        Ok(record)
    }
}
