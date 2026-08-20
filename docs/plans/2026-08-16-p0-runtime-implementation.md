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

Six documentation corrections precede implementation. ADR 0032 is updated to
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
Backend Operation Bound Set, a canonical Runtime Overhead Catalog with
Lifecycle Overhead Qualification, a bounded lifecycle-classified Owner-Thread
Support Budget, Sequenced Event Interference Bound with a fixed scheduling cut,
and companion intrinsic Stale Plan Disposition Bound,
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

Registration durably commits only C10c-verified complete canonical Model
Descriptor frames and sealed identity/hash/vocabulary beside the Model Manifest
in the Control State Snapshot; the Manifest's expected typed descriptor hash is
only a comparison input. The exact V1 payload retains the build-bound capability
and conservative residency resource/time semantics while `model_descriptor`
treats those bytes as opaque and grants no authority from them. Bootstrap runs
the same verifier before Service Readiness, and missing or mismatched bytes or
fields enter Control Repair Mode rather than regenerating vocabulary authority
from artifacts, logits, model defaults, or a digest. Post-load observation must
pass that verifier and equal the restored C10d-sealed frame, identity, hash, and
vocabulary size.

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
Exhausted or trustworthy exact-absent reservation outcome therefore atomically closes
the unowned Prepared Carry Reservation, resumes ordinary support, and leaves no mutation
Effect or owner. An indeterminate reservation commit or parent barrier latches Storage
Barrier Failure without a mutation ID or Control Outcome Indeterminate response and
makes no unchanged-high-water claim. At a
synchronized boundary the Event Loop atomically revalidates the complete mutation token,
exact C26 carry token, current Support Ledger Generation, current next value, and full
headroom. Any nonfatal drift closes the unowned carry, resumes ordinary support, and
returns a no-ID state conflict. Only then
does the daemon allocate one Operation ID, and Core atomically records the
prepublication Effect, one-in-flight proposal owner, and headroom charge. Owner
creation atomically closes the Core-owned Runtime Closure Gate until the terminal
Result. The build-generated Runtime Closure Registry exhaustively owns every Core
transition that can increase future sequence-and-Audit closure liability, including
Request Acceptance, Core connection-state creation, Reservation or Residency Demand,
and a new independent Operation or Effect. While closed, only a registered transition
that advances or replaces already charged obligations without increasing its
remaining maximum may commit; read-only status that creates no Core object remains
available. C25 owns the registry and Core gate state; S11 consumes its generated
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
cancellation routes the protected prepublication terminal Result through C27, consumes
the exact C26 Prepared Carry Reservation, preserves predecessor Budget and charges,
resumes ordinary support, closes the Effect, owner, and headroom, and reopens the
Runtime Closure Gate; a fail-stop safety outcome fabricates no Result or carry release.
Exhaustion assigns the current next value to one nonpublication failure Effect Result
and uses that same C27 consumer to consume the carry token exactly once, preserve the
predecessor, resume ordinary support, close the Operation ID, proposal owner, and
remaining headroom, and reopen the Runtime Closure Gate before stable typed busy. An
`audit_degraded` outcome is daemon-owned I/O custody: entry, failed supervision,
and recovery assign no Event Sequence or Result. It keeps the Operation ID, owner,
Prepared Carry Reservation and suballocations, headroom, and exact Core stage pending,
then resumes Fence acquisition while
allowing only the already budgeted safety and registered nonincreasing closure transitions.
Those closure transitions must be registered nonincreasing Runtime Closure Gate
transitions.
Audit Safety Reserve separately provides build-derived nonborrowable count-and-byte
capacity for those records, so ordinary queue saturation cannot consume it.

At a synchronized Turn boundary, after the closed Runtime Closure Gate has driven
accepted requests, Turn Plans, Timing Commitments, Future Turn Support Entitlements, request Backend ownership,
pending releases, and output reservations to zero, a pure Core validator produces
a current zero-request-liability witness. The gate, rather than that validator,
makes the witness stable because no permitted transition can recreate liability.
The Event Loop performs final token/fence/witness revalidation and enters the
bounded Control Publication Commit Barrier by assigning
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
the Control Mutation Cancel Gate is closed, every authorized Control Mutation
cancellation completes before Core.
An exact tuple receives typed `cancel_window_closed`; an unknown or different Daemon
Instance ID, Operation ID, or barrier generation receives opaque `commit_in_progress`
without disclosing identity existence. Both release their reservation after write or
that Control Plane connection terminates and create no Core Event, Event Sequence,
Effect Result, Audit Record, Domain Rejection, headroom charge, Store outcome, or
unchanged-state claim. The Event Loop gate remains closed through C27, barrier release,
and exact-envelope completion. S18 clears it only after accepting C27's typed terminal
disposition and releasing the owner; Storage Barrier Failure never clears it. The originating
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
sequenced Store-result slot through C27 as one nonpublication failure Result, consumes
the Prepared Carry Reservation, preserves predecessor Budget and charges, resumes
ordinary support, closes the Effect, owner, and headroom, reopens C25's Runtime Closure
Gate, and returns the typed terminal disposition that lets S18 clear the Event Loop gate,
in the same Event Loop handling step without an intervening dequeue or Core transition,
without activation or a dependent Effect. Trustworthy exact committed success instead
drives C27 with the exact
carry token, atomically activates the complete successor and its same-or-replacement
Budget over the unchanged C18-complete support ledger, advances Runtime Overhead Generation exactly
once, advances the same owner, emits one dependent exact-envelope completion Effect,
and preserves the last protected Result slot before the commit barrier releases. An
indeterminate outcome retains the carry token and produces no Result. Barrier release does not
release the owner or ordinary work: the Runtime Closure Gate and Control Mutation
Cancel Gate remain closed, and discretionary Turn dispatch stays deferred while only
headroom-charged bounded safety and registry-approved nonincreasing closure transitions
may interleave with exact-envelope completion. Audit append,
synchronization, verification, and a `FULL` pending-clear transaction plus parent
synchronization then return that Effect's ordinary sequenced Result. Only its
accepted C27 transition releases the owner/headroom, reopens C25's Runtime Closure
Gate, and returns the typed completion disposition; S20 then clears the Event Loop gate
in the same handling step without an intervening dequeue or Core transition and
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
can drive C27's atomic activation.

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

A final focused correction defines Admission's bounded Certification Applicability
Selection, one derived immutable Runtime Overhead Bound Set, a canonical
build-locked Runtime Overhead Catalog, and a Core-owned Runtime Overhead
Generation. The Set binds the exact two-level daemon build, complete
Configuration Snapshot, Hot-Path maxima, Environment Fingerprint, every selected
Certification Record and Case Bound Table, Backend Operation Bound Set,
branch-span schema, and support sliding-horizon schema. Its exact ADR scope is
the D05 ledger row below. Receipt and Plan Rejection are disjoint complete
envelopes; every
support operation owns a complete lifecycle-classified daemon envelope plus an
independent direct-call watchdog; Core alone owns arbitrarily aligned support
charges and carry; a separately qualified local-stale envelope closes the
pre-call race without citing the stale Set; stale evidence invalidates older Plans
before execution; and online samples may invalidate but never widen a Set or
become Model Ledger service.

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
  in-process Adapter and Backend Interface identities; and
- `scheduler-performance` exposes only the old `measure-release-core` request,
  Plan, latency/work, and IPC fields, so it cannot encode Receipt/rejection/support
  envelopes, direct-call watchdogs, sliding horizons, pool carry, or their oracle.

TurnVector must not copy that descriptor or reintroduce a Backend process,
per-Turn IPC, native snapshot recovery, or a private serialized Backend protocol
to satisfy stale expectations. Compatible existing lanes may provide partial
evidence. Full P0 qualification requires a separately authorized, read-only-to-
this-task TurnVectorBenchmark update that rebinds the source contract, public
protocol descriptors, P0 persistence semantics, same-process fail-stop model,
and certification dimensions, including a new benchmark-owned
`scheduler-performance` schema, suite, runner, oracle, and gates before Q04.
Until that fixed revision exists, P0-7 benchmark-lane execution and the ready-PR
gate remain explicitly pending rather than being reported as passed.

## Module Seams

Parallel implementation ownership, crate-private interface laws, agent waves,
and serialized landing are defined in the
[P0 Parallel Module Delivery Plan](2026-08-19-p0-parallel-module-delivery.md).
That delivery split does not change the ledger order or create independent Core
commit authorities.

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
describe_model(registration)                  -> RawModelDescriptor
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
not retried in the same process. `describe_model` returns one bounded raw canonical
frame plus untrusted identity, hash, and vocabulary claims; C10c's private verifier
alone validates those values and seals the Verified Model Descriptor before
registration. The daemon-owned `request` input to stateless
`describe_request` contains the frozen Token Request, exact validated Model
Descriptor, and current Backend Generation, all of which the Result binds; no
Backend registry lookup, request handle, KV, or Sampling State exists at this
point. The Result returns a complete finite Capability Requirement Set, never a
future batch/Shape choice, Envelope, applicable Record, or exact authorized Key.
Only a current-generation Result may reach Admission. C12 is the sole description-
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
and Core withdraws every unconsumed C15 output/transient component and C16 obligation or entitlement claim in
the same terminal transition. A zero-ownership
Materialization Result also makes no release call but authorizes Core to split its
proven never-allocated remainder from proven
actual allocation entering Pending Reclaim; C22 also withdraws daemon output/transient
capacity and closes the impossible support paths from Core-owned facts. A partial-
ownership failure performs the same daemon-component withdrawal while retaining only
the Backend Budget and release obligation. Any complete or partial Backend
ownership is released exactly once by `release_request` on the owner thread.
Terminal Core state retains the complete Request Backend Allocation Budget until
its accepted Release Result consumes ownership, releases the Budget's proven
never-allocated remainder, and moves proven actual allocation into Pending
Reclaim, where it remains until fresh allocator and footprint convergence proves
reclamation. C22 settles daemon output/transient capacity after a zero- or partial-
ownership Materialization Result, and C28 settles it for a proven pre-materialization or
queued cancellation after invalidating candidate membership. Only an ordinary Turn
terminal or in-flight-Turn cancellation reaches C41, which settles those daemon
components after Receipt output enqueue/discard has transferred or released all concrete
occupancy and no Turn remains in flight. The Backend Budget settles through the
zero-ownership Materialization Result or C30's ownership-consuming Release Result. An
operation unable to return the required synchronized deterministic
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
route those accepted requests through C12, and mark only its affected Model dirty. A
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
against the exact registered Manifest, C10c verifies the complete raw frame and
claims against the Manifest expectation, and Core requires exact C10d-sealed
frame, identity, typed hash, and vocabulary equality. It then repeats
`describe_request` for each bounded remaining Warming waiter against that verified
descriptor and current Generation through C12; every unrelated invalidated
request uses the same owner.
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

From successful Backend Initialization until immediately before shutdown, every
non-Turn, non-Residency, non-Exclusive Backend call consumes its externally
verified finite continuous blocking and next-safe-point bound from the active
Backend Operation Bound Set. A canonical deterministic build step first creates
and locks the finite Runtime Overhead Catalog under an acyclic two-level daemon
build identity. Before initialization the daemon selects one candidate
Owner-Thread Support Budget, Sequenced Event Interference Bound, and companion
Stale Plan Disposition Bound by matching
the Daemon Core Build, complete active Configuration Snapshot, stable platform
facts, lifecycle/span schema, horizons, pools, and expected Backend descriptors.
No exact catalog entry fails startup. After initialization, a daemon-owned pre-
activation matcher completes Lifecycle Overhead Qualification against every
returned descriptor before activating all three bounds or making a support call. The
same matcher is the sole selector for a successor: it qualifies one exact replacement
entry against fresh stable facts and the unchanged returned descriptors and returns an
immutable witness binding the proposed Configuration, selected entry, descriptor identities,
stable facts, and witness generation before durable publication may begin. Core only
checks that witness identity/current generation, prepares carry, and may activate exactly
that witness after committed publication; it never matches external lifecycle facts or
selects descriptors. Neither path derives request Certification Applicability. Thus
bootstrap description and the first safety sample are bounded before readiness or
Admission. The Owner-Thread Support Budget separately bounds the
complete daemon envelope for describe, materialize, release, Candidate Formation,
Receipt observation, and resource sampling: optional Ordinary Reservation Claim,
obligation, credit, and active-record creation or
standalone C07 `begin_support` conversion after any predecessor envelope ends, preparation, direct
call, result translation/validation, accepted Core transition,
and immediate in-memory Effect issuance. The direct-call interval retains an independent watchdog subspan, but
Deadline Cost uses the larger complete envelope. A closed lifecycle matrix admits
only bootstrap description and safety samples pre-ready; all seven support
operations while ready; only required observation, release, description
revalidation, and safety sampling while evidence is degraded or recovering; and
only required observation, release, and safety sampling while draining. New
materialization and Candidate Formation require readiness.

The budget defines `support_interference(H)` for every configured positive
Monotonic Time horizon as the checked maximum active-charge and all-unstarted-obligation
complete-envelope capacity, including `conditional` and `pending` states, in any
half-open `[t, t + H)`. It has no alignment
boundary: carry-in and crossing calls are charged once at their full reserved bound,
and ordinary, mandatory-completion, and safety-sampling pools all contribute without
borrowing. Each Catalog entry also carries one finite Support Start Count Bound: for
every operation and pool it limits starts in each configured half-open window,
including the Catalog Retention Horizon. C07 alone enforces those checked counts with
fixed typed physical start credits shared by all unstarted operation obligations and retained
active records; conversion moves one credit rather than creating another. Every actual
Backend support call has exactly one operation-scoped obligation and credit, regardless
of Batch member count. Ordinary exhaustion defers or rejects before an envelope starts,
while Admission, mandatory completion, and safety sampling obtain their nonborrowable
credits and funding claims before the causal obligation exists.
Successful Admission atomically obtains actual initial-Materialization,
initial-Candidate-Formation, and ownership-release obligations with their start credits
plus one conserved reusable Future Turn Support Entitlement with a finite per-operation/
pool/horizon Support Outstanding Credit Vector in the sole Core support ledger before
its Timing Commitment exists. The vector is derived from the finite request closure and
reserves the unbatched worst case against later Admission. Each later Turn Plan creates
one shared Receipt-observation obligation, one conditional continuation-formation
obligation, and the mutually exclusive rejection/local-stale formation obligation. Each
is funded atomically by all member entitlements and carries one distinct physical
credit. The entitlement also funds
one terminal membership-change Candidate-Formation obligation only when removal
invalidates candidate membership. Each shared obligation occupies one distinct vector
slot in each funder, and active or terminal claims remain occupied through Catalog
retention.
After the complete causal predecessor envelope has ended and immediately before
preparation, one standalone Core `begin_support` transition atomically converts the
pending obligation to an active absolute-time charge; no Receipt, rejection, local-stale,
cancellation, or prior support Result transition may also begin the successor envelope.
Before conversion, cancellation or membership change may atomically rebind, split,
merge, or close operation obligations only with exact call-scope, physical-credit, and
per-funder-vector conservation; active obligations never rebind. Only typed proof that
the call can no longer occur may release unused unstarted capacity. A truly optional
ordinary support envelope starts with its typed reservation transition. The Event Loop
stores no parallel ledger and a shorter
observation earns no refund. Active charges and their start credits remain until the
build-derived Catalog Retention Horizon, the maximum across every production entry,
can no longer reference them; terminal entitlements remain as bounded funder tombstones
until linked claims expire. B04 proves fixed record/obligation/vector/lifecycle-reserve
capacity under maximum consecutive-Turn and sequential churn, maximum Batch split, and
every Support Start Count Bound plus one dedicated
nonborrowable Prepared Carry slot and worst-case dual-Budget mandatory/safety
suballocation for every activation sequence. Required completion and safety capacity
is pre-reserved before it becomes mandatory. In particular, a bounded post-load
description-obligation set is reserved before a load Effect; a Receipt-observation
obligation carries its possible post-update description set before the call; and
qualification plus the sole sample trigger maintains one safety-sample obligation
before readiness or evidence freshness depends on it;
nonessential work defers or rejects if any horizon would overflow. Initialization
and shutdown remain outside this lifecycle, Residency Service is separately gated,
Exclusive execution pauses shared dispatch, and no queued maintenance is free
background work.

