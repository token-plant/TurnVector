use crate::{
    BoundedVec, CandidateCoordinates, CandidateId, CapabilityKey, DomainValueError, Duration,
    GenerationVector, RequestId, RuntimeOverheadBoundSetId, SchedulingSnapshot, TokenCount,
    TurnPlanId,
};

macro_rules! digest_identity {
    ($name:ident) => {
        #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name([u8; 32]);

        impl $name {
            pub fn new(bytes: [u8; 32]) -> Result<Self, DomainValueError> {
                (bytes != [0; 32])
                    .then_some(Self(bytes))
                    .ok_or(DomainValueError::Zero)
            }

            pub const fn get(self) -> [u8; 32] {
                self.0
            }
        }
    };
}
macro_rules! copy_record {
    ($name:ident { $($field:ident => $ty:ty),+ $(,)? }) => {
        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        pub struct $name { $(pub $field: $ty),+ }
    };
}
digest_identity!(FutureTurnSupportEntitlementId);
digest_identity!(SupportOutstandingCreditVectorId);
digest_identity!(SupportOperationObligationId);
digest_identity!(PhysicalStartCreditId);
digest_identity!(StalePlanDispositionBoundId);
digest_identity!(PersistentStateIsolationEvidenceId);

copy_record!(PlanMemberFunding {
    request_id => RequestId, entitlement => FutureTurnSupportEntitlementId,
    credit_vector => SupportOutstandingCreditVectorId,
});

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlanSupportObligation<const MEMBERS: usize> {
    pub id: SupportOperationObligationId,
    pub physical_credit: PhysicalStartCreditId,
    pub funders: BoundedVec<PlanMemberFunding, MEMBERS>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlanSupportObligations<const MEMBERS: usize> {
    pub receipt_observation: PlanSupportObligation<MEMBERS>,
    pub conditional_continuation_formation: PlanSupportObligation<MEMBERS>,
    pub rejection_or_local_stale_formation: PlanSupportObligation<MEMBERS>,
}

copy_record!(TurnBudget {
    target_engine_service => Duration, hard_execution_bound => Duration,
    stale_disposition_bound => StalePlanDispositionBoundId,
    stale_successor_ceiling => Duration, phase_work_ceiling => TokenCount,
});
copy_record!(TurnPlanIdentity {
    id => TurnPlanId, candidate_id => CandidateId,
    coordinates => CandidateCoordinates, capability_key => CapabilityKey,
    generations => GenerationVector, bound_set => RuntimeOverheadBoundSetId,
    budget => TurnBudget,
});

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlanValidationError {
    UnknownCandidate,
    MemberSetMismatch,
    NonCanonicalMembers,
    DuplicateFundingIdentity,
    InvalidWorkBudget,
    FundingMismatch,
    ReusedSupportIdentity,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReceiptValidationError {
    MemberMismatch,
    ProgressMismatch,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TurnPlan<const MEMBERS: usize> {
    identity: TurnPlanIdentity,
    members: BoundedVec<PlanMemberFunding, MEMBERS>,
    support: PlanSupportObligations<MEMBERS>,
}

impl<const MEMBERS: usize> TurnPlan<MEMBERS> {
    pub fn try_new<const ELIGIBLE: usize, const CANDIDATES: usize>(
        id: TurnPlanId,
        snapshot: &SchedulingSnapshot<ELIGIBLE, CANDIDATES, MEMBERS>,
        candidate_id: CandidateId,
        members: BoundedVec<PlanMemberFunding, MEMBERS>,
        budget: TurnBudget,
        support: PlanSupportObligations<MEMBERS>,
    ) -> Result<Self, PlanValidationError> {
        let candidate = snapshot
            .candidates()
            .iter()
            .find(|candidate| candidate.id() == candidate_id)
            .ok_or(PlanValidationError::UnknownCandidate)?;
        let generations = snapshot.generations();
        if members.len() != candidate.members().len()
            || members
                .iter()
                .any(|m| !candidate.members().contains(&m.request_id))
        {
            return Err(PlanValidationError::MemberSetMismatch);
        }
        for (index, member) in members.iter().enumerate() {
            for prior in members.iter().take(index) {
                if prior.request_id >= member.request_id {
                    return Err(PlanValidationError::NonCanonicalMembers);
                }
                if prior.entitlement == member.entitlement
                    || prior.credit_vector == member.credit_vector
                {
                    return Err(PlanValidationError::DuplicateFundingIdentity);
                }
            }
        }
        if budget.target_engine_service.as_micros() == 0
            || budget.target_engine_service > budget.hard_execution_bound
            || budget.stale_successor_ceiling.as_micros() == 0
            || budget.phase_work_ceiling.get() == 0
        {
            return Err(PlanValidationError::InvalidWorkBudget);
        }
        let obligations = [
            &support.receipt_observation,
            &support.conditional_continuation_formation,
            &support.rejection_or_local_stale_formation,
        ];
        for (index, obligation) in obligations.iter().enumerate() {
            if obligation.funders != members {
                return Err(PlanValidationError::FundingMismatch);
            }
            for prior in &obligations[..index] {
                if prior.id == obligation.id || prior.physical_credit == obligation.physical_credit
                {
                    return Err(PlanValidationError::ReusedSupportIdentity);
                }
            }
        }
        Ok(Self {
            identity: TurnPlanIdentity {
                id,
                candidate_id: candidate.id(),
                coordinates: candidate.coordinates(),
                capability_key: candidate.capability_key(),
                generations,
                bound_set: candidate.bound_set(),
                budget,
            },
            members,
            support,
        })
    }

    pub const fn identity(&self) -> TurnPlanIdentity {
        self.identity
    }
    pub const fn members(&self) -> &BoundedVec<PlanMemberFunding, MEMBERS> {
        &self.members
    }
    pub const fn support(&self) -> &PlanSupportObligations<MEMBERS> {
        &self.support
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MemberOutcome {
    Completed,
    Cancelled,
    Partial,
    Failed(Option<PersistentStateIsolationEvidenceId>),
}

copy_record!(TurnProgress {
    start => TokenCount, end => TokenCount,
    has_continuation => bool,
});

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum YieldReason {
    WorkCeiling,
    ServiceTarget,
    Completed,
    Cancelled,
    BackendFailure,
}

copy_record!(TurnReceiptMember {
    request_id => RequestId, progress => Option<TurnProgress>,
    outcome => MemberOutcome, still_runnable => bool,
});
copy_record!(TurnReceiptIdentity {
    plan => TurnPlanIdentity, engine_service => Duration,
    resumable => bool, yield_reason => YieldReason,
});

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TurnReceipt<const MEMBERS: usize> {
    identity: TurnReceiptIdentity,
    members: BoundedVec<TurnReceiptMember, MEMBERS>,
}

impl<const MEMBERS: usize> TurnReceipt<MEMBERS> {
    pub fn try_new(
        plan: &TurnPlan<MEMBERS>,
        engine_service: Duration,
        resumable: bool,
        yield_reason: YieldReason,
        members: BoundedVec<TurnReceiptMember, MEMBERS>,
    ) -> Result<Self, ReceiptValidationError> {
        if members.len() != plan.members.len()
            || members
                .iter()
                .zip(plan.members.iter())
                .any(|(a, b)| a.request_id != b.request_id)
        {
            return Err(ReceiptValidationError::MemberMismatch);
        }
        if members
            .iter()
            .filter_map(|m| m.progress)
            .any(|progress| progress.end < progress.start)
        {
            return Err(ReceiptValidationError::ProgressMismatch);
        }
        Ok(Self {
            identity: TurnReceiptIdentity {
                plan: plan.identity,
                engine_service,
                resumable,
                yield_reason,
            },
            members,
        })
    }

    pub const fn identity(&self) -> TurnReceiptIdentity {
        self.identity
    }
    pub const fn members(&self) -> &BoundedVec<TurnReceiptMember, MEMBERS> {
        &self.members
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digest_identities_reject_zero_and_round_trip_nonzero() {
        let zero = [0; 32];
        let nonzero = [1; 32];
        assert_eq!(
            FutureTurnSupportEntitlementId::new(zero),
            Err(DomainValueError::Zero)
        );
        assert_eq!(
            SupportOutstandingCreditVectorId::new(zero),
            Err(DomainValueError::Zero)
        );
        assert_eq!(
            SupportOperationObligationId::new(zero),
            Err(DomainValueError::Zero)
        );
        assert_eq!(
            PhysicalStartCreditId::new(zero),
            Err(DomainValueError::Zero)
        );
        assert_eq!(
            StalePlanDispositionBoundId::new(zero),
            Err(DomainValueError::Zero)
        );
        assert_eq!(
            PersistentStateIsolationEvidenceId::new(zero),
            Err(DomainValueError::Zero)
        );
        assert_eq!(
            FutureTurnSupportEntitlementId::new(nonzero).unwrap().get(),
            nonzero
        );
        assert_eq!(
            SupportOutstandingCreditVectorId::new(nonzero)
                .unwrap()
                .get(),
            nonzero
        );
        assert_eq!(
            SupportOperationObligationId::new(nonzero).unwrap().get(),
            nonzero
        );
        assert_eq!(PhysicalStartCreditId::new(nonzero).unwrap().get(), nonzero);
        assert_eq!(
            StalePlanDispositionBoundId::new(nonzero).unwrap().get(),
            nonzero
        );
        assert_eq!(
            PersistentStateIsolationEvidenceId::new(nonzero)
                .unwrap()
                .get(),
            nonzero
        );
    }
}
