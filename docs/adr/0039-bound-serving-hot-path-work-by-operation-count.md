# Bound Serving Hot-Path Work by Operation Count

TurnVector defines binary hard maxima for active Models, eligible requests per Model, Work Candidates, Batch members, Core Events examined, bytes copied, allocations, and invariant work in one serving decision or Core Transition. Admission and ingress reject before accepted state can exceed those maxima, and no Implementation may silently truncate work. Scheduling Snapshots use incremental indexes and immutable views, Candidate Formation recomputes only dirty Models, Receipt processing touches only its members and maintained aggregates, and serving invariants use bounded incremental witnesses. Full-state scans remain available for Bootstrap, replay, tests, and explicit diagnostics.

Verification asserts these operation counts as contract behavior in addition to wall-clock performance. This keeps the single-process architecture's overhead bounded without adding actors, threads, lock-free structures, or IPC to hide algorithmic cost.
