# Freeze P0 Runtime Architecture Contracts Before Behavior

Related decisions: ADR 0020 fixes the bounded in-process Backend Interface and
its two adapters; ADR 0032 fixes the three primary crates and keeps Runtime Core
independent of protocol and I/O; ADR 0033 keeps Core transitions atomic and
fail-closed. The accepted P0 plans retain authority over ledger order and the
rows that implement behavior.

TurnVector will establish one canonical P0 Architecture Contract Baseline
before delegating the remaining implementation rows. The baseline records the
final private Module paths, primary and contributing ledger ownership, schema
family ownership, Interface operation vocabulary, adapter composition,
visibility, compile role, and declared dependency graphs. It does not define
schema fields, Rust representations, algorithms, capacities, failure variants,
or evidence applicability.

Contract-only Rust paths contain exactly one Module-level documentation line
and no Rust item. Production and test-only paths may receive private module
declarations so the compiler checks the final topology; the L01-L02 release
identity path remains unlinked until its offline-tool target is implemented.
The Protocol crate is present in the workspace but item-free and has no runtime
dependency edge before P04. No trait, function, method, type, state, allocation,
Effect, callback, fallback, panic, or runtime dispatch is introduced by the
baseline.

The baseline is a one-time exception to the plans' prior prohibition on source
paths before their owning rows. It does not add or reorder a ledger row. Each
owning row replaces its exact contract-only path, changes its manifest status to
`implemented`, supplies the complete behavior and tests, and regenerates every
affected build identity. A material ownership, Interface, schema, dependency,
or topology change remains an architecture change rather than an implementation
detail.

## Consequences

- Delegated work starts from compiler-visible final paths and one mechanically
  checked ownership vocabulary rather than reconstructing topology per row.
- Contract-only status never means a capability is implemented or ready; some
  coordination behavior may still reside temporarily in an existing owner file.
- The structural validator fails closed on malformed or noncanonical records,
  ownership gaps, duplicate owners, dependency cycles, incomplete adapters,
  public leakage, or implementation-bearing contract shells.
- Runtime correctness, Backend conformance, protocol compatibility,
  persistence, performance, resource safety, and qualification remain unproved
  until their scheduled rows and evidence gates pass.
