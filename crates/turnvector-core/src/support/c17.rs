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
}
