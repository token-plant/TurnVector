# TODO: Own Native Model Graphs and Operators

Status: future work; not an accepted performance claim or a P0 readiness gate

## Outcome

TurnVector should ultimately own the production model graphs, KV semantics, and
performance-critical operators executed by its C++/MLX Adapter. The Adapter
should call the pinned MLX C++ interface directly while preserving the existing
coarse Backend Interface presented to Rust.

An interim exported-graph path may use pinned external Python model definitions
to create reproducible artifacts and numerical reference outputs. That path is
a bootstrap and qualification aid, not the permanent source of production model
semantics. Python and external LLM packages must remain outside the serving
runtime.

This TODO does not replace
[ADR 0018](adr/0018-default-the-experimental-mlx-backend-to-an-in-process-cpp-interface.md).
It records the intended internal evolution of that Adapter after the initial
native baseline is working and qualified.

## Why This Work Exists

Linking the Adapter to `libmlx` supplies tensor, graph, stream, allocator, and
Metal execution primitives. It does not supply a complete model implementation:
layer topology, weight mapping, attention and rotary-position semantics, MoE
routing, quantized linear behavior, KV layout and updates, or the exact
Prefill/Decode contracts.

Using exported model graphs can establish a small reproducible baseline without
putting Python on the serving path. Permanent reliance on externally maintained
model definitions would, however, leave production graph behavior and important
optimization choices outside TurnVector's ownership. TurnVector needs that
ownership to evolve model support, KV behavior, batching, graph specialization,
and fused operators under its own compatibility and qualification rules.

## Exact Execution Profiles

Every native baseline or optimization is represented by one canonical bounded
Execution Route descriptor. Its Evidence Hash is the Execution Route Identity
carried by the exact Capability Key and the compiled Certified Execution Profile.
The descriptor fixes:

- graph ABI and graph artifact identities;
- weight-layout ABI;
- memory-plan and arena identity;
- kernel-bundle and fusion-plan identities;
- KV/cache layout ABI whose exact identity distinguishes non-paged and PagedKV
  layouts;
- independent Attention Path identity;
- Speculative Decode plan or explicit `NONE`;
- Prefix Reuse plan or explicit `NONE`; and
- command-submission or replay plan or explicit `NONE`.

The Attention Path is a composition identity, not a second owner of its
constituents. It owns the stable path kind, compilation timing and no-fallback
policies, and canonical references to the exact graph, kernel/fusion, KV/cache,
memory, and command members. Attention compute input/output and accumulation
dtype semantics plus mask/position ABIs remain owned by the graph member;
model-weight storage, quantization, packing, and dequantization by the
weight-layout ABI; dispatch, kernel artifacts, and compilation inputs by the
kernel/fusion member; KV storage dtype/quantization, access, page encoding, and
block-table reader by the KV/cache member; scratch by the memory-plan member;
and submission/replay by the command member. Execution Phase and runtime Shape
remain in the exact Capability Key and Case Bound Table rather than being
duplicated in the route descriptor.

The Profile is only a compact read-only projection of the existing Certification
Record, Environment Qualification, and Case Bound Table authority. It is not a
Serving Profile or a mutable Backend configuration. Installed unified-memory
size and the exact Mac/GPU/macOS build remain stable Environment facts; current
allocator state, process footprint, available memory, pressure, swap,
compressor, and Pending Reclaim remain dynamic Resource Evidence. An exact
Profile match can therefore be temporarily infeasible, while favorable runtime
memory observations can never authorize a missing Profile.

A baseline route uses exact identities for every required member, including its
non-paged KV/cache layout ABI, and represents every absent optional plan as
`NONE`. Changing any route member produces a new identity and requires exact
applicability and bound evidence; it never mutates the meaning of an existing
Profile. A fixed arena means a qualified preallocated lifetime and stable
offsets. It does not promise a permanent raw GPU virtual address unless the
pinned Metal/MLX implementation exposes and separately qualifies that property.

## Required Ownership

TurnVector-owned source and tests should eventually define:

- supported model-family configurations and validated weight-name mappings;
- quantization metadata interpretation and supported quantized linear paths;
- model graph topology for Prefill and Decode;
- attention masks, rotary-position handling, normalization, and projections;
- Dense and MoE feed-forward paths, including deterministic expert routing;
- KV cache layout, allocation, update, slicing, and release semantics;
- exact logits and updated-KV outputs at the native model seam;
- performance-critical fused or specialized operators when qualification
  justifies them;
- graph, operator, weight-layout, and KV ABI identities used by manifests and
  Certification Records.

The implementation may continue to build on `libmlx`; owning these Modules does
not mean forking or reimplementing MLX itself.

## Seam Constraints

The change must remain private to the C++/MLX Adapter implementation:

```text
Rust Runtime Core
      |
      | coarse Backend Interface operations
      v
C++/MLX Adapter
      |
      | TurnVector-owned model graphs, KV, and operators
      v
pinned MLX C++ interface -> Metal -> Apple GPU
```

- Do not expose tensors, graph nodes, KV layout, C++ objects, or per-operator MLX
  calls through the Rust-facing Backend Interface.
- Do not move cross-model scheduling, Admission, Resource Mode, or global
  residency policy into model code.
