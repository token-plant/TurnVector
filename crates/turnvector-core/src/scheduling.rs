//! ```compile_fail
//! fn bypass<const E: usize, const C: usize, const M: usize>(
//!     candidate: &mut turnvector_core::WorkCandidate<M>,
//!     snapshot: &mut turnvector_core::SchedulingSnapshot<E, C, M>,
//! ) {
//!     candidate.members = todo!(); snapshot.candidates = todo!();
//! }
//! ```

use crate::{
    BoundedSet, BoundedVec, CandidateId, DomainValueError, GenerationVector, ModelId,
    MonotonicTime, RequestId, RuntimeOverheadGeneration,
};

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct CapabilityKey([u8; 32]);

impl CapabilityKey {
    pub fn new(bytes: [u8; 32]) -> Result<Self, DomainValueError> {
        (bytes != [0; 32])
            .then_some(Self(bytes))
            .ok_or(DomainValueError::Zero)
    }

    pub const fn get(self) -> [u8; 32] {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct RuntimeOverheadBoundSetId([u8; 32]);

impl RuntimeOverheadBoundSetId {
    pub fn new(bytes: [u8; 32]) -> Result<Self, DomainValueError> {
        (bytes != [0; 32])
            .then_some(Self(bytes))
            .ok_or(DomainValueError::Zero)
    }

    pub const fn get(self) -> [u8; 32] {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct BatchBucket(pub u16);

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ExecutionPhase {
    Prefill,
    Decode,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ServiceClass {
    Interactive,
    Standard,
    Background,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct CandidateCoordinates {
    pub model_id: ModelId,
    pub phase: ExecutionPhase,
    pub service_class: ServiceClass,
    pub batch_bucket: BatchBucket,
}

pub type AuthorizedCapabilitySet<const CAPACITY: usize> = BoundedSet<CapabilityKey, CAPACITY>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CandidateMember<const CAPABILITIES: usize> {
    pub request_id: RequestId,
    pub coordinates: CandidateCoordinates,
    pub authorized_capabilities: AuthorizedCapabilitySet<CAPABILITIES>,
    pub bound_set: RuntimeOverheadBoundSetId,
    pub runtime_overhead_generation: RuntimeOverheadGeneration,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CandidateExclusionReason {
    IncompatibleBatch,
    IncompatibleShape,
    TimingInfeasible,
    ProgressInfeasible,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CandidateExclusion {
    pub request_id: RequestId,
    pub reason: CandidateExclusionReason,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CandidateValidationError {
    EmptyCandidate,
    DuplicateMember,
    MemberCoordinatesMismatch,
    CapabilityNotAuthorized,
    BoundSetMismatch,
    RuntimeOverheadGenerationMismatch,
    SnapshotGenerationMismatch,
    DuplicateCandidateId,
    DuplicateCandidateBucket,
    UnknownEligibleRequest,
    DuplicateExclusion,
    CoveredAndExcluded,
    MissingDisposition,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkCandidate<const MEMBERS: usize> {
    id: CandidateId,
    coordinates: CandidateCoordinates,
    capability_key: CapabilityKey,
    members: BoundedSet<RequestId, MEMBERS>,
    bound_set: RuntimeOverheadBoundSetId,
    runtime_overhead_generation: RuntimeOverheadGeneration,
}

impl<const MEMBERS: usize> WorkCandidate<MEMBERS> {
    pub const fn id(&self) -> CandidateId {
        self.id
    }
    pub const fn coordinates(&self) -> CandidateCoordinates {
        self.coordinates
    }
    pub const fn capability_key(&self) -> CapabilityKey {
        self.capability_key
    }
    pub const fn members(&self) -> &BoundedSet<RequestId, MEMBERS> {
        &self.members
    }
    pub const fn bound_set(&self) -> RuntimeOverheadBoundSetId {
        self.bound_set
    }
    pub const fn runtime_overhead_generation(&self) -> RuntimeOverheadGeneration {
        self.runtime_overhead_generation
    }
    pub fn try_new<const CAPABILITIES: usize>(
        id: CandidateId,
        coordinates: CandidateCoordinates,
        capability_key: CapabilityKey,
        evidence: BoundedVec<CandidateMember<CAPABILITIES>, MEMBERS>,
    ) -> Result<Self, CandidateValidationError> {
        let first = evidence
            .iter()
            .next()
            .ok_or(CandidateValidationError::EmptyCandidate)?;
        let (bound_set, generation) = (first.bound_set, first.runtime_overhead_generation);
        let mut members = BoundedSet::new();
        for member in evidence.iter() {
            if member.coordinates != coordinates {
                return Err(CandidateValidationError::MemberCoordinatesMismatch);
            }
            if !member.authorized_capabilities.contains(&capability_key) {
                return Err(CandidateValidationError::CapabilityNotAuthorized);
            }
            if member.bound_set != bound_set {
                return Err(CandidateValidationError::BoundSetMismatch);
            }
            if member.runtime_overhead_generation != generation {
                return Err(CandidateValidationError::RuntimeOverheadGenerationMismatch);
            }
            members
                .try_insert(member.request_id)
                .map_err(|_| CandidateValidationError::DuplicateMember)?;
        }
        Ok(Self {
            id,
            coordinates,
            capability_key,
            members,
            bound_set,
            runtime_overhead_generation: generation,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SchedulingSnapshot<const ELIGIBLE: usize, const CANDIDATES: usize, const MEMBERS: usize>
{
    formed_at: MonotonicTime,
    generations: GenerationVector,
    candidates: BoundedVec<WorkCandidate<MEMBERS>, CANDIDATES>,
    exclusions: BoundedVec<CandidateExclusion, ELIGIBLE>,
}

impl<const E: usize, const C: usize, const M: usize> SchedulingSnapshot<E, C, M> {
    pub const fn formed_at(&self) -> MonotonicTime {
        self.formed_at
    }
    pub const fn generations(&self) -> GenerationVector {
        self.generations
    }
    pub const fn candidates(&self) -> &BoundedVec<WorkCandidate<M>, C> {
        &self.candidates
    }
    pub const fn exclusions(&self) -> &BoundedVec<CandidateExclusion, E> {
        &self.exclusions
    }
    pub fn try_new(
        formed_at: MonotonicTime,
        generations: GenerationVector,
        eligible: BoundedSet<RequestId, E>,
        candidates: BoundedVec<WorkCandidate<M>, C>,
        exclusions: BoundedVec<CandidateExclusion, E>,
    ) -> Result<Self, CandidateValidationError> {
        let mut covered = BoundedSet::<RequestId, E>::new();
        for (index, candidate) in candidates.iter().enumerate() {
            if candidate.runtime_overhead_generation != generations.runtime_overhead {
                return Err(CandidateValidationError::SnapshotGenerationMismatch);
            }
            for prior in candidates.iter().take(index) {
                if prior.id == candidate.id {
                    return Err(CandidateValidationError::DuplicateCandidateId);
                }
                if prior.coordinates == candidate.coordinates {
                    return Err(CandidateValidationError::DuplicateCandidateBucket);
                }
            }
            for member in candidate.members.iter() {
                if !eligible.contains(member) {
                    return Err(CandidateValidationError::UnknownEligibleRequest);
                }
                if !covered.contains(member) {
                    covered
                        .try_insert(*member)
                        .map_err(|_| CandidateValidationError::UnknownEligibleRequest)?;
                }
            }
        }
        let mut excluded = BoundedSet::<RequestId, E>::new();
        for exclusion in exclusions.iter() {
            if !eligible.contains(&exclusion.request_id) {
                return Err(CandidateValidationError::UnknownEligibleRequest);
            }
            if covered.contains(&exclusion.request_id) {
                return Err(CandidateValidationError::CoveredAndExcluded);
            }
            excluded
                .try_insert(exclusion.request_id)
                .map_err(|_| CandidateValidationError::DuplicateExclusion)?;
        }
        if eligible
            .iter()
            .any(|item| !covered.contains(item) && !excluded.contains(item))
        {
            return Err(CandidateValidationError::MissingDisposition);
        }
        Ok(Self {
            formed_at,
            generations,
            candidates,
            exclusions,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scheduling_digest_identities_reject_zero_and_round_trip_nonzero() {
        let zero = [0; 32];
        let nonzero = [1; 32];
        assert_eq!(CapabilityKey::new(zero), Err(DomainValueError::Zero));
        assert_eq!(
            RuntimeOverheadBoundSetId::new(zero),
            Err(DomainValueError::Zero)
        );
        let key = CapabilityKey::new(nonzero).unwrap();
        assert_eq!(key.get(), nonzero);
        let bound = RuntimeOverheadBoundSetId::new(nonzero).unwrap();
        assert_eq!(bound.get(), nonzero);
    }
}
