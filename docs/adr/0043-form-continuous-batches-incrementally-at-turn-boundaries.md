# Form Continuous Batches Incrementally at Turn Boundaries

TurnVector defines Continuous Batching as deterministic same-Model batch membership that may change only between synchronized Turns. Every accepted Receipt still returns control to a fresh Scheduling Snapshot and Turn Plan; incremental indexes, dirty-Model-only Candidate Formation, and reuse of still-current immutable candidates reduce planning work without authorizing automatic continuation, mid-Turn insertion, arbitrary subset enumeration, or cross-Model batching.

The Runtime Core owns eligibility indexes, dependency generations, dirty-Model state, and Candidate Associations, while the private Model Planner owns same-Model structural compatibility and canonical batch construction. The Model Planner receives no Certification Applicability or Authorized Capability Set and may only propose structurally supported exact route, batch, Shape, and compatibility facts; Core accepts the resulting Candidate's exact Capability Key only after every member's Authorized Capability Set and all current Core evidence admit that Key. The Turn Arbiter continues to own global urgency, fairness, and optimization. Formation Result reuse is valid only when its Model-local dependency vector is unchanged, Candidate Association reuse separately requires every Core dependency to remain current, and neither reuse means a reused Plan.

Dirtying or invalidating a Formation Result never authorizes a Backend call. Every actual recomputation remains caused and funded by exactly one existing initial-progress, Receipt-continuation, rejection, local-stale, or terminal-membership-change Support Operation Obligation. Generic route-catalog, Backend Generation, authorization, policy, or resource drift creates no sixth formation cause; if it invalidates structural planner output, affected Candidates remain unavailable until an existing funded cause permits recomputation, unless a later architecture decision adds and qualifies a new obligation kind.

## Consequences

- P0 retains one synchronized Turn at a time and fresh global arbitration after every Receipt.
- Candidate Formation work scales with changed Models and bounded configured batch buckets rather than all live requests on every Turn.
- Batch membership remains frozen by the Turn Plan, and every absent eligible request still receives a typed Candidate Exclusion.
- Native multi-member execution and each batch assembly/padding strategy require exact route-specific qualification. The route's graph ABI carries a stable Batch Execution Kind, so a genuine tensor batch and a sequential per-member loop at the same batch and Shape bucket have different Execution Route Identities and Capability Keys; a loop cannot be reported or certified as a tensor batch.
