# TurnVector P-1A/B/C Pre-Development Technical Validation Report

| Gate | Status | Product scope / decision |
|---|---|---|
| P-1A | **YELLOW** | Permit calibrated or synchronized engine-service fairness only; do not claim MLX command-buffer service-time fairness. |
| P-1B | **PENDING** | The two-hour shared-host precheck is complete, but it is not an exclusive 24-hour soak and does not certify 32/64/128 GB systems. |
| P-1C | **RED** | Do not select a Rust FFI boundary. Retain C++ Direct as the experimental baseline and make no product technology choice until the full-model per-op rerun is complete. |

This document records technical feasibility validation and technology-selection evidence gathered before formal product development. It is not a product implementation and contains no business logic. Apart from repository foundation files and rules that prevent accidental commits, TurnVector Git history retains only this English P-1 conclusion report. Experiment source code, run scripts, protocols, raw data, models, build products, and the Chinese report are not repository content.

The engineering disposition is to accept P-1A for P0 using an engine-service ledger and to accept P-1B as the development safety baseline for the current 256 GB host. Their formal statuses remain YELLOW and PENDING respectively; neither is expanded into a command-buffer fairness claim or a 32/64/128 GB support claim. The next step is a pure-Rust, backend-neutral P0 Turn Contract, Scheduler Core, and Fake L1 replay. P-1C remains RED, and no MLX FFI boundary is frozen until the full-model per-op rerun passes.

## Fixed environment

- TurnVector Git baseline: `d2930194413acad934c1207786d0f3b66092523f`. The experiment implementation was a dirty worktree, and every manifest records that fact.
- MLX: `68cf2fddd8de5edd8ab3d926391772b2e2cedad8`; mlx-c: `fba4470b89073180056c9ea46c443051375f7399`; Rust: `1.97.1`.
- macOS `26.4.1` (`25E253`), Apple M3 Ultra, 60-core GPU, 256 GB unified memory.
- Dense: `mlx-community/Qwen3-0.6B-4bit@73e3e38d981303bc594367cd910ea6eb48349da8`; MoE: `mlx-community/Qwen1.5-MoE-A2.7B-4bit@11aaad5b454a361ae33f19fb47b72bc74b3c3b55`.
- Input seed: `20260812`. During execution, models, build caches, complete JSONL/CSV files, and Instruments traces were under ignored `.work/`. After the experiment, they were moved with the validation source into an archive outside the repository. Git contains no model weights or raw traces.

## P-1A: Turn-Time Observability

The decision is **YELLOW**. The 18 Dense/MoE Decode and Prefill cases recorded 18,000 measured Turns and 981,813 Command Buffers. Timestamp coverage was 100.00%; attribution errors were 0, and missing turn IDs were 0. The maximum in-bucket Host/GPU ratio CV was 5.42%. The permitted product scope is `calibrated-or-synchronized-engine-service`.

Green threshold details:

| Metric | Observed | Green threshold | Result |
|---|---:|---:|---|
| Instruments median relative error | 7.83% | <= 10% | PASS |
| Instruments p95 relative error | 49.39% | <= 20% | FAIL |
| Telemetry throughput regression | -0.60% | <= 2% | PASS |
| Telemetry TPOT p95 regression | 2.00% | <= 3% | PASS |

The Metal System Trace calibration tail-aligned ordered in-process Command Buffers: 12,493 telemetry records, 13,232 Instruments intervals, and 12,493 resulting pairs. The two sources had no shared command-buffer identifier, so this pairing cannot support a Green p95 accuracy claim. No outliers were removed from the raw run.

## P-1B: Memory Truth Precheck

The formal status remains **PENDING**. The shared-host run lasted 2.000 hours with 35,268 samples at 200 ms. It covered static dual-model residency, shape churn, prefix restore/re-prefill/evict, an 8 GiB safety staircase, 100 load/unload/clear/reload cycles, and a mixed Decode plus Prefill soak. `available_memory` is a conservative `free+inactive+purgeable` host estimate because macOS does not expose `os_proc_available_memory()` to command-line tools. It is an experiment stop guard, not an OS capacity claim.

| Observation | Result |
|---|---:|
| Initial swap | 18.83 GiB |
| Swap growth | 0.00 GiB |
| Swapins / swapouts delta | 28 / 0 pages |
| Compressor delta | -0.43 GiB |
| Minimum available memory | 152.85 GiB |
| Footprint p95 | 11.09 GiB |
| Stable-soak footprint drift | 0.00 GiB |
| Lifecycle reclaim convergence | 100 / 100 |
| Reclaim p95 | 412.982 ms |
| Maximum pending reclaim | 0.00 GiB |
| Pressure events | none |
| Missing protected services | none |

