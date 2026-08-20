# Qualify Prefix Sharing as a Separate Execution Route

This decision depends on ADR 0042, Distinguish Paged KV Layout from Attention Execution. ADR 0042 must be accepted first because this route requires its independently identified Paged KV/cache ABI and Attention Path compatibility.

TurnVector will keep initial P0 KV state private to one request and introduce Prefix Sharing only as a separately identified, optional Execution Route whose Prefix Reuse plan kind is `NATIVE_PAGE_SHARING` after a compatible Paged KV/cache ABI is qualified. The first native sharing route is in-memory and restart-empty, publishes only synchronized immutable prefix pages from a fixed route-local pool already covered by Model Residency, requires exact Model Revision, route, position/media, and token-prefix identity, and uses reference counting plus copy-on-write before divergent mutation.

The first route retains each request's complete conservative private-KV Resource Reservation for private fallback, tail growth, and copy-on-write even when physical pages are shared. A cache hit may reduce observed work and physical allocation, but it cannot authorize Admission, release reserved capacity early, transfer request-owned allocation into cache ownership, or make a Timing Commitment depend on mutable cache presence. Shared physical-memory accounting, durable Prefix Snapshots, or cross-restart restoration require later decisions and independent qualification.

## Consequences

- Prefix Reuse with plan kind `PRIVATE_REUSE` may be qualified before live physical-page sharing.
- Prefix Sharing remains private to the C++/MLX Adapter; page IDs, refcounts, and cache entries do not cross the Backend Interface.
- The first route adopts a hit atomically only during owner-thread Materialization. A miss follows that exact sharing route's qualified private-state behavior rather than authorizing another route implicitly; any later adoption operation requires a separate decision and Backend Interface contract.
- Producer cancellation or cache eviction cannot invalidate pages retained by a consumer, and every divergent write first obtains private ownership.
- P0 readiness, restart semantics, and authoritative Control State remain unchanged.
