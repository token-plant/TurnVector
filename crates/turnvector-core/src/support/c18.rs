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

/// The one dedicated Prepared Carry slot. C18 only ever observes `Vacant`; the
/// `Prepared` variant exists so C26 and C27 extend this owner instead of
/// adding a parallel one.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum CarrySlot {
    #[default]
    Vacant,
    Prepared(CarrySummary),
}

/// The exact old/new Budget pair and both nonfungible suballocations a later
/// Prepared carry accounts simultaneously.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CarrySummary {
    pub(crate) predecessor: BudgetIdentity,
    pub(crate) successor: BudgetIdentity,
    pub(crate) sequence: u32,
    pub(crate) mandatory: u32,
    pub(crate) safety: u32,
}

/// Ordinary reservations are paused by C26 around a carry, never by C18.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum OrdinaryReservations {
    #[default]
    Running,
    Paused,
}

/// The incremental conservation accumulator. Every typed support transition
/// updates it atomically with owner state; a full fold over occupied owners is
/// a test, bootstrap, and replay diagnostic only, never a hot-path operation.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct Accumulator {
    /// Occupied obligations by state and pool.
    pub(crate) obligations: [[u32; POOLS]; STATES],
    /// Consumed physical start credits, one per started call.
    pub(crate) physical_credits: u32,
    /// Funding claims by typed claim kind and pool.
    pub(crate) claims: [[u32; POOLS]; CLAIM_KINDS],
    /// Live entitlements and retained terminal tombstones.
    pub(crate) entitlements: u32,
    pub(crate) tombstones: u32,
    /// Unreleased links held by retained tombstones.
    pub(crate) links: u32,
    /// Accumulated support interference per horizon, in microseconds.
    pub(crate) interference_us: [u64; 8],
}

impl Accumulator {
    /// Applies one signed occupancy delta. Underflow is internal noncanonical
    /// state and fails closed rather than saturating.
    pub(crate) fn apply(counter: &mut u32, delta: i32) -> Result<(), SupportLedgerError> {
        let next = if delta >= 0 {
            counter.checked_add(delta.unsigned_abs())
        } else {
            counter.checked_sub(delta.unsigned_abs())
        };
        *counter = next.ok_or(SupportLedgerError::Storage(FixedStorageError::NonCanonical))?;
        Ok(())
    }
}

/// The complete immutable observation Admission consumes. It is an owned
/// fixed-size value: copying it is legal, but it is authority only while its
/// instance seal, generation, identities, and `at` are revalidated.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RetentionFacts<const H: usize> {
    pub(crate) nonce: u64,
    pub(crate) generation: SupportLedgerGeneration,
    pub(crate) at: MonotonicTime,
    pub(crate) catalog: CatalogIdentity,
    pub(crate) configuration: ConfigurationIdentity,
    pub(crate) budget: BudgetIdentity,
    pub(crate) retention_horizon: Duration,
    pub(crate) horizons: [Duration; H],
    pub(crate) accumulator: Accumulator,
    pub(crate) interference_limit_us: [u64; H],
    /// Groups already due at `at` but not yet reclaimed. They remain fully
    /// charged in every used count until a bounded expiry commit releases them.
    pub(crate) expiry_due: u32,
    pub(crate) expiry_scheduled: u32,
    pub(crate) next_expiry_at: Option<MonotonicTime>,
    pub(crate) carry_slot: CarrySlot,
    pub(crate) carry_capacity: u32,
    pub(crate) ordinary_reservations: OrdinaryReservations,
}

impl<const H: usize> RetentionFacts<H> {
    /// Checked interference headroom for one horizon.
    pub(crate) fn interference_headroom(&self, horizon: usize) -> Option<u64> {
        self.interference_limit_us
            .get(horizon)?
            .checked_sub(*self.accumulator.interference_us.get(horizon)?)
    }
}