A separate Sequenced Event Interference Bound exhaustively classifies every
non-Backend Event Loop/Core event that may pass a scheduling cut while a Timing
Commitment survives. At every synchronized scheduling-trigger boundary before any
fresh Snapshot, the Event Loop freezes the already admitted bounded event prefix plus
one fixed nonborrowable mandatory-crossing allowance. The same path covers the first
runnable plan, Receipt, Plan Rejection, local stale, support Result, and idle re-entry;
later ordinary events cannot overtake the fresh Snapshot and selected Turn.
Later cancellation, disconnect, Critical, failure, or shutdown work consumes that
allowance, then coalesces or defers, or invalidates affected commitments and enters
bounded fail-stop when deferral is illegal. External evidence supplies complete per-
event envelopes, and `sequenced_event_interference(H)` combines both finite counts
with Hot-Path maxima, Core Event Reserve, and the cut rule without a second runtime
ledger.

A Control mutation pauses ordinary support before durable publication. An
unchanged Configuration carries the same qualified Budget as both predecessor and
successor; a Configuration identity change requires the daemon matcher's exact immutable
successor qualification witness, which Core verifies but never selects.
Core reserves one Prepared Carry Reservation in that dedicated slot, bound to the
Support Ledger Generation, both Budget identities and Support Start Count Bound tables,
every Catalog-retained active charge and consumed start credit, every unstarted
`conditional` or `pending` Support Operation Obligation with its physical credit and
funding claims, every Future Turn Support Entitlement vector and retained tombstone,
every description/safety lifecycle reserve, and the Catalog
Retention Horizon, and the build-proved maximum separately typed dual-Budget
mandatory-completion and safety capacity for the bounded publication-through-
activation interval. Later mandatory/safety work
must consume that token while satisfying both Budgets and both count-bound tables; unrelated ledger mutation
invalidates it. Under the closed Runtime Closure Gate, the Event Loop drains all
request liability, obtains C26's current zero-liability witness, and revalidates
that witness plus the token at Commit Barrier entry before Store I/O. C25's gate,
not C26's pure validator, keeps the witness stable. Atomic activation retains the
same Core ledger and token-consumed records while changing Snapshot, Budget
identity, and Runtime Overhead Generation exactly once, without refund or window
reset. Before Store publication, graceful cancel or Fence-attempt exhaustion returns
one typed C27 nonpublication abort that consumes the carry token, preserves predecessor
charges/Budget, releases only unused prospective capacity, and resumes ordinary
reservations. Audit Degraded retains the token; a durability-indeterminate path
fabricates no Result or release. A failed post-commit invariant is fail-stop, never
partial activation.

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
GPU, unified-memory, and macOS facts and joins them with the exact TurnVector
Daemon Build Identity, initialized Adapter and MLX builds, Interface, Bootstrap Manifest, exact
Capability descriptor, Signal Contract, and externally verified Operation Bound
Set identities into one fresh Environment Fingerprint. Admission alone evaluates
exact Certification Applicability and may
cache each answer in a fixed-capacity recency cache keyed by every determining
Control, record, capability, exact daemon/Adapter/MLX build, interface, and
environment identity. Every hit rechecks the current evidence's identity and
freshness; stale evidence fails closed, and any changed member invalidates the
entry. For each Admission attempt Core reconstructs one bounded immutable
Certification Applicability Selection over every Requirement and its exact Record
and Case Bound Table; the cache never authorizes or stores that aggregate.
Certification Records have no invented wall-clock expiry. The cache is neither
persisted nor audited as authority.

Runtime Overhead remains daemon-owned. The pure Admission calculation
deterministically constructs one finite immutable Runtime Overhead Bound Set and
complete accepted decision from its exact immutable Certification Applicability
Selection and the already active Lifecycle-qualified Support Budget, Sequenced
Event Interference Bound, and Stale Plan Disposition Bound. One Core transition
commits the decision's exact Authorized Capability Set, Bound Set, Timing Commitment,
C15 resource components, C16 initial-Materialization, initial-Candidate-Formation and
release obligations, reusable Future Turn Support Entitlement with its complete vector,
finite per-Turn and terminal membership-change operation requirements, and composite
Resource Reservation together or rejects without allocation. Each later Turn Plan must
atomically create its exact Receipt-observation obligation, conditional continuation-
formation obligation, and mutually exclusive rejection/local-stale formation obligation
against every member entitlement before the Turn starts. Each shared operation owns one
distinct physical credit while
occupying one vector claim per funder. A queued cancellation that removes candidate
membership atomically rebinds, splits, merges, or closes unstarted obligations and funds
any required terminal membership-change obligation in C28. The
Set identity binds the
span-schema version, exact daemon build, complete Configuration Snapshot identity,
Hot-Path Work Budget identity and maxima, fresh Environment Fingerprint, every
selected Certification Record and Case Bound Table identity, Backend Operation
Bound Set identity, complete finite Turn-result case table, exact Support Budget
identity plus sliding-horizon table, active Sequenced Event Interference Bound
identity/table, and active local-stale bound identity/value.
It is neither
persisted Control State nor a Backend result, Cost Profile, online estimator, or
Event Loop applicability decision. Runtime instrumentation enforces and measures
the Core-supplied Set only. Missing evidence, identity drift, or Turn-envelope
excess makes the Set inapplicable and blocks unsafe new Timing Commitments; a
support, sequenced-event, or local-stale envelope excess invalidates Lifecycle Overhead Qualification
and fail-stops before another Backend or scheduling call. Online observations may
invalidate or diagnose evidence but never create or widen it.

The Set partitions one Turn attempt from the Scheduling Snapshot used for that
attempt. Its Receipt envelope contains pre-call intrinsic overhead, the complete
`execute_turn` direct-call interval as Engine Service, and post-call validation,
Core commit, and directly resulting Output Publication enqueue overhead. Its Plan
Rejection envelope contains the same pre-call intrinsic class, the complete
rejection direct-call interval as Runtime Overhead, rejection validation and Core
commit, and one intrinsic fresh-Snapshot/replanning disposition; it contains zero
Engine Service. Every Plan freezes an intrinsic local-disposition successor ceiling
admitted beneath this branch plus one already funded operation obligation for possible
rejection/local-stale Candidate Formation. A local Stale Turn Plan uses the separately active
qualified intrinsic bound only when it fits that ceiling, submits one dedicated Core
Event, and makes no Backend call; it does not cite the stale Set, fabricate Plan
Rejection, or create a third unbounded branch. The intrinsic disposition ends before
`form_candidates`; required Candidate Formation begins only after a later standalone
C07 `begin_support` transition converts the exact pending operation obligation and owns
one separate support envelope, while typed causal impossibility closes it.
The Event Loop captures any call interval once and classifies it only from the typed
result. Instrumentation proves Turn, support, and sequenced-event envelopes ordered
and complete without a gap or overlap. Let `B` be the greater branch envelope.
Admission proves every authorized future phase/batch/Shape/result case has a
certified positive horizon `H` satisfying checked `B + support_interference(H) +
sequenced_event_interference(H) <= H` and reserves the maximum across that closure.
Deadline-cost composition later selects the smallest valid `H` for an exact
Candidate; absence rejects it. Deadline Cost is that checked sum, with each
interference term added once, so support or scheduling-cut event work is not hidden
in intrinsic spans and a fixed-window edge cannot admit another uncharged burst. The
rejection-driven intrinsic disposition belongs only to that rejection branch; if it
selects a replacement, the replacement partition begins after any separately
measured Candidate Formation and cannot remeasure the same intrinsic Snapshot or
selection interval. The replacement uses a new bound; infeasible remaining
commitments emit SLO Risk rather than receiving an online-widened promise.

The support subdescriptor exhaustively classifies `describe_model`,
`describe_request`, `materialize_request`, `release_request`, `form_candidates`,
`observe_turn_receipt`, and `sample_backend_resources`, every lifecycle state, and
every complete daemon-envelope result branch. Initialization precedes the support
lifecycle, Receipt/rejection classifies `execute_turn`, Residency and Exclusive
retain separate gates, and shutdown follows the lifecycle. Adding an Interface
operation, Core-event kind, lifecycle state, result branch, or Configuration field
outside the complete Snapshot-bound evidence identity fails closed; no queued owner-thread
work is free background work or Model Ledger service.

Runtime Overhead Generation is the Core-owned fourth member of Generation Vector.
The non-Control evidence-invalidation transition is its sole owner for Set drift;
the Control-successor transition is its sole owner for every durable Control
activation and advances it exactly once. An unchanged Configuration keeps the same
qualified lifecycle descriptors through an old-equals-new carry token; a changed
Configuration activates only the exact immutable successor witness supplied by the daemon
matcher after Core verifies its identity and current generation. Profile Revalidation
consumes either already advanced value and cannot advance it again. Both paths invalidate all
older not-started Snapshots, candidate-to-Set associations, and Plans before
dispatch resumes.
The Event Loop checks exact generation plus Set identity immediately before
`execute_turn`; mismatch submits the dedicated Local Stale Plan Core Event without
a Backend call, Plan Rejection, or Turn Receipt, under the current qualified bound
and frozen ceiling. A Control successor proposal must freeze the daemon matcher's exact
immutable same-or-replacement qualification witness and Support Budget carry proof before
durable publication; Core verifies but never selects the witness. Owner creation closes the
Runtime Closure Gate to new request liability, and Commit Barrier entry waits for
every accepted request, Plan, Timing Commitment, Future Turn Support Entitlement,
request Backend ownership, pending release, and output reservation to close through
registered nonincreasing edges.
After Store commit the successor activates atomically with Configuration, unchanged
charge ledger, and generation; intervening evidence drift cannot roll back Control
and instead leaves Admission closed until fresh successor evidence is attached.

