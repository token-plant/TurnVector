//! C18 — Catalog-retained support history, bounded deferred expiry, the sole
//! dedicated Prepared Carry slot, and the incremental conservation accumulator.
//!
//! Everything here is a private field of the sole `support_ledger` owner. C18
//! adds no second ledger, selects no lifecycle witness, owns no Control
//! outcome, and emits no Effect: it retains terminal state until its Catalog
//! Retention Horizon release condition is met, reclaims whole release groups
//! through one bounded generation-checked transition, and exposes complete
//! immutable facts to Admission and to the later C26/C27 carry work.

use crate::{
    Duration, FixedStartCountBound, FixedStorageError, MonotonicTime, SupportLedgerGeneration,
};
use std::num::NonZeroU32;

use super::{CLAIMS, POOLS, SupportLedgerError};

/// The seven `SupportOperation` variants times the three `SupportPool`
/// variants: the catalog-wide start-history and vector axis shared with
/// `FixedWindowCounter<21, H>`.
pub(crate) const CELLS: usize = 21;
/// The six `SupportObligationState` variants.
pub(crate) const STATES: usize = 6;
/// The four `SupportFundingClaim` variants.
pub(crate) const CLAIM_KINDS: usize = 4;
/// The five `LifecycleReserveKind` variants.
pub(crate) const LIFECYCLE_KINDS: usize = 5;
/// B04 proves exactly one nonborrowable Prepared Carry slot outside every
/// support pool, so the cardinality is a constant rather than an input.
pub(crate) const CARRY_SLOTS: u32 = 1;
/// How many due groups an observation counts before saturating. An observation
/// must not scan the owner set, so the count is deliberately bounded.
pub(crate) const EXPIRY_OBSERVATION_GROUPS: usize = 8;

fn invalid() -> SupportLedgerError {
    SupportLedgerError::InvalidInput
}

fn capacity() -> SupportLedgerError {
    SupportLedgerError::Storage(FixedStorageError::Capacity)
}

/// A positive elapsed interval. A zero horizon would make every started record
/// eligible at its own start and is rejected at construction.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct NonZeroDuration(Duration);

impl NonZeroDuration {
    pub(crate) fn new(value: Duration) -> Result<Self, SupportLedgerError> {
        (value.as_micros() > 0)
            .then_some(Self(value))
            .ok_or(invalid())
    }

    pub(crate) const fn get(self) -> Duration {
        self.0
    }
}

/// Content-addressed B04 Runtime Overhead Catalog identity.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct CatalogIdentity(pub(crate) [u8; 32]);
/// Content-addressed active Configuration Snapshot identity.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct ConfigurationIdentity(pub(crate) [u8; 32]);
/// Content-addressed active Owner-Thread Support Budget identity.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct BudgetIdentity(pub(crate) [u8; 32]);

impl CatalogIdentity {
    fn nonzero(self) -> bool {
        self.0 != [0; 32]
    }
}

impl ConfigurationIdentity {
    fn nonzero(self) -> bool {
        self.0 != [0; 32]
    }
}

impl BudgetIdentity {
    fn nonzero(self) -> bool {
        self.0 != [0; 32]
    }
}

/// One finite B04 `(predecessor, successor, activation sequence, support
/// class)` cell. An absent tuple is not activatable: C26 may only evaluate a
/// pair that the Catalog sealed here.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PairCell {
    pub(crate) predecessor: BudgetIdentity,
    pub(crate) successor: BudgetIdentity,
    pub(crate) sequence: u32,
    pub(crate) mandatory: u32,
    pub(crate) safety: u32,
    pub(crate) history_reset: bool,
}

/// The exact finite number of B04-generated activation sequences. A Catalog
/// that proves a different number is capacity drift: the constant is
/// regenerated and reviewed rather than inferred at run time.
pub(crate) const PAIRS: usize = 4;

/// The complete sealed B04 pair proof: one fixed cell for every generated
/// `(predecessor, successor, activation sequence, support class)` tuple. It is
/// a Catalog-sealed array, never a runtime map, default, or caller estimate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PairCapacity(pub(crate) [PairCell; PAIRS]);

impl PairCapacity {
    /// Canonical iff every cell is identity-nonzero, the whole table is
    /// strictly ordered by `(predecessor, successor, sequence)` so no tuple can
    /// appear twice, and no activation resets the catalog-wide start history.
    fn valid(&self) -> bool {
        self.0.iter().all(|cell| {
            cell.predecessor.nonzero() && cell.successor.nonzero() && !cell.history_reset
        }) && self
            .0
            .windows(2)
            .all(|pair| Self::key(&pair[0]) < Self::key(&pair[1]))
    }

