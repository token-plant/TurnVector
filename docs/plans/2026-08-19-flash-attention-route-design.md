# Flash Attention Route Design

Status: design only; future implementation work; no performance or readiness
claim

## Decision Summary

Design PagedAttention and FlashAttention together at their internal contract,
then implement and qualify them in sequence.

The shared design is necessary because a direct page-reading attention kernel
must agree with the PagedKV block-table reader ABI, token positions, masking,
grouped-query semantics, scratch bounds, cancellation points, and Turn Receipt
fields. The implementation must be staged because changing KV storage and the
attention algorithm at the same time removes the qualified comparison route
needed to localize correctness, memory, and performance regressions.

The default sequence is:

1. Preserve the contiguous pinned-MLX-SDPA baseline.
2. Introduce PagedKV with an exact gather-to-pinned-MLX-SDPA route.
3. Qualify any pinned-MLX required-fused route separately, if the selected pin
   exposes a usable and bounded contract.
4. Complete PagedAttention `PA04`, the sole native block-table Decode slice,
   with one exact geometry.
5. Start FlashAttention implementation with one native block-table tiled
   Prefill route and one exact geometry.
6. Expand exact shape cases, then qualify Continuous Batching and Prefix Reuse
   combinations independently.

This order is a dependency order, not a claim that every step should ship. A
step proceeds only when its declared correctness, resource, failure, and
end-to-end performance evidence justifies the additional route.

## Scope

This design defines:

- the private seam between KV layout and attention execution;
- the attention fields bound into an exact Execution Route;
- distinct contiguous, gathered, required-fused, native Decode, and native
  tiled-Prefill paths;
- Turn Plan, Turn, Turn Receipt, Plan Rejection, and fallback laws;
- the dependency order between PagedKV and attention implementations;
- correctness, resource, failure, and performance qualification gates; and
- how later Continuous Batching and Prefix Reuse routes compose with attention.

It does not implement a kernel, change the Rust-facing Backend Interface,
modify the scheduler, change P0 readiness, or certify a product claim.

## Terminology And Evidence Boundary

