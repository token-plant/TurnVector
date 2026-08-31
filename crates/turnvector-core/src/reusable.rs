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

impl<const N: usize> PatriciaAssignmentPlan<N> {
    fn new(
        arena: u16,
        expected_generation: u64,
        after_header: ReusableIndexHeader,
    ) -> Result<Self, FixedStorageError> {
        if arena == 0 || N == 0 {
            return Err(FixedStorageError::Capacity);
        }
        Ok(Self {
            arena,
            expected_generation,
            after_header,
            assignments: [Assignment::NOOP; N],
            order_refs: [AssignmentOrderRef::ZERO; N],
            edits: [AssignmentOrderKey::ZERO; ASSIGNMENT_EDIT_MAX],
            len: 0,
            edit_len: 0,
            current_edit: 0,
            current_is_generation: false,
            prior_edit: AssignmentOrderKey::ZERO,
            has_current_edit: false,
            has_prior_edit: false,
            next_ordinal: 0,
        })
    }

    pub(crate) fn assignments(&self) -> &[Assignment] {
        &self.assignments[..self.len]
    }

    #[cfg(test)]
    pub(crate) fn corrupt_first_assignment_payload_for_test(&mut self) {
        self.assignments[0].payload[0] ^= 1;
    }

    pub(crate) fn visit_assignments(
        &self,
        visitor: &mut dyn FnMut(AssignmentOrderKey, Assignment),
    ) {
        for index in 0..self.len {
            visitor(
                self.resolve_order_ref(self.order_refs[index])
                    .expect("sealed assignment order reference"),
                self.assignments[index],
            );
        }
    }

    fn append_edit(
        &mut self,
        edit: AssignmentOrderKey,
        generation: bool,
    ) -> Result<(), FixedStorageError> {
        if self.edit_len == ASSIGNMENT_EDIT_MAX {
            return Err(FixedStorageError::Capacity);
        }
        self.current_edit = u8::try_from(self.edit_len).map_err(|_| FixedStorageError::Capacity)?;
        self.current_is_generation = generation;
        self.edits[self.edit_len] = edit;
        self.edit_len += 1;
        self.has_current_edit = true;
        self.next_ordinal = 0;
        Ok(())
    }

    fn finish_current_envelope(&mut self) -> Result<(), FixedStorageError> {
        if !self.has_current_edit || self.current_is_generation {
            return Ok(());
        }
        if self.next_ordinal > 9 {
            return Err(FixedStorageError::Capacity);
        }
        while self.next_ordinal < 9 {
            if self.len == N {
                return Err(FixedStorageError::Capacity);
            }
            self.order_refs[self.len] = self.next_order_ref()?;
            self.assignments[self.len] = Assignment::NOOP;
            self.len += 1;
        }
        Ok(())
    }

    fn begin_edit(
        &mut self,
        key: &[u8],
        kind: PatriciaEditKind,
        value: &[u8],
    ) -> Result<(), FixedStorageError> {
        let current = AssignmentOrderKey::edit(self.arena, key, kind, value)?;
        if self.has_prior_edit && self.prior_edit.semantic_cmp(&current) != Ordering::Less {
            return Err(FixedStorageError::Duplicate);
        }
        self.finish_current_envelope()?;
        self.append_edit(current, false)?;
        self.prior_edit = current;
        self.has_prior_edit = true;
        Ok(())
    }

    fn begin_generation(&mut self) -> Result<(), FixedStorageError> {
        self.finish_current_envelope()?;
        self.append_edit(AssignmentOrderKey::generation(self.arena)?, true)
    }

    fn next_order_ref(&mut self) -> Result<AssignmentOrderRef, FixedStorageError> {
        if !self.has_current_edit
            || (!self.current_is_generation && self.next_ordinal >= 9)
            || (self.current_is_generation && self.next_ordinal >= 1)
        {
            return Err(FixedStorageError::NonCanonical);
        }
        let ordinal = self.next_ordinal;
        self.next_ordinal = self
            .next_ordinal
            .checked_add(1)
            .ok_or(FixedStorageError::Capacity)?;
        Ok(AssignmentOrderRef {
            edit: self.current_edit,
            ordinal,
        })
    }

    fn resolve_order_ref(&self, reference: AssignmentOrderRef) -> Option<AssignmentOrderKey> {
        let mut key = *self.edits.get(usize::from(reference.edit))?;
        (usize::from(reference.edit) < self.edit_len).then(|| {
            key.ordinal = reference.ordinal;
            key
        })
    }

    fn find_destination(&self, kind: DestinationKind, slot: u32) -> Option<usize> {
        self.assignments().iter().rposition(|assignment| {
            assignment.destination_kind == kind as u8 && assignment.destination_slot == slot
        })
    }

    fn image(&self, kind: DestinationKind, slot: u32) -> Option<&[u8]> {
        let assignment = &self.assignments[self.find_destination(kind, slot)?];
        Some(&assignment.payload[..usize::from(assignment.image_len)])
    }

    fn set(
        &mut self,
        kind: DestinationKind,
        slot: u32,
        expected_generation: u64,
        image: &[u8],
    ) -> Result<(), FixedStorageError> {
        if !matches!(image.len(), 4 | 8 | 40 | 48 | 56 | 64 | 112) {
            return Err(FixedStorageError::Capacity);
        }
        let order_ref = self.next_order_ref()?;
        let mut payload = [0; 112];
        payload[..image.len()].copy_from_slice(image);
        let assignment = Assignment {
            destination_arena: self.arena,
            destination_kind: kind as u8,
            image_len: image.len() as u8,
            destination_slot: slot,
            expected_generation,
            payload,
        };
        if !assignment.validate() {
            return Err(FixedStorageError::NonCanonical);
        }
        if self.len == N {
            return Err(FixedStorageError::Capacity);
        }
        self.assignments[self.len] = assignment;
        self.order_refs[self.len] = order_ref;
        self.len += 1;
        Ok(())
    }

    fn finish(&mut self) -> Result<(), FixedStorageError> {
        for index in 1..self.len {
            let mut cursor = index;
            while cursor > 0 {
                let current = self
                    .resolve_order_ref(self.order_refs[cursor])
                    .ok_or(FixedStorageError::NonCanonical)?;
                let prior = self
                    .resolve_order_ref(self.order_refs[cursor - 1])
                    .ok_or(FixedStorageError::NonCanonical)?;
                if current >= prior {
                    break;
                }
                self.order_refs.swap(cursor - 1, cursor);
                self.assignments.swap(cursor - 1, cursor);
                cursor -= 1;
            }
        }
        if self.edit_len == 0
            || !self.current_is_generation
            || self.next_ordinal != 1
            || self.len != (self.edit_len - 1) * 9 + 1
            || !self.assignments().iter().all(Assignment::validate)
            || (0..self.len).any(|index| {
                self.resolve_order_ref(self.order_refs[index]).is_none()
                    || index > 0
                        && self
                            .resolve_order_ref(self.order_refs[index - 1])
                            .zip(self.resolve_order_ref(self.order_refs[index]))
                            .is_none_or(|(prior, current)| prior >= current)
            })
            || self.edits[..self.edit_len]
                .windows(2)
                .any(|pair| pair[0] >= pair[1])
            || self.edits[..self.edit_len]
                .iter()
                .any(|edit| edit.arena != self.arena)
            || self.assignments[self.len..]
                .iter()
                .any(|assignment| *assignment != Assignment::NOOP)
            || self.order_refs[self.len..]
                .iter()
                .any(|reference| *reference != AssignmentOrderRef::ZERO)
            || self.edits[self.edit_len..]
                .iter()
                .any(|edit| *edit != AssignmentOrderKey::ZERO)
        {
            return Err(FixedStorageError::NonCanonical);
        }
        Ok(())
    }
}

/// One fixed-capacity, generation-bearing Patricia over an exact-width key and
/// canonical byte value. Every persistent node is a manual byte image; Rust
/// enum layout and native padding never participate in persistence.
#[derive(Clone, Debug, Eq, PartialEq)]
#[repr(C)]
pub(crate) struct ReusablePatricia<const K: usize, const V: usize> {
    header: ReusableIndexHeader,
    leaves: Box<[u8]>,
    branches: Box<[u8]>,
    free_leaves: Box<[u8]>,
    free_branches: Box<[u8]>,
}

impl<const K: usize, const V: usize> ReusablePatricia<K, V> {
    pub(crate) fn storage_bytes(capacity: usize) -> Option<u64> {
        if capacity == 0 || capacity >= (1 << NODE_TAG_SHIFT) {
            return None;
        }
        let leaf = u64::try_from(leaf_bytes(K, V)?).ok()?;
        let leaves = u64::try_from(capacity).ok()?;
        let branches = leaves.checked_sub(1)?;
        u64::try_from(INDEX_HEADER_BYTES + BOX_SLICE_DESCRIPTORS * size_of::<Box<[u8]>>())
            .ok()?
            .checked_add(leaves.checked_mul(leaf)?)?
            .checked_add(branches.checked_mul(BRANCH_SLOT_BYTES as u64)?)?
            .checked_add(leaves.checked_mul(size_of::<u32>() as u64)?)?
            .checked_add(branches.checked_mul(size_of::<u32>() as u64)?)
    }

    pub(crate) fn try_new(capacity: usize) -> Result<Self, FixedStorageError> {
        let leaf_width = leaf_bytes(K, V).ok_or(FixedStorageError::Capacity)?;
        let branch_capacity = capacity.checked_sub(1).ok_or(FixedStorageError::Capacity)?;
        if capacity >= (1 << NODE_TAG_SHIFT)
            || branch_capacity >= (1 << NODE_TAG_SHIFT)
            || K == 0
            || V == 0
        {
            return Err(FixedStorageError::Capacity);
        }
        let leaf_backing = capacity
            .checked_mul(leaf_width)
            .ok_or(FixedStorageError::Capacity)?;
        let branch_backing = branch_capacity
            .checked_mul(BRANCH_SLOT_BYTES)
            .ok_or(FixedStorageError::Capacity)?;
        let free_leaf_backing = capacity
            .checked_mul(size_of::<u32>())
            .ok_or(FixedStorageError::Capacity)?;
        let free_branch_backing = branch_capacity
            .checked_mul(size_of::<u32>())
            .ok_or(FixedStorageError::Capacity)?;
        if [
            leaf_backing,
            branch_backing,
            free_leaf_backing,
            free_branch_backing,
        ]
        .into_iter()
        .any(|bytes| bytes > isize::MAX as usize)
        {
            return Err(FixedStorageError::Capacity);
        }
        let mut value = Self {
            header: ReusableIndexHeader {
                generation: 0,
                root: NodeHandle::SENTINEL,
                occupied: 0,
                leaf_capacity: u32::try_from(capacity).map_err(|_| FixedStorageError::Capacity)?,
                branch_capacity: u32::try_from(branch_capacity)
                    .map_err(|_| FixedStorageError::Capacity)?,
                free_leaf_len: u32::try_from(capacity).map_err(|_| FixedStorageError::Capacity)?,
                free_branch_len: u32::try_from(branch_capacity)
                    .map_err(|_| FixedStorageError::Capacity)?,
                reserved: 0,
            },
            leaves: zeroed(leaf_backing)?,
            branches: zeroed(branch_backing)?,
            free_leaves: zeroed(free_leaf_backing)?,
            free_branches: zeroed(free_branch_backing)?,
        };
        for index in 0..capacity {
            let position = capacity - 1 - index;
            value.write_leaf_vacant(index, 0, position as u32);
            write_u32(&mut value.free_leaves, position * 4, index as u32);
        }
        for index in 0..branch_capacity {
            let position = branch_capacity - 1 - index;
            value.write_branch_vacant(index, 0, position as u32);
            write_u32(&mut value.free_branches, position * 4, index as u32);
        }
        value.validate_header()?;
        Ok(value)
    }

    #[cfg(test)]
    fn try_new_with_backing_lengths(
        capacity: usize,
        lengths: [usize; 4],
    ) -> Result<Self, FixedStorageError> {
        let expected = [
            capacity
                .checked_mul(leaf_bytes(K, V).ok_or(FixedStorageError::Capacity)?)
                .ok_or(FixedStorageError::Capacity)?,
            capacity
                .checked_sub(1)
                .and_then(|value| value.checked_mul(BRANCH_SLOT_BYTES))
                .ok_or(FixedStorageError::Capacity)?,
            capacity
                .checked_mul(size_of::<u32>())
                .ok_or(FixedStorageError::Capacity)?,
            capacity
                .checked_sub(1)
                .and_then(|value| value.checked_mul(size_of::<u32>()))
                .ok_or(FixedStorageError::Capacity)?,
        ];
        if lengths != expected {
            return Err(FixedStorageError::Capacity);
        }
        Self::try_new(capacity)
    }