    fn key(cell: &PairCell) -> ([u8; 32], [u8; 32], u32) {
        (cell.predecessor.0, cell.successor.0, cell.sequence)
    }

    /// The exact sealed suballocations for one activation, or `None` when the
    /// Catalog never proved that tuple.
    pub(crate) fn lookup(
        &self,
        predecessor: BudgetIdentity,
        successor: BudgetIdentity,
        sequence: u32,
    ) -> Option<(u32, u32)> {
        self.0
            .iter()
            .find(|cell| {
                cell.predecessor == predecessor
                    && cell.successor == successor
                    && cell.sequence == sequence
            })
            .map(|cell| (cell.mandatory, cell.safety))
    }
}

/// The sealed Catalog-derived retention facts bound once at construction. Every
/// field is an exact B04 output: none is optional, defaulted, or inferred, and
/// the ledger never widens one at run time.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SupportHistoryLimits<const H: usize> {
    pub(crate) catalog: CatalogIdentity,
    pub(crate) configuration: ConfigurationIdentity,
    pub(crate) budget: BudgetIdentity,
    pub(crate) retention_horizon: NonZeroDuration,
    pub(crate) horizons: [NonZeroDuration; H],
    pub(crate) start_history_capacity: [u32; CELLS],
    pub(crate) active_start_bound: [[FixedStartCountBound; H]; CELLS],
    pub(crate) interference_limit_us: [u64; H],
    pub(crate) operation_capacity: [[u32; POOLS]; STATES],
    pub(crate) physical_credit_capacity: [u32; CELLS],
    pub(crate) funding_claim_capacity: [[u32; POOLS]; CLAIM_KINDS],
    pub(crate) ordinary_claim_capacity: [u32; POOLS],
    pub(crate) owner_capacity: u32,
    pub(crate) link_capacity: u32,
    pub(crate) entitlement_capacity: u32,
    pub(crate) vector_capacity: [[u64; H]; CELLS],
    pub(crate) lifecycle_capacity: [[[u32; POOLS]; STATES]; LIFECYCLE_KINDS],
    pub(crate) mandatory_pair_capacity: PairCapacity,
    pub(crate) safety_pair_capacity: PairCapacity,
    pub(crate) expiry_ticket_capacity: u32,
    pub(crate) expiry_groups_per_transition: NonZeroU32,
    pub(crate) expiry_units_per_transition: NonZeroU32,
    pub(crate) largest_atomic_release_group_units: NonZeroU32,
}

impl<const H: usize> SupportHistoryLimits<H> {
    /// Complete construction-time validation. Every rejection leaves no usable
    /// ledger, emits no Effect, and never falls back to a smaller bound.
    pub(crate) fn validate(&self) -> Result<(), SupportLedgerError> {
        let identities =
            self.catalog.nonzero() && self.configuration.nonzero() && self.budget.nonzero();
        // The horizon vector is positive, strictly increasing, duplicate-free,
        // and ends exactly at the Catalog Retention Horizon.
        let horizons = (1..=8).contains(&H)
            && self.horizons.windows(2).all(|pair| pair[0] < pair[1])
            && self.horizons[H - 1] == self.retention_horizon;
        // Each cell's active bounds share the horizon vector, are positive and
        // nondecreasing, and never exceed the physical history capacity.
        let bounds = (0..CELLS).all(|cell| {
            let row = &self.active_start_bound[cell];
            row.iter()
                .zip(&self.horizons)
                .all(|(bound, horizon)| bound.0 == horizon.get())
                && row[0].1 > 0
                && row.windows(2).all(|pair| pair[0].1 <= pair[1].1)
                && row[H - 1].1 <= self.start_history_capacity[cell]
        });
        let limits = self.interference_limit_us.iter().all(|limit| *limit > 0);
        let pairs = self.mandatory_pair_capacity.valid() && self.safety_pair_capacity.valid();
        // A dormant expiry ticket is constructible for every releasable root,
        // so terminalizing a record never needs new capacity.
        let roots = self.releasable_roots().ok_or_else(invalid)?;
        let tickets = self.expiry_ticket_capacity == roots;
        let quotas = self.expiry_units_per_transition >= self.largest_atomic_release_group_units;
        (identities && horizons && bounds && limits && pairs && tickets && quotas)
            .then_some(())
            .ok_or_else(invalid)
    }