/// The result of one bounded expiry transition.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ExpiryCommit {
    /// Unchanged exactly when `released_groups` is zero.
    pub(crate) generation: SupportLedgerGeneration,
    pub(crate) released_groups: u32,
    pub(crate) released_units: u32,
    /// More groups were due at `at` than one bounded batch could release.
    pub(crate) more_due: bool,
    pub(crate) next_expiry_at: Option<MonotonicTime>,
}

/// The retention boundary of a started record: its credit and every linked
/// claim release together at `max(terminal_at, start_at + R_cat)`.
pub(crate) fn started_release_at(
    start_at: MonotonicTime,
    terminal_at: MonotonicTime,
    retention: Duration,
) -> Result<MonotonicTime, SupportLedgerError> {
    let horizon = start_at.checked_add(retention).map_err(|_| invalid())?;
    Ok(terminal_at.max(horizon))
}

/// The retention boundary of a terminal entitlement tombstone: it is
/// unschedulable while any link remains, and otherwise releases at
/// `max(tombstone_at + R_cat, latest_link_release_at)`.
pub(crate) fn tombstone_release_at(
    tombstone_at: MonotonicTime,
    latest_link_release_at: Option<MonotonicTime>,
    retention: Duration,
) -> Result<MonotonicTime, SupportLedgerError> {
    let horizon = tombstone_at.checked_add(retention).map_err(|_| invalid())?;
    Ok(latest_link_release_at.map_or(horizon, |link| horizon.max(link)))
}

/// A typed-impossible close consumes no start, so the whole group becomes due
/// at its terminal instant rather than at a start-anchored horizon.
pub(crate) fn unstarted_release_at(terminal_at: MonotonicTime) -> MonotonicTime {
    terminal_at
}

#[allow(dead_code, reason = "C19 consumes the ordinary-claim axis")]
pub(crate) const ORDINARY_CLAIM_CLASS: usize = CLAIMS;

/// The complete C18 state owned by the sole support ledger: the sealed Catalog
/// limits, the preallocated expiry heap, the incremental accumulator, the one
/// dedicated carry slot, and the ledger time floor.
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct SupportC18<const H: usize> {
    /// Boxed once at construction: the sealed tensors are large, immutable,
    /// and read only through a reference, so keeping them out of the inline
    /// ledger keeps every prepare/commit frame small. This is the constructor
    /// reservation the error algebra already maps to `Storage(Allocation)`,
    /// not a hot-path allocation.
    limits: Box<[SupportHistoryLimits<H>]>,
    expiry: ExpiryHeap,
    accumulator: Accumulator,
    carry: CarrySlot,
    ordinary: OrdinaryReservations,
    time_floor: MonotonicTime,
}

impl<const H: usize> SupportC18<H> {
    /// Seeds empty histories, dormant tickets, and the sole `Vacant` carry
    /// slot after validating every sealed Catalog fact.
    pub(crate) fn try_new(
        limits: SupportHistoryLimits<H>,
        starts: &[[FixedStartCountBound; H]; CELLS],
    ) -> Result<Self, SupportLedgerError> {
        limits.validate()?;
        // The sealed active bounds must be the ledger's own bounds: a ledger
        // whose history disagrees with its Catalog seal is not constructible.
        (limits.active_start_bound == *starts)
            .then_some(())
            .ok_or_else(invalid)?;
        let expiry = ExpiryHeap::try_new(limits.expiry_ticket_capacity)?;
        let mut sealed = Vec::new();
        sealed
            .try_reserve_exact(1)
            .map_err(|_| SupportLedgerError::Storage(FixedStorageError::Allocation))?;
        sealed.push(limits);
        Ok(Self {
            limits: sealed.into_boxed_slice(),
            expiry,
            accumulator: Accumulator::default(),
            carry: CarrySlot::Vacant,
            ordinary: OrdinaryReservations::Running,
            time_floor: MonotonicTime::from_micros(0),
        })
    }

    pub(crate) fn limits(&self) -> &SupportHistoryLimits<H> {
        &self.limits[0]
    }

