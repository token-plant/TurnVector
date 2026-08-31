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

impl SourceRecord {
    fn normalized_eq(&self, other: &Self) -> bool {
        self.key == other.key
            && self.kind == other.kind
            && self.initial_kind == other.initial_kind
            && self.cancellation_kind == other.cancellation_kind
            && self.request.key == other.request.key
            && self.accepted_identity == other.accepted_identity
            && self.domain == other.domain
            && self.previous_anchor == other.previous_anchor
            && self.occurred_at == other.occurred_at
            && self.cancellation_fact == other.cancellation_fact
    }

    fn canonical(&self) -> bool {
        let lifecycle = match (self.kind, self.state) {
            (SourceKind::InitialReady, SourceStateTag::InitialCreated) => {
                !self.create_event.is_absent() && self.consumed_event.is_absent()
            }
            (SourceKind::NewlyEligible, SourceStateTag::Pending) => {
                self.create_event.is_absent() && self.consumed_event.is_absent()
            }
            (SourceKind::NewlyEligible, SourceStateTag::Consumed)
            | (SourceKind::Cancellation, SourceStateTag::Consumed) => {
                self.create_event.is_absent() && !self.consumed_event.is_absent()
            }
            _ => false,
        };
        self.key[0] == self.kind as u8
            && self.key[1..] == self.accepted_identity
            && self.accepted_identity != [0; 32]
            && self.request.canonical()
            && self.previous_anchor.canonical()
            && event_ref_canonical(self.create_event)
            && event_ref_canonical(self.consumed_event)
            && self.occurred_at > 0
            && lifecycle
            && match self.kind {
                SourceKind::InitialReady => {
                    matches!(self.initial_kind, 1 | 2)
                        && self.cancellation_kind == 0
                        && self.domain != [0; 16]
                        && self.previous_anchor.is_absent()
                        && self.cancellation_fact == 0
                }
                SourceKind::NewlyEligible => {
                    self.initial_kind == 0
                        && self.cancellation_kind == 0
                        && self.domain == [0; 16]
                        && !self.previous_anchor.is_absent()
                        && self.cancellation_fact == 0
                }
                SourceKind::Cancellation => {
                    self.initial_kind == 0
                        && matches!(self.cancellation_kind, 1..=4)
                        && self.domain == [0; 16]
                        && !self.previous_anchor.is_absent()
                        && self.cancellation_fact > 0
                }
            }
    }

    fn encode(&self) -> Result<[u8; SOURCE_IMAGE_BYTES], RequestError> {
        if !self.canonical() {
            return Err(RequestError::Storage(FixedStorageError::NonCanonical));
        }
        let mut bytes = [0; SOURCE_IMAGE_BYTES];
        bytes[8..41].copy_from_slice(&self.key);
        bytes[41] = self.kind as u8;
        bytes[42] = self.initial_kind;
        bytes[43] = self.cancellation_kind;
        bytes[44] = self.state as u8;
        bytes[48..104].copy_from_slice(&self.request.encode());
        bytes[104..136].copy_from_slice(&self.accepted_identity);
        bytes[136..152].copy_from_slice(&self.domain);
        bytes[152..216].copy_from_slice(&self.previous_anchor.0);
        write_u64(&mut bytes, 216, self.occurred_at);
        write_u64(&mut bytes, 224, self.cancellation_fact);
        encode_event_ref(&mut bytes[232..240], self.create_event);
        encode_event_ref(&mut bytes[240..248], self.consumed_event);
        Ok(bytes)
    }

