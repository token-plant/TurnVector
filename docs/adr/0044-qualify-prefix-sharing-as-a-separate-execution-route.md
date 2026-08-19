# Qualify Prefix Sharing as a Separate Execution Route

TurnVector will keep initial P0 KV state private to one request and introduce Prefix Sharing only as a separately identified, optional Execution Route after a compatible Paged KV/cache ABI is qualified. The first native sharing route is in-memory and restart-empty, publishes only synchronized immutable prefix pages from a fixed route-local pool already covered by Model Residency, requires exact Model Revision, graph, KV, route, position/media, and token-prefix identity, and uses reference counting plus copy-on-write before divergent mutation.

The first route retains each request's complete conservative private-KV Resource Reservation for private fallback, tail growth, and copy-on-write even when physical pages are shared. A cache hit may reduce observed work and physical allocation, but it cannot authorize Admission, release reserved capacity early, transfer request-owned allocation into cache ownership, or make a Timing Commitment depend on mutable cache presence. Shared physical-memory accounting, durable Prefix Snapshots, or cross-restart restoration require later decisions and independent qualification.

## Consequences

- Prefix Reuse by copying or restoring private state may be qualified before live physical-page sharing.
- Prefix Sharing remains private to the C++/MLX Adapter; page IDs, refcounts, and cache entries do not cross the Backend Interface.
- A hit is adopted atomically during owner-thread Materialization or another explicitly designed synchronized operation; a miss follows an already authorized non-sharing route.
- Producer cancellation or cache eviction cannot invalidate pages retained by a consumer, and every divergent write first obtains private ownership.
- P0 readiness, restart semantics, and authoritative Control State remain unchanged.
