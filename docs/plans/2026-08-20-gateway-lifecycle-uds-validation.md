# Gateway Lifecycle and Unix Connection Validation

Status: validation contract for later implementation and qualification

Contract identity: `turnvector.gateway-validation.v1`

Date: 2026-08-20

Related architecture:

- `docs/adr/0030-bound-ingress-and-keep-external-io-off-the-device-loop.md`;
- `docs/adr/0034-acknowledge-and-bound-the-live-request-lifecycle.md`;
- `docs/adr/0041-run-compatibility-gateways-outside-the-daemon.md`;
- `docs/plans/2026-08-19-compatibility-gateway-design.md`;
- `docs/plans/2026-08-20-compatibility-gateway-performance-architecture.md`.

## 1. Decision and scope

Gateway validation is split across three evidence layers:

1. this document owns terminology, formulas, observation requirements, claim
   limits, and the decision order;
2. TurnVector owns only production behavior and content-free observations from
   the real Gateway, Data Plane, daemon, and Backend lifecycle;
3. TurnVectorBenchmark owns cases, independent clients and collectors, metric
   reduction, gates, reports, and raw-artifact custody.

The analysis answers two questions:

- whether Request Backend ownership can end before the external HTTP response
  lifetime ends; and
- whether one Unix Data Plane connection per request costs enough to justify a
  later optimization experiment.

The first question is a correctness and ownership question. The second is a
performance and architecture-decision input. Neither permits a performance
claim from formulas, a fixture, or an in-memory Data Plane adapter.

This contract does not own the complete Gateway hot-path performance model.
Stage work, allocation, categorized copy bytes, queue occupancy,
instrumentation overhead, and semantic-only Data Plane placement are defined by
the Gateway performance architecture. This document retains sole ownership of
the `t0` through `t5` lifecycle decision and the per-request Unix connection
candidate rule so the same cost is not judged under two contracts.

"Model lifetime" is not used for this analysis. Model Residency can span many
requests and is governed independently. The measured lifetime is one request's
Backend ownership.

## 2. Module and repository placement

`OpenAiChatGateway.handle` remains the Gateway Module Interface. Validation
does not add lifecycle callbacks, collector traits, or a test-only public
Interface to it. The existing private `DataPlanePort` remains the only internal
Seam, with the production Unix adapter and scripted in-memory adapter.

Production observations use the normal bounded telemetry path. They carry no
prompt text, token IDs, response bytes, credentials, principal identifiers, or
unbounded labels. A qualification adapter may launch the real processes and
return their immutable identities, but it cannot report a case as passed or
substitute generated observations for production behavior.

The independent Benchmark Module has one Interface:

```text
load gateway validation contract
    -> inspect exact CasePlan
    -> validate immutable evidence
    -> GatewayValidationReport
```

Planning, trace validation, formulas, reduction, gates, and claim status remain
behind that Interface. They do not enter the Gateway or daemon.

## 3. Identity and clock contract

Every run binds:

- TurnVector and TurnVectorBenchmark Git revisions and clean status before and
  after collection;
- Gateway and daemon executable hashes;
- Compatibility Profile, Route Manifest, tokenizer/template, Model Revision,
  and Data Plane descriptor hashes;
- effective Gateway and Data Plane limit identities;
- hardware identity and exact macOS build;
- workload contract, case plan, warmup/sample counts, and artifact hashes.

Version-one claimable evidence uses one monotonic clock domain shared by every
event source. Wall clock is metadata only. A calibrated multi-clock mode needs
a later contract with a hash-bound calibration artifact and an independent
mapping reducer; a calibration digest alone is insufficient.

The evidence manifest and run manifest bind every lifecycle row to one run, and
each row names its case. Nested events carry a bounded request role (`a` or
`b`), event kind, monotonic timestamp, and bounded sequence. They never contain
request or response content.

## 4. Lifetime definitions

For request A, the minimum timepoints are:

