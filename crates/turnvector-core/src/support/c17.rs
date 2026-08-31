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

    #[cfg(test)]
    pub(super) fn corrupt_inactive_lifecycle_record_for_test(&mut self, ordinal: usize) {
        let reference = self
            .inactive_reference(ordinal)
            .expect("test lifecycle ordinal is inactive");
        let mut image = *self
            .lifecycle
            .image(reference, &[3])
            .expect("test lifecycle record exists");
        image[1_024] = 1;
        self.lifecycle.replace_image_prevalidated(reference, image);
    }

    #[cfg(test)]
    pub(super) fn raw_owner_value_for_test(&self, key: [u8; 32]) -> [u8; 8] {
        self.raw
            .find(&key)
            .expect("test Raw index is canonical")
            .expect("test Raw owner exists")
    }

    #[cfg(test)]
    pub(super) fn corrupt_raw_owner_pointer_for_test(&mut self, key: [u8; 32]) {
        let handle = self
            .raw
            .find_handle(&key)
            .expect("test Raw index is canonical")
            .expect("test Raw owner exists");
        let mut value = self
            .raw
            .find(&key)
            .expect("test Raw index is canonical")
            .expect("test Raw owner exists");
        value[0] ^= 1;
        self.raw.replace_value_direct(handle, value);
    }

    pub(super) fn attached(&self, class: usize, pool: usize) -> Result<u32, SupportLedgerError> {
        if class >= 4 || pool >= 3 {
            return Err(noncanonical_error());
        }
        Ok(read_u32(&self.header.0, (class * 3 + pool) * 4))
    }

    pub(super) fn pending_lifecycle_aggregate(
        &self,
    ) -> Result<Option<LifecycleAggregate>, SupportLedgerError> {
        match self.pending_state()? {
            PendingState::Empty => Ok(None),
            PendingState::Staging | PendingState::Aborting => Ok(Some(self.pending_aggregate()?)),
        }
    }

    pub(super) fn validate_attached_change(
        &self,
        delta: [[i32; 3]; 4],
    ) -> Result<[[u32; 3]; 4], SupportLedgerError> {
        let mut after = [[0; 3]; 4];
        for class in 0..4 {
            for pool in 0..3 {
                let before = self.attached(class, pool)?;
                after[class][pool] = if delta[class][pool] >= 0 {
                    before
                        .checked_add(delta[class][pool] as u32)
                        .ok_or_else(capacity_error)?
                } else {
                    before
                        .checked_sub(delta[class][pool].unsigned_abs())
                        .ok_or_else(noncanonical_error)?
                };
            }
        }
        Ok(after)
    }

    pub(super) fn commit_attached_change(&mut self, after: [[u32; 3]; 4]) {
        for (class, row) in after.into_iter().enumerate() {
            for (pool, value) in row.into_iter().enumerate() {
                write_u32(&mut self.header.0, (class * 3 + pool) * 4, value);
            }
        }
    }

    pub(super) fn prepare_legacy_insert(
        &self,
        record_slot: usize,
        obligation: [u8; 32],
        credit: [u8; 32],
    ) -> Result<PreparedLegacyInsert, SupportLedgerError> {
        self.prepare_legacy_insert_batch(std::iter::once((record_slot, obligation, credit)))
    }

    pub(super) fn prepare_legacy_insert_batch(
        &self,
        records: impl IntoIterator<Item = (usize, [u8; 32], [u8; 32])>,
    ) -> Result<PreparedLegacyInsert, SupportLedgerError> {
        let mut entries = [([0; 32], [0; 8]); LEGACY_RAW_EDIT_MAX];
        let mut entry_count = 0usize;
        for (record_slot, obligation, credit) in records {
            if entry_count + 2 > entries.len()
                || obligation == [0; 32]
                || credit == [0; 32]
                || obligation == credit
            {
                return Err(SupportLedgerError::InvalidInput);
            }
            let owner = ArenaRef {
                slot: u32::try_from(record_slot).map_err(|_| capacity_error())?,
                generation: 1,
            };
            entries[entry_count] = (
                obligation,
                encode_raw_owner(
                    RawOwnerKind::LegacyObligation,
                    RawOwnerState::Committed,
                    owner,
                )?,
            );
            entries[entry_count + 1] = (
                credit,
                encode_raw_owner(RawOwnerKind::LegacyCredit, RawOwnerState::Committed, owner)?,
            );
            entry_count += 2;
        }
        if entry_count == 0 {
            return Err(SupportLedgerError::InvalidInput);
        }
        entries[..entry_count].sort_unstable_by_key(|entry| entry.0);
        if entries[..entry_count]
            .windows(2)
            .any(|pair| pair[0].0 >= pair[1].0)
        {
            return Err(SupportLedgerError::InvalidInput);
        }
        self.raw.validate_insert_batch(&entries[..entry_count])?;
        self.generation()
            .checked_add(1)
            .ok_or_else(capacity_error)?;
        Ok(PreparedLegacyInsert {
            expected_c17: self.generation(),
            expected_raw: self.raw.generation(),
            entries,
            entry_count,
        })
    }

    pub(super) fn validate_legacy_insert(
        &self,
        change: &PreparedLegacyInsert,
    ) -> Result<(), SupportLedgerError> {
        if self.generation() != change.expected_c17
            || self.raw.generation() != change.expected_raw
            || !(2..=LEGACY_RAW_EDIT_MAX).contains(&change.entry_count)
            || change.entry_count % 2 != 0
            || change.entries[change.entry_count..]
                .iter()
                .any(|entry| *entry != ([0; 32], [0; 8]))
        {
            return Err(SupportLedgerError::Generation);
        }
        self.raw
            .validate_insert_batch(&change.entries[..change.entry_count])?;
        Ok(())
    }

    pub(super) fn commit_legacy_insert(&mut self, change: PreparedLegacyInsert) {
        self.validate_legacy_insert(&change)
            .expect("validated legacy Raw insertion");
        self.raw
            .insert_batch_prevalidated(&change.entries[..change.entry_count]);
        self.advance_generation();
    }

    pub(super) fn prepare_legacy_update(
        &self,
        record_slot: usize,
        obligation: [u8; 32],
        credit: [u8; 32],
        retained: bool,
    ) -> Result<PreparedLegacyUpdate, SupportLedgerError> {
        self.prepare_legacy_update_batch(std::iter::once((
            record_slot,
            obligation,
            credit,
            retained,
        )))
    }

    pub(super) fn prepare_legacy_update_batch(
        &self,
        records: impl IntoIterator<Item = (usize, [u8; 32], [u8; 32], bool)>,
    ) -> Result<PreparedLegacyUpdate, SupportLedgerError> {
        let mut updates = [([0; 32], NodeHandle::SENTINEL, [0; 8]); LEGACY_RAW_EDIT_MAX];
        let mut update_count = 0usize;
        for (record_slot, obligation, credit, retained) in records {
            if update_count + 2 > updates.len()
                || obligation == [0; 32]
                || credit == [0; 32]
                || obligation == credit
            {
                return Err(SupportLedgerError::InvalidInput);
            }
            let expected_owner = ArenaRef {
                slot: u32::try_from(record_slot).map_err(|_| capacity_error())?,
                generation: 1,
            };
            let next_state = if retained {
                RawOwnerState::Retained
            } else {
                RawOwnerState::Committed
            };
            for (offset, (key, expected_kind)) in [
                (obligation, RawOwnerKind::LegacyObligation),
                (credit, RawOwnerKind::LegacyCredit),
            ]
            .into_iter()
            .enumerate()
            {
                let handle = self
                    .raw
                    .find_handle(&key)?
                    .ok_or(SupportLedgerError::InvalidTransition)?;
                let (kind, state, owner) = decode_raw_owner(self.raw.value_at(handle)?)?;
                if kind != expected_kind
                    || owner != expected_owner
                    || !matches!(state, RawOwnerState::Committed | RawOwnerState::Retained)
                    || state == RawOwnerState::Retained && !retained
                {
                    return Err(noncanonical_error());
                }
                updates[update_count + offset] = (
                    key,
                    handle,
                    encode_raw_owner(expected_kind, next_state, expected_owner)?,
                );
            }
            update_count += 2;
        }
        if update_count == 0 {
            return Err(SupportLedgerError::InvalidInput);
        }
        updates[..update_count].sort_unstable_by_key(|entry| entry.0);
        if updates[..update_count]
            .windows(2)
            .any(|pair| pair[0].0 >= pair[1].0)
        {
            return Err(SupportLedgerError::InvalidInput);
        }
        self.raw.validate_update_batch(&updates[..update_count])?;
        self.generation()
            .checked_add(1)
            .ok_or_else(capacity_error)?;
        Ok(PreparedLegacyUpdate {
            expected_c17: self.generation(),
            expected_raw: self.raw.generation(),
            updates,
            update_count,
        })
    }

    pub(super) fn validate_legacy_update(
        &self,
        change: &PreparedLegacyUpdate,
    ) -> Result<(), SupportLedgerError> {
        if self.generation() != change.expected_c17
            || self.raw.generation() != change.expected_raw
            || !(2..=LEGACY_RAW_EDIT_MAX).contains(&change.update_count)
            || change.update_count % 2 != 0
            || change.updates[change.update_count..]
                .iter()
                .any(|entry| *entry != ([0; 32], NodeHandle::SENTINEL, [0; 8]))
        {
            return Err(SupportLedgerError::Generation);
        }
        self.raw
            .validate_update_batch(&change.updates[..change.update_count])?;
        Ok(())
    }

    pub(super) fn commit_legacy_update(&mut self, change: PreparedLegacyUpdate) {
        self.validate_legacy_update(&change)
            .expect("validated legacy Raw update");
        self.raw
            .update_batch_prevalidated(&change.updates[..change.update_count]);
        self.advance_generation();
    }

    /// Preflights up to the full landed lifecycle bound without allocating or
    /// moving a 1,024-record capability. `record` is an immutable indexed view
    /// over two independently sorted key sequences.
    pub(super) fn prepare_legacy_insert_stream<F>(
        &self,
        record_count: usize,
        record: F,
    ) -> Result<PreparedLegacyInsertStream, SupportLedgerError>
    where
        F: Fn(usize) -> (usize, [u8; 32], [u8; 32]) + Copy,
    {
        if !(1..=LIFECYCLE_BATCH_MAX).contains(&record_count) {
            return Err(SupportLedgerError::InvalidInput);
        }
        let mut prior_obligation = None;
        let mut prior_credit = None;
        for ordinal in 0..record_count {
            let (slot, obligation, credit) = record(ordinal);
            if obligation == [0; 32]
                || credit == [0; 32]
                || obligation == credit
                || prior_obligation.is_some_and(|prior| prior >= obligation)
                || prior_credit.is_some_and(|prior| prior >= credit)
            {
                return Err(SupportLedgerError::InvalidInput);
            }
            let owner = ArenaRef {
                slot: u32::try_from(slot).map_err(|_| capacity_error())?,
                generation: 1,
            };
            encode_raw_owner(
                RawOwnerKind::LegacyObligation,
                RawOwnerState::Committed,
                owner,
            )?;
            encode_raw_owner(RawOwnerKind::LegacyCredit, RawOwnerState::Committed, owner)?;
            prior_obligation = Some(obligation);
            prior_credit = Some(credit);
        }
        let edit_count = record_count.checked_mul(2).ok_or_else(capacity_error)?;
        self.raw
            .validate_insert_stream(legacy_insert_stream(record_count, record), edit_count)?;
        self.generation()
            .checked_add(1)
            .ok_or_else(capacity_error)?;
        Ok(PreparedLegacyInsertStream {
            expected_c17: self.generation(),
            expected_raw: self.raw.generation(),
            record_count,
        })
    }

    pub(super) fn commit_legacy_insert_stream<F>(
        &mut self,
        change: PreparedLegacyInsertStream,
        record: F,
    ) where
        F: Fn(usize) -> (usize, [u8; 32], [u8; 32]) + Copy,
    {
        assert_eq!(self.generation(), change.expected_c17);
        assert_eq!(self.raw.generation(), change.expected_raw);
        let edit_count = change.record_count * 2;
        self.raw.insert_stream_prevalidated(
            legacy_insert_stream(change.record_count, record),
            edit_count,
        );
        self.advance_generation();
    }

    pub(super) fn prepare_legacy_update_stream<F>(
        &self,
        record_count: usize,
        record: F,
    ) -> Result<PreparedLegacyUpdateStream, SupportLedgerError>
    where
        F: Fn(usize) -> (usize, [u8; 32], [u8; 32], bool) + Copy,
    {
        if !(1..=LIFECYCLE_BATCH_MAX).contains(&record_count) {
            return Err(SupportLedgerError::InvalidInput);
        }
        let mut prior_obligation = None;
        let mut prior_credit = None;
        for ordinal in 0..record_count {
            let (slot, obligation, credit, retained) = record(ordinal);
            if obligation == [0; 32]
                || credit == [0; 32]
                || obligation == credit
                || prior_obligation.is_some_and(|prior| prior >= obligation)
                || prior_credit.is_some_and(|prior| prior >= credit)
            {
                return Err(SupportLedgerError::InvalidInput);
            }
            let expected_owner = ArenaRef {
                slot: u32::try_from(slot).map_err(|_| capacity_error())?,
                generation: 1,
            };
            for (key, expected_kind) in [
                (obligation, RawOwnerKind::LegacyObligation),
                (credit, RawOwnerKind::LegacyCredit),
            ] {
                let value = self
                    .raw
                    .find(&key)?
                    .ok_or(SupportLedgerError::InvalidTransition)?;
                let (kind, state, owner) = decode_raw_owner(value)?;
                if kind != expected_kind
                    || owner != expected_owner
                    || !matches!(state, RawOwnerState::Committed | RawOwnerState::Retained)
                    || state == RawOwnerState::Retained && !retained
                {
                    return Err(noncanonical_error());
                }
            }
            prior_obligation = Some(obligation);
            prior_credit = Some(credit);
        }
        let edit_count = record_count.checked_mul(2).ok_or_else(capacity_error)?;
        self.raw
            .validate_update_stream(legacy_update_stream(record_count, record), edit_count)?;
        self.generation()
            .checked_add(1)
            .ok_or_else(capacity_error)?;
        Ok(PreparedLegacyUpdateStream {
            expected_c17: self.generation(),
            expected_raw: self.raw.generation(),
            record_count,
        })
    }

    pub(super) fn commit_legacy_update_stream<F>(
        &mut self,
        change: PreparedLegacyUpdateStream,
        record: F,
    ) where
        F: Fn(usize) -> (usize, [u8; 32], [u8; 32], bool) + Copy,
    {
        assert_eq!(self.generation(), change.expected_c17);
        assert_eq!(self.raw.generation(), change.expected_raw);
        let edit_count = change.record_count * 2;
        self.raw.update_stream_prevalidated(
            legacy_update_stream(change.record_count, record),
            edit_count,
        );
        self.advance_generation();
    }

    pub(super) fn validate_legacy_raw(
        &self,
        record_slot: usize,
        obligation: [u8; 32],
        credit: [u8; 32],
    ) -> Result<(), SupportLedgerError> {
        let owner = ArenaRef {
            slot: u32::try_from(record_slot).map_err(|_| capacity_error())?,
            generation: 1,
        };
        for (key, expected_kind) in [
            (obligation, RawOwnerKind::LegacyObligation),
            (credit, RawOwnerKind::LegacyCredit),
        ] {
            let value = self
                .raw
                .find(&key)?
                .ok_or(SupportLedgerError::InvalidTransition)?;
            let (kind, _, actual_owner) = decode_raw_owner(value)?;
            if kind != expected_kind || actual_owner != owner {
                return Err(noncanonical_error());
            }
        }
        Ok(())
    }

    pub(super) fn c16_owner_header_ref(
        &self,
        record_slot: u32,
        record: &BundleRecord,
    ) -> Result<ArenaRef, SupportLedgerError> {
        let references = [
            self.owner_headers.reference_at(record_slot, &[1])?,
            self.owner_rows.reference_at(record_slot, &[1])?,
            self.owner_indices.reference_at(record_slot, &[1])?,
            self.owners.reference_at(record_slot, &[1])?,
        ];
        validate_c16_owner_set(
            [
                self.owner_headers.image(references[0], &[1])?.as_slice(),
                self.owner_rows.image(references[1], &[1])?.as_slice(),
                self.owner_indices.image(references[2], &[1])?.as_slice(),
                self.owners.image(references[3], &[1])?.as_slice(),
            ],
            references,
            record_slot,
            record,
            OWNER_STATE_LIVE,
        )?;
        Ok(references[0])
    }

    pub(super) fn prepare_c16_bundle(
        &self,
        record_slot: u32,
        record: &BundleRecord,
        cells: &[OutstandingCreditCell],
    ) -> Result<PreparedC16Bundle, SupportLedgerError> {
        if cells.is_empty()
            || cells.len() != record.vector_len as usize
            || u16::try_from(cells.len()).is_err()
        {
            return Err(SupportLedgerError::InvalidInput);
        }
        let owner_headers = self.owner_headers.prepare_reserve::<1>(1)?;
        let owner_rows = self.owner_rows.prepare_reserve::<1>(1)?;
        let owner_indices = self.owner_indices.prepare_reserve::<1>(1)?;
        let owners = self.owners.prepare_reserve::<1>(1)?;
        let references = [owner_headers[0], owner_rows[0], owner_indices[0], owners[0]];
        if references
            .iter()
            .any(|reference| reference.slot != record_slot)
        {
            return Err(noncanonical_error());
        }
        self.generation()
            .checked_add(1)
            .ok_or_else(capacity_error)?;
        read_u64(&self.header.0, 80)
            .checked_add(1)
            .ok_or_else(capacity_error)?;

        let tagged = record.tagged_keys();
        let mut raw_entries = [([0; 32], [0; 8]); C16_RAW_OWNERS];
        for (ordinal, key) in tagged.into_iter().enumerate() {
            raw_entries[ordinal] = (
                key.identity,
                encode_raw_owner_at(
                    c16_raw_kind(ordinal)?,
                    RawOwnerState::Committed,
                    ordinal as u8,
                    owner_headers[0],
                )?,
            );
        }
        raw_entries.sort_unstable_by_key(|entry| entry.0);
        if raw_entries.windows(2).any(|pair| pair[0].0 >= pair[1].0) {
            return Err(SupportLedgerError::InvalidInput);
        }
        self.raw.validate_insert_batch(&raw_entries)?;
        let images =
            encode_c16_owner_set(record_slot, self.generation(), references, record, cells)?;
        Ok(PreparedC16Bundle {
            expected_c17: self.generation(),
            expected_raw: self.raw.generation(),
            expected_owner_arenas: [
                self.owner_headers.generation(),
                self.owner_rows.generation(),
                self.owner_indices.generation(),
                self.owners.generation(),
            ],
            record_slot,
            owner_headers,
            owner_rows,
            owner_indices,
            owners,
            raw_entries,
            images,
        })
    }

    pub(super) fn validate_c16_bundle(
        &self,
        change: &PreparedC16Bundle,
    ) -> Result<(), SupportLedgerError> {
        if self.generation() != change.expected_c17
            || self.raw.generation() != change.expected_raw
            || [
                self.owner_headers.generation(),
                self.owner_rows.generation(),
                self.owner_indices.generation(),
                self.owners.generation(),
            ] != change.expected_owner_arenas
            || self.owner_headers.prepare_reserve::<1>(1)?.as_slice()
                != change.owner_headers.as_slice()
            || self.owner_rows.prepare_reserve::<1>(1)?.as_slice() != change.owner_rows.as_slice()
            || self.owner_indices.prepare_reserve::<1>(1)?.as_slice()
                != change.owner_indices.as_slice()
            || self.owners.prepare_reserve::<1>(1)?.as_slice() != change.owners.as_slice()
            || [
                change.owner_headers[0].slot,
                change.owner_rows[0].slot,
                change.owner_indices[0].slot,
                change.owners[0].slot,
            ] != [change.record_slot; 4]
        {
            return Err(SupportLedgerError::Generation);
        }
        self.raw.validate_insert_batch(&change.raw_entries)?;
        Ok(())
    }

    pub(super) fn commit_c16_bundle(&mut self, change: PreparedC16Bundle) {
        self.validate_c16_bundle(&change)
            .expect("validated C16 unified-owner migration");
        self.owner_headers
            .commit_reserve(&change.owner_headers)
            .expect("validated OwnerHeader selection");
        self.owner_rows
            .commit_reserve(&change.owner_rows)
            .expect("validated OwnerRow selection");
        self.owner_indices
            .commit_reserve(&change.owner_indices)
            .expect("validated OwnerIndex selection");
        self.owners
            .commit_reserve(&change.owners)
            .expect("validated Owner selection");
        self.owner_headers
            .install_reserved(change.owner_headers[0], change.images.header, 1)
            .expect("validated OwnerHeader image");
        self.owner_rows
            .install_reserved(change.owner_rows[0], change.images.row, 1)
            .expect("validated OwnerRow image");
        self.owner_indices
            .install_reserved(change.owner_indices[0], change.images.index, 1)
            .expect("validated OwnerIndex image");
        self.owners
            .install_reserved(change.owners[0], change.images.owner, 1)
            .expect("validated Owner image");
        self.raw.insert_batch_prevalidated(&change.raw_entries);
        self.increment_header_generation(80);
        self.advance_generation();
    }

    pub(super) fn prepare_c16_withdrawal(
        &self,
        record_slot: u32,
        record: &BundleRecord,
    ) -> Result<PreparedC16Withdrawal, SupportLedgerError> {
        let references = [
            self.owner_headers.reference_at(record_slot, &[1])?,
            self.owner_rows.reference_at(record_slot, &[1])?,
            self.owner_indices.reference_at(record_slot, &[1])?,
            self.owners.reference_at(record_slot, &[1])?,
        ];
        let images = [
            self.owner_headers.image(references[0], &[1])?.as_slice(),
            self.owner_rows.image(references[1], &[1])?.as_slice(),
            self.owner_indices.image(references[2], &[1])?.as_slice(),
            self.owners.image(references[3], &[1])?.as_slice(),
        ];
        validate_c16_owner_set(images, references, record_slot, record, OWNER_STATE_LIVE)?;
        validate_withdrawable_owner_row(images[1])?;
        let mut raw_keys = [[0; 32]; C16_RAW_OWNERS];
        for (ordinal, key) in record.tagged_keys().into_iter().enumerate() {
            let value = self
                .raw
                .find(&key.identity)?
                .ok_or_else(noncanonical_error)?;
            let (kind, state, stored_ordinal, owner) = decode_raw_owner_at(value)?;
            if kind != c16_raw_kind(ordinal)?
                || state != RawOwnerState::Committed
                || usize::from(stored_ordinal) != ordinal
                || owner != references[0]
            {
                return Err(noncanonical_error());
            }
            raw_keys[ordinal] = key.identity;
        }
        raw_keys.sort_unstable();
        self.raw.validate_remove_batch(&raw_keys)?;
        self.owner_headers
            .validate_release_batch(&references[..1])?;
        self.owner_rows.validate_release_batch(&references[1..2])?;
        self.owner_indices
            .validate_release_batch(&references[2..3])?;
        self.owners.validate_release_batch(&references[3..4])?;
        self.generation()
            .checked_add(1)
            .ok_or_else(capacity_error)?;
        read_u64(&self.header.0, 80)
            .checked_add(1)
            .ok_or_else(capacity_error)?;
        Ok(PreparedC16Withdrawal {
            expected_c17: self.generation(),
            expected_raw: self.raw.generation(),
            expected_owner_arenas: [
                self.owner_headers.generation(),
                self.owner_rows.generation(),
                self.owner_indices.generation(),
                self.owners.generation(),
            ],
            record_slot,
            references,
            raw_keys,
        })
    }

    pub(super) fn validate_c16_withdrawal(
        &self,
        change: &PreparedC16Withdrawal,
    ) -> Result<(), SupportLedgerError> {
        if self.generation() != change.expected_c17
            || self.raw.generation() != change.expected_raw
            || [
                self.owner_headers.generation(),
                self.owner_rows.generation(),
                self.owner_indices.generation(),
                self.owners.generation(),
            ] != change.expected_owner_arenas
            || change
                .references
                .iter()
                .any(|reference| reference.slot != change.record_slot)
        {
            return Err(SupportLedgerError::Generation);
        }
        self.raw.validate_remove_batch(&change.raw_keys)?;
        self.owner_headers
            .validate_release_batch(&change.references[..1])?;
        self.owner_rows
            .validate_release_batch(&change.references[1..2])?;
        self.owner_indices
            .validate_release_batch(&change.references[2..3])?;
        self.owners
            .validate_release_batch(&change.references[3..4])?;
        Ok(())
    }

    pub(super) fn commit_c16_withdrawal(&mut self, change: PreparedC16Withdrawal) {
        self.validate_c16_withdrawal(&change)
            .expect("validated C16 unified-owner withdrawal");
        self.raw.remove_batch_prevalidated(&change.raw_keys);
        self.owner_headers
            .release_batch_prevalidated(&change.references[..1]);
        self.owner_rows
            .release_batch_prevalidated(&change.references[1..2]);
        self.owner_indices
            .release_batch_prevalidated(&change.references[2..3]);
        self.owners
            .release_batch_prevalidated(&change.references[3..4]);
        self.increment_header_generation(80);
        self.advance_generation();
    }

    pub(super) fn prepare_c16_tombstone(
        &self,
        record_slot: u32,
        record: &BundleRecord,
    ) -> Result<PreparedC16Tombstone, SupportLedgerError> {
        let references = [
            self.owner_headers.reference_at(record_slot, &[1])?,
            self.owner_rows.reference_at(record_slot, &[1])?,
            self.owner_indices.reference_at(record_slot, &[1])?,
            self.owners.reference_at(record_slot, &[1])?,
        ];
        let current = C16OwnerSetImages {
            header: *self.owner_headers.image(references[0], &[1])?,
            row: *self.owner_rows.image(references[1], &[1])?,
            index: *self.owner_indices.image(references[2], &[1])?,
            owner: *self.owners.image(references[3], &[1])?,
        };
        validate_c16_owner_set(
            [
                current.header.as_slice(),
                current.row.as_slice(),
                current.index.as_slice(),
                current.owner.as_slice(),
            ],
            references,
            record_slot,
            record,
            OWNER_STATE_LIVE,
        )?;
        let mut raw_updates = [([0; 32], NodeHandle::SENTINEL, [0; 8]); C16_RAW_OWNERS];
        for (ordinal, key) in record.tagged_keys().into_iter().enumerate() {
            let handle = self
                .raw
                .find_handle(&key.identity)?
                .ok_or(SupportLedgerError::InvalidTransition)?;
            let (kind, state, stored_ordinal, owner) =
                decode_raw_owner_at(self.raw.value_at(handle)?)?;
            if kind != c16_raw_kind(ordinal)?
                || state != RawOwnerState::Committed
                || usize::from(stored_ordinal) != ordinal
                || owner != references[0]
            {
                return Err(noncanonical_error());
            }
            raw_updates[ordinal] = (
                key.identity,
                handle,
                encode_raw_owner_at(
                    kind,
                    RawOwnerState::Tombstone,
                    stored_ordinal,
                    references[0],
                )?,
            );
        }
        raw_updates.sort_unstable_by_key(|entry| entry.0);
        self.raw.validate_update_batch(&raw_updates)?;
        self.owner_headers.validate_advance_generation()?;
        self.owner_rows.validate_advance_generation()?;
        self.owner_indices.validate_advance_generation()?;
        self.owners.validate_advance_generation()?;
        self.generation()
            .checked_add(1)
            .ok_or_else(capacity_error)?;
        read_u64(&self.header.0, 80)
            .checked_add(1)
            .ok_or_else(capacity_error)?;
        Ok(PreparedC16Tombstone {
            expected_c17: self.generation(),
            expected_raw: self.raw.generation(),
            expected_owner_arenas: [
                self.owner_headers.generation(),
                self.owner_rows.generation(),
                self.owner_indices.generation(),
                self.owners.generation(),
            ],
            record_slot,
            references,
            raw_updates,
            images: tombstone_owner_images(current),
        })
    }

    pub(super) fn validate_c16_tombstone(
        &self,
        change: &PreparedC16Tombstone,
        record: &BundleRecord,
    ) -> Result<(), SupportLedgerError> {
        if self.generation() != change.expected_c17
            || self.raw.generation() != change.expected_raw
            || [
                self.owner_headers.generation(),
                self.owner_rows.generation(),
                self.owner_indices.generation(),
                self.owners.generation(),
            ] != change.expected_owner_arenas
            || change
                .references
                .iter()
                .any(|reference| reference.slot != change.record_slot)
        {
            return Err(SupportLedgerError::Generation);
        }
        let current = C16OwnerSetImages {
            header: *self.owner_headers.image(change.references[0], &[1])?,
            row: *self.owner_rows.image(change.references[1], &[1])?,
            index: *self.owner_indices.image(change.references[2], &[1])?,
            owner: *self.owners.image(change.references[3], &[1])?,
        };
        validate_c16_owner_set(
            [
                current.header.as_slice(),
                current.row.as_slice(),
                current.index.as_slice(),
                current.owner.as_slice(),
            ],
            change.references,
            change.record_slot,
            record,
            OWNER_STATE_LIVE,
        )?;
        if tombstone_owner_images(current) != change.images {
            return Err(SupportLedgerError::Generation);
        }
        self.raw.validate_update_batch(&change.raw_updates)?;
        self.owner_headers.validate_advance_generation()?;
        self.owner_rows.validate_advance_generation()?;
        self.owner_indices.validate_advance_generation()?;
        self.owners.validate_advance_generation()?;
        Ok(())
    }

    pub(super) fn commit_c16_tombstone(
        &mut self,
        change: PreparedC16Tombstone,
        record: &BundleRecord,
    ) {
        self.validate_c16_tombstone(&change, record)
            .expect("validated C16 unified-owner tombstone");
        *self
            .owner_headers
            .image_mut(change.references[0], &[1])
            .expect("validated OwnerHeader") = change.images.header;
        *self
            .owner_rows
            .image_mut(change.references[1], &[1])
            .expect("validated OwnerRow") = change.images.row;
        *self
            .owner_indices
            .image_mut(change.references[2], &[1])
            .expect("validated OwnerIndex") = change.images.index;
        *self
            .owners
            .image_mut(change.references[3], &[1])
            .expect("validated Owner") = change.images.owner;
        self.owner_headers.advance_generation_prevalidated();
        self.owner_rows.advance_generation_prevalidated();
        self.owner_indices.advance_generation_prevalidated();
        self.owners.advance_generation_prevalidated();
        self.raw.update_batch_prevalidated(&change.raw_updates);
        self.increment_header_generation(80);
        self.advance_generation();
    }

    pub(super) fn prepare_c16_touch(
        &self,
        record_slot: u32,
        record: &BundleRecord,
    ) -> Result<PreparedC16Touch, SupportLedgerError> {
        let owner = self.owner_headers.reference_at(record_slot, &[1])?;
        let references = [
            owner,
            self.owner_rows.reference_at(record_slot, &[1])?,
            self.owner_indices.reference_at(record_slot, &[1])?,
            self.owners.reference_at(record_slot, &[1])?,
        ];
        validate_c16_owner_set(
            [
                self.owner_headers.image(references[0], &[1])?.as_slice(),
                self.owner_rows.image(references[1], &[1])?.as_slice(),
                self.owner_indices.image(references[2], &[1])?.as_slice(),
                self.owners.image(references[3], &[1])?.as_slice(),
            ],
            references,
            record_slot,
            record,
            OWNER_STATE_LIVE,
        )?;
        let mut raw_updates = [([0; 32], NodeHandle::SENTINEL, [0; 8]); C16_RAW_OWNERS];
        for (ordinal, key) in record.tagged_keys().into_iter().enumerate() {
            let handle = self
                .raw
                .find_handle(&key.identity)?
                .ok_or(SupportLedgerError::InvalidTransition)?;
            let value = self.raw.value_at(handle)?;
            let (kind, state, stored_ordinal, actual_owner) = decode_raw_owner_at(value)?;
            if kind != c16_raw_kind(ordinal)?
                || state != RawOwnerState::Committed
                || usize::from(stored_ordinal) != ordinal
                || actual_owner != owner
            {
                return Err(noncanonical_error());
            }
            raw_updates[ordinal] = (key.identity, handle, value);
        }
        raw_updates.sort_unstable_by_key(|entry| entry.0);
        self.raw.validate_update_batch(&raw_updates)?;
        self.generation()
            .checked_add(1)
            .ok_or_else(capacity_error)?;
        Ok(PreparedC16Touch {
            expected_c17: self.generation(),
            expected_raw: self.raw.generation(),
            record_slot,
            owner,
            raw_updates,
        })
    }

    pub(super) fn validate_c16_touch(
        &self,
        change: &PreparedC16Touch,
    ) -> Result<(), SupportLedgerError> {
        if self.generation() != change.expected_c17
            || self.raw.generation() != change.expected_raw
            || self.owner_headers.reference_at(change.record_slot, &[1])? != change.owner
        {
            return Err(SupportLedgerError::Generation);
        }
        self.raw.validate_update_batch(&change.raw_updates)?;
        Ok(())
    }

    pub(super) fn commit_c16_touch(&mut self, change: PreparedC16Touch) {
        self.validate_c16_touch(&change)
            .expect("validated C16 owner touch");
        self.raw.update_batch_prevalidated(&change.raw_updates);
        self.advance_generation();
    }

    pub(super) fn validate_c16_raw_reciprocity(
        &self,
        raw: [u8; 32],
        record_slot: u32,
        record: &BundleRecord,
    ) -> Result<(), SupportLedgerError> {
        let value = self
            .raw
            .find(&raw)?
            .ok_or(SupportLedgerError::InvalidTransition)?;
        let (kind, state, ordinal, owner) = decode_raw_owner_at(value)?;
        let expected_state = match record.state {
            BundleState::LivePristine | BundleState::LiveConsumed => RawOwnerState::Committed,
            BundleState::RetainedTombstone => RawOwnerState::Tombstone,
        };
        let expected_owner_state = match record.state {
            BundleState::LivePristine | BundleState::LiveConsumed => OWNER_STATE_LIVE,
            BundleState::RetainedTombstone => OWNER_STATE_TOMBSTONE,
        };
        if state != expected_state
            || owner.slot != record_slot
            || owner != self.owner_headers.reference_at(record_slot, &[1])?
            || c16_raw_kind(usize::from(ordinal))? != kind
            || record
                .tagged_key(ordinal)
                .is_none_or(|key| key.identity != raw)
        {
            return Err(noncanonical_error());
        }
        let references = [
            owner,
            self.owner_rows.reference_at(record_slot, &[1])?,
            self.owner_indices.reference_at(record_slot, &[1])?,
            self.owners.reference_at(record_slot, &[1])?,
        ];
        validate_c16_owner_set(
            [
                self.owner_headers.image(references[0], &[1])?.as_slice(),
                self.owner_rows.image(references[1], &[1])?.as_slice(),
                self.owner_indices.image(references[2], &[1])?.as_slice(),
                self.owners.image(references[3], &[1])?.as_slice(),
            ],
            references,
            record_slot,
            record,
            expected_owner_state,
        )
    }

    fn increment_header_generation(&mut self, offset: usize) {
        let next = read_u64(&self.header.0, offset)
            .checked_add(1)
            .expect("prepared C17 owner generation");
        write_u64(&mut self.header.0, offset, next);
    }

    pub(super) fn prepare_plan_create(
        &self,
        input: PlanCreateInput,
        owner_records: [Option<BundleRecord>; PLAN_MEMBERS_MAX],
    ) -> Result<PreparedPlanCreate, SupportLedgerError> {
        input.validate()?;
        let generation_after = self
            .generation()
            .checked_add(1)
            .ok_or_else(capacity_error)?;
        let expected_arena_headers = self.plan_arena_headers();

        if let Some(value) = self.authority.find(&input.authority_key)? {
            let group = decode_arena_ref(&value)?;
            let image = self.groups.image(group, &[1])?;
            let formation = decode_arena_ref(&image[16..24])?;
            let formation_image = self.formations.image(formation, &[1])?;
            if formation_image[8..8 + PLAN_IDENTITY_BYTES] == input.identity {
                return Err(SupportLedgerError::InvalidTransition);
            }
            return Err(noncanonical_error());
        }

        let groups = self
            .groups
            .prepare_reserve::<PLAN_BRANCHES>(PLAN_BRANCHES)?;
        let external_heads = self
            .external_heads
            .prepare_reserve::<PLAN_BRANCHES>(PLAN_BRANCHES)?;
        let formations = self
            .formations
            .prepare_reserve::<PLAN_BRANCHES>(PLAN_BRANCHES)?;
        let funders = self
            .funders
            .prepare_reserve::<PLAN_FUNDER_ROWS>(PLAN_FUNDER_ROWS)?;
        let members = self
            .members
            .prepare_reserve::<PLAN_MEMBER_ROWS>(PLAN_MEMBER_ROWS)?;
        let links = self
            .links
            .prepare_reserve::<PLAN_MEMBERS_MAX>(input.member_count)?;
        let mutations = self.mutations.prepare_reserve::<1>(1)?;

        let mut authority_entries = [([0; 17], [0; 8]); 1];
        authority_entries[0] = (input.authority_key, encode_arena_ref_value(groups[0]));
        self.authority.validate_insert_batch(&authority_entries)?;

        let mut raw_entries = [([0; 32], [0; 8]); PLAN_RAW_EDITS];
        for branch in 0..PLAN_BRANCHES {
            raw_entries[branch * 2] = (
                input.obligations[branch],
                encode_raw_owner_at(
                    RawOwnerKind::PlanRoot,
                    RawOwnerState::Committed,
                    branch as u8,
                    external_heads[branch],
                )?,
            );
            raw_entries[branch * 2 + 1] = (
                input.credits[branch],
                encode_raw_owner_at(
                    RawOwnerKind::Formation,
                    RawOwnerState::Committed,
                    branch as u8,
                    external_heads[branch],
                )?,
            );
        }
        raw_entries.sort_unstable_by_key(|entry| entry.0);
        if raw_entries.windows(2).any(|pair| pair[0].0 >= pair[1].0) {
            return Err(SupportLedgerError::InvalidInput);
        }
        self.raw.validate_insert_batch(&raw_entries)?;

        let mut local_entries = [([0; 17], [0; 8]); PLAN_LOCAL_EDITS];
        let mut local_count = 0;
        for reference in groups.as_slice() {
            local_entries[local_count] = (
                local_key(LocalKind::Group, *reference),
                encode_arena_ref_value(*reference),
            );
            local_count += 1;
        }
        for reference in funders.as_slice() {
            local_entries[local_count] = (
                local_key(LocalKind::Funder, *reference),
                encode_arena_ref_value(*reference),
            );
            local_count += 1;
        }
        for reference in links.as_slice() {
            local_entries[local_count] = (
                local_key(LocalKind::Link, *reference),
                encode_arena_ref_value(*reference),
            );
            local_count += 1;
        }
        local_entries[local_count] = (
            local_key(LocalKind::Mutation, mutations[0]),
            encode_arena_ref_value(mutations[0]),
        );
        local_count += 1;
        debug_assert_eq!(local_count, 3 + 12 + input.member_count + 1);
        local_entries[..local_count].sort_unstable_by_key(|entry| entry.0);
        self.local
            .validate_insert_batch(&local_entries[..local_count])?;

        let mut group_images = [[0; GROUP_BYTES]; PLAN_BRANCHES];
        let mut head_images = [[0; EXTERNAL_HEAD_BYTES]; PLAN_BRANCHES];
        let mut formation_images = [[0; FORMATION_BYTES]; PLAN_BRANCHES];
        let mut funder_images = [[0; FUNDER_BYTES]; PLAN_FUNDER_ROWS];
        let mut member_images = [[0; MEMBER_BYTES]; PLAN_MEMBER_ROWS];
        let mut link_images = [[0; LINK_BYTES]; PLAN_MEMBERS_MAX];

        for branch in 0..PLAN_BRANCHES {
            let row_start = branch * PLAN_MEMBERS_MAX;
            group_images[branch] = self.groups.prepare_reserved_image_after(
                groups[branch],
                encode_plan_group(
                    branch,
                    input.member_count,
                    input.authority_key,
                    formations[branch],
                    external_heads[branch],
                    members.as_slice()[row_start..row_start + PLAN_MEMBERS_MAX]
                        .try_into()
                        .expect("fixed Plan member range"),
                    groups.as_slice().try_into().expect("three Plan groups"),
                ),
                1,
            )?;
            formation_images[branch] = self.formations.prepare_reserved_image_after(
                formations[branch],
                encode_plan_formation(
                    input.identity,
                    branch,
                    groups[branch],
                    external_heads[branch],
                    input.occurred_at,
                ),
                1,
            )?;
            head_images[branch] = self.external_heads.prepare_reserved_image_after(
                external_heads[branch],
                encode_plan_head(
                    branch,
                    input.member_count,
                    groups[branch],
                    formations[branch],
                    input.obligations[branch],
                    input.credits[branch],
                    input.authority_key,
                ),
                1,
            )?;
            for ordinal in 0..PLAN_MEMBERS_MAX {
                let row = row_start + ordinal;
                let active = ordinal < input.member_count;
                funder_images[row] = self.funders.prepare_reserved_image_after(
                    funders[row],
                    encode_plan_funder(
                        branch,
                        ordinal,
                        active,
                        groups[branch],
                        formations[branch],
                        members[row],
                        input.members[ordinal],
                    ),
                    1,
                )?;
                member_images[row] = self.members.prepare_reserved_image_after(
                    members[row],
                    encode_plan_member(
                        branch,
                        ordinal,
                        active,
                        groups[branch],
                        funders[row],
                        input.members[ordinal],
                    ),
                    1,
                )?;
            }
        }
        for ordinal in 0..input.member_count {
            link_images[ordinal] = self.links.prepare_reserved_image_after(
                links[ordinal],
                encode_plan_link(
                    input.members[ordinal].owner_header,
                    groups[0],
                    formations[0],
                    input.authority_key,
                    self.generation(),
                ),
                1,
            )?;
        }
        let mutation_image = self.mutations.prepare_reserved_image_after(
            mutations[0],
            encode_plan_mutation(
                input.authority_key,
                groups.as_slice().try_into().expect("three Plan groups"),
                formations
                    .as_slice()
                    .try_into()
                    .expect("three Plan formations"),
                self.generation(),
                input.occurred_at,
            ),
            1,
        )?;
        let (owner_references, owner_row_images, owner_images) =
            self.prepare_plan_owner_updates(input, owner_records, links.as_slice())?;
        self.owner_rows.validate_advance_generation()?;
        self.owners.validate_advance_generation()?;
        let arena_headers_after = self.prepare_plan_arena_headers_after(
            &groups,
            &external_heads,
            &formations,
            &funders,
            &members,
            &links,
            &mutations,
        )?;
        let raw_plan = self
            .raw
            .prepare_insert_assignment_plan(RAW_INDEX_ASSIGNMENT_ARENA, &raw_entries)?;
        let authority_plan = self
            .authority
            .prepare_insert_assignment_plan(AUTHORITY_INDEX_ASSIGNMENT_ARENA, &authority_entries)?;
        let local_plan = self.local.prepare_insert_assignment_plan(
            LOCAL_INDEX_ASSIGNMENT_ARENA,
            &local_entries[..local_count],
        )?;
        let mut header_after = self.header;
        write_u64(&mut header_after.0, 48, generation_after);

        Ok(PreparedPlanCreate {
            expected_c17: self.generation(),
            expected_raw: self.raw.generation(),
            expected_authority: self.authority.generation(),
            expected_local: self.local.generation(),
            expected_arena_headers,
            arena_headers_after,
            input,
            groups,
            external_heads,
            formations,
            funders,
            members,
            links,
            mutations,
            authority_entries,
            raw_entries,
            local_entries,
            local_count,
            group_images,
            head_images,
            formation_images,
            funder_images,
            member_images,
            link_images,
            mutation_image,
            owner_references,
            owner_row_images,
            owner_images,
            header_after,
            raw_plan,
            authority_plan,
            local_plan,
        })
    }

    pub(super) fn validate_plan_create(
        &self,
        change: &PreparedPlanCreate,
        owner_records: [Option<BundleRecord>; PLAN_MEMBERS_MAX],
    ) -> Result<(), SupportLedgerError> {
        change.input.validate()?;
        let generation_after = self
            .generation()
            .checked_add(1)
            .ok_or_else(capacity_error)?;
        let expected_arena_headers = self.plan_arena_headers();
        let arena_headers_after = self.prepare_plan_arena_headers_after(
            &change.groups,
            &change.external_heads,
            &change.formations,
            &change.funders,
            &change.members,
            &change.links,
            &change.mutations,
        )?;
        let mut header_after = self.header;
        write_u64(&mut header_after.0, 48, generation_after);
        if self.generation() != change.expected_c17
            || self.raw.generation() != change.expected_raw
            || self.authority.generation() != change.expected_authority
            || self.local.generation() != change.expected_local
            || expected_arena_headers != change.expected_arena_headers
            || arena_headers_after != change.arena_headers_after
            || header_after != change.header_after
            || !self.raw.validates_assignment_plan(&change.raw_plan)
            || !self
                .authority
                .validates_assignment_plan(&change.authority_plan)
            || !self.local.validates_assignment_plan(&change.local_plan)
            || self
                .groups
                .prepare_reserve::<PLAN_BRANCHES>(PLAN_BRANCHES)?
                .as_slice()
                != change.groups.as_slice()
            || self
                .external_heads
                .prepare_reserve::<PLAN_BRANCHES>(PLAN_BRANCHES)?
                .as_slice()
                != change.external_heads.as_slice()
            || self
                .formations
                .prepare_reserve::<PLAN_BRANCHES>(PLAN_BRANCHES)?
                .as_slice()
                != change.formations.as_slice()
            || self
                .funders
                .prepare_reserve::<PLAN_FUNDER_ROWS>(PLAN_FUNDER_ROWS)?
                .as_slice()
                != change.funders.as_slice()
            || self
                .members
                .prepare_reserve::<PLAN_MEMBER_ROWS>(PLAN_MEMBER_ROWS)?
                .as_slice()
                != change.members.as_slice()
            || self
                .links
                .prepare_reserve::<PLAN_MEMBERS_MAX>(change.input.member_count)?
                .as_slice()
                != change.links.as_slice()
            || self.mutations.prepare_reserve::<1>(1)?.as_slice() != change.mutations.as_slice()
            || change.local_count != 3 + 12 + change.input.member_count + 1
            || change.local_entries[change.local_count..]
                .iter()
                .any(|entry| *entry != ([0; 17], [0; 8]))
            || change.link_images[change.input.member_count..]
                .iter()
                .any(|image| *image != [0; LINK_BYTES])
            || !self.validates_plan_slot_images(change)?
        {
            return Err(SupportLedgerError::Generation);
        }
        let (owner_references, owner_row_images, owner_images) =
            self.prepare_plan_owner_updates(change.input, owner_records, change.links.as_slice())?;
        if owner_references != change.owner_references
            || owner_row_images != change.owner_row_images
            || owner_images != change.owner_images
        {
            return Err(SupportLedgerError::Generation);
        }
        self.owner_rows.validate_advance_generation()?;
        self.owners.validate_advance_generation()?;
        if let Some(value) = self.authority.find(&change.input.authority_key)? {
            let group = decode_arena_ref(&value)?;
            let group_image = self.groups.image(group, &[1])?;
            let formation = decode_arena_ref(&group_image[16..24])?;
            let image = self.formations.image(formation, &[1])?;
            return if image[8..8 + PLAN_IDENTITY_BYTES] == change.input.identity {
                Err(SupportLedgerError::InvalidTransition)
            } else {
                Err(noncanonical_error())
            };
        }
        self.authority
            .validate_insert_batch(&change.authority_entries)?;
        self.raw.validate_insert_batch(&change.raw_entries)?;
        self.local
            .validate_insert_batch(&change.local_entries[..change.local_count])?;
        if self
            .raw
            .prepare_insert_assignment_plan(RAW_INDEX_ASSIGNMENT_ARENA, &change.raw_entries)?
            != change.raw_plan
            || self.authority.prepare_insert_assignment_plan(
                AUTHORITY_INDEX_ASSIGNMENT_ARENA,
                &change.authority_entries,
            )? != change.authority_plan
            || self.local.prepare_insert_assignment_plan(
                LOCAL_INDEX_ASSIGNMENT_ARENA,
                &change.local_entries[..change.local_count],
            )? != change.local_plan
        {
            return Err(SupportLedgerError::Generation);
        }
        Ok(())
    }

    fn validates_plan_slot_images(
        &self,
        change: &PreparedPlanCreate,
    ) -> Result<bool, SupportLedgerError> {
        for branch in 0..PLAN_BRANCHES {
            let row_start = branch * PLAN_MEMBERS_MAX;
            let group = self.groups.prepare_reserved_image_after(
                change.groups[branch],
                encode_plan_group(
                    branch,
                    change.input.member_count,
                    change.input.authority_key,
                    change.formations[branch],
                    change.external_heads[branch],
                    change.members.as_slice()[row_start..row_start + PLAN_MEMBERS_MAX]
                        .try_into()
                        .expect("fixed Plan member range"),
                    change
                        .groups
                        .as_slice()
                        .try_into()
                        .expect("three Plan groups"),
                ),
                1,
            )?;
            let formation = self.formations.prepare_reserved_image_after(
                change.formations[branch],
                encode_plan_formation(
                    change.input.identity,
                    branch,
                    change.groups[branch],
                    change.external_heads[branch],
                    change.input.occurred_at,
                ),
                1,
            )?;
            let head = self.external_heads.prepare_reserved_image_after(
                change.external_heads[branch],
                encode_plan_head(
                    branch,
                    change.input.member_count,
                    change.groups[branch],
                    change.formations[branch],
                    change.input.obligations[branch],
                    change.input.credits[branch],
                    change.input.authority_key,
                ),
                1,
            )?;
            if change.group_images[branch] != group
                || change.formation_images[branch] != formation
                || change.head_images[branch] != head
            {
                return Ok(false);
            }
            for ordinal in 0..PLAN_MEMBERS_MAX {
                let row = row_start + ordinal;
                let active = ordinal < change.input.member_count;
                let funder = self.funders.prepare_reserved_image_after(
                    change.funders[row],
                    encode_plan_funder(
                        branch,
                        ordinal,
                        active,
                        change.groups[branch],
                        change.formations[branch],
                        change.members[row],
                        change.input.members[ordinal],
                    ),
                    1,
                )?;
                let member = self.members.prepare_reserved_image_after(
                    change.members[row],
                    encode_plan_member(
                        branch,
                        ordinal,
                        active,
                        change.groups[branch],
                        change.funders[row],
                        change.input.members[ordinal],
                    ),
                    1,
                )?;
                if change.funder_images[row] != funder || change.member_images[row] != member {
                    return Ok(false);
                }
            }
        }
        for ordinal in 0..change.input.member_count {
            let link = self.links.prepare_reserved_image_after(
                change.links[ordinal],
                encode_plan_link(
                    change.input.members[ordinal].owner_header,
                    change.groups[0],
                    change.formations[0],
                    change.input.authority_key,
                    self.generation(),
                ),
                1,
            )?;
            if change.link_images[ordinal] != link {
                return Ok(false);
            }
        }
        let mutation = self.mutations.prepare_reserved_image_after(
            change.mutations[0],
            encode_plan_mutation(
                change.input.authority_key,
                change
                    .groups
                    .as_slice()
                    .try_into()
                    .expect("three Plan groups"),
                change
                    .formations
                    .as_slice()
                    .try_into()
                    .expect("three Plan formations"),
                self.generation(),
                change.input.occurred_at,
            ),
            1,
        )?;
        Ok(change.mutation_image == mutation)
    }

    pub(super) fn commit_plan_create_prevalidated(
        &mut self,
        change: PreparedPlanCreate,
        apply_index_plans: bool,
    ) {
        for branch in 0..PLAN_BRANCHES {
            self.groups
                .install_reserved_image_direct(change.groups[branch], change.group_images[branch]);
            self.external_heads.install_reserved_image_direct(
                change.external_heads[branch],
                change.head_images[branch],
            );
            self.formations.install_reserved_image_direct(
                change.formations[branch],
                change.formation_images[branch],
            );
        }
        for row in 0..PLAN_FUNDER_ROWS {
            self.funders
                .install_reserved_image_direct(change.funders[row], change.funder_images[row]);
            self.members
                .install_reserved_image_direct(change.members[row], change.member_images[row]);
        }
        for ordinal in 0..change.input.member_count {
            self.links
                .install_reserved_image_direct(change.links[ordinal], change.link_images[ordinal]);
        }
        self.mutations
            .install_reserved_image_direct(change.mutations[0], change.mutation_image);
        for ordinal in 0..change.input.member_count {
            self.owner_rows.replace_image_prevalidated(
                change.owner_references[ordinal][1],
                change.owner_row_images[ordinal],
            );
            self.owners.replace_image_prevalidated(
                change.owner_references[ordinal][3],
                change.owner_images[ordinal],
            );
        }
        if apply_index_plans {
            self.authority
                .commit_assignment_plan_prevalidated(change.authority_plan);
            self.raw
                .commit_assignment_plan_prevalidated(change.raw_plan);
            self.local
                .commit_assignment_plan_prevalidated(change.local_plan);
        }
        self.assign_plan_arena_headers(change.arena_headers_after);
        self.header = change.header_after;
    }

    pub(super) fn plan_links(change: &PreparedPlanCreate) -> &[ArenaRef] {
        change.links.as_slice()
    }

    fn prepare_plan_owner_updates(
        &self,
        input: PlanCreateInput,
        owner_records: [Option<BundleRecord>; PLAN_MEMBERS_MAX],
        links: &[ArenaRef],
    ) -> Result<
        (
            [[ArenaRef; 4]; PLAN_MEMBERS_MAX],
            [[u8; OWNER_ROW_BYTES]; PLAN_MEMBERS_MAX],
            [[u8; OWNER_BYTES]; PLAN_MEMBERS_MAX],
        ),
        SupportLedgerError,
    > {
        if links.len() != input.member_count
            || owner_records[..input.member_count]
                .iter()
                .any(Option::is_none)
            || owner_records[input.member_count..]
                .iter()
                .any(Option::is_some)
        {
            return Err(SupportLedgerError::InvalidInput);
        }
        let mut references = [[ArenaRef::default(); 4]; PLAN_MEMBERS_MAX];
        let mut rows = [[0; OWNER_ROW_BYTES]; PLAN_MEMBERS_MAX];
        let mut owners = [[0; OWNER_BYTES]; PLAN_MEMBERS_MAX];
        for ordinal in 0..input.member_count {
            let member = input.members[ordinal];
            let record = owner_records[ordinal].expect("validated active Plan owner");
            if member.record_slot != member.owner_header.slot
                || record.request_owner != member.request.expect("active Plan member")
                || crate::request_book::c17::request_key(record.request_owner) != member.request_key
                || record.entitlement.get() != member.entitlement
                || record.vector.get() != member.vector
                || !matches!(
                    record.state,
                    BundleState::LivePristine | BundleState::LiveConsumed
                )
            {
                return Err(SupportLedgerError::InvalidTransition);
            }
            references[ordinal] = [
                self.owner_headers.reference_at(member.record_slot, &[1])?,
                self.owner_rows.reference_at(member.record_slot, &[1])?,
                self.owner_indices.reference_at(member.record_slot, &[1])?,
                self.owners.reference_at(member.record_slot, &[1])?,
            ];
            if references[ordinal][0] != member.owner_header {
                return Err(noncanonical_error());
            }
            let current = [
                self.owner_headers
                    .image(references[ordinal][0], &[1])?
                    .as_slice(),
                self.owner_rows
                    .image(references[ordinal][1], &[1])?
                    .as_slice(),
                self.owner_indices
                    .image(references[ordinal][2], &[1])?
                    .as_slice(),
                self.owners.image(references[ordinal][3], &[1])?.as_slice(),
            ];
            validate_c16_owner_set(
                current,
                references[ordinal],
                member.record_slot,
                &record,
                OWNER_STATE_LIVE,
            )?;
            let mut row = *self.owner_rows.image(references[ordinal][1], &[1])?;
            let mut owner = *self.owners.image(references[ordinal][3], &[1])?;
            if row[OWNER_ROW_ACTIVE_LINK..OWNER_ROW_ACTIVE_LINK + 8]
                .iter()
                .any(|byte| *byte != 0)
            {
                return Err(SupportLedgerError::InvalidTransition);
            }
            let next_claims = record
                .linked_claims
                .checked_add(PLAN_BRANCHES as u32)
                .ok_or_else(capacity_error)?;
            let next_current = read_u64(&row, OWNER_ROW_CURRENT)
                .checked_add(PLAN_BRANCHES as u64)
                .ok_or_else(capacity_error)?;
            let mut next_branches = [0u64; PLAN_BRANCHES];
            for (branch, next_slot) in next_branches.iter_mut().enumerate() {
                let offset = OWNER_ROW_BRANCH_CURRENT + branch * 8;
                *next_slot = read_u64(&row, offset)
                    .checked_add(1)
                    .ok_or_else(capacity_error)?;
            }
            let terminal_current = read_u64(&row, OWNER_ROW_BRANCH_CURRENT + 3 * 8);
            let formation_current = next_branches[1]
                .checked_add(next_branches[2])
                .and_then(|value| value.checked_add(terminal_current))
                .ok_or_else(capacity_error)?;
            if next_branches[0] > member.branch_limits[0]
                || formation_current > member.branch_limits[1].min(member.branch_limits[2])
            {
                return Err(capacity_error());
            }
            for (branch, next) in next_branches.into_iter().enumerate() {
                write_u64(&mut row, OWNER_ROW_BRANCH_CURRENT + branch * 8, next);
            }
            write_u32(&mut row, OWNER_ROW_LINKED_CLAIMS, next_claims);
            write_u64(&mut row, OWNER_ROW_CURRENT, next_current);
            encode_arena_ref(
                &mut row[OWNER_ROW_ACTIVE_LINK..OWNER_ROW_ACTIVE_LINK + 8],
                links[ordinal],
            );
            write_u32(&mut owner, OWNER_IMAGE_LINKED_CLAIMS, next_claims);
            rows[ordinal] = row;
            owners[ordinal] = owner;
        }
        Ok((references, rows, owners))
    }

    fn prepare_plan_arena_headers_after(
        &self,
        groups: &ArenaSelection<PLAN_BRANCHES>,
        external_heads: &ArenaSelection<PLAN_BRANCHES>,
        formations: &ArenaSelection<PLAN_BRANCHES>,
        funders: &ArenaSelection<PLAN_FUNDER_ROWS>,
        members: &ArenaSelection<PLAN_MEMBER_ROWS>,
        links: &ArenaSelection<PLAN_MEMBERS_MAX>,
        mutations: &ArenaSelection<1>,
    ) -> Result<[ByteArenaHeaderImage; 11], SupportLedgerError> {
        Ok([
            self.groups
                .prepare_reserve_header_after(groups, groups.len(), 0)?,
            self.external_heads.prepare_reserve_header_after(
                external_heads,
                external_heads.len(),
                0,
            )?,
            self.formations
                .prepare_reserve_header_after(formations, formations.len(), 0)?,
            self.funders
                .prepare_reserve_header_after(funders, funders.len(), 0)?,
            self.members
                .prepare_reserve_header_after(members, members.len(), 0)?,
            self.links
                .prepare_reserve_header_after(links, links.len(), 0)?,
            self.mutations
                .prepare_reserve_header_after(mutations, mutations.len(), 0)?,
            self.owner_headers.header_image(),
            self.owner_rows.prepare_generation_header_after()?,
            self.owner_indices.header_image(),
            self.owners.prepare_generation_header_after()?,
        ])
    }

    fn assign_plan_arena_headers(&mut self, headers: [ByteArenaHeaderImage; 11]) {
        self.groups.assign_header_direct(headers[0]);
        self.external_heads.assign_header_direct(headers[1]);
        self.formations.assign_header_direct(headers[2]);
        self.funders.assign_header_direct(headers[3]);
        self.members.assign_header_direct(headers[4]);
        self.links.assign_header_direct(headers[5]);
        self.mutations.assign_header_direct(headers[6]);
        self.owner_headers.assign_header_direct(headers[7]);
        self.owner_rows.assign_header_direct(headers[8]);
        self.owner_indices.assign_header_direct(headers[9]);
        self.owners.assign_header_direct(headers[10]);
    }

    fn plan_arena_headers(&self) -> [ByteArenaHeaderImage; 11] {
        [
            self.groups.header_image(),
            self.external_heads.header_image(),
            self.formations.header_image(),
            self.funders.header_image(),
            self.members.header_image(),
            self.links.header_image(),
            self.mutations.header_image(),
            self.owner_headers.header_image(),
            self.owner_rows.header_image(),
            self.owner_indices.header_image(),
            self.owners.header_image(),
        ]
    }

    fn prepare_begin_pending_image(
        &self,
        batch: u64,
        aggregate: LifecycleAggregate,
        before_support: SupportLedgerGeneration,
        selection: &ArenaSelection<LIFECYCLE_BATCH_MAX>,
    ) -> Result<PendingLifecycleHeaderImage, SupportLedgerError> {
        let mut pending = PendingLifecycleHeaderImage::ZERO;
        pending.0[PENDING_STATE] = PendingState::Staging as u8;
        write_u64(&mut pending.0, PENDING_BATCH, batch);
        write_u16(&mut pending.0, PENDING_TOTAL, selection.len() as u16);
        write_u16(&mut pending.0, PENDING_CURSOR, selection.len() as u16);
        write_u16(&mut pending.0, PENDING_RESERVED, selection.len() as u16);
        for (ordinal, reference) in selection.as_slice().iter().enumerate() {
            write_u16(
                &mut pending.0,
                PENDING_SLOTS + ordinal * 2,
                reference.slot as u16,
            );
        }
        pending.0[PENDING_AGGREGATE..PENDING_BEFORE_SUPPORT].copy_from_slice(&aggregate.encode());
        write_u64(&mut pending.0, PENDING_BEFORE_SUPPORT, before_support.get());
        write_u64(&mut pending.0, PENDING_EXPECTED_RAW, self.raw.generation());
        for (index, value) in aggregate.withholding()?.into_iter().enumerate() {
            write_u64(&mut pending.0, PENDING_WITHHELD + index * 8, value);
        }
        Ok(pending)
    }

    pub(crate) fn prepare_begin_batch(
        &self,
        total: usize,
        aggregate: LifecycleAggregate,
        before_support: SupportLedgerGeneration,
    ) -> Result<PreparedLifecycleBegin, SupportLedgerError> {
        if self.pending_state()? != PendingState::Empty
            || !(1..=LIFECYCLE_BATCH_MAX).contains(&total)
            || aggregate == LifecycleAggregate::ZERO
        {
            return Err(SupportLedgerError::InvalidTransition);
        }
        let batch = read_u64(&self.pending.0, PENDING_BATCH)
            .checked_add(1)
            .ok_or_else(capacity_error)?;
        let generation_after = self
            .generation()
            .checked_add(1)
            .ok_or_else(capacity_error)?;
        let selection = self
            .lifecycle
            .prepare_reserve::<LIFECYCLE_BATCH_MAX>(total)?;
        let expected_arena_header = self.lifecycle.header_image();
        let arena_header_after = self
            .lifecycle
            .prepare_reserve_header_after(&selection, 0, 0)?;
        let pending_after =
            self.prepare_begin_pending_image(batch, aggregate, before_support, &selection)?;
        Ok(PreparedLifecycleBegin {
            expected_c17: self.generation(),
            generation_after,
            expected_arena_header,
            arena_header_after,
            before_support,
            batch,
            aggregate,
            selection,
            pending_before: self.pending,
            pending_after,
        })
    }

    pub(crate) fn validate_begin_batch(
        &self,
        change: &PreparedLifecycleBegin,
    ) -> Result<(), SupportLedgerError> {
        if self.pending_state()? != PendingState::Empty
            || self.pending != change.pending_before
            || self.generation() != change.expected_c17
            || change.expected_c17.checked_add(1) != Some(change.generation_after)
            || self.lifecycle.header_image() != change.expected_arena_header
            || self
                .lifecycle
                .prepare_reserve_header_after(&change.selection, 0, 0)?
                != change.arena_header_after
            || change.aggregate == LifecycleAggregate::ZERO
            || self
                .lifecycle
                .prepare_reserve::<LIFECYCLE_BATCH_MAX>(change.selection.len())?
                .as_slice()
                != change.selection.as_slice()
            || self.prepare_begin_pending_image(
                change.batch,
                change.aggregate,
                change.before_support,
                &change.selection,
            )? != change.pending_after
        {
            return Err(SupportLedgerError::Generation);
        }
        Ok(())
    }

    pub(crate) fn commit_begin_batch(&mut self, change: PreparedLifecycleBegin) {
        self.lifecycle.reserve_selection_direct(&change.selection);
        self.lifecycle
            .assign_header_direct(change.arena_header_after);
        self.pending = change.pending_after;
        write_u64(&mut self.header.0, 48, change.generation_after);
    }

    pub(crate) fn next_lifecycle_ordinal(&self) -> Result<usize, SupportLedgerError> {
        if self.pending_state()? != PendingState::Staging {
            return Err(SupportLedgerError::InvalidTransition);
        }
        Ok(usize::from(read_u16(&self.pending.0, PENDING_STAGED)))
    }

    pub(crate) fn prepare_stage_chunk(
        &self,
        records: &[LifecycleRecordInput],
    ) -> Result<PreparedLifecycleStage, SupportLedgerError> {
        if self.pending_state()? != PendingState::Staging
            || records.is_empty()
            || records.len() > LIFECYCLE_CHUNK_MAX
        {
            return Err(SupportLedgerError::InvalidTransition);
        }
        let staged = usize::from(read_u16(&self.pending.0, PENDING_STAGED));
        let total = usize::from(read_u16(&self.pending.0, PENDING_TOTAL));
        if staged + records.len() > total {
            return Err(SupportLedgerError::InvalidTransition);
        }
        let batch = read_u64(&self.pending.0, PENDING_BATCH);
        let mut retained = [LifecycleRecordInput::ZERO; LIFECYCLE_CHUNK_MAX];
        let mut record_images = [[0; LIFECYCLE_BYTES]; LIFECYCLE_CHUNK_MAX];
        let mut references = [ArenaRef::default(); LIFECYCLE_CHUNK_MAX];
        let expected_arena_header = self.lifecycle.header_image();
        let mut raw_entries = [([0; 32], [0; 8]); 2 * LIFECYCLE_CHUNK_MAX];
        let mut raw_count = 0;
        for (offset, record) in records.iter().copied().enumerate() {
            record.validate()?;
            let ordinal = staged + offset;
            let reference = self.reserved_reference(ordinal)?;
            self.validate_lifecycle_record_owner_set(record, reference)?;
            retained[offset] = record;
            references[offset] = reference;
            record_images[offset] = self.lifecycle.prepare_reserved_image_after(
                reference,
                record.encode(reference, batch, ordinal)?,
                3,
            )?;
            for (kind, key) in [
                (RawOwnerKind::LifecycleObligation, record.obligation_raw),
                (RawOwnerKind::LifecycleCredit, record.credit_raw),
            ] {
                raw_entries[raw_count] = (
                    key,
                    encode_raw_owner(kind, RawOwnerState::Inactive, reference)?,
                );
                raw_count += 1;
            }
        }
        raw_entries[..raw_count].sort_unstable_by_key(|entry| entry.0);
        for index in 1..raw_count {
            if raw_entries[index - 1].0 >= raw_entries[index].0 {
                return Err(SupportLedgerError::InvalidInput);
            }
        }
        let raw_plan = self.raw.prepare_insert_assignment_plan(
            RAW_INDEX_ASSIGNMENT_ARENA,
            &raw_entries[..raw_count],
        )?;
        let generation_after = self
            .generation()
            .checked_add(1)
            .ok_or_else(capacity_error)?;
        let raw_generation_after = self
            .raw
            .generation()
            .checked_add(1)
            .ok_or_else(capacity_error)?;
        let mut pending_after = self.pending;
        write_u16(
            &mut pending_after.0,
            PENDING_STAGED,
            (staged + records.len()) as u16,
        );
        write_u64(
            &mut pending_after.0,
            PENDING_EXPECTED_RAW,
            raw_generation_after,
        );
        let arena_header_after = self.lifecycle.prepare_install_reserved_header_after(
            &references[..records.len()],
            0,
            records.len(),
        )?;
        Ok(PreparedLifecycleStage {
            expected_c17: self.generation(),
            generation_after,
            expected_raw: self.raw.generation(),
            expected_arena_header,
            arena_header_after,
            batch,
            first: staged,
            len: records.len(),
            records: retained,
            record_images,
            references,
            raw_entries,
            raw_count,
            pending_after,
            raw_plan,
        })
    }

    pub(crate) fn validate_stage_chunk(
        &self,
        change: &PreparedLifecycleStage,
    ) -> Result<(), SupportLedgerError> {
        let staged_after = change
            .first
            .checked_add(change.len)
            .and_then(|value| u16::try_from(value).ok())
            .ok_or(SupportLedgerError::Generation)?;
        let raw_generation_after = change
            .expected_raw
            .checked_add(1)
            .ok_or(SupportLedgerError::Generation)?;
        let mut pending_after = self.pending;
        write_u16(&mut pending_after.0, PENDING_STAGED, staged_after);
        write_u64(
            &mut pending_after.0,
            PENDING_EXPECTED_RAW,
            raw_generation_after,
        );
        if self.pending_state()? != PendingState::Staging
            || self.generation() != change.expected_c17
            || change.expected_c17.checked_add(1) != Some(change.generation_after)
            || self.raw.generation() != change.expected_raw
            || self.lifecycle.header_image() != change.expected_arena_header
            || self.lifecycle.prepare_install_reserved_header_after(
                &change.references[..change.len],
                0,
                change.len,
            )? != change.arena_header_after
            || pending_after != change.pending_after
            || read_u64(&self.pending.0, PENDING_BATCH) != change.batch
            || usize::from(read_u16(&self.pending.0, PENDING_STAGED)) != change.first
            || !(1..=LIFECYCLE_CHUNK_MAX).contains(&change.len)
            || change.raw_count != change.len * 2
            || change.records[change.len..]
                .iter()
                .any(|record| *record != LifecycleRecordInput::ZERO)
            || change.record_images[change.len..]
                .iter()
                .flatten()
                .any(|byte| *byte != 0)
            || change.references[change.len..]
                .iter()
                .any(|reference| *reference != ArenaRef::default())
            || change.raw_entries[change.raw_count..]
                .iter()
                .any(|entry| *entry != ([0; 32], [0; 8]))
        {
            return Err(SupportLedgerError::Generation);
        }
        for offset in 0..change.len {
            let ordinal = change.first + offset;
            if self.reserved_reference(ordinal)? != change.references[offset] {
                return Err(SupportLedgerError::Generation);
            }
            change.records[offset].validate()?;
            self.validate_lifecycle_record_owner_set(
                change.records[offset],
                change.references[offset],
            )?;
            if self.lifecycle.prepare_reserved_image_after(
                change.references[offset],
                change.records[offset].encode(change.references[offset], change.batch, ordinal)?,
                3,
            )? != change.record_images[offset]
            {
                return Err(SupportLedgerError::Generation);
            }
        }
        self.raw
            .validate_insert_batch(&change.raw_entries[..change.raw_count])?;
        if !self.raw.validates_assignment_plan(&change.raw_plan) {
            return Err(SupportLedgerError::Generation);
        }
        Ok(())
    }

    pub(crate) fn commit_stage_chunk(&mut self, change: PreparedLifecycleStage) {
        for offset in 0..change.len {
            self.lifecycle.install_reserved_image_direct(
                change.references[offset],
                change.record_images[offset],
            );
        }
        self.lifecycle
            .assign_header_direct(change.arena_header_after);
        self.raw
            .commit_assignment_plan_prevalidated(change.raw_plan);
        self.pending = change.pending_after;
        write_u64(&mut self.header.0, 48, change.generation_after);
    }

    fn prepare_finalize_raw_update(
        &self,
        reference: ArenaRef,
        image: &[u8; LIFECYCLE_BYTES],
        key_offset: usize,
    ) -> Result<LifecycleRawUpdate, SupportLedgerError> {
        let mut key = [0; 32];
        key.copy_from_slice(&image[key_offset..key_offset + 32]);
        let handle = self
            .raw
            .find_handle(&key)?
            .ok_or(SupportLedgerError::InvalidTransition)?;
        let before = self.raw.value_at(handle)?;
        let (kind, state, owner) = decode_raw_owner(before)?;
        if state != RawOwnerState::Inactive || owner != reference {
            return Err(noncanonical_error());
        }
        Ok(LifecycleRawUpdate {
            handle,
            after: encode_raw_owner(kind, RawOwnerState::Committed, reference)?,
        })
    }

    fn prepare_finalize_owner_outcome(
        &self,
        publications: &[LifecyclePublication],
        index: usize,
    ) -> Result<LifecycleOwnerOutcome, SupportLedgerError> {
        let publication = publications[index];
        let owner_row = self.owner_rows.reference_at(publication.owner_slot, &[1])?;
        let owner = self.owners.reference_at(publication.owner_slot, &[1])?;
        let row = self.owner_rows.image(owner_row, &[1])?;
        let owner_image = self.owners.image(owner, &[1])?;
        let owner_delta = u64::try_from(
            publications
                .iter()
                .filter(|candidate| candidate.owner_slot == publication.owner_slot)
                .count(),
        )
        .map_err(|_| capacity_error())?;
        let linked_base = read_u32(row, OWNER_ROW_LINKED_CLAIMS);
        if read_u32(owner_image, OWNER_IMAGE_LINKED_CLAIMS) != linked_base {
            return Err(noncanonical_error());
        }
        let linked_after = u64::from(linked_base)
            .checked_add(owner_delta)
            .and_then(|value| u32::try_from(value).ok())
            .ok_or_else(capacity_error)?;
        let current_after = read_u64(row, OWNER_ROW_CURRENT)
            .checked_add(owner_delta)
            .ok_or_else(capacity_error)?;
        let mut branches_after = [0; OWNER_FUNDING_BRANCHES];
        for (branch, after) in branches_after.iter_mut().enumerate() {
            let delta = u64::try_from(
                publications
                    .iter()
                    .filter(|candidate| {
                        candidate.owner_slot == publication.owner_slot
                            && usize::from(candidate.branch) == branch
                    })
                    .count(),
            )
            .map_err(|_| capacity_error())?;
            *after = read_u64(row, OWNER_ROW_BRANCH_CURRENT + branch * 8)
                .checked_add(delta)
                .ok_or_else(capacity_error)?;
        }
        Ok(LifecycleOwnerOutcome {
            owner_slot: publication.owner_slot,
            linked_after,
            current_after,
            branches_after,
        })
    }

    fn prepare_finalize_funder_outcome(
        &self,
        publications: &[LifecyclePublication],
        index: usize,
    ) -> Result<LifecycleFunderOutcome, SupportLedgerError> {
        let reference = publications[index].funder;
        let image = self.funders.image(reference, &[1])?;
        let delta = u64::try_from(
            publications
                .iter()
                .filter(|candidate| candidate.funder == reference)
                .count(),
        )
        .map_err(|_| capacity_error())?;
        let current_after = read_u64(image, 112)
            .checked_add(delta)
            .ok_or_else(capacity_error)?;
        if current_after > read_u64(image, 120) {
            return Err(capacity_error());
        }
        Ok(LifecycleFunderOutcome {
            reference,
            current_after,
        })
    }

    pub(crate) fn prepare_finalize_batch(
        &self,
        expected_support: SupportLedgerGeneration,
    ) -> Result<PreparedLifecycleFinalize, SupportLedgerError> {
        self.validate_finalizable(expected_support)?;
        let aggregate = self.pending_aggregate()?;
        let (_, publication_count) = self.lifecycle_publications()?;
        let expected_arena_header = self.lifecycle.header_image();
        let expected_publication_arena_headers = [
            self.owner_rows.header_image(),
            self.owners.header_image(),
            self.funders.header_image(),
        ];
        let publication_arena_headers_after = [
            self.owner_rows.prepare_generation_header_after()?,
            self.owners.prepare_generation_header_after()?,
            self.funders.prepare_generation_header_after()?,
        ];

        let total = usize::from(read_u16(&self.pending.0, PENDING_TOTAL));
        let mut references = [ArenaRef::default(); LIFECYCLE_BATCH_MAX];
        let mut occupied_tags_after = [0; LIFECYCLE_BATCH_MAX];
        let mut raw_updates = [LifecycleRawUpdate::ZERO; 2 * LIFECYCLE_BATCH_MAX];
        for ordinal in 0..total {
            let reference = self.inactive_reference(ordinal)?;
            let image = self.lifecycle.image(reference, &[3])?;
            references[ordinal] = reference;
            occupied_tags_after[ordinal] = self
                .lifecycle
                .prepare_committed_inactive_tag_after(reference)?;
            raw_updates[ordinal * 2] = self.prepare_finalize_raw_update(reference, image, 96)?;
            raw_updates[ordinal * 2 + 1] =
                self.prepare_finalize_raw_update(reference, image, 128)?;
        }
        let raw_generation_plan = self
            .raw
            .prepare_generation_assignment_plan::<RAW_GENERATION_ASSIGNMENTS>(
                RAW_INDEX_ASSIGNMENT_ARENA,
            )?;
        let arena_header_after = self
            .lifecycle
            .prepare_commit_inactive_header_after(&references[..total])?;
        let generation_after = self
            .generation()
            .checked_add(1)
            .ok_or_else(capacity_error)?;
        let batch = read_u64(&self.pending.0, PENDING_BATCH);
        let mut pending_after = PendingLifecycleHeaderImage::ZERO;
        write_u64(&mut pending_after.0, PENDING_BATCH, batch);
        Ok(PreparedLifecycleFinalize {
            expected_c17: self.generation(),
            generation_after,
            expected_raw: self.raw.generation(),
            expected_arena_header,
            arena_header_after,
            expected_publication_arena_headers,
            publication_arena_headers_after,
            batch,
            total: total as u16,
            expected_support,
            aggregate,
            pending_before: self.pending,
            pending_after,
            references,
            occupied_tags_after,
            raw_updates,
            raw_generation_plan,
            publication_count,
        })
    }

    pub(crate) fn validate_finalize_batch(
        &self,
        change: &PreparedLifecycleFinalize,
    ) -> Result<(), SupportLedgerError> {
        self.validate_finalizable(change.expected_support)?;
        let (_, publication_count) = self.lifecycle_publications()?;
        let total = usize::from(change.total);
        if self.generation() != change.expected_c17
            || change.expected_c17.checked_add(1) != Some(change.generation_after)
            || self.raw.generation() != change.expected_raw
            || !self
                .raw
                .validates_assignment_plan(&change.raw_generation_plan)
            || self
                .raw
                .prepare_generation_assignment_plan::<RAW_GENERATION_ASSIGNMENTS>(
                    RAW_INDEX_ASSIGNMENT_ARENA,
                )?
                != change.raw_generation_plan
            || self.lifecycle.header_image() != change.expected_arena_header
            || self
                .lifecycle
                .prepare_commit_inactive_header_after(&change.references[..total])?
                != change.arena_header_after
            || [
                self.owner_rows.header_image(),
                self.owners.header_image(),
                self.funders.header_image(),
            ] != change.expected_publication_arena_headers
            || [
                self.owner_rows.prepare_generation_header_after()?,
                self.owners.prepare_generation_header_after()?,
                self.funders.prepare_generation_header_after()?,
            ] != change.publication_arena_headers_after
            || self.pending != change.pending_before
            || read_u64(&self.pending.0, PENDING_BATCH) != change.batch
            || read_u16(&self.pending.0, PENDING_TOTAL) != change.total
            || self.pending_aggregate()? != change.aggregate
            || publication_count != change.publication_count
            || change.references[total..]
                .iter()
                .any(|reference| *reference != ArenaRef::default())
            || change.occupied_tags_after[total..]
                .iter()
                .any(|tag| *tag != 0)
            || change.raw_updates[total * 2..]
                .iter()
                .any(|update| *update != LifecycleRawUpdate::ZERO)
        {
            return Err(SupportLedgerError::Generation);
        }
        let mut pending_after = PendingLifecycleHeaderImage::ZERO;
        write_u64(&mut pending_after.0, PENDING_BATCH, change.batch);
        if change.pending_after != pending_after {
            return Err(SupportLedgerError::Generation);
        }
        for ordinal in 0..total {
            let reference = self.inactive_reference(ordinal)?;
            let image = self.lifecycle.image(reference, &[3])?;
            if reference != change.references[ordinal]
                || self
                    .lifecycle
                    .prepare_committed_inactive_tag_after(reference)?
                    != change.occupied_tags_after[ordinal]
                || self.prepare_finalize_raw_update(reference, image, 96)?
                    != change.raw_updates[ordinal * 2]
                || self.prepare_finalize_raw_update(reference, image, 128)?
                    != change.raw_updates[ordinal * 2 + 1]
            {
                return Err(SupportLedgerError::Generation);
            }
        }
        self.owner_rows.validate_advance_generation()?;
        self.owners.validate_advance_generation()?;
        self.funders.validate_advance_generation()?;
        Ok(())
    }

    pub(super) fn visit_finalize_publications(
        &self,
        change: &PreparedLifecycleFinalize,
        visitor: &mut dyn FnMut(LifecyclePublication) -> Result<(), SupportLedgerError>,
    ) -> Result<(), SupportLedgerError> {
        let (publications, count) = self.lifecycle_publications()?;
        if count != change.publication_count {
            return Err(SupportLedgerError::Generation);
        }
        for publication in publications[..count].iter().copied() {
            visitor(publication)?;
        }
        Ok(())
    }

    pub(super) fn prepare_finalize_owner_outcomes(
        &self,
        change: &PreparedLifecycleFinalize,
        owner_outcomes: &mut [LifecycleOwnerOutcome; LIFECYCLE_CAPACITY],
        funder_outcomes: &mut [LifecycleFunderOutcome; LIFECYCLE_CAPACITY],
    ) -> Result<(usize, usize), SupportLedgerError> {
        let (publications, count) = self.lifecycle_publications()?;
        if count != change.publication_count {
            return Err(SupportLedgerError::Generation);
        }
        let mut owner_count = 0usize;
        let mut funder_count = 0usize;
        for index in 0..count {
            let publication = publications[index];
            if !publications[..index]
                .iter()
                .any(|prior| prior.owner_slot == publication.owner_slot)
            {
                if owner_count == owner_outcomes.len() {
                    return Err(capacity_error());
                }
                owner_outcomes[owner_count] =
                    self.prepare_finalize_owner_outcome(&publications[..count], index)?;
                owner_count += 1;
            }
            if !publications[..index]
                .iter()
                .any(|prior| prior.funder == publication.funder)
            {
                if funder_count == funder_outcomes.len() {
                    return Err(capacity_error());
                }
                funder_outcomes[funder_count] =
                    self.prepare_finalize_funder_outcome(&publications[..count], index)?;
                funder_count += 1;
            }
        }
        Ok((owner_count, funder_count))
    }

    pub(super) fn validate_finalize_owner_outcomes(
        &self,
        change: &PreparedLifecycleFinalize,
        owner_outcomes: &[LifecycleOwnerOutcome; LIFECYCLE_CAPACITY],
        owner_count: usize,
        funder_outcomes: &[LifecycleFunderOutcome; LIFECYCLE_CAPACITY],
        funder_count: usize,
    ) -> Result<(), SupportLedgerError> {
        if owner_count > owner_outcomes.len()
            || funder_count > funder_outcomes.len()
            || owner_outcomes[owner_count..]
                .iter()
                .any(|outcome| *outcome != LifecycleOwnerOutcome::ZERO)
            || funder_outcomes[funder_count..]
                .iter()
                .any(|outcome| *outcome != LifecycleFunderOutcome::ZERO)
        {
            return Err(SupportLedgerError::Generation);
        }
        let (publications, count) = self.lifecycle_publications()?;
        if count != change.publication_count {
            return Err(SupportLedgerError::Generation);
        }
        let mut seen_owners = 0usize;
        let mut seen_funders = 0usize;
        for index in 0..count {
            let publication = publications[index];
            if !publications[..index]
                .iter()
                .any(|prior| prior.owner_slot == publication.owner_slot)
            {
                if seen_owners == owner_count
                    || self.prepare_finalize_owner_outcome(&publications[..count], index)?
                        != owner_outcomes[seen_owners]
                {
                    return Err(SupportLedgerError::Generation);
                }
                seen_owners += 1;
            }
            if !publications[..index]
                .iter()
                .any(|prior| prior.funder == publication.funder)
            {
                if seen_funders == funder_count
                    || self.prepare_finalize_funder_outcome(&publications[..count], index)?
                        != funder_outcomes[seen_funders]
                {
                    return Err(SupportLedgerError::Generation);
                }
                seen_funders += 1;
            }
        }
        (seen_owners == owner_count && seen_funders == funder_count)
            .then_some(())
            .ok_or(SupportLedgerError::Generation)
    }

    pub(crate) fn commit_finalize_records(&mut self, change: &PreparedLifecycleFinalize) {
        let total = usize::from(change.total);
        for ordinal in 0..total {
            self.lifecycle.assign_slot_tag_direct(
                change.references[ordinal],
                change.occupied_tags_after[ordinal],
            );
        }
        self.lifecycle
            .assign_header_direct(change.arena_header_after);
        for update in &change.raw_updates[..total * 2] {
            self.raw.replace_value_direct(update.handle, update.after);
        }
        let raw_header = change
            .raw_generation_plan
            .assignments()
            .first()
            .expect("sealed Raw generation assignment");
        self.raw.commit_assignment_direct(raw_header);
    }

    pub(super) fn commit_finalize_owner_sets(
        &mut self,
        change: &PreparedLifecycleFinalize,
        owner_outcomes: &[LifecycleOwnerOutcome],
        funder_outcomes: &[LifecycleFunderOutcome],
    ) {
        for outcome in owner_outcomes {
            let row = self.owner_rows.image_mut_slot_direct(outcome.owner_slot);
            write_u32(row, OWNER_ROW_LINKED_CLAIMS, outcome.linked_after);
            write_u64(row, OWNER_ROW_CURRENT, outcome.current_after);
            for (branch, after) in outcome.branches_after.into_iter().enumerate() {
                write_u64(row, OWNER_ROW_BRANCH_CURRENT + branch * 8, after);
            }
            write_u32(
                self.owners.image_mut_slot_direct(outcome.owner_slot),
                OWNER_IMAGE_LINKED_CLAIMS,
                outcome.linked_after,
            );
        }
        for outcome in funder_outcomes {
            write_u64(
                self.funders.image_mut_prevalidated(outcome.reference),
                112,
                outcome.current_after,
            );
        }
        self.owner_rows
            .assign_header_direct(change.publication_arena_headers_after[0]);
        self.owners
            .assign_header_direct(change.publication_arena_headers_after[1]);
        self.funders
            .assign_header_direct(change.publication_arena_headers_after[2]);
    }

    pub(crate) fn complete_finalize_batch(&mut self, change: PreparedLifecycleFinalize) {
        self.pending = change.pending_after;
        write_u64(&mut self.header.0, 48, change.generation_after);
    }

    pub(crate) fn prepare_abort_chunk(&self) -> Result<PreparedLifecycleAbort, SupportLedgerError> {
        let state = self.pending_state()?;
        if !matches!(state, PendingState::Staging | PendingState::Aborting) {
            return Err(SupportLedgerError::InvalidTransition);
        }
        let total = usize::from(read_u16(&self.pending.0, PENDING_TOTAL));
        let staged = usize::from(read_u16(&self.pending.0, PENDING_STAGED));
        let cursor = if state == PendingState::Staging {
            total
        } else {
            usize::from(read_u16(&self.pending.0, PENDING_CURSOR))
        };
        if cursor == 0 || staged > total || cursor > total {
            return Err(SupportLedgerError::InvalidTransition);
        }
        let first = cursor.saturating_sub(LIFECYCLE_CHUNK_MAX);
        let len = cursor - first;
        let expected_arena_header = self.lifecycle.header_image();
        let mut references = [ArenaRef::default(); LIFECYCLE_CHUNK_MAX];
        let mut free_positions = [0; LIFECYCLE_CHUNK_MAX];
        let mut vacant_images = [[0; LIFECYCLE_BYTES]; LIFECYCLE_CHUNK_MAX];
        let mut free_cell_images = [ByteArenaFreeCellImage::ZERO; LIFECYCLE_CHUNK_MAX];
        let mut raw_keys = [[0; 32]; 2 * LIFECYCLE_CHUNK_MAX];
        let mut raw_count = 0;
        for (offset, ordinal) in (first..cursor).rev().enumerate() {
            let reference = if ordinal < staged {
                let reference = self.inactive_reference(ordinal)?;
                let image = self.lifecycle.image(reference, &[3])?;
                for key_offset in [96, 128] {
                    raw_keys[raw_count].copy_from_slice(&image[key_offset..key_offset + 32]);
                    let encoded = self
                        .raw
                        .find(&raw_keys[raw_count])?
                        .ok_or(SupportLedgerError::InvalidTransition)?;
                    let (_, state, owner) = decode_raw_owner(encoded)?;
                    if state != RawOwnerState::Inactive || owner != reference {
                        return Err(noncanonical_error());
                    }
                    raw_count += 1;
                }
                reference
            } else {
                self.reserved_reference(ordinal)?
            };
            references[offset] = reference;
            let (position, vacant, free_cell) = self
                .lifecycle
                .prepare_release_outcome_after(reference, offset)?;
            free_positions[offset] = position;
            vacant_images[offset] = vacant;
            free_cell_images[offset] = free_cell;
        }
        let arena_header_after = self
            .lifecycle
            .prepare_release_header_after(&references[..len])?;
        raw_keys[..raw_count].sort_unstable();
        let raw_plan = if raw_count == 0 {
            self.raw
                .prepare_generation_assignment_plan(RAW_INDEX_ASSIGNMENT_ARENA)?
        } else {
            self.raw.prepare_remove_assignment_plan(
                RAW_INDEX_ASSIGNMENT_ARENA,
                &raw_keys[..raw_count],
            )?
        };
        self.lifecycle.validate_release_batch(&references[..len])?;
        let generation_after = self
            .generation()
            .checked_add(1)
            .ok_or_else(capacity_error)?;
        let raw_generation_after = self
            .raw
            .generation()
            .checked_add(1)
            .ok_or_else(capacity_error)?;
        let batch = read_u64(&self.pending.0, PENDING_BATCH);
        let pending_after = if first == 0 {
            let mut pending = PendingLifecycleHeaderImage::ZERO;
            write_u64(&mut pending.0, PENDING_BATCH, batch);
            pending
        } else {
            let mut pending = self.pending;
            pending.0[PENDING_STATE] = PendingState::Aborting as u8;
            write_u16(&mut pending.0, PENDING_CURSOR, first as u16);
            write_u64(&mut pending.0, PENDING_EXPECTED_RAW, raw_generation_after);
            pending
        };
        Ok(PreparedLifecycleAbort {
            expected_c17: self.generation(),
            generation_after,
            expected_raw: self.raw.generation(),
            expected_arena_header,
            arena_header_after,
            batch,
            first,
            len,
            references,
            free_positions,
            vacant_images,
            free_cell_images,
            raw_keys,
            raw_count,
            pending_after,
            raw_plan,
        })
    }

    pub(crate) fn validate_abort_chunk(
        &self,
        change: &PreparedLifecycleAbort,
    ) -> Result<(), SupportLedgerError> {
        let state = self.pending_state()?;
        let cursor = if state == PendingState::Staging {
            usize::from(read_u16(&self.pending.0, PENDING_TOTAL))
        } else if state == PendingState::Aborting {
            usize::from(read_u16(&self.pending.0, PENDING_CURSOR))
        } else {
            return Err(SupportLedgerError::InvalidTransition);
        };
        let staged = usize::from(read_u16(&self.pending.0, PENDING_STAGED));
        let cursor_after = change
            .first
            .checked_add(change.len)
            .ok_or(SupportLedgerError::Generation)?;
        if self.generation() != change.expected_c17
            || change.expected_c17.checked_add(1) != Some(change.generation_after)
            || self.raw.generation() != change.expected_raw
            || self.lifecycle.header_image() != change.expected_arena_header
            || self
                .lifecycle
                .prepare_release_header_after(&change.references[..change.len])?
                != change.arena_header_after
            || read_u64(&self.pending.0, PENDING_BATCH) != change.batch
            || cursor != cursor_after
            || change.first != cursor.saturating_sub(LIFECYCLE_CHUNK_MAX)
            || !(1..=LIFECYCLE_CHUNK_MAX).contains(&change.len)
            || change.raw_count > 2 * change.len
            || change.raw_count % 2 != 0
            || change.references[change.len..]
                .iter()
                .any(|reference| *reference != ArenaRef::default())
            || change.free_positions[change.len..]
                .iter()
                .any(|position| *position != 0)
            || change.vacant_images[change.len..]
                .iter()
                .flatten()
                .any(|byte| *byte != 0)
            || change.free_cell_images[change.len..]
                .iter()
                .any(|image| *image != ByteArenaFreeCellImage::ZERO)
            || change.raw_keys[change.raw_count..]
                .iter()
                .any(|key| *key != [0; 32])
        {
            return Err(SupportLedgerError::Generation);
        }
        let raw_generation_after = change
            .expected_raw
            .checked_add(1)
            .ok_or(SupportLedgerError::Generation)?;
        let expected_pending_after = if change.first == 0 {
            let mut pending = PendingLifecycleHeaderImage::ZERO;
            write_u64(&mut pending.0, PENDING_BATCH, change.batch);
            pending
        } else {
            let mut pending = self.pending;
            pending.0[PENDING_STATE] = PendingState::Aborting as u8;
            write_u16(&mut pending.0, PENDING_CURSOR, change.first as u16);
            write_u64(&mut pending.0, PENDING_EXPECTED_RAW, raw_generation_after);
            pending
        };
        if expected_pending_after != change.pending_after {
            return Err(SupportLedgerError::Generation);
        }
        let mut expected_keys = [[0; 32]; 2 * LIFECYCLE_CHUNK_MAX];
        let mut expected_count = 0usize;
        for (offset, ordinal) in (change.first..cursor).rev().enumerate() {
            let expected_reference = if ordinal < staged {
                let reference = self.inactive_reference(ordinal)?;
                let image = self.lifecycle.image(reference, &[3])?;
                for key_offset in [96, 128] {
                    expected_keys[expected_count]
                        .copy_from_slice(&image[key_offset..key_offset + 32]);
                    let encoded = self
                        .raw
                        .find(&expected_keys[expected_count])?
                        .ok_or(SupportLedgerError::InvalidTransition)?;
                    let (_, owner_state, owner) = decode_raw_owner(encoded)?;
                    if owner_state != RawOwnerState::Inactive || owner != reference {
                        return Err(noncanonical_error());
                    }
                    expected_count += 1;
                }
                reference
            } else {
                self.reserved_reference(ordinal)?
            };
            if change.references[offset] != expected_reference {
                return Err(SupportLedgerError::Generation);
            }
            let (position, vacant, free_cell) = self
                .lifecycle
                .prepare_release_outcome_after(expected_reference, offset)?;
            if change.free_positions[offset] != position
                || change.vacant_images[offset] != vacant
                || change.free_cell_images[offset] != free_cell
            {
                return Err(SupportLedgerError::Generation);
            }
        }
        expected_keys[..expected_count].sort_unstable();
        if expected_count != change.raw_count
            || expected_keys[..expected_count] != change.raw_keys[..change.raw_count]
        {
            return Err(SupportLedgerError::Generation);
        }
        if change.raw_count == 0 {
            self.raw.validate_advance_generation()?;
        } else {
            self.raw
                .validate_remove_batch(&change.raw_keys[..change.raw_count])?;
        }
        if !self.raw.validates_assignment_plan(&change.raw_plan) {
            return Err(SupportLedgerError::Generation);
        }
        self.lifecycle
            .validate_release_batch(&change.references[..change.len])?;
        Ok(())
    }

    pub(crate) fn commit_abort_chunk(&mut self, change: PreparedLifecycleAbort) -> bool {
        self.raw
            .commit_assignment_plan_prevalidated(change.raw_plan);
        for offset in 0..change.len {
            self.lifecycle.install_reserved_image_direct(
                change.references[offset],
                change.vacant_images[offset],
            );
            self.lifecycle.assign_free_cell_direct(
                change.free_positions[offset],
                change.free_cell_images[offset],
            );
        }
        self.lifecycle
            .assign_header_direct(change.arena_header_after);
        self.pending = change.pending_after;
        write_u64(&mut self.header.0, 48, change.generation_after);
        change.first == 0
    }

    pub(super) fn funder_image(
        &self,
        reference: ArenaRef,
    ) -> Result<&[u8; FUNDER_BYTES], SupportLedgerError> {
        Ok(self.funders.image(reference, &[1])?)
    }

    pub(crate) fn lifecycle_record_by_raw(
        &self,
        key: [u8; 32],
    ) -> Result<Option<&[u8; LIFECYCLE_BYTES]>, SupportLedgerError> {
        let Some(value) = self.raw.find(&key)? else {
            return Ok(None);
        };
        let (_, state, reference) = decode_raw_owner(value)?;
        if state != RawOwnerState::Committed {
            return Ok(None);
        }
        let image = self.lifecycle.image(reference, &[1])?;
        let reciprocal = image[96..128] == key || image[128..160] == key;
        reciprocal
            .then_some(Some(image))
            .ok_or_else(noncanonical_error)
    }

    fn lifecycle_publications(
        &self,
    ) -> Result<([LifecyclePublication; LIFECYCLE_PUBLICATION_MAX], usize), SupportLedgerError>
    {
        if self.pending_state()? != PendingState::Staging {
            return Err(SupportLedgerError::InvalidTransition);
        }
        let total = usize::from(read_u16(&self.pending.0, PENDING_TOTAL));
        if total == 0 || usize::from(read_u16(&self.pending.0, PENDING_STAGED)) != total {
            return Err(SupportLedgerError::InvalidTransition);
        }
        let mut publications = [LifecyclePublication::ZERO; LIFECYCLE_PUBLICATION_MAX];
        let mut publication_count = 0usize;
        for ordinal in 0..total {
            let reserve = self.inactive_reference(ordinal)?;
            let record = LifecycleRecordInput::decode(self.lifecycle.image(reserve, &[3])?)?;
            self.validate_lifecycle_record_owner_set(record, reserve)?;
            let axis = u8::try_from(record.aggregate[2]).map_err(|_| noncanonical_error())?;
            let horizon = u8::try_from(record.aggregate[3]).map_err(|_| noncanonical_error())?;
            let branch = funding_branch(record.final_owner[17])?;
            for owner in record.owners {
                if owner == LifecycleOwnerRow::ZERO {
                    break;
                }
                let owner_header = decode_arena_ref(&owner.owner.to_le_bytes())?;
                let owner_row = decode_arena_ref(&owner.request.to_le_bytes())?;
                let owner_image = self.owners.reference_at(owner_header.slot, &[1])?;
                if owner_row.slot != owner_header.slot || owner_image.slot != owner_header.slot {
                    return Err(noncanonical_error());
                }
                let member = decode_arena_ref(&owner.source.to_le_bytes())?;
                let member_image = self.members.image(member, &[1])?;
                let funder = decode_arena_ref(&member_image[24..32])?;
                let publication = LifecyclePublication {
                    owner_slot: owner_header.slot,
                    funder,
                    branch,
                    axis,
                    horizon,
                    zero: 0,
                };
                let prior = &publications[..publication_count];
                let owner_delta = u64::try_from(
                    prior
                        .iter()
                        .filter(|candidate| candidate.owner_slot == owner_header.slot)
                        .count()
                        + 1,
                )
                .map_err(|_| capacity_error())?;
                let branch_delta = u64::try_from(
                    prior
                        .iter()
                        .filter(|candidate| {
                            candidate.owner_slot == owner_header.slot && candidate.branch == branch
                        })
                        .count()
                        + 1,
                )
                .map_err(|_| capacity_error())?;
                let funder_delta = u64::try_from(
                    prior
                        .iter()
                        .filter(|candidate| candidate.funder == funder)
                        .count()
                        + 1,
                )
                .map_err(|_| capacity_error())?;
                let row = self.owner_rows.image(owner_row, &[1])?;
                u64::from(read_u32(row, OWNER_ROW_LINKED_CLAIMS))
                    .checked_add(owner_delta)
                    .filter(|value| u32::try_from(*value).is_ok())
                    .ok_or_else(capacity_error)?;
                read_u64(row, OWNER_ROW_CURRENT)
                    .checked_add(owner_delta)
                    .ok_or_else(capacity_error)?;
                read_u64(row, OWNER_ROW_BRANCH_CURRENT + usize::from(branch) * 8)
                    .checked_add(branch_delta)
                    .ok_or_else(capacity_error)?;
                let funder_image = self.funders.image(funder, &[1])?;
                let funder_current = read_u64(funder_image, 112)
                    .checked_add(funder_delta)
                    .ok_or_else(capacity_error)?;
                if funder_current > read_u64(funder_image, 120) {
                    return Err(capacity_error());
                }
                publications[publication_count] = publication;
                publication_count += 1;
            }
        }
        Ok((publications, publication_count))
    }

    pub(super) fn validate_lifecycle_publication_record(
        &self,
        publication: LifecyclePublication,
        record: &BundleRecord,
    ) -> Result<(), SupportLedgerError> {
        let slot = publication.owner_slot;
        let references = [
            self.owner_headers.reference_at(slot, &[1])?,
            self.owner_rows.reference_at(slot, &[1])?,
            self.owner_indices.reference_at(slot, &[1])?,
            self.owners.reference_at(slot, &[1])?,
        ];
        validate_c16_owner_set(
            [
                self.owner_headers.image(references[0], &[1])?.as_slice(),
                self.owner_rows.image(references[1], &[1])?.as_slice(),
                self.owner_indices.image(references[2], &[1])?.as_slice(),
                self.owners.image(references[3], &[1])?.as_slice(),
            ],
            references,
            slot,
            record,
            OWNER_STATE_LIVE,
        )
    }

    fn validate_finalizable(
        &self,
        expected_support: SupportLedgerGeneration,
    ) -> Result<(), SupportLedgerError> {
        if self.pending_state()? != PendingState::Staging {
            return Err(SupportLedgerError::InvalidTransition);
        }
        let total = usize::from(read_u16(&self.pending.0, PENDING_TOTAL));
        let before_support = read_u64(&self.pending.0, PENDING_BEFORE_SUPPORT);
        if total == 0
            || usize::from(read_u16(&self.pending.0, PENDING_STAGED)) != total
            || expected_support.get() < before_support
            || read_u64(&self.pending.0, PENDING_EXPECTED_RAW) != self.raw.generation()
        {
            return Err(SupportLedgerError::InvalidTransition);
        }
        let batch = read_u64(&self.pending.0, PENDING_BATCH);
        let target = self.pending_aggregate()?;
        let mut recomputed = LifecycleAggregate::ZERO;
        let mut previous_obligation = None;
        let mut previous_credit = None;
        for ordinal in 0..total {
            let reference = self.inactive_reference(ordinal)?;
            let image = self.lifecycle.image(reference, &[3])?;
            if read_u64(image, 8) != batch
                || usize::from(read_u16(image, 16)) != ordinal
                || image[480..488]
                    != encode_raw_owner(
                        RawOwnerKind::LifecycleObligation,
                        RawOwnerState::Inactive,
                        reference,
                    )?
            {
                return Err(noncanonical_error());
            }
            let record = LifecycleRecordInput::decode(image)?;
            self.validate_lifecycle_record_owner_set(record, reference)?;
            if previous_obligation.is_some_and(|previous| previous >= record.obligation_raw)
                || previous_credit.is_some_and(|previous| previous >= record.credit_raw)
            {
                return Err(SupportLedgerError::InvalidInput);
            }
            previous_obligation = Some(record.obligation_raw);
            previous_credit = Some(record.credit_raw);
            recomputed.accrue(record)?;
            for key_offset in [96, 128] {
                let mut key = [0; 32];
                key.copy_from_slice(&image[key_offset..key_offset + 32]);
                let value = self
                    .raw
                    .find(&key)?
                    .ok_or(SupportLedgerError::InvalidTransition)?;
                let (_, state, owner) = decode_raw_owner(value)?;
                if state != RawOwnerState::Inactive || owner != reference {
                    return Err(noncanonical_error());
                }
            }
        }
        if recomputed != target {
            return Err(noncanonical_error());
        }
        let mut references = [ArenaRef::default(); LIFECYCLE_BATCH_MAX];
        for (ordinal, reference) in references[..total].iter_mut().enumerate() {
            *reference = self.inactive_reference(ordinal)?;
        }
        self.lifecycle
            .validate_commit_inactive_batch(&references[..total])?;
        Ok(())
    }

    fn pending_aggregate(&self) -> Result<LifecycleAggregate, SupportLedgerError> {
        LifecycleAggregate::decode(&self.pending.0[PENDING_AGGREGATE..PENDING_BEFORE_SUPPORT])
    }

    fn validate_pending_header(&self, state: PendingState) -> Result<(), SupportLedgerError> {
        let total = usize::from(read_u16(&self.pending.0, PENDING_TOTAL));
        let staged = usize::from(read_u16(&self.pending.0, PENDING_STAGED));
        let cursor = usize::from(read_u16(&self.pending.0, PENDING_CURSOR));
        let reserved = usize::from(read_u16(&self.pending.0, PENDING_RESERVED));
        if !(1..=LIFECYCLE_BATCH_MAX).contains(&total)
            || staged > total
            || reserved != total
            || cursor > total
            || (state == PendingState::Staging && cursor != total)
            || (state == PendingState::Aborting && cursor == 0)
            || self.pending.0[PENDING_SLOTS + total * 2..PENDING_AGGREGATE]
                .iter()
                .any(|byte| *byte != 0)
            || self.pending.0[PENDING_WITHHELD + 32..PENDING_WITHHELD + 64]
                .iter()
                .any(|byte| *byte != 0)
            || self.pending.0[PENDING_WITHHELD + 64..]
                .iter()
                .any(|byte| *byte != 0)
            || read_u64(&self.pending.0, PENDING_BEFORE_SUPPORT) == 0
        {
            return Err(noncanonical_error());
        }
        let aggregate = self.pending_aggregate()?;
        if aggregate == LifecycleAggregate::ZERO {
            return Err(noncanonical_error());
        }
        for (index, expected) in aggregate.withholding()?.into_iter().enumerate() {
            if read_u64(&self.pending.0, PENDING_WITHHELD + index * 8) != expected {
                return Err(noncanonical_error());
            }
        }
        let mut slots = [0u16; LIFECYCLE_BATCH_MAX];
        for ordinal in 0..total {
            let slot = read_u16(&self.pending.0, PENDING_SLOTS + ordinal * 2);
            if usize::from(slot) >= self.lifecycle.capacity() || slots[..ordinal].contains(&slot) {
                return Err(noncanonical_error());
            }
            slots[ordinal] = slot;
        }
        Ok(())
    }

    fn pending_state(&self) -> Result<PendingState, SupportLedgerError> {
        if self.pending.0[1..8].iter().any(|byte| *byte != 0) {
            return Err(noncanonical_error());
        }
        match self.pending.0[PENDING_STATE] {
            0 => {
                let batch = read_u64(&self.pending.0, PENDING_BATCH);
                let mut canonical = PendingLifecycleHeaderImage::ZERO;
                write_u64(&mut canonical.0, PENDING_BATCH, batch);
                (canonical == self.pending)
                    .then_some(PendingState::Empty)
                    .ok_or_else(noncanonical_error)
            }
            1 => {
                self.validate_pending_header(PendingState::Staging)?;
                Ok(PendingState::Staging)
            }
            2 => {
                self.validate_pending_header(PendingState::Aborting)?;
                Ok(PendingState::Aborting)
            }
            _ => Err(noncanonical_error()),
        }
    }

    fn reserved_reference(&self, ordinal: usize) -> Result<ArenaRef, SupportLedgerError> {
        let total = usize::from(read_u16(&self.pending.0, PENDING_TOTAL));
        if ordinal >= total {
            return Err(SupportLedgerError::InvalidTransition);
        }
        let slot = u32::from(read_u16(&self.pending.0, PENDING_SLOTS + ordinal * 2));
        Ok(self.lifecycle.reference_at(slot, &[2])?)
    }

    fn inactive_reference(&self, ordinal: usize) -> Result<ArenaRef, SupportLedgerError> {
        let total = usize::from(read_u16(&self.pending.0, PENDING_TOTAL));
        if ordinal >= total {
            return Err(SupportLedgerError::InvalidTransition);
        }
        let slot = u32::from(read_u16(&self.pending.0, PENDING_SLOTS + ordinal * 2));
        Ok(self.lifecycle.reference_at(slot, &[3])?)
    }

    fn reset_pending(&mut self, batch: u64) {
        self.pending.0.fill(0);
        write_u64(&mut self.pending.0, PENDING_BATCH, batch);
    }

    fn advance_generation(&mut self) {
        let next = self
            .generation()
            .checked_add(1)
            .expect("prepared C17 generation");
        write_u64(&mut self.header.0, 48, next);
        if matches!(
            self.pending.0[PENDING_STATE],
            value if value == PendingState::Staging as u8 || value == PendingState::Aborting as u8
        ) {
            write_u64(
                &mut self.pending.0,
                PENDING_EXPECTED_RAW,
                self.raw.generation(),
            );
        }
    }
}

