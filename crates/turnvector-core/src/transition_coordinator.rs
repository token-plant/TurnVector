//! Cross-owner C17 transition preparation and infallible commit coordination.
//!
//! RequestBook remains the sole owner of request membership and causal facts;
//! Support remains the sole owner of roots, conservation, and retained history.
//! This module only holds both immutable preparations long enough to revalidate
//! them and commit one Core transition.

use std::cmp::Ordering;

use crate::c17_layout::{Assignment, DestinationKind, ORDINARY_ASSIGNMENTS};
use crate::core::{C17LifecycleRecordSpec, C17LifecycleRootSpec};
use crate::request_book::c17::{
    CancellationMarker, EligibilityMarker, InitialReadyMarker, MembershipEventInput,
    MergeInitialMarker, PreparedCancellation,
    PreparedCreateStandalone as PreparedRequestCreateStandalone,
    PreparedMembershipEvent as PreparedRequestMembershipEvent,
    PreparedMergeInitial as PreparedRequestMergeInitial, PreparedNewlyEligible,
};
use crate::request_book::{RequestBook, RequestBookGeneration, RequestError};
use crate::reusable::AssignmentOrderKey;
use crate::support::c17::{
    LifecycleAggregate, LifecycleRecordInput, ObservationResolution, PlanDisposition,
    PreparedLifecycleAbort, PreparedLifecycleStage, RootAction,
};
use crate::support::{
    PreparedC17CreateStandalone, PreparedC17LifecycleBegin, PreparedC17LifecycleFinalize,
    PreparedC17MembershipTopology, PreparedC17MergeInitial, PreparedC17PlanCreate,
    PreparedC17RootBatch, SupportChargeLedger, SupportLedgerError,
};
use crate::work::{ExactWorkCensus, WorkRecorder};
use crate::{
    CloseAuthority, FixedStorageError, MonotonicTime, RequestId, RequestStatusVersion,
    SupportLedgerGeneration, TurnPlan, TurnPlanIdentity, TypedCloseInput, WorkMeter,
};

pub(crate) trait AssignmentSource {
    fn visit_assignments(&self, visitor: &mut dyn FnMut(AssignmentOrderKey, Assignment));
}

macro_rules! assignment_source {
    ($($ty:ty),+ $(,)?) => {
        $(impl AssignmentSource for $ty {
            fn visit_assignments(
                &self,
                visitor: &mut dyn FnMut(AssignmentOrderKey, Assignment),
            ) {
                <$ty>::visit_assignments(self, visitor);
            }
        })+
    };
}

assignment_source!(
    PreparedRequestCreateStandalone,
    PreparedRequestMergeInitial,
    PreparedRequestMembershipEvent,
    PreparedCancellation,
    PreparedNewlyEligible,
    PreparedC17PlanCreate,
    PreparedC17RootBatch,
    PreparedC17CreateStandalone,
    PreparedC17MergeInitial,
    PreparedC17MembershipTopology,
    PreparedLifecycleStage,
    PreparedLifecycleAbort,
);

/// The fixed, globally ordered cross-owner Patricia journal. Every retained
/// owner plan is first validated as nine slots per semantic edit plus one
/// generation slot. Construction then compacts real destinations into global
/// semantic order and leaves the remaining accepted 393 slots canonical Noops.
#[derive(Debug, Eq, PartialEq)]
struct CombinedAssignmentJournal {
    assignments: [Assignment; ORDINARY_ASSIGNMENTS],
}

struct CombinedCommitPermit {
    support_before: SupportLedgerGeneration,
    support_after: SupportLedgerGeneration,
    request_before: RequestBookGeneration,
    request_after: RequestBookGeneration,
}

pub(crate) trait CombinedRequestOwner: AssignmentSource {
    fn generation_after(&self) -> RequestBookGeneration;
}

pub(crate) trait CombinedSupportOwner: AssignmentSource {
    fn generation_after(&self) -> SupportLedgerGeneration;
}

macro_rules! combined_request_owner {
    ($($ty:ty),+ $(,)?) => {
        $(impl CombinedRequestOwner for $ty {
            fn generation_after(&self) -> RequestBookGeneration {
                <$ty>::generation_after(self)
            }
        })+
    };
}

macro_rules! combined_support_owner {
    ($($ty:ty),+ $(,)?) => {
        $(impl CombinedSupportOwner for $ty {
            fn generation_after(&self) -> SupportLedgerGeneration {
                <$ty>::generation_after(self)
            }
        })+
    };
}

combined_request_owner!(
    PreparedRequestCreateStandalone,
    PreparedRequestMergeInitial,
    PreparedRequestMembershipEvent,
    PreparedCancellation,
);

combined_support_owner!(
    PreparedC17CreateStandalone,
    PreparedC17MergeInitial,
    PreparedC17MembershipTopology,
);

impl CombinedAssignmentJournal {
    fn supported_arena(arena: u16) -> bool {
        matches!(arena, 1..=3 | 20..=22)
    }

    fn valid_order_binding(order: AssignmentOrderKey, assignment: Assignment) -> bool {
        Self::supported_arena(order.arena_id())
            && order.arena_id() == assignment.destination_arena
            && (order.is_generation()
                == (assignment.destination_kind == DestinationKind::Header as u8))
            && (!order.is_generation()
                || (order.assignment_ordinal() == 0 && assignment.destination_slot == 0))
    }

    fn valid_order_pair(prior: AssignmentOrderKey, current: AssignmentOrderKey) -> bool {
        match prior.semantic_cmp(&current) {
            Ordering::Less => true,
            Ordering::Greater => false,
            Ordering::Equal if prior.is_generation() || current.is_generation() => {
                prior.is_generation()
                    && current.is_generation()
                    && prior.assignment_ordinal() == 0
                    && current.assignment_ordinal() == 0
                    && prior.arena_id() < current.arena_id()
            }
            Ordering::Equal => {
                prior.arena_id() == current.arena_id()
                    && prior.assignment_ordinal() < current.assignment_ordinal()
            }
        }
    }

    fn single<Source: AssignmentSource>(source: &Source) -> Result<Self, FixedStorageError> {
        Self::collect(|visitor| source.visit_assignments(visitor))
    }

    fn new<Request: AssignmentSource, Support: AssignmentSource>(
        request_change: &Request,
        support_change: &Support,
    ) -> Result<Self, FixedStorageError> {
        Self::collect(|visitor| {
            support_change.visit_assignments(visitor);
            request_change.visit_assignments(visitor);
        })
    }

    fn collect(
        visit: impl FnOnce(&mut dyn FnMut(AssignmentOrderKey, Assignment)),
    ) -> Result<Self, FixedStorageError> {
        let mut assignments = [Assignment::NOOP; ORDINARY_ASSIGNMENTS];
        let mut order_keys = [AssignmentOrderKey::ZERO; ORDINARY_ASSIGNMENTS];
        let mut logical_len = 0usize;
        let mut len = 0usize;
        let mut overflow = false;
        {
            let mut push = |order_key: AssignmentOrderKey, assignment: Assignment| {
                logical_len = match logical_len.checked_add(1) {
                    Some(value) => value,
                    None => {
                        overflow = true;
                        return;
                    }
                };
                if logical_len > ORDINARY_ASSIGNMENTS {
                    overflow = true;
                    return;
                }
                if assignment == Assignment::NOOP {
                    return;
                }
                if len == ORDINARY_ASSIGNMENTS {
                    overflow = true;
                    return;
                }
                order_keys[len] = order_key;
                assignments[len] = assignment;
                len += 1;
            };
            visit(&mut push);
        }
        if overflow {
            return Err(FixedStorageError::Capacity);
        }
        for index in 1..len {
            let mut cursor = index;
            while cursor > 0 && order_keys[cursor] < order_keys[cursor - 1] {
                order_keys.swap(cursor - 1, cursor);
                assignments.swap(cursor - 1, cursor);
                cursor -= 1;
            }
        }
        if len == 0
            || order_keys[..len]
                .windows(2)
                .any(|pair| !Self::valid_order_pair(pair[0], pair[1]))
            || order_keys[..len]
                .iter()
                .copied()
                .zip(assignments[..len].iter().copied())
                .any(|(order, assignment)| !Self::valid_order_binding(order, assignment))
            || assignments[..len]
                .iter()
                .any(|assignment| !assignment.validate())
            || assignments[len..]
                .iter()
                .any(|assignment| *assignment != Assignment::NOOP)
            || order_keys[len..]
                .iter()
                .any(|key| *key != AssignmentOrderKey::ZERO)
        {
            return Err(FixedStorageError::NonCanonical);
        }
        let journal = Self { assignments };
        journal
            .canonical()
            .then_some(journal)
            .ok_or(FixedStorageError::NonCanonical)
    }

