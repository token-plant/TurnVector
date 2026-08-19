# Paged Attention Execution Route Plan

Status: design only; post-P0 optimization; no Certification or performance claim

Governing decision: `docs/adr/0042-distinguish-paged-kv-layout-from-attention-execution.md`

## Objective

Add a native Attention path that can read a qualified paged KV layout directly
through a block table, without widening the Rust-facing Backend Interface or
making Paged KV, Prefix Sharing, or native Paged Attention a P0 prerequisite.

The design deliberately separates two questions:

1. How persistent K/V state is allocated, addressed, updated, and released.
2. How one exact Attention implementation consumes that state for one Turn.

The first question is the KV layout. The second is the Attention path. They may
evolve independently and must never share Certification by implication.

## Non-Goals

- Port a CUDA serving runtime or expose a public block-table interface.
- Expose tensors, page IDs, graph nodes, kernel geometry, or MLX objects to Rust.
- Add cross-request Prefix Sharing or shared physical-memory accounting.
- Enable concurrent Metal Turns, command-buffer preemption, or asynchronous
  execution inside one Turn.
- Make Paged KV or native Paged Attention part of initial P0 Service Readiness.
- Claim lower latency, higher throughput, or lower memory before a qualified run.

## Module And Seam

The existing Backend Interface remains the only Rust/native Seam:

```text
Rust Runtime Core
  Work Candidate -> Turn Plan -> execute_turn -> Turn Receipt
                            |
                            v
C++/MLX Adapter
  ModelRuntime
    |- KV Layout Module
    |    |- contiguous layout
    |    `- paged layout + private block table
    `- Attention Module
         |- contiguous MLX SDPA
         |- paged gather -> MLX SDPA
         `- native block-table Attention
                            |
                            v
                     pinned MLX -> Metal