- Keep all MLX model, graph, stream, KV, and cache ownership on the Device
  Executor owner thread.
- Preserve one bounded Adapter operation and one typed result for each Backend
  Interface call.
- Treat native model implementations as internal Modules, not public plugins or
  independently authoritative schedulers.

## Optimization Order

Each step keeps the coarse Backend Interface unchanged and adds a new exact
Execution Route rather than widening the prior Profile:

1. Own exact Prefill/Decode graphs and a preallocated fixed-offset arena.
2. Design the Attention and KV Layout Module contracts together, including the
   exact route fields, block-table reader ABI, mask/position semantics, scratch
   bounds, and Turn Receipt observations shared by native page-reading routes.
3. Add a PagedKV layout with an exact page, dtype, quantization, allocation,
   update, and release ABI, retaining a gathered pinned-MLX-SDPA reference
   route.
4. Add separately identified phase-specific attention paths and operator-fusion
   plans. The PagedAttention delivery owns the first native block-table Decode
   slice; the later FlashAttention delivery owns tiled Prefill. Qualify them
   independently and never represent the family with a `flash_attention`
   boolean.
5. Add Prefix Reuse only on a qualified PagedKV/cache ABI and bind the complete
   model, token-prefix, graph, KV, producer, and publication identities.
6. Add Speculative Decode with exact draft/verifier models, acceptance semantics,
   synchronization points, and additional memory/time bounds.
7. Add Metal command replay or ICB only when the pinned implementation exposes
   a stable contract and correctness, cancellation, command-buffer, and
   performance qualification passes.

No later step is a prerequisite for shipping or certifying an earlier route.
The attention/KV ownership boundary, sequential delivery order, route matrix,
and promotion gates are defined in
[ADR 0045](adr/0045-qualify-flash-attention-as-exact-phase-specific-routes.md)
and its
[detailed design](plans/2026-08-19-flash-attention-route-design.md).

## Transition Work

- [ ] Define the private native model Module Interface for model construction,
  Prefill, Decode, KV operations, and teardown.
- [ ] Define the canonical Execution Route descriptor and repository-owned
  configuration, weight-layout, graph, operator, memory-plan, and KV ABI
  identities; bind them into the Model Manifest and exact Capability Key.
- [ ] Implement the first supported Dense model graph and its required
  operators using the pinned MLX C++ interface.
- [ ] Implement the first supported MoE model graph, including deterministic
  top-k routing and expert execution.
- [ ] Add deterministic weight-loading and quantization validation that rejects
  unsupported or ambiguous artifacts before constructing live MLX state.
- [ ] Build independent numerical fixtures for logits, KV updates, sampling
  inputs, and multi-token state transitions across every certified shape.
- [ ] Compare the TurnVector-owned path with the interim exported-graph path and
  an independent offline reference without making either external path a
  serving dependency.
- [ ] Measure Engine Service, TTFT, TPOT, throughput, command-buffer shape,
  allocator state, process footprint, and transient headroom for Dense and MoE
  Prefill/Decode matrices.
- [ ] Qualify cancellation, partial progress, cleanup, load/unload, memory
  pressure, and fail-stop behavior through the production Backend Interface.
- [ ] Compile each qualified route into finite exact-key Certified Execution
  Profile entries and prove that every one-field route or environment drift
  fails closed.
- [ ] Make the TurnVector-owned path the production default only after its
  complete correctness, resource, performance, and failure envelopes pass.
- [ ] Decide through a later reviewed change whether the exported-graph path
  remains a qualification fallback or is removed after replacement evidence is
  durable.

## Promotion Gates

The TurnVector-owned path must not replace the interim baseline until all of the
following are true:

1. Exact supported-model revisions, weights, Execution Route and graph/operator/
   memory/KV identities, MLX build, hardware/software Envelope, and shape
   coverage are recorded.
2. Logits, updated KV, token selection, stop handling, and seeded multi-token
   generation pass the required deterministic parity matrix.
3. Owner-thread, call-order, synchronization, cancellation, typed-result,
   release, and shutdown conformance passes through the same Backend Interface
   fixtures as the Fake Execution Backend and prior native baseline.
4. Memory reservation, actual allocation, transient headroom, pending reclaim,
   and process-footprint convergence remain within predeclared bounds.
5. Performance thresholds are declared before measurement and pass for the full
   supported Dense/MoE Prefill and Decode matrix; a microbenchmark alone cannot
   authorize promotion.
6. The serving runtime has no Python or external LLM package dependency, and
   every production model-semantic source is versioned by TurnVector.

## Non-Goals

- Reimplementing MLX tensor primitives, its allocator, or its Metal backend.
- Calling MLX operators individually across the Rust/C++ seam.
- Expanding the public Data or Control Plane protocol.
- Adding concurrent Metal Turns or changing the P0 owner-thread topology.
- Promising permanent raw GPU addresses from a fixed-offset arena without a
  separately exposed and qualified Metal/MLX contract.
- Claiming that native ownership is faster before complete comparative evidence
  passes its declared gates.
- Supporting arbitrary model families without explicit implementation,
  manifests, fixtures, and Certification Records.
