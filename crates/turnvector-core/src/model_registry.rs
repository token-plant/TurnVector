#![allow(dead_code, reason = "C09 Core wiring belongs to the integration row")]
use crate::WorkDimension::{CopiedBytes, InvariantChecks, VisitedEntities};
use crate::{
    FixedIdentityIndex, FixedIndexError, HotPathWorkBudget, ModelId, WorkBudgetError, WorkMeter,
};
const INDEX_COMMIT_WORK: (u64, u64) = (827, 256); // Fixed-index insert lookup and path.
macro_rules! digest_identity {
    ($name:ident) => {
        #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub(crate) struct $name([u8; 32]);
        impl $name {
            pub(crate) fn new(bytes: [u8; 32]) -> RegistryResult<Self> {
                (bytes != [0; 32])
                    .then_some(Self(bytes))
                    .ok_or(RegistryError::InvalidInput)
            }
        }
    };
}
macro_rules! values {
    ($($visibility:vis enum $name:ident { $($variant:ident $(($($value:ty),+))?),+ $(,)? })+) => {$(
        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        $visibility enum $name { $($variant $(($($value),+))?),+ }
    )+};
}
type RegistryResult<T> = Result<T, RegistryError>;
digest_identity!(ModelRevisionId);
digest_identity!(ModelManifestId);
digest_identity!(ModelAliasId);
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RegistryGeneration(u64);
impl RegistryGeneration {
    pub(crate) fn new(value: u64) -> RegistryResult<Self> {
        (value != 0)
            .then_some(Self(value))
            .ok_or(RegistryError::InvalidInput)
    }
    fn next(self) -> RegistryResult<Self> {
        let value = self.0.checked_add(1).ok_or(RegistryError::Generation)?;
        Ok(Self(value))
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RegisteredRevision {
    pub(crate) model: ModelId,
    pub(crate) revision: ModelRevisionId,
    pub(crate) manifest: ModelManifestId,
    pub(crate) lifecycle: RevisionLifecycle,
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct RegistryCounts {
    pub(crate) registered: u32,
    pub(crate) available: u32,
    pub(crate) retiring: u32,
    pub(crate) unavailable: u32,
    pub(crate) aliases: u32,
}
values! {
    pub(crate) enum RevisionLifecycle { Available, Retiring, Unavailable }
    pub(crate) enum RegistryError {
        InvalidInput, Generation, RegistryLimit, AliasLimit, RevisionExists, UnknownRevision,
        AliasFrozen, InvalidLifecycle, PreparedChangeStale, Index(FixedIndexError),
        Work(WorkBudgetError),
    }
    pub(crate) enum RegistryCommand {
        Register(ModelId, ModelRevisionId, ModelManifestId),
        BindAlias(ModelAliasId, ModelRevisionId),
        Retire(ModelRevisionId), MarkUnavailable(ModelRevisionId),
    }
    enum PreparedDelta {
        Register(RegisteredRevision), BindAlias((ModelAliasId, usize)),
        SetLifecycle(usize, RegisteredRevision, RevisionLifecycle),
    }
}
use RegistryCommand::{BindAlias, MarkUnavailable, Register, Retire};
use RevisionLifecycle::{Available, Retiring, Unavailable};
impl<T: Into<FixedIndexError>> From<T> for RegistryError {
    fn from(error: T) -> Self {
        match error.into() {
            FixedIndexError::Work(error) => Self::Work(error),
            error => Self::Index(error),
        }
    }
}
#[derive(Debug)]
pub(crate) struct RegistryChange {
    expected: RegistryGeneration,
    before: RegistryCounts,
    after: RegistryCounts,
    delta: PreparedDelta,
}
pub(crate) struct ModelRegistry<const REVISIONS: usize, const ALIASES: usize> {
    generation: RegistryGeneration,
    revisions: [Option<RegisteredRevision>; REVISIONS],
    revision_index: FixedIdentityIndex<usize>,
    alias_index: FixedIdentityIndex<usize>,
    counts: RegistryCounts,
}
impl<const R: usize, const A: usize> ModelRegistry<R, A> {
    pub(crate) fn try_new(generation: RegistryGeneration) -> RegistryResult<Self> {
        if R == 0 || A == 0 {
            return Err(RegistryError::InvalidInput);
        }
        Ok(Self {
            generation,
            revisions: [None; R],
            revision_index: FixedIdentityIndex::try_new(R)?,
            alias_index: FixedIdentityIndex::try_new(A)?,
            counts: RegistryCounts::default(),
        })
    }
    pub(crate) const fn generation(&self) -> RegistryGeneration {
        self.generation
    }
    pub(crate) const fn counts(&self) -> RegistryCounts {
        self.counts
    }
    pub(crate) fn revision(
        &self,
        id: ModelRevisionId,
        work: &mut WorkMeter,
    ) -> RegistryResult<Option<RegisteredRevision>> {
        let index = self.revision_index.find(key(id.0), work)?;
        Ok(index.map(|index| self.revisions[index].expect("registered prefix")))
    }
    pub(crate) fn resolve_alias(
        &self,
        alias: ModelAliasId,
        work: &mut WorkMeter,
    ) -> RegistryResult<Option<RegisteredRevision>> {
        let index = self.alias_index.find(key(alias.0), work)?;
        Ok(index.map(|index| self.revisions[index].expect("registered prefix")))
    }
    pub(crate) fn prepare(
        &self,
        expected: RegistryGeneration,
        command: RegistryCommand,
        work: &mut WorkMeter,
    ) -> RegistryResult<RegistryChange> {
        work.record(InvariantChecks, 2)?;
        if expected != self.generation {
            return Err(RegistryError::Generation);
        }
        self.generation.next()?;
        let (delta, after) = match command {
            Register(model, revision, manifest) => {
                work.record(InvariantChecks, 2)?;
                if self.revision_index.find(key(revision.0), work)?.is_some() {
                    return Err(RegistryError::RevisionExists);
                }
                if self.counts.registered as usize == R {
                    return Err(RegistryError::RegistryLimit);
                }
                let mut after = self.counts;
                after.registered += 1;
                after.available += 1;
                let record = RegisteredRevision {
                    model,
                    revision,
                    manifest,
                    lifecycle: Available,
                };
                (PreparedDelta::Register(record), after)
            }
            BindAlias(alias, revision) => {
                work.record(InvariantChecks, 4)?;
                if self.alias_index.find(key(alias.0), work)?.is_some() {
                    return Err(RegistryError::AliasFrozen);
                }
                if self.counts.aliases as usize == A {
                    return Err(RegistryError::AliasLimit);
                }
                let found = self.revision_index.find(key(revision.0), work)?;
                let index = found.ok_or(RegistryError::UnknownRevision)?;
                if self.revisions[index].expect("registered prefix").lifecycle != Available {
                    return Err(RegistryError::InvalidLifecycle);
                }
                let mut after = self.counts;
                after.aliases += 1;
                (PreparedDelta::BindAlias((alias, index)), after)
            }
            Retire(revision) | MarkUnavailable(revision) => {
                work.record(InvariantChecks, 4)?;
                let found = self.revision_index.find(key(revision.0), work)?;
                let index = found.ok_or(RegistryError::UnknownRevision)?;
                let record = self.revisions[index].expect("registered prefix");
                let lifecycle = match (record.lifecycle, command) {
                    (Available, Retire(..)) => Retiring,
                    (Available | Retiring, MarkUnavailable(..)) => Unavailable,
                    _ => return Err(RegistryError::InvalidLifecycle),
                };
                let mut after = self.counts;
                let prior = lifecycle_count(&mut after, record.lifecycle);
                *prior = prior
                    .checked_sub(1)
                    .ok_or(RegistryError::PreparedChangeStale)?;
                *lifecycle_count(&mut after, lifecycle) += 1;
                (PreparedDelta::SetLifecycle(index, record, lifecycle), after)
            }
        };
        let indexed = u64::from(!matches!(delta, PreparedDelta::SetLifecycle(..)));
        work.record(VisitedEntities, indexed * INDEX_COMMIT_WORK.0)?;
        let copied = std::mem::size_of::<RegistryChange>() as u64 + indexed * INDEX_COMMIT_WORK.1;
        work.record(CopiedBytes, copied)?;
        work.record(InvariantChecks, 6 + indexed * 3)?;
        Ok(RegistryChange {
            expected,
            before: self.counts,
            after,
            delta,
        })
    }
    pub(crate) fn validate(&self, change: &RegistryChange) -> RegistryResult<()> {
        let target = match change.delta {
            PreparedDelta::Register(_) => {
                self.revisions.get(change.before.registered as usize) == Some(&None)
            }
            PreparedDelta::BindAlias(_) => self.alias_index.len() == change.before.aliases as usize,
            PreparedDelta::SetLifecycle(index, before, _) => {
                self.revisions.get(index) == Some(&Some(before))
            }
        };
        (self.generation == change.expected && self.counts == change.before && target)
            .then_some(())
            .ok_or(RegistryError::PreparedChangeStale)
    }
    pub(crate) fn commit(&mut self, change: RegistryChange) -> RegistryResult<RegistryGeneration> {
        self.validate(&change)?;
        let next = change.expected.next()?;
        match change.delta {
            PreparedDelta::Register(record) => {
                let index = change.before.registered as usize;
                insert_index(&mut self.revision_index, key(record.revision.0), index)?;
                self.revisions[index] = Some(record);
            }
            PreparedDelta::BindAlias(binding) => {
                insert_index(&mut self.alias_index, key(binding.0.0), binding.1)?
            }
            PreparedDelta::SetLifecycle(index, _, lifecycle) => {
                let record = self.revisions[index].as_mut().expect("prepared revision");
                record.lifecycle = lifecycle;
            }
        }
        self.counts = change.after;
        self.generation = next;
        Ok(next)
    }
}
type RegistryIndex = FixedIdentityIndex<usize>;
fn insert_index(index: &mut RegistryIndex, key: [u8; 33], value: usize) -> RegistryResult<()> {
    index
        .try_insert_sorted(&[(key, value)], &mut full_work())
        .map_err(|_| RegistryError::PreparedChangeStale)
}
fn lifecycle_count(counts: &mut RegistryCounts, lifecycle: RevisionLifecycle) -> &mut u32 {
    match lifecycle {
        Available => &mut counts.available,
        Retiring => &mut counts.retiring,
        Unavailable => &mut counts.unavailable,
    }
}
fn key(digest: [u8; 32]) -> [u8; 33] {
    let mut key = [0; 33];
    key[..32].copy_from_slice(&digest);
    key
}
fn full_work() -> WorkMeter {
    WorkMeter::new(HotPathWorkBudget::binary_maximum())
}
#[cfg(test)]
mod tests {
    use super::*;
    use RegistryError::*;
    type Reg<const R: usize, const A: usize> = ModelRegistry<R, A>;
    type Change = RegistryResult<RegistryChange>;
    fn rev(value: u8) -> ModelRevisionId {
        ModelRevisionId::new([value; 32]).unwrap()
    }
    fn man(value: u8) -> ModelManifestId {
        ModelManifestId::new([value + 10; 32]).unwrap()
    }
    fn name(value: u8) -> ModelAliasId {
        ModelAliasId::new([value + 20; 32]).unwrap()
    }
    fn registration(value: u8) -> RegistryCommand {
        Register(ModelId::new(value as u128).unwrap(), rev(value), man(value))
    }
    fn prep<const R: usize, const A: usize>(r: &Reg<R, A>, c: RegistryCommand) -> Change {
        r.prepare(r.generation(), c, &mut full_work())
    }
    fn measured(r: &Reg<8, 8>, c: RegistryCommand, expected: [u64; 5]) -> Change {
        let mut work = full_work();
        let result = r.prepare(r.generation(), c, &mut work);
        assert_eq!(work.witness(), crate::HotPathWorkWitness::new(expected));
        result
    }
    fn bad<const R: usize, const A: usize>(r: &Reg<R, A>, c: RegistryCommand) -> RegistryError {
        prep(r, c).unwrap_err()
    }
    fn apply<const R: usize, const A: usize>(r: &mut Reg<R, A>, c: RegistryCommand) {
        r.commit(prep(r, c).unwrap()).unwrap();
    }
    #[test]
    fn registry_contract() {
        let mut r = ModelRegistry::<2, 2>::try_new(RegistryGeneration(1)).unwrap();
        let change = prep(&r, registration(2)).unwrap();
        assert_eq!(r.counts(), RegistryCounts::default());
        r.validate(&change).unwrap();
        r.commit(change).unwrap();
        assert_eq!(r.revisions[0].unwrap().manifest, man(2));
        let duplicate = Register(ModelId::new(9).unwrap(), rev(2), man(9));
        assert_eq!(bad(&r, duplicate), RevisionExists);
        apply(&mut r, registration(1));
        assert_eq!(bad(&r, registration(3)), RegistryLimit);
        apply(&mut r, BindAlias(name(2), rev(1)));
        for target in [rev(1), rev(2)] {
            assert_eq!(bad(&r, BindAlias(name(2), target)), AliasFrozen);
        }
        let before = r.counts();
        let change = prep(&r, Retire(rev(1))).unwrap();
        assert_eq!(r.counts(), before);
        r.commit(change).unwrap();
        let alias = r.resolve_alias(name(2), &mut full_work()).unwrap().unwrap();
        assert_eq!(alias.lifecycle, Retiring);
        assert_eq!(bad(&r, BindAlias(name(1), rev(1))), InvalidLifecycle);
        apply(&mut r, MarkUnavailable(rev(1)));
        for command in [Retire(rev(1)), MarkUnavailable(rev(1))] {
            assert_eq!(bad(&r, command), InvalidLifecycle);
        }
        apply(&mut r, BindAlias(name(1), rev(2)));
        assert_eq!(bad(&r, BindAlias(name(3), rev(1))), AliasLimit);
        let alias = r.resolve_alias(name(2), &mut full_work()).unwrap().unwrap();
        assert_eq!(alias.lifecycle, Unavailable);
        let c = r.counts();
        assert_eq!([c.registered, c.available, c.aliases], [2, 1, 2]);
        assert_eq!((c.retiring, c.unavailable), (0, 1));
    }
    #[test]
    fn failures_preserve_state() {
        assert_eq!(RegistryGeneration::new(0), Err(RegistryError::InvalidInput));
        let mut r = ModelRegistry::<2, 2>::try_new(RegistryGeneration(1)).unwrap();
        let first = prep(&r, registration(1)).unwrap();
        let stale = prep(&r, registration(2)).unwrap();
        r.commit(first).unwrap();
        let before = (r.generation(), r.counts());
        let rejected = r.prepare(RegistryGeneration(1), registration(2), &mut full_work());
        assert_eq!(rejected.unwrap_err(), Generation);
        assert_eq!(r.validate(&stale), Err(PreparedChangeStale));
        assert_eq!(r.commit(stale), Err(PreparedChangeStale));
        let mut work = full_work();
        work.record(CopiedBytes, 2_097_152).unwrap();
        let command = registration(2);
        let error = r.prepare(r.generation(), command, &mut work).unwrap_err();
        assert!(matches!(error, RegistryError::Work(_)));
        assert_eq!((r.generation(), r.counts()), before);
        assert_eq!(bad(&r, Retire(rev(2))), UnknownRevision);
    }
    #[test]
    fn transition_work_is_exact_at_registry_cardinality() {
        let mut r = ModelRegistry::<8, 8>::try_new(RegistryGeneration(1)).unwrap();
        for value in 1..8 {
            apply(&mut r, registration(value));
            apply(&mut r, BindAlias(name(value), rev(value)));
        }
        let commands = [registration(8), BindAlias(name(8), rev(8)), Retire(rev(8))];
        let errors = [RevisionExists, AliasFrozen, InvalidLifecycle];
        let w = |visits, bytes, checks| [visits, bytes, 0, 0, checks];
        let ok = [w(830, 416, 13), w(833, 416, 15), w(2, 160, 12)];
        let no = [w(2, 0, 4), w(3, 0, 6), w(2, 0, 6)];
        let cases = commands.into_iter().zip(errors).zip(ok).zip(no);
        for (((command, error), success), rejected) in cases {
            let change = measured(&r, command, success).unwrap();
            r.commit(change).unwrap();
            let before = (r.generation(), r.counts());
            assert_eq!(measured(&r, command, rejected).unwrap_err(), error);
            assert_eq!((r.generation(), r.counts()), before);
        }
    }
}
