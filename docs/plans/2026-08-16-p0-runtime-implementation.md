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
6. P0-6: Qualified authority volume, SQLite Control Store, bounded P0 Audit, and
   Daemon Instance Lock.
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
the accepted P0 order above. ADRs 0008, 0011-0014, 0018, 0020, 0022, 0025,
0031, and 0034 plus
every directly affected native/Fake glossary entry are updated to include
stateless Model Descriptor and descriptor-complete Request Description, post-Admission
Request Materialization, result-gated request/residency release, Receipt-driven
dirty-Model Cost Profile updates, finite Capability Requirement/Authorized Sets,
ownership-gated release-before-unload, strong rollback for failed loads, explicit
Exclusive execution, load/unload/cache reclaim through Residency Transition, a
manifest-bound complete Backend Capability descriptor, a versioned Backend
Resource Signal Contract with contract-bound samples, an externally verified
Backend Operation Bound Set, a bounded readiness-time Owner-Thread Support Budget,
post-load current-generation description revalidation, exactly-once result-gated
Backend Shutdown, and a shared read-only Device Control Signal view.
Those changes also remove `indeterminate` as an ordinary Turn
Receipt: process fail-stop prevents a Receipt from being fabricated.

ADR 0022 and the request/sampling glossary entries then freeze one closed P0
Generation Parameters contract. Every request explicitly supplies Sampling Mode,
finite IEEE-754 binary32 Temperature and Top P, and unsigned 32-bit Top K;
omitted values have no default, and negative zero, NaN, or infinity is invalid.
Greedy requires positive zero Temperature, Top P `1.0`, and Top K `0`.
Categorical requires Temperature in `(0, 2]`, Top P in `(0, 1]`, and Top K either
`0` (disabled) or below the registered Model Descriptor's nonzero vocabulary
size. Accepted binary32 bit
patterns are preserved in the immutable request.

The categorical algorithm is exact. A non-finite model logit or a finite wider
logit whose binary32 cast is non-finite fails the Turn before sampling. It casts
the remaining finite logits to binary32 and evaluates these pinned-MLX
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
tests. A qualification-only tensor seam outside the production Backend Interface
injects NaN, both infinities, and finite wider values whose binary32 cast is
non-finite; every case must fail before RNG split, Sampling State advance, or
output publication.

One canonical Generation Semantics descriptor enumerates that complete
schema/domain, exact token-selection algorithm, and RNG transition; deterministic
double generation reproduces its bytes and Evidence Hash identity. The daemon
Domain Type registry and the Backend Bootstrap Manifest-bound complete Capability
descriptor carry the same expected hash; Backend Initialization must match it
exactly. Environment Qualification, every fresh Environment Fingerprint,
Certification Envelope comparison, and the Admission recency-cache key carry that
identity, so any schema or algorithm drift fails closed before shared Admission.

Registration durably commits the complete canonical Model Descriptor bytes beside
the Model Manifest in the Control State Snapshot; the Manifest's descriptor hash
is only their validator. Bootstrap restores and rehashes those bytes before
Service Readiness, and missing or mismatched bytes enter Control Repair Mode rather
than regenerating vocabulary authority from artifacts, logits, model defaults, or
the hash. Post-load observation must equal the restored canonical bytes, identity,
hash, and vocabulary size.

P0 uses the existing immutable Storage Qualification Record format without
importing the Integrity Profile's Identity Anchor or Rebind machinery. An offline
qualifier holds the installation qualification lock, executes the build-owned
real syscall profile on the exact Runtime Authority Volume, synchronizes one
immutable record, and publishes it through the fixed two-slot Head. Installation
Policy names one exact record identity, never `latest`. Before any Runtime writer,
Bootstrap freezes that record and verifies its volume UUID, OS build, TurnVector
build, Storage Capability Profile, record chain, and bytes. For an existing or
possibly committed identity this proof authorizes SQLite rollback recovery only;
after recovery the immutable Runtime Identity Record must bind the same exact
record before any later Runtime write. Control Initialization additionally holds
the qualification lock, verifies that the Policy record is currently Head-
selected, and persists the binding. P0 offers no same-ID qualification rebind, so
binding or environment drift requires a new runtime rather than an implicit Head
selection, daemon write probe, or durability downgrade.

A new profile-specific ADR then defines P0 Audit sequence authority without
silently importing the Integrity Profile's Identity Anchor. The SQLite Control
Store owns a small durable P0 Audit Sequence State outside the semantic Control
State Snapshot: History Epoch, next range predecessor, and reserved high-water.
Control Initialization creates it; afterward only the guarded range-reservation
primitive on the sole Store executor may advance it. It is daemon-owned pre-operation
infrastructure that consumes no Event Sequence and creates no Core Event, Effect,
Result, mutation Operation ID, Audit Record, or Pending Audit Envelope. A complete
range becomes assignable only after its `FULL` SQLite commit and explicit database-
parent synchronization. Control Mutation consumes an already-reserved value and never
updates high-water. The installation-scoped Daemon Instance Lock has one fixed
identity outside every Runtime and proves process exclusivity across Runtime ID
replacement; exact
Audit chain validation against reserved high-water proves the tail. Bootstrap
first appends, synchronizes, verifies, and clears every exact Store-published
Pending Audit Envelope. Only when no pending envelope remains may graceful
shutdown write a strongly synchronized Clean Shutdown Boundary for its unused
suffix; Bootstrap verifies that boundary, or an unclean restart reserves a new
range and writes one Crash Tail Boundary for the complete unrecoverable prior
suffix through its high-water. That suffix includes never-assigned values and
values assigned only to lost, unsynchronized records; exact Store-published
envelopes are completed first and therefore excluded. No lost record is guessed
and no assigned value is reused. P0 never emits or claims Anchor
`CLAIMED`/`CLEAN_RELEASED`. Unreconcilable Store/Audit mismatch is non-ready and
requires a new runtime. The ADR updates the profile boundaries in ADRs 0019, 0021,
0026, 0027, 0029, 0036, and 0040 plus their glossary terms; Integrity semantics
remain unchanged.

The same ADR defines P0 Control Initialization without Locator, Initialization
Manifest, Identity Anchor, Runtime Metadata, or hidden defaults. Under the held
Daemon Instance Lock, the daemon acquires the installation qualification lock;
the offline qualifier never acquires a Runtime lock, so no reverse order exists.
The daemon retains that lock through initialization commit and database-parent
synchronization, then may release it. An authorized complete proposed
Configuration Snapshot plus the exact verified Policy-selected storage record
causes the daemon to generate the random Runtime ID and History Epoch.
Initial Model, Alias, Certification, and model-scoped Configuration sets are
explicitly empty and may change only by later Control Mutation. Initialization
publishes the immutable qualification binding, version-one Control State, the
first fully synchronized sequence range, and one exact pending Event Sequence one
Epoch Open envelope in a new SQLite Store. Transaction commit and explicit
database-parent synchronization are distinct crash boundaries; together they form
the live Native Authority publication barrier. Before generating any new identity
or sequence after a crash, the next process completes SQLite rollback recovery and
classifies exact Store bytes. An absent transaction remains uninitialized. An
exact committed candidate reuses its qualification, Runtime ID, History Epoch,
range, generation, and pending envelope, repeats the parent synchronization, and
finishes forward. Any third state fails closed. Epoch Open fixes the Audit Registry
Identity and initial Generation Hash before readiness, and no path creates a
second identity from a possibly committed transaction.

For every later P0 Control Mutation, a bounded executor validates one complete
successor while the predecessor remains active. Before allocating a Control Mutation
Operation ID, the Event Loop proves the complete build-derived Control Mutation
Sequence Headroom remains in already durable ordinary ranges; if not, it completes
the guarded range-reservation transaction first. Checked arithmetic with no
aggregation covers the Effect-creation transition, the P0 Control Fence Attempt
Limit times its per-attempt discretionary Intervening Assignment Limit, all
mandatory Critical/cancellation/disconnect/Device-Failure/shutdown transitions
admitted through bounded Core Event Reserve, every worst-case remaining ordinary
safe completion in the build-generated Runtime Closure Registry, and the larger
terminal branch of one nonpublication failure-or-cancellation Result, including
trustworthy exact Store absence, or the Store-publication plus exact-envelope-
completion Result pair. It cannot borrow Terminal Sequence Reserve. Sequence
Exhausted or trustworthy exact-absent reservation outcome therefore leaves no mutation
Effect or owner. An indeterminate reservation commit or parent barrier latches Storage
Barrier Failure without a mutation ID or Control Outcome Indeterminate response and
makes no unchanged-high-water claim. At a
synchronized boundary the Event Loop revalidates the complete mutation token,
current next value, and full headroom; drift is a no-ID state conflict. Only then
does the daemon allocate one Operation ID, and Core atomically records the
prepublication Effect, one-in-flight proposal owner, and headroom charge. Owner
creation atomically closes the Core-owned Runtime Closure Gate until the terminal
Result. The build-generated Runtime Closure Registry exhaustively owns every Core
transition that can increase future sequence-and-Audit closure liability, including
Request Acceptance, Core connection-state creation, Reservation or Residency Demand,
and a new independent Operation or Effect. While closed, only a registered transition
that advances or replaces already charged obligations without increasing its
remaining maximum may commit; read-only status that creates no Core object remains
available. C37 owns the registry and Core gate state; S11 consumes its generated
checked maxima but cannot add transition kinds or change gate policy. The daemon then
requests a typed Predecessor Fence
from the single Audit Writer: every earlier assigned record must be appended,
strongly synchronized, verified, and the returned Audit Head must exactly match the
proposed predecessor. The writer returns this daemon-orchestration witness without
creating a Core Event or Effect Result. Each attempt has one closed outcome:
`granted`, `stale`, `audit_degraded`, or `storage_barrier_failure`. Intervening
assignment, Core Event, safety event, head change, target mismatch, or writer-
generation change is `stale`; it grants no publication authority and assigns no
publication sequence. Each outstanding attempt permits only the nonzero build-owned
discretionary Intervening Assignment Limit; after that many discretionary ordinary
assignments, the Event Loop defers more discretionary work while bounded Fence I/O
completes. Mandatory safety events remain processable and consume their separately
calculated component of the same headroom, never Terminal Sequence Reserve.
The daemon may retry only below the nonzero P0 Control Fence Attempt Limit. Individual
stale attempts are internal and unsequenced. Before barrier entry, a known graceful
cancellation uses the protected prepublication terminal Result to close the Effect,
owner, and headroom and reopen the Runtime Closure Gate; a fail-stop safety outcome
fabricates no Result. Exhaustion assigns the current next
value to one nonpublication failure Effect Result; Core closes the Operation ID,
proposal owner, and remaining headroom and reopens the Runtime Closure Gate before
stable typed busy. An
`audit_degraded` outcome is daemon-owned I/O custody: entry, failed supervision,
and recovery assign no Event Sequence or Result. It keeps the Operation ID, owner,
headroom, and exact Core stage pending, then resumes Fence acquisition while
allowing only the already budgeted safety and registered nonincreasing closure transitions.
Those closure transitions must be registered nonincreasing Runtime Closure Gate
transitions.
Audit Safety Reserve separately provides build-derived nonborrowable count-and-byte
capacity for those records, so ordinary queue saturation cannot consume it.

