# Model Runtime Execution Transaction Design

Status: accepted design; implementation and native qualification pending

Governing decision:
`docs/adr/0047-commit-model-runtime-turns-through-an-execution-transaction.md`

Design lineage: `TV-MR-EXECUTION-TRANSACTION-20260820`

Frozen proposal SHA-256:
`7fbca7128f26864dcbf77ef4e5c30447da56dd799a8b9372d6bbbecd4e4d0524`

Accepted review round: `TV-MR-ET-R5-20260820T091941Z-1B86D6FA`

## Objective

Give Model Runtime one bounded, fail-closed mechanism for executing an exact
Turn Plan without exposing a mixture of old and new native request state. The
mechanism must preserve the existing coarse Backend Interface, owner-thread
topology, synchronized Turn Receipt, Runtime Core commit, and Output Publication
contracts.

The design must answer four questions before implementation:

1. What identity, membership, route, and capacity become immutable before native
   work begins?
2. What state can change after a route operation starts, and when does that state
   become logically visible?
3. What result owns enough storage to survive transaction, Model Runtime, and
   loaded-model destruction?
4. Which failures return a truthful synchronized Receipt, and which require
   process-wide fail-stop?

## Scope And Non-Goals

Execution Transaction covers exactly one accepted Turn Plan. One request may
participate in many transactions. One exact tensor batch is one transaction
whose members and row order are fixed before execution.

It owns the internal orchestration of:

- exact-plan and exact-route validation;
- transaction resource reservation and route-arena leasing;
- route start, native execution, and synchronization;
- typed member and first-use-artifact commit preparation;
- one logical native commit;
- failure and isolation classification; and
- transfer of one owned synchronized result.

It does not own:

- Candidate Formation, global scheduling, or Turn Plan creation;
- Certification Applicability, Admission, Resource Mode, or cross-Model policy;
- Runtime Core receipt acceptance or Request State commit;
- Output Publication, socket lifetime, or external HTTP response behavior;
- restart recovery or persistent inference-state logging;
- automatic route fallback, retry, speculative branches, or graph capture; or
- public Backend Interface methods beyond the existing `execute_turn` call.

Source file names, private helper signatures, enum layout, arena implementation,
numeric bucket set, and physical KV representation are intentionally not frozen.

## Ownership And Lifetime

The Device Executor owner thread constructs, calls, synchronizes, and destroys
each Model Runtime and its active transaction. A Model Runtime owns one private
transaction slot. Its active count is always zero or one. The current single
Device Executor also prevents active transactions in different Model Runtimes
from overlapping; any later concurrent executor or stream design invalidates
the arena and visibility proof and requires a new architecture decision.

The ownership split is:

| Object | Owner during execution | Lifetime rule |
| --- | --- | --- |
| Turn Plan and exact-route assets | Existing caller/Model Runtime | Borrowed only for the synchronous call. |
| Model graph and committed request-native state | Model Runtime | Never owned by the transaction; changed only by typed commit. |
| Frame, workspace, journal, pending-page metadata | Route arena | Exclusively leased by the transaction and reusable only after release. |
| Pending KV/COW payload and native temporaries | Existing bounded native pools | Separately reserved and charged; not hidden inside arena accounting. |
| Result and receipt storage | Result/output pool | Owned lease acquired before start; transferred or released after return. |
| Published output | Reserved outbound owner | Created only after Runtime Core accepts the receipt. |

The returned result contains owned immutable values or ownership-transferred
buffers. Its borrow set is empty with respect to Model Runtime, model graph, KV,
route arena, transaction, Device Executor stack, and thread-local storage:

```text
borrow_set(returned_result,
           {model_runtime, model_graph, kv_cache, route_arena,
            execution_transaction, device_executor_thread}) = empty
```

Destroying a Model Runtime requires no active transaction, but an already
returned result does not keep that Runtime or model resident. Its bytes remain
charged to the result/output pool until Core consumes them or the outbound owner
releases them. The synchronous Event Loop permits at most one returned but not
yet Core-accepted Backend result.

This ownership decouples returned-result storage from model lifetime. It does
not decouple the whole request or HTTP response from every request resource. A
slow consumer may retain bounded output capacity and request ownership under the
request lifecycle contract; it cannot retain Model Runtime through a result
borrow.

