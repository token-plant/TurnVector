# P0 Runtime Implementation Plan

Status: active delivery plan

This plan turns the accepted TurnVector architecture into executable software.
`CONTEXT.md` and accepted ADRs remain the architecture authority. This document
owns implementation order, commit sizing, review gates, and verification
sequencing. When an accepted ADR and this order differ, the ADR is corrected in
an independently reviewed documentation commit before dependent code begins.

## Fixed Scope And Claims

The accepted P0 order is fixed:

1. P0-1: Pure Runtime Core and deterministic replay.
2. P0-2: Fake Execution Backend, Device Executor, and Runtime Event Loop.
3. P0-3: Resource Governor, Reservations, Residency, and Pending Reclaim.
4. P0-4: In-process C++/MLX Adapter, owner-thread enforcement, and conformance.
5. P0-5: Data and Control Unix sockets, Protobuf, and output backpressure.
6. P0-6: SQLite Control Store, bounded P0 Audit, and Daemon Instance Lock.
7. P0-7: TurnVectorBenchmark qualification for native correctness, cross-model
   interference, observability, recovery, supervision, and certification.

The P0 Serving Profile does not implement or advertise concurrent Metal Turns,
an out-of-process Backend, live request or KV recovery, Runtime Metadata
authority, Identity Anchor, Bundle, Restore, Clone, Compatibility Custody, or
the Integrity Serving Profile. Restart reconstructs authoritative Control State
and begins with empty inference state.

P-1 evidence constrains every product claim:

- Engine Service is synchronized Device Executor service measured around a
  started valid Turn's direct Backend call. It is not Metal command-buffer
  service time or GPU occupancy while P-1A remains YELLOW.
- Memory defaults apply only to the observed 256 GB host. P-1B remains PENDING
  and certifies neither a 24-hour soak nor a smaller-memory system.
- The accepted native topology is an in-process C++ Interface, but P-1C remains
  RED and supplies no relative-performance claim.

The first reproducible native baseline pins Rust `1.97.1` and MLX
`68cf2fddd8de5edd8ab3d926391772b2e2cedad8`; graph export alone pins mlx-lm
`254d153fdeb6f150edd4fc5a54f9828638481fa8`. The reviewed mlx-c revision
`fba4470b89073180056c9ea46c443051375f7399` remains evidence, not a production
dependency of the C++ Interface. Qualification fixtures bind Dense
`mlx-community/Qwen3-0.6B-4bit@73e3e38d981303bc594367cd910ea6eb48349da8`
and MoE
`mlx-community/Qwen1.5-MoE-A2.7B-4bit@11aaad5b454a361ae33f19fb47b72bc74b3c3b55`.
Dependency checkouts, weights, and caches remain under ignored `.work/`.

### Architecture Authority Prerequisites

Five documentation corrections precede implementation. ADR 0032 is updated to
the accepted P0 order above. ADRs 0018 and 0020 plus every directly affected
native/Fake glossary entry are updated to include stateless Model and Request
Description, post-Admission Request Materialization, Receipt-driven Cost Profile
updates, explicit request-state release, Exclusive execution, a Backend-only
resource sample, and a shared read-only Device Control Signal view. Those
changes also remove `indeterminate` as an ordinary Turn Receipt: process
fail-stop prevents a Receipt from being fabricated.

ADR 0022 and the request/sampling glossary entries then freeze one closed P0
Generation Parameters contract. Every request explicitly supplies Sampling Mode,
finite IEEE-754 binary32 Temperature and Top P, and unsigned 32-bit Top K;
omitted values have no default, and negative zero, NaN, or infinity is invalid.
Greedy requires positive zero Temperature, Top P `1.0`, and Top K `0`.
Categorical requires Temperature in `(0, 2]`, Top P in `(0, 1]`, and Top K either
`0` (disabled) or below the registered vocabulary size. Accepted binary32 bit
patterns are preserved in the immutable request.

The categorical algorithm is exact. Non-finite model logits fail the Turn before
sampling. It casts finite logits to binary32 and evaluates these pinned-MLX
binary32 tensors in order: `centered = logits - max(logits)`,
`weights = exp(centered)`, `normalizer = sum(weights)`,
`log_probabilities = centered - log(normalizer)`, and
`probabilities = exp(log_probabilities)`. It does not reuse `weights / normalizer`
or depend on a nonexistent C++ `log_softmax` convenience primitive; the pinned
MLX build fixes each primitive's rounding and reduction behavior.

Top P `1.0` retains every entry. Otherwise the Adapter stably orders entries
ascending by `(log_probability, token_id)`, gathers the separately computed
`probabilities`, takes their inclusive binary32 cumulative sum, masks entries
whose cumulative value is less than or equal to binary32 `1 - Top P`, and retains
the first entry that strictly crosses that threshold plus every later entry. If
rounding yields no crossing, it forcibly retains the single greatest log
probability, with lower token ID winning. Top K `0` is disabled; otherwise it
retains the greatest `min(Top K, surviving_count)` entries, with lower token ID
winning an equality. The support is therefore never empty.

The Adapter compacts only retained `(token_id, log_probability)` pairs in
ascending token-ID order; masked vocabulary entries never reach
`random::categorical`. It subtracts the greatest compacted log probability,
guaranteeing at least one finite zero, divides by Temperature, samples that
compact vector with the request draw key, and maps the returned compact index
back to its token ID. Thus even a zero-uniform/Gumbel `-inf` draw or the smallest
positive binary32 Temperature cannot select a masked token. Greedy uses stable
argmax with lower token ID winning and performs none of those filters.

Sampling Seed is an optional presence-tracked unsigned 64-bit value; every value,
including zero, is valid. When absent, the daemon obtains exactly eight bytes
from macOS `SecRandomCopyBytes`, interprets them as little-endian `u64`, fixes and
supplies that value to Core before Request Acceptance, returns it in the
Acceptance response, and carries the origin/value in the Audit Effect. CSPRNG
failure rejects before Request Acceptance and never falls back to time,
process-global RNG, or a constant. The Adapter initializes each request's state with
`mlx::core::random::key(seed)`, the `uint32[2] {high32, low32}` mapping of the
pinned MLX build. For every Categorical token it calls `split(state)`, persists
the first result as the next state, and supplies the second result as the sole
categorical draw key. Greedy performs no split. Every sampled token advances the
Categorical state exactly once even when stop-sequence handling hides it or
retains it in an ambiguous suffix; batching never substitutes a shared RNG.

Sampling Mode maps to `UNSPECIFIED=0` (invalid), `GREEDY=1`, and
`CATEGORICAL=2`. P0 supports no configurable logits bias,
repetition/presence/frequency penalty, Min P, XTC, or Backend-native opaque
parameter. Service Class maps to `UNSPECIFIED=0` (invalid), `INTERACTIVE=1`,
`STANDARD=2`, and `BACKGROUND=3`; callers cannot omit it or supply a raw
deadline. The Generation Parameters and RNG schema identities, complete allowed
domain, and exact transition algorithm enter the Certification Envelope; an
applicable record must cover those identities and bound the worst case across its
declared parameter domain. D03 locks cutoff-equality, tied-cutoff, explicit-zero,
no-crossing/subnormal Top P, smallest-Temperature, equal-logit, omitted-seed
replay, survivor-count-below-Top-K, zero-uniform/Gumbel-`-inf`, compact-index
mapping, and multi-token state vectors used unchanged by Native and qualification
tests.

A new profile-specific ADR then defines P0 Audit sequence authority without
silently importing the Integrity Profile's Identity Anchor. The SQLite Control
Store owns a small durable P0 Audit Sequence State outside the semantic Control
State Snapshot: History Epoch, next range predecessor, and reserved high-water.
It reserves a complete range in a `FULL` SQLite transaction and explicitly
synchronizes the database parent directory before assignment. The Daemon Instance
Lock proves process exclusivity; exact Audit chain validation against the
reserved high-water proves the tail. Bootstrap first appends, synchronizes,
verifies, and clears every exact Store-published Pending Audit Envelope. Only
when no pending envelope remains may graceful shutdown write a strongly
synchronized Clean Shutdown Boundary for its unused suffix; Bootstrap verifies
that boundary, or an unclean restart reserves a new range and writes one Crash
Tail Boundary for the complete unrecoverable prior suffix through its high-water.
That suffix includes never-assigned values and values assigned only to lost,
unsynchronized records; exact Store-published envelopes are completed first and
therefore excluded. No lost record is guessed and no assigned value is reused.
P0 never emits or claims Anchor `CLAIMED`/`CLEAN_RELEASED`. Unreconcilable
Store/Audit mismatch is non-ready and requires a new runtime. The ADR updates the
P0 branches of ADRs 0021, 0026, 0027, 0036, and 0040 plus their glossary terms;
Integrity semantics remain unchanged.