At a synchronized Turn boundary the Event Loop performs final token/fence
revalidation and enters the bounded Control Publication Commit Barrier by assigning
the protected Store-result slot's next contiguous value to the pending Store
publication Effect Result before external I/O and building its exact pending
envelope. Until the typed Store publication result is consumed, that pre-sequenced
result transition is the only Core Transition permitted; no other Backend call,
Event Sequence assignment, or in-memory Control activation may occur. Barrier entry
also publishes the Event Loop-owned Control Mutation Cancel Gate for the exact
`{Daemon Instance ID, Operation ID, checked nonzero barrier generation}` after
assigning the protected value and establishing the stage, but before external I/O.
The session-local generation advances on every publication and never wraps. Control Plane authorization,
protocol and Command ID validation, cancel permission, and bounded cancellation-ingress
and Direct Response reservation precede the gate check on the single ordered Event Loop ingress path.
The Event Loop rechecks the exact gate before Core conversion, and one connection
completes each receive-order admission decision before a later command advances. While
any gate is closed, every authorized Control Mutation cancellation completes before Core.
An exact tuple receives typed `cancel_window_closed`; an unknown or different Daemon
Instance ID, Operation ID, or barrier generation receives opaque `commit_in_progress`
without disclosing identity existence. Both release their reservation after write or
that Control Plane connection terminates and create no Core Event, Event Sequence,
Effect Result, Audit Record, Domain Rejection, headroom charge, Store outcome, or
unchanged-state claim. The gate remains
closed through C19, barrier release, and exact-envelope completion and clears only
with terminal owner release; Storage Barrier Failure never clears it. The originating
Control Plane connection's disconnect ends only mutation-result delivery and may
create at most one already charged deduplicated connection transition. Data Plane
Client Disconnect retains its canonical audited cancellation of every owned request.
None can overtake the Store Result, abort classification, select the pre-entry
cancellation branch, or claim unchanged state. P24 owns the ordered protocol handler
over a read-only gate projection; S18 alone publishes and clears that projection with
the mutation barrier and owner lifecycle.
The Event Loop performs no direct disk I/O; the sole bounded Store executor
serializes this work with range reservation.

One guarded `FULL` SQLite transaction writes the complete candidate, current
pointer, and pending envelope while leaving P0 Audit Sequence State unchanged.
Explicit database-parent synchronization completes P0's sole post-initialization
Control Native Authority barrier. Trustworthy exact absence consumes the pre-
sequenced Store-result slot as one nonpublication failure Result, preserves the
predecessor, and closes the Effect, owner, and headroom without activation or a
dependent Effect. Trustworthy exact committed success instead drives C19, activates
the generation, advances the same owner, emits one dependent exact-envelope
completion Effect, and preserves the last protected Result slot before the commit
barrier releases. An indeterminate outcome produces no Result. Barrier release does not
release the owner or ordinary work: the Runtime Closure Gate and Control Mutation
Cancel Gate remain closed, and discretionary Turn dispatch stays deferred while only
headroom-charged bounded safety and registry-approved nonincreasing closure transitions
may interleave with exact-envelope completion. Audit append,
synchronization, verification, and a `FULL` pending-clear transaction plus parent
synchronization then return that Effect's ordinary sequenced Result. Only its
accepted Core transition releases the owner/headroom, reopens both gates, and
acknowledges success. Every Control Mutation cancellation continues to receive its
bounded pre-Core Direct Response: `cancel_window_closed` for the exact tuple or opaque
`commit_in_progress` for any other tuple. The originating Control Plane disconnect
affects only delivery. Neither rolls back the candidate, repeats
Store publication, or closes the owner. An
ordinary post-publication append failure before the required sync attempt retains
the same Effect, owner, headroom, and exact envelope-completion stage under Audit
Degraded; supervised recovery may resume only that Effect, never allocate another
Operation ID or repeat Store publication.
After Operation ID allocation, failure of an actually attempted required
Predecessor Fence file/parent synchronization, SQLite commit, database-parent,
Audit synchronization/verification, pending-clear commit, or pending-clear parent
barrier enters P0 Storage Barrier Failure and returns stable
`outcome_indeterminate` without claiming either predecessor or candidate. Before ID
allocation, candidate/token/capacity and trustworthy exact-absent range outcomes are
typed pre-operation failures, but a range commit or parent-barrier failure still
latches Storage Barrier Failure without a mutation ID or unchanged-high-water claim.
Queue capacity,
ordinary append failure, or inability to reach a required synchronization attempt
is `audit_degraded` and retains the exact pending operation stage; individual
`stale` attempts remain internal, while bounded exhaustion uses the ordinary
sequenced failure Result described above. None assigns the publication sequence or
latches Storage Barrier Failure. A required fence durability failure occurs before
publication-sequence assignment, so the current next value and all remaining
headroom values remain unassigned, the live Effect is discarded only by fail-stop
Daemon Failure, and the successor process closes that tail without recovering the
Effect. Failure never
causes the daemon to fabricate a missing Effect Result. After
barrier entry, Bootstrap first classifies the Store publication:
an absent candidate leaves the assigned value to close as assigned-but-
unsynchronized-lost, one exact committed candidate completes any exact Pending
Audit Envelope forward and excludes its sequence from the tail, and a third state
fails closed. Storage Barrier Failure stops Device work and readiness, forbids
every later Runtime write and same-session retry, retains the Daemon Instance Lock,
and exposes only authenticated read-only status plus best-effort diagnostics until
OS signal termination. It writes no failure marker or Clean Shutdown Boundary.
Only a later process may acquire the OS-released lock and reconcile stored bytes
forward. P0 creates no Metadata staging or Anchor state for this protocol.

After rollback recovery, Bootstrap classifies each P0 durable boundary by its own
persisted evidence and never recovers or trusts a session-local Fence. Initialization
is absent and Uninitialized, one exact committed identity candidate whose pending
Epoch Open uses the Epoch Genesis Hash, or invalid. Range reservation is absent with
the predecessor high-water, one exact committed P0 Audit Sequence State that receives
any missing parent barrier and has no Pending Audit Envelope, or invalid. Control
Mutation is absent with the predecessor, one exact committed candidate carrying one
exact envelope, or invalid. The mutation envelope may be appended only when its
persisted predecessor equals the current verified Audit head; if the exact record is
already present, its sequence, predecessor, bytes, and identity must verify in-chain
before clear. Pending-clear is classified separately as uncleared, exactly cleared,
or invalid. Commit-before-parent-sync, parent-sync completion, Audit append, and
pending-clear recovery each have separate witnesses. A live required durability
error latches Storage Barrier Failure; a process crash leaves byte classification to
the successor.

The bounded SQLite executor applies the same commit-plus-parent-directory barrier
to every successful P0 write transaction, including registry, Configuration,
Certification, sequence, initialization, mutation, and pending-clear writes. No
caller may observe a published generation, assigned sequence, cleared pending
envelope, or successful acknowledgement between those two barriers.

Model, Configuration, and Certification Store helpers can encode and validate
immutable rows only inside a caller-owned uncommitted transaction. They expose no
commit, current-pointer, in-memory activation, or acknowledgement operation. P0
Initialization is the only genesis path; afterward the unified mutation
transaction is the only path that can advance Control authority, while the
guarded range-reservation primitive alone advances P0 Audit Sequence State. The
mutation parent barrier is the only point after which Core may activate its
successor. External model, Configuration, and Certification commands therefore
create a complete proposal Effect, not an active Core transition. Only the typed
committed-publication result consumed inside the Control Publication Commit Barrier
can drive C19's atomic activation.

A single daemon-session Storage Barrier Failure latch and write guard exist
before any Runtime Store/Audit writer or Device startup. The first typed required-
barrier error atomically closes that guard, sets the Device shutdown signal, and
fixes the read-only Barrier Failure Observation; every later Runtime write is
rejected before a syscall. Repeated observations cannot replace the first fact,
retry a write, emit Audit, release the instance lock, or enter graceful shutdown.
All P0-6 writers consume this one guard rather than implementing local recovery.

The installation-scoped Daemon Instance Lock has no live-process release operation
and no Runtime-derived path. Its descriptor
remains owned through Backend destruction, Clean Shutdown Boundary, and the last
daemon instruction, and only OS process termination releases it. A successor
therefore cannot acquire that unchanged lock while the prior process is alive,
whether the predecessor exits gracefully, fails, or is replaced under a new Runtime ID; acquisition
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