```

The KV Layout Module and Attention Module are private Modules inside one
ModelRuntime. Their internal seam exists because there are multiple real layout
and Attention implementations. The external Backend Interface remains deep:
callers submit one bounded Turn and receive one typed synchronized result.

All MLX arrays, page tables, compiled kernels, scratch arenas, and destruction
remain on the Device Executor owner thread.

## Canonical Route Description

The canonical Execution Route descriptor must record independent identities for:

- graph ABI and graph artifacts;
- weight-layout ABI;
- memory plan and arena;
- KV/cache layout ABI, including page size, block-table encoding, dtype,
  quantization, allocation, append, trim, and release semantics;
- Attention path, including implementation kind, kernel bundle, mask and
  position ABI, supported phase and Shape domain, block-table reader ABI, and
  route-local scratch plan;
- fusion, Speculative Decode, Prefix Reuse, and command-submission/replay plans.

The first three Attention path kinds are:

| Attention path | KV input | Meaning |
|---|---|---|
| `CONTIGUOUS_MLX_SDPA` | Contiguous | Baseline path over contiguous K/V. |
| `PAGED_GATHER_MLX_SDPA` | Paged | Gather the exact logical K/V view, then call the pinned MLX SDPA path. |
| `NATIVE_BLOCK_TABLE_ATTENTION` | Paged | A qualified native kernel reads the block table directly. |

These names describe route semantics, not environment flags. The final schema
uses stable numeric discriminants and canonical member encodings. Every absent
optimization remains explicit. A change in page geometry, mask semantics,
kernel source, compilation options, scratch plan, or fallback policy changes the
Execution Route Identity.

## Route Selection Laws

1. Candidate Formation may present only an exact route contained in every
   member's Authorized Capability Set.
2. The Work Candidate and Turn Plan freeze one Attention path and one KV layout.
3. The Adapter validates phase, Shape, dtype, layout, page-table, memory-plan,
   and kernel applicability before MLX execution begins.
4. An applicability mismatch returns Plan Rejection and consumes zero Engine
   Service. It cannot substitute another route within the call.
5. A failed or untrustworthy started native execution follows the existing Turn
   failure or fail-stop contract; it cannot be relabeled as a gather route.
6. A later fresh Scheduling Snapshot may select a separately authorized gather
   route after a rejection, subject to its own Capability Key and bounds.
7. Receipt telemetry records the intended route and typed actual outcome without
   turning observation into authorization.

An implementation may compile a kernel lazily only when the complete compile
operation and failure path are covered by the route's certified bound. A
runtime-generated kernel variant has a distinct artifact identity; geometry
success observed on one host cannot authorize another geometry or environment.

## Resource And Failure Contract

Admission and Turn feasibility must conservatively cover all route-specific
resources:

- persistent KV pages and block tables;
- allocator metadata and fragmentation allowance;
- gather destination buffers for `PAGED_GATHER_MLX_SDPA`;
- native kernel scratch, reductions, masks, and command submission;
- first-use compilation or an explicitly precompiled artifact;
- maximum in-flight lazy graph state through synchronization;
- rollback, request release, and Pending Reclaim.

Pool exhaustion, page-table corruption, unsupported geometry, and missing
artifacts are typed failures. No route may grow into unreserved contiguous state
or bypass the Resource Governor. Released physical memory remains charged until
the existing allocation-result and Pending Reclaim contracts prove reuse safe.

## Correctness Qualification

Qualification is exact for one Capability Key and includes:

- complete logits and updated-KV parity against the qualified baseline;
- deterministic greedy tokens and exact Sampling State behavior;
- Prefill and Decode cases for every declared batch and Shape bucket;
- page-boundary, partial-page, trim, cancellation, and maximum-context cases;
- supported Dense, MoE, grouped-query, mask, position, dtype, and quantization
  domains, with explicit exclusions for every unsupported layout;
- block-table bounds, duplicate/reused-page rejection where illegal, and stale
  generation rejection;
- Plan Rejection before execution for every unsupported geometry;
- owner-thread, synchronization, cleanup, and request-release fault injection;
- route identity drift and most-specific quarantine behavior.

Native and gather routes use separate records even when their tokens match.
Passing a single decode geometry cannot certify Prefill, multi-batch, another
head dimension, another page size, or another model family.

## Performance Probes

Every comparison records immutable route and environment identity plus:

- TTFT, TPOT, Turn latency, and end-to-end latency distributions;
- prompt and generation throughput and aggregate tokens per second;
- active MLX memory, allocator cache, process physical footprint, peak memory,
  compressor/swap deltas, and Pending Reclaim convergence;
- bytes gathered, gather operations, block-table entries read, scratch bytes,
  allocations, lazy evaluations, and command-buffer shape/coverage;
- kernel compilation count and time, Plan Rejections, failures, and route splits;
- output, logits, and updated-KV hashes.

Thresholds and workload matrices must be fixed before measurement. A native
route is promoted only when its complete required matrix passes correctness and
resource gates and its measured tradeoff is accepted for the exact environment.
An isolated kernel microbenchmark is supporting evidence, not a serving claim.

## Delivery Slices

| ID | Deliverable | Required verification |
|---|---|---|
| PA01 | Add canonical KV-layout and Attention-path descriptor members and stable discriminants. | Double-generation identity, one-member drift, malformed/unknown rejection. |
| PA02 | Add complete Capability Requirement, Profile, quarantine, and telemetry propagation for the exact path. | Fake/native conformance and exact-key isolation. |
| PA03 | Implement and qualify `PAGED_GATHER_MLX_SDPA` over private Paged KV. | Baseline parity, page/trim/exhaustion cases, gather resource bounds. |
| PA04 | Add one predeclared native decode geometry behind a separate route identity. | Kernel oracle, unsupported geometry rejection, no silent fallback. |
| PA05 | Expand only to explicitly qualified page, batch, Shape, dtype, mask, and model domains. | Complete case matrix and per-domain identity drift. |
| PA06 | Run serving, memory, and command-buffer qualification against the gather and contiguous routes. | Fixed thresholds, distributions, artifacts, and failure preservation. |
| PA07 | Consider promotion for exact profiles whose complete gates pass. | Profile-specific decision record; unchanged P0 baseline. |

Each slice keeps the coarse Backend Interface unchanged. PA04 and later do not
block PA03, and no native route is a prerequisite for Paged KV correctness.

## Completion Criteria

This plan is complete only when:

- the canonical descriptor makes KV layout and Attention path independently
  visible to exact Certification and quarantine;
- no native details cross the Rust/native Seam;
- every supported native case has exact correctness, resource, and performance
  evidence, with unsupported cases rejected before execution;
- no started Turn silently changes Attention paths;
- route-specific memory and cleanup remain conservative under cancellation,
  exhaustion, and failure; and
- product wording remains limited to the strongest completed evidence gate.
