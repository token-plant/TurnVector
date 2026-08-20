# Commit Model Runtime Turns Through an Execution Transaction

Related decisions: ADR 0016 defines the pre-start rejection and started-Turn
terminal law; ADR 0043 fixes batch membership and requires fresh arbitration;
and ADR 0045 preserves that start barrier for first-use compilation. ADR 0042
governs the paged KV/COW specialization below; contiguous B=1 does not depend
on paged KV.

TurnVector will execute each accepted Turn Plan through one private,
owner-thread-confined Execution Transaction inside the selected Model Runtime.
The transaction is a deep implementation boundary behind the existing coarse
`execute_turn` Backend Interface operation. It is not a public begin/commit API,
a Scheduler authority, a database transaction, a whole request, or an external
response lifetime. One request may participate in many transactions, while one
tensor batch is one transaction with fixed ordered membership.

The Device Executor thread remains the sole mutable owner. At most one
transaction may be active per Model Runtime, and the current single Device
Executor still runs at most one Backend operation at a time. Each transaction
binds the exact Turn Plan, Model Runtime generation, Execution Route Identity,
Batch Execution Kind, route bucket, and ordered member `{slot, generation}` handles.
Membership, order, and route cannot change after preflight.

The non-fatal state machine is:

```text
Idle -> Preflight -> Armed -> Started -> Synchronized
     -> PreparedCommit -> Committed -> Returned -> Idle
```

`FailStop` is the terminal path after start when a trustworthy synchronized
result or complete no-fail commit cannot be established. Preflight validates
identity and checked bounds and atomically reserves the complete transaction
resource vector, including reusable arena, owned-result, native-temporary,
pending KV/COW, and compilation capacity. `Started` begins immediately before
the first route operation, including bounded first-use compilation. Before that
barrier, an accepted pre-execution failure may be a Plan Rejection. After it,
exactly one synchronized Turn Receipt must be committed and returned, or the
process fail-stops. There is no retry, fallback, member removal, route
substitution, or silent destructor rollback inside the transaction.

For each member, the transaction prepares one complete typed native commit that
binds the expected old native-state fingerprint and generation, receipt member,
actual progress, staged output extent, and exactly one disposition:
`Advance(complete_successor)`, `RetainOld(no-mutation-proof)`, or
`Quarantine(reason)`. The transaction keeps KV view and logical length, cursor,
phase, RNG, stop-matcher state, validity, and native generation logically old
until one allocation-free, non-throwing owner-thread commit publishes the
complete successor tuple for every member. Token progress is evidence inside
that record; it is not itself a native-state delta.

A failed member may be isolated only by durable evidence binding canonical
physical backing identities and byte ranges or pages, copy-on-write and
disjointness facts, synchronization, failure class, typed commit, and the
integrity of every unaffected member's result and successor. Without that
proof, the complete batch is quarantined. A bounded first-use artifact is a
separate typed model-level disposition and becomes registry-visible in the same
logical commit as member state. Failed or indeterminate compilation installs no
artifact.

The Model Runtime commit precedes return from `execute_turn`. Runtime Core then
performs the separate existing commit: it validates the Turn Receipt, atomically
updates Core Request State, and publishes staged output only through reserved
outbound capacity. No later callback is a native commit hook. An internally
inconsistent result fails closed rather than allowing Core and native state to
diverge.

Returned results contain owned immutable values or ownership-transferred
buffers and borrow no model graph, KV cache, MLX array, route arena,
transaction, Device Executor stack, or thread-local state. Their storage may
therefore outlive the transaction, Model Runtime, and loaded model while it
remains charged to a bounded result/output pool. This does not make the entire
request or external response independent of request-native resources: a slow
consumer may retain bounded output capacity and request ownership under those
separate contracts.

## Consequences

- Exact resource reservations and all checked capacity arithmetic complete
  before `Started`; transaction bookkeeping allocates nothing afterward.
- Logical old-or-complete-new commit is required, while contiguous tails,
  detached deltas, pending page tables, COW, and result copy versus transfer
  remain private representation choices.
- Full KV snapshots, persistent WAL, hot-path artifact I/O, hidden scheduling,
  and post-start route fallback are excluded.
- The transaction-attributable five-category memory bound is not a
  whole-process RSS bound. Result/output charges may remain live after a Turn
  and are accounted separately.
- Overlapping transactions, multiple Device Executors, speculation, and a new
  public Backend Interface operation require later decisions and qualification.
- The implementation order is fake state-machine fault injection, contiguous
  B=1, exact tensor batches, chunked Prefill, and only then qualified paged/COW
  routes.
- P-1B remains pending and P-1C remains RED. This decision establishes a design
  contract, not current MLX conformance or a latency, throughput, memory, or
  energy improvement claim.

The complete invariants, symbolic resource bounds, failure matrix, and rollout
gates are defined in
`docs/plans/2026-08-20-modelruntime-execution-transaction.md`.
