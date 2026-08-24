# P0 Parallel Module Delivery Plan

Status: delivery contract for the remaining P0 implementation ledger

Base plan: [P0 Runtime Implementation Plan](2026-08-16-p0-runtime-implementation.md)

Architecture authority: [ADR 0032](../adr/0032-separate-the-pure-core-from-protocol-and-io.md),
[ADR 0020](../adr/0020-use-a-narrow-in-process-backend-interface.md), and
[ADR 0046](../adr/0046-own-model-descriptor-integrity.md)

## Purpose

This plan divides the work after C07 into modules with one implementation owner
at a time, records the interfaces between those modules, and defines how
independent agents may develop in parallel without creating multiple commit
authorities or weakening the ordered P0 ledger.

The module assignment and approved C08 and C10a-C10e refinements are delivery
mechanisms. They do not change the P0 architecture, reorder later ledger rows,
combine independently green behaviors, authorize a new process, or make
private Core modules public.

## Non-Goals

Except for the item-free Architecture Contract Baseline authorized by ADR 0048,
this plan does not:

- add behavior-bearing placeholder crates, public traits, or implemented
  production behavior before its ledger row;
- expose a Support, Resource, Request, Certification, or Scheduler interface to
  the Event Loop;
- let module agents commit parts of one Core transition independently;
- move Benchmark-owned schemas, suites, runners, or oracles into TurnVector;
- fold the Compatibility Gateway into the daemon; or
- make full TurnVector-owned native graphs and operators a P0 readiness gate.

## Architecture Contract Baseline

Before the remaining module rows are delegated, the canonical
`schemas/p0-runtime-architecture-v1.jsonl` freezes the final Module paths,
primary and contributing ledger ownership, Interface operation vocabulary,
schema-family owners, two Backend adapters, visibility, compile roles, and
declared dependency graphs. This structural manifest does not define Rust
representations, fields, error variants, algorithms, capacities, or runtime
behavior.

Contract-only paths contain one Module-level documentation line and no Rust
item. Production and test-only paths are declared privately; the release
identity path remains unlinked until L01. The item-free Protocol crate reserves
its final workspace identity without activating its P04 dependency edges. Each
scheduled owner row replaces its exact shell, updates its status to
`implemented`, and supplies behavior and tests without gaining another module's
authority.

## Fixed Architecture

The sole public Runtime Core interface remains:

```text
Core::handle(CoreEvent) -> CoreTransition
```

`Core::handle` is the only mutation seam. Request Lifecycle, Support Ledger,
Resource Ledger, Certification, Admission, Scheduling, Plan lifecycle, and
Control carry remain private pure modules. A private Transition Coordinator
stages their changes, verifies the cross-module invariants, and commits once.

The bounded in-process Backend Interface remains the second deliberate seam.
It has a Fake Adapter and a C++/MLX Adapter, so it is a real seam. The private
Core modules have one implementation each; adding public traits for them would
create hypothetical seams and move ordering knowledge into callers.

Protocol, SQLite, filesystem access, process sampling, wall-clock reads, MLX
objects, and Backend calls remain outside Core. The Event Loop supplies typed
facts and explicit Monotonic Time, executes ordered Effects, and returns typed
Results as later Core Events.

## Remaining Ledger Inventory

The original combined C08 implementation measured 449 `rustfmt`-normalized,
counted non-documentation changed lines: 281 production and 168 focused-test
lines. That exceeded both C08's 400-line target and the global 420-line plan
ceiling. The base ledger therefore replaces C08 with two consecutive,
independently green delivery rows under the same sole `support_ledger` owner and
private interface:

- C08a, `feat(core): start ordinary support reservations`, contains the scoped
  record foundation and atomic optional ordinary reservation transition. Its
  formatted estimate is 145-165 counted lines and its fixed cap is 180.
- C08b, `feat(core): reserve lifecycle support`, depends on C08a and adds the
  typed pre-trigger description and safety reserves and their result
  transitions. Its exact Rust 1.97.1 `rustfmt`-normalized, focused-green source
  diff is 344 additions plus 13 deletions, or 357 counted lines. The normal
  B03-B05 and three-fixture generated cascade adds 18 counted lines, projecting
  375 against its fixed cap of 380 and leaving a five-line margin.

C08a deliberately preserves C07's generic `LifecycleReserve` behavior as a
compatibility placeholder. C08b alone replaces that placeholder with typed
lifecycle authority, rejects the generic construction bypass, and completes the
original C08 behavior. The split creates no second ledger, module, public trait,
or transition authority; neither row may borrow the other's cap, and C09 remains
ordered after C08b.

C08b remains one independently green row because its typed lifecycle reserves,
held-capacity accounting, closed result matrix, and closure of the generic
`LifecycleReserve` construction bypass are one transition of the sole
`support_ledger` authority. Splitting those responsibilities would temporarily
create duplicate lifecycle authority or leave the generic bypass open.

The descriptor-integrity and registration implementation has five ordered,
separately owned delivery responsibilities. The exact Rust 1.97.1
`rustfmt`-normalized, focused-green C10a source diff remains 224 human-counted
lines: `bounded.rs` contributes 21 additions plus 3 deletions, and `support.rs`
contributes 172 additions plus 28 deletions. Its fixed 18-line B03-B05 and
three-fixture cascade projects 242 against the unchanged cap of 260. C10b's
fixed SHA-256 extraction measures approximately 160 human lines plus the same
18-line cascade, projecting 178 against a cap of 220. C10c's canonical frame
and verifier are projected at 187-263 human lines plus 18 generated lines, or
205-281 against a cap of 300. C10d retains a human hard maximum of 362 for the
registry implementation adapted to sealed descriptor values; its fixed cascade
brings the row maximum to 380, equal to its cap and 40 below the global 420-line
ceiling. C10e's integrated Core transition remains projected at 198-248 total
lines against a cap of 280.

