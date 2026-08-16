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

## Transition Work

- [ ] Define the private native model Module Interface for model construction,
  Prefill, Decode, KV operations, and teardown.
- [ ] Define repository-owned configuration, weight-layout, graph, operator,
  and KV ABI identities and bind them into the Model Manifest.
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
- [ ] Make the TurnVector-owned path the production default only after its
  complete correctness, resource, performance, and failure envelopes pass.
- [ ] Decide through a later reviewed change whether the exported-graph path
  remains a qualification fallback or is removed after replacement evidence is
  durable.

## Promotion Gates

The TurnVector-owned path must not replace the interim baseline until all of the
following are true:

1. Exact supported-model revisions, weights, graph/operator identities, MLX
   build, hardware/software Envelope, and shape coverage are recorded.
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
- Claiming that native ownership is faster before complete comparative evidence
  passes its declared gates.
- Supporting arbitrary model families without explicit implementation,
  manifests, fixtures, and Certification Records.
