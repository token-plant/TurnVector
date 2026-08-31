use super::{
    BundleRecord, BundleState, OutstandingCreditCell, SupportLedgerError, SupportLedgerGeneration,
    SupportObligationState,
};
#[cfg(test)]
use crate::WorkMeter;
use crate::c17_generated::SUPPORT_LEDGER_CEILING_BYTES;
use crate::c17_layout::{
    AUTHORITY_CAPACITY, Assignment, C17HeaderImage, CREATE_STANDALONE_BUDGET,
    DESTINATION_ROOT_CAPACITY, EXTERNAL_HEAD_CAPACITY, ExternalHeadImage, FORMATION_CAPACITY,
    FUNDER_CAPACITY, FormationImage, FunderImage, GroupImage, INITIAL_ROOT_CAPACITY,
    INITIAL_WRAPPER_CAPACITY, InitialWrapperImage, LIFECYCLE_ASSIGNMENTS, LIFECYCLE_BATCH_MAX,
    LIFECYCLE_CAPACITY, LIFECYCLE_CHUNK_MAX, LINK_CAPACITY, LOCAL_CAPACITY,
    LifecycleRecordSlotImage, LinkImage, MEMBER_CAPACITY, MEMBERSHIP_CAPACITY,
    MERGE_INITIAL_BUDGET, MUTATION_CAPACITY, MemberImage, MembershipImage, MutationImage,
    ORDINARY_COPIED_BYTES, OwnerHeaderImage, OwnerImage, OwnerIndexImage, OwnerRowImage,
    POST_CREATE_BUDGET, PendingLifecycleHeaderImage, RAW_CAPACITY, ROOT_GROUP_CAPACITY,
    SUPPORT_HISTORIES, WORK_CLOSE, WORK_CREATE_STANDALONE, WORK_JOIN_REBIND, WORK_MERGE,
    WORK_MERGE_INITIAL, WORK_NEWLY_ELIGIBLE, WORK_PLAN_CREATE, WORK_PLAN_DISPOSITION,
    WORK_REMOVE_BOUND, WORK_REMOVE_ELIGIBLE, WORK_RESOLVE_OBSERVATION, WORK_SPLIT,
    WORK_STATE_TRANSITION, WORK_TOMBSTONE,
};
use crate::reusable::{
    ArenaRef, ArenaSelection, AssignmentOrderKey, ByteArenaFreeCellImage, ByteArenaHeaderImage,
    FixedByteArena, NodeHandle, PatriciaAssignmentPlan, PatriciaEdit, ReusablePatricia,
};
use crate::{FixedStorageError, HotPathWorkWitness, RequestId};
use std::mem::size_of;

mod membership;
mod semantic;
mod topology;
pub(crate) use membership::*;
pub(crate) use semantic::*;
pub(crate) use topology::*;

const GROUP_BYTES: usize = size_of::<GroupImage>();
const EXTERNAL_HEAD_BYTES: usize = size_of::<ExternalHeadImage>();
const FORMATION_BYTES: usize = size_of::<FormationImage>();
const FUNDER_BYTES: usize = size_of::<FunderImage>();
const MEMBER_BYTES: usize = size_of::<MemberImage>();
const WRAPPER_BYTES: usize = size_of::<InitialWrapperImage>();
const OWNER_HEADER_BYTES: usize = size_of::<OwnerHeaderImage>();
const OWNER_ROW_BYTES: usize = size_of::<OwnerRowImage>();
const OWNER_INDEX_BYTES: usize = size_of::<OwnerIndexImage>();
const OWNER_BYTES: usize = size_of::<OwnerImage>();
const LINK_BYTES: usize = size_of::<LinkImage>();
const MEMBERSHIP_BYTES: usize = size_of::<MembershipImage>();
const MUTATION_BYTES: usize = size_of::<MutationImage>();
const LIFECYCLE_BYTES: usize = size_of::<LifecycleRecordSlotImage>();
pub(crate) const C17_PHYSICAL_BYTES: u64 = 57_569_620;

const RAW_INDEX_ASSIGNMENT_ARENA: u16 = 1;
const AUTHORITY_INDEX_ASSIGNMENT_ARENA: u16 = 2;
const LOCAL_INDEX_ASSIGNMENT_ARENA: u16 = 3;
const RAW_ASSIGNMENT_MAX: usize = 9 * 16 + 1;
const RAW_GENERATION_ASSIGNMENTS: usize = 1;
const AUTHORITY_ASSIGNMENT_MAX: usize = 9 * 5 + 1;
const LOCAL_ASSIGNMENT_MAX: usize = 9 * 48 + 1;