The same ADR defines P0 Control Initialization without Locator, Initialization
Manifest, Identity Anchor, Runtime Metadata, or hidden defaults. Under the held
Daemon Instance Lock, an authorized complete proposed Configuration Snapshot
causes the daemon to generate the random Runtime ID and History Epoch. Initial
Model, Alias, Certification, and model-scoped Configuration sets are explicitly
empty and may change only by later Control Mutation. Initialization publishes
version-one Control State, the
first fully synchronized sequence range, and one exact pending Event Sequence
one Epoch Open envelope in a new SQLite Store. Transaction commit and explicit
database-parent synchronization are distinct crash boundaries; together they
form the live Native Authority publication barrier. Before generating any new
identity or sequence after a crash, the next process completes SQLite rollback
recovery and classifies exact Store bytes. An absent transaction remains
uninitialized. An exact committed candidate reuses its Runtime ID, History Epoch,
range, generation, and pending envelope, repeats the parent synchronization, and
finishes forward. Any third state fails closed. Epoch Open fixes the Audit
Registry Identity and initial Generation Hash before readiness, and no path
creates a second identity from a possibly committed transaction.

For every later P0 Control Mutation, a bounded executor validates one complete
successor while the predecessor remains active. Before publishing a pending
envelope, the Event Loop obtains a typed Predecessor Fence from the single Audit
Writer: every earlier assigned record has been appended, strongly synchronized,
verified, and the returned Audit Head exactly matches the proposed predecessor.
The Event Loop performs no direct I/O. At a synchronized Turn boundary it
revalidates the complete token and fence, assigns the mutation's Event Sequence
as the immediate successor, and builds the exact pending envelope. Any intervening
Core Event or safety event invalidates the fence; that event is handled first and
the mutation must obtain a new fence rather than publish against an old head.

One guarded `FULL` SQLite transaction then writes the complete candidate, current
pointer, sequence state, and pending envelope; explicit synchronization of the
database parent directory completes P0's sole post-initialization Native Authority
barrier. Only then may the exact generation become active in memory. Audit
append/sync/verification and a `FULL` pending-clear transaction plus its parent-
directory synchronization precede success acknowledgement. The Predecessor Fence
and every publication phase have fixed bounds and fault classification. Any
required SQLite, Audit, native-file, or parent-directory barrier failure enters P0
Storage Barrier Failure. The daemon
returns stable `outcome_indeterminate` when an operation exists, stops Device
work and readiness, forbids every later Runtime write and same-session retry,
retains its Daemon Instance Lock, and exposes only authenticated read-only status
plus best-effort diagnostics until an OS signal terminates the process. It writes
no failure marker or Clean Shutdown Boundary. Only a later process may acquire
the OS-released lock and reconcile the stored bytes forward. P0 creates no
Metadata staging or Anchor state for this protocol.

The same three-state rule governs every P0 range and mutation crash boundary.
After rollback recovery, absence preserves the predecessor/uninitialized state;
an exact committed row is never regenerated or reused and instead receives the
missing parent barrier plus exact pending-envelope completion; any other state is
non-ready. Commit-before-parent-sync, parent-sync completion, Audit append, and
pending-clear recovery each have separate witnesses. A live barrier error latches
Storage Barrier Failure; a process crash leaves classification to the successor.

The bounded SQLite executor applies the same commit-plus-parent-directory barrier
to every successful P0 write transaction, including registry, Configuration,
Certification, sequence, initialization, mutation, and pending-clear writes. No
caller may observe a published generation, assigned sequence, cleared pending
envelope, or successful acknowledgement between those two barriers.

Model, Configuration, and Certification Store helpers can encode and validate
immutable rows only inside a caller-owned uncommitted transaction. They expose no
commit, current-pointer, in-memory activation, or acknowledgement operation. P0
Initialization is the only genesis path; afterward the unified mutation
transaction above is the only path that can advance Control authority, and its
parent barrier is the only point after which Core may activate that successor.
External model, Configuration, and Certification commands therefore create a
complete proposal Effect, not an active Core transition. Only the typed committed-
publication result from that unified path can drive C19's atomic activation.

A single daemon-session Storage Barrier Failure latch and write guard exist
before any Runtime Store/Audit writer or Device startup. The first typed required-
barrier error atomically closes that guard, sets the Device shutdown signal, and
fixes the read-only Barrier Failure Observation; every later Runtime write is
rejected before a syscall. Repeated observations cannot replace the first fact,
retry a write, emit Audit, release the instance lock, or enter graceful shutdown.
All P0-6 writers consume this one guard rather than implementing local recovery.

The Daemon Instance Lock has no live-process release operation. Its descriptor
remains owned through Backend destruction, Clean Shutdown Boundary, and the last
daemon instruction, and only OS process termination releases it. A successor
therefore cannot acquire the lock while the prior process is alive; acquisition
is combined with a fresh post-exit Resource Evidence baseline before the Process
Reclaim Barrier can clear.

A final focused ADR binds daemon-owned Runtime Overhead limits to exact build,
Configuration, Hot-Path maxima, environment, and Certification evidence. It
updates the deadline/admission decisions in ADRs 0015 and 0024: online samples
may invalidate applicability but cannot widen a bound, and Runtime Overhead is
never Backend Cost Profile state or Model Ledger service.

No dependent implementation commit starts until these corrections have passed
the same three-reviewer gate as code.

### Paired Benchmark Compatibility

The read-only TurnVectorBenchmark revision inspected for this plan is
`7e8045c0811bec899d7833d27a52f725c0dbc441`. Its expectation is bound to old
TurnVector revision `7cbfe2caef3f2f9f95a03e17eb8741ed1acf98a2` and is not the
production protocol authority. The incompatibilities are broader than one
Worker lane:

- seven implementation-source paths name superseded ADRs;
- its benchmark-owned Data Plane descriptor exposes only submit/cancel input,
  lacks the accepted Token Request selector and parameters, and cannot express
  explicit status query/subscription commands;
- `persistence-and-recovery` requires native snapshot round-trip and restore,
  while P0 restarts with empty inference state;
- `protocol-and-worker-supervision` requires a Backend process handshake,
  transport, crash, and replacement; and
- `certification-envelopes` includes `worker_build` rather than the accepted
  in-process Adapter and Backend Interface identities.

TurnVector must not copy that descriptor or reintroduce a Backend process,
per-Turn IPC, native snapshot recovery, or a private serialized Backend protocol
to satisfy stale expectations. Compatible existing lanes may provide partial
evidence. Full P0 qualification requires a separately authorized, read-only-to-
this-task TurnVectorBenchmark update that rebinds the source contract, public
protocol descriptors, P0 persistence semantics, same-process fail-stop model,
and certification dimensions. Until that fixed revision exists, P0-7 and the
ready-PR gate remain explicitly pending rather than being reported as passed.

## Module Seams

The stable external Runtime Core Interface is deliberately deep:

```text
Core::handle(CoreEvent) -> CoreTransition
```

Callers learn one sequenced Event input and one atomic Transition result.
Request Lifecycle, Scheduling and Arbitration, Resource Policy, Operation
Ledger, Admission, and Transition coordination remain private pure Modules.
Tests observe behavior through `Core::handle`; they do not trait-wrap or commit
those Modules independently.

The second deliberate Seam is the bounded in-process Backend Interface:

```text
initialize(control_view)                       -> BackendInitialization
describe_model(registration)                  -> ModelDescriptor
describe_request(request)                    -> RequestDescription
materialize_request(description, reservation)-> MaterializationResult
release_request(request, reason, control_view) -> RequestReleaseResult
form_candidates(eligible, hard_constraints)  -> FormationResult
execute_turn(plan, control_view)              -> TurnResult
observe_turn_receipt(receipt)                 -> CostProfileUpdate
execute_exclusive(operation, control_view)    -> ExclusiveResult
transition_residency(operation, control_view) -> ResidencyResult
sample_backend_resources()                    -> BackendResourceSample
shutdown(control_view)                        -> ShutdownResult
```

`describe_model` generates the hash-bound Model Descriptor before registration.
`describe_request` is stateless and allocates no request handle, KV, or Sampling
State. `materialize_request` is permitted only after Admission atomically creates
the request Resource Reservation and Timing Commitment. Any materialized or
partially materialized Backend state is released exactly once by
`release_request` on the owner thread. A successful Release Result consumes
ownership and reports the actually allocated bytes by reservation class; the
never-allocated reservation remainder releases immediately, while allocated
bytes remain charged as Pending Reclaim through fresh allocator and footprint
convergence. An operation unable to return that synchronized deterministic
result triggers process fail-stop: it is never retried, treated as success, or
cleaned by cross-thread destruction. Terminal Core state retains the applicable
charges through that boundary.
Candidate Formation reports the Backend-owned Cost Profile version and ordinary
estimates. Only an accepted Receipt may update that profile between Turns; the
Adapter never widens certified bounds. An accepted profile update advances
Backend Generation and invalidates every candidate formed against the prior
generation. A Turn Plan grants target Engine Service and work ceilings, while
the Backend chooses the concrete Prefill token range. `TurnResult` carries
bounded opaque progress, staged token output, and per-member outcomes; tensors,
raw logits, and KV layout never cross the Seam. Qualification-only logits/KV
hashes use a separate test build seam and never enter Core or the serving ABI.

