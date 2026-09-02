use crate::work::{WorkMeter, WorkRecorder};
use crate::{Duration, MonotonicTime, WorkBudgetError, WorkDimension};
use std::{
    collections::VecDeque,
    mem::{size_of, size_of_val},
};

/// A checked fixed-capacity collection insertion failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BoundedCollectionError {
    Full,
    Duplicate,
}

/// An insertion-ordered vector whose storage and maximum length are fixed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BoundedVec<T, const CAPACITY: usize> {
    slots: [Option<T>; CAPACITY],
    len: usize,
}

impl<T, const CAPACITY: usize> BoundedVec<T, CAPACITY> {
    #[must_use]
    pub fn new() -> Self {
        Self {
            slots: std::array::from_fn(|_| None),
            len: 0,
        }
    }

    #[must_use]
    pub const fn len(&self) -> usize {
        self.len
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    #[must_use]
    pub const fn capacity(&self) -> usize {
        CAPACITY
    }

    pub fn try_push(&mut self, value: T) -> Result<(), BoundedCollectionError> {
        if self.len == CAPACITY {
            return Err(BoundedCollectionError::Full);
        }
        self.slots[self.len] = Some(value);
        self.len += 1;
        Ok(())
    }

    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &T> + ExactSizeIterator {
        self.slots[..self.len]
            .iter()
            .map(|slot| slot.as_ref().expect("occupied bounded-vector prefix"))
    }

    pub(crate) fn get(&self, index: usize) -> Option<&T> {
        self.slots.get(index)?.as_ref()
    }

    pub(crate) fn as_slice(&self) -> &[Option<T>] {
        &self.slots[..self.len]
    }

    pub(crate) fn ordered_at(&self, index: usize, value: &T) -> bool
    where
        T: Ord,
    {
        index <= self.len
            && self
                .get(index.wrapping_sub(1))
                .is_none_or(|existing| existing < value)
            && self.get(index).is_none_or(|existing| value < existing)
    }

    pub(crate) fn insert_at(&mut self, index: usize, value: T) {
        assert!(self.len < CAPACITY && index <= self.len);
        for cursor in (index..self.len).rev() {
            self.slots[cursor + 1] = self.slots[cursor].take();
        }
        self.slots[index] = Some(value);
        self.len += 1;
    }
}

impl<T, const CAPACITY: usize> Default for BoundedVec<T, CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}

/// An insertion-ordered unique set with fixed storage.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BoundedSet<T, const CAPACITY: usize>(BoundedVec<T, CAPACITY>);

impl<T: Eq, const CAPACITY: usize> BoundedSet<T, CAPACITY> {
    #[must_use]
    pub fn new() -> Self {
        Self(BoundedVec::new())
    }

    #[must_use]
    pub const fn len(&self) -> usize {
        self.0.len()
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    #[must_use]
    pub fn contains(&self, value: &T) -> bool {
        self.0.iter().any(|existing| existing == value)
    }

    pub fn try_insert(&mut self, value: T) -> Result<(), BoundedCollectionError> {
        if self.contains(&value) {
            return Err(BoundedCollectionError::Duplicate);
        }
        self.0.try_push(value)
    }

    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &T> + ExactSizeIterator {
        self.0.iter()
    }
}

impl<T: Eq, const CAPACITY: usize> Default for BoundedSet<T, CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}

/// An insertion-ordered key/value map with fixed storage.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BoundedMap<K, V, const CAPACITY: usize>(BoundedVec<(K, V), CAPACITY>);

impl<K: Eq, V, const CAPACITY: usize> BoundedMap<K, V, CAPACITY> {
    #[must_use]
    pub fn new() -> Self {
        Self(BoundedVec::new())
    }

    #[must_use]
    pub const fn len(&self) -> usize {
        self.0.len()
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    #[must_use]
    pub fn get(&self, key: &K) -> Option<&V> {
        self.0
            .iter()
            .find(|(existing, _)| existing == key)
            .map(|(_, value)| value)
    }

    pub fn try_insert(&mut self, key: K, value: V) -> Result<(), BoundedCollectionError> {
        if self.get(&key).is_some() {
            return Err(BoundedCollectionError::Duplicate);
        }
        self.0.try_push((key, value))
    }
}

impl<K: Eq, V, const CAPACITY: usize> Default for BoundedMap<K, V, CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}

const BRANCH_TAG: u32 = 1 << 31;
const NO_NODE: u32 = u32::MAX;
const IDENTITY_BYTES: u64 = 33;
const IDENTITY_BITS: u16 = 33 * 8;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FixedIndexError {
    Allocation,
    Capacity,
    Duplicate,
    NonCanonical,
    Work(WorkBudgetError),
}

impl From<WorkBudgetError> for FixedIndexError {
    fn from(error: WorkBudgetError) -> Self {
        Self::Work(error)
    }
}

#[derive(Debug, Eq, PartialEq)]
struct IndexLeaf<V> {
    key: [u8; 33],
    value: V,
}

#[derive(Debug, Eq, PartialEq)]
struct IndexBranch {
    bit: u16,
    zero: u32,
    one: u32,
}

#[derive(Debug, Eq, PartialEq)]
pub struct FixedIdentityIndex<V> {
    leaves: Vec<IndexLeaf<V>>,
    branches: Vec<IndexBranch>,
    root: u32,
    capacity: usize,
}

impl<V: Copy> FixedIdentityIndex<V> {
    pub fn try_new(capacity: usize) -> Result<Self, FixedIndexError> {
        if capacity >= BRANCH_TAG as usize {
            return Err(FixedIndexError::Capacity);
        }
        Ok(Self {
            leaves: reserved_index(capacity)?,
            branches: reserved_index(capacity.saturating_sub(1))?,
            root: NO_NODE,
            capacity,
        })
    }

    pub fn len(&self) -> usize {
        self.leaves.len()
    }