The Runtime Overhead witness is continuing. B03 locks the payload-independent
Core identity, B04 compiles the Catalog, Catalog Retention Horizon, dedicated carry
capacity, and event-cut registry against it, and B05 embeds the payload and locks the
Catalog/outer identities; C15 owns non-support admitted resource capacity, while
C07, C08a, C08b, C16, C17, and C18 incrementally implement one Core-owned
support ledger over active charges, unstarted
conditional/pending obligations, entitlement vectors/tombstones, lifecycle reserves,
retention, and standalone post-predecessor `begin_support` conversion, while C26 owns
the publication-specific carry token;
E15 enforces and measures the finite sequenced-event prefix; E16 closes
both Turn-result branches over the first working Fake Event Loop and only enforces
a Core-supplied Set; E17 selects, qualifies, and witnesses the exact lifecycle descriptors;
E18 closes every permitted support operation's complete daemon envelope, obligation
conversion, and watchdog subspan; and E19 integrates both interference terms and
carry without introducing a second owner for either ledger. Every
later commit that adds covered
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
   staged path are forbidden. T01 is the sole bootstrap: its candidate checker
   output is test evidence, while the review authority remains an explicit
   path/mode/blob table and full diff. T02 keeps that bootstrap review path
   while binding executable policy authority; installed authority permanently
   consumes the bootstrap and rejects restoration of the bootstrap helper before merge. After signed T02 installation,
   the canonical command is:

   ```sh
   set -eu
   head="$(env -i PATH="$PATH" LC_ALL=C GIT_NO_REPLACE_OBJECTS=1 \
     git -C . rev-parse --verify HEAD^{commit})"
   remote="$(env -i PATH="$PATH" LC_ALL=C GIT_NO_REPLACE_OBJECTS=1 \
     git -C . rev-parse --verify refs/remotes/origin/main^{commit})"
   auditor="$(mktemp "${TMPDIR:-/tmp}/turnvector-policy.XXXXXX")"
   trap 'unlink "$auditor"' EXIT
   entry="$(env -i PATH="$PATH" LC_ALL=C GIT_NO_REPLACE_OBJECTS=1 \
     git -C . ls-tree "$head" -- scripts/check_worktree_policy.py)"
   test "${entry%% *}" = 100755
   entry="${entry#* }"; test "${entry%% *}" = blob
   env -i PATH="$PATH" LC_ALL=C GIT_NO_REPLACE_OBJECTS=1 git -C . \
     cat-file blob "${head}:scripts/check_worktree_policy.py" >"$auditor"
   test -s "$auditor"
   env -i PATH="$PATH" LC_ALL=C \
     python3 -I -B "$auditor" --base "$head" --limit 420
   test "$remote" = "$(env -i PATH="$PATH" LC_ALL=C GIT_NO_REPLACE_OBJECTS=1 \
     git -C . rev-parse --verify refs/remotes/origin/main^{commit})"
   test "$head" = "$(env -i PATH="$PATH" LC_ALL=C GIT_NO_REPLACE_OBJECTS=1 \
     git -C . rev-parse --verify HEAD^{commit})"
   ```

   It starts isolated from the accepted `HEAD` Git object rather than the
   candidate worktree, selects the exact remote-base policy revision or the
   first reviewed T01 installation when the remote base predates T01, preserves
   the exact remote tip as the separate configuration-lineage endpoint, executes
   that accepted auditor instead of changed candidate policy code, enumerates every
   tracked and untracked path, reports counted LOC, writes the ignored review
   manifest, and prints its SHA-256. Earlier documentation-only commits use an
   explicit path/blob table from `git hash-object`, plus `git diff --binary`
   for tracked files or `git diff --no-index` for an untracked file.
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
| D04 | `docs(adr): define p0 audit sequence authority` | Define exact P0 storage-qualification binding, Control Initialization, sole later Control mutation authority, range-only sequence-state authority, build-derived mutation headroom and Audit Safety Reserve, Core-owned Runtime Closure Gate, Event Loop-owned Control Mutation Cancel Gate, bounded noncreating Predecessor Fence interleaving, known-absent closure, two-stage success Results with no post-C27 discretionary work, installation-scoped process lock across every successor, Control Publication Commit Barrier, commit/parent recovery, pending-before-tail, lost suffix, and pre-writer Storage Barrier Failure; align ADRs 0019/0021/0026/0027/0029/0036/0040 plus glossary | No Anchor/Locator/Metadata, implicit latest qualification, same-ID P0 Rebind, second pointer, sequence-owner/headroom overlap or terminal borrowing, closure-liability fanout, delayed or identity-cycling cancel custody, safety-queue starvation, Event Loop publication/result gap, post-C27 discretionary assignment, clean-restart or cross-Runtime lock bypass, fabricated indeterminate Result, or unchanged-state claim after a required barrier failure | docs only |
| D05 | `docs(adr): bind runtime overhead evidence` | Define the acyclic daemon/Catalog build identity, generated Runtime Overhead Catalog and Lifecycle Evidence Tables, repeatable daemon-selected pre-activation Lifecycle Overhead Qualification witness and readiness gate, Admission-owned applicability selection, separate Core-owned Resource Capacity and Support Charge Ledgers with independently green support-capacity, ordinary/lifecycle, request-entitlement, Plan-obligation, and retention slices, Catalog-bound Support Start Count Bounds, operation-scoped Support Operation Obligations, typed Support Funding Claims including Ordinary Reservation Claim, finite request Support Outstanding Credit Vectors, pre-trigger description/safety lifecycle reserves, reusable Future Turn Support Entitlements, standalone post-predecessor `begin_support`, explicit daemon-component terminal settlement, dedicated prepared carry, Admission-derived daemon Bound Sets, Core-owned Runtime Overhead Generation, Sequenced Event Interference Bound and all-trigger scheduling cut, dedicated intrinsic local-stale transition, disjoint Turn/support/event envelopes, request-quiescent Control publication, complete Configuration binding, and closed Backend lifecycle classification; update ADRs 0005/0008/0015/0016/0018/0020/0021/0022/0024/0025/0027/0030/0031/0034/0036/0040 plus glossary and remove implementation-row coupling | Catalog-wide retention, per-operation/pool half-open start-count enforcement, optional ordinary nonempty claims, consecutive-Turn/sequential churn, B1/B4 one-call-one-credit conservation including separate observation and conditional continuation, conditional-state credit/vector/horizon/carry occupancy, mixed initial+entitlement funding, newly eligible member join, Batch split/merge/cancel rebind, and activation-sequence capacity including lifecycle/carry/suballocation maxima; startup/successor qualification witness, first pre-ready sample through drain, atomic two-ledger Admission, initial obligations, request-lifetime vectors plus three distinct Plan-scoped operation obligations, post-load/post-observation description sets, safety trigger, first-plan/Receipt/rejection/local-stale/support-result/idle scheduling-cut prefix and crossing allowance, intrinsic local stale disposition, complete nonoverlapping envelopes including Output Publication before observation, sole ledger/generation/selector owners and separate Core/Event-Loop gate owners, typed prepublication carry abort, pre-owner carry/generation/headroom revalidation and ordinary-support restoration, stable zero request-liability before Store/Audit pause, exact evidence/build identity, single-count three-term Deadline Cost, pre-dispatch drift rejection, and no online widening | docs only |
| D06 | `docs(architecture): define exact execution profiles` | Amend ADRs 0007, 0017, and 0031, the glossary, this plan, and the native ownership todo to define Certified Execution Profile and Execution Route Identity, distinguish stable exact identity from dynamic Resource Evidence, and bind every graph, memory, kernel/fusion, KV, speculative, prefix, and command-replay route change to a new exact Capability Key | Exact versus one-field-drift matrix, exact required-member baselines, explicit absent optional plans, no Serving Profile collision, runtime range inference, favorable-resource authorization, raw GPU-address promise, or P-1 status upgrade | docs only |
| T01 | `build: audit unstaged commit scope` | Freeze a side-effect-free entry path/type/mode/raw-byte identity before filters, then record the stable staging-equivalent tracked/untracked path hashes and documentation-aware LOC | Unit fixtures for add/delete/rename/binary/untracked, self-count, transformed content, and forward-sorted filter mutation | <= 420 |
| T02 | `build: bind contribution-policy authority` | Fail-closed extraction and execution of the accepted Git-object auditor/helper rather than candidate policy code; authenticate the executing helper against the explicit accepted base rather than checkout HEAD; clear Git selectors/config injection and disable replacement objects before extraction; bind both branch config lineages, checked-out base/fetched PR/event identities, path-specific policy-owner modes, policy-only transitions, one-time bootstrap consumption to irreversible installed-helper history with bootstrap-helper restoration rejection, isolated local/CI post-commit startup, config-invariant capture, secure manifest publication, and post-commit parity including clean base-sync merges | Missing/empty accepted object, wrong accepted source, candidate self-change, successful policy-only helper update, helper relaxation, add/remove and divergent-remote config history, pre-T01 divergent-base post-merge worktree continuity, in-scan remote-ref drift, stale event/ref identity, unsigned commit, installed-authority helper-restoration rejection, squash-installed bootstrap replay, symlink/mode/delete/rename, Git selector/config injection, replace ref, hostile startup for both worktree and post-commit commands, ambient diff configuration, manifest path replacement, workflow mixing, and exact post-commit clean/extra-payload/unrelated-parent merge fixtures | <= 420 |
| B01 | `build: initialize the Rust workspace` | Rust 1.97.1 toolchain, workspace, core crate, format/lint/test entrypoints | Format, clippy, workspace tests | <= 180 |
| B02 | `build: lock the generation semantics descriptor` | Canonical descriptor generator and Evidence Hash lock shared by daemon Domain Types, Fake, Manifest, Native, and qualification | Double-generation bytes/hash; incomplete/unknown descriptor rejection; every schema/domain/greedy/categorical/RNG semantic mutation changes identity | <= 300 |
| B03 | `build(runtime): lock the daemon core build identity` | Canonical payload-independent Core build descriptor/generator binding an exact dependency-traced runtime source-closure manifest, tool/dependency locks, features/profiles, protocol/domain registries, native inputs, the private Model Descriptor V1 frame/schema/domain tags and size/arena maxima, and exact repository-owned SHA-256 derivative source bytes whose in-source provenance header binds the upstream commit, archive digest, and selected license, plus Catalog schema/capacity/lookup/worst-case work, Support Start Count Bound, Support Funding Claim including Ordinary Reservation Claim, and Support Outstanding Credit Vector schemas/binary maxima, conditional/pending-operation-obligation/active-record/ordinary-claim/entitlement-tombstone/lifecycle-reserve maxima and their Ingress Budget Warming/active plus Model Registry cardinality inputs, one dedicated nonborrowable Prepared Carry slot with dual-Budget mandatory/safety suballocation maxima, event-registry maxima, and executable/native text-section identities while excluding generated payload bytes | Double generation, undeclared build-input rejection, every determining-input drift including SHA source bytes or their provenance header, frame/domain/schema, descriptor arena and registry cardinality, unrelated-repository-file invariance including the Markdown provenance notice, payload-byte invariance, schema/capacity/start-count/funding-claim/ordinary-claim/vector/conditional/pending-obligation/carry-slot drift, section mismatch, and final-binary self-hash rejection | <= 360 |
| B04 | `build(runtime): compile the runtime overhead catalog` | Consume one B03 Core identity and deterministically compile/lock the bounded finite Catalog payload over external Core-bound Lifecycle Evidence Tables; bind one finite per-operation/pool Support Start Count Bound for every configured half-open window, derive the Catalog Retention Horizon, and prove fixed active-charge/physical-credit/ordinary-reservation-claim/funding-claim/conditional+pending-operation-obligation/entitlement-vector+tombstone/post-load/post-observation-description/safety-reserve capacity plus one dedicated Prepared Carry record and worst-case dual-Budget mandatory/safety suballocation over finite request closure, maximum consecutive-Turn and sequential churn, unbatched demand, separate Receipt-observation and conditional-continuation calls, Batch split/merge/cancel, every entry activation sequence, maximum accepted-request cardinality, and global/per-connection Warming/active plus Model Registry cardinalities, even at full ordinary capacity; reject request Certification Case Bound inputs and exhaustively cover twelve-operation/event/local-stale lifecycle, span, horizon, pool, entry-count, and byte maxima; synthetic fixture evidence carries a test-only nonproduction type | Double generation, zero/missing/overflowed count/vector, optional ordinary claim and ordinary-pool exhaustion, exact-window and one-past consecutive-Turn/sequential churn, B1/B4 one credit per observation/continuation/non-Receipt call versus unbatched maximum, conditional-state retention and conditional-to-pending-or-close, mixed initial+entitlement claims, newly eligible-member join, split/merge/cancel before start and frozen-after-start, terminal tombstone retention, post-load/post-observation description and first/next safety triggers, short-to-long history, activation-sequence/accepted+preparation+registry-cardinality/full-ledger/carry-slot/suballocation capacity edges, mandatory/safety nonborrowability, event registry/cut maxima; direct/transitive outer-build self-reference, request Case Bound input, missing/duplicate/unknown/noncanonical Lifecycle Evidence Table, unclassified operation/event/state/branch, and fixture/production type separation | <= 400 |
| B05 | `build(runtime): bind the runtime overhead catalog` | Place B04 payload in a deterministic non-executable read-only section, lock Catalog identity over payload and outer daemon identity over B03 plus Catalog identities, and embed the verified tuple without changing B03 code identities; only test targets may embed a fixture Catalog before L01 | Final tuple/section verification, code-section invariance across payload fixtures, worst-case maximum-size lookup, payload/Catalog/outer drift, production fixture rejection/non-readiness, and no final-binary self hash; runtime fact matching remains E17 | <= 300 |

### P0-1: Pure Runtime Core And Replay

C08 is an empirical delivery-only refinement of the original combined
ordinary/lifecycle slice. A `rustfmt`-normalized implementation with the
required contract tests measured 449 counted non-documentation changed lines
(281 production and 168 tests), exceeding both its 400-line target and the
global 420-line plan ceiling. It is therefore delivered as two consecutive,
independently green rows without changing the sole `support_ledger` owner, its
private interface, or any domain authority. The formatted implementation
estimate is 145-165 counted lines for C08a, whose fixed cap remains 180. The
exact Rust 1.97.1 `rustfmt`-normalized, focused-green C08b source diff is 344
additions plus 13 deletions, or 357 counted lines. Its normal B03-B05 and three-
fixture generated cascade adds 18 counted lines, projecting 375 and leaving a
five-line margin below the fixed C08b row cap of 380 and a 45-line margin below
the global 420-line ceiling.

C08a preserves C07's generic `LifecycleReserve` behavior as a compatibility
placeholder and installs no typed lifecycle-reserve authority. C08b depends on
C08a, replaces that placeholder path with typed lifecycle authority, and
completes every lifecycle behavior formerly assigned to C08. Neither row may
borrow the other's LOC cap or be reviewed as one combined commit.

C08b remains one independently green row because its typed lifecycle reserves,
held-capacity accounting, closed result matrix, and closure of the generic
`LifecycleReserve` construction bypass are one transition of the sole
`support_ledger` authority. Splitting those responsibilities would temporarily
create duplicate lifecycle authority or leave the generic bypass open.

The descriptor-integrity and registration implementation is likewise refined
only for delivery. The exact Rust 1.97.1 `rustfmt`-normalized, focused-green
C10a source diff remains 224 human-counted lines: `bounded.rs` contributes 21
additions plus 3 deletions, and `support.rs` contributes 172 additions plus 28
deletions. Its fixed 18-line B03-B05 and three-fixture generated cascade projects
242 against the unchanged cap of 260. C10b's fixed SHA-256 extraction measures
approximately 160 human lines plus the same fixed 18-line cascade, projecting
178 against a cap of 220. C10c's canonical frame and verifier are projected at
187-263 human lines plus 18 generated lines, or 205-281 against a cap of 300.
C10d retains a human hard maximum of 362 for the registry implementation adapted
to sealed descriptor values; its fixed cascade brings the row maximum to 380,
equal to its cap and 40 below the global 420-line ceiling. C10e's integrated
Core transition remains projected at 198-248 total lines against a cap of 280.

C10a, C10b, C10c, C10d, and C10e are ordered, independently green delivery rows.
C10a changes only the `support_ledger` prepared-change seam. C10b adds only the
private fixed SHA-256 primitive. C10c completes the private deep
`model_descriptor` verifier and sealed value but grants no registry, Core,
Support, Effect, or runtime authority. C10d adapts the sole `model_registry` to
consume sealed values without Effect or runtime authority. C10e alone installs
the original descriptor-registration behavior through `Core::handle`. The
refinement creates no second ledger, registry, descriptor verifier, or commit
authority and does not renumber C11 or any later row.

C10a remains one independently green row because its prepared
`FixedWindowCounter` start, opaque generation-bound `SupportChange`, and direct
ordinary-start/active-finish exact-Work regression form one prepared-change seam
in the sole `support_ledger`. Splitting those responsibilities would either
duplicate start/commit authority or lose independently-green compatibility
evidence that the legacy C07/C08 entry points preserve state and Hot-Path Work.

C10b and C10c are a deep-module authority split rather than a split of one
registry transition. C10b owns one private SHA-256-only one-shot primitive and
its known-answer/differential/Work evidence. C10c is its sole consumer and owns
the complete frame parser, two independent domains, untrusted-claim comparison,
and non-forgeable verified value. This ordering is independently green without
exposing a generic crypto seam. C10d remains cohesive because `DescriptionPlan`,
sealed descriptor retention, descriptor-bound `RegistryChange`, exact readback,
and post-load equality form one invariant in the sole `model_registry`.
Splitting C10d would expose a partially registered interface; folding it into
C10c would leak registry state and authority into the descriptor integrity
module.