Exclusive execution begins only from an explicit Control Plane request after a
synchronized shared boundary. Core pauses shared dispatch, rejects new Admission
and Residency Demand, and requires one conservative resource bound, certified
Exclusive Safety Point, and renewable Exclusive Lease. Lease disconnect/expiry,
Critical, or explicit stop sets the shared control signal; no Backend or Governor
may enter Exclusive Mode automatically. Resource Mode itself remains solely a
Governor result. Control may update the Resource Threshold Profile, never set the
mode directly.

The Device Executor creates one shared `DeviceControlSignal`; external threads
may only set its bounded atomic flags, and each native operation receives a
read-only view that it polls at declared safe points while the direct call is
still active. There is no second Backend call, command channel, or per-Turn
queue for cancellation. The Device Executor records daemon Monotonic Time
immediately before and after `execute_turn` and classifies the interval from its
typed result. A started Turn that returns a synchronized Receipt charges the
complete direct-call interval as Engine Service. A pre-execution Plan Rejection
charges zero Engine Service; its complete synchronous validation-call interval
belongs to Runtime Overhead and produces no Receipt or Model Ledger charge. The
Executor separately brackets Residency Transition to produce a Residency Receipt
with elapsed Residency Service. The Adapter may return diagnostic subspans but
owns neither fairness nor residency clocks.

Backend Initialization returns exact Adapter, MLX, Backend Interface, and
Capability identities, never an authorization decision. A daemon-owned bounded
platform probe supplies device, GPU, unified-memory, and macOS facts and joins
them into one fresh Environment Fingerprint. Admission alone evaluates exact
Certification Applicability and may cache the answer in a fixed-capacity
recency cache keyed by every determining Control, record, capability, build,
interface, and environment identity. Every hit rechecks the current evidence's
identity and freshness; stale evidence fails closed, and any changed member
invalidates the entry. Certification Records have no invented wall-clock expiry.
The cache is neither persisted nor audited as authority.

Runtime Overhead remains daemon-owned. Versioned conservative bounds cover Plan
formation/selection and, after a synchronized Receipt, result validation, Core
commit, and output publication. On that started-Turn branch, the complete
synchronous `execute_turn` call belongs only to Engine Service. On a
pre-execution Plan Rejection branch, the complete synchronous rejection call and
bounded fresh-Snapshot/replanning work belong instead to Runtime Overhead; they
produce no Engine Service or Model Ledger charge. Instrumentation proves each
branch is ordered, disjoint, and covers its declared decision timeline without a
gap or overlap. Every span bound is evidence-bound by the exact daemon build,
configuration, Hot-Path maxima, environment, and Certification Record; online
samples cannot widen it. Core adds the applicable overhead bounds to the
certified Engine Service upper bound to form Deadline Cost Bound; measured drift
invalidates the applicable overhead version and blocks unsafe new timing
commitments. Runtime Overhead is never supplied by the Backend or charged to a
Model Ledger.

The Runtime Overhead witness is continuing. E15 closes both result branches over
the first working Fake Event Loop. Every later commit that adds covered work,
especially protocol conversion, Core result handling, and Output Publication,
extends the same span partition, evidence key, and bound regression in that
commit; a partial timeline cannot be treated as the production bound.

`BackendResourceSample` contains only owner-thread MLX allocator/cache evidence.
A daemon-owned sampler collects process footprint, available memory, swap,
compressor, and macOS pressure events without hot-path shell commands. A bounded
assembler retains each source's provenance, sequence, and freshness and is the
only component that emits complete `ResourceEvidence`; the Backend never selects
Resource Mode.

The deterministic Fake Execution Backend and the C++/MLX Adapter implement the
same Interface and conformance fixtures. Every logical per-Model `ModelRuntime`
capsule maps to the one Device Executor OS thread, and the P0 Backend Capability
declares `max_overlapping_turns = 1`.

Core requests a concrete Turn Output Reservation before authorizing any
output-producing Turn. Lacking capacity makes that request non-runnable. After
an accepted Receipt atomically commits Request State, ordered Effects publish
only staged visible tokens into the already-held capacity. Event Sequence
orders cancellation against Receipt commit and Output Publication: an earlier
cancellation enters Cancel Pending and discards staged output at the synchronized
boundary, while later cancellation cannot retract published output.

Protocol, SQLite, filesystem, process sampling, and wall-clock reads remain
outside the pure Runtime Core. The Event Loop validates them into Domain Types
and supplies Monotonic Time explicitly.

## Commit And Review Protocol

Every pull-request commit follows all of these rules:

1. It covers one independently green behavior and changes at most 500 non-
   documentation lines. The planned ceiling is 420 lines, including production
   source, tests, fixtures, generated source, migrations, and lockfiles. Human-
   authored Markdown is exempt. A measured slice above 420 is split before
   review; approaching 500 is not an acceptable plan.
2. Behavior is developed red, then green. A commit is reviewed only after its
   focused test and all applicable prior tests pass.
3. Every intended path is unstaged during review. Unrelated changes and any
   staged path are forbidden. From T01 onward,
   `python3 -B scripts/check_worktree_policy.py --base HEAD --limit 420`
   enumerates tracked and untracked paths, reports counted LOC, writes the
   ignored review manifest, and prints its SHA-256. Before T01, documentation-
   only commits use an explicit path/blob table from `git hash-object`, plus
   `git diff --binary` for tracked files or `git diff --no-index` for an
   untracked file.
4. Three independent reviewers inspect the same complete unstaged manifest and
   diff. Each rereads repository-required internal references, checks the whole
   specification and architecture, runs or requests relevant tests, checks
   failure behavior and measured LOC, and returns `APPROVE` or findings.
5. Any content change or finding invalidates all approvals. The complete revised
   diff receives three fresh reviews; a majority is insufficient.
6. Only after three approvals are the exact reviewed paths staged. The staged
   path/blob manifest must equal the approved manifest, repository-native tests
   run again, and the commit is signed. `git verify-commit --raw HEAD` must
   verify that exact commit before the contribution-policy checker validates it.
7. Review transcripts, manifests, test output, native builds, models, traces,
   and qualification artifacts remain under ignored `.work/`.
8. After the first push, fixes are new commits. Shared history is never rewritten
   or force-pushed.

Before every commit, both repositories are checked with `git status --short`.
TurnVectorBenchmark remains read-only. Any cross-repository verification records
both HEADs and statuses before and after; completion requires no Benchmark diff.

Hot-Path Work Budget is a continuing contract, not a late retrofit. After C06,
every change touching Core transitions, candidate formation, Admission, Event
Loop routing, protocol-to-Core conversion, Receipt processing, or output
publication extends its production witness and operation-count regression in the
same commit. Every phase exit reruns all dimensions: visited entities,
copied/encoded bytes, allocations, candidate work, and incremental invariant
checks.

## Commit Ledger

IDs describe required order and ownership, not final Git SHAs. Rows are never
combined merely because their sum appears below the limit. Each row must remain
independently green and at or below its target when measured.

### Plan Foundation

| ID | Commit subject | Behavior slice | Required verification | Target LOC |
|---|---|---|---|---:|
| D00 | `docs: define P0 runtime implementation plan` | This plan, ledger, gates, and blockers | Links, policy consistency, three approvals | docs only |
| D01 | `docs(adr): align the accepted P0 implementation order` | Correct ADR 0032's final implementation-order sentence | ADR/CONTEXT consistency and three approvals | docs only |
| D02 | `docs(adr): complete the in-process backend seam` | Update ADRs 0018/0020 and affected glossary for initialization identity, Model/Request Description, Materialization/release, Cost Profile observation, Exclusive operation, Backend sample, signal view, and fail-stop | Request, model, ownership, and conformance consistency | docs only |
| D03 | `docs(adr): freeze the p0 token generation contract` | Update ADR 0022 and glossary with compact nonempty-support Generation Parameters, RNG advancement, Certification Envelope identity, exact binary32 tensor flow, and Service Class mapping | Enum/presence/range/tie/cutoff/subnormal/no-crossing/survivor-below-K/zero-uniform/state-vector matrix; no opaque Backend parameters | docs only |
| D04 | `docs(adr): define p0 audit sequence authority` | Define P0 Initialization, sole later mutation authority, durable Predecessor Fence, commit/parent three-state recovery, pending-before-tail, complete lost suffix, range/high-water, and pre-writer Storage Barrier Failure latch; update P0 branches of ADRs 0021/0026/0027/0036/0040 plus glossary | No Anchor/Locator/Metadata or second pointer path; genesis, mutation, tails, registry, terminal reserve, fail-closed session complete | docs only |
| D05 | `docs(adr): bind runtime overhead evidence` | Add daemon-owned evidence-bound, result-branch-disjoint overhead and update ADRs 0015/0024 plus glossary | Receipt and Plan Rejection each have a complete no-gap/no-overlap Deadline Cost Bound without online widening | docs only |
| T01 | `build: audit unstaged commit scope` | Tracked/untracked path hashing and documentation-aware LOC checker | Unit fixtures for add/delete/rename/binary/untracked and self-count | <= 300 |
| B01 | `build: initialize the Rust workspace` | Rust 1.97.1 toolchain, workspace, core crate, format/lint/test entrypoints | Format, clippy, workspace tests | <= 180 |

