use super::semantic::{
    AggregateDelta, RootMemberSnapshot, RootSnapshot, apply_i32_u32, apply_i32_u64,
    request_id_from_key_for_support, transition_aggregate,
};
use super::*;
use crate::request_book::c17::{
    MembershipDestination, MembershipEventKind, MembershipEventRecord, MembershipTag,
    PreparedCancellation, PreparedMembershipIntent, SupportMembershipAnchor,
};
use crate::work::WorkRecorder;

const SOURCE_MAX: usize = 3;
const FORMATION_MAX: usize = SOURCE_MAX + 1;
const FUNDER_MAX: usize = FORMATION_MAX * PLAN_MEMBERS_MAX;
const MEMBER_MAX: usize = PLAN_MEMBERS_MAX;
const WRAPPER_MAX: usize = SOURCE_MAX + 1;
const LINK_MAX: usize = PLAN_MEMBERS_MAX;
const MUTATION_MAX: usize = SOURCE_MAX + 1;
const LOCAL_MAX: usize = 32;
const STANDALONE_BRANCH: u8 = 3;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TopologyOwner {
    slot: u32,
    owner: ArenaRef,
    request_key: [u8; 40],
    source: usize,
    branch_delta: [i32; 4],
    vector_delta: [i32; 4],
    linked_delta: i32,
}