Before `initialize`, a build-generated daemon-embedded Backend Bootstrap Manifest
binds the expected implementation, complete Backend Capability descriptor, Signal
Contract, and Operation Bound Set identities plus the externally certified
initialization blocking/safe-point entry. The daemon verifies that artifact and
starts the pre-call watchdog from its bound. `initialize` returns the complete
bounded versioned descriptors, which must exactly match the trusted Manifest before
any later call; no Backend self-report supplies its expected value. A nominal
success with mismatched evidence fail-stops without another Backend call or raw
deallocation. A trustworthy
initialization failure strongly rolls back all root ownership before return and is
not retried in the same process. `describe_model` generates the hash-bound Model
Descriptor before registration. The daemon-owned `request` input to stateless
`describe_request` contains the frozen Token Request, exact validated Model
Descriptor, and current Backend Generation, all of which the Result binds; no
Backend registry lookup, request handle, KV, or Sampling State exists at this
point. The Result returns a complete finite Capability Requirement Set, never a
future batch/Shape choice, Envelope, applicable Record, or exact authorized Key.
Only a current-generation Result may reach Admission. C10 is the sole description-
refresh owner: any Backend Generation advance invalidates all older not-yet-
admitted Results in O(1) by generation identity. It reissues resident Preparing
requests in stable Request ID order under the original timeout; Warming requests
remain stale until their own post-load gate. A stale Result causes no Admission
decision or terminal rejection. Its EffectResult is consumed by current-generation validation before
another Backend-mutating Effect is selected. The daemon joins every requirement
with fresh environment evidence; Admission verifies the complete exact-key closure
and freezes the Authorized Capability Set plus worst-case bounds.
`materialize_request` is permitted only after Admission atomically creates the
typed request Resource Reservation and Timing Commitment. Only its Request Backend
Allocation Budget crosses the Seam. The Result binds the exact Operation, Request,
Reservation, Budget, releasable ownership when nonzero, and checked complete
Budget partition; it carries no output, transient-headroom, or timing authority.
A proven never-started or pre-materialization path makes no Backend release call
and Core withdraws only its applicable unconsumed Reservation. A zero-ownership
Materialization Result also makes no release call but authorizes Core to split its
proven never-allocated remainder from proven
actual allocation entering Pending Reclaim. Any complete or partial Backend
ownership is released exactly once by `release_request` on the owner thread.
Terminal Core state retains the complete Request Backend Allocation Budget until
its accepted Release Result consumes ownership, releases the Budget's proven
never-allocated remainder, and moves proven actual allocation into Pending
Reclaim, where it remains until fresh allocator and footprint convergence proves
reclamation. An operation unable to return the required synchronized deterministic
result triggers process fail-stop: it is never retried, treated as success, or
cleaned by cross-thread destruction.

Candidate Formation reports one exact Capability Key contained in every member's
frozen Authorized Capability Set, the Backend-owned Cost Profile version, and
ordinary estimates. Only an accepted Receipt may enter the compare-and-set
`observe_turn_receipt` call between Turns. An unexpected version mismatch rejects
before mutation but proves divergence and fail-stops; a match returns unchanged
or atomically installs a new profile and returns exact old/new identities. Core
mirrors only the identity; a Cost Profile Commit Barrier makes the result the
next Core Event with no intervening Transition or Backend call. Core must
immediately commit an applied update, advance Backend Generation, invalidate every
older Scheduling Snapshot, Turn Plan, and not-yet-admitted Request Description,
route those accepted requests through C10, and mark only its affected Model dirty. A
malformed result or failed Core commit after Backend mutation fail-stops before
another Backend call. The Adapter never widens certified bounds. A Turn Plan grants
target Engine Service and work ceilings, while the Backend chooses the concrete
Prefill token range.
`TurnResult` carries bounded opaque progress, staged token output, and per-member
outcomes; tensors, raw logits, and KV layout never cross the Seam.
Qualification-only logits/KV hashes use a separate test build seam and never enter
Core or the serving ABI.

`transition_residency` has serialized load, unload, and bounded allocator/cache-
reclaim variants. Every variant declares Resource Impact, blocking and safe-point
bounds, cannot overlap a Turn, and produces a Residency Result; cache reclaim may
leave Model Residency unchanged, changes no ownership, and does not prove
physical reclamation. Core acceptance of a successful load Result retains
ownership and the full Residency Reservation, advances Backend Generation, and
invalidates that Revision's pre-load Model Descriptor observation plus every
older not-yet-admitted Request Description. While it remains
unavailable for Candidate Formation, the owner thread repeats `describe_model`
against the exact registered Manifest and requires the returned identity and hash
to equal the registered Model Descriptor. It then repeats `describe_request` for
each bounded remaining Warming waiter against that descriptor and current
Generation through C10; every unrelated invalidated request uses the same owner.
Only fresh Results may reach Admission. Descriptor drift marks only the loaded
Revision Unavailable and fails its waiters; Generation change alone does not fail
an unrelated request. A terminal failed/cancelled load
strongly rolls back to zero ownership before its accepted Result authorizes Core
to release the proven never-allocated remainder and move proven actual allocation
to Pending Reclaim. Unload requires zero request ownership and zero pending
release; successful unload performs the same split, while failed unload retains
ownership and charge. Backend Resource Samples bind the exact initialization
Signal Contract identity; daemon sampling joins them with process and system facts
before the Governor derives Resource Mode.

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

Every other Backend call permitted while Service Readiness is true consumes its
externally verified finite continuous blocking and next-safe-point bound from the
active Backend Operation Bound Set. Complete describe,
materialize, release, Candidate Formation, Receipt-observation, and resource-
sampling direct-call intervals consume a versioned Owner-Thread Support Budget as
Runtime Overhead. The budget has fixed per-call, call-count, and aggregate-time
maxima per finite scheduling window; every active Deadline Cost Bound reserves
its worst-case interference. Mandatory release and safety sampling use separately
pre-reserved capacity, while nonessential work defers or rejects when it cannot
fit. Initialization runs before readiness, shutdown after readiness is false,
Residency Service remains separately gated, and Exclusive execution pauses shared
dispatch. No queued maintenance is treated as free background work.

`shutdown` is the final exactly-once owner-thread operation after readiness is
false, no Backend operation is in flight, and every request ownership and pending
release is zero. It may destroy remaining resident model, stream, cache, and root
state under the verified Shutdown entry in the Operation Bound Set and leaves only
an empty opaque shell containing no Backend or MLX object. The private nonthrowing,
bounded raw deallocator accepts that shell only after a
successful `ShutdownResult`, or after trustworthy initialization failure already
proved zero root ownership; it rejects live initialized state. Only a trustworthy
synchronized `ShutdownResult` proving zero Backend root, model, and request
ownership permits shell deallocation and a Clean Shutdown Boundary. It never
proves physical reclaim, and all current-process capacity remains unavailable
until process exit and a successor's fresh Process Reclaim Barrier baseline.
Failure, an untrustworthy Result, or a missed safe point fail-stops without retry,
raw live-state destruction, fabricated Result, or Clean Shutdown Boundary.

Backend Initialization must match the preverified Backend Bootstrap Manifest and
returns exact Adapter, MLX, Backend Interface, complete Backend Capability
descriptor, Backend Resource Signal Contract, and Backend Operation Bound Set
identities plus both complete bounded descriptors, never an authorization or
Resource Mode decision. A daemon-owned bounded platform probe supplies device,
GPU, unified-memory, and macOS facts and joins them with the initialized build,
Interface, Bootstrap Manifest, exact Capability descriptor, Signal Contract, and
externally verified Operation Bound Set identities into one fresh Environment
Fingerprint. Admission alone evaluates exact
Certification Applicability and may cache the answer in a fixed-capacity
recency cache keyed by every determining Control, record, capability, build,
interface, and environment identity. Every hit rechecks the current evidence's
identity and freshness; stale evidence fails closed, and any changed member
invalidates the entry. Certification Records have no invented wall-clock expiry.
The cache is neither persisted nor audited as authority.

Runtime Overhead remains daemon-owned. Versioned conservative bounds cover Plan
formation/selection, Owner-Thread Support Budget interference and, after a
synchronized Receipt, result validation, Core commit, and output publication. On that started-Turn branch, the complete
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

The Runtime Overhead witness is continuing. E16 closes both Turn-result branches
over the first working Fake Event Loop; E17 adds every readiness-time support-call
span and its aggregate interference budget. Every later commit that adds covered
work, especially protocol conversion, Core result handling, and Output
Publication, extends the same span partition, evidence key, and bound regression
in that commit; a partial timeline cannot be treated as the production bound.

