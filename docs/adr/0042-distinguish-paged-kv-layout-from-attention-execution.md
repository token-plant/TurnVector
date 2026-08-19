# Distinguish Paged KV Layout from Attention Execution

TurnVector will identify KV storage layout and Attention Path as separate members of every native Execution Route. Attention Path is a composition identity: it owns the stable path kind, compilation timing and no-fallback policies, and exact references to the graph, kernel/fusion, KV/cache, memory, and command members, while those members remain the sole owners of their ABI payloads and phase/Shape remains in the exact Capability Key and Case Bound Table. A paged KV layout may be consumed either by an explicitly identified gather-to-MLX-SDPA path or by an explicitly identified native block-table path; neither is implied by the word "paged," and changing between them creates a new Execution Route Identity and requires independent applicability, correctness, resource, and performance qualification.

The C++/MLX Adapter owns page tables, tensors, kernels, geometry checks, and route-local scratch memory behind the existing coarse Backend Interface. A native route may reject an inapplicable Turn before any route operation starts, but it cannot silently fall back to another Attention Path after execution or first-use compilation starts. The initial P0 route remains contiguous KV with `CONTIGUOUS_MLX_SDPA`, and Paged KV or native Paged Attention is not a P0 readiness requirement.

## Consequences

- Paged KV can ship and be qualified with gather-to-MLX-SDPA before a native block-table kernel exists.
- Native Paged Attention is a separate optional route, not a property automatically granted to every paged cache.
- Capability Requirements, Certification Records, Case Bound Tables, route telemetry, and quarantine must distinguish the exact Attention Path.
- Prefix Sharing and shared physical-page accounting remain separate decisions even when they later reuse the same paged layout.
