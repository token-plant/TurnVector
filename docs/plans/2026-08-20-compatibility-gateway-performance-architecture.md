# Compatibility Gateway Performance Architecture

Status: design-only performance contract; no measured product claim

Date: 2026-08-20

Governing decision: `docs/adr/0041-run-compatibility-gateways-outside-the-daemon.md`

Related contracts:

- `docs/plans/2026-08-19-compatibility-gateway-design.md`;
- `docs/plans/2026-08-20-gateway-lifecycle-uds-validation.md`;
- `docs/adr/0030-bound-ingress-and-keep-external-io-off-the-device-loop.md`;
- `docs/adr/0034-acknowledge-and-bound-the-live-request-lifecycle.md`.

## 1. Decision and scope

The Compatibility Gateway remains one separate, deep Module with one external
Interface:

```text
OpenAiChatGateway.handle(BoundedHttpRequest, ClientContext)
    -> GatewayExchange
```

Performance work deepens its Implementation behind that Interface. It does not
add a second external Interface, expose private stages, or move Request
Acceptance, Admission, scheduling, Model Residency, Backend ownership, KV,
execution-route, or durable authority out of the TurnVector Daemon.

The target Gateway is a bounded protocol compiler and stream bridge:

```text
bounded HTTP exchange
  -> authenticate and parse once
  -> freeze profile, route, defaults, and response bounds
  -> negotiate one request-owned Data Plane session
  -> render and tokenize once
  -> submit one immutable Token Request with attached follow
  <- consume ordered bounded Output Frames and terminal Status
  -> incrementally decode and frame once
  -> drain through one bounded HTTP writer
```

This document owns the allowed Gateway hot-path shape, work categories,
observations, and qualification requirements. It does not choose numeric
latency targets, authorize connection pooling or multiplexing, or claim that
any optimization is effective before a real production-path measurement.

Daemon and ModelRuntime optimization is deliberately out of scope. The only
cross-process performance contract is the semantic Data Plane contract in
section 10.

## 2. Research disposition and claim limit

The design uses two source-level patterns without importing their deployment
assumptions or benchmark results:

