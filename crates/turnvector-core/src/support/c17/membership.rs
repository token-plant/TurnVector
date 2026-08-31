use super::*;
use crate::SourceRecordRef;
use crate::request_book::c17::SupportMembershipAnchor;

const STANDALONE_BRANCH: u8 = 3;
const STANDALONE_FUNDER_ROWS: usize = PLAN_MEMBERS_MAX;
const STANDALONE_MEMBER_ROWS: usize = PLAN_MEMBERS_MAX;
const STANDALONE_LOCAL_EDITS: usize = 8;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct MembershipFunding {
    pub(crate) request: RequestId,
    pub(crate) request_key: [u8; 40],
    pub(crate) record_slot: u32,
    pub(crate) owner_header: ArenaRef,
    pub(crate) entitlement: [u8; 32],
    pub(crate) vector: [u8; 32],
    pub(crate) branch_limit: u64,
}

impl MembershipFunding {
    fn validate(self) -> Result<(), SupportLedgerError> {
        if self.request_key == [0; 40]
            || self.owner_header.generation == 0
            || self.owner_header.slot != self.record_slot
            || self.entitlement == [0; 32]
            || self.vector == [0; 32]
            || self.branch_limit == 0
            || crate::request_book::c17::request_key(self.request) != self.request_key
        {
            return Err(SupportLedgerError::InvalidInput);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CreateStandaloneInput {
    pub(crate) authority_key: [u8; 17],
    pub(crate) domain: [u8; 16],
    pub(crate) source: SourceRecordRef,
    pub(crate) initial_kind: u8,
    pub(crate) event_id: u64,
    pub(crate) anchor: SupportMembershipAnchor,
    pub(crate) occurred_at: u64,
    pub(crate) obligation: [u8; 32],
    pub(crate) credit: [u8; 32],
    pub(crate) funding: MembershipFunding,
}

impl CreateStandaloneInput {
    fn validate(self) -> Result<(), SupportLedgerError> {
        self.funding.validate()?;
        if self.authority_key[0] != 0x31
            || self.authority_key[1..] != self.domain
            || self.domain == [0; 16]
            || self.source.is_absent()
            || !matches!(self.initial_kind, 1 | 2)
            || self.event_id == 0
            || self.anchor.is_absent()
            || self.anchor.authority_key() != self.authority_key
            || self.anchor.branch() != STANDALONE_BRANCH
            || self.anchor.group() != self.anchor.root()
            || self.anchor.root_version() != 1
            || self.occurred_at == 0
            || self.obligation == [0; 32]
            || self.credit == [0; 32]
            || self.obligation == self.credit
        {
            return Err(SupportLedgerError::InvalidInput);
        }
        Ok(())
    }
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct PreparedCreateStandaloneRoot {
    expected_c17: u64,
    expected_raw: u64,
    expected_authority: u64,
    expected_local: u64,
    expected_arena_headers: [ByteArenaHeaderImage; 10],
    arena_headers_after: [ByteArenaHeaderImage; 10],
    input: CreateStandaloneInput,
    group: ArenaSelection<1>,
    formation: ArenaSelection<1>,
    funders: ArenaSelection<STANDALONE_FUNDER_ROWS>,
    members: ArenaSelection<STANDALONE_MEMBER_ROWS>,
    wrapper: ArenaSelection<1>,
    link: ArenaSelection<1>,
    membership: ArenaSelection<1>,
    mutation: ArenaSelection<1>,
    authority_before: Option<([u8; 8], NodeHandle)>,
    authority_after: [u8; 8],
    raw_before: [[u8; 8]; 2],
    raw_updates: [([u8; 32], NodeHandle, [u8; 8]); 2],
    local_entries: [([u8; 17], [u8; 8]); STANDALONE_LOCAL_EDITS],
    group_image: [u8; GROUP_BYTES],
    formation_image: [u8; FORMATION_BYTES],
    funder_images: [[u8; FUNDER_BYTES]; STANDALONE_FUNDER_ROWS],
    member_images: [[u8; MEMBER_BYTES]; STANDALONE_MEMBER_ROWS],
    wrapper_image: [u8; WRAPPER_BYTES],
    link_image: [u8; LINK_BYTES],
    membership_image: [u8; MEMBERSHIP_BYTES],
    mutation_image: [u8; MUTATION_BYTES],
    owner_references: [ArenaRef; 4],
    owner_record_before: BundleRecord,
    owner_record_after: BundleRecord,
    owner_row_after: [u8; OWNER_ROW_BYTES],
    owner_after: [u8; OWNER_BYTES],
    header_after: C17HeaderImage,
    raw_plan: PatriciaAssignmentPlan<RAW_ASSIGNMENT_MAX>,
    authority_plan: PatriciaAssignmentPlan<AUTHORITY_ASSIGNMENT_MAX>,
    local_plan: PatriciaAssignmentPlan<LOCAL_ASSIGNMENT_MAX>,
}

impl PreparedCreateStandaloneRoot {
    pub(crate) const fn owner_slot(&self) -> u32 {
        self.input.funding.record_slot
    }

    pub(in crate::support) const fn owner_record_before(&self) -> BundleRecord {
        self.owner_record_before
    }

    pub(in crate::support) const fn owner_record_after(&self) -> BundleRecord {
        self.owner_record_after
    }

    pub(crate) const fn anchor(&self) -> SupportMembershipAnchor {
        self.input.anchor
    }

    pub(crate) fn visit_assignments(
        &self,
        visitor: &mut dyn FnMut(AssignmentOrderKey, Assignment),
    ) {
        self.raw_plan.visit_assignments(visitor);
        self.authority_plan.visit_assignments(visitor);
        self.local_plan.visit_assignments(visitor);
    }
}

impl SupportC17 {
    pub(crate) fn preview_create_standalone_anchor(
        &self,
        authority_key: [u8; 17],
    ) -> Result<SupportMembershipAnchor, SupportLedgerError> {
        if authority_key[0] != 0x31 || authority_key[1..] == [0; 16] {
            return Err(SupportLedgerError::InvalidInput);
        }
        let group = self.groups.prepare_reserve::<1>(1)?[0];
        SupportMembershipAnchor::try_new(
            authority_key,
            STANDALONE_BRANCH,
            group.slot,
            group.generation,
            group.slot,
            group.generation,
            1,
        )
        .map_err(|_| SupportLedgerError::InvalidInput)
    }
}