| ID | Commit subject | Behavior slice | Required verification | Target LOC |
|---|---|---|---|---:|
| C01 | `feat(core): add checked domain identities` | Distinct IDs, units, sequences, durations, and Monotonic Time | Overflow, zero, and cross-type rejection | <= 360 |
| C02 | `feat(core): add generation and bounded collection types` | Four-member Generation Vector including Runtime Overhead Generation plus checked fixed-capacity collections | Capacity and every generation mismatch case | <= 340 |
| C03 | `feat(core): define scheduling snapshots and candidates` | Snapshot with Runtime Overhead Generation and daemon-associated exact Bound Set per exact-key Work Candidate, authorized-member set, and Candidate Exclusion contracts | Construction, evidence/Key/Set membership, generation, and completeness bounds | <= 380 |
| C04 | `feat(core): define turn plans and receipts` | Frozen Plan membership, Runtime Overhead Generation, exact Bound Set identity, each member's Future Turn Support Entitlement/vector identity, one Plan-scoped Receipt-observation obligation, one conditional continuation-formation obligation, and the mutually exclusive rejection/local-stale formation obligation, each funded by the canonical member set and carrying one distinct physical credit, plus completed/cancelled/partial/failed member outcomes | Stable order, evidence/generation/entitlement/vector/obligation field identity, B1/B4 structural one-call-one-credit invariants for observation, continuation, and non-Receipt formation, outcome variants, and member bounds; no live ledger mutation in this type slice | <= 400 |
| C05 | `feat(core): apply atomic core transitions` | `Core::handle`, contiguous Event Sequence, ordered Effects, rejection/fault shell | Failed invariant preserves state and emits no Effect | <= 400 |
| C06 | `feat(core): establish hot-path work budgets` | Incremental witness types, binary maxima, counted transition shell, and hard rejection | Exact base counts, overflow, no truncation or full-state scan | <= 380 |
| C07 | `feat(core): define support ledger capacity` | Sole fixed-capacity Support Charge Ledger foundation with Support Ledger Generation, nonborrowable ordinary/mandatory-completion/safety pools, B04 Support Start Count Bound enforcement, fixed typed physical credits, canonical nonempty Support Funding Claim sets, and closed unstarted `conditional`/`pending` plus active/retained record states; every unstarted obligation owns one credit and all funder claims from creation, `conditional` can only become `pending` or typed-impossible closed, and standalone `begin_support` moves one `pending` obligation/credit to one active absolute-time charge after its predecessor envelope | Capacity and generation edges; exact-window/one-past; conditional/pending/active/retained credit and claim conservation; conditional cannot begin, disappear, or release headroom; predecessor-end and no-refund rules; nonborrowable pools; no Event Loop/Backend owner or public unchecked constructor | <= 380 |
| C08a | `feat(core): start ordinary support reservations` | Extend the sole ledger with exact support-call scope and atomic optional Ordinary Reservation Claim, matching one-operation obligation, legal credit, and scoped active-record creation; C07 generic lifecycle records retain their zero compatibility scope and behavior | Optional nonempty claim/type/scope, ordinary exhaustion before Effect, legal start/usage/scoped-record identity, exact prior state and no Effect on rejection or fault, C07 lifecycle regression, stable generation, and Hot-Path Work | <= 180 |
| C08b | `feat(core): reserve lifecycle support` | Extend the same scoped ledger with typed bounded pre-trigger post-load/post-observation description and first/next safety lifecycle reserves; an exact reserved trigger moves its obligation to `pending` or typed impossibility closes it, no trigger allocates capacity, and the generic `LifecycleReserve` construction bypass now rejects | Description and safety maxima; load/observation/sample success, failure, and cancel branches; first pre-ready and next-before-expiry reserve; real post-observation and next-before-expiry public transitions; wrong/duplicate trigger, pending/begin/impossible-close, expiry/capacity edges, mandatory/safety nonborrowability, exact prior state and no Effect on rejection or fault, stable generation, and Hot-Path Work | <= 380 |
| C09 | `feat(core): manage immutable model revisions` | Manifest identities, Alias freeze, Available/Retiring/Unavailable lifecycle | No alias repoint, registry limits, and incremental counts | <= 400 |
| C10a | `feat(core): prepare support charge changes` | Add one crate-private, non-forgeable, Support-Ledger-Generation-bound `SupportChange` prepare/validate/commit seam for ordinary start and finish plus a prepared `FixedWindowCounter` start; existing C07/C08 entry points delegate without changing runtime behavior | Read-only preparation, exact-generation stale rejection, single-use commit, ordinary start/finish identity and state, fixed capacity, exact prior state on rejection or fault, stable Hot-Path Work witnesses, and all C07/C08 regressions | <= 260 |
| C10b | `feat(core): add fixed descriptor sha-256` | Add only `model_descriptor::sha256`, a private safe-Rust SHA-256-only portable derivative from the fixed `sha2-fv` source, with one bounded one-shot input and exact compression-block Work; no dependency, allocation, streaming, dispatch, generic crypto seam, or public API | NIST/FIPS known-answer vectors, exact-upstream differential fixtures, zero/maximum/preimage edges through 16,425 bytes, length and padding blocks, exact Work, zero allocation, selected-license/provenance inventory, and no formal/CAVP claim | <= 220 |
| C10c | `feat(core): validate canonical model descriptors` | Complete the private deep `model_descriptor` module: parse an exact V1 frame of at most 16,384 bytes, derive independent field-private `ModelDescriptorId` and typed `ModelDescriptorHash`, compare all untrusted Backend claims plus the Manifest expectation, and return only non-forgeable `VerifiedModelDescriptor` | Version/vocabulary/payload length, zero/oversize/trailing/padding, every byte and domain drift, ID/hash independence, wrong raw claim or Manifest hash, stable sealed equality, exact parse/compare/copy/two-hash Work, zero allocation, no mutation, registry/Backend/Core authority, or unchecked constructor | <= 300 |
| C10d | `feat(core): retain model descriptors` | Extend the private `model_registry` with a bounded `DescriptionPlan`, fixed 256-Revision cardinality and independent 4,194,304-byte retained-frame arena, only C10c-sealed complete frame/ID/hash/vocabulary values, descriptor-bound registration changes, and exact readback/post-load equality; remove the bare `Register` and raw-field bypasses | Plan and Registry Generation binding, stale/duplicate/count/arena-capacity rejection at exact and one-past bounds, sealed readback and exact equality/drift, no borrowing, raw construction, or partial registration, and no Core, Support, Effect, or runtime authority before C10e | <= 380 |
| C10e | `feat(core): describe model registrations` | Give Core sole custody of Support and Registry state; only after an exact C08a ordinary claim, `describe_model` obligation, credit, and active charge commits may it retain one pending `DescriptionPlan` and emit the stateless raw Model Descriptor Effect; an accepted Result must pass C10c verification and atomically finish that charge and commit C10d's `RegistryChange`, or commit neither | `Core::handle` start/Result observability; no Effect without the exact active charge; optional ordinary exhaustion; pending-plan identity/generation/result validation; duplicate, stale, malformed frame, raw-claim, Manifest-hash, vocabulary, and Registry rejection; exact rollback with no Effects; retained sealed values and post-load equality | <= 280 |
| C11 | `feat(core): accept requests into preparing` | Ownership, frozen Revision, C10c/C10d sealed-descriptor-authoritative Top K validation, explicit Service Class, closed Generation Parameters, immutable effective `u64` Sampling Seed plus origin, status version, and preparation timeout | Verified descriptor vocabulary bounds, explicit zero/caller/daemon origin; Acceptance is not Admission; retries never inherit state | <= 400 |
| C12 | `feat(core): drive and refresh request descriptions` | Sole initial/post-load/post-observation/stale-generation description owner with O(1) invalidation, stable resident reissue, and deferred Warming refresh; initial description emits an Effect only after C08a atomically creates its typed optional ordinary claim/obligation/credit/active charge, while post-load/post-observation refresh consumes only exact pending lifecycle obligations from the C08b set reserved before the causal load or observation starts and never creates support capacity | Initial optional exhaustion/no-Effect, cross-model advance/counts, C10c verification plus C10d sealed post-load equality, bounded model/request set, failure/cancel/impossible close, no stale rejection/Admission, no unreserved refresh, original timeout, registry, handle, or Resource Reservation | <= 400 |
| C13 | `feat(core): authorize exact certification keys` | Current-generation finite requirement-to-Key closure over immutable records and the read-only exact-key Authorization Index; every Key includes one exact Execution Route Identity and resolves to one Certified Execution Profile entry | Stale routes to C12 without lookup; omitted/overflowed/missing/drifted/quarantined Route or other Key member fails closed | <= 400 |
| C14 | `feat(core): derive certification applicability` | Fresh Environment Fingerprint including exact daemon/Capability/Generation Semantics/Resource Signal/Operation Bound identities, fixed recency cache, and complete finite immutable Applicability Selection over exact Profile entries | Every hit and complete selection recheck freshness/evidence; build/schema/algorithm/Route/other drift and selection-race invalidate; miss/eviction; Resource Evidence never creates applicability | <= 400 |
| C15 | `feat(core): own admitted resource capacity` | Sole fixed-capacity Resource Capacity Ledger for Request Backend Allocation Budgets, daemon output capacity, typed transient headroom, checked generation, atomic reserve/withdraw, and all-or-nothing rollback; only C22 zero/partial materialization, C28 cancellation, and C41 post-output Turn-terminal facts may settle daemon components, while C22's zero-ownership Materialization Result and C30's ownership-consuming Release Result are the only Backend Budget partition facts; Resource Governor supplies limits and mode but owns no ledger mutation | Cross-component and multi-request conservation, generation TOCTOU, capacity edge/overflow, pre-materialization/zero/partial/queued/in-flight-Turn/normal-Turn-terminal settlement, output-occupancy transfer, reserve/withdraw/rollback, no support entry, and no Backend or Governor mutation authority | <= 400 |
| C16 | `feat(core): reserve request support entitlements` | Extend the sole ledger with checked actual initial-Materialization/initial-Candidate-Formation/release obligation requirements, reusable request-lifetime Future Turn Support Entitlements, finite Support Outstanding Credit Vectors, terminal tombstones, and one atomic reserve/withdraw primitive over a fully supplied request-support requirement bundle; this slice neither constructs Admission nor creates a Turn Plan | Capacity/generation/identity edges; three distinct initial/release claims and credits; finite per-Turn and terminal branch requirements; vector exact-window/one-past and unbatched maxima; reserve/withdraw/rollback, terminal tombstone, mandatory-pool nonborrowability, and no partial state or Admission/Plan authority | <= 360 |
| C17 | `feat(core): manage plan support obligations` | From only C16-reserved live entitlements, atomically create separate Plan/model-scoped Receipt-observation, conditional-continuation, and rejection/local-stale many-funder obligations, bind C08b's post-observation lifecycle-reserve set, and create terminal membership-change formation only when required; create/fund/rebind/split/merge/typed-impossible-close conserves one global credit and one claim per affected request | Consecutive-Turn and sequential churn; B1/B4 one call/credit versus unbatched demand; initial/entitlement mixed Candidate Formation; newly eligible-member join, split/merge/member cancel, active no-rebind, conditional credit/vector/horizon retention and conditional-to-pending-or-close only on observation Result, post-observation reserve binding, membership removal, terminal branch and tombstone conservation | <= 400 |
| C18 | `feat(core): retain and snapshot support history` | Complete the sole ledger with Catalog Retention Horizon expiry, retained active records/credits/linked claims, one dedicated Prepared Carry slot, and an immutable bounded carry input over every conditional/pending obligation, entitlement vector/tombstone, ordinary claim, lifecycle reserve, pool and generation; expose the complete capacity/interference snapshot consumed by Admission and later C26/C27 integration without selecting a lifecycle witness or owning a Control outcome | Carry-in/out accounting, adjacent windows, deferred expiry, short-to-long Budget history, full ordinary capacity plus dedicated slot, dual-Budget mandatory/safety suballocation, conditional-obligation snapshot, stable generation, no reset/refund/borrow, witness selector, Control owner, or second ledger | <= 380 |
| C19 | `feat(core): construct bounds and decide admission` | From exact current C15 resource-capacity and C18-complete support-ledger snapshot, purely construct one Runtime Overhead Bound Set from C14 selection, complete Configuration Snapshot, and one active immutable daemon-produced Lifecycle Overhead Qualification witness; prove `B + support + sequenced-event` closure, Support Start Count Bounds, finite request closure and complete Support Outstanding Credit Vector, initial operation obligations, and one conserved Future Turn Support Entitlement funding separate Plan-scoped observation and conditional-continuation obligations versus rejection/local-stale plus terminal membership-change formation, then return one complete accepted Admission decision after checking current description/Revision, Authorized Capability Set, worst-case timing, Resource Mode, and both capacities | Core verifies but never selects lifecycle witness; no Event Loop/Backend applicability or Set authority; stale/missing/drifted Catalog/qualification/evidence, missing horizon/start-credit/vector/event/support/resource/obligation/entitlement/membership-change/Key, either ledger race, or failed condition returns rejection without allocation | <= 400 |
| C20 | `feat(core): commit timing and request reservations` | In one Core transition revalidate both ledger generations and atomically commit one C19 decision's C15 Backend/output/transient components, C16 initial-Materialization/initial-Candidate-Formation/release obligations and Future Turn Support Entitlement/vector, plus request-owned Authorized Capability Set, Runtime Overhead Bound Set, Timing Commitment, finite operation-branch requirements, and composite Resource Reservation | Complete decision/evidence/component/obligation/entitlement/vector identity, two-ledger TOCTOU, exact Candidate value never exceeds admitted maximum, multi-request conservation, original-versus-current witness, cross-ledger rollback, and no partial commitment or duplicated component; initial and entitlement state remains C16-owned while later Plan obligations are C17-owned | <= 400 |
| C21 | `feat(core): materialize admitted requests` | Only after C07 `begin_support` has converted the exact initial-Materialization obligation to an active charge, emit one Post-Admission materialization Effect while retaining the distinct initial-Candidate-Formation and ownership-release obligations plus stable Backend request ownership identity; C21 never mutates support state | No Effect before active charge or before composite Resource Reservation, Timing Commitment, entitlement/vector, and all three initial obligations; no second/duplicate conversion; zero/partial/full ownership causality | <= 380 |
| C22 | `feat(core): apply request materialization results` | Success owns state, marks the initial-Candidate-Formation obligation's materialization predecessor complete without converting it, and preserves that obligation plus the Future Turn Support Entitlement/vector for later same-envelope-end merge/begin; zero-ownership failure applies the Result's exact Request Backend Allocation Budget partition through C15, releases proven never-allocated capacity, moves proven actual allocation to Pending Reclaim, withdraws daemon output/transient components, and closes initial formation plus release obligations and the entitlement; partial-ownership failure withdraws daemon output/transient capacity, closes initial formation and every future operation-funding branch, and retains only the complete Backend Budget plus release obligation until C30 | Two-ledger Budget/component/entitlement/vector/obligation conservation, first-formation pending/merge/begin ordering, complete/partial/zero ownership, output/transient settlement, zero-allocation/allocated rollback, no-call/retry/early split/leak | <= 400 |
| C23 | `feat(core): invalidate evidence and revalidate profiles` | For non-Control Capability/environment/Bound-Set drift only, advance Runtime Overhead Generation once, invalidate all older not-started artifacts, and run the shared pure C14/C19-derived current-feasibility helper; expose that helper to C27 after its already advanced Control-successor transition | Non-Control exactly-one advance, every Control successor has no second advance, drift-before-dispatch race, original promise/evidence retained, current witness replacement, SLO Risk and new-Admission block without stale execution, silent re-promise, or online widening | <= 400 |
| C24 | `feat(core): quarantine certified bound violations` | Preserve Receipt, remove exact key, optionally escalate its parent, require explicit recertification, and route Runtime Overhead evidence invalidation through C23 without a second owner | Estimate miss does not quarantine; Turn-envelope excess performs quarantine then one generation advance; automatic widening and generation-only handling forbidden | <= 400 |
| C25 | `feat(core): gate runtime closure liability` | Build-generated exhaustive Runtime Closure Registry plus the sole Core-owned gate state covering every liability-increasing constructor, checked maximum, registered nonincreasing closure edge, and stability of a current C26 zero-request-liability witness | Every existing constructor/edge enumerated and wrapped; Request Acceptance, Core connection creation, Reservation/Residency Demand, and new Operation/Effect reject while closed; every later constructor extends the registry in its own commit; zero witness cannot be invalidated by an allowed edge; no C26 gate authority; closure conservation, read-only status, terminal reopen, Audit-Degraded retention | <= 400 |
| C26 | `feat(core): prepare support budget carry` | Verify one exact immutable successor Lifecycle Overhead Qualification witness supplied solely by E17, then pause ordinary reservations and create one generation-bound Prepared Carry Reservation only in B03/B04's dedicated nonborrowable slot over its old/new Budgets and Support Start Count Bounds, every Catalog-retained support active charge, conditional/pending operation obligation, physical credit, entitlement vector/tombstone, lifecycle reserve, the Catalog Retention Horizon, and worst-case dual-Budget mandatory/safety suballocations; provide a pure bounded validator for the current zero-request-liability witness without selecting lifecycle evidence or owning/stabilizing C25's gate. Every nonfatal failure before owner creation atomically closes the unowned carry and resumes ordinary reservations | Core never matches platform/Backend facts or selects a descriptor; full ordinary ledger still admits the dedicated token; missing/stale witness, unrelated-generation invalidation, carry-token/Support-Ledger-Generation drift, dual-Budget/count/vector/lifecycle maxima, short-to-long history, entitlement/conditional-to-pending/obligation race, deferred expiration, pre-owner failure closure/restoration, zero request liability, witness generation, no gate authority, and no borrowing | <= 400 |
| C27 | `feat(core): resolve control publication outcomes` | Sole Core consumer of one exact current C26 carry token plus publication or dependent-completion Result. Graceful prepublication cancel, Fence exhaustion, or exact-absent Store returns a typed nonpublication disposition that preserves predecessor Budget/charges, releases only unused prospective carry capacity, resumes ordinary reservations, closes the token and Core Effect/owner/headroom, and reopens C25's Runtime Closure Gate. A committed Result atomically installs the complete successor and the exact same-or-replacement lifecycle descriptors named by E17's immutable qualification witness with the same C18-complete support ledger, advances Runtime Overhead Generation once, invalidates old artifacts, invokes C23 helper without another advance, advances the owner, and emits the dependent completion Effect while leaving C25 closed; only its accepted completion Result releases owner/headroom and reopens C25. Every terminal transition returns a typed disposition for immediate same-Event-Loop-step daemon integration before any dequeue or Core transition and never reads or mutates the Event Loop-owned Control Mutation Cancel Gate | Abort/commit/completion close once with no activation leak or double release; Audit Degraded retains exact token/suballocations/stage, indeterminate fabricates no Result or release; unchanged/changed Configuration, witness identity/current-generation equality, Core never selects descriptors, every Control successor one generation advance, pre-Store witness mismatch returns to C26 only before owner, post-commit mismatch fail-stop; no refund/reset, second selector/owner, Event Loop gate mutation, disposition-integration gap, dispatch gap, early acknowledgement, partial visibility, or rewritten promise | <= 400 |
| C28 | `feat(core): order cooperative cancellation` | One typed matrix shared by client/control cancellation and Governor `critical_eviction`: Preparing/Warming no-Reservation cancellation; post-Admission pre-materialization no-call cancellation atomically withdraws every C15 component and closes all unstarted initial/release obligations plus the entitlement/vector; post-materialization/pre-first-formation cancellation closes initial formation/future funding, withdraws daemon output/transient capacity, and retains only Backend Budget/release ownership for C30; queued cancellation invalidates affected candidates and not-started Plans, then asks C17 to atomically rebind/split/merge/close their unstarted many-funder obligations and creates one terminal membership-change formation obligation for surviving members only when needed; in-flight work freezes active obligation scopes, enters Cancel Pending, and defers final daemon-component settlement to C41 after Receipt output enqueue/discard. Only after the cancellation envelope ends may C07 convert a pending survivor obligation. A terminal entitlement stops new funding but remains a tombstone until every linked retained claim expires | Pre-start/post-materialization/queued/in-flight matrix; B1/B4 member removal, all/one/some cancellation, batch split/merge and exact one-credit-per-resulting-call conservation; active no-rebind; all initial obligations, both ledgers, Backend ownership/release, Receipt/output transfer, tombstone expiry, and no early component/vector reuse or conversion-span overlap | <= 400 |
| C29 | `feat(core): reserve per-turn output capacity` | Concrete pre-execution Turn Output Reservation Effect and non-runnable backpressure state | No output Plan without capacity; cancellation releases reserve | <= 380 |
| C30 | `feat(core): release backend request state` | Only after C07 `begin_support` has converted the exact ownership-release obligation to an active charge, emit one ownership-gated exactly-once release Effect without mutating support state; only its accepted Result consumes ownership, partitions the exact Request Backend Allocation Budget, and moves actual allocation to Pending Reclaim | No Effect before active charge, no second/duplicate conversion, obligation exists before materialization, typed zero-ownership close, no daemon-capacity authority, no-call unowned path, partial allocation/use, terminal/cancel/eviction, no retry/unload/early reuse | <= 400 |
| C31 | `feat(core): bound terminal request history` | Count/time Tombstones, connection Request High-Water Mark, and authorized Gone | Evicted, foreign, and never-issued IDs remain distinct | <= 400 |
| C32 | `feat(core): manage exclusive mode leases` | Requested/active/exit states, renewal/expiry/disconnect, shared dispatch pause | No automatic entry; new Admission/Residency Demand reject | <= 400 |
| C33 | `feat(core): authorize exclusive operations` | One operation with conservative peak bound and certified Exclusive Safety Point | Uncertified shared work never silently probes | <= 380 |
| C34 | `feat(core): filter unsafe scheduling work` | Resource Mode, all four generations, exact current candidate-to-Bound-Set association, live member Future Turn Support Entitlements/vectors, and fundable operation-obligation branches before urgency/fairness | Unsafe, stale-generation, stale-Set, missing vector capacity, or unfundable obligation work never reaches selection | <= 360 |
| C35 | `feat(core): account runnable weighted service` | Runnable-only Model Ledger and Device Executor Receipt charging | 1:3 example and idle re-entry without stored credit | <= 400 |
| C36 | `feat(core): compose deadline cost bounds` | Checked intrinsic branch maximum plus one `support_interference(H)` and one `sequenced_event_interference(H)` from the smallest certified horizon satisfying the three-term closure; support counts each operation-scoped call once and includes initial Materialization/Formation, distinct Plan-scoped Receipt observation and conditional continuation formation, alternative rejection/local-stale formation, terminal membership-change formation, post-load/post-observation description revalidation, safety sampling, and release capacity, while event interference combines the static scheduling-cut prefix and mandatory-crossing allowance | Half-open alignment, B1/B4 one observation credit plus one conditional-continuation credit versus unbatched demand, consecutive Turn vector claims, first-obligation path, Receipt update, queued-cancel split/merge, carry-in/out, crossing call, adjacent burst, short-to-long horizon, cut-prefix/crossing maxima, later-event non-overtake/coalesce/defer/fail-stop, multi-horizon, missing horizon, drift; no gap, overlap, per-member double count, online widening, or event ledger | <= 400 |
| C37 | `feat(core): select deadline-aware turns` | Latest Safe Start, Urgent Set, fair fallback, and stable ties | Independent scheduler oracle scenarios | <= 400 |
| C38 | `feat(core): enforce candidate formation laws` | Complete bounded formation after C07 `begin_support` has converted one exact operation-scoped initial, continuation, rejection, local-stale, or membership-change obligation funded by the canonical affected typed claim set, using Admission initial claims before first formation and entitlement-vector claims thereafter and permitting an exact mixed set; same class, frozen call scope/members, exact Key in every member's Authorized Capability Set, identical current Runtime Overhead Generation/Bound Set witness across members, stable Exclusions | Missing/wrong/duplicate obligation, B1/B4 one physical credit, funder/member/claim-variant mismatch, newly eligible-member join, split/merge/cancel before start, sponsor rejection, active no-rebind, conversion-before-predecessor-end, typed causal-impossibility close, missing Exclusion, out-of-set Key, unequal or Backend-supplied Set, and substituted member rejection | <= 400 |
| C39 | `feat(core): apply typed turn results` | Backend Plan Rejection and synchronized Receipt acceptance only. Rejection closes the distinct Plan-scoped observation and conditional continuation obligations and marks the shared rejection formation obligation pending until the completed rejection envelope permits later C07 conversion or typed impossibility closes it. Receipt closes the non-Receipt alternative, retains the distinct observation obligation as pending, and leaves the separately funded continuation-formation obligation conditional; none converts inside Receipt commit. Terminal members stop new funding but retain entitlement tombstones until every linked claim/output owner detaches | Duplicate/late/unknown/stale results, missing/wrong/duplicate obligation, B1/B4 funder membership, two Receipt-call credits versus one non-Receipt-call credit, conditional/pending/active states, terminal/runnable/output/tombstone exclusivity; local stale cannot enter | <= 400 |
| C40 | `feat(core): close local stale plans` | Dedicated Local Stale Plan Event validates mismatch, current qualified intrinsic disposition bound and frozen ceiling, closes the Plan once, closes Receipt obligations, and marks the shared rejection/local-stale formation obligation pending or proves it unnecessary while preserving each still-runnable member's entitlement/vector and emitting Audit plus intrinsic fresh-scheduling Effects without a Backend Result. It never converts an obligation; after the intrinsic envelope ends, C07 may begin required formation | Generation/Set races, duplicate/unknown Plan, entitlement/vector/obligation pending-close identity, B1/B4 funders, unavailable/excessive/over-ceiling fail-stop, no Backend call/Engine Service/Plan Rejection/Receipt, and no conversion/Candidate Formation inside intrinsic span | <= 400 |
| C41 | `feat(core): publish committed staged output` | Ordered publication Effect after Receipt commit; visible Output Sequence advances once. Enqueue converts concrete Turn Output Reservation to output occupancy, while discard releases it. Only after enqueue/discard closes the Receipt envelope may C07 convert the one Plan-scoped observation obligation funded by all frozen members. For terminal members with no in-flight operation, C41 atomically transfers actual output occupancy to connection ownership and withdraws C15 aggregate output/transient components; Backend Budget/release and retained obligation claims remain separate | Earlier cancel discards; later cancel cannot retract; failed enqueue disconnects; B1/B4 one observation call/credit, zero-output and terminal paths, output/request conservation, observation cannot start before publication, active funders frozen, and no daemon-capacity/vector early reuse | <= 400 |
| C42 | `feat(core): commit cost profile updates` | After one separately converted Plan-scoped observation obligation and completed support call, prevalidate Receipt/profile Generation; Commit Barrier mirrors identity, invalidates plans/dirty candidates, and marks the exact bounded post-observation description-obligation set that was pre-reserved before observation pending for C12 or closes it. The accepted result likewise marks the distinct conditional continuation-formation obligation pending or closes it from exact runnable/dirty state, but converts neither inside the observation envelope; only later C07 `begin_support` transitions may start them | Cross-model refresh/no interleaving; B1/B4 funder/scope and distinct observation/continuation credits; unchanged/applied update with runnable/terminal members; bounded description set; required continuation after envelope end; mismatch/no-change/stale/duplicate/malformed/commit-failure; conditional-to-pending/impossible-close conservation, no post-result capacity allocation, fail-stop/counts, and no adjacent overlap | <= 400 |
| C43 | `test(core): replay bounded dual-model prefill` | One Prefill Chunk per fresh Plan with Decode interleaving | Byte-identical fixed-seed transitions and operation counts | <= 380 |
| C44 | `feat(replay): add the bounded core replay driver` | Strict replay input/output over `Core::handle` | Golden, malformed, and repeatability cases | <= 400 |
| C45 | `perf(core): measure release scheduler decisions` | Core-only Release measurements outside serialization and IPC | 100 warmups, >=1000 decisions, one sample per decision | <= 360 |