    pub(crate) fn accumulator_mut(&mut self) -> &mut Accumulator {
        &mut self.accumulator
    }

    /// Schedules the whole-group release ticket a terminal transition owes.
    /// The dormant ticket was reserved at creation, so this never allocates.
    pub(crate) fn schedule(&mut self, ticket: ExpiryTicket) -> Result<(), SupportLedgerError> {
        self.expiry.schedule(ticket)
    }

    /// A time-bearing mutation may not move the ledger backwards.
    pub(crate) fn check_floor(&self, at: MonotonicTime) -> Result<(), SupportLedgerError> {
        (at >= self.time_floor)
            .then_some(())
            .ok_or(SupportLedgerError::Storage(FixedStorageError::InvalidTime))
    }

    /// The complete immutable retention observation at `at`. Creating it never
    /// advances the generation and returns no Effect.
    pub(crate) fn facts(
        &self,
        nonce: u64,
        generation: SupportLedgerGeneration,
        at: MonotonicTime,
    ) -> RetentionFacts<H> {
        // Bounded on purpose: the observation must not scan the owner set, so
        // the due count saturates at the sealed group quota and the caller
        // learns "more are due" from `next_expiry_at` together with this cap.
        let (_, due, _, _) = self
            .expiry
            .due_prefix::<EXPIRY_OBSERVATION_GROUPS>(at, u32::MAX);
        RetentionFacts {
            nonce,
            generation,
            at,
            catalog: self.limits().catalog,
            configuration: self.limits().configuration,
            budget: self.limits().budget,
            retention_horizon: self.limits().retention(),
            horizons: std::array::from_fn(|index| self.limits().horizons[index].get()),
            accumulator: self.accumulator,
            interference_limit_us: self.limits().interference_limit_us,
            expiry_due: u32::try_from(due).expect("quota-bounded due count"),
            expiry_scheduled: self.expiry.len(),
            next_expiry_at: self.expiry.next_release(),
            carry_slot: self.carry,
            carry_capacity: CARRY_SLOTS,
            ordinary_reservations: self.ordinary,
        }
    }

    /// Read-only selection of the bounded due prefix. It mutates nothing, so
    /// dropping the returned selection changes no state.
    pub(crate) fn select_expiry<const E_GROUPS: usize, const E_UNITS: usize>(
        &self,
        at: MonotonicTime,
    ) -> Result<([ExpiryTicket; E_GROUPS], usize, bool, u64), SupportLedgerError> {
        let (sealed_groups, sealed_units) = self.limits().quotas();
        // The sealed quota pair must be used exactly; any other const pair is
        // rejected before any state is read.
        (u32::try_from(E_GROUPS) == Ok(sealed_groups)
            && u32::try_from(E_UNITS) == Ok(sealed_units))
        .then_some(())
        .ok_or_else(invalid)?;
        self.check_floor(at)?;
        Ok(self.expiry.due_prefix::<E_GROUPS>(at, sealed_units))
    }

    /// Re-derives the due prefix under the sealed quotas, for revalidating a
    /// prepared selection against the current heap.
    pub(crate) fn reselect<const E_GROUPS: usize>(
        &self,
        at: MonotonicTime,
    ) -> ([ExpiryTicket; E_GROUPS], usize) {
        let (_, units) = self.limits().quotas();
        let (selected, count, _, _) = self.expiry.due_prefix::<E_GROUPS>(at, units);
        (selected, count)
    }

    /// Releases exactly the validated whole groups and advances the time floor.
    /// The caller advances the ledger generation once when the batch is
    /// nonempty; a zero-group batch leaves the generation unchanged.
    pub(crate) fn commit_expiry(
        &mut self,
        at: MonotonicTime,
        count: usize,
        units: u32,
    ) -> (u32, u32) {
        self.expiry.release_prefix(count);
        self.time_floor = at;
        (
            u32::try_from(count).expect("constructor-bounded group count"),
            units,
        )
    }

