use crate::{
    BoundedVec, CancellationFactId, CandidateCoordinates, CandidateId, CapabilityKey,
    DomainValueError, Duration, FormationDomainId, GenerationVector, MembershipEventId,
    MonotonicTime, PlanCausalEventId, RequestId, RuntimeOverheadBoundSetId, SchedulingSnapshot,
    TokenCount, TurnPlanId,
};
use std::mem::{align_of, offset_of, size_of};

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

/// One generation-checked retained RequestBook source record.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(C)]
pub struct SourceRecordRef {
    pub(crate) slot: u16,
    pub(crate) reserved: u16,
    pub(crate) generation: u32,
}

impl SourceRecordRef {
    pub(crate) const ABSENT: Self = Self {
        slot: 0,
        reserved: 0,
        generation: 0,
    };

    pub(crate) const fn from_canonical_parts(slot: u16, generation: u32) -> Self {
        Self {
            slot,
            reserved: 0,
            generation,
        }
    }

    #[must_use]
    pub const fn slot(self) -> u16 {
        self.slot
    }

    #[must_use]
    pub const fn generation(self) -> u32 {
        self.generation
    }

    pub(crate) const fn is_absent(self) -> bool {
        self.slot == 0 && self.reserved == 0 && self.generation == 0
    }
}

/// One of the five C17 root families.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(u8)]
pub enum PlanBranch {
    Observation = 0,
    Continuation = 1,
    Rejection = 2,
    Standalone = 3,
    Terminal = 4,
}

impl PlanBranch {
    #[must_use]
    pub const fn ordinal(self) -> u8 {
        self as u8
    }
}

/// A generation- and version-bearing reference to one current root.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(C)]
pub struct RootRef {
    slot: u32,
    generation: u32,
    version: u64,
}

impl RootRef {
    pub fn new(slot: u32, generation: u32, version: u64) -> Result<Self, DomainValueError> {
        if generation == 0 || version == 0 {
            return Err(DomainValueError::Zero);
        }
        Ok(Self {
            slot,
            generation,
            version,
        })
    }

    #[must_use]
    pub const fn slot(self) -> u32 {
        self.slot
    }

    #[must_use]
    pub const fn generation(self) -> u32 {
        self.generation
    }

    #[must_use]
    pub const fn version(self) -> u64 {
        self.version
    }
}

/// A typed, nonzero impossibility reason retained in a close Formation.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(transparent)]
pub struct TypedImpossible(u8);

impl TypedImpossible {
    pub const CAUSAL_CALL: Self = Self(1);
    pub const NO_CONTINUATION_AFTER_OBSERVATION: Self = Self(2);
    pub const TERMINAL_MEMBERSHIP: Self = Self(3);

    pub fn new(value: u8) -> Result<Self, DomainValueError> {
        (value != 0)
            .then_some(Self(value))
            .ok_or(DomainValueError::Zero)
    }

    #[must_use]
    pub const fn get(self) -> u8 {
        self.0
    }
}

/// The exact causal authority accepted by a typed root close.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CloseAuthority {
    Plan {
        identity: TurnPlanIdentity,
        event: PlanCausalEventId,
    },
    Standalone {
        domain: FormationDomainId,
        source: SourceRecordRef,
        event: MembershipEventId,
    },
    Cancellation {
        fact: CancellationFactId,
        event: MembershipEventId,
        request_generation: crate::request_book::RequestBookGeneration,
    },
}

/// A complete typed-close request over one exact current root.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TypedCloseInput {
    pub group: u32,
    pub branch: PlanBranch,
    pub root: RootRef,
    pub occurred_at: MonotonicTime,
    pub reason: TypedImpossible,
    pub authority: CloseAuthority,
}

#[cfg(turnvector_c17_probe)]
pub(crate) fn b03_probe_rows() -> Vec<(&'static str, usize)> {
    vec![
        ("abi.close_authority.align", align_of::<CloseAuthority>()),
        ("abi.close_authority.size", size_of::<CloseAuthority>()),
        ("abi.root_ref.align", align_of::<RootRef>()),
        ("abi.root_ref.generation", offset_of!(RootRef, generation)),
        ("abi.root_ref.size", size_of::<RootRef>()),
        ("abi.root_ref.slot", offset_of!(RootRef, slot)),
        ("abi.root_ref.version", offset_of!(RootRef, version)),
        ("abi.typed_impossible.align", align_of::<TypedImpossible>()),
        ("abi.typed_impossible.size", size_of::<TypedImpossible>()),
    ]
}

const _: () = {
    assert!(size_of::<RootRef>() == 16);
    assert!(align_of::<RootRef>() == 8);
    assert!(offset_of!(RootRef, slot) == 0);
    assert!(offset_of!(RootRef, generation) == 4);
    assert!(offset_of!(RootRef, version) == 8);
    assert!(size_of::<TypedImpossible>() == 1);
    assert!(align_of::<TypedImpossible>() == 1);
    assert!(size_of::<CloseAuthority>() == 240);
    assert!(align_of::<CloseAuthority>() == 16);
};

copy_record!(PlanMemberFunding {
    request_id => RequestId, entitlement => FutureTurnSupportEntitlementId,
    credit_vector => SupportOutstandingCreditVectorId,
});

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PlanSupportObligation<const MEMBERS: usize> {
    pub id: SupportOperationObligationId,
    pub physical_credit: PhysicalStartCreditId,
    pub funders: BoundedVec<PlanMemberFunding, MEMBERS>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
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
    fn c17_public_close_abi_is_exact() {
        assert_eq!(size_of::<RootRef>(), 16);
        assert_eq!(align_of::<RootRef>(), 8);
        assert_eq!(offset_of!(RootRef, slot), 0);
        assert_eq!(offset_of!(RootRef, generation), 4);
        assert_eq!(offset_of!(RootRef, version), 8);
        assert_eq!(size_of::<TypedImpossible>(), 1);
        assert_eq!(align_of::<TypedImpossible>(), 1);
        assert_eq!(size_of::<CloseAuthority>(), 240);
        assert_eq!(align_of::<CloseAuthority>(), 16);
    }

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
