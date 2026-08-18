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