P0-1 exits only when Core has no Tokio, Protobuf, SQLite, MLX, I/O, async,
callback, or system-clock dependency and the three Core benchmark lanes can be
driven through a thin adapter without embedding their oracle.

### P0-2: Fake Backend And Device Loop

| ID | Commit subject | Behavior slice | Required verification | Target LOC |
|---|---|---|---|---:|
| E01 | `feat(runtime): define the bounded backend interface` | Owned inputs/results, per-call blocking/safe-point declarations, signal view, and affinity token for every coarse operation | Bound completeness and illegal-call-order rejection | <= 400 |
| E02 | `feat(fake): initialize backend identity` | Preverified Bootstrap Manifest plus deterministic Adapter/MLX/interface/complete Capability including Generation Semantics identities and bounded Resource Signal/Operation Sets | Pre-call watchdog, trusted expected Capability, returned-descriptor equality, semantic identity/field/unit/quality/default/operation-bound drift; no Backend Support-Budget authority | <= 380 |
| E03 | `feat(runtime): collect certification environment` | Daemon platform probe joined with exact TurnVector build, build-verified Generation Semantics, and exact Bootstrap/Capability/Resource Signal/externally verified Operation Bound identities; stable installed UMA and OS facts remain distinct from Execution Route and dynamic Resource Evidence | Freshness, unavailable facts, daemon/semantic/descriptor mismatch, refresh, exact fingerprint/cache-key change; Route mismatch cannot be repaired by environment matching, and Configuration/lifecycle descriptors remain separate Case Bound inputs | <= 400 |
| E04 | `feat(fake): describe model registrations` | Deterministic bounded initial and post-load raw Model Descriptor frame plus untrusted ID/hash/vocabulary claims | C10c verifier acceptance, every raw-claim/frame/Manifest drift, current Generation, and capability drift makes Revision Unavailable | <= 360 |
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
| E15 | `feat(runtime): drive effects through the event loop` | Sequenced `Core -> Effect -> EffectResult`; before every fresh Snapshot with a live Timing Commitment, freeze the already admitted event-prefix scheduling cut and fixed mandatory-crossing allowance, measure every allowed non-Backend Core-event envelope against the active static bound, and schedule fresh before later ordinary events; cover first runnable plan, Receipt, Plan Rejection, local stale, support Result, cancellation membership change, and idle re-entry; exact Runtime Overhead Generation and Set check immediately before `execute_turn`; mismatch becomes C40 local stale. A pending Support Operation Obligation cannot enter C07 `begin_support` until the Receipt path has completed C41 enqueue/discard or the rejection/local-stale/cancellation/previous-support envelope has otherwise ended | No inline continuation or direct Snapshot bypass; stable Effect order; zero/nonzero prefix, finite crossing counts, later-event non-overtake/coalesce/defer/fail-stop, every event and scheduling trigger classified, local stale closes once without Backend result, predecessor-end before support conversion, and event/disposition excess fail-stops | <= 400 |
| E16 | `feat(runtime): measure turn-path runtime overhead` | Enforce the Core-supplied exact Bound Set and measure Receipt pre/Engine/post versus Plan Rejection pre/call/intrinsic-replanning partitions over the working Event Loop without deriving applicability or Set authority; end intrinsic replanning before Candidate Formation | Exact evidence/generation/local-stale identity; both Backend branches complete and disjoint from support/event envelopes; measured Turn excess preserves Receipt then drives C24 quarantine and C23 invalidation; drift invalidates before call; no stale-Set authority, construction, widening, Candidate-Formation overlap, or Ledger charge | <= 400 |
| E17 | `feat(runtime): qualify lifecycle overhead` | Sole daemon selector: verify B03-B05's embedded Catalog/build tuple, Catalog Retention Horizon, Support Start Count Bounds, lifecycle-reserve maxima, and event registry/cut maxima; collect stable platform facts, select one exact Configuration entry, run pre-activation Lifecycle Overhead Qualification against every returned Backend descriptor at startup/before successor publication, and return one immutable witness binding all inputs plus selected Support Budget, Sequenced Event Interference Bound, and Stale Plan Disposition Bound. At startup activate that witness and have C08b create the first pre-ready safety-sample obligation before Service Readiness; for a successor give the witness to C26/C27 without Core external matching | Catalog/build/evidence/config/platform/expected-or-returned descriptor/count/lifecycle-reserve drift, startup/successor sole selection, witness generation/current identity, first pre-ready sample obligation before freshness, no Core selector/request authority/cache, all support/event kinds, illegal state/operation/event, and unclassified rejection | <= 400 |
| E18 | `feat(runtime): measure complete support envelopes` | For optional ordinary support, begin with one C08a transition that atomically creates a typed Ordinary Reservation Claim, matching one-operation obligation, legal credit, and active record; for committed work, use standalone C07 `begin_support` after the complete predecessor envelope to move exactly one pending Admission-, many-entitlement-, post-load/post-observation-description-, or safety-trigger-funded Support Operation Obligation and its one physical credit into an active charge immediately before preparation; then measure conversion/prepare/direct-call/validate/Core/immediate-Effect work while retaining the independent Backend-call watchdog | Every result branch complete/disjoint; optional claim nonempty/type/scope and ordinary exhaustion before Effect; exact-window/one-past and consecutive-Turn vector claims; B1/B4 one call/credit, including separate observation then conditional continuation whose credit, claims, vector slots, and applicable-horizon headroom remain occupied before pending; split/merge/rebind before start, active frozen; no Effect without active obligation; predecessor publication/rejection/local-stale/cancellation/observation/load/trigger ordering; first Materialization/Formation; Receipt update; queued membership change; post-load/post-observation description; first/next safety sample; release distinct; conditional-to-pending-or-close, pending/convert/impossible-close, full-bound no-refund, watchdog/envelope fail-stop, and no Event Loop ledger | <= 400 |
| E19 | `feat(runtime): integrate owner-thread interference and carry` | Drive support deferral, start-count/vector/obligation enforcement and interference from the C18-complete sole ledger and scheduling interference from E15's static event cut; consume E17's immutable same-or-replacement qualification witness without a second selector, pause ordinary support, drain request liability under C25, compute C26's zero witness, atomically revalidate mutation/carry tokens, Support Ledger Generation, next/headroom before owner creation, and revalidate witness plus carry at Commit Barrier entry; route bounded dual-Budget operation/vector/lifecycle/mandatory/safety suballocations through carry, close unowned carry and resume ordinary support on every nonfatal pre-owner failure, and deliver exact witness/results to C27 | Multi-request/consecutive-Turn vector and operation capacity, B1/B4, sequential churn/count edges, description/safety lifecycle reserves, full-ledger carry, adjacent/multi-horizon/pool, short-to-long successor; selector/qualification/capacity/generation/token drift before owner restores support; cancel/exhaustion/absence/degraded/indeterminate, conditional/pending/active/tombstone transfer, unchanged/changed Configuration, TOCTOU, request-liability closure, no live-request Store/Audit pause, reset/refund/borrow, event ledger, second selector, or second support ledger owner | <= 400 |
| E20 | `feat(runtime): share cooperative control signals` | External atomic setters and Backend safe-point read view | Cancel/Critical/lease/shutdown before and after safe point | <= 360 |
| E21 | `feat(runtime): fail stop an indeterminate backend operation` | Watchdog escalation, no fabricated Result/Receipt, whole-process termination hook | Missed safe point or post-profile-mutation commit failure cannot restart only the owner thread | <= 400 |
| E22 | `test(runtime): replay executor failure boundaries` | Duplicate/stale result, profile divergence, forced stop, and deterministic Fake crash traces | Stable final hashes and no operation replay or continued divergent Backend | <= 380 |
| E23 | `feat(fake): shut down backend state` | Final exactly-once owner-thread operation returning zero-ownership Shutdown Result and empty-handle typestate | Readiness false, no in-flight/release ownership, destroy order, bound/safe point, no retry/fabrication/bypass/Clean Shutdown Boundary authority | <= 380 |
| E24 | `test(runtime): enforce backend conformance fixtures` | Shared Fake runner for Bootstrap/initialization/Capability/Generation Semantics/Signal/Operation descriptors, Catalog/Lifecycle qualification, C10c verification plus C10d sealed post-load Model Descriptor equality and Request Description refresh, full support envelopes/lifecycle states, ownership, CAS, transitions, release, and shutdown | Trusted semantic/build/Catalog/descriptor identities, raw-claim rejection, call order, pre-ready/recovery/drain no-call matrix, local-stale independence, release-before-unload, empty-handle-only raw deallocation, Core-owned sliding budget and watchdog split, result-gated Clean Shutdown Boundary/fail-stop | <= 400 |