    pub(crate) const fn generation(&self) -> u64 {
        self.header.generation
    }

    #[cfg_attr(
        not(test),
        allow(dead_code, reason = "C17 structural tests consume this observer")
    )]
    pub(crate) const fn len(&self) -> usize {
        self.header.occupied as usize
    }

    #[cfg_attr(
        not(test),
        allow(dead_code, reason = "C17 structural tests consume this observer")
    )]
    pub(crate) const fn capacity(&self) -> usize {
        self.header.leaf_capacity as usize
    }

    #[cfg_attr(
        not(test),
        allow(dead_code, reason = "C17 structural tests consume this observer")
    )]
    pub(crate) const fn free_leaf_len(&self) -> usize {
        self.header.free_leaf_len as usize
    }

    #[cfg_attr(
        not(test),
        allow(dead_code, reason = "C17 structural tests consume this observer")
    )]
    pub(crate) const fn free_branch_len(&self) -> usize {
        self.header.free_branch_len as usize
    }

    pub(crate) fn find(&self, key: &[u8; K]) -> Result<Option<[u8; V]>, FixedStorageError> {
        let Some(handle) = self.find_handle(key)? else {
            return Ok(None);
        };
        let slot = self.leaf_slot(handle)?;
        let mut value = [0; V];
        value.copy_from_slice(&slot[16 + K..16 + K + V]);
        Ok(Some(value))
    }

    pub(crate) fn find_handle(
        &self,
        key: &[u8; K],
    ) -> Result<Option<NodeHandle>, FixedStorageError> {
        self.validate_header()?;
        if self.header.root.is_sentinel() {
            return Ok(None);
        }
        let (leaf, _) = self.locate(key)?;
        let slot = self.leaf_slot(leaf)?;
        Ok((slot[16..16 + K] == key[..]).then_some(leaf))
    }

    pub(crate) fn validate_insert(&self, key: &[u8; K]) -> Result<(), FixedStorageError> {
        self.validate_header()?;
        self.header
            .generation
            .checked_add(1)
            .ok_or(FixedStorageError::Capacity)?;
        self.selected_free_leaf()?;
        if self.header.root.is_sentinel() {
            return Ok(());
        }
        let (leaf, _) = self.locate(key)?;
        let slot = self.leaf_slot(leaf)?;
        if slot[16..16 + K] == key[..] {
            return Err(FixedStorageError::Duplicate);
        }
        self.selected_free_branch()?;
        first_difference(key, &slot[16..16 + K])?;
        Ok(())
    }

    pub(crate) fn validate_insert_batch(
        &self,
        entries: &[([u8; K], [u8; V])],
    ) -> Result<(), FixedStorageError> {
        self.validate_header()?;
        if entries.is_empty()
            || entries.len() > self.header.free_leaf_len as usize
            || entries.len()
                > self.header.free_branch_len as usize + usize::from(self.header.root.is_sentinel())
        {
            return Err(FixedStorageError::Capacity);
        }
        self.header
            .generation
            .checked_add(1)
            .ok_or(FixedStorageError::Capacity)?;
        for (index, (key, _)) in entries.iter().enumerate() {
            if entries[..index].iter().any(|(prior, _)| prior == key) {
                return Err(FixedStorageError::Duplicate);
            }
            if self.find(key)?.is_some() {
                return Err(FixedStorageError::Duplicate);
            }
            let position = self.header.free_leaf_len as usize - 1 - index;
            let slot_index = read_u32(&self.free_leaves, position * 4) as usize;
            let slot = self.leaf_bytes_at(slot_index);
            if slot[0] != 0 || read_u32(slot, 8) as usize != position {
                return Err(FixedStorageError::NonCanonical);
            }
            read_u32(slot, 4)
                .checked_add(1)
                .ok_or(FixedStorageError::Capacity)?;
        }
        let branches = entries
            .len()
            .checked_sub(usize::from(self.header.root.is_sentinel()))
            .ok_or(FixedStorageError::NonCanonical)?;
        for index in 0..branches {
            let position = self.header.free_branch_len as usize - 1 - index;
            let slot_index = read_u32(&self.free_branches, position * 4) as usize;
            let slot = self.branch_bytes_at(slot_index);
            if slot[0] != 0 || read_u32(slot, 8) as usize != position {
                return Err(FixedStorageError::NonCanonical);
            }
            read_u32(slot, 4)
                .checked_add(1)
                .ok_or(FixedStorageError::Capacity)?;
        }
        Ok(())
    }

    pub(crate) fn prepare_insert_assignment_plan<const N: usize>(
        &self,
        arena: u16,
        entries: &[([u8; K], [u8; V])],
    ) -> Result<PatriciaAssignmentPlan<N>, FixedStorageError> {
        self.validate_insert_batch(entries)?;
        if entries.windows(2).any(|pair| pair[0].0 >= pair[1].0) {
            return Err(FixedStorageError::Duplicate);
        }
        let mut header = self.header;
        let mut plan = PatriciaAssignmentPlan::new(arena, header.generation, header)?;
        for (key, value) in entries.iter().copied() {
            plan.begin_edit(&key, PatriciaEditKind::Insert, &value)?;
            let empty = header.root.is_sentinel();
            let terminal = if empty {
                None
            } else {
                let (leaf, _) = self.shadow_locate(&plan, header.root, &key)?;
                let slot = self.shadow_leaf(&plan, leaf)?;
                if slot[16..16 + K] == key {
                    return Err(FixedStorageError::Duplicate);
                }
                Some((leaf, slot))
            };
            let leaf_position = header
                .free_leaf_len
                .checked_sub(1)
                .ok_or(FixedStorageError::Capacity)? as usize;
            let leaf_index = read_u32(&self.free_leaves, leaf_position * 4) as usize;
            let vacant_leaf = self.leaf_bytes_at(leaf_index);
            if vacant_leaf[0] != 0 || read_u32(vacant_leaf, 8) as usize != leaf_position {
                return Err(FixedStorageError::NonCanonical);
            }
            let leaf_generation = read_u32(vacant_leaf, 4)
                .checked_add(1)
                .ok_or(FixedStorageError::Capacity)?;
            let leaf_handle = NodeHandle::new(leaf_index as u32 | LEAF_TAG, leaf_generation);
            header.free_leaf_len -= 1;
            if empty {
                let image = occupied_leaf_image::<K, V>(
                    leaf_generation,
                    NodeHandle::SENTINEL,
                    &key,
                    &value,
                )?;
                self.shadow_set_leaf(&mut plan, leaf_index, &image)?;
                header.root = leaf_handle;
            } else {
                let (terminal, peer) = terminal.expect("nonempty shadow has terminal");
                let bit = first_difference(&key, &peer[16..16 + K])?;
                let (parent, child) = self.shadow_insertion_point(&plan, header.root, &key, bit)?;
                let branch_position = header
                    .free_branch_len
                    .checked_sub(1)
                    .ok_or(FixedStorageError::Capacity)?
                    as usize;
                let branch_index = read_u32(&self.free_branches, branch_position * 4) as usize;
                let vacant_branch = self.branch_bytes_at(branch_index);
                if vacant_branch[0] != 0 || read_u32(vacant_branch, 8) as usize != branch_position {
                    return Err(FixedStorageError::NonCanonical);
                }
                let branch_generation = read_u32(vacant_branch, 4)
                    .checked_add(1)
                    .ok_or(FixedStorageError::Capacity)?;
                let branch_handle =
                    NodeHandle::new(branch_index as u32 | BRANCH_TAG, branch_generation);
                header.free_branch_len -= 1;
                let children = if key_bit(&key, bit) == 0 {
                    [leaf_handle, child]
                } else {
                    [child, leaf_handle]
                };
                let leaf_image =
                    occupied_leaf_image::<K, V>(leaf_generation, branch_handle, &key, &value)?;
                let branch_image = occupied_branch_image(branch_generation, parent, bit, children);
                self.shadow_set_leaf(&mut plan, leaf_index, &leaf_image)?;
                self.shadow_set_branch(&mut plan, branch_index, &branch_image)?;
                self.shadow_set_parent(&mut plan, child, branch_handle)?;
                if parent.is_sentinel() {
                    header.root = branch_handle;
                } else {
                    self.shadow_replace_child(&mut plan, parent, child, branch_handle)?;
                }
                let _ = terminal;
            }
            header.occupied = header
                .occupied
                .checked_add(1)
                .ok_or(FixedStorageError::Capacity)?;
        }
        header.generation = header
            .generation
            .checked_add(1)
            .ok_or(FixedStorageError::Capacity)?;
        plan.after_header = header;
        plan.begin_generation()?;
        plan.set(
            DestinationKind::Header,
            0,
            self.header.generation,
            &encode_index_header(header),
        )?;
        plan.finish()?;
        Ok(plan)
    }

    pub(crate) fn prepare_update_assignment_plan<const N: usize>(
        &self,
        arena: u16,
        entries: &[([u8; K], NodeHandle, [u8; V])],
    ) -> Result<PatriciaAssignmentPlan<N>, FixedStorageError> {
        self.validate_update_batch(entries)?;
        if entries.windows(2).any(|pair| pair[0].0 >= pair[1].0) {
            return Err(FixedStorageError::Duplicate);
        }
        let mut header = self.header;
        let mut plan = PatriciaAssignmentPlan::new(arena, header.generation, header)?;
        for (key, handle, value) in entries.iter().copied() {
            plan.begin_edit(&key, PatriciaEditKind::Update, &value)?;
            let mut image = self.shadow_leaf(&plan, handle)?;
            image[16 + K..16 + K + V].copy_from_slice(&value);
            self.shadow_set_leaf(&mut plan, leaf_index(handle)?, &image)?;
        }
        header.generation = header
            .generation
            .checked_add(1)
            .ok_or(FixedStorageError::Capacity)?;
        plan.after_header = header;
        plan.begin_generation()?;
        plan.set(
            DestinationKind::Header,
            0,
            self.header.generation,
            &encode_index_header(header),
        )?;
        plan.finish()?;
        Ok(plan)
    }

    pub(crate) fn prepare_generation_assignment_plan<const N: usize>(
        &self,
        arena: u16,
    ) -> Result<PatriciaAssignmentPlan<N>, FixedStorageError> {
        self.validate_advance_generation()?;
        let mut header = self.header;
        header.generation = header
            .generation
            .checked_add(1)
            .ok_or(FixedStorageError::Capacity)?;
        let mut plan = PatriciaAssignmentPlan::new(arena, self.header.generation, header)?;
        plan.begin_generation()?;
        plan.set(
            DestinationKind::Header,
            0,
            self.header.generation,
            &encode_index_header(header),
        )?;
        plan.finish()?;
        Ok(plan)
    }

    pub(crate) fn prepare_remove_assignment_plan<const N: usize>(
        &self,
        arena: u16,
        keys: &[[u8; K]],
    ) -> Result<PatriciaAssignmentPlan<N>, FixedStorageError> {
        self.validate_remove_batch(keys)?;
        if keys.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(FixedStorageError::Duplicate);
        }
        let mut header = self.header;
        let mut plan = PatriciaAssignmentPlan::new(arena, header.generation, header)?;
        for key in keys {
            plan.begin_edit(key, PatriciaEditKind::Remove, &[])?;
            let path = self.shadow_removal_path(&plan, header.root, key)?;
            let leaf_image = self.shadow_leaf(&plan, path.leaf)?;
            if leaf_image[16..16 + K] != key[..] {
                return Err(FixedStorageError::NonCanonical);
            }
            if path.parent.is_sentinel() {
                if !path.grandparent.is_sentinel() || !path.sibling.is_sentinel() {
                    return Err(FixedStorageError::NonCanonical);
                }
                header.root = NodeHandle::SENTINEL;
            } else {
                if path.sibling.is_sentinel() {
                    return Err(FixedStorageError::NonCanonical);
                }
                self.shadow_set_parent(&mut plan, path.sibling, path.grandparent)?;
                if path.grandparent.is_sentinel() {
                    header.root = path.sibling;
                } else {
                    self.shadow_replace_child(
                        &mut plan,
                        path.grandparent,
                        path.parent,
                        path.sibling,
                    )?;
                }
                let branch_index = branch_index(path.parent)?;
                let branch_image = self.shadow_branch(&plan, path.parent)?;
                let generation = read_u32(&branch_image, 4);
                let free_position = header.free_branch_len;
                let vacant = vacant_branch_image(generation, free_position);
                self.shadow_set_branch(&mut plan, branch_index, &vacant)?;
                plan.set(
                    DestinationKind::FreeCell,
                    FREE_BRANCH_CELL_FLAG | free_position,
                    self.header.generation,
                    &(branch_index as u32).to_le_bytes(),
                )?;
                header.free_branch_len = header
                    .free_branch_len
                    .checked_add(1)
                    .ok_or(FixedStorageError::Capacity)?;
            }
            let leaf_index = leaf_index(path.leaf)?;
            let generation = read_u32(&leaf_image, 4);
            let free_position = header.free_leaf_len;
            let vacant = vacant_leaf_image::<K, V>(generation, free_position)?;
            self.shadow_set_leaf(&mut plan, leaf_index, &vacant)?;
            plan.set(
                DestinationKind::FreeCell,
                free_position,
                self.header.generation,
                &(leaf_index as u32).to_le_bytes(),
            )?;
            header.free_leaf_len = header
                .free_leaf_len
                .checked_add(1)
                .ok_or(FixedStorageError::Capacity)?;
            header.occupied = header
                .occupied
                .checked_sub(1)
                .ok_or(FixedStorageError::NonCanonical)?;
        }
        header.generation = header
            .generation
            .checked_add(1)
            .ok_or(FixedStorageError::Capacity)?;
        plan.after_header = header;
        plan.begin_generation()?;
        plan.set(
            DestinationKind::Header,
            0,
            self.header.generation,
            &encode_index_header(header),
        )?;
        plan.finish()?;
        Ok(plan)
    }

    pub(crate) fn prepare_mixed_assignment_plan<const N: usize>(
        &self,
        arena: u16,
        edits: &[PatriciaEdit<K, V>],
    ) -> Result<PatriciaAssignmentPlan<N>, FixedStorageError> {
        self.validate_header()?;
        if edits.is_empty() {
            return Err(FixedStorageError::Capacity);
        }
        let mut header = self.header;
        let mut plan = PatriciaAssignmentPlan::new(arena, header.generation, header)?;
        for edit in edits.iter().copied() {
            let value = edit.value().map_or(&[][..], |value| value.as_slice());
            plan.begin_edit(edit.key(), edit.kind(), value)?;
            match edit {
                PatriciaEdit::Insert { key, value } => {
                    self.shadow_apply_insert(&mut plan, &mut header, key, value)?;
                }
                PatriciaEdit::Update { key, handle, value } => {
                    let mut image = self.shadow_leaf(&plan, handle)?;
                    if image[16..16 + K] != key {
                        return Err(FixedStorageError::NonCanonical);
                    }
                    image[16 + K..16 + K + V].copy_from_slice(&value);
                    self.shadow_set_leaf(&mut plan, leaf_index(handle)?, &image)?;
                }
                PatriciaEdit::Remove { key } => {
                    self.shadow_apply_remove(&mut plan, &mut header, &key)?;
                }
            }
        }
        header.generation = header
            .generation
            .checked_add(1)
            .ok_or(FixedStorageError::Capacity)?;
        plan.after_header = header;
        plan.begin_generation()?;
        plan.set(
            DestinationKind::Header,
            0,
            self.header.generation,
            &encode_index_header(header),
        )?;
        plan.finish()?;
        Ok(plan)
    }

    fn shadow_apply_insert<const N: usize>(
        &self,
        plan: &mut PatriciaAssignmentPlan<N>,
        header: &mut ReusableIndexHeader,
        key: [u8; K],
        value: [u8; V],
    ) -> Result<(), FixedStorageError> {
        let empty = header.root.is_sentinel();
        let terminal = if empty {
            None
        } else {
            let (leaf, _) = self.shadow_locate(plan, header.root, &key)?;
            let slot = self.shadow_leaf(plan, leaf)?;
            if slot[16..16 + K] == key {
                return Err(FixedStorageError::Duplicate);
            }
            Some((leaf, slot))
        };
        let leaf_position = header
            .free_leaf_len
            .checked_sub(1)
            .ok_or(FixedStorageError::Capacity)? as usize;
        let leaf_index = read_u32(&self.free_leaves, leaf_position * 4) as usize;
        let vacant_leaf = self.leaf_bytes_at(leaf_index);
        if vacant_leaf[0] != 0 || read_u32(vacant_leaf, 8) as usize != leaf_position {
            return Err(FixedStorageError::NonCanonical);
        }
        let leaf_generation = read_u32(vacant_leaf, 4)
            .checked_add(1)
            .ok_or(FixedStorageError::Capacity)?;
        let leaf_handle = NodeHandle::new(leaf_index as u32 | LEAF_TAG, leaf_generation);
        header.free_leaf_len -= 1;
        if empty {
            let image =
                occupied_leaf_image::<K, V>(leaf_generation, NodeHandle::SENTINEL, &key, &value)?;
            self.shadow_set_leaf(plan, leaf_index, &image)?;
            header.root = leaf_handle;
        } else {
            let (terminal, peer) = terminal.expect("nonempty shadow has terminal");
            let bit = first_difference(&key, &peer[16..16 + K])?;
            let (parent, child) = self.shadow_insertion_point(plan, header.root, &key, bit)?;
            let branch_position = header
                .free_branch_len
                .checked_sub(1)
                .ok_or(FixedStorageError::Capacity)? as usize;
            let branch_index = read_u32(&self.free_branches, branch_position * 4) as usize;
            let vacant_branch = self.branch_bytes_at(branch_index);
            if vacant_branch[0] != 0 || read_u32(vacant_branch, 8) as usize != branch_position {
                return Err(FixedStorageError::NonCanonical);
            }
            let branch_generation = read_u32(vacant_branch, 4)
                .checked_add(1)
                .ok_or(FixedStorageError::Capacity)?;
            let branch_handle =
                NodeHandle::new(branch_index as u32 | BRANCH_TAG, branch_generation);
            header.free_branch_len -= 1;
            let children = if key_bit(&key, bit) == 0 {
                [leaf_handle, child]
            } else {
                [child, leaf_handle]
            };
            let leaf_image =
                occupied_leaf_image::<K, V>(leaf_generation, branch_handle, &key, &value)?;
            let branch_image = occupied_branch_image(branch_generation, parent, bit, children);
            self.shadow_set_leaf(plan, leaf_index, &leaf_image)?;
            self.shadow_set_branch(plan, branch_index, &branch_image)?;
            self.shadow_set_parent(plan, child, branch_handle)?;
            if parent.is_sentinel() {
                header.root = branch_handle;
            } else {
                self.shadow_replace_child(plan, parent, child, branch_handle)?;
            }
            let _ = terminal;
        }
        header.occupied = header
            .occupied
            .checked_add(1)
            .ok_or(FixedStorageError::Capacity)?;
        Ok(())
    }

    fn shadow_apply_remove<const N: usize>(
        &self,
        plan: &mut PatriciaAssignmentPlan<N>,
        header: &mut ReusableIndexHeader,
        key: &[u8; K],
    ) -> Result<(), FixedStorageError> {
        let path = self.shadow_removal_path(plan, header.root, key)?;
        let leaf_image = self.shadow_leaf(plan, path.leaf)?;
        if leaf_image[16..16 + K] != key[..] {
            return Err(FixedStorageError::NonCanonical);
        }
        if path.parent.is_sentinel() {
            if !path.grandparent.is_sentinel() || !path.sibling.is_sentinel() {
                return Err(FixedStorageError::NonCanonical);
            }
            header.root = NodeHandle::SENTINEL;
        } else {
            if path.sibling.is_sentinel() {
                return Err(FixedStorageError::NonCanonical);
            }
            self.shadow_set_parent(plan, path.sibling, path.grandparent)?;
            if path.grandparent.is_sentinel() {
                header.root = path.sibling;
            } else {
                self.shadow_replace_child(plan, path.grandparent, path.parent, path.sibling)?;
            }
            let branch_index = branch_index(path.parent)?;
            let branch_image = self.shadow_branch(plan, path.parent)?;
            let generation = read_u32(&branch_image, 4);
            let free_position = header.free_branch_len;
            let vacant = vacant_branch_image(generation, free_position);
            self.shadow_set_branch(plan, branch_index, &vacant)?;
            plan.set(
                DestinationKind::FreeCell,
                FREE_BRANCH_CELL_FLAG | free_position,
                self.header.generation,
                &(branch_index as u32).to_le_bytes(),
            )?;
            header.free_branch_len = header
                .free_branch_len
                .checked_add(1)
                .ok_or(FixedStorageError::Capacity)?;
        }
        let leaf_index = leaf_index(path.leaf)?;
        let generation = read_u32(&leaf_image, 4);
        let free_position = header.free_leaf_len;
        let vacant = vacant_leaf_image::<K, V>(generation, free_position)?;
        self.shadow_set_leaf(plan, leaf_index, &vacant)?;
        plan.set(
            DestinationKind::FreeCell,
            free_position,
            self.header.generation,
            &(leaf_index as u32).to_le_bytes(),
        )?;
        header.free_leaf_len = header
            .free_leaf_len
            .checked_add(1)
            .ok_or(FixedStorageError::Capacity)?;
        header.occupied = header
            .occupied
            .checked_sub(1)
            .ok_or(FixedStorageError::NonCanonical)?;
        Ok(())
    }

    pub(crate) fn validates_assignment_plan<const N: usize>(
        &self,
        plan: &PatriciaAssignmentPlan<N>,
    ) -> bool {
        if plan.expected_generation != self.header.generation
            || plan.assignments().is_empty()
            || plan.assignments().iter().any(|assignment| {
                !assignment.validate()
                    || *assignment != Assignment::NOOP && assignment.destination_arena != plan.arena
            })
            || plan.edit_len == 0
            || plan.edit_len > ASSIGNMENT_EDIT_MAX
            || !plan.current_is_generation
            || plan.next_ordinal != 1
            || plan.len != (plan.edit_len - 1) * 9 + 1
            || (0..plan.len).any(|index| {
                plan.resolve_order_ref(plan.order_refs[index])
                    .is_none_or(|key| key.arena != plan.arena)
                    || index > 0
                        && plan
                            .resolve_order_ref(plan.order_refs[index - 1])
                            .zip(plan.resolve_order_ref(plan.order_refs[index]))
                            .is_none_or(|(prior, current)| prior >= current)
            })
            || plan.edits[..plan.edit_len]
                .windows(2)
                .any(|pair| pair[0] >= pair[1])
            || plan.edits[..plan.edit_len]
                .iter()
                .any(|edit| edit.arena != plan.arena)
            || plan.order_refs[plan.len..]
                .iter()
                .any(|reference| *reference != AssignmentOrderRef::ZERO)
            || plan.edits[plan.edit_len..]
                .iter()
                .any(|edit| *edit != AssignmentOrderKey::ZERO)
        {
            return false;
        }
        let mut header_seen = false;
        for (assignment_index, assignment) in plan.assignments().iter().enumerate() {
            let prior_destination = || {
                plan.assignments[..assignment_index]
                    .iter()
                    .rev()
                    .find(|prior| {
                        **prior != Assignment::NOOP
                            && prior.destination_kind == assignment.destination_kind
                            && prior.destination_slot == assignment.destination_slot
                    })
            };
            let valid = match assignment.destination_kind {
                value if value == DestinationKind::Leaf as u8 => {
                    let index = assignment.destination_slot as usize;
                    index < self.header.leaf_capacity as usize
                        && assignment.image_len as usize
                            == assignment_image_width(leaf_bytes(K, V).unwrap_or(0)).unwrap_or(0)
                        && prior_destination().map_or_else(
                            || read_u32(self.leaf_bytes_at(index), 4) as u64,
                            |prior| read_u32(&prior.payload, 4) as u64,
                        ) == assignment.expected_generation
                }
                value if value == DestinationKind::Branch as u8 => {
                    let index = assignment.destination_slot as usize;
                    index < self.header.branch_capacity as usize
                        && assignment.image_len as usize == BRANCH_SLOT_BYTES
                        && prior_destination().map_or_else(
                            || read_u32(self.branch_bytes_at(index), 4) as u64,
                            |prior| read_u32(&prior.payload, 4) as u64,
                        ) == assignment.expected_generation
                }
                value if value == DestinationKind::Header as u8 => {
                    if header_seen {
                        return false;
                    }
                    header_seen = true;
                    assignment.destination_slot == 0
                        && assignment.image_len as usize == INDEX_HEADER_BYTES
                        && assignment.expected_generation == self.header.generation
                        && assignment.payload[..INDEX_HEADER_BYTES]
                            == encode_index_header(plan.after_header)
                }
                value if value == DestinationKind::FreeCell as u8 => {
                    let branch = assignment.destination_slot & FREE_BRANCH_CELL_FLAG != 0;
                    let position = (assignment.destination_slot & !FREE_BRANCH_CELL_FLAG) as usize;
                    assignment.image_len == 4
                        && assignment.expected_generation == self.header.generation
                        && if branch {
                            position < self.header.branch_capacity as usize
                        } else {
                            position < self.header.leaf_capacity as usize
                        }
                }
                value if value == DestinationKind::Noop as u8 => *assignment == Assignment::NOOP,
                _ => false,
            };
            if !valid {
                return false;
            }
        }
        header_seen
    }

    pub(crate) fn commit_assignment_plan<const N: usize>(
        &mut self,
        plan: PatriciaAssignmentPlan<N>,
    ) {
        assert!(
            self.validates_assignment_plan(&plan),
            "validated direct Patricia assignment plan"
        );
        self.commit_assignment_plan_prevalidated(plan);
    }

    pub(crate) fn commit_assignment_plan_prevalidated<const N: usize>(
        &mut self,
        plan: PatriciaAssignmentPlan<N>,
    ) {
        let PatriciaAssignmentPlan {
            after_header,
            assignments,
            len,
            ..
        } = plan;
        for assignment in &assignments[..len] {
            match assignment.destination_kind {
                value if value == DestinationKind::Leaf as u8 => {
                    let destination = self.leaf_bytes_at_mut(assignment.destination_slot as usize);
                    destination.copy_from_slice(&assignment.payload[..destination.len()]);
                }
                value if value == DestinationKind::Branch as u8 => {
                    let destination =
                        self.branch_bytes_at_mut(assignment.destination_slot as usize);
                    destination.copy_from_slice(&assignment.payload[..BRANCH_SLOT_BYTES]);
                }
                value if value == DestinationKind::Header as u8 => {}
                value if value == DestinationKind::FreeCell as u8 => {
                    let branch = assignment.destination_slot & FREE_BRANCH_CELL_FLAG != 0;
                    let position = (assignment.destination_slot & !FREE_BRANCH_CELL_FLAG) as usize;
                    let destination = if branch {
                        &mut self.free_branches[position * 4..position * 4 + 4]
                    } else {
                        &mut self.free_leaves[position * 4..position * 4 + 4]
                    };
                    destination.copy_from_slice(&assignment.payload[..4]);
                }
                value if value == DestinationKind::Noop as u8 => {}
                _ => unreachable!("validated direct Patricia destination"),
            }
        }
        self.header = after_header;
    }

    pub(crate) fn commit_assignment_direct(&mut self, assignment: &Assignment) {
        match assignment.destination_kind {
            value if value == DestinationKind::Leaf as u8 => {
                let destination = self.leaf_bytes_at_mut(assignment.destination_slot as usize);
                destination.copy_from_slice(&assignment.payload[..destination.len()]);
            }
            value if value == DestinationKind::Branch as u8 => {
                let destination = self.branch_bytes_at_mut(assignment.destination_slot as usize);
                destination.copy_from_slice(&assignment.payload[..BRANCH_SLOT_BYTES]);
            }
            value if value == DestinationKind::Header as u8 => {
                self.header = ReusableIndexHeader {
                    generation: read_handle(&assignment.payload, 0).0,
                    root: read_handle(&assignment.payload, 8),
                    occupied: read_u32(&assignment.payload, 16),
                    leaf_capacity: read_u32(&assignment.payload, 20),
                    branch_capacity: read_u32(&assignment.payload, 24),
                    free_leaf_len: read_u32(&assignment.payload, 28),
                    free_branch_len: read_u32(&assignment.payload, 32),
                    reserved: read_u32(&assignment.payload, 36),
                };
            }
            value if value == DestinationKind::FreeCell as u8 => {
                let branch = assignment.destination_slot & FREE_BRANCH_CELL_FLAG != 0;
                let position = (assignment.destination_slot & !FREE_BRANCH_CELL_FLAG) as usize;
                let destination = if branch {
                    &mut self.free_branches[position * 4..position * 4 + 4]
                } else {
                    &mut self.free_leaves[position * 4..position * 4 + 4]
                };
                destination.copy_from_slice(&assignment.payload[..4]);
            }
            value if value == DestinationKind::Noop as u8 => {}
            _ => unreachable!("validated direct Patricia destination"),
        }
    }

    fn shadow_leaf<const N: usize>(
        &self,
        plan: &PatriciaAssignmentPlan<N>,
        handle: NodeHandle,
    ) -> Result<[u8; 112], FixedStorageError> {
        let index = leaf_index(handle)?;
        let width = leaf_bytes(K, V).ok_or(FixedStorageError::Capacity)?;
        let source = plan
            .image(DestinationKind::Leaf, index as u32)
            .unwrap_or_else(|| self.leaf_bytes_at(index));
        if source.len() < width || source[0] != 1 || read_u32(source, 4) != handle.generation() {
            return Err(FixedStorageError::NonCanonical);
        }
        let mut image = [0; 112];
        image[..width].copy_from_slice(&source[..width]);
        Ok(image)
    }

    fn shadow_branch<const N: usize>(
        &self,
        plan: &PatriciaAssignmentPlan<N>,
        handle: NodeHandle,
    ) -> Result<[u8; BRANCH_SLOT_BYTES], FixedStorageError> {
        let index = branch_index(handle)?;
        let source = plan
            .image(DestinationKind::Branch, index as u32)
            .unwrap_or_else(|| self.branch_bytes_at(index));
        if source.len() != BRANCH_SLOT_BYTES
            || source[0] != 1
            || read_u32(source, 4) != handle.generation()
        {
            return Err(FixedStorageError::NonCanonical);
        }
        let mut image = [0; BRANCH_SLOT_BYTES];
        image.copy_from_slice(source);
        Ok(image)
    }

    fn shadow_set_leaf<const N: usize>(
        &self,
        plan: &mut PatriciaAssignmentPlan<N>,
        index: usize,
        image: &[u8; 112],
    ) -> Result<(), FixedStorageError> {
        let width = leaf_bytes(K, V).ok_or(FixedStorageError::Capacity)?;
        let slot = u32::try_from(index).map_err(|_| FixedStorageError::Capacity)?;
        let expected = plan.image(DestinationKind::Leaf, slot).map_or_else(
            || read_u32(self.leaf_bytes_at(index), 4) as u64,
            |prior| read_u32(prior, 4) as u64,
        );
        let stored_width = assignment_image_width(width)?;
        plan.set(
            DestinationKind::Leaf,
            slot,
            expected,
            &image[..stored_width],
        )
    }

    fn shadow_set_branch<const N: usize>(
        &self,
        plan: &mut PatriciaAssignmentPlan<N>,
        index: usize,
        image: &[u8; BRANCH_SLOT_BYTES],
    ) -> Result<(), FixedStorageError> {
        let slot = u32::try_from(index).map_err(|_| FixedStorageError::Capacity)?;
        let expected = plan.image(DestinationKind::Branch, slot).map_or_else(
            || read_u32(self.branch_bytes_at(index), 4) as u64,
            |prior| read_u32(prior, 4) as u64,
        );
        plan.set(DestinationKind::Branch, slot, expected, image)
    }

    fn shadow_locate<const N: usize>(
        &self,
        plan: &PatriciaAssignmentPlan<N>,
        root: NodeHandle,
        key: &[u8; K],
    ) -> Result<(NodeHandle, Option<u16>), FixedStorageError> {
        let mut handle = root;
        let mut parent = NodeHandle::SENTINEL;
        let mut prior = None;
        loop {
            match node_tag(handle.node()) {
                LEAF_TAG => {
                    let slot = self.shadow_leaf(plan, handle)?;
                    if read_handle(&slot, 8) != parent {
                        return Err(FixedStorageError::NonCanonical);
                    }
                    return Ok((handle, prior));
                }
                BRANCH_TAG => {
                    let slot = self.shadow_branch(plan, handle)?;
                    if read_handle(&slot, 8) != parent {
                        return Err(FixedStorageError::NonCanonical);
                    }
                    let bit = read_u16(&slot, 16);
                    if bit as usize >= K * 8 || prior.is_some_and(|value| value >= bit) {
                        return Err(FixedStorageError::NonCanonical);
                    }
                    let children = [read_handle(&slot, 24), read_handle(&slot, 32)];
                    if children[0] == children[1]
                        || children[0].is_sentinel()
                        || children[1].is_sentinel()
                    {
                        return Err(FixedStorageError::NonCanonical);
                    }
                    parent = handle;
                    prior = Some(bit);
                    handle = children[key_bit(key, bit)];
                }
                _ => return Err(FixedStorageError::NonCanonical),
            }
        }
    }

    fn shadow_insertion_point<const N: usize>(
        &self,
        plan: &PatriciaAssignmentPlan<N>,
        root: NodeHandle,
        key: &[u8; K],
        bit: u16,
    ) -> Result<(NodeHandle, NodeHandle), FixedStorageError> {
        let mut parent = NodeHandle::SENTINEL;
        let mut child = root;
        let mut prior = None;
        while node_tag(child.node()) == BRANCH_TAG {
            let slot = self.shadow_branch(plan, child)?;
            if read_handle(&slot, 8) != parent {
                return Err(FixedStorageError::NonCanonical);
            }
            let child_bit = read_u16(&slot, 16);
            if child_bit as usize >= K * 8 || prior.is_some_and(|value| value >= child_bit) {
                return Err(FixedStorageError::NonCanonical);
            }
            if child_bit >= bit {
                break;
            }
            let children = [read_handle(&slot, 24), read_handle(&slot, 32)];
            parent = child;
            prior = Some(child_bit);
            child = children[key_bit(key, child_bit)];
        }
        Ok((parent, child))
    }

    fn shadow_removal_path<const N: usize>(
        &self,
        plan: &PatriciaAssignmentPlan<N>,
        root: NodeHandle,
        key: &[u8; K],
    ) -> Result<RemovalPath, FixedStorageError> {
        let mut grandparent = NodeHandle::SENTINEL;
        let mut parent = NodeHandle::SENTINEL;
        let mut sibling = NodeHandle::SENTINEL;
        let mut leaf = root;
        let mut prior = None;
        while node_tag(leaf.node()) == BRANCH_TAG {
            let slot = self.shadow_branch(plan, leaf)?;
            if read_handle(&slot, 8) != parent {
                return Err(FixedStorageError::NonCanonical);
            }
            let bit = read_u16(&slot, 16);
            if bit as usize >= K * 8 || prior.is_some_and(|value| value >= bit) {
                return Err(FixedStorageError::NonCanonical);
            }
            let children = [read_handle(&slot, 24), read_handle(&slot, 32)];
            let selected = key_bit(key, bit);
            grandparent = parent;
            parent = leaf;
            sibling = children[1 - selected];
            leaf = children[selected];
            prior = Some(bit);
        }
        self.shadow_leaf(plan, leaf)?;
        Ok(RemovalPath {
            grandparent,
            parent,
            sibling,
            leaf,
        })
    }

    fn shadow_set_parent<const N: usize>(
        &self,
        plan: &mut PatriciaAssignmentPlan<N>,
        child: NodeHandle,
        parent: NodeHandle,
    ) -> Result<(), FixedStorageError> {
        match node_tag(child.node()) {
            LEAF_TAG => {
                let index = leaf_index(child)?;
                let mut image = self.shadow_leaf(plan, child)?;
                write_handle(&mut image, 8, parent);
                self.shadow_set_leaf(plan, index, &image)
            }
            BRANCH_TAG => {
                let index = branch_index(child)?;
                let mut image = self.shadow_branch(plan, child)?;
                write_handle(&mut image, 8, parent);
                self.shadow_set_branch(plan, index, &image)
            }
            _ => Err(FixedStorageError::NonCanonical),
        }
    }

    fn shadow_replace_child<const N: usize>(
        &self,
        plan: &mut PatriciaAssignmentPlan<N>,
        parent: NodeHandle,
        before: NodeHandle,
        after: NodeHandle,
    ) -> Result<(), FixedStorageError> {
        let index = branch_index(parent)?;
        let mut image = self.shadow_branch(plan, parent)?;
        let offset = if read_handle(&image, 24) == before {
            24
        } else if read_handle(&image, 32) == before {
            32
        } else {
            return Err(FixedStorageError::NonCanonical);
        };
        write_handle(&mut image, offset, after);
        self.shadow_set_branch(plan, index, &image)
    }

    pub(crate) fn insert_batch_prevalidated(&mut self, entries: &[([u8; K], [u8; V])]) {
        self.validate_insert_batch(entries)
            .expect("validated Patricia insertion batch");
        let before = self.header.generation;
        for (key, value) in entries.iter().copied() {
            self.insert(key, value)
                .expect("prevalidated Patricia insertion");
            self.header.generation = before;
        }
        self.header.generation = before
            .checked_add(1)
            .expect("validated Patricia generation");
    }

    /// Validates a strictly key-sorted insertion stream without retaining a
    /// second hot-path buffer. The exact count seals both free backings, and a
    /// single index-generation increment covers the later batch commit.
    pub(crate) fn validate_insert_stream(
        &self,
        entries: impl IntoIterator<Item = ([u8; K], [u8; V])>,
        count: usize,
    ) -> Result<(), FixedStorageError> {
        self.validate_header()?;
        if count == 0
            || count > self.header.free_leaf_len as usize
            || count
                > self.header.free_branch_len as usize + usize::from(self.header.root.is_sentinel())
        {
            return Err(FixedStorageError::Capacity);
        }
        self.header
            .generation
            .checked_add(1)
            .ok_or(FixedStorageError::Capacity)?;
        let mut previous = None;
        let mut seen = 0usize;
        for (key, _) in entries {
            if seen >= count {
                return Err(FixedStorageError::NonCanonical);
            }
            if previous.is_some_and(|prior| prior >= key) {
                return Err(FixedStorageError::Duplicate);
            }
            if self.find(&key)?.is_some() {
                return Err(FixedStorageError::Duplicate);
            }
            let position = self.header.free_leaf_len as usize - 1 - seen;
            let slot_index = read_u32(&self.free_leaves, position * 4) as usize;
            let slot = self.leaf_bytes_at(slot_index);
            if slot[0] != 0 || read_u32(slot, 8) as usize != position {
                return Err(FixedStorageError::NonCanonical);
            }
            read_u32(slot, 4)
                .checked_add(1)
                .ok_or(FixedStorageError::Capacity)?;
            previous = Some(key);
            seen += 1;
        }
        if seen != count {
            return Err(FixedStorageError::NonCanonical);
        }
        let branches = count
            .checked_sub(usize::from(self.header.root.is_sentinel()))
            .ok_or(FixedStorageError::NonCanonical)?;
        for index in 0..branches {
            let position = self.header.free_branch_len as usize - 1 - index;
            let slot_index = read_u32(&self.free_branches, position * 4) as usize;
            let slot = self.branch_bytes_at(slot_index);
            if slot[0] != 0 || read_u32(slot, 8) as usize != position {
                return Err(FixedStorageError::NonCanonical);
            }
            read_u32(slot, 4)
                .checked_add(1)
                .ok_or(FixedStorageError::Capacity)?;
        }
        Ok(())
    }

    pub(crate) fn insert_stream_prevalidated(
        &mut self,
        entries: impl IntoIterator<Item = ([u8; K], [u8; V])>,
        count: usize,
    ) {
        let before = self.header.generation;
        let mut seen = 0usize;
        for (key, value) in entries {
            assert!(seen < count, "prevalidated Patricia stream count");
            self.insert(key, value)
                .expect("prevalidated Patricia stream insertion");
            self.header.generation = before;
            seen += 1;
        }
        assert_eq!(seen, count, "prevalidated Patricia stream count");
        self.header.generation = before
            .checked_add(1)
            .expect("validated Patricia generation");
    }

    pub(crate) fn value_at(&self, handle: NodeHandle) -> Result<[u8; V], FixedStorageError> {
        let slot = self.leaf_slot(handle)?;
        let mut value = [0; V];
        value.copy_from_slice(&slot[16 + K..16 + K + V]);
        Ok(value)
    }

    pub(crate) fn insert(
        &mut self,
        key: [u8; K],
        value: [u8; V],
    ) -> Result<NodeHandle, FixedStorageError> {
        self.validate_header()?;
        let next_generation = self
            .header
            .generation
            .checked_add(1)
            .ok_or(FixedStorageError::Capacity)?;
        let empty = self.header.root.is_sentinel();
        let terminal = if empty {
            None
        } else {
            let (leaf, _) = self.locate(&key)?;
            let slot = self.leaf_slot(leaf)?;
            if slot[16..16 + K] == key {
                return Err(FixedStorageError::Duplicate);
            }
            Some(leaf)
        };
        let leaf_index = self.selected_free_leaf()?;
        let leaf_generation = read_u32(self.leaf_bytes_at(leaf_index), 4)
            .checked_add(1)
            .ok_or(FixedStorageError::Capacity)?;
        let leaf_handle = NodeHandle::new(leaf_index as u32 | LEAF_TAG, leaf_generation);
        if empty {
            self.pop_free_leaf(leaf_index)?;
            self.write_leaf_occupied(
                leaf_index,
                leaf_generation,
                NodeHandle::SENTINEL,
                &key,
                &value,
            );
            self.header.root = leaf_handle;
        } else {
            let terminal = terminal.expect("nonempty Patricia has a terminal leaf");
            let peer = self.leaf_slot(terminal)?;
            let bit = first_difference(&key, &peer[16..16 + K])?;
            let (parent, child) = self.insertion_point(&key, bit)?;
            let branch_index = self.selected_free_branch()?;
            let branch_generation = read_u32(self.branch_bytes_at(branch_index), 4)
                .checked_add(1)
                .ok_or(FixedStorageError::Capacity)?;
            let branch_handle =
                NodeHandle::new(branch_index as u32 | BRANCH_TAG, branch_generation);
            self.pop_free_leaf(leaf_index)?;
            self.pop_free_branch(branch_index)?;
            let children = if key_bit(&key, bit) == 0 {
                [leaf_handle, child]
            } else {
                [child, leaf_handle]
            };
            self.write_leaf_occupied(leaf_index, leaf_generation, branch_handle, &key, &value);
            self.write_branch_occupied(branch_index, branch_generation, parent, bit, children);
            self.set_parent(child, branch_handle)?;
            if parent.is_sentinel() {
                self.header.root = branch_handle;
            } else {
                self.replace_child(parent, child, branch_handle)?;
            }
        }
        self.header.occupied = self
            .header
            .occupied
            .checked_add(1)
            .ok_or(FixedStorageError::Capacity)?;
        self.header.generation = next_generation;
        Ok(leaf_handle)
    }

    pub(crate) fn update(
        &mut self,
        key: &[u8; K],
        expected: NodeHandle,
        value: [u8; V],
    ) -> Result<(), FixedStorageError> {
        self.validate_header()?;
        let found = self
            .find_handle(key)?
            .ok_or(FixedStorageError::NonCanonical)?;
        if found != expected {
            return Err(FixedStorageError::NonCanonical);
        }
        let next_generation = self
            .header
            .generation
            .checked_add(1)
            .ok_or(FixedStorageError::Capacity)?;
        let index = leaf_index(found)?;
        self.leaf_bytes_at_mut(index)[16 + K..16 + K + V].copy_from_slice(&value);
        self.header.generation = next_generation;
        Ok(())
    }

    pub(crate) fn validate_update_batch(
        &self,
        entries: &[([u8; K], NodeHandle, [u8; V])],
    ) -> Result<(), FixedStorageError> {
        self.validate_header()?;
        if entries.is_empty() {
            return Err(FixedStorageError::Capacity);
        }
        self.header
            .generation
            .checked_add(1)
            .ok_or(FixedStorageError::Capacity)?;
        for (index, (key, expected, _)) in entries.iter().enumerate() {
            if entries[..index].iter().any(|(prior, _, _)| prior == key)
                || self.find_handle(key)? != Some(*expected)
            {
                return Err(FixedStorageError::NonCanonical);
            }
        }
        Ok(())
    }

    pub(crate) fn update_batch_prevalidated(&mut self, entries: &[([u8; K], NodeHandle, [u8; V])]) {
        self.validate_update_batch(entries)
            .expect("validated Patricia update batch");
        let next = self
            .header
            .generation
            .checked_add(1)
            .expect("validated Patricia generation");
        for (_, handle, value) in entries.iter().copied() {
            let index = leaf_index(handle).expect("validated Patricia leaf handle");
            self.leaf_bytes_at_mut(index)[16 + K..16 + K + V].copy_from_slice(&value);
        }
        self.header.generation = next;
    }

    pub(crate) fn replace_value_direct(&mut self, handle: NodeHandle, value: [u8; V]) {
        debug_assert_eq!(handle.node() & !NODE_INDEX_MASK, LEAF_TAG);
        let index = (handle.node() & NODE_INDEX_MASK) as usize;
        self.leaf_bytes_at_mut(index)[16 + K..16 + K + V].copy_from_slice(&value);
    }

    pub(crate) fn validate_update_stream(
        &self,
        entries: impl IntoIterator<Item = ([u8; K], [u8; V])>,
        count: usize,
    ) -> Result<(), FixedStorageError> {
        self.validate_header()?;
        if count == 0 {
            return Err(FixedStorageError::Capacity);
        }
        self.header
            .generation
            .checked_add(1)
            .ok_or(FixedStorageError::Capacity)?;
        let mut previous = None;
        let mut seen = 0usize;
        for (key, _) in entries {
            if seen >= count || previous.is_some_and(|prior| prior >= key) {
                return Err(FixedStorageError::NonCanonical);
            }
            self.find_handle(&key)?
                .ok_or(FixedStorageError::NonCanonical)?;
            previous = Some(key);
            seen += 1;
        }
        (seen == count)
            .then_some(())
            .ok_or(FixedStorageError::NonCanonical)
    }

    pub(crate) fn update_stream_prevalidated(
        &mut self,
        entries: impl IntoIterator<Item = ([u8; K], [u8; V])>,
        count: usize,
    ) {
        let before = self.header.generation;
        let mut seen = 0usize;
        for (key, value) in entries {
            assert!(seen < count, "prevalidated Patricia stream count");
            let handle = self
                .find_handle(&key)
                .expect("validated Patricia stream lookup")
                .expect("validated Patricia stream key");
            self.update(&key, handle, value)
                .expect("prevalidated Patricia stream update");
            self.header.generation = before;
            seen += 1;
        }
        assert_eq!(seen, count, "prevalidated Patricia stream count");
        self.header.generation = before
            .checked_add(1)
            .expect("validated Patricia generation");
    }

    pub(crate) fn validate_remove_batch(&self, keys: &[[u8; K]]) -> Result<(), FixedStorageError> {
        self.validate_header()?;
        if keys.is_empty() || keys.len() > self.header.occupied as usize {
            return Err(FixedStorageError::Capacity);
        }
        self.header
            .generation
            .checked_add(1)
            .ok_or(FixedStorageError::Capacity)?;
        for (index, key) in keys.iter().enumerate() {
            if keys[..index].contains(key) || self.find(key)?.is_none() {
                return Err(FixedStorageError::NonCanonical);
            }
        }
        Ok(())
    }

    pub(crate) fn remove_batch_prevalidated(&mut self, keys: &[[u8; K]]) {
        self.validate_remove_batch(keys)
            .expect("validated Patricia removal batch");
        let before = self.header.generation;
        for key in keys {
            self.remove(key)
                .expect("prevalidated Patricia removal")
                .expect("prevalidated Patricia key exists");
            self.header.generation = before;
        }
        self.header.generation = before
            .checked_add(1)
            .expect("validated Patricia generation");
    }

    pub(crate) fn validate_advance_generation(&self) -> Result<(), FixedStorageError> {
        self.validate_header()?;
        self.header
            .generation
            .checked_add(1)
            .ok_or(FixedStorageError::Capacity)
            .map(|_| ())
    }

    pub(crate) fn advance_generation_prevalidated(&mut self) {
        self.validate_advance_generation()
            .expect("validated Patricia generation advance");
        self.header.generation += 1;
    }

    pub(crate) fn remove(&mut self, key: &[u8; K]) -> Result<Option<[u8; V]>, FixedStorageError> {
        self.validate_header()?;
        if self.header.root.is_sentinel() {
            return Ok(None);
        }
        let path = self.removal_path(key)?;
        let leaf_slot = self.leaf_slot(path.leaf)?;
        if leaf_slot[16..16 + K] != key[..] {
            return Ok(None);
        }
        let mut value = [0; V];
        value.copy_from_slice(&leaf_slot[16 + K..16 + K + V]);
        let next_generation = self
            .header
            .generation
            .checked_add(1)
            .ok_or(FixedStorageError::Capacity)?;
        if self.header.free_leaf_len == self.header.leaf_capacity
            || self.header.free_branch_len > self.header.branch_capacity
        {
            return Err(FixedStorageError::NonCanonical);
        }
        if path.parent.is_sentinel() {
            if !path.grandparent.is_sentinel() || !path.sibling.is_sentinel() {
                return Err(FixedStorageError::NonCanonical);
            }
            self.header.root = NodeHandle::SENTINEL;
        } else {
            if path.sibling.is_sentinel() {
                return Err(FixedStorageError::NonCanonical);
            }
            self.set_parent(path.sibling, path.grandparent)?;
            if path.grandparent.is_sentinel() {
                self.header.root = path.sibling;
            } else {
                self.replace_child(path.grandparent, path.parent, path.sibling)?;
            }
            let branch = branch_index(path.parent)?;
            let generation = read_u32(self.branch_bytes_at(branch), 4);
            self.push_free_branch(branch, generation)?;
        }
        let leaf = leaf_index(path.leaf)?;
        let generation = read_u32(self.leaf_bytes_at(leaf), 4);
        self.push_free_leaf(leaf, generation)?;
        self.header.occupied = self
            .header
            .occupied
            .checked_sub(1)
            .ok_or(FixedStorageError::NonCanonical)?;
        self.header.generation = next_generation;
        Ok(Some(value))
    }

    #[cfg_attr(
        not(test),
        allow(dead_code, reason = "C17 structural tests consume this validator")
    )]
    pub(crate) fn validate_structure(&self) -> Result<(), FixedStorageError> {
        self.validate_header()?;
        let occupied = self.header.occupied as usize;
        if occupied == 0 {
            return self
                .header
                .root
                .is_sentinel()
                .then_some(())
                .ok_or(FixedStorageError::NonCanonical);
        }
        if self.header.root.is_sentinel() {
            return Err(FixedStorageError::NonCanonical);
        }
        let mut leaves = 0usize;
        let mut branches = 0usize;
        let mut stack = vec![(self.header.root, NodeHandle::SENTINEL, None)];
        while let Some((handle, parent, prior_bit)) = stack.pop() {
            match node_tag(handle.node()) {
                LEAF_TAG => {
                    let slot = self.leaf_slot(handle)?;
                    if read_handle(slot, 8) != parent {
                        return Err(FixedStorageError::NonCanonical);
                    }
                    leaves += 1;
                }
                BRANCH_TAG => {
                    let slot = self.branch_slot(handle)?;
                    if read_handle(slot, 8) != parent {
                        return Err(FixedStorageError::NonCanonical);
                    }
                    let bit = read_u16(slot, 16);
                    if bit as usize >= K * 8 || prior_bit.is_some_and(|prior| prior >= bit) {
                        return Err(FixedStorageError::NonCanonical);
                    }
                    let zero = read_handle(slot, 24);
                    let one = read_handle(slot, 32);
                    if zero == one || zero.is_sentinel() || one.is_sentinel() {
                        return Err(FixedStorageError::NonCanonical);
                    }
                    branches += 1;
                    stack.push((one, handle, Some(bit)));
                    stack.push((zero, handle, Some(bit)));
                }
                _ => return Err(FixedStorageError::NonCanonical),
            }
            if leaves + branches > occupied.saturating_mul(2) {
                return Err(FixedStorageError::NonCanonical);
            }
        }
        (leaves == occupied && branches + 1 == leaves)
            .then_some(())
            .ok_or(FixedStorageError::NonCanonical)
    }

    fn validate_header(&self) -> Result<(), FixedStorageError> {
        let leaf_width = leaf_bytes(K, V).ok_or(FixedStorageError::Capacity)?;
        let leaves = self.header.leaf_capacity as usize;
        let branches = self.header.branch_capacity as usize;
        let canonical = self.header.reserved == 0
            && leaves > 0
            && leaves < (1 << NODE_TAG_SHIFT)
            && branches + 1 == leaves
            && self.leaves.len() == leaves.checked_mul(leaf_width).unwrap_or(usize::MAX)
            && self.branches.len()
                == branches
                    .checked_mul(BRANCH_SLOT_BYTES)
                    .unwrap_or(usize::MAX)
            && self.free_leaves.len() == leaves.checked_mul(4).unwrap_or(usize::MAX)
            && self.free_branches.len() == branches.checked_mul(4).unwrap_or(usize::MAX)
            && self.header.free_leaf_len <= self.header.leaf_capacity
            && self.header.free_branch_len <= self.header.branch_capacity
            && self.header.occupied <= self.header.leaf_capacity
            && self.header.occupied + self.header.free_leaf_len == self.header.leaf_capacity
            && (self.header.occupied == 0) == self.header.root.is_sentinel();
        canonical
            .then_some(())
            .ok_or(FixedStorageError::NonCanonical)
    }

    fn locate(&self, key: &[u8; K]) -> Result<(NodeHandle, Option<u16>), FixedStorageError> {
        let mut handle = self.header.root;
        let mut parent = NodeHandle::SENTINEL;
        let mut prior = None;
        loop {
            match node_tag(handle.node()) {
                LEAF_TAG => {
                    let slot = self.leaf_slot(handle)?;
                    if read_handle(slot, 8) != parent {
                        return Err(FixedStorageError::NonCanonical);
                    }
                    return Ok((handle, prior));
                }
                BRANCH_TAG => {
                    let slot = self.branch_slot(handle)?;
                    if read_handle(slot, 8) != parent {
                        return Err(FixedStorageError::NonCanonical);
                    }
                    let bit = read_u16(slot, 16);
                    if bit as usize >= K * 8 || prior.is_some_and(|value| value >= bit) {
                        return Err(FixedStorageError::NonCanonical);
                    }
                    let children = [read_handle(slot, 24), read_handle(slot, 32)];
                    if children[0] == children[1]
                        || children[0].is_sentinel()
                        || children[1].is_sentinel()
                    {
                        return Err(FixedStorageError::NonCanonical);
                    }
                    parent = handle;
                    prior = Some(bit);
                    handle = children[key_bit(key, bit)];
                }
                _ => return Err(FixedStorageError::NonCanonical),
            }
        }
    }

    fn insertion_point(
        &self,
        key: &[u8; K],
        bit: u16,
    ) -> Result<(NodeHandle, NodeHandle), FixedStorageError> {
        let mut parent = NodeHandle::SENTINEL;
        let mut child = self.header.root;
        let mut prior = None;
        while node_tag(child.node()) == BRANCH_TAG {
            let slot = self.branch_slot(child)?;
            if read_handle(slot, 8) != parent {
                return Err(FixedStorageError::NonCanonical);
            }
            let child_bit = read_u16(slot, 16);
            if child_bit as usize >= K * 8 || prior.is_some_and(|value| value >= child_bit) {
                return Err(FixedStorageError::NonCanonical);
            }
            if child_bit >= bit {
                break;
            }
            let children = [read_handle(slot, 24), read_handle(slot, 32)];
            parent = child;
            prior = Some(child_bit);
            child = children[key_bit(key, child_bit)];
        }
        Ok((parent, child))
    }

    fn removal_path(&self, key: &[u8; K]) -> Result<RemovalPath, FixedStorageError> {
        let mut grandparent = NodeHandle::SENTINEL;
        let mut parent = NodeHandle::SENTINEL;
        let mut sibling = NodeHandle::SENTINEL;
        let mut leaf = self.header.root;
        let mut prior = None;
        while node_tag(leaf.node()) == BRANCH_TAG {
            let slot = self.branch_slot(leaf)?;
            if read_handle(slot, 8) != parent {
                return Err(FixedStorageError::NonCanonical);
            }
            let bit = read_u16(slot, 16);
            if bit as usize >= K * 8 || prior.is_some_and(|value| value >= bit) {
                return Err(FixedStorageError::NonCanonical);
            }
            let children = [read_handle(slot, 24), read_handle(slot, 32)];
            let selected = key_bit(key, bit);
            grandparent = parent;
            parent = leaf;
            sibling = children[1 - selected];
            leaf = children[selected];
            prior = Some(bit);
        }
        self.leaf_slot(leaf)?;
        Ok(RemovalPath {
            grandparent,
            parent,
            sibling,
            leaf,
        })
    }

    fn selected_free_leaf(&self) -> Result<usize, FixedStorageError> {
        let length = self.header.free_leaf_len as usize;
        let position = length.checked_sub(1).ok_or(FixedStorageError::Capacity)?;
        let index = read_u32(&self.free_leaves, position * 4) as usize;
        let slot = self.leaf_bytes_at(index);
        let canonical = index < self.header.leaf_capacity as usize
            && slot[0] == 0
            && slot[1..4].iter().all(|byte| *byte == 0)
            && read_u32(slot, 8) as usize == position
            && slot[12..].iter().all(|byte| *byte == 0);
        canonical
            .then_some(index)
            .ok_or(FixedStorageError::NonCanonical)
    }

    fn selected_free_branch(&self) -> Result<usize, FixedStorageError> {
        let length = self.header.free_branch_len as usize;
        let position = length.checked_sub(1).ok_or(FixedStorageError::Capacity)?;
        let index = read_u32(&self.free_branches, position * 4) as usize;
        let slot = self.branch_bytes_at(index);
        let canonical = index < self.header.branch_capacity as usize
            && slot[0] == 0
            && slot[1..4].iter().all(|byte| *byte == 0)
            && read_u32(slot, 8) as usize == position
            && slot[12..].iter().all(|byte| *byte == 0);
        canonical
            .then_some(index)
            .ok_or(FixedStorageError::NonCanonical)
    }

    fn pop_free_leaf(&mut self, expected: usize) -> Result<(), FixedStorageError> {
        let selected = self.selected_free_leaf()?;
        if selected != expected {
            return Err(FixedStorageError::NonCanonical);
        }
        self.header.free_leaf_len -= 1;
        Ok(())
    }

    fn pop_free_branch(&mut self, expected: usize) -> Result<(), FixedStorageError> {
        let selected = self.selected_free_branch()?;
        if selected != expected {
            return Err(FixedStorageError::NonCanonical);
        }
        self.header.free_branch_len -= 1;
        Ok(())
    }

    fn push_free_leaf(&mut self, index: usize, generation: u32) -> Result<(), FixedStorageError> {
        let position = self.header.free_leaf_len as usize;
        if position >= self.header.leaf_capacity as usize {
            return Err(FixedStorageError::NonCanonical);
        }
        self.write_leaf_vacant(index, generation, position as u32);
        write_u32(&mut self.free_leaves, position * 4, index as u32);
        self.header.free_leaf_len += 1;
        Ok(())
    }

    fn push_free_branch(&mut self, index: usize, generation: u32) -> Result<(), FixedStorageError> {
        let position = self.header.free_branch_len as usize;
        if position >= self.header.branch_capacity as usize {
            return Err(FixedStorageError::NonCanonical);
        }
        self.write_branch_vacant(index, generation, position as u32);
        write_u32(&mut self.free_branches, position * 4, index as u32);
        self.header.free_branch_len += 1;
        Ok(())
    }

    fn leaf_slot(&self, handle: NodeHandle) -> Result<&[u8], FixedStorageError> {
        let index = leaf_index(handle)?;
        let slot = self.leaf_bytes_at(index);
        (slot[0] == 1
            && slot[1..4].iter().all(|byte| *byte == 0)
            && read_u32(slot, 4) == handle.generation())
        .then_some(slot)
        .ok_or(FixedStorageError::NonCanonical)
    }

    fn branch_slot(&self, handle: NodeHandle) -> Result<&[u8], FixedStorageError> {
        let index = branch_index(handle)?;
        let slot = self.branch_bytes_at(index);
        (slot[0] == 1
            && slot[1..4].iter().all(|byte| *byte == 0)
            && read_u32(slot, 4) == handle.generation()
            && slot[18..24].iter().all(|byte| *byte == 0))
        .then_some(slot)
        .ok_or(FixedStorageError::NonCanonical)
    }

    fn replace_child(
        &mut self,
        parent: NodeHandle,
        before: NodeHandle,
        after: NodeHandle,
    ) -> Result<(), FixedStorageError> {
        let index = branch_index(parent)?;
        let slot = self.branch_bytes_at_mut(index);
        if slot[0] != 1 || read_u32(slot, 4) != parent.generation() {
            return Err(FixedStorageError::NonCanonical);
        }
        let offset = if read_handle(slot, 24) == before {
            24
        } else if read_handle(slot, 32) == before {
            32
        } else {
            return Err(FixedStorageError::NonCanonical);
        };
        write_handle(slot, offset, after);
        Ok(())
    }

    fn set_parent(
        &mut self,
        child: NodeHandle,
        parent: NodeHandle,
    ) -> Result<(), FixedStorageError> {
        match node_tag(child.node()) {
            LEAF_TAG => {
                let index = leaf_index(child)?;
                let slot = self.leaf_bytes_at_mut(index);
                if slot[0] != 1 || read_u32(slot, 4) != child.generation() {
                    return Err(FixedStorageError::NonCanonical);
                }
                write_handle(slot, 8, parent);
            }
            BRANCH_TAG => {
                let index = branch_index(child)?;
                let slot = self.branch_bytes_at_mut(index);
                if slot[0] != 1 || read_u32(slot, 4) != child.generation() {
                    return Err(FixedStorageError::NonCanonical);
                }
                write_handle(slot, 8, parent);
            }
            _ => return Err(FixedStorageError::NonCanonical),
        }
        Ok(())
    }

    fn write_leaf_vacant(&mut self, index: usize, generation: u32, free_position: u32) {
        let slot = self.leaf_bytes_at_mut(index);
        slot.fill(0);
        write_u32(slot, 4, generation);
        write_u32(slot, 8, free_position);
    }

    fn write_leaf_occupied(
        &mut self,
        index: usize,
        generation: u32,
        parent: NodeHandle,
        key: &[u8; K],
        value: &[u8; V],
    ) {
        let slot = self.leaf_bytes_at_mut(index);
        slot.fill(0);
        slot[0] = 1;
        write_u32(slot, 4, generation);
        write_handle(slot, 8, parent);
        slot[16..16 + K].copy_from_slice(key);
        slot[16 + K..16 + K + V].copy_from_slice(value);
    }

    fn write_branch_vacant(&mut self, index: usize, generation: u32, free_position: u32) {
        let slot = self.branch_bytes_at_mut(index);
        slot.fill(0);
        write_u32(slot, 4, generation);
        write_u32(slot, 8, free_position);
    }

    fn write_branch_occupied(
        &mut self,
        index: usize,
        generation: u32,
        parent: NodeHandle,
        bit: u16,
        children: [NodeHandle; 2],
    ) {
        let slot = self.branch_bytes_at_mut(index);
        slot.fill(0);
        slot[0] = 1;
        write_u32(slot, 4, generation);
        write_handle(slot, 8, parent);
        write_u16(slot, 16, bit);
        write_handle(slot, 24, children[0]);
        write_handle(slot, 32, children[1]);
    }

    fn leaf_bytes_at(&self, index: usize) -> &[u8] {
        let width = leaf_bytes(K, V).expect("validated leaf width");
        let start = index.checked_mul(width).expect("validated leaf index");
        &self.leaves[start..start + width]
    }

    fn leaf_bytes_at_mut(&mut self, index: usize) -> &mut [u8] {
        let width = leaf_bytes(K, V).expect("validated leaf width");
        let start = index.checked_mul(width).expect("validated leaf index");
        &mut self.leaves[start..start + width]
    }

    fn branch_bytes_at(&self, index: usize) -> &[u8] {
        let start = index
            .checked_mul(BRANCH_SLOT_BYTES)
            .expect("validated branch index");
        &self.branches[start..start + BRANCH_SLOT_BYTES]
    }

    fn branch_bytes_at_mut(&mut self, index: usize) -> &mut [u8] {
        let start = index
            .checked_mul(BRANCH_SLOT_BYTES)
            .expect("validated branch index");
        &mut self.branches[start..start + BRANCH_SLOT_BYTES]
    }
}

