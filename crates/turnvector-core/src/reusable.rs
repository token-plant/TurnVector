use crate::FixedStorageError;
use crate::c17_layout::{Assignment, DestinationKind};
use std::cmp::Ordering;
#[cfg(turnvector_c17_probe)]
use std::mem::offset_of;
use std::mem::{align_of, size_of};

const NODE_TAG_SHIFT: u32 = 30;
const NODE_INDEX_MASK: u32 = (1 << NODE_TAG_SHIFT) - 1;
const LEAF_TAG: u32 = 0;
const BRANCH_TAG: u32 = 1 << NODE_TAG_SHIFT;
const INVALID_TAG: u32 = 2 << NODE_TAG_SHIFT;
const SENTINEL_NODE: u32 = u32::MAX;
const FREE_BRANCH_CELL_FLAG: u32 = 1 << 31;
const BRANCH_SLOT_BYTES: usize = 40;
const INDEX_HEADER_BYTES: usize = 40;
const ARENA_HEADER_BYTES: usize = 32;
const BOX_SLICE_DESCRIPTORS: usize = 4;

const fn align8(value: usize) -> Option<usize> {
    match value.checked_add(7) {
        Some(value) => Some(value & !7),
        None => None,
    }
}

pub(crate) const fn leaf_bytes(key_bytes: usize, value_bytes: usize) -> Option<usize> {
    match 16usize.checked_add(key_bytes) {
        Some(prefix) => match prefix.checked_add(value_bytes) {
            Some(value) => align8(value),
            None => None,
        },
        None => None,
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(transparent)]
pub(crate) struct NodeHandle(u64);

impl NodeHandle {
    pub(crate) const SENTINEL: Self = Self(SENTINEL_NODE as u64);

    const fn new(node: u32, generation: u32) -> Self {
        Self((generation as u64) << 32 | node as u64)
    }

    pub(crate) const fn node(self) -> u32 {
        self.0 as u32
    }

    pub(crate) const fn generation(self) -> u32 {
        (self.0 >> 32) as u32
    }

    const fn is_sentinel(self) -> bool {
        self.node() == SENTINEL_NODE && self.generation() == 0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
struct ReusableIndexHeader {
    generation: u64,
    root: NodeHandle,
    occupied: u32,
    leaf_capacity: u32,
    branch_capacity: u32,
    free_leaf_len: u32,
    free_branch_len: u32,
    reserved: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub(crate) enum PatriciaEditKind {
    Insert = 1,
    Update = 2,
    Remove = 3,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PatriciaEdit<const K: usize, const V: usize> {
    Insert {
        key: [u8; K],
        value: [u8; V],
    },
    Update {
        key: [u8; K],
        handle: NodeHandle,
        value: [u8; V],
    },
    Remove {
        key: [u8; K],
    },
}

impl<const K: usize, const V: usize> PatriciaEdit<K, V> {
    pub(crate) const fn key(&self) -> &[u8; K] {
        match self {
            Self::Insert { key, .. } | Self::Update { key, .. } | Self::Remove { key } => key,
        }
    }

    pub(crate) const fn kind(&self) -> PatriciaEditKind {
        match self {
            Self::Insert { .. } => PatriciaEditKind::Insert,
            Self::Update { .. } => PatriciaEditKind::Update,
            Self::Remove { .. } => PatriciaEditKind::Remove,
        }
    }

    pub(crate) const fn value(&self) -> Option<&[u8; V]> {
        match self {
            Self::Insert { value, .. } | Self::Update { value, .. } => Some(value),
            Self::Remove { .. } => None,
        }
    }
}

/// Semantic order retained while an assignment plan is sealed. The public
/// Assignment ABI remains exactly 128 bytes; this key is preparation metadata
/// used to assemble the one globally ordered cross-owner journal.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct AssignmentOrderKey {
    key: [u8; 40],
    key_len: u8,
    edit_kind: u8,
    value: [u8; 56],
    value_len: u8,
    arena: u16,
    ordinal: u16,
}

impl AssignmentOrderKey {
    pub(crate) const ZERO: Self = Self {
        key: [0; 40],
        key_len: 0,
        edit_kind: 0,
        value: [0; 56],
        value_len: 0,
        arena: 0,
        ordinal: 0,
    };

    fn edit(
        arena: u16,
        key: &[u8],
        edit_kind: PatriciaEditKind,
        value: &[u8],
    ) -> Result<Self, FixedStorageError> {
        if arena == 0 || key.is_empty() || key.len() > 40 || value.len() > 56 {
            return Err(FixedStorageError::Capacity);
        }
        let mut order = Self::ZERO;
        order.key[..key.len()].copy_from_slice(key);
        order.key_len = key.len() as u8;
        order.edit_kind = edit_kind as u8;
        order.value[..value.len()].copy_from_slice(value);
        order.value_len = value.len() as u8;
        order.arena = arena;
        Ok(order)
    }

    pub(crate) fn generation(arena: u16) -> Result<Self, FixedStorageError> {
        if arena == 0 {
            return Err(FixedStorageError::Capacity);
        }
        Ok(Self {
            key: [u8::MAX; 40],
            key_len: 40,
            edit_kind: u8::MAX,
            value: [u8::MAX; 56],
            value_len: 56,
            arena,
            ordinal: 0,
        })
    }

    pub(crate) const fn arena_id(self) -> u16 {
        self.arena
    }

    pub(crate) const fn assignment_ordinal(self) -> u16 {
        self.ordinal
    }

    pub(crate) fn is_generation(self) -> bool {
        self.key_len == 40
            && self.key == [u8::MAX; 40]
            && self.edit_kind == u8::MAX
            && self.value_len == 56
            && self.value == [u8::MAX; 56]
    }

    pub(crate) fn semantic_cmp(&self, other: &Self) -> Ordering {
        let left_key = &self.key[..usize::from(self.key_len)];
        let right_key = &other.key[..usize::from(other.key_len)];
        left_key
            .cmp(right_key)
            .then_with(|| self.edit_kind.cmp(&other.edit_kind))
            .then_with(|| {
                self.value[..usize::from(self.value_len)]
                    .cmp(&other.value[..usize::from(other.value_len)])
            })
    }

    fn total_cmp(&self, other: &Self) -> Ordering {
        self.semantic_cmp(other)
            .then_with(|| self.arena.cmp(&other.arena))
            .then_with(|| self.ordinal.cmp(&other.ordinal))
    }
}

const ASSIGNMENT_EDIT_MAX: usize = 49;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct AssignmentOrderRef {
    edit: u8,
    ordinal: u16,
}

impl AssignmentOrderRef {
    const ZERO: Self = Self {
        edit: 0,
        ordinal: 0,
    };
}

impl Ord for AssignmentOrderKey {
    fn cmp(&self, other: &Self) -> Ordering {
        self.total_cmp(other)
    }
}

impl PartialOrd for AssignmentOrderKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// A fixed, non-allocating journal of exact post-images for one Patricia edit
/// batch. Construction performs all lookup and structural validation. Commit
/// first checks every direct destination and then copies only these images; it
/// never searches the tree or returns an error after the first write.
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct PatriciaAssignmentPlan<const N: usize> {
    arena: u16,
    expected_generation: u64,
    after_header: ReusableIndexHeader,
    assignments: [Assignment; N],
    order_refs: [AssignmentOrderRef; N],
    edits: [AssignmentOrderKey; ASSIGNMENT_EDIT_MAX],
    len: usize,
    edit_len: usize,
    current_edit: u8,
    current_is_generation: bool,
    prior_edit: AssignmentOrderKey,
    has_current_edit: bool,
    has_prior_edit: bool,
    next_ordinal: u16,
}