The term **FlashAttention** in this document means the IO-aware exact-attention
algorithm family introduced by the
[FlashAttention paper](https://arxiv.org/abs/2205.14135) and refined in
[FlashAttention-2](https://arxiv.org/abs/2307.08691). The papers establish
algorithm vocabulary. They do not establish performance, resource bounds, or
Metal behavior for TurnVector.

Frozen inspection of upstream MLX commit
[`8a81722b1d71cac9b7dde47e56a438c4b529129b`](https://github.com/ml-explore/mlx/commit/8a81722b1d71cac9b7dde47e56a438c4b529129b)
shows that its Python SDPA binding exposes a required-fused policy and that the
Metal implementation contains several shape-dependent paths and may prefer an
unfused path unless fusion is required:

- [`python/src/fast.cpp`](https://github.com/ml-explore/mlx/blob/8a81722b1d71cac9b7dde47e56a438c4b529129b/python/src/fast.cpp#L229-L338)
- [`mlx/backend/metal/scaled_dot_product_attention.cpp`](https://github.com/ml-explore/mlx/blob/8a81722b1d71cac9b7dde47e56a438c4b529129b/mlx/backend/metal/scaled_dot_product_attention.cpp#L601-L723)

That source is design input only. It does not qualify the P-1 tested MLX pin
`68cf2fddd8de5edd8ab3d926391772b2e2cedad8`, a future production pin, the C++
API surface, or any TurnVector route. A selected dependency revision must be
audited and certified as part of the exact route that uses it. This design does
not change P-1A `YELLOW`, P-1B `PENDING`, or P-1C `RED`.

Consequently, `flash_attention = true` is not a valid capability or route
field. It would collapse different phases, layouts, dispatch rules, kernels,
and evidence envelopes into an identity that cannot fail closed.

## Architectural Boundary

The design stays below the existing coarse Backend Interface:

```text
Rust Runtime Core
  Scheduling Snapshot -> Turn Plan -> one synchronized Turn -> Turn Receipt
                         |
                         v
                 C++/MLX Adapter
                  /             \
         KV Layout Module    Attention Module
                  \             /
                   exact Execution Route
                         |
                         v
                    MLX / Metal
```

The Runtime Core sees only exact capabilities, bounded cost/resource estimates,
typed operation results, and Turn Receipts. It never sees tensors, page-table
pointers, MLX objects, Metal pipelines, or per-kernel dispatch choices.

The KV Layout Module owns:

- contiguous and PagedKV storage ABIs;
- page allocation, ownership, update, release, and reclamation state;
- logical-token to physical-page mapping and block-table validation;
- gather/materialization plans used by reference routes; and
- the block-table reader ABI consumed by native attention routes.

The Attention Module owns:

- exact attention mathematics and graph-declared compute input, output, and
  accumulation dtype semantics;
- Prefill and Decode phase handling;
- mask, position, causal-alignment, grouped-query, and scaling semantics;
- MLX dispatch policy and native kernel bundles;
- scratch/reduction plans and attention-specific resource accounting; and
- attention-path observations written into the Turn Receipt.

It does not own model-weight quantization or KV storage encoding. The existing
weight-layout ABI owns weight dtype, packing, quantization metadata, and
dequantization contract. The KV/cache layout ABI owns KV storage dtype,
quantization, and page encoding. Attention kernels consume those exact ABIs and
are qualified against them without becoming a second authority.

Neither module owns Admission, cross-model scheduling, global Resource Mode,
residency selection, or retry policy. Both run under the Device Executor owner
thread. Attention-route execution invokes them only from the already authorized
internal Turn Plan. Outside Turn execution, the KV Layout Module may serve
owner-thread Request Materialization and Request State Release, and either
module may participate in applicable load, unload, or cache-reclaim Residency
Transition operations. Those coarse Backend Interface operations cannot select
or execute an Attention Path or create route authority.

The module boundary is semantic rather than a requirement for a generic virtual
interface. The first implementation should use the narrowest private typed
descriptors needed by real routes. A common storage abstraction is introduced
only when at least two implementations need it and it hides more complexity
than it exports.

## Exact Route Identity

Attention adds one independent top-level composition identity to the canonical
Execution Route descriptor, not a second copy of the fields owned by its graph,
kernel/fusion, KV/cache, memory, or command members. A separate identity is
necessary because contiguous, gathered, and direct block-table execution cross
those members and vary independently from KV layout. The Attention Path owns its
stable implementation kind and the canonical references that compose one path;
it owns no copied ABI payload.

| Authority or route member | Attention-specific content |
| --- | --- |
| Graph ABI and graph artifacts | `attention_semantics_abi` and `mask_position_abi`, uniquely fixing compute input/output and accumulation dtypes, scaling, softmax, causal alignment, positions, grouped-query mapping, and output semantics. Every multi-member graph ABI also fixes a stable Batch Execution Kind of `TENSOR_BATCH` or `SEQUENTIAL_MEMBER_LOOP`. |
| Weight-layout ABI | Model-weight storage dtype, packing, quantization metadata, and dequantization contract; attention consumes this ABI but does not redefine it. |
| Kernel-bundle and fusion-plan identities | `dispatch_policy`, exact reachable MLX/Metal `kernel_bundle_id`, compilation inputs, and conformance to the graph and weight ABIs. |
| KV/cache layout ABI | `kv_layout_abi`, `kv_access_path`, KV storage dtype/quantization, page encoding, and exact `block_table_reader_abi` or `NONE`. |
| Memory-plan and arena identity | Exact gather, temporary, online-softmax, reduction, and command-buffer `scratch_plan_id` and bounds. |
| Attention Path identity | Stable `attention_path_kind`, exact `PRECOMPILED_REQUIRED` or `BOUNDED_FIRST_USE` compilation timing policy, `NONE_AFTER_START` fallback behavior, and canonical references to the exact graph, kernel/fusion, KV/cache, memory, and command members that form the path. It defines composition and lifecycle policy, not those members' payloads. |
| Speculative Decode and Prefix Reuse plans | Existing exact plan identity or explicit `NONE`; Prefix Reuse fixes only its stable kind `NONE`, `PRIVATE_REUSE`, or `NATIVE_PAGE_SHARING` and bounded implementation identity. Producer publication generation, complete token prefix, and other entry-compatibility facts are dynamic Request Materialization compatibility facts, not route members. |
| Command-submission or replay plan | Exact submission/replay behavior or explicit `NONE`; it never authorizes route substitution. |
| Capability Key and Case Bound Table, outside the route descriptor | Exact `attention_phase` and configured runtime batch/Shape case. They reference, rather than redefine, the route-owned model geometry, compute dtype, weight layout, KV storage/page ABI, kernel tile, and scratch identities. |

There is one authority for each concept and no independently mutable Attention
Path by KV-layout compatibility matrix. The complete canonical Execution Route
descriptor is the sole representation of one structural composition. The Model
Descriptor binds its finite route identities, the Capability Requirement Set
enumerates them per request, and the Model Planner is the sole runtime owner of
structural path/layout compatibility. It may consider only complete declared
compositions compatible with the eligible request facts and supplied hard
constraints; it cannot infer a new pair.

The offline Certification compiler verifies the Coverage Manifest and evidence
for each declared composition and emits a read-only Certified Execution Profile
into the Certification Authorization Index; it is not an authorization owner. It
rejects an Attention Path whose reference does not equal the corresponding
canonical route member, targets an unknown member, declares support outside that
member, or combines members whose support declarations do not intersect for the
exact Key. The immutable Certification Record supplies authorization, and
Admission alone derives runtime applicability and each request's Authorized
Capability Set. A change to the path kind or any referenced member changes the
Attention Path and Execution Route identities. Phase and shape are not
duplicated as route members; graph, kernel, KV, and memory members declare only
the exact support that their Capability Key and Case Bound Table may reference.

During Candidate Formation the Model Planner receives eligible request facts and
hard constraints, not an Authorized Capability Set or Certification
Applicability evidence. It may propose only a finite structurally declared exact
Key; if no compatible Candidate contains an eligible request, the result carries
a Candidate Exclusion. Core associates a returned Candidate only when every
member's Authorized Capability Set contains that Key. The Adapter performs final
canonical equality and route/state applicability validation on a still-current
Turn Plan; drift may return Plan Rejection only before any route operation
starts. This locates each rejection at the earliest layer that can prove it
without creating a second compatibility or authorization owner.

The canonical members produce one Execution Route Identity. The Capability Key
combines that identity with Model Revision, phase, configured batch/Shape,
builds, Interface revision, and Certification Envelope. Its Case Bound Table
supplies the exact case bounds. Environment Qualification remains reusable
evidence for its exact Envelope and does not embed the phase or Shape case. The
finite Coverage Manifest maps each exact Key and Route to the applicable Adapter
Conformance, Model Revision, Environment, and Case evidence. Certification
Records compile from those complete Evidence Sets, and the offline compiler
emits one Certified Execution Profile for each exact Key with references to its
applicable Record, Environment Qualification, and Case Bound Table. Changing
only phase or Shape changes the Key and selected exact case; one finite Case
Bound Table or Environment Qualification may be reused only when the Coverage
Manifest explicitly maps that case and the complete Envelope is unchanged.

Runtime measurements can make an authorized route temporarily infeasible, but
cannot authorize an absent route. Likewise, observing a native or fused kernel
in one Turn Receipt cannot promote a broader shape, phase, layout, or
environment.

## Route Matrix

This plan reuses the canonical PagedAttention design's Attention Path names and
independent composition identity. `PINNED_AUTO` and `REQUIRE_FUSED` are
kernel/fusion-owned dispatch-policy values referenced by a path, not a second
set of Attention Path discriminants. The normative mapping is:

| Attention path | KV access | Dispatch/kernel member | Phase | Role and qualification boundary |
| --- | --- | --- | --- | --- |
| `CONTIGUOUS_MLX_SDPA` | Direct contiguous | `PINNED_AUTO` | Prefill or Decode as separate Capability Keys | Baseline reference role. Every reachable pinned dispatch variant must be enumerated and bounded for each certified case. It is not called FlashAttention. |
| `PAGED_GATHER_MLX_SDPA` | Paged gather to contiguous | `PINNED_AUTO` | Prefill or Decode as separate Capability Keys | First PagedKV reference route. It qualifies layout/update semantics independently from native page-reading attention. Gather costs and transient memory are part of its exact bounds. |
| `CONTIGUOUS_MLX_SDPA` or `PAGED_GATHER_MLX_SDPA` | Matching access path | `REQUIRE_FUSED` plus exact reachable bundle | Prefill or Decode as separate Capability Keys | Optional pinned-MLX route combination. Known missing support excludes the Work Candidate; Backend-owned drift on a still-current Turn Plan returns Plan Rejection only before any route operation starts, never permission to use automatic dispatch. |
| `NATIVE_BLOCK_TABLE_ATTENTION` | Direct PagedKV reader | `NATIVE_EXACT` plus Decode kernel bundle | Decode | PagedAttention `PA04` owns the sole implementation and qualification of one exact small-query geometry. It validates direct page access and Decode semantics; it is not automatically described as FlashAttention. |
| `NATIVE_BLOCK_TABLE_ATTENTION` | Direct PagedKV reader | `NATIVE_EXACT` plus tiled online-softmax bundle | Prefill | FlashAttention `FA03` owns one exact IO-aware tiled Prefill geometry. This is the first route for which the FlashAttention family name is technically meaningful. |

The Execution Route Identity is the hash of the canonical descriptor members,
not a hash of this explanatory table. The Capability Key combines that identity
with the row's exact phase and shape case. The `attention_path` token alone is
never authoritative, and Prefill and Decode never inherit one another's
evidence even when they share source files or compiled libraries.

## Compilation And Cold-Start Policy

Every Attention Path fixes exactly one compilation timing policy:

- `PRECOMPILED_REQUIRED` prepares the exact kernel bundle inside the existing
  model-load variant of the bounded owner-thread Residency Transition. This is
  artifact preparation, not a new Residency variant, Attention Path selection,
  or execution. The route and policy already exist in the immutable Model
  Descriptor, Certification Record, and Certification Authorization Index; a
  later Capability Requirement Set may enumerate them, but load mutates none of
  those artifacts and cannot rewrite a frozen Authorized Capability Set. Its
  compilation time is Residency Service under the applicable Backend Operation
  Bound Set, while the retained artifact and load allocations remain covered by
  the Residency Reservation and route resource evidence. Successful load
  advances Backend Generation, and the Revision remains unavailable for
  Candidate Formation until post-load `describe_model` reproduces the registered
  Model Descriptor exactly. A failed or cooperatively cancelled started load is
  a Residency Failure: it strongly rolls back to zero retained Backend residency
  ownership, marks the Revision Unavailable, fails every Warming waiter, and
  stops automatic retry. Cancellation by the last waiter before loading starts
  remains ordinary Residency Demand withdrawal and makes no Backend load call.
- `BOUNDED_FIRST_USE` permits compilation only inside `execute_turn`. Beginning
  that compilation starts the Turn, and the complete direct-call interval is
  Engine Service. The exact Case Bound Table and Resource Reservation include
  worst-case cold compilation time, allocations, inference, synchronization,
  and failure cleanup. The Model Descriptor and Residency Reservation reserve
  any compiled artifact retained after first use before Admission; the Turn
  cannot grow unreserved persistent model state. The same Key always schedules
  against the conservative cold bound; warm-cache telemetry cannot narrow it or
  authorize a warm-only case. A tighter warm-only bound requires a distinct
  `PRECOMPILED_REQUIRED` route identity and complete independent qualification.

Shared-production promotion defaults to `PRECOMPILED_REQUIRED`. A
`BOUNDED_FIRST_USE` route may be promoted only through an explicit product gate
that accepts its cold-start availability consequence and after repeated exact
compile success, failure, resource, and bound qualification on the named MLX,
Metal, macOS, Adapter, and device Envelope. A missing required precompiled
artifact detected before a route operation starts may produce Plan Rejection. A
first-use compile failure after start is a typed started-Turn failure: that
started Turn fails, no gathered or automatic route runs in its place, and only a
later fresh Scheduling Snapshot may select a separately authorized alternative.

Compilation is not assumed to expose an interruptible interval, and compile
completion is not a P0 cancellation boundary. While the synchronous owner-thread
call is active, an external thread may set the read-only Device Control Signal.
That atomic flag asks the operation to reach its next declared safe point but
carries no semantic order, cannot enter Cancel Pending, cannot identify the
winning cancellation/Turn Receipt order, is not Cancellation Accepted, and
cannot select one Batch member's terminal outcome. The Adapter does not
force-abort MLX or Metal compilation; it continues the exact planned route to
its next qualified synchronized Turn boundary and returns a trustworthy
Turn Receipt within the exact bound.

Once owner-thread control returns, the Event Loop reconciles the queued
cancellation command and returned Turn Receipt through the existing contiguous
Event Sequence before Output Publication. If Core orders an authorized
cancellation first, that member enters Cancel Pending; Turn Receipt commit
consumes the started result and discards the member's staged output, including
when none exists. If Turn Receipt commit and Output Publication order first, a
later cancellation cannot revoke them. A signal without a matching Core-ordered
command cannot cancel a request, fabricate a Member Outcome, or determine Batch
scope. A route with no conservative bound to the synchronized boundary is
ineligible for shared execution.

## Scheduling, Execution, And Fallback Laws

The existing
`Scheduling Snapshot -> Turn Plan -> Turn -> Turn Receipt` lifecycle remains
unchanged.

### Scheduling Snapshot

The Scheduling Snapshot contains the loaded immutable Revision, exact installed
and qualified route identities, bounded reusable allocations,
allocator/resource observations, and relevant Pending Reclaim state. It does
not dynamically discover a new fused path and add authority. A
`PRECOMPILED_REQUIRED` route is declared before load but remains infeasible while
the Revision is non-resident or awaiting exact post-load Model Descriptor
revalidation; a `BOUNDED_FIRST_USE` route carries its complete cold bound without
relying on current cache warmth.

### Candidate Formation And Turn Plan

Candidate Formation may emit only a structurally declared Work Candidate whose
exact Capability Key is compatible with the eligible request facts and supplied
hard constraints. Known structural incompatibility produces a Candidate
Exclusion; it is not a fallback. The Model Planner receives no Certification
Applicability or Authorized Capability Set. Core creates a current Candidate
Association only after the exact Key is present in every member's Authorized
Capability Set and all current Core evidence admits it. The Turn Arbiter may
select only that associated Candidate, and selection creates a Turn Plan that
records its exact route, case bounds, worst-case Engine Service, reservation,
transient headroom, scratch plan, output bound, and typed preconditions. The
associated Key, route, and case bind phase, mask, position, graph-owned compute
dtype, weight-layout ABI, KV storage ABI, model geometry, sequence bounds,
runtime batch/Shape, dependency revision, compiled kernel identity, and
environment.

If Backend-owned pre-execution validation finds that a still-current Turn Plan
cannot begin, `execute_turn` returns Plan Rejection only before any route
operation, including first-use compilation, starts. The accepted rejection
consumes no Engine Service and produces no Turn Receipt. The Scheduler first
takes the required fresh Scheduling Snapshot and cannot substitute work in the
rejected Turn Plan. Rejection-driven Candidate Formation then runs as its
separately obligated support envelope and may create a new Work Candidate for
another already authorized route. Only a later fresh Scheduling Snapshot may
select that candidate and create a new Turn Plan with complete independent
bounds. No rejected route is rewritten or retried.

### Turn

The owner thread executes only the route named by the Turn Plan. It does not
retry, change KV access, switch from native to gather, or switch dispatch policy
after any route operation starts. Beginning first-use compilation starts the
Turn even before an MLX kernel runs. Compilation failure is a typed started-Turn
failure, compilation-bound excess is a Bound Violation, and neither can be
relabeled as Plan Rejection. Any started required-fused or native path must
return a trustworthy synchronized Turn Receipt with typed outcomes under the
existing isolation/quarantine rules. If it cannot, the process fail-stops and
fabricates no Turn Receipt.

Cancellation remains cooperative at qualified synchronized Turn boundaries.
Compile completion alone is not such a boundary in P0, and Device Control Signal
observation supplies no cancellation order. A compiler or kernel that cannot
return the Turn Receipt within the declared bound and next-safe-point allowance
is ineligible rather than made interruptible by adding unbounded polling,
force-abort, or concurrent Metal work.

### Turn Receipt

The Turn Receipt records the planned route identity and bounded observations
needed for audit, including the actual MLX/native implementation variant where
it can be observed reliably, scratch high-water marks, gather/materialization
use, command-buffer counts, timings, and failure classification. Observations
refine evidence and detect drift; they do not grant authority.

### Contract Surface And Conformance

The operation-start boundary is one lockstep contract across the `CONTEXT.md`
definitions of Plan Rejection, Engine Service, Cooperative Cancellation, and
Cancel Pending; ADR 0016; ADR 0042; the PagedAttention route laws; the Backend
Interface `execute_turn` result types; and fake/native conformance tests. An
implementation slice must audit all of those surfaces together and reject any
older rule that treats MLX kernel submission, rather than first route operation,
as Turn start.

The contract fixture must distinguish at least: unsupported or missing state
before route work as Plan Rejection; compilation followed by success, typed
failure, or bound excess as started-Turn paths requiring either a trustworthy
Turn Receipt or fail-stop; Device Control Signal observation versus a
Core-ordered cancellation; and queued cancellation before versus after
Turn Receipt commit. Decode B1 fixtures include a compile-period Device Control
Signal with no matching Core cancellation and no staged output. Decode B4
fixtures order authorized cancellation for exactly one, some, and all four
members both before and after Turn Receipt commit, including no-output discard.
Every started case asserts the frozen route identity and proves that no alternate
path executed.

## PagedKV Coordination

PagedKV is a storage and lifetime mechanism. FlashAttention is an attention
algorithm family. PagedAttention commonly combines direct block-table access
with attention execution, but TurnVector must not merge these concerns into one
unversioned feature.

The PagedKV design owns:

- page size and page-table encoding;
- logical length, tail handling, and partial-page semantics;
- allocation, copy/update, release, and Pending Reclaim behavior;
- page dtype and quantization layout;
- block-table validation and reader ABI; and
- a gathered contiguous reference view.

This design owns the consumer side of that reader ABI. Native attention must
reject malformed, stale, out-of-bound, semantically incompatible, or illegally
aliased tables before submitting GPU work. Duplicate mutable references,
cross-pool aliases, and sharing outside the exact ownership ABI are illegal.
An Execution Route whose Prefix Reuse plan kind is `NATIVE_PAGE_SHARING` may
permit reference-counted immutable physical pages to appear in multiple request
block tables. Its static route binds the stable plan implementation, KV/cache
layout ABI, pool, copy-on-write, release, and consumer reader ABI. Only
owner-thread Request Materialization may adopt a dynamic entry after validating
complete route equality and every additional prefix-entry compatibility field,
including producer publication generation and exact token prefix; the
Materialization Result binds the adopted prefix, route, and Backend Generation.
`PRIVATE_REUSE` retains private physical state and cannot authorize shared pages.

The detailed PagedAttention proposal in
[PR #16](https://github.com/token-plant/TurnVector/pull/16) has merged. Its shared
contract now lives in the tracked
[`CONTEXT.md`](../../CONTEXT.md),
[ADR 0016](../adr/0016-distinguish-plan-rejection-from-started-turn-outcomes.md),
[ADR 0031](../adr/0031-authorize-shared-work-only-from-scoped-certification.md),
[ADR 0042](../adr/0042-distinguish-paged-kv-layout-from-attention-execution.md),
[PagedAttention plan](2026-08-19-paged-attention-route.md), and
[native-model TODO](../todo-own-native-model-graphs-and-operators.md). This
FlashAttention design consumes those canonical sources rather than pinning a
live pull-request head or carrying a merge-order dependency.

The resulting shared contract is:

1. Attention Path is a composition identity whose stable kind varies
   independently from KV layout.
2. Its references must equal the route's graph, kernel/fusion, KV/cache, memory,
   and command members; those members remain the sole owners of their ABI
   payloads.
3. PagedAttention owns the gathered reference and first native block-table
   Decode slices. FlashAttention does not redefine them.
4. First-use compilation starts the Turn; compile failure is a typed started-Turn
   failure, compile-bound excess is a Bound Violation, and neither is Plan Rejection.

This change adds only the Flash-specific decision, detailed route design, and
remaining delivery order. Any later edit to a canonical shared source must
revalidate this design before merge; a PR-head SHA is not an authority.

## Continuous Batching And Prefix Reuse

The accepted
[Continuous Batching design](2026-08-19-continuous-batching-optimization.md) and
[Prefix Sharing design](2026-08-19-prefix-sharing.md) own their canonical
semantics. This plan owns only their composition with attention routes.
Continuous Batching changes the member/row shape, graph ABI, and scheduling
case. Prefix Reuse changes the static plan kind and bounded implementation in the
route, while dynamic prefix-entry compatibility remains Request Materialization
facts. Neither capability is implied by qualifying a single-request attention
route.

Expansion follows these rules:

- first qualify one-request exact Prefill and Decode cases;
- represent same-Model multi-member batches as separate exact route cases;
- bind `TENSOR_BATCH` versus `SEQUENTIAL_MEMBER_LOOP` into the graph ABI's stable
  Batch Execution Kind, so the same batch/Shape bucket has different Execution
  Route Identities and Capability Keys and shares neither Certification nor
  tensor-batch telemetry;
- bind padding, packing, ragged representation, assembly, and member reduction
  order into the graph, kernel, and memory-plan members, then freeze ordered
  membership and member-local work ceilings in the Turn Plan through its
  synchronized Turn Receipt with no mid-Turn insertion, removal, or reordering;
  after route work starts, cancellation follows the frozen member-local
  Turn Receipt contract rather than changing Turn Plan membership;
- bind only the stable Prefix Reuse plan kind and bounded implementation into the
  static route. For `NATIVE_PAGE_SHARING`, owner-thread Request Materialization
  validates the complete additional entry compatibility fields defined by the
  Prefix Sharing design, and the Materialization Result binds the adopted prefix,
  Execution Route, and Backend Generation;
- rerun correctness and bound evidence for every supported combination; and
- omit unsupported Cartesian combinations rather than adding wildcard keys.

A scheduler may compare costs among already certified combinations, but does
not inspect kernels or synthesize a combination that lacks a Capability Key.

## Resource And Failure Accounting

Each route declares conservative bounds for every resource it can retain or
transiently create, including:

- resident Q/K/V and output storage;
- KV pages and block tables;
- gather/materialization buffers;
- tiled softmax statistics and reduction scratch;
- compiled libraries, pipeline state, graph caches, and command buffers;
- either the retained precompiled artifact and bounded Residency preparation or
  bounded first-use compilation time, allocations, and temporary artifacts;
- lazy-evaluation temporaries and synchronization materialization;
- allocator fragmentation and declared safety margin; and
- teardown allocations that remain Pending Reclaim until observed convergence.

Required-fused and native paths do not get a smaller reservation merely because
they are expected to reduce memory traffic. Only measured high-water evidence
inside the exact case envelope can support a new conservative bound.

Failure tests cover invalid route identity, unsupported geometry, malformed
block tables, stale page generations, insufficient reservation, required-fused
unavailability, kernel compilation/load failure, numerical non-finites, bound
overrun, cancellation, backend loss, teardown, and failure to observe reclaim
convergence. They distinguish a missing `PRECOMPILED_REQUIRED` artifact before
route work, which may be Plan Rejection, from a `BOUNDED_FIRST_USE` compile that
has begun, whose failure is a typed started-Turn failure and whose excess is a
Bound Violation. They also prove that a compile-period Device Control Signal
neither force-aborts compilation nor orders cancellation, the Turn reaches its
qualified synchronized boundary within the bound, and only the later contiguous
Event Sequence can place a member in Cancel Pending and discard its staged
output, including the no-output case. An untrustworthy started result fail-stops,
and a started-Turn failure never triggers hidden replay through the gathered
reference route.

## Correctness Qualification

The first native PagedKV attention route requires a three-way oracle where the
model and shape permit it:

1. independent offline reference attention;
2. pinned MLX SDPA over a gathered contiguous view of the same logical KV; and
3. the native direct block-table route.

Qualification covers forward outputs and multi-turn state transitions, not a
single isolated kernel call. The matrix includes:

- Prefill and Decode as separate suites;
- graph-owned compute input/output and accumulation dtype behavior;
- weight-layout-owned storage, packing, quantization, and dequantization;
- KV-layout-owned storage dtype, quantization, and page encoding;
- causal masks and nonzero query/key position offsets;
- grouped-query or multi-query head mapping where supported;
- boundary sequence lengths around page and tile edges;
- partial final pages, empty/one-token edges, and maximum certified lengths;
- deterministic page fragmentation and non-monotonic physical page order;
- rejection of mutable duplicates, cross-pool aliases, and aliases outside the
  exact KV ownership ABI;
- acceptance and copy-on-write isolation of immutable shared pages only for an
  exact qualified route whose Prefix Reuse plan kind is `NATIVE_PAGE_SHARING`,
  after owner-thread Request Materialization validates complete dynamic entry
  compatibility and its Materialization Result binds the adoption;
- gathered and direct-reader equivalence;
- finite-value and error-tolerance policies declared before results; and
- load, cancellation, release, and subsequent-request isolation.

Sliding windows, attention sinks, arbitrary masks, unsupported mixed weight/KV
quantization combinations, ragged batches, or other semantics are rejected
until each is explicitly represented and qualified. Passing a nearby case does
not authorize them.

## Performance And Evidence Gates

No microbenchmark or paper result can promote a serving route. Before data is
collected, each candidate declares its exact environment, cases, repetitions,
warmup, thresholds, comparison route, measurement points, and raw-artifact
retention according to `docs/evidence-policy.md`.

The end-to-end matrix records at least:

- TTFT, TPOT, Engine Service, and request latency distributions;
- throughput and runnable-only fairness under the supported workload;
- process footprint, allocator observations, reserved bytes, transient
  high-water, and reclaim convergence;
- gather bytes, scratch bytes, command-buffer shape, and compilation/warmup;
- cancellation and failure-path service bounds; and
- acceleration and actual implementation-path observations where reliable.

Comparisons are phase-specific and exact-route-specific. A Prefill win cannot
offset a Decode regression unless a separately declared product workload and
promotion rule says so. A result from current upstream MLX, a different Mac,
another model revision, or an uncertified batch shape is research input only.

Promotion requires all of the following:

1. Exact identity and applicability cases are finite and fail closed under
   one-field drift.
2. Correctness, multi-turn KV state, cancellation, cleanup, and isolation pass.
3. The exact artifact compiles and loads on the named Envelope;
   `PRECOMPILED_REQUIRED` routes remain infeasible until the existing model-load
   transition, Backend Generation advance, and post-load Model Descriptor
   equality succeed without mutating capability or authorization descriptors,
   while any promoted `BOUNDED_FIRST_USE` route passes its cold-start
   availability, resource, cancellation, and failure gates.
4. Resource and Engine Service bounds pass with the declared safety margins.
5. End-to-end comparison meets predeclared thresholds for every promoted case.
6. Turn Receipt telemetry proves the intended route ran without being used as
   authorization.
7. The P0 owner-thread topology and coarse Backend Interface remain unchanged.

## Delivery Slices

Each implementation slice is independently reviewable and must use signed,
policy-compliant commits. A later slice never widens an earlier route identity.

Ownership is singular:

| Owner and slice | Sole responsibility | Consumer contract |
| --- | --- | --- |
| PagedAttention `PA01`-`PA02` | Initial KV-layout and independent Attention Path identities plus exact Capability/Profile/quarantine propagation. | FlashAttention adds exact path compositions without redefining the stable kinds or ownership schema. |
| PagedAttention `PA03` | PagedKV layout, lifecycle, and `PAGED_GATHER_MLX_SDPA` reference route. | Every native PagedKV attention route uses its qualified reader ABI and gathered oracle. |
| PagedAttention `PA04` | First `NATIVE_BLOCK_TABLE_ATTENTION` Decode implementation and qualification. | FlashAttention consumes the reader/evidence boundary and never duplicates Decode. |
| FlashAttention `FA01` onward | Exact dispatch extensions, tiled Prefill implementation, expansion, composition, and promotion. | It does not own PagedKV allocation/lifetime or the prior PA slices. |

### FA01: Exact Dispatch Identity Extension

- add automatic, required-fused, and native Attention Path compositions without
  changing PA path discriminants;
- bind one exact compilation timing policy and reject unknown or mismatched
  path/KV-layout compositions in the offline compiler;
- reference mask/position, scratch, reachable kernel artifacts, KV access, and
  submission through their existing graph, memory, kernel, KV, and command
  members without copying those payloads;
- compile finite exact Capability Keys and reject wildcard combinations; and
- add contract-level fake/native fixtures proving Candidate Exclusion for an
  absent pair, Plan Rejection before route work, started-Turn classification
  after compile begins, exact Decode B1 signal-only/no-output behavior, exact
  Decode B4 one/some/all-member cancellation ordering, no-output discard, and no
  fallback.

### FA02: Baseline Dispatch Evidence

- inventory every MLX SDPA implementation reachable at the selected pin;
- expose reliable implementation-path observations where the native API allows;
- qualify pinned automatic dispatch only when all reachable variants are
  enumerated and bounded;
- prove exact precompile/load success or the complete first-use cold path on the
  named Envelope, including load failure/cancellation rollback, Backend
  Generation advance only on success, post-load Model Descriptor equality, and
  no descriptor, index, or Authorized Capability Set mutation; and
- optionally add required-fused cases as separate routes, never as fallback.

### PA03 Shared Dependency: PagedKV Gather Reference

The PagedKV stream supplies its exact layout, block-table reader ABI, lifecycle,
and gathered MLX SDPA route. This is not reimplemented by the FlashAttention
stream. Native page-reading work does not start until that reference passes its
own correctness and resource gates.

### PA04 Shared Dependency: Native Block-Table Decode

PagedAttention `PA04` is the sole owner of the first native block-table Decode
implementation and qualification. It binds the qualified reader ABI, one exact
Decode geometry, kernel, scratch, mask/position, and case evidence. The
FlashAttention stream consumes that reader and evidence boundary but neither
reimplements nor independently certifies the Decode route.

### FA03: Native Block-Table Tiled Prefill

- implement IO-aware tiled exact attention for one exact Prefill geometry;
- qualify tile/page boundaries, online-softmax numerics, and scratch bounds;
- compare it with the gathered reference for the identical logical KV; and
- promote only the exact cases that pass every declared gate.

### FA04: Shape Expansion

- add one exact geometry at a time;
- prefer finite generated case tables over runtime heuristics;
- repeat correctness, resource, failure, and performance qualification; and
- preserve old identities and evidence unchanged.

### FA05: Batch And Prefix Compositions

- consume the accepted Continuous Batching contract and add separately qualified
  multi-member routes whose graph ABI fixes `TENSOR_BATCH` or
  `SEQUENTIAL_MEMBER_LOOP`; the same batch/Shape bucket produces different
  Route/Key identities, and every Turn Plan freezes ordered membership through
  its synchronized Turn Receipt;
- add exact Prefix Reuse combinations only after the applicable static plan
  implementation and PagedKV ABI qualify; keep dynamic entry compatibility and
  adoption exclusively in owner-thread Request Materialization and its
  Materialization Result;
- account for shared resource attribution and row/member lifecycle; and
- leave every unsupported combination absent.

### FA06: Promotion And Operations

- publish Certification Records, Case Bound Tables, Environment Qualification,
  and raw evidence references;
- add drift, quarantine, cleanup, and observability conformance;
- update operator-facing capability status without exposing kernel controls;
  and
- preserve the gathered route as a certified alternative only where its own
  cases remain supported.

## Deferred Decisions

- the first model family, compute/accumulation dtype, weight-layout ABI,
  KV-storage ABI, head geometry, page size, and tile size;
- whether the selected MLX C++ pin exposes a supportable required-fused control
  and implementation-path observation;
- the exact native kernel language, compilation, and artifact packaging path;
- promotion thresholds and workload weights, which must be declared before the
  corresponding experiment; and
- whether gathered reference routes remain production-capable after native
  routes qualify.

These are intentionally deferred to evidence-backed implementation changes.
They are not mutable runtime knobs.

## Non-Goals

- Replacing MLX tensor primitives, allocator, streams, or Metal ownership.
- Exposing attention or KV implementation controls through public APIs.
- Calling per-operator MLX or Metal work across the Rust/C++ seam.
- Adding concurrent accelerator Turns or moving work off the owner thread.
- Treating all fused SDPA kernels as FlashAttention.
- Silently falling back after a Turn starts.
- Authorizing a route from runtime observation, current upstream source, a
  paper, or a microbenchmark.
- Making FlashAttention, PagedKV, Continuous Batching, or Prefix Reuse a P0
  readiness requirement.