const PLAN_BRANCHES: usize = 3;
const OWNER_FUNDING_BRANCHES: usize = 4;
const PLAN_MEMBERS_MAX: usize = 4;
const PLAN_FUNDER_ROWS: usize = PLAN_BRANCHES * PLAN_MEMBERS_MAX;
const PLAN_MEMBER_ROWS: usize = PLAN_BRANCHES * PLAN_MEMBERS_MAX;
const PLAN_RAW_EDITS: usize = PLAN_BRANCHES * 2;
const PLAN_LOCAL_EDITS: usize = 3 + PLAN_FUNDER_ROWS + PLAN_MEMBERS_MAX + 1;
const PLAN_RAW_ASSIGNMENT_MAX: usize = 9 * PLAN_RAW_EDITS + 1;
const PLAN_AUTHORITY_ASSIGNMENT_MAX: usize = 9 + 1;
const PLAN_LOCAL_ASSIGNMENT_MAX: usize = 9 * PLAN_LOCAL_EDITS + 1;
pub(super) const PLAN_IDENTITY_BYTES: usize = 212;

fn funding_branch(root_branch: u8) -> Result<u8, SupportLedgerError> {
    match root_branch {
        0..=2 => Ok(root_branch),
        3 | 4 => Ok((OWNER_FUNDING_BRANCHES - 1) as u8),
        _ => Err(noncanonical_error()),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct PlanCreateMember {
    pub(super) request: Option<RequestId>,
    pub(super) request_key: [u8; 40],
    pub(super) record_slot: u32,
    pub(super) owner_header: ArenaRef,
    pub(super) entitlement: [u8; 32],
    pub(super) vector: [u8; 32],
    pub(super) branch_limits: [u64; PLAN_BRANCHES],
}

impl PlanCreateMember {
    pub(super) const ZERO: Self = Self {
        request: None,
        request_key: [0; 40],
        record_slot: 0,
        owner_header: ArenaRef {
            slot: 0,
            generation: 0,
        },
        entitlement: [0; 32],
        vector: [0; 32],
        branch_limits: [0; PLAN_BRANCHES],
    };

    fn active(self) -> bool {
        self.request.is_some()
            && self.request_key != [0; 40]
            && self.owner_header.generation != 0
            && self.entitlement != [0; 32]
            && self.vector != [0; 32]
            && self.branch_limits.into_iter().all(|limit| limit > 0)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct PlanCreateInput {
    pub(super) authority_key: [u8; 17],
    pub(super) identity: [u8; PLAN_IDENTITY_BYTES],
    pub(super) obligations: [[u8; 32]; PLAN_BRANCHES],
    pub(super) credits: [[u8; 32]; PLAN_BRANCHES],
    pub(super) members: [PlanCreateMember; PLAN_MEMBERS_MAX],
    pub(super) member_count: usize,
    pub(super) occurred_at: u64,
}

impl PlanCreateInput {
    fn validate(self) -> Result<(), SupportLedgerError> {
        if self.authority_key[0] != 0x30
            || self.authority_key[1..] == [0; 16]
            || self.identity == [0; PLAN_IDENTITY_BYTES]
            || !(1..=PLAN_MEMBERS_MAX).contains(&self.member_count)
            || self.occurred_at == 0
            || self.obligations.contains(&[0; 32])
            || self.credits.contains(&[0; 32])
            || self.members[..self.member_count]
                .iter()
                .any(|member| !member.active())
            || self.members[self.member_count..]
                .iter()
                .any(|member| *member != PlanCreateMember::ZERO)
        {
            return Err(SupportLedgerError::InvalidInput);
        }
        for index in 0..self.member_count {
            let member = self.members[index];
            if self.members[..index].iter().any(|prior| {
                prior.request_key >= member.request_key
                    || prior.owner_header == member.owner_header
                    || prior.entitlement == member.entitlement
                    || prior.vector == member.vector
            }) {
                return Err(SupportLedgerError::InvalidInput);
            }
        }
        Ok(())
    }
}

#[derive(Debug)]
pub(super) struct PreparedPlanCreate {
    expected_c17: u64,
    expected_raw: u64,
    expected_authority: u64,
    expected_local: u64,
    expected_arena_headers: [ByteArenaHeaderImage; 11],
    arena_headers_after: [ByteArenaHeaderImage; 11],
    input: PlanCreateInput,
    groups: ArenaSelection<PLAN_BRANCHES>,
    external_heads: ArenaSelection<PLAN_BRANCHES>,
    formations: ArenaSelection<PLAN_BRANCHES>,
    funders: ArenaSelection<PLAN_FUNDER_ROWS>,
    members: ArenaSelection<PLAN_MEMBER_ROWS>,
    links: ArenaSelection<PLAN_MEMBERS_MAX>,
    mutations: ArenaSelection<1>,
    authority_entries: [([u8; 17], [u8; 8]); 1],
    raw_entries: [([u8; 32], [u8; 8]); PLAN_RAW_EDITS],
    local_entries: [([u8; 17], [u8; 8]); PLAN_LOCAL_EDITS],
    local_count: usize,
    group_images: [[u8; GROUP_BYTES]; PLAN_BRANCHES],
    head_images: [[u8; EXTERNAL_HEAD_BYTES]; PLAN_BRANCHES],
    formation_images: [[u8; FORMATION_BYTES]; PLAN_BRANCHES],
    funder_images: [[u8; FUNDER_BYTES]; PLAN_FUNDER_ROWS],
    member_images: [[u8; MEMBER_BYTES]; PLAN_MEMBER_ROWS],
    link_images: [[u8; LINK_BYTES]; PLAN_MEMBERS_MAX],
    mutation_image: [u8; MUTATION_BYTES],
    owner_references: [[ArenaRef; 4]; PLAN_MEMBERS_MAX],
    owner_row_images: [[u8; OWNER_ROW_BYTES]; PLAN_MEMBERS_MAX],
    owner_images: [[u8; OWNER_BYTES]; PLAN_MEMBERS_MAX],
    header_after: C17HeaderImage,
    raw_plan: PatriciaAssignmentPlan<PLAN_RAW_ASSIGNMENT_MAX>,
    authority_plan: PatriciaAssignmentPlan<PLAN_AUTHORITY_ASSIGNMENT_MAX>,
    local_plan: PatriciaAssignmentPlan<PLAN_LOCAL_ASSIGNMENT_MAX>,
}

impl PreparedPlanCreate {
    pub(crate) fn visit_assignments(
        &self,
        visitor: &mut dyn FnMut(AssignmentOrderKey, Assignment),
    ) {
        self.raw_plan.visit_assignments(visitor);
        self.authority_plan.visit_assignments(visitor);
        self.local_plan.visit_assignments(visitor);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
enum LocalKind {
    Group = 1,
    Funder = 2,
    Link = 3,
    Membership = 4,
    Mutation = 5,
}

fn local_key(kind: LocalKind, reference: ArenaRef) -> [u8; 17] {
    let mut key = [0; 17];
    key[0] = kind as u8;
    let identity = (u128::from(reference.generation) << 32) | (u128::from(reference.slot) + 1);
    key[1..].copy_from_slice(&identity.to_be_bytes());
    key
}

fn encode_arena_ref_value(reference: ArenaRef) -> [u8; 8] {
    let mut value = [0; 8];
    encode_arena_ref(&mut value, reference);
    value
}

fn encode_plan_group(
    branch: usize,
    member_count: usize,
    authority_key: [u8; 17],
    formation: ArenaRef,
    head: ArenaRef,
    members: [ArenaRef; PLAN_MEMBERS_MAX],
    siblings: [ArenaRef; PLAN_BRANCHES],
) -> [u8; GROUP_BYTES] {
    let mut image = GroupImage::ZERO.0;
    image[8] = branch as u8;
    image[9] = RootState::Conditional as u8;
    image[10] = member_count as u8;
    image[11] = 1;
    encode_arena_ref(&mut image[16..24], formation);
    encode_arena_ref(&mut image[24..32], head);
    write_u64(&mut image, 32, 1);
    image[40..57].copy_from_slice(&authority_key);
    for (ordinal, reference) in members.into_iter().enumerate() {
        encode_arena_ref(&mut image[64 + ordinal * 8..72 + ordinal * 8], reference);
    }
    for (ordinal, reference) in siblings.into_iter().enumerate() {
        encode_arena_ref(&mut image[96 + ordinal * 8..104 + ordinal * 8], reference);
    }
    image
}

fn encode_plan_formation(
    identity: [u8; PLAN_IDENTITY_BYTES],
    branch: usize,
    group: ArenaRef,
    head: ArenaRef,
    occurred_at: u64,
) -> [u8; FORMATION_BYTES] {
    let mut image = FormationImage::ZERO.0;
    image[8..8 + PLAN_IDENTITY_BYTES].copy_from_slice(&identity);
    image[220] = branch as u8;
    image[221] = RootState::Conditional as u8;
    image[222] = FormationCause::Plan as u8;
    image[223] = 0;
    write_u64(&mut image, 224, 1);
    write_u64(&mut image, 232, occurred_at);
    encode_arena_ref(&mut image[240..248], group);
    encode_arena_ref(&mut image[248..256], head);
    image
}

fn encode_plan_head(
    branch: usize,
    member_count: usize,
    group: ArenaRef,
    formation: ArenaRef,
    obligation: [u8; 32],
    credit: [u8; 32],
    authority_key: [u8; 17],
) -> [u8; EXTERNAL_HEAD_BYTES] {
    let mut image = ExternalHeadImage::ZERO.0;
    image[8] = branch as u8;
    image[9] = RootState::Conditional as u8;
    image[10] = member_count as u8;
    encode_arena_ref(&mut image[16..24], group);
    encode_arena_ref(&mut image[24..32], formation);
    image[32..64].copy_from_slice(&obligation);
    image[64..96].copy_from_slice(&credit);
    image[96..113].copy_from_slice(&authority_key);
    write_u64(&mut image, 120, 1);
    image
}

fn encode_plan_funder(
    branch: usize,
    ordinal: usize,
    active: bool,
    group: ArenaRef,
    formation: ArenaRef,
    member: ArenaRef,
    input: PlanCreateMember,
) -> [u8; FUNDER_BYTES] {
    let mut image = FunderImage::ZERO.0;
    image[8] = u8::from(active);
    image[9] = branch as u8;
    image[10] = 1;
    image[11] = ordinal as u8;
    encode_arena_ref(&mut image[16..24], group);
    encode_arena_ref(&mut image[24..32], formation);
    encode_arena_ref(&mut image[32..40], member);
    if active {
        encode_arena_ref(&mut image[40..48], input.owner_header);
        image[48..80].copy_from_slice(&input.entitlement);
        image[80..112].copy_from_slice(&input.vector);
        write_u64(&mut image, 112, 1);
        write_u64(&mut image, 120, input.branch_limits[branch]);
    }
    image
}

fn encode_plan_member(
    branch: usize,
    ordinal: usize,
    active: bool,
    group: ArenaRef,
    funder: ArenaRef,
    input: PlanCreateMember,
) -> [u8; MEMBER_BYTES] {
    let mut image = MemberImage::ZERO.0;
    image[8] = u8::from(active);
    image[9] = branch as u8;
    image[10] = ordinal as u8;
    encode_arena_ref(&mut image[16..24], group);
    encode_arena_ref(&mut image[24..32], funder);
    if active {
        image[32..72].copy_from_slice(&input.request_key);
        encode_arena_ref(&mut image[72..80], input.owner_header);
        image[80..112].copy_from_slice(&input.entitlement);
    }
    image
}

fn encode_plan_link(
    owner: ArenaRef,
    group: ArenaRef,
    formation: ArenaRef,
    authority_key: [u8; 17],
    generation: u64,
) -> [u8; LINK_BYTES] {
    let mut image = LinkImage::ZERO.0;
    image[8] = 1;
    image[9] = 1;
    encode_arena_ref(&mut image[16..24], owner);
    encode_arena_ref(&mut image[24..32], group);
    encode_arena_ref(&mut image[32..40], formation);
    write_u64(&mut image, 48, generation);
    image[56..73].copy_from_slice(&authority_key);
    image
}

fn encode_plan_mutation(
    authority_key: [u8; 17],
    groups: [ArenaRef; PLAN_BRANCHES],
    formations: [ArenaRef; PLAN_BRANCHES],
    generation: u64,
    occurred_at: u64,
) -> [u8; MUTATION_BYTES] {
    let mut image = MutationImage::ZERO.0;
    image[8] = SemanticOperation::PlanCreateO as u8;
    image[9] = PLAN_BRANCHES as u8;
    write_u64(&mut image, 16, generation);
    write_u64(&mut image, 24, occurred_at);
    image[32..49].copy_from_slice(&authority_key);
    for (ordinal, reference) in groups.into_iter().enumerate() {
        encode_arena_ref(&mut image[56 + ordinal * 8..64 + ordinal * 8], reference);
    }
    for (ordinal, reference) in formations.into_iter().enumerate() {
        encode_arena_ref(&mut image[80 + ordinal * 8..88 + ordinal * 8], reference);
    }
    image
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub(crate) enum RootState {
    Conditional = 1,
    Pending = 2,
    Active = 3,
    Retained = 4,
    ClosedConditional = 5,
    ClosedPending = 6,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub(crate) enum FormationCause {
    Plan = 1,
    InitialReady = 2,
    MembershipConsumed = 3,
    Receipt = 4,
    Rejection = 5,
    LocalStale = 6,
    ObservationCompleted = 7,
    PredecessorEnded = 8,
    BeganSupport = 9,
    FinishedSupport = 10,
    TypedImpossible = 11,
    CancellationMembership = 12,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct C16OwnerSetImages {
    header: [u8; OWNER_HEADER_BYTES],
    row: [u8; OWNER_ROW_BYTES],
    index: [u8; OWNER_INDEX_BYTES],
    owner: [u8; OWNER_BYTES],
}

#[derive(Debug)]
pub(super) struct PreparedLegacyInsert {
    expected_c17: u64,
    expected_raw: u64,
    entries: [([u8; 32], [u8; 8]); LEGACY_RAW_EDIT_MAX],
    entry_count: usize,
}

#[derive(Debug)]
pub(super) struct PreparedLegacyUpdate {
    expected_c17: u64,
    expected_raw: u64,
    updates: [([u8; 32], NodeHandle, [u8; 8]); LEGACY_RAW_EDIT_MAX],
    update_count: usize,
}

#[derive(Debug)]
pub(super) struct PreparedLegacyInsertStream {
    expected_c17: u64,
    expected_raw: u64,
    record_count: usize,
}

#[derive(Debug)]
pub(super) struct PreparedLegacyUpdateStream {
    expected_c17: u64,
    expected_raw: u64,
    record_count: usize,
}

#[derive(Debug)]
pub(super) struct PreparedC16Bundle {
    expected_c17: u64,
    expected_raw: u64,
    expected_owner_arenas: [u64; 4],
    record_slot: u32,
    owner_headers: ArenaSelection<1>,
    owner_rows: ArenaSelection<1>,
    owner_indices: ArenaSelection<1>,
    owners: ArenaSelection<1>,
    raw_entries: [([u8; 32], [u8; 8]); C16_RAW_OWNERS],
    images: C16OwnerSetImages,
}

#[derive(Debug)]
pub(super) struct PreparedC16Withdrawal {
    expected_c17: u64,
    expected_raw: u64,
    expected_owner_arenas: [u64; 4],
    record_slot: u32,
    references: [ArenaRef; 4],
    raw_keys: [[u8; 32]; C16_RAW_OWNERS],
}

#[derive(Debug)]
pub(super) struct PreparedC16Tombstone {
    expected_c17: u64,
    expected_raw: u64,
    expected_owner_arenas: [u64; 4],
    record_slot: u32,
    references: [ArenaRef; 4],
    raw_updates: [([u8; 32], NodeHandle, [u8; 8]); C16_RAW_OWNERS],
    images: C16OwnerSetImages,
}

#[derive(Debug)]
pub(super) struct PreparedC16Touch {
    expected_c17: u64,
    expected_raw: u64,
    record_slot: u32,
    owner: ArenaRef,
    raw_updates: [([u8; 32], NodeHandle, [u8; 8]); C16_RAW_OWNERS],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct LifecycleAggregate {
    pub(crate) usage: [[u32; 3]; 5],
    pub(crate) reserved: [[u32; 3]; 5],
    pub(crate) attached: [[u32; 3]; 4],
    pub(crate) vector: [[u64; 3]; 21],
}

impl LifecycleAggregate {
    pub(crate) const ZERO: Self = Self {
        usage: [[0; 3]; 5],
        reserved: [[0; 3]; 5],
        attached: [[0; 3]; 4],
        vector: [[0; 3]; 21],
    };

    pub(crate) fn from_records(
        records: &[LifecycleRecordInput],
    ) -> Result<Self, SupportLedgerError> {
        let mut aggregate = Self::ZERO;
        for record in records.iter().copied() {
            aggregate.accrue(record)?;
        }
        Ok(aggregate)
    }

    fn accrue(&mut self, record: LifecycleRecordInput) -> Result<(), SupportLedgerError> {
        record.validate()?;
        let class = usize::try_from(record.aggregate[0]).map_err(|_| capacity_error())?;
        let pool = usize::try_from(record.aggregate[1]).map_err(|_| capacity_error())?;
        let axis = usize::try_from(record.aggregate[2]).map_err(|_| capacity_error())?;
        let horizon = usize::try_from(record.aggregate[3]).map_err(|_| capacity_error())?;
        let amount = u32::try_from(record.aggregate[4]).map_err(|_| capacity_error())?;
        let attached = amount.checked_sub(1).ok_or_else(noncanonical_error)?;
        let add = |cell: &mut u32, delta| {
            *cell = cell.checked_add(delta).ok_or_else(capacity_error)?;
            Ok::<(), SupportLedgerError>(())
        };
        add(&mut self.usage[class][pool], 1)?;
        add(&mut self.usage[3][pool], 1)?;
        add(&mut self.usage[4][pool], amount)?;
        add(&mut self.reserved[class][pool], amount)?;
        add(&mut self.reserved[3][pool], amount)?;
        add(&mut self.reserved[4][pool], amount)?;
        add(&mut self.attached[class][pool], attached)?;
        add(&mut self.attached[3][pool], attached)?;
        self.vector[axis][horizon] = self.vector[axis][horizon]
            .checked_add(u64::from(amount))
            .ok_or_else(capacity_error)?;
        Ok(())
    }

    fn encode(self) -> [u8; 672] {
        let mut image = [0; 672];
        let mut offset = 0;
        for rows in [self.usage, self.reserved] {
            for row in rows {
                for value in row {
                    write_u32(&mut image, offset, value);
                    offset += 4;
                }
            }
        }
        for row in self.attached {
            for value in row {
                write_u32(&mut image, offset, value);
                offset += 4;
            }
        }
        for row in self.vector {
            for value in row {
                write_u64(&mut image, offset, value);
                offset += 8;
            }
        }
        debug_assert_eq!(offset, image.len());
        image
    }

    fn decode(image: &[u8]) -> Result<Self, SupportLedgerError> {
        if image.len() != 672 {
            return Err(noncanonical_error());
        }
        let mut value = Self::ZERO;
        let mut offset = 0;
        for rows in [&mut value.usage, &mut value.reserved] {
            for row in rows {
                for cell in row {
                    *cell = read_u32(image, offset);
                    offset += 4;
                }
            }
        }
        for row in &mut value.attached {
            for cell in row {
                *cell = read_u32(image, offset);
                offset += 4;
            }
        }
        for row in &mut value.vector {
            for cell in row {
                *cell = read_u64(image, offset);
                offset += 8;
            }
        }
        (offset == image.len())
            .then_some(value)
            .ok_or_else(noncanonical_error)
    }

    fn withholding(self) -> Result<[u64; 4], SupportLedgerError> {
        let sum_u32 = |rows: &[[u32; 3]]| {
            rows.iter()
                .flatten()
                .try_fold(0u64, |total, value| total.checked_add(u64::from(*value)))
        };
        let vector = self
            .vector
            .iter()
            .flatten()
            .try_fold(0u64, |total, value| total.checked_add(*value));
        Ok([
            sum_u32(&self.usage).ok_or_else(capacity_error)?,
            sum_u32(&self.reserved).ok_or_else(capacity_error)?,
            sum_u32(&self.attached).ok_or_else(capacity_error)?,
            vector.ok_or_else(capacity_error)?,
        ])
    }
}

#[derive(Debug)]
pub(crate) struct PreparedLifecycleBegin {
    expected_c17: u64,
    generation_after: u64,
    expected_arena_header: ByteArenaHeaderImage,
    arena_header_after: ByteArenaHeaderImage,
    before_support: SupportLedgerGeneration,
    batch: u64,
    aggregate: LifecycleAggregate,
    selection: ArenaSelection<LIFECYCLE_BATCH_MAX>,
    pending_before: PendingLifecycleHeaderImage,
    pending_after: PendingLifecycleHeaderImage,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct LifecycleOwnerRow {
    pub(crate) owner: u64,
    pub(crate) request: u64,
    pub(crate) entitlement: u64,
    pub(crate) vector: u64,
    pub(crate) source: u64,
    pub(crate) group: u64,
    pub(crate) root: u64,
    pub(crate) formation: u64,
    pub(crate) link: u64,
    pub(crate) reserve: u64,
    pub(crate) class: u64,
    pub(crate) pool: u64,
    pub(crate) amount: u64,
    pub(crate) generation: u64,
    pub(crate) state: u64,
    pub(crate) zero: u64,
}

impl LifecycleOwnerRow {
    pub(crate) const ZERO: Self = Self {
        owner: 0,
        request: 0,
        entitlement: 0,
        vector: 0,
        source: 0,
        group: 0,
        root: 0,
        formation: 0,
        link: 0,
        reserve: 0,
        class: 0,
        pool: 0,
        amount: 0,
        generation: 0,
        state: 0,
        zero: 0,
    };

    const fn fields(self) -> [u64; 16] {
        [
            self.owner,
            self.request,
            self.entitlement,
            self.vector,
            self.source,
            self.group,
            self.root,
            self.formation,
            self.link,
            self.reserve,
            self.class,
            self.pool,
            self.amount,
            self.generation,
            self.state,
            self.zero,
        ]
    }

    const fn from_fields(fields: [u64; 16]) -> Self {
        Self {
            owner: fields[0],
            request: fields[1],
            entitlement: fields[2],
            vector: fields[3],
            source: fields[4],
            group: fields[5],
            root: fields[6],
            formation: fields[7],
            link: fields[8],
            reserve: fields[9],
            class: fields[10],
            pool: fields[11],
            amount: fields[12],
            generation: fields[13],
            state: fields[14],
            zero: fields[15],
        }
    }

    fn encode(self) -> [u8; 128] {
        let mut image = [0; 128];
        for (index, field) in self.fields().into_iter().enumerate() {
            write_u64(&mut image, index * 8, field);
        }
        image
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct LifecycleRecordInput {
    pub(crate) final_owner: [u8; 64],
    pub(crate) owner_set_ref: [u8; 8],
    pub(crate) obligation_raw: [u8; 32],
    pub(crate) credit_raw: [u8; 32],
    pub(crate) predecessor: [u8; 32],
    pub(crate) scope: [u8; 32],
    pub(crate) claim: [u8; 32],
    pub(crate) physical_credit: [u8; 32],
    pub(crate) kind: u8,
    pub(crate) occurred_at: u64,
    pub(crate) expires_at: Option<u64>,
    pub(crate) aggregate: [u64; 21],
    pub(crate) owners: [LifecycleOwnerRow; 4],
}

impl LifecycleRecordInput {
    pub(crate) const ZERO: Self = Self {
        final_owner: [0; 64],
        owner_set_ref: [0; 8],
        obligation_raw: [0; 32],
        credit_raw: [0; 32],
        predecessor: [0; 32],
        scope: [0; 32],
        claim: [0; 32],
        physical_credit: [0; 32],
        kind: 0,
        occurred_at: 0,
        expires_at: None,
        aggregate: [0; 21],
        owners: [LifecycleOwnerRow::ZERO; 4],
    };

    fn validate(self) -> Result<(), SupportLedgerError> {
        let class = usize::try_from(self.aggregate[0]).map_err(|_| capacity_error())?;
        let pool = usize::try_from(self.aggregate[1]).map_err(|_| capacity_error())?;
        let axis = usize::try_from(self.aggregate[2]).map_err(|_| capacity_error())?;
        let horizon = usize::try_from(self.aggregate[3]).map_err(|_| capacity_error())?;
        let amount = self.aggregate[4];
        if self.final_owner == [0; 64]
            || self.owner_set_ref == [0; 8]
            || self.obligation_raw == [0; 32]
            || self.credit_raw == [0; 32]
            || self.obligation_raw >= self.credit_raw
            || [
                self.predecessor,
                self.scope,
                self.claim,
                self.physical_credit,
            ]
            .contains(&[0; 32])
            || self.kind == 0
            || self.occurred_at == 0
            || self
                .expires_at
                .is_some_and(|expiry| expiry <= self.occurred_at)
            || class >= 3
            || pool >= 3
            || axis >= 21
            || horizon >= 3
            || amount == 0
            || u32::try_from(amount).is_err()
            || self.aggregate[5] != 1
            || self.aggregate[6..].iter().any(|field| *field != 0)
        {
            return Err(SupportLedgerError::InvalidInput);
        }
        let mut owner_count = 0usize;
        let mut owner_amount = 0u64;
        let mut inactive = false;
        for owner in self.owners {
            if owner == LifecycleOwnerRow::ZERO {
                inactive = true;
                continue;
            }
            let fields = owner.fields();
            if inactive
                || fields[..10].contains(&0)
                || owner.class != class as u64
                || owner.pool != pool as u64
                || owner.amount != 1
                || owner.generation == 0
                || owner.state == 0
                || owner.zero != 0
            {
                return Err(SupportLedgerError::InvalidInput);
            }
            owner_amount = owner_amount
                .checked_add(owner.amount)
                .ok_or_else(capacity_error)?;
            owner_count += 1;
        }
        if !(1..=4).contains(&owner_count) || owner_count as u64 != amount || owner_amount != amount
        {
            return Err(SupportLedgerError::InvalidInput);
        }
        Ok(())
    }

    fn decode(image: &[u8; LIFECYCLE_BYTES]) -> Result<Self, SupportLedgerError> {
        if image[18..24].iter().any(|byte| *byte != 0)
            || image[488..512].iter().any(|byte| *byte != 0)
            || image[1_024..].iter().any(|byte| *byte != 0)
        {
            return Err(noncanonical_error());
        }
        let mut final_owner = [0; 64];
        final_owner.copy_from_slice(&image[24..88]);
        let mut owner_set_ref = [0; 8];
        owner_set_ref.copy_from_slice(&image[88..96]);
        let mut obligation_raw = [0; 32];
        obligation_raw.copy_from_slice(&image[96..128]);
        let mut credit_raw = [0; 32];
        credit_raw.copy_from_slice(&image[128..160]);
        let mut identities = [[0; 32]; 4];
        for (index, value) in identities.iter_mut().enumerate() {
            value.copy_from_slice(&image[160 + index * 32..192 + index * 32]);
        }
        let kind = u8::try_from(read_u64(image, 288)).map_err(|_| noncanonical_error())?;
        let expiry = read_u64(image, 304);
        let mut aggregate = [0; 21];
        for (index, value) in aggregate.iter_mut().enumerate() {
            *value = read_u64(image, 312 + index * 8);
        }
        let mut owners = [LifecycleOwnerRow::ZERO; 4];
        for (owner_index, owner) in owners.iter_mut().enumerate() {
            let mut fields = [0; 16];
            for (field_index, field) in fields.iter_mut().enumerate() {
                *field = read_u64(image, 512 + owner_index * 128 + field_index * 8);
            }
            *owner = LifecycleOwnerRow::from_fields(fields);
        }
        let value = Self {
            final_owner,
            owner_set_ref,
            obligation_raw,
            credit_raw,
            predecessor: identities[0],
            scope: identities[1],
            claim: identities[2],
            physical_credit: identities[3],
            kind,
            occurred_at: read_u64(image, 296),
            expires_at: (expiry != 0).then_some(expiry),
            aggregate,
            owners,
        };
        value.validate()?;
        Ok(value)
    }

    fn encode(
        self,
        reference: ArenaRef,
        batch: u64,
        ordinal: usize,
    ) -> Result<[u8; LIFECYCLE_BYTES], SupportLedgerError> {
        self.validate()?;
        let mut image = LifecycleRecordSlotImage::ZERO.0;
        write_u64(&mut image, 8, batch);
        write_u16(&mut image, 16, ordinal as u16);
        image[24..88].copy_from_slice(&self.final_owner);
        image[88..96].copy_from_slice(&self.owner_set_ref);
        image[96..128].copy_from_slice(&self.obligation_raw);
        image[128..160].copy_from_slice(&self.credit_raw);
        for (offset, value) in [
            (160, self.predecessor),
            (192, self.scope),
            (224, self.claim),
            (256, self.physical_credit),
        ] {
            image[offset..offset + 32].copy_from_slice(&value);
        }
        write_u64(&mut image, 288, u64::from(self.kind));
        write_u64(&mut image, 296, self.occurred_at);
        write_u64(&mut image, 304, self.expires_at.unwrap_or(0));
        for (index, value) in self.aggregate.into_iter().enumerate() {
            write_u64(&mut image, 312 + index * 8, value);
        }
        for (index, owner) in self.owners.into_iter().enumerate() {
            image[512 + index * 128..640 + index * 128].copy_from_slice(&owner.encode());
        }
        let encoded = encode_raw_owner(
            RawOwnerKind::LifecycleObligation,
            RawOwnerState::Inactive,
            reference,
        )?;
        image[480..488].copy_from_slice(&encoded);
        Ok(image)
    }
}

#[derive(Debug)]
pub(crate) struct PreparedLifecycleStage {
    expected_c17: u64,
    generation_after: u64,
    expected_raw: u64,
    expected_arena_header: ByteArenaHeaderImage,
    arena_header_after: ByteArenaHeaderImage,
    batch: u64,
    first: usize,
    len: usize,
    records: [LifecycleRecordInput; LIFECYCLE_CHUNK_MAX],
    record_images: [[u8; LIFECYCLE_BYTES]; LIFECYCLE_CHUNK_MAX],
    references: [ArenaRef; LIFECYCLE_CHUNK_MAX],
    raw_entries: [([u8; 32], [u8; 8]); 2 * LIFECYCLE_CHUNK_MAX],
    raw_count: usize,
    pending_after: PendingLifecycleHeaderImage,
    raw_plan: PatriciaAssignmentPlan<RAW_ASSIGNMENT_MAX>,
}

impl PreparedLifecycleStage {
    pub(crate) fn visit_assignments(
        &self,
        visitor: &mut dyn FnMut(AssignmentOrderKey, Assignment),
    ) {
        self.raw_plan.visit_assignments(visitor);
    }
}

pub(super) const LIFECYCLE_PUBLICATION_MAX: usize = LIFECYCLE_BATCH_MAX * PLAN_MEMBERS_MAX;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct LifecyclePublication {
    owner_slot: u32,
    funder: ArenaRef,
    branch: u8,
    axis: u8,
    horizon: u8,
    zero: u8,
}

impl LifecyclePublication {
    const ZERO: Self = Self {
        owner_slot: 0,
        funder: ArenaRef {
            slot: 0,
            generation: 0,
        },
        branch: 0,
        axis: 0,
        horizon: 0,
        zero: 0,
    };

    pub(super) const fn owner_slot(self) -> u32 {
        self.owner_slot
    }

    pub(super) const fn funder(self) -> ArenaRef {
        self.funder
    }

    pub(super) const fn branch(self) -> usize {
        self.branch as usize
    }

    pub(super) const fn axis(self) -> usize {
        self.axis as usize
    }

    pub(super) const fn horizon(self) -> usize {
        self.horizon as usize
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct LifecycleRawUpdate {
    handle: NodeHandle,
    after: [u8; 8],
}

impl LifecycleRawUpdate {
    const ZERO: Self = Self {
        handle: NodeHandle::SENTINEL,
        after: [0; 8],
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct LifecycleOwnerOutcome {
    owner_slot: u32,
    linked_after: u32,
    current_after: u64,
    branches_after: [u64; OWNER_FUNDING_BRANCHES],
}

impl LifecycleOwnerOutcome {
    pub(super) const ZERO: Self = Self {
        owner_slot: 0,
        linked_after: 0,
        current_after: 0,
        branches_after: [0; OWNER_FUNDING_BRANCHES],
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct LifecycleFunderOutcome {
    reference: ArenaRef,
    current_after: u64,
}

impl LifecycleFunderOutcome {
    pub(super) const ZERO: Self = Self {
        reference: ArenaRef {
            slot: 0,
            generation: 0,
        },
        current_after: 0,
    };
}

#[derive(Debug)]
pub(crate) struct PreparedLifecycleFinalize {
    expected_c17: u64,
    generation_after: u64,
    expected_raw: u64,
    expected_arena_header: ByteArenaHeaderImage,
    arena_header_after: ByteArenaHeaderImage,
    expected_publication_arena_headers: [ByteArenaHeaderImage; 3],
    publication_arena_headers_after: [ByteArenaHeaderImage; 3],
    batch: u64,
    total: u16,
    expected_support: SupportLedgerGeneration,
    aggregate: LifecycleAggregate,
    pending_before: PendingLifecycleHeaderImage,
    pending_after: PendingLifecycleHeaderImage,
    references: [ArenaRef; LIFECYCLE_BATCH_MAX],
    occupied_tags_after: [u8; LIFECYCLE_BATCH_MAX],
    raw_updates: [LifecycleRawUpdate; 2 * LIFECYCLE_BATCH_MAX],
    raw_generation_plan: PatriciaAssignmentPlan<RAW_GENERATION_ASSIGNMENTS>,
    publication_count: usize,
}

impl PreparedLifecycleFinalize {
    pub(crate) const fn aggregate(&self) -> LifecycleAggregate {
        self.aggregate
    }

    #[cfg(test)]
    pub(crate) fn raw_generation_assignment_for_test(&self) -> Assignment {
        assert_eq!(self.raw_generation_plan.assignments().len(), 1);
        self.raw_generation_plan.assignments()[0]
    }

    #[cfg(test)]
    pub(crate) fn corrupt_raw_generation_assignment_for_test(&mut self) {
        self.raw_generation_plan
            .corrupt_first_assignment_payload_for_test();
    }
}

#[derive(Debug)]
pub(crate) struct PreparedLifecycleAbort {
    expected_c17: u64,
    generation_after: u64,
    expected_raw: u64,
    expected_arena_header: ByteArenaHeaderImage,
    arena_header_after: ByteArenaHeaderImage,
    batch: u64,
    first: usize,
    len: usize,
    references: [ArenaRef; LIFECYCLE_CHUNK_MAX],
    free_positions: [u32; LIFECYCLE_CHUNK_MAX],
    vacant_images: [[u8; LIFECYCLE_BYTES]; LIFECYCLE_CHUNK_MAX],
    free_cell_images: [ByteArenaFreeCellImage; LIFECYCLE_CHUNK_MAX],
    raw_keys: [[u8; 32]; 2 * LIFECYCLE_CHUNK_MAX],
    raw_count: usize,
    pending_after: PendingLifecycleHeaderImage,
    raw_plan: PatriciaAssignmentPlan<RAW_ASSIGNMENT_MAX>,
}

impl PreparedLifecycleAbort {
    pub(crate) fn visit_assignments(
        &self,
        visitor: &mut dyn FnMut(AssignmentOrderKey, Assignment),
    ) {
        self.raw_plan.visit_assignments(visitor);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
enum RawOwnerKind {
    LegacyObligation = 1,
    LegacyCredit = 2,
    C16Bundle = 3,
    C16Entitlement = 4,
    C16Claim = 5,
    PlanRoot = 6,
    Formation = 7,
    Funder = 8,
    LifecycleObligation = 9,
    LifecycleCredit = 10,
    Tombstone = 11,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
enum RawOwnerState {
    Inactive = 1,
    Committed = 2,
    Retained = 3,
    Tombstone = 4,
}

fn legacy_insert_stream<F>(
    record_count: usize,
    record: F,
) -> impl Iterator<Item = ([u8; 32], [u8; 8])>
where
    F: Fn(usize) -> (usize, [u8; 32], [u8; 32]) + Copy,
{
    let mut obligation = 0usize;
    let mut credit = 0usize;
    std::iter::from_fn(move || {
        if obligation == record_count && credit == record_count {
            return None;
        }
        let take_obligation = if obligation == record_count {
            false
        } else if credit == record_count {
            true
        } else {
            record(obligation).1 <= record(credit).2
        };
        let index = if take_obligation {
            let index = obligation;
            obligation += 1;
            index
        } else {
            let index = credit;
            credit += 1;
            index
        };
        let (slot, obligation_key, credit_key) = record(index);
        let owner = ArenaRef {
            slot: u32::try_from(slot).expect("prevalidated legacy slot"),
            generation: 1,
        };
        let (key, kind) = if take_obligation {
            (obligation_key, RawOwnerKind::LegacyObligation)
        } else {
            (credit_key, RawOwnerKind::LegacyCredit)
        };
        Some((
            key,
            encode_raw_owner(kind, RawOwnerState::Committed, owner)
                .expect("prevalidated legacy Raw owner"),
        ))
    })
}

fn legacy_update_stream<F>(
    record_count: usize,
    record: F,
) -> impl Iterator<Item = ([u8; 32], [u8; 8])>
where
    F: Fn(usize) -> (usize, [u8; 32], [u8; 32], bool) + Copy,
{
    let mut obligation = 0usize;
    let mut credit = 0usize;
    std::iter::from_fn(move || {
        if obligation == record_count && credit == record_count {
            return None;
        }
        let take_obligation = if obligation == record_count {
            false
        } else if credit == record_count {
            true
        } else {
            record(obligation).1 <= record(credit).2
        };
        let index = if take_obligation {
            let index = obligation;
            obligation += 1;
            index
        } else {
            let index = credit;
            credit += 1;
            index
        };
        let (slot, obligation_key, credit_key, retained) = record(index);
        let owner = ArenaRef {
            slot: u32::try_from(slot).expect("prevalidated legacy slot"),
            generation: 1,
        };
        let (key, kind) = if take_obligation {
            (obligation_key, RawOwnerKind::LegacyObligation)
        } else {
            (credit_key, RawOwnerKind::LegacyCredit)
        };
        let state = if retained {
            RawOwnerState::Retained
        } else {
            RawOwnerState::Committed
        };
        Some((
            key,
            encode_raw_owner(kind, state, owner).expect("prevalidated legacy Raw owner"),
        ))
    })
}

fn c16_raw_kind(ordinal: usize) -> Result<RawOwnerKind, SupportLedgerError> {
    match ordinal {
        0..=5 => Ok(RawOwnerKind::C16Bundle),
        6..=8 => Ok(RawOwnerKind::C16Claim),
        9..=10 => Ok(RawOwnerKind::C16Entitlement),
        _ => Err(noncanonical_error()),
    }
}

fn encode_c16_owner_set(
    record_slot: u32,
    owner_generation: u64,
    references: [ArenaRef; 4],
    record: &BundleRecord,
    cells: &[OutstandingCreditCell],
) -> Result<C16OwnerSetImages, SupportLedgerError> {
    let vector_len = u16::try_from(cells.len()).map_err(|_| capacity_error())?;
    if cells.is_empty()
        || cells.len() != record.vector_len as usize
        || owner_generation == 0
        || references.iter().any(|reference| reference.generation == 0)
    {
        return Err(SupportLedgerError::InvalidInput);
    }
    let mut header = OwnerHeaderImage::ZERO.0;
    header[8] = OWNER_STATE_LIVE;
    header[9] = C16_RAW_OWNERS as u8;
    write_u32(&mut header, OWNER_HEADER_RECORD, record_slot);
    write_u64(&mut header, OWNER_HEADER_GENERATION, owner_generation);
    header[OWNER_HEADER_REQUEST..OWNER_HEADER_REQUEST + 40]
        .copy_from_slice(&crate::request_book::c17::request_key(record.request_owner));
    header[OWNER_HEADER_ENTITLEMENT..OWNER_HEADER_ENTITLEMENT + 32]
        .copy_from_slice(&record.entitlement.get());
    header[OWNER_HEADER_VECTOR..OWNER_HEADER_VECTOR + 32].copy_from_slice(&record.vector.get());

    let mut row = OwnerRowImage::ZERO.0;
    row[8] = OWNER_STATE_LIVE;
    row[9] = C16_RAW_OWNERS as u8;
    write_u16(&mut row, OWNER_ROW_VECTOR_LEN, vector_len);
    write_u32(&mut row, OWNER_ROW_RECORD, record_slot);
    write_u64(&mut row, OWNER_ROW_GENERATION, owner_generation);

    let mut index = OwnerIndexImage::ZERO.0;
    for (ordinal, reference) in references.into_iter().enumerate() {
        encode_arena_ref(&mut index[8 + ordinal * 8..16 + ordinal * 8], reference);
    }
    write_u32(&mut index, 40, record_slot);
    write_u64(&mut index, 48, owner_generation);
    write_u16(&mut index, 56, vector_len);
    index[OWNER_INDEX_STATE] = OWNER_STATE_LIVE;
    index[59] = C16_RAW_OWNERS as u8;

    let mut owner = OwnerImage::ZERO.0;
    owner[OWNER_IMAGE_STATE] = OWNER_STATE_LIVE;
    owner[9] = C16_RAW_OWNERS as u8;
    write_u16(&mut owner, 10, vector_len);
    write_u32(&mut owner, OWNER_IMAGE_RECORD, record_slot);
    write_u16(&mut owner, OWNER_IMAGE_VECTOR_LEN, vector_len);
    for (ordinal, reference) in references.into_iter().enumerate() {
        encode_arena_ref(&mut owner[32 + ordinal * 8..40 + ordinal * 8], reference);
    }
    owner[64..96].copy_from_slice(&record.entitlement.get());
    owner[96..128].copy_from_slice(&record.vector.get());
    Ok(C16OwnerSetImages {
        header,
        row,
        index,
        owner,
    })
}

fn tombstone_owner_images(mut images: C16OwnerSetImages) -> C16OwnerSetImages {
    images.header[8] = OWNER_STATE_TOMBSTONE;
    images.row[8] = OWNER_STATE_TOMBSTONE;
    images.index[OWNER_INDEX_STATE] = OWNER_STATE_TOMBSTONE;
    images.owner[OWNER_IMAGE_STATE] = OWNER_STATE_TOMBSTONE;
    images
}

fn validate_c16_owner_set(
    images: [&[u8]; 4],
    references: [ArenaRef; 4],
    record_slot: u32,
    record: &BundleRecord,
    expected_state: u8,
) -> Result<(), SupportLedgerError> {
    let [header, row, index, owner] = images;
    let vector_len = u16::try_from(record.vector_len).map_err(|_| noncanonical_error())?;
    if !matches!(expected_state, OWNER_STATE_LIVE | OWNER_STATE_TOMBSTONE)
        || header.len() != OWNER_HEADER_BYTES
        || row.len() != OWNER_ROW_BYTES
        || index.len() != OWNER_INDEX_BYTES
        || owner.len() != OWNER_BYTES
    {
        return Err(noncanonical_error());
    }
    let owner_generation = read_u64(header, OWNER_HEADER_GENERATION);
    let header_valid = header[8] == expected_state
        && header[9] == C16_RAW_OWNERS as u8
        && header[10..12].iter().all(|byte| *byte == 0)
        && read_u32(header, OWNER_HEADER_RECORD) == record_slot
        && owner_generation != 0
        && header[OWNER_HEADER_REQUEST..OWNER_HEADER_REQUEST + 40]
            == crate::request_book::c17::request_key(record.request_owner)
        && header[OWNER_HEADER_ENTITLEMENT..OWNER_HEADER_ENTITLEMENT + 32]
            == record.entitlement.get()
        && header[OWNER_HEADER_VECTOR..OWNER_HEADER_VECTOR + 32] == record.vector.get();
    let branch_current = (0..4).try_fold(0u64, |total, branch| {
        total.checked_add(read_u64(row, OWNER_ROW_BRANCH_CURRENT + branch * 8))
    });
    let active_link =
        decode_optional_arena_ref(&row[OWNER_ROW_ACTIVE_LINK..OWNER_ROW_ACTIVE_LINK + 8]);
    let source = decode_optional_arena_ref(&row[OWNER_ROW_SOURCE..OWNER_ROW_SOURCE + 8]);
    let row_valid = row[8] == expected_state
        && row[9] == C16_RAW_OWNERS as u8
        && read_u16(row, OWNER_ROW_VECTOR_LEN) == vector_len
        && read_u32(row, OWNER_ROW_LINKED_CLAIMS) == record.linked_claims
        && read_u64(row, OWNER_ROW_CURRENT) == u64::from(record.linked_claims)
        && active_link.is_ok()
        && source.is_ok()
        && read_u32(row, OWNER_ROW_RECORD) == record_slot
        && row[44..48].iter().all(|byte| *byte == 0)
        && read_u64(row, OWNER_ROW_GENERATION) == owner_generation
        && read_u32(row, 56) == 0
        && row[60..64].iter().all(|byte| *byte == 0)
        && branch_current == Some(u64::from(record.linked_claims))
        && row[96..].iter().all(|byte| *byte == 0);
    let index_refs = [
        decode_arena_ref(&index[8..16])?,
        decode_arena_ref(&index[16..24])?,
        decode_arena_ref(&index[24..32])?,
        decode_arena_ref(&index[32..40])?,
    ];
    let index_valid = index_refs == references
        && read_u32(index, 40) == record_slot
        && read_u64(index, 48) == owner_generation
        && read_u16(index, 56) == vector_len
        && index[OWNER_INDEX_STATE] == expected_state
        && index[59] == C16_RAW_OWNERS as u8
        && index[60..].iter().all(|byte| *byte == 0);
    let owner_refs = [
        decode_arena_ref(&owner[32..40])?,
        decode_arena_ref(&owner[40..48])?,
        decode_arena_ref(&owner[48..56])?,
        decode_arena_ref(&owner[56..64])?,
    ];
    let owner_valid = owner[OWNER_IMAGE_STATE] == expected_state
        && owner[9] == C16_RAW_OWNERS as u8
        && read_u16(owner, 10) == vector_len
        && read_u32(owner, OWNER_IMAGE_RECORD) == record_slot
        && read_u32(owner, OWNER_IMAGE_VECTOR_HEAD) == 0
        && read_u16(owner, OWNER_IMAGE_VECTOR_LEN) == vector_len
        && owner[22..24].iter().all(|byte| *byte == 0)
        && read_u32(owner, OWNER_IMAGE_LINKED_CLAIMS) == record.linked_claims
        && owner[28..32].iter().all(|byte| *byte == 0)
        && owner_refs == references
        && owner[64..96] == record.entitlement.get()
        && owner[96..128] == record.vector.get();
    (header_valid && row_valid && index_valid && owner_valid)
        .then_some(())
        .ok_or_else(noncanonical_error)
}

fn validate_withdrawable_owner_row(image: &[u8]) -> Result<(), SupportLedgerError> {
    let canonical = image.len() == OWNER_ROW_BYTES
        && image[8] == OWNER_STATE_LIVE
        && read_u16(image, OWNER_ROW_VECTOR_LEN) > 0
        && read_u32(image, OWNER_ROW_LINKED_CLAIMS) == 0
        && read_u64(image, OWNER_ROW_CURRENT) == 0
        && image[OWNER_ROW_ACTIVE_LINK..OWNER_ROW_SOURCE + 8]
            .iter()
            .all(|byte| *byte == 0);
    canonical.then_some(()).ok_or_else(noncanonical_error)
}

fn encode_arena_ref(bytes: &mut [u8], reference: ArenaRef) {
    debug_assert_eq!(bytes.len(), 8);
    write_u32(bytes, 0, reference.slot);
    write_u32(bytes, 4, reference.generation);
}

fn decode_optional_arena_ref(bytes: &[u8]) -> Result<Option<ArenaRef>, SupportLedgerError> {
    if bytes.len() != 8 {
        return Err(noncanonical_error());
    }
    if bytes.iter().all(|byte| *byte == 0) {
        return Ok(None);
    }
    decode_arena_ref(bytes).map(Some)
}