C10a remains one independently green row because its prepared
`FixedWindowCounter` start, opaque generation-bound `SupportChange`, and direct
ordinary-start/active-finish exact-Work regression form one prepared-change seam
in the sole `support_ledger`. Splitting those responsibilities would either
duplicate start/commit authority or lose independently-green compatibility
evidence that the legacy C07/C08 entry points preserve state and Hot-Path Work.

C10b and C10c form a deep-module authority split. C10b owns one private
SHA-256-only one-shot primitive and its known-answer, differential, and exact-
Work evidence. C10c is its sole consumer and owns the complete frame parser,
independent identity and evidence domains, untrusted-claim comparison, and
non-forgeable `VerifiedModelDescriptor`. This ordering is independently green
without exposing a generic crypto seam. C10d remains one independently green
row because its opaque `DescriptionPlan`, sealed descriptor retention,
descriptor-bound `RegistryChange`, descriptor-arena accounting, and exact
readback/post-load equality form one invariant in the sole private
`model_registry`. Splitting C10d would expose a partially registered interface;
folding it into C10c would leak registry state into descriptor integrity.

C10a, C10b, C10c, C10d, and C10e are consecutive and independently green.
C10a installs a crate-private, non-forgeable, generation-bound `SupportChange`
and prepared `FixedWindowCounter` start while legacy C07/C08 entry points
delegate with no new runtime behavior. C10b installs only the private fixed
SHA-256 primitive.
C10c completes the private deep `model_descriptor` verifier and its field-private
sealed value without registry, Core, Support, Effect, or runtime authority. C10d
installs a bounded `DescriptionPlan`, stores only sealed descriptor values,
removes bare `Register` and raw-field bypasses, and proves exact readback and
post-load equality without gaining Effect or runtime authority. C10e gives the
integration owner Core custody of Support and Registry changes, requires the
exact C08a active charge before a Describe Model Effect, retains pending-plan
custody, verifies the raw Result through C10c, and atomically finishes Support
plus commits C10d's `RegistryChange` or commits neither. Only C10e completes the
original descriptor-registration behavior. C11a, C11b, and C11c refine the
original C11 delivery boundary; C12 and every later row retain their existing
identities and order. C11a owns the complete bounded Token Request, including
its exclusive direct-Revision-or-Alias selector. C11b keeps Manifest request
facts and acceptance preparation in the sole `model_registry` and
`request_book`; its integration-owner contribution carries and tests the
context limit through the existing registration path but exposes no Request
Acceptance. C11c alone resolves the request's selector and integrates the
prepared change through `Core::handle` to expose successful Request Acceptance.

C15 and C16 have one canonical delivery refinement under Design Lineage
`TV-C15-C16-CANONICAL-DELIVERY-20260824`, Proposal Revision
`a3ffe9908a51bded502fae0592cdf7ef8d84efd3376581a8b8db6e323a4992e4`,
and unanimous Review Round
`TV-C15-C16-CANONICAL-DELIVERY-20260824-C15-REFINEMENT-R5-20260824T103037Z`.
The Launch Record SHA-256 is
`c2b836de8e219334892181835f357ee2bb9fe37e886e2dd7f1d54816d0611472`;
the passing round-record SHA-256 is
`3c8278d3d1a8b5b6dc49579b615f70518c73fde8d80a5a06c702b17c5e457da0`.

Only the C15a numeric delivery boundary was subsequently recalibrated under the
same Design Lineage from `origin/main@192b57a2802d2e0fcc5bd1558bc498d0fcb0b046`.
Proposal Revision
`591f0f1f80fd9f37b1c97a2662f3383edefe855d0e261ea7debc175ae91d608f`,
Review Round
`TV-C15-C16-CANONICAL-DELIVERY-20260824-C15A-CAP-R1-20260824T171650Z`,
Launch Record SHA-256
`309b40546d791f3b7282f4911b26fc91d8f274d54c117e2d1c460d11740e351f`,
Mathematical Gate Record SHA-256
`568511f6dffe9679348133e7959c2b20a1b58ef9bd9a1a80dff0955b974a44a4`,
and passing round-record SHA-256
`7574aff639bec952bc538eb3c804a67780fce0f784e4ab1dc7842b09013d2587`
bind that revision. Every non-numeric C15a/C15b/C16 decision remains unchanged.
The user explicitly approved the 821-line maximum, its required
`commit-size-exception`, and squash merge of the docs authority revision on
2026-08-25; the new boundary activates only after that revision merges.

The corrected complete C15 proof measures `858 + 8 + 18 = 884` counted
non-documentation lines, 184 above the cumulative 700-line row cap. The accepted
delivery therefore refines C15 into consecutive C15a and C15b rows while keeping
one private `resource_ledger`, storage authority, generation, Interface, and
commit path. C15a closes reserve and pre-materialization withdrawal; C15b
extends that same owner with terminal settlement, Backend partition, Pending
Reclaim, and convergence. This is row refinement, not Module or authority split.

C15a has an 821-line per-commit and PR-cumulative maximum. Its current
projection is `727 + 14 + 18 = 759`, preserving the prior 62-line contingency:
727 is measured behavior, 14 is the specified and proof-reconstructed
architecture transition, and the specified 18-line cascade remains to be
verified against final generated bytes. An actual commit from 501 through 821
requires `commit-size-exception` plus exact full-SHA, per-commit, cumulative,
approval, and atomic-reason PR disclosure. Every additional source,
architecture, generated, or follow-up line consumes the margin one-for-one; one
post-push runtime repair also spends another 18-line cascade and therefore
leaves at most 44 source lines when no other drift occurred. C15b has
an independent 500-line maximum, uses no exception, and measures
`(305 + 53) + 18 = 376`, leaving 124 lines. C16 independently retains its
500-line per-commit and cumulative maximum and never uses an exception. C15a
headroom cannot be borrowed by either later row. A C15a count above 821 or any
non-numeric design drift stops for a new user decision and Gate.