const C16_RAW_OWNERS: usize = 11;
const LEGACY_RAW_EDIT_MAX: usize = 2;
const RAW_OWNER_SLOT_BITS: u32 = 22;
const RAW_OWNER_SLOT_MASK: u32 = (1 << RAW_OWNER_SLOT_BITS) - 1;
const RAW_OWNER_ORDINAL_SHIFT: u32 = RAW_OWNER_SLOT_BITS;
const RAW_OWNER_STATE_SHIFT: u32 = 26;
const RAW_OWNER_KIND_SHIFT: u32 = 28;
const OWNER_STATE_LIVE: u8 = 1;
const OWNER_STATE_TOMBSTONE: u8 = 2;
const OWNER_HEADER_RECORD: usize = 12;
const OWNER_HEADER_GENERATION: usize = 16;
const OWNER_HEADER_REQUEST: usize = 24;
const OWNER_HEADER_ENTITLEMENT: usize = 64;
const OWNER_HEADER_VECTOR: usize = 96;
const OWNER_ROW_VECTOR_LEN: usize = 10;
const OWNER_ROW_LINKED_CLAIMS: usize = 12;
const OWNER_ROW_CURRENT: usize = 16;
const OWNER_ROW_ACTIVE_LINK: usize = 24;
const OWNER_ROW_SOURCE: usize = 32;
const OWNER_ROW_RECORD: usize = 40;
const OWNER_ROW_GENERATION: usize = 48;
const OWNER_ROW_BRANCH_CURRENT: usize = 64;
const OWNER_INDEX_STATE: usize = 58;
const OWNER_IMAGE_STATE: usize = 8;
const OWNER_IMAGE_RECORD: usize = 12;
const OWNER_IMAGE_VECTOR_HEAD: usize = 16;
const OWNER_IMAGE_VECTOR_LEN: usize = 20;
const OWNER_IMAGE_LINKED_CLAIMS: usize = 24;

const PENDING_STATE: usize = 0;
const PENDING_BATCH: usize = 8;
const PENDING_TOTAL: usize = 16;
const PENDING_STAGED: usize = 18;
const PENDING_CURSOR: usize = 20;
const PENDING_RESERVED: usize = 22;
const PENDING_SLOTS: usize = 24;
const PENDING_AGGREGATE: usize = 2_072;
const PENDING_BEFORE_SUPPORT: usize = 2_744;
const PENDING_EXPECTED_RAW: usize = 2_752;
const PENDING_WITHHELD: usize = 2_760;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
enum PendingState {
    Empty = 0,
    Staging = 1,
    Aborting = 2,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SupportC17Capacities {
    raw: usize,
    authority: usize,
    local: usize,
    groups: usize,
    external_heads: usize,
    formations: usize,
    funders: usize,
    members: usize,
    wrappers: usize,
    owner_headers: usize,
    owner_rows: usize,
    owner_indices: usize,
    owners: usize,
    links: usize,
    memberships: usize,
    mutations: usize,
    lifecycle: usize,
}

impl SupportC17Capacities {
    pub(crate) const fn production() -> Self {
        Self {
            raw: RAW_CAPACITY,
            authority: AUTHORITY_CAPACITY,
            local: LOCAL_CAPACITY,
            groups: ROOT_GROUP_CAPACITY,
            external_heads: EXTERNAL_HEAD_CAPACITY,
            formations: FORMATION_CAPACITY,
            funders: FUNDER_CAPACITY,
            members: MEMBER_CAPACITY,
            wrappers: INITIAL_WRAPPER_CAPACITY,
            owner_headers: SUPPORT_HISTORIES,
            owner_rows: SUPPORT_HISTORIES,
            owner_indices: SUPPORT_HISTORIES,
            owners: SUPPORT_HISTORIES,
            links: LINK_CAPACITY,
            memberships: MEMBERSHIP_CAPACITY,
            mutations: MUTATION_CAPACITY,
            lifecycle: LIFECYCLE_CAPACITY,
        }
    }

    #[cfg(test)]
    pub(crate) const fn testing() -> Self {
        Self {
            raw: 128,
            authority: 8,
            local: 320,
            groups: 20,
            external_heads: 8,
            formations: 64,
            funders: 256,
            members: 80,
            wrappers: 40,
            owner_headers: 16,
            owner_rows: 16,
            owner_indices: 16,
            owners: 16,
            links: 32,
            memberships: 32,
            mutations: 128,
            lifecycle: 16,
        }
    }

    #[cfg(test)]
    pub(crate) const fn lifecycle_testing(capacity: usize) -> Self {
        Self {
            lifecycle: capacity,
            raw: capacity * 2 + 128,
            mutations: 128,
            ..Self::testing()
        }
    }