#[derive(Clone, Copy)]
struct RemovalPath {
    grandparent: NodeHandle,
    parent: NodeHandle,
    sibling: NodeHandle,
    leaf: NodeHandle,
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(C)]
pub(crate) struct ArenaRef {
    pub(crate) slot: u32,
    pub(crate) generation: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ArenaSelection<const N: usize> {
    refs: [ArenaRef; N],
    len: usize,
}

impl<const N: usize> ArenaSelection<N> {
    pub(crate) const fn empty() -> Self {
        Self {
            refs: [ArenaRef {
                slot: 0,
                generation: 0,
            }; N],
            len: 0,
        }
    }

    pub(crate) const fn as_slice(&self) -> &[ArenaRef] {
        let (prefix, _) = self.refs.split_at(self.len);
        prefix
    }

    pub(crate) const fn len(&self) -> usize {
        self.len
    }
}

impl<const N: usize> std::ops::Index<usize> for ArenaSelection<N> {
    type Output = ArenaRef;

    fn index(&self, index: usize) -> &Self::Output {
        &self.as_slice()[index]
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub(crate) struct ByteArenaHeaderImage(pub(crate) [u8; 32]);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub(crate) struct ByteArenaFreeCellImage(pub(crate) [u8; 4]);

impl ByteArenaHeaderImage {
    pub(crate) const ZERO: Self = Self([0; 32]);
}

impl ByteArenaFreeCellImage {
    pub(crate) const ZERO: Self = Self([0; 4]);
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
struct ByteArenaHeader {
    generation: u64,
    occupied: u32,
    capacity: u32,
    free_len: u32,
    reserved_count: u32,
    inactive_count: u32,
    reserved: u32,
}

/// A fixed two-slice arena whose exact physical equation is
/// `64 + L * (T + 4)`. Slot generations are inline in each canonical image.
#[derive(Clone, Debug, Eq, PartialEq)]
#[repr(C)]
pub(crate) struct FixedByteArena<const T: usize> {
    header: ByteArenaHeader,
    slots: Box<[u8]>,
    free: Box<[u8]>,
}

impl<const T: usize> FixedByteArena<T> {
    pub(crate) fn storage_bytes(capacity: usize) -> Option<u64> {
        if capacity == 0 || T < 16 {
            return None;
        }
        64u64.checked_add(
            u64::try_from(capacity)
                .ok()?
                .checked_mul(u64::try_from(T.checked_add(4)?).ok()?)?,
        )
    }

    pub(crate) fn try_new(capacity: usize) -> Result<Self, FixedStorageError> {
        if capacity == 0 || T < 16 || capacity >= u32::MAX as usize {
            return Err(FixedStorageError::Capacity);
        }
        let slot_bytes = capacity.checked_mul(T).ok_or(FixedStorageError::Capacity)?;
        let free_bytes = capacity.checked_mul(4).ok_or(FixedStorageError::Capacity)?;
        if slot_bytes > isize::MAX as usize || free_bytes > isize::MAX as usize {
            return Err(FixedStorageError::Capacity);
        }
        let mut value = Self {
            header: ByteArenaHeader {
                generation: 0,
                occupied: 0,
                capacity: capacity as u32,
                free_len: capacity as u32,
                reserved_count: 0,
                inactive_count: 0,
                reserved: 0,
            },
            slots: zeroed(slot_bytes)?,
            free: zeroed(free_bytes)?,
        };
        for index in 0..capacity {
            let position = capacity - 1 - index;
            value.write_vacant(index, 0, position as u32);
            write_u32(&mut value.free, position * 4, index as u32);
        }
        value.validate_header()?;
        Ok(value)
    }

    pub(crate) const fn generation(&self) -> u64 {
        self.header.generation
    }

    pub(crate) fn header_image(&self) -> ByteArenaHeaderImage {
        Self::encode_header(self.header)
    }

    pub(crate) fn prepare_reserve_header_after<const N: usize>(
        &self,
        selection: &ArenaSelection<N>,
        installed_occupied: usize,
        installed_inactive: usize,
    ) -> Result<ByteArenaHeaderImage, FixedStorageError> {
        self.prepare_reserve_header_after_advances(
            selection,
            installed_occupied,
            installed_inactive,
            1,
        )
    }

    pub(crate) fn prepare_reserve_header_after_advances<const N: usize>(
        &self,
        selection: &ArenaSelection<N>,
        installed_occupied: usize,
        installed_inactive: usize,
        generation_advances: u64,
    ) -> Result<ByteArenaHeaderImage, FixedStorageError> {
        let expected = self.prepare_reserve::<N>(selection.len)?;
        if expected.as_slice() != selection.as_slice() || generation_advances == 0 {
            return Err(FixedStorageError::NonCanonical);
        }
        let installed = installed_occupied
            .checked_add(installed_inactive)
            .ok_or(FixedStorageError::Capacity)?;
        if installed > selection.len {
            return Err(FixedStorageError::NonCanonical);
        }
        let selected = u32::try_from(selection.len).map_err(|_| FixedStorageError::Capacity)?;
        let occupied =
            u32::try_from(installed_occupied).map_err(|_| FixedStorageError::Capacity)?;
        let inactive =
            u32::try_from(installed_inactive).map_err(|_| FixedStorageError::Capacity)?;
        let mut header = self.header;
        header.generation = header
            .generation
            .checked_add(generation_advances)
            .ok_or(FixedStorageError::Capacity)?;
        header.free_len = header
            .free_len
            .checked_sub(selected)
            .ok_or(FixedStorageError::NonCanonical)?;
        header.reserved_count = header
            .reserved_count
            .checked_add(selected)
            .and_then(|value| value.checked_sub(occupied))
            .and_then(|value| value.checked_sub(inactive))
            .ok_or(FixedStorageError::NonCanonical)?;
        header.occupied = header
            .occupied
            .checked_add(occupied)
            .ok_or(FixedStorageError::Capacity)?;
        header.inactive_count = header
            .inactive_count
            .checked_add(inactive)
            .ok_or(FixedStorageError::Capacity)?;
        Ok(Self::encode_header(header))
    }

    pub(crate) fn prepare_generation_header_after(
        &self,
    ) -> Result<ByteArenaHeaderImage, FixedStorageError> {
        self.prepare_generation_header_after_advances(1)
    }

    pub(crate) fn prepare_generation_header_after_advances(
        &self,
        generation_advances: u64,
    ) -> Result<ByteArenaHeaderImage, FixedStorageError> {
        self.validate_header()?;
        if generation_advances == 0 {
            return Err(FixedStorageError::NonCanonical);
        }
        let mut header = self.header;
        header.generation = header
            .generation
            .checked_add(generation_advances)
            .ok_or(FixedStorageError::Capacity)?;
        Ok(Self::encode_header(header))
    }

    pub(crate) fn prepare_install_reserved_header_after(
        &self,
        references: &[ArenaRef],
        installed_occupied: usize,
        installed_inactive: usize,
    ) -> Result<ByteArenaHeaderImage, FixedStorageError> {
        self.validate_header()?;
        let installed = installed_occupied
            .checked_add(installed_inactive)
            .ok_or(FixedStorageError::Capacity)?;
        if references.is_empty() || installed != references.len() {
            return Err(FixedStorageError::NonCanonical);
        }
        for (index, reference) in references.iter().enumerate() {
            if references[..index].contains(reference) {
                return Err(FixedStorageError::NonCanonical);
            }
            self.image(*reference, &[2])?;
        }
        let occupied =
            u32::try_from(installed_occupied).map_err(|_| FixedStorageError::Capacity)?;
        let inactive =
            u32::try_from(installed_inactive).map_err(|_| FixedStorageError::Capacity)?;
        let installed = u32::try_from(installed).map_err(|_| FixedStorageError::Capacity)?;
        let mut header = self.header;
        header.reserved_count = header
            .reserved_count
            .checked_sub(installed)
            .ok_or(FixedStorageError::NonCanonical)?;
        header.occupied = header
            .occupied
            .checked_add(occupied)
            .ok_or(FixedStorageError::Capacity)?;
        header.inactive_count = header
            .inactive_count
            .checked_add(inactive)
            .ok_or(FixedStorageError::Capacity)?;
        Ok(Self::encode_header(header))
    }

    pub(crate) fn prepare_commit_inactive_header_after(
        &self,
        references: &[ArenaRef],
    ) -> Result<ByteArenaHeaderImage, FixedStorageError> {
        self.validate_commit_inactive_batch(references)?;
        let count = u32::try_from(references.len()).map_err(|_| FixedStorageError::Capacity)?;
        let mut header = self.header;
        header.generation = header
            .generation
            .checked_add(1)
            .ok_or(FixedStorageError::Capacity)?;
        header.inactive_count = header
            .inactive_count
            .checked_sub(count)
            .ok_or(FixedStorageError::NonCanonical)?;
        header.occupied = header
            .occupied
            .checked_add(count)
            .ok_or(FixedStorageError::Capacity)?;
        Ok(Self::encode_header(header))
    }

    pub(crate) fn prepare_committed_inactive_tag_after(
        &self,
        reference: ArenaRef,
    ) -> Result<u8, FixedStorageError> {
        self.image(reference, &[3])?;
        Ok(1)
    }

    pub(crate) fn prepare_release_header_after(
        &self,
        references: &[ArenaRef],
    ) -> Result<ByteArenaHeaderImage, FixedStorageError> {
        self.validate_release_batch(references)?;
        let count = u32::try_from(references.len()).map_err(|_| FixedStorageError::Capacity)?;
        let mut header = self.header;
        for reference in references {
            match self.image(*reference, &[1, 2, 3])?[4] {
                1 => {
                    header.occupied = header
                        .occupied
                        .checked_sub(1)
                        .ok_or(FixedStorageError::NonCanonical)?;
                }
                2 => {
                    header.reserved_count = header
                        .reserved_count
                        .checked_sub(1)
                        .ok_or(FixedStorageError::NonCanonical)?;
                }
                3 => {
                    header.inactive_count = header
                        .inactive_count
                        .checked_sub(1)
                        .ok_or(FixedStorageError::NonCanonical)?;
                }
                _ => unreachable!("validated fixed arena release tag"),
            }
        }
        header.free_len = header
            .free_len
            .checked_add(count)
            .filter(|value| *value <= header.capacity)
            .ok_or(FixedStorageError::Capacity)?;
        header.generation = header
            .generation
            .checked_add(1)
            .ok_or(FixedStorageError::Capacity)?;
        Ok(Self::encode_header(header))
    }

    pub(crate) fn prepare_release_outcome_after(
        &self,
        reference: ArenaRef,
        ordinal: usize,
    ) -> Result<(u32, [u8; T], ByteArenaFreeCellImage), FixedStorageError> {
        self.validate_header()?;
        self.image(reference, &[1, 2, 3])?;
        let position = (self.header.free_len as usize)
            .checked_add(ordinal)
            .filter(|position| *position < self.header.capacity as usize)
            .ok_or(FixedStorageError::Capacity)?;
        let position = u32::try_from(position).map_err(|_| FixedStorageError::Capacity)?;
        let mut slot = [0; T];
        write_u32(&mut slot, 0, reference.generation);
        write_u32(&mut slot, 8, position);
        Ok((
            position,
            slot,
            ByteArenaFreeCellImage(reference.slot.to_le_bytes()),
        ))
    }

    pub(crate) fn prepare_reserved_image_after(
        &self,
        reference: ArenaRef,
        mut image: [u8; T],
        tag: u8,
    ) -> Result<[u8; T], FixedStorageError> {
        if !matches!(tag, 1 | 3)
            || image[..8].iter().any(|byte| *byte != 0)
            || reference.generation == 0
        {
            return Err(FixedStorageError::NonCanonical);
        }
        image[..8].fill(0);
        write_u32(&mut image, 0, reference.generation);
        image[4] = tag;
        Ok(image)
    }

    pub(crate) fn reserve_selection_direct<const N: usize>(
        &mut self,
        selection: &ArenaSelection<N>,
    ) {
        for reference in selection.as_slice() {
            let slot = self.slot_mut_prevalidated(reference.slot as usize);
            slot.fill(0);
            write_u32(slot, 0, reference.generation);
            slot[4] = 2;
        }
    }

    pub(crate) fn install_reserved_image_direct(&mut self, reference: ArenaRef, image: [u8; T]) {
        *self.slot_mut_prevalidated(reference.slot as usize) = image;
    }

    pub(crate) fn assign_slot_tag_direct(&mut self, reference: ArenaRef, tag: u8) {
        self.slot_mut_prevalidated(reference.slot as usize)[4] = tag;
    }

    pub(crate) fn assign_free_cell_direct(&mut self, position: u32, image: ByteArenaFreeCellImage) {
        let start = position as usize * 4;
        self.free[start..start + 4].copy_from_slice(&image.0);
    }

    pub(crate) fn assign_header_direct(&mut self, image: ByteArenaHeaderImage) {
        self.header = ByteArenaHeader {
            generation: u64::from_le_bytes(
                image.0[0..8].try_into().expect("fixed arena generation"),
            ),
            occupied: read_u32(&image.0, 8),
            capacity: read_u32(&image.0, 12),
            free_len: read_u32(&image.0, 16),
            reserved_count: read_u32(&image.0, 20),
            inactive_count: read_u32(&image.0, 24),
            reserved: read_u32(&image.0, 28),
        };
    }

    fn encode_header(header: ByteArenaHeader) -> ByteArenaHeaderImage {
        let mut image = ByteArenaHeaderImage::ZERO;
        image.0[0..8].copy_from_slice(&header.generation.to_le_bytes());
        write_u32(&mut image.0, 8, header.occupied);
        write_u32(&mut image.0, 12, header.capacity);
        write_u32(&mut image.0, 16, header.free_len);
        write_u32(&mut image.0, 20, header.reserved_count);
        write_u32(&mut image.0, 24, header.inactive_count);
        write_u32(&mut image.0, 28, header.reserved);
        image
    }

    #[cfg_attr(
        not(test),
        allow(dead_code, reason = "C17 structural tests consume this observer")
    )]
    pub(crate) const fn capacity(&self) -> usize {
        self.header.capacity as usize
    }

    #[cfg_attr(
        not(test),
        allow(dead_code, reason = "C17 structural tests consume this observer")
    )]
    pub(crate) const fn occupied(&self) -> usize {
        self.header.occupied as usize
    }