### P0-3: Resource Governor And Residency

| ID | Commit subject | Behavior slice | Required verification | Target LOC |
|---|---|---|---|---:|
| G01 | `feat(resources): sequence backend resource samples` | Sole safety-trigger owner over Signal Contract-bound allocator/cache provenance and freshness: after E17/C08b's first obligation, request the next C08b safety-sample obligation from the pre-reserved nonborrowable schedule before evidence expiry, then consume only its typed Result; owns no credit/ledger mutation | First pre-ready and later normal/degraded/recovery/drain samples; exact trigger/obligation/result identity, early/duplicate/stale/missing, shutdown-impossible close, no post-expiry allocation, no ordinary borrowing, and contract mismatch | <= 380 |
| G02 | `feat(resources): sample macos process and vm state` | `phys_footprint`, available memory, swap, and compressor via native APIs | Typed unavailable/overflow and no-shell assertion | <= 400 |
| G03 | `feat(resources): observe macos memory pressure` | Bounded dispatch pressure source and monotonic sequence | Normal/warning/critical transitions and teardown | <= 360 |
| G04 | `feat(resources): assemble complete resource evidence` | Join Backend, process, VM, and pressure sources without erasing provenance | Independent freshness and out-of-order rejection | <= 400 |
| G05 | `feat(governor): classify resource modes` | Normal, Guarded, StopAdmission, Critical, hysteretic recovery | Immediate restriction and dwell-bound recovery | <= 400 |
| G06 | `feat(governor): activate resource configuration` | Atomic Threshold Profile, eviction rank, wait, and residency-limit propagation from the complete successor | Stricter profile advances Safety Generation; looser recovery obeys hysteresis | <= 400 |
| G07 | `feat(governor): integrate request capacity policy` | Feed current Resource Mode and checked resource-capacity limits into C15, then use its existing Resource Capacity Ledger for typed Request Backend Allocation Budget, daemon output, and transient-headroom decisions; Governor creates no second admitted-capacity ledger or mutation path | Policy-versus-ledger authority, stale capacity witness, conservation, overflow, rollback, and no owner migration | <= 400 |
| G08 | `feat(governor): reserve residency before load` | Separate Model Descriptor-based Residency Reservation plus requirement for C08b's bounded post-load Model/Request-description obligation set before any load Effect | No load without both reservations; exact waiter/model bound; cancel-before-start makes no Backend/description call and rolls both back | <= 400 |
| G09 | `feat(runtime): coordinate residency demands` | Shared loads/waiters/FIFO/timeout/rollback; before load, obtain C12's bounded revalidation scope and C08b's nonborrowable operation-obligation set; failure/cancel closes it, successful load advances Generation and marks the exact set pending for C12 | Resident cross-model reissue, exact post-load model+request set/credits, unrelated Warming defer, current loaded waiters before Admission, failure/impossible close, no post-success allocation, drift scopes Unavailable | <= 400 |
| G10 | `feat(governor): gate shared residency transitions` | Conservative blocking bound must fit every current timing budget or wait for explicit Exclusive Mode | No automatic Exclusive entry and no blocking shared transition | <= 400 |
| G11 | `feat(governor): bound residency activity` | Configured transition frequency and aggregate Residency Service occupancy | Window rollover, saturation, and checked arithmetic | <= 380 |
| G12 | `feat(runtime): protect resident model leases` | Lease acquisition/release and unload precondition over zero request ownership/pending release | Active lease or any request ownership prevents unload | <= 360 |
| G13 | `feat(governor): select ordinary reclaim actions` | Bounded cache-reclaim Residency Transition then deterministic idle-model victims independent of Model Weight | Transition gating, no immediate free-capacity claim, stable tie, and protected-victim cases | <= 380 |
| G14 | `feat(governor): perform critical eviction` | Select by Eviction Rank/ties, route every affected request through C28's typed `critical_eviction` cancellation/terminal matrix, wait for C41 in-flight output settlement and C30 ownership release, then unload; Governor never mutates either Core ledger | Cache/idle exhaustion, queued and in-flight paths, candidate membership change, release-before-unload, release fail-stop, no forced interruption or second capacity owner | <= 400 |
| G15 | `feat(governor): retain only observed reclaim charges` | Consume the already committed C15 allocation and Pending Reclaim facts produced solely by C22/C30 without reinterpreting a request Backend result or mutating its Budget partition; separately apply typed rolled-back-load or successful-unload partitions to Governor-owned Residency Reservations, and keep every proven actual allocation charged until converged Resource Evidence | Core-versus-Governor authority, zero/partial request facts, load rollback, use/unload/cache, convergence/stall, no second request-result parser or ledger mutation path, and no premature reuse | <= 400 |
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

The first Certified Execution Profile qualification target is Dense
`mlx-community/Qwen3-0.6B-4bit@73e3e38d981303bc594367cd910ea6eb48349da8`
on Mac Studio `Mac15,14`, Apple M3 Ultra with a 60-core GPU and 256 GB unified
memory, macOS `26.4.1` build `25E253`, and Metal 4 support, for Decode B1
context 512 and Prefill B1 chunk 64. Its baseline Execution Route uses the exact
exported graph plus one preallocated memory arena with stable offsets. The
kernel bundle, fusion plan, and non-paged KV/cache layout use exact baseline
identities, and the Attention Path is exactly `CONTIGUOUS_MLX_SDPA`;
Speculative Decode and command replay are explicit `NONE`, and the Prefix Reuse
plan kind is `NONE`. These values define a qualification target, not completed
Certification:
the environment and every bound require a fresh run, and P-1A remains `YELLOW`,
P-1B `PENDING`, and P-1C `RED`.

