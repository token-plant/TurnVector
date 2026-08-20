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
bound by the Model Descriptor, enumerated by the Capability Requirement Set, and
covered by an immutable Certification Record. The Model Planner is the sole
runtime owner of structural path/layout compatibility and may propose only those
finite declared compositions. The offline Certification compiler verifies their
evidence and emits the read-only index; Admission alone derives each Authorized
Capability Set, Core enforces exact-key membership, and the Adapter may reject
later drift only before a route operation starts.

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
default to `PRECOMPILED_REQUIRED`: the exact artifact is prepared inside the
existing bounded owner-thread model-load Residency Transition. The route and
policy are already fixed in the immutable Model Descriptor and certification
evidence; load does not add a Residency variant or mutate a Backend Capability
descriptor, Certification Authorization Index, or Authorized Capability Set.
Successful load advances Backend Generation, and the Revision remains
unavailable for Candidate Formation until post-load Model Descriptor equality
succeeds. A failed or cooperatively cancelled started load is a Residency
Failure: it strongly rolls back to zero retained Backend residency ownership,
marks the Revision Unavailable, fails every Warming waiter, and stops automatic
retry. Cancellation by the last waiter before loading starts remains ordinary
Residency Demand withdrawal. A
`BOUNDED_FIRST_USE` route is a distinct exact route whose Case Bound Table and
resource evidence conservatively include cold compilation on every use; a warm
observation cannot narrow that bound. Compilation inside `execute_turn` is
Engine Service, not Runtime Overhead. Qualification must prove exact
compilability and the declared cold-start availability policy. If compilation
nevertheless fails after the Turn starts, that Turn fails rather than degrading
to another route; only a later fresh Scheduling Snapshot may select a separately
authorized alternative.

Compilation is not assumed interruptible, and compile completion is not a P0
cancellation boundary. While the owner-thread call is active, an external Device
Control Signal may request timely return but carries no semantic order, cannot
enter Cancel Pending, constitute Cancellation Accepted, or select a Batch member
or terminal outcome. The Adapter does not force-abort MLX or Metal compilation;
it continues the exact planned route to its next qualified synchronized Turn
boundary and returns one trustworthy Turn Receipt within the exact bound. After
owner-thread control returns, the Event Loop orders the queued cancellation
command and Turn Receipt commit through the existing contiguous Event Sequence.
Only a cancellation ordered first enters Cancel Pending and discards that
member's staged output, including the no-output case; a signal alone has no
cancellation authority, and a later cancellation cannot revoke published
output. An unbounded or untrustworthy path to that boundary is ineligible.

This decision does not change the P0 baseline or claim that a fused or tiled
route is faster. A route is promoted only after exact correctness, resource,
failure, and end-to-end performance evidence passes its predeclared gates. The
detailed route matrix, sequencing, evidence limits, and promotion criteria are
defined in `docs/plans/2026-08-19-flash-attention-route-design.md`.

This ADR refines ADR 0016 for attention-route compilation without superseding
its general Plan Rejection, Turn Receipt, cancellation-ordering, or fail-stop
contract.