## Transaction Identity And Membership

Every transaction identity binds:

- the exact Turn Plan identity or digest and Engine Service identity;
- Model Runtime generation;
- exact Execution Route and route bucket;
- Batch Execution Kind; and
- ordered member handles.

Each member uses a stable `{slot, generation}` handle. Preflight builds a compact
`row -> slot` table in Turn Plan order. Membership and row order freeze at
`Armed`; padding is route-local storage and never becomes a member, outcome, or
receipt row. A stale handle, duplicate member, wrong route, or inconsistent plan
rejects before start. Discovering one after start is an invariant violation and
fails closed.

B=1 and B>1 use this same protocol. They may bind different exact compiled
artifacts. A sequential per-member loop remains a distinct Execution Route and
cannot report the Batch Execution Kind of a genuine tensor batch.

## State Machine And Start Barrier

The only non-fatal phase sequence is:

```text
Idle -> Preflight -> Armed -> Started -> Synchronized
     -> PreparedCommit -> Committed -> Returned -> Idle
```

`FailStop` is a terminal state after start. The phases mean:

| Phase | Required condition |
| --- | --- |
| `Idle` | No active identity, lease, or transaction charge. |
| `Preflight` | Validate plan, generations, route, membership, bounds, checked arithmetic, and headroom; derive the exact five-category demand, atomically reserve it, and acquire the bounded arena and result leases without changing persistent native state. Any failure releases the complete provisional transaction state. |
| `Armed` | Every reservation, transaction-control resource, and concrete lease is present. Transaction bookkeeping can no longer allocate. Only separately certified bounded native route allocation may occur later. |
| `Started` | Enter immediately before the first route operation, including first-use compilation. Plan Rejection is now forbidden. |
| `Synchronized` | Every issued native operation reached its certified synchronization boundary; no indeterminate work remains in flight. |
| `PreparedCommit` | Complete member commits, outcomes, artifact disposition, isolation evidence references, output extents, and receipt body occupy preallocated owned storage. |
| `Committed` | Publish all native member and artifact dispositions once in an allocation-free, non-throwing, non-blocking section with no Engine Service call. |
| `Returned` | Move the owned result to Runtime Core with no intervening Backend operation or unrelated Core mutation. |

The terminal law is:

```text
not Started  => an accepted pre-execution reason may return Plan Rejection
Started      => exactly one synchronized Turn Receipt XOR FailStop
```

Only the displayed forward transitions, a pre-start rejection back to `Idle`,
and a post-start transition to `FailStop` are legal. The transaction contains no
retry, member removal, fallback, route substitution, or rollback edge. Dropping
or unwinding before start discards only non-authoritative scratch. Doing so
after start without a committed transferred result is a fail-stop invariant
violation, not a destructor rollback opportunity.

First-use compilation is a route operation and crosses the start barrier. A
compile failure is therefore a typed started-Turn failure; compile-bound excess
is a Bound Violation. Neither can become Plan Rejection.

## Native State And Logical Commit

The complete logically visible native state of member `i` is:

```text
S_i = (validity, kv_committed_view, kv_logical_length, decode_cursor,
       request_phase, rng_state, stop_matcher_state,
       native_request_generation)
```

Output token IDs are staged result data, not persistent native state and not
published output.

Actual token progress `p_i` and native state change `N_i` are distinct types.
`p_i` is the checked difference in a present receipt `TurnProgress`, or zero
when progress is absent. `N_i` is a complete `NativeMemberCommit` that binds:

- expected old state fingerprint and request generation;
- receipt-member digest and actual progress;
- staged-output extent; and
- one disposition: `Advance(complete_successor)`,
  `RetainOld(no-mutation-proof)`, or `Quarantine(reason)`.

`Advance` carries every successor component of `S_i`, including the KV commit
descriptor and new logical length. It is never reconstructed from `p_i`.
`RetainOld` requires proof that persistent native state did not change.
`Quarantine` changes validity and makes the old backing unreadable by future
routes while bounded release remains pending.

The receipt/disposition relation is:

| Receipt member outcome | Allowed native disposition |
| --- | --- |
| `Completed` | `Advance`, or `RetainOld` only for proved zero progress and no mutation; member is terminal. |
| `Partial` | `Advance`, or `RetainOld` only at a certified zero-progress safe return; continuation and `still_runnable` must agree. |
| `Cancelled` | `Advance` for actual synchronized progress, or `RetainOld` with no-mutation proof; Core later resolves cancellation order and output discard. |
| `Failed(Some(isolation_id))` | Failed member is `Quarantine`; every other member uses its own legal disposition and the evidence binds disjointness and unaffected-result integrity. |
| Any `Failed(None)` in a batch | Every member is `Quarantine`, non-runnable, and failed without member-local continuation. |

While route work is active, it may prepare successors, but `S_i` remains
logically old. One owner-thread commit changes every member to exactly
`apply(S_i_old, N_i)`. No legal observer can see new cursor with old KV, new KV
length with old RNG, or another component mixture. This is single-owner logical
atomicity; it does not require cross-thread hardware atomics.

### First-Use Artifact Commit

The visible exact-route artifact state is:

```text
A_R = (route_key, artifact_status, artifact_digest, artifact_generation)
```

The transaction prepares either `ArtifactNoChange` or
`InstallVerifiedArtifact(complete_successor)`. A cold transaction may use its
private verified artifact because no Turn overlaps, but the route registry stays
old until the same logical commit that publishes member dispositions. Compile
failure installs nothing. An indeterminate compilation result can never become
a reusable artifact.

Opaque allocator caches may change as bounded non-authoritative consequences of
native work. Their bytes remain charged or are reclassified into an existing
reserved cache budget; they cannot determine progress, outcome, policy, or route
fallback. This design is not whole-process heap rollback.

### Two Commit Points

There are exactly two deliberately separate commits:

1. Model Runtime commits synchronized native request and route-artifact state
   before returning its owned result.
2. Runtime Core validates the Turn Receipt, atomically commits Core Request
   State, and only then publishes staged output through the existing Turn Output
   Reservation.

`observe_turn_receipt` and later callbacks are not native commit hooks. Deferring
native commit across the Interface would introduce hidden pending native state
and require another operation. Native commit itself never publishes client
output. If the result violates the receipt contract and Core cannot accept it,
the executor fails closed rather than continuing with divergent Core and native
state.

## Physical State Mechanisms And Isolation

The logical rule is fixed; storage mechanics are not. Contiguous KV may use
detached deltas or an uncommitted reserved tail. The visible view and logical
length remain old until commit, and a dirty tail is overwritten before reuse.
Paged KV may populate pending pages or table entries after capacity is proved;
live block-table references change only at commit. Shared pages are immutable,
and every write first obtains exclusive ownership through copy-on-write.

A member-isolation evidence ID is not a Boolean assertion. It binds at least:

- exact route, Plan, member `{slot, generation}`, and typed commit identities;
- canonical physical backing identities plus writable byte ranges or pages;
- disjointness or COW facts for every other member;
- synchronization status and exact failure classification; and
- each unaffected member's staged-result integrity plus expected old and
  complete successor fingerprints.

Canonical backing identity refers to the actual allocation across every MLX
view, wrapper, page-table alias, and storage-generation handle. Descriptor
inequality alone is insufficient. For mutable physical locations `Loc_i`:

```text
for all i != j: Loc_i intersect Loc_j = empty
write(page) => ExclusiveWrite(page), otherwise copy-on-write first
```

Independent recovery is legal only when both backing isolation and unaffected
result/successor integrity are trustworthy. Otherwise the complete batch is
quarantined.

## Progress And Output Bounds

For each member, all arithmetic is checked in preflight:

```text
0 <= L_i <= Cap_i <= U_w
1 <= Q_plan <= U_w
1 <= Q_i <= min(Q_plan, Cap_i - L_i)
0 <= p_i <= Q_i
0 <= g_i <= min(p_i, G_i)
0 <= G_i <= Q_i
0 <= r_i <= Smax_i
Vbound_i = Smax_i + G_i <= U_w
0 <= Vactual_i <= r_i + g_i <= Vbound_i
```