| ID | Commit subject | Behavior slice | Required verification | Target LOC |
|---|---|---|---|---:|
| N01 | `build(native): add the versioned C shim` | Pinned MLX, locked Generation Semantics descriptor/hash, embedded Manifest, CMake/Ninja, opaque typestate handles, identity/error translation, bounded nonthrowing empty-shell deallocator | Locked descriptor/Manifest equality, compile/SHA/ABI; live/indeterminate rejects; shell has no MLX object and only rolled-back-init/post-Shutdown deallocates | <= 400 |
| N02 | `feat(native): initialize on the executor owner thread` | Pre-call Manifest watchdog, MLX/stream creation, exact Adapter/MLX/interface/complete Capability including Generation Semantics identities and Resource Signal/Operation Sets | Semantic/other drift fail-stop; only trustworthy zero-root failure deallocates; timeout, missed rollback, or success-with-drift never destroys live state | <= 400 |
| N03 | `build(native): pin the graph export toolchain` | Ignored task Python, exact mlx-lm/MLX/dependency lock, model fetch/hash wrapper | Missing/drifted tool/model and offline replay manifest | <= 380 |
| N04 | `build(native): define the exported graph abi` | Canonical exact-shape signatures, logits/new-KV outputs, artifact manifest/import contract, and baseline Execution Route descriptor binding the exported graph, fixed memory plan, exact non-paged KV/cache layout ABI, and every absent optional plan | Double-export and Route-descriptor byte identity, explicit `NONE` optional-plan members, Python round-trip harness, and one-member drift changes Route identity | <= 400 |
| N05 | `feat(native): implement the graph importer` | Bounded C++ Direct importer for verified Graph ABI signatures and complete manifest | No production import outside Residency Effect; deterministic tiny-fixture parity and drift rejection | <= 400 |
| N06 | `build(native): export qwen3 dense graphs` | Complete Qwen3-0.6B graph recipe for certified Decode/Prefill buckets | Exact revision, two exports, Python/C++ logits/KV parity, no tracked artifact | <= 400 |
| N07 | `build(native): export qwen15 moe graphs` | Complete Qwen1.5-MoE graph recipe including top-k routing for certified buckets | Exact revision, two exports, Python/C++ logits/KV parity, no tracked artifact | <= 400 |
| N08 | `feat(native): verify registered model artifacts` | Canonical Artifact Root, Execution Route descriptor, and typed File Hash checks before and after load | Ad hoc path, mutation, Route drift, and Revision Unavailable cases | <= 400 |
| N09 | `feat(native): own logical model runtime capsules` | Per-Model imported graph/KV/cache capsules plus the baseline preallocated arena and fixed-offset memory plan on the single owner thread | Isolation, arena lifetime/offset, no raw GPU-address promise, and lifecycle tests | <= 380 |
| N10 | `feat(native): describe model registrations` | Generate the bounded raw V1 Model Descriptor frame and untrusted ID/hash/nonzero-vocabulary claims including exact Execution Route Identity from the registered graph manifest/capability before registration and after load | Nonresident operation; C10c verifier acceptance; raw claim, frame, Manifest, and Route drift; C10d sealed post-load equality; drift marks Revision Unavailable | <= 400 |
| N11 | `feat(native): expose cooperative signal views` | Read-only atomic signal view and declared safe-point helper before any cancellable operation | Cross-thread setter ordering and stale-view rejection | <= 360 |
| N12 | `feat(native): transition model residency` | Serialized load with strong rollback, ownership-gated unload, and bounded allocator-cache reclaim using typed results | Success identity/ownership; zero/partial/cancel rollback split, release-before-unload, unchanged reclaim, fail-stop | <= 400 |
| N13 | `feat(native): describe token requests` | Stateless validation of frozen Token Request plus C10c-verified Model Descriptor/current Backend Generation and finite route-bearing Capability Requirement Set generation | Initial/post-load generation and sealed descriptor binding plus complete phase/batch/Shape/Route matrix; no Core refresh, Envelope, registry, or allocation | <= 400 |
| N14 | `feat(native): materialize admitted requests` | Resident-model handle/KV/Sampling State with synchronized complete/partial/zero-ownership exact Request Backend Allocation Budget result | Identity/conservation; no daemon-capacity authority; zero-ownership rollback; partial failure requires release | <= 400 |
| N15 | `feat(native): release request state` | Owner-thread destruction with trustworthy exact Request Backend Allocation Budget split only when opaque ownership exists | Unowned no-call; no output/transient/timing authority; terminal ownership retains Budget; uncertain cleanup fail-stops and blocks unload | <= 400 |
| N16 | `feat(native): form costed native candidates` | Canonical Batch/Shape compatibility, exact route-bearing authorized Key, Cost Profile version/estimate, typed Exclusions | Every member Set contains the same Key and Execution Route Identity; Fake/native parity | <= 400 |
| N17 | `feat(native): compare and set cost profiles` | Accepted Receipt-bound version returns mismatch, unchanged, or atomically applied Backend-owned profile under Commit Barrier | Core mirrors identity once; no interleaving; malformed/post-mutation failure fail-stops; no widening | <= 400 |
| N18 | `feat(native): sample per request deterministically` | Implement the exact Generation Semantics descriptor's binary32 tensor flow, compact nonempty Top P/Top K support, max-shifted Temperature scaling, greedy, `key(seed) -> split(state)` transitions, and a qualification-only tensor seam outside the production Backend Interface | NaN, both infinities, and finite-to-nonfinite-cast fail before split/state/output; identity-bound shared subnormal/no-crossing/equal-logit/survivor-below-K/smallest-Temperature/zero-uniform compact-index and multi-token vectors; hidden-stop and Dense/MoE B1/B4 invariance | <= 400 |
| N19 | `feat(native): apply stop and output limits` | Longest ambiguous stop suffix retention, hidden matched tokens, exact Max Output terminal | Prefix overlap, cross-Turn match, cancellation, and limit cases | <= 400 |
| N20 | `feat(native): execute synchronized decode turns` | Bounded Decode on the exact resident Execution Route with staged visible token output | Dense/MoE output parity, Route/Plan equality, signals, and cleanup | <= 400 |
| N21 | `feat(native): execute one prefill chunk` | One exact resident Execution Route range within Plan target/ceilings, synchronized continuation | Dense/MoE parity, Route/Plan equality, signals, and no hidden loop | <= 400 |
| N22 | `feat(native): execute exclusive operations` | Conservative resource-bound operation with certified periodic safety points | Lease/Critical cancellation and rollback | <= 400 |
| N23 | `feat(native): report backend allocator evidence` | Support-budgeted MLX active/cache samples bound to Signal Contract identity with quality and source sequence | Blocking/safe-point bound, contract mismatch, telemetry availability, error mapping | <= 360 |
| N24 | `test(native): expose qualification-only numerical hashes` | Test-build output/logits/KV hashes outside serving Interface | Prove no qualification DTO in Core or production ABI | <= 360 |
| N25 | `feat(native): shut down backend state` | Final exactly-once owner-thread destruction, validated zero-ownership Result, then empty-shell deallocation | Preconditions/order/safe point; live raw destroy rejects; failure has no retry, deallocation, Clean Shutdown Boundary, or reclaim claim | <= 400 |
| N26 | `test(native): pass full backend conformance` | Fake/native Bootstrap/Capability/Signal/Operation descriptors, C10c raw-claim verification and C10d sealed post-load equality, post-load Generation refresh, exact Execution Route authorization, support bounds, ownership, profile CAS, transitions, Exclusive, shutdown | Trusted identities/fixtures; one-field descriptor/Route drift and raw bypass reject; unavailable evidence never becomes synthetic pass or Clean Shutdown Boundary | <= 400 |

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
| P24 | `feat(daemon): reject closed mutation cancellation` | Ordered Control Plane pre-Core handler over the active `{Daemon Instance ID, Operation ID, checked barrier generation}` gate after authorization/validation and cancellation-ingress plus Direct Response reservation; exact tuple returns `cancel_window_closed`, every other tuple returns opaque `commit_in_progress` | Same-socket ordering, ABA/unknown/different identity flood during barrier and post-C27 Audit Degraded, identity nondisclosure, both responses no-Core/no-Audit/no-headroom, write/disconnect reservation release, no retained/replayed command, Data Plane disconnect separation | <= 380 |

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
| S04 | `feat(store): encode immutable model registry rows` | Transaction-scoped Manifests, C10c-sealed complete Model Descriptor frame/ID/hash/vocabulary, Aliases, lifecycle, canonical roots, and typed hashes with no commit/pointer API | Same-verifier frame/ID/vocabulary/hash round-trip, 256 registry and 4,194,304-byte arena limits, corruption rejection; caller rollback leaves live Store unchanged | <= 400 |
| S05 | `feat(store): encode configuration snapshot rows` | Transaction-scoped complete validated Configuration successor with no commit/pointer API | Semantic/generation hashes and caller rollback; current generation cannot change | <= 380 |
| S06 | `feat(store): encode certification record rows` | Transaction-scoped immutable record/evidence and Execution Route identities plus finite Coverage Manifest references with no activation Interface | Candidate round-trip, explicit `NONE` Route members, invalid reference rejection, current generation unchanged, and no precompiled Profile authority | <= 400 |
| S07 | `feat(certification): compile the authorization index` | Offline verified Coverage Manifest to a read-only exact-key index whose entries are finite Certified Execution Profiles | Missing reference, one-field Route/environment drift, wildcard, runtime range inference, and dominance-proof expansion cases | <= 400 |
| S08 | `feat(certification): prepare certification successors` | Validate candidate record/index replacement, explicit recertification evidence, and bounded Profile Revalidation plan without persistence or activation | Quarantine remains until unified mutation activation; changed bounds identify exact descriptions/candidates | <= 400 |
| S09 | `feat(daemon): hold the instance lock` | One installation-fixed absolute descriptor identity outside every Runtime, acquired before Runtime-specific locks/writers and held from Bootstrap until OS process termination with no explicit unlock path | Same- and cross-Runtime-ID competing processes remain blocked through the last live instruction and latched read-only custody | <= 360 |
| S10 | `build(audit): lock the p0 audit schema` | Bounded payload `.proto`, append-only nonzero record-kind registry, canonical descriptor lock, Audit Registry Identity | Double generation, golden payloads, unknown/reserved/reused kind rejection | <= 400 |
| S11 | `feat(audit): compute sequence reserves` | Build-time checked Terminal Sequence Reserve plus P0 Control Mutation Sequence Headroom consuming C25's exhaustive registered maxima, Effect creation, attempt x noncreating discretionary-interleaving limits, monotonic per-object mandatory safety edges with duplicate-signal coalescing, and the larger failure/success Result branch | C25 registry projection is complete; binary maxima, no aggregation/borrowing, repeated raw signals assign no event, Runtime Closure Gate prevents downstream fanout, checked overflow, exact edge values, Sequence Exhausted | <= 400 |
| S12 | `feat(audit): encode bounded chained records` | Framing, sequence, registry identity, checksum/hash link, content exclusion | Golden, oversize, corruption | <= 380 |
| S13 | `feat(audit): run one bounded audit writer` | Ordered guarded ordinary queue plus build-derived nonborrowable Audit Safety Reserve count/bytes aligned with Core Event/Terminal Sequence maxima; strong file/namespace barriers; pre-barrier failures are Audit Degraded and required-barrier failures feed S02 | Ordinary saturation cannot consume safety capacity; count/byte maximum edge, append, file/parent sync, classification, latch, and no Event Loop direct I/O | <= 400 |
| S14 | `feat(audit): fence durable predecessors` | Typed sync-through operation drains all assigned records and returns exactly granted, stale, audit-degraded, or storage-barrier-failure; only granted carries a session-local Predecessor Fence | Assigned-unsynced drain, stale target/head/generation/intervening work, pre-sync degradation, actual file/parent sync failure | <= 380 |
| S15 | `feat(store): reserve p0 audit sequence ranges` | Sole post-genesis writer of SQLite P0 Audit Sequence State; daemon-owned unsequenced/no-ID/no-Effect/no-Audit primitive whose guarded `FULL` commit plus database-parent sync publishes high-water and can prove full S11 mutation headroom before assignment on the single Store executor | Exact headroom/range edge; known absence versus indeterminate latch; absent/exact/third-state custody at commit and parent crash points; no unchanged claim, pending envelope, mutation high-water update, terminal borrowing/reuse, or Anchor bytes | <= 400 |
| S16 | `feat(store): initialize p0 control and audit` | While holding installation-scoped S09, acquire the qualification lock, require the Policy-selected U03 record to be Head-selected, and hold that lock through commit-plus-parent publication of its immutable binding with daemon IDs, empty registries, version-one State, genesis sequence state, pending sequence-one Epoch Open, and exact Audit root | Lock order has no reverse path; wrong/stale/orphan record rejects before identity; crash leaves absent or one exact non-ready identity/pending custody; required-barrier failure returns `initialization_outcome_indeterminate` without a mutation token | <= 400 |
| S17 | `feat(store): validate p0 control mutation intents` | Bounded complete successor validation/token check while predecessor remains active, producing no Operation ID or owner | Stale/duplicate/busy/invalid successor rejects without ID; no external publication or daemon-only owner | <= 380 |
| S18 | `feat(store): fence p0 control mutations` | Prove full S11 headroom, atomically revalidate the complete mutation token, exact C26 carry token, current Support Ledger Generation, current next value, and headroom, then allocate daemon Operation ID plus Core Effect/owner/headroom charge and close C25's Runtime Closure Gate until terminal; every nonfatal pre-owner mismatch closes the unowned carry and resumes ordinary support with no ID. Drain every request-liability component through registered nonincreasing edges before barrier entry; enforce bounded interleaving and state-edge-deduplicated safety; pre-barrier cancel or stale-attempt exhaustion returns one typed terminal Result through C27 so it also consumes C26's carry token, while Audit Degraded retains stage/charge/token without a Core Event; granted protects Store-result/conditional-completion slots, Commit Barrier publishes P24's exact checked-generation Cancel Gate, and S18 clears that Event Loop gate immediately after accepting C27's typed terminal disposition in the same handling step, with no intervening dequeue or Core transition; C27 stops discretionary assignment | Capacity/qualification/mutation-token/carry-token/Support-Generation/next/headroom drift before owner has no ID/token leak and restores ordinary support; assignments conserve charge; C25 closure/quiescence, raw-safety coalescing, cancel/exhaustion no-activation/no-double-release, degraded exact-token resume, no live-request Store/Audit pause, gate publication-before-I/O and persistence through C27/degraded/failure, cancel-flood reserve conservation, Core-versus-Event-Loop gate ownership, gapless disposition integration, and terminal clear, fail-stop no Result, and no post-C27 ordinary work/terminal borrowing | <= 400 |
| S19 | `feat(store): publish p0 control mutations` | Inside S18, use the sole guarded post-genesis Control transaction for candidate/pointer/pending envelope without sequence-state change; trustworthy exact absence returns the pre-sequenced nonpublication Result, exact committed success drives C27 activation/owner advance, and indeterminate returns no Result | Absent closes without activation/dependent Effect; committed success cannot release owner; no helper authority, unsequenced callback, overtaking, early activation, fabricated result, or unstable third-state classification | <= 400 |
| S20 | `feat(store): complete p0 mutation audit` | With the Runtime Closure and Control Mutation Cancel Gates closed and discretionary dispatch deferred, execute C27's dependent exact-envelope completion Effect: append/sync/verify fenced bytes, guarded `FULL` pending-clear commit plus parent sync, then return one ordinary sequenced Result whose accepted C27 transition releases owner/headroom, reopens C25's Runtime Closure Gate, and returns a typed completion disposition; S20 consumes that disposition, clears the Event Loop-owned Cancel Gate in the same handling step with no intervening dequeue or Core transition, and only then acknowledges success, while append failure retains Effect/stage/charge and the originating Control Plane disconnect ends only delivery | Protected final slot under maximum safety edges, same-ID exact-Effect resume, exact and nonmatching mutation cancels remain bounded no-Core/no-Audit/no-headroom Direct Responses, Data Plane disconnect still cancels owned requests, no rollback, Store replay, or unsequenced callback; Core-versus-Event-Loop gate ownership and gapless disposition integration, only actual durability failures indeterminate, and no regeneration/early release | <= 400 |
| S21 | `feat(audit): retain bounded p0 segments` | Segment boundaries, synchronized retention boundary, garbage eligibility | Capacity, file/parent barrier, latch, and reclaim failures | <= 380 |
| S22 | `feat(daemon): order policy-first bootstrap` | Installation Policy before socket, lock, or Runtime authority; construct S02, acquire installation-scoped S09 before every Runtime-specific lock/writer, then freeze and validate the exact Policy-selected U03 record against the live volume/build/Profile before opening a Runtime writer | Syscall/byte ordering across fresh/replacement Runtime IDs, missing/foreign record, no implicit Head selection, and no pre-lock/pre-guard/pre-qualification Runtime writer | <= 400 |
| S23 | `feat(daemon): classify interrupted p0 publications` | Under the frozen record proof, classify initialization, range, mutation, and pending-clear separately; repeat only missing parent sync; range has no pending envelope and no path recovers a Fence | Epoch-Genesis init, no-envelope range, persisted-predecessor mutation, separate pending-clear fixtures; binding mismatch/third state fail closed; no regeneration or reuse | <= 400 |
| S24 | `feat(daemon): reconcile pending p0 audit` | After S23, complete initialization only against Epoch Genesis Hash; append mutation only at its persisted predecessor or verify its exact in-chain record; then clear before tail | No session-local Fence input; already-present exact record, predecessor mismatch, every Audit/clear crash point, malformed/multiple pending reject | <= 400 |
| S25 | `feat(audit): reconcile the p0 journal tail` | Only with no pending envelope, verify Clean or append one Crash Tail closing every never-assigned and assigned-but-unsynchronized-lost value through old high-water | Both lost classes, pending refusal, truncation/mismatch/sequence exhaustion, no reconstruction/reuse/Anchor fallback | <= 400 |
| S26 | `test(daemon): integrate storage barrier custody` | Exercise S02 against every real SQLite, Audit/file, parent-sync, Bootstrap, retention, and shutdown barrier while retaining the OS-only lock | Device/write stop, first observation stable, authenticated status only, no marker/Clean/retry, next process alone reconciles | <= 400 |
| S27 | `feat(daemon): restart with empty inference state` | Rebuild Control State including complete registered Model Descriptor frames only after S23-S25; run the same C10c verifier and exact C10d sealed-value equality before readiness; no request/KV/Residency restoration, normalization, or descriptor regeneration | Initialization/mutation three-boundary and clean restart; missing/malformed/mismatched frame, ID, typed hash, or vocabulary enters Control Repair Mode; post-restart Top K uses verified restored vocabulary | <= 360 |
| S28 | `feat(daemon): enforce the process reclaim barrier` | After any predecessor initialized Backend, acquisition of the unchanged installation-scoped lock proves that process exited for graceful same-ID restart, Daemon Failure, and cross-Runtime replacement; sampler then establishes a fresh post-exit baseline | Real clean same-ID, failed same-ID, and cross-Runtime-ID parent/child lifetime tests; Shutdown Result/Clean Boundary and lock-before-exit samples reject; no acquire-before-exit, new path/generation, Fake/time/socket substitute | <= 400 |
| S29 | `feat(daemon): gate p0 service readiness` | Require the installation-scoped lock, exact frozen/bound Storage Qualification Record equality, Store/registry schema, reconciled prior bytes/Audit, Device Executor, Adapter, exact verified B03-B05 daemon/Catalog tuple, current E17 Lifecycle Overhead Qualification and its active Support/Event/Stale descriptors, fresh environment/Resource Evidence, and the Process Reclaim Barrier after every predecessor Backend initialization; current-session latch remains non-ready | Every successor may become ready only after exact forward reconciliation, current Configuration/returned-descriptor qualification, and required post-exit/acquisition-of-the-same-lock Resource Evidence; true first genesis is explicit; clean same-ID, failed same-ID, and cross-Runtime cases reject new lock path, pre-lock sample, logical shutdown proof, missing/drifted lifecycle evidence, and binding drift | <= 400 |
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
metric reduction, or synthetic success. B03-B05 implement deterministic tooling
and typed test-target fixture catalogs but do not claim a production-qualified
daemon at those early commits; a production target has no applicable Catalog and
cannot become Service Ready before L01. After K05, L01 uses the final runtime
source closure to build a payload-independent candidate, import exact ignored
external lifecycle evidence,
compile and embed the production Catalog, and prove that final code identities
are unchanged. L02 then compiles the independently sourced, pre-frozen request
Certification set against that outer build; its records cannot feed back into the
Catalog or either build identity. L01, L02, and Q00 may be implemented while the
paired contract is pending; a lane commit runs only after the separately authorized
Benchmark
revision is fixed and recorded. Qualification adapters are separate launcher or
test targets outside the production daemon's runtime source closure and cannot be
linked into or conditionally enable code in that daemon. Production instrumentation
needed by a lane must already exist before L01. Any later fix that changes the
runtime closure invalidates L01, its outer build, request Certification evidence,
and all later lane results, and returns the sequence to L01 before qualification.