C15a changes the architecture ledger series from `C12-C45` to `C12-C14`,
`C15a-C15b`, `C16-C45`, changes the primary row count from 193 to 194, assigns
`resource_ledger` to `C15a-C15b, C29`, and marks the source `implemented` only
after the complete closed foundation is present. C15b changes no architecture
status or dependency edge. The intermediate status means a real implemented
foundation, not settlement completion or permission for Admission consumers.

C15a owns unique reservation and Backend-budget identities, fixed reusable
storage, typed three-dimensional capacity, read-only prepare, metered validate,
an instance-bound non-forgeable, non-`Copy`, non-`Clone` validated capability,
infallible commit, snapshot, reserve, and withdrawal. The capability exclusively
borrows its exact ledger, so two validated changes for one ledger cannot coexist;
commit consumes it once, while drop without commit preserves state. C15b adds
the four daemon terminal facts (partial materialization,
queued-after-invalidation, in-flight-after-Receipt, and ordinary-after-Receipt),
zero-materialization and ownership-consuming Backend partitions, and Pending
Reclaim convergence bound to key, opened generation, cursor/evidence floor, and
fresh observed cursor/evidence. Equal/lower cursors reject, skipped higher
cursors are allowed, evidence replay/rebind rejects, and unrelated ledger
generations do not invalidate the exact anchor. Record removal and active-budget
identity reuse occur only after daemon ownership and Backend/Pending Reclaim
ownership both close. C16 remains wholly `support_ledger`-owned and atomically
preflights its logical and physical request-entitlement bundle.

After construction, every branch meters all five Work dimensions before
mutation and reports zero `Allocations` and zero `CandidateWork`. For C15a/C15b,
the exact Rust 1.97.1 lookup bound is `q(0) = 0` and
`q(n) = ceil(log2(n)) + 1` for positive `n`. With constructor-fixed capacity
`R`, concrete record size `s_r`, Backend-budget identity size `s_b = 32`,
variant lookup count `k_v <= 4`, and `|D| = 3`:

```text
VisitedEntities_v <= k_v * q(R) + 2 * |D|
CopiedBytes_v <= max((R + 1) * (s_r + s_b),
                     2 * (s_r + s_b) + s_r)
                 + size_of(ResourceChange)
                 + size_of(ValidatedResourceChange)
Allocations_v = 0
CandidateWork_v = 0
InvariantChecks_v <= exact compiler- and branch-derived maximum
StorageBytes = size_of(ResourceCapacityLedger<R>) + R * (s_r + s_b)
```

All arithmetic is checked before allocation, `R = 0` is invalid, and full-ledger
conservation scans are forbidden. The corrected final proof freezes the
implementation witnesses `[10,1296,0,0,15]` at `R = 1`, `q(3) = 3` with
VisitedEntities `18`, `[50,197552,0,0,15]` at `R = 1024`, and
`[66,2097008,0,0,15]` with StorageBytes `2096296` at `R = 10917`; it rejects
`R = 10918`, with VisitedEntities `66`, before allocation because copied work
`2097200` exceeds `2097152`.
For `R > 0`, the implementation expression is
`usize::BITS - (R - 1).leading_zeros() + 1`, verified against found and
not-found targets by an exact Rust 1.97.1 comparator probe. These values are not
production-capacity claims. C15a and C15b each recompute their exact landing
witness. C16 remains `O(3 + v)` with no `O(v^2)` duplicate scan and preflights
every logical, physical, and Work dimension.

The authority update, C15a, C15b, and C16 use separate worktrees, commits, PRs,
manifests, generated cascades, reviews, and budgets. Each begins from fresh
post-predecessor `origin/main`; no source, generated output, review, or approval
carries across the boundary. C17, C19, C20, and every Resource Capacity consumer
remain blocked through C15b; C17 and every later Core row remain blocked until
C16 merges. Any count overflow, owner expansion, missing exact witness, test or
validation removal, `rustfmt::skip`, or hidden transition control flow stops for
a new user decision and Design Proposal Gate. For this explicitly authorized
sequence, the coordinator squash-merges each ready, green PR without another
prompt, verifies the remote merge and branch cleanup, and only then begins the
next row from fresh `origin/main`.

C15 path custody is exact. The authority PR changes only the two canonical plans.
C15a may change only `crates/turnvector-core/src/resource_ledger.rs`,
`schemas/p0-runtime-architecture-v1.jsonl`,
`tests/test_p0_runtime_architecture.py`, and the nine B03-B05/fixture paths below.
C15b may change only `crates/turnvector-core/src/resource_ledger.rs` and those
same nine regenerated paths:

```text
schemas/daemon-core-build-v1.json
schemas/daemon-core-build-v1.lock.json
tests/fixtures/runtime-overhead-catalog-v1/lifecycle-operations.json
tests/fixtures/runtime-overhead-catalog-v1/local-stale.json
tests/fixtures/runtime-overhead-catalog-v1/sequenced-events.json
schemas/runtime-overhead-catalog-v1.json
schemas/runtime-overhead-catalog-v1.lock.json
schemas/daemon-build-v1.json
schemas/daemon-build-v1.lock.json
```

C15a must prove every dimension exact/one-past/overflow; both duplicate identity
axes; full/reuse/churn; wrong authority and every before-image axis; stale
prepare; validated-capability exact-instance, single-use, exclusivity, and drop
behavior; generation overflow; `R = 0`; exact `R = 3`; derived maximum/first
invalid `R`; an attainable five-axis witness and one-under failure for every
nonzero Work dimension; exact rollback; and exact-capacity, equal-length, sorted
unique indices with zero post-construction Allocation/CandidateWork. It runs the
exact Rust 1.97.1 fmt, clippy `-D warnings`, focused/Core/workspace debug and
release, `RUSTFLAGS=-Dwarnings`, pinned comparator, architecture, independent
B03/B04/B05 reproduction/check, full Python discovery, auditor-at-821,
diff-check, candidate/Benchmark authentication, and three fresh review gates.