    pub fn is_empty(&self) -> bool {
        self.leaves.is_empty()
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn find(&self, key: [u8; 33], work: &mut WorkMeter) -> Result<Option<V>, FixedIndexError> {
        self.find_with(key, work)
    }

    fn find_with<W: WorkRecorder + ?Sized>(
        &self,
        key: [u8; 33],
        work: &mut W,
    ) -> Result<Option<V>, FixedIndexError> {
        if self.root == NO_NODE {
            return Ok(None);
        }
        let mut node = self.root;
        while is_branch(node) {
            work.record(WorkDimension::VisitedEntities, 1)?;
            let branch = &self.branches[branch_index(node)];
            node = [branch.zero, branch.one][identity_bit(&key, branch.bit)];
        }
        work.record(WorkDimension::VisitedEntities, 1)?;
        let leaf = &self.leaves[node as usize];
        Ok((leaf.key == key).then_some(leaf.value))
    }

    pub fn try_insert_sorted(
        &mut self,
        entries: &[([u8; 33], V)],
        work: &mut WorkMeter,
    ) -> Result<(), FixedIndexError> {
        self.try_insert_sorted_with(entries, work)
    }

    fn try_insert_sorted_with<W: WorkRecorder + ?Sized>(
        &mut self,
        entries: &[([u8; 33], V)],
        work: &mut W,
    ) -> Result<(), FixedIndexError> {
        work.record(WorkDimension::InvariantChecks, 1)?;
        let count = entries.len();
        if count > self.capacity - self.leaves.len() {
            return Err(FixedIndexError::Capacity);
        }
        work.record(WorkDimension::InvariantChecks, 1)?;
        let count = u64::try_from(count).map_err(|_| {
            FixedIndexError::Work(WorkBudgetError::CounterOverflow(
                WorkDimension::InvariantChecks,
            ))
        })?;
        let new_branches = count - u64::from(self.root == NO_NODE && count > 0);
        let copied = count
            .checked_mul(std::mem::size_of::<IndexLeaf<V>>() as u64)
            .and_then(|bytes| {
                new_branches
                    .checked_mul(std::mem::size_of::<IndexBranch>() as u64)
                    .and_then(|branch_bytes| bytes.checked_add(branch_bytes))
            })
            .ok_or(FixedIndexError::Work(WorkBudgetError::CounterOverflow(
                WorkDimension::CopiedBytes,
            )))?;
        let maximum_insert_visits =
            count * (2 * IDENTITY_BITS as u64 + 1) + new_branches * IDENTITY_BYTES;
        let maximum_visits = count * (IDENTITY_BITS as u64 + 1) + maximum_insert_visits;
        work.ensure(crate::HotPathWorkWitness::new([
            maximum_visits,
            copied,
            0,
            0,
            count,
        ]))?;
        let mut previous = None;
        for &(key, _) in entries {
            work.record(WorkDimension::InvariantChecks, 1)?;
            if let Some(prior) = previous {
                if prior == key {
                    return Err(FixedIndexError::Duplicate);
                }
                if prior > key {
                    return Err(FixedIndexError::NonCanonical);
                }
            }
            if self.find_with(key, work)?.is_some() {
                return Err(FixedIndexError::Duplicate);
            }
            previous = Some(key);
        }
        work.record(WorkDimension::CopiedBytes, copied)?;
        let mut insert_visits = 0;
        for &(key, value) in entries {
            insert_visits += self.insert(key, value);
        }
        work.record(WorkDimension::VisitedEntities, insert_visits)
            .expect("preflight covers exact insertion work");
        Ok(())
    }

    fn insert(&mut self, key: [u8; 33], value: V) -> u64 {
        let leaf = self.leaves.len() as u32;
        self.leaves.push(IndexLeaf { key, value });
        if self.root == NO_NODE {
            self.root = leaf;
            return 0;
        }
        let mut visits = 0;
        let mut peer = self.root;
        while is_branch(peer) {
            visits += 1;
            let branch = &self.branches[branch_index(peer)];
            peer = [branch.zero, branch.one][identity_bit(&key, branch.bit)];
        }
        visits += 1;
        let (bit, byte_visits) = first_difference(&key, &self.leaves[peer as usize].key);
        visits += byte_visits;
        let (mut parent, mut node) = (NO_NODE, self.root);
        while is_branch(node) {
            visits += 1;
            if self.branches[branch_index(node)].bit >= bit {
                break;
            }
            parent = node;
            let branch = &self.branches[branch_index(node)];
            node = [branch.zero, branch.one][identity_bit(&key, branch.bit)];
        }
        let children = if identity_bit(&key, bit) == 0 {
            [leaf, node]
        } else {
            [node, leaf]
        };
        let branch = BRANCH_TAG | self.branches.len() as u32;
        self.branches.push(IndexBranch {
            bit,
            zero: children[0],
            one: children[1],
        });
        if parent == NO_NODE {
            self.root = branch;
        } else {
            let parent = &mut self.branches[branch_index(parent)];
            *[&mut parent.zero, &mut parent.one][identity_bit(&key, parent.bit)] = branch;
        }
        visits
    }
}

fn reserved_index<T>(capacity: usize) -> Result<Vec<T>, FixedIndexError> {
    let mut values = Vec::new();
    values
        .try_reserve_exact(capacity)
        .map_err(|_| FixedIndexError::Allocation)?;
    if values.capacity() != capacity {
        return Err(FixedIndexError::Capacity);
    }
    Ok(values)
}

/// A fixed index preallocated once and filled with a sentinel, for bucket
/// heads and intrusive links that must be addressable from construction.
fn vec_filled(capacity: usize, value: u32) -> Result<Vec<u32>, FixedIndexError> {
    let mut values = reserved_index(capacity)?;
    values.resize(capacity, value);
    Ok(values)
}

fn is_branch(node: u32) -> bool {
    node & BRANCH_TAG != 0
}

fn branch_index(node: u32) -> usize {
    (node & !BRANCH_TAG) as usize
}

fn identity_bit(bytes: &[u8; 33], bit: u16) -> usize {
    ((bytes[bit as usize / 8] >> (7 - bit % 8)) & 1) as usize
}

fn first_difference(left: &[u8; 33], right: &[u8; 33]) -> (u16, u64) {
    for (index, (left, right)) in left.iter().zip(right).enumerate() {
        let difference = left ^ right;
        if difference != 0 {
            return (
                index as u16 * 8 + difference.leading_zeros() as u16,
                index as u64 + 1,
            );
        }
    }
    unreachable!("distinct fixed identities have distinct bytes")
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct AvlNode {
    key: [u8; 33],
    record: u32,
    left: u32,
    right: u32,
    parent: u32,
    height: u8,
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct AvlIndex {
    nodes: Vec<AvlNode>,
    root: u32,
    /// Head of the intrusive free list threaded through vacated slots' `left`
    /// links. Nodes address one another by index, so a removed slot is retained
    /// and reused rather than shifted out; the list costs no extra storage
    /// because it reuses the padding beside `root`.
    free_head: u32,
    free_len: u32,
    capacity: usize,
}

impl AvlIndex {
    fn try_new(capacity: usize) -> Result<Self, FixedStorageError> {
        if capacity >= u32::MAX as usize {
            return Err(FixedStorageError::Capacity);
        }
        Ok(Self {
            nodes: reserved_index(capacity)?,
            root: NO_NODE,
            free_head: NO_NODE,
            free_len: 0,
            capacity,
        })
    }

    pub(crate) fn height_bound(capacity: usize) -> Result<u8, FixedStorageError> {
        let mut prior = 0usize;
        let mut current = 1usize;
        let mut height = 1u8;
        while current <= capacity {
            let next = 1usize
                .checked_add(current)
                .and_then(|value| value.checked_add(prior))
                .ok_or(FixedStorageError::Capacity)?;
            prior = current;
            current = next;
            height = height.checked_add(1).ok_or(FixedStorageError::Capacity)?;
        }
        Ok(height - 1)
    }

    fn find<W: WorkRecorder + ?Sized>(
        &self,
        key: [u8; 33],
        work: &mut W,
    ) -> Result<Option<u32>, FixedStorageError> {
        let mut node = self.root;
        while node != NO_NODE {
            work.record(WorkDimension::VisitedEntities, 1)?;
            let current = self
                .nodes
                .get(node as usize)
                .ok_or(FixedStorageError::NonCanonical)?;
            match key.cmp(&current.key) {
                std::cmp::Ordering::Less => node = current.left,
                std::cmp::Ordering::Greater => node = current.right,
                std::cmp::Ordering::Equal => return Ok(Some(current.record)),
            }
        }
        Ok(None)
    }

    fn find_precharged(&self, key: [u8; 33]) -> Result<Option<u32>, FixedStorageError> {
        let mut node = self.root;
        while node != NO_NODE {
            let current = self
                .nodes
                .get(node as usize)
                .ok_or(FixedStorageError::NonCanonical)?;
            match key.cmp(&current.key) {
                std::cmp::Ordering::Less => node = current.left,
                std::cmp::Ordering::Greater => node = current.right,
                std::cmp::Ordering::Equal => return Ok(Some(current.record)),
            }
        }
        Ok(None)
    }

    /// The number of live keys. Removed slots stay in `nodes` for index
    /// stability, so the physical length is not the live count.
    pub(crate) fn live(&self) -> usize {
        self.nodes.len() - self.free_len as usize
    }

    fn claim_slot(&mut self, node: AvlNode) -> u32 {
        if self.free_head == NO_NODE {
            let slot = u32::try_from(self.nodes.len()).expect("constructor-bounded AVL index");
            self.nodes.push(node);
            return slot;
        }
        let slot = self.free_head;
        self.free_head = self.nodes[slot as usize].left;
        self.free_len -= 1;
        self.nodes[slot as usize] = node;
        slot
    }

    /// Returns one vacated slot to the intrusive free list. The slot keeps its
    /// position so every other node's stored index stays valid.
    fn release_slot(&mut self, slot: u32) {
        self.nodes[slot as usize] = AvlNode {
            key: [0; 33],
            record: 0,
            left: self.free_head,
            right: NO_NODE,
            parent: NO_NODE,
            height: 0,
        };
        self.free_head = slot;
        self.free_len += 1;
    }

    fn insert_prevalidated(&mut self, key: [u8; 33], record: u32) {
        let mut parent = NO_NODE;
        let mut node = self.root;
        while node != NO_NODE {
            parent = node;
            let current = &self.nodes[node as usize];
            node = if key < current.key {
                current.left
            } else {
                current.right
            };
        }
        let index = self.claim_slot(AvlNode {
            key,
            record,
            left: NO_NODE,
            right: NO_NODE,
            parent,
            height: 1,
        });
        if parent == NO_NODE {
            self.root = index;
            return;
        }
        if key < self.nodes[parent as usize].key {
            self.nodes[parent as usize].left = index;
        } else {
            self.nodes[parent as usize].right = index;
        }
        self.rebalance_after_insert(parent);
    }

    /// Removes one key and returns the record it addressed. Deletion splices the
    /// in-order successor when the target has two children, rebalances the whole
    /// parent chain, and returns the vacated slot to the free list. Every other
    /// node keeps its index, so record indices held elsewhere stay valid.
    fn remove(&mut self, key: [u8; 33]) -> Option<u32> {
        let mut node = self.root;
        while node != NO_NODE {
            let current = &self.nodes[node as usize];
            if key == current.key {
                break;
            }
            node = if key < current.key {
                current.left
            } else {
                current.right
            };
        }
        if node == NO_NODE {
            return None;
        }
        let record = self.nodes[node as usize].record;
        // With two children the successor's key and record move into this node
        // and the successor itself becomes the physically removed slot; it has
        // no left child by construction.
        let target = if self.nodes[node as usize].left != NO_NODE
            && self.nodes[node as usize].right != NO_NODE
        {
            let mut successor = self.nodes[node as usize].right;
            while self.nodes[successor as usize].left != NO_NODE {
                successor = self.nodes[successor as usize].left;
            }
            self.nodes[node as usize].key = self.nodes[successor as usize].key;
            self.nodes[node as usize].record = self.nodes[successor as usize].record;
            successor
        } else {
            node
        };
        let value = self.nodes[target as usize];
        let child = if value.left != NO_NODE {
            value.left
        } else {
            value.right
        };
        if child != NO_NODE {
            self.nodes[child as usize].parent = value.parent;
        }
        if value.parent == NO_NODE {
            self.root = child;
        } else if self.nodes[value.parent as usize].left == target {
            self.nodes[value.parent as usize].left = child;
        } else {
            self.nodes[value.parent as usize].right = child;
        }
        self.release_slot(target);
        self.rebalance_after_delete(value.parent);
        Some(record)
    }

    /// Unlike an insertion, a deletion can shorten a subtree, so rebalancing
    /// continues above a rotation instead of stopping at the first one.
    fn rebalance_after_delete(&mut self, mut node: u32) {
        while node != NO_NODE {
            let old_height = self.nodes[node as usize].height;
            self.refresh(node);
            let balance = self.balance(node);
            let settled = if balance == 2 {
                let left = self.nodes[node as usize].left;
                if self.balance(left) < 0 {
                    self.rotate_left(left);
                }
                self.rotate_right(node)
            } else if balance == -2 {
                let right = self.nodes[node as usize].right;
                if self.balance(right) > 0 {
                    self.rotate_right(right);
                }
                self.rotate_left(node)
            } else {
                if self.nodes[node as usize].height == old_height {
                    return;
                }
                node
            };
            node = self.nodes[settled as usize].parent;
        }
    }

    fn height(&self, node: u32) -> u8 {
        if node == NO_NODE {
            0
        } else {
            self.nodes[node as usize].height
        }
    }

    fn refresh(&mut self, node: u32) {
        let value = self.nodes[node as usize];
        self.nodes[node as usize].height =
            1 + self.height(value.left).max(self.height(value.right));
    }

    fn balance(&self, node: u32) -> i16 {
        let value = self.nodes[node as usize];
        i16::from(self.height(value.left)) - i16::from(self.height(value.right))
    }

    fn rebalance_after_insert(&mut self, mut node: u32) {
        while node != NO_NODE {
            let old_height = self.nodes[node as usize].height;
            self.refresh(node);
            let balance = self.balance(node);
            if balance == 2 {
                let left = self.nodes[node as usize].left;
                if self.balance(left) < 0 {
                    self.rotate_left(left);
                }
                self.rotate_right(node);
                break;
            } else if balance == -2 {
                let right = self.nodes[node as usize].right;
                if self.balance(right) > 0 {
                    self.rotate_right(right);
                }
                self.rotate_left(node);
                break;
            } else if self.nodes[node as usize].height == old_height {
                break;
            }
            node = self.nodes[node as usize].parent;
        }
    }

    fn rotate_left(&mut self, node: u32) -> u32 {
        let promoted = self.nodes[node as usize].right;
        let displaced = self.nodes[promoted as usize].left;
        self.replace_parent(node, promoted);
        self.nodes[promoted as usize].left = node;
        self.nodes[node as usize].parent = promoted;
        self.nodes[node as usize].right = displaced;
        if displaced != NO_NODE {
            self.nodes[displaced as usize].parent = node;
        }
        self.refresh(node);
        self.refresh(promoted);
        promoted
    }

    fn rotate_right(&mut self, node: u32) -> u32 {
        let promoted = self.nodes[node as usize].left;
        let displaced = self.nodes[promoted as usize].right;
        self.replace_parent(node, promoted);
        self.nodes[promoted as usize].right = node;
        self.nodes[node as usize].parent = promoted;
        self.nodes[node as usize].left = displaced;
        if displaced != NO_NODE {
            self.nodes[displaced as usize].parent = node;
        }
        self.refresh(node);
        self.refresh(promoted);
        promoted
    }

    fn replace_parent(&mut self, old: u32, new: u32) {
        let parent = self.nodes[old as usize].parent;
        self.nodes[new as usize].parent = parent;
        if parent == NO_NODE {
            self.root = new;
        } else if self.nodes[parent as usize].left == old {
            self.nodes[parent as usize].left = new;
        } else {
            self.nodes[parent as usize].right = new;
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FixedStorageError {
    Allocation,
    Capacity,
    Duplicate,
    NonCanonical,
    InvalidTime,
    WindowExceeded,
    Work(WorkBudgetError),
}

impl From<WorkBudgetError> for FixedStorageError {
    fn from(error: WorkBudgetError) -> Self {
        Self::Work(error)
    }
}

impl From<FixedIndexError> for FixedStorageError {
    fn from(error: FixedIndexError) -> Self {
        match error {
            FixedIndexError::Allocation => Self::Allocation,
            FixedIndexError::Capacity => Self::Capacity,
            FixedIndexError::Duplicate => Self::Duplicate,
            FixedIndexError::NonCanonical => Self::NonCanonical,
            FixedIndexError::Work(error) => Self::Work(error),
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
pub struct FixedRecordArena<V, C, const KEYS: usize> {
    records: Vec<V>,
    claims: Vec<C>,
    /// One packed `(start, len)` claim span per record slot. A prefix sum
    /// cannot express a hole, and reclaiming a terminal group leaves holes, so
    /// each slot addresses its claims explicitly. A vacated slot stores the
    /// next free slot index instead and is marked by `VACANT_SLOT`.
    claim_spans: Vec<u64>,
    identities: AvlIndex,
    /// Head of the intrusive vacated-slot list threaded through `claim_spans`.
    free_record_head: u32,
    free_records: u32,
    /// Released claim spans, bucketed by exact length. `free_span_head[len]`
    /// is the first free span of that length and `free_span_next[start]` links
    /// to the next, so reuse is a constant-time pop rather than a scan. A
    /// linear search here would be unbounded hot-path work under ADR 0039.
    free_span_head: Vec<u32>,
    free_span_next: Vec<u32>,
    free_spans: u32,
    claim_high_water: usize,
    record_capacity: usize,
    claim_capacity: usize,
}

/// Marks a `claim_spans` entry as a vacated-slot link rather than a span.
const VACANT_SLOT: u64 = 1 << 63;

const fn span(start: usize, len: usize) -> u64 {
    ((start as u64) << 32) | len as u64
}

const fn span_parts(value: u64) -> (usize, usize) {
    ((value >> 32) as usize, (value & 0xFFFF_FFFF) as usize)
}

impl<V, C: Copy, const KEYS: usize> FixedRecordArena<V, C, KEYS> {
    pub fn try_new(
        record_capacity: usize,
        claim_capacity: usize,
    ) -> Result<Self, FixedStorageError> {
        if KEYS == 0 || record_capacity > u32::MAX as usize || claim_capacity > u32::MAX as usize {
            return Err(FixedStorageError::Capacity);
        }
        Ok(Self {
            records: reserved_index(record_capacity)?,
            claims: reserved_index(claim_capacity)?,
            claim_spans: reserved_index(record_capacity)?,
            free_record_head: NO_NODE,
            free_records: 0,
            free_span_head: vec_filled(claim_capacity + 1, NO_NODE)?,
            free_span_next: vec_filled(claim_capacity, NO_NODE)?,
            free_spans: 0,
            claim_high_water: 0,
            identities: AvlIndex::try_new(
                record_capacity
                    .checked_mul(KEYS)
                    .ok_or(FixedStorageError::Capacity)?,
            )?,
            record_capacity,
            claim_capacity,
        })
    }

    pub fn try_push(
        &mut self,
        keys: [[u8; 33]; KEYS],
        record: V,
        claims: &[C],
        work: &mut WorkMeter,
    ) -> Result<usize, FixedStorageError> {
        self.try_push_with(keys, record, claims, work)
    }

    fn try_push_with<W: WorkRecorder + ?Sized>(
        &mut self,
        keys: [[u8; 33]; KEYS],
        record: V,
        claims: &[C],
        work: &mut W,
    ) -> Result<usize, FixedStorageError> {
        work.record(WorkDimension::InvariantChecks, 2)?;
        self.validate_capacity(claims.len())?;
        for key in keys {
            work.record(WorkDimension::InvariantChecks, 1)?;
            if self.identities.find(key, work)?.is_some() {
                return Err(FixedStorageError::Duplicate);
            }
        }
        let height = u64::from(AvlIndex::height_bound(self.identities.capacity)?);
        let copied = size_of::<V>()
            .checked_add(size_of::<u32>())
            .and_then(|value| value.checked_add(size_of_val(claims)))
            .and_then(|value| value.checked_add(KEYS.checked_mul(size_of::<AvlNode>())?))
            .ok_or(FixedStorageError::Capacity)?;
        work.charge(crate::HotPathWorkWitness::new([
            KEYS as u64 * (4 * height + 17),
            u64::try_from(copied).map_err(|_| FixedStorageError::Capacity)?,
            0,
            0,
            0,
        ]))?;
        Ok(self.push_prevalidated(keys, record, claims))
    }

    pub(crate) fn validate_capacity(&self, claims: usize) -> Result<(), FixedStorageError> {
        // Reclaimed slots, spans and identity nodes are all reusable, so each
        // bound is checked against live occupancy rather than a physical
        // length that only ever grows.
        // Constant time: either the exact-length bucket holds a released span
        // or the bump pointer still has room. No scan on a preflight path.
        // A run longer than the whole claim arena can never be placed; index
        // the bucket only after proving the length is addressable, or an
        // oversized request panics instead of failing closed.
        let span_available = claims == 0
            || (claims <= self.claim_capacity && self.free_span_head[claims] != NO_NODE)
            || self
                .claim_high_water
                .checked_add(claims)
                .is_some_and(|end| end <= self.claim_capacity);
        if self.live() == self.record_capacity
            || !span_available
            || self
                .identities
                .live()
                .checked_add(KEYS)
                .is_none_or(|end| end > self.identities.capacity)
        {
            return Err(FixedStorageError::Capacity);
        }
        Ok(())
    }

    pub(crate) fn push_prevalidated(
        &mut self,
        keys: [[u8; 33]; KEYS],
        record: V,
        claims: &[C],
    ) -> usize {
        let start = self.claim_start(claims.len());
        for (offset, claim) in claims.iter().enumerate() {
            if start + offset < self.claims.len() {
                self.claims[start + offset] = *claim;
            } else {
                self.claims.push(*claim);
            }
        }
        let index = if self.free_record_head == NO_NODE {
            let index = self.records.len();
            self.records.push(record);
            self.claim_spans.push(span(start, claims.len()));
            index
        } else {
            let index = self.free_record_head as usize;
            self.free_record_head = (self.claim_spans[index] & 0xFFFF_FFFF) as u32;
            self.free_records -= 1;
            self.records[index] = record;
            self.claim_spans[index] = span(start, claims.len());
            index
        };
        for key in keys {
            self.identities.insert_prevalidated(key, index as u32);
        }
        index
    }

    /// Places one claim run in constant time: an exact released span of the
    /// same length, or the bump pointer. Spans are never split, so a released
    /// span always returns to the bucket it came from and reuse cannot degrade
    /// into a search.
    fn claim_start(&mut self, len: usize) -> usize {
        if len == 0 || len > self.claim_capacity {
            return self.claim_high_water;
        }
        let head = self.free_span_head[len];
        if head != NO_NODE {
            self.free_span_head[len] = self.free_span_next[head as usize];
            self.free_span_next[head as usize] = NO_NODE;
            self.free_spans -= 1;
            return head as usize;
        }
        let start = self.claim_high_water;
        self.claim_high_water += len;
        start
    }

    /// Returns one released span to its exact-length bucket in constant time.
    fn release_span(&mut self, start: usize, len: usize) {
        self.free_span_next[start] = self.free_span_head[len];
        self.free_span_head[len] = u32::try_from(start).expect("constructor-bounded claim start");
        self.free_spans += 1;
    }

    /// Releases one record slot: its identity keys leave the index, its claim
    /// span returns to the free list, and the slot itself joins the vacated
    /// list. Every other record keeps its index, so indices held elsewhere stay
    /// valid, and the slot may later be reused by a new identity.
    pub(crate) fn remove(&mut self, index: usize, keys: [[u8; 33]; KEYS]) -> bool {
        if index >= self.records.len() || self.claim_spans[index] & VACANT_SLOT != 0 {
            return false;
        }
        for key in keys {
            self.identities.remove(key);
        }
        let (start, len) = span_parts(self.claim_spans[index]);
        if len > 0 {
            self.release_span(start, len);
        }
        self.claim_spans[index] = VACANT_SLOT | u64::from(self.free_record_head);
        self.free_record_head = u32::try_from(index).expect("constructor-bounded record index");
        self.free_records += 1;
        true
    }

    /// The slot the next push will occupy. Vacated slots are reused, so a
    /// caller that must predict the slot cannot use the physical length.
    pub(crate) fn next_slot(&self) -> usize {
        if self.free_record_head == NO_NODE {
            self.records.len()
        } else {
            self.free_record_head as usize
        }
    }

    /// The number of live records. Vacated slots keep their position, so the
    /// physical length is not the live count.
    pub(crate) fn live(&self) -> usize {
        self.records.len() - self.free_records as usize
    }

    /// Vacated record slots available for immediate reuse.
    #[allow(dead_code, reason = "C18 retention tests assert slot reuse")]
    pub(crate) fn free_record_len(&self) -> usize {
        self.free_records as usize
    }

    pub fn find(
        &self,
        key: [u8; 33],
        work: &mut WorkMeter,
    ) -> Result<Option<usize>, FixedStorageError> {
        self.find_with(key, work)
    }

    pub(crate) fn find_with<W: WorkRecorder + ?Sized>(
        &self,
        key: [u8; 33],
        work: &mut W,
    ) -> Result<Option<usize>, FixedStorageError> {
        Ok(self.identities.find(key, work)?.map(|index| index as usize))
    }

    pub(crate) fn find_precharged(
        &self,
        key: [u8; 33],
    ) -> Result<Option<usize>, FixedStorageError> {
        Ok(self
            .identities
            .find_precharged(key)?
            .map(|index| index as usize))
    }

    pub(crate) fn maximum_identity_height(&self) -> Result<u8, FixedStorageError> {
        AvlIndex::height_bound(self.identities.capacity)
    }

    pub fn get(&self, index: usize) -> Option<&V> {
        self.records.get(index)
    }

    pub fn get_mut(&mut self, index: usize) -> Option<&mut V> {
        self.records.get_mut(index)
    }

    pub fn claims(&self, index: usize) -> Option<&[C]> {
        let value = *self.claim_spans.get(index)?;
        if value & VACANT_SLOT != 0 {
            return None;
        }
        let (start, len) = span_parts(value);
        Some(&self.claims[start..start + len])
    }

    pub(crate) fn backing_capacities(&self) -> [usize; 6] {
        [
            self.records.capacity(),
            self.claims.capacity(),
            self.claim_spans.capacity(),
            self.identities.nodes.capacity(),
            self.free_span_head.capacity(),
            self.free_span_next.capacity(),
        ]
    }

    #[cfg(test)]
    pub(crate) fn allocation_facts(&self) -> [(usize, usize); 4] {
        [
            (self.records.as_ptr() as usize, self.records.capacity()),
            (self.claims.as_ptr() as usize, self.claims.capacity()),
            (
                self.claim_spans.as_ptr() as usize,
                self.claim_spans.capacity(),
            ),
            (
                self.identities.nodes.as_ptr() as usize,
                self.identities.nodes.capacity(),
            ),
        ]
    }

    pub fn len(&self) -> usize {
        self.records.len()
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FixedStartCountBound(pub Duration, pub u32);

#[derive(Debug, Eq, PartialEq)]
pub struct FixedWindowCounter<const CELLS: usize, const H: usize> {
    bounds: [[FixedStartCountBound; H]; CELLS],
    history: [VecDeque<MonotonicTime>; CELLS],
}

pub(crate) struct FixedWindowStart(usize, MonotonicTime);

impl<const CELLS: usize, const H: usize> FixedWindowCounter<CELLS, H> {
    /// `bounds` are the active Budget's admission limits; `retained` is the
    /// catalog-wide physical capacity, which is the maximum any production
    /// Budget can retain at the Catalog Retention Horizon. Sizing storage by
    /// the active bound instead would lose the predecessor starts a shorter
    /// Budget must still hand to a longer successor.
    pub fn try_new(
        bounds: [[FixedStartCountBound; H]; CELLS],
        retained: [u32; CELLS],
    ) -> Result<Self, FixedStorageError> {
        if CELLS == 0 || H == 0 || H > 8 {
            return Err(FixedStorageError::Capacity);
        }
        let common = &bounds[0];
        let valid = bounds.iter().all(|cell| {
            cell.iter().zip(common).all(|(bound, reference)| {
                bound.0 == reference.0 && bound.0.as_micros() > 0 && bound.1 > 0
            }) && cell
                .windows(2)
                .all(|pair| pair[0].0 < pair[1].0 && pair[0].1 <= pair[1].1)
        });
        if !valid {
            return Err(FixedStorageError::NonCanonical);
        }
        if bounds
            .iter()
            .zip(&retained)
            .any(|(cell, capacity)| cell[H - 1].1 > *capacity)
        {
            return Err(FixedStorageError::Capacity);
        }
        let mut history = std::array::from_fn(|_| VecDeque::new());
        for (queue, cell) in history.iter_mut().zip(&retained) {
            let capacity = *cell as usize;
            queue
                .try_reserve_exact(capacity)
                .map_err(|_| FixedStorageError::Allocation)?;
            if queue.capacity() != capacity {
                return Err(FixedStorageError::Capacity);
            }
        }
        Ok(Self { bounds, history })
    }

    pub(crate) fn backing_capacities(&self) -> [usize; CELLS] {
        self.history.each_ref().map(VecDeque::capacity)
    }

    pub(crate) fn bounds(&self, cell: usize) -> Option<&[FixedStartCountBound; H]> {
        self.bounds.get(cell)
    }

    pub fn try_start(
        &mut self,
        cell: usize,
        at: MonotonicTime,
        work: &mut WorkMeter,
    ) -> Result<(), FixedStorageError> {
        let start = self.prepare_start(cell, at, work)?;
        self.apply_start(start);
        Ok(())
    }

    pub(crate) fn prepare_start<W: WorkRecorder + ?Sized>(
        &self,
        cell: usize,
        at: MonotonicTime,
        work: &mut W,
    ) -> Result<FixedWindowStart, FixedStorageError> {
        work.record(WorkDimension::InvariantChecks, 1)?;
        let (bounds, history) = self
            .bounds
            .get(cell)
            .zip(self.history.get(cell))
            .ok_or(FixedStorageError::Capacity)?;
        work.record(WorkDimension::InvariantChecks, 1)?;
        if history.back().is_some_and(|prior| at < *prior) {
            return Err(FixedStorageError::InvalidTime);
        }
        for bound in bounds {
            work.record(WorkDimension::InvariantChecks, 1)?;
            if history.len() >= bound.1 as usize {
                work.record(WorkDimension::VisitedEntities, 1)?;
                let prior = history[history.len() - bound.1 as usize];
                let elapsed = at
                    .checked_duration_since(prior)
                    .map_err(|_| FixedStorageError::InvalidTime)?;
                if elapsed < bound.0 {
                    return Err(FixedStorageError::WindowExceeded);
                }
            }
        }
        work.record(
            WorkDimension::CopiedBytes,
            size_of::<MonotonicTime>() as u64,
        )?;
        Ok(FixedWindowStart(cell, at))
    }

    pub(crate) fn prepare_start_precharged(
        &self,
        cell: usize,
        at: MonotonicTime,
    ) -> Result<FixedWindowStart, FixedStorageError> {
        let (bounds, history) = self
            .bounds
            .get(cell)
            .zip(self.history.get(cell))
            .ok_or(FixedStorageError::Capacity)?;
        if history.back().is_some_and(|prior| at < *prior) {
            return Err(FixedStorageError::InvalidTime);
        }
        for bound in bounds {
            if history.len() >= bound.1 as usize {
                let prior = history[history.len() - bound.1 as usize];
                let elapsed = at
                    .checked_duration_since(prior)
                    .map_err(|_| FixedStorageError::InvalidTime)?;
                if elapsed < bound.0 {
                    return Err(FixedStorageError::WindowExceeded);
                }
            }
        }
        Ok(FixedWindowStart(cell, at))
    }

    pub(crate) fn apply_start(&mut self, start: FixedWindowStart) {
        let FixedWindowStart(cell, at) = start;
        let history = &mut self.history[cell];
        if history.len() == self.bounds[cell][H - 1].1 as usize {
            history.pop_front();
        }
        history.push_back(at);
    }

    pub fn len(&self, cell: usize) -> Option<usize> {
        self.history.get(cell).map(VecDeque::len)
    }
}

#[cfg(test)]
mod fixed_index_tests {
    use super::*;
    use crate::HotPathWorkBudget;

    #[test]
    fn fixed_index_is_bounded_incremental_and_atomic() {
        let mut index = FixedIdentityIndex::try_new(1027).unwrap();
        let mut entries = Vec::new();
        for value in 0..1026u16 {
            let mut key = [0; 33];
            key[29..].copy_from_slice(&u32::from(value).to_be_bytes());
            entries.push((key, value));
        }
        entries.sort_unstable();
        let mut work = WorkMeter::new(HotPathWorkBudget::binary_maximum());
        index.try_insert_sorted(&entries[513..], &mut work).unwrap();
        index.try_insert_sorted(&entries[..513], &mut work).unwrap();
        assert_eq!(index.len(), 1026);
        assert_eq!(work.witness().value(WorkDimension::VisitedEntities), 51_249);
        assert_eq!(work.witness().value(WorkDimension::InvariantChecks), 1030);
        assert!(work.witness().value(WorkDimension::CopiedBytes) > 0);
        assert_eq!(work.witness().value(WorkDimension::Allocations), 0);
        let insertion_visits = |key| {
            let mut index = FixedIdentityIndex::try_new(2).unwrap();
            let mut work = WorkMeter::new(HotPathWorkBudget::binary_maximum());
            index.try_insert_sorted(&[([0; 33], 0)], &mut work).unwrap();
            index.try_insert_sorted(&[(key, 1)], &mut work).unwrap();
            work.witness().value(WorkDimension::VisitedEntities)
        };
        let mut late = [0; 33];
        late[32] = 1;
        assert_eq!((insertion_visits([1; 33]), insertion_visits(late)), (3, 35));
        for (key, value) in entries {
            assert_eq!(index.find(key, &mut work).unwrap(), Some(value));
        }
        let before = index.len();
        assert_eq!(
            index.try_insert_sorted(&[([0; 33], 0)], &mut work),
            Err(FixedIndexError::Duplicate)
        );
        assert_eq!(index.len(), before);

        let mut empty = FixedIdentityIndex::try_new(2).unwrap();
        let mut rejected = WorkMeter::new(HotPathWorkBudget::binary_maximum());
        let noncanonical = [([2; 33], 2), ([1; 33], 1)];
        assert_eq!(
            empty.try_insert_sorted(&noncanonical, &mut rejected),
            Err(FixedIndexError::NonCanonical)
        );
        assert_eq!(empty.len(), 0);
        assert_eq!(rejected.witness().value(WorkDimension::InvariantChecks), 4);

        let mut duplicate_index = FixedIdentityIndex::try_new(2).unwrap();
        let mut rejected = WorkMeter::new(HotPathWorkBudget::binary_maximum());
        let duplicate = [([0; 33], 0), ([0; 33], 1)];
        assert_eq!(
            duplicate_index.try_insert_sorted(&duplicate, &mut rejected),
            Err(FixedIndexError::Duplicate)
        );
        assert_eq!(duplicate_index.len(), 0);
        assert_eq!(rejected.witness().value(WorkDimension::InvariantChecks), 4);

        let mut constrained = FixedIdentityIndex::try_new(1).unwrap();
        let copied_zero =
            HotPathWorkBudget::try_new(crate::HotPathWorkWitness::new([1_000_000, 0, 0, 2, 2_100]))
                .unwrap();
        assert!(matches!(
            constrained.try_insert_sorted(&[([1; 33], 1)], &mut WorkMeter::new(copied_zero)),
            Err(FixedIndexError::Work(WorkBudgetError::BudgetExceeded(
                WorkDimension::CopiedBytes,
                0,
                _
            )))
        ));
        assert_eq!(constrained.len(), 0);

        let invariant_one = HotPathWorkBudget::try_new(crate::HotPathWorkWitness::new([
            1_000_000, 2_097_152, 0, 2, 1,
        ]))
        .unwrap();
        let mut invariant_meter = WorkMeter::new(invariant_one);
        assert!(matches!(
            constrained.try_insert_sorted(&[([1; 33], 1)], &mut invariant_meter),
            Err(FixedIndexError::Work(WorkBudgetError::BudgetExceeded(
                WorkDimension::InvariantChecks,
                1,
                2
            )))
        ));
        assert_eq!(
            invariant_meter
                .witness()
                .value(WorkDimension::InvariantChecks),
            1
        );
        assert_eq!(constrained.len(), 0);

        let mut visited_meter = WorkMeter::new(HotPathWorkBudget::binary_maximum());
        visited_meter
            .record(WorkDimension::VisitedEntities, 1_704_075)
            .unwrap();
        assert!(matches!(
            constrained.try_insert_sorted(&[([1; 33], 1)], &mut visited_meter),
            Err(FixedIndexError::Work(WorkBudgetError::BudgetExceeded(
                WorkDimension::VisitedEntities,
                1_704_575,
                _
            )))
        ));
        assert_eq!(constrained.len(), 0);

        let mut full_meter = WorkMeter::new(HotPathWorkBudget::binary_maximum());
        assert_eq!(
            constrained.try_insert_sorted(&[([1; 33], 1), ([2; 33], 2)], &mut full_meter),
            Err(FixedIndexError::Capacity)
        );
        assert_eq!(constrained.len(), 0);
        assert_eq!(
            full_meter.witness().value(WorkDimension::InvariantChecks),
            1
        );

        let mut zero = FixedIdentityIndex::try_new(0).unwrap();
        let mut zero_meter = WorkMeter::new(HotPathWorkBudget::binary_maximum());
        assert_eq!(
            zero.try_insert_sorted(&[([1; 33], 1)], &mut zero_meter),
            Err(FixedIndexError::Capacity)
        );
        assert_eq!(zero.len(), 0);
        assert_eq!(
            zero_meter.witness().value(WorkDimension::InvariantChecks),
            1
        );
    }

    #[test]
    fn avl_layout_height_recurrence_and_structure_are_exact() {
        use std::mem::{align_of, offset_of, size_of};

        assert_eq!((size_of::<AvlNode>(), align_of::<AvlNode>()), (56, 4));
        assert_eq!(
            (
                offset_of!(AvlNode, key),
                offset_of!(AvlNode, record),
                offset_of!(AvlNode, left),
                offset_of!(AvlNode, right),
                offset_of!(AvlNode, parent),
                offset_of!(AvlNode, height),
            ),
            (0, 36, 40, 44, 48, 52)
        );
        assert_eq!(AvlIndex::height_bound(2_050), Ok(15));
        assert_eq!(AvlIndex::height_bound(16_130), Ok(19));

        fn key(value: u32) -> [u8; 33] {
            let mut key = [0; 33];
            key[29..].copy_from_slice(&value.to_be_bytes());
            key
        }
        fn oracle(index: &AvlIndex, node: u32, parent: u32, seen: &mut usize) -> u8 {
            if node == NO_NODE {
                return 0;
            }
            let value = &index.nodes[node as usize];
            assert_eq!(value.parent, parent);
            if value.left != NO_NODE {
                assert!(index.nodes[value.left as usize].key < value.key);
            }
            if value.right != NO_NODE {
                assert!(index.nodes[value.right as usize].key > value.key);
            }
            let left = oracle(index, value.left, node, seen);
            let right = oracle(index, value.right, node, seen);
            assert!((i16::from(left) - i16::from(right)).abs() <= 1);
            assert_eq!(value.height, 1 + left.max(right));
            *seen += 1;
            value.height
        }
        let rotations = [
            vec![3, 2, 1],
            vec![1, 2, 3],
            vec![3, 1, 2],
            vec![1, 3, 2],
            (0..128).collect(),
            (0..128).rev().collect(),
        ];
        for values in rotations {
            let mut index = AvlIndex::try_new(values.len()).unwrap();
            let pointer = index.nodes.as_ptr();
            let capacity = index.nodes.capacity();
            for value in values {
                index.insert_prevalidated(key(value), value);
            }
            assert_eq!(
                (index.nodes.as_ptr(), index.nodes.capacity()),
                (pointer, capacity)
            );
            let mut seen = 0;
            let height = oracle(&index, index.root, NO_NODE, &mut seen);
            assert_eq!(seen, index.nodes.len());
            assert!(height <= AvlIndex::height_bound(index.capacity).unwrap());
            for node in &index.nodes {
                assert_eq!(
                    index.find(
                        node.key,
                        &mut WorkMeter::new(HotPathWorkBudget::binary_maximum())
                    ),
                    Ok(Some(node.record))
                );
            }
        }
    }

    #[test]
    fn fixed_record_arena_owns_claims_and_rejects_atomically() {
        let mut arena = FixedRecordArena::<u8, u8, 2>::try_new(2, 3).unwrap();
        assert!(arena.is_empty());
        let mut work = WorkMeter::new(HotPathWorkBudget::binary_maximum());
        let first = [[0; 33], [1; 33]];
        assert_eq!(arena.try_push(first, 7, &[2, 3], &mut work), Ok(0));
        assert_eq!(arena.find([0; 33], &mut work), Ok(Some(0)));
        assert_eq!(
            (arena.get(0), arena.claims(0)),
            (Some(&7), Some([2, 3].as_slice()))
        );
        assert_eq!(
            arena.try_push(first, 8, &[4], &mut work),
            Err(FixedStorageError::Duplicate)
        );
        assert_eq!((arena.len(), arena.claims(0)), (1, Some([2, 3].as_slice())));
        assert_eq!(
            arena.try_push([[2; 33], [3; 33]], 8, &[4, 5], &mut work),
            Err(FixedStorageError::Capacity)
        );
        assert_eq!(arena.len(), 1);
    }

    #[test]
    fn fixed_window_counter_enforces_half_open_windows_and_atomic_work() {
        let bounds = [[
            FixedStartCountBound(Duration::from_micros(10), 1),
            FixedStartCountBound(Duration::from_micros(20), 2),
        ]];
        let mut counter = FixedWindowCounter::try_new(bounds, [2]).unwrap();
        let mut work = WorkMeter::new(HotPathWorkBudget::binary_maximum());
        assert_eq!(
            counter.try_start(1, MonotonicTime::from_micros(5), &mut work),
            Err(FixedStorageError::Capacity)
        );
        let invariant_zero = HotPathWorkBudget::try_new(crate::HotPathWorkWitness::new([
            1_000_000, 2_097_152, 0, 2, 0,
        ]))
        .unwrap();
        assert!(matches!(
            counter.try_start(
                1,
                MonotonicTime::from_micros(5),
                &mut WorkMeter::new(invariant_zero)
            ),
            Err(FixedStorageError::Work(WorkBudgetError::BudgetExceeded(
                WorkDimension::InvariantChecks,
                0,
                1
            )))
        ));
        assert_eq!(counter.len(0), Some(0));
        counter
            .try_start(0, MonotonicTime::from_micros(5), &mut work)
            .unwrap();
        assert_eq!(
            counter.try_start(0, MonotonicTime::from_micros(14), &mut work),
            Err(FixedStorageError::WindowExceeded)
        );
        counter
            .try_start(0, MonotonicTime::from_micros(15), &mut work)
            .unwrap();
        counter
            .try_start(0, MonotonicTime::from_micros(25), &mut work)
            .unwrap();
        assert_eq!(counter.len(0), Some(2));
        let copied_zero =
            HotPathWorkBudget::try_new(crate::HotPathWorkWitness::new([1_000_000, 0, 0, 2, 2_100]))
                .unwrap();
        let before = counter.len(0);
        assert!(matches!(
            counter.try_start(
                0,
                MonotonicTime::from_micros(35),
                &mut WorkMeter::new(copied_zero)
            ),
            Err(FixedStorageError::Work(WorkBudgetError::BudgetExceeded(
                WorkDimension::CopiedBytes,
                0,
                _
            )))
        ));
        assert_eq!(counter.len(0), before);
    }
}

#[cfg(test)]
mod avl_removal_probe {
    use super::*;

    fn key(n: u8) -> [u8; 33] {
        let mut value = [0; 33];
        value[0] = n;
        value
    }

    /// Every live key remains findable, every removed key is gone, and the
    /// tree stays height-balanced with intact parent links after each removal.
    fn audit(index: &AvlIndex, live: &[u8]) {
        for n in 0..=32u8 {
            let found = index.find_precharged(key(n)).unwrap();
            assert_eq!(
                found.is_some(),
                live.contains(&n),
                "key {n} presence after removals"
            );
        }
        fn check(index: &AvlIndex, node: u32, parent: u32) -> u8 {
            if node == NO_NODE {
                return 0;
            }
            let value = index.nodes[node as usize];
            assert_eq!(value.parent, parent, "parent link");
            let left = check(index, value.left, node);
            let right = check(index, value.right, node);
            assert_eq!(value.height, 1 + left.max(right), "stored height");
            assert!(
                (i16::from(left) - i16::from(right)).abs() <= 1,
                "AVL balance"
            );
            value.height
        }
        check(index, index.root, NO_NODE);
        assert_eq!(index.live(), live.len(), "live count");
    }

    /// Claim-span reuse must be constant time: a released span returns to its
    /// exact-length bucket and is popped from the head, never searched for.
    /// A linear search here would be unbounded unmetered hot-path work.
    #[test]
    fn claim_span_reuse_is_exact_fit_and_scan_free() {
        let mut arena: FixedRecordArena<u8, u16, 1> = FixedRecordArena::try_new(4, 8).unwrap();
        let key = |n: u8| {
            let mut value = [0; 33];
            value[0] = n;
            value
        };
        // Two records with different claim lengths, then a third that must
        // start fresh because no bucket holds its length.
        let one = arena.push_prevalidated([key(1)], 1, &[10, 11]);
        let two = arena.push_prevalidated([key(2)], 2, &[20]);
        assert_eq!(arena.claim_high_water, 3);

        arena.remove(one, [key(1)]);
        assert_eq!(arena.free_spans, 1, "the two-slot span is banked");
        assert_eq!(arena.free_span_head[2], 0, "banked under its exact length");
        assert_eq!(arena.free_span_head[1], NO_NODE, "not under another length");

        // A same-length request reuses the banked span exactly and does not
        // advance the bump pointer.
        let three = arena.push_prevalidated([key(3)], 3, &[30, 31]);
        assert_eq!(arena.claims(three), Some(&[30, 31][..]));
        assert_eq!(arena.claim_high_water, 3, "reused, not appended");
        assert_eq!(arena.free_spans, 0, "the bucket is drained");

        // A different-length request cannot use a banked span of another
        // length and takes fresh storage instead of splitting.
        arena.remove(three, [key(3)]);
        let four = arena.push_prevalidated([key(4)], 4, &[40]);
        assert_eq!(arena.claims(four), Some(&[40][..]));
        assert_eq!(arena.claim_high_water, 4, "no split, fresh storage");
        assert_eq!(arena.free_spans, 1, "the two-slot span stays banked");
        assert_eq!(
            arena.claims(two),
            Some(&[20][..]),
            "untouched record intact"
        );
    }

    #[test]
    fn removal_preserves_search_order_balance_and_parent_links() {
        let mut index = AvlIndex::try_new(64).unwrap();
        let mut live: Vec<u8> = Vec::new();
        for n in 1..=24u8 {
            index.insert_prevalidated(key(n), u32::from(n));
            live.push(n);
        }
        audit(&index, &live);
        // Remove in an order that exercises leaf, one-child and two-child
        // deletions and forces rebalancing above a rotation.
        for n in [12u8, 1, 24, 13, 2, 23, 7, 18, 3] {
            assert_eq!(index.remove(key(n)), Some(u32::from(n)), "removing {n}");
            live.retain(|value| *value != n);
            audit(&index, &live);
        }
        assert_eq!(index.remove(key(12)), None, "second removal is absent");
        // Vacated slots are reused rather than growing the arena.
        let physical = index.nodes.len();
        for n in [12u8, 1, 24] {
            index.insert_prevalidated(key(n), u32::from(n));
            live.push(n);
        }
        assert_eq!(index.nodes.len(), physical, "vacated slots reused");
        audit(&index, &live);
    }
}

#[cfg(test)]
mod reclamation_boundary_probe {
    use super::*;
    use crate::HotPathWorkBudget;

    /// Start history is sized by the catalog-wide retained capacity, not by the
    /// active Budget's admission bound. Sizing it by the active bound would
    /// discard the predecessor starts a shorter Budget must still hand to a
    /// longer successor.
    #[test]
    fn start_history_is_sized_by_catalog_capacity_not_the_active_bound() {
        let bounds = [[
            FixedStartCountBound(Duration::from_micros(10), 1),
            FixedStartCountBound(Duration::from_micros(20), 2),
        ]];
        let counter = FixedWindowCounter::<1, 2>::try_new(bounds, [9]).unwrap();
        assert_eq!(
            counter.backing_capacities(),
            [9],
            "physical history follows the catalog capacity"
        );
        // The active bound still gates admission at 2, independently of the 9
        // slots the catalog retains.
        let mut counter = counter;
        let mut work = WorkMeter::new(HotPathWorkBudget::binary_maximum());
        // One start per 10us window, two per 20us window.
        for micros in [1, 12] {
            counter
                .try_start(0, MonotonicTime::from_micros(micros), &mut work)
                .unwrap();
        }
        assert_eq!(
            counter.try_start(0, MonotonicTime::from_micros(13), &mut work),
            Err(FixedStorageError::WindowExceeded),
            "the active bound still gates at 1 per 10us"
        );
        // A capacity below the active bound is not constructible.
        assert_eq!(
            FixedWindowCounter::<1, 2>::try_new(bounds, [1]).unwrap_err(),
            FixedStorageError::Capacity
        );
    }

    /// A claim run longer than the entire claim arena must fail closed, not
    /// index past the bucket table.
    #[test]
    fn an_oversized_claim_run_fails_closed_instead_of_panicking() {
        let arena: FixedRecordArena<u8, u16, 1> = FixedRecordArena::try_new(4, 8).unwrap();
        assert_eq!(arena.validate_capacity(8), Ok(()));
        assert_eq!(
            arena.validate_capacity(9),
            Err(FixedStorageError::Capacity),
            "one past the claim arena"
        );
        assert_eq!(
            arena.validate_capacity(1_024),
            Err(FixedStorageError::Capacity),
            "far past the claim arena"
        );
    }
}