`BackendResourceSample` contains only owner-thread MLX allocator/cache evidence
and binds the exact Backend Resource Signal Contract identity returned at
initialization. A daemon-owned sampler collects process footprint, available
memory, swap, compressor, and macOS pressure events without hot-path shell
commands. A bounded assembler rejects a mismatched contract, retains each
source's provenance, sequence, quality, and freshness, and is the only component
that emits complete `ResourceEvidence`; the Backend never selects Resource Mode.

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
| D02 | `docs(adr): complete the in-process backend seam` | Update ADRs 0008, 0011-0014, 0018, 0020, 0022, 0025, 0031, 0034 plus glossary for capability closure, ownership release/unload, load rollback, profile CAS, transition, Bootstrap/Signal/Operation descriptors, Support Budget, Exclusive, fail-stop | Admission, timing, release, reclaim, resource-signal, ownership, and conformance consistency | docs only |
| D03 | `docs(adr): freeze the p0 token generation contract` | Update ADRs 0022/0027/0031 and glossary with compact nonempty-support Generation Parameters, RNG advancement, durable registered Model Descriptor authority, build-bound Generation Semantics/Certification identity, exact binary32 tensor flow, and Service Class mapping | Descriptor restart, enum/presence/range/tie/cutoff/non-finite/subnormal/no-crossing/survivor-below-K/zero-uniform/state-vector and applicability-invalidation matrix; no opaque Backend parameters | docs only |
| D04 | `docs(adr): define p0 audit sequence authority` | Define exact P0 storage-qualification binding, Control Initialization, sole later Control mutation authority, range-only sequence-state authority, build-derived mutation headroom and Audit Safety Reserve, Core-owned Runtime Closure Gate, Event Loop-owned Control Mutation Cancel Gate, bounded noncreating Predecessor Fence interleaving, known-absent closure, two-stage success Results with no post-C19 discretionary work, installation-scoped process lock across every successor, Control Publication Commit Barrier, commit/parent recovery, pending-before-tail, lost suffix, and pre-writer Storage Barrier Failure; align ADRs 0019/0021/0026/0027/0029/0036/0040 plus glossary | No Anchor/Locator/Metadata, implicit latest qualification, same-ID P0 Rebind, second pointer, sequence-owner/headroom overlap or terminal borrowing, closure-liability fanout, delayed or identity-cycling cancel custody, safety-queue starvation, Event Loop publication/result gap, post-C19 discretionary assignment, clean-restart or cross-Runtime lock bypass, fabricated indeterminate Result, or unchanged-state claim after a required barrier failure | docs only |
| D05 | `docs(adr): bind runtime overhead evidence` | Add daemon-owned evidence-bound, result-branch-disjoint overhead plus complete Owner-Thread Support Budget interference and update ADRs 0015/0024 plus glossary | Receipt, Plan Rejection, and every readiness-time support call have complete nonoverlapping spans and Deadline Cost coverage without online widening | docs only |
| T01 | `build: audit unstaged commit scope` | Tracked/untracked path hashing and documentation-aware LOC checker | Unit fixtures for add/delete/rename/binary/untracked and self-count | <= 300 |
| B01 | `build: initialize the Rust workspace` | Rust 1.97.1 toolchain, workspace, core crate, format/lint/test entrypoints | Format, clippy, workspace tests | <= 180 |
| B02 | `build: lock the generation semantics descriptor` | Canonical descriptor generator and Evidence Hash lock shared by daemon Domain Types, Fake, Manifest, Native, and qualification | Double-generation bytes/hash; incomplete/unknown descriptor rejection; every schema/domain/greedy/categorical/RNG semantic mutation changes identity | <= 300 |

### P0-1: Pure Runtime Core And Replay

| ID | Commit subject | Behavior slice | Required verification | Target LOC |
|---|---|---|---|---:|
| C01 | `feat(core): add checked domain identities` | Distinct IDs, units, sequences, durations, and Monotonic Time | Overflow, zero, and cross-type rejection | <= 360 |
| C02 | `feat(core): add generation and bounded collection types` | Generation Vector plus checked fixed-capacity collections | Capacity and generation mismatch cases | <= 340 |
| C03 | `feat(core): define scheduling snapshots and candidates` | Snapshot, exact-key Work Candidate, authorized-member set, and Candidate Exclusion contracts | Construction, Key membership, and completeness bounds | <= 380 |
| C04 | `feat(core): define turn plans and receipts` | Frozen Plan membership and completed/cancelled/partial/failed member outcomes | Stable order and member bounds | <= 380 |
| C05 | `feat(core): apply atomic core transitions` | `Core::handle`, contiguous Event Sequence, ordered Effects, rejection/fault shell | Failed invariant preserves state and emits no Effect | <= 400 |
| C06 | `feat(core): establish hot-path work budgets` | Incremental witness types, binary maxima, counted transition shell, and hard rejection | Exact base counts, overflow, no truncation or full-state scan | <= 380 |
| C07 | `feat(core): manage immutable model revisions` | Manifest identities, Alias freeze, Available/Retiring/Unavailable lifecycle | No alias repoint, registry limits, and incremental counts | <= 400 |
| C08 | `feat(core): describe model registrations` | Registration proposal, stateless Model Descriptor Effect, complete canonical descriptor and nonzero `u32` vocabulary retention, hash binding, and rejection | No Revision or vocabulary authority exists before descriptor validation; in-memory record preserves exact bytes/hash and post-load equality | <= 380 |
| C09 | `feat(core): accept requests into preparing` | Ownership, frozen Revision, descriptor-authoritative Top K validation, explicit Service Class, closed Generation Parameters, immutable effective `u64` Sampling Seed plus origin, status version, and preparation timeout | Descriptor vocabulary bounds, explicit zero/caller/daemon origin; Acceptance is not Admission; retries never inherit state | <= 400 |
| C10 | `feat(core): drive and refresh request descriptions` | Sole initial/post-load/stale-generation owner with O(1) invalidation, stable resident reissue, and deferred Warming refresh | Cross-model advance/counts, exact post-load equality, no stale rejection/Admission, original timeout, registry, handle, or Reservation | <= 400 |
| C11 | `feat(core): authorize exact certification keys` | Current-generation finite requirement-to-Key closure over immutable records and the read-only exact-key Authorization Index | Stale routes to C10 without lookup; omitted/overflowed/missing/drifted/quarantined cases fail closed | <= 400 |
| C12 | `feat(core): derive certification applicability` | Fresh Environment Fingerprint including exact Capability, build-verified Generation Semantics, Resource Signal, and verified Operation Bound identities plus Admission-owned fixed recency cache | Every hit rechecks freshness/evidence; schema/algorithm/other identity drift invalidates; miss/eviction | <= 400 |
| C13 | `feat(core): apply the fixed admission check` | Current-generation description, Revision, complete Authorized Capability Set, worst-case timing/support interference, Resource Mode, and capacity sufficient conditions | Stale never enters Admission; every missing Key or failed condition rejects before state allocation | <= 400 |
| C14 | `feat(core): commit timing and request reservations` | Atomic frozen Authorized Capability Set, Timing Commitment, typed Request Backend Allocation Budget, daemon output/transient components, and complete Resource Reservation after Admission | Complete-set/component identity, conservation, rollback, and no partial commitment | <= 400 |
| C15 | `feat(core): materialize admitted requests` | Post-Admission materialization Effect and stable Backend request ownership identity | No materialization before Reservation | <= 360 |
| C16 | `feat(core): apply request materialization results` | Success owns state; zero-ownership failure splits the Request Backend Allocation Budget directly; partial ownership retains its complete Budget for release | Budget identity/conservation, daemon-component isolation, zero-allocation/allocated rollback, no-call/retry/early split/leak | <= 400 |
| C17 | `feat(core): revalidate certified profiles` | Candidate/description invalidation and existing Reservation feasibility | SLO Risk and new-Admission block without silent re-promise | <= 380 |
| C18 | `feat(core): quarantine certified bound violations` | Preserve Receipt, remove exact key, optional parent escalation, explicit recertification | Estimate miss does not quarantine; automatic widening forbidden | <= 400 |
| C19 | `feat(core): activate complete control successors` | Exact-absent Store Result preserves the predecessor and closes Effect/owner/headroom without a dependent Effect; only a committed Store Result atomically installs the complete Snapshot at a Turn boundary, then advances the same owner and emits its dependent exact-envelope completion Effect while retaining the final headroom Result slot; Weight baseline/new-Admission Timing Commitments | Known absence closes once; proposal cannot activate; Store success cannot acknowledge/release before accepted evidence-completion Result; no missing Effect, partial visibility, historical debt, or rewritten promise | <= 400 |
| C20 | `feat(core): order cooperative cancellation` | Preparing/Warming/pre-materialization no-call cancellation, owned queued release, in-flight Cancel Pending, staged-output discard, status transition | Event Sequence, ownership, Receipt, and Publication matrix | <= 400 |
| C21 | `feat(core): reserve per-turn output capacity` | Concrete pre-execution Turn Output Reservation Effect and non-runnable backpressure state | No output Plan without capacity; cancellation releases reserve | <= 380 |
| C22 | `feat(core): release backend request state` | Ownership-gated exactly-once release Effect; only accepted Result consumes ownership, partitions the exact Request Backend Allocation Budget, and moves actual allocation to Pending Reclaim | No daemon-capacity authority, no-call unowned path, partial allocation/use, terminal/cancel/eviction, no retry/unload/early reuse | <= 400 |
| C23 | `feat(core): bound terminal request history` | Count/time Tombstones, connection Request High-Water Mark, and authorized Gone | Evicted, foreign, and never-issued IDs remain distinct | <= 400 |
| C24 | `feat(core): manage exclusive mode leases` | Requested/active/exit states, renewal/expiry/disconnect, shared dispatch pause | No automatic entry; new Admission/Residency Demand reject | <= 400 |
| C25 | `feat(core): authorize exclusive operations` | One operation with conservative peak bound and certified Exclusive Safety Point | Uncertified shared work never silently probes | <= 380 |
| C26 | `feat(core): filter unsafe scheduling work` | Resource Mode and all generations before urgency/fairness | Unsafe or stale work never reaches selection | <= 360 |
| C27 | `feat(core): account runnable weighted service` | Runnable-only Model Ledger and Device Executor Receipt charging | 1:3 example and idle re-entry without stored credit | <= 400 |
| C28 | `feat(core): compose deadline cost bounds` | Certified Engine Service plus exact record-bound, disjoint Runtime Overhead and worst-case Owner-Thread Support Budget interference | Applicability/drift invalidation; no gap, double count, or Ledger charge | <= 380 |
| C29 | `feat(core): select deadline-aware turns` | Latest Safe Start, Urgent Set, fair fallback, and stable ties | Independent scheduler oracle scenarios | <= 400 |
| C30 | `feat(core): enforce candidate formation laws` | Complete bounded formation, same class, frozen members, exact Key in every member's Authorized Capability Set, stable Exclusions | Missing Exclusion, out-of-set Key, and substituted member rejection | <= 400 |
| C31 | `feat(core): apply typed turn results` | Plan Rejection and synchronized Receipt acceptance only | Duplicate/late/unknown/stale results never reapply state | <= 400 |
| C32 | `feat(core): commit cost profile updates` | Prevalidate Receipt/profile Generation; Commit Barrier mirrors identity, invalidates plans/dirty candidates, and routes stale pre-Admission descriptions to C10 | Cross-model refresh plus no interleaving; mismatch/no-change/stale/duplicate/malformed/commit-failure; fail-stop/counts | <= 400 |
| C33 | `feat(core): publish committed staged output` | Ordered publication Effect after Receipt commit; visible Output Sequence advances once | Earlier cancel discards; later cancel cannot retract; failed enqueue disconnects | <= 400 |
| C34 | `test(core): replay bounded dual-model prefill` | One Prefill Chunk per fresh Plan with Decode interleaving | Byte-identical fixed-seed transitions and operation counts | <= 380 |
| C35 | `feat(replay): add the bounded core replay driver` | Strict replay input/output over `Core::handle` | Golden, malformed, and repeatability cases | <= 400 |
| C36 | `perf(core): measure release scheduler decisions` | Core-only Release measurements outside serialization and IPC | 100 warmups, >=1000 decisions, one sample per decision | <= 360 |
| C37 | `feat(core): gate runtime closure liability` | Build-generated exhaustive Runtime Closure Registry plus Core-owned gate state covering every liability-increasing constructor, checked maximum, and registered nonincreasing closure edge | Every constructor/edge enumerated; Request Acceptance, Core connection creation, Reservation/Residency Demand, and new Operation/Effect reject while closed; closure conservation, read-only status, terminal reopen, Audit-Degraded retention | <= 400 |