C15b must prove all four daemon facts and all terminal orders; zero/nonzero
Backend partitions and both daemon/Backend orders; exact Pending Reclaim
conservation; wrong key/budget/opened generation/floor cursor/floor evidence;
equal/lower cursor rejection, skipped higher cursor success, same-evidence
rejection, new-evidence success, unrelated-generation success, replay rejection,
and reuse only after complete closure; capacity/overflow/stale/Work rollback;
and final maximum/one-past/five-axis witnesses. It independently repeats the same
frozen-source Rust, generation, Python, auditor-at-500, repository, Benchmark,
and three-reviewer gates; no C15a artifact or approval carries forward.

The accepted implementation order contains 194 rows after C07.

| Area | Rows | Count | Delivery result |
|---|---:|---:|---|
| Core foundations | C08a-C08b, C09, C10a-C10e, C11a-C11c, C12-C14, C15a-C15b, C16-C18 | 19 | Support, descriptor integrity, registry, request, Certification, and resource foundations |
| Core lifecycle | C19-C31 | 13 | Admission, materialization, invalidation, carry, cancellation, output, and release |
| Scheduling and Plan lifecycle | C32-C45 | 14 | Exclusive, scheduling, Turn results, replay, and performance |
| Backend runtime | E01-E24 | 24 | Backend Interface, Fake Adapter, Device Executor, Event Loop, and qualification |
| Resource governance | G01-G17 | 17 | Resource Evidence, Governor policy, Residency, and reclaim |
| Native Adapter | N01-N26 | 26 | C shim, graph ABI/import, native lifecycle, Turn execution, and conformance |
| Protocol and daemon ingress | P01-P24 | 24 | Policy, authentication, schemas, negotiation, bounded I/O, and commands |
| Volume and durable authority | U01-U03, S01-S31 | 34 | Volume qualification, Control Store, Audit, recovery, readiness, and shutdown |
| Aggregate gate | K01-K05 | 5 | Integrated Core properties, sequences, faults, and work bounds |
| Release and qualification | L01-L02, Q00-Q15 | 18 | Closure freeze, subject adapters, qualification, and finding resolution |
| **Total** |  | **194** |  |

Rows remain ordered exactly as written in the base plan. A row may depend on
several modules, but it has one primary implementation owner and one integrated
commit result.

## Core Module Ownership

### Private Modules

| Module | Primary rows or private contribution | Owns | Must not own |
|---|---|---|---|
| `support_ledger` | C08a-C08b, C10a, C16-C18, C26 | Support Ledger Generation, prepared Support changes and fixed-window starts, pools, Funding Claims, credits, obligations, entitlements, lifecycle reserves, retained history, and Prepared Carry | Lifecycle witness selection, Resource Capacity, Admission, or Control publication outcome |
| `model_descriptor` | C10b-C10c | Exact V1 frame parsing, private SHA-256-only one-shot implementation, independent descriptor ID/hash derivation, untrusted-claim comparison, and field-private verified values | Registry lifecycle/counts, Core transitions, Backend semantics, public crypto, or general hashing |
| `model_registry` | C09, C10d | Immutable Model Revision, Alias freeze, lifecycle, Description Plan, sealed Model Descriptor retention/arena accounting, and incremental registry counts | Descriptor parsing/hashing, request state, Backend handles, Residency, Effect emission, or scheduling policy |
| `request_book` | C11a-C12, C21, C30-C31 | Bounded Token Request values, prepared acceptance, Preparing and later request states, description freshness, ownership identity, release lifecycle, and bounded terminal history | Support or Resource capacity, Certification applicability, Backend execution, or visible transition coordination |
| `certification` | C13-C14, C23-C24 | Exact Authorization Index access, Environment Fingerprint, finite Applicability Selection, invalidation, and quarantine decisions | Online widening, lifecycle evidence selection, Resource Evidence policy, or ledger mutation |
| `resource_ledger` | C15a-C15b, C29 | Request Backend Allocation Budgets, daemon output capacity, transient headroom, Pending Reclaim, checked generation, and atomic reserve or settlement | Support charges, Governor policy, Resource Evidence interpretation, Backend mutation, or request lifecycle authority |
| `admission` | C19 | Pure bound construction and complete accepted or rejected Admission decision | Allocation, Effect emission, evidence selection, or state mutation |
| `turn_plans` | C38; private contribution to C39-C42 | Frozen candidate and Batch membership, Plan provenance and lifecycle, Local Stale and Result progression, and cost-profile update staging | Support credits, output publication, cross-module commit, Backend execution, or scheduler policy |
| `scheduler` | C32-C37, C43-C45 | Exclusive feasibility, bounded candidate filtering, service accounting, deadline closure, deterministic selection, replay, and scheduler measurement | Request lifecycle, candidate execution, ledger mutation, Plan result progression, or native state |
| `closure_control` | C25 | Runtime Closure Gate state and zero-request-liability stability | Event Loop cancel gate, lifecycle evidence selection, Store publication, or Prepared Carry ownership |
| `transition_coordinator` | C10e; registration-path contribution to C11b; C11c, C20, C22, C27-C28, C39-C42 | Cross-module staging, generation revalidation, all-or-nothing commit, ordered Effects, and integrated dispositions | A duplicate ledger, descriptor verifier, request invariant, durable authority, native execution, or policy hidden from the owning module |

C10e coordinates C10a, C10c, and C10d changes without moving any module's local
invariants into Core. C10c alone consumes C10b's private SHA primitive. C17
remains implemented in `support_ledger` because
Plan-scoped obligations are Support Ledger facts. C28 and C30 consult request
state and ledger owners, but
their row owner must deliver one atomic integrated transition. C29's capacity
fact remains Resource Ledger-owned even when its Effect is coordinated by
`Core::handle`.