### P0-1: Pure Runtime Core And Replay

| ID | Commit subject | Behavior slice | Required verification | Target LOC |
|---|---|---|---|---:|
| C01 | `feat(core): add checked domain identities` | Distinct IDs, units, sequences, durations, and Monotonic Time | Overflow, zero, and cross-type rejection | <= 360 |
| C02 | `feat(core): add generation and bounded collection types` | Generation Vector plus checked fixed-capacity collections | Capacity and generation mismatch cases | <= 340 |
| C03 | `feat(core): define scheduling snapshots and candidates` | Snapshot, Work Candidate, and Candidate Exclusion contracts | Construction and completeness bounds | <= 380 |
| C04 | `feat(core): define turn plans and receipts` | Frozen Plan membership and completed/cancelled/partial/failed member outcomes | Stable order and member bounds | <= 380 |
| C05 | `feat(core): apply atomic core transitions` | `Core::handle`, contiguous Event Sequence, ordered Effects, rejection/fault shell | Failed invariant preserves state and emits no Effect | <= 400 |
| C06 | `feat(core): establish hot-path work budgets` | Incremental witness types, binary maxima, counted transition shell, and hard rejection | Exact base counts, overflow, no truncation or full-state scan | <= 380 |
| C07 | `feat(core): manage immutable model revisions` | Manifest identities, Alias freeze, Available/Retiring/Unavailable lifecycle | No alias repoint, registry limits, and incremental counts | <= 400 |
| C08 | `feat(core): describe model registrations` | Registration proposal, stateless Model Descriptor Effect, hash binding, and rejection | No Revision exists before descriptor validation | <= 380 |
| C09 | `feat(core): accept requests into preparing` | Ownership, frozen Revision, explicit Service Class, closed Generation Parameters, immutable effective `u64` Sampling Seed plus origin, status version, and preparation timeout | Explicit zero/caller/daemon origin; Acceptance is not Admission; retries never inherit state | <= 400 |
| C10 | `feat(core): drive warming and request description` | Stateless description Effect, Residency Demand, revalidation, and cancellation | No pre-Admission Backend handle or Reservation | <= 400 |
| C11 | `feat(core): authorize exact certification keys` | Immutable record identities and read-only exact-key Authorization Index lookup | Missing/drifted/quarantined cases fail closed | <= 400 |
| C12 | `feat(core): derive certification applicability` | Explicit fresh Environment Fingerprint plus Admission-owned fixed recency cache over every determining identity | Every hit rechecks freshness; miss/eviction/identity invalidation | <= 400 |
| C13 | `feat(core): apply the fixed admission check` | Revision, exact applicability, timing, Resource Mode, and capacity sufficient conditions | Every missing condition rejects before state allocation | <= 400 |
| C14 | `feat(core): commit timing and request reservations` | Atomic Timing Commitment and request Resource Reservation after Admission | Conservation, rollback, and no partial commitment | <= 400 |
| C15 | `feat(core): materialize admitted requests` | Post-Admission materialization Effect and stable Backend request ownership identity | No materialization before Reservation | <= 360 |
| C16 | `feat(core): apply request materialization results` | Success to Queued; terminal failure preserves any release-required partial state | No Warming return, retry, or leaked partial ownership | <= 380 |
| C17 | `feat(core): revalidate certified profiles` | Candidate/description invalidation and existing Reservation feasibility | SLO Risk and new-Admission block without silent re-promise | <= 380 |
| C18 | `feat(core): quarantine certified bound violations` | Preserve Receipt, remove exact key, optional parent escalation, explicit recertification | Estimate miss does not quarantine; automatic widening forbidden | <= 400 |
| C19 | `feat(core): activate complete control successors` | Only a committed publication result atomically installs the complete Model/Alias/Certification/Configuration Snapshot at a Turn boundary; Weight baseline alignment and new-Admission-only Timing Commitments | Proposal alone cannot mutate active authority; no partial visibility, historical debt, or rewritten promise | <= 400 |
| C20 | `feat(core): order cooperative cancellation` | Queued removal, in-flight Cancel Pending, staged-output discard, status transition | Event Sequence before/after Receipt and Publication matrix | <= 400 |
| C21 | `feat(core): reserve per-turn output capacity` | Concrete pre-execution Turn Output Reservation Effect and non-runnable backpressure state | No output Plan without capacity; cancellation releases reserve | <= 380 |
| C22 | `feat(core): release backend request state` | Exactly-once release Effect/Result; return unallocated remainder and move actual allocation to Pending Reclaim | Partial allocation/use, terminal/cancel/eviction, no retry or early reuse | <= 400 |
| C23 | `feat(core): bound terminal request history` | Count/time Tombstones, connection Request High-Water Mark, and authorized Gone | Evicted, foreign, and never-issued IDs remain distinct | <= 400 |
| C24 | `feat(core): manage exclusive mode leases` | Requested/active/exit states, renewal/expiry/disconnect, shared dispatch pause | No automatic entry; new Admission/Residency Demand reject | <= 400 |
| C25 | `feat(core): authorize exclusive operations` | One operation with conservative peak bound and certified Exclusive Safety Point | Uncertified shared work never silently probes | <= 380 |
| C26 | `feat(core): filter unsafe scheduling work` | Resource Mode and all generations before urgency/fairness | Unsafe or stale work never reaches selection | <= 360 |
| C27 | `feat(core): account runnable weighted service` | Runnable-only Model Ledger and Device Executor Receipt charging | 1:3 example and idle re-entry without stored credit | <= 400 |
| C28 | `feat(core): compose deadline cost bounds` | Certified Engine Service plus exact record-bound, disjoint Runtime Overhead version | Applicability/drift invalidation; no double count or Ledger charge | <= 380 |
| C29 | `feat(core): select deadline-aware turns` | Latest Safe Start, Urgent Set, fair fallback, and stable ties | Independent scheduler oracle scenarios | <= 400 |
| C30 | `feat(core): enforce candidate formation laws` | Complete bounded formation, same class, frozen members, stable Exclusions | Missing Exclusion and substituted member rejection | <= 400 |
| C31 | `feat(core): apply typed turn results` | Plan Rejection and synchronized Receipt acceptance only | Duplicate/late/unknown/stale results never reapply state | <= 400 |
| C32 | `feat(core): commit cost profile updates` | Accept only the Receipt-caused update, advance Backend Generation, invalidate all old candidates for the dirty Model | Stale/duplicate/unaccepted updates and exact incremental counts | <= 400 |
| C33 | `feat(core): publish committed staged output` | Ordered publication Effect after Receipt commit; visible Output Sequence advances once | Earlier cancel discards; later cancel cannot retract; failed enqueue disconnects | <= 400 |
| C34 | `test(core): replay bounded dual-model prefill` | One Prefill Chunk per fresh Plan with Decode interleaving | Byte-identical fixed-seed transitions and operation counts | <= 380 |
| C35 | `feat(replay): add the bounded core replay driver` | Strict replay input/output over `Core::handle` | Golden, malformed, and repeatability cases | <= 400 |
| C36 | `perf(core): measure release scheduler decisions` | Core-only Release measurements outside serialization and IPC | 100 warmups, >=1000 decisions, one sample per decision | <= 360 |

P0-1 exits only when Core has no Tokio, Protobuf, SQLite, MLX, I/O, async,
callback, or system-clock dependency and the three Core benchmark lanes can be
driven through a thin adapter without embedding their oracle.

### P0-2: Fake Backend And Device Loop