The development Governor should stop admission or stress escalation when estimated available memory is at or below 64 GiB, pressure reaches warning/critical, new swap since this run began exceeds 64 MiB, or any protected service disappears. Memory requested for reclaim remains charged as `pending_reclaim_bytes` until process footprint converges. These values are precheck guards for the current 256 GB host, not a capacity envelope for smaller-memory systems.

The gate cannot become Green because the host was shared, was not rebooted to clear existing swap, ran for only two hours, and supplied no evidence from real 32/64/128 GB systems. The experiment did not stop or restart the VMs, SES orchestrator, or other services. Per-sample liveness tracked only the PIDs identified at startup: 66058, 66059, and 68893. A second VM and the Qiang SES orchestrator appeared only in process/background-load evidence and were not included in the per-sample set for this run.

## P-1C: FFI Grain

The decision is **RED**; there is no selectable full-model per-op boundary. Each microbenchmark path covered six Dense/MoE Decode buckets and three Prefill buckets, with 100 warmup and 1,000 measured Turns per case. A was C++ Direct, B was official mlx-c per-op at 11 calls per Turn, and C1 was an mlx-c compiled closure at one call per Turn. Because neither B nor C1 passed the complete Decode matrix, they triggered C2, a minimal C++ Turn ABI at one call per Turn. C2 used a C++ baseline resampled in the same phase and interleaved by case.

| Path | Maximum Decode p50 regression | Maximum Decode p95 regression | Output equivalent | CB shape fully equivalent | Failed cases |
|---|---:|---:|---|---|---:|
| `rust-per-op` | 19.81% | 12.91% | yes | yes | 2 |
| `rust-compiled-closure` | 15.27% | 14.73% | yes | no | 1 |
| `rust-coarse-turn` | 2.62% | 23.98% | yes | yes | 6 |

The specific per-op failures were:

| Model | Batch | Shape/context | p50 regression | p95 regression |
|---|---:|---:|---:|---:|
| moe | B1 | 512 | 19.81% | 12.91% |
| moe | B1 | 8192 | -0.11% | 11.88% |

All microbenchmark paths produced identical bit fingerprints. Per-op and C2 preserved the C++ Direct Command Buffer shape; C1 changed it through graph compilation and fusion. The microbenchmark used real first-layer weights but covered only RMSNorm plus a 4-bit MLP: the standard Dense MLP and the MoE shared expert.

The supporting full-model control covered 18 exact-shape cases with deterministic tokens and synthetic zero KV inputs. It compared the C++ API with a single official mlx-c import call. Cross-language fingerprints were stable and equivalent, complete-logits SHA256 values were equivalent, and workload `ops/bytes` shapes were equivalent; strict Command Buffer count distributions were not equivalent. Export also used Python direct to round-trip logits and the new KV slice for each signature. This control includes complete logits, updated KV, attention, and MoE top-k routing, but both sides invoke one exported complete graph and use synthetic zero KV state. It therefore cannot replace a full-model per-op performance test and does not change the RED decision.

## Evidence Archive and Git Boundary

- The P-1 external archive root is `/Users/chenyu/Documents/research-archives/TurnVector/p1-20260813/`. Its `repo-overlay/` directory preserves the complete pre-cleanup experiment worktree, including validation source, protocols, the MLX patch, compact results, raw run data, models, toolchains, and build products.
- At the archive root, `MANIFEST.md` describes the contents and recovery process, `SHA256SUMS` verifies every regular file, `SYMLINKS.tsv` records symbolic links separately, and `SOURCE-PDFS.sha256` records the paths and digests of the five external source PDFs. These archive indexes are not part of Git.
- The `/Users/chenyu/Documents/github/TurnVector/.work/` paths in original manifests are historical execution paths. Their archived equivalents are under `repo-overlay/.work/`. Reproduction should restore the overlay into a disposable worktree based on Git baseline `d2930194413acad934c1207786d0f3b66092523f`, not overwrite the current product repository.
- The Chinese report is retained locally at ignored `.work/reports/p1-experiment-report.zh-CN.md`. The five source PDFs remain in `/Users/chenyu/Downloads/doc/` and were not copied into either Git or the P-1 archive. Extracted text and page screenshots likewise remain outside Git.
