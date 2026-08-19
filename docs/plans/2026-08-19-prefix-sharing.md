# Prefix Sharing Design Plan

Status: design only; post-P0 optional route; no Certification, capacity, or performance claim

Governing decision: `docs/adr/0044-qualify-prefix-sharing-as-a-separate-execution-route.md`

Required predecessor: ADR 0042, Distinguish Paged KV Layout from Attention
Execution. Its independent Paged KV/cache ABI and Attention Path identities must
be accepted before this plan can be accepted or implemented.

## Objective

Allow compatible requests for one exact Model Revision to reuse completed prefix
computation and, in a later route over a qualified Paged KV/cache ABI, retain the
same immutable physical KV pages until divergent writes require copy-on-write.
The design must preserve the
existing Backend Interface, conservative Admission, request isolation,
owner-thread MLX rules, restart-empty P0 state, and exact Certification.

This plan distinguishes three capabilities:

| Capability | Consumer state | Shared live pages |
|---|---|---:|
| Prefix Reuse by recomputation avoidance | Copy, deserialize, or repage a compatible prefix into private request state. | No |
| Native Prefix Sharing | Atomically retain compatible immutable in-memory pages and copy on divergent append. | Yes |
| Shared physical-memory accounting | Admit more work by charging shared pages separately from request-private growth. | Yes, with a new Core accounting decision |

The first two can be delivered without the third. None is required for initial
P0 Service Readiness.

## Non-Goals

- Change P0's request-private KV or restart-empty inference-state contract.
- Let mutable cache presence authorize Admission or weaken worst-case bounds.
- Expose prefix lookup, page IDs, refcounts, tensors, or KV layout to Rust.
- Share across Model Revisions, graph/KV ABIs, incompatible positions, media,
  adapters, or token sequences.
- Persist native MLX handles or treat a cache artifact as Control State.
- Add cross-request Sampling State, output, cancellation, or request ownership.
- Claim TTFT, throughput, or capacity improvement before exact qualification.

## Terminology

**Prefix Reuse** avoids recomputing an exact compatible token prefix. Its exact
Execution Route member is a Prefix Reuse plan with stable kind `NONE`,
`PRIVATE_REUSE`, or `NATIVE_PAGE_SHARING`; the consumer may still own a private
physical copy under `PRIVATE_REUSE`.

**Native Prefix Sharing** means two live request states retain references to the
same immutable physical KV pages. It is stronger than Prefix Reuse and requires
reference-count, copy-on-write, eviction, and failure-isolation laws.

**Prefix Publication** is the owner-thread atomic transition that makes a
synchronized immutable prefix state eligible for a later lookup. It grants no
Admission or scheduling authority.

These terms describe private Adapter implementation. The Runtime Core continues
to reason in request ownership, exact Execution Routes, conservative Resource
Reservations, Candidates, Plans, and Receipts.

## Module And Seam

```text
Rust Runtime Core
  Admission assumes complete private worst case
  Candidate/Plan carries one exact route
                    |
                    v
C++/MLX Adapter / one owner thread
  ModelRuntime
    |- Request KV View -------- private tail / COW writes
    |- Prefix Index ----------- exact identity -> immutable entry
    |- Shared Page Pool ------- page generation + refcount
    `- Publication/Reclaim ---- retain, release, evict, demote
                    |
                    v
              pinned MLX -> Metal