| ID | Commit subject | Behavior slice | Required verification | Target LOC |
|---|---|---|---|---:|
| E01 | `feat(runtime): define the bounded backend interface` | Owned inputs/results for every accepted coarse operation, signal view, and affinity token | Bound and illegal-call-order rejection | <= 400 |
| E02 | `feat(fake): initialize backend identity` | Deterministic Adapter/MLX/interface identities and bounded Capability set | Identity drift and capability-limit fixtures | <= 340 |
| E03 | `feat(runtime): collect certification environment` | Daemon-owned device/GPU/unified-memory/macOS probe joined with Backend initialization identity | Freshness, unavailable facts, refresh, and exact fingerprint change | <= 400 |
| E04 | `feat(fake): describe model registrations` | Deterministic hash-bound Model Descriptor generation | Manifest/capability mismatch cases | <= 340 |
| E05 | `feat(fake): describe and materialize requests` | Stateless descriptions of exact request parameters followed by post-Reservation opaque handles/Sampling State | Parameter matrix and allocation counters prove the two-phase boundary | <= 380 |
| E06 | `feat(fake): release request state` | Explicit release after success or partial allocation with allocated/unallocated byte result | Exactly once; split accounting; indeterminate cleanup invokes fail-stop | <= 380 |
| E07 | `feat(fake): form costed candidates` | Versioned Cost Profile, complete candidates, typed Exclusions, ordinary estimates | Profile identity and fixed certified bounds | <= 400 |
| E08 | `feat(fake): execute and observe turns` | Scripted progress/output/failure plus accepted-Receipt-only calibration | Frozen membership and one-chunk Prefill fixtures | <= 400 |
| E09 | `feat(fake): model residency and resource samples` | Scripted load/unload plus Backend-only allocator/cache evidence | Sticky failure and provenance fixtures | <= 360 |
| E10 | `feat(fake): execute exclusive operations` | Resource-bounded operation and certified periodic safety points | Lease/Critical stop and rollback fixtures | <= 380 |
| E11 | `feat(runtime): run one owner-thread executor` | Same thread creates, calls, releases, and destroys Backend state | Owner success and cross-thread rejection for every operation | <= 400 |
| E12 | `feat(runtime): measure synchronized engine service` | Daemon Monotonic Time brackets the direct `execute_turn` call and classifies its typed result | Plan Rejection charges zero; Receipt charges the exact complete call interval | <= 340 |
| E13 | `feat(runtime): measure synchronized residency service` | Daemon time brackets transition and creates an independent Residency Receipt | Elapsed bound and watchdog cases | <= 360 |
| E14 | `feat(runtime): drive effects through the event loop` | Sequenced `Core -> Effect -> EffectResult`; fresh scheduling after every Receipt or Plan Rejection | No inline continuation and stable Effect order | <= 400 |
| E15 | `feat(runtime): bound runtime overhead` | Receipt branch pre/post spans plus Plan Rejection call/replanning spans over the working Event Loop against exact evidence-bound limits | Both branches have no gap/overlap; drift invalidates; no widening/Ledger charge | <= 400 |
| E16 | `feat(runtime): share cooperative control signals` | External atomic setters and Backend safe-point read view | Cancel/Critical/lease/shutdown before and after safe point | <= 360 |
| E17 | `feat(runtime): fail stop an indeterminate backend operation` | Watchdog escalation, no fabricated Result/Receipt, whole-process termination hook | Missed safe point cannot restart only the owner thread | <= 400 |
| E18 | `test(runtime): replay executor failure boundaries` | Duplicate/stale result, shutdown, and deterministic Fake crash traces | Stable final hashes and no operation replay | <= 380 |
| E19 | `test(runtime): enforce backend conformance fixtures` | Shared Fake contract runner for initialization, exact parameters, ownership, bounded operations, and release | Call order, parameter matrix, synchronization, partial ownership, outcomes | <= 400 |

### P0-3: Resource Governor And Residency

| ID | Commit subject | Behavior slice | Required verification | Target LOC |
|---|---|---|---|---:|
| G01 | `feat(resources): sequence backend resource samples` | Allocator/cache sample provenance and freshness | Missing, duplicate, and stale source cases | <= 340 |
| G02 | `feat(resources): sample macos process and vm state` | `phys_footprint`, available memory, swap, and compressor via native APIs | Typed unavailable/overflow and no-shell assertion | <= 400 |
| G03 | `feat(resources): observe macos memory pressure` | Bounded dispatch pressure source and monotonic sequence | Normal/warning/critical transitions and teardown | <= 360 |
| G04 | `feat(resources): assemble complete resource evidence` | Join Backend, process, VM, and pressure sources without erasing provenance | Independent freshness and out-of-order rejection | <= 400 |
| G05 | `feat(governor): classify resource modes` | Normal, Guarded, StopAdmission, Critical, hysteretic recovery | Immediate restriction and dwell-bound recovery | <= 400 |
| G06 | `feat(governor): activate resource configuration` | Atomic Threshold Profile, eviction rank, wait, and residency-limit propagation from the complete successor | Stricter profile advances Safety Generation; looser recovery obeys hysteresis | <= 400 |
| G07 | `feat(governor): reserve request capacity` | KV, output, transient, and admitted capacity ledger | Conservation, overflow, rollback | <= 400 |
| G08 | `feat(governor): reserve residency before load` | Separate Model Descriptor-based Residency Reservation committed before Effect | No load without reservation; cancel-before-start rollback | <= 380 |
| G09 | `feat(runtime): coordinate residency demands` | Shared pending loads, waiters, FIFO ordering, timeout, sticky unavailable state | Coalescing and no Service Class reorder | <= 400 |
| G10 | `feat(governor): gate shared residency transitions` | Conservative blocking bound must fit every current timing budget or wait for explicit Exclusive Mode | No automatic Exclusive entry and no blocking shared transition | <= 400 |
| G11 | `feat(governor): bound residency activity` | Configured transition frequency and aggregate Residency Service occupancy | Window rollover, saturation, and checked arithmetic | <= 380 |
| G12 | `feat(runtime): protect resident model leases` | Lease acquisition/release and serialized unload boundary | Active lease prevents unload | <= 360 |
| G13 | `feat(governor): select ordinary reclaim actions` | Cache then deterministic idle-model victims independent of Model Weight | Stable tie and protected-victim cases | <= 380 |
| G14 | `feat(governor): perform critical eviction` | Eviction Rank, reclaimable-byte/stable-ID ties, terminally fail affected work before unload | Cache/idle exhaustion and no forced in-flight interruption | <= 400 |
| G15 | `feat(governor): retain only allocated reclaim charges` | Immediately return never-allocated reservation remainder; actual request/residency allocation stays charged to convergence | Partial materialization/use/unload splits, stall, no premature reuse | <= 400 |
| G16 | `feat(governor): decide the process reclaim barrier` | Require typed old-process lifetime proof plus fresh post-reclaim baseline | Fake proof contract; elapsed time/socket loss never establishes capacity | <= 360 |
| G17 | `test(governor): replay pressure and residency lifecycle` | Load/unload/reload, shared/Exclusive gating, Critical eviction, request release, reclaim, stale evidence | Governor rules plus hot-path counts | <= 400 |

G16 closes only the Core/Governor decision contract and its typed-proof tests.
It does not claim that P0-3 can establish old-process termination. S28 supplies
the real Daemon Instance Lock proof and fresh native sample, then integrates that
proof with the same decision contract before service readiness.

### P0-4: In-Process C++/MLX Adapter

The tracked export recipe pins `mlx-lm`
`254d153fdeb6f150edd4fc5a54f9828638481fa8`, the tested MLX revision, Python
dependencies, source Model Revision, Graph ABI, and every certified exact-shape
signature. It exports complete logits and updated KV outputs, runs Python-direct
round trips, and emits a canonical hash manifest. A clean checkout plus external
model revision can regenerate and byte-compare each Dense/MoE artifact under
`.work/`; Python and mlx-lm are never serving-runtime dependencies. The C++
Adapter imports only artifacts whose complete export/tool/model/signature
manifest verifies.

