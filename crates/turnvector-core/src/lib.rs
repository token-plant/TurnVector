//! Serialization-independent deterministic Runtime Core.
//!
//! Values from different domains are intentionally not interchangeable:
//!
//! ```compile_fail
//! use turnvector_core::{ConnectionId, ModelId};
//!
//! let connection = ConnectionId::new(1).unwrap();
//! let _: ModelId = connection;
//! ```
//!
//! Units and sequences retain the same boundary:
//!
//! ```compile_fail
//! use turnvector_core::{ByteCount, TokenCount};
//!
//! let bytes = ByteCount::new(1);
//! let tokens = TokenCount::new(1);
//! let _ = bytes.checked_add(tokens);
//! ```
//!
//! ```compile_fail
//! use turnvector_core::{EventSequence, OutputSequence};
//!
//! let event = EventSequence::new(1).unwrap();
//! let _: OutputSequence = event;
//! ```

use std::error::Error;
use std::fmt;
use std::num::{NonZeroU64, NonZeroU128};

mod bounded;
mod core;
mod scheduling;
mod turns;

pub use bounded::{BoundedCollectionError, BoundedMap, BoundedSet, BoundedVec};
pub use core::{
    Core, CoreEvent, CoreFault, CoreOutcome, CoreState, CoreTransition, DomainRejection, Effect,
};
pub use scheduling::{
    AuthorizedCapabilitySet, BatchBucket, CandidateCoordinates, CandidateExclusion,
    CandidateExclusionReason, CandidateMember, CandidateValidationError, CapabilityKey,
    ExecutionPhase, RuntimeOverheadBoundSetId, SchedulingSnapshot, ServiceClass, WorkCandidate,
};
pub use turns::{
    FutureTurnSupportEntitlementId, MemberOutcome, PersistentStateIsolationEvidenceId,
    PhysicalStartCreditId, PlanMemberFunding, PlanSupportObligation, PlanSupportObligations,
    PlanValidationError, StalePlanDispositionBoundId, SupportOperationObligationId,
    SupportOutstandingCreditVectorId, TurnBudget, TurnPlan, TurnPlanIdentity, TurnProgress,
    TurnReceipt, TurnReceiptIdentity, TurnReceiptMember, YieldReason,
};

/// Failure to construct or advance a checked domain value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DomainValueError {
    Zero,
    Overflow,
    Underflow,
}

impl fmt::Display for DomainValueError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Zero => "value must be nonzero",
            Self::Overflow => "value overflowed its domain",
            Self::Underflow => "value underflowed its domain",
        })
    }
}

impl Error for DomainValueError {}

macro_rules! nonzero_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(NonZeroU128);

        impl $name {
            pub fn new(value: u128) -> Result<Self, DomainValueError> {
                NonZeroU128::new(value)
                    .map(Self)
                    .ok_or(DomainValueError::Zero)
            }

            #[must_use]
            pub const fn get(self) -> u128 {
                self.0.get()
            }
        }
    };
}

macro_rules! nonzero_sequence {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(NonZeroU64);

        impl $name {
            pub fn new(value: u64) -> Result<Self, DomainValueError> {
                NonZeroU64::new(value)
                    .map(Self)
                    .ok_or(DomainValueError::Zero)
            }

            #[must_use]
            pub const fn get(self) -> u64 {
                self.0.get()
            }

            pub fn next(self) -> Result<Self, DomainValueError> {
                self.get()
                    .checked_add(1)
                    .ok_or(DomainValueError::Overflow)
                    .and_then(Self::new)
            }
        }
    };
}

macro_rules! checked_unit {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(u64);

        impl $name {
            #[must_use]
            pub const fn new(value: u64) -> Self {
                Self(value)
            }

            #[must_use]
            pub const fn get(self) -> u64 {
                self.0
            }

            pub fn checked_add(self, other: Self) -> Result<Self, DomainValueError> {
                self.0
                    .checked_add(other.0)
                    .map(Self)
                    .ok_or(DomainValueError::Overflow)
            }

            pub fn checked_sub(self, other: Self) -> Result<Self, DomainValueError> {
                self.0
                    .checked_sub(other.0)
                    .map(Self)
                    .ok_or(DomainValueError::Underflow)
            }
        }
    };
}

nonzero_id!(/// One daemon process lifetime.
DaemonInstanceId);
nonzero_id!(/// One accepted Data Plane connection.
ConnectionId);
nonzero_id!(/// The stable fairness identity shared by a model's revisions.
ModelId);
nonzero_id!(/// One effect or authority-publication operation.
OperationId);
nonzero_id!(/// One stable opaque schedulable choice.
CandidateId);
nonzero_id!(/// One authorization for a bounded Turn.
TurnPlanId);