```

The Prefix Index, Shared Page Pool, and request KV view are private Modules. The
Backend Interface remains deep: request Materialization, Candidate Formation,
Turn execution, Receipt observation, and exactly-once request release carry
opaque ownership and typed bounded results without exposing cache internals.

All lookup, retain, publication, copy-on-write, eviction, and native destruction
occur on the Device Executor owner thread. A synchronized internal lock may
protect non-MLX metadata only when its acquisition and hold time are included in
the owner-thread support bound.

## Compatibility Identity

A native entry and consumer match only when their complete route and additional
prefix identities are equal.

The route and Capability witnesses are:

- exact Model Revision, registered artifact/weight digest set, and Model
  Descriptor;
- one exact Execution Route Identity carrying a `NATIVE_PAGE_SHARING` Prefix
  Reuse plan; and
- that route's graph ABI and artifact, KV/cache layout ABI, page geometry, dtype,
  quantization, layer topology, and Attention Path identities.

Execution Route Identity equality already subsumes the decomposed graph,
KV/cache, Attention Path, and Prefix Reuse plan fields. They are revalidated as
integrity and mismatch-diagnostic witnesses; they are not additional authority,
and cannot make unequal route identities compatible. The Materialization Result
binds the adopted route identity; Core accepts it only when at least one exact
Capability Key in the request's frozen Authorized Capability Set carries that
identity. The Adapter never receives or chooses the Set, and no witness can
authorize a missing Key.

The additional prefix-entry compatibility fields not subsumed by Execution
Route Identity are:

- adapter/overlay identity, including any model-affecting fine-tune or sidecar;
- Generation Semantics identity and every prompt-state parameter that affects
  KV construction;
- exact request position, mask, rotary, and cache-offset values;
- media or other non-token input identity when the route supports it;
- exact prefix token count, canonical token-byte encoding, content hash, and
  byte-for-byte token equality after hash match; and
- producer publication generation and entry-format identity.

TurnVector's native Data Plane is token-native. Exact token IDs are therefore
the execution compatibility authority; a tokenizer or chat-template name alone
cannot establish a match. A Compatibility Gateway may record its own tokenizer
and template identity before producing token IDs, but that edge metadata cannot
substitute for native token equality.

A missing, unknown, unsupported, or mismatched field is a miss. Hash equality
without canonical byte equality is never a hit.

## Publication And Adoption

### Publication

An entry may be published only when:

1. a started Prefill Turn has synchronized all persistent KV state for the
   published token range;
2. the exact producer route and prefix identity are complete;
3. every page in the shared range is immutable and block-aligned;
4. the complete cache-entry reference can be retained atomically; and
5. failure before the publication point leaves no visible entry or leaked ref.

The private partial tail remains request-owned and unpublished. Publication is
idempotent by exact entry identity. A duplicate may retain or replace only under
one deterministic policy and cannot create two mutable authorities for a page.

### Adoption

PS04 commits the first route to owner-thread Request Materialization as its only
adoption operation:

1. Admission has already reserved the complete no-hit private-KV worst case and
   authorized every possible route in the request's finite capability closure.
2. Materialization performs an exact longest-prefix lookup under a bounded
   request and entry count.
3. A hit transactionally retains every referenced page and constructs one
   opaque request KV view. Partial retain failure rolls back every increment.
4. The Materialization Result records the exact adopted-prefix and Execution
   Route identities plus Backend Generation; Core validates the route against
   the request's frozen Authorized Capability Set before accepting the Result.
5. Later Candidate Formation emits only a Capability Key compatible with that
   materialized state.
6. A miss creates the ordinary private baseline state; it is not a failure.

This plan does not permit adoption in another operation. A future design may add
a separate synchronized adoption operation only through its own architecture
decision, bounded Backend Interface member, Support Operation Obligation,
watchdog, lifecycle legality, and exact result contract; it cannot be hidden
inside Candidate Formation or added under PS04.

Mutable cache presence is not frozen before Admission. A hit may reduce actual
Prefill work, but Timing Commitments and Resource Reservations remain valid for
the no-hit case. If an already retained entry becomes invalid before execution,
the request state must be released or explicitly rematerialized under a fresh
authorized transition; Candidate Formation cannot silently substitute state.

## Ownership And Copy-On-Write Laws

1. The cache entry owns one reference to each published page.
2. Every adopting request atomically owns one additional reference before the
   entry or producer can release its reference.
3. Cache eviction releases only the cache entry's references. Live request views
   remain valid.
4. Producer completion, cancellation, failure, or release cannot invalidate a
   page retained by another request.
5. A request may read a shared page but cannot mutate it.
6. Before the first divergent append or in-place update, the request allocates
   and fully initializes a private page, then atomically swaps its own view.
7. Failed allocation or copy leaves the old shared view unchanged and returns a
   typed request-local failure.
8. Trimming a request releases only references beyond its new logical prefix.
9. Exactly-once Request State Release drops every retained and private reference;
   duplicate release cannot decrement twice.
10. Physical page identifiers carry a pool generation or equivalent ABA-safe
    identity and are never accepted after reuse under a different generation.

The pool rejects underflow, overflow, unknown page, cross-pool reference,
mutable alias, and geometry mismatch. An untrustworthy ownership result invokes
the existing fail-stop contract rather than guessing which request owns memory.

## Resource Accounting

### First Sharing Route

Before a sharing-enabled Model becomes resident, its exact route reserves a
fixed maximum shared page pool through Model Residency. Every page eligible for
publication is allocated from that pool from the beginning; publication changes
references and immutability state but never transfers request-owned allocation
into cache ownership. Unpublished pages return to the same pool.

Each request also retains its complete conservative private-KV Resource
Reservation, including private fallback, partial-tail growth, maximum divergent
copy-on-write growth, and maximum context growth. This intentionally reserves
more logical headroom than a later shared-accounting design might need while the
sharing mechanism is first qualified. Pool capacity and request-private capacity
are separate reservation components and never claim ownership of the same
physical page.

The Adapter may report actual allocation and sharing telemetry, but:

- a hit cannot release request-reserved bytes early;
- cache eviction cannot create Core capacity;
- observed low physical use cannot authorize another request;
- COW growth is covered by the consumer's private budget, never by transferring
  charge from the shared pool; and
- actual release still enters Pending Reclaim until evidence converges.

The Resource Governor separately observes the bounded resident pool allowance as
part of the exact route and Model Residency profile. Pool exhaustion degrades to
a miss before request ownership is retained, or produces a typed request-local
failure after ownership exists; it never grows unbounded or borrows from a
request without allocating a request-private page under that request's budget.

### Deferred Shared Accounting

Charging one physical page once while dividing logical ownership across requests
would require a new Core-owned shared-pool ledger, reservation transfer laws,
admission race handling, eviction priority, producer/consumer settlement, and
Pending Reclaim semantics. That is a separate architecture decision. It cannot
be inferred from successful native refcount tests or physical-memory telemetry.

## Eviction And Pressure

The Prefix Index uses a finite deterministic policy with hard entry, token, and
byte maxima. Eviction candidates exclude pages required by live request views;
eviction drops only the cache-owned reference and index metadata.

Under pressure:

1. stop new publication and adoption before violating the hard pool bound;
2. evict unpinned cache entries in deterministic order;
3. request route-local cache reclaim through the existing Residency Transition;
4. retain released bytes as Pending Reclaim until allocator and process evidence
   converge; and
5. follow normal Resource Mode escalation if convergence fails.

The Scheduler, Turn Arbiter, and Resource Governor never directly mutate the
pool. They consume typed route/resource facts and direct bounded operations.

## Restart And Durability

The first route is memory-only. Restart begins with an empty Prefix Index and no
native page ownership. Requests, cache entries, page tables, and refcounts are
not restored, and P0 readiness does not wait for cache recovery.

A durable Prefix Snapshot route requires a separate plan covering immutable
payload pages, exact content/producer identity, checksums, file and directory
synchronization, metadata-last publication, corruption quarantine, pre-restore
reservation, schema readers, and cross-revision rejection. Native MLX handles
and raw page identifiers are never persisted.

## Certification And Probes

### Correctness And Isolation

- exact hit and longest compatible block-aligned prefix;
- content-hash collision with unequal token bytes;
- every identity-field mismatch and unsupported layout;
- zero, partial-tail, page-boundary, and maximum-context prefixes;
- producer release/cancellation before and after consumer retain;
- two consumers diverging at the same and different token positions;
- COW allocation failure, pool exhaustion, trim, eviction, and exactly-once
  release;
- stale generation, ABA page reuse, duplicate publication, and corruption;
- logits, updated KV, greedy tokens, Sampling State, and output-order parity
  against no-hit private execution;
- Dense, MoE, grouped-query, mask/position, media, dtype, and quantization cases
  only where the exact route declares support;
- fail-stop injection for untrustworthy ownership and cleanup results.

### Performance And Resource Evidence

- lookup, retain, publication, COW, eviction, and release latency distributions;
- hit/miss/adopted-token distributions with fixed workload provenance;
- TTFT, TPOT, completion latency, and aggregate throughput;
- logical referenced bytes, unique physical bytes, shared-page refcounts,
  fragmentation, metadata, scratch, and process physical footprint;
- MLX active/cache memory, compressor/swap, pressure, and Pending Reclaim;
- owner-thread support time, allocations, bytes copied, and command-buffer shape;
- failures, fallbacks, pool exhaustion, identity misses, and cleanup outcomes.

Thresholds and workload matrices are fixed before execution. A memory reduction
observed in one repeated-prefix workload is not a capacity claim, and a cache
microbenchmark cannot promote the serving route.

## Delivery Slices

| ID | Deliverable | Required verification |
|---|---|---|
| PS01 | Add the canonical Prefix Reuse plan member, three stable kinds, identity schema, and finite capability closure. | Double generation, kind/field drift, unknown/missing rejection. |
| PS02 | Implement bounded private Prefix Reuse by copy/repage. | No-hit/private parity, exact identity, resource bounds. |
| PS03 | Add an in-memory immutable page pool and atomic publication behind a separate route. | Refcount, publication rollback, page/generation invariants. |
| PS04 | Add Materialization-time longest-prefix adoption with full per-request reservation retained. | Hit/miss, rollback, Backend Generation, candidate compatibility. |
| PS05 | Add copy-on-write, trim, eviction, pressure reclaim, and exactly-once release. | Generated ownership sequences, failure injection, Pending Reclaim. |
| PS06 | Qualify exact model/layout/route matrices and mixed repeated-prefix serving. | Numerical, latency, throughput, memory, pressure, and soak artifacts. |
| PS07 | Consider exact-profile promotion for the first sharing route. | Decision record; unchanged P0 and no shared-capacity claim. |
| PS08 | Propose shared physical-memory accounting only if PS06 proves material value. | Separate ADR and Core ledger design; not part of this plan. |

PS02 can ship without live sharing. PS03-PS07 do not authorize PS08. Durable
Prefix Snapshots remain a separate future route.

## Completion Criteria

The first Prefix Sharing route is complete only when:

- exact compatibility and token equality fail closed;
- publication, adoption, COW, trim, eviction, cancellation, and release preserve
  ownership under generated and fault-injected sequences;
- consumer correctness is identical to the exact private baseline;
- all native state stays behind the Backend Interface on the owner thread;
- each request remains conservatively reserved for its complete private worst
  case and no cache observation creates Core capacity;
- restart remains empty and P0 readiness remains independent of the cache; and
- route-specific performance and memory claims do not exceed completed evidence.