| ID | Commit subject | Behavior slice | Required verification | Target LOC |
|---|---|---|---|---:|
| N01 | `build(native): add the versioned C shim` | Pinned MLX checkout manifest, CMake/Ninja build, opaque handles, build/interface identity, error translation, destroy | C/C++ compile, SHA verification, ABI assertions, `scripts/verify-native.sh` | <= 400 |
| N02 | `feat(native): initialize on the executor owner thread` | MLX/stream creation plus exact Adapter/MLX/interface identities and capabilities | Owner, drift, and fail-closed teardown traces | <= 400 |
| N03 | `build(native): pin the graph export toolchain` | Ignored task Python, exact mlx-lm/MLX/dependency lock, model fetch/hash wrapper | Missing/drifted tool/model and offline replay manifest | <= 380 |
| N04 | `build(native): define the exported graph abi` | Canonical exact-shape signatures, logits/new-KV outputs, artifact manifest and import contract | Double-export byte identity and Python round-trip harness | <= 400 |
| N05 | `feat(native): implement the graph importer` | Bounded C++ Direct importer for verified Graph ABI signatures and complete manifest | No production import outside Residency Effect; deterministic tiny-fixture parity and drift rejection | <= 400 |
| N06 | `build(native): export qwen3 dense graphs` | Complete Qwen3-0.6B graph recipe for certified Decode/Prefill buckets | Exact revision, two exports, Python/C++ logits/KV parity, no tracked artifact | <= 400 |
| N07 | `build(native): export qwen15 moe graphs` | Complete Qwen1.5-MoE graph recipe including top-k routing for certified buckets | Exact revision, two exports, Python/C++ logits/KV parity, no tracked artifact | <= 400 |
| N08 | `feat(native): verify registered model artifacts` | Canonical Artifact Root and typed File Hash checks before and after load | Ad hoc path, mutation, and Revision Unavailable cases | <= 400 |
| N09 | `feat(native): own logical model runtime capsules` | Per-Model imported graph/KV/cache capsules on the single owner thread | Isolation and lifecycle tests | <= 380 |
| N10 | `feat(native): describe model registrations` | Generate versioned conservative Model Descriptor from verified graph manifest/capability | Descriptor hash and nonresident operation | <= 400 |
| N11 | `feat(native): expose cooperative signal views` | Read-only atomic signal view and declared safe-point helper before any cancellable operation | Cross-thread setter ordering and stale-view rejection | <= 360 |
| N12 | `feat(native): transition model residency` | Serialized import/load/unload/clear using the signal view and typed results | Real resident graph before request state; cancellation and cleanup | <= 400 |
| N13 | `feat(native): describe token requests` | Stateless validation of exact Service Class/Generation Parameters against model vocabulary and certified bounded description | Complete mode/range/unsupported matrix; no handle/KV/Sampling allocation | <= 400 |
| N14 | `feat(native): materialize admitted requests` | Resident-model per-request handle, KV, and exact immutable Sampling State after Reservation | Parameter identity preserved; partial failure reports release-required ownership | <= 400 |
| N15 | `feat(native): release request state` | Owner-thread destruction with actual allocation split and staged-stop cleanup | All terminal paths; uncertain cleanup fail-stops without retry | <= 400 |
| N16 | `feat(native): form costed native candidates` | Canonical Batch/Shape compatibility, Cost Profile version/estimate, typed Exclusions | Fake/native fixture parity | <= 400 |
| N17 | `feat(native): update cost profiles from receipts` | Accepted authoritative observations produce one versioned update between Turns | Core accepts once; no stale update or bound widening | <= 380 |
| N18 | `feat(native): sample per request deterministically` | Exact binary32 tensor flow, compact nonempty Top P/Top K support, max-shifted Temperature scaling, greedy, and `key(seed) -> split(state)` categorical transitions | Shared subnormal/no-crossing/equal-logit/survivor-below-K/smallest-Temperature/zero-uniform compact-index and multi-token state vectors; hidden-stop and Dense/MoE B1/B4 invariance | <= 400 |
| N19 | `feat(native): apply stop and output limits` | Longest ambiguous stop suffix retention, hidden matched tokens, exact Max Output terminal | Prefix overlap, cross-Turn match, cancellation, and limit cases | <= 400 |
| N20 | `feat(native): execute synchronized decode turns` | Bounded Decode on resident imported graph with staged visible token output | Dense/MoE output parity, signals, and cleanup | <= 400 |
| N21 | `feat(native): execute one prefill chunk` | One imported-graph range within Plan target/ceilings, synchronized continuation | Dense/MoE parity, signals, and no hidden loop | <= 400 |
| N22 | `feat(native): execute exclusive operations` | Conservative resource-bound operation with certified periodic safety points | Lease/Critical cancellation and rollback | <= 400 |
| N23 | `feat(native): report backend allocator evidence` | MLX active/cache samples with quality and source sequence | Telemetry availability and error mapping | <= 340 |
| N24 | `test(native): expose qualification-only numerical hashes` | Test-build output/logits/KV hashes outside serving Interface | Prove no qualification DTO in Core or production ABI | <= 360 |
| N25 | `test(native): pass full backend conformance` | Fake/native initialization, ownership/release, graph, formation/calibration, Exclusive, synchronization | External fixtures are unavailable, never synthetic pass | <= 400 |

Weights, exported graphs, build caches, raw tensors, traces, and numerical
qualification artifacts remain outside Git with path/hash manifests only.

### P0-5: Local Protocol And Output

The repository defines its first production Data and Control contracts here.
Because no TurnVector production descriptor has been published, each begins at
major 1, minor 0. The benchmark-owned `turnvector.benchmark.data_plane.v1`
descriptor is neither copied nor advertised. Each production family has its own
source `.proto`, append-only capability registry, canonical descriptor lock,
support manifest, and compatibility matrix.

Data Plane v1.0 represents Sampling Mode and Service Class as nonzero closed
enums and uses presence-tracked scalar fields for Temperature, Top P, and Top K.
All four Generation Parameters and Service Class must be present; zero enum,
omission, negative-zero/non-finite float, out-of-range value, or unsupported
parameter is a Domain Rejection before Request Acceptance. The descriptor and
Domain Types use the exact mapping and processing order frozen by D03; no daemon,
Adapter, or model configuration supplies a fallback.

Sampling Seed is a presence-tracked `optional fixed64`, not an ordinary proto3
scalar. Absence selects daemon generation; explicit zero and every nonzero `u64`
remain caller-owned values. The accepted response exposes the effective Seed and
its origin before later status/output so an omitted-seed request can be replayed
explicitly without confusing zero with absence.

| ID | Commit subject | Behavior slice | Required verification | Target LOC |
|---|---|---|---|---:|
| P01 | `feat(policy): load the bounded installation policy` | Parse/identify fixed Data, Control, live-maintenance, and offline-maintenance UID/GID allowlists | Canonical identity plus malformed/oversize and independent-list tests | <= 400 |
| P02 | `feat(daemon): authenticate data-plane peers` | Permission-restricted Unix socket plus macOS `LOCAL_PEERCRED` check | Filesystem-only and unlisted-root rejection | <= 380 |
| P03 | `feat(daemon): authenticate control-plane peers` | Stricter socket allowlist and connection-scoped Maintenance Capability | Disconnect/session expiry and no protocol-only grant | <= 400 |
| P04 | `build(protocol): generate canonical protocol artifacts` | Pinned deterministic descriptor/registry/support-manifest tooling | Double generation and registry zero/reuse tests | <= 400 |
| P05 | `build(protocol): lock data-plane v1.0` | Token IDs, selector, required Service Class, closed presence-tracked Generation Parameters, optional `fixed64` Sampling Seed, bounded stop sequences, Max Output, submit/query/subscribe/cancel/status/output | Descriptor hash; enum numbers; omitted/zero/nonzero Seed; subnormal positive versus negative-zero/nonfinite floats; greedy/categorical golden frames and limits | <= 400 |
| P06 | `build(protocol): lock control-plane v1.0` | P0 initialize with complete global Configuration/no IDs, plus model/config/certification/Exclusive/management/status | Independent descriptor, empty scoped sets, limits, golden frames | <= 400 |
| P07 | `feat(protocol): negotiate the data plane` | Exact descriptor, capability intersection, typed limits, frozen write durations | Old/new, malformed, revoked, and mismatch matrix | <= 400 |
| P08 | `feat(protocol): negotiate the control plane` | Independent Hello, support manifest, model registry limit, Maintenance gate | Old/new, unknown required, and privilege cases | <= 400 |
| P09 | `feat(daemon): enforce ingress budgets` | Global/per-connection command count/bytes, Preparing, Warming, active request, and output-backlog capacity | Overloaded precedes Request ID and fairness debt | <= 400 |
| P10 | `feat(daemon): reserve safety event ingress` | Independent cancellation capacity and Core Event Reserve while retaining each socket's receive order | Saturation cannot starve or move cancellation ahead of an earlier command | <= 380 |
| P11 | `feat(daemon): bound command and request history` | Strict Command IDs, `u64::MAX` ingress close, Request High-Water Mark, Tombstone/Gone mapping | Exhausted, evicted, foreign, never-issued cases | <= 400 |
| P12 | `feat(daemon): reserve direct response capacity` | Ordinary and fixed rejection reservations before Event Loop admission | Exactly one response or bounded close | <= 360 |
| P13 | `feat(daemon): serve data lifecycle commands` | Validate/map exact Service Class and Generation Parameters; preserve present `u64` Seed or generate/expose an absent one; then serve lifecycle commands in receive order | Missing versus explicit zero, injected CSPRNG failure, replayed effective Seed, audit origin, command/cancel ordering, ownership, hot-path counts | <= 400 |
| P14 | `feat(daemon): reserve outbound turn capacity` | Concrete Turn Output Reservation results backed by isolated output capacity | Reserve precedes Turn, converts once to occupancy, or releases on cancel/rejection | <= 380 |
| P15 | `feat(daemon): publish bounded request output` | Output and terminal reserves, ordered single writer, at-most-once frames | Device Executor never waits for a client | <= 400 |
| P16 | `feat(daemon): enforce backpressure and disconnect` | Frozen deadlines, cooperative cancellation, History Gap, no replay | Slow/partial reader and terminal truth cases | <= 400 |
| P17 | `feat(daemon): accept p0 initialization intent` | Complete global proposed Configuration only while Uninitialized; daemon IDs and empty model/alias/cert/model-config sets | Caller ID/nonempty scoped set, hidden default, repeat, partial, serving-state rejection | <= 400 |
| P18 | `feat(daemon): serve model registry commands` | Convert registration/Descriptor, Alias, retire, and unavailable commands into one complete Control successor proposal; status stays read-only | Authorization, registry bounds, typed rejection, and no direct active-state mutation | <= 400 |
| P19 | `feat(daemon): serve configuration commands` | Convert Configuration and Resource Threshold changes into one complete Control successor proposal | Token/version conflicts; caller cannot set Resource Mode or bypass unified publication | <= 360 |
| P20 | `feat(daemon): serve certification commands` | Convert record replacement and evidence-bound recertification into one complete Control successor proposal | Quarantined key cannot clear through a flag or direct activation | <= 360 |
| P21 | `feat(daemon): serve exclusive lease commands` | Explicit enter/renew/stop/exit and one resource-bounded operation | Shared pause, expiry/disconnect, certified safety point | <= 400 |
| P22 | `feat(daemon): serve management cancellation` | Locked privileged request inspection and cancellation DTOs | Ownership separation and ordered Core Event | <= 340 |
| P23 | `test(daemon): exercise saturated concurrent clients` | Multiple clients over one Turn with every ordinary budget full | Cancellation order, Critical, shutdown, output, cross-model, hot-path counts | <= 400 |

