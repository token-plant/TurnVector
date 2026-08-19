# Form Continuous Batches Incrementally at Turn Boundaries

TurnVector defines Continuous Batching as deterministic same-Model batch membership that may change only between synchronized Turns. Every accepted Receipt still returns control to a fresh Scheduling Snapshot and Turn Plan; incremental indexes, dirty-Model-only Candidate Formation, and reuse of still-current immutable candidates reduce planning work without authorizing automatic continuation, mid-Turn insertion, arbitrary subset enumeration, or cross-Model batching.

The Runtime Core owns eligibility indexes, dependency generations, dirty-Model state, and cached candidate association, while the private Model Planner owns same-Model compatibility and canonical batch construction. The Turn Arbiter continues to own global urgency, fairness, and optimization. Candidate reuse is valid only when its complete dependency vector is unchanged, and a reused candidate never means a reused Plan.

## Consequences

- P0 retains one synchronized Turn at a time and fresh global arbitration after every Receipt.
- Candidate Formation work scales with changed Models and bounded configured batch buckets rather than all live requests on every Turn.
- Batch membership remains frozen by the Turn Plan, and every absent eligible request still receives a typed Candidate Exclusion.
- Native multi-member execution and each batch assembly/padding strategy require exact route-specific qualification; a per-request loop cannot be reported as a tensor batch.
