# Continuous Batching Optimization Plan

Status: design only; P0 scheduling refinement plus separately qualified native batch routes

Governing decision: `docs/adr/0043-form-continuous-batches-incrementally-at-turn-boundaries.md`

## Objective

Make same-Model Continuous Batching efficient under the existing bounded Turn
contract while preserving global cross-Model arbitration, exact Certification,
complete Candidate Exclusions, and deterministic replay.

TurnVector Continuous Batching means:

- multiple requests may be live concurrently;
- a Work Candidate may contain compatible requests for one Model, phase, Service
  Class, exact Execution Route, batch bucket, and Shape/KV domain;
- membership may change after a synchronized Receipt, cancellation, rejection,
  local stale disposition, or newly eligible request; and
- every next execution still requires a fresh Scheduling Snapshot and Turn Plan.

It does not mean simultaneous cross-Model Metal execution, insertion into an
already started Turn, or an Adapter-owned scheduling loop.

## Non-Goals

- Weaken fresh Snapshot, generation, Capability Key, or Turn Plan validation.
- Continue the previous batch automatically because its members remain runnable.
- Move urgency, Model Weight, service debt, or relative priority into the Model
  Planner.
- Enumerate arbitrary request subsets or let the Backend silently omit requests.
- Fuse different Models, phases, or Service Classes into one native batch.
- Claim throughput or latency improvement before the exact routes are measured.

## Module Ownership

```text
Runtime Core
  Eligibility Index
  Dirty Model Set
  Formation Result Cache
  Current Candidate Associations
  fresh Scheduling Snapshot
             |
             | eligible handles + hard constraints
             v
Execution Backend / private Model Planner
  per-Model Planning Catalog
  compatibility partitions
  canonical bounded batch construction
             |
             | candidates + complete exclusions
             v
Runtime Core / private Turn Arbiter
  safety -> urgency -> fairness -> optimization
             |
             v
        one fresh Turn Plan
             |
             v
ModelRuntime.execute -> synchronized Receipt
```

The external Backend Interface remains one coarse `form_candidates` operation
and one coarse `execute_turn` operation. Internal indexes deepen those Modules;
they do not create a second scheduler or expose KV, Shape, graph, or tensor state
to the Runtime Core.

### Runtime Core Responsibilities

The Runtime Core owns:

- a bounded per-Model eligible-handle index;
- exact request, Scheduler, Backend, Runtime Overhead, Resource, Certification,
  Configuration, and Control dependency generations;
- the dirty-Model set and typed reason each Model became dirty;
- immutable Formation Results returned by completed Candidate Formation;
- current candidate associations that bind those results to Core-owned evidence;
- invalidation before a stale candidate or Plan can execute; and
- fresh Snapshot construction and global Turn selection.

The Core does not infer same-Model compatibility or alter candidate membership.

### Model Planner Responsibilities

The Model Planner owns:

- request-local phase, cursor, KV, Shape, graph, and route compatibility facts;
- bounded compatibility partitions for one Model;
- canonical batch construction within each partition;
- route-specific cost and Resource Impact facts; and
- one typed Candidate Exclusion for every supplied eligible handle absent from
  all returned candidates.

It cannot read Model Weight, service debt, global urgency, or another Model's
candidate set.

## Incremental Formation Model

### Two-Level Dependency Vectors

Planner compatibility and current Core feasibility have different reasons to
change. They therefore use two cache levels rather than one over-broad key.

Every cached Formation Result binds:

- Model ID and Model Revision;
- exact eligible-membership generation for the Model;
- Backend Generation and Cost Profile identity;
- Backend Capability and Model route-catalog identity;
- phase, Service Class, configured batch buckets, and Shape/KV compatibility
  identities; and
- ordered member handles plus their current request-status versions.

Every current candidate association separately binds one Formation Result to:

- the fresh Scheduler Generation and Scheduling Snapshot;
- Runtime Overhead Generation and exact Bound Set witness shared by members;
- Resource Capacity and Support Charge Ledger generations relevant to the Plan;
- Configuration, Certification, Authorized Capability Set, Capability Key, and
  quarantine identities; and