P0-1 exits only when Core has no Tokio, Protobuf, SQLite, MLX, I/O, async,
callback, or system-clock dependency and the three Core benchmark lanes can be
driven through a thin adapter without embedding their oracle.

### P0-2: Fake Backend And Device Loop

| ID | Commit subject | Behavior slice | Required verification | Target LOC |
|---|---|---|---|---:|
| E01 | `feat(runtime): define the bounded backend interface` | Owned inputs/results, per-call blocking/safe-point declarations, signal view, and affinity token for every coarse operation | Bound completeness and illegal-call-order rejection | <= 400 |
| E02 | `feat(fake): initialize backend identity` | Preverified Bootstrap Manifest plus deterministic Adapter/MLX/interface/complete Capability including Generation Semantics identities and bounded Resource Signal/Operation Sets | Pre-call watchdog, trusted expected Capability, returned-descriptor equality, semantic identity/field/unit/quality/default/operation-bound drift | <= 380 |
| E03 | `feat(runtime): collect certification environment` | Daemon platform probe joined with build-verified Generation Semantics and exact Bootstrap/Capability/Resource Signal/externally verified Operation Bound identities | Freshness, unavailable facts, semantic/descriptor mismatch, refresh, exact fingerprint/cache-key change | <= 400 |
| E04 | `feat(fake): describe model registrations` | Deterministic hash-bound initial and post-load Model Descriptor generation with nonzero `u32` vocabulary size | Registered field/identity equality, current Generation, Manifest/capability drift makes Revision Unavailable | <= 360 |
| E05 | `feat(fake): describe and materialize requests` | Generation-bound stateless composite/Requirement Set followed by post-Reservation Materialization Result with complete/partial/zero ownership and exact Request Backend Allocation Budget partition | Generation binding, Requirement closure, Budget isolation, rollback/no-call/counters; Core owns refresh | <= 400 |
| E06 | `feat(fake): release request state` | Ownership-gated result release with exact Request Backend Allocation Budget partition | Never-started no-call, no output/transient/timing authority, exactly once, terminal charge, conservation, untrustworthy cleanup fail-stop | <= 380 |
| E07 | `feat(fake): form costed candidates` | Exact authorized Key, versioned Cost Profile, complete candidates, typed Exclusions, ordinary estimates | Per-member Set membership, profile identity, and fixed certified bounds | <= 400 |
| E08 | `feat(fake): execute scripted turns` | Scripted Plan Rejection and synchronized progress/output/member outcomes | Frozen membership and one-chunk Prefill fixtures | <= 380 |
| E09 | `feat(fake): compare and set cost profiles` | Receipt/profile-generation CAS returning rejected, unchanged, or applied old/new identities under Commit Barrier | No interleaving, pre-mutation mismatch, mandatory mirror-commit/fail-stop fixtures | <= 380 |
| E10 | `feat(fake): model residency and resource samples` | Scripted typed load/unload/cache-reclaim Results plus Signal Contract-bound Backend allocator/cache evidence | Success ownership/result, zero/partial/cancel rollback, unload ownership, unchanged reclaim, contract/provenance; no G09 dependency | <= 400 |
| E11 | `feat(fake): execute exclusive operations` | Resource-bounded operation and certified periodic safety points | Lease/Critical stop and rollback fixtures | <= 380 |
| E12 | `feat(runtime): run one owner-thread executor` | Same thread creates/calls Backend state and deallocates only a proven empty shell | Cross-thread calls and raw destruction of initialized or indeterminate state reject | <= 400 |
| E13 | `feat(runtime): measure synchronized engine service` | Daemon Monotonic Time brackets the direct `execute_turn` call and classifies its typed result | Plan Rejection charges zero; Receipt charges the exact complete call interval | <= 340 |
| E14 | `feat(runtime): measure synchronized residency service` | Daemon time brackets every load/unload/cache-reclaim transition and creates an independent Residency Receipt | Variant/result identity, elapsed bound, ownership/split, and watchdog cases | <= 380 |
| E15 | `feat(runtime): drive effects through the event loop` | Sequenced `Core -> Effect -> EffectResult`; fresh scheduling after every Receipt or Plan Rejection | No inline continuation and stable Effect order | <= 400 |
| E16 | `feat(runtime): bound turn-path runtime overhead` | Receipt branch pre/post spans plus Plan Rejection call/replanning spans over the working Event Loop against exact evidence-bound limits | Both branches have no gap/overlap; drift invalidates; no widening/Ledger charge | <= 400 |
| E17 | `feat(runtime): budget owner-thread support calls` | Evidence-bound per-call blocking/safe-point plus call-count/aggregate Runtime Overhead budget and pre-reserved mandatory capacity | Every readiness operation classified; interference fits Deadline Cost; nonessential defers; no free queue | <= 400 |
| E18 | `feat(runtime): share cooperative control signals` | External atomic setters and Backend safe-point read view | Cancel/Critical/lease/shutdown before and after safe point | <= 360 |
| E19 | `feat(runtime): fail stop an indeterminate backend operation` | Watchdog escalation, no fabricated Result/Receipt, whole-process termination hook | Missed safe point or post-profile-mutation commit failure cannot restart only the owner thread | <= 400 |
| E20 | `test(runtime): replay executor failure boundaries` | Duplicate/stale result, profile divergence, forced stop, and deterministic Fake crash traces | Stable final hashes and no operation replay or continued divergent Backend | <= 380 |
| E21 | `feat(fake): shut down backend state` | Final exactly-once owner-thread operation returning zero-ownership Shutdown Result and empty-handle typestate | Readiness false, no in-flight/release ownership, destroy order, bound/safe point, no retry/fabrication/bypass/Clean Shutdown Boundary authority | <= 380 |
| E22 | `test(runtime): enforce backend conformance fixtures` | Shared Fake runner for Bootstrap/initialization/Capability/Generation Semantics/Signal/Operation descriptors, post-load Model Descriptor equality and Request Description refresh, support bounds, ownership, CAS, transitions, release, and shutdown | Trusted semantic identity and other identities, call order, no-call, release-before-unload, empty-handle-only raw deallocation, support budget, result-gated Clean Shutdown Boundary/fail-stop | <= 400 |

### P0-3: Resource Governor And Residency

