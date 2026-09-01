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