    pub(crate) fn next_release(&self) -> Option<MonotonicTime> {
        self.expiry.next_release()
    }

    pub(crate) fn accumulator(&self) -> &Accumulator {
        &self.accumulator
    }

    pub(crate) fn scheduled(&self) -> &[ExpiryTicket] {
        self.expiry.scheduled()
    }

    /// The borrowed canonical views a carry input exposes.
    pub(crate) fn views(&self) -> (&[ExpiryTicket], &Accumulator, &CarrySlot) {
        (self.expiry.scheduled(), &self.accumulator, &self.carry)
    }
}

impl<'ledger, const H: usize> SupportCarryInput<'ledger, H> {
    pub(crate) fn new(
        snapshot: SupportLedgerSnapshot<H>,
        scheduled: &'ledger [ExpiryTicket],
        accumulator: &'ledger Accumulator,
        history: [u64; CELLS],
        vectors: &'ledger [[u64; H]; CELLS],
        reserved: &'ledger [[u32; POOLS]; 5],
        carry: &'ledger CarrySlot,
    ) -> Self {
        Self {
            snapshot,
            scheduled,
            accumulator,
            history,
            vectors,
            reserved,
            carry,
        }
    }
}

/// The complete immutable observation Admission consumes: the existing C16
/// capacity facts plus every C18 retention, interference, expiry and carry
/// fact. It is an owned fixed-size value; it is authority only while its seal,
/// generation, identities and `at` are revalidated.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SupportLedgerSnapshot<const H: usize> {
    pub(crate) capacity: super::SupportCapacitySnapshot<H>,
    pub(crate) retention: RetentionFacts<H>,
}

/// The complete immutable carry input C26 later consumes. It borrows the
/// ledger, so it cannot be cloned, copied, sent to another thread, stored in
/// Control state, or outlive the immutable borrow; creating it copies no whole
/// state and advances no generation.
#[derive(Debug)]
pub(crate) struct SupportCarryInput<'ledger, const H: usize> {
    snapshot: SupportLedgerSnapshot<H>,
    /// Every scheduled release group, in heap storage order. This is the
    /// retention view, not the operation inventory.
    scheduled: &'ledger [ExpiryTicket],
    /// The canonical occupancy inventory: obligations by state and pool,
    /// physical start credits, funding claims by typed kind and pool, live
    /// entitlements, retained tombstones and unreleased links.
    accumulator: &'ledger Accumulator,
    /// Retained catalog-wide start count per `(operation, pool)` cell, so a
    /// later carry evaluates short-to-long activation without a second copy.
    history: [u64; CELLS],
    /// Support Outstanding Credit Vector occupancy on every
    /// `(operation, pool, horizon)` axis.
    vectors: &'ledger [[u64; H]; CELLS],
    /// Held lifecycle reserves by capacity class and pool. The ledger tracks
    /// reserves on this axis; it holds no per-kind occupancy tensor, and this
    /// view does not invent one.
    reserved: &'ledger [[u32; POOLS]; 5],
    carry: &'ledger CarrySlot,
}

impl<const H: usize> SupportCarryInput<'_, H> {
    pub(crate) fn snapshot(&self) -> &SupportLedgerSnapshot<H> {
        &self.snapshot
    }

    /// The canonical scheduled-release view. It exposes vacancy and length and
    /// cannot filter.
    pub(crate) fn scheduled(&self) -> &[ExpiryTicket] {
        self.scheduled
    }

    /// Retained catalog-wide start count per `(operation, pool)` cell.
    pub(crate) fn history(&self) -> &[u64; CELLS] {
        &self.history
    }

    /// Support Outstanding Credit Vector occupancy on the same axes.
    pub(crate) fn vectors(&self) -> &[[u64; H]; CELLS] {
        self.vectors
    }

    /// Held reserves by capacity class and pool.
    pub(crate) fn reserved(&self) -> &[[u32; POOLS]; 5] {
        self.reserved
    }

    pub(crate) fn accumulator(&self) -> &Accumulator {
        self.accumulator
    }

    pub(crate) fn carry_slot(&self) -> &CarrySlot {
        self.carry
    }
}