| ID | Commit subject | Behavior slice | Required verification | Target LOC |
|---|---|---|---|---:|
| G01 | `feat(resources): sequence backend resource samples` | Signal Contract-bound allocator/cache sample provenance and freshness | Missing, duplicate, stale, and contract-identity mismatch cases | <= 340 |
| G02 | `feat(resources): sample macos process and vm state` | `phys_footprint`, available memory, swap, and compressor via native APIs | Typed unavailable/overflow and no-shell assertion | <= 400 |
| G03 | `feat(resources): observe macos memory pressure` | Bounded dispatch pressure source and monotonic sequence | Normal/warning/critical transitions and teardown | <= 360 |
| G04 | `feat(resources): assemble complete resource evidence` | Join Backend, process, VM, and pressure sources without erasing provenance | Independent freshness and out-of-order rejection | <= 400 |
| G05 | `feat(governor): classify resource modes` | Normal, Guarded, StopAdmission, Critical, hysteretic recovery | Immediate restriction and dwell-bound recovery | <= 400 |
| G06 | `feat(governor): activate resource configuration` | Atomic Threshold Profile, eviction rank, wait, and residency-limit propagation from the complete successor | Stricter profile advances Safety Generation; looser recovery obeys hysteresis | <= 400 |
| G07 | `feat(governor): reserve request capacity` | Typed Request Backend Allocation Budget plus separately typed daemon output and transient-headroom components in the admitted capacity ledger | Cross-component authority, conservation, overflow, rollback | <= 400 |
| G08 | `feat(governor): reserve residency before load` | Separate Model Descriptor-based Residency Reservation committed before Effect | No load without reservation; cancel-before-start makes no Backend call and rolls back | <= 380 |
| G09 | `feat(runtime): coordinate residency demands` | Shared loads/waiters/FIFO/timeout/rollback and successful-load Generation advance invoking C10 refresh | Resident cross-model reissue, unrelated Warming defer, current loaded waiters before Admission, drift scopes Unavailable | <= 400 |
| G10 | `feat(governor): gate shared residency transitions` | Conservative blocking bound must fit every current timing budget or wait for explicit Exclusive Mode | No automatic Exclusive entry and no blocking shared transition | <= 400 |
| G11 | `feat(governor): bound residency activity` | Configured transition frequency and aggregate Residency Service occupancy | Window rollover, saturation, and checked arithmetic | <= 380 |
| G12 | `feat(runtime): protect resident model leases` | Lease acquisition/release and unload precondition over zero request ownership/pending release | Active lease or any request ownership prevents unload | <= 360 |
| G13 | `feat(governor): select ordinary reclaim actions` | Bounded cache-reclaim Residency Transition then deterministic idle-model victims independent of Model Weight | Transition gating, no immediate free-capacity claim, stable tie, and protected-victim cases | <= 380 |
| G14 | `feat(governor): perform critical eviction` | Eviction Rank/ties, terminally fail affected work, consume every request ownership, then unload | Cache/idle exhaustion, release-before-unload, release fail-stop, no forced in-flight interruption | <= 400 |
| G15 | `feat(governor): retain only allocated reclaim charges` | Accepted request results partition only the exact Request Backend Allocation Budget; rolled-back load or successful unload results partition Residency Reservation; actual allocation stays charged to convergence | Cross-component authority, zero/partial materialization/load rollback, use/unload/cache, stall, no premature reuse | <= 400 |
| G16 | `feat(governor): decide the process reclaim barrier` | Require typed old-process lifetime proof plus fresh post-reclaim baseline | Fake proof contract; elapsed time/socket loss never establishes capacity | <= 360 |
| G17 | `test(governor): replay pressure and residency lifecycle` | Load failure/cancel/rollback, unload/reload, shared/Exclusive gating, Critical terminal-release-unload, reclaim, stale evidence | Governor rules, ownership order, allocation conservation, hot-path counts | <= 400 |

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
| N01 | `build(native): add the versioned C shim` | Pinned MLX, locked Generation Semantics descriptor/hash, embedded Manifest, CMake/Ninja, opaque typestate handles, identity/error translation, bounded nonthrowing empty-shell deallocator | Locked descriptor/Manifest equality, compile/SHA/ABI; live/indeterminate rejects; shell has no MLX object and only rolled-back-init/post-Shutdown deallocates | <= 400 |
| N02 | `feat(native): initialize on the executor owner thread` | Pre-call Manifest watchdog, MLX/stream creation, exact Adapter/MLX/interface/complete Capability including Generation Semantics identities and Resource Signal/Operation Sets | Semantic/other drift fail-stop; only trustworthy zero-root failure deallocates; timeout, missed rollback, or success-with-drift never destroys live state | <= 400 |
| N03 | `build(native): pin the graph export toolchain` | Ignored task Python, exact mlx-lm/MLX/dependency lock, model fetch/hash wrapper | Missing/drifted tool/model and offline replay manifest | <= 380 |
| N04 | `build(native): define the exported graph abi` | Canonical exact-shape signatures, logits/new-KV outputs, artifact manifest and import contract | Double-export byte identity and Python round-trip harness | <= 400 |
| N05 | `feat(native): implement the graph importer` | Bounded C++ Direct importer for verified Graph ABI signatures and complete manifest | No production import outside Residency Effect; deterministic tiny-fixture parity and drift rejection | <= 400 |
| N06 | `build(native): export qwen3 dense graphs` | Complete Qwen3-0.6B graph recipe for certified Decode/Prefill buckets | Exact revision, two exports, Python/C++ logits/KV parity, no tracked artifact | <= 400 |
| N07 | `build(native): export qwen15 moe graphs` | Complete Qwen1.5-MoE graph recipe including top-k routing for certified buckets | Exact revision, two exports, Python/C++ logits/KV parity, no tracked artifact | <= 400 |
| N08 | `feat(native): verify registered model artifacts` | Canonical Artifact Root and typed File Hash checks before and after load | Ad hoc path, mutation, and Revision Unavailable cases | <= 400 |
| N09 | `feat(native): own logical model runtime capsules` | Per-Model imported graph/KV/cache capsules on the single owner thread | Isolation and lifecycle tests | <= 380 |
| N10 | `feat(native): describe model registrations` | Generate versioned conservative Model Descriptor including nonzero `u32` vocabulary size from exact registered graph manifest/capability before registration and after load | Nonresident operation, field/identity/hash post-load equality, drift marks Revision Unavailable | <= 400 |
| N11 | `feat(native): expose cooperative signal views` | Read-only atomic signal view and declared safe-point helper before any cancellable operation | Cross-thread setter ordering and stale-view rejection | <= 360 |
| N12 | `feat(native): transition model residency` | Serialized load with strong rollback, ownership-gated unload, and bounded allocator-cache reclaim using typed results | Success identity/ownership; zero/partial/cancel rollback split, release-before-unload, unchanged reclaim, fail-stop | <= 400 |
| N13 | `feat(native): describe token requests` | Stateless validation of frozen Token Request plus Model Descriptor/current Backend Generation and finite Capability Requirement Set generation | Initial/post-load generation binding and complete phase/batch/Shape matrix; no Core refresh, Envelope, registry, or allocation | <= 400 |
| N14 | `feat(native): materialize admitted requests` | Resident-model handle/KV/Sampling State with synchronized complete/partial/zero-ownership exact Request Backend Allocation Budget result | Identity/conservation; no daemon-capacity authority; zero-ownership rollback; partial failure requires release | <= 400 |
| N15 | `feat(native): release request state` | Owner-thread destruction with trustworthy exact Request Backend Allocation Budget split only when opaque ownership exists | Unowned no-call; no output/transient/timing authority; terminal ownership retains Budget; uncertain cleanup fail-stops and blocks unload | <= 400 |
| N16 | `feat(native): form costed native candidates` | Canonical Batch/Shape compatibility, exact authorized Key, Cost Profile version/estimate, typed Exclusions | Every member Set contains Key; Fake/native parity | <= 400 |
| N17 | `feat(native): compare and set cost profiles` | Accepted Receipt-bound version returns mismatch, unchanged, or atomically applied Backend-owned profile under Commit Barrier | Core mirrors identity once; no interleaving; malformed/post-mutation failure fail-stops; no widening | <= 400 |
| N18 | `feat(native): sample per request deterministically` | Implement the exact Generation Semantics descriptor's binary32 tensor flow, compact nonempty Top P/Top K support, max-shifted Temperature scaling, greedy, `key(seed) -> split(state)` transitions, and a qualification-only tensor seam outside the production Backend Interface | NaN, both infinities, and finite-to-nonfinite-cast fail before split/state/output; identity-bound shared subnormal/no-crossing/equal-logit/survivor-below-K/smallest-Temperature/zero-uniform compact-index and multi-token vectors; hidden-stop and Dense/MoE B1/B4 invariance | <= 400 |
| N19 | `feat(native): apply stop and output limits` | Longest ambiguous stop suffix retention, hidden matched tokens, exact Max Output terminal | Prefix overlap, cross-Turn match, cancellation, and limit cases | <= 400 |
| N20 | `feat(native): execute synchronized decode turns` | Bounded Decode on resident imported graph with staged visible token output | Dense/MoE output parity, signals, and cleanup | <= 400 |
| N21 | `feat(native): execute one prefill chunk` | One imported-graph range within Plan target/ceilings, synchronized continuation | Dense/MoE parity, signals, and no hidden loop | <= 400 |
| N22 | `feat(native): execute exclusive operations` | Conservative resource-bound operation with certified periodic safety points | Lease/Critical cancellation and rollback | <= 400 |
| N23 | `feat(native): report backend allocator evidence` | Support-budgeted MLX active/cache samples bound to Signal Contract identity with quality and source sequence | Blocking/safe-point bound, contract mismatch, telemetry availability, error mapping | <= 360 |
| N24 | `test(native): expose qualification-only numerical hashes` | Test-build output/logits/KV hashes outside serving Interface | Prove no qualification DTO in Core or production ABI | <= 360 |
| N25 | `feat(native): shut down backend state` | Final exactly-once owner-thread destruction, validated zero-ownership Result, then empty-shell deallocation | Preconditions/order/safe point; live raw destroy rejects; failure has no retry, deallocation, Clean Shutdown Boundary, or reclaim claim | <= 400 |
| N26 | `test(native): pass full backend conformance` | Fake/native Bootstrap/Capability/Signal/Operation descriptors, post-load Generation refresh, authorization, support bounds, ownership, profile CAS, transitions, Exclusive, shutdown | Trusted identities/fixtures; raw bypass rejects; unavailable evidence never becomes synthetic pass or Clean Shutdown Boundary | <= 400 |

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
| P01 | `feat(policy): load the bounded installation policy` | Parse/identify fixed Data, Control, live-maintenance, and offline-maintenance UID/GID allowlists plus an opaque strongly typed File Hash naming the exact P0 Runtime Authority Volume qualification record; record decoding remains U01/S22-owned | Canonical identity, explicit record presence, malformed/oversize, and independent-list tests; no storage read or `latest` selector | <= 400 |
| P02 | `feat(daemon): authenticate data-plane peers` | Permission-restricted Unix socket plus macOS `LOCAL_PEERCRED` check | Filesystem-only and unlisted-root rejection | <= 380 |
| P03 | `feat(daemon): authenticate control-plane peers` | Stricter socket allowlist and connection-scoped Maintenance Capability | Disconnect/session expiry and no protocol-only grant | <= 400 |
| P04 | `build(protocol): generate canonical protocol artifacts` | Pinned deterministic descriptor/registry/support-manifest tooling | Double generation and registry zero/reuse tests | <= 400 |
| P05 | `build(protocol): lock data-plane v1.0` | Token IDs, selector, required Service Class, closed presence-tracked Generation Parameters, optional `fixed64` Sampling Seed, bounded stop sequences, Max Output, submit/query/subscribe/cancel/status/output | Descriptor hash; enum numbers; omitted/zero/nonzero Seed; subnormal positive versus negative-zero/nonfinite floats; greedy/categorical golden frames and limits | <= 400 |
| P06 | `build(protocol): lock control-plane v1.0` | P0 initialize with complete global Configuration/no IDs, initialization-specific indeterminate response without mutation token, plus model/config/certification/Exclusive/management/status, sequenced fence-busy terminal, pre-barrier cancellation, Event Loop pre-Core `cancel_window_closed` and opaque `commit_in_progress` Direct Responses, Control Plane mutation-delivery disconnect, pre-ID range-barrier fatal status, and Audit-Degraded pending Operation ID/stage/remaining-headroom status | Independent descriptor, empty scoped sets, exact versus nonmatching cancel-window frames, identity nondisclosure/no-Audit classification, Data Plane disconnect separation, limits, golden frames | <= 400 |
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
| P24 | `feat(daemon): reject closed mutation cancellation` | Ordered Control Plane pre-Core handler over the active `{Daemon Instance ID, Operation ID, checked barrier generation}` gate after authorization/validation and cancellation-ingress plus Direct Response reservation; exact tuple returns `cancel_window_closed`, every other tuple returns opaque `commit_in_progress` | Same-socket ordering, ABA/unknown/different identity flood during barrier and post-C19 Audit Degraded, identity nondisclosure, both responses no-Core/no-Audit/no-headroom, write/disconnect reservation release, no retained/replayed command, Data Plane disconnect separation | <= 380 |

