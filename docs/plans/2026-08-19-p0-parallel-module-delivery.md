# P0 Parallel Module Delivery Plan

Status: delivery contract for the remaining P0 implementation ledger

Base plan: [P0 Runtime Implementation Plan](2026-08-16-p0-runtime-implementation.md)

Architecture authority: [ADR 0032](../adr/0032-separate-the-pure-core-from-protocol-and-io.md)
and [ADR 0020](../adr/0020-use-a-narrow-in-process-backend-interface.md)

## Purpose

This plan divides the work after C07 into modules with one implementation owner
at a time, records the interfaces between those modules, and defines how
independent agents may develop in parallel without creating multiple commit
authorities or weakening the ordered P0 ledger.

The module assignment and approved C08 refinement are delivery mechanisms. They
do not change the P0 architecture, reorder later ledger rows, combine
independently green behaviors, authorize a new process, or make private Core
modules public.

## Non-Goals

This plan does not:

- add placeholder crates, public traits, or production source before its ledger
  row;
- expose a Support, Resource, Request, Certification, or Scheduler interface to
  the Event Loop;
- let module agents commit parts of one Core transition independently;
- move Benchmark-owned schemas, suites, runners, or oracles into TurnVector;
- fold the Compatibility Gateway into the daemon; or
- make full TurnVector-owned native graphs and operators a P0 readiness gate.

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

The accepted implementation order contains 187 rows after C07.

| Area | Rows | Count | Delivery result |
|---|---:|---:|---|
| Core foundations | C08a-C08b, C09-C18 | 12 | Support, registry, request, Certification, and resource foundations |
| Core lifecycle | C19-C31 | 13 | Admission, materialization, invalidation, carry, cancellation, output, and release |
| Scheduling and Plan lifecycle | C32-C45 | 14 | Exclusive, scheduling, Turn results, replay, and performance |
| Backend runtime | E01-E24 | 24 | Backend Interface, Fake Adapter, Device Executor, Event Loop, and qualification |
| Resource governance | G01-G17 | 17 | Resource Evidence, Governor policy, Residency, and reclaim |
| Native Adapter | N01-N26 | 26 | C shim, graph ABI/import, native lifecycle, Turn execution, and conformance |
| Protocol and daemon ingress | P01-P24 | 24 | Policy, authentication, schemas, negotiation, bounded I/O, and commands |
| Volume and durable authority | U01-U03, S01-S31 | 34 | Volume qualification, Control Store, Audit, recovery, readiness, and shutdown |
| Aggregate gate | K01-K05 | 5 | Integrated Core properties, sequences, faults, and work bounds |
| Release and qualification | L01-L02, Q00-Q15 | 18 | Closure freeze, subject adapters, qualification, and finding resolution |
| **Total** |  | **187** |  |

Rows remain ordered exactly as written in the base plan. A row may depend on
several modules, but it has one primary implementation owner and one integrated
commit result.

## Core Module Ownership

### Private Modules

| Module | Primary rows or private contribution | Owns | Must not own |
|---|---|---|---|
| `support_ledger` | C08a-C08b, C16-C18, C26 | Support Ledger Generation, pools, Funding Claims, credits, obligations, entitlements, lifecycle reserves, retained history, and Prepared Carry | Lifecycle witness selection, Resource Capacity, Admission, or Control publication outcome |
| `model_registry` | C09-C10 | Immutable Model Revision, Alias freeze, lifecycle, Model Descriptor retention, and incremental registry counts | Request state, Backend handles, Residency, or scheduling policy |
| `request_book` | C11-C12, C21, C30-C31 | Preparing and later request states, description freshness, ownership identity, release lifecycle, and bounded terminal history | Support or Resource capacity, Certification applicability, or Backend execution |
| `certification` | C13-C14, C23-C24 | Exact Authorization Index access, Environment Fingerprint, finite Applicability Selection, invalidation, and quarantine decisions | Online widening, lifecycle evidence selection, Resource Evidence policy, or ledger mutation |
| `resource_ledger` | C15, C29 | Request Backend Allocation Budgets, daemon output capacity, transient headroom, Pending Reclaim, checked generation, and atomic reserve or settlement | Support charges, Governor policy, Backend mutation, or request lifecycle authority |
| `admission` | C19 | Pure bound construction and complete accepted or rejected Admission decision | Allocation, Effect emission, evidence selection, or state mutation |
| `turn_plans` | C38; private contribution to C39-C42 | Frozen candidate and Batch membership, Plan provenance and lifecycle, Local Stale and Result progression, and cost-profile update staging | Support credits, output publication, cross-module commit, Backend execution, or scheduler policy |
| `scheduler` | C32-C37, C43-C45 | Exclusive feasibility, bounded candidate filtering, service accounting, deadline closure, deterministic selection, replay, and scheduler measurement | Request lifecycle, candidate execution, ledger mutation, Plan result progression, or native state |
| `closure_control` | C25 | Runtime Closure Gate state and zero-request-liability stability | Event Loop cancel gate, lifecycle evidence selection, Store publication, or Prepared Carry ownership |
| `transition_coordinator` | C20, C22, C27-C28, C39-C42 | Cross-module staging, generation revalidation, all-or-nothing commit, ordered Effects, and integrated dispositions | A duplicate ledger, durable authority, native execution, or policy hidden from the owning module |

C17 remains implemented in `support_ledger` because Plan-scoped obligations are
Support Ledger facts. C28 and C30 consult request state and ledger owners, but
their row owner must deliver one atomic integrated transition. C29's capacity
fact remains Resource Ledger-owned even when its Effect is coordinated by
`Core::handle`.

### Crate-Private Interface Shape

These are interface shapes, not public Rust traits and not frozen function
signatures:

```text
support_ledger.prepare(input, work) -> SupportChange
model_registry.prepare(command, work) -> RegistryChange
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
   only to the exact state generation from which it was derived.
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
| Agent A: Support | `support_ledger` | C08a-C08b, C16-C18, C26 |
| Agent B: Registry and Request Capacity | `model_registry`, `request_book`, `resource_ledger` | C09-C12, C15, C21, C29-C31 |
| Agent C: Certification and Scheduling | `certification`, `admission`, `scheduler`, `turn_plans`, `closure_control` | C13-C14, C19, C23-C25, C32-C38, C43-C45; private Plan changes for C39-C42 |
| Integration owner | `transition_coordinator`, `core.rs`, cross-module fixtures, generated identity cascade | C20, C22, C27-C28, C39-C42 |

This is parallel authoring, not parallel authority. The merge order is now
C08a, C08b, C09, and onward through C45. An agent may prepare a later row
locally, but that row cannot become ready or retain generated artifacts until
every predecessor has landed and the branch is synchronized with the exact
current `main`.

For C39-C42, Agent C remains the sole editor of `turn_plans`, while the
integration owner is the sole editor of the Transition Coordinator and shared
cross-module fixtures. Those contributions form one row-scoped commit and one
atomic `Core::handle` behavior; they are not independent commit authorities.

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
4. Only one row PR is merge-ready at a time. Other work may remain local or in
   draft PRs while its ledger predecessors are unresolved.
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
- no public interface or production placeholder precedes its ledger row;
- worktree, PR, review, signing, and generated-identity custody are explicit;
- Benchmark ownership and the separate delivery tracks remain unambiguous; and
- the documentation-only PR is merged before implementation agents begin their
  first row branches.