    fn active_len(&self) -> usize {
        self.assignments
            .iter()
            .rposition(|assignment| *assignment != Assignment::NOOP)
            .map_or(0, |index| index + 1)
    }

    fn canonical(&self) -> bool {
        let active_len = self.active_len();
        active_len > 0
            && self.assignments[..active_len].iter().all(|assignment| {
                *assignment != Assignment::NOOP
                    && assignment.validate()
                    && Self::supported_arena(assignment.destination_arena)
            })
            && self.assignments[active_len..]
                .iter()
                .all(|assignment| *assignment == Assignment::NOOP)
    }

    fn matches_source<Source: AssignmentSource>(&self, source: &Source) -> bool {
        self.canonical()
            && Self::single(source).is_ok_and(|expected| expected.assignments == self.assignments)
    }

    fn matches_sources<Request: AssignmentSource, Support: AssignmentSource>(
        &self,
        request: &Request,
        support: &Support,
    ) -> bool {
        self.canonical()
            && Self::new(request, support)
                .is_ok_and(|expected| expected.assignments == self.assignments)
    }

    fn commit_request_direct<
        const REQUESTS: usize,
        const INPUT: usize,
        const STOPS: usize,
        const STOP_TOKENS: usize,
    >(
        self,
        requests: &mut RequestBook<REQUESTS, INPUT, STOPS, STOP_TOKENS>,
    ) {
        for assignment in &self.assignments {
            if *assignment == Assignment::NOOP {
                break;
            }
            match assignment.destination_arena {
                20..=22 => requests.commit_c17_assignment_direct(assignment),
                _ => unreachable!("validated RequestBook assignment arena"),
            }
        }
    }

    fn commit_support_direct<const RECORDS: usize, const CLAIMS: usize, const HORIZONS: usize>(
        self,
        support: &mut SupportChargeLedger<RECORDS, CLAIMS, HORIZONS>,
    ) {
        for assignment in &self.assignments {
            if *assignment == Assignment::NOOP {
                break;
            }
            match assignment.destination_arena {
                1..=3 => support.commit_c17_assignment_direct(assignment),
                _ => unreachable!("validated Support assignment arena"),
            }
        }
    }

    fn commit_support_direct_boxed<
        const RECORDS: usize,
        const CLAIMS: usize,
        const HORIZONS: usize,
    >(
        self: Box<Self>,
        support: &mut SupportChargeLedger<RECORDS, CLAIMS, HORIZONS>,
    ) {
        for assignment in &self.assignments {
            if *assignment == Assignment::NOOP {
                break;
            }
            match assignment.destination_arena {
                1..=3 => support.commit_c17_assignment_direct(assignment),
                _ => unreachable!("validated Support assignment arena"),
            }
        }
    }

    fn commit_direct<
        const REQUESTS: usize,
        const INPUT: usize,
        const STOPS: usize,
        const STOP_TOKENS: usize,
        const RECORDS: usize,
        const CLAIMS: usize,
        const HORIZONS: usize,
    >(
        self,
        requests: &mut RequestBook<REQUESTS, INPUT, STOPS, STOP_TOKENS>,
        support: &mut SupportChargeLedger<RECORDS, CLAIMS, HORIZONS>,
    ) {
        for assignment in &self.assignments {
            if *assignment == Assignment::NOOP {
                break;
            }
            match assignment.destination_arena {
                1..=3 => support.commit_c17_assignment_direct(assignment),
                20..=22 => requests.commit_c17_assignment_direct(assignment),
                _ => unreachable!("validated combined assignment arena"),
            }
        }
    }
}

/// A single-use, non-`Clone` seal retaining both validated owner changes and
/// their one canonical fixed journal until commit.
pub(crate) struct CombinedOwnerSeal<Request, Support> {
    request: Request,
    support: Support,
    journal: CombinedAssignmentJournal,
    support_before: SupportLedgerGeneration,
    support_after: SupportLedgerGeneration,
    request_before: RequestBookGeneration,
    request_after: RequestBookGeneration,
}

impl<Request: CombinedRequestOwner, Support: CombinedSupportOwner>
    CombinedOwnerSeal<Request, Support>
{
    fn new(
        request: Request,
        support: Support,
        support_before: SupportLedgerGeneration,
        request_before: RequestBookGeneration,
    ) -> Result<Self, FixedStorageError> {
        let support_after = support.generation_after();
        let request_after = request.generation_after();
        if support_before.next().ok() != Some(support_after)
            || request_before.next().ok() != Some(request_after)
        {
            return Err(FixedStorageError::NonCanonical);
        }
        let journal = CombinedAssignmentJournal::new(&request, &support)?;
        Ok(Self {
            request,
            support,
            journal,
            support_before,
            support_after,
            request_before,
            request_after,
        })
    }

    #[cfg(test)]
    pub(crate) fn assignment_census(&self) -> ([usize; 6], [usize; 6], usize) {
        let mut edits = [0usize; 6];
        let mut generations = [0usize; 6];
        let mut slots = 0usize;
        let mut visit = |order: AssignmentOrderKey, _assignment: Assignment| {
            slots += 1;
            if order.assignment_ordinal() != 0 {
                return;
            }
            let family = match order.arena_id() {
                1 => 0,
                3 => 1,
                2 => 2,
                20 => 3,
                21 => 4,
                22 => 5,
                _ => panic!("unexpected C17 assignment arena"),
            };
            if order.is_generation() {
                generations[family] += 1;
            } else {
                edits[family] += 1;
            }
        };
        self.support.visit_assignments(&mut visit);
        self.request.visit_assignments(&mut visit);
        (edits, generations, slots)
    }

    fn matches(
        &self,
        support_generation: SupportLedgerGeneration,
        request_generation: RequestBookGeneration,
    ) -> bool {
        self.support_before == support_generation
            && self.request_before == request_generation
            && self.support_after == self.support.generation_after()
            && self.request_after == self.request.generation_after()
            && support_generation.next().ok() == Some(self.support_after)
            && request_generation.next().ok() == Some(self.request_after)
            && self.journal.matches_sources(&self.request, &self.support)
    }
}

fn combined_commit_permit(
    journal: &CombinedAssignmentJournal,
    support_before: SupportLedgerGeneration,
    support_after: SupportLedgerGeneration,
    request_before: RequestBookGeneration,
    request_after: RequestBookGeneration,
) -> CombinedCommitPermit {
    assert!(
        journal.canonical(),
        "validated combined C17 assignment journal"
    );
    assert_eq!(
        support_before.next().ok(),
        Some(support_after),
        "sealed Support generation continuity"
    );
    assert_eq!(
        request_before.next().ok(),
        Some(request_after),
        "sealed RequestBook generation continuity"
    );
    CombinedCommitPermit {
        support_before,
        support_after,
        request_before,
        request_after,
    }
}

pub(crate) struct PreparedPlanCreate {
    support: PreparedC17PlanCreate,
    journal: Box<CombinedAssignmentJournal>,
}

pub(crate) fn prepare_plan_create<
    const RECORDS: usize,
    const CLAIMS: usize,
    const HORIZONS: usize,
    const MEMBERS: usize,
>(
    support: &SupportChargeLedger<RECORDS, CLAIMS, HORIZONS>,
    plan: &TurnPlan<MEMBERS>,
    occurred_at: MonotonicTime,
    work: &mut WorkMeter,
) -> Result<Box<PreparedPlanCreate>, SupportLedgerError> {
    let mut census = ExactWorkCensus::new();
    let change =
        support.prepare_c17_plan_create(support.generation(), plan, occurred_at, &mut census)?;
    support.validate_c17_plan_create(&change)?;
    let journal =
        Box::new(CombinedAssignmentJournal::single(&change).map_err(SupportLedgerError::Storage)?);
    work.charge(census.witness())?;
    Ok(Box::new(PreparedPlanCreate {
        support: change,
        journal,
    }))
}

