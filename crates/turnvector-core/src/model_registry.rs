#![allow(dead_code, reason = "C10e wires registration; later rows wire more")]
use crate::WorkDimension::{CopiedBytes, InvariantChecks, VisitedEntities};
use crate::model_descriptor::{
    MAX_FRAME_BYTES, ModelDescriptorHash, ModelDescriptorId, VerifiedModelDescriptor,
};
use crate::{
    FixedIdentityIndex, FixedIndexError, HotPathWorkBudget, ModelId, TokenCount, WorkBudgetError,
    WorkMeter,
};
const INDEX_COMMIT_WORK: (u64, u64) = (827, 256); // Fixed-index insert lookup and path.
pub(crate) const MODEL_REGISTRY_LIMIT: usize = 256;
pub(crate) const DESCRIPTOR_ARENA_LIMIT: usize = MODEL_REGISTRY_LIMIT * MAX_FRAME_BYTES;
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
    pub(crate) context_limit: TokenCount,
    pub(crate) lifecycle: RevisionLifecycle,
    descriptor: RetainedDescriptor,
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct RegistryCounts {
    pub(crate) registered: u32,
    pub(crate) available: u32,
    pub(crate) retiring: u32,
    pub(crate) unavailable: u32,
    pub(crate) aliases: u32,
    pub(crate) descriptor_bytes: u32,
}
#[rustfmt::skip]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RegistrationIntent { pub(crate) model: ModelId, pub(crate) revision: ModelRevisionId, pub(crate) manifest: ModelManifestId, pub(crate) expected_descriptor_hash: ModelDescriptorHash, pub(crate) context_limit: TokenCount }
#[rustfmt::skip]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct DescriptionPlan { expected: RegistryGeneration, before: RegistryCounts, intent: RegistrationIntent }
type RetainedDescriptor = (u32, u16, ModelDescriptorId, ModelDescriptorHash, u32);
#[rustfmt::skip]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RegisteredDescriptor<'a> { frame: &'a [u8], id: ModelDescriptorId, hash: ModelDescriptorHash, vocabulary: u32 }
impl RegisteredDescriptor<'_> {
    #[rustfmt::skip]
    pub(crate) const fn values(&self) -> (&[u8], ModelDescriptorId, ModelDescriptorHash, u32) { (self.frame, self.id, self.hash, self.vocabulary) }
    #[rustfmt::skip]
    pub(crate) fn exactly_matches(&self, descriptor: &VerifiedModelDescriptor, work: &mut WorkMeter) -> RegistryResult<bool> { work.ensure(crate::HotPathWorkWitness::new([self.frame.len() as u64, 0, 0, 0, 4]))?; work.record(InvariantChecks, 4)?; work.record(VisitedEntities, self.frame.len() as u64)?; Ok(self.frame == descriptor.frame() && self.id == descriptor.id() && self.hash == descriptor.hash() && self.vocabulary == descriptor.vocabulary()) }
}
#[rustfmt::skip]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RevisionSelection { Direct(ModelRevisionId), Alias(ModelAliasId) }
#[rustfmt::skip]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RequestRevision { generation: RegistryGeneration, selection: RevisionSelection, record: RegisteredRevision, vocabulary: u32 }
#[rustfmt::skip]
impl RequestRevision {
    pub(crate) const fn generation(self) -> RegistryGeneration { self.generation }
    pub(crate) const fn selection(self) -> RevisionSelection { self.selection }
    pub(crate) const fn revision(self) -> ModelRevisionId { self.record.revision }
    pub(crate) const fn lifecycle(self) -> RevisionLifecycle { self.record.lifecycle }
    pub(crate) const fn vocabulary(self) -> u32 { self.vocabulary }
    pub(crate) const fn context_limit(self) -> TokenCount { self.record.context_limit }
}
values! {
    pub(crate) enum RevisionLifecycle { Available, Retiring, Unavailable }
    pub(crate) enum RegistryError {
        InvalidInput, Generation, RegistryLimit, DescriptorArenaLimit, DescriptorMismatch,
        AliasLimit, RevisionExists, UnknownRevision, AliasFrozen, InvalidLifecycle,
        PreparedChangeStale, Index(FixedIndexError), Work(WorkBudgetError),
    }
    pub(crate) enum RegistryCommand {
        BindAlias(ModelAliasId, ModelRevisionId),
        Retire(ModelRevisionId), MarkUnavailable(ModelRevisionId),
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PreparedDelta<'a> {
    Register(RegistrationIntent, &'a VerifiedModelDescriptor),
    BindAlias((ModelAliasId, usize)),
    SetLifecycle(usize, RegisteredRevision, RevisionLifecycle),
}
use RegistryCommand::{BindAlias, MarkUnavailable, Retire};
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
pub(crate) struct RegistryChange<'a> {
    expected: RegistryGeneration,
    before: RegistryCounts,
    after: RegistryCounts,
    delta: PreparedDelta<'a>,
}
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct ModelRegistry<
    const REVISIONS: usize,
    const ALIASES: usize,
    const DESCRIPTOR_BYTES: usize = DESCRIPTOR_ARENA_LIMIT,
> {
    generation: RegistryGeneration,
    revisions: [Option<RegisteredRevision>; REVISIONS],
    revision_index: FixedIdentityIndex<usize>,
    alias_index: FixedIdentityIndex<usize>,
    descriptor_arena: Vec<u8>,
    counts: RegistryCounts,
}
impl<const R: usize, const A: usize, const D: usize> ModelRegistry<R, A, D> {
    #[cfg(test)]
    #[rustfmt::skip]
    pub(crate) fn try_new(generation: RegistryGeneration) -> RegistryResult<Self> { Self::try_new_with_limits(generation) }
    fn try_new_with_limits(generation: RegistryGeneration) -> RegistryResult<Self> {
        if R == 0 || R > MODEL_REGISTRY_LIMIT || A == 0 || D == 0 || D > DESCRIPTOR_ARENA_LIMIT {
            return Err(RegistryError::InvalidInput);
        }
        let mut descriptor_arena = Vec::new();
        descriptor_arena
            .try_reserve_exact(D)
            .map_err(|_| RegistryError::InvalidInput)?;
        Ok(Self {
            generation,
            revisions: [None; R],
            revision_index: FixedIdentityIndex::try_new(R)?,
            alias_index: FixedIdentityIndex::try_new(A)?,
            descriptor_arena,
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
    pub(crate) fn descriptor(
        &self,
        id: ModelRevisionId,
        work: &mut WorkMeter,
    ) -> RegistryResult<Option<RegisteredDescriptor<'_>>> {
        let Some(index) = self.revision_index.find(key(id.0), work)? else {
            return Ok(None);
        };
        let (offset, length, id, hash, vocabulary) =
            self.revisions[index].expect("registered prefix").descriptor;
        let start = offset as usize;
        let end = start + usize::from(length);
        let descriptor = RegisteredDescriptor {
            frame: &self.descriptor_arena[start..end],
            id,
            hash,
            vocabulary,
        };
        work.record(CopiedBytes, std::mem::size_of_val(&descriptor) as u64)?;
        Ok(Some(descriptor))
    }
    #[rustfmt::skip]
    pub(crate) fn request_revision_fact(&self, expected: RegistryGeneration, selection: RevisionSelection, work: &mut WorkMeter) -> RegistryResult<Option<RequestRevision>> {
        work.record(InvariantChecks, 1)?;
        if expected != self.generation { return Err(RegistryError::Generation); }
        let record = match selection {
            RevisionSelection::Direct(revision) => self.revision(revision, work)?,
            RevisionSelection::Alias(alias) => self.resolve_alias(alias, work)?,
        };
        let Some(record) = record else { return Ok(None); };
        let fact = RequestRevision { generation: expected, selection, record, vocabulary: record.descriptor.4 };
        work.record(CopiedBytes, std::mem::size_of_val(&fact) as u64)?;
        Ok(Some(fact))
    }
    #[rustfmt::skip]
    pub(crate) fn validate_request_revision(&self, fact: RequestRevision) -> RegistryResult<()> { (self.generation == fact.generation()).then_some(()).ok_or(RegistryError::Generation) }
    pub(crate) fn prepare_description(
        &self,
        expected: RegistryGeneration,
        intent: RegistrationIntent,
        work: &mut WorkMeter,
    ) -> RegistryResult<DescriptionPlan> {
        work.record(InvariantChecks, 5)?;
        if expected != self.generation {
            return Err(RegistryError::Generation);
        }
        self.generation.next()?;
        if intent.context_limit.get() == 0 {
            return Err(RegistryError::InvalidInput);
        }
        if self
            .revision_index
            .find(key(intent.revision.0), work)?
            .is_some()
        {
            return Err(RegistryError::RevisionExists);
        }
        if self.counts.registered as usize == R {
            return Err(RegistryError::RegistryLimit);
        }
        work.record(CopiedBytes, std::mem::size_of::<DescriptionPlan>() as u64)?;
        Ok(DescriptionPlan {
            expected,
            before: self.counts,
            intent,
        })
    }
    pub(crate) fn prepare_registration<'a>(
        &self,
        plan: DescriptionPlan,
        descriptor: &'a VerifiedModelDescriptor,
        work: &mut WorkMeter,
    ) -> RegistryResult<RegistryChange<'a>> {
        work.record(InvariantChecks, 6)?;
        if plan.expected != self.generation || plan.before != self.counts {
            return Err(RegistryError::PreparedChangeStale);
        }
        if descriptor.hash() != plan.intent.expected_descriptor_hash {
            return Err(RegistryError::DescriptorMismatch);
        }
        let frame_len = descriptor.frame().len();
        let descriptor_bytes = usize::try_from(self.counts.descriptor_bytes)
            .ok()
            .and_then(|used| used.checked_add(frame_len))
            .filter(|end| *end <= D)
            .ok_or(RegistryError::DescriptorArenaLimit)?;
        let mut after = self.counts;
        after.registered += 1;
        after.available += 1;
        after.descriptor_bytes = descriptor_bytes as u32;
        let copied = std::mem::size_of::<RegistryChange<'_>>() as u64
            + INDEX_COMMIT_WORK.1
            + frame_len as u64;
        let required = [INDEX_COMMIT_WORK.0, copied, 0, 0, 9];
        work.ensure(crate::HotPathWorkWitness::new(required))?;
        work.record(VisitedEntities, INDEX_COMMIT_WORK.0)?;
        work.record(CopiedBytes, copied)?;
        work.record(InvariantChecks, 9)?;
        Ok(RegistryChange {
            expected: plan.expected,
            before: plan.before,
            after,
            delta: PreparedDelta::Register(plan.intent, descriptor),
        })
    }
    pub(crate) fn prepare(
        &self,
        expected: RegistryGeneration,
        command: RegistryCommand,
        work: &mut WorkMeter,
    ) -> RegistryResult<RegistryChange<'static>> {
        work.record(InvariantChecks, 2)?;
        if expected != self.generation {
            return Err(RegistryError::Generation);
        }
        self.generation.next()?;
        let (delta, after) = match command {
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
        let indexed = u64::from(matches!(delta, PreparedDelta::BindAlias(..)));
        work.record(VisitedEntities, indexed * INDEX_COMMIT_WORK.0)?;
        let copied =
            std::mem::size_of::<RegistryChange<'_>>() as u64 + indexed * INDEX_COMMIT_WORK.1;
        work.record(CopiedBytes, copied)?;
        work.record(InvariantChecks, 6 + indexed * 3)?;
        Ok(RegistryChange {
            expected,
            before: self.counts,
            after,
            delta,
        })
    }
    pub(crate) fn validate(&self, change: &RegistryChange<'_>) -> RegistryResult<()> {
        let target = match &change.delta {
            PreparedDelta::Register(_, descriptor) => {
                self.revisions
                    .get(change.before.registered as usize)
                    .is_some_and(Option::is_none)
                    && self.descriptor_arena.len() == change.before.descriptor_bytes as usize
                    && self.descriptor_arena.len() + descriptor.frame().len() <= D
            }
            PreparedDelta::BindAlias(_) => self.alias_index.len() == change.before.aliases as usize,
            PreparedDelta::SetLifecycle(index, before, _) => {
                self.revisions.get(*index) == Some(&Some(*before))
            }
        };
        (self.generation == change.expected && self.counts == change.before && target)
            .then_some(())
            .ok_or(RegistryError::PreparedChangeStale)
    }
    pub(crate) fn commit(
        &mut self,
        change: RegistryChange<'_>,
    ) -> RegistryResult<RegistryGeneration> {
        self.validate(&change)?;
        let next = change.expected.next()?;
        match change.delta {
            PreparedDelta::Register(intent, descriptor) => {
                let index = change.before.registered as usize;
                insert_index(&mut self.revision_index, key(intent.revision.0), index)?;
                let offset = self.descriptor_arena.len();
                self.descriptor_arena.extend_from_slice(descriptor.frame());
                self.revisions[index] = Some(RegisteredRevision {
                    model: intent.model,
                    revision: intent.revision,
                    manifest: intent.manifest,
                    context_limit: intent.context_limit,
                    lifecycle: Available,
                    descriptor: (
                        offset as u32,
                        descriptor.frame().len() as u16,
                        descriptor.id(),
                        descriptor.hash(),
                        descriptor.vocabulary(),
                    ),
                });
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
#[cfg(not(test))]
impl<const A: usize> ModelRegistry<MODEL_REGISTRY_LIMIT, A> {
    #[rustfmt::skip]
    pub(crate) fn try_new(generation: RegistryGeneration) -> RegistryResult<Self> { Self::try_new_with_limits(generation) }
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
    use crate::TokenCount;
    use crate::model_descriptor::{ModelDescriptorHash, RawModelDescriptor, verify};
    use RegistryError::*;
    type Reg<const R: usize, const A: usize, const D: usize = DESCRIPTOR_ARENA_LIMIT> =
        ModelRegistry<R, A, D>;
    type Change<'a> = RegistryResult<RegistryChange<'a>>;
    const FRAME: [u8; 13] = [0, 0, 0, 1, 0, 0, 0, 7, 0, 0, 0, 1, b'x'];
    #[rustfmt::skip]
    const ID: [u8; 32] = [0xc9, 0x1c, 0x14, 0x09, 0x1c, 0xea, 0x08, 0xf4, 0x58, 0xa4, 0xe2, 0x75, 0x96, 0xc1, 0x5b, 0x2c, 0xf0, 0xc8, 0x74, 0x34, 0x2d, 0x30, 0x3e, 0xad, 0xe8, 0x9f, 0x29, 0x0e, 0xd0, 0x13, 0x38, 0x21];
    #[rustfmt::skip]
    const HASH: [u8; 32] = [0xe2, 0x24, 0x6d, 0x47, 0x7f, 0x70, 0xd3, 0xe6, 0x58, 0x8b, 0xb5, 0x45, 0xe2, 0x14, 0xc0, 0xbb, 0xa1, 0x76, 0x6e, 0xf3, 0x39, 0x7a, 0x50, 0x71, 0x89, 0x29, 0xc9, 0x4f, 0xe9, 0x62, 0x1e, 0x9b];
    const DRIFT_FRAME: [u8; 13] = [0, 0, 0, 1, 0, 0, 0, 7, 0, 0, 0, 1, b'y'];
    #[rustfmt::skip]
    const DRIFT_ID: [u8; 32] = [0xef, 0xdb, 0xdd, 0x9f, 0xda, 0xe3, 0x78, 0xc0, 0x52, 0x87, 0x4a, 0x59, 0x41, 0x3f, 0xb9, 0xdd, 0xa4, 0x63, 0xf2, 0xb7, 0x8f, 0x71, 0x4f, 0xc4, 0x2a, 0x87, 0x49, 0xf6, 0xf9, 0xc8, 0xb2, 0x34];
    #[rustfmt::skip]
    const DRIFT_HASH: [u8; 32] = [0x40, 0x5d, 0xa4, 0x3d, 0x2f, 0x31, 0xdf, 0x79, 0x2f, 0x49, 0x7f, 0x8d, 0x86, 0x85, 0xb3, 0x61, 0x1e, 0x6c, 0xa5, 0x1d, 0x19, 0x57, 0x70, 0xa0, 0x03, 0xd4, 0x74, 0xcb, 0x5e, 0x65, 0xde, 0x3c];
    fn rev(value: u8) -> ModelRevisionId {
        ModelRevisionId::new([value; 32]).unwrap()
    }
    fn man(value: u8) -> ModelManifestId {
        ModelManifestId::new([value + 10; 32]).unwrap()
    }
    fn name(value: u8) -> ModelAliasId {
        ModelAliasId::new([value + 20; 32]).unwrap()
    }
    #[rustfmt::skip]
    fn sealed(frame: &[u8], id: [u8; 32], hash: [u8; 32]) -> VerifiedModelDescriptor { let expected = ModelDescriptorHash::from_manifest(1, hash).unwrap(); verify(RawModelDescriptor { frame, id, hash_schema_version: 1, hash, vocabulary: 7 }, expected, &mut full_work()).unwrap() }
    #[rustfmt::skip]
    fn descriptor() -> VerifiedModelDescriptor { sealed(&FRAME, ID, HASH) }
    #[rustfmt::skip]
    fn drift_descriptor() -> VerifiedModelDescriptor { sealed(&DRIFT_FRAME, DRIFT_ID, DRIFT_HASH) }
    fn registration(value: u8) -> RegistrationIntent {
        RegistrationIntent {
            model: ModelId::new(value as u128).unwrap(),
            revision: rev(value),
            manifest: man(value),
            expected_descriptor_hash: descriptor().hash(),
            context_limit: TokenCount::new(8),
        }
    }
    #[rustfmt::skip]
    fn prep<const R: usize, const A: usize, const D: usize>(r: &Reg<R, A, D>, c: RegistryCommand) -> Change<'static> { r.prepare(r.generation(), c, &mut full_work()) }
    #[rustfmt::skip]
    fn prep_registration<'a, const R: usize, const A: usize, const D: usize>(r: &Reg<R, A, D>, intent: RegistrationIntent, descriptor: &'a VerifiedModelDescriptor) -> Change<'a> { let plan = r.prepare_description(r.generation(), intent, &mut full_work())?; r.prepare_registration(plan, descriptor, &mut full_work()) }
    fn measured(r: &Reg<8, 8>, c: RegistryCommand, expected: [u64; 5]) -> Change<'static> {
        let mut work = full_work();
        let result = r.prepare(r.generation(), c, &mut work);
        exact(&work, expected);
        result
    }
    #[rustfmt::skip]
    fn exact(work: &WorkMeter, expected: [u64; 5]) { assert_eq!(work.witness(), crate::HotPathWorkWitness::new(expected)); }
    #[rustfmt::skip]
    fn descriptor_match(stored: &RegisteredDescriptor<'_>, descriptor: &VerifiedModelDescriptor) -> bool { let mut work = full_work(); let result = stored.exactly_matches(descriptor, &mut work).unwrap(); exact(&work, [13, 0, 0, 0, 4]); result }
    #[rustfmt::skip]
    fn bad<const R: usize, const A: usize, const D: usize>(r: &Reg<R, A, D>, c: RegistryCommand) -> RegistryError { prep(r, c).unwrap_err() }
    #[rustfmt::skip]
    fn apply<const R: usize, const A: usize, const D: usize>(r: &mut Reg<R, A, D>, c: RegistryCommand) { r.commit(prep(r, c).unwrap()).unwrap(); }
    #[rustfmt::skip]
    fn apply_registration<const R: usize, const A: usize, const D: usize>(r: &mut Reg<R, A, D>, intent: RegistrationIntent) { let descriptor = descriptor(); r.commit(prep_registration(r, intent, &descriptor).unwrap()).unwrap(); }
    #[rustfmt::skip]
    fn description_error<const R: usize, const A: usize, const D: usize>(r: &Reg<R, A, D>, intent: RegistrationIntent, error: RegistryError, witness: [u64; 5]) { let mut work = full_work(); assert_eq!(r.prepare_description(r.generation(), intent, &mut work).unwrap_err(), error); assert_eq!(work.witness(), crate::HotPathWorkWitness::new(witness)); }
    #[test]
    #[rustfmt::skip]
    fn registry_contract() {
        let mut r = ModelRegistry::<2, 2>::try_new(RegistryGeneration(1)).unwrap();
        let sealed = descriptor();
        let plan = r
            .prepare_description(r.generation(), registration(2), &mut full_work())
            .unwrap();
        assert_eq!(r.counts(), RegistryCounts::default());
        let change = r
            .prepare_registration(plan, &sealed, &mut full_work())
            .unwrap();
        r.validate(&change).unwrap();
        r.commit(change).unwrap();
        assert_eq!((r.revisions[0].unwrap().manifest, r.revisions[0].unwrap().context_limit), (man(2), TokenCount::new(8)));
        let fact = r
            .request_revision_fact(
                r.generation(),
                RevisionSelection::Direct(rev(2)),
                &mut full_work(),
            )
            .unwrap()
            .unwrap();
        assert_eq!((fact.selection(), fact.revision(), fact.lifecycle(), fact.vocabulary(), fact.context_limit()), (RevisionSelection::Direct(rev(2)), rev(2), Available, 7, TokenCount::new(8)));
        let mut read_work = full_work();
        let retained = r.descriptor(rev(2), &mut read_work).unwrap().unwrap();
        exact(&read_work, [1, 88, 0, 0, 0]);
        assert!(descriptor_match(&retained, &sealed));
        assert!(!descriptor_match(&retained, &drift_descriptor()));
        assert_eq!(
            retained.values(),
            (sealed.frame(), sealed.id(), sealed.hash(), 7)
        );
        let duplicate = RegistrationIntent {
            model: ModelId::new(9).unwrap(),
            ..registration(2)
        };
        description_error(&r, duplicate, RevisionExists, [1, 0, 0, 0, 5]);
        apply_registration(&mut r, registration(1));
        description_error(&r, registration(3), RegistryLimit, [2, 0, 0, 0, 5]);
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
    #[rustfmt::skip]
    fn failures_preserve_state() {
        assert_eq!(RegistryGeneration::new(0), Err(RegistryError::InvalidInput));
        let mut r = ModelRegistry::<2, 2>::try_new(RegistryGeneration(1)).unwrap(); let descriptor = descriptor(); let first = prep_registration(&r, registration(1), &descriptor).unwrap(); let stale = prep_registration(&r, registration(2), &descriptor).unwrap(); r.commit(first).unwrap();
        let fact = r.request_revision_fact(r.generation(), RevisionSelection::Direct(rev(1)), &mut full_work()).unwrap().unwrap(); apply(&mut r, BindAlias(name(1), rev(1))); assert_eq!(r.validate_request_revision(fact), Err(Generation));
        let mut invalid = registration(2); invalid.context_limit = TokenCount::new(0); description_error(&r, invalid, InvalidInput, [0, 0, 0, 0, 5]);
        let retained = r.descriptor(rev(1), &mut full_work()).unwrap().unwrap(); let mut equality_work = full_work(); equality_work.record(VisitedEntities, 1_704_575).unwrap(); assert_eq!(retained.exactly_matches(&descriptor, &mut equality_work).unwrap_err(), Work(WorkBudgetError::BudgetExceeded(VisitedEntities, 1_704_575, 1_704_588))); exact(&equality_work, [1_704_575, 0, 0, 0, 0]);
        let before = (r.generation(), r.counts());
        let mut generation_work = full_work(); assert_eq!(r.prepare_description(RegistryGeneration(1), registration(2), &mut generation_work).unwrap_err(), Generation); assert_eq!(generation_work.witness(), crate::HotPathWorkWitness::new([0, 0, 0, 0, 5]));
        assert_eq!(r.validate(&stale), Err(PreparedChangeStale)); assert_eq!(r.commit(stale), Err(PreparedChangeStale));
        let plan = r.prepare_description(r.generation(), registration(2), &mut full_work()).unwrap(); let mut work = full_work(); work.record(CopiedBytes, 2_097_152).unwrap(); assert_eq!(r.prepare_registration(plan, &descriptor, &mut work).unwrap_err(), Work(WorkBudgetError::BudgetExceeded(CopiedBytes, 2_097_152, 2_097_677))); assert_eq!(work.witness(), crate::HotPathWorkWitness::new([0, 2_097_152, 0, 0, 6]));
        assert_eq!((r.generation(), r.counts()), before);
        assert_eq!(bad(&r, Retire(rev(2))), UnknownRevision);
    }
    #[test]
    fn transition_work_is_exact_at_registry_cardinality() {
        let mut r = ModelRegistry::<8, 8>::try_new(RegistryGeneration(1)).unwrap();
        for value in 1..8 {
            apply_registration(&mut r, registration(value));
            apply(&mut r, BindAlias(name(value), rev(value)));
        }
        let mut plan_work = full_work();
        let plan = r
            .prepare_description(r.generation(), registration(8), &mut plan_work)
            .unwrap();
        exact(&plan_work, [3, 160, 0, 0, 5]);
        let mut registration_work = full_work();
        let descriptor = descriptor();
        let change = r
            .prepare_registration(plan, &descriptor, &mut registration_work)
            .unwrap();
        exact(&registration_work, [827, 525, 0, 0, 15]);
        r.commit(change).unwrap();
        let commands = [BindAlias(name(8), rev(8)), Retire(rev(8))];
        let errors = [AliasFrozen, InvalidLifecycle];
        let w = |visits, bytes, checks| [visits, bytes, 0, 0, checks];
        let ok = [w(833, 512, 15), w(2, 256, 12)];
        let no = [w(3, 0, 6), w(2, 0, 6)];
        let cases = commands.into_iter().zip(errors).zip(ok).zip(no);
        for (((command, error), success), rejected) in cases {
            let change = measured(&r, command, success).unwrap();
            r.commit(change).unwrap();
            let before = (r.generation(), r.counts());
            assert_eq!(measured(&r, command, rejected).unwrap_err(), error);
            assert_eq!((r.generation(), r.counts()), before);
        }
    }
    #[test]
    #[rustfmt::skip]
    fn registration_plan_and_arena_edges_are_atomic() {
        assert_eq!((MODEL_REGISTRY_LIMIT, DESCRIPTOR_ARENA_LIMIT), (256, 4_194_304)); assert!(ModelRegistry::<MODEL_REGISTRY_LIMIT, 1>::try_new(RegistryGeneration(1)).is_ok()); assert!(matches!(ModelRegistry::<257, 1, 1>::try_new(RegistryGeneration(1)), Err(InvalidInput)));
        let descriptor = descriptor(); let mut stale = ModelRegistry::<2, 1, 26>::try_new(RegistryGeneration(1)).unwrap(); let plan = stale.prepare_description(stale.generation(), registration(2), &mut full_work()).unwrap(); apply_registration(&mut stale, registration(1)); let before = (stale.generation(), stale.counts()); let mut work = full_work(); assert_eq!(stale.prepare_registration(plan, &descriptor, &mut work).unwrap_err(), PreparedChangeStale); assert_eq!(work.witness(), crate::HotPathWorkWitness::new([0, 0, 0, 0, 6])); assert_eq!((stale.generation(), stale.counts()), before);
        let mut exact_arena = ModelRegistry::<3, 1, 39>::try_new(RegistryGeneration(1)).unwrap(); for value in 1..=3 { apply_registration(&mut exact_arena, registration(value)); } assert_eq!(exact_arena.counts().descriptor_bytes, 39); let mut arena = ModelRegistry::<3, 1, 38>::try_new(RegistryGeneration(1)).unwrap(); for value in 1..=2 { apply_registration(&mut arena, registration(value)); } let before = (arena.generation(), arena.counts()); let plan = arena.prepare_description(arena.generation(), registration(3), &mut full_work()).unwrap(); let mut work = full_work(); assert_eq!(arena.prepare_registration(plan, &descriptor, &mut work).unwrap_err(), DescriptorArenaLimit); assert_eq!(work.witness(), crate::HotPathWorkWitness::new([0, 0, 0, 0, 6])); assert_eq!((arena.generation(), arena.counts()), before);
        let registry = ModelRegistry::<1, 1, 13>::try_new(RegistryGeneration(1)).unwrap(); let mut intent = registration(1); intent.expected_descriptor_hash = ModelDescriptorHash::from_manifest(1, [0; 32]).unwrap(); let plan = registry.prepare_description(registry.generation(), intent, &mut full_work()).unwrap(); let mut work = full_work(); assert_eq!(registry.prepare_registration(plan, &descriptor, &mut work).unwrap_err(), DescriptorMismatch); assert_eq!(work.witness(), crate::HotPathWorkWitness::new([0, 0, 0, 0, 6])); assert_eq!(registry.counts(), RegistryCounts::default());
    }
}