| Symbol | Content-free event | Meaning |
|---|---|---|
| `t0` | `exchange_reserved` | Gateway reserves the bounded external exchange slot |
| `t1` | `backend_ownership_acquired` | accepted materialization creates complete request Backend ownership |
| `t2` | `terminal_observed` | Gateway observes the request's terminal Data Plane status |
| `t3` | `request_state_release_accepted` | Core accepts Backend Request State Release; request Backend ownership ends |
| `t4` | `http_last_byte_committed` | the last response byte is accepted by the HTTP transport |
| `t5` | `response_closed` | the external response stream is closed |

The primary durations are:

```text
T_backend  = t3 - t1
T_tail     = max(0, t5 - t3)
T_response = t5 - t0
```

`T_tail > 0` is necessary but not sufficient evidence of useful decoupling. A
qualifying slow/stalled-client case also proves that request B makes production
progress after A's `t3` and before A's `t5`. This prevents a timestamp-only
implementation from hiding a global owner, admission slot, or executor stall.

The Gateway may retain its bounded response buffer, accounting, and HTTP task
through `t5`. Those are Gateway resources, not Backend ownership and not
Backend or Runtime service time.

## 5. Bounded decoupling semantics

The target behavior is bounded decoupling, not unconditional early release:

- the Device Executor never writes or waits for an HTTP or Unix socket;
- every produced Output Frame has prior bounded capacity;
- output that fits the reserved response capacity can reach terminal and
  Request State Release even while an external reader drains slowly;
- a full response queue makes the request non-runnable or orders bounded
  cancellation/disconnect according to the frozen limits;
- no queue grows to preserve decoupling, and no terminal or release result is
  silently discarded;
- external disconnect never causes retry, replay, resume, or ownership transfer.

If a producer emits at `r_p` bytes per second, a client drains at `r_c`, current
occupancy is `b`, and usable reserved capacity is `B`, then for `r_p > r_c`:

```text
T_fill = (B - b) / (r_p - r_c)
```

This is a prediction for case construction. It is not an observed backpressure
deadline and cannot replace the production event trace.

At steady state, Little's Law is retained only as a diagnostic consistency
check:

```text
N_backend = lambda * E[T_backend]
N_tail    = lambda * E[T_tail]
```

It is not a safety gate. Reservations, hard maxima, event ordering, and
deadlines remain the safety contract.

## 6. Lifecycle CasePlan and gates

The version-one CasePlan contains these exact behaviors:

| Case | Response | Reader | Required result |
|---|---|---|---|
| `fast-fit` | non-streaming, fits reservation | fast | normal control path and complete release |
| `slow-fit` | streaming, fits reservation | slow | `t3 < t5` and request B progresses in A's tail |
| `stalled-fit` | streaming, fits reservation | stalled | terminal/release precede response close and B progresses |
| `stalled-overflow` | streaming, exceeds reservation | stalled | backpressure, frozen deadline outcome, cancellation/close, and no leak |
| `disconnect-mid-stream` | streaming | disconnects | ordered cancellation, terminal/release, no replay, and no leak |

The independent judge derives at least these gates:

- exact causal event order and one terminal/release outcome per accepted request;
- zero Device Executor socket waits;
- zero unreserved output publications and capacity overruns;
- zero duplicate output, retry, replay, resume, or ownership-transfer events;
- slow/stalled fit cases demonstrate Backend-to-response lifetime decoupling and
  peer progress;
- overflow and disconnect cases finish within their frozen effective limit
  envelope and retain no Backend, exchange, queue, or file-descriptor leak;
- reported `T_backend`, `T_tail`, `T_response`, and queue occupancy are
  recomputed from raw events rather than accepted from the subject.

An in-memory adapter can pass state-machine conformance but remains
`not_claimable_fixture`. The lifetime claim requires the real HTTP Gateway,
production Unix Data Plane, daemon, and Backend lifecycle.