    fn valid(self) -> bool {
        let maxima = Self::production();
        [
            (self.raw, maxima.raw),
            (self.authority, maxima.authority),
            (self.local, maxima.local),
            (self.groups, maxima.groups),
            (self.external_heads, maxima.external_heads),
            (self.formations, maxima.formations),
            (self.funders, maxima.funders),
            (self.members, maxima.members),
            (self.wrappers, maxima.wrappers),
            (self.owner_headers, maxima.owner_headers),
            (self.owner_rows, maxima.owner_rows),
            (self.owner_indices, maxima.owner_indices),
            (self.owners, maxima.owners),
            (self.links, maxima.links),
            (self.memberships, maxima.memberships),
            (self.mutations, maxima.mutations),
            (self.lifecycle, maxima.lifecycle),
        ]
        .into_iter()
        .all(|(actual, maximum)| actual > 0 && actual <= maximum)
    }
}

#[derive(Debug, Eq, PartialEq)]
#[repr(C)]
pub(crate) struct SupportC17 {
    header: C17HeaderImage,
    raw: ReusablePatricia<32, 8>,
    authority: ReusablePatricia<17, 8>,
    local: ReusablePatricia<17, 8>,
    groups: FixedByteArena<GROUP_BYTES>,
    external_heads: FixedByteArena<EXTERNAL_HEAD_BYTES>,
    formations: FixedByteArena<FORMATION_BYTES>,
    funders: FixedByteArena<FUNDER_BYTES>,
    members: FixedByteArena<MEMBER_BYTES>,
    wrappers: FixedByteArena<WRAPPER_BYTES>,
    owner_headers: FixedByteArena<OWNER_HEADER_BYTES>,
    owner_rows: FixedByteArena<OWNER_ROW_BYTES>,
    owner_indices: FixedByteArena<OWNER_INDEX_BYTES>,
    owners: FixedByteArena<OWNER_BYTES>,
    links: FixedByteArena<LINK_BYTES>,
    memberships: FixedByteArena<MEMBERSHIP_BYTES>,
    mutations: FixedByteArena<MUTATION_BYTES>,
    lifecycle: FixedByteArena<LIFECYCLE_BYTES>,
    pending: PendingLifecycleHeaderImage,
}

impl SupportC17 {
    pub(crate) fn try_new(capacities: SupportC17Capacities) -> Result<Self, SupportLedgerError> {
        if !capacities.valid() {
            return Err(capacity_error());
        }
        Self::physical_bytes(capacities).ok_or_else(capacity_error)?;
        let mut header = C17HeaderImage::ZERO;
        for offset in [48, 56, 64, 72, 80] {
            write_u64(&mut header.0, offset, 1);
        }
        for (offset, value) in [
            (104, INITIAL_ROOT_CAPACITY),
            (108, DESTINATION_ROOT_CAPACITY),
            (112, MUTATION_CAPACITY),
            (116, LIFECYCLE_CAPACITY),
        ] {
            write_u32(
                &mut header.0,
                offset,
                u32::try_from(value).map_err(|_| capacity_error())?,
            );
        }
        Ok(Self {
            header,
            raw: ReusablePatricia::try_new(capacities.raw)?,
            authority: ReusablePatricia::try_new(capacities.authority)?,
            local: ReusablePatricia::try_new(capacities.local)?,
            groups: FixedByteArena::try_new(capacities.groups)?,
            external_heads: FixedByteArena::try_new(capacities.external_heads)?,
            formations: FixedByteArena::try_new(capacities.formations)?,
            funders: FixedByteArena::try_new(capacities.funders)?,
            members: FixedByteArena::try_new(capacities.members)?,
            wrappers: FixedByteArena::try_new(capacities.wrappers)?,
            owner_headers: FixedByteArena::try_new(capacities.owner_headers)?,
            owner_rows: FixedByteArena::try_new(capacities.owner_rows)?,
            owner_indices: FixedByteArena::try_new(capacities.owner_indices)?,
            owners: FixedByteArena::try_new(capacities.owners)?,
            links: FixedByteArena::try_new(capacities.links)?,
            memberships: FixedByteArena::try_new(capacities.memberships)?,
            mutations: FixedByteArena::try_new(capacities.mutations)?,
            lifecycle: FixedByteArena::try_new(capacities.lifecycle)?,
            pending: PendingLifecycleHeaderImage::ZERO,
        })
    }