    /// The checked number of releasable operation plus entitlement roots.
    fn releasable_roots(&self) -> Option<u32> {
        self.operation_capacity
            .iter()
            .flatten()
            .try_fold(0u32, |total, capacity| total.checked_add(*capacity))?
            .checked_add(self.entitlement_capacity)
    }

    /// The exact sealed quota pair every `prepare_expiry` invocation must use.
    pub(crate) fn quotas(&self) -> (u32, u32) {
        (
            self.expiry_groups_per_transition.get(),
            self.expiry_units_per_transition.get(),
        )
    }

    pub(crate) fn retention(&self) -> Duration {
        self.retention_horizon.get()
    }
}

/// Which owner store holds the group a ticket releases. This is cleanup
/// ownership, deliberately distinct from the record's terminal state: records
/// in different stores can share a terminal state, and one store's slot index
/// means nothing to another.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(u8)]
pub(crate) enum OwnerFamily {
    /// A legacy ordinary or lifecycle record in the sole record arena.
    LegacyRecord = 0,
    /// A C16 request-bundle initial obligation.
    InitialBundle = 1,
    /// A retained terminal entitlement tombstone.
    Tombstone = 2,
}

impl OwnerFamily {
    const fn tag(self) -> u8 {
        self as u8
    }
}

/// One dormant or scheduled whole-group release ticket. Ordering is exactly
/// `(release_at, owner family, slot_index)`, which is total because a slot
/// index is unique within its family.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ExpiryTicket {
    pub(crate) release_at: MonotonicTime,
    pub(crate) family: OwnerFamily,
    pub(crate) slot_index: u32,
    /// The whole-group unit count charged when this ticket is released. A
    /// group is released in full or not at all.
    pub(crate) units: u32,
    /// The group's owning identity. The record does not carry it and releasing
    /// the group must delete its identity keys, so the ticket binds it at
    /// terminalization.
    pub(crate) identity: [u8; 32],
}

/// The vacant entry of a fixed selection array. It is never due, never
/// selected, and never released: `count` bounds every read.
pub(crate) const DORMANT_TICKET: ExpiryTicket = ExpiryTicket {
    release_at: MonotonicTime::from_micros(u64::MAX),
    family: OwnerFamily::LegacyRecord,
    slot_index: u32::MAX,
    units: 0,
    identity: [0; 32],
};

impl ExpiryTicket {
    fn key(&self) -> (u64, u8, u32) {
        (
            self.release_at.as_micros(),
            self.family.tag(),
            self.slot_index,
        )
    }
}

impl Ord for ExpiryTicket {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.key().cmp(&other.key())
    }
}

impl PartialOrd for ExpiryTicket {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// The fixed preallocated min-heap that orders due release groups. Capacity is
/// reserved once at construction; no push after construction can allocate,
/// because one dormant ticket exists for every releasable root.
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct ExpiryHeap {
    tickets: Vec<ExpiryTicket>,
    capacity: u32,
}

impl ExpiryHeap {
    pub(crate) fn try_new(tickets: u32) -> Result<Self, SupportLedgerError> {
        let slots = usize::try_from(tickets).map_err(|_| capacity())?;
        let mut heap = Vec::new();
        heap.try_reserve_exact(slots)
            .map_err(|_| SupportLedgerError::Storage(FixedStorageError::Allocation))?;
        (heap.capacity() >= slots)
            .then_some(())
            .ok_or_else(capacity)?;
        Ok(Self {
            tickets: heap,
            capacity: tickets,
        })
    }

    pub(crate) fn len(&self) -> u32 {
        u32::try_from(self.tickets.len()).expect("constructor-bounded ticket count")
    }

    /// Schedules one whole-group ticket. Rejects rather than allocates when the
    /// sealed ticket capacity is already exhausted.
    pub(crate) fn schedule(&mut self, ticket: ExpiryTicket) -> Result<(), SupportLedgerError> {
        (self.len() < self.capacity)
            .then_some(())
            .ok_or_else(capacity)?;
        let mut child = self.tickets.len();
        self.tickets.push(ticket);
        while child > 0 {
            let parent = (child - 1) / 2;
            if self.tickets[parent] <= self.tickets[child] {
                break;
            }
            self.tickets.swap(parent, child);
            child = parent;
        }
        Ok(())
    }

