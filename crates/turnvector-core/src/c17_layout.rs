#![allow(
    dead_code,
    reason = "C17 layout probes consume the complete closed image set"
)]

use std::mem::{align_of, offset_of, size_of};

pub(crate) const SUPPORT_HISTORIES: usize = 1_152;
pub(crate) const REQUEST_CAPACITY: usize = 1_024;
pub(crate) const CREATE_STANDALONE_BUDGET: usize = 3_456;
pub(crate) const MERGE_INITIAL_BUDGET: usize = 1_152;
pub(crate) const POST_CREATE_BUDGET: usize = 128;
pub(crate) const INITIAL_ROOT_CAPACITY: usize = 4_736;
pub(crate) const DESTINATION_ROOT_CAPACITY: usize = 5_120;
pub(crate) const ROOT_GROUP_CAPACITY: usize = 8_576;
pub(crate) const MEMBER_CAPACITY: usize = 34_304;
pub(crate) const FORMATION_CAPACITY: usize = 27_904;
pub(crate) const FUNDER_CAPACITY: usize = 111_616;
pub(crate) const AUTHORITY_CAPACITY: usize = 4_608;
pub(crate) const EXTERNAL_HEAD_CAPACITY: usize = 3_840;
pub(crate) const INITIAL_WRAPPER_CAPACITY: usize = 15_232;
pub(crate) const MEMBERSHIP_CAPACITY: usize = 4_736;
pub(crate) const LINK_CAPACITY: usize = 5_120;
pub(crate) const MUTATION_CAPACITY: usize = 25_216;
pub(crate) const LOCAL_CAPACITY: usize = 155_264;
pub(crate) const RAW_CAPACITY: usize = 53_412;
pub(crate) const EVENT_CAPACITY: usize = 4_736;
pub(crate) const SOURCE_CAPACITY: usize = 4_736;
pub(crate) const LIFECYCLE_CAPACITY: usize = 1_152;
pub(crate) const LIFECYCLE_BATCH_MAX: usize = 1_024;
pub(crate) const LIFECYCLE_CHUNK_MAX: usize = 8;

macro_rules! byte_image {
    ($name:ident, $bytes:expr) => {
        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        #[repr(C, align(8))]
        pub(crate) struct $name(pub(crate) [u8; $bytes]);
        impl $name {
            pub(crate) const ZERO: Self = Self([0; $bytes]);
        }
    };
}

byte_image!(C17HeaderImage, 128);
byte_image!(RequestBookC17HeaderImage, 128);
byte_image!(GroupImage, 128);
byte_image!(ExternalHeadImage, 128);
byte_image!(FormationImage, 256);
byte_image!(FunderImage, 128);
byte_image!(MemberImage, 128);
byte_image!(InitialWrapperImage, 128);
byte_image!(OwnerHeaderImage, 128);
byte_image!(OwnerRowImage, 128);
byte_image!(OwnerIndexImage, 64);
byte_image!(OwnerImage, 128);
byte_image!(LinkImage, 128);
byte_image!(MembershipImage, 128);
byte_image!(MutationImage, 128);
byte_image!(LifecycleRecordSlotImage, 1_152);
byte_image!(PendingLifecycleHeaderImage, 4_096);
byte_image!(RequestSlotImage, 640);
byte_image!(EventSlotImage, 1_536);
byte_image!(SourceSlotImage, 384);
byte_image!(TxnHeaderPage, 4_096);
byte_image!(OwnedInputPage, 4_096);
byte_image!(OutcomePage, 4_096);
byte_image!(Descriptor96, 96);
byte_image!(BundleSnapshot, 1_216);
byte_image!(CellSnapshot, 64);
byte_image!(DirectImage, 320);
byte_image!(DirectRecordPair, 640);
byte_image!(FreeSelection, 16);
byte_image!(PreparedBase, 239_360);
byte_image!(ValidatedBase, 239_384);
byte_image!(PreparedSplit, 538_960);
byte_image!(ValidatedSplit, 538_984);
byte_image!(BeginReservationImage, 48);
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub(crate) struct BeginFixedOutcome(pub(crate) [u8; 20]);
impl BeginFixedOutcome {
    pub(crate) const ZERO: Self = Self([0; 20]);
}
byte_image!(VisibilityOutcome, 24);
byte_image!(LifecyclePage, 4_096);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C, align(8))]
pub(crate) struct ScratchTopologyNode {
    pub(crate) tag: u8,
    pub(crate) flags: u8,
    pub(crate) bit: u16,
    pub(crate) parent: u32,
    pub(crate) zero: u32,
    pub(crate) one: u32,
    pub(crate) live_handle: u64,
    pub(crate) assignment_ordinal: u16,
    pub(crate) destination_kind: u8,
    pub(crate) image_len: u8,
    pub(crate) zero_tail: [u8; 4],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub(crate) enum ScratchTag {
    LeafRoute = 0,
    BranchRoute = 1,
    Terminal = 2,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub(crate) enum DestinationKind {
    Noop = 0,
    Leaf = 1,
    Branch = 2,
    Header = 3,
    Root = 4,
    Occupied = 5,
    FreeLength = 6,
    FreeCell = 7,
    IndexGeneration = 8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C, align(8))]
pub(crate) struct Assignment {
    pub(crate) destination_arena: u16,
    pub(crate) destination_kind: u8,
    pub(crate) image_len: u8,
    pub(crate) destination_slot: u32,
    pub(crate) expected_generation: u64,
    pub(crate) payload: [u8; 112],
}

impl Assignment {
    pub(crate) const NOOP: Self = Self {
        destination_arena: 0,
        destination_kind: DestinationKind::Noop as u8,
        image_len: 0,
        destination_slot: 0,
        expected_generation: 0,
        payload: [0; 112],
    };

