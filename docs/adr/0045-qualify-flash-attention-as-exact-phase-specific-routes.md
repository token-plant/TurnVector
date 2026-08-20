# Qualify Flash Attention as Exact Phase-Specific Routes

TurnVector treats FlashAttention as an implementation family, not as a global
feature flag, a public capability name, or a synonym for every fused attention
kernel. Each production attention implementation is authorized only through a
phase-specific exact Capability Key. The Key binds Prefill or Decode and the
exact shape case to an Execution Route whose independent Attention Path identity
composes exact graph, memory, kernel/fusion, KV/cache, and command-plan members.
Prefill evidence never authorizes Decode, and contiguous-KV evidence never
authorizes a PagedKV route.

Attention Path is a top-level composition identity because contiguous, gathered,
and direct block-table execution cross several route members and vary
independently from KV layout. It owns only the stable path kind and exact member
references plus compilation timing and no-fallback policies. Graph owns
attention mathematics and mask/position semantics; kernel/fusion owns dispatch,
artifacts, and compilation inputs; KV/cache owns storage, access, and reader ABI;
memory owns scratch; and the command member owns submission and replay. A
canonical mismatch between a path reference and its route member is invalid.
There is no separate mutable Attention Path by KV-layout compatibility matrix.
A production combination exists only as one complete canonical Execution Route
declared by the Backend Capability descriptor and accepted by the offline
Certification compiler as an exact Certified Execution Profile. The Model
Planner is the sole runtime owner of structural path/layout compatibility and
may propose only those finite declared compositions; Core enforces exact-key
authorization, and the Adapter may reject later drift only before a route
operation starts.

The PagedKV and attention contracts are designed together because a native
page-reading kernel depends on the block-table reader ABI, mask and position
semantics, resource bounds, and Turn Receipt fields. Their implementation and
qualification remain sequential. The PagedKV work first supplies a qualified
layout and a gather-to-pinned-MLX-SDPA reference route. The PagedAttention
delivery owns the first native block-table Decode route; the FlashAttention
delivery starts with a separate tiled Prefill route. Both compare against the
gathered reference. TurnVector does not introduce a new KV layout and a new
attention algorithm in the same first qualification step.

The private C++/MLX Adapter contains separate KV Layout and Attention Modules.
The KV Layout Module owns page allocation, block tables, update, release, and
materialization. The Attention Module owns attention semantics, dispatch,
kernels, masks, positions, and scratch. A combined native route binds both
module identities without exposing either module, tensors, or kernel choices
through the coarse Rust-facing Backend Interface. All MLX and Metal state
remains on the Device Executor owner thread.

Pinned MLX automatic SDPA dispatch, pinned MLX required-fused dispatch, native
block-table Decode, and native block-table tiled Prefill are distinct exact
route combinations. An automatic MLX route is eligible only if every
implementation variant reachable by its exact certified cases is enumerated and
bounded. The implementation variant observed during execution is Turn Receipt
telemetry, never authority to widen a Turn Plan.
Known inapplicability excludes a route during Candidate Formation. Backend-owned
inapplicability of a still-current Turn Plan returns Plan Rejection only before
any route operation, including first-use compilation, starts. After the accepted
rejection, the required fresh Scheduling Snapshot cannot substitute work; an
alternative requires separately obligated rejection-driven Candidate
Formation, a new Work Candidate, a later fresh Scheduling Snapshot, and a new
Turn Plan. Beginning compilation starts the Turn: compile failure is a typed
started-Turn failure, compile-bound excess is a Bound Violation, and a
trustworthy synchronized Turn Receipt is required unless the process fail-stops.
No started Turn silently falls back.

Every Attention Path fixes either `PRECOMPILED_REQUIRED` or
`BOUNDED_FIRST_USE` as its compilation timing policy. Shared-production routes
default to `PRECOMPILED_REQUIRED`: the exact artifact is compiled or loaded by a
bounded owner-thread Residency Transition and the route is not installed or
eligible for a Scheduling Snapshot until that operation succeeds on the exact
qualified MLX, Metal, OS, Adapter, and device Envelope. A
`BOUNDED_FIRST_USE` route is a distinct exact route whose Case Bound Table and
resource evidence conservatively include cold compilation on every use; a warm
observation cannot narrow that bound. Compilation inside `execute_turn` is
Engine Service, not Runtime Overhead. Qualification must prove exact
compilability and the declared cold-start availability policy. If compilation
nevertheless fails after the Turn starts, that Turn fails rather than degrading
to another route; only a later fresh Snapshot may select a separately authorized
alternative.

Compilation is not assumed interruptible. A cancellation ordered while
compilation is active enters Cancel Pending, but does not force-abort MLX or
Metal compilation. The route continues to its first qualified synchronized
state-safe boundary within the exact Turn bound. If compile completion is such a
boundary, no inference submission starts and the Adapter returns the cancelled
Member Outcome in the Turn Receipt; otherwise it continues to the next qualified
boundary. This rule does not depend on staged output existing. A command that has
only arrived at an external queue is not Cancellation Accepted until the Core
orders it, and an unbounded or untrustworthy path to the boundary is ineligible.

This decision does not change the P0 baseline or claim that a fused or tiled
route is faster. A route is promoted only after exact correctness, resource,
failure, and end-to-end performance evidence passes its predeclared gates. The
detailed route matrix, sequencing, evidence limits, and promotion criteria are
defined in `docs/plans/2026-08-19-flash-attention-route-design.md`.

This ADR refines ADR 0016 for attention-route compilation without superseding
its general Plan Rejection, Turn Receipt, cancellation-ordering, or fail-stop
contract.