    pub(crate) const fn free_len(&self) -> usize {
        self.header.free_len as usize
    }

    #[cfg_attr(
        not(test),
        allow(dead_code, reason = "C17 structural tests consume this observer")
    )]
    pub(crate) const fn reserved_count(&self) -> usize {
        self.header.reserved_count as usize
    }

    #[cfg_attr(
        not(test),
        allow(dead_code, reason = "C17 structural tests consume this observer")
    )]
    pub(crate) const fn inactive_count(&self) -> usize {
        self.header.inactive_count as usize
    }

    pub(crate) fn prepare_reserve<const N: usize>(
        &self,
        count: usize,
    ) -> Result<ArenaSelection<N>, FixedStorageError> {
        self.validate_header()?;
        if count == 0 || count > N || count > self.header.free_len as usize {
            return Err(FixedStorageError::Capacity);
        }
        self.header
            .generation
            .checked_add(1)
            .ok_or(FixedStorageError::Capacity)?;
        let start = self.header.free_len as usize - count;
        let mut refs = [ArenaRef::default(); N];
        for (ordinal, position) in (start..self.header.free_len as usize).rev().enumerate() {
            let index = read_u32(&self.free, position * 4) as usize;
            let slot = self.slot(index)?;
            if slot[4] != 0 || read_u32(slot, 8) as usize != position {
                return Err(FixedStorageError::NonCanonical);
            }
            let generation = read_u32(slot, 0)
                .checked_add(1)
                .ok_or(FixedStorageError::Capacity)?;
            refs[ordinal] = ArenaRef {
                slot: index as u32,
                generation,
            };
        }
        Ok(ArenaSelection { refs, len: count })
    }

    pub(crate) fn commit_reserve<const N: usize>(
        &mut self,
        selection: &ArenaSelection<N>,
    ) -> Result<(), FixedStorageError> {
        let expected = self.prepare_reserve::<N>(selection.len)?;
        if expected.as_slice() != selection.as_slice() {
            return Err(FixedStorageError::NonCanonical);
        }
        let next_generation = self
            .header
            .generation
            .checked_add(1)
            .ok_or(FixedStorageError::Capacity)?;
        for reference in selection.as_slice() {
            let slot = self.slot_mut(reference.slot as usize)?;
            slot.fill(0);
            write_u32(slot, 0, reference.generation);
            slot[4] = 2;
        }
        self.header.free_len -= selection.len as u32;
        self.header.reserved_count = self
            .header
            .reserved_count
            .checked_add(selection.len as u32)
            .ok_or(FixedStorageError::Capacity)?;
        self.header.generation = next_generation;
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn reserve<const N: usize>(
        &mut self,
        count: usize,
    ) -> Result<ArenaSelection<N>, FixedStorageError> {
        let selection = self.prepare_reserve::<N>(count)?;
        self.commit_reserve(&selection)?;
        Ok(selection)
    }

    pub(crate) fn install_reserved(
        &mut self,
        reference: ArenaRef,
        mut image: [u8; T],
        tag: u8,
    ) -> Result<(), FixedStorageError> {
        if !matches!(tag, 1 | 3) {
            return Err(FixedStorageError::NonCanonical);
        }
        let slot = self.slot(reference.slot as usize)?;
        if slot[4] != 2 || read_u32(slot, 0) != reference.generation {
            return Err(FixedStorageError::NonCanonical);
        }
        if image[..8].iter().any(|byte| *byte != 0) {
            return Err(FixedStorageError::NonCanonical);
        }
        image[..8].fill(0);
        write_u32(&mut image, 0, reference.generation);
        image[4] = tag;
        *self.slot_mut(reference.slot as usize)? = image;
        self.header.reserved_count = self
            .header
            .reserved_count
            .checked_sub(1)
            .ok_or(FixedStorageError::NonCanonical)?;
        if tag == 1 {
            self.header.occupied = self
                .header
                .occupied
                .checked_add(1)
                .ok_or(FixedStorageError::Capacity)?;
        } else {
            self.header.inactive_count = self
                .header
                .inactive_count
                .checked_add(1)
                .ok_or(FixedStorageError::Capacity)?;
        }
        Ok(())
    }

    pub(crate) fn commit_inactive(&mut self, reference: ArenaRef) -> Result<(), FixedStorageError> {
        let slot = self.slot_mut(reference.slot as usize)?;
        if slot[4] != 3 || read_u32(slot, 0) != reference.generation {
            return Err(FixedStorageError::NonCanonical);
        }
        slot[4] = 1;
        self.header.inactive_count = self
            .header
            .inactive_count
            .checked_sub(1)
            .ok_or(FixedStorageError::NonCanonical)?;
        self.header.occupied = self
            .header
            .occupied
            .checked_add(1)
            .ok_or(FixedStorageError::Capacity)?;
        Ok(())
    }

    pub(crate) fn reference_at(
        &self,
        slot: u32,
        accepted_tags: &[u8],
    ) -> Result<ArenaRef, FixedStorageError> {
        let image = self.slot(slot as usize)?;
        if !accepted_tags.contains(&image[4]) {
            return Err(FixedStorageError::NonCanonical);
        }
        let generation = read_u32(image, 0);
        if generation == 0 {
            return Err(FixedStorageError::NonCanonical);
        }
        Ok(ArenaRef { slot, generation })
    }
}