`L_i`, `Cap_i`, `Q_plan`, `Q_i`, and `p_i` are token counts. `g_i`, `G_i`,
`r_i`, `Smax_i`, `Vactual_i`, and `Vbound_i` count token IDs. `G_i` separates
generated output from generic phase progress, so pure Prefill has `G_i=0`.
`Vbound_i` includes both the maximum previously ambiguous stop suffix and the
maximum new output because one new token may release both. Result and Turn
Output Reservation sizing therefore uses `Vbound_i`, not `Q_i` alone.

If no positive `Q_i` fits, or any representation or bound check fails, the
route rejects before start. Finding that condition after start is a defect and
cannot be relabeled as Plan Rejection.

## Resource Bound And Accounting

For exact route `R` and one admissible Plan:

```text
J_demand(R,plan) = J0_R + sum_i(D_i)
P_demand(R,plan) = P0_R + sum_i(P_i)

J_demand(R,plan) <= Jcap_R
P_demand(R,plan) <= Pcap_R

A_demand(R,plan) = align_R(F_R) + align_R(W_R)
                   + align_R(J_demand(R,plan))
                   + align_R(P_demand(R,plan))
```

`F_R` and `W_R` are fixed frame and workspace bytes. `D_i` and `P_i` are
per-member journal and pending-metadata bytes. The route arena covers the finite
maximum `A_demand` over every certified exact route and admissible Plan, not an
assumed largest-bucket shortcut.

The independent owned-result requirement is:

```text
H_demand(R,plan) = H0_R + s_tok * sum_i(Vbound_i) + sum_i(E_i)
0 <= H_actual(x) <= H_demand(R,plan)

ResultLeaseBytes(plan,R) >= H_demand(R,plan) + X_R(plan)
LiveResultBytes(before x) + ResultLeaseBytes(plan,R) <= C_result
LiveResultBytes(t) <= C_result
UnacceptedBackendResultCount(t) <= 1
```

`X_R` covers simultaneous source and destination bytes when a bounded copy is
used; it may be zero for ownership transfer. No returned byte becomes reusable
arena capacity.

The complete transaction-attributable extra footprint is the five-category
vector:

```text
Reserve_x = (A_demand(R,plan),
             ResultLeaseBytes(plan,R),
             NativeTmp(R,plan,c),
             PendingBytes(R,plan),
             CompileBytes(R,c))

M_tx_extra(R,plan,c) = sum(Reserve_x)
Feasible_x(Reserve_x, LedgerState_before_x) = true
LedgerState_reserved_x = atomic_reserve(LedgerState_before_x, Reserve_x)

0 <=component Charge_x(t) <=component Reserve_x
M_tx_attributable_peak(x) <= M_tx_extra(R,plan,c)
```

`NativeTmp` contains MLX, Metal, and allocator temporaries outside the arena.
`PendingBytes` contains detached KV, uncommitted tails, pending pages/tables, and
COW payload while old state remains live. `CompileBytes` includes first-use
temporaries and a new artifact until it is released or reclassified into the
already reserved model/cache budget.

All five components and every shared physical-envelope constraint are acquired
atomically immediately before `Armed`. They are non-borrowable until release or
reclassification. A scalar free-memory sample is not sufficient evidence and
no allocation may consume the serving envelope outside the same coordinator.
A pending or compiled allocation transfers to an existing request allocation or
model/cache Residency Reservation at commit, or it is released.

A `BOUNDED_FIRST_USE` route always reserves its certified cold compile bound,
even when the cache is observed warm. Compile demand is zero only for a
separately certified warm route or `PRECOMPILED_REQUIRED` with exact artifact
identity and generation locked through preflight.

`M_tx_attributable_peak` is not a whole-process high-water or RSS bound. Daemon,
Core, gateway, output, allocator-cache, and unrelated growth require a separate
co-running envelope. Earlier returned result/output charges may remain live and
are bounded separately by `C_result`.

### Finite Control Work And Copy Bounds

For each route in the finite certified route set, later certification supplies
fixed nonnegative coefficients whose units are operations or bytes:

```text
Work_ctl(R,m,p,r) <= a_R + b_R*m + c_R*(B_R-m)
                     + d_R*sum_i(p_i) + e_R*sum_i(r_i)
Copy_ctl(R,m,p,r) <= f_R + g_R*m + h_R*(B_R-m)
                     + q_R*sum_i(p_i) + k_R*sum_i(r_i)

sum_i(p_i) <= m*Qmax_R
sum_i(r_i) <= m*Smax_R
```