    pub(crate) fn physical_bytes(capacities: SupportC17Capacities) -> Option<u64> {
        if !capacities.valid() {
            return None;
        }
        checked_sum([
            size_of::<C17HeaderImage>() as u64,
            ReusablePatricia::<32, 8>::storage_bytes(capacities.raw)?,
            ReusablePatricia::<17, 8>::storage_bytes(capacities.authority)?,
            ReusablePatricia::<17, 8>::storage_bytes(capacities.local)?,
            FixedByteArena::<GROUP_BYTES>::storage_bytes(capacities.groups)?,
            FixedByteArena::<EXTERNAL_HEAD_BYTES>::storage_bytes(capacities.external_heads)?,
            FixedByteArena::<FORMATION_BYTES>::storage_bytes(capacities.formations)?,
            FixedByteArena::<FUNDER_BYTES>::storage_bytes(capacities.funders)?,
            FixedByteArena::<MEMBER_BYTES>::storage_bytes(capacities.members)?,
            FixedByteArena::<WRAPPER_BYTES>::storage_bytes(capacities.wrappers)?,
            FixedByteArena::<OWNER_HEADER_BYTES>::storage_bytes(capacities.owner_headers)?,
            FixedByteArena::<OWNER_ROW_BYTES>::storage_bytes(capacities.owner_rows)?,
            FixedByteArena::<OWNER_INDEX_BYTES>::storage_bytes(capacities.owner_indices)?,
            FixedByteArena::<OWNER_BYTES>::storage_bytes(capacities.owners)?,
            FixedByteArena::<LINK_BYTES>::storage_bytes(capacities.links)?,
            FixedByteArena::<MEMBERSHIP_BYTES>::storage_bytes(capacities.memberships)?,
            FixedByteArena::<MUTATION_BYTES>::storage_bytes(capacities.mutations)?,
            FixedByteArena::<LIFECYCLE_BYTES>::storage_bytes(capacities.lifecycle)?,
            size_of::<PendingLifecycleHeaderImage>() as u64,
        ])
    }

    pub(crate) const fn generation(&self) -> u64 {
        read_u64_const(&self.header.0, 48)
    }

    pub(crate) fn commit_assignment_direct(&mut self, assignment: &Assignment) {
        match assignment.destination_arena {
            RAW_INDEX_ASSIGNMENT_ARENA => self.raw.commit_assignment_direct(assignment),
            AUTHORITY_INDEX_ASSIGNMENT_ARENA => self.authority.commit_assignment_direct(assignment),
            LOCAL_INDEX_ASSIGNMENT_ARENA => self.local.commit_assignment_direct(assignment),
            _ => unreachable!("validated Support assignment arena"),
        }
    }

    #[cfg(test)]
    pub(super) fn current_counts_for_test(&self) -> [usize; 18] {
        [
            self.raw.len(),
            self.authority.len(),
            self.local.len(),
            self.groups.occupied(),
            self.external_heads.occupied(),
            self.formations.occupied(),
            self.funders.occupied(),
            self.members.occupied(),
            self.wrappers.occupied(),
            self.owner_headers.occupied(),
            self.owner_rows.occupied(),
            self.owner_indices.occupied(),
            self.owners.occupied(),
            self.links.occupied(),
            self.memberships.occupied(),
            self.mutations.occupied(),
            self.lifecycle.occupied(),
            self.lifecycle.inactive_count(),
        ]
    }

    #[cfg(test)]
    pub(super) const fn raw_generation_for_test(&self) -> u64 {
        self.raw.generation()
    }

    #[cfg(test)]
    pub(super) const fn pending_header_for_test(&self) -> PendingLifecycleHeaderImage {
        self.pending
    }

    #[cfg(test)]
    pub(super) fn retained_budgets_for_test(&self) -> [u32; 3] {
        [
            read_u32(&self.header.0, 88),
            read_u32(&self.header.0, 92),
            read_u32(&self.header.0, 96),
        ]
    }

    #[cfg(test)]
    pub(super) fn set_retained_budget_for_test(
        &mut self,
        operation: SemanticOperation,
        value: u32,
    ) {
        let (offset, maximum) = operation
            .retained_budget()
            .expect("operation has a retained budget");
        assert!(value <= maximum, "test retained budget is in range");
        write_u32(&mut self.header.0, offset, value);
    }

    #[cfg(test)]
    pub(super) fn inactive_lifecycle_image_for_test(
        &self,
        ordinal: usize,
    ) -> LifecycleRecordSlotImage {
        let reference = self
            .inactive_reference(ordinal)
            .expect("test lifecycle ordinal is inactive");
        LifecycleRecordSlotImage(
            *self
                .lifecycle
                .image(reference, &[3])
                .expect("test lifecycle record exists"),
        )
    }
}