pub(crate) fn validate_plan_create<
    const RECORDS: usize,
    const CLAIMS: usize,
    const HORIZONS: usize,
>(
    support: &SupportChargeLedger<RECORDS, CLAIMS, HORIZONS>,
    change: &PreparedPlanCreate,
) -> Result<(), SupportLedgerError> {
    if !change.journal.matches_source(&change.support) {
        return Err(SupportLedgerError::Generation);
    }
    support.validate_c17_plan_create(&change.support)
}

pub(crate) fn commit_plan_create<
    const RECORDS: usize,
    const CLAIMS: usize,
    const HORIZONS: usize,
>(
    support: &mut SupportChargeLedger<RECORDS, CLAIMS, HORIZONS>,
    change: Box<PreparedPlanCreate>,
) {
    assert_eq!(
        support.generation(),
        change.support.expected_generation(),
        "sealed coordinated Plan generation"
    );
    let PreparedPlanCreate {
        support: change,
        journal,
    } = *change;
    journal.commit_support_direct(support);
    support.commit_c17_plan_create_prevalidated(change, false);
}

pub(crate) type PreparedCreateStandalone =
    CombinedOwnerSeal<PreparedRequestCreateStandalone, PreparedC17CreateStandalone>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CreateStandalonePrepareError {
    Request(RequestError),
    Support(SupportLedgerError),
}

pub(crate) fn prepare_create_standalone<
    const REQUESTS: usize,
    const INPUT: usize,
    const STOPS: usize,
    const STOP_TOKENS: usize,
    const RECORDS: usize,
    const CLAIMS: usize,
    const HORIZONS: usize,
>(
    requests: &RequestBook<REQUESTS, INPUT, STOPS, STOP_TOKENS>,
    support: &SupportChargeLedger<RECORDS, CLAIMS, HORIZONS>,
    marker: InitialReadyMarker,
    work: &mut WorkMeter,
) -> Result<PreparedCreateStandalone, CreateStandalonePrepareError> {
    let domain = crate::FormationDomainId::new(u128::from_be_bytes(marker.domain))
        .map_err(|_| CreateStandalonePrepareError::Request(RequestError::InvalidTransition))?;
    let anchor = support
        .preview_c17_create_standalone_anchor(domain)
        .map_err(CreateStandalonePrepareError::Support)?;
    let request = requests
        .prepare_create_standalone(marker, anchor)
        .map_err(CreateStandalonePrepareError::Request)?;
    let mut census = ExactWorkCensus::new();
    let support_change = support
        .prepare_c17_create_standalone(support.generation(), request.event(), marker, &mut census)
        .map_err(CreateStandalonePrepareError::Support)?;
    requests
        .validate_create_standalone(&request)
        .map_err(CreateStandalonePrepareError::Request)?;
    support
        .validate_c17_create_standalone(&support_change)
        .map_err(CreateStandalonePrepareError::Support)?;
    let sealed = CombinedOwnerSeal::new(
        request,
        support_change,
        support.generation(),
        requests.generation(),
    )
    .map_err(|error| CreateStandalonePrepareError::Support(SupportLedgerError::Storage(error)))?;
    work.charge(census.witness())
        .map_err(SupportLedgerError::from)
        .map_err(CreateStandalonePrepareError::Support)?;
    Ok(sealed)
}

#[allow(
    dead_code,
    reason = "C17 exposes an immutable seal validation seam for owner adapters"
)]
pub(crate) fn validate_create_standalone<
    const REQUESTS: usize,
    const INPUT: usize,
    const STOPS: usize,
    const STOP_TOKENS: usize,
    const RECORDS: usize,
    const CLAIMS: usize,
    const HORIZONS: usize,
>(
    requests: &RequestBook<REQUESTS, INPUT, STOPS, STOP_TOKENS>,
    support: &SupportChargeLedger<RECORDS, CLAIMS, HORIZONS>,
    change: &PreparedCreateStandalone,
) -> Result<(), CreateStandalonePrepareError> {
    if !change.matches(support.generation(), requests.generation()) {
        return Err(CreateStandalonePrepareError::Support(
            SupportLedgerError::Generation,
        ));
    }
    requests
        .validate_create_standalone(&change.request)
        .map_err(CreateStandalonePrepareError::Request)?;
    support
        .validate_c17_create_standalone(&change.support)
        .map_err(CreateStandalonePrepareError::Support)
}

pub(crate) fn commit_create_standalone<
    const REQUESTS: usize,
    const INPUT: usize,
    const STOPS: usize,
    const STOP_TOKENS: usize,
    const RECORDS: usize,
    const CLAIMS: usize,
    const HORIZONS: usize,
>(
    requests: &mut RequestBook<REQUESTS, INPUT, STOPS, STOP_TOKENS>,
    support: &mut SupportChargeLedger<RECORDS, CLAIMS, HORIZONS>,
    change: PreparedCreateStandalone,
) {
    assert_eq!(
        support.generation(),
        change.support_before,
        "sealed CreateStandalone Support generation"
    );
    assert_eq!(
        requests.generation(),
        change.request_before,
        "sealed CreateStandalone RequestBook generation"
    );
    let CombinedOwnerSeal {
        request,
        support: support_change,
        journal,
        support_before,
        support_after,
        request_before,
        request_after,
    } = change;
    let permit = combined_commit_permit(
        &journal,
        support_before,
        support_after,
        request_before,
        request_after,
    );
    journal.commit_direct(requests, support);
    support.commit_c17_create_standalone_prevalidated(
        support_change,
        permit.support_before,
        permit.support_after,
        false,
    );
    requests.commit_create_standalone_prevalidated(
        request,
        permit.request_before,
        permit.request_after,
        false,
    );
}

pub(crate) type PreparedMergeInitial =
    CombinedOwnerSeal<PreparedRequestMergeInitial, PreparedC17MergeInitial>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MergeInitialPrepareError {
    Request(RequestError),
    Support(SupportLedgerError),
}

pub(crate) fn prepare_merge_initial<
    const REQUESTS: usize,
    const INPUT: usize,
    const STOPS: usize,
    const STOP_TOKENS: usize,
    const RECORDS: usize,
    const CLAIMS: usize,
    const HORIZONS: usize,
>(
    requests: &RequestBook<REQUESTS, INPUT, STOPS, STOP_TOKENS>,
    support: &SupportChargeLedger<RECORDS, CLAIMS, HORIZONS>,
    marker: MergeInitialMarker,
    work: &mut WorkMeter,
) -> Result<PreparedMergeInitial, MergeInitialPrepareError> {
    let (anchors, source_count) = requests
        .merge_initial_source_anchors(marker)
        .map_err(MergeInitialPrepareError::Request)?;
    let preview = support
        .preview_c17_merge_initial(
            support.generation(),
            anchors,
            source_count,
            marker.domain,
            marker.occurred_at,
        )
        .map_err(MergeInitialPrepareError::Support)?;
    let request = requests
        .prepare_merge_initial(marker, preview.destination())
        .map_err(MergeInitialPrepareError::Request)?;
    let mut census = ExactWorkCensus::new();
    let support_change = support
        .prepare_c17_merge_initial(support.generation(), preview, *request.event(), &mut census)
        .map_err(MergeInitialPrepareError::Support)?;
    requests
        .validate_merge_initial(&request)
        .map_err(MergeInitialPrepareError::Request)?;
    support
        .validate_c17_merge_initial(&support_change)
        .map_err(MergeInitialPrepareError::Support)?;
    let sealed = CombinedOwnerSeal::new(
        request,
        support_change,
        support.generation(),
        requests.generation(),
    )
    .map_err(|error| MergeInitialPrepareError::Support(SupportLedgerError::Storage(error)))?;
    work.charge(census.witness())
        .map_err(SupportLedgerError::from)
        .map_err(MergeInitialPrepareError::Support)?;
    Ok(sealed)
}