    /// The earliest scheduled release time, or `None` when no ticket is active.
    pub(crate) fn next_release(&self) -> Option<MonotonicTime> {
        self.tickets.first().map(|ticket| ticket.release_at)
    }

    /// The due prefix in exact key order, bounded componentwise by the sealed
    /// group and unit quotas. A group whose units do not fit is not split: the
    /// selection stops and the remaining groups stay fully charged.
    ///
    /// The walk is a bounded best-first descent over the implicit heap, so it
    /// visits only the due prefix and a frontier of at most `G + 1` nodes. It
    /// allocates nothing: the selection lands in a caller-owned fixed array and
    /// the frontier is a fixed array too.
    pub(crate) fn due_prefix<const G: usize>(
        &self,
        at: MonotonicTime,
        max_units: u32,
    ) -> ([ExpiryTicket; G], usize, bool, u64)
    where
        ExpiryTicket: Copy,
    {
        let mut selected = [DORMANT_TICKET; G];
        let mut frontier = [u32::MAX; G];
        let mut frontier_len = 0usize;
        let mut visited = 0u64;
        if !self.tickets.is_empty() && G > 0 {
            frontier[0] = 0;
            frontier_len = 1;
        }
        let (mut count, mut units) = (0usize, 0u32);
        let mut more_due = false;
        while count < G && frontier_len > 0 {
            // The smallest frontier node is the next candidate in exact
            // (release_at, family, slot) order.
            let mut best = 0usize;
            for position in 1..frontier_len {
                visited += 1;
                if self.tickets[frontier[position] as usize] < self.tickets[frontier[best] as usize]
                {
                    best = position;
                }
            }
            let node = frontier[best] as usize;
            let ticket = self.tickets[node];
            visited += 1;
            if ticket.release_at > at {
                break;
            }
            if units
                .checked_add(ticket.units)
                .is_none_or(|total| total > max_units)
            {
                more_due = true;
                break;
            }
            selected[count] = ticket;
            units += ticket.units;
            count += 1;
            // Replace the consumed node with its children; the frontier can
            // hold them because each step removes one and adds at most two.
            frontier[best] = frontier[frontier_len - 1];
            frontier_len -= 1;
            for child in [2 * node + 1, 2 * node + 2] {
                if child < self.tickets.len() && frontier_len < G {
                    frontier[frontier_len] = child as u32;
                    frontier_len += 1;
                }
            }
        }
        if !more_due {
            more_due = (0..frontier_len)
                .any(|position| self.tickets[frontier[position] as usize].release_at <= at);
        }
        (selected, count, more_due, visited)
    }

    /// Removes the exact due prefix the selection named. Because the selection
    /// is the heap's smallest `count` entries in key order, removing it is
    /// `count` ordinary minimum extractions, each `O(log n)` and allocating
    /// nothing.
    pub(crate) fn release_prefix(&mut self, count: usize) {
        for _ in 0..count {
            self.pop_min();
        }
    }

    fn pop_min(&mut self) -> Option<ExpiryTicket> {
        if self.tickets.is_empty() {
            return None;
        }
        let smallest = self.tickets.swap_remove(0);
        let len = self.tickets.len();
        let mut parent = 0usize;
        loop {
            let (left, right) = (2 * parent + 1, 2 * parent + 2);
            let mut next = parent;
            if left < len && self.tickets[left] < self.tickets[next] {
                next = left;
            }
            if right < len && self.tickets[right] < self.tickets[next] {
                next = right;
            }
            if next == parent {
                break;
            }
            self.tickets.swap(parent, next);
            parent = next;
        }
        Some(smallest)
    }

    /// The complete canonical scheduled view in heap storage order.
    pub(crate) fn scheduled(&self) -> &[ExpiryTicket] {
        &self.tickets
    }

    pub(crate) fn release(&mut self, selected: &[ExpiryTicket]) {
        self.tickets.retain(|ticket| !selected.contains(ticket));
        let len = self.tickets.len();
        for start in (0..len / 2).rev() {
            let mut parent = start;
            loop {
                let (left, right) = (2 * parent + 1, 2 * parent + 2);
                let mut smallest = parent;
                if left < len && self.tickets[left] < self.tickets[smallest] {
                    smallest = left;
                }
                if right < len && self.tickets[right] < self.tickets[smallest] {
                    smallest = right;
                }
                if smallest == parent {
                    break;
                }
                self.tickets.swap(parent, smallest);
                parent = smallest;
            }
        }
    }
}
