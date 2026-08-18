use crate::work::WorkMeter;
use crate::{WorkBudgetError, WorkDimension};

/// A checked fixed-capacity collection insertion failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BoundedCollectionError {
    Full,
    Duplicate,
}

/// An insertion-ordered vector whose storage and maximum length are fixed.
#[derive(Clone, Debug, Eq, PartialEq)]
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

struct IndexLeaf<V> {
    key: [u8; 33],
    value: V,
}

struct IndexBranch {
    bit: u16,
    zero: u32,
    one: u32,
}

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
            if self.find(key, work)?.is_some() {
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
            .record(WorkDimension::VisitedEntities, 999_500)
            .unwrap();
        assert!(matches!(
            constrained.try_insert_sorted(&[([1; 33], 1)], &mut visited_meter),
            Err(FixedIndexError::Work(WorkBudgetError::BudgetExceeded(
                WorkDimension::VisitedEntities,
                1_000_000,
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
}