#[allow(
    dead_code,
    reason = "C17 exposes an immutable seal validation seam for owner adapters"
)]
pub(crate) fn validate_merge_initial<
    const REQUESTS: usize,
    const INPUT: usize,
    const STOPS: usize,
    const STOP_TOKENS: usize,
    const RECORDS: usize,
    const CLAIMS: usize,
    const HORIZONS: usize,
>(
    requests: &RequestBook<REQUESTS, INPUT, STOPS, STOP_TOKENS>,
    support: &SupportChargeLedger<RECORDS, CLAIMS, HORIZONS>,
    change: &PreparedMergeInitial,
) -> Result<(), MergeInitialPrepareError> {
    if !change.matches(support.generation(), requests.generation()) {
        return Err(MergeInitialPrepareError::Support(
            SupportLedgerError::Generation,
        ));
    }
    requests
        .validate_merge_initial(&change.request)
        .map_err(MergeInitialPrepareError::Request)?;
    support
        .validate_c17_merge_initial(&change.support)
        .map_err(MergeInitialPrepareError::Support)
}

pub(crate) fn commit_merge_initial<
    const REQUESTS: usize,
    const INPUT: usize,
    const STOPS: usize,
    const STOP_TOKENS: usize,
    const RECORDS: usize,
    const CLAIMS: usize,
    const HORIZONS: usize,
>(
    requests: &mut RequestBook<REQUESTS, INPUT, STOPS, STOP_TOKENS>,
    support: &mut SupportChargeLedger<RECORDS, CLAIMS, HORIZONS>,
    change: PreparedMergeInitial,
) {
    assert_eq!(
        support.generation(),
        change.support_before,
        "sealed MergeInitial Support generation"
    );
    assert_eq!(
        requests.generation(),
        change.request_before,
        "sealed MergeInitial RequestBook generation"
    );
    let CombinedOwnerSeal {
        request,
        support: support_change,
        journal,
        support_before,
        support_after,
        request_before,
        request_after,
    } = change;
    let permit = combined_commit_permit(
        &journal,
        support_before,
        support_after,
        request_before,
        request_after,
    );
    journal.commit_direct(requests, support);
    support.commit_c17_merge_initial_prevalidated(
        support_change,
        permit.support_before,
        permit.support_after,
        false,
    );
    requests.commit_merge_initial_prevalidated(
        request,
        permit.request_before,
        permit.request_after,
        false,
    );
}

pub(crate) type PreparedMembershipTopology =
    CombinedOwnerSeal<PreparedRequestMembershipEvent, PreparedC17MembershipTopology>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MembershipTopologyPrepareError {
    Request(RequestError),
    Support(SupportLedgerError),
}

pub(crate) fn prepare_membership_topology<
    const REQUESTS: usize,
    const INPUT: usize,
    const STOPS: usize,
    const STOP_TOKENS: usize,
    const RECORDS: usize,
    const CLAIMS: usize,
    const HORIZONS: usize,
>(
    requests: &RequestBook<REQUESTS, INPUT, STOPS, STOP_TOKENS>,
    support: &SupportChargeLedger<RECORDS, CLAIMS, HORIZONS>,
    input: MembershipEventInput,
    work: &mut WorkMeter,
) -> Result<PreparedMembershipTopology, MembershipTopologyPrepareError> {
    let intent = requests
        .prepare_membership_event(input)
        .map_err(MembershipTopologyPrepareError::Request)?;
    requests
        .validate_membership_intent(&intent)
        .map_err(MembershipTopologyPrepareError::Request)?;
    let preview = support
        .preview_c17_membership_topology(support.generation(), &intent)
        .map_err(MembershipTopologyPrepareError::Support)?;
    let destinations = preview.destination_anchors();
    let destination_count = preview.destination_count();
    let request = requests
        .seal_membership_event(intent, destinations, destination_count)
        .map_err(MembershipTopologyPrepareError::Request)?;
    requests
        .validate_membership_event(&request)
        .map_err(MembershipTopologyPrepareError::Request)?;
    let mut census = ExactWorkCensus::new();
    let support_change = support
        .prepare_c17_membership_topology(
            support.generation(),
            preview,
            *request.event(),
            &mut census,
        )
        .map_err(MembershipTopologyPrepareError::Support)?;
    requests
        .validate_membership_event(&request)
        .map_err(MembershipTopologyPrepareError::Request)?;
    support
        .validate_c17_membership_topology(&support_change)
        .map_err(MembershipTopologyPrepareError::Support)?;
    let sealed = CombinedOwnerSeal::new(
        request,
        support_change,
        support.generation(),
        requests.generation(),
    )
    .map_err(|error| MembershipTopologyPrepareError::Support(SupportLedgerError::Storage(error)))?;
    work.charge(census.witness())
        .map_err(SupportLedgerError::from)
        .map_err(MembershipTopologyPrepareError::Support)?;
    Ok(sealed)
}

#[allow(
    dead_code,
    reason = "C17 exposes an immutable seal validation seam for owner adapters"
)]
pub(crate) fn validate_membership_topology<
    const REQUESTS: usize,
    const INPUT: usize,
    const STOPS: usize,
    const STOP_TOKENS: usize,
    const RECORDS: usize,
    const CLAIMS: usize,
    const HORIZONS: usize,
>(
    requests: &RequestBook<REQUESTS, INPUT, STOPS, STOP_TOKENS>,
    support: &SupportChargeLedger<RECORDS, CLAIMS, HORIZONS>,
    change: &PreparedMembershipTopology,
) -> Result<(), MembershipTopologyPrepareError> {
    if !change.matches(support.generation(), requests.generation()) {
        return Err(MembershipTopologyPrepareError::Support(
            SupportLedgerError::Generation,
        ));
    }
    requests
        .validate_membership_event(&change.request)
        .map_err(MembershipTopologyPrepareError::Request)?;
    support
        .validate_c17_membership_topology(&change.support)
        .map_err(MembershipTopologyPrepareError::Support)
}

pub(crate) fn commit_membership_topology<
    const REQUESTS: usize,
    const INPUT: usize,
    const STOPS: usize,
    const STOP_TOKENS: usize,
    const RECORDS: usize,
    const CLAIMS: usize,
    const HORIZONS: usize,
>(
    requests: &mut RequestBook<REQUESTS, INPUT, STOPS, STOP_TOKENS>,
    support: &mut SupportChargeLedger<RECORDS, CLAIMS, HORIZONS>,
    change: PreparedMembershipTopology,
) {
    assert_eq!(
        support.generation(),
        change.support_before,
        "sealed membership-topology Support generation"
    );
    assert_eq!(
        requests.generation(),
        change.request_before,
        "sealed membership-topology RequestBook generation"
    );
    let CombinedOwnerSeal {
        request,
        support: support_change,
        journal,
        support_before,
        support_after,
        request_before,
        request_after,
    } = change;
    let permit = combined_commit_permit(
        &journal,
        support_before,
        support_after,
        request_before,
        request_after,
    );
    journal.commit_direct(requests, support);
    support.commit_c17_membership_topology_prevalidated(
        support_change,
        permit.support_before,
        permit.support_after,
        false,
    );
    requests.commit_membership_event_prevalidated(
        request,
        permit.request_before,
        permit.request_after,
        false,
    );
}

pub(crate) fn newly_eligible<
    const REQUESTS: usize,
    const INPUT: usize,
    const STOPS: usize,
    const STOP_TOKENS: usize,
>(
    requests: &mut RequestBook<REQUESTS, INPUT, STOPS, STOP_TOKENS>,
    marker: EligibilityMarker,
    work: &mut WorkMeter,
) -> Result<(), RequestError> {
    let mut census = ExactWorkCensus::new();
    let change = requests.prepare_newly_eligible(marker, &mut census)?;
    requests.validate_newly_eligible(&change)?;
    let journal = CombinedAssignmentJournal::single(&change).map_err(RequestError::Storage)?;
    assert!(
        journal.matches_source(&change),
        "sealed NewlyEligible assignment journal"
    );
    work.charge(census.witness())?;
    journal.commit_request_direct(requests);
    requests.commit_newly_eligible_prevalidated(change, false);
    Ok(())
}

#[derive(Debug)]
pub(crate) struct PreparedSupportRootSeal {
    support: PreparedC17RootBatch,
    journal: Box<CombinedAssignmentJournal>,
}