| ID | Commit subject | Behavior slice | Required verification | Target LOC |
|---|---|---|---|---:|
| L01 | `build(release): finalize the qualified daemon build` | Orchestrate B03-B05 over the final dependency-traced runtime source closure: build the payload-independent candidate, verify/import exact external lifecycle-operation, sequenced-event, and local-stale evidence under ignored artifact custody, compile the finite production Catalog, embed it, verify unchanged Core code identities, and emit a compact outer-build manifest | Missing/stale/wrong-Core evidence, source/dependency/native drift, retention/event/record/token Catalog capacity, candidate/final code-section difference, embedded tuple, two clean reproductions, ignored bulky evidence, and no request Certification self-reference | <= 400 |
| L02 | `build(release): freeze request certification inputs` | Use S07's offline compiler with independently sourced immutable Adapter, Model Revision, Environment Qualification, Case Bound, and Coverage evidence to emit the finite pre-frozen Certification Record/index inputs for the exact L01 outer build under ignored bulky-evidence custody | Exact outer-build binding, every evidence hash, finite coverage, two clean reproductions, missing/inapplicable/drifted evidence, no Catalog/build-identity input edge, and compact manifest | <= 360 |
| Q00 | `build(benchmark): add the qualification launcher` | Pin Benchmark HEAD/expectation/cert/fixtures in ignored config; enforce clean-before/after | `scripts/qualify.sh inspect`, missing/stale contract failure | <= 360 |
| Q01 | `build(benchmark): add the subject handshake` | Shared Subject hello, supported lanes, build/dependency/environment identities, Data Plane descriptor, artifact roots | Hello precedes every case; containment and identity tests | <= 380 |
| Q02 | `feat(benchmark): expose core replay qualification` | `core-event-replay` adapter over real Core | Lane run and raw Transition evidence | <= 340 |
| Q03 | `feat(benchmark): expose scheduler policy qualification` | `scheduler-policy` adapter | Oracle remains Benchmark-owned | <= 340 |
| Q04 | `feat(benchmark): expose scheduler performance qualification` | After the independent Benchmark blocker adds its new schema/suite/runner/oracle/gates, implement the `scheduler-performance` adapter with Release samples and exact build/Catalog/qualification-witness/applicability/Configuration/Runtime Overhead Generation/Bound Set/local-stale/event-cut/two-Core-ledger identities | Old `measure-release-core` rejects first; IPC excluded; Turn/support/event partition; atomic two-ledger Admission/terminal settlement; finite request vectors with same-horizon consecutive Turns and later-Admission non-steal; optional ordinary typed claim and exhaustion; B1/B4 one credit for each actual observation, conditional-continuation, or non-Receipt operation, unbatched bound, initial/entitlement/lifecycle/ordinary claim variants, mixed first+continuation formation, newly eligible-member join, no sponsor, split/merge/member-cancel/rebind and active freeze; Support Start Count exact-window/one-past/sequential churn; initial obligations; three distinct Plan-scoped operation obligations, conditional-state credit/vector/horizon/carry occupancy, and conditional-to-pending-or-close Result transition; post-load/post-observation description under maximum Warming/Preparing+registry cardinality and first/next safety obligations with exact exhaustion/recovery/drain; mandatory/safety nonborrowability; predecessor-before-`begin_support`; lifecycle matrix; daemon sole qualification selector; carry/pre-owner drift and all terminal outcomes; adjacent/multi-horizon/short-to-long; scheduling cut; three-term single-count Deadline Cost, no-widening, and Model Ledger exclusion | <= 400 |
| Q05 | `feat(benchmark): expose request lifecycle qualification` | `request-serving-lifecycle` through production Data Plane including parameter rejection and cancel/output/release races | Locked production descriptor, Service Class mapping, and real process identity | <= 400 |
| Q06 | `feat(benchmark): expose native correctness qualification` | `mlx-native-correctness`, reproducible full Dense/MoE graph manifests, B1/B4 omitted/zero/nonzero Seeds, complete greedy/categorical extreme-parameter and stop/Max Output matrix, external hashes | Shared NaN/infinity/finite-cast-overflow fail-before-split/state/output plus exact-tensor/subnormal/no-crossing/survivor-below-K/smallest-Temperature/zero-uniform compact-index/key-state vectors, omitted-seed replay, Python/export/import parity, batch invariance | <= 400 |
| Q07 | `feat(benchmark): expose bounded turn qualification` | `bounded-turn-and-ffi` over production native boundary | One-chunk Prefill, Exclusive Safety Point, signal cleanup | <= 380 |
| Q08 | `feat(benchmark): expose governor qualification` | `residency-and-memory-governor` with real system samples | Reservation, Residency Service limits, Critical eviction, reclaim | <= 400 |
| Q09 | `feat(benchmark): expose cross-model qualification` | `cross-model-serving` through production Data Plane | Timing, fairness, progress, throughput, output evidence | <= 400 |
| Q10 | `feat(benchmark): expose observability qualification` | `observability-qualification` with honest P-1A quality | No command-buffer fairness overclaim | <= 360 |
| Q11 | `feat(benchmark): expose persistence qualification` | P0 exact qualification binding, initialization-specific indeterminate response, Control/Audit init, unsequenced range reservation, separate sequence/current-pointer authorities, build-derived headroom/Audit Safety Reserve, exhaustive Runtime Closure Gate, request-quiescent Commit Barrier, four-way Fence bounds, exhaustion, exact-absent closure, pre-barrier cancellation and post-barrier exact/nonmatching Control Mutation Cancel Gate flood/disconnect, degraded continuation, two-stage success with post-C27 dispatch deferral, installation-lock clean/failure/replacement restart, typed recovery, pending-before-tail, Storage Failure, empty restart | Range absence versus indeterminate no-ID latch, headroom/safety conservation and signal coalescing, every closure-liability constructor/edge, all request-liability components zero and stable before Store/Audit pause, owner/Result closure, same-Effect resume, authenticated ordered cancel responses with no Core/Audit/headroom and capacity release, unchanged lock plus fresh post-lock baseline for every successor, no Store replay/range envelope/recovered Fence/publication gap, assigned-lost closure/readiness | <= 400 |
| Q12 | `feat(benchmark): expose same-process failure qualification` | Architecture-compatible protocol/owner/watchdog/fail-stop lane | No Backend process or private IPC | <= 380 |
| Q13 | `feat(benchmark): expose certification qualification` | Production exact environment and Generation Semantics applicability/cache invalidation, quarantine, revalidation, recertification | Schema/algorithm identity drift through thin adapter over real probe, index, and Core | <= 400 |
| Q14 | `chore: aggregate qualification evidence` | Run-level report, checksums, lane closure, and artifact containment | Every required lane represented exactly once | <= 360 |
| Q15 | `fix: resolve one qualification finding` | Exactly one finding class; repeat as separate commits; any runtime-source-closure change invalidates the L01 build and all dependent Certification/lane evidence before rerunning L01 | Affected lane plus full regression; unchanged-closure proof or repeated L01 outer build and complete later qualification | <= 400 each |

After L01 finalizes the outer build, L02 freezes its one-way request Certification
inputs, and the compatible benchmark contract is fixed, complete qualification
means every required lane and matrix passes in one clean run against an applicable,
pre-frozen Certification Record for that exact outer build. Raw evidence stays
outside Git with hashes and manifests. Failed, stale, unavailable, or inapplicable
evidence remains explicit.

## Executable Verification

From B01 onward, every implementation commit runs the applicable subset of:

```sh
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo test --workspace --release
python3 -I -B -m unittest discover -s tests -v
```

The worktree policy command is the complete accepted-object auditor block in
Commit Protocol step 3; the candidate path is never executable authority.

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
set -eu
commit="$(env -i PATH="$PATH" LC_ALL=C GIT_NO_REPLACE_OBJECTS=1 \
  git -C . rev-parse --verify HEAD^{commit})"
parent1="$(env -i PATH="$PATH" LC_ALL=C GIT_NO_REPLACE_OBJECTS=1 \
  git -C . rev-parse --verify "${commit}^1")"
test -z "$(env -i PATH="$PATH" LC_ALL=C GIT_NO_REPLACE_OBJECTS=1 \
  git -C . rev-parse --verify "${commit}^3" 2>/dev/null || true)"
base="$parent1"; remote=
if parent2="$(env -i PATH="$PATH" LC_ALL=C GIT_NO_REPLACE_OBJECTS=1 \
  git -C . rev-parse --verify "${commit}^2" 2>/dev/null)"; then
  remote="$(env -i PATH="$PATH" LC_ALL=C GIT_NO_REPLACE_OBJECTS=1 \
    git -C . rev-parse --verify refs/remotes/origin/main^{commit})"
  env -i PATH="$PATH" LC_ALL=C GIT_NO_REPLACE_OBJECTS=1 \
    git -C . merge-base --is-ancestor "$parent2" "$remote"
  base="$remote"
fi
env -i PATH="$PATH" HOME="$HOME" LC_ALL=C GIT_NO_REPLACE_OBJECTS=1 \
  git -C . verify-commit --raw "$commit"
helper="$(mktemp "${TMPDIR:-/tmp}/turnvector-policy.XXXXXX")"
trap 'unlink "$helper"' EXIT
entry="$(env -i PATH="$PATH" LC_ALL=C GIT_NO_REPLACE_OBJECTS=1 \
  git -C . ls-tree "$base" -- scripts/check_commit_policy.py)"
test "${entry%% *}" = 100755
entry="${entry#* }"; test "${entry%% *}" = blob
env -i PATH="$PATH" LC_ALL=C GIT_NO_REPLACE_OBJECTS=1 git -C . \
  cat-file blob "${base}:scripts/check_commit_policy.py" >"$helper"
test -s "$helper"
env -i PATH="$PATH" LC_ALL=C BASE="$base" \
  COMMIT="$commit" HELPER="$helper" python3 -I -B - <<'PY'
import os, subprocess, tempfile
git_env = {"PATH": os.environ["PATH"], "LC_ALL": "C", "GIT_CONFIG_NOSYSTEM": "1",
           "GIT_CONFIG_GLOBAL": "/dev/null", "GIT_NO_REPLACE_OBJECTS": "1"}
root = os.fsdecode(subprocess.check_output(
    ("git", "-C", ".", "rev-parse", "--show-toplevel"), env=git_env
).removesuffix(b"\n"))
with tempfile.TemporaryDirectory(prefix="turnvector-policy-repo-") as sandbox:
    subprocess.run(("git", "init", "--quiet", "--bare", sandbox),
                   check=True, env=git_env)
    refs = ((os.environ["BASE"], "refs/turnvector/accepted-base"),
            (os.environ["COMMIT"], "refs/turnvector/candidate"))
    subprocess.run(("git", "-C", sandbox, "fetch", "--quiet", "--no-tags",
                    "--no-write-fetch-head", "--", root,
                    *(f"{oid}:{ref}" for oid, ref in refs)),
                   check=True, env=git_env)
    for expected, ref in refs:
        actual = subprocess.check_output(
            ("git", "-C", sandbox, "rev-parse", "--verify", f"{ref}^{{commit}}"),
            env=git_env, text=True).strip()
        if actual != expected:
            raise SystemExit(f"sandbox object mismatch for {ref}")
    subprocess.run(("git", "-C", sandbox, "update-ref", "--no-deref", "HEAD",
                    os.environ["BASE"]), check=True, env=git_env)
    actual = subprocess.check_output(("git", "-C", sandbox, "rev-parse", "HEAD"),
                                     env=git_env, text=True).strip()
    if actual != os.environ["BASE"]:
        raise SystemExit("accepted-base sandbox HEAD mismatch")
    command = ("python3", "-I", "-B", os.environ["HELPER"],
               "--base", os.environ["BASE"], "--head", os.environ["COMMIT"],
               "--branch", "feat/p0-runtime-implementation",
               "--title", "feat: implement P0 runtime")
    raise SystemExit(subprocess.run(command, cwd=sandbox,
                                    env={"PATH": os.environ["PATH"], "LC_ALL": "C"}).returncode)
PY
test -z "$remote" || test "$remote" = "$(env -i PATH="$PATH" LC_ALL=C \
  GIT_NO_REPLACE_OBJECTS=1 git -C . rev-parse --verify refs/remotes/origin/main^{commit})"
test "$commit" = "$(env -i PATH="$PATH" LC_ALL=C GIT_NO_REPLACE_OBJECTS=1 \
  git -C . rev-parse --verify HEAD^{commit})"
```

The helper is always extracted from the already accepted `base` object, never
from the commit under audit. It runs in an automatically removed bare repository
that fetches the frozen accepted-base and candidate object closures into private
refs, verifies both identities, and pins `HEAD` to the accepted base while the
immutable candidate SHA remains data. This also covers an accepted remote tip
that is not reachable from candidate `HEAD`. It makes the migration from the
installed helper's older checkout-`HEAD` self-authentication independently green
without granting the candidate authority. The successor helper treats `--base`
as its explicit policy source and authenticates its executing bytes against that
tree. T02 is therefore checked by its reviewed T01 predecessor; later commits,
including policy-only helper updates, are checked by the installed predecessor
authority without executing candidate policy code.

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