### Crate-Private Interface Shape

These are interface shapes, not public Rust traits and not frozen function
signatures:

```text
support_ledger.prepare(input, work) -> SupportChange
model_descriptor.verify(raw_claims, expected_hash, work) -> VerifiedModelDescriptor
model_registry.prepare(command, verified_descriptor, work) -> RegistryChange
resource_ledger.prepare(input, work) -> ResourceChange
request_book.prepare(event, facts, work) -> RequestChange
certification.resolve(requirements, evidence, work) -> CertificationDecision
admission.decide(facts, work) -> AdmissionDecision
scheduler.select(snapshot, work) -> SchedulingDecision
turn_plans.prepare(event, facts, work) -> PlanChange
closure_control.prepare(event, facts, work) -> ClosureChange
transition_coordinator.commit(staged_changes) -> CoreTransition
```

Every private interface follows the same laws:

1. Inputs are canonical immutable facts and include every expected identity and
   generation needed for validation.
2. Validation and Hot-Path Work accounting occur before mutation.
3. A prepared change is crate-private, non-forgeable by callers, and applicable
   only to the exact owner instance, generation, and before-image from which it
   was derived. A validated capability is non-`Copy`, non-`Clone`, exclusively
   binds its exact owner so two for that owner cannot coexist, consumes its
   commit once, and leaves state unchanged if dropped without commit.
4. No mutable ledger reference, unchecked generic delta, callback, I/O handle,
   wall clock, or Backend object crosses a private interface.
5. The Transition Coordinator checks aggregate capacity and cross-module
   conservation, then commits every prepared change once or commits none.
6. Rejection or Core Fault preserves the exact prior Core state and emits no
   Effect. Successful Effects retain their required order.
7. Production behavior and tests remain observable through `Core::handle`.

The owning module may expose narrower crate-private constructors or readers when
a later row proves they are needed. It must not introduce a common
`DomainSlice` trait merely to make unlike invariants look uniform.

## First Parallel Development Wave

Three module agents may prepare disjoint source and focused tests concurrently.
The integration owner retains sole custody of `core.rs`, the Transition
Coordinator, row ordering, cross-module tests, and generated identity output.

| Owner | Source ownership | Primary rows and private contributions |
|---|---|---|
| Agent A: Support | `support_ledger`, including C10a's crate-private `FixedWindowCounter` preparation helper | C08a-C08b, C10a, C16-C18, C26 |
| Agent B: Descriptor, Registry, and Request Capacity | `model_descriptor`, `model_registry`, `request_book`, `resource_ledger` | C09, C10b-C10d, C11a-C11b, C12, C15a-C15b, C21, C29-C31 |
| Agent C: Certification and Scheduling | `certification`, `admission`, `scheduler`, `turn_plans`, `closure_control` | C13-C14, C19, C23-C25, C32-C38, C43-C45; private Plan changes for C39-C42 |
| Integration owner | `transition_coordinator`, `core.rs`, cross-module fixtures, generated identity cascade | C10e, C11b registration-path contribution, C11c, C20, C22, C27-C28, C39-C42 |

This is parallel authoring, not parallel authority. The merge order is now
C08a, C08b, C09, C10a, C10b, C10c, C10d, C10e, C11a, C11b, C11c,
C12-C14, C15a, C15b, C16, and onward through C45. An agent may generally prepare
a later row locally, but that row cannot become ready or retain generated
artifacts until every predecessor has landed and the branch is synchronized with
the exact current `main`. The authority -> C15a -> C15b -> C16 chain is stricter:
no implementation work for its next row begins before the predecessor is
squash-merged and fresh `origin/main` is authenticated.

For C39-C42, Agent C remains the sole editor of `turn_plans`, while the
integration owner is the sole editor of the Transition Coordinator and shared
cross-module fixtures. Those contributions form one row-scoped commit and one
atomic `Core::handle` behavior; they are not independent commit authorities.
Likewise, C11b is one independently-green row: Agent B owns the Manifest fact
and request-state seams, while the integration owner alone carries that fact
through the existing registration transition and owns its cross-module tests.
That contribution does not add Request Acceptance, which remains wholly C11c.

## Later Agent Waves

The same three agent slots rotate after the preceding phase closes. Within each
wave, one agent owns each named source subtree and no other agent edits it.

### Late-Wave Module Interfaces

These interfaces are delivery contracts. Their exact Rust types land only in
their scheduled rows. Every runtime interface uses bounded input and output,
reports typed failures, and accounts for the operation work required by its
accepted bound.

