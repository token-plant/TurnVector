#![allow(dead_code, reason = "C14 consumes the exact-key closure")]
use crate::WorkDimension::{CopiedBytes, InvariantChecks, VisitedEntities};
use crate::model_registry::ModelRevisionId;
use crate::request_book::{CapabilityRequirement, RequestDescriptionFacts};
use crate::{
    BackendGeneration, BoundedVec, CapabilityKey, HotPathWorkWitness, WorkBudgetError, WorkMeter,
};
use std::mem::size_of;
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
    CaseBoundTableId
);
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
    fn matches(self, revision: ModelRevisionId, requirement: CapabilityRequirement) -> bool {
        self.revision == revision && self.requirement == requirement
    }
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
    records: BoundedVec<CertificationRecord, RECORDS>,
    profiles: BoundedVec<CertifiedExecutionProfile, PROFILES>,
}
impl<const RECORDS: usize, const PROFILES: usize>
    CertificationAuthorizationIndex<RECORDS, PROFILES>
{
    pub(crate) fn try_new(
        records: BoundedVec<CertificationRecord, RECORDS>,
        profiles: BoundedVec<CertifiedExecutionProfile, PROFILES>,
    ) -> CertificationResult<Self> {
        if records
            .iter()
            .zip(records.iter().skip(1))
            .any(|(left, right)| left.identity >= right.identity)
            || profiles
                .iter()
                .zip(profiles.iter().skip(1))
                .any(|(left, right)| left.key.identity.0 >= right.key.identity.0)
        {
            return Err(CertificationError::NonCanonicalIndex);
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
        Ok(Self { records, profiles })
    }
    fn record(
        &self,
        identity: CertificationRecordId,
        work: &mut WorkMeter,
    ) -> CertificationResult<&CertificationRecord> {
        for record in self.records.iter() {
            work.record(VisitedEntities, 1)?;
            work.record(InvariantChecks, 1)?;
            if record.identity == identity {
                return Ok(record);
            }
        }
        Err(CertificationError::MissingRecord)
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
    MissingRecord,
    RecordDrift,
    MissingRequirement,
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
    for (requirement_index, requirement) in facts.requirements.iter().copied().enumerate() {
        for prior in facts.requirements.iter().take(requirement_index) {
            work.record(VisitedEntities, 1)?;
            work.record(InvariantChecks, 1)?;
            if *prior == requirement {
                return Err(CertificationError::DuplicateRequirement);
            }
        }
        let before = entries.len();
        // Quarantine removes exact keys from the immutable successor index.
        for profile in index.profiles.iter().copied() {
            work.record(VisitedEntities, 1)?;
            work.record(InvariantChecks, 1)?;
            if !profile.key.matches(revision, requirement) {
                continue;
            }
            let record = *index.record(profile.record, work)?;
            work.record(InvariantChecks, 1)?;
            if record.envelope != profile.key.envelope {
                return Err(CertificationError::RecordDrift);
            }
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
        if entries.len() == before {
            return Err(CertificationError::MissingRequirement);
        }
    }
    Ok(CertificationResolution::Current(CertificationClosure {
        requirement_count: u16::try_from(facts.requirements.len())
            .map_err(|_| CertificationError::ClosureCapacity)?,
        entries,
    }))
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::request_book::REQUEST_REQUIREMENT_LIMIT;
    use crate::{
        BatchBucket, ByteCount, Duration, ExecutionPhase, HotPathWorkBudget, WorkDimension,
    };
    fn id(tag: u8) -> [u8; 32] {
        [tag; 32]
    }
    fn bounded<T, const N: usize>(values: impl IntoIterator<Item = T>) -> BoundedVec<T, N> {
        let mut result = BoundedVec::new();
        for value in values {
            result.try_push(value).unwrap();
        }
        result
    }
    fn requirement(route: u8) -> CapabilityRequirement {
        CapabilityRequirement {
            phase: ExecutionPhase::Prefill,
            batch: BatchBucket(1),
            shape: 64,
            route: id(route),
            adapter_build: id(11),
            mlx_build: id(12),
            backend_interface: 1,
        }
    }
    fn facts(requirements: &[CapabilityRequirement]) -> RequestDescriptionFacts {
        RequestDescriptionFacts {
            requirements: bounded::<_, REQUEST_REQUIREMENT_LIMIT>(requirements.iter().copied()),
            backend_capabilities: id(13),
            ordinary_estimate: Duration::from_micros(1),
            conservative_time: Duration::from_micros(2),
            resource_bytes: ByteCount::new(3),
            output_bytes: ByteCount::new(4),
            residency_bytes: ByteCount::new(5),
        }
    }
    fn record(envelope: u8) -> CertificationRecord {
        CertificationRecord {
            identity: CertificationRecordId::try_new(id(20)).unwrap(),
            envelope: CertificationEnvelopeId::try_new(id(envelope)).unwrap(),
            environment: EnvironmentQualificationId::try_new(id(40)).unwrap(),
        }
    }
    fn profile(tag: u8, route: u8) -> CertifiedExecutionProfile {
        CertifiedExecutionProfile {
            key: ExactCapabilityKey::try_new(
                CapabilityKey(id(tag)),
                ModelRevisionId::new(id(1)).unwrap(),
                requirement(route),
                CertificationEnvelopeId::try_new(id(30)).unwrap(),
            )
            .unwrap(),
            record: CertificationRecordId::try_new(id(20)).unwrap(),
            case_bounds: CaseBoundTableId::try_new(id(tag + 40)).unwrap(),
        }
    }
    fn index() -> CertificationAuthorizationIndex<1, 2> {
        CertificationAuthorizationIndex::try_new(
            bounded([record(30)]),
            bounded([profile(50, 2), profile(51, 3)]),
        )
        .unwrap()
    }
    fn work() -> WorkMeter {
        WorkMeter::new(HotPathWorkBudget::binary_maximum())
    }
    fn current<const C: usize>(
        facts: &RequestDescriptionFacts,
        index: &CertificationAuthorizationIndex<1, 2>,
        work: &mut WorkMeter,
    ) -> CertificationResult<CertificationResolution<C>> {
        let generation = BackendGeneration::new(1).unwrap();
        resolve(
            generation,
            generation,
            ModelRevisionId::new(id(1)).unwrap(),
            facts,
            index,
            work,
        )
    }
    #[test]
    fn authorizes_the_finite_exact_key_closure() {
        let index = index();
        let input = facts(&[requirement(2), requirement(3)]);
        let mut meter = work();
        let CertificationResolution::Current(closure) =
            current::<2>(&input, &index, &mut meter).unwrap()
        else {
            panic!("current description must resolve");
        };
        assert_eq!((closure.requirement_count, closure.entries.len()), (2, 2));
        assert_eq!(
            closure.entries.get(1).unwrap().profile.key.identity,
            CapabilityKey(id(51))
        );
        assert_eq!(meter.witness(), HotPathWorkWitness::new([9, 736, 0, 0, 29]));
        let mut meter = work();
        assert_eq!(
            current::<1>(&input, &index, &mut meter),
            Err(CertificationError::ClosureCapacity)
        );
        let mut meter = work();
        assert_eq!(
            current::<2>(&facts(&[requirement(9)]), &index, &mut meter),
            Err(CertificationError::MissingRequirement)
        );
    }
    #[test]
    fn stale_drift_and_work_rejections_fail_closed() {
        let generation = BackendGeneration::new(1).unwrap();
        let empty =
            CertificationAuthorizationIndex::<0, 0>::try_new(BoundedVec::new(), BoundedVec::new())
                .unwrap();
        let mut meter = work();
        assert_eq!(
            resolve::<0, 0, 0>(
                generation,
                BackendGeneration::new(2).unwrap(),
                ModelRevisionId::new(id(1)).unwrap(),
                &facts(&[]),
                &empty,
                &mut meter,
            )
            .unwrap(),
            CertificationResolution::Stale
        );
        assert_eq!(meter.witness(), HotPathWorkWitness::new([0, 0, 0, 0, 1]));
        assert_eq!(
            CertificationAuthorizationIndex::<1, 1>::try_new(
                bounded([record(31)]),
                bounded([profile(50, 2)])
            ),
            Err(CertificationError::RecordDrift)
        );
        assert_eq!(
            CertificationAuthorizationIndex::<1, 2>::try_new(
                bounded([record(30)]),
                bounded([profile(50, 2), profile(50, 2)])
            ),
            Err(CertificationError::NonCanonicalIndex)
        );
        let budget =
            HotPathWorkBudget::try_new(HotPathWorkWitness::new([1_000_000, 0, 0, 2, 28_708]))
                .unwrap();
        let mut meter = WorkMeter::new(budget);
        assert!(matches!(
            current::<1>(&facts(&[requirement(2)]), &index(), &mut meter),
            Err(CertificationError::Work(WorkBudgetError::BudgetExceeded(
                WorkDimension::CopiedBytes,
                0,
                _
            )))
        ));
    }
}