nonzero_sequence!(/// A client command position within one connection.
CommandId);
nonzero_sequence!(/// The daemon order of Runtime Event Loop inputs and results.
EventSequence);
nonzero_sequence!(/// A visible token position within one request.
OutputSequence);
nonzero_sequence!(/// A successful acceptance position within one connection.
RequestSequence);
nonzero_sequence!(/// An externally visible request-state version.
RequestStatusVersion);
nonzero_sequence!(/// The version of Scheduler-owned request and obligation state.
SchedulerGeneration);
nonzero_sequence!(/// The version of Backend state that affects execution validity.
BackendGeneration);
nonzero_sequence!(/// The version of effective resource restrictions.
SafetyGeneration);
nonzero_sequence!(/// The version of applicable daemon runtime-overhead evidence.
RuntimeOverheadGeneration);

checked_unit!(/// A count of bytes.
ByteCount);
checked_unit!(/// A count of tokens.
TokenCount);

/// The identity of one accepted request within a daemon lifetime.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct RequestId {
    daemon_instance: DaemonInstanceId,
    connection: ConnectionId,
    sequence: RequestSequence,
}

impl RequestId {
    #[must_use]
    pub const fn new(
        daemon_instance: DaemonInstanceId,
        connection: ConnectionId,
        sequence: RequestSequence,
    ) -> Self {
        Self {
            daemon_instance,
            connection,
            sequence,
        }
    }

    #[must_use]
    pub const fn daemon_instance(self) -> DaemonInstanceId {
        self.daemon_instance
    }

    #[must_use]
    pub const fn connection(self) -> ConnectionId {
        self.connection
    }

    #[must_use]
    pub const fn sequence(self) -> RequestSequence {
        self.sequence
    }
}

/// A nonnegative elapsed interval measured in microseconds.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Duration(u64);

impl Duration {
    #[must_use]
    pub const fn from_micros(microseconds: u64) -> Self {
        Self(microseconds)
    }

    #[must_use]
    pub const fn as_micros(self) -> u64 {
        self.0
    }

    pub fn checked_add(self, other: Self) -> Result<Self, DomainValueError> {
        self.0
            .checked_add(other.0)
            .map(Self)
            .ok_or(DomainValueError::Overflow)
    }
}

/// A daemon-owned non-decreasing clock reading in microseconds.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct MonotonicTime(u64);

impl MonotonicTime {
    #[must_use]
    pub const fn from_micros(microseconds: u64) -> Self {
        Self(microseconds)
    }

    #[must_use]
    pub const fn as_micros(self) -> u64 {
        self.0
    }

    pub fn checked_add(self, duration: Duration) -> Result<Self, DomainValueError> {
        self.0
            .checked_add(duration.0)
            .map(Self)
            .ok_or(DomainValueError::Overflow)
    }

    pub fn checked_duration_since(self, earlier: Self) -> Result<Duration, DomainValueError> {
        self.0
            .checked_sub(earlier.0)
            .map(Duration)
            .ok_or(DomainValueError::Underflow)
    }
}

/// The generations captured by a Scheduling Snapshot and copied into a Turn Plan.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct GenerationVector {
    scheduler: SchedulerGeneration,
    backend: BackendGeneration,
    safety: SafetyGeneration,
    runtime_overhead: RuntimeOverheadGeneration,
}

impl GenerationVector {
    #[must_use]
    pub const fn new(
        scheduler: SchedulerGeneration,
        backend: BackendGeneration,
        safety: SafetyGeneration,
        runtime_overhead: RuntimeOverheadGeneration,
    ) -> Self {
        Self {
            scheduler,
            backend,
            safety,
            runtime_overhead,
        }
    }

    pub fn validate_current(self, current: Self) -> Result<(), GenerationMismatch> {
        [
            (
                GenerationComponent::Scheduler,
                self.scheduler.get(),
                current.scheduler.get(),
            ),
            (
                GenerationComponent::Backend,
                self.backend.get(),
                current.backend.get(),
            ),
            (
                GenerationComponent::Safety,
                self.safety.get(),
                current.safety.get(),
            ),
            (
                GenerationComponent::RuntimeOverhead,
                self.runtime_overhead.get(),
                current.runtime_overhead.get(),
            ),
        ]
        .into_iter()
        .find(|(_, planned, now)| planned != now)
        .map_or(Ok(()), |(component, planned, current)| {
            Err(GenerationMismatch {
                component,
                planned,
                current,
            })
        })
    }
}

/// One member of the four-part Generation Vector.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GenerationComponent {
    Scheduler,
    Backend,
    Safety,
    RuntimeOverhead,
}

/// The first stale component found in deterministic Generation Vector order.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GenerationMismatch {
    pub component: GenerationComponent,
    pub planned: u64,
    pub current: u64,
}