impl TopologyOwner {
    const ZERO: Self = Self {
        slot: 0,
        owner: ArenaRef {
            slot: 0,
            generation: 0,
        },
        request_key: [0; 40],
        source: 0,
        branch_delta: [0; 4],
        vector_delta: [0; 4],
        linked_delta: 0,
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct MergeInitialPreview {
    expected_c17: u64,
    sources: [Option<RootSnapshot>; SOURCE_MAX],
    source_count: usize,
    destination: SupportMembershipAnchor,
    aggregate: AggregateDelta,
    owners: [TopologyOwner; PLAN_MEMBERS_MAX],
    owner_count: usize,
    occurred_at: u64,
}

impl MergeInitialPreview {
    pub(crate) const fn destination(&self) -> SupportMembershipAnchor {
        self.destination
    }

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
        (index < self.owner_count).then_some(self.owners[index].vector_delta)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SourceJournal {
    before: RootSnapshot,
    locator_after_ref: ArenaRef,
    group_after: [u8; GROUP_BYTES],
    locator_after: [u8; WRAPPER_BYTES],
    formation_after: [u8; FORMATION_BYTES],
    funder_after: [[u8; FUNDER_BYTES]; PLAN_MEMBERS_MAX],
    member_after: [[u8; MEMBER_BYTES]; PLAN_MEMBERS_MAX],
    mutation_after: [u8; MUTATION_BYTES],
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct PreparedMergeInitialTopology {
    expected_c17: u64,
    expected_authority: u64,
    expected_local: u64,
    expected_arena_headers: [ByteArenaHeaderImage; 11],
    arena_headers_after: [ByteArenaHeaderImage; 11],
    preview: MergeInitialPreview,
    event: MembershipEventRecord,
    groups: ArenaSelection<1>,
    formations: ArenaSelection<FORMATION_MAX>,
    funders: ArenaSelection<FUNDER_MAX>,
    members: ArenaSelection<MEMBER_MAX>,
    wrappers: ArenaSelection<WRAPPER_MAX>,
    links: ArenaSelection<LINK_MAX>,
    memberships: ArenaSelection<1>,
    mutations: ArenaSelection<MUTATION_MAX>,
    source_journals: [Option<SourceJournal>; SOURCE_MAX],
    destination_group: [u8; GROUP_BYTES],
    destination_formation: [u8; FORMATION_BYTES],
    destination_funders: [[u8; FUNDER_BYTES]; PLAN_MEMBERS_MAX],
    destination_members: [[u8; MEMBER_BYTES]; PLAN_MEMBERS_MAX],
    destination_wrapper: [u8; WRAPPER_BYTES],
    membership_image: [u8; MEMBERSHIP_BYTES],
    membership_mutation: [u8; MUTATION_BYTES],
    local_entries: [([u8; 17], [u8; 8]); LOCAL_MAX],
    local_count: usize,
    authority_key: [u8; 17],
    authority_before: [u8; 8],
    authority_handle: NodeHandle,
    authority_after: [u8; 8],
    owner_records_before: [Option<BundleRecord>; PLAN_MEMBERS_MAX],
    owner_records_after: [Option<BundleRecord>; PLAN_MEMBERS_MAX],
    owner_references: [[ArenaRef; 4]; PLAN_MEMBERS_MAX],
    owner_rows_after: [[u8; OWNER_ROW_BYTES]; PLAN_MEMBERS_MAX],
    owners_after: [[u8; OWNER_BYTES]; PLAN_MEMBERS_MAX],
    retired_links: [ArenaRef; PLAN_MEMBERS_MAX],
    retired_link_before: [[u8; LINK_BYTES]; PLAN_MEMBERS_MAX],
    retired_link_after: [[u8; LINK_BYTES]; PLAN_MEMBERS_MAX],
    replacement_links: [[u8; LINK_BYTES]; PLAN_MEMBERS_MAX],
    header_after: C17HeaderImage,
    authority_plan: PatriciaAssignmentPlan<AUTHORITY_ASSIGNMENT_MAX>,
    local_plan: PatriciaAssignmentPlan<LOCAL_ASSIGNMENT_MAX>,
}

impl PreparedMergeInitialTopology {
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

    pub(crate) fn visit_assignments(
        &self,
        visitor: &mut dyn FnMut(AssignmentOrderKey, Assignment),
    ) {
        self.authority_plan.visit_assignments(visitor);
        self.local_plan.visit_assignments(visitor);
    }
}

impl SupportC17 {
    pub(crate) fn inspect_merge_initial(
        &self,
        anchors: [SupportMembershipAnchor; SOURCE_MAX],
        source_count: u8,
        domain: [u8; 16],
        occurred_at: u64,
    ) -> Result<MergeInitialPreview, SupportLedgerError> {
        let count = usize::from(source_count);
        if !(2..=SOURCE_MAX).contains(&count)
            || domain == [0; 16]
            || occurred_at == 0
            || anchors[..count].iter().any(|anchor| anchor.is_absent())
            || anchors[count..].iter().any(|anchor| !anchor.is_absent())
        {
            return Err(SupportLedgerError::InvalidInput);
        }
        let mut authority_key = [0; 17];
        authority_key[0] = 0x31;
        authority_key[1..].copy_from_slice(&domain);
        let mut roots: [Option<RootSnapshot>; SOURCE_MAX] = [None; SOURCE_MAX];
        for index in 0..count {
            let anchor = anchors[index];
            if anchor.authority_key() != authority_key
                || anchor.branch() != STANDALONE_BRANCH
                || anchor.group() != anchor.root()
                || anchors[..index].contains(&anchor)
            {
                return Err(SupportLedgerError::InvalidTransition);
            }
            let root = self.root_at_group(anchor.group(), authority_key, STANDALONE_BRANCH)?;
            if root.state != RootState::Pending
                || root.member_count != 1
                || root.locator_kind != 2
                || occurred_at <= root.occurred_at
                || roots[..index]
                    .iter()
                    .flatten()
                    .any(|prior| prior.group == root.group)
            {
                return Err(SupportLedgerError::InvalidTransition);
            }
            roots[index] = Some(root);
        }
        roots[..count].sort_unstable_by_key(|root| {
            root.expect("active MergeInitial source").members[0].request_key
        });
        let mut aggregate = AggregateDelta::ZERO;
        let mut owners = [TopologyOwner::ZERO; PLAN_MEMBERS_MAX];
        for index in 0..count {
            let root = roots[index].expect("active MergeInitial source");
            let member = root.members[0];
            if index > 0 {
                let previous = owners[index - 1];
                if previous.request_key >= member.request_key
                    || previous.owner == member.owner
                    || roots[..index].iter().flatten().any(|prior| {
                        let prior = prior.members[0];
                        prior.entitlement == member.entitlement || prior.vector == member.vector
                    })
                {
                    return Err(SupportLedgerError::InvalidInput);
                }
            }
            aggregate.add(transition_aggregate(
                RootState::Pending,
                RootState::ClosedPending,
                1,
            )?)?;
            owners[index] = TopologyOwner {
                slot: member.owner.slot,
                owner: member.owner,
                request_key: member.request_key,
                source: index,
                branch_delta: [0; 4],
                vector_delta: [0; 4],
                linked_delta: 0,
            };
        }
        aggregate.add(materialize_pending_delta(count)?)?;
        let destination_group = self.groups.prepare_reserve::<1>(1)?[0];
        let destination = SupportMembershipAnchor::try_new(
            authority_key,
            STANDALONE_BRANCH,
            destination_group.slot,
            destination_group.generation,
            destination_group.slot,
            destination_group.generation,
            1,
        )
        .map_err(|_| SupportLedgerError::InvalidTransition)?;
        Ok(MergeInitialPreview {
            expected_c17: self.generation(),
            sources: roots,
            source_count: count,
            destination,
            aggregate,
            owners,
            owner_count: count,
            occurred_at,
        })
    }
}