impl PreparedSupportRootSeal {
    fn new(support: PreparedC17RootBatch) -> Result<Box<Self>, SupportLedgerError> {
        let journal = Box::new(
            CombinedAssignmentJournal::single(&support).map_err(SupportLedgerError::Storage)?,
        );
        Ok(Box::new(Self { support, journal }))
    }

    fn validate<const RECORDS: usize, const CLAIMS: usize, const HORIZONS: usize>(
        &self,
        support: &SupportChargeLedger<RECORDS, CLAIMS, HORIZONS>,
    ) -> Result<(), SupportLedgerError> {
        if !self.journal.matches_source(&self.support) {
            return Err(SupportLedgerError::Generation);
        }
        support.validate_c17_root_batch(&self.support)
    }

    fn commit<const RECORDS: usize, const CLAIMS: usize, const HORIZONS: usize>(
        self,
        support: &mut SupportChargeLedger<RECORDS, CLAIMS, HORIZONS>,
    ) {
        assert_eq!(
            support.generation(),
            self.support.expected_generation(),
            "sealed coordinated Support generation"
        );
        self.journal.commit_support_direct_boxed(support);
        support.commit_c17_root_batch_prevalidated(self.support, false);
    }
}

#[derive(Debug)]
pub(crate) struct PreparedMembershipRootAction {
    request: RequestId,
    expected_status: RequestStatusVersion,
    anchor: crate::request_book::c17::SupportMembershipAnchor,
    support: Box<PreparedSupportRootSeal>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MembershipRootPrepareError {
    Request(RequestError),
    Support(SupportLedgerError),
}

pub(crate) fn prepare_membership_root_action<
    const REQUESTS: usize,
    const INPUT: usize,
    const STOPS: usize,
    const STOP_TOKENS: usize,
    const RECORDS: usize,
    const CLAIMS: usize,
    const HORIZONS: usize,
>(
    requests: &RequestBook<REQUESTS, INPUT, STOPS, STOP_TOKENS>,
    support: &SupportChargeLedger<RECORDS, CLAIMS, HORIZONS>,
    request: RequestId,
    expected_status: RequestStatusVersion,
    action: RootAction,
    occurred_at: MonotonicTime,
    work: &mut WorkMeter,
) -> Result<PreparedMembershipRootAction, MembershipRootPrepareError> {
    let anchor = requests
        .c17_membership_anchor(request, expected_status)
        .map_err(MembershipRootPrepareError::Request)?;
    let mut census = ExactWorkCensus::new();
    let support_change = support
        .prepare_c17_membership_root_action(
            support.generation(),
            anchor,
            action,
            occurred_at,
            &mut census,
        )
        .map_err(MembershipRootPrepareError::Support)?;
    requests
        .c17_membership_anchor(request, expected_status)
        .map_err(MembershipRootPrepareError::Request)
        .and_then(|current| {
            (current == anchor)
                .then_some(())
                .ok_or(MembershipRootPrepareError::Request(
                    RequestError::PreparedChangeStale,
                ))
        })?;
    support
        .validate_c17_root_batch(&support_change)
        .map_err(MembershipRootPrepareError::Support)?;
    let support_change = PreparedSupportRootSeal::new(support_change)
        .map_err(MembershipRootPrepareError::Support)?;
    work.charge(census.witness())
        .map_err(SupportLedgerError::from)
        .map_err(MembershipRootPrepareError::Support)?;
    Ok(PreparedMembershipRootAction {
        request,
        expected_status,
        anchor,
        support: support_change,
    })
}

pub(crate) fn commit_membership_root_action<
    const REQUESTS: usize,
    const INPUT: usize,
    const STOPS: usize,
    const STOP_TOKENS: usize,
    const RECORDS: usize,
    const CLAIMS: usize,
    const HORIZONS: usize,
>(
    requests: &RequestBook<REQUESTS, INPUT, STOPS, STOP_TOKENS>,
    support: &mut SupportChargeLedger<RECORDS, CLAIMS, HORIZONS>,
    change: PreparedMembershipRootAction,
) {
    let _ = requests;
    change.support.commit(support);
}

pub(crate) type PreparedCancellationRemove =
    CombinedOwnerSeal<PreparedCancellation, PreparedC17MembershipTopology>;

pub(crate) fn prepare_cancellation_remove<
    const REQUESTS: usize,
    const INPUT: usize,
    const STOPS: usize,
    const STOP_TOKENS: usize,
    const RECORDS: usize,
    const CLAIMS: usize,
    const HORIZONS: usize,
>(
    requests: &RequestBook<REQUESTS, INPUT, STOPS, STOP_TOKENS>,
    support: &SupportChargeLedger<RECORDS, CLAIMS, HORIZONS>,
    marker: CancellationMarker,
    work: &mut WorkMeter,
) -> Result<PreparedCancellationRemove, CancellationPrepareError> {
    let request = requests
        .prepare_cancellation(marker)
        .map_err(CancellationPrepareError::Request)?;
    requests
        .validate_cancellation(&request)
        .map_err(CancellationPrepareError::Request)?;
    let preview = support
        .preview_c17_cancellation_topology(support.generation(), &request)
        .map_err(CancellationPrepareError::Support)?;
    let member_keys = preview.source_member_keys();
    let member_count = preview.source_member_count();
    let survivor = preview.cancellation_survivor();
    let request = requests
        .seal_cancellation(request, member_keys, member_count, survivor)
        .map_err(CancellationPrepareError::Request)?;
    requests
        .validate_cancellation(&request)
        .map_err(CancellationPrepareError::Request)?;
    let mut census = ExactWorkCensus::new();
    let support_change = support
        .prepare_c17_membership_topology(
            support.generation(),
            preview,
            *request.event(),
            &mut census,
        )
        .map_err(CancellationPrepareError::Support)?;
    requests
        .validate_cancellation(&request)
        .map_err(CancellationPrepareError::Request)?;
    support
        .validate_c17_membership_topology(&support_change)
        .map_err(CancellationPrepareError::Support)?;
    let sealed = CombinedOwnerSeal::new(
        request,
        support_change,
        support.generation(),
        requests.generation(),
    )
    .map_err(|error| CancellationPrepareError::Support(SupportLedgerError::Storage(error)))?;
    work.charge(census.witness())
        .map_err(SupportLedgerError::from)
        .map_err(CancellationPrepareError::Support)?;
    Ok(sealed)
}

#[allow(
    dead_code,
    reason = "C17 exposes an immutable seal validation seam for owner adapters"
)]
pub(crate) fn validate_cancellation_remove<
    const REQUESTS: usize,
    const INPUT: usize,
    const STOPS: usize,
    const STOP_TOKENS: usize,
    const RECORDS: usize,
    const CLAIMS: usize,
    const HORIZONS: usize,
>(
    requests: &RequestBook<REQUESTS, INPUT, STOPS, STOP_TOKENS>,
    support: &SupportChargeLedger<RECORDS, CLAIMS, HORIZONS>,
    change: &PreparedCancellationRemove,
) -> Result<(), CancellationPrepareError> {
    if !change.matches(support.generation(), requests.generation()) {
        return Err(CancellationPrepareError::Support(
            SupportLedgerError::Generation,
        ));
    }
    requests
        .validate_cancellation(&change.request)
        .map_err(CancellationPrepareError::Request)?;
    support
        .validate_c17_membership_topology(&change.support)
        .map_err(CancellationPrepareError::Support)
}

pub(crate) fn commit_cancellation_remove<
    const REQUESTS: usize,
    const INPUT: usize,
    const STOPS: usize,
    const STOP_TOKENS: usize,
    const RECORDS: usize,
    const CLAIMS: usize,
    const HORIZONS: usize,