### P0-6: Durable P0 Control And Audit

The Audit payload schema is a private durable-format contract, not either public
socket protocol. It has its own append-only registry, canonical descriptor lock,
and deterministic verification command; a public protocol minor cannot alter an
epoch's fixed Audit Registry Identity.

| ID | Commit subject | Behavior slice | Required verification | Target LOC |
|---|---|---|---|---:|
| S01 | `build(store): add the bounded sqlite executor` | One connection/executor, FULL durability profile, checked bindings, explicit database-parent sync, and typed barrier errors | Open/configuration/commit/parent failure tests over temporary non-Runtime stores | <= 360 |
| S02 | `feat(daemon): latch storage barrier failures` | Session-wide first-failure observation, write guard, Device shutdown signal, read-only mode, and no retry/marker/Clean path before any Runtime writer is wired | First/duplicate failure, every later write denied before syscall, status-only custody | <= 400 |
| S03 | `feat(store): create the versioned control schema` | Runtime/history identity, generations, current-state pointer, bounded tables through the shared write guard | Fresh/open/unknown schema and latched-guard cases | <= 400 |
| S04 | `feat(store): encode immutable model registry rows` | Transaction-scoped Manifests, Aliases, lifecycle, canonical roots, and typed hashes with no commit/pointer API | Candidate round-trip and registry limit; caller rollback leaves live Store unchanged | <= 400 |
| S05 | `feat(store): encode configuration snapshot rows` | Transaction-scoped complete validated Configuration successor with no commit/pointer API | Semantic/generation hashes and caller rollback; current generation cannot change | <= 380 |
| S06 | `feat(store): encode certification record rows` | Transaction-scoped immutable record/evidence identities and finite Coverage Manifest with no activation API | Candidate round-trip, invalid reference rejection, current generation unchanged | <= 400 |
| S07 | `feat(certification): compile the authorization index` | Offline verified Coverage Manifest to read-only exact-key index | Missing reference, drift, wildcard, and dominance-proof cases | <= 400 |
| S08 | `feat(certification): prepare certification successors` | Validate candidate record/index replacement, explicit recertification evidence, and bounded Profile Revalidation plan without persistence or activation | Quarantine remains until unified mutation activation; changed bounds identify exact descriptions/candidates | <= 400 |
| S09 | `feat(daemon): hold the instance lock` | Exclusive descriptor ownership from Bootstrap until OS process termination; no explicit unlock path | Competing process remains blocked through the last live instruction and latched read-only custody | <= 360 |
| S10 | `build(audit): lock the p0 audit schema` | Bounded payload `.proto`, append-only nonzero record-kind registry, canonical descriptor lock, Audit Registry Identity | Double generation, golden payloads, unknown/reserved/reused kind rejection | <= 400 |
| S11 | `feat(audit): compute the terminal sequence reserve` | Build-time checked worst-case formula and ordinary/terminal split | Binary maxima, no aggregation, overflow/Sequence Exhausted | <= 360 |
| S12 | `feat(audit): encode bounded chained records` | Framing, sequence, registry identity, checksum/hash link, content exclusion | Golden, oversize, corruption | <= 380 |
| S13 | `feat(audit): run one bounded audit writer` | Ordered guarded queue and strong file/namespace barriers; pre-barrier failures are Audit Degraded and required-barrier failures feed S02 | Saturation, append, file/parent sync, classification, latch, and no Event Loop direct I/O | <= 400 |
| S14 | `feat(audit): fence durable predecessors` | Typed sync-through operation drains all assigned records, strongly synchronizes/verifies the exact head, and returns a bounded Predecessor Fence | Assigned-unsynced drain, stale target/head mismatch, intervening append, and barrier failure | <= 360 |
| S15 | `feat(store): reserve p0 audit sequence ranges` | SQLite P0 Audit Sequence State; guarded `FULL` commit plus database-parent sync publishes high-water before assignment | Absent/exact/third-state custody at commit and parent crash points; no reuse/Anchor bytes | <= 400 |
| S16 | `feat(store): initialize p0 control and audit` | From proposed Configuration publish daemon IDs, empty registries, version-one State, pending sequence-one Epoch Open, and exact Audit root through guarded barriers | Crash leaves absent or one exact non-ready identity/pending custody; no second identity or forward-resume claim | <= 400 |
| S17 | `feat(store): validate p0 control mutation intents` | Bounded complete successor validation, daemon Operation ID, complete token, and one-in-flight ownership while predecessor remains active | Stale/duplicate/busy, invalid complete successor, and bounded pre-fence failures | <= 360 |
| S18 | `feat(store): fence p0 control mutations` | Obtain S14, revalidate token/fence at a Turn boundary, assign its immediate-successor sequence, and build exact pending envelope | Assigned-unsynced predecessor, intervening safety/Core Event invalidation, stale fence, no sequence on abort | <= 380 |
| S19 | `feat(store): publish p0 control mutations` | Sole guarded post-genesis transaction encodes complete candidate and advances current pointer/sequence/pending; database-parent sync precedes sole in-memory activation | No S04-S08 authority path; absent/exact/third state at commit/parent crash, no early activation, stable `outcome_indeterminate` | <= 400 |
| S20 | `feat(store): complete p0 mutation audit` | Append/sync/verify exact envelope whose predecessor was fenced, then guarded `FULL` pending-clear commit plus database-parent sync before acknowledgement | Assigned-unsynced-before-mutation crash, every Audit/clear barrier, no rollback/regeneration/early acknowledgement | <= 400 |
| S21 | `feat(audit): retain bounded p0 segments` | Segment boundaries, synchronized retention boundary, garbage eligibility | Capacity, file/parent barrier, latch, and reclaim failures | <= 380 |
| S22 | `feat(daemon): order policy-first bootstrap` | Installation Policy before socket, lock, or Runtime authority; construct S02 before opening Runtime writers, then fail-closed schema inspection | Syscall/byte ordering and no pre-guard writer fixture | <= 380 |
| S23 | `feat(daemon): classify interrupted p0 publications` | After rollback recovery, keep absent predecessor/uninitialized state, or repeat parent sync for one exact committed range/identity/generation/pending row; reject every third state | Initialization/range/mutation three-state fixtures; never regenerate or reuse possible commits | <= 400 |
| S24 | `feat(daemon): reconcile pending p0 audit` | After S23, append/verify/clear every exact Store-published initialization or mutation envelope before tail handling | Fenced predecessor required; every Audit/clear crash point; malformed/multiple pending rejects | <= 400 |
| S25 | `feat(audit): reconcile the p0 journal tail` | Only with no pending envelope, verify Clean or append one Crash Tail closing every never-assigned and assigned-but-unsynchronized-lost value through old high-water | Both lost classes, pending refusal, truncation/mismatch/sequence exhaustion, no reconstruction/reuse/Anchor fallback | <= 400 |
| S26 | `test(daemon): integrate storage barrier custody` | Exercise S02 against every real SQLite, Audit/file, parent-sync, Bootstrap, retention, and shutdown barrier while retaining the OS-only lock | Device/write stop, first observation stable, authenticated status only, no marker/Clean/retry, next process alone reconciles | <= 400 |
| S27 | `feat(daemon): restart with empty inference state` | Rebuild Control State only after S23-S25; no request/KV/Residency restoration | Initialization and mutation three-boundary crash matrices plus clean restart | <= 360 |
| S28 | `feat(daemon): enforce the process reclaim barrier` | Successor lock acquisition proves prior process exited; sampler establishes fresh post-exit baseline | Real parent/child lifetime test; no acquire-before-exit, Fake/time/socket substitute | <= 400 |
| S29 | `feat(daemon): gate p0 service readiness` | Require lock, Store/registry schema, reconciled prior bytes/Audit, Device Executor, Adapter, environment, Resource Evidence, and reclaim barrier; current-session latch remains non-ready | Successor may become ready only after exact forward reconciliation and every fresh prerequisite | <= 400 |
| S30 | `feat(daemon): perform graceful shutdown` | Reject work, cancel/release live work, owner-thread Backend shutdown, sync Clean Boundary, then terminate while still holding lock | OS-only lock release, terminal reserve, safe-point/barrier failures, no `CLEAN_RELEASED` claim | <= 400 |
| S31 | `test(store): expose bounded fault custody` | Real relative files, phase markers, initialization/mutation/restart/shutdown inspection, no Effect replay | Assigned-unsynced predecessor then mutation crash, atomic publish/corrupt/truncate/SIGTERM, assigned-lost tail, all barrier-failure custody | <= 400 |