- [AX Engine architecture at
  `80f2a3e60db1a2ee1ff630bf318d1b3d376af178`](https://github.com/defai-digital/ax-engine/blob/80f2a3e60db1a2ee1ff630bf318d1b3d376af178/docs/ARCHITECTURE.md)
  keeps HTTP, SSE, async orchestration, and JSON at the server edge while its
  execution core and MLX layer own runtime behavior;
- [NInfer concurrent inference architecture at
  `feaf4dd0983fdaeb2ba4c06eec6da350e644fb3a`](https://github.com/Neroued/ninfer/blob/feaf4dd0983fdaeb2ba4c06eec6da350e644fb3a/docs/maintainer/concurrent-inference-architecture.md)
  keeps CPU ingress and output work outside its single GPU execution owner and
  puts compact batches, stable execution memory, and graph assets in the model
  runtime.

These are architecture inputs only. Their hardware, concurrency, batching,
graph, latency, and throughput results do not establish TurnVector Gateway
performance. No upstream constant becomes a TurnVector default through this
document.

The resulting placement rule is:

| Concern | Gateway placement | Daemon or runtime placement |
|---|---|---|
| External protocol, identity, TLS, JSON, SSE | owned | not owned |
| Template, tokenizer, incremental text decode | owned | not owned |
| Bounded external admission and slow-client handling | owned | daemon still owns Request Admission |
| Request lifecycle and output sequence | observed and translated | authoritative |
| Batch formation, KV, prefix hit, graph, attention route | forbidden | authoritative |
| Model Residency, memory victim choice, fairness | forbidden | authoritative |

## 3. Module depth and internal values

The private `DataPlanePort` remains the only internal Seam. The production Unix
Adapter and scripted in-memory Adapter justify that Seam. Tokenizer, template,
serializer, buffer-pool, clock, counter, or stage-specific traits must not be
added to the external Interface merely for testing.

The Implementation may use these request-local private values:

| Private value | Frozen contents | Lifetime |
|---|---|---|
| `RoutedExchange` | profile, immutable route, mapped parameters, principal decision, request and response bounds | route completion through close |
| `PreparedTokenRequest` | rendered input token IDs, stop token sequences, immutable Revision, generation parameters, Service Class | tokenization completion through submit outcome |
| `AcceptedExchange` | Request ID, attached follow state, output ordering state, decoder state, terminal classification | observed Acceptance through local close |

These names describe internal state transitions, not new Modules, Interfaces,
wire types, or persisted records. A test observes their effects through
`OpenAiChatGateway.handle`; it does not construct or mutate them directly.

The existing private Modules retain their current ownership:

- `CompatibilityProfile` owns strict schema, defaults, response bytes, and
  error rules;
- `RevisionRouteCatalog` owns immutable route and asset lookup;
- `PromptCodec` owns rendering, tokenization, stop tokenization, and
  incremental detokenization;
- `TurnVectorSession` owns the request-lifetime Data Plane sequence;
- `StreamBridge` owns ordered bounded JSON/SSE publication;
- `EdgePolicy` owns external authentication, route authorization, and external
  concurrency and rate limits.

No caller coordinates those Modules. Deleting the deep Gateway Module would
recreate their ordering, error, resource, and cancellation complexity in every
network entrypoint.

## 4. Request pipeline

The hot path preserves the ordering in the detailed design:

| Stage | Input | Output | Required behavior |
|---|---|---|---|
| Reserve | accepted transport exchange | fixed exchange and request-byte reservations | fail before body growth when capacity is absent |
| Authenticate | bounded headers and peer facts | bounded `ClientContext` | no request-content logging |
| Parse and map | reserved body bytes | `RoutedExchange` | one strict parse, one route lookup, checked bounds |
| Session prepare | frozen route and effective limits | negotiated `TurnVectorSession` | authenticate, select exact descriptor, verify required capability and limits |
| Codec prepare | routed messages and immutable route codec | `PreparedTokenRequest` | one render and tokenization pass on the bounded CPU executor |
| Submit and follow | prepared Token Request | rejection or `AcceptedExchange` | one submit, complete Direct Response before external success, no replay |
| Translate | ordered Output Frames and terminal Status | bounded JSON or SSE bytes | one incremental decoder, stable output ordering and terminal mapping |
| Drain and close | Gateway-owned response bytes | closed exchange | bounded writer deadlines, release all reservations exactly once |

Session preparation remains before tokenization so an unavailable or
incompatible daemon does not consume an expensive tokenization job. The
request's negotiated Data Plane limits must also prove that the exact mapped
request and maximum Direct Response fit before submission.

CPU-heavy rendering and tokenization run only on the bounded CPU executor.
Socket tasks may move bounded frames and bytes but do not execute templates,
tokenize, perform full JSON reprocessing, or wait on an unrelated exchange.

## 5. Single-work laws

For every external exchange that reaches the named stage, the Implementation
must satisfy these laws:

1. **Single parse.** The reserved request body is strictly parsed once. Later
   stages consume the parsed bounded representation and never parse the JSON
   body again.
2. **Single route fixation.** External model lookup, profile defaults, route
   authorization, and the exact immutable route identity are fixed once.
   Readiness refresh or daemon status cannot silently reroute the request.
3. **Single render.** The chat template is executed once for the accepted
   message sequence.
4. **Single tokenization.** Rendered prompt text and every stop string are
   tokenized once under the same frozen tokenizer. The daemon receives token
   IDs and never asks the Gateway to tokenize again.
5. **Single incremental decode.** Each ordered output token contributes to one
   request-local decoder state exactly once. Split UTF-8 and tokenizer suffix
   state may be retained only within the declared bound.
6. **Single external serialization.** Each visible output byte is escaped and
   framed for its final JSON or SSE position once. Partial socket writes resume
   the same frame; they do not reconstruct it.
7. **No text re-tokenization.** Usage, progress, finish reason, stop handling,
   and prefix compatibility are never derived by tokenizing decoded output.
8. **No request-path asset I/O.** Route, template, tokenizer, profile, and
   serializer tables are process-owned immutable objects created before
   readiness. Request handling never reopens their paths.

A failed stage releases its owned reservations without rerunning an earlier
stage. A client-created new HTTP request is a new exchange and is outside these
single-exchange laws.

## 6. Immutable process working set

Before Gateway Readiness, startup must:

- verify and single-read the canonical profile, route manifest, template, and
  tokenizer artifacts under the existing ownership and digest rules;
- construct the immutable bounded route lookup structure;
- instantiate each route's tokenizer and validated executable template
  representation;
- construct fixed response, error, and SSE constant fragments whose bytes are
  profile-defined;
- validate all derived size formulas and reject a route whose maximum request
  or response cannot fit binary hard maxima.

The lookup Implementation may use a sorted table, bounded map, or another
deterministic structure, but it must declare and test its worst-case entries
examined and key bytes compared. It may not linearly scan an unbounded route
list or create a request-content cache.

No shared prompt, rendered-text, token-sequence, or decoded-output
request-content cache is authorized. Such a cache would introduce content
retention, cross-principal isolation, invalidation, and independent resource
policy that this profile does not need. An authenticator-owned bounded
credential or verification-key cache is allowed only when it is part of the
fixed authenticator profile, has explicit privacy, expiry, and capacity rules,
and never caches route authorization or Admission decisions. Other reuse is
limited to immutable code, route assets, serializer fragments, and bounded
empty buffers whose logical length is reset before reassignment and whose
serialization never reads unwritten capacity.

## 7. Work, allocation, and copy budgets

Structural byte bounds alone are insufficient. Before production enablement,
the Gateway build must declare a Gateway Work Budget for one exchange and for
the configured maximum concurrent exchanges. This is a local Gateway design
artifact, not daemon Admission, scheduling, or Backend budget authority.

For one exchange define:

```text
H = accepted HTTP header bytes
J = accepted JSON body bytes
R = rendered prompt bytes
I = input token count plus stop-token count
F = received Output Frame count
T = received output token count
D = decoded visible output bytes
E = final external JSON or SSE bytes
```

The work envelope is compositional:

```text
W_prepare = W_auth(H)
          + W_parse(J)
          + W_route
          + W_render(J, R)
          + W_tokenize(R, I)
          + W_session_setup

W_stream  = W_frame_decode(F, T)
          + W_text_decode(T, D)
          + W_external_frame(D, E)
          + W_socket_write(E)
```

Every term is bounded by canonical profile or negotiated Data Plane maxima.
These formulas classify work; they are not latency predictions. Numeric
operation, byte, and allocation ceilings require implementation evidence and
must be fixed before the relevant production gate.

The per-exchange memory envelope separately accounts for:

- reserved HTTP headers and body;
- parsed bounded request representation;
- rendered prompt bytes;
- input and stop token storage;
- Data Plane encode/decode frame storage;
- incremental detokenizer suffix;
- pending decoded bytes and JSON/SSE frame storage;
- non-streaming response or streaming output queue capacity;
- fixed request, identity, deadline, and counter state.

Process-wide immutable route assets are charged to startup capacity and not
charged again to every exchange. Shared buffer capacity is charged while
reserved even when its current logical length is zero.

The Implementation must expose content-free counters for:

- allocations and allocated capacity by fixed stage class;
- bytes copied into rendered text, token storage, Data Plane frames, decoder
  output, external frames, and socket writes;
- frames encoded, decoded, coalesced, and partially written;
- maximum request-local and process-wide queue occupancy;
- route entries examined and tokenizer/template invocations.

Zero-copy is not a design claim. TLS, Unix transport, Protobuf, tokenizer, and
JSON boundaries may require copies. The requirement is to identify unavoidable
copies, remove duplicate application-layer transformations, bound the
remaining work, and measure it without request content.

## 8. Concurrency and backpressure

Gateway exchange admission remains external edge policy, not daemon Request
Admission. It may reject before submission when its fixed concurrent-exchange,
principal, route-class, rate, body, tokenization-job, file-descriptor, or
response-capacity limits are unavailable. It cannot infer daemon queue position
or accept work that the daemon rejects.

The bounded CPU executor has separate active-worker and pending-job limits.
Queuing a tokenization job consumes a reservation fixed before enqueue. An
exchange that cannot reserve the job or its complete result bounds fails
without growing a general task queue.

Each accepted request has independent Data Plane and external response
capacity. A slow or stalled reader can consume only its reservation. It cannot
hold a global serializer lock, tokenization worker, Data Plane reader for an
unrelated request, daemon Runtime capacity after qualified release, or Device
Executor work.

Output publication follows the existing bounded-decoupling rule:

- output that fits the reserved Gateway response capacity may reach terminal
  and Request State Release while the HTTP reader continues to drain;
- a full response queue makes the request non-runnable or orders the frozen
  cancellation/disconnect outcome;
- no queue grows to manufacture decoupling, and no terminal result is dropped.

The exact lifecycle proof and `t0` through `t5` definitions remain in
`docs/plans/2026-08-20-gateway-lifecycle-uds-validation.md`.

## 9. Unix transport baseline and later candidates

Version one retains one external exchange per fresh Data Plane Unix
connection. The request owns the complete negotiation, follow stream,
cancellation, failure, and close outcome. This preserves fault isolation and
keeps the local connection as the Request Ownership unit.

This document authorizes no pooling or multiplexing. The fixed setup cost `F`,
hypothetical reuse factor `k`, candidate overhead `M`, and decision inequality
are defined by the lifecycle and Unix validation contract.

If measured evidence justifies a candidate, the first allowed experiment is a
pre-negotiated single-borrower connection pool behind the existing private
`DataPlanePort`. Each exchange has exclusive use of one connection for the
complete borrow; no concurrent borrower shares its Request ID, frame, failure,
or cancellation state. The candidate must define a reset-or-retire outcome and
prove that a connection returned to the pool carries no descriptor, Command ID,
Request ID, output, terminal, or cancellation state into the next borrow. A
connection that cannot prove the reset is retired rather than returned.

Multiplexing multiple live requests on one connection changes ownership,
failure isolation, fairness, cancellation, and backpressure semantics. It
requires a new ADR, Data Plane capability, validation contract, and profile
compatibility review.

## 10. Semantic-only Data Plane contract

The Gateway may send only profile-mapped semantic request facts already owned
by the native Data Plane contract:

- immutable Model Revision;
- input token IDs and Stop Token Sequences;
- closed Generation Parameters and optional Sampling Seed;
- Max Output Tokens and required Service Class;
- the bounded metadata explicitly permitted by the Data Plane profile.

It may consume only typed Request Acceptance, Request ID, ordered token Output
Frames, progress, terminal reason, and protocol/session facts needed for
translation.

The Gateway must not send or receive optimization authority for:

- candidate or batch membership, compact row, batch bucket, or padding;
- prefix hit, prefix entry, KV page, KV layout, or cache victim;
- graph, compilation, attention, speculative, or operator route;
- Model Residency choice, memory reservation, fairness debt, queue position,
  Turn budget, or overlapping-Turn policy.

Exact token IDs are execution and prefix-compatibility input. Gateway
tokenizer/template identity proves how it produced them but cannot substitute
for daemon validation or native token equality. An optimization hint never
authorizes a Backend path, so version one carries no such hint.

## 11. Production observations

Observability uses fixed event kinds and bounded numeric fields. It records no
headers, prompt text, rendered text, token IDs, decoded output, response bytes,
credentials, principal identifiers, local paths, or daemon free text.

The minimum Gateway-local stage events are:

| Event | Observation |
|---|---|
| `authentication_complete` | bounded authentication class completed |
| `profile_parse_complete` | strict parse and mapping completed |
| `route_fixed` | immutable route identity selected |
| `data_session_ready` | peer authentication, negotiation, capability, and limits accepted |
| `tokenization_complete` | rendered/input/stop counts and declared work counters finalized |
| `submit_started` | first submit byte may be delivered |
| `acceptance_observed` | complete Direct Response accepted by the Gateway |
| `first_output_observed` | first ordered Output Frame accepted by the Gateway |

The unchanged lifecycle events `exchange_reserved`,
`backend_ownership_acquired`, `terminal_observed`,
`request_state_release_accepted`, `http_last_byte_committed`, and
`response_closed` are imported from the lifecycle and Unix validation contract.
This document does not redefine their meanings, clock requirements, or gates.

Absent optional events remain absent rather than receiving fabricated zero
durations. One monotonic clock domain is required for claimable stage and
lifecycle comparisons. Wall time remains presentation metadata.

Each trial retains at least stage durations, process CPU time, context switches,
allocation counters, categorized copy bytes, exact frame and external byte
counts, queue occupancy, connection count, file-descriptor high-water mark,
terminal class, and error class. Instrumentation overhead requires paired
enabled/disabled measurements and may diagnose Gateway cost only; it cannot be
charged to or described as Backend Engine Service.

## 12. Verification and qualification

### 12.1 Module laws

Tests through `OpenAiChatGateway.handle` must prove:

- identical external bytes between the baseline and any optimized
  Implementation for every golden request, response, SSE split, and error;
- exact single-work invocation counts at the applicable stages;
- no request-path asset open or metadata read after readiness;
- bounded allocation, copy, lookup, queue, and frame counters at minima,
  maxima, and one-past rejection cases;
- streaming and non-streaming equivalence under every token and UTF-8 split;
- independent progress for concurrent fast and slow exchanges;
- complete reservation release after success, every typed failure,
  cancellation, disconnect, and partial write.

The in-memory Adapter proves deterministic Module behavior and remains
non-claimable for production latency, Unix cost, and lifetime decoupling.

### 12.2 Production matrix

The real HTTP Gateway, Unix Data Plane, daemon, and Backend lifecycle are
required for a publishable result. The fixed matrix crosses:

- request class: minimum accepted and profile-maximum bounded;
- response class: non-streaming, streaming, slow, stalled, overflow, and
  disconnect;
- concurrency: `1`, `8`, `32`, and `128` where admitted by the frozen profile;
- process state: cold and warm, reported separately;
- instrumentation: disabled and enabled;
- outcome: success and every exercised typed failure.

This matrix extends rather than redefines the fixed lifecycle `CasePlan`.
Lifecycle outcomes and gates are reduced under the lifecycle and Unix
validation contract.

The report preserves p50, p95, p99, min, max, sample count, all failures, and
immutable identity. Raw artifacts remain outside Git under the custody rules in
`docs/evidence-policy.md`; `report.json` is the structured source of truth and
derived Markdown cannot add a stronger claim.

No comparison may subtract model time from end-to-end latency and call the
remainder exact Gateway service unless the event ordering, clock domain, and
overlap make that decomposition valid. Stage spans, CPU counters, and byte/work
counters remain the primary Gateway evidence.

## 13. Delivery integration

The existing G01 through G08 sequence remains authoritative. This performance
contract adds these gates without adding new delivery slices:

| Existing slice | Added performance gate |
|---|---|
| G01 | canonical profile fixes every work and byte variable needed by the budget |
| G02 | the deep Module retains one Interface and exposes deterministic stage counters only through normal telemetry |
| G03 | immutable codecs and single render/tokenize/decode laws pass at exact bounds |
| G04 | one-request Unix sessions expose setup stages and categorized wire work |
| G05 | stream buffers, partial writes, slow readers, and lifetime tails remain bounded and independent |
| G06 | edge limits reject before expensive work and never become daemon Admission authority |
| G07 | allocation, copy, lookup, queue, file-descriptor, fault, and instrumentation gates pass |
| G08 | the independent report keeps prediction, observation, and claim state separate |

Production enablement still requires the daemon dependencies and all G01
through G07 gates named in the detailed Gateway design. G08 qualifies claims;
it does not repair a failed resource or behavior gate.

## 14. Target result

The design is complete when it permits an Implementation with all of these
properties:

- one deep Gateway Module and one private owned transport Seam;
- one bounded pass from external request bytes to an immutable Token Request;
- one bounded pass from token Output Frames to final external bytes;
- immutable startup route and codec objects with no request-path asset I/O;
- explicit operation, allocation, copy, queue, connection, and byte budgets;
- Backend ownership that can end before a slow HTTP response closes when
  reserved capacity permits;
- one-request connection ownership retained until evidence justifies a
  separately reviewed candidate;
- no scheduler, ModelRuntime, KV, prefix, graph, or execution-route authority in
  the Gateway;
- no performance claim before a reproducible production-path report.

This is an edge-path optimization contract. It deliberately leaves
Daemon/ModelRuntime vertical optimization for a separate architecture decision
and review.