| Module | Primary rows | Interface shape | Ordering, failure, and work laws | Must not own |
|---|---|---|---|---|
| `backend_contract` | E01 | The twelve-operation Backend Interface already fixed by the base plan and ADR 0020 | Every call and Result is typed and bounded; initialize is first, shutdown is last, and an indeterminate call fail-stops the process | Core policy, Event Sequence, or adapter implementation state |
| `fake_backend` | E02, E04-E11, E23-E24 | Implements `backend_contract` from scripted bounded state | Deterministic call order and one Result per accepted call; injected failure never fabricates completion; conformance covers every operation | Production qualification, native ownership, or scheduler authority |
| `device_executor` | E12, E20-E22 | Direct owner-thread invocation of each typed `backend_contract` operation; `shutdown() -> ShutdownResult` | There is no Backend or per-Turn command queue; only coarse Runtime Events may be queued before the Event Loop invokes one direct call on the sole owner thread; watchdog and operation bounds apply; indeterminate execution is terminal | Candidate selection, Core mutation, retry policy, or Event Sequence |
| `event_loop` | E15 | `accept(ValidatedIngress) -> SequencedCoreEvent`; `drive(CoreTransition) -> SequencedResults` | Sole Event Sequence driver; completes each ordered Effect and submits its Result before unrelated dequeue; preserves cancel and publication cuts | Domain decisions, ledger mutation, Backend implementation, or lifecycle witness selection |
| `runtime_qualification` | E03, E17 | `collect(EnvironmentInputs) -> CertificationEnvironment`; `qualify(QualificationInputs) -> LifecycleOverheadQualification` | Uses the exact build, platform, Configuration, Catalog, and returned Backend descriptors; missing, stale, overflowed, or drifted input fails closed; it is the sole daemon selector | Service measurement, Admission, Support or Resource mutation, online widening, or Backend self-authorization |
| `runtime_measurement` | E13-E14, E16, E18 | `measure_engine(EngineCall) -> EngineServiceResult`; `measure_residency(ResidencyCall) -> ResidencyServiceResult`; `measure_turn(TurnPath) -> TurnPathDisposition`; `measure_support(SupportEnvelope) -> SupportDisposition` | Brackets direct calls with daemon monotonic time, preserves disjoint Engine, Residency, Turn, support, and event partitions, and enforces the exact supplied bounds without deriving applicability or ledger authority | Catalog selection, Support mutation, Event sequencing, retry, or Backend policy |
| `runtime_carry` | E19 | `coordinate(CarryInput) -> CarryDisposition` | Consumes the immutable qualification witness and Core-produced carry facts, preserves support deferral and every dual-Budget revalidation, and restores ordinary support on each nonfatal pre-owner failure | A second selector, a second Support Ledger, Store/Audit pause authority, or configuration publication |
| `resource_evidence` | G01-G04 | `sample(ResourceSignalSet) -> ResourceEvidence` | Samples are sequenced, complete, contract-bound, and explicitly unavailable on failure; no missing signal becomes zero | Governor policy, Admission, Residency action, or ledger settlement |
| `resource_governor` | G05-G08, G10-G11, G13, G15-G16 | `decide(ResourceEvidence, ResourceView) -> GovernorDecision` | Pure bounded decision over current evidence and configuration; pressure and reclaim states fail safe; actions remain typed proposals | Resource Ledger mutation, request priority, MLX buffers, or direct Backend calls |
| `residency_coordinator` | G09, G12, G14, G17 | `coordinate(GovernorDecision, CoreView) -> OrderedResidencyWork` | Preserves leases, Reservation causality, critical-eviction ordering, and observed reclaim settlement through normal Effects and Results | Governor policy, direct memory freeing, or alternate resource accounting |
| `native_build` | N01, N03-N08, N24 | `verify(NativeArtifactSet) -> VerifiedNativeArtifacts` | Build-time exact identity, ABI, import, and numerical-hash validation; drift or malformed input fails before runtime construction | Runtime policy, live MLX state, Certification applicability, or serving fallback |
| `native_runtime` | N02, N09-N17, N23, N25 | Implements lifecycle and evidence operations of `backend_contract` | All MLX objects remain in owner-thread ModelRuntime capsules; ownership, cleanup, and shutdown are exact and bounded | Cross-model scheduling, Core state, protocol, or Governor policy |
| `native_turns` | N18-N22, N26 | Implements sampling and Turn operations of `backend_contract` | Deterministic request-local sampling, bounded Decode/Prefill/Exclusive work, synchronized completion, typed partial or failed Results | Shared RNG, concurrent Metal authorization, output publication, or policy widening |
| `protocol_authority` | P01-P08 | `authenticate(Peer, Plane) -> AuthenticatedPeer`; `negotiate(Hello) -> ProtocolSession` | Policy loads before acceptance; identities and versions are exact; malformed, unauthenticated, or unsupported peers fail before Core visibility | Core invariants, durable mutation, or native state |
| `data_plane` | P09-P16 | `receive(BoundedFrame) -> DataIngress`; `publish(ReservedOutput) -> DataDisposition` | Ingress, direct response, and outbound capacity are reserved before use; per-connection order and backpressure are bounded; disconnect is typed | Control mutation, Admission decisions, ledger internals, or unreserved output |
| `control_plane` | P17-P24 | `receive(AuthenticatedCommand) -> ControlIntent` | Commands preserve ordered cancellation and one mutation owner; saturation or closed cancel gate rejects before mutation | Store transaction internals, Core ledger mutation, or publication acknowledgement |
| `volume_qualification` | U01-U03 | `qualify(VolumeProbe) -> StorageQualificationRecord` | Offline exact syscall/profile validation publishes one immutable qualified record; missing or drifted capability is non-ready | Runtime writes, Control repair, or inferred compatibility |
| `control_store` | S01, S03-S06, S15-S20 | `apply(DurableCommand) -> StoreResult` | One serialized executor and atomic transaction protocol; required barrier failure latches custody and cannot return unchanged-state success | Certification compilation, Audit authorship, live inference recovery, native objects, or daemon policy |
| `certification_tooling` | S07-S08 | `prepare(CertificationInputs) -> PreparedCertificationSuccessor` | Offline exact-key compilation and successor validation are finite, deterministic, and persistence-free; wildcard, range inference, missing evidence, or invalid replacement fails before a successor exists | Store mutation, current-pointer activation, online applicability widening, or runtime evidence selection |
| `audit_journal` | S10-S14, S21, S25, S31 | `append(AuditEnvelope) -> AuditResult`; `reconcile(TailInput) -> TailDisposition` | Bounded reserve, chained record, predecessor fence, retention, and reconciliation order; indeterminate write never fabricates a record | Control State authority, sequence reuse, or request/KV recovery |
| `daemon_custody` | S02, S09, S22-S24, S26-S30 | `bootstrap(BootstrapInputs) -> Readiness`; `shutdown(ShutdownInput) -> ShutdownDisposition` | Sole process-level owner of policy-first bootstrap, instance lock, publication recovery, readiness, reclaim barrier, and graceful shutdown ordering | Store/Audit implementation, Core policy, or live inference restoration |
| `scheduling_gate` | K01 | `run(SchedulingGateInput) -> GateEvidence` | Test-only bounded scheduling and Admission property verification over real Core transitions | Production state, runtime authorization, or qualification evidence fabrication |
| `lifecycle_gate` | K02 | `run(LifecycleGateInput) -> GateEvidence` | Test-only bounded lifecycle and Residency sequence generation over real Core transitions | Production state, runtime authorization, or qualification evidence fabrication |
| `fault_gate` | K03 | `run(FaultGateInput) -> GateEvidence` | Test-only bounded fault injection with exact state, Effect, and work witnesses | Production state, runtime authorization, or qualification evidence fabrication |
| `core_gate` | K04-K05 | `aggregate(GateEvidenceSet) -> GateReport` | Verifies exact lane evidence, the aggregate work budget, and final Core closure; K05 closes only after K01-K04 are exact | Lane fixture implementation, production state, runtime authorization, or evidence fabrication |
| `release_identity` | L01-L02 | `finalize(QualifiedBuildInputs) -> ReleaseIdentity` | Build-time closure freeze and exact request Certification inputs; any runtime-source drift invalidates the result | Runtime behavior, Benchmark oracle, or mutable latest selection |
| `qualification_core_adapters` | Q00-Q04 | `run(CoreQualificationRequest) -> SubjectResult` | Thin launcher, handshake, replay, scheduler, and scheduler-performance targets stay outside the production runtime closure and return raw subject facts | Benchmark schema, suite, runner, oracle, gate, or production authority |
| `qualification_lifecycle_adapters` | Q05-Q08 | `run(LifecycleQualificationRequest) -> SubjectResult` | Thin lifecycle, native, Turn, and Governor targets stay outside the production runtime closure and return raw subject facts | Benchmark schema, suite, runner, oracle, gate, or production authority |
| `qualification_system_adapters` | Q09-Q13 | `run(SystemQualificationRequest) -> SubjectResult` | Thin cross-model, observability, persistence, failure, and Certification targets stay outside the production runtime closure and return raw subject facts | Benchmark schema, suite, runner, oracle, gate, or production authority |
| `qualification_integration` | Q14; coordination on Q15 | `aggregate(EvidenceSet) -> QualificationDisposition` | Aggregates exact lane evidence and routes each finding to its current module owner; it may not mark unresolved or stale evidence passing | The source remediation itself, Benchmark oracle, or alternate release identity |