P0 durability never substitutes Runtime Metadata or Integrity-profile repair.
Unreconcilable corruption is non-ready and requires a new runtime.

### P0 Core Gate

The Core Gate has an explicit owner after P0-6 and before qualification. Each
commit extends one deterministic matrix; K05 runs the aggregate gate and emits a
machine-readable result without claiming MLX correctness or performance.

| ID | Commit subject | Behavior slice | Required verification | Target LOC |
|---|---|---|---|---:|
| K01 | `test(gate): cover scheduling and admission properties` | Weighted service/config changes, complete Deadline Cost Bound, exact environment applicability, Admission, Exclusive lease/pause | Example/property tests and fixed seeds | <= 400 |
| K02 | `test(gate): generate lifecycle and residency sequences` | Cancel/output, Tombstone/Gone, request release, partial Reservation/Pending Reclaim split, Residency limits, Critical eviction | Generated state machine and shrinkable replay | <= 400 |
| K03 | `test(gate): inject executor and audit faults` | P0 initialization/mutation three-state recovery, durable predecessor fence, pending-before-complete-lost-tail, OS-only lock, Graceful Shutdown, Executor Failure, Audit Degraded, Storage Barrier Failure | Assigned-unsynced-before-mutation, Device/write stop, successor readiness, read-only custody, no second identity/reuse/fabricated Receipt/Effect replay/Anchor/failure marker | <= 400 |
| K04 | `test(gate): assert incremental core work` | Every operation count, complete Exclusions, dirty-Model-only recompute, member-local Receipt | Exact count witnesses at binary maxima | <= 400 |
| K05 | `test(gate): close the p0 core gate` | Aggregate examples, properties, generated sequences, faults, replay hashes | One command, complete matrix manifest, repeatable result | <= 300 |

### P0-7: Qualification And Delivery

These thin adapters expose production behavior and never contain oracle answers,
metric reduction, or synthetic success. Q00 may be implemented while the paired
contract is pending; a lane commit runs only after the separately authorized
Benchmark revision is fixed and recorded.

| ID | Commit subject | Behavior slice | Required verification | Target LOC |
|---|---|---|---|---:|
| Q00 | `build(benchmark): add the qualification launcher` | Pin Benchmark HEAD/expectation/cert/fixtures in ignored config; enforce clean-before/after | `scripts/qualify.sh inspect`, missing/stale contract failure | <= 360 |
| Q01 | `build(benchmark): add the subject handshake` | Shared Subject hello, supported lanes, build/dependency/environment identities, Data Plane descriptor, artifact roots | Hello precedes every case; containment and identity tests | <= 380 |
| Q02 | `feat(benchmark): expose core replay qualification` | `core-event-replay` adapter over real Core | Lane run and raw Transition evidence | <= 340 |
| Q03 | `feat(benchmark): expose scheduler policy qualification` | `scheduler-policy` adapter | Oracle remains Benchmark-owned | <= 340 |
| Q04 | `feat(benchmark): expose scheduler performance qualification` | `scheduler-performance` adapter, Release samples, and Runtime Overhead versions | IPC/serialization excluded; overhead drift and ledger exclusion declared | <= 360 |
| Q05 | `feat(benchmark): expose request lifecycle qualification` | `request-serving-lifecycle` through production Data Plane including parameter rejection and cancel/output/release races | Locked production descriptor, Service Class mapping, and real process identity | <= 400 |
| Q06 | `feat(benchmark): expose native correctness qualification` | `mlx-native-correctness`, reproducible full Dense/MoE graph manifests, B1/B4 omitted/zero/nonzero Seeds, complete greedy/categorical extreme-parameter and stop/Max Output matrix, external hashes | Shared exact-tensor/subnormal/no-crossing/survivor-below-K/smallest-Temperature/zero-uniform compact-index/key-state vectors, omitted-seed replay, Python/export/import parity, batch invariance | <= 400 |
| Q07 | `feat(benchmark): expose bounded turn qualification` | `bounded-turn-and-ffi` over production native boundary | One-chunk Prefill, Exclusive Safety Point, signal cleanup | <= 380 |
| Q08 | `feat(benchmark): expose governor qualification` | `residency-and-memory-governor` with real system samples | Reservation, Residency Service limits, Critical eviction, reclaim | <= 400 |
| Q09 | `feat(benchmark): expose cross-model qualification` | `cross-model-serving` through production Data Plane | Timing, fairness, progress, throughput, output evidence | <= 400 |
| Q10 | `feat(benchmark): expose observability qualification` | `observability-qualification` with honest P-1A quality | No command-buffer fairness overclaim | <= 360 |
| Q11 | `feat(benchmark): expose persistence qualification` | P0 Control/Audit initialization, sole mutation path, predecessor fence, commit/parent tri-state, pending-before-complete-lost-tail, Storage Barrier Failure, restart without native snapshot recovery | Assigned-unsynced-before-mutation, genesis/mutation custody, assigned-lost closure, next-process reconciliation/readiness, empty inference restart | <= 400 |
| Q12 | `feat(benchmark): expose same-process failure qualification` | Architecture-compatible protocol/owner/watchdog/fail-stop lane | No Backend process or private IPC | <= 380 |
| Q13 | `feat(benchmark): expose certification qualification` | Production exact environment applicability/cache invalidation, quarantine, revalidation, recertification | Thin adapter over real probe, index, and Core | <= 400 |
| Q14 | `chore: aggregate qualification evidence` | Run-level report, checksums, lane closure, and artifact containment | Every required lane represented exactly once | <= 360 |
| Q15 | `fix: resolve one qualification finding` | Exactly one finding class; repeat as separate commits | Affected lane plus full regression | <= 400 each |

After the compatible benchmark contract is fixed, complete qualification means
every required lane and matrix passes in one clean run against an applicable,
pre-frozen Certification Record. Raw evidence stays outside Git with hashes and
manifests. Failed, stale, unavailable, or inapplicable evidence remains explicit.

## Executable Verification

From B01 onward, every implementation commit runs the applicable subset of:

```sh
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo test --workspace --release
python3 -B -m unittest discover -s tests -v
python3 -B scripts/check_worktree_policy.py --base HEAD --limit 420
```

From N01 onward, native verification is exactly:

```sh
scripts/verify-native.sh
```

That script configures `.work/build/native` with CMake and Ninja, builds the
Release target, and runs `ctest --output-on-failure`; it does not write tracked
generated output. N03 adds `scripts/verify-model-export.sh`; N06 and N07 run it
with `--model dense` and `--model moe` against the exact external revisions in
ignored `.work/native/models.json`. The command exports twice into separate
ignored roots, byte-compares artifacts/manifests, runs Python-direct and C++
import logits/new-KV checks, and rejects missing or mismatched identities.

K05 provides `scripts/verify-p0-core-gate.sh` as the aggregate Core Gate
command. Protocol commits provide `scripts/verify-protocols.sh`, which
regenerates both descriptors twice, compares their locks, and runs both family
compatibility matrices. S10 provides `scripts/verify-audit-schema.sh`, which
independently regenerates the Audit descriptor and registry identity twice,
checks the lock, and runs record-kind compatibility fixtures.

After a commit is signed, its one-commit policy check is:

```sh
git verify-commit --raw HEAD
python3 -B scripts/check_commit_policy.py \
  --base HEAD^ \
  --head HEAD \
  --branch feat/p0-runtime-implementation \
  --title 'feat: implement P0 runtime'
```

After Q00 and a separately authorized compatible Benchmark contract are both
present, qualification commands are exact wrappers:

```sh
scripts/qualify.sh inspect
scripts/qualify.sh lane cross-model-serving
scripts/qualify.sh all
```

The ignored `.work/qualification/contract.json` pins absolute repository path,
Benchmark HEAD, expectation hash/path, subject manifest, Certification Record,
external fixtures, profile, and output root. The wrapper rejects a missing or
different identity and proves both repositories clean before and after.

No timing, memory, hardware, or model result is generalized beyond its exact
code, dependency, model, OS, device, memory, workload, and Certification
Envelope identity.

## Phase Exit Evidence

Each phase closes only when:

- every ledger commit for the phase is signed, policy-compliant, and at or below
  its measured 420-line plan ceiling;
- focused, workspace, Release, operation-count, and applicable native/protocol
  tests are green;
- all applicable earlier-phase tests remain green;
- three approvals exist for every exact unstaged review manifest;
- TurnVector has no unintended tracked or untracked files;
- TurnVectorBenchmark has no new changes; and
- limitations and unavailable evidence remain explicit rather than passes.

P0-6 does not exit until K01-K05 close the authoritative P0 Core Gate. P0-7 does
not exit, and the final pull request does not become ready, until the separately
authorized architecture-compatible Benchmark revision exists and one complete
valid qualification run passes. Before that point the implementation branch and
pull request remain work in progress. Merge is always a separate user-authorized
action.
