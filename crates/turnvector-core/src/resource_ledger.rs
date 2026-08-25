//! Fixed-capacity accounting for admitted request resources.

#![allow(dead_code, reason = "linked by later Runtime Core rows")]

use crate::{HotPathWorkBudget, HotPathWorkWitness, WorkBudgetError, WorkDimension, WorkMeter};
use std::cell::{RefCell, RefMut};
use std::mem::size_of;
use std::num::NonZeroU64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ResourceDimension {
    BackendAllocation,
    DaemonOutput,
    TransientHeadroom,
}
use ResourceDimension::{
    BackendAllocation as Backend, DaemonOutput as Output, TransientHeadroom as Transient,
};
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ResourceLedgerError {
    ZeroIdentity,
    ZeroGeneration,
    InvalidCapacity,
    CapacityExceeded(ResourceDimension),
    ArithmeticOverflow,
    WrongAuthority,
    StaleGeneration,
    GenerationOverflow,
    DuplicateReservation,
    DuplicateBackendBudget,
    Full,
    MissingReservation,
    BeforeImageMismatch,
    InvalidSettlement,
    InvalidBackendPartition,
    WrongPendingReclaimAnchor,
    ResourceEvidenceNotNewer,
    ResourceEvidenceReplay,
    WrongLedger,
    ExclusiveCapability,
    Work(WorkBudgetError),
}
use ResourceLedgerError::*;
pub(crate) type LedgerResult<T> = Result<T, ResourceLedgerError>;
impl From<WorkBudgetError> for ResourceLedgerError {
    fn from(error: WorkBudgetError) -> Self {
        Self::Work(error)
    }
}
fn require(condition: bool, error: ResourceLedgerError) -> LedgerResult<()> {
    condition.then_some(()).ok_or(error)
}
fn arithmetic<T>(value: Option<T>) -> LedgerResult<T> {
    value.ok_or(ArithmeticOverflow)
}

