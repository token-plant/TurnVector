use turnvector_core::{
    AuthorizedCapabilitySet, BackendGeneration, BatchBucket, BoundedSet, BoundedVec,
    CandidateCoordinates, CandidateExclusion, CandidateExclusionReason, CandidateId,
    CandidateMember, CandidateValidationError, CapabilityKey, ConnectionId, DaemonInstanceId,
    ExecutionPhase, GenerationVector, ModelId, MonotonicTime, RequestId, RequestSequence,
    RuntimeOverheadBoundSetId, RuntimeOverheadGeneration, SafetyGeneration, SchedulerGeneration,
    SchedulingSnapshot, ServiceClass, WorkCandidate,
};
type Candidate = WorkCandidate<4>;
type Error = CandidateValidationError;
type CandidateResult = Result<Candidate, Error>;
type Snapshot = SchedulingSnapshot<4, 2, 4>;
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

mod turn_plan_contract {
    use super::*;
    use turnvector_core::{
        Duration, FutureTurnSupportEntitlementId, MemberOutcome,
        PersistentStateIsolationEvidenceId, PhysicalStartCreditId, PlanMemberFunding,
        PlanSupportObligation, PlanSupportObligations, PlanValidationError,
        StalePlanDispositionBoundId, SupportOperationObligationId,
        SupportOutstandingCreditVectorId, TokenCount, TurnBudget, TurnPlan, TurnPlanId,
        TurnProgress, TurnReceipt, TurnReceiptMember, YieldReason,
    };

    type Plan = TurnPlan<4>;
    type Evidence = (
        BoundedVec<PlanMemberFunding, 4>,
        TurnBudget,
        PlanSupportObligations<4>,
    );

    fn obligation(
        seed: u8,
        funders: &BoundedVec<PlanMemberFunding, 4>,
    ) -> PlanSupportObligation<4> {
        PlanSupportObligation {
            id: SupportOperationObligationId([seed; 32]),
            physical_credit: PhysicalStartCreditId([seed + 10; 32]),
            funders: *funders,
        }
    }

    fn fixture(count: usize) -> (Snapshot, Evidence) {
        let key = CapabilityKey([1; 32]);
        let all: [PlanMemberFunding; 4] = std::array::from_fn(|index| PlanMemberFunding {
            request_id: request(index as u64 + 1),
            entitlement: FutureTurnSupportEntitlementId([index as u8 + 1; 32]),
            credit_vector: SupportOutstandingCreditVectorId([index as u8 + 21; 32]),
        });
        let members = vector(&all[..count]);
        let candidate_members: [CandidateMember<2>; 4] =
            std::array::from_fn(|index| member(all[index].request_id, key));
        let work = candidate(1, key, &candidate_members[..count]).unwrap();
        let requests: [RequestId; 4] = std::array::from_fn(|index| all[index].request_id);
        let snapshot = snapshot(1, &requests[..count], &[work], &[]).unwrap();
        let budget = TurnBudget {
            target_engine_service: Duration::from_micros(50),
            hard_execution_bound: Duration::from_micros(100),
            stale_disposition_bound: StalePlanDispositionBoundId([3; 32]),
            stale_successor_ceiling: Duration::from_micros(25),
            phase_work_ceiling: TokenCount::new(64),
        };
        let support = PlanSupportObligations {
            receipt_observation: obligation(1, &members),
            conditional_continuation_formation: obligation(2, &members),
            rejection_or_local_stale_formation: obligation(3, &members),
        };
        (snapshot, (members, budget, support))
    }

    fn build(snapshot: &Snapshot, evidence: Evidence) -> Result<Plan, PlanValidationError> {
        let (members, budget, support) = evidence;
        let plan_id = TurnPlanId::new(1).unwrap();
        let candidate_id = CandidateId::new(1).unwrap();
        TurnPlan::try_new(plan_id, snapshot, candidate_id, members, budget, support)
    }

    fn receipt(
        plan: &Plan,
        members: BoundedVec<TurnReceiptMember, 4>,
    ) -> Result<TurnReceipt<4>, PlanValidationError> {
        let service = Duration::from_micros(60);
        TurnReceipt::try_new(plan, service, true, YieldReason::WorkCeiling, members)
    }

    #[test]
    fn plan_freezes_b1_b4_funding_and_snapshot_identity() {
        for count in [1, 4] {
            let (snapshot, evidence) = fixture(count);
            let expected = evidence.clone();
            let plan = build(&snapshot, evidence).unwrap();
            let support = plan.support();
            assert_eq!(plan.members(), &expected.0);
            assert_eq!(support, &expected.2);
            assert_eq!(plan.identity().generations, snapshot.generations());
            assert_eq!(
                plan.identity().bound_set,
                RuntimeOverheadBoundSetId([3; 32])
            );
            assert_eq!(plan.identity().budget, expected.1);
            let mut reused = expected;
            reused.2.conditional_continuation_formation.physical_credit =
                reused.2.receipt_observation.physical_credit;
            assert_eq!(
                build(&snapshot, reused).unwrap_err(),
                PlanValidationError::ReusedSupportIdentity
            );
        }
    }

    #[test]
    fn receipt_preserves_order_progress_and_typed_outcomes() {
        let (snapshot, evidence) = fixture(4);
        let plan = build(&snapshot, evidence).unwrap();
        let outcomes = [
            MemberOutcome::Completed,
            MemberOutcome::Cancelled,
            MemberOutcome::Partial,
            MemberOutcome::Failed(Some(PersistentStateIsolationEvidenceId([9; 32]))),
        ];
        let mut rows: [TurnReceiptMember; 4] = std::array::from_fn(|index| TurnReceiptMember {
            request_id: plan.members().iter().nth(index).unwrap().request_id,
            progress: Some(TurnProgress {
                start: TokenCount::new(0),
                end: TokenCount::new(1),
                has_continuation: outcomes[index] == MemberOutcome::Partial,
            }),
            outcome: outcomes[index],
            still_runnable: outcomes[index] == MemberOutcome::Partial,
        });
        let accepted = receipt(&plan, vector(&rows)).unwrap();
        assert_eq!(accepted.identity().plan, plan.identity());
        assert_eq!(accepted.members(), &vector(&rows));
        rows[3].outcome = MemberOutcome::Failed(None);
        assert!(receipt(&plan, vector(&rows)).is_ok());
        rows[0].progress.as_mut().unwrap().start = TokenCount::new(2);
        assert_eq!(
            receipt(&plan, vector(&rows)).unwrap_err(),
            PlanValidationError::ReceiptProgressMismatch
        );
    }
}