`B_R` is the exact route bucket, `m` is active membership, and `B_R-m` is
padding. The coefficients never depend on live progress, retained suffix, or
historical KV length. Since both expressions are linear in `m`, the conservative
per-route maximum is the larger endpoint at `m=1` or `m=B_R`; the global bound
is the maximum over the finite certified route set. It is not assumed to belong
to the largest bucket.

This proves only a finite absolute bound for transaction bookkeeping and that
its structural work contains no historical-context term. It makes no
cross-route asymptotic, allocator-call, latency, or throughput claim. A new
unbounded route family requires a new uniform derivation.

### Snapshot Copy Break-Even

For exact native representation `rho_i`, let
`SnapBytes_i(L_i,rho_i)` include every byte needed to snapshot member `i`'s old
state: fixed metadata, alignment, page rounding, block tables, and payload. The
incremental copy required only for logical commit safety is bounded by:

```text
IncSafetyCopy_R(m,p,n)
    <= u_R + v_R*m + z_R*sum_i(p_i) + y_R*sum_i(n_i)

Copy_snapshot_safety = sum_i(SnapBytes_i(L_i,rho_i))
Copy_logical_safety  = IncSafetyCopy_R(m,p,n)
```

`n_i` counts new or privatized pages. The route checks its maximum before start.
Detached deltas, COW copies, pending tables, alignment, and fixed metadata must
appear in the route coefficients; direct writes to an exclusive uncommitted
tail may use a zero payload-copy coefficient only when certification proves it.
Common control and result copies are excluded from this comparison and remain
covered by the preceding bounds.

Logical commit has lower rollback-safety copy volume for a concrete route and
state only when:

```text
sum_i(SnapBytes_i(L_i,rho_i))
    > u_R + v_R*m + z_R*sum_i(p_i) + y_R*sum_i(n_i)
```

There is no universal numeric break-even until the exact functions and
coefficients are measured. Snapshot cost depends on historical state; logical
commit cost depends on new work and pages. Even where a small-context snapshot
copies fewer bytes, it is not the semantic default because it cannot make an
indeterminate native operation trustworthy. Qualification measures both
representations without claiming a universal copy win.

## Cancellation And Failure Laws

- Device Control Signal is only a timely-return hint. It cannot remove or
  reorder members, select a route, determine Core cancellation order, or
  publish/discard output.
- After start, signal observation may cause return only at a certified
  synchronized boundary with actual progress. It cannot invent progress.
- Before start, signal observation permits Plan Rejection only through an
  already accepted Backend rejection reason; it does not classify final request
  cancellation.
- A native operation that may remain in flight is indeterminate. The transaction
  obtains a trustworthy synchronized result or process-wide fail-stops; it never
  guesses completion or fabricates a Receipt.
- Output remains in the pre-reserved result lease. Runtime Core alone decides
  publication after receipt acceptance and ordered cancellation resolution.
- No complete KV snapshot, persistent WAL, hot-path I/O, force abort, silent
  unwind rollback, or hidden route retry is an accepted recovery path.

## Visibility And Exactly-Once Laws

For committed transaction `x`, let `t_c(x)` be its logical owner-thread commit.
The new member state remains visible until the first later commit affecting the
same `{slot, generation}`, or until release/retirement, slot-generation reuse,
or Model Runtime destruction. Before commit the old state remains visible:

```text
start(x) <= t < t_c(x)
    => Visible_i(t) = S_i^(x,0)

t_c(x) <= t < t_end_member(i,x)
    => Visible_i(t) = apply(S_i^(x,0), N_i^x)
```

Artifact visibility uses the same interval rule until a later commit for the
same exact route key, eviction, quarantine, removal, or registry/Runtime
destruction. The live-state function becomes undefined after those endpoints;
longer-lived audit facts reside in owned Core receipt/evidence records rather
than invented native tombstones.

Every execution prefix must satisfy:

```text
ActiveTransactionCount(model_runtime) in {0,1}
CommitCount_x in {0,1}
ReceiptCount_x in {0,1}

PreStartRejection_x => CommitCount_x=0 and ReceiptCount_x=0

Started_x => eventually exactly one of:
  Returned_x and CommitCount_x=1 and ReceiptCount_x=1
  XOR FailStop_x
```

