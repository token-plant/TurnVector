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
pub(crate) struct ResourceSnapshot {
    generation: ResourceCapacityLedgerGeneration,
    limit: ResourceCapacity,
    used: ResourceCapacity,
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
}
use ResourceAction::{Reserve, WithdrawBeforeMaterialization as Withdraw};
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ResourceInput {
    pub(crate) action: ResourceAction,
    pub(crate) authority: ResourceAuthorityId,
    pub(crate) expected: ResourceCapacityLedgerGeneration,
    pub(crate) reservation: ResourceReservation,
}
#[derive(Clone, Debug, Eq, PartialEq)]
struct ResourceIndices {
    records: Vec<ResourceReservation>,
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
            .binary_search_by_key(&reservation.id, |value| value.id);
        let budget = self.budgets.binary_search(&reservation.backend_budget);
        (record, budget)
    }
    fn prepare<const R: usize>(
        &self,
        action: ResourceAction,
        reservation: ResourceReservation,
    ) -> LedgerResult<(usize, usize)> {
        self.invariant()?;
        let (record, budget) = self.positions(reservation);
        match action {
            Reserve => {
                require(self.records.len() < R, Full)?;
                Ok((
                    record.map_or_else(Ok, |_| Err(DuplicateReservation))?,
                    budget.map_or_else(Ok, |_| Err(DuplicateBackendBudget))?,
                ))
            }
            Withdraw => {
                let record = record.map_err(|_| MissingReservation)?;
                require(self.records[record] == reservation, BeforeImageMismatch)?;
                Ok((record, budget.map_err(|_| BeforeImageMismatch)?))
            }
        }
    }
    fn target_matches(&self, action: &PreparedAction) -> bool {
        let positions = self.positions(action.reservation);
        match action.action {
            Reserve => positions == (Err(action.positions.0), Err(action.positions.1)),
            Withdraw => {
                positions == (Ok(action.positions.0), Ok(action.positions.1))
                    && self.records[action.positions.0] == action.reservation
            }
        }
    }
    fn apply(&mut self, action: &PreparedAction) {
        match action.action {
            Reserve => {
                self.records.insert(action.positions.0, action.reservation);
                self.budgets
                    .insert(action.positions.1, action.reservation.backend_budget);
            }
            Withdraw => {
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
    action: ResourceAction,
    reservation: ResourceReservation,
    positions: (usize, usize),
    next_used: ResourceCapacity,
    generation: ResourceCapacityLedgerGeneration,
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
        let positions = state.indices.prepare::<R>(input.action, reservation)?;
        let next_used = match input.action {
            Reserve => state
                .used
                .checked_reserve(reservation.capacity, self.limit)?,
            Withdraw => state.used.checked_release(reservation.capacity)?,
        };
        let action = PreparedAction {
            action: input.action,
            reservation,
            positions,
            next_used,
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
        let replaced = arithmetic(replaced.checked_add(size_of::<ResourceReservation>()))?;
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
        arithmetic(size_of::<ResourceReservation>().checked_add(size_of::<BackendBudgetId>()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use WorkDimension::*;
    type Capacity = ResourceCapacity;
    type Change<'a, const R: usize> = ResourceChange<'a, R>;
    type Error = ResourceLedgerError;
    type Input = ResourceInput;
    type Ledger<const R: usize> = ResourceCapacityLedger<R>;
    type Generation = ResourceCapacityLedgerGeneration;
    type Reservation = ResourceReservation;
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
    type IndexPointers = (*const Reservation, *const BackendBudgetId);
    fn assert_index<const R: usize>(ledger: &Ledger<R>, length: usize, pointers: IndexPointers) {
        let state = ledger.state.borrow();
        let (r, b) = (&state.indices.records, &state.indices.budgets);
        assert_eq!((r.capacity(), r.len(), r.as_ptr()), (R, length, pointers.0));
        assert_eq!((b.capacity(), b.len(), b.as_ptr()), (R, length, pointers.1));
        assert!(r.windows(2).all(|pair| pair[0].id < pair[1].id));
        assert!(b.windows(2).all(|pair| pair[0] < pair[1]));
    }
    fn assert_maximum<const R: usize>(expected: [u64; 5]) {
        assert_eq!(
            Ledger::<R>::maximum_work().unwrap(),
            HotPathWorkWitness::new(expected)
        );
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
        let mutations: [fn(&mut Change<'_, 3>); 11] = [
            |value| value.before.limit.backend.0 += 1,
            |value| value.before.limit.output.0 += 1,
            |value| value.before.limit.transient.0 += 1,
            |value| value.before.used.backend.0 += 1,
            |value| value.before.used.output.0 += 1,
            |value| value.before.used.transient.0 += 1,
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
        assert_eq!(size_of::<BackendBudgetId>(), 32);
        let capability_bytes = size_of::<Change<'static, 1>>() + size_of::<Validated<'static, 1>>();
        assert_eq!(capability_bytes, 384);
        assert_maximum::<1>([10, 712, 0, 0, 18]);
        assert_maximum::<3>([18, 864, 0, 0, 18]);
        assert_maximum::<1_024>([50, 123_384, 0, 0, 18]);
        assert_maximum::<17_472>([70, 2_097_144, 0, 0, 18]);
        assert_eq!(Ledger::<17_472>::storage_bytes().unwrap(), 2_096_784);
        drop(self::ledger::<17_472>(amounts(0, 0, 0)));
        let exceeded = WorkBudgetError::BudgetExceeded(CopiedBytes, 2_097_152, 2_097_264);
        let oversized = Work(exceeded);
        assert_eq!(construction_error::<17_473>(), Some(oversized));
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
        for length in (1_usize..=64).chain([1_024, 10_917, 10_918, 17_472, 17_473]) {
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
}