    pub(crate) fn validate(&self) -> bool {
        let width = matches!(self.image_len, 4 | 8 | 40 | 48 | 56 | 64 | 112);
        match self.destination_kind {
            value if value == DestinationKind::Noop as u8 => *self == Self::NOOP,
            value
                if (DestinationKind::Leaf as u8..=DestinationKind::IndexGeneration as u8)
                    .contains(&value) =>
            {
                self.destination_arena != 0
                    && width
                    && self.payload[usize::from(self.image_len)..]
                        .iter()
                        .all(|byte| *byte == 0)
            }
            _ => false,
        }
    }
}

pub(crate) const PREPARED_BASE_BYTES: u64 = 239_360;
pub(crate) const VALIDATED_BASE_BYTES: u64 = 239_384;
pub(crate) const PREPARED_SPLIT_BYTES: u64 = 538_960;
pub(crate) const VALIDATED_SPLIT_BYTES: u64 = 538_984;
pub(crate) const ORDINARY_COPIED_BYTES: u64 = 1_616_904;
pub(crate) const SPLIT_ROUTE_POSITIONS: usize = 7_686;
pub(crate) const ORDINARY_ASSIGNMENTS: usize = 393;
pub(crate) const LIFECYCLE_ASSIGNMENTS: usize = 145;
pub(crate) const D_CELLS: usize = 848;
pub(crate) const J_ROWS: usize = 42;
pub(crate) const F_CELLS: usize = 128;
pub(crate) const X_ROWS: usize = 64;
pub(crate) const T_ROWS: usize = 9;
pub(crate) const B_ROWS: usize = 32;
pub(crate) const C_CELLS: usize = 192;
pub(crate) const L_CELLS: usize = 15;

pub(crate) const WORK_PLAN_CREATE: [u64; 5] = [15_297, 1_616_904, 0, 0, 14_139];
pub(crate) const WORK_CREATE_STANDALONE: [u64; 5] = [11_086, 1_616_904, 0, 0, 10_042];
pub(crate) const WORK_MERGE_INITIAL: [u64; 5] = [18_126, 1_616_904, 0, 0, 16_902];
pub(crate) const WORK_NEWLY_ELIGIBLE: [u64; 5] = [7_290, 1_616_904, 0, 0, 6_358];
pub(crate) const WORK_PLAN_DISPOSITION: [u64; 5] = [9_818, 1_616_904, 0, 0, 8_787];
pub(crate) const WORK_RESOLVE_OBSERVATION: [u64; 5] = [10_970, 1_616_904, 0, 0, 9_903];
pub(crate) const WORK_STATE_TRANSITION: [u64; 5] = [7_225, 1_616_904, 0, 0, 6_276];
pub(crate) const WORK_JOIN_REBIND: [u64; 5] = [16_264, 1_616_904, 0, 0, 15_094];
pub(crate) const WORK_SPLIT: [u64; 5] = [22_030, 1_616_904, 0, 0, 20_725];
pub(crate) const WORK_MERGE: [u64; 5] = [18_568, 1_616_904, 0, 0, 17_326];
pub(crate) const WORK_REMOVE_BOUND: [u64; 5] = [19_050, 1_616_904, 0, 0, 17_808];
pub(crate) const WORK_REMOVE_ELIGIBLE: [u64; 5] = [19_596, 1_616_904, 0, 0, 18_345];
pub(crate) const WORK_CLOSE: [u64; 5] = [14_341, 1_616_904, 0, 0, 13_217];
pub(crate) const WORK_TOMBSTONE: [u64; 5] = [9_048, 1_616_904, 0, 0, 8_053];
pub(crate) const WORK_MIGRATED_C16: [u64; 5] = [20_268, 1_616_904, 0, 0, 14_006];
pub(crate) const WORK_LIFECYCLE_BEGIN: [u64; 5] = [3_203, 64_000, 0, 0, 13_824];
pub(crate) const WORK_LIFECYCLE_STAGE: [u64; 5] = [9_378, 527_280, 0, 0, 9_223];
pub(crate) const WORK_LIFECYCLE_FINALIZE: [u64; 5] = [2_304, 1_269_760, 0, 0, 13_824];
pub(crate) const WORK_LIFECYCLE_ABORT: [u64; 5] = [9_386, 527_280, 0, 0, 9_223];

pub(crate) const fn legacy_migrated(work: [u64; 5]) -> Option<[u64; 5]> {
    Some([
        match work[0].checked_add(1_059) {
            Some(value) => value,
            None => return None,
        },
        ORDINARY_COPIED_BYTES,
        0,
        0,
        match work[4].checked_add(1_040) {
            Some(value) => value,
            None => return None,
        },
    ])
}