No later transaction starts before `Returned_x`.

## Implementation Sequence

1. Build a fake private transaction state machine with phase assertions,
   state fingerprints, checked arithmetic, allocation counters, the complete
   outcome/disposition matrix, and fault injection at every transition.
2. Implement contiguous-KV B=1 with a bounded result envelope. Prove that a
   returned result remains valid after Model Runtime destruction and remains
   charged until release.
3. Add only separately accepted exact tensor-batch routes. Prove row/member
   parity, exact membership, failure classification, and isolation evidence for
   each supported bucket.
4. Reuse the protocol for chunked Prefill without moving planning or automatic
   continuation into Model Runtime.
5. Add paged KV/COW only after exact physical-page identity, sharing, and
   isolation evidence is available.

No implementation slice adds a public Backend Interface operation. Descriptor
content and resource profiles change only when an implementation supplies the
new bounded values and conformance evidence.

## Required Verification

The Fake and native implementations must eventually prove:

- every accepted injected pre-start rejection mutates no persistent native
  state and returns no Turn Receipt, while an unclassifiable invariant defect
  fails closed rather than fabricating a rejection;
- every started path returns exactly one matching synchronized Receipt or
  triggers the fail-stop harness;
- no injected failure exposes a mixed native-state tuple;
- receipt membership, order, progress, outcome, and typed commits exactly match
  the accepted Plan;
- cold verified artifacts install once by exact key/digest only at commit, while
  failed or indeterminate compilation installs none;
- dirty contiguous tails are not read and shared pages are not mutated;
- every isolated failure binds the exact failure class and proves unaffected
  staged-result and successor integrity;
- maximum visible output includes a released old ambiguous stop suffix and
  remains within the pre-start reservation;
- control signals do not alter membership or semantic event order;
- reservation contention acquires all five categories and common envelopes
  atomically, with no borrowing or partial acquisition;
- result buffers contain no Runtime/model/arena borrow, stay within `C_result`,
  and remain charged while retained after later Turns; and
- allocation, copy, memory, synchronization, TTFT, inter-token latency, and
  throughput observations are reported by exact route without converting a
  transaction-attributable peak into process RSS.

Boundary coverage includes B=1 and every supported maximum bucket; zero and
ceiling progress; exact KV capacity; checked-integer maxima; maximum stop suffix
release; one, several, and all failed members; cancellation before start, during
work, and after synchronization; cold and warm compilation; maximum native,
pending/COW, arena, and result demand together; stale generations; duplicate
commit; unwind in every phase; result access after Runtime destruction; and
member/artifact visibility endpoints.

TurnVectorBenchmark remains read-only during any cross-repository verification.

## Alternatives Rejected

- Full native-state snapshots add history-sized rollback copying and still
  cannot make an indeterminate native operation trustworthy.
- Direct mutation can expose mixed KV, cursor, RNG, phase, and stop state.
- Deferring native commit to a Core callback creates hidden pending state across
  the Interface and requires a new operation.
- Always failing the whole batch discards the accepted isolated-member recovery
  path; whole-batch quarantine remains mandatory when proof is absent.
- Persistent transaction logging conflicts with restart-empty inference state
  and the no-hot-path-I/O boundary.
- RAII rollback after start cannot reverse unknown native effects or establish a
  truthful Receipt.
- A public transaction class adds surface without caller authority or evidence
  that the existing execute/result boundary is insufficient.

## Evidence And Claim Boundary

The design review establishes internal coherence of the state machine,
ownership, symbolic bounds, and fail-closed behavior. It does not establish
current native behavior or performance. P-1B remains pending and P-1C remains
RED in the required historical evidence. Exact coefficients, capacities,
allocator behavior, aliasing, synchronization cost, and route measurements are
still implementation and qualification obligations.

The permitted design claim is:

> TurnVector should use one private, owner-thread-confined, bounded Execution
> Transaction for each accepted Turn Plan, with a pre-native start barrier,
> synchronized old-or-complete-new native-state commit, owned returned results,
> and fail-closed handling when a truthful Receipt cannot be formed.

This document does not permit a claim that current MLX execution already
satisfies the contract or that the design improves latency, throughput, memory,
or energy.
