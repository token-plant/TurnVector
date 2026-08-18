use turnvector_core::{
    AuthorizedCapabilitySet, BackendGeneration, BatchBucket, BoundedSet, BoundedVec,
    CandidateCoordinates, CandidateExclusion, CandidateExclusionReason, CandidateId,
    CandidateMember, CandidateValidationError, CapabilityKey, ConnectionId, DaemonInstanceId,
    ExecutionPhase, GenerationVector, ModelId, MonotonicTime, RequestId, RequestSequence,
    RuntimeOverheadBoundSetId, RuntimeOverheadGeneration, SafetyGeneration, SchedulerGeneration,
    SchedulingSnapshot, ServiceClass, WorkCandidate,
};
type Candidate = WorkCandidate<2>;
type Error = CandidateValidationError;
type CandidateResult = Result<Candidate, Error>;
type Snapshot = SchedulingSnapshot<3, 2, 2>;
fn vector<T: Clone, const N: usize>(items: &[T]) -> BoundedVec<T, N> {
    let mut values = BoundedVec::new();
    for item in items {
        values.try_push(item.clone()).unwrap();
    }
    values
}
fn generations(runtime: u64) -> GenerationVector {
    GenerationVector::new(
        SchedulerGeneration::new(1).unwrap(),
        BackendGeneration::new(1).unwrap(),
        SafetyGeneration::new(1).unwrap(),
        RuntimeOverheadGeneration::new(runtime).unwrap(),
    )
}
fn request(sequence: u64) -> RequestId {
    let daemon = DaemonInstanceId::new(1).unwrap();
    let connection = ConnectionId::new(1).unwrap();
    RequestId::new(daemon, connection, RequestSequence::new(sequence).unwrap())
}
fn member(request_id: RequestId, key: CapabilityKey) -> CandidateMember<2> {
    let mut authorized = AuthorizedCapabilitySet::new();
    authorized.try_insert(key).unwrap();
    CandidateMember {
        request_id,
        coordinates: coords(),
        authorized_capabilities: authorized,
        bound_set: RuntimeOverheadBoundSetId([3; 32]),
        runtime_overhead_generation: RuntimeOverheadGeneration::new(1).unwrap(),
    }
}
fn coords() -> CandidateCoordinates {
    CandidateCoordinates {
        model_id: ModelId::new(1).unwrap(),
        phase: ExecutionPhase::Prefill,
        service_class: ServiceClass::Interactive,
        batch_bucket: BatchBucket(2),
    }
}
fn candidate(id: u128, key: CapabilityKey, members: &[CandidateMember<2>]) -> CandidateResult {
    let id = CandidateId::new(id).unwrap();
    WorkCandidate::try_new(id, coords(), key, vector(members))
}
fn snapshot(
    runtime: u64,
    eligible: &[RequestId],
    candidates: &[Candidate],
    exclusions: &[CandidateExclusion],
) -> Result<Snapshot, CandidateValidationError> {
    let mut eligible_set = BoundedSet::new();
    eligible
        .iter()
        .for_each(|request| eligible_set.try_insert(*request).unwrap());
    SchedulingSnapshot::try_new(
        MonotonicTime::from_micros(10),
        generations(runtime),
        eligible_set,
        vector(candidates),
        vector(exclusions),
    )
}
#[test]
fn candidate_requires_authorized_uniform_member_evidence() {
    let key = CapabilityKey([1; 32]);
    let requests = [request(1), request(2)];
    let evidence = [member(requests[0], key), member(requests[1], key)];
    let valid = candidate(1, key, &evidence).unwrap();
    assert_eq!(valid.members().len(), 2);
    let error = candidate(2, CapabilityKey([2; 32]), &evidence[..1]).unwrap_err();
    assert_eq!(error, Error::CapabilityNotAuthorized);
    let mut changed = evidence[1].clone();
    changed.bound_set = RuntimeOverheadBoundSetId([4; 32]);
    let error = candidate(2, key, &[evidence[0].clone(), changed]).unwrap_err();
    assert_eq!(error, Error::BoundSetMismatch);
    let mut changed = evidence[1].clone();
    changed.runtime_overhead_generation = RuntimeOverheadGeneration::new(2).unwrap();
    let error = candidate(2, key, &[evidence[0].clone(), changed]).unwrap_err();
    assert_eq!(error, Error::RuntimeOverheadGenerationMismatch);
}
#[test]
fn snapshot_requires_current_generation_and_complete_dispositions() {
    let requests = [request(1), request(2), request(3)];
    let key = CapabilityKey([1; 32]);
    let evidence = [member(requests[0], key), member(requests[1], key)];
    let work = candidate(1, key, &evidence).unwrap();
    let exclusion = CandidateExclusion {
        request_id: requests[2],
        reason: CandidateExclusionReason::IncompatibleShape,
    };
    let one = std::slice::from_ref(&work);
    let valid = snapshot(1, &requests, one, &[exclusion]).unwrap();
    assert_eq!(valid.candidates().len(), 1);
    let error = snapshot(2, &requests, one, &[exclusion]).unwrap_err();
    assert_eq!(error, Error::SnapshotGenerationMismatch);
    let error = snapshot(1, &requests, one, &[]).unwrap_err();
    assert_eq!(error, Error::MissingDisposition);
    let alternate = candidate(2, key, &[member(requests[0], key)]).unwrap();
    let duplicate = [work.clone(), alternate];
    let error = snapshot(1, &requests, &duplicate, &[exclusion]).unwrap_err();
    assert_eq!(error, Error::DuplicateCandidateBucket);
    let covered = CandidateExclusion {
        request_id: requests[0],
        reason: CandidateExclusionReason::TimingInfeasible,
    };
    let error = snapshot(1, &requests, &[work], &[exclusion, covered]).unwrap_err();
    assert_eq!(error, Error::CoveredAndExcluded);
}