### Backend Runtime: E01-E24

| Agent | Modules | Rows | Responsibility |
|---|---|---|---|
| A | `backend_contract`, `fake_backend` | E01-E02, E04-E11, E23-E24 | Backend Interface, Fake Adapter operations, shutdown, and conformance |
| B | `device_executor`, `event_loop` | E12, E15, E20-E22 | Device Executor, Event Loop, cooperative signals, fail-stop, and failure replay |
| C | `runtime_qualification`, `runtime_measurement`, `runtime_carry` | E03, E13-E14, E16-E19 | Environment evidence, service measurement, overhead qualification, envelopes, and carry interference |

E01 lands first and defines the code-level Backend Interface. No placeholder
Backend trait is added by this documentation PR.

### Resource Governance: G01-G17

| Agent | Modules | Rows | Responsibility |
|---|---|---|---|
| A | `resource_evidence` | G01-G04 | Backend, process, VM, and pressure evidence assembly |
| B | `resource_governor` | G05-G08, G10-G11, G13, G15-G16 | Governor modes, configuration, capacity policy, Residency admission, reclaim, and process barrier decisions |
| C | `residency_coordinator` | G09, G12, G14, G17 | Runtime Residency coordination, leases, critical eviction, and integrated replay |

The Governor proposes limits and actions. It never mutates the Resource Capacity
Ledger or frees a ModelRuntime's buffers directly.

### Native Adapter: N01-N26

| Agent | Modules | Rows | Responsibility |
|---|---|---|---|
| A | `native_build` | N01, N03-N08, N24 | C shim, export toolchain, graph ABI, graph artifacts, import verification, and qualification hashes |
| B | `native_runtime` | N02, N09-N17, N23, N25 | Owner-thread initialization, ModelRuntime capsules, lifecycle operations, profiles, allocator evidence, and shutdown |
| C | `native_turns` | N18-N22, N26 | Sampling, stop/output limits, Decode, Prefill, Exclusive, and full conformance |

The Native Adapter stays behind the E01 Backend Interface. Model graphs, KV,
operators, weights, and MLX objects never widen the Core interface.

### Protocol: P01-P24

| Agent | Modules | Rows | Responsibility |
|---|---|---|---|
| A | `protocol_authority` | P01-P08 | Installation policy, authentication, generated schemas, and negotiation |
| B | `data_plane` | P09-P16 | Ingress budgets, response/output capacity, data lifecycle commands, backpressure, and disconnect |
| C | `control_plane` | P17-P24 | Initialization and control commands, management cancellation, concurrency tests, and cancel-gate rejection |

The Protocol module validates bounded wire input into Domain Types. It does not
own Core invariants or expose native state.

### Durable Authority: U01-U03 And S01-S31

| Agent | Modules | Rows | Responsibility |
|---|---|---|---|
| A | `volume_qualification`, `control_store`, `certification_tooling` | U01-U03, S01, S03-S08, S15-S20 | Volume qualification, SQLite executor, Control schema, immutable rows, offline Certification compilation, successor preparation, and Control mutation transaction |
| B | `audit_journal` | S10-S14, S21, S25, S31 | Audit schema, reserves, chained records, writer, fences, retention, tail reconciliation, and fault custody |
| C | `daemon_custody` | S02, S09, S22-S24, S26-S30 | Storage barrier latch, instance lock, bootstrap, interrupted-publication recovery, readiness, reclaim barrier, restart, and shutdown |