    fn decode(bytes: &[u8; SOURCE_IMAGE_BYTES]) -> Result<Self, RequestError> {
        if bytes[45..48].iter().any(|byte| *byte != 0) || bytes[248..].iter().any(|byte| *byte != 0)
        {
            return Err(RequestError::Storage(FixedStorageError::NonCanonical));
        }
        let mut key = [0; 33];
        key.copy_from_slice(&bytes[8..41]);
        let kind = match bytes[41] {
            1 => SourceKind::InitialReady,
            2 => SourceKind::NewlyEligible,
            3 => SourceKind::Cancellation,
            _ => return Err(RequestError::Storage(FixedStorageError::NonCanonical)),
        };
        let state = match bytes[44] {
            0 => SourceStateTag::Pending,
            1 => SourceStateTag::InitialCreated,
            2 => SourceStateTag::Consumed,
            _ => return Err(RequestError::Storage(FixedStorageError::NonCanonical)),
        };
        let mut identity = [0; 32];
        identity.copy_from_slice(&bytes[104..136]);
        let mut domain = [0; 16];
        domain.copy_from_slice(&bytes[136..152]);
        let mut anchor = [0; 64];
        anchor.copy_from_slice(&bytes[152..216]);
        let value = Self {
            key,
            kind,
            initial_kind: bytes[42],
            cancellation_kind: bytes[43],
            state,
            request: RequestAddress::decode(&bytes[48..104])?,
            accepted_identity: identity,
            domain,
            previous_anchor: SupportMembershipAnchor(anchor),
            occurred_at: read_u64(bytes, 216),
            cancellation_fact: read_u64(bytes, 224),
            create_event: decode_event_ref(&bytes[232..240])?,
            consumed_event: decode_event_ref(&bytes[240..248])?,
        };
        value
            .canonical()
            .then_some(value)
            .ok_or(RequestError::Storage(FixedStorageError::NonCanonical))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct MembershipEventRecord {
    pub(crate) id: u64,
    pub(crate) kind: MembershipEventKind,
    pub(crate) source_count: u8,
    pub(crate) member_count: u8,
    pub(crate) consumed_by_support: bool,
    pub(crate) sources: [SourceRecordRef; 3],
    pub(crate) affected: [Option<RequestAddress>; 4],
    pub(crate) before: [Option<MembershipStateRow>; 4],
    pub(crate) after: [Option<MembershipStateRow>; 4],
    pub(crate) generation_before: u64,
    pub(crate) generation_after: u64,
    pub(crate) occurred_at: u64,
    pub(crate) cancellation_fact: u64,
}

impl MembershipEventRecord {
    fn canonical(&self) -> bool {
        let source_count = match self.kind {
            MembershipEventKind::CreateStandalone | MembershipEventKind::Join => {
                self.source_count == 1
            }
            MembershipEventKind::MergeInitial => (2..=3).contains(&self.source_count),
            MembershipEventKind::Rebind => self.source_count <= 1,
            MembershipEventKind::Split
            | MembershipEventKind::Merge
            | MembershipEventKind::Close => self.source_count == 0,
            MembershipEventKind::CancellationRemove => (1..=2).contains(&self.source_count),
        };
        let member_count = usize::from(self.member_count);
        let member_shape = match self.kind {
            MembershipEventKind::CreateStandalone => member_count == 1,
            MembershipEventKind::CancellationRemove => (1..=4).contains(&member_count),
            MembershipEventKind::MergeInitial => (2..=3).contains(&member_count),
            MembershipEventKind::Join | MembershipEventKind::Split | MembershipEventKind::Merge => {
                (2..=4).contains(&member_count)
            }
            MembershipEventKind::Rebind | MembershipEventKind::Close => {
                (1..=4).contains(&member_count)
            }
        };
        self.id > 0
            && source_count
            && member_shape
            && self.sources[..usize::from(self.source_count)]
                .iter()
                .all(|reference| !reference.is_absent() && source_ref_canonical(*reference))
            && self.sources[..usize::from(self.source_count)]
                .iter()
                .enumerate()
                .all(|(index, reference)| {
                    !self.sources[..index].iter().any(|prior| prior == reference)
                })
            && self.sources[usize::from(self.source_count)..]
                .iter()
                .all(|reference| reference.is_absent())
            && self.affected[..member_count]
                .iter()
                .all(|address| address.is_some_and(RequestAddress::canonical))
            && self.before[..member_count]
                .iter()
                .chain(&self.after[..member_count])
                .all(|row| row.is_some_and(MembershipStateRow::canonical))
            && self.affected[..member_count]
                .windows(2)
                .all(|pair| pair[0] < pair[1])
            && self.affected[member_count..].iter().all(Option::is_none)
            && self.before[member_count..]
                .iter()
                .chain(&self.after[member_count..])
                .all(Option::is_none)
            && self.generation_after == self.generation_before.checked_add(1).unwrap_or(0)
            && self.occurred_at > 0
            && self.consumed_by_support
            && match self.kind {
                MembershipEventKind::CancellationRemove => self.cancellation_fact > 0,
                _ => self.cancellation_fact == 0,
            }
    }

    fn encode(&self) -> Result<[u8; EVENT_IMAGE_BYTES], RequestError> {
        if !self.canonical() {
            return Err(RequestError::Storage(FixedStorageError::NonCanonical));
        }
        let mut bytes = [0; EVENT_IMAGE_BYTES];
        write_u64(&mut bytes, 8, self.id);
        bytes[16] = self.kind as u8;
        bytes[17] = self.source_count;
        bytes[18] = self.member_count;
        bytes[19] = u8::from(self.consumed_by_support);
        for (index, reference) in self.sources.iter().copied().enumerate() {
            encode_source_ref(&mut bytes[24 + index * 8..32 + index * 8], reference);
        }
        let mut offset = 48;
        for address in self.affected {
            if let Some(address) = address {
                bytes[offset..offset + 56].copy_from_slice(&address.encode());
            }
            offset += 56;
        }
        for rows in [self.before, self.after] {
            for row in rows {
                if let Some(row) = row {
                    bytes[offset..offset + MEMBERSHIP_BYTES].copy_from_slice(&row.encode()?);
                }
                offset += MEMBERSHIP_BYTES;
            }
        }
        write_u64(&mut bytes, 1_168, self.generation_before);
        write_u64(&mut bytes, 1_176, self.generation_after);
        write_u64(&mut bytes, 1_184, self.occurred_at);
        write_u64(&mut bytes, 1_192, self.cancellation_fact);
        Ok(bytes)
    }

    fn decode(bytes: &[u8; EVENT_IMAGE_BYTES]) -> Result<Self, RequestError> {
        if bytes[20..24].iter().any(|byte| *byte != 0)
            || bytes[1_200..].iter().any(|byte| *byte != 0)
        {
            return Err(RequestError::Storage(FixedStorageError::NonCanonical));
        }
        let kind = match bytes[16] {
            1 => MembershipEventKind::CreateStandalone,
            2 => MembershipEventKind::MergeInitial,
            3 => MembershipEventKind::Join,
            4 => MembershipEventKind::Rebind,
            5 => MembershipEventKind::Split,
            6 => MembershipEventKind::Merge,
            7 => MembershipEventKind::CancellationRemove,
            8 => MembershipEventKind::Close,
            _ => return Err(RequestError::Storage(FixedStorageError::NonCanonical)),
        };
        let source_count = bytes[17];
        let member_count = bytes[18];
        let consumed_by_support = match bytes[19] {
            0 => false,
            1 => true,
            _ => return Err(RequestError::Storage(FixedStorageError::NonCanonical)),
        };
        let mut sources = [SourceRecordRef::ABSENT; 3];
        for (index, reference) in sources.iter_mut().enumerate() {
            *reference = decode_source_ref(&bytes[24 + index * 8..32 + index * 8])?;
        }
        let active = usize::from(member_count);
        if active > 4 {
            return Err(RequestError::Storage(FixedStorageError::NonCanonical));
        }
        let mut affected = [None; 4];
        let mut offset = 48;
        for (index, address) in affected.iter_mut().enumerate() {
            let image = &bytes[offset..offset + REQUEST_VALUE_BYTES];
            if index < active {
                *address = Some(RequestAddress::decode(image)?);
            } else if image.iter().any(|byte| *byte != 0) {
                return Err(RequestError::Storage(FixedStorageError::NonCanonical));
            }
            offset += REQUEST_VALUE_BYTES;
        }
        let mut before = [None; 4];
        let mut after = [None; 4];
        for rows in [&mut before, &mut after] {
            for (index, row) in rows.iter_mut().enumerate() {
                let image = &bytes[offset..offset + MEMBERSHIP_BYTES];
                if index < active {
                    *row = Some(MembershipStateRow::decode(image)?);
                } else if image.iter().any(|byte| *byte != 0) {
                    return Err(RequestError::Storage(FixedStorageError::NonCanonical));
                }
                offset += MEMBERSHIP_BYTES;
            }
        }
        debug_assert_eq!(offset, 1_168);
        let event = Self {
            id: read_u64(bytes, 8),
            kind,
            source_count,
            member_count,
            consumed_by_support,
            sources,
            affected,
            before,
            after,
            generation_before: read_u64(bytes, 1_168),
            generation_after: read_u64(bytes, 1_176),
            occurred_at: read_u64(bytes, 1_184),
            cancellation_fact: read_u64(bytes, 1_192),
        };
        event
            .canonical()
            .then_some(event)
            .ok_or(RequestError::Storage(FixedStorageError::NonCanonical))
    }
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct PreparedRequestInstall {
    expected_request_header: ByteArenaHeaderImage,
    request_header_after: ByteArenaHeaderImage,
    selection: ArenaSelection<1>,
    address: RequestAddress,
    slot_image: [u8; REQUEST_IMAGE_BYTES],
    index_plan: PatriciaAssignmentPlan<SINGLE_INDEX_ASSIGNMENTS>,
    header_after: RequestBookC17HeaderImage,
}

#[derive(Debug, Eq, PartialEq)]
struct PreparedRequestDirectImages {
    generation_after: RequestBookGeneration,
    expected_arena_headers: [ByteArenaHeaderImage; 3],
    arena_headers_after: [ByteArenaHeaderImage; 3],
    source_update: Option<(ArenaRef, [u8; SOURCE_IMAGE_BYTES])>,
    source_install: Option<(ArenaRef, [u8; SOURCE_IMAGE_BYTES])>,
    event_install: Option<(ArenaRef, [u8; EVENT_IMAGE_BYTES])>,
    request_slots: [Option<(ArenaRef, [u8; REQUEST_IMAGE_BYTES], RequestStatusVersion)>; 4],
    header_after: RequestBookC17HeaderImage,
}

#[derive(Debug)]
pub(crate) struct PreparedNewlyEligible {
    expected_generation: u64,
    request: RequestAddress,
    before: MembershipStateRow,
    after_status: u64,
    source: SourceRecord,
    source_selection: ArenaSelection<1>,
    source_index_plan: PatriciaAssignmentPlan<SINGLE_INDEX_ASSIGNMENTS>,
    request_index_plan: PatriciaAssignmentPlan<REQUEST_UPDATE_ASSIGNMENTS>,
    direct: PreparedRequestDirectImages,
}

impl PreparedNewlyEligible {
    pub(crate) const fn generation_after(&self) -> RequestBookGeneration {
        self.direct.generation_after
    }

    pub(crate) const fn request(&self) -> RequestAddress {
        self.request
    }

    pub(crate) fn visit_assignments(
        &self,
        visitor: &mut dyn FnMut(AssignmentOrderKey, Assignment),
    ) {
        self.source_index_plan.visit_assignments(visitor);
        self.request_index_plan.visit_assignments(visitor);
    }
}

#[derive(Debug)]
pub(crate) struct PreparedCancellation {
    expected_generation: u64,
    request: RequestAddress,
    before: MembershipStateRow,
    after_status: [u64; 4],
    cancellation: SourceRecord,
    pending: Option<(SourceRecordRef, SourceRecord, SourceRecord)>,
    source_selection: ArenaSelection<1>,
    event_selection: ArenaSelection<1>,
    event: MembershipEventRecord,
    source_index_plan: PatriciaAssignmentPlan<SINGLE_INDEX_ASSIGNMENTS>,
    event_index_plan: PatriciaAssignmentPlan<SINGLE_INDEX_ASSIGNMENTS>,
    request_index_plan: PatriciaAssignmentPlan<REQUEST_UPDATE_ASSIGNMENTS>,
    direct: PreparedRequestDirectImages,
}

impl PreparedCancellation {
    pub(crate) const fn generation_after(&self) -> RequestBookGeneration {
        self.direct.generation_after
    }

    pub(crate) const fn request(&self) -> RequestAddress {
        self.request
    }

    pub(crate) const fn previous_anchor(&self) -> SupportMembershipAnchor {
        self.before.anchor
    }

    pub(crate) const fn source_count(&self) -> u8 {
        self.event.source_count
    }

    pub(crate) const fn event_id(&self) -> u64 {
        self.event.id
    }

    pub(crate) const fn fact_id(&self) -> u64 {
        self.event.cancellation_fact
    }

    pub(crate) const fn event(&self) -> &MembershipEventRecord {
        &self.event
    }

    pub(crate) fn visit_assignments(
        &self,
        visitor: &mut dyn FnMut(AssignmentOrderKey, Assignment),
    ) {
        self.source_index_plan.visit_assignments(visitor);
        self.event_index_plan.visit_assignments(visitor);
        self.request_index_plan.visit_assignments(visitor);
    }
}

#[derive(Debug)]
pub(crate) struct PreparedCreateStandalone {
    expected_generation: u64,
    request: RequestAddress,
    before: MembershipStateRow,
    after_status: u64,
    source: SourceRecord,
    source_selection: ArenaSelection<1>,
    event_selection: ArenaSelection<1>,
    event: MembershipEventRecord,
    source_index_plan: PatriciaAssignmentPlan<SINGLE_INDEX_ASSIGNMENTS>,
    event_index_plan: PatriciaAssignmentPlan<SINGLE_INDEX_ASSIGNMENTS>,
    request_index_plan: PatriciaAssignmentPlan<REQUEST_UPDATE_ASSIGNMENTS>,
    direct: PreparedRequestDirectImages,
}

impl PreparedCreateStandalone {
    pub(crate) const fn generation_after(&self) -> RequestBookGeneration {
        self.direct.generation_after
    }

    pub(crate) const fn request(&self) -> RequestAddress {
        self.request
    }

    pub(crate) const fn event_id(&self) -> u64 {
        self.event.id
    }

    pub(crate) const fn source_ref(&self) -> SourceRecordRef {
        self.event.sources[0]
    }

    pub(crate) const fn event(&self) -> &MembershipEventRecord {
        &self.event
    }

    pub(crate) fn visit_assignments(
        &self,
        visitor: &mut dyn FnMut(AssignmentOrderKey, Assignment),
    ) {
        self.source_index_plan.visit_assignments(visitor);
        self.event_index_plan.visit_assignments(visitor);
        self.request_index_plan.visit_assignments(visitor);
    }
}

#[derive(Debug)]
pub(crate) struct PreparedMergeInitial {
    expected_generation: u64,
    event_selection: ArenaSelection<1>,
    event: MembershipEventRecord,
    after_status: [u64; 4],
    event_index_plan: PatriciaAssignmentPlan<SINGLE_INDEX_ASSIGNMENTS>,
    request_index_plan: PatriciaAssignmentPlan<REQUEST_UPDATE_ASSIGNMENTS>,
    direct: PreparedRequestDirectImages,
}

impl PreparedMergeInitial {
    pub(crate) const fn generation_after(&self) -> RequestBookGeneration {
        self.direct.generation_after
    }

    pub(crate) const fn event_id(&self) -> u64 {
        self.event.id
    }

    pub(crate) const fn source_count(&self) -> u8 {
        self.event.source_count
    }

    pub(crate) const fn event(&self) -> &MembershipEventRecord {
        &self.event
    }

    pub(crate) fn visit_assignments(
        &self,
        visitor: &mut dyn FnMut(AssignmentOrderKey, Assignment),
    ) {
        self.event_index_plan.visit_assignments(visitor);
        self.request_index_plan.visit_assignments(visitor);
    }
}

#[derive(Debug)]
pub(crate) struct PreparedMembershipIntent {
    expected_generation: u64,
    event_selection: ArenaSelection<1>,
    event: MembershipEventRecord,
    after_status: [u64; 4],
    destinations: [MembershipDestination; 4],
    destination_count: u8,
    source_update: Option<(SourceRecordRef, SourceRecord, SourceRecord)>,
    event_index_plan: PatriciaAssignmentPlan<SINGLE_INDEX_ASSIGNMENTS>,
}

impl PreparedMembershipIntent {
    pub(crate) const fn event_id(&self) -> u64 {
        self.event.id
    }

    pub(crate) const fn kind(&self) -> MembershipEventKind {
        self.event.kind
    }

    pub(crate) const fn member_count(&self) -> usize {
        self.event.member_count as usize
    }

    pub(crate) const fn destination_count(&self) -> usize {
        self.destination_count as usize
    }

    pub(crate) const fn occurred_at(&self) -> u64 {
        self.event.occurred_at
    }

    pub(crate) const fn event(&self) -> &MembershipEventRecord {
        &self.event
    }

    pub(crate) const fn destination(&self, index: usize) -> Option<MembershipDestination> {
        if index < self.event.member_count as usize {
            Some(self.destinations[index])
        } else {
            None
        }
    }
}

#[derive(Debug)]
pub(crate) struct PreparedMembershipEvent {
    intent: PreparedMembershipIntent,
    request_index_plan: PatriciaAssignmentPlan<REQUEST_UPDATE_ASSIGNMENTS>,
    direct: PreparedRequestDirectImages,
}

impl PreparedMembershipEvent {
    pub(crate) const fn generation_after(&self) -> RequestBookGeneration {
        self.direct.generation_after
    }

    pub(crate) const fn event_id(&self) -> u64 {
        self.intent.event.id
    }

    pub(crate) const fn kind(&self) -> MembershipEventKind {
        self.intent.event.kind
    }

    pub(crate) const fn event(&self) -> &MembershipEventRecord {
        &self.intent.event
    }

    pub(crate) fn visit_assignments(
        &self,
        visitor: &mut dyn FnMut(AssignmentOrderKey, Assignment),
    ) {
        self.intent.event_index_plan.visit_assignments(visitor);
        self.request_index_plan.visit_assignments(visitor);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RequestBookC17Capacities {
    requests: usize,
    events: usize,
    sources: usize,
}

impl RequestBookC17Capacities {
    pub(crate) const fn production() -> Self {
        Self {
            requests: REQUEST_CAPACITY,
            events: EVENT_CAPACITY,
            sources: SOURCE_CAPACITY,
        }
    }

    #[cfg(test)]
    pub(crate) const fn testing(requests: usize) -> Self {
        Self {
            requests,
            events: 64,
            sources: 64,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[repr(C)]
pub(crate) struct RequestBookC17 {
    header: RequestBookC17HeaderImage,
    requests: FixedByteArena<REQUEST_IMAGE_BYTES>,
    request_index: ReusablePatricia<40, 56>,
    events: FixedByteArena<EVENT_IMAGE_BYTES>,
    event_index: ReusablePatricia<8, 8>,
    sources: FixedByteArena<SOURCE_IMAGE_BYTES>,
    source_index: ReusablePatricia<33, 8>,
}

impl RequestBookC17 {
    pub(crate) fn try_new(
        capacities: RequestBookC17Capacities,
        generation: RequestBookGeneration,
    ) -> Result<Self, RequestError> {
        if capacities.requests == 0
            || capacities.requests > REQUEST_CAPACITY
            || capacities.events == 0
            || capacities.events > EVENT_CAPACITY
            || capacities.sources == 0
            || capacities.sources > SOURCE_CAPACITY
        {
            return Err(RequestError::Storage(FixedStorageError::Capacity));
        }
        let mut header = RequestBookC17HeaderImage::ZERO;
        for (offset, value) in [
            (0, 1),
            (8, 1),
            (16, 1),
            (24, generation.get()),
            (32, 1),
            (40, 1),
            (48, 1),
            (56, 1),
        ] {
            write_u64(&mut header.0, offset, value);
        }
        write_u32(&mut header.0, 64, capacities.requests as u32);
        write_u32(&mut header.0, 68, capacities.requests as u32);
        write_u32(&mut header.0, 72, capacities.events as u32);
        write_u32(&mut header.0, 76, capacities.events as u32);
        write_u32(&mut header.0, 80, capacities.sources as u32);
        write_u32(&mut header.0, 84, capacities.sources as u32);
        Ok(Self {
            header,
            requests: FixedByteArena::try_new(capacities.requests)?,
            request_index: ReusablePatricia::try_new(capacities.requests)?,
            events: FixedByteArena::try_new(capacities.events)?,
            event_index: ReusablePatricia::try_new(capacities.events)?,
            sources: FixedByteArena::try_new(capacities.sources)?,
            source_index: ReusablePatricia::try_new(capacities.sources)?,
        })
    }

    pub(crate) const fn generation(&self) -> u64 {
        read_u64_const(&self.header.0, 24)
    }

    pub(crate) fn commit_assignment_direct(&mut self, assignment: &Assignment) {
        match assignment.destination_arena {
            REQUEST_INDEX_ASSIGNMENT_ARENA => {
                self.request_index.commit_assignment_direct(assignment)
            }
            EVENT_INDEX_ASSIGNMENT_ARENA => self.event_index.commit_assignment_direct(assignment),
            SOURCE_INDEX_ASSIGNMENT_ARENA => self.source_index.commit_assignment_direct(assignment),
            _ => unreachable!("validated RequestBook assignment arena"),
        }
    }

    pub(crate) fn validate_request_book_generation(
        &self,
        expected: RequestBookGeneration,
    ) -> Result<(), RequestError> {
        (self.generation() == expected.get())
            .then_some(())
            .ok_or(RequestError::PreparedChangeStale)
    }

    pub(crate) fn commit_request_book_generation(
        &mut self,
        expected: RequestBookGeneration,
        next: RequestBookGeneration,
    ) {
        self.validate_request_book_generation(expected)
            .expect("validated RequestBook generation");
        assert_eq!(
            expected.get().checked_add(1),
            Some(next.get()),
            "prepared RequestBook generation"
        );
        write_u64(&mut self.header.0, 24, next.get());
    }

    #[cfg(test)]
    pub(crate) fn current_counts(&self) -> [u32; 4] {
        [
            read_u32(&self.header.0, 96),
            read_u32(&self.header.0, 100),
            read_u32(&self.header.0, 104),
            read_u32(&self.header.0, 108),
        ]
    }

    #[cfg(test)]
    pub(crate) fn force_generation_for_test(&mut self, generation: u64) {
        write_u64(&mut self.header.0, 24, generation);
    }
}
