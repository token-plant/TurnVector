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
| Graph ABI and graph artifacts | `attention_semantics_abi` and `mask_position_abi`, uniquely fixing compute input/output and accumulation dtypes, scaling, softmax, causal alignment, positions, grouped-query mapping, and output semantics. |
| Weight-layout ABI | Model-weight storage dtype, packing, quantization metadata, and dequantization contract; attention consumes this ABI but does not redefine it. |
| Kernel-bundle and fusion-plan identities | `dispatch_policy`, exact reachable MLX/Metal `kernel_bundle_id`, compilation inputs, and conformance to the graph and weight ABIs. |
| KV/cache layout ABI | `kv_layout_abi`, `kv_access_path`, KV storage dtype/quantization, page encoding, and exact `block_table_reader_abi` or `NONE`. |
| Memory-plan and arena identity | Exact gather, temporary, online-softmax, reduction, and command-buffer `scratch_plan_id` and bounds. |
| Attention Path identity | Stable `attention_path_kind`, compilation timing policy, `NONE_AFTER_START` fallback behavior, and canonical references to the exact graph, kernel/fusion, KV/cache, memory, and command members that form the path. It defines composition and lifecycle policy, not those members' payloads. |
| Speculative Decode and Prefix Reuse plans | Existing exact plan identity or explicit `NONE`; Prefix Reuse carries its producer, publication, immutable-sharing, and lifetime contract. |
| Command-submission or replay plan | Exact submission/replay behavior or explicit `NONE`; it never authorizes route substitution. |
| Capability Key and Case Bound Table, outside the route descriptor | Exact `attention_phase` and configured runtime batch/Shape case. They reference, rather than redefine, the route-owned model geometry, compute dtype, weight layout, KV storage/page ABI, kernel tile, and scratch identities. |

There is one authority for each concept. The offline compiler rejects an
Attention Path whose reference does not equal the corresponding canonical route
member, targets an unknown member, or declares support outside that member. A
change to the path kind or any referenced member changes the Attention Path and
Execution Route identities. Phase and shape are not duplicated as route members;
graph, kernel, KV, and memory members declare only the exact support that their
Capability Key and Case Bound Table may reference.

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

This plan reuses the pending PagedAttention proposal's Attention Path names and
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

## Scheduling, Execution, And Fallback Laws

The existing
`Scheduling Snapshot -> Turn Plan -> Turn -> Turn Receipt` lifecycle remains
unchanged.

### Scheduling Snapshot

The Scheduling Snapshot contains the loaded immutable Revision, exact installed
and qualified route identities, bounded reusable allocations,
allocator/resource observations, and relevant Pending Reclaim state. It does
not dynamically discover a new fused path and add authority.

### Candidate Formation And Turn Plan

Candidate Formation may emit only a Work Candidate whose exact Capability Key
is already in every member's Authorized Capability Set. Known route
inapplicability produces a Candidate Exclusion; it is not a fallback. Selection
of a Work Candidate creates a Turn Plan that records its exact route, case
bounds, worst-case Engine Service, reservation, transient headroom, scratch
plan, output bound, and typed preconditions. Applicability includes phase, mask,
position, graph-owned compute dtype, weight-layout ABI, KV storage ABI, model
geometry, sequence bounds, runtime batch/Shape, dependency revision, compiled
kernel identity, and environment.

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

Cancellation remains cooperative at qualified Turn boundaries. A kernel that
cannot meet the declared bound is ineligible rather than made interruptible by
adding unbounded polling or concurrent Metal work.

### Turn Receipt

The Turn Receipt records the planned route identity and bounded observations
needed for audit, including the actual MLX/native implementation variant where
it can be observed reliably, scratch high-water marks, gather/materialization
use, command-buffer counts, timings, and failure classification. Observations
refine evidence and detect drift; they do not grant authority.

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
Separately identified Prefix Reuse routes may permit reference-counted immutable
physical pages to appear in multiple request block tables only when their exact
producer, publication, generation, pool, copy-on-write, and release contract is
bound and validated. The combined route binds both the producer layout ABI and
consumer reader ABI so either side can evolve without silently changing an
existing identity.

At the time of writing, the detailed PagedAttention proposal is under review in
[PR #16](https://github.com/token-plant/TurnVector/pull/16) at head
`97562561beeaed734929167f9a1155ae17b0be66`. That pending change is coordination
context, not accepted architecture. Its current top-level Attention Path member,
stable path names, `PA03`/`PA04` ownership, and operation-start boundary are
compatible with this design.

The shared contract is frozen here independently of merge order:

1. Attention Path is a composition identity whose stable kind varies
   independently from KV layout.
2. Its references must equal the route's graph, kernel/fusion, KV/cache, memory,
   and command members; those members remain the sole owners of their ABI
   payloads.
3. PagedAttention owns the gathered reference and first native block-table
   Decode slices. FlashAttention does not redefine them.
4. First-use compilation starts the Turn; compile failure is a typed started-Turn
   failure, compile-bound excess is a Bound Violation, and neither is Plan
   Rejection.

This change updates `CONTEXT.md`, ADR 0045, and the native-model TODO, so its
meaning does not depend on PR #16 merging first. Either branch may land first;
the second must resolve overlapping glossary or TODO text without changing this
contract and must be reviewed at its final head. If PR #16 changes from the
frozen head, the coordination section must be revalidated before this PR merges.

## Continuous Batching And Prefix Reuse

Continuous Batching changes the member/row shape and scheduling case. Prefix
Reuse changes the KV producer, publication, sharing, and lifetime identities.
Neither is implied by qualifying a single-request attention route.

Expansion follows these rules:

- first qualify one-request exact Prefill and Decode cases;
- represent same-model multi-member batches as separate exact route cases;
- bind padding/ragged representation and row-removal semantics into the batch
  route;
- bind prefix producer, complete token prefix, graph, model, KV layout, page
  geometry, publication, and sharing identities into a Prefix Reuse route;
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
- bounded first-use compilation time, allocations, and temporary artifacts;
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
convergence. They prove that pre-operation inapplicability is Plan Rejection,
compile failure is a typed started-Turn failure, compile-bound excess is a Bound
Violation, and an untrustworthy started result fail-stops. A started-Turn
failure never triggers hidden replay through the gathered reference route.

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
  exact qualified Prefix Reuse route;
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
3. Resource and Engine Service bounds pass with the declared safety margins.
4. End-to-end comparison meets predeclared thresholds for every promoted case.
5. Turn Receipt telemetry proves the intended route ran without being used as
   authorization.
6. The P0 owner-thread topology and coarse Backend Interface remain unchanged.

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
- reference mask/position, scratch, reachable kernel artifacts, KV access, and
  submission through their existing graph, memory, kernel, KV, and command
  members without copying those payloads;
- compile finite exact Capability Keys and reject wildcard combinations; and
- add one-field drift and canonical-identity fixtures.

### FA02: Baseline Dispatch Evidence

- inventory every MLX SDPA implementation reachable at the selected pin;
- expose reliable implementation-path observations where the native API allows;
- qualify pinned automatic dispatch only when all reachable variants are
  enumerated and bounded; and
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

- add exact Continuous Batching combinations only after its own design lands;
- add exact Prefix Reuse combinations only after its publication/lifetime
  contract and PagedKV ABI qualify;
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