- current timing, output-capacity, entitlement, and hard-feasibility witnesses.

A Model-local Formation dependency change invalidates that Model's Formation
Result and requires a bounded `form_candidates` call. A Core evidence, resource,
time, fairness, or global-generation change invalidates only the current
association unless it also changes a hard constraint supplied to formation.
Core can then re-associate an unchanged Formation Result without calling the
Model Planner. The fresh Snapshot may use an association only when both levels
match, and it never reuses an older Turn Plan.

### Dirtying Rules

Local changes dirty only the affected Model:

- successful Materialization or first eligibility;
- accepted Receipt progress, phase transition, completion, or member failure;
- accepted Cost Profile update;
- cancellation that removes eligible or candidate membership;
- model-local Plan Rejection or local stale disposition;
- Model load, unload, route, or Backend Generation change.

Global evidence or policy changes invalidate exactly their declared association
scope. A Monotonic Time advance, Model service charge, urgency change, ledger
advance, or new Bound Set does not require rebuilding unchanged backend
compatibility; Core re-associates current Formation Results and the fresh Turn
Arbiter reevaluates those facts. An authorization, route-catalog, or hard
formation-constraint change dirties only the Models whose possible candidates
can change.

### Bounded Call Scope

One Candidate Formation call receives only the canonical sorted eligible handles
for its affected bounded Model scope plus hard constraints. Core may coalesce
multiple unstarted dirty causes for that scope into the one already conserved
Support Operation Obligation, but it cannot coalesce across an active obligation
or lose any funder, typed cause, or membership-change requirement.

Formation work is bounded by:

- affected Models per call;
- eligible handles examined per affected Model;
- compatibility partitions touched;
- configured batch buckets considered;
- candidates and exclusions emitted;
- bytes copied and allocations performed; and
- invariant witnesses updated.

No path silently truncates when a hard maximum is reached. Admission or ingress
rejects before accepted state could exceed the binary limits.

## Canonical Batch Construction

For each affected Model, the Model Planner performs this deterministic order:

1. Validate supplied handles and exact current Backend-owned request state.
2. Exclude handles lacking an authorized exact route or hard resource/timing fit.
3. Partition by Model Revision, Execution Phase, Service Class, exact Execution
   Route, and Shape/KV compatibility identity.
4. Order members by the canonical stable request key defined by the domain
   schema, never by global urgency, Model Weight, or service debt.
5. For each configured certified batch bucket, construct at most one candidate
   using the canonical compatible prefix of that order.
6. Emit complete typed exclusions for every supplied handle absent from all
   candidates, including exact incompatibility or bounded-capacity cause.

The configured bucket set is finite and versioned. B1, B2, B4, or any other
bucket is available only when the exact route and Shape have applicable
Certification; this plan does not preselect a universal bucket set.

Candidates for different buckets may overlap because they describe alternatives.
The Turn Arbiter selects one complete presented candidate and cannot edit it.
After selection, the Turn Plan freezes ordered membership and member-local work
ceilings through the synchronized Receipt.

## Native Batch Execution

The native route must distinguish at least:

- a genuine tensor batch with one graph execution over the frozen members;
- a sequential per-member loop, which is not reported as tensor batching;
- padding, packing, or ragged assembly rules;
- batch/Shape bucket and graph artifact identity; and
- member-local output, KV update, failure, and cancellation semantics.

Any change to assembly, padding, mask construction, graph artifacts, kernels,
or member reduction order changes the bounded Execution Route descriptor. The
Capability Key already fixes configured batch and Shape; implementation must
also bind the exact batch assembly identity into the route's graph, kernel, and
memory-plan members before a multi-member route can be certified.

One member cannot be inserted, removed, or reordered after execution starts.
Cancellation observed before the safe point rejects or invalidates the Plan;
cancellation after start follows the frozen member-local Receipt contract.

## Fresh Arbitration Is Preserved

The optimized loop is:

```text
Receipt accepted
  -> update only Receipt members and maintained aggregates
  -> mark affected Model dirty when compatibility changed
  -> complete required observation and formation support envelopes
  -> rebuild only dirty Model candidates
  -> create a fresh Scheduling Snapshot
  -> reuse only dependency-current unaffected candidate associations
  -> Turn Arbiter selects under current safety/urgency/fairness policy
  -> create and revalidate one fresh Turn Plan
```