Store and Audit remain separate executors and authorities. The daemon coordinates
their ordered protocol without making the Audit journal a recovery database.

### Aggregate Gate And Qualification

| Owner | Modules | Rows | Responsibility |
|---|---|---|---|
| Agent A | `scheduling_gate`, `qualification_core_adapters` | K01, Q00-Q04 | Scheduling/admission properties, launcher, handshake, Core replay, and scheduler qualification adapters |
| Agent B | `lifecycle_gate`, `qualification_lifecycle_adapters` | K02, Q05-Q08 | Lifecycle/residency sequence generation and lifecycle/native/Turn/Governor adapters |
| Agent C | `fault_gate`, `qualification_system_adapters` | K03, Q09-Q13 | Fault injection and cross-model/observability/persistence/failure/Certification adapters |
| Integration owner | `core_gate`, `release_identity`, `qualification_integration` | K04-K05, L01-L02, Q14 | Work-budget gate, aggregate gate, final closure, frozen inputs, evidence aggregation, and Q15 remediation routing |
| Affected module owner | Module named by the finding | Q15 | Implement each source remediation and focused regression without transferring module ownership |

Q rows add thin TurnVector subject and launcher targets outside the production
runtime source closure. TurnVectorBenchmark continues to own its schemas,
suites, runner, oracles, and gates. Any required Benchmark change is a separate,
explicitly authorized PR in that independent repository.

Q15 is a repeatable coordination row, not a blanket source-ownership grant. For
each finding, the affected module owner is the primary implementation owner.
The integration owner retains only shared-fixture, generated-cascade,
qualification-aggregation, and ready-PR custody. Both contributions produce one
independently green Q15 commit under the ordinary row, pull-request, and
generated-identity rules. A remediation that changes a Core transition also
uses the Transition Coordinator; a non-Core remediation must not route through
that Core-only authority.

## Worktree And Pull Request Protocol

1. Each agent receives one worktree and one active source ownership assignment.
2. Every worktree starts from the exact current `origin/main`. Existing dirty or
   independently diverged worktrees are never repurposed or reset.
3. Every implementation PR remains row-scoped and independently green. A
   module branch is not merged wholesale merely because several future rows
   were prepared there.
4. Only one row PR is merge-ready at a time. Other work may generally remain
   local or in draft PRs while its ledger predecessors are unresolved. The
   authority -> C15a -> C15b -> C16 chain is excluded: its next-row
   implementation may not start until the predecessor is squash-merged.
5. Before a row becomes ready, its branch incorporates current `main`. Before
   first push this may be a local rebase. After first push it uses an explicit
   clean base-synchronization merge; shared history is never rewritten or
   force-pushed.
6. The assigned owner edits only its module and focused tests. The integration
   owner alone edits the Transition Coordinator, shared cross-module fixtures,
   ledger dependency metadata, and generated identity output.
7. Cross-module behavior is handed to the integration owner as canonical input
   facts, expected prepared changes, and focused tests. It is not handed off as
   a second mutable implementation of another module's invariant.
8. Every content change invalidates review approvals. Review, staging, signed
   commit, push, and ready-PR confirmation follow the base plan unchanged.
9. C15a uses the accepted-object auditor at limit 821 and accumulates every
   non-merge payload commit against the same 821-line PR cap. C15b and C16 each
   use limit 500 and each has its own cumulative PR cap. All other rows retain
   the base plan's ordinary 420-line plan ceiling and 500-line policy limit.

## Generated Identity Single-Writer Protocol

B03, B04, and B05 form one ordered identity cascade:

```text
runtime source and tests
  -> B03 daemon Core build identity
  -> B04 Runtime Overhead Catalog
  -> B05 embedded Catalog and outer daemon identity
```

Only the integration owner writes generated descriptors, locks, Catalog output,
or embedded binding output during a landing window. Module agents may change
human-authored source and focused tests, but must not carry generated diffs
across a rebase or base-synchronization merge.

Before a runtime-source row becomes ready, the integration owner:

1. synchronizes the row branch to the exact current predecessor;
2. discards stale generated output from older bases;
3. regenerates B03, then B04, then B05 in that order;
4. includes the complete required cascade in the same independently green row
   commit and its LOC accounting;
5. runs every applicable canonical check and records the exact identities; and
6. verifies TurnVectorBenchmark status before and after any authorized paired
   verification.

L01 freezes the final qualified runtime closure. Any later runtime-source change,
including a Q15 fix, invalidates L01, L02, and dependent qualification evidence
and must repeat the complete cascade. Documentation-only and Benchmark-only
changes do not alter the runtime closure.

## Separate Delivery Tracks

The [Compatibility Gateway](2026-08-19-compatibility-gateway-design.md) remains
a separate process and delivery track. Refer to its rows as `Gateway G01-G08`
to distinguish them from Resource Governor rows G01-G17. Gateway implementation
must not gain Core, ledger, Admission, Candidate Formation, or Backend ownership.

The [native model ownership TODO](../todo-own-native-model-graphs-and-operators.md)
remains post-P0 work. P0 N01-N26 uses the accepted interim exported-graph
baseline while preserving the private Native Adapter seam and exact Execution
Route identities. Promoting full TurnVector-owned graphs and operators is not a
P0 readiness gate.

## Delivery Exit Criteria

This module split is ready to drive implementation only when:

- the base plan links this delivery plan;
- every row from C08a through Q15 belongs to one active wave and one primary
  owner;
- private interface laws preserve atomic Core transitions and Hot-Path Work
  accounting;
- no public interface or behavior-bearing placeholder precedes its ledger row;
- worktree, PR, review, signing, and generated-identity custody are explicit;
- C15a, C15b, and C16 obey their independent 821/500/500 per-commit and
  cumulative bounds and ordered fresh-main boundaries;
- Benchmark ownership and the separate delivery tracks remain unambiguous; and
- the documentation-only PR is merged before implementation agents begin their
  first row branches.
