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

pub(crate) fn b03_layout_probe() -> Vec<u8> {
    let mut rows = vec![
        ("assignment.align", align_of::<Assignment>()),
        (
            "assignment.destination_arena",
            offset_of!(Assignment, destination_arena),
        ),
        (
            "assignment.destination_kind",
            offset_of!(Assignment, destination_kind),
        ),
        (
            "assignment.destination_slot",
            offset_of!(Assignment, destination_slot),
        ),
        (
            "assignment.expected_generation",
            offset_of!(Assignment, expected_generation),
        ),
        ("assignment.image_len", offset_of!(Assignment, image_len)),
        ("assignment.payload", offset_of!(Assignment, payload)),
        ("assignment.size", size_of::<Assignment>()),
        ("box_slice.align", align_of::<Box<[u8]>>()),
        ("box_slice.size", size_of::<Box<[u8]>>()),
        ("c17_header.size", size_of::<C17HeaderImage>()),
        ("event_slot.size", size_of::<EventSlotImage>()),
        ("external_head.size", size_of::<ExternalHeadImage>()),
        ("formation.size", size_of::<FormationImage>()),
        ("funder.size", size_of::<FunderImage>()),
        ("group.size", size_of::<GroupImage>()),
        ("initial_wrapper.size", size_of::<InitialWrapperImage>()),
        (
            "lifecycle_record.size",
            size_of::<LifecycleRecordSlotImage>(),
        ),
        ("link.size", size_of::<LinkImage>()),
        ("member.size", size_of::<MemberImage>()),
        ("membership.size", size_of::<MembershipImage>()),
        ("mutation.size", size_of::<MutationImage>()),
        ("owner.size", size_of::<OwnerImage>()),
        ("owner_header.size", size_of::<OwnerHeaderImage>()),
        ("owner_index.size", size_of::<OwnerIndexImage>()),
        ("owner_row.size", size_of::<OwnerRowImage>()),
        (
            "pending_header.size",
            size_of::<PendingLifecycleHeaderImage>(),
        ),
        ("prepared_base.size", size_of::<PreparedBase>()),
        ("prepared_split.size", size_of::<PreparedSplit>()),
        ("request_slot.size", size_of::<RequestSlotImage>()),
        ("scratch.align", align_of::<ScratchTopologyNode>()),
        (
            "scratch.assignment_ordinal",
            offset_of!(ScratchTopologyNode, assignment_ordinal),
        ),
        (
            "scratch.destination_kind",
            offset_of!(ScratchTopologyNode, destination_kind),
        ),
        ("scratch.flags", offset_of!(ScratchTopologyNode, flags)),
        (
            "scratch.image_len",
            offset_of!(ScratchTopologyNode, image_len),
        ),
        (
            "scratch.live_handle",
            offset_of!(ScratchTopologyNode, live_handle),
        ),
        ("scratch.parent", offset_of!(ScratchTopologyNode, parent)),
        ("scratch.size", size_of::<ScratchTopologyNode>()),
        ("source_slot.size", size_of::<SourceSlotImage>()),
        ("validated_base.size", size_of::<ValidatedBase>()),
        ("validated_split.size", size_of::<ValidatedSplit>()),
        ("begin_fixed_outcome.size", size_of::<BeginFixedOutcome>()),
        ("begin_reservation.size", size_of::<BeginReservationImage>()),
        ("bundle_snapshot.size", size_of::<BundleSnapshot>()),
        ("cell_snapshot.size", size_of::<CellSnapshot>()),
        ("descriptor96.size", size_of::<Descriptor96>()),
        ("direct_image.size", size_of::<DirectImage>()),
        ("direct_record_pair.size", size_of::<DirectRecordPair>()),
        ("free_selection.size", size_of::<FreeSelection>()),
        ("funder.align", align_of::<FunderImage>()),
        ("group.align", align_of::<GroupImage>()),
        ("lifecycle_page.size", size_of::<LifecyclePage>()),
        ("member.align", align_of::<MemberImage>()),
        ("outcome_page.size", size_of::<OutcomePage>()),
        ("owned_input_page.size", size_of::<OwnedInputPage>()),
        (
            "request_book_c17_header.size",
            size_of::<RequestBookC17HeaderImage>(),
        ),
        ("scratch.bit", offset_of!(ScratchTopologyNode, bit)),
        ("scratch.one", offset_of!(ScratchTopologyNode, one)),
        ("scratch.tag", offset_of!(ScratchTopologyNode, tag)),
        ("scratch.zero", offset_of!(ScratchTopologyNode, zero)),
        (
            "scratch.zero_tail",
            offset_of!(ScratchTopologyNode, zero_tail),
        ),
        ("txn_header_page.size", size_of::<TxnHeaderPage>()),
        ("visibility_outcome.size", size_of::<VisibilityOutcome>()),
        ("wrapper.align", align_of::<InitialWrapperImage>()),
    ];
    rows.sort_unstable_by_key(|row| row.0);
    let mut output = b"turnvector.c17.b03.v1\n".to_vec();
    for (name, value) in rows {
        output.extend_from_slice(name.as_bytes());
        output.push(b'=');
        output.extend_from_slice(value.to_string().as_bytes());
        output.push(b'\n');
    }
    for (name, value) in [
        ("capacity.authority", AUTHORITY_CAPACITY),
        ("capacity.destination_root", DESTINATION_ROOT_CAPACITY),
        ("capacity.event", EVENT_CAPACITY),
        ("capacity.formation", FORMATION_CAPACITY),
        ("capacity.funder", FUNDER_CAPACITY),
        ("capacity.group", ROOT_GROUP_CAPACITY),
        ("capacity.initial_root", INITIAL_ROOT_CAPACITY),
        ("capacity.initial_wrapper", INITIAL_WRAPPER_CAPACITY),
        ("capacity.lifecycle", LIFECYCLE_CAPACITY),
        ("capacity.link", LINK_CAPACITY),
        ("capacity.local", LOCAL_CAPACITY),
        ("capacity.member", MEMBER_CAPACITY),
        ("capacity.membership", MEMBERSHIP_CAPACITY),
        ("capacity.mutation", MUTATION_CAPACITY),
        ("capacity.raw", RAW_CAPACITY),
        ("capacity.request", REQUEST_CAPACITY),
        ("capacity.source", SOURCE_CAPACITY),
        ("census.b", B_ROWS),
        ("census.c", C_CELLS),
        ("census.d", D_CELLS),
        ("census.f", F_CELLS),
        ("census.j", J_ROWS),
        ("census.l", L_CELLS),
        ("census.t", T_ROWS),
        ("census.x", X_ROWS),
        ("lifecycle.batch_max", LIFECYCLE_BATCH_MAX),
        ("lifecycle.chunk_max", LIFECYCLE_CHUNK_MAX),
        ("ordinary.assignments", ORDINARY_ASSIGNMENTS),
        ("ordinary.route_positions", SPLIT_ROUTE_POSITIONS),
        ("tag.destination.branch", DestinationKind::Branch as usize),
        (
            "tag.destination.free_cell",
            DestinationKind::FreeCell as usize,
        ),
        (
            "tag.destination.free_length",
            DestinationKind::FreeLength as usize,
        ),
        ("tag.destination.header", DestinationKind::Header as usize),
        (
            "tag.destination.index_generation",
            DestinationKind::IndexGeneration as usize,
        ),
        ("tag.destination.leaf", DestinationKind::Leaf as usize),
        ("tag.destination.noop", DestinationKind::Noop as usize),
        (
            "tag.destination.occupied",
            DestinationKind::Occupied as usize,
        ),
        ("tag.destination.root", DestinationKind::Root as usize),
        ("tag.scratch.branch", ScratchTag::BranchRoute as usize),
        ("tag.scratch.leaf", ScratchTag::LeafRoute as usize),
        ("tag.scratch.terminal", ScratchTag::Terminal as usize),
    ] {
        output.extend_from_slice(name.as_bytes());
        output.push(b'=');
        output.extend_from_slice(value.to_string().as_bytes());
        output.push(b'\n');
    }
    for (name, row) in [
        ("work.begin", WORK_STATE_TRANSITION),
        ("work.close", WORK_CLOSE),
        ("work.create_standalone", WORK_CREATE_STANDALONE),
        ("work.join_rebind", WORK_JOIN_REBIND),
        ("work.lifecycle_abort", WORK_LIFECYCLE_ABORT),
        ("work.lifecycle_begin", WORK_LIFECYCLE_BEGIN),
        ("work.lifecycle_finalize", WORK_LIFECYCLE_FINALIZE),
        ("work.lifecycle_stage", WORK_LIFECYCLE_STAGE),
        ("work.merge", WORK_MERGE),
        ("work.merge_initial", WORK_MERGE_INITIAL),
        ("work.migrated_c16", WORK_MIGRATED_C16),
        ("work.newly_eligible", WORK_NEWLY_ELIGIBLE),
        ("work.plan_create", WORK_PLAN_CREATE),
        ("work.plan_disposition", WORK_PLAN_DISPOSITION),
        ("work.remove_bound", WORK_REMOVE_BOUND),
        ("work.remove_eligible", WORK_REMOVE_ELIGIBLE),
        ("work.resolve_observation", WORK_RESOLVE_OBSERVATION),
        ("work.split", WORK_SPLIT),
        ("work.tombstone", WORK_TOMBSTONE),
    ] {
        output.extend_from_slice(name.as_bytes());
        output.push(b'=');
        for (index, value) in row.into_iter().enumerate() {
            if index != 0 {
                output.push(b',');
            }
            output.extend_from_slice(value.to_string().as_bytes());
        }
        output.push(b'\n');
    }
    output
}

