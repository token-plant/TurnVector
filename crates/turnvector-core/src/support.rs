use crate::{
    FixedRecordArena, FixedStartCountBound, FixedStorageError, FixedWindowCounter,
    HotPathWorkWitness, MonotonicTime, PhysicalStartCreditId, SupportLedgerGeneration,
    SupportOperationObligationId, WorkDimension, WorkMeter,
};
const POOLS: usize = 3;
const CONDITIONAL: usize = 0;
const PENDING: usize = 1;
const ACTIVE: usize = 2;
const CREDITS: usize = 3;
const CLAIMS: usize = 4;
macro_rules! values {
    ($($name:ident { $($variant:ident $(($value:ty))?),+ $(,)? })+) => {$(
        #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        #[repr(usize)]
        pub enum $name { $($variant $(($value))?),+ }
    )+};
}
values! {
    SupportOperation {
        DescribeModel, DescribeRequest, MaterializeRequest, ReleaseRequest,
        FormCandidates, ObserveTurnReceipt, SampleBackendResources,
    }
    SupportPool { Ordinary, MandatoryCompletion, SafetySampling }
    SupportFundingClaim {
        OrdinaryReservation([u8; 32]), AdmissionInitial([u8; 32]),
        EntitlementVector([u8; 32]), LifecycleReserve([u8; 32]),
    }
    SupportObligationState {
        Conditional, Pending, Active, Retained, ClosedConditional, ClosedPending,
    }
}
use SupportObligationState::*;
use SupportTransition::{BeginSupport, CloseCausalCallImpossible, FinishSupport, PredecessorEnded};
impl SupportFundingClaim {
    fn valid_for(self, pool: SupportPool) -> bool {
        let (identity, pools) = match self {
            Self::OrdinaryReservation(id) => (id, 0b001),
            Self::AdmissionInitial(id) | Self::EntitlementVector(id) => (id, 0b010),
            Self::LifecycleReserve(id) => (id, 0b110),
        };
        identity != [0; 32] && pools & (1 << pool as usize) != 0
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SupportCausalPredecessorId(pub [u8; 32]);
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SupportCallScopeId(pub(crate) [u8; 32]);
pub struct SupportObligationSpec<'a> {
    pub id: SupportOperationObligationId,
    pub operation: SupportOperation,
    pub pool: SupportPool,
    pub physical_credit: PhysicalStartCreditId,
    pub predecessor: SupportCausalPredecessorId,
    pub claims: &'a [SupportFundingClaim],
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct OrdinarySupportSpec {
    pub(crate) id: SupportOperationObligationId,
    pub(crate) operation: SupportOperation,
    pub(crate) physical_credit: PhysicalStartCreditId,
    pub(crate) scope: SupportCallScopeId,
    pub(crate) claim: SupportFundingClaim,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SupportTransition {
    PredecessorEnded(SupportCausalPredecessorId, MonotonicTime),
    BeginSupport(MonotonicTime),
    FinishSupport,
    CloseCausalCallImpossible,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SupportLedgerError {
    InvalidInput,
    Generation,
    InvalidTransition,
    Storage(FixedStorageError),
}
const CAPACITY_ERROR: SupportLedgerError = SupportLedgerError::Storage(FixedStorageError::Capacity);
macro_rules! check {
    ($work:expr, $condition:expr, $error:expr) => {{
        $work.record(WorkDimension::InvariantChecks, 1)?;
        $condition.then_some(()).ok_or($error)
    }};
}
impl<T: Into<FixedStorageError>> From<T> for SupportLedgerError {
    fn from(error: T) -> Self {
        Self::Storage(error.into())
    }
}
type Record = (
    SupportOperation,
    SupportPool,
    SupportCausalPredecessorId,
    SupportObligationState,
    MonotonicTime,
    SupportCallScopeId,
);
pub struct SupportChargeLedger<const R: usize, const F: usize, const H: usize> {
    generation: SupportLedgerGeneration,
    capacities: [[u32; POOLS]; 5],
    max_claims: u16,
    records: FixedRecordArena<Record, SupportFundingClaim, 2>,
    usage: [[u32; POOLS]; 5],
    starts: FixedWindowCounter<21, H>,
}
impl<const R: usize, const F: usize, const H: usize> SupportChargeLedger<R, F, H> {
    #[allow(dead_code, reason = "C08 installs the Catalog adapter")]
    pub(crate) fn try_new(
        generation: SupportLedgerGeneration,
        capacities: [[u32; POOLS]; 5],
        max_claims: u16,
        starts: [[FixedStartCountBound; H]; 21],
    ) -> Result<Self, SupportLedgerError> {
        let valid = (1..=1_024).contains(&max_claims)
            && total(capacities[..3].iter().flatten().copied()) <= R as u64
            && total(capacities[CREDITS]) <= R as u64
            && total(capacities[CLAIMS]) <= F as u64;
        valid
            .then_some(())
            .ok_or(SupportLedgerError::InvalidInput)?;
        Ok(Self {
            generation,
            capacities,
            max_claims,
            records: FixedRecordArena::try_new(R, F)?,
            usage: [[0; POOLS]; 5],
            starts: FixedWindowCounter::try_new(starts)?,
        })
    }
    pub const fn generation(&self) -> SupportLedgerGeneration {
        self.generation
    }
    pub fn reserve(
        &mut self,
        expected: SupportLedgerGeneration,
        spec: SupportObligationSpec<'_>,
        work: &mut WorkMeter,
    ) -> Result<SupportLedgerGeneration, SupportLedgerError> {
        let next = self.next(expected, work)?;
        let count = spec.claims.len();
        let pool = spec.pool as usize;
        let invalid = SupportLedgerError::InvalidInput;
        for identity in [spec.id.0, spec.physical_credit.0, spec.predecessor.0] {
            check!(work, identity != [0; 32], invalid)?;
        }
        let valid_count = (1..=self.max_claims as usize).contains(&count);
        check!(work, valid_count, invalid)?;
        let remaining = HotPathWorkWitness::new([count as u64, 66, 0, 0, (2 * count + 3) as u64]);
        work.ensure(remaining)?;
        check!(work, spec.pool != SupportPool::Ordinary, invalid)?;
        let mut previous = None;
        for claim in spec.claims {
            work.record(WorkDimension::VisitedEntities, 1)?;
            if let Some(prior) = previous {
                check!(work, prior < claim, invalid)?;
            }
            check!(work, claim.valid_for(spec.pool), invalid)?;
            previous = Some(claim);
        }
        let claims = count as u32;
        for (class, added) in [(CONDITIONAL, 1), (CREDITS, 1), (CLAIMS, claims)] {
            check!(work, self.available(class, pool, added), CAPACITY_ERROR)?;
        }
        work.record(WorkDimension::CopiedBytes, 66)?;
        let keys = [key(0, spec.id.0), key(1, spec.physical_credit.0)];
        let record = (
            spec.operation,
            spec.pool,
            spec.predecessor,
            Conditional,
            Default::default(),
            SupportCallScopeId([0; 32]),
        );
        self.records.try_push(keys, record, spec.claims, work)?;
        self.usage[CONDITIONAL][pool] += 1;
        self.usage[CREDITS][pool] += 1;
        self.usage[CLAIMS][pool] += claims;
        self.generation = next;
        Ok(next)
    }
    #[allow(dead_code, reason = "C10 and C12 install the ordinary support callers")]
    pub(crate) fn begin_ordinary(
        &mut self,
        expected: SupportLedgerGeneration,
        spec: OrdinarySupportSpec,
        at: MonotonicTime,
        work: &mut WorkMeter,
    ) -> Result<SupportLedgerGeneration, SupportLedgerError> {
        let next = self.next(expected, work)?;
        work.record(WorkDimension::InvariantChecks, 3)?;
        let valid = [spec.id.0, spec.physical_credit.0, spec.scope.0]
            .into_iter()
            .all(|id| id != [0; 32])
            && matches!(spec.claim, SupportFundingClaim::OrdinaryReservation(id) if id != [0; 32]);
        check!(work, valid, SupportLedgerError::InvalidInput)?;
        let pool = SupportPool::Ordinary as usize;
        for class in [ACTIVE, CREDITS, CLAIMS] {
            check!(work, self.available(class, pool, 1), CAPACITY_ERROR)?;
        }
        for identity in [key(0, spec.id.0), key(1, spec.physical_credit.0)] {
            let absent = self.records.find(identity, work)?.is_none();
            check!(work, absent, FixedStorageError::Duplicate)?;
        }
        work.ensure(insertion_work())?;
        self.starts
            .try_start(spec.operation as usize * POOLS + pool, at, work)?;
        let record = (
            spec.operation,
            SupportPool::Ordinary,
            SupportCausalPredecessorId([0; 32]),
            Active,
            at,
            spec.scope,
        );
        let keys = [key(0, spec.id.0), key(1, spec.physical_credit.0)];
        self.records
            .try_push(keys, record, &[spec.claim], work)
            .expect("ordinary insertion was fully prevalidated");
        for class in [ACTIVE, CREDITS, CLAIMS] {
            self.usage[class][pool] += 1;
        }
        self.generation = next;
        Ok(next)
    }
    pub fn transition(
        &mut self,
        expected: SupportLedgerGeneration,
        id: SupportOperationObligationId,
        transition: SupportTransition,
        work: &mut WorkMeter,
    ) -> Result<SupportLedgerGeneration, SupportLedgerError> {
        let next = self.next(expected, work)?;
        work.record(WorkDimension::CopiedBytes, 33)?;
        let found = self.records.find(key(0, id.0), work)?;
        work.record(WorkDimension::InvariantChecks, 1)?;
        let index = found.ok_or(SupportLedgerError::InvalidTransition)?;
        let record_bytes = std::mem::size_of::<Record>() as u64;
        work.record(WorkDimension::CopiedBytes, record_bytes)?;
        let record = *self.records.get(index).expect("indexed support record");
        work.record(WorkDimension::InvariantChecks, 1)?;
        let (state, time) = match (record.3, transition) {
            (Conditional, PredecessorEnded(id, at)) if id == record.2 => (Pending, at),
            (Pending, BeginSupport(at)) if at >= record.4 => (Active, at),
            (Active, FinishSupport) => (Retained, record.4),
            (Conditional, CloseCausalCallImpossible) => (ClosedConditional, record.4),
            (Pending, CloseCausalCallImpossible) => (ClosedPending, record.4),
            _ => return Err(SupportLedgerError::InvalidTransition),
        };
        let pool = record.1 as usize;
        let (before, after) = (state_class(record.3), state_class(state));
        if before != after {
            check!(work, self.available(after, pool, 1), CAPACITY_ERROR)?;
        }
        if state == Active {
            self.starts
                .try_start(record.0 as usize * POOLS + pool, time, work)?;
        }
        if before != after {
            self.usage[before][pool] -= 1;
            self.usage[after][pool] += 1;
        }
        let record = self.records.get_mut(index).expect("indexed support record");
        record.3 = state;
        record.4 = time;
        self.generation = next;
        Ok(next)
    }
    fn next(
        &self,
        expected: SupportLedgerGeneration,
        work: &mut WorkMeter,
    ) -> Result<SupportLedgerGeneration, SupportLedgerError> {
        let current = expected == self.generation;
        check!(work, current, SupportLedgerError::Generation)?;
        work.record(WorkDimension::InvariantChecks, 1)?;
        self.generation
            .next()
            .map_err(|_| SupportLedgerError::Generation)
    }
    fn available(&self, class: usize, pool: usize, added: u32) -> bool {
        self.usage[class][pool]
            .checked_add(added)
            .is_some_and(|value| value <= self.capacities[class][pool])
    }
}
#[allow(dead_code, reason = "used by the C08 adapter constructor")]
fn total(values: impl IntoIterator<Item = u32>) -> u64 {
    values.into_iter().map(u64::from).sum()
}
fn state_class(state: SupportObligationState) -> usize {
    [CONDITIONAL, PENDING, ACTIVE, ACTIVE, CONDITIONAL, PENDING][state as usize]
}
fn insertion_work() -> HotPathWorkWitness {
    let copied = std::mem::size_of::<(Record, SupportFundingClaim)>() as u64 + 172;
    HotPathWorkWitness::new([1_662, copied, 0, 0, 16])
}
fn key(tag: u8, id: [u8; 32]) -> [u8; 33] {
    let mut key = [0; 33];
    key[0] = tag;
    key[1..].copy_from_slice(&id);
    key
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Duration, HotPathWorkBudget, WorkBudgetError};
    use FixedStorageError::{Duplicate, WindowExceeded};
    use SupportFundingClaim::{
        AdmissionInitial as Initial, LifecycleReserve as Lifecycle, OrdinaryReservation as Reserved,
    };
    use SupportLedgerError::{Generation as Stale, InvalidInput, InvalidTransition};
    use SupportPool::{MandatoryCompletion as Mandatory, Ordinary, SafetySampling as Safety};
    use SupportTransition::{
        BeginSupport as Begin, CloseCausalCallImpossible as Close, FinishSupport as Finish,
    };
    type Ledger = SupportChargeLedger<12, 12, 1>;
    type Result = std::result::Result<SupportLedgerGeneration, SupportLedgerError>;
    type Claim = SupportFundingClaim;
    fn new_ledger() -> Ledger {
        let generation = SupportLedgerGeneration::new(1).unwrap();
        let capacities = [[1, 2, 1], [0, 1, 0], [1, 2, 1], [1, 4, 1], [1, 4, 1]];
        let starts = [[FixedStartCountBound(Duration::from_micros(10), 1); 1]; 21];
        Ledger::try_new(generation, capacities, 2, starts).unwrap()
    }
    fn ordinary_ledger() -> Ledger {
        let mut ledger = new_ledger();
        for class in [ACTIVE, CREDITS, CLAIMS] {
            ledger.capacities[class][0] = 2;
        }
        ledger
    }
    fn work() -> WorkMeter {
        WorkMeter::new(HotPathWorkBudget::binary_maximum())
    }
    fn spec(n: u8, credit: u8, pool: SupportPool, claims: &[Claim]) -> SupportObligationSpec<'_> {
        SupportObligationSpec {
            id: SupportOperationObligationId([n; 32]),
            operation: SupportOperation::MaterializeRequest,
            pool,
            physical_credit: PhysicalStartCreditId([credit; 32]),
            predecessor: SupportCausalPredecessorId([n; 32]),
            claims,
        }
    }
    fn put(ledger: &mut Ledger, n: u8, c: u8, pool: SupportPool, claims: &[Claim]) -> Result {
        ledger.reserve(ledger.generation(), spec(n, c, pool, claims), &mut work())
    }
    fn add(ledger: &mut Ledger, n: u8, credit: u8) -> Result {
        put(ledger, n, credit, Mandatory, &[Initial([n; 32])])
    }
    fn go(ledger: &mut Ledger, n: u8, transition: SupportTransition) -> Result {
        let id = SupportOperationObligationId([n; 32]);
        ledger.transition(ledger.generation(), id, transition, &mut work())
    }
    #[test]
    fn support_ledger_contract() {
        let fail = |result: Result, error| assert_eq!(result, Err(error));
        let at = MonotonicTime::from_micros;
        let end = |n: u8, value| PredecessorEnded(SupportCausalPredecessorId([n; 32]), at(value));
        let mut ledger = new_ledger();
        add(&mut ledger, 1, 1).unwrap();
        fail(go(&mut ledger, 1, Begin(at(1))), InvalidTransition);
        fail(go(&mut ledger, 1, end(2, 1)), InvalidTransition);
        go(&mut ledger, 1, end(1, 5)).unwrap();
        go(&mut ledger, 1, Begin(at(5))).unwrap();
        go(&mut ledger, 1, Finish).unwrap();
        add(&mut ledger, 2, 2).unwrap();
        go(&mut ledger, 2, end(2, 5)).unwrap();
        fail(go(&mut ledger, 2, Begin(at(14))), WindowExceeded.into());
        go(&mut ledger, 2, Begin(at(15))).unwrap();
        add(&mut ledger, 3, 3).unwrap();
        go(&mut ledger, 3, end(3, 15)).unwrap();
        fail(go(&mut ledger, 3, Begin(at(25))), CAPACITY_ERROR);
        let mut ledger = new_ledger();
        let before = ledger.generation();
        for claims in [[Initial([1; 32]); 2], [Initial([2; 32]), Initial([1; 32])]] {
            let mut measured = work();
            let result = ledger.reserve(before, spec(7, 7, Mandatory, &claims), &mut measured);
            fail(result, InvalidInput);
            assert_eq!(measured.witness().value(WorkDimension::VisitedEntities), 2);
            assert_eq!(measured.witness().value(WorkDimension::InvariantChecks), 9);
        }
        assert_eq!((ledger.generation(), ledger.records.len()), (before, 0));
        let mut measured = work();
        let valid = spec(7, 7, Mandatory, &[Initial([7; 32])]);
        ledger.reserve(before, valid, &mut measured).unwrap();
        assert_eq!(measured.witness().value(WorkDimension::VisitedEntities), 3);
        let mut ledger = new_ledger();
        let before = ledger.generation();
        let mut exhausted = work();
        exhausted
            .record(WorkDimension::VisitedEntities, 1_000_000)
            .unwrap();
        let result = ledger.reserve(
            before,
            spec(7, 7, Mandatory, &[Initial([7; 32])]),
            &mut exhausted,
        );
        let error =
            WorkBudgetError::BudgetExceeded(WorkDimension::VisitedEntities, 1_000_000, 1_000_001);
        fail(result, error.into());
        assert_eq!((ledger.generation(), ledger.records.len()), (before, 0));
        fail(
            put(&mut ledger, 1, 1, Ordinary, &[Reserved([1; 32])]),
            InvalidInput,
        );
        add(&mut ledger, 1, 1).unwrap();
        put(&mut ledger, 9, 9, Safety, &[Lifecycle([9; 32])]).unwrap();
        assert_eq!(ledger.records.get(1).unwrap().5.0, [0; 32]);
        go(&mut ledger, 1, Close).unwrap();
        fail(add(&mut ledger, 2, 1), Duplicate.into());
        add(&mut ledger, 2, 2).unwrap();
        fail(add(&mut ledger, 3, 3), CAPACITY_ERROR);
        go(&mut ledger, 2, end(2, 1)).unwrap();
        go(&mut ledger, 2, Close).unwrap();
        add(&mut ledger, 3, 3).unwrap();
        fail(go(&mut ledger, 3, end(3, 1)), CAPACITY_ERROR);
        fail(put(&mut ledger, 7, 7, Safety, &[]), InvalidInput);
        let stale = SupportLedgerGeneration::new(1).unwrap();
        let id = SupportOperationObligationId([2; 32]);
        let result = ledger.transition(stale, id, end(2, 1), &mut work());
        fail(result, Stale);
    }

    fn ordinary(parts: (u8, u8, u8, Claim)) -> OrdinarySupportSpec {
        let (id, credit, scope, claim) = parts;
        OrdinarySupportSpec {
            id: SupportOperationObligationId([id; 32]),
            operation: SupportOperation::DescribeRequest,
            physical_credit: PhysicalStartCreditId([credit; 32]),
            scope: SupportCallScopeId([scope; 32]),
            claim,
        }
    }
    fn begin(ledger: &mut Ledger, spec: OrdinarySupportSpec, at: MonotonicTime) -> Result {
        ledger.begin_ordinary(ledger.generation(), spec, at, &mut work())
    }
    #[test]
    fn c08a_ordinary_reservation_contract() {
        let snapshot = |ledger: &Ledger| {
            let claims = |index| ledger.records.claims(index).map(<[_]>::to_vec);
            (
                ledger.generation(),
                std::array::from_fn::<_, 12, _>(|index| ledger.records.get(index).copied()),
                std::array::from_fn::<_, 12, _>(claims),
                std::array::from_fn::<_, 21, _>(|cell| ledger.starts.len(cell)),
                ledger.usage,
            )
        };
        macro_rules! rejected {
            ($ledger:ident, $action:expr, $error:expr) => {{
                let before = snapshot(&$ledger);
                assert_eq!($action, Err($error));
                assert_eq!(snapshot(&$ledger), before);
            }};
        }

        let at = MonotonicTime::from_micros;
        let valid = ordinary((1, 21, 41, Reserved([1; 32])));
        let mut ledger = ordinary_ledger();
        for invalid in [
            ordinary((0, 21, 41, Reserved([1; 32]))),
            ordinary((1, 0, 41, Reserved([1; 32]))),
            ordinary((1, 21, 0, Reserved([1; 32]))),
            ordinary((1, 21, 41, Reserved([0; 32]))),
            ordinary((1, 21, 41, Initial([1; 32]))),
        ] {
            rejected!(ledger, begin(&mut ledger, invalid, at(1)), InvalidInput);
        }
        let initial = ledger.generation();
        let next = begin(&mut ledger, valid, at(5)).unwrap();
        let record = ledger.records.get(0).unwrap();
        assert_eq!(next, initial.next().unwrap());
        let predecessor = SupportCausalPredecessorId([0; 32]);
        assert_eq!(
            (record.0, record.1, record.2),
            (valid.operation, Ordinary, predecessor)
        );
        assert_eq!((record.3, record.4, record.5), (Active, at(5), valid.scope));
        assert_eq!(ledger.records.claims(0), Some(&[valid.claim][..]));
        for class in [ACTIVE, CREDITS, CLAIMS] {
            assert_eq!(ledger.usage[class][0], 1);
        }
        assert_eq!(ledger.starts.len(valid.operation as usize * POOLS), Some(1));

        let second = ordinary((2, 22, 42, Reserved([2; 32])));
        let error = SupportLedgerError::Storage(Duplicate);
        for (id, credit) in [(1, 22), (2, 21)] {
            let duplicate = ordinary((id, credit, 42, Reserved([2; 32])));
            rejected!(ledger, begin(&mut ledger, duplicate, at(6)), error);
        }
        let error = SupportLedgerError::Storage(WindowExceeded);
        rejected!(ledger, begin(&mut ledger, second, at(6)), error);
        rejected!(
            ledger,
            ledger.begin_ordinary(initial, second, at(15), &mut work()),
            Stale
        );

        let mut ledger = new_ledger();
        begin(&mut ledger, valid, at(5)).unwrap();
        rejected!(ledger, begin(&mut ledger, second, at(15)), CAPACITY_ERROR);

        let mut ledger = new_ledger();
        let mut exhausted = work();
        exhausted
            .record(WorkDimension::VisitedEntities, 999_998)
            .unwrap();
        let before = snapshot(&ledger);
        let fault = ledger.begin_ordinary(ledger.generation(), valid, at(1), &mut exhausted);
        assert!(matches!(
            fault,
            Err(SupportLedgerError::Storage(FixedStorageError::Work(_)))
        ));
        assert_eq!(snapshot(&ledger), before);
    }
}