## 7. Unix connection cost model

The Data Plane wire cost includes every four-byte big-endian frame length and
the exact serialized Protobuf payload:

```text
B_wire = sum(4 + protobuf_payload_bytes)
```

For one request, define fixed connection/session setup cost:

```text
F = socket
  + connect_accept
  + peer_credential
  + Hello
  + descriptor_validation
```

Let `V` be request-variable transport work after setup. One request per
connection costs:

```text
C_one = F + V
```

For a hypothetical reuse factor `k` and extra per-request reuse or
multiplexing cost `M`:

```text
C_reuse = F / k + V + M
benefit exists only if F * (1 - 1 / k) > M
```

Before a candidate exists, `M` is unknown. The version-one report therefore
publishes the perfect-reuse upper bound `F * (1 - 1 / k)` separately from
observed measurements and never labels it measured savings.

## 8. Unix measurement matrix

The Benchmark freezes warmups, repetitions, execution order, and cooldown
before collection. It crosses:

- probe path: kernel Unix connect/accept/peer-credential and full production
  Data Plane request session;
- concurrency: `1`, `8`, `32`, and `128`;
- request wire class: minimum accepted request and profile-maximum bounded
  request/Direct Response;
- state: process-cold and process-warm, reported separately.

Every raw trial retains stage durations, CPU time, context switches, exact
frame/payload byte counts, connection count, file-descriptor high-water mark,
errors, and client-observed first-response latency. Reports retain p50, p95,
p99, min, max, and sample count; no failure is removed as an outlier.

The OS probe can isolate the maximum cost attributable to Unix setup. Only the
full production path can support a Gateway design decision. Neither can support
model latency, throughput, or fairness claims.

## 9. Evidence and report states

Raw JSONL, process samples, traces, and profiler output stay outside Git in a
Benchmark-created artifact root. Git contains only compact contracts, schemas,
reducers, gates, tests, and dated conclusion reports. Every external file is a
regular non-symlinked relative path with recorded size and SHA-256.

`report.json` is the structured source of truth. Markdown is derived from it.
The report keeps predicted and observed values in separate fields and uses:

- `not_claimable_fixture` for scripted or in-memory evidence;
- `not_publishable` for identity, custody, completeness, calibration, or
  lifecycle-gate failure;
- `publishable` for complete real-system evidence;
- `measured_baseline` for the Unix result, never `pooling_qualified`;
- `backend_response_lifetimes_decoupled` only when every lifecycle gate passes.

Valid unfavorable measurements remain in the report. They do not become
missing data or a failed harness.

## 10. Architecture decision rule

Version one validates the current one-request-per-connection design only. It
does not implement or authorize connection pooling or multiplexing.

Decision order after a real run:

1. compare the perfect-reuse upper bound against a performance budget declared
   before the run;
2. retain per-request connections if even perfect reuse cannot repay the
   budgeted complexity or material latency/CPU cost;
3. if it can, prototype a pre-negotiated single-borrower connection pool and
   measure its real `M` in a paired same-session contract;
4. retain one-request ownership and discard the candidate if the measured
   inequality does not hold or any lifecycle/isolation gate regresses;
5. require a new ADR and a new qualification contract before multiplexing,
   because multiplexing changes failure isolation, peer ownership, cancellation,
   fairness, and backpressure semantics.

No model fitting or benchmark result automatically changes a runtime default.
The architecture changes only through a reviewed versioned decision with its
supporting immutable evidence.

## 11. Delivery order

1. Land this contract and the independent Benchmark judge.
2. Keep the Benchmark fixture non-claimable while G04/G05 are unavailable.
3. Add content-free production observations with the Gateway implementation;
   do not add a second public Gateway Interface.
4. Add the real qualification adapter after G04 and G05 pass.
5. Run the fixed lifecycle and Unix matrices with both repositories unchanged.
6. Publish a dated report and make the ownership/connection decision separately.
