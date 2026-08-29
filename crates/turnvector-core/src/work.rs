const DIMENSIONS: [WorkDimension; 5] = [
    WorkDimension::VisitedEntities,
    WorkDimension::CopiedBytes,
    WorkDimension::Allocations,
    WorkDimension::CandidateWork,
    WorkDimension::InvariantChecks,
];
#[repr(usize)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkDimension {
    VisitedEntities,
    CopiedBytes,
    Allocations,
    CandidateWork,
    InvariantChecks,
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct HotPathWorkWitness([u64; 5]);
impl HotPathWorkWitness {
    #[must_use]
    pub const fn new(values: [u64; 5]) -> Self {
        Self(values)
    }
    pub fn checked_add(self, other: Self) -> Result<Self, WorkBudgetError> {
        let mut values = [0; 5];
        for (index, dimension) in DIMENSIONS.into_iter().enumerate() {
            values[index] = self.0[index]
                .checked_add(other.0[index])
                .ok_or(WorkBudgetError::CounterOverflow(dimension))?;
        }
        Ok(Self(values))
    }
    pub const fn value(self, dimension: WorkDimension) -> u64 {
        self.0[dimension as usize]
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HotPathWorkBudget(HotPathWorkWitness);
impl HotPathWorkBudget {
    #[rustfmt::skip]
    #[must_use]
    pub const fn binary_maximum() -> Self {
        Self(HotPathWorkWitness::new([1_704_575, 2_097_152, 0, 2, 28_708]))
    }
    pub fn try_new(maxima: HotPathWorkWitness) -> Result<Self, WorkBudgetError> {
        let binary = Self::binary_maximum().0;
        for dimension in DIMENSIONS {
            let (actual, maximum) = (maxima.value(dimension), binary.value(dimension));
            if actual > maximum {
                let error = WorkBudgetError::BinaryMaximumExceeded(dimension, maximum, actual);
                return Err(error);
            }
            if dimension == WorkDimension::VisitedEntities && actual < 1_000_000 {
                return Err(WorkBudgetError::BudgetExceeded(dimension, actual, maximum));
            }
        }
        Ok(Self(maxima))
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkBudgetError {
    BinaryMaximumExceeded(WorkDimension, u64, u64),
    CounterOverflow(WorkDimension),
    BudgetExceeded(WorkDimension, u64, u64),
}
pub struct WorkMeter(HotPathWorkBudget, HotPathWorkWitness);
impl WorkMeter {
    pub const fn new(budget: HotPathWorkBudget) -> Self {
        Self(budget, HotPathWorkWitness([0; 5]))
    }
    pub fn record(&mut self, dimension: WorkDimension, amount: u64) -> Result<(), WorkBudgetError> {
        let overflow = WorkBudgetError::CounterOverflow(dimension);
        let current = self.1.0[dimension as usize];
        let attempted = current.checked_add(amount).ok_or(overflow)?;
        let maximum = self.0.0.value(dimension);
        if attempted > maximum {
            let error = WorkBudgetError::BudgetExceeded(dimension, maximum, attempted);
            return Err(error);
        }
        self.1.0[dimension as usize] = attempted;
        Ok(())
    }
    #[allow(
        dead_code,
        reason = "C16 transaction phases atomically charge full witnesses"
    )]
    pub(crate) fn charge(&mut self, witness: HotPathWorkWitness) -> Result<(), WorkBudgetError> {
        let next = self.1.checked_add(witness)?;
        for dimension in DIMENSIONS {
            let attempted = next.value(dimension);
            let maximum = self.0.0.value(dimension);
            if attempted > maximum {
                return Err(WorkBudgetError::BudgetExceeded(
                    dimension, maximum, attempted,
                ));
            }
        }
        self.1 = next;
        Ok(())
    }
    pub(crate) fn ensure(&self, required: HotPathWorkWitness) -> Result<(), WorkBudgetError> {
        for dimension in DIMENSIONS {
            let current = self.1.value(dimension);
            let attempted = current
                .checked_add(required.value(dimension))
                .ok_or(WorkBudgetError::CounterOverflow(dimension))?;
            let maximum = self.0.0.value(dimension);
            if attempted > maximum {
                return Err(WorkBudgetError::BudgetExceeded(
                    dimension, maximum, attempted,
                ));
            }
        }
        Ok(())
    }
    pub const fn witness(&self) -> HotPathWorkWitness {
        self.1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_witness_charge_is_atomic_and_reports_the_complete_attempt() {
        let mut meter = WorkMeter::new(HotPathWorkBudget::binary_maximum());
        meter
            .record(WorkDimension::VisitedEntities, 1_704_575)
            .unwrap();
        let before = meter.witness();
        assert_eq!(
            meter.charge(HotPathWorkWitness::new([75, 366, 0, 0, 20])),
            Err(WorkBudgetError::BudgetExceeded(
                WorkDimension::VisitedEntities,
                1_704_575,
                1_704_650,
            ))
        );
        assert_eq!(meter.witness(), before);
    }

    #[test]
    fn full_witness_charge_assigns_once_and_overflow_is_atomic() {
        let mut meter = WorkMeter::new(HotPathWorkBudget::binary_maximum());
        let witness = HotPathWorkWitness::new([75, 366, 0, 0, 20]);
        meter.charge(witness).unwrap();
        assert_eq!(meter.witness(), witness);

        let mut overflow = WorkMeter(
            HotPathWorkBudget(HotPathWorkWitness::new([u64::MAX; 5])),
            HotPathWorkWitness::new([u64::MAX, 1, 2, 3, 4]),
        );
        let before = overflow.witness();
        assert_eq!(
            overflow.charge(HotPathWorkWitness::new([1, 1, 1, 1, 1])),
            Err(WorkBudgetError::CounterOverflow(
                WorkDimension::VisitedEntities
            ))
        );
        assert_eq!(overflow.witness(), before);
    }
}