/// The non-forgeable read-only expiry selection. It is bound to the exact
/// ledger instance, expected generation, aggregate before-image and borrowed
/// Work charge; dropping it changes no state.
pub(crate) struct PreparedSupportExpiry<'work, const E_GROUPS: usize> {
    pub(crate) work: &'work mut crate::WorkMeter,
    pub(crate) nonce: u64,
    pub(crate) expected: SupportLedgerGeneration,
    pub(crate) at: MonotonicTime,
    pub(crate) before: Accumulator,
    /// A caller-owned fixed selection: preparing allocates nothing.
    pub(crate) selected: [ExpiryTicket; E_GROUPS],
    pub(crate) count: usize,
    pub(crate) more_due: bool,
}

impl<const E_GROUPS: usize> PreparedSupportExpiry<'_, E_GROUPS> {
    /// The selected whole groups, in exact key order.
    pub(crate) fn groups(&self) -> &[ExpiryTicket] {
        &self.selected[..self.count]
    }
}

impl<const E_GROUPS: usize> std::fmt::Debug for PreparedSupportExpiry<'_, E_GROUPS> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PreparedSupportExpiry")
            .field("nonce", &self.nonce)
            .field("expected", &self.expected)
            .field("at", &self.at)
            .field("selected", &self.count)
            .field("more_due", &self.more_due)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
impl<const H: usize> SupportHistoryLimits<H> {
    /// Sealed limits consistent with one test ledger's own start bounds.
    /// Production limits are supplied by the C08 Catalog adapter; this builder
    /// only keeps the fixture self-consistent.
    pub(crate) fn testing(starts: [[FixedStartCountBound; H]; CELLS]) -> Self {
        let horizons = std::array::from_fn(|index| NonZeroDuration(starts[0][index].0));
        let pair = |predecessor: u8, successor: u8| PairCell {
            predecessor: BudgetIdentity([predecessor; 32]),
            successor: BudgetIdentity([successor; 32]),
            sequence: 0,
            mandatory: 7_297,
            safety: 8,
            history_reset: false,
        };
        let cells = [pair(1, 1), pair(1, 2), pair(2, 1), pair(2, 2)];
        Self {
            catalog: CatalogIdentity([1; 32]),
            configuration: ConfigurationIdentity([1; 32]),
            budget: BudgetIdentity([1; 32]),
            retention_horizon: horizons[H - 1],
            horizons,
            start_history_capacity: std::array::from_fn(|cell| starts[cell][H - 1].1),
            active_start_bound: starts,
            interference_limit_us: [u64::MAX; H],
            operation_capacity: [[1; POOLS]; STATES],
            physical_credit_capacity: [u32::MAX; CELLS],
            funding_claim_capacity: [[u32::MAX; POOLS]; CLAIM_KINDS],
            ordinary_claim_capacity: [u32::MAX; POOLS],
            owner_capacity: u32::MAX,
            link_capacity: u32::MAX,
            entitlement_capacity: 1,
            vector_capacity: std::array::from_fn(|cell| {
                std::array::from_fn(|horizon| u64::from(starts[cell][horizon].1))
            }),
            lifecycle_capacity: [[[u32::MAX; POOLS]; STATES]; LIFECYCLE_KINDS],
            mandatory_pair_capacity: PairCapacity(cells),
            safety_pair_capacity: PairCapacity(cells),
            expiry_ticket_capacity: (STATES * POOLS) as u32 + 1,
            expiry_groups_per_transition: NonZeroU32::new(1).expect("positive quota"),
            expiry_units_per_transition: NonZeroU32::new(1).expect("positive quota"),
            largest_atomic_release_group_units: NonZeroU32::new(1).expect("positive group"),
        }
    }
}