>(
    requests: &mut RequestBook<REQUESTS, INPUT, STOPS, STOP_TOKENS>,
    support: &mut SupportChargeLedger<RECORDS, CLAIMS, HORIZONS>,
    change: PreparedCancellationRemove,
) {
    assert_eq!(
        support.generation(),
        change.support_before,
        "sealed CancellationRemove Support generation"
    );
    assert_eq!(
        requests.generation(),
        change.request_before,
        "sealed CancellationRemove RequestBook generation"
    );
    let CombinedOwnerSeal {
        request,
        support: support_change,
        journal,
        support_before,
        support_after,
        request_before,
        request_after,
    } = change;
    let permit = combined_commit_permit(
        &journal,
        support_before,
        support_after,
        request_before,
        request_after,
    );
    journal.commit_direct(requests, support);
    support.commit_c17_membership_topology_prevalidated(
        support_change,
        permit.support_before,
        permit.support_after,
        false,
    );
    requests.commit_cancellation_prevalidated(
        request,
        permit.request_before,
        permit.request_after,
        false,
    );
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CancellationPrepareError {
    Request(RequestError),
    Support(SupportLedgerError),
}

pub(crate) enum PreparedPlanTransition {
    Disposition(Box<PreparedSupportRootSeal>),
    Root(Box<PreparedSupportRootSeal>),
    Observation(Box<PreparedSupportRootSeal>),
}

pub(crate) fn prepare_plan_disposition<
    const RECORDS: usize,
    const CLAIMS: usize,
    const HORIZONS: usize,
>(
    support: &SupportChargeLedger<RECORDS, CLAIMS, HORIZONS>,
    identity: TurnPlanIdentity,
    disposition: PlanDisposition,
    occurred_at: MonotonicTime,
    work: &mut WorkMeter,
) -> Result<PreparedPlanTransition, SupportLedgerError> {
    let mut census = ExactWorkCensus::new();
    let change = support.prepare_c17_plan_disposition(
        support.generation(),
        identity,
        disposition,
        occurred_at,
        &mut census,
    )?;
    support.validate_c17_root_batch(&change)?;
    let change = PreparedSupportRootSeal::new(change)?;
    work.charge(census.witness())?;
    Ok(PreparedPlanTransition::Disposition(change))
}

pub(crate) fn prepare_plan_root_action<
    const RECORDS: usize,
    const CLAIMS: usize,
    const HORIZONS: usize,
>(
    support: &SupportChargeLedger<RECORDS, CLAIMS, HORIZONS>,
    identity: TurnPlanIdentity,
    branch: u8,
    action: RootAction,
    occurred_at: MonotonicTime,
    work: &mut WorkMeter,
) -> Result<PreparedPlanTransition, SupportLedgerError> {
    let mut census = ExactWorkCensus::new();
    let change = support.prepare_c17_plan_root_action(
        support.generation(),
        identity,
        branch,
        action,
        occurred_at,
        &mut census,
    )?;
    support.validate_c17_root_batch(&change)?;
    let change = PreparedSupportRootSeal::new(change)?;
    work.charge(census.witness())?;
    Ok(PreparedPlanTransition::Root(change))
}

pub(crate) fn prepare_observation_resolution<
    const RECORDS: usize,
    const CLAIMS: usize,
    const HORIZONS: usize,
>(
    support: &SupportChargeLedger<RECORDS, CLAIMS, HORIZONS>,
    identity: TurnPlanIdentity,
    resolution: ObservationResolution,
    occurred_at: MonotonicTime,
    work: &mut WorkMeter,
) -> Result<PreparedPlanTransition, SupportLedgerError> {
    let mut census = ExactWorkCensus::new();
    let change = support.prepare_c17_observation_resolution(
        support.generation(),
        identity,
        resolution,
        occurred_at,
        &mut census,
    )?;
    support.validate_c17_root_batch(&change)?;
    let change = PreparedSupportRootSeal::new(change)?;
    work.charge(census.witness())?;
    Ok(PreparedPlanTransition::Observation(change))
}

pub(crate) fn commit_plan_transition<
    const RECORDS: usize,
    const CLAIMS: usize,
    const HORIZONS: usize,
>(
    support: &mut SupportChargeLedger<RECORDS, CLAIMS, HORIZONS>,
    change: PreparedPlanTransition,
) {
    let change = match change {
        PreparedPlanTransition::Disposition(change)
        | PreparedPlanTransition::Root(change)
        | PreparedPlanTransition::Observation(change) => change,
    };
    change.commit(support);
}

pub(crate) struct PreparedTypedClose {
    input: TypedCloseInput,
    support: Box<PreparedSupportRootSeal>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TypedClosePrepareError {
    Request(RequestError),
    Support(SupportLedgerError),
}

pub(crate) fn prepare_typed_close<
    const REQUESTS: usize,
    const INPUT: usize,
    const STOPS: usize,
    const STOP_TOKENS: usize,
    const RECORDS: usize,
    const CLAIMS: usize,
    const HORIZONS: usize,
>(
    requests: Option<&RequestBook<REQUESTS, INPUT, STOPS, STOP_TOKENS>>,
    support: &SupportChargeLedger<RECORDS, CLAIMS, HORIZONS>,
    input: TypedCloseInput,
    work: &mut WorkMeter,
) -> Result<PreparedTypedClose, TypedClosePrepareError> {
    validate_typed_close_canonical(input)?;
    validate_typed_close_request(requests, input.authority)?;
    let mut census = ExactWorkCensus::new();
    let support_change = support
        .prepare_c17_typed_close(support.generation(), input, &mut census)
        .map_err(TypedClosePrepareError::Support)?;
    support
        .validate_c17_root_batch(&support_change)
        .map_err(TypedClosePrepareError::Support)?;
    let support_change =
        PreparedSupportRootSeal::new(support_change).map_err(TypedClosePrepareError::Support)?;
    validate_typed_close_request(requests, input.authority)?;
    work.charge(census.witness())
        .map_err(SupportLedgerError::from)
        .map_err(TypedClosePrepareError::Support)?;
    Ok(PreparedTypedClose {
        input,
        support: support_change,
    })
}

pub(crate) fn validate_typed_close<
    const REQUESTS: usize,
    const INPUT: usize,
    const STOPS: usize,
    const STOP_TOKENS: usize,
    const RECORDS: usize,
    const CLAIMS: usize,
    const HORIZONS: usize,
>(
    requests: Option<&RequestBook<REQUESTS, INPUT, STOPS, STOP_TOKENS>>,
    support: &SupportChargeLedger<RECORDS, CLAIMS, HORIZONS>,
    change: &PreparedTypedClose,
) -> Result<(), TypedClosePrepareError> {
    validate_typed_close_canonical(change.input)?;
    validate_typed_close_request(requests, change.input.authority)?;
    change
        .support
        .validate(support)
        .map_err(TypedClosePrepareError::Support)
}

pub(crate) fn commit_typed_close<
    const REQUESTS: usize,
    const INPUT: usize,
    const STOPS: usize,
    const STOP_TOKENS: usize,
    const RECORDS: usize,
    const CLAIMS: usize,
    const HORIZONS: usize,
>(
    requests: Option<&RequestBook<REQUESTS, INPUT, STOPS, STOP_TOKENS>>,
    support: &mut SupportChargeLedger<RECORDS, CLAIMS, HORIZONS>,
    change: PreparedTypedClose,
) {
    let _ = requests;
    change.support.commit(support);
}

fn validate_typed_close_canonical(input: TypedCloseInput) -> Result<(), TypedClosePrepareError> {
    let malformed_source = matches!(
        input.authority,
        CloseAuthority::Standalone { source, .. }
            if source.reserved != 0 || source.generation() == 0
    );
    if input.occurred_at.as_micros() == 0 || input.group != input.root.slot() || malformed_source {
        return Err(TypedClosePrepareError::Support(
            SupportLedgerError::InvalidInput,
        ));
    }
    Ok(())
}

fn validate_typed_close_request<
    const REQUESTS: usize,
    const INPUT: usize,
    const STOPS: usize,
    const STOP_TOKENS: usize,
>(
    requests: Option<&RequestBook<REQUESTS, INPUT, STOPS, STOP_TOKENS>>,
    authority: CloseAuthority,
) -> Result<(), TypedClosePrepareError> {
    let CloseAuthority::Cancellation {
        fact,
        event,
        request_generation,
    } = authority
    else {
        return Ok(());
    };
    requests
        .ok_or(TypedClosePrepareError::Request(
            RequestError::InvalidTransition,
        ))?
        .validate_cancellation_close_authority(fact, event, request_generation)
        .map_err(TypedClosePrepareError::Request)
}

pub(crate) fn prepare_lifecycle_begin<
    const RECORDS: usize,
    const CLAIMS: usize,
    const HORIZONS: usize,
>(
    support: &SupportChargeLedger<RECORDS, CLAIMS, HORIZONS>,
    total: usize,
    aggregate: LifecycleAggregate,
    work: &mut WorkMeter,
) -> Result<PreparedC17LifecycleBegin, SupportLedgerError> {
    let change =
        support.prepare_c17_lifecycle_begin(support.generation(), total, aggregate, work)?;
    Ok(change)
}

pub(crate) fn commit_lifecycle_begin<
    const RECORDS: usize,
    const CLAIMS: usize,
    const HORIZONS: usize,
>(
    support: &mut SupportChargeLedger<RECORDS, CLAIMS, HORIZONS>,
    change: PreparedC17LifecycleBegin,
) {
    support.commit_c17_lifecycle_begin(change);
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct LifecycleMembershipSeal {
    request: RequestId,
    expected_status: RequestStatusVersion,
    anchor: crate::request_book::c17::SupportMembershipAnchor,
}

#[derive(Debug)]
pub(crate) struct PreparedLifecycleStageOwned {
    support: PreparedLifecycleStage,
    _memberships: [Option<LifecycleMembershipSeal>; 8],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LifecycleStagePrepareError {
    Request(RequestError),
    Support(SupportLedgerError),
}

pub(crate) fn prepare_lifecycle_stage<
    const REQUESTS: usize,
    const INPUT: usize,
    const STOPS: usize,
    const STOP_TOKENS: usize,
    const RECORDS: usize,
    const CLAIMS: usize,
    const HORIZONS: usize,
>(
    requests: Option<&RequestBook<REQUESTS, INPUT, STOPS, STOP_TOKENS>>,
    support: &SupportChargeLedger<RECORDS, CLAIMS, HORIZONS>,
    specs: &[Option<C17LifecycleRecordSpec>],
    work: &mut WorkMeter,
) -> Result<PreparedLifecycleStageOwned, LifecycleStagePrepareError> {
    if specs.is_empty() || specs.len() > 8 || specs.iter().any(Option::is_none) {
        return Err(LifecycleStagePrepareError::Support(
            SupportLedgerError::InvalidInput,
        ));
    }
    let first = support
        .c17_lifecycle_stage_start()
        .map_err(LifecycleStagePrepareError::Support)?;
    let mut records = [LifecycleRecordInput::ZERO; 8];
    let mut memberships = [None; 8];
    for (index, spec) in specs.iter().copied().enumerate() {
        let spec = spec.expect("validated active lifecycle specification");
        let anchor = match spec.root {
            C17LifecycleRootSpec::Plan { identity, branch } => support
                .c17_lifecycle_plan_anchor(identity, branch)
                .map_err(LifecycleStagePrepareError::Support)?,
            C17LifecycleRootSpec::Membership {
                request,
                expected_status,
            } => {
                let requests = requests.ok_or(LifecycleStagePrepareError::Request(
                    RequestError::InvalidTransition,
                ))?;
                let membership = requests
                    .c17_membership_anchor(request, expected_status)
                    .map_err(LifecycleStagePrepareError::Request)?;
                memberships[index] = Some(LifecycleMembershipSeal {
                    request,
                    expected_status,
                    anchor: membership,
                });
                support
                    .c17_lifecycle_membership_anchor(membership)
                    .map_err(LifecycleStagePrepareError::Support)?
            }
        };
        let ordinal = first
            .checked_add(index)
            .ok_or(LifecycleStagePrepareError::Support(
                SupportLedgerError::Storage(crate::FixedStorageError::Capacity),
            ))?;
        records[index] = support
            .bind_c17_lifecycle_record_spec(anchor, ordinal, spec)
            .map_err(LifecycleStagePrepareError::Support)?;
    }
    let change = support
        .prepare_c17_lifecycle_stage(&records[..specs.len()], work)
        .map_err(LifecycleStagePrepareError::Support)?;
    Ok(PreparedLifecycleStageOwned {
        support: change,
        _memberships: memberships,
    })
}

pub(crate) fn commit_lifecycle_stage<
    const REQUESTS: usize,
    const INPUT: usize,
    const STOPS: usize,
    const STOP_TOKENS: usize,
    const RECORDS: usize,
    const CLAIMS: usize,
    const HORIZONS: usize,
>(
    requests: Option<&RequestBook<REQUESTS, INPUT, STOPS, STOP_TOKENS>>,
    support: &mut SupportChargeLedger<RECORDS, CLAIMS, HORIZONS>,
    change: PreparedLifecycleStageOwned,
) {
    let PreparedLifecycleStageOwned {
        support: change,
        _memberships: _,
    } = change;
    let _ = requests;
    support.commit_c17_lifecycle_stage(change);
}

pub(crate) fn prepare_lifecycle_finalize<
    const RECORDS: usize,
    const CLAIMS: usize,
    const HORIZONS: usize,
>(
    support: &SupportChargeLedger<RECORDS, CLAIMS, HORIZONS>,
    work: &mut WorkMeter,
) -> Result<PreparedC17LifecycleFinalize, SupportLedgerError> {
    let change = support.prepare_c17_lifecycle_finalize(work)?;
    Ok(change)
}

pub(crate) fn commit_lifecycle_finalize<
    const RECORDS: usize,
    const CLAIMS: usize,
    const HORIZONS: usize,
>(
    support: &mut SupportChargeLedger<RECORDS, CLAIMS, HORIZONS>,
    change: PreparedC17LifecycleFinalize,
) {
    support.commit_c17_lifecycle_finalize(change);
}

pub(crate) fn prepare_lifecycle_abort<
    const RECORDS: usize,
    const CLAIMS: usize,
    const HORIZONS: usize,
>(
    support: &SupportChargeLedger<RECORDS, CLAIMS, HORIZONS>,
    work: &mut WorkMeter,
) -> Result<PreparedLifecycleAbort, SupportLedgerError> {
    let change = support.prepare_c17_lifecycle_abort(work)?;
    Ok(change)
}

pub(crate) fn commit_lifecycle_abort<
    const RECORDS: usize,
    const CLAIMS: usize,
    const HORIZONS: usize,
>(
    support: &mut SupportChargeLedger<RECORDS, CLAIMS, HORIZONS>,
    change: PreparedLifecycleAbort,
) -> bool {
    support.commit_c17_lifecycle_abort(change)
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::reusable::{PatriciaAssignmentPlan, ReusablePatricia};

    struct TestAssignments<const N: usize>(PatriciaAssignmentPlan<N>);

    impl<const N: usize> AssignmentSource for TestAssignments<N> {
        fn visit_assignments(&self, visitor: &mut dyn FnMut(AssignmentOrderKey, Assignment)) {
            self.0.visit_assignments(visitor);
        }
    }

    struct JournalFixture {
        journal: CombinedAssignmentJournal,
        support_change: TestAssignments<10>,
        request_change: TestAssignments<10>,
        support: SupportLedgerGeneration,
        request: RequestBookGeneration,
    }

    fn journal_fixture() -> JournalFixture {
        let support = SupportLedgerGeneration::new(7).unwrap();
        let request = RequestBookGeneration::new(11).unwrap();
        let support_index = ReusablePatricia::<32, 8>::try_new(4).unwrap();
        let request_index = ReusablePatricia::<8, 8>::try_new(4).unwrap();
        let support_change = TestAssignments(
            support_index
                .prepare_insert_assignment_plan::<10>(1, &[([1; 32], [3; 8])])
                .unwrap(),
        );
        let request_change = TestAssignments(
            request_index
                .prepare_insert_assignment_plan::<10>(21, &[([2; 8], [4; 8])])
                .unwrap(),
        );
        let journal = CombinedAssignmentJournal::new(&request_change, &support_change).unwrap();
        JournalFixture {
            journal,
            support_change,
            request_change,
            support,
            request,
        }
    }

    fn interleaved_journal_fixture() -> (
        CombinedAssignmentJournal,
        TestAssignments<19>,
        TestAssignments<19>,
    ) {
        let support_index = ReusablePatricia::<32, 8>::try_new(4).unwrap();
        let request_index = ReusablePatricia::<8, 8>::try_new(4).unwrap();
        let support_change = TestAssignments(
            support_index
                .prepare_insert_assignment_plan::<19>(1, &[([1; 32], [9; 8]), ([3; 32], [7; 8])])
                .unwrap(),
        );
        let request_change = TestAssignments(
            request_index
                .prepare_insert_assignment_plan::<19>(21, &[([2; 8], [8; 8]), ([4; 8], [6; 8])])
                .unwrap(),
        );
        let journal = CombinedAssignmentJournal::new(&request_change, &support_change).unwrap();
        (journal, support_change, request_change)
    }

    fn active_ordered_assignments(
        support: &impl AssignmentSource,
        request: &impl AssignmentSource,
    ) -> Vec<(AssignmentOrderKey, Assignment)> {
        let mut entries = Vec::new();
        let mut visit = |order, assignment| {
            if assignment != Assignment::NOOP {
                entries.push((order, assignment));
            }
        };
        support.visit_assignments(&mut visit);
        request.visit_assignments(&mut visit);
        entries.sort_unstable_by_key(|entry| entry.0);
        entries
    }

    #[test]
    fn combined_assignment_journal_globally_interleaves_semantic_edits() {
        let (journal, support, request) = interleaved_journal_fixture();
        let ordered = active_ordered_assignments(&support, &request);

        assert_eq!(
            journal.assignments[..journal.active_len()],
            ordered
                .iter()
                .map(|(_, assignment)| *assignment)
                .collect::<Vec<_>>()
        );
        assert!(
            ordered
                .windows(2)
                .all(|pair| CombinedAssignmentJournal::valid_order_pair(pair[0].0, pair[1].0))
        );

        let mut semantic_arenas = Vec::new();
        for (index, (order, _)) in ordered.iter().enumerate() {
            if order.is_generation() {
                continue;
            }
            if index == 0 || ordered[index - 1].0.semantic_cmp(order) != Ordering::Equal {
                semantic_arenas.push(order.arena_id());
            }
        }
        assert_eq!(semantic_arenas, [1, 21, 1, 21]);
        assert_eq!(
            ordered
                .iter()
                .filter_map(|(order, _)| order.is_generation().then_some(order.arena_id()))
                .collect::<Vec<_>>(),
            [1, 21]
        );
    }

    #[test]
    fn combined_assignment_journal_rejects_semantic_duplicates_and_bad_envelopes() {
        let support_index = ReusablePatricia::<8, 8>::try_new(2).unwrap();
        let request_index = ReusablePatricia::<8, 8>::try_new(2).unwrap();
        let support = TestAssignments(
            support_index
                .prepare_insert_assignment_plan::<10>(1, &[([5; 8], [6; 8])])
                .unwrap(),
        );
        let request = TestAssignments(
            request_index
                .prepare_insert_assignment_plan::<10>(21, &[([5; 8], [6; 8])])
                .unwrap(),
        );
        assert_eq!(
            CombinedAssignmentJournal::new(&request, &support),
            Err(FixedStorageError::NonCanonical)
        );

        let unsupported = TestAssignments(
            ReusablePatricia::<8, 8>::try_new(2)
                .unwrap()
                .prepare_insert_assignment_plan::<10>(19, &[([1; 8], [2; 8])])
                .unwrap(),
        );
        assert_eq!(
            CombinedAssignmentJournal::single(&unsupported),
            Err(FixedStorageError::NonCanonical)
        );

        struct Empty;
        impl AssignmentSource for Empty {
            fn visit_assignments(&self, _visitor: &mut dyn FnMut(AssignmentOrderKey, Assignment)) {}
        }
        assert_eq!(
            CombinedAssignmentJournal::single(&Empty),
            Err(FixedStorageError::NonCanonical)
        );

        struct Overflow;
        impl AssignmentSource for Overflow {
            fn visit_assignments(&self, visitor: &mut dyn FnMut(AssignmentOrderKey, Assignment)) {
                for _ in 0..=ORDINARY_ASSIGNMENTS {
                    visitor(AssignmentOrderKey::ZERO, Assignment::NOOP);
                }
            }
        }
        assert_eq!(
            CombinedAssignmentJournal::single(&Overflow),
            Err(FixedStorageError::Capacity)
        );
    }

    #[test]
    fn combined_assignment_journal_rejects_every_assignment_corruption_class() {
        let rejected = |mutate: fn(&mut CombinedAssignmentJournal)| {
            let mut fixture = journal_fixture();
            mutate(&mut fixture.journal);
            assert!(
                !fixture
                    .journal
                    .matches_sources(&fixture.request_change, &fixture.support_change),
                "corrupted journal must not match its sealed owners"
            );
        };

        rejected(|journal| journal.assignments[0].destination_arena = 0);
        rejected(|journal| journal.assignments[0].destination_arena = 2);
        rejected(|journal| journal.assignments[0].destination_kind = 9);
        rejected(|journal| journal.assignments[0].destination_slot ^= 1);
        rejected(|journal| {
            journal.assignments[0].image_len = match journal.assignments[0].image_len {
                56 => 64,
                _ => 56,
            };
        });
        rejected(|journal| journal.assignments[0].expected_generation ^= 1);
        rejected(|journal| journal.assignments[0].payload[0] ^= 1);
        rejected(|journal| {
            let index = usize::from(journal.assignments[0].image_len);
            journal.assignments[0].payload[index] = 1;
        });
        rejected(|journal| {
            let header = journal.assignments[..journal.active_len()]
                .iter()
                .position(|assignment| assignment.destination_kind == DestinationKind::Header as u8)
                .unwrap();
            journal.assignments[header].payload[39] ^= 1;
        });
        rejected(|journal| {
            let tail = journal.active_len();
            journal.assignments[tail].destination_slot = 1;
        });
        rejected(|journal| journal.assignments.swap(0, 1));
    }

    #[test]
    fn combined_assignment_journal_is_fixed_sorted_and_generation_bound() {
        let fixture = journal_fixture();
        let journal = &fixture.journal;

        assert_eq!(journal.assignments.len(), ORDINARY_ASSIGNMENTS);
        assert_eq!(
            std::mem::size_of::<CombinedAssignmentJournal>(),
            ORDINARY_ASSIGNMENTS * std::mem::size_of::<Assignment>()
        );
        assert!(journal.canonical());
        assert!(journal.matches_sources(&fixture.request_change, &fixture.support_change));
        assert_eq!(journal.active_len(), 4);
        assert!(
            journal.assignments[..journal.active_len()]
                .iter()
                .all(|assignment| *assignment != Assignment::NOOP)
        );
        assert!(
            journal.assignments[..journal.active_len()]
                .iter()
                .any(|assignment| assignment.destination_arena == 1)
        );
        assert!(
            journal.assignments[..journal.active_len()]
                .iter()
                .any(|assignment| assignment.destination_arena == 21)
        );
        assert!(
            journal.assignments[journal.active_len()..]
                .iter()
                .all(|assignment| *assignment == Assignment::NOOP)
        );

        let support_after = fixture.support.next().unwrap();
        let request_after = fixture.request.next().unwrap();
        let permit = combined_commit_permit(
            &fixture.journal,
            fixture.support,
            support_after,
            fixture.request,
            request_after,
        );
        assert_eq!(permit.support_before, fixture.support);
        assert_eq!(permit.support_after, support_after);
        assert_eq!(permit.request_before, fixture.request);
        assert_eq!(permit.request_after, request_after);

        let mut fixture = journal_fixture();
        fixture.journal.assignments[fixture.journal.active_len()].destination_slot = 1;
        assert!(!fixture.journal.canonical());

        let mut fixture = journal_fixture();
        fixture.journal.assignments.swap(0, 1);
        assert!(fixture.journal.canonical());
        assert!(
            !fixture
                .journal
                .matches_sources(&fixture.request_change, &fixture.support_change)
        );

        let mut fixture = journal_fixture();
        fixture.journal.assignments[0].payload[0] ^= 1;
        assert!(fixture.journal.canonical());
        assert!(
            !fixture
                .journal
                .matches_sources(&fixture.request_change, &fixture.support_change)
        );
    }
}
