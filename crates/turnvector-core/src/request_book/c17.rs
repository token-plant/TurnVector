use super::{AcceptedRequest, RequestBookGeneration, RequestError};
use crate::c17_layout::{
    Assignment, EVENT_CAPACITY, EventSlotImage, REQUEST_CAPACITY, ROOT_GROUP_CAPACITY,
    RequestBookC17HeaderImage, RequestSlotImage, SOURCE_CAPACITY, SourceSlotImage,
};
use crate::reusable::{
    ArenaRef, ArenaSelection, AssignmentOrderKey, ByteArenaHeaderImage, FixedByteArena, NodeHandle,
    PatriciaAssignmentPlan, ReusablePatricia,
};
use crate::{
    FixedStorageError, MonotonicTime, PhysicalStartCreditId, PlanMemberFunding, RequestId,
    RequestStatusVersion, SourceRecordRef, SupportOperationObligationId,
};
use std::mem::size_of;

const MEMBERSHIP_BYTES: usize = 112;
const REQUEST_IMAGE_BYTES: usize = size_of::<RequestSlotImage>();
const EVENT_IMAGE_BYTES: usize = size_of::<EventSlotImage>();
const SOURCE_IMAGE_BYTES: usize = size_of::<SourceSlotImage>();
const REQUEST_VALUE_BYTES: usize = 56;
const REQUEST_INDEX_ASSIGNMENT_ARENA: u16 = 20;
const EVENT_INDEX_ASSIGNMENT_ARENA: u16 = 21;
const SOURCE_INDEX_ASSIGNMENT_ARENA: u16 = 22;
const SINGLE_INDEX_ASSIGNMENTS: usize = 9 + 1;
const REQUEST_UPDATE_ASSIGNMENTS: usize = 9 * 4 + 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub(crate) enum SourceKind {
    InitialReady = 1,
    NewlyEligible = 2,
    Cancellation = 3,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub(crate) enum InitialReadyKind {
    MaterializationCompleted = 1,
    InitialFormationCompleted = 2,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub(crate) enum CancellationKind {
    Client = 1,
    Deadline = 2,
    DaemonShutdown = 3,
    InternalFailure = 4,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub(crate) enum SourceStateTag {
    Pending = 0,
    InitialCreated = 1,
    Consumed = 2,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub(crate) enum MembershipTag {
    Unready = 0,
    Bound = 1,
    EligibleUnbound = 2,
    Cancelled = 3,
    Closed = 4,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub(crate) enum MembershipEventKind {
    CreateStandalone = 1,
    MergeInitial = 2,
    Join = 3,
    Rebind = 4,
    Split = 5,
    Merge = 6,
    CancellationRemove = 7,
    Close = 8,
}

impl SourceRecordRef {
    fn from_arena(reference: ArenaRef) -> Result<Self, RequestError> {
        if usize::try_from(reference.slot).map_or(true, |slot| slot >= SOURCE_CAPACITY)
            || reference.generation == 0
        {
            return Err(RequestError::Storage(FixedStorageError::NonCanonical));
        }
        Ok(Self::from_canonical_parts(
            u16::try_from(reference.slot)
                .map_err(|_| RequestError::Storage(FixedStorageError::Capacity))?,
            reference.generation,
        ))
    }

    fn arena(self) -> Result<ArenaRef, RequestError> {
        if self.reserved != 0 || self.generation == 0 || usize::from(self.slot) >= SOURCE_CAPACITY {
            return Err(RequestError::Storage(FixedStorageError::NonCanonical));
        }
        Ok(ArenaRef {
            slot: u32::from(self.slot),
            generation: self.generation,
        })
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(C)]
pub(crate) struct EventRecordRef {
    pub(crate) slot: u16,
    reserved: u16,
    pub(crate) generation: u32,
}

impl EventRecordRef {
    pub(crate) const ABSENT: Self = Self {
        slot: 0,
        reserved: 0,
        generation: 0,
    };

    fn from_arena(reference: ArenaRef) -> Result<Self, RequestError> {
        if usize::try_from(reference.slot).map_or(true, |slot| slot >= EVENT_CAPACITY)
            || reference.generation == 0
        {
            return Err(RequestError::Storage(FixedStorageError::NonCanonical));
        }
        Ok(Self {
            slot: u16::try_from(reference.slot)
                .map_err(|_| RequestError::Storage(FixedStorageError::Capacity))?,
            reserved: 0,
            generation: reference.generation,
        })
    }

    fn arena(self) -> Result<ArenaRef, RequestError> {
        if self.reserved != 0 || self.generation == 0 || usize::from(self.slot) >= EVENT_CAPACITY {
            return Err(RequestError::Storage(FixedStorageError::NonCanonical));
        }
        Ok(ArenaRef {
            slot: u32::from(self.slot),
            generation: self.generation,
        })
    }

    pub(crate) const fn is_absent(self) -> bool {
        self.slot == 0 && self.reserved == 0 && self.generation == 0
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(C)]
pub(crate) struct RequestAddress {
    pub(crate) key: [u8; 40],
    pub(crate) slot: u16,
    reserved: u16,
    pub(crate) slot_generation: u32,
    pub(crate) status: u64,
}

impl RequestAddress {
    fn canonical(self) -> bool {
        self.reserved == 0
            && self.slot_generation != 0
            && self.status != 0
            && usize::from(self.slot) < REQUEST_CAPACITY
            && request_id_from_key(self.key).is_ok()
    }

    fn encode(self) -> [u8; REQUEST_VALUE_BYTES] {
        let mut bytes = [0; REQUEST_VALUE_BYTES];
        bytes[..40].copy_from_slice(&self.key);
        write_u16(&mut bytes, 40, self.slot);
        write_u16(&mut bytes, 42, self.reserved);
        write_u32(&mut bytes, 44, self.slot_generation);
        write_u64(&mut bytes, 48, self.status);
        bytes
    }

    fn decode(bytes: &[u8]) -> Result<Self, RequestError> {
        if bytes.len() != REQUEST_VALUE_BYTES {
            return Err(RequestError::Storage(FixedStorageError::NonCanonical));
        }
        let mut key = [0; 40];
        key.copy_from_slice(&bytes[..40]);
        let value = Self {
            key,
            slot: read_u16(bytes, 40),
            reserved: read_u16(bytes, 42),
            slot_generation: read_u32(bytes, 44),
            status: read_u64(bytes, 48),
        };
        if !value.canonical() {
            return Err(RequestError::Storage(FixedStorageError::NonCanonical));
        }
        Ok(value)
    }

    fn arena(self) -> ArenaRef {
        ArenaRef {
            slot: u32::from(self.slot),
            generation: self.slot_generation,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(transparent)]
pub(crate) struct SupportMembershipAnchor([u8; 64]);

impl SupportMembershipAnchor {
    pub(crate) const ABSENT: Self = Self([0; 64]);

    #[allow(
        clippy::too_many_arguments,
        reason = "the anchor seals every typed Support pointer"
    )]
    pub(crate) fn try_new(
        authority_key: [u8; 17],
        branch: u8,
        group_slot: u32,
        group_generation: u32,
        root_slot: u32,
        root_generation: u32,
        root_version: u64,
    ) -> Result<Self, RequestError> {
        if authority_key == [0; 17]
            || branch > 4
            || usize::try_from(group_slot).map_or(true, |slot| slot >= ROOT_GROUP_CAPACITY)
            || usize::try_from(root_slot).map_or(true, |slot| slot >= ROOT_GROUP_CAPACITY)
            || group_generation == 0
            || root_generation == 0
            || root_version == 0
        {
            return Err(RequestError::InvalidTransition);
        }
        let mut bytes = [0; 64];
        bytes[..17].copy_from_slice(&authority_key);
        bytes[17] = branch;
        write_u32(&mut bytes, 20, group_slot);
        write_u32(&mut bytes, 24, group_generation);
        write_u32(&mut bytes, 28, root_slot);
        write_u32(&mut bytes, 32, root_generation);
        write_u64(&mut bytes, 40, root_version);
        Ok(Self(bytes))
    }

    pub(crate) const fn is_absent(self) -> bool {
        let mut index = 0;
        while index < self.0.len() {
            if self.0[index] != 0 {
                return false;
            }
            index += 1;
        }
        true
    }

    fn canonical(self) -> bool {
        if self.is_absent() {
            return true;
        }
        self.0[..17] != [0; 17]
            && self.0[17] <= 4
            && self.0[18..20].iter().all(|byte| *byte == 0)
            && (read_u32(&self.0, 20) as usize) < ROOT_GROUP_CAPACITY
            && read_u32(&self.0, 24) != 0
            && (read_u32(&self.0, 28) as usize) < ROOT_GROUP_CAPACITY
            && read_u32(&self.0, 32) != 0
            && self.0[36..40].iter().all(|byte| *byte == 0)
            && read_u64(&self.0, 40) != 0
            && self.0[48..].iter().all(|byte| *byte == 0)
    }

    pub(crate) const fn authority_key(self) -> [u8; 17] {
        let mut key = [0; 17];
        let mut index = 0;
        while index < key.len() {
            key[index] = self.0[index];
            index += 1;
        }
        key
    }

    pub(crate) const fn branch(self) -> u8 {
        self.0[17]
    }

    pub(crate) fn group(self) -> ArenaRef {
        ArenaRef {
            slot: read_u32(&self.0, 20),
            generation: read_u32(&self.0, 24),
        }
    }

    pub(crate) fn root(self) -> ArenaRef {
        ArenaRef {
            slot: read_u32(&self.0, 28),
            generation: read_u32(&self.0, 32),
        }
    }

    pub(crate) fn root_version(self) -> u64 {
        read_u64(&self.0, 40)
    }

    pub(crate) const fn bytes(self) -> [u8; 64] {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct MembershipStateRow {
    pub(crate) tag: MembershipTag,
    pub(crate) epoch: u32,
    pub(crate) anchor: SupportMembershipAnchor,
    pub(crate) initial: SourceRecordRef,
    pub(crate) pending: SourceRecordRef,
    pub(crate) cancellation: SourceRecordRef,
    pub(crate) cancellation_fact: u64,
    pub(crate) cancellation_at: u64,
}

impl MembershipStateRow {
    const fn unready() -> Self {
        Self {
            tag: MembershipTag::Unready,
            epoch: 0,
            anchor: SupportMembershipAnchor::ABSENT,
            initial: SourceRecordRef::ABSENT,
            pending: SourceRecordRef::ABSENT,
            cancellation: SourceRecordRef::ABSENT,
            cancellation_fact: 0,
            cancellation_at: 0,
        }
    }

    fn canonical(self) -> bool {
        if !self.anchor.canonical()
            || !source_ref_canonical(self.initial)
            || !source_ref_canonical(self.pending)
            || !source_ref_canonical(self.cancellation)
        {
            return false;
        }
        match self.tag {
            MembershipTag::Unready => self == Self::unready(),
            MembershipTag::Bound => {
                self.epoch > 0
                    && !self.anchor.is_absent()
                    && !self.initial.is_absent()
                    && self.pending.is_absent()
                    && self.cancellation.is_absent()
                    && self.cancellation_fact == 0
                    && self.cancellation_at == 0
            }
            MembershipTag::EligibleUnbound => {
                self.epoch > 0
                    && !self.anchor.is_absent()
                    && !self.initial.is_absent()
                    && !self.pending.is_absent()
                    && self.cancellation.is_absent()
                    && self.cancellation_fact == 0
                    && self.cancellation_at == 0
            }
            MembershipTag::Cancelled => {
                self.epoch > 0
                    && self.anchor.is_absent()
                    && !self.initial.is_absent()
                    && self.pending.is_absent()
                    && !self.cancellation.is_absent()
                    && self.cancellation_fact > 0
                    && self.cancellation_at > 0
            }
            MembershipTag::Closed => {
                self.epoch > 0
                    && self.anchor.is_absent()
                    && !self.initial.is_absent()
                    && self.pending.is_absent()
                    && self.cancellation.is_absent()
                    && self.cancellation_fact == 0
                    && self.cancellation_at == 0
            }
        }
    }

    fn encode(self) -> Result<[u8; MEMBERSHIP_BYTES], RequestError> {
        if !self.canonical() {
            return Err(RequestError::Storage(FixedStorageError::NonCanonical));
        }
        let mut bytes = [0; MEMBERSHIP_BYTES];
        bytes[0] = self.tag as u8;
        write_u32(&mut bytes, 4, self.epoch);
        bytes[8..72].copy_from_slice(&self.anchor.0);
        encode_source_ref(&mut bytes[72..80], self.initial);
        encode_source_ref(&mut bytes[80..88], self.pending);
        encode_source_ref(&mut bytes[88..96], self.cancellation);
        write_u64(&mut bytes, 96, self.cancellation_fact);
        write_u64(&mut bytes, 104, self.cancellation_at);
        Ok(bytes)
    }

    fn decode(bytes: &[u8]) -> Result<Self, RequestError> {
        if bytes.len() != MEMBERSHIP_BYTES || bytes[1..4].iter().any(|byte| *byte != 0) {
            return Err(RequestError::Storage(FixedStorageError::NonCanonical));
        }
        let tag = match bytes[0] {
            0 => MembershipTag::Unready,
            1 => MembershipTag::Bound,
            2 => MembershipTag::EligibleUnbound,
            3 => MembershipTag::Cancelled,
            4 => MembershipTag::Closed,
            _ => return Err(RequestError::Storage(FixedStorageError::NonCanonical)),
        };
        let mut anchor = [0; 64];
        anchor.copy_from_slice(&bytes[8..72]);
        let value = Self {
            tag,
            epoch: read_u32(bytes, 4),
            anchor: SupportMembershipAnchor(anchor),
            initial: decode_source_ref(&bytes[72..80])?,
            pending: decode_source_ref(&bytes[80..88])?,
            cancellation: decode_source_ref(&bytes[88..96])?,
            cancellation_fact: read_u64(bytes, 96),
            cancellation_at: read_u64(bytes, 104),
        };
        value
            .canonical()
            .then_some(value)
            .ok_or(RequestError::Storage(FixedStorageError::NonCanonical))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct InitialReadyMarker {
    pub(crate) request: RequestId,
    pub(crate) kind: InitialReadyKind,
    pub(crate) identity: [u8; 32],
    pub(crate) domain: [u8; 16],
    pub(crate) occurred_at: MonotonicTime,
    pub(crate) funding: PlanMemberFunding,
    pub(crate) obligation: SupportOperationObligationId,
    pub(crate) credit: PhysicalStartCreditId,
}

impl InitialReadyMarker {
    pub(crate) fn validate(self) -> Result<(), RequestError> {
        (self.identity != [0; 32]
            && self.domain != [0; 16]
            && self.occurred_at.as_micros() != 0
            && self.funding.request_id == self.request
            && self.obligation.get() != self.credit.get())
        .then_some(())
        .ok_or(RequestError::InvalidTransition)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct MergeInitialMarker {
    pub(crate) identities: [[u8; 32]; 3],
    pub(crate) source_count: u8,
    pub(crate) domain: [u8; 16],
    pub(crate) occurred_at: MonotonicTime,
}

impl MergeInitialMarker {
    fn validate(self) -> Result<(), RequestError> {
        let count = usize::from(self.source_count);
        if !(2..=3).contains(&count)
            || self.domain == [0; 16]
            || self.occurred_at.as_micros() == 0
            || self.identities[..count].contains(&[0; 32])
            || self.identities[count..]
                .iter()
                .any(|identity| *identity != [0; 32])
        {
            return Err(RequestError::InvalidTransition);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MembershipDestination {
    Destination(u8),
    Closed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct MembershipMutation {
    pub(crate) request: RequestId,
    pub(crate) expected_status: RequestStatusVersion,
    pub(crate) destination: MembershipDestination,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct MembershipEventInput {
    pub(crate) kind: MembershipEventKind,
    pub(crate) source_identity: Option<[u8; 32]>,
    pub(crate) member_count: u8,
    pub(crate) destination_count: u8,
    pub(crate) members: [Option<MembershipMutation>; 4],
    pub(crate) occurred_at: MonotonicTime,
}

impl MembershipEventInput {
    fn validate(self) -> Result<(), RequestError> {
        let count = usize::from(self.member_count);
        let destination_count = usize::from(self.destination_count);
        let valid_count = match self.kind {
            MembershipEventKind::Join | MembershipEventKind::Merge => (2..=4).contains(&count),
            MembershipEventKind::Rebind | MembershipEventKind::Close => (1..=4).contains(&count),
            MembershipEventKind::Split => count == 4,
            _ => false,
        };
        let valid_destinations = match self.kind {
            MembershipEventKind::Join
            | MembershipEventKind::Rebind
            | MembershipEventKind::Merge => destination_count == 1,
            MembershipEventKind::Split => destination_count == 4,
            MembershipEventKind::Close => destination_count == 0,
            _ => false,
        };
        let valid_source = match self.kind {
            MembershipEventKind::Join => self.source_identity.is_some(),
            MembershipEventKind::Rebind => true,
            MembershipEventKind::Split
            | MembershipEventKind::Merge
            | MembershipEventKind::Close => self.source_identity.is_none(),
            _ => false,
        };
        if !valid_count
            || !valid_destinations
            || !valid_source
            || self.occurred_at.as_micros() == 0
            || self.members[..count].iter().any(Option::is_none)
            || self.members[count..].iter().any(Option::is_some)
            || self
                .source_identity
                .is_some_and(|identity| identity == [0; 32])
        {
            return Err(RequestError::InvalidTransition);
        }
        let mut used = [false; 4];
        for member in self.members[..count].iter().flatten() {
            match member.destination {
                MembershipDestination::Destination(ordinal) => {
                    let ordinal = usize::from(ordinal);
                    if self.kind == MembershipEventKind::Close || ordinal >= destination_count {
                        return Err(RequestError::InvalidTransition);
                    }
                    used[ordinal] = true;
                }
                MembershipDestination::Closed => {
                    if self.kind != MembershipEventKind::Close {
                        return Err(RequestError::InvalidTransition);
                    }
                }
            }
        }
        if used[..destination_count].iter().any(|used| !used)
            || used[destination_count..].iter().any(|used| *used)
        {
            return Err(RequestError::InvalidTransition);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct EligibilityMarker {
    pub(crate) request: RequestId,
    pub(crate) identity: [u8; 32],
    pub(crate) previous_anchor: SupportMembershipAnchor,
    pub(crate) occurred_at: MonotonicTime,
}

impl EligibilityMarker {
    fn validate(self) -> Result<(), RequestError> {
        (self.identity != [0; 32]
            && !self.previous_anchor.is_absent()
            && self.occurred_at.as_micros() != 0)
            .then_some(())
            .ok_or(RequestError::InvalidTransition)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CancellationMarker {
    pub(crate) request: RequestId,
    pub(crate) identity: [u8; 32],
    pub(crate) kind: CancellationKind,
    pub(crate) previous_anchor: SupportMembershipAnchor,
    pub(crate) occurred_at: MonotonicTime,
}

impl CancellationMarker {
    fn validate(self) -> Result<(), RequestError> {
        (self.identity != [0; 32]
            && !self.previous_anchor.is_absent()
            && self.occurred_at.as_micros() != 0)
            .then_some(())
            .ok_or(RequestError::InvalidTransition)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SourceRecord {
    key: [u8; 33],
    kind: SourceKind,
    initial_kind: u8,
    cancellation_kind: u8,
    state: SourceStateTag,
    request: RequestAddress,
    accepted_identity: [u8; 32],
    domain: [u8; 16],
    previous_anchor: SupportMembershipAnchor,
    occurred_at: u64,
    cancellation_fact: u64,
    create_event: EventRecordRef,
    consumed_event: EventRecordRef,
}