### P0-6: Qualified Durable P0 Control And Audit

The Audit payload schema is a private durable-format contract, not either public
socket protocol. It has its own append-only registry, canonical descriptor lock,
and deterministic verification command; a public protocol minor cannot alter an
epoch's fixed Audit Registry Identity.

| ID | Commit subject | Behavior slice | Required verification | Target LOC |
|---|---|---|---|---:|
| U01 | `build(storage): lock the p0 capability formats` | Storage Capability Profile plus bounded immutable Qualification Record and two-slot Head codecs/identities shared with later Integrity work | Double generation, golden records/slots, unknown format, wrong profile/volume/build/OS, torn/equal-disagreeing slots | <= 400 |
| U02 | `feat(storage): probe p0 authority volumes` | Offline qualifier under the installation qualification lock, and with no Runtime lock, runs the exact build-owned syscall/durability profile and writes one synchronized immutable candidate record | Real temporary-volume probes, fixed EINTR deadline, failure before selection, no daemon write probe or reverse lock acquisition | <= 400 |
| U03 | `feat(storage): publish p0 qualification records` | Validate lineage predecessor, publish U02 candidate through the fixed Head barrier, and return its exact identity for explicit Installation Policy use | Record/Head crash points, one-slot healing, orphan non-adoption, lineage disagreement, explicit identity verification, no `latest` API | <= 400 |
| S01 | `build(store): add the bounded sqlite executor` | One connection/executor, FULL durability profile, checked bindings, explicit database-parent sync, and typed barrier errors | Open/configuration/commit/parent failure tests over temporary non-Runtime stores | <= 360 |
| S02 | `feat(daemon): latch storage barrier failures` | Session-wide first-failure observation, write guard, Device shutdown signal, read-only mode, and no retry/marker/Clean path before any Runtime writer is wired | First/duplicate failure, every later write denied before syscall, status-only custody | <= 400 |
| S03 | `feat(store): create the versioned control schema` | Runtime/history identity, generations, current-state pointer, bounded tables through the shared write guard | Fresh/open/unknown schema and latched-guard cases | <= 400 |
| S04 | `feat(store): encode immutable model registry rows` | Transaction-scoped Manifests, complete canonical registered Model Descriptor bytes, Aliases, lifecycle, canonical roots, and typed hashes with no commit/pointer API | Descriptor bytes/vocabulary/hash and candidate round-trip, registry limit, corruption rejection; caller rollback leaves live Store unchanged | <= 400 |
| S05 | `feat(store): encode configuration snapshot rows` | Transaction-scoped complete validated Configuration successor with no commit/pointer API | Semantic/generation hashes and caller rollback; current generation cannot change | <= 380 |
| S06 | `feat(store): encode certification record rows` | Transaction-scoped immutable record/evidence identities and finite Coverage Manifest with no activation API | Candidate round-trip, invalid reference rejection, current generation unchanged | <= 400 |
| S07 | `feat(certification): compile the authorization index` | Offline verified Coverage Manifest to read-only exact-key index | Missing reference, drift, wildcard, and dominance-proof cases | <= 400 |
| S08 | `feat(certification): prepare certification successors` | Validate candidate record/index replacement, explicit recertification evidence, and bounded Profile Revalidation plan without persistence or activation | Quarantine remains until unified mutation activation; changed bounds identify exact descriptions/candidates | <= 400 |
| S09 | `feat(daemon): hold the instance lock` | One installation-fixed absolute descriptor identity outside every Runtime, acquired before Runtime-specific locks/writers and held from Bootstrap until OS process termination with no explicit unlock path | Same- and cross-Runtime-ID competing processes remain blocked through the last live instruction and latched read-only custody | <= 360 |
| S10 | `build(audit): lock the p0 audit schema` | Bounded payload `.proto`, append-only nonzero record-kind registry, canonical descriptor lock, Audit Registry Identity | Double generation, golden payloads, unknown/reserved/reused kind rejection | <= 400 |
| S11 | `feat(audit): compute sequence reserves` | Build-time checked Terminal Sequence Reserve plus P0 Control Mutation Sequence Headroom consuming C37's exhaustive registered maxima, Effect creation, attempt x noncreating discretionary-interleaving limits, monotonic per-object mandatory safety edges with duplicate-signal coalescing, and the larger failure/success Result branch | C37 registry projection is complete; binary maxima, no aggregation/borrowing, repeated raw signals assign no event, Runtime Closure Gate prevents downstream fanout, checked overflow, exact edge values, Sequence Exhausted | <= 400 |
| S12 | `feat(audit): encode bounded chained records` | Framing, sequence, registry identity, checksum/hash link, content exclusion | Golden, oversize, corruption | <= 380 |
| S13 | `feat(audit): run one bounded audit writer` | Ordered guarded ordinary queue plus build-derived nonborrowable Audit Safety Reserve count/bytes aligned with Core Event/Terminal Sequence maxima; strong file/namespace barriers; pre-barrier failures are Audit Degraded and required-barrier failures feed S02 | Ordinary saturation cannot consume safety capacity; count/byte maximum edge, append, file/parent sync, classification, latch, and no Event Loop direct I/O | <= 400 |
| S14 | `feat(audit): fence durable predecessors` | Typed sync-through operation drains all assigned records and returns exactly granted, stale, audit-degraded, or storage-barrier-failure; only granted carries a session-local Predecessor Fence | Assigned-unsynced drain, stale target/head/generation/intervening work, pre-sync degradation, actual file/parent sync failure | <= 380 |
| S15 | `feat(store): reserve p0 audit sequence ranges` | Sole post-genesis writer of SQLite P0 Audit Sequence State; daemon-owned unsequenced/no-ID/no-Effect/no-Audit primitive whose guarded `FULL` commit plus database-parent sync publishes high-water and can prove full S11 mutation headroom before assignment on the single Store executor | Exact headroom/range edge; known absence versus indeterminate latch; absent/exact/third-state custody at commit and parent crash points; no unchanged claim, pending envelope, mutation high-water update, terminal borrowing/reuse, or Anchor bytes | <= 400 |
| S16 | `feat(store): initialize p0 control and audit` | While holding installation-scoped S09, acquire the qualification lock, require the Policy-selected U03 record to be Head-selected, and hold that lock through commit-plus-parent publication of its immutable binding with daemon IDs, empty registries, version-one State, genesis sequence state, pending sequence-one Epoch Open, and exact Audit root | Lock order has no reverse path; wrong/stale/orphan record rejects before identity; crash leaves absent or one exact non-ready identity/pending custody; required-barrier failure returns `initialization_outcome_indeterminate` without a mutation token | <= 400 |
| S17 | `feat(store): validate p0 control mutation intents` | Bounded complete successor validation/token check while predecessor remains active, producing no Operation ID or owner | Stale/duplicate/busy/invalid successor rejects without ID; no external publication or daemon-only owner | <= 380 |
| S18 | `feat(store): fence p0 control mutations` | Prove full S11 headroom, revalidate token/next/headroom, then atomically allocate daemon Operation ID plus Core Effect/owner/headroom charge and close C37's Runtime Closure Gate until terminal; enforce registered nonincreasing interleaving and state-edge-deduplicated safety; pre-barrier cancel/exhaustion consumes one terminal Result, Audit Degraded retains stage/charge without a Core Event, granted protects Store-result/conditional-completion slots, Commit Barrier publishes P24's exact checked-generation Cancel Gate for all Control Mutation cancellation, and C19 stops discretionary assignment | Range edge/Sequence Exhausted/drift has no owner; assignments conserve charge; C37 constructor/closure integration, raw-safety coalescing, pre-entry cancel/exhaustion closure, gate publication-before-I/O and persistence through C19/degraded/failure, exact/nonmatching cancel floods conserve headroom and Audit reserve, terminal clear, fail-stop no Result, degraded resume unsequenced, and no post-C19 ordinary work/terminal borrowing | <= 400 |
| S19 | `feat(store): publish p0 control mutations` | Inside S18, use the sole guarded post-genesis Control transaction for candidate/pointer/pending envelope without sequence-state change; trustworthy exact absence returns the pre-sequenced nonpublication Result, exact committed success drives C19 activation/owner advance, and indeterminate returns no Result | Absent closes without activation/dependent Effect; committed success cannot release owner; no helper authority, unsequenced callback, overtaking, early activation, fabricated result, or unstable third-state classification | <= 400 |
| S20 | `feat(store): complete p0 mutation audit` | With the Runtime Closure and Control Mutation Cancel Gates closed and discretionary dispatch deferred, execute C19's dependent exact-envelope completion Effect: append/sync/verify fenced bytes, guarded `FULL` pending-clear commit plus parent sync, then return one ordinary sequenced Result whose accepted Core transition alone acknowledges success, releases owner/headroom, and reopens both gates; append failure retains Effect/stage/charge and the originating Control Plane disconnect ends only delivery | Protected final slot under maximum safety edges, same-ID exact-Effect resume, exact and nonmatching mutation cancels remain bounded no-Core/no-Audit/no-headroom Direct Responses, Data Plane disconnect still cancels owned requests, no rollback, Store replay, or unsequenced callback; only actual durability failures indeterminate; no regeneration/early release | <= 400 |
| S21 | `feat(audit): retain bounded p0 segments` | Segment boundaries, synchronized retention boundary, garbage eligibility | Capacity, file/parent barrier, latch, and reclaim failures | <= 380 |
| S22 | `feat(daemon): order policy-first bootstrap` | Installation Policy before socket, lock, or Runtime authority; construct S02, acquire installation-scoped S09 before every Runtime-specific lock/writer, then freeze and validate the exact Policy-selected U03 record against the live volume/build/Profile before opening a Runtime writer | Syscall/byte ordering across fresh/replacement Runtime IDs, missing/foreign record, no implicit Head selection, and no pre-lock/pre-guard/pre-qualification Runtime writer | <= 400 |
| S23 | `feat(daemon): classify interrupted p0 publications` | Under the frozen record proof, classify initialization, range, mutation, and pending-clear separately; repeat only missing parent sync; range has no pending envelope and no path recovers a Fence | Epoch-Genesis init, no-envelope range, persisted-predecessor mutation, separate pending-clear fixtures; binding mismatch/third state fail closed; no regeneration or reuse | <= 400 |
| S24 | `feat(daemon): reconcile pending p0 audit` | After S23, complete initialization only against Epoch Genesis Hash; append mutation only at its persisted predecessor or verify its exact in-chain record; then clear before tail | No session-local Fence input; already-present exact record, predecessor mismatch, every Audit/clear crash point, malformed/multiple pending reject | <= 400 |
| S25 | `feat(audit): reconcile the p0 journal tail` | Only with no pending envelope, verify Clean or append one Crash Tail closing every never-assigned and assigned-but-unsynchronized-lost value through old high-water | Both lost classes, pending refusal, truncation/mismatch/sequence exhaustion, no reconstruction/reuse/Anchor fallback | <= 400 |
| S26 | `test(daemon): integrate storage barrier custody` | Exercise S02 against every real SQLite, Audit/file, parent-sync, Bootstrap, retention, and shutdown barrier while retaining the OS-only lock | Device/write stop, first observation stable, authenticated status only, no marker/Clean/retry, next process alone reconciles | <= 400 |
| S27 | `feat(daemon): restart with empty inference state` | Rebuild Control State including complete registered Model Descriptors only after S23-S25; rehash before readiness; no request/KV/Residency restoration or descriptor regeneration | Initialization/mutation three-boundary and clean restart; missing/mismatched descriptor enters Control Repair Mode; post-restart Top K uses restored vocabulary | <= 360 |
| S28 | `feat(daemon): enforce the process reclaim barrier` | After any predecessor initialized Backend, acquisition of the unchanged installation-scoped lock proves that process exited for graceful same-ID restart, Daemon Failure, and cross-Runtime replacement; sampler then establishes a fresh post-exit baseline | Real clean same-ID, failed same-ID, and cross-Runtime-ID parent/child lifetime tests; Shutdown Result/Clean Boundary and lock-before-exit samples reject; no acquire-before-exit, new path/generation, Fake/time/socket substitute | <= 400 |
| S29 | `feat(daemon): gate p0 service readiness` | Require the installation-scoped lock, exact frozen/bound Storage Qualification Record equality, Store/registry schema, reconciled prior bytes/Audit, Device Executor, Adapter, environment, and the Process Reclaim Barrier after every predecessor Backend initialization; current-session latch remains non-ready | Every successor may become ready only after exact forward reconciliation and required post-exit/acquisition-of-the-same-lock Resource Evidence; true first genesis is explicit; clean same-ID, failed same-ID, and cross-Runtime cases reject new lock path, pre-lock sample, logical shutdown proof, and binding drift | <= 400 |
| S30 | `feat(daemon): perform graceful shutdown` | Reject work, cancel/release live work, accept one validated zero-ownership Result plus confirmed shell deallocation, sync Clean Shutdown Boundary, then terminate with lock | Boundary only after Result/deallocation; failure/no-result/safe-point/barrier has no retry, boundary, or `CLEAN_RELEASED` claim | <= 400 |
| S31 | `test(store): expose bounded fault custody` | Real relative files, phase markers, initialization/mutation/restart/shutdown inspection, no Effect replay | Assigned-unsynced predecessor then mutation crash, atomic publish/corrupt/truncate/SIGTERM, assigned-lost tail, all barrier-failure custody | <= 400 |

