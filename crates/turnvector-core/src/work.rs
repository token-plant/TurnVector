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