Batch stickiness may appear only as a versioned optimization fact used after
safety, urgency, and fairness, and only if its cost and starvation effects are
qualified. It cannot bypass a candidate, Snapshot, Plan, or service-debt choice.

## Performance And Correctness Probes

### Planning Probes

- Candidate Formation calls and typed causes;
- Models marked dirty and Models actually recomputed;
- eligible handles, partitions, buckets, and candidates examined;
- Candidate Exclusions emitted by reason;
- Formation Result reuse/invalidation and candidate re-association by dependency;
- bytes copied, allocations, invariant operations, and complete call latency;
- fresh Snapshots and Plans created per accepted Receipt;
- stale association and Plan rejection counts.

### Serving Probes

- batch fill and padding ratios by exact route/bucket/Shape;
- batch membership churn and time spent at each batch size;
- TTFT, TPOT, Turn latency, completion latency, and throughput distributions;
- prefill/decode interference and per-Service-Class results;
- Model Engine Service, debt, starvation, and deadline outcomes;
- KV, transient, output, allocator, process-footprint, and Pending Reclaim data;
- per-member logits, token, KV, Sampling State, output-order, and failure parity.

Measurements compare an incremental implementation with a deterministic full
recompute oracle and with the exact B1 baseline. Thresholds, matrices, route
identities, environment, inputs, and seeds are fixed before a qualification run.

## Verification Laws

1. Incremental formation produces byte-identical candidates and exclusions to
   the bounded full-recompute oracle for the same complete input.
2. A member-local Receipt touches only its members, maintained aggregates, and
   the affected Model's dirty state.
3. An unaffected Model performs zero compatibility scans and preserves its
   Formation Result identity when its local dependency vector is unchanged;
   Core-only drift may replace the association without a planner call.
4. Every accepted Receipt is followed by a fresh Snapshot and Plan before any
   continuation executes.
5. Every eligible handle appears in a candidate or one typed exclusion.
6. Candidate counts and work stay within every Hot-Path Work Budget dimension.
7. Candidate reuse cannot survive request, route, evidence, resource, cost, or
   generation drift.
8. Fake and native Backends pass the same formation and member-local Receipt
   conformance fixtures.
9. Batch-invariant sampling and stable member output order hold for every
   certified B1 and multi-member route.

## Delivery Slices

| ID | Deliverable | Required verification |
|---|---|---|
| CB01 | Add operation-count instrumentation and a bounded full-recompute oracle. | Exact counts, maxima, deterministic candidates/exclusions. |
| CB02 | Add Core per-Model eligibility indexes and typed dirty causes. | Add/remove/phase/cancel/generated sequences and no full-state scan. |
| CB03 | Add two-level Formation Result reuse and current Core association for unaffected Models. | Model-local versus Core-only one-field drift and unchanged result reuse. |
| CB04 | Add private Model Planner compatibility partitions and canonical bucket fill. | No arbitrary subsets, stable order, complete exclusions. |
| CB05 | Integrate fresh Snapshot/Plan selection over cached and rebuilt associations. | Receipt, rejection, local stale, cancellation, and timing cases. |
| CB06 | Add exact native multi-member batch assembly routes. | B1/multi-member logits, KV, RNG, output, and failure parity. |
| CB07 | Run mixed prefill/decode and cross-Model qualification. | Planning overhead, TTFT/TPOT/throughput, fairness, memory artifacts. |

CB01-CB05 optimize the P0 scheduling Implementation without changing its
external Interface. CB06-CB07 are separately qualified native execution work;
they cannot be inferred from pure-Core success.

## Completion Criteria

The optimization is complete only when:

- operation counts prove dirty-Model-only work at binary maxima;
- incremental and full-recompute oracles agree for every generated sequence;
- no fresh Snapshot, Plan, authorization, or fairness law is weakened;
- every multi-member native route has exact numerical, resource, failure, and
  performance evidence; and
- product wording distinguishes request concurrency, same-Model Continuous
  Batching, and any future concurrent Metal capability.