P0 durability never substitutes Runtime Metadata or Integrity-profile repair.
Unreconcilable corruption is non-ready and requires a new runtime.

### P0 Core Gate

The Core Gate has an explicit owner after P0-6 and before qualification. Each
commit extends one deterministic matrix; K05 runs the aggregate gate and emits a
machine-readable result without claiming MLX correctness or performance.

| ID | Commit subject | Behavior slice | Required verification | Target LOC |
|---|---|---|---|---:|
| K01 | `test(gate): cover scheduling and admission properties` | Weighted service/config changes, finite authorization, exact environment, cross-model load/profile Generation refresh, Exclusive | No stale-result rejection or Admission; bounded stable reissue, examples/properties/fixed seeds | <= 400 |
| K02 | `test(gate): generate lifecycle and residency sequences` | Never-started no-call, zero/partial materialization, owned release, release-before-unload, load re-description, Pending Reclaim, Critical eviction | Generated state machine, current description identity, allocation conservation, shrinkable replay | <= 400 |
| K03 | `test(gate): inject executor and audit faults` | Profile post-mutation commit failure, P0 initialization-specific indeterminate response, qualification/binding, unsequenced range reservation/barrier failure, headroom edges, ordinary Audit saturation plus safety charge/byte edge, exhaustive Runtime Closure Registry constructors/closures, max Fence interleaving, repeated raw safety signals, four Fence outcomes, exhaustion/pre-entry cancel/exact-absent closure, authenticated ordered Control Mutation Cancel Gate exact/nonmatching flood and disconnect, degraded same-Effect continuation, two-stage success, dispatch deferral, clean same-ID and cross-Runtime process reclaim, typed recovery, shutdown and storage faults | Range reserve has no ID/Event/Effect/record/envelope; every liability-increasing constructor rejects and registered nonincreasing closure conserves charge; raw repeats coalesce; exact closed-window cancel returns `cancel_window_closed`, unknown/different returns opaque `commit_in_progress`, both release capacity on write/Control disconnect without Core/Audit/headroom change, Data disconnect remains audited request cancellation; every successor baseline follows same-lock acquisition; no starvation/borrowing, leaked charge/owner, fabricated Result/token, replay, recovered Fence, publication gap, lock-path bypass, unchanged claim, or early Clean | <= 400 |
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
| Q04 | `feat(benchmark): expose scheduler performance qualification` | `scheduler-performance` adapter, Release samples, Runtime Overhead and Owner-Thread Support Budget versions | IPC/serialization excluded; support/overhead drift and Ledger exclusion declared | <= 360 |
| Q05 | `feat(benchmark): expose request lifecycle qualification` | `request-serving-lifecycle` through production Data Plane including parameter rejection and cancel/output/release races | Locked production descriptor, Service Class mapping, and real process identity | <= 400 |
| Q06 | `feat(benchmark): expose native correctness qualification` | `mlx-native-correctness`, reproducible full Dense/MoE graph manifests, B1/B4 omitted/zero/nonzero Seeds, complete greedy/categorical extreme-parameter and stop/Max Output matrix, external hashes | Shared NaN/infinity/finite-cast-overflow fail-before-split/state/output plus exact-tensor/subnormal/no-crossing/survivor-below-K/smallest-Temperature/zero-uniform compact-index/key-state vectors, omitted-seed replay, Python/export/import parity, batch invariance | <= 400 |
| Q07 | `feat(benchmark): expose bounded turn qualification` | `bounded-turn-and-ffi` over production native boundary | One-chunk Prefill, Exclusive Safety Point, signal cleanup | <= 380 |
| Q08 | `feat(benchmark): expose governor qualification` | `residency-and-memory-governor` with real system samples | Reservation, Residency Service limits, Critical eviction, reclaim | <= 400 |
| Q09 | `feat(benchmark): expose cross-model qualification` | `cross-model-serving` through production Data Plane | Timing, fairness, progress, throughput, output evidence | <= 400 |
| Q10 | `feat(benchmark): expose observability qualification` | `observability-qualification` with honest P-1A quality | No command-buffer fairness overclaim | <= 360 |
| Q11 | `feat(benchmark): expose persistence qualification` | P0 exact qualification binding, initialization-specific indeterminate response, Control/Audit init, unsequenced range reservation, separate sequence/current-pointer authorities, build-derived headroom/Audit Safety Reserve, exhaustive Runtime Closure Gate, four-way Fence bounds, exhaustion, exact-absent closure, pre-barrier cancellation and post-barrier exact/nonmatching Control Mutation Cancel Gate flood/disconnect, degraded continuation, two-stage success with post-C19 dispatch deferral, installation-lock clean/failure/replacement restart, typed recovery, pending-before-tail, Storage Failure, empty restart | Range absence versus indeterminate no-ID latch, headroom/safety conservation and signal coalescing, every closure-liability constructor/edge, owner/Result closure, same-Effect resume, authenticated ordered `cancel_window_closed`/opaque `commit_in_progress` no-Core/no-Audit/no-headroom responses with capacity release and Data/Control disconnect separation, unchanged lock plus fresh post-lock baseline for every successor, no Store replay/range envelope/recovered Fence/publication gap, assigned-lost closure/readiness | <= 400 |
| Q12 | `feat(benchmark): expose same-process failure qualification` | Architecture-compatible protocol/owner/watchdog/fail-stop lane | No Backend process or private IPC | <= 380 |
| Q13 | `feat(benchmark): expose certification qualification` | Production exact environment and Generation Semantics applicability/cache invalidation, quarantine, revalidation, recertification | Schema/algorithm identity drift through thin adapter over real probe, index, and Core | <= 400 |
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