macro_rules! identity {
    ($name:ident) => {
        #[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
        pub(crate) struct $name([u8; 32]);
        impl $name {
            pub(crate) fn new(value: [u8; 32]) -> LedgerResult<Self> {
                require(value != [0; 32], ZeroIdentity)?;
                Ok(Self(value))
            }
        }
    };
}
identity!(ResourceAuthorityId);
identity!(ResourceReservationId);
identity!(BackendBudgetId);
identity!(ResourceEvidenceId);
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct BackendAllocationCapacity(pub(crate) u64);
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct DaemonOutputCapacity(pub(crate) u64);
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct TransientHeadroomCapacity(pub(crate) u64);
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ResourceCapacityLedgerGeneration(NonZeroU64);
impl ResourceCapacityLedgerGeneration {
    pub(crate) fn new(value: u64) -> LedgerResult<Self> {
        NonZeroU64::new(value).map(Self).ok_or(ZeroGeneration)
    }
    pub(crate) const fn get(self) -> u64 {
        self.0.get()
    }
    fn next(self) -> LedgerResult<Self> {
        Self::new(self.get().checked_add(1).ok_or(GenerationOverflow)?)
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ResourceEvidenceCursor(pub(crate) u64);
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ResourceEvidencePoint {
    pub(crate) cursor: ResourceEvidenceCursor,
    pub(crate) evidence: ResourceEvidenceId,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DaemonTerminalFact {
    PartialMaterialization,
    QueuedAfterInvalidation,
    InFlightAfterReceipt,
    OrdinaryAfterReceipt,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BackendPartitionSource {
    ZeroMaterialization,
    OwnershipConsumed,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct BackendPartitionFact {
    pub(crate) source: BackendPartitionSource,
    pub(crate) allocated: BackendAllocationCapacity,
    pub(crate) never_allocated: BackendAllocationCapacity,
    pub(crate) floor: ResourceEvidencePoint,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PendingReclaimAnchor {
    pub(crate) reservation: ResourceReservationId,
    pub(crate) backend_budget: BackendBudgetId,
    pub(crate) opened_generation: ResourceCapacityLedgerGeneration,
    pub(crate) floor: ResourceEvidencePoint,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PendingReclaimConvergence {
    pub(crate) anchor: PendingReclaimAnchor,
    pub(crate) observed: ResourceEvidencePoint,
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct ResourceCapacity {
    pub(crate) backend: BackendAllocationCapacity,
    pub(crate) output: DaemonOutputCapacity,
    pub(crate) transient: TransientHeadroomCapacity,
}
impl ResourceCapacity {
    pub(crate) const fn new(backend: u64, output: u64, transient: u64) -> Self {
        Self {
            backend: BackendAllocationCapacity(backend),
            output: DaemonOutputCapacity(output),
            transient: TransientHeadroomCapacity(transient),
        }
    }
    fn checked_reserve(self, amount: Self, limit: Self) -> LedgerResult<Self> {
        Ok(Self::new(
            reserve_one(self.backend.0, amount.backend.0, limit.backend.0, Backend)?,
            reserve_one(self.output.0, amount.output.0, limit.output.0, Output)?,
            reserve_one(
                self.transient.0,
                amount.transient.0,
                limit.transient.0,
                Transient,
            )?,
        ))
    }
    fn checked_release(self, amount: Self) -> LedgerResult<Self> {
        let backend = release_one(self.backend.0, amount.backend.0)?;
        let output = release_one(self.output.0, amount.output.0)?;
        let transient = release_one(self.transient.0, amount.transient.0)?;
        Ok(Self::new(backend, output, transient))
    }
}
fn reserve_one(
    used: u64,
    amount: u64,
    limit: u64,
    dimension: ResourceDimension,
) -> LedgerResult<u64> {
    let available = limit.checked_sub(used).ok_or(BeforeImageMismatch)?;
    require(amount <= available, CapacityExceeded(dimension))?;
    used.checked_add(amount).ok_or(ArithmeticOverflow)
}
fn release_one(used: u64, amount: u64) -> LedgerResult<u64> {
    used.checked_sub(amount).ok_or(BeforeImageMismatch)
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ResourceReservation {
    pub(crate) id: ResourceReservationId,
    pub(crate) backend_budget: BackendBudgetId,
    pub(crate) capacity: ResourceCapacity,
}
impl ResourceReservation {
    pub(crate) const fn new(
        id: ResourceReservationId,
        backend_budget: BackendBudgetId,
        capacity: ResourceCapacity,
    ) -> Self {
        Self {
            id,
            backend_budget,
            capacity,
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PendingReclaimState {
    opened_generation: ResourceCapacityLedgerGeneration,
    amount: BackendAllocationCapacity,
    floor: ResourceEvidencePoint,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BackendSettlement {
    Held,
    Pending(PendingReclaimState),
    Closed,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ResourceSettlement {
    daemon_open: bool,
    backend: BackendSettlement,
}
impl ResourceSettlement {
    const HELD: Self = Self {
        daemon_open: true,
        backend: BackendSettlement::Held,
    };
    fn mutation(self) -> RecordMutation {
        match (self.daemon_open, self.backend) {
            (false, BackendSettlement::Closed) => RecordMutation::Remove,
            _ => RecordMutation::Replace(self),
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ResourceRecord {
    reservation: ResourceReservation,
    settlement: ResourceSettlement,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ResourceSnapshot {
    generation: ResourceCapacityLedgerGeneration,
    limit: ResourceCapacity,
    used: ResourceCapacity,
    pending_reclaim: BackendAllocationCapacity,
    reservations: usize,
    backend_budgets: usize,
}
impl ResourceSnapshot {
    pub(crate) const fn generation(self) -> ResourceCapacityLedgerGeneration {
        self.generation
    }
    pub(crate) const fn limit(self) -> ResourceCapacity {
        self.limit
    }
    pub(crate) const fn used(self) -> ResourceCapacity {
        self.used
    }
    pub(crate) const fn pending_reclaim(self) -> BackendAllocationCapacity {
        self.pending_reclaim
    }
    pub(crate) const fn reservations(self) -> usize {
        self.reservations
    }
    pub(crate) const fn backend_budgets(self) -> usize {
        self.backend_budgets
    }
    pub(crate) fn available(self) -> LedgerResult<ResourceCapacity> {
        self.limit.checked_release(self.used)
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ResourceAction {
    Reserve,
    WithdrawBeforeMaterialization,
    SettleDaemon(DaemonTerminalFact),
    ApplyBackendPartition(BackendPartitionFact),
    ConvergePendingReclaim(PendingReclaimConvergence),
}
use ResourceAction::{
    ApplyBackendPartition, ConvergePendingReclaim, Reserve, SettleDaemon,
    WithdrawBeforeMaterialization as Withdraw,
};
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ResourceInput {
    pub(crate) action: ResourceAction,
    pub(crate) authority: ResourceAuthorityId,
    pub(crate) expected: ResourceCapacityLedgerGeneration,
    pub(crate) reservation: ResourceReservation,
}
#[derive(Clone, Debug, Eq, PartialEq)]
struct ResourceIndices {
    records: Vec<ResourceRecord>,
    budgets: Vec<BackendBudgetId>,
}
type IndexPositions = (Result<usize, usize>, Result<usize, usize>);
impl ResourceIndices {
    fn try_new<const R: usize>() -> LedgerResult<Self> {
        Ok(Self {
            records: Self::allocate::<_, R>()?,
            budgets: Self::allocate::<_, R>()?,
        })
    }
    fn allocate<T, const R: usize>() -> LedgerResult<Vec<T>> {
        let mut index = Vec::new();
        index.try_reserve_exact(R).map_err(|_| InvalidCapacity)?;
        Self::seal::<_, R>(index)
    }
    fn seal<T, const R: usize>(index: Vec<T>) -> LedgerResult<Vec<T>> {
        require(index.capacity() == R, InvalidCapacity)?;
        Ok(index)
    }
    fn invariant(&self) -> LedgerResult<()> {
        require(
            self.records.len() == self.budgets.len(),
            BeforeImageMismatch,
        )
    }
    fn positions(&self, reservation: ResourceReservation) -> IndexPositions {
        let record = self
            .records
            .binary_search_by_key(&reservation.id, |value| value.reservation.id);
        let budget = self.budgets.binary_search(&reservation.backend_budget);
        (record, budget)
    }
    fn prepare<const R: usize>(
        &self,
        action: ResourceAction,
        reservation: ResourceReservation,
    ) -> LedgerResult<((usize, usize), Option<ResourceSettlement>)> {
        self.invariant()?;
        let (record, budget) = self.positions(reservation);
        match action {
            Reserve => {
                require(self.records.len() < R, Full)?;
                Ok((
                    (
                        record.map_or_else(Ok, |_| Err(DuplicateReservation))?,
                        budget.map_or_else(Ok, |_| Err(DuplicateBackendBudget))?,
                    ),
                    None,
                ))
            }
            Withdraw | SettleDaemon(_) | ApplyBackendPartition(_) | ConvergePendingReclaim(_) => {
                let record = record.map_err(|_| MissingReservation)?;
                let stored = self.records[record];
                require(stored.reservation == reservation, BeforeImageMismatch)?;
                Ok((
                    (record, budget.map_err(|_| BeforeImageMismatch)?),
                    Some(stored.settlement),
                ))
            }
        }
    }
    fn target_matches(&self, action: &PreparedAction) -> bool {
        let positions = self.positions(action.reservation);
        match action.before_settlement {
            None => positions == (Err(action.positions.0), Err(action.positions.1)),
            Some(settlement) => {
                positions == (Ok(action.positions.0), Ok(action.positions.1))
                    && self.records[action.positions.0]
                        == ResourceRecord {
                            reservation: action.reservation,
                            settlement,
                        }
            }
        }
    }
    fn apply(&mut self, action: &PreparedAction) {
        match action.mutation {
            RecordMutation::Insert => {
                self.records.insert(
                    action.positions.0,
                    ResourceRecord {
                        reservation: action.reservation,
                        settlement: ResourceSettlement::HELD,
                    },
                );
                self.budgets
                    .insert(action.positions.1, action.reservation.backend_budget);
            }
            RecordMutation::Replace(settlement) => {
                self.records[action.positions.0].settlement = settlement;
            }
            RecordMutation::Remove => {
                self.records.remove(action.positions.0);
                self.budgets.remove(action.positions.1);
            }
        }
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
struct ResourceState {
    generation: ResourceCapacityLedgerGeneration,
    used: ResourceCapacity,
    pending_reclaim: BackendAllocationCapacity,
    indices: ResourceIndices,
}
pub(crate) struct ResourceCapacityLedger<const R: usize> {
    authority: ResourceAuthorityId,
    limit: ResourceCapacity,
    state: RefCell<ResourceState>,
}
pub(crate) struct ResourceChange<'a, const R: usize> {
    owner: &'a ResourceCapacityLedger<R>,
    before: ResourceSnapshot,
    action: PreparedAction,
}
struct PreparedAction {
    reservation: ResourceReservation,
    positions: (usize, usize),
    before_settlement: Option<ResourceSettlement>,
    mutation: RecordMutation,
    next_used: ResourceCapacity,
    next_pending_reclaim: BackendAllocationCapacity,
    generation: ResourceCapacityLedgerGeneration,
}
#[derive(Clone, Copy)]
enum RecordMutation {
    Insert,
    Replace(ResourceSettlement),
    Remove,
}
pub(crate) struct ValidatedResourceChange<'a, const R: usize> {
    state: RefMut<'a, ResourceState>,
    action: PreparedAction,
    capacity: std::marker::PhantomData<[(); R]>,
}
impl<const R: usize> ResourceCapacityLedger<R> {
    pub(crate) fn try_new(
        authority: ResourceAuthorityId,
        generation: ResourceCapacityLedgerGeneration,
        limit: ResourceCapacity,
    ) -> LedgerResult<Self> {
        require(R != 0, InvalidCapacity)?;
        Self::maximum_work()?;
        Self::storage_bytes()?;
        Ok(Self {
            authority,
            limit,
            state: RefCell::new(ResourceState {
                generation,
                used: ResourceCapacity::default(),
                pending_reclaim: BackendAllocationCapacity(0),
                indices: ResourceIndices::try_new::<R>()?,
            }),
        })
    }
    pub(crate) fn prepare<'a>(
        &'a self,
        input: ResourceInput,
        work: &mut WorkMeter,
    ) -> LedgerResult<ResourceChange<'a, R>> {
        Self::charge(work, Self::prepare_work()?)?;
        let state = self.state.try_borrow().map_err(|_| ExclusiveCapability)?;
        require(input.authority == self.authority, WrongAuthority)?;
        require(input.expected == state.generation, StaleGeneration)?;
        let before = self.snapshot_from(&state);
        let reservation = input.reservation;
        let generation = state.generation.next()?;
        let (positions, before_settlement) =
            state.indices.prepare::<R>(input.action, reservation)?;
        let (mutation, next_used, next_pending_reclaim) = match input.action {
            Reserve => (
                RecordMutation::Insert,
                state
                    .used
                    .checked_reserve(reservation.capacity, self.limit)?,
                state.pending_reclaim,
            ),
            Withdraw => {
                require(
                    before_settlement == Some(ResourceSettlement::HELD),
                    InvalidSettlement,
                )?;
                (
                    RecordMutation::Remove,
                    state.used.checked_release(reservation.capacity)?,
                    state.pending_reclaim,
                )
            }
            SettleDaemon(_) => {
                let mut settlement = before_settlement.ok_or(BeforeImageMismatch)?;
                require(settlement.daemon_open, InvalidSettlement)?;
                settlement.daemon_open = false;
                let daemon = ResourceCapacity::new(
                    0,
                    reservation.capacity.output.0,
                    reservation.capacity.transient.0,
                );
                (
                    settlement.mutation(),
                    state.used.checked_release(daemon)?,
                    state.pending_reclaim,
                )
            }
            ApplyBackendPartition(partition) => {
                let mut settlement = before_settlement.ok_or(BeforeImageMismatch)?;
                require(
                    settlement.backend == BackendSettlement::Held,
                    InvalidSettlement,
                )?;
                let total = arithmetic(
                    partition
                        .allocated
                        .0
                        .checked_add(partition.never_allocated.0),
                )?;
                require(
                    total == reservation.capacity.backend.0,
                    InvalidBackendPartition,
                )?;
                let closes_daemon = partition.source == BackendPartitionSource::ZeroMaterialization;
                require(!closes_daemon || settlement.daemon_open, InvalidSettlement)?;
                settlement.daemon_open &= !closes_daemon;
                settlement.backend = if partition.allocated.0 == 0 {
                    BackendSettlement::Closed
                } else {
                    BackendSettlement::Pending(PendingReclaimState {
                        opened_generation: generation,
                        amount: partition.allocated,
                        floor: partition.floor,
                    })
                };
                let released = ResourceCapacity::new(
                    partition.never_allocated.0,
                    u64::from(closes_daemon) * reservation.capacity.output.0,
                    u64::from(closes_daemon) * reservation.capacity.transient.0,
                );
                let pending =
                    arithmetic(state.pending_reclaim.0.checked_add(partition.allocated.0))?;
                (
                    settlement.mutation(),
                    state.used.checked_release(released)?,
                    BackendAllocationCapacity(pending),
                )
            }
            ConvergePendingReclaim(convergence) => {
                let mut settlement = before_settlement.ok_or(BeforeImageMismatch)?;
                let pending = match settlement.backend {
                    BackendSettlement::Pending(pending) => pending,
                    BackendSettlement::Held | BackendSettlement::Closed => {
                        return Err(InvalidSettlement);
                    }
                };
                require(
                    convergence.anchor
                        == PendingReclaimAnchor {
                            reservation: reservation.id,
                            backend_budget: reservation.backend_budget,
                            opened_generation: pending.opened_generation,
                            floor: pending.floor,
                        },
                    WrongPendingReclaimAnchor,
                )?;
                require(
                    convergence.observed.cursor.0 > pending.floor.cursor.0,
                    ResourceEvidenceNotNewer,
                )?;
                require(
                    convergence.observed.evidence != pending.floor.evidence,
                    ResourceEvidenceReplay,
                )?;
                settlement.backend = BackendSettlement::Closed;
                let next_pending = release_one(state.pending_reclaim.0, pending.amount.0)?;
                let released = ResourceCapacity::new(pending.amount.0, 0, 0);
                (
                    settlement.mutation(),
                    state.used.checked_release(released)?,
                    BackendAllocationCapacity(next_pending),
                )
            }
        };
        let action = PreparedAction {
            reservation,
            positions,
            before_settlement,
            mutation,
            next_used,
            next_pending_reclaim,
            generation,
        };
        Ok(ResourceChange {
            owner: self,
            before,
            action,
        })
    }
    pub(crate) fn validate<'a>(
        &'a self,
        change: ResourceChange<'a, R>,
        work: &mut WorkMeter,
    ) -> LedgerResult<ValidatedResourceChange<'a, R>> {
        Self::charge(work, Self::validate_work()?)?;
        require(std::ptr::eq(self, change.owner), WrongLedger)?;
        let state = self
            .state
            .try_borrow_mut()
            .map_err(|_| ExclusiveCapability)?;
        state.indices.invariant()?;
        require(
            state.generation == change.before.generation,
            StaleGeneration,
        )?;
        require(
            self.snapshot_from(&state) == change.before,
            BeforeImageMismatch,
        )?;
        require(
            state.indices.target_matches(&change.action),
            BeforeImageMismatch,
        )?;
        Ok(ValidatedResourceChange {
            state,
            action: change.action,
            capacity: std::marker::PhantomData,
        })
    }
    pub(crate) fn commit(
        mut change: ValidatedResourceChange<'_, R>,
    ) -> ResourceCapacityLedgerGeneration {
        let action = change.action;
        let state = &mut *change.state;
        state.indices.apply(&action);
        state.used = action.next_used;
        state.pending_reclaim = action.next_pending_reclaim;
        state.generation = action.generation;
        action.generation
    }
    pub(crate) fn snapshot(&self, work: &mut WorkMeter) -> LedgerResult<ResourceSnapshot> {
        let witness = HotPathWorkWitness::new([0, size_of::<ResourceSnapshot>() as u64, 0, 0, 1]);
        Self::charge(work, witness)?;
        let state = self.state.try_borrow().map_err(|_| ExclusiveCapability)?;
        Ok(self.snapshot_from(&state))
    }
    fn snapshot_from(&self, state: &ResourceState) -> ResourceSnapshot {
        ResourceSnapshot {
            generation: state.generation,
            limit: self.limit,
            used: state.used,
            pending_reclaim: state.pending_reclaim,
            reservations: state.indices.records.len(),
            backend_budgets: state.indices.budgets.len(),
        }
    }
    fn charge(work: &mut WorkMeter, witness: HotPathWorkWitness) -> LedgerResult<()> {
        work.ensure(witness)?;
        for dimension in [
            WorkDimension::VisitedEntities,
            WorkDimension::CopiedBytes,
            WorkDimension::Allocations,
            WorkDimension::CandidateWork,
            WorkDimension::InvariantChecks,
        ] {
            work.record(dimension, witness.value(dimension))?;
        }
        Ok(())
    }
    fn lookup_bound() -> LedgerResult<u64> {
        require(R != 0, InvalidCapacity)?;
        Ok(u64::from(usize::BITS - (R - 1).leading_zeros() + 1))
    }
    fn prepare_work() -> LedgerResult<HotPathWorkWitness> {
        Self::phase_work(size_of::<ResourceChange<'static, R>>(), 8)
    }
    fn validate_work() -> LedgerResult<HotPathWorkWitness> {
        let item = Self::item_bytes()?;
        let shifted = arithmetic(R.checked_add(1).and_then(|value| value.checked_mul(item)))?;
        let replaced = arithmetic(item.checked_mul(2))?;
        let replaced = arithmetic(replaced.checked_add(size_of::<ResourceRecord>()))?;
        let copied = shifted.max(replaced);
        let copied =
            arithmetic(copied.checked_add(size_of::<ValidatedResourceChange<'static, R>>()))?;
        Self::phase_work(copied, 10)
    }
    fn phase_work(copied: usize, checks: u64) -> LedgerResult<HotPathWorkWitness> {
        let visited = arithmetic(Self::lookup_bound()?.checked_mul(2))?;
        let visited = arithmetic(visited.checked_add(3))?;
        Ok(HotPathWorkWitness::new([
            visited,
            u64::try_from(copied).map_err(|_| ArithmeticOverflow)?,
            0,
            0,
            checks,
        ]))
    }
    fn maximum_work() -> LedgerResult<HotPathWorkWitness> {
        let maximum = Self::prepare_work()?.checked_add(Self::validate_work()?)?;
        WorkMeter::new(HotPathWorkBudget::binary_maximum()).ensure(maximum)?;
        Ok(maximum)
    }
    fn storage_bytes() -> LedgerResult<usize> {
        let records = arithmetic(R.checked_mul(Self::item_bytes()?))?;
        arithmetic(records.checked_add(size_of::<Self>()))
    }
    fn item_bytes() -> LedgerResult<usize> {
        arithmetic(size_of::<ResourceRecord>().checked_add(size_of::<BackendBudgetId>()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use WorkDimension::*;
    use {BackendPartitionSource::*, DaemonTerminalFact::*};
    type Capacity = ResourceCapacity;
    type Change<'a, const R: usize> = ResourceChange<'a, R>;
    type Error = ResourceLedgerError;
    type Evidence = ResourceEvidencePoint;
    type Input = ResourceInput;
    type Ledger<const R: usize> = ResourceCapacityLedger<R>;
    type Generation = ResourceCapacityLedgerGeneration;
    type ReclaimAnchor = PendingReclaimAnchor;
    type Reservation = ResourceReservation;
    type Source = BackendPartitionSource;
    type Validated<'a, const R: usize> = ValidatedResourceChange<'a, R>;
    type Command = (ResourceAction, Generation, Reservation);
    const AUTHORITY: ResourceAuthorityId = ResourceAuthorityId([1; 32]);
    const INITIAL: Generation = ResourceCapacityLedgerGeneration(NonZeroU64::MIN);
    fn meter() -> WorkMeter {
        WorkMeter::new(HotPathWorkBudget::binary_maximum())
    }
    fn amounts(backend: u64, output: u64, transient: u64) -> Capacity {
        Capacity::new(backend, output, transient)
    }
    fn ledger<const R: usize>(limit: Capacity) -> Ledger<R> {
        Ledger::try_new(AUTHORITY, INITIAL, limit).unwrap()
    }
    fn construction_error<const R: usize>() -> Option<Error> {
        Ledger::<R>::try_new(AUTHORITY, INITIAL, Capacity::default()).err()
    }
    fn reservation(id: u8, budget: u8, capacity: Capacity) -> Reservation {
        Reservation::new(
            ResourceReservationId::new([id; 32]).unwrap(),
            BackendBudgetId::new([budget; 32]).unwrap(),
            capacity,
        )
    }
    fn unit(id: u8, budget: u8) -> Reservation {
        reservation(id, budget, amounts(1, 1, 1))
    }
    fn evidence(cursor: u64, id: u8) -> Evidence {
        ResourceEvidencePoint {
            cursor: ResourceEvidenceCursor(cursor),
            evidence: ResourceEvidenceId::new([id; 32]).unwrap(),
        }
    }
    fn partition(kind: Source, used: u64, free: u64, floor: Evidence) -> ResourceAction {
        ApplyBackendPartition(BackendPartitionFact {
            source: kind,
            allocated: BackendAllocationCapacity(used),
            never_allocated: BackendAllocationCapacity(free),
            floor,
        })
    }
    fn anchor(value: Reservation, opened: Generation, floor: Evidence) -> ReclaimAnchor {
        ReclaimAnchor {
            reservation: value.id,
            backend_budget: value.backend_budget,
            opened_generation: opened,
            floor,
        }
    }
    fn reclaim(anchor: ReclaimAnchor, observed: Evidence) -> ResourceAction {
        ConvergePendingReclaim(PendingReclaimConvergence { anchor, observed })
    }
    fn input(action: ResourceAction, expected: Generation, reservation: Reservation) -> Input {
        Input {
            action,
            authority: AUTHORITY,
            expected,
            reservation,
        }
    }
    fn prepared<const R: usize>(ledger: &Ledger<R>, command: Command) -> Change<'_, R> {
        let mut work = meter();
        let (kind, at, value) = command;
        ledger.prepare(input(kind, at, value), &mut work).unwrap()
    }
    fn transact<const R: usize>(ledger: &Ledger<R>, command: Input) -> Generation {
        let mut work = meter();
        let change = ledger.prepare(command, &mut work).unwrap();
        let generation = Ledger::commit(ledger.validate(change, &mut work).unwrap());
        assert_eq!(work.witness(), Ledger::<R>::maximum_work().unwrap());
        generation
    }
    fn step<const R: usize>(ledger: &Ledger<R>, command: Command) -> Generation {
        let (action, generation, value) = command;
        transact(ledger, input(action, generation, value))
    }
    fn reject_input<const R: usize>(ledger: &Ledger<R>, command: Input, error: Error) {
        let before = ledger.state.borrow().clone();
        let mut work = meter();
        assert_eq!(ledger.prepare(command, &mut work).err(), Some(error));
        assert_eq!(*ledger.state.borrow(), before);
        assert_eq!(work.witness(), Ledger::<R>::prepare_work().unwrap());
    }
    fn reject<const R: usize>(ledger: &Ledger<R>, command: Command, error: Error) {
        let (action, generation, value) = command;
        reject_input(ledger, input(action, generation, value), error);
    }
    fn validate_fails<const R: usize>(ledger: &Ledger<R>, change: Change<'_, R>, error: Error) {
        let before = ledger.state.borrow().clone();
        let mut work = meter();
        assert_eq!(ledger.validate(change, &mut work).err(), Some(error));
        assert_eq!(*ledger.state.borrow(), before);
        assert_eq!(work.witness(), Ledger::<R>::validate_work().unwrap());
    }
    fn capacity_for(dimension: ResourceDimension, selected: u64, other: u64) -> Capacity {
        match dimension {
            Backend => amounts(selected, other, other),
            Output => amounts(other, selected, other),
            Transient => amounts(other, other, selected),
        }
    }
    type IndexPointers = (*const ResourceRecord, *const BackendBudgetId);
    fn assert_index<const R: usize>(ledger: &Ledger<R>, length: usize, pointers: IndexPointers) {
        let state = ledger.state.borrow();
        let (r, b) = (&state.indices.records, &state.indices.budgets);
        assert_eq!((r.capacity(), r.len(), r.as_ptr()), (R, length, pointers.0));
        assert_eq!((b.capacity(), b.len(), b.as_ptr()), (R, length, pointers.1));
        let id = |value: &ResourceRecord| value.reservation.id;
        assert!(r.windows(2).all(|pair| id(&pair[0]) < id(&pair[1])));
        assert!(b.windows(2).all(|pair| pair[0] < pair[1]));
    }
    fn assert_maximum<const R: usize>(expected: [u64; 5]) {
        assert_eq!(
            Ledger::<R>::maximum_work().unwrap(),
            HotPathWorkWitness::new(expected)
        );
    }
    fn totals<const R: usize>(ledger: &Ledger<R>) -> (Capacity, u64, usize) {
        let s = ledger.snapshot(&mut meter()).unwrap();
        assert_eq!(s.reservations(), s.backend_budgets());
        (s.used(), s.pending_reclaim().0, s.reservations())
    }
    fn pending_state(value: &mut ResourceSettlement) -> Option<&mut PendingReclaimState> {
        match &mut value.backend {
            BackendSettlement::Pending(pending) => Some(pending),
            _ => None,
        }
    }

    #[test]
    fn fixed_independent_indices_fill_churn_and_reuse() {
        let record_seal = ResourceIndices::seal::<_, 3>(Vec::<Reservation>::with_capacity(4)).err();
        let budget_seal =
            ResourceIndices::seal::<_, 3>(Vec::<BackendBudgetId>::with_capacity(4)).err();
        assert_eq!([record_seal, budget_seal], [Some(InvalidCapacity); 2]);
        let ledger = ledger::<3>(amounts(9, 9, 9));
        let state = ledger.state.borrow();
        let indices = &state.indices;
        let pointers = (indices.records.as_ptr(), indices.budgets.as_ptr());
        drop(state);
        let initial = [unit(2, 8), unit(4, 2), unit(6, 4)];
        let replacement = [unit(2, 7), unit(5, 2), unit(6, 4)];
        assert_eq!(initial[0].id.0, initial[1].backend_budget.0);
        let mut generation = INITIAL;
        for (length, value) in initial.into_iter().enumerate() {
            generation = transact(&ledger, input(Reserve, generation, value));
            assert_index(&ledger, length + 1, pointers);
        }
        reject(&ledger, (Reserve, generation, unit(9, 9)), Full);
        for (old, new) in initial.into_iter().zip(replacement) {
            generation = transact(&ledger, input(Withdraw, generation, old));
            assert_index(&ledger, 2, pointers);
            generation = transact(&ledger, input(Reserve, generation, new));
            assert_index(&ledger, 3, pointers);
        }
        let snapshot = ledger.snapshot(&mut meter()).unwrap();
        assert_eq!(snapshot.limit(), amounts(9, 9, 9));
        assert_eq!(snapshot.used(), amounts(3, 3, 3));
        assert_eq!(snapshot.available(), Ok(amounts(6, 6, 6)));
        assert_eq!(snapshot.generation().get(), 10);
    }
    #[test]
    fn capacity_identity_and_generation_rejections_are_exact() {
        assert_eq!(ResourceAuthorityId::new([0; 32]).unwrap_err(), ZeroIdentity);
        let reservation_zero = ResourceReservationId::new([0; 32]).unwrap_err();
        assert_eq!(reservation_zero, ZeroIdentity);
        assert_eq!(BackendBudgetId::new([0; 32]).unwrap_err(), ZeroIdentity);
        assert_eq!(ResourceEvidenceId::new([0; 32]).unwrap_err(), ZeroIdentity);
        assert_eq!(Generation::new(0).unwrap_err(), ZeroGeneration);
        for (id, dimension) in [2, 6, 10].into_iter().zip([Backend, Output, Transient]) {
            let error = CapacityExceeded(dimension);
            let exact_capacity = capacity_for(dimension, 2, 0);
            let exact = ledger::<1>(exact_capacity);
            let held = reservation(id, id + 1, exact_capacity);
            let generation = transact(&exact, input(Reserve, INITIAL, held));
            assert_eq!(exact.snapshot(&mut meter()).unwrap().used, exact_capacity);
            transact(&exact, input(Withdraw, generation, held));
            let one_past = ledger::<1>(exact_capacity);
            let excessive = reservation(id, id + 1, capacity_for(dimension, 3, 0));
            reject(&one_past, (Reserve, INITIAL, excessive), error);
            let overflow = ledger::<2>(capacity_for(dimension, u64::MAX, 0));
            let held = reservation(id, id + 1, capacity_for(dimension, u64::MAX, 0));
            let generation = transact(&overflow, input(Reserve, INITIAL, held));
            let extra = reservation(id + 2, id + 3, capacity_for(dimension, 1, 0));
            reject(&overflow, (Reserve, generation, extra), error);
            let nonborrowing = ledger::<1>(capacity_for(dimension, 0, 2));
            let requested = reservation(id, id + 1, capacity_for(dimension, 1, 0));
            reject(&nonborrowing, (Reserve, INITIAL, requested), error);
        }
        let ledger = ledger::<3>(amounts(2, 2, 2));
        let base = unit(2, 3);
        let mut wrong = input(Reserve, INITIAL, base);
        wrong.authority = ResourceAuthorityId::new([9; 32]).unwrap();
        reject_input(&ledger, wrong, WrongAuthority);
        let stale = Generation::new(2).unwrap();
        reject(&ledger, (Reserve, stale, base), StaleGeneration);
        let generation = transact(&ledger, input(Reserve, INITIAL, base));
        for (value, error) in [
            (reservation(2, 4, amounts(0, 0, 0)), DuplicateReservation),
            (reservation(4, 3, amounts(0, 0, 0)), DuplicateBackendBudget),
        ] {
            reject(&ledger, (Reserve, generation, value), error);
        }
        for wrong in [
            reservation(2, 4, amounts(1, 1, 1)),
            reservation(2, 3, amounts(0, 1, 1)),
            reservation(2, 3, amounts(1, 0, 1)),
            reservation(2, 3, amounts(1, 1, 0)),
        ] {
            reject(&ledger, (Withdraw, generation, wrong), BeforeImageMismatch);
        }
        let missing = unit(9, 3);
        reject(&ledger, (Withdraw, generation, missing), MissingReservation);
        let maximum = Generation::new(u64::MAX).unwrap();
        let overflow = Ledger::<1>::try_new(AUTHORITY, maximum, amounts(1, 1, 1)).unwrap();
        let value = unit(2, 3);
        reject(&overflow, (Reserve, maximum, value), GenerationOverflow);
    }

    #[test]
    fn validated_changes_bind_instance_before_image_and_generation() {
        let first = ledger::<3>(amounts(3, 3, 3));
        let second = ledger::<3>(amounts(3, 3, 3));
        let wrong_owner = prepared(&first, (Reserve, INITIAL, unit(2, 3)));
        validate_fails(&second, wrong_owner, WrongLedger);
        let before = first.snapshot(&mut meter()).unwrap();
        let one = prepared(&first, (Reserve, INITIAL, unit(2, 3)));
        let two = prepared(&first, (Reserve, INITIAL, unit(4, 5)));
        let validated = first.validate(one, &mut meter()).unwrap();
        let mut blocked_work = meter();
        let blocked = first.validate(two, &mut blocked_work).err();
        let expected = Ledger::<3>::validate_work().unwrap();
        let actual = (blocked, blocked_work.witness());
        assert_eq!(actual, (Some(ExclusiveCapability), expected));
        drop(validated);
        let next = prepared(&first, (Reserve, INITIAL, unit(6, 7)));
        let dropped = first.validate(next, &mut meter()).unwrap();
        drop(dropped);
        assert_eq!(first.snapshot(&mut meter()).unwrap(), before);
        let _: fn(Validated<'_, 3>) -> Generation = Ledger::<3>::commit;
        let stale = prepared(&first, (Reserve, INITIAL, unit(2, 3)));
        let temporary = unit(4, 5);
        let generation = transact(&first, input(Reserve, INITIAL, temporary));
        transact(&first, input(Withdraw, generation, temporary));
        let mut restored = first.snapshot(&mut meter()).unwrap();
        restored.generation = before.generation;
        assert_eq!(restored, before);
        validate_fails(&first, stale, StaleGeneration);
        let held = unit(2, 3);
        let generation = transact(&second, input(Reserve, INITIAL, held));
        let mutations: [fn(&mut Change<'_, 3>); 12] = [
            |value| value.before.limit.backend.0 += 1,
            |value| value.before.limit.output.0 += 1,
            |value| value.before.limit.transient.0 += 1,
            |value| value.before.used.backend.0 += 1,
            |value| value.before.used.output.0 += 1,
            |value| value.before.used.transient.0 += 1,
            |value| value.before.pending_reclaim.0 += 1,
            |value| value.before.reservations += 1,
            |value| value.before.backend_budgets += 1,
            |value| value.action.positions.0 += 1,
            |value| value.action.positions.1 += 1,
            |value| value.action.reservation = unit(4, 5),
        ];
        for mutation in mutations {
            let mut change = prepared(&second, (Withdraw, generation, held));
            mutation(&mut change);
            validate_fails(&second, change, BeforeImageMismatch);
        }
        let change = prepared(&second, (Withdraw, generation, held));
        let unrelated = BackendBudgetId::new([9; 32]).unwrap();
        second.state.borrow_mut().indices.budgets.push(unrelated);
        validate_fails(&second, change, BeforeImageMismatch);
        let skewed = (Reserve, generation, unit(4, 5));
        reject(&second, skewed, BeforeImageMismatch);
    }
    #[test]
    fn snapshot_work_layout_limits_and_rust_search_are_exact() {
        let ledger = ledger::<3>(amounts(3, 3, 3));
        let expected = HotPathWorkWitness::new([0, size_of::<ResourceSnapshot>() as u64, 0, 0, 1]);
        let mut empty_work = meter();
        ledger.snapshot(&mut empty_work).unwrap();
        let mut generation = INITIAL;
        for value in [unit(2, 3), unit(4, 5), unit(6, 7)] {
            generation = transact(&ledger, input(Reserve, generation, value));
        }
        let mut full_work = meter();
        let mut full = ledger.snapshot(&mut full_work).unwrap();
        let observed = (empty_work.witness(), full_work.witness());
        assert_eq!(observed, (expected, expected));
        assert_eq!([full.reservations(), full.backend_budgets()], [3, 3]);
        full.used.backend.0 += 1;
        assert_eq!(full.available(), Err(BeforeImageMismatch));
        assert_eq!(size_of::<Reservation>(), 88);
        assert_eq!(size_of::<ResourceRecord>(), 160);
        assert_eq!(size_of::<BackendBudgetId>(), 32);
        let capability_bytes = size_of::<Change<'static, 1>>() + size_of::<Validated<'static, 1>>();
        assert_eq!(capability_bytes, 680);
        assert_maximum::<1>([10, 1_224, 0, 0, 18]);
        assert_maximum::<3>([18, 1_448, 0, 0, 18]);
        assert_maximum::<1_024>([50, 197_480, 0, 0, 18]);
        assert_maximum::<10_918>([66, 2_097_128, 0, 0, 18]);
        assert_eq!(Ledger::<10_918>::storage_bytes().unwrap(), 2_096_408);
        drop(self::ledger::<10_918>(amounts(0, 0, 0)));
        let exceeded = WorkBudgetError::BudgetExceeded(CopiedBytes, 2_097_152, 2_097_320);
        let oversized = Work(exceeded);
        assert_eq!(construction_error::<10_919>(), Some(oversized));
        assert_eq!(construction_error::<0>(), Some(InvalidCapacity));
        let arithmetic = construction_error::<{ usize::MAX }>();
        assert_eq!(arithmetic, Some(ArithmeticOverflow));
        let exact = self::ledger::<3>(amounts(3, 3, 3));
        let maximum = Ledger::<3>::maximum_work().unwrap();
        let request = reservation(2, 3, amounts(3, 3, 3));
        for (dimension, binary) in [
            (VisitedEntities, 1_704_575),
            (CopiedBytes, 2_097_152),
            (InvariantChecks, 28_708),
        ] {
            let mut work = meter();
            work.record(dimension, binary - maximum.value(dimension) + 1)
                .unwrap();
            let command = input(Reserve, INITIAL, request);
            let change = exact.prepare(command, &mut work).unwrap();
            let before_state = exact.state.borrow().clone();
            let before_work = work.witness();
            assert!(matches!(
                exact.validate(change, &mut work),
                Err(Work(WorkBudgetError::BudgetExceeded(actual, _, _))) if actual == dimension
            ));
            assert_eq!(*exact.state.borrow(), before_state);
            assert_eq!(work.witness(), before_work);
        }
        let mut work = meter();
        let command = input(Reserve, INITIAL, request);
        let change = exact.prepare(command, &mut work).unwrap();
        let generation = Ledger::commit(exact.validate(change, &mut work).unwrap());
        assert_eq!(work.witness(), maximum);
        assert_eq!(maximum.value(Allocations), 0);
        assert_eq!(maximum.value(CandidateWork), 0);
        transact(&exact, input(Withdraw, generation, request));
        fn comparisons(values: &[usize], target: usize) -> u64 {
            let mut count = 0;
            let _ = values.binary_search_by(|value| {
                count += 1;
                value.cmp(&target)
            });
            count
        }
        for length in (1_usize..=64).chain([1_024, 10_917, 10_918, 10_919]) {
            let values = (0..length).map(|value| value * 2).collect::<Vec<_>>();
            let expected = u64::from(usize::BITS - (length - 1).leading_zeros() + 1);
            let mut found = 0;
            let mut missing = 0;
            for target in 0..=length {
                found = found.max(comparisons(&values, target.min(length - 1) * 2));
                missing = missing.max(comparisons(&values, target * 2 + 1));
            }
            assert_eq!((found, missing), (expected, expected));
        }
    }
    #[test]
    fn settlement_contract_is_exact() {
        let held = reservation(2, 3, amounts(9, 7, 5));
        let floor = evidence(4, 5);
        for (source, allocated, never_allocated) in [
            (ZeroMaterialization, 6, 3),
            (ZeroMaterialization, 0, 9),
            (OwnershipConsumed, 0, 9),
        ] {
            let ledger = ledger::<1>(amounts(9, 7, 5));
            let mut generation = transact(&ledger, input(Reserve, INITIAL, held));
            let action = partition(source, allocated, never_allocated, floor);
            generation = step(&ledger, (action, generation, held));
            let daemon = u64::from(source == OwnershipConsumed);
            let count = usize::from(allocated != 0 || daemon != 0);
            let expected = (amounts(allocated, 7 * daemon, 5 * daemon), allocated, count);
            assert_eq!(totals(&ledger), expected);
            if allocated != 0 {
                let action = reclaim(anchor(held, generation, floor), evidence(9, 6));
                generation = step(&ledger, (action, generation, held));
            }
            if daemon != 0 {
                reject(&ledger, (Withdraw, generation, held), InvalidSettlement);
                reject(&ledger, (action, generation, held), InvalidSettlement);
                let action = SettleDaemon(PartialMaterialization);
                generation = step(&ledger, (action, generation, held));
            }
            assert_eq!(totals(&ledger), (Capacity::default(), 0, 0));
            transact(&ledger, input(Reserve, generation, held));
        }
        let facts = [
            PartialMaterialization,
            QueuedAfterInvalidation,
            InFlightAfterReceipt,
            OrdinaryAfterReceipt,
        ];
        for (index, fact) in facts.into_iter().enumerate() {
            let ledger = ledger::<2>(amounts(18, 14, 10));
            let mut generation = transact(&ledger, input(Reserve, INITIAL, held));
            let backend = partition(OwnershipConsumed, 6, 3, floor);
            let daemon = SettleDaemon(fact);
            let actions = [[backend, daemon], [daemon, backend]][index % 2];
            let mut opened = generation;
            for action in actions {
                generation = step(&ledger, (action, generation, held));
                if action == backend {
                    opened = generation;
                }
                if index == 1 && action == daemon {
                    reject(&ledger, (Withdraw, generation, held), InvalidSettlement);
                    reject(&ledger, (daemon, generation, held), InvalidSettlement);
                }
            }
            assert_eq!(totals(&ledger), (amounts(6, 0, 0), 6, 1));
            reject(&ledger, (Withdraw, generation, held), InvalidSettlement);
            let duplicate = (Reserve, generation, reservation(8, 3, Capacity::default()));
            reject(&ledger, duplicate, DuplicateBackendBudget);
            if index == 0 {
                let other = reservation(8, 9, amounts(9, 7, 5));
                generation = step(&ledger, (Reserve, generation, other));
                generation = step(&ledger, (backend, generation, other));
                let other_opened = generation;
                assert_eq!(totals(&ledger), (amounts(12, 7, 5), 12, 2));
                let action = reclaim(anchor(held, opened, floor), evidence(9, 6));
                generation = step(&ledger, (action, generation, held));
                assert_eq!(totals(&ledger), (amounts(6, 7, 5), 6, 1));
                let action = reclaim(anchor(other, other_opened, floor), evidence(9, 6));
                generation = step(&ledger, (action, generation, other));
                generation = step(&ledger, (SettleDaemon(fact), generation, other));
                assert_eq!(totals(&ledger), (Capacity::default(), 0, 0));
            } else {
                let action = reclaim(anchor(held, opened, floor), evidence(9, 6));
                generation = step(&ledger, (action, generation, held));
            }
            transact(&ledger, input(Reserve, generation, held));
        }
        let ledger = ledger::<2>(amounts(9, 7, 5));
        let at = transact(&ledger, input(Reserve, INITIAL, held));
        let invalid = partition(OwnershipConsumed, 7, 3, floor);
        reject(&ledger, (invalid, at, held), InvalidBackendPartition);
        let overflow = partition(OwnershipConsumed, u64::MAX, 1, floor);
        reject(&ledger, (overflow, at, held), ArithmeticOverflow);
        let action = partition(OwnershipConsumed, 6, 3, floor);
        let opened = step(&ledger, (action, at, held));
        let daemon = SettleDaemon(OrdinaryAfterReceipt);
        reject(&ledger, (daemon, at, held), StaleGeneration);
        let pending = anchor(held, opened, floor);
        let mut wrong = [pending; 5];
        wrong[0].reservation = unit(8, 9).id;
        wrong[1].backend_budget = unit(8, 9).backend_budget;
        wrong[2].opened_generation = at;
        wrong[3].floor = evidence(3, 5);
        wrong[4].floor = evidence(4, 6);
        for anchor in wrong {
            let action = reclaim(anchor, evidence(9, 6));
            reject(&ledger, (action, opened, held), WrongPendingReclaimAnchor);
        }
        for observed in [evidence(4, 6), evidence(3, 6)] {
            let action = reclaim(pending, observed);
            reject(&ledger, (action, opened, held), ResourceEvidenceNotNewer);
        }
        let replay = reclaim(pending, evidence(9, 5));
        reject(&ledger, (replay, opened, held), ResourceEvidenceReplay);
        let converge = reclaim(pending, evidence(9, 6));
        let generation = step(&ledger, (converge, opened, held));
        reject(&ledger, (converge, generation, held), InvalidSettlement);
        let generation = step(&ledger, (daemon, generation, held));
        let generation = transact(&ledger, input(Reserve, generation, held));
        let opened = step(&ledger, (action, generation, held));
        let command = (SettleDaemon(PartialMaterialization), opened, held);
        let before = ledger.state.borrow().indices.records[0].settlement;
        let mutations: [fn(&mut ResourceSettlement); 6] = [
            |value| value.daemon_open = false,
            |value| value.backend = BackendSettlement::Closed,
            |value| pending_state(value).unwrap().amount.0 += 1,
            |value| pending_state(value).unwrap().opened_generation = INITIAL,
            |value| pending_state(value).unwrap().floor.cursor.0 += 1,
            |value| pending_state(value).unwrap().floor.evidence = evidence(9, 9).evidence,
        ];
        for mutation in mutations {
            let change = prepared(&ledger, command);
            mutation(&mut ledger.state.borrow_mut().indices.records[0].settlement);
            validate_fails(&ledger, change, BeforeImageMismatch);
            ledger.state.borrow_mut().indices.records[0].settlement = before;
        }
    }
}