const _: () = {
    assert!(size_of::<C17HeaderImage>() == 128);
    assert!(size_of::<RequestBookC17HeaderImage>() == 128);
    assert!(size_of::<Assignment>() == 128);
    assert!(align_of::<Assignment>() == 8);
    assert!(offset_of!(Assignment, destination_arena) == 0);
    assert!(offset_of!(Assignment, destination_kind) == 2);
    assert!(offset_of!(Assignment, image_len) == 3);
    assert!(offset_of!(Assignment, destination_slot) == 4);
    assert!(offset_of!(Assignment, expected_generation) == 8);
    assert!(offset_of!(Assignment, payload) == 16);
    assert!(size_of::<ScratchTopologyNode>() == 32);
    assert!(align_of::<ScratchTopologyNode>() == 8);
    assert!(size_of::<LifecycleRecordSlotImage>() == 1_152);
    assert!(size_of::<PendingLifecycleHeaderImage>() == 4_096);
    assert!(size_of::<PreparedBase>() == 239_360);
    assert!(size_of::<ValidatedBase>() == 239_384);
    assert!(size_of::<PreparedSplit>() == 538_960);
    assert!(size_of::<ValidatedSplit>() == 538_984);
    assert!(size_of::<Box<[u8]>>() == 16);
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probes_are_byte_identical_and_cover_zero_tails_and_tags() {
        assert_eq!(b03_layout_probe(), b03_layout_probe());
        assert!(Assignment::NOOP.validate());
        for tag in 0..=8 {
            let mut assignment = Assignment::NOOP;
            assignment.destination_kind = tag;
            if tag != 0 {
                assignment.destination_arena = 1;
                assignment.image_len = 8;
                assignment.payload[0] = 1;
            }
            assert!(assignment.validate());
        }
        let mut invalid = Assignment::NOOP;
        invalid.destination_kind = 9;
        assert!(!invalid.validate());
        let mut tail = Assignment::NOOP;
        tail.destination_kind = DestinationKind::Leaf as u8;
        tail.image_len = 8;
        tail.payload[8] = 1;
        assert!(!tail.validate());
        assert_eq!(ScratchTag::LeafRoute as u8, 0);
        assert_eq!(ScratchTag::BranchRoute as u8, 1);
        assert_eq!(ScratchTag::Terminal as u8, 2);
    }

    #[test]
    fn cardinality_and_transient_products_are_exact() {
        assert_eq!(4 + 4 + 2, 10);
        assert_eq!(2 * MERGE_INITIAL_BUDGET, 2_304);
        assert_eq!(26_496 + 11 * POST_CREATE_BUDGET, FORMATION_CAPACITY);
        assert_eq!(ROOT_GROUP_CAPACITY * 4, MEMBER_CAPACITY);
        assert_eq!(FORMATION_CAPACITY * 4, FUNDER_CAPACITY);
        assert_eq!(
            ROOT_GROUP_CAPACITY
                + FUNDER_CAPACITY
                + LINK_CAPACITY
                + MEMBERSHIP_CAPACITY
                + MUTATION_CAPACITY,
            LOCAL_CAPACITY
        );
        assert_eq!(size_of::<BundleSnapshot>() * 32, 38_912);
        assert_eq!(size_of::<CellSnapshot>() * 192, 12_288);
        assert_eq!(size_of::<DirectRecordPair>() * 256, 163_840);
        assert_eq!(
            size_of::<ScratchTopologyNode>() * SPLIT_ROUTE_POSITIONS,
            245_952
        );
        assert_eq!(size_of::<Assignment>() * ORDINARY_ASSIGNMENTS, 50_304);
        assert_eq!(9 * 16 + 1, LIFECYCLE_ASSIGNMENTS);
        assert_eq!(LIFECYCLE_ASSIGNMENTS * size_of::<Assignment>(), 18_560);
        assert_eq!(16 * 258 * size_of::<ScratchTopologyNode>(), 132_096);
        let lifecycle_transient = 132_096
            + 18_560
            + 258 * LIFECYCLE_CHUNK_MAX
            + LIFECYCLE_CHUNK_MAX * size_of::<LifecycleRecordSlotImage>()
            + 16 * size_of::<Descriptor96>()
            + 3 * size_of::<LifecyclePage>();
        assert_eq!(lifecycle_transient, 175_760);
        assert_eq!(3 * lifecycle_transient, WORK_LIFECYCLE_STAGE[1] as usize);
        assert_eq!(3 * lifecycle_transient, WORK_LIFECYCLE_ABORT[1] as usize);
        assert_eq!(
            LIFECYCLE_BATCH_MAX * size_of::<BeginReservationImage>()
                + 3 * size_of::<LifecyclePage>()
                + 128 * size_of::<BeginFixedOutcome>(),
            WORK_LIFECYCLE_BEGIN[1] as usize
        );
        assert_eq!(
            LIFECYCLE_BATCH_MAX * size_of::<LifecycleRecordSlotImage>()
                + LIFECYCLE_BATCH_MAX * size_of::<VisibilityOutcome>()
                + 16 * size_of::<LifecyclePage>(),
            WORK_LIFECYCLE_FINALIZE[1] as usize
        );
        assert_eq!((LIFECYCLE_BATCH_MAX + 128) * 12, 13_824);
        assert_eq!(
            16 * 520 + 135 + LIFECYCLE_CHUNK_MAX * 24 + 8 * 4 * 16 + 64,
            9_223
        );
        assert_eq!(2 * LIFECYCLE_BATCH_MAX + 3, 2_051);
        assert_eq!(LIFECYCLE_ASSIGNMENTS + LIFECYCLE_CHUNK_MAX + 2, 155);
        assert_eq!(LIFECYCLE_ASSIGNMENTS + 2 * LIFECYCLE_CHUNK_MAX + 2, 163);
        assert_eq!(LIFECYCLE_BATCH_MAX + 128, 1_152);
        assert_eq!(WORK_LIFECYCLE_BEGIN, [1_152 + 2_051, 64_000, 0, 0, 13_824]);
        assert_eq!(WORK_LIFECYCLE_STAGE, [9_223 + 155, 527_280, 0, 0, 9_223]);
        assert_eq!(
            WORK_LIFECYCLE_FINALIZE,
            [1_152 + 1_152, 1_269_760, 0, 0, 13_824]
        );
        assert_eq!(WORK_LIFECYCLE_ABORT, [9_223 + 163, 527_280, 0, 0, 9_223]);
        assert_eq!(
            PREPARED_SPLIT_BYTES + VALIDATED_SPLIT_BYTES + PREPARED_SPLIT_BYTES,
            ORDINARY_COPIED_BYTES
        );
    }

    #[test]
    fn operation_work_rows_and_legacy_transform_are_closed() {
        let rows = [
            WORK_PLAN_CREATE,
            WORK_CREATE_STANDALONE,
            WORK_MERGE_INITIAL,
            WORK_NEWLY_ELIGIBLE,
            WORK_PLAN_DISPOSITION,
            WORK_RESOLVE_OBSERVATION,
            WORK_STATE_TRANSITION,
            WORK_JOIN_REBIND,
            WORK_SPLIT,
            WORK_MERGE,
            WORK_REMOVE_BOUND,
            WORK_REMOVE_ELIGIBLE,
            WORK_CLOSE,
            WORK_TOMBSTONE,
            WORK_MIGRATED_C16,
        ];
        assert!(
            rows.iter()
                .all(|row| row[1] == ORDINARY_COPIED_BYTES && row[2] == 0 && row[3] == 0)
        );
        assert_eq!(
            legacy_migrated([75, 366, 0, 0, 20]),
            Some([1_134, 1_616_904, 0, 0, 1_060])
        );
        assert_eq!(legacy_migrated([u64::MAX, 0, 0, 0, 0]), None);
    }
}
