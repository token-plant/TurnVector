#![allow(dead_code, reason = "C14 consumes the exact-key closure")]
use crate::WorkDimension::{CopiedBytes, InvariantChecks, VisitedEntities};
use crate::model_registry::ModelRevisionId;
use crate::request_book::{CapabilityRequirement, RequestDescriptionFacts};
use crate::{
    BackendGeneration, BoundedVec, CapabilityKey, HotPathWorkWitness, WorkBudgetError, WorkMeter,
};
use std::{cmp::Ordering, mem::size_of};
type CertificationResult<T> = Result<T, CertificationError>;
macro_rules! identities {
    ($($name:ident),+ $(,)?) => {$(
        #[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
        pub(crate) struct $name([u8; 32]);
        impl $name {
            pub(crate) fn try_new(bytes: [u8; 32]) -> CertificationResult<Self> {
                (bytes != [0; 32]).then_some(Self(bytes)).ok_or(CertificationError::InvalidIdentity)
            }
        }
    )+};
}
identities!(
    CertificationEnvelopeId,
    CertificationRecordId,
    EnvironmentQualificationId,
    CaseBoundTableId,
    GenerationHash,
    CertificationAuthorizationIndexId
);
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct EnvironmentIdentity {
    device: [u8; 32],
    gpu: [u8; 32],
    unified_memory_bytes: u64,
    os_build: [u8; 32],
    daemon_build: [u8; 32],
    adapter_build: [u8; 32],
    mlx_build: [u8; 32],
    backend_interface: u32,
    bootstrap_manifest: [u8; 32],
    backend_capabilities: [u8; 32],
    generation_semantics: [u8; 32],
    resource_signal: [u8; 32],
    operation_bounds: [u8; 32],
}
impl EnvironmentIdentity {
    fn valid(self) -> bool {
        self.unified_memory_bytes != 0
            && self.backend_interface != 0
            && [
                self.device,
                self.gpu,
                self.os_build,
                self.daemon_build,
                self.adapter_build,
                self.mlx_build,
                self.bootstrap_manifest,
                self.backend_capabilities,
                self.generation_semantics,
                self.resource_signal,
                self.operation_bounds,
            ]
            .iter()
            .all(|identity| *identity != [0; 32])
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct EnvironmentQualification {
    identity: EnvironmentQualificationId,
    environment: EnvironmentIdentity,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct EnvironmentFingerprint {
    identity: EnvironmentIdentity,
    fresh: bool,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ApplicabilityEvidence {
    generation: GenerationHash,
    index: CertificationAuthorizationIndexId,
    environment: EnvironmentFingerprint,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ExactCapabilityKey {
    identity: CapabilityKey,
    revision: ModelRevisionId,
    requirement: CapabilityRequirement,
    envelope: CertificationEnvelopeId,
}
impl ExactCapabilityKey {
    pub(crate) fn try_new(
        identity: CapabilityKey,
        revision: ModelRevisionId,
        requirement: CapabilityRequirement,
        envelope: CertificationEnvelopeId,
    ) -> CertificationResult<Self> {
        let valid = identity.0 != [0; 32]
            && requirement.batch.0 != 0
            && requirement.shape != 0
            && requirement.route != [0; 32]
            && requirement.adapter_build != [0; 32]
            && requirement.mlx_build != [0; 32]
            && requirement.backend_interface != 0;
        valid
            .then_some(Self {
                identity,
                revision,
                requirement,
                envelope,
            })
            .ok_or(CertificationError::InvalidIdentity)
    }
    fn requirement_order(
        self,
        revision: ModelRevisionId,
        requirement: CapabilityRequirement,
    ) -> Ordering {
        let phase = |value| match value {
            crate::ExecutionPhase::Prefill => 0,
            crate::ExecutionPhase::Decode => 1,
        };
        self.revision
            .cmp(&revision)
            .then_with(|| phase(self.requirement.phase).cmp(&phase(requirement.phase)))
            .then_with(|| self.requirement.batch.cmp(&requirement.batch))
            .then_with(|| self.requirement.shape.cmp(&requirement.shape))
            .then_with(|| self.requirement.route.cmp(&requirement.route))
            .then_with(|| {
                self.requirement
                    .adapter_build
                    .cmp(&requirement.adapter_build)
            })
            .then_with(|| self.requirement.mlx_build.cmp(&requirement.mlx_build))
            .then_with(|| {
                self.requirement
                    .backend_interface
                    .cmp(&requirement.backend_interface)
            })
    }
    fn canonical_order(self, other: Self) -> Ordering {
        self.requirement_order(other.revision, other.requirement)
            .then_with(|| self.identity.cmp(&other.identity))
    }
}
fn binary_find(
    len: usize,
    mut order: impl FnMut(usize) -> Ordering,
    work: &mut WorkMeter,
) -> CertificationResult<Option<usize>> {
    let (mut low, mut high) = (0, len);
    while low < high {
        let middle = low + (high - low) / 2;
        work.record(VisitedEntities, 1)?;
        work.record(InvariantChecks, 1)?;
        match order(middle) {
            Ordering::Less => low = middle + 1,
            Ordering::Equal => return Ok(Some(middle)),
            Ordering::Greater => high = middle,
        }
    }
    Ok(None)
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CertificationRecord {
    pub(crate) identity: CertificationRecordId,
    pub(crate) envelope: CertificationEnvelopeId,
    pub(crate) environment: EnvironmentQualificationId,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CertifiedExecutionProfile {
    pub(crate) key: ExactCapabilityKey,
    pub(crate) record: CertificationRecordId,
    pub(crate) case_bounds: CaseBoundTableId,
}
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct CertificationAuthorizationIndex<const RECORDS: usize, const PROFILES: usize> {
    generation: GenerationHash,
    identity: CertificationAuthorizationIndexId,
    environments: BoundedVec<EnvironmentQualification, RECORDS>,
    records: BoundedVec<CertificationRecord, RECORDS>,
    profiles: BoundedVec<CertifiedExecutionProfile, PROFILES>,
}
impl<const RECORDS: usize, const PROFILES: usize>
    CertificationAuthorizationIndex<RECORDS, PROFILES>
{
    pub(crate) fn try_new(
        generation: GenerationHash,
        identity: CertificationAuthorizationIndexId,
        environments: BoundedVec<EnvironmentQualification, RECORDS>,
        records: BoundedVec<CertificationRecord, RECORDS>,
        profiles: BoundedVec<CertifiedExecutionProfile, PROFILES>,
    ) -> CertificationResult<Self> {
        if environments.iter().any(|entry| !entry.environment.valid())
            || environments
                .iter()
                .zip(environments.iter().skip(1))
                .any(|(left, right)| left.identity >= right.identity)
            || records
                .iter()
                .zip(records.iter().skip(1))
                .any(|(left, right)| left.identity >= right.identity)
            || profiles
                .iter()
                .zip(profiles.iter().skip(1))
                .any(|(left, right)| left.key.canonical_order(right.key) != Ordering::Less)
            || profiles.iter().enumerate().any(|(index, left)| {
                profiles
                    .iter()
                    .skip(index + 1)
                    .any(|right| left.key.identity == right.key.identity)
            })
        {
            return Err(CertificationError::NonCanonicalIndex);
        }
        if records.iter().any(|record| {
            !environments
                .iter()
                .any(|environment| environment.identity == record.environment)
        }) {
            return Err(CertificationError::MissingEnvironment);
        }
        for profile in profiles.iter() {
            let record = records
                .iter()
                .find(|record| record.identity == profile.record)
                .ok_or(CertificationError::MissingRecord)?;
            if record.envelope != profile.key.envelope {
                return Err(CertificationError::RecordDrift);
            }
        }
        Ok(Self {
            generation,
            identity,
            environments,
            records,
            profiles,
        })
    }
    fn record(
        &self,
        identity: CertificationRecordId,
        work: &mut WorkMeter,
    ) -> CertificationResult<&CertificationRecord> {
        let position = binary_find(
            self.records.len(),
            |index| self.records.get(index).unwrap().identity.cmp(&identity),
            work,
        )?
        .ok_or(CertificationError::MissingRecord)?;
        Ok(self.records.get(position).unwrap())
    }
    fn environment(
        &self,
        identity: EnvironmentQualificationId,
        work: &mut WorkMeter,
    ) -> CertificationResult<&EnvironmentQualification> {
        let position = binary_find(
            self.environments.len(),
            |index| {
                self.environments
                    .get(index)
                    .unwrap()
                    .identity
                    .cmp(&identity)
            },
            work,
        )?
        .ok_or(CertificationError::MissingEnvironment)?;
        Ok(self.environments.get(position).unwrap())
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ResolvedProfile {
    requirement: u16,
    profile: CertifiedExecutionProfile,
    record: CertificationRecord,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CertificationClosure<const CAPACITY: usize> {
    requirement_count: u16,
    backend_capabilities: [u8; 32],
    entries: BoundedVec<ResolvedProfile, CAPACITY>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CertificationApplicabilitySelection<const CAPACITY: usize> {
    generation: GenerationHash,
    index: CertificationAuthorizationIndexId,
    environment: EnvironmentFingerprint,
    requirement_count: u16,
    entries: BoundedVec<ResolvedProfile, CAPACITY>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum CertificationResolution<const CAPACITY: usize> {
    Stale,
    Current(CertificationClosure<CAPACITY>),
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CertificationError {
    InvalidIdentity,
    InvalidRequirement,
    DuplicateRequirement,
    NonCanonicalIndex,
    MissingEnvironment,
    MissingRecord,
    RecordDrift,
    MissingRequirement,
    StaleEvidence,
    EvidenceChanged,
    MissingApplicableRequirement,
    ClosureCapacity,
    Work(WorkBudgetError),
}
impl From<WorkBudgetError> for CertificationError {
    fn from(error: WorkBudgetError) -> Self {
        Self::Work(error)
    }
}
pub(crate) fn resolve<const R: usize, const P: usize, const C: usize>(
    described_backend: BackendGeneration,
    current_backend: BackendGeneration,
    revision: ModelRevisionId,
    facts: &RequestDescriptionFacts,
    index: &CertificationAuthorizationIndex<R, P>,
    work: &mut WorkMeter,
) -> CertificationResult<CertificationResolution<C>> {
    work.record(InvariantChecks, 1)?;
    if described_backend != current_backend {
        return Ok(CertificationResolution::Stale);
    }
    if !facts.valid(work)? {
        return Err(CertificationError::InvalidRequirement);
    }
    let mut entries = BoundedVec::new();
    let mut seen = [false; P];
    for (requirement_index, requirement) in facts.requirements.iter().copied().enumerate() {
        let Some(mut first) = binary_find(
            index.profiles.len(),
            |position| {
                index
                    .profiles
                    .get(position)
                    .unwrap()
                    .key
                    .requirement_order(revision, requirement)
            },
            work,
        )?
        else {
            return Err(CertificationError::MissingRequirement);
        };
        while first > 0 {
            work.record(VisitedEntities, 1)?;
            work.record(InvariantChecks, 1)?;
            if index
                .profiles
                .get(first - 1)
                .unwrap()
                .key
                .requirement_order(revision, requirement)
                != Ordering::Equal
            {
                break;
            }
            first -= 1;
        }
        work.record(InvariantChecks, 1)?;
        if seen[first] {
            return Err(CertificationError::DuplicateRequirement);
        }
        seen[first] = true;
        let mut matched = false;
        // Quarantine removes exact keys from the immutable successor index.
        for profile in index.profiles.iter().skip(first).copied() {
            work.record(VisitedEntities, 1)?;
            work.record(InvariantChecks, 1)?;
            match profile.key.requirement_order(revision, requirement) {
                Ordering::Less => return Err(CertificationError::NonCanonicalIndex),
                Ordering::Equal => matched = true,
                Ordering::Greater => break,
            }
            let record = *index.record(profile.record, work)?;
            work.record(InvariantChecks, 1)?;
            if entries.len() == entries.capacity() {
                return Err(CertificationError::ClosureCapacity);
            }
            let copied = size_of::<ResolvedProfile>() as u64;
            work.ensure(HotPathWorkWitness::new([0, copied, 0, 0, 0]))?;
            work.record(CopiedBytes, copied)?;
            entries
                .try_push(ResolvedProfile {
                    requirement: u16::try_from(requirement_index)
                        .map_err(|_| CertificationError::ClosureCapacity)?,
                    profile,
                    record,
                })
                .expect("prevalidated certification closure capacity");
        }
        if !matched {
            return Err(CertificationError::MissingRequirement);
        }
    }
    Ok(CertificationResolution::Current(CertificationClosure {
        requirement_count: u16::try_from(facts.requirements.len())
            .map_err(|_| CertificationError::ClosureCapacity)?,
        backend_capabilities: facts.backend_capabilities,
        entries,
    }))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ApplicabilityCacheKey {
    generation: GenerationHash,
    index: CertificationAuthorizationIndexId,
    environment: EnvironmentIdentity,
    profile: CertifiedExecutionProfile,
    record: CertificationRecord,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ApplicabilityCacheEntry {
    key: ApplicabilityCacheKey,
    applicable: bool,
    recent: bool,
}
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct ApplicabilityCache<const CAPACITY: usize> {
    entries: [Option<ApplicabilityCacheEntry>; CAPACITY],
    hand: usize,
}
impl<const CAPACITY: usize> ApplicabilityCache<CAPACITY> {
    pub(crate) fn new() -> Self {
        Self {
            entries: [None; CAPACITY],
            hand: 0,
        }
    }
    fn lookup(
        &mut self,
        key: ApplicabilityCacheKey,
        work: &mut WorkMeter,
    ) -> CertificationResult<Option<bool>> {
        for slot in &mut self.entries {
            work.record(VisitedEntities, 1)?;
            if let Some(entry) = slot {
                work.record(InvariantChecks, 1)?;
                if entry.key == key {
                    entry.recent = true;
                    return Ok(Some(entry.applicable));
                }
            }
        }
        Ok(None)
    }
    fn advance(&mut self) {
        self.hand += 1;
        if self.hand == CAPACITY {
            self.hand = 0;
        }
    }
    fn insert(
        &mut self,
        key: ApplicabilityCacheKey,
        applicable: bool,
        work: &mut WorkMeter,
    ) -> CertificationResult<()> {
        if CAPACITY == 0 {
            return Ok(());
        }
        work.record(CopiedBytes, size_of::<ApplicabilityCacheEntry>() as u64)?;
        let entry = ApplicabilityCacheEntry {
            key,
            applicable,
            recent: true,
        };
        for _ in 0..CAPACITY {
            work.record(VisitedEntities, 1)?;
            if self.entries[self.hand].is_none() {
                self.entries[self.hand] = Some(entry);
                self.advance();
                return Ok(());
            }
            self.advance();
        }
        for _ in 0..CAPACITY {
            work.record(VisitedEntities, 1)?;
            let current = self.entries[self.hand].as_mut().unwrap();
            if !current.recent {
                self.entries[self.hand] = Some(entry);
                self.advance();
                return Ok(());
            }
            current.recent = false;
            self.advance();
        }
        self.entries[self.hand] = Some(entry);
        self.advance();
        Ok(())
    }
}

fn entry_is_applicable<const R: usize, const P: usize>(
    entry: ResolvedProfile,
    backend_capabilities: [u8; 32],
    evidence: ApplicabilityEvidence,
    index: &CertificationAuthorizationIndex<R, P>,
    work: &mut WorkMeter,
) -> CertificationResult<bool> {
    let current_profile = binary_find(
        index.profiles.len(),
        |position| {
            index
                .profiles
                .get(position)
                .unwrap()
                .key
                .canonical_order(entry.profile.key)
        },
        work,
    )?
    .and_then(|position| index.profiles.get(position).copied());
    let record = *index.record(entry.record.identity, work)?;
    let environment = *index.environment(entry.record.environment, work)?;
    work.record(InvariantChecks, 8)?;
    Ok(current_profile == Some(entry.profile)
        && record == entry.record
        && environment.environment == evidence.environment.identity
        && backend_capabilities == environment.environment.backend_capabilities
        && entry.profile.key.requirement.adapter_build == environment.environment.adapter_build
        && entry.profile.key.requirement.mlx_build == environment.environment.mlx_build
        && entry.profile.key.requirement.backend_interface
            == environment.environment.backend_interface)
}

pub(crate) fn select_applicable<
    const R: usize,
    const P: usize,
    const C: usize,
    const CACHE: usize,
>(
    closure: &CertificationClosure<C>,
    index: &CertificationAuthorizationIndex<R, P>,
    cache: &mut ApplicabilityCache<CACHE>,
    start: ApplicabilityEvidence,
    finish: ApplicabilityEvidence,
    work: &mut WorkMeter,
) -> CertificationResult<CertificationApplicabilitySelection<C>> {
    work.record(InvariantChecks, 3)?;
    if !start.environment.fresh
        || start.generation != index.generation
        || start.index != index.identity
    {
        return Err(CertificationError::StaleEvidence);
    }
    let mut entries = BoundedVec::new();
    let mut covered = [false; C];
    for entry in closure.entries.iter().copied() {
        let key = ApplicabilityCacheKey {
            generation: start.generation,
            index: start.index,
            environment: start.environment.identity,
            profile: entry.profile,
            record: entry.record,
        };
        let applicable = if let Some(applicable) = cache.lookup(key, work)? {
            applicable
        } else {
            let applicable =
                entry_is_applicable(entry, closure.backend_capabilities, start, index, work)?;
            cache.insert(key, applicable, work)?;
            applicable
        };
        if applicable {
            let position = usize::from(entry.requirement);
            let Some(requirement) = covered.get_mut(position) else {
                return Err(CertificationError::MissingApplicableRequirement);
            };
            *requirement = true;
            work.record(CopiedBytes, size_of::<ResolvedProfile>() as u64)?;
            entries
                .try_push(entry)
                .map_err(|_| CertificationError::ClosureCapacity)?;
        }
    }
    work.record(InvariantChecks, 2)?;
    if finish != start || !finish.environment.fresh {
        return Err(CertificationError::EvidenceChanged);
    }
    for entry in entries.iter().copied() {
        if !entry_is_applicable(entry, closure.backend_capabilities, finish, index, work)? {
            return Err(CertificationError::EvidenceChanged);
        }
    }
    if covered
        .iter()
        .take(usize::from(closure.requirement_count))
        .any(|covered| !covered)
    {
        return Err(CertificationError::MissingApplicableRequirement);
    }
    Ok(CertificationApplicabilitySelection {
        generation: start.generation,
        index: start.index,
        environment: start.environment,
        requirement_count: closure.requirement_count,
        entries,
    })
}
#[rustfmt::skip]
#[cfg(test)]
mod tests {
    use super::*;
    use crate::request_book::REQUEST_REQUIREMENT_LIMIT;
    use crate::{BatchBucket, ByteCount, Duration, ExecutionPhase, HotPathWorkBudget, WorkDimension};
    fn id(tag: u8) -> [u8; 32] { [tag; 32] }
    fn wide_id(tag: u16) -> [u8; 32] { let mut identity = [0; 32]; identity[..2].copy_from_slice(&tag.to_be_bytes()); identity }
    fn bounded<T, const N: usize>(values: impl IntoIterator<Item = T>) -> BoundedVec<T, N> { let mut result = BoundedVec::new(); for value in values { result.try_push(value).unwrap(); } result }
    fn requirement(route: u8) -> CapabilityRequirement { CapabilityRequirement { phase: ExecutionPhase::Prefill, batch: BatchBucket(1), shape: 64, route: id(route), adapter_build: id(11), mlx_build: id(12), backend_interface: 1 } }
    fn facts(requirements: &[CapabilityRequirement]) -> RequestDescriptionFacts { RequestDescriptionFacts { requirements: bounded::<_, REQUEST_REQUIREMENT_LIMIT>(requirements.iter().copied()), backend_capabilities: id(13), ordinary_estimate: Duration::from_micros(1), conservative_time: Duration::from_micros(2), resource_bytes: ByteCount::new(3), output_bytes: ByteCount::new(4), residency_bytes: ByteCount::new(5) } }
    fn record(envelope: u8) -> CertificationRecord { CertificationRecord { identity: CertificationRecordId::try_new(id(20)).unwrap(), envelope: CertificationEnvelopeId::try_new(id(envelope)).unwrap(), environment: EnvironmentQualificationId::try_new(id(40)).unwrap() } }
    fn profile(tag: u8, route: u8) -> CertifiedExecutionProfile { CertifiedExecutionProfile { key: ExactCapabilityKey::try_new(CapabilityKey(id(tag)), ModelRevisionId::new(id(1)).unwrap(), requirement(route), CertificationEnvelopeId::try_new(id(30)).unwrap()).unwrap(), record: CertificationRecordId::try_new(id(20)).unwrap(), case_bounds: CaseBoundTableId::try_new(id(tag + 40)).unwrap() } }
    fn maximum_requirement(index: usize) -> CapabilityRequirement { let mut value = requirement(2); value.shape = u16::try_from(index).unwrap() + 1; value }
    fn maximum_record(index: usize) -> CertificationRecord { CertificationRecord { identity: CertificationRecordId::try_new(wide_id(u16::try_from(index).unwrap() + 1)).unwrap(), envelope: CertificationEnvelopeId::try_new(id(30)).unwrap(), environment: EnvironmentQualificationId::try_new(id(40)).unwrap() } }
    fn maximum_profile(index: usize) -> CertifiedExecutionProfile { let index = u16::try_from(index).unwrap(); CertifiedExecutionProfile { key: ExactCapabilityKey::try_new(CapabilityKey(wide_id(index + 257)), ModelRevisionId::new(id(1)).unwrap(), maximum_requirement(index.into()), CertificationEnvelopeId::try_new(id(30)).unwrap()).unwrap(), record: CertificationRecordId::try_new(wide_id(index + 1)).unwrap(), case_bounds: CaseBoundTableId::try_new(wide_id(index + 513)).unwrap() } }
    fn environment() -> EnvironmentQualification { EnvironmentQualification { identity: EnvironmentQualificationId::try_new(id(40)).unwrap(), environment: EnvironmentIdentity { device: id(1), gpu: id(2), unified_memory_bytes: 256, os_build: id(3), daemon_build: id(4), adapter_build: id(11), mlx_build: id(12), backend_interface: 1, bootstrap_manifest: id(5), backend_capabilities: id(13), generation_semantics: id(6), resource_signal: id(7), operation_bounds: id(8) } } }
    fn evidence(fresh: bool) -> ApplicabilityEvidence { ApplicabilityEvidence { generation: GenerationHash::try_new(id(60)).unwrap(), index: CertificationAuthorizationIndexId::try_new(id(61)).unwrap(), environment: EnvironmentFingerprint { identity: environment().environment, fresh } } }
    fn index() -> CertificationAuthorizationIndex<1, 3> { CertificationAuthorizationIndex::try_new(evidence(true).generation, evidence(true).index, bounded([environment()]), bounded([record(30)]), bounded([profile(50, 2), profile(51, 2), profile(52, 3)])).unwrap() }
    fn work() -> WorkMeter { WorkMeter::new(HotPathWorkBudget::binary_maximum()) }
    fn current<const C: usize>(facts: &RequestDescriptionFacts, index: &CertificationAuthorizationIndex<1, 3>, work: &mut WorkMeter) -> CertificationResult<CertificationResolution<C>> { let generation = BackendGeneration::new(1).unwrap(); resolve(generation, generation, ModelRevisionId::new(id(1)).unwrap(), facts, index, work) }
    #[test]
    fn authorizes_the_finite_exact_key_closure() {
        let index = index(); let input = facts(&[requirement(2), requirement(3)]); let mut meter = work();
        let CertificationResolution::Current(closure) = current::<3>(&input, &index, &mut meter).unwrap() else { panic!("current description must resolve"); };
        assert_eq!((closure.requirement_count, closure.entries.len()), (2, 3));
        assert_eq!((closure.entries.get(1).unwrap().profile.key.identity, closure.entries.get(2).unwrap().profile.key.identity), (CapabilityKey(id(51)), CapabilityKey(id(52))));
        assert_eq!(meter.witness(), HotPathWorkWitness::new([14, 1_104, 0, 0, 35]));
        let mut meter = work(); assert_eq!(current::<2>(&input, &index, &mut meter), Err(CertificationError::ClosureCapacity));
        let mut meter = work(); assert_eq!(current::<3>(&facts(&[requirement(9)]), &index, &mut meter), Err(CertificationError::MissingRequirement));
    }
    #[test]
    fn maximum_finite_closure_fits_binary_work_budget() {
        let requirements: [_; REQUEST_REQUIREMENT_LIMIT] = std::array::from_fn(maximum_requirement);
        let index: CertificationAuthorizationIndex<REQUEST_REQUIREMENT_LIMIT, REQUEST_REQUIREMENT_LIMIT> = CertificationAuthorizationIndex::try_new(evidence(true).generation, evidence(true).index, bounded([environment()]), bounded((0..REQUEST_REQUIREMENT_LIMIT).map(maximum_record)), bounded((0..REQUEST_REQUIREMENT_LIMIT).map(maximum_profile))).unwrap();
        let generation = BackendGeneration::new(1).unwrap(); let mut meter = work();
        let CertificationResolution::Current(closure) = resolve::<REQUEST_REQUIREMENT_LIMIT, REQUEST_REQUIREMENT_LIMIT, REQUEST_REQUIREMENT_LIMIT>(generation, generation, ModelRevisionId::new(id(1)).unwrap(), &facts(&requirements), &index, &mut meter).unwrap() else { panic!("current maximum description must resolve"); };
        assert_eq!((closure.requirement_count, closure.entries.len()), (256, REQUEST_REQUIREMENT_LIMIT));
        assert_eq!((closure.entries.get(0).unwrap().requirement, closure.entries.get(255).unwrap().requirement), (0, 255));
        assert_eq!(meter.witness(), HotPathWorkWitness::new([4_626, 94_208, 0, 0, 6_170]));
    }
    #[test]
    fn stale_drift_and_work_rejections_fail_closed() {
        let generation = BackendGeneration::new(1).unwrap();
        let empty = CertificationAuthorizationIndex::<0, 0>::try_new(evidence(true).generation, evidence(true).index, BoundedVec::new(), BoundedVec::new(), BoundedVec::new()).unwrap(); let mut meter = work();
        assert_eq!(resolve::<0, 0, 0>(generation, BackendGeneration::new(2).unwrap(), ModelRevisionId::new(id(1)).unwrap(), &facts(&[]), &empty, &mut meter).unwrap(), CertificationResolution::Stale);
        assert_eq!(meter.witness(), HotPathWorkWitness::new([0, 0, 0, 0, 1]));
        assert_eq!(CertificationAuthorizationIndex::<1, 0>::try_new(evidence(true).generation, evidence(true).index, BoundedVec::new(), bounded([record(30)]), BoundedVec::new()), Err(CertificationError::MissingEnvironment));
        assert_eq!(CertificationAuthorizationIndex::<1, 1>::try_new(evidence(true).generation, evidence(true).index, bounded([environment()]), bounded([record(31)]), bounded([profile(50, 2)])), Err(CertificationError::RecordDrift));
        assert_eq!(CertificationAuthorizationIndex::<1, 2>::try_new(evidence(true).generation, evidence(true).index, bounded([environment()]), bounded([record(30)]), bounded([profile(50, 2), profile(50, 2)])), Err(CertificationError::NonCanonicalIndex));
        let budget = HotPathWorkBudget::try_new(HotPathWorkWitness::new([1_000_000, 0, 0, 2, 28_708])).unwrap(); let mut meter = WorkMeter::new(budget);
        assert!(matches!(current::<1>(&facts(&[requirement(2)]), &index(), &mut meter), Err(CertificationError::Work(WorkBudgetError::BudgetExceeded(WorkDimension::CopiedBytes, 0, _)))));
    }
    #[test]
    fn derives_fresh_selection_and_rechecks_cache_hits() {
        let index = index(); let input = facts(&[requirement(2), requirement(3)]); let mut meter = work();
        let CertificationResolution::Current(closure) = current::<3>(&input, &index, &mut meter).unwrap() else { panic!("current description must resolve"); };
        let mut cache = ApplicabilityCache::<3>::new(); let snapshot = evidence(true); let mut first_work = work();
        let first = select_applicable(&closure, &index, &mut cache, snapshot, snapshot, &mut first_work).unwrap();
        assert_eq!((first.requirement_count, first.entries.len(), first.environment), (2, 3, snapshot.environment));
        let mut hit_work = work(); let hit = select_applicable(&closure, &index, &mut cache, snapshot, snapshot, &mut hit_work).unwrap();
        assert_eq!(hit, first); assert!(hit_work.witness().value(VisitedEntities) < first_work.witness().value(VisitedEntities));
    }
    #[test]
    fn stale_drift_and_selection_race_invalidate_applicability() {
        let index = index(); let mut meter = work(); let CertificationResolution::Current(closure) = current::<2>(&facts(&[requirement(2)]), &index, &mut meter).unwrap() else { panic!("current description must resolve"); }; let snapshot = evidence(true); let mut cache = ApplicabilityCache::<2>::new(); let mut meter = work(); select_applicable(&closure, &index, &mut cache, snapshot, snapshot, &mut meter).unwrap();
        let mut meter = work(); assert_eq!(select_applicable(&closure, &index, &mut cache, evidence(false), evidence(false), &mut meter), Err(CertificationError::StaleEvidence));
        let mut raced = snapshot; raced.environment.identity.daemon_build = id(90); let mut meter = work(); assert_eq!(select_applicable(&closure, &index, &mut cache, snapshot, raced, &mut meter), Err(CertificationError::EvidenceChanged));
        for drift in [|value: &mut EnvironmentIdentity| value.daemon_build = id(90), |value: &mut EnvironmentIdentity| value.backend_capabilities = id(91), |value: &mut EnvironmentIdentity| value.generation_semantics = id(92), |value: &mut EnvironmentIdentity| value.resource_signal = id(93), |value: &mut EnvironmentIdentity| value.operation_bounds = id(94)] { let mut changed = snapshot; drift(&mut changed.environment.identity); let mut meter = work(); assert_eq!(select_applicable(&closure, &index, &mut cache, changed, changed, &mut meter), Err(CertificationError::MissingApplicableRequirement)); }
    }
    #[test]
    fn cache_misses_and_eviction_never_replace_complete_selection() {
        let index = index(); let snapshot = evidence(true); let mut cache = ApplicabilityCache::<1>::new();
        for route in [2, 3, 2] { let mut meter = work(); let CertificationResolution::Current(closure) = current::<2>(&facts(&[requirement(route)]), &index, &mut meter).unwrap() else { panic!("current description must resolve"); }; let mut meter = work(); let selection = select_applicable(&closure, &index, &mut cache, snapshot, snapshot, &mut meter).unwrap(); assert_eq!((selection.requirement_count, selection.entries.len()), (1, if route == 2 { 2 } else { 1 })); }
        assert_eq!(cache.entries[0].unwrap().key.profile.key.identity, CapabilityKey(id(51)));
    }
    #[test]
    fn maximum_applicability_selection_fits_binary_work_budget() {
        let requirements: [_; REQUEST_REQUIREMENT_LIMIT] = std::array::from_fn(maximum_requirement); let index: CertificationAuthorizationIndex<REQUEST_REQUIREMENT_LIMIT, REQUEST_REQUIREMENT_LIMIT> = CertificationAuthorizationIndex::try_new(evidence(true).generation, evidence(true).index, bounded([environment()]), bounded((0..REQUEST_REQUIREMENT_LIMIT).map(maximum_record)), bounded((0..REQUEST_REQUIREMENT_LIMIT).map(maximum_profile))).unwrap(); let generation = BackendGeneration::new(1).unwrap(); let mut meter = work(); let CertificationResolution::Current(closure) = resolve::<REQUEST_REQUIREMENT_LIMIT, REQUEST_REQUIREMENT_LIMIT, REQUEST_REQUIREMENT_LIMIT>(generation, generation, ModelRevisionId::new(id(1)).unwrap(), &facts(&requirements), &index, &mut meter).unwrap() else { panic!("current maximum description must resolve"); };
        let mut cache = ApplicabilityCache::<1>::new(); let snapshot = evidence(true); let mut meter = work(); let selection = select_applicable(&closure, &index, &mut cache, snapshot, snapshot, &mut meter).unwrap(); assert_eq!((selection.requirement_count, selection.entries.len()), (256, REQUEST_REQUIREMENT_LIMIT));
    }
}
