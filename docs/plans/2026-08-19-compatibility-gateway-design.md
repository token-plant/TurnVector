# TurnVector Compatibility Gateway Detailed Design

Status: proposed implementation design

Date: 2026-08-19

Decision: ADR 0041

First external profile: `turnvector.openai-chat.v1`

Required local protocol: TurnVector Data Plane major 1

## 1. Purpose

TurnVector deliberately exposes only authenticated, bounded local Data and
Control Plane sockets. Applications still need a conventional network edge for
existing client libraries. The Compatibility Gateway supplies that edge without
turning the daemon into an Internet-facing server or confusing the private
C-compatible native shim with a public protocol adapter.

This design fixes the first production shape:

```text
OpenAI-compatible HTTPS client
              |
              | strict JSON and optional SSE
              v
  Compatibility Gateway process
    - TLS, external authentication, quotas
    - exact profile and route manifest
    - chat template, tokenizer, detokenizer
    - parameter and error translation
              |
              | authenticated local Unix socket
              | TurnVector Data Plane v1 only
              v
       TurnVector Daemon
    - Request Acceptance and ownership
    - Admission, scheduling, resource policy
    - Device Executor and private C shim
              |
              v
             MLX
```

The first external profile is a strict text-only subset of
`POST /v1/chat/completions`. The official OpenAI reference currently defines
message-based requests, non-streaming completion objects, and streamed chat
completion chunks over SSE. It also contains many features TurnVector P0 does
not implement. The source snapshot for this design is the official OpenAI API
reference read on 2026-08-19:

<https://developers.openai.com/api/reference/resources/chat/subresources/completions/methods/create>

That page is a source for the external shape only. The canonical TurnVector
profile artifacts and golden fixtures defined below are the implementation
contract. Later changes to the external page do not silently change a deployed
gateway.

## 2. Scope

### 2.1 In scope

- one separate Rust gateway process;
- HTTPS, bearer or mutual-TLS authentication selected by deployment;
- one strict OpenAI Chat Completions text profile;
- `stream=false` JSON and `stream=true` SSE;
- exact external model route to immutable Model Revision;
- exact chat template, tokenizer, stop-token, and detokenizer behavior;
- deterministic parameter defaults owned by the compatibility profile;
- one request-lifetime Data Plane connection per external exchange;
- bounded request parsing, tokenization, buffering, streaming, and writes;
- typed translation of gateway, protocol, and request outcomes;
- content-free metrics and identity-rich diagnostics;
- an in-memory Data Plane adapter for deterministic tests.

### 2.2 Out of scope

- a network listener inside the TurnVector Daemon;
- any Control Plane proxy or access from the gateway identity;
- direct calls into the Backend Interface or C++/MLX Adapter;
- tools, function calls, images, audio, files, embeddings, or moderation;
- multiple choices, log probabilities, structured output, or hidden reasoning;
- persisted conversations, request recovery, output replay, or idempotency;
- transparent retry after any local protocol or daemon failure;
- a generic provider-neutral intermediate representation or plugin framework;
- hot reload of routes, templates, tokenizers, limits, or credentials;
- a claim of complete OpenAI API compatibility;
- changes to TurnVectorBenchmark in this design task.

The Responses API may become a distinct future Compatibility Profile. It does
not share a profile version or silently reuse Chat Completions semantics.

## 3. Ownership

| Concern | Sole owner | Explicit non-owner |
|---|---|---|
| External TLS, authentication, rate limits, HTTP and SSE | Compatibility Gateway | TurnVector Daemon |
| External model name and compatibility defaults | Gateway Profile and Route Manifest | model config, Backend, daemon |
| Chat rendering, tokenization, stop-string tokenization, text decoding | Compatibility Gateway | TurnVector Daemon, Backend |
| Immutable Model Revision and Token Request validation | TurnVector Daemon | Compatibility Gateway |
| Request ID, Request Ownership, lifecycle, cancellation, output sequence | TurnVector Daemon | Compatibility Gateway |
| Admission, Resource Reservation, Service Class policy, scheduling | TurnVector Daemon | Compatibility Gateway |
| Model Residency and artifact validation | TurnVector Daemon | Compatibility Gateway |
| MLX objects, KV, streams, Turns, native errors | Device Executor and C++/MLX Adapter | Compatibility Gateway |
| External response shape and error translation | Compatibility Gateway | TurnVector Daemon |
| Durable Control State and Audit | TurnVector Daemon | Compatibility Gateway |

The gateway does not improve or weaken a daemon decision. It may reject work
before submission under its own external contract, but it cannot convert a
daemon rejection into acceptance, change Service Class after Request
Acceptance, or request Exclusive Mode.

## 4. Deep module shape

The gateway is one deep module. Its caller and black-box tests use one
interface:

```text
OpenAiChatGateway.handle(BoundedHttpRequest, ClientContext)
    -> GatewayExchange
```

`GatewayExchange` is either one bounded JSON response or one bounded SSE event
stream. The interface includes the documented limits, cancellation behavior,
and error modes; callers do not coordinate tokenization, Data Plane commands,
or stream ordering themselves.

Private implementation modules are:

| Private module | Responsibility |
|---|---|
| `CompatibilityProfile` | Strict request schema, defaults, field mapping, response and error rules |
| `RevisionRouteCatalog` | Validated external model routes and immutable tokenizer/template assets |
| `PromptCodec` | Chat rendering, tokenization, stop tokenization, and incremental detokenization |
| `TurnVectorSession` | One request-lifetime Data Plane negotiation, command sequence, ownership, cancellation, and terminal classification |
| `StreamBridge` | Output ordering, bounded JSON/SSE construction, slow-reader handling, and usage accounting |
| `EdgePolicy` | External authentication result, route authorization, concurrency and rate limits |

Only the private `DataPlanePort` is an internal seam. The production adapter
uses the local Unix socket and a scripted in-memory adapter drives module-level
tests. No other private module gets a public trait merely for mocking.

The intended crate layout is:

```text
crates/turnvector-openai-gateway/
  src/lib.rs          # OpenAiChatGateway interface
  src/profile.rs      # canonical profile and limit validation
  src/routes.rs       # route manifest and immutable assets
  src/prompt.rs       # template/tokenizer/detokenizer
  src/session.rs      # request-lifetime Data Plane state machine
  src/stream.rs       # JSON and SSE publication
  src/http.rs         # HTTP transport adapter
  src/main.rs         # composition root only
```

The layout is guidance, not permission to implement before the protocol gates
in section 16 pass.

## 5. Canonical profile and route inputs

### 5.1 Compatibility Profile

The build contains one canonical `turnvector.openai-chat.v1` descriptor and
identity. Its canonical bytes define:

- the exact accepted JSON fields and value domains;
- every profile-owned default;
- message roles and content forms;
- parameter-to-Token-Request mapping;
- response objects, SSE chunks, and finish reasons;
- error codes and HTTP status mapping;
- every binary hard maximum and limit formula;
- the required Data Plane capabilities and typed limits.

Generation is deterministic and checked twice. The binary embeds the expected
descriptor identity and refuses readiness on drift. Profile v1 has no external
minor negotiation: any change to accepted fields, defaults, response presence,
or behavior creates a new profile major, name, descriptor, and coordinated
gateway replacement. A client cannot select a different profile through a
header, query parameter, or request field.

The profile requires the negotiated Data Plane capability
`ACCEPTED_REQUEST_FOLLOW_V1`. A submit carrying `follow_output=true` has this
linearization contract:

1. before Request Acceptance, the daemon reserves the complete per-request
   follow state, Direct Response, and terminal Status capacity and attaches live
   Output Frame and Status delivery to the submitting owning connection; later
   Output Frames still consume their existing P14/P15 per-Turn reservations;
2. inability to reserve or attach returns a bounded rejection before Request ID
   allocation;
3. after attachment, Request Acceptance may become visible to the Runtime Core;
4. the connection's single writer emits the complete acceptance Direct Response
   before every causally later Output Frame or Status Update;
5. while the connection remains live, exactly one terminal Status carries the
   closed terminal-reason enum and exact generated-token progress after every
   preceding Output Frame; and
6. disconnect releases outbound capacity and orders cancellation, while no later
   subscription, connection, snapshot, or History Gap can replace the attached
   output-and-terminal follow stream.

The negotiated descriptor exposes a nonzero
`max_accepted_request_follows_per_connection` and the effective response, frame,
token, and terminal limits. The gateway requires at least one follow because it
uses one request per connection. A peer that exposes ordinary submit,
subscription, or output commands without this capability is incompatible and
cannot make Gateway Readiness true.

### 5.2 Revision Route Manifest

One bounded, canonical, root-owned, non-symlink regular file is single-read at
startup before the network listener becomes ready. Group or other write access,
duplicate keys, unknown fields, non-canonical identities, missing assets, or
digest mismatch keeps Gateway Readiness false. Replacement takes effect only
after drain and restart.

Every referenced tokenizer artifact is likewise single-read into bounded,
immutable process-owned bytes before readiness. Request handling never reopens a
route, template, or tokenizer pathname. Startup captures the path type, mode,
owner, bytes, and digest before and after each read and fails closed on drift.

Each route contains:

```text
external_model_name
immutable_model_revision_id
model_manifest_hash
tokenizer_identity and exact artifact digest set
chat_template_identity and exact template bytes
context_limit_tokens
default_max_output_tokens
maximum_input_tokens
maximum_decoded_bytes_per_token
allowed_service_classes
route_authorization_class
```

The route manifest is generated deployment configuration, not daemon Control
authority. Its offline builder consumes one exact reviewed Model Manifest
record plus the tokenizer and template artifacts, verifies their declared
identities, and emits the route identity and generation evidence. The gateway
has no Control access and cannot query or mutate the registry. It always submits
the immutable Model Revision ID, never the external name or a mutable Model
Alias. The daemon remains authoritative and may reject a stale, unavailable,
unknown, or incompatible Revision.

A route cannot reinterpret one Model Revision with a different tokenizer or
template. A changed tokenizer, template, context limit, or default creates a
new route identity and requires restart. The exact route identity is exposed in
readiness and response diagnostics.

## 6. External profile v1

### 6.1 Endpoints

The first profile exposes only:

```text
POST /v1/chat/completions
GET  /health/live
GET  /health/ready
```

`/v1/models`, stored completion endpoints, and every other OpenAI-shaped path
return a stable not-supported response. Adding any of them is a separate
profile major with its own name, bounds, and tests.

### 6.2 JSON acceptance

The parser rejects duplicate keys, unknown top-level fields, invalid UTF-8,
non-finite numbers, excessive nesting, trailing non-whitespace bytes, and every
count or byte limit excess. Chunked request bodies are accumulated only through
the fixed body reservation and stop at the maximum before further allocation.
Monotonic TLS-handshake, authentication, header no-progress, header total, body
no-progress, and body total deadlines begin as soon as the exchange slot is
reserved. Expiry closes or returns the bounded timeout response and releases the
slot; authenticated slow readers cannot retain capacity indefinitely.

Messages preserve exact Unicode scalar values and whitespace; the gateway
performs no Unicode normalization. Profile v1 accepts a nonempty bounded array
of messages whose role is `developer`, `system`, `user`, or `assistant` and
whose content is one JSON string. Content-part arrays, names, refusals, tool
calls, images, audio, files, and null content are rejected.

The exact route chat template validates role order and renders the messages.
The exact route tokenizer converts the rendered bytes to input token IDs. The
gateway then checks:

```text
input_tokens <= route.maximum_input_tokens
input_tokens + max_output_tokens <= route.context_limit_tokens
```

Checked arithmetic is mandatory. The daemon repeats its own Model Revision and
context validation; gateway success never bypasses it.

### 6.3 Field mapping

| External field | Profile v1 rule | TurnVector value |
|---|---|---|
| `model` | Required exact route name | Immutable Model Revision ID |
| `messages` | Required bounded text-only messages | Rendered and tokenized input IDs |
| `max_completion_tokens` | Optional positive integer within route bound | Max Output Tokens |
| `max_tokens` | Legacy alias accepted only when `max_completion_tokens` is absent | Max Output Tokens |
| neither max field | Use route's explicit `default_max_output_tokens` | Max Output Tokens |
| `temperature` | Omitted means profile value `1.0`; accepted range `[0, 2]` | Sampling Mode and Temperature |
| `top_p` | Omitted means profile value `1.0`; accepted range `(0, 1]` | Top P |
| `seed` | Omitted stays absent; explicit nonnegative signed-64 value maps exactly, including zero | Optional Sampling Seed |
| `stop` | Omitted, one nonempty string, or bounded nonempty string array | Exact Stop Token Sequences |
| `service_tier` | See mapping below | Required Service Class |
| `stream` | Omitted is false | JSON or SSE response |
| `stream_options.include_usage` | Valid only with `stream=true` | Optional final usage chunk |
| `stream_options.include_obfuscation` | Must be false when present | No obfuscation field |
| `n` | Omitted or exactly `1` | One request and one choice |
| `response_format` | Omitted or exactly `{ "type": "text" }` | Text output |

`max_completion_tokens` and `max_tokens` together are rejected. A zero,
fractional, negative, or out-of-route maximum is rejected; values are never
clamped. Profile v1 has no hidden reasoning tokens, so the accepted value means
TurnVector visible-generation Max Output Tokens.

Sampling mapping is exact:

```text
temperature == +0.0 and top_p == 1.0
    -> GREEDY, Temperature +0.0, Top P 1.0, Top K 0

temperature > 0.0 and top_p > 0.0
    -> CATEGORICAL, exact binary32 Temperature and Top P, Top K 0
```

Negative zero, a zero Top P, or greedy with Top P other than `1.0` is rejected.
The JSON number must round to a finite accepted binary32 value without leaving
the TurnVector Generation Parameters domain.

Service tier mapping is fixed:

| External `service_tier` | TurnVector Service Class | Response value |
|---|---|---|
| omitted, `auto`, `default` | `STANDARD` | `default` |
| `priority` | `INTERACTIVE` | `priority` |
| `flex` | `BACKGROUND` | `flex` |
| `scale` or any other value | rejected | none |

After mapping, the gateway requires the resulting Service Class to be present
in the selected route's `allowed_service_classes` and permitted by the
principal's edge policy. Failure returns 403 `service_tier_not_allowed` before
tokenization or Data Plane submission. The daemon still validates the submitted
Service Class and remains authoritative for Admission and scheduling.

Each stop string is rendered as itself, without template processing, and
tokenized under the same exact tokenizer. Empty strings, an empty token result,
excessive count/bytes/tokens, or an unsupported tokenizer result are rejected.
The daemon's Stop Token Sequence behavior decides which generated tokens become
visible; the gateway does not scan decoded text to implement a second stop
algorithm.

Every other top-level field is rejected as `unsupported_parameter`, including
tools, tool choice, parallel tool calls, log probabilities, logit bias,
presence/frequency penalties, multiple choices, modalities, audio, prediction,
reasoning effort, structured output, storage, metadata, web search, and
provider-side moderation. No field is silently ignored.

## 7. Request-lifetime state machine

One external exchange owns one Data Plane connection. This is intentional: it
aligns external cancellation with immutable Request Ownership and avoids one
local socket failure canceling unrelated external requests. A global gateway
limit bounds simultaneous exchanges and local connections.

```text
RECEIVING
  -> AUTHENTICATED
  -> PARSED
  -> ROUTED
  -> DATA_CONNECTED
  -> TOKENIZED
  -> SUBMITTED
  -> ACCEPTED
  -> STREAMING_OR_BUFFERING
  -> TERMINAL
  -> CLOSED

Any pre-SUBMITTED failure -> bounded HTTP error -> CLOSED
Submit outcome unknown    -> close Data Plane, no replay -> CLOSED
Client disconnect         -> cancel if accepted, close Data Plane -> CLOSED
Data Plane disconnect     -> no replay or reconnect -> CLOSED
```

The order is:

1. Reserve one gateway exchange slot and bounded request bytes.
2. Authenticate the principal without reading or logging request content.
3. Strictly parse and validate the external request.
4. Resolve the immutable route and profile-owned defaults, then authorize the
   principal for that route class.
5. Open one Data Plane connection, authenticate through `LOCAL_PEERCRED`, and
   negotiate one exact supported Data Plane v1 minor.
6. Require daemon Service Readiness and all profile-required capabilities and
   typed limits.
7. Render and tokenize on a bounded CPU executor.
8. Submit one Token Request with `follow_output=true`, the immutable Revision,
   and the next Command ID.
9. Wait for the complete Direct Response before publishing external success
   headers or any completion chunk.
10. On acceptance, bind the returned Request ID and consume the already attached
    ordered Output Frames and terminal Status from the owning Data Plane
    connection.
11. Publish exactly one external terminal outcome, then close the local
    connection.

P05/P13, or an explicit later Data Plane v1 capability extension, must implement
`ACCEPTED_REQUEST_FOLLOW_V1` exactly as section 5.1 defines. Production
integration stops until the named capability, typed limit, descriptor lock, and
old-peer rejection matrix exist. The gateway never issues a later Status
Subscription and guesses that no Output Frame was missed.

The gateway uses strictly increasing local Command IDs and never treats them as
idempotency keys. A Data Plane submit whose Direct Response delivery is unknown
is never replayed. A new external HTTP request is a new request, even if its
body is byte-identical.

A client disconnect before submit writes begin proves that no TurnVector
request exists. Once any submit byte may have reached the daemon, absence of an
acceptance Direct Response is indeterminate: the daemon may have accepted and
attached the request while the gateway has not observed it. The gateway closes
the owning Data Plane connection, which orders disconnect cancellation for any
such request, records only the content-free unknown-delivery class, sends no
external response, and never asserts rejection or retries. Acceptance racing
with that close is covered by the same ownership-cancellation rule.

## 8. Streaming and text reconstruction

The daemon publishes ordered token-ID Output Frames only after accepting a Turn
Receipt. `PromptCodec` uses the exact route detokenizer to convert that sequence
incrementally. It may retain only the bounded suffix needed by the tokenizer's
decode algorithm and incomplete UTF-8 scalar; it emits neither replacement
text nor a partial scalar to make progress.

For `stream=true`, after Request Acceptance the gateway writes:

1. one assistant-role chat completion chunk;
2. zero or more content-delta chunks in Output Sequence order;
3. one terminal chunk with `finish_reason`;
4. when requested, one usage chunk with an empty `choices` array;
5. `data: [DONE]` only after a successful terminal mapping.

Every chunk uses the same completion ID, external model route, creation time,
and system fingerprint. Large decoded segments split only at UTF-8 scalar
boundaries and within `max_sse_event_bytes`; concatenation must reproduce the
exact deterministic decoded output.

For `stream=false`, the same decoder appends into one pre-reserved bounded
response buffer. The bound is derived with checked arithmetic from Max Output
Tokens, the route's maximum decoded bytes per token, and JSON framing. If the
proof cannot fit the configured response maximum, the request is rejected
before Data Plane submission.

Finish reasons are:

| TurnVector terminal fact | External finish reason |
|---|---|
| Natural model or Stop Token Sequence completion | `stop` |
| Max Output Tokens reached | `length` |
| Any failure, cancellation, quarantine, indeterminate delivery, or unsupported terminal | error path, no fabricated finish reason |

`prompt_tokens` is the exact submitted input-token count. `completion_tokens`
comes from the daemon's terminal generated-token progress, not from re-tokenizing
text. It may exceed visible text tokens when TurnVector hides a matched stop
suffix; the profile records that rule in its golden fixtures.

### 8.1 Exact success objects

The profile serializer emits compact UTF-8 JSON with no insignificant
whitespace. Object fields appear in the order shown below; numbers use canonical
base-10 integer rendering, and strings escape only JSON-required characters and
control scalars. Field order is a deterministic fixture rule, not a client
authorization or parsing requirement.

A successful non-streaming response uses `application/json` and the following
exact field presence and order; the schema is expanded here while wire bytes are
compact:

```text
{
  "id": completion_id,
  "object": "chat.completion",
  "created": created_unix_seconds,
  "model": external_route_name,
  "choices": [{
    "index": 0,
    "message": {"role": "assistant", "content": decoded_text},
    "logprobs": null,
    "finish_reason": "stop" | "length"
  }],
  "usage": {
    "prompt_tokens": submitted_input_token_count,
    "completion_tokens": terminal_generated_token_count,
    "total_tokens": checked_sum_of_the_two_counts
  },
  "service_tier": "default" | "priority" | "flex",
  "system_fingerprint": system_fingerprint
}
```

No tool, refusal, annotation, audio, reasoning, or token-detail field appears.
A successful non-streaming response is published only after a successful typed
terminal reason; failure never returns a partial `chat.completion` object.

Successful streaming uses `text/event-stream; charset=utf-8`, disables proxy
buffering and content transformation, and sends no content length. Each payload
is one `data: ` prefix, one compact JSON object, and two line feeds. Every chunk
field order is `id`,
`object="chat.completion.chunk"`, `created`, `model`, `choices`,
`service_tier`, `system_fingerprint`, then optional `usage`. The `choices` value
for each ordinary chunk is:

```text
role chunk:
  "choices": [{"index": 0, "delta": {"role": "assistant"},
               "logprobs": null, "finish_reason": null}]

content chunk:
  "choices": [{"index": 0, "delta": {"content": nonempty_decoded_text},
               "logprobs": null, "finish_reason": null}]

terminal chunk:
  "choices": [{"index": 0, "delta": {},
               "logprobs": null, "finish_reason": "stop" | "length"}]
```

When `include_usage` is false, `usage` is absent from every chunk. When it is
true, each role, content, and terminal chunk ends with `"usage": null`; one
additional chunk then repeats the common identity fields, uses
`"choices": []`, and ends with the same non-null usage object defined above.
Only after that optional usage chunk does the gateway write exactly
`data: [DONE]\n\n`. No content chunk is emitted for an empty decoded segment.

## 9. Response identity

After Request Acceptance:

- completion ID is `chatcmpl-tv-` plus the lowercase Request ID rendering;
- `model` echoes the frozen external route name;
- `created` is gateway wall-clock Unix seconds captured once at acceptance and
  is presentation metadata, not TurnVector ordering authority;
- one choice always has index zero;
- the normalized service tier is returned;
- `system_fingerprint` is `tv-` plus a bounded digest of Gateway Build,
  Compatibility Profile, Revision Route, Model Revision, Data Plane descriptor,
  daemon build, and Generation Semantics identities.

The full exact identities are available in the bounded, content-free headers
`x-turnvector-gateway-build`, `x-turnvector-compatibility-profile`,
`x-turnvector-route`, `x-turnvector-model-revision`,
`x-turnvector-data-plane-descriptor`, `x-turnvector-daemon-instance`, and
`x-turnvector-generation-semantics`. They are also available in gateway
diagnostics. These headers and the short system fingerprint are provenance, not
authorization or replacements for the canonical identities.

## 10. Cancellation, disconnect, and retry

External cancellation and HTTP disconnect are edge events, not Backend calls.
Before submit they close the local session and no TurnVector request exists.
During submit, the outcome is indeterminate and follows the close-without-replay
rule in section 7. After observed Acceptance the gateway sends one cancellation
command when the Data Plane connection is still writable, then closes that
connection. Closing it independently orders TurnVector disconnect cancellation,
so gateway cleanup does not wait indefinitely for a cancellation response.

The following rules are absolute:

- no Data Plane reconnect inherits a Request ID;
- no gateway restart resumes a request;
- no request or output is persisted for replay;
- no failed submit is retried automatically;
- no SSE chunk is retransmitted;
- a slow external reader never blocks the daemon or Device Executor;
- a write deadline closes both external and local sides and preserves no
  detached generation.

Client libraries may explicitly issue a new HTTP request. That creates a new
TurnVector request and must not be described as resume or idempotent retry.

## 11. Error mapping

Once HTTP response framing is available, every pre-SSE error uses
`application/json` and this exact compact shape and field order. A timeout before
HTTP framing exists closes the transport without fabricating an HTTP response.

```text
{"error":{"message":stable_profile_message,"type":stable_type,"param":top_level_parameter_or_null,"code":stable_code}}
```

`param` names only a recognized offending top-level request field and is null
for authentication, policy, daemon, transport, and terminal errors. Messages
come from a bounded profile table and contain no prompt, token, output, local
path, credential, native diagnostic, or daemon-supplied free text. After SSE
headers, if the socket is writable, the gateway emits exactly one `data: ` event
whose payload is the same error object, then closes without `[DONE]`.

| Condition | HTTP or stream outcome | Retry meaning |
|---|---|---|
| Malformed JSON, unsupported field/value, context overflow | 400 `invalid_request_error` | Correct request first |
| Unknown external route | 404 `model_not_found` | Configuration or model name must change |
| Missing/invalid authentication | 401 | Authenticate |
| Authenticated but route-forbidden | 403 | Policy change required |
| Service tier not allowed for the route or principal | 403 `service_tier_not_allowed` | Policy or tier must change |
| TLS deadline before HTTP framing | close transport | New request only |
| Authentication, header, or body receive deadline | 408 `gateway_receive_timeout` | New request only |
| Body exceeds hard byte limit | 413 | Smaller request required |
| Gateway rate or concurrency limit | 429 `gateway_overloaded` | Explicit later request allowed |
| TurnVector Overloaded before Request Acceptance | 429 `turnvector_overloaded` | Explicit later request allowed |
| Frozen Revision unavailable before Acceptance | 503 `model_unavailable` | Route or daemon state must change |
| Daemon non-ready, incompatible protocol, invalid route assets | 503 `gateway_not_ready` | Wait for readiness; gateway does not retry |
| Data Plane delivery unknown before external success headers | 502 `turnvector_delivery_unknown` | Do not assume rejection; gateway does not retry |
| Failure after SSE headers or content | If writable, one bounded SSE error object; always close without `[DONE]` | No resume or replay |
| External disconnect | No response; cancellation and close | No detached request |

The `ACCEPTED_REQUEST_FOLLOW_V1` terminal enum is closed and maps exhaustively:

| Accepted terminal reason | External outcome before SSE | Outcome after SSE starts | Retry meaning |
|---|---|---|---|
| `COMPLETED_NATURAL` or `COMPLETED_STOP` | success with `finish_reason="stop"` | terminal success chunk | none |
| `COMPLETED_MAX_OUTPUT` | success with `finish_reason="length"` | terminal success chunk | none |
| `CANCELED` | 409 `turnvector_request_cancelled` | error event then close | New request only; never resume |
| `PREPARATION_TIMEOUT` | 504 `turnvector_preparation_timeout` | error event then close | New request only |
| `RESIDENCY_WAIT_TIMEOUT` | 504 `turnvector_residency_wait_timeout` | error event then close | New request only |
| `REVISION_UNAVAILABLE` | 503 `model_unavailable` | error event then close | Route or daemon state must change |
| `ADMISSION_OVERLOADED` | 429 `turnvector_overloaded` | error event then close | Explicit later request allowed |
| `ADMISSION_REJECTED` | 503 `turnvector_admission_rejected` | error event then close | Configuration or capacity must change |
| `EXECUTION_FAILED` | 500 `turnvector_execution_failed` | error event then close | New request only; no gateway retry |
| `QUARANTINED` | 500 `turnvector_state_quarantined` | error event then close | Operator action may be required |
| `DAEMON_FAILURE` | 502 `turnvector_session_lost` | error event if writable, then close | Outcome may be incomplete; no gateway retry |

An unknown enum value, success reason inconsistent with generated-token
progress, History Gap in the attached follow stream, missing terminal Status, or
any other impossible combination is 502 `turnvector_protocol_error` before SSE
and the bounded error event after SSE. It closes the Data Plane connection and
never maps to a client input error. Adding a terminal reason requires a new
capability/profile major with exhaustive old/new golden coverage.

The gateway does not translate a daemon protocol violation into a client input
error. It marks its daemon session unusable, fails the external exchange, and
requires a fresh independently negotiated request session for later traffic.

## 12. Bounded resource model

The canonical profile names binary hard maxima for:

- HTTP header count and bytes;
- JSON body bytes and nesting depth;
- message count, per-message bytes, and total text bytes;
- route count and route-manifest bytes;
- template bytes and tokenizer artifact bytes;
- input tokens, output tokens, stop strings, and stop-token count;
- concurrent exchanges and pending tokenization jobs;
- simultaneous Data Plane connections;
- pending output frames and token IDs per exchange;
- detokenizer suffix bytes;
- SSE event bytes and non-streaming response bytes;
- TLS-handshake, authentication, header/body no-progress, header/body total,
  local-connect, negotiate, tokenize, output-idle, no-progress write, total
  write, and drain durations;
- fixed-label metric cardinality and diagnostic record size.

Every formula uses checked integers. Startup verifies route values are within
binary maxima. Each negotiated Data Plane session verifies its effective limits
can carry the exact mapped request and maximum Direct Response before submit.
No queue, cache, map, body, event, or label set grows from unbounded client
input.

Tokenization runs on a bounded CPU executor outside the async socket loop.
Output decoding and JSON/SSE framing are charged to gateway budgets only; they
never enter TurnVector Runtime Overhead or Backend service accounting.

## 13. Security and privacy

The gateway is the network trust boundary. Non-loopback listeners require TLS;
plaintext bearer credentials are never accepted. Deployment selects one fixed
authenticator implementation and a fixed principal-to-route/rate policy at
startup. This design does not create a runtime plugin registry for
authenticators.

The process runs under a dedicated OS identity that appears in the TurnVector
Installation Policy Data allowlist and is absent from Control, live-maintenance,
and offline-maintenance allowlists. Filesystem permissions also deny the
Control socket. Integration tests must attempt Control negotiation under the
real gateway identity and prove rejection.

The gateway never writes request or response content to logs, metrics, Audit,
or crash attachments. It retains text and token buffers only for the live
exchange and does not claim protection from a malicious host administrator,
root, or process-memory inspection. Secret material is excluded from the route
manifest and every identity digest.

Network authentication does not become TurnVector Request Ownership. Inside
the daemon, all requests remain owned by the gateway's live Data Plane
connection; principal identity is carried only as bounded external audit
metadata when policy permits.

## 14. Liveness and readiness

Gateway Liveness means the edge process and its local health path respond.
Gateway Readiness requires all of:

- canonical profile identity verified;
- route manifest and every required asset verified;
- TLS and authenticator initialized;
- gateway limits valid under binary maxima;
- Data Plane socket reachable and authenticated;
- one exact compatible Data Plane v1 descriptor selected;
- `ACCEPTED_REQUEST_FOLLOW_V1` and every other profile-required capability
  effective with nonzero sufficient typed limits;
- latest authenticated daemon status reports Service Readiness;
- no drain or terminal gateway fault active.

Readiness reports Gateway Build, Compatibility Profile, Route Manifest, selected
Data Plane descriptor, Daemon Instance ID, and daemon Service Readiness as
separate fields. It never reports gateway readiness as daemon readiness. One
Unavailable Model Revision affects that route's requests but does not by itself
make the daemon or unrelated routes non-ready.

A readiness monitor may use its own bounded status connection, but it owns no
request. Every request still negotiates and checks its own session; cached
readiness cannot authorize submission.

## 15. Verification

### 15.1 Canonical artifacts

- deterministic double generation of profile and route schemas;
- exact descriptor and identity locks;
- duplicate, unknown, missing, malformed, oversized, and non-canonical cases;
- every default and field mapping represented by golden request bytes;
- every response and error represented by golden JSON/SSE bytes;
- profile-selector attempts rejected and any behavior change requiring a new
  profile major;
- official-shape examples used only as external compatibility inputs.

### 15.2 Module tests through `OpenAiChatGateway.handle`

- every supported field at exact minima/maxima and one past each bound;
- omitted versus explicit zero seed;
- `+0.0` versus `-0.0`, binary32 rounding, and all sampling branches;
- both max-token names, omission default, conflict, zero, and overflow;
- service-tier mapping, route/principal allowed-class matrix, and rejection;
- all unsupported and unknown fields rejected rather than ignored;
- role ordering, Unicode preservation, template identity, and context formula;
- stop strings crossing tokenizer and Output Frame boundaries;
- incremental UTF-8 and tokenizer decode split at every token boundary;
- streaming and non-streaming output byte equivalence;
- usage count with hidden stop suffix;
- exact JSON/SSE discriminators, field order, presence/null rules, usage/error
  envelopes, completion ID, route, tier, fingerprint, and finish reason.

### 15.3 Data Plane state-machine tests

- Hello incompatibility, absent/zero/insufficient
  `ACCEPTED_REQUEST_FOLLOW_V1`, and every other capability/limit failure;
- Request Acceptance, rejection, Overloaded, and Direct Response ordering;
- output and terminal immediately after acceptance with the attached follow
  stream installed at the linearization point and no subscription gap;
- strict Command IDs and no retry after unknown delivery;
- cancellation before submit, submit-write versus Acceptance/disconnect races,
  during output, and at terminal;
- external disconnect, local disconnect, daemon restart, gateway restart;
- partial local frame, partial SSE write, slow reader, and deadline expiry;
- terminal Status with and without a final Output Frame;
- no Request ID inheritance, reconnect, replay, or detached work.

### 15.4 Security and operational tests

- real TLS and authenticator rejection paths;
- stalled TLS, authentication, partial headers, and partial/chunked bodies at
  no-progress and total receive deadlines with slot release;
- gateway UID accepted on Data and rejected on Control;
- route authorization and rate-limit isolation between principals;
- content absence from logs, metrics, and errors;
- exhaustive accepted terminal-reason/error mapping and unknown-reason privacy;
- distinct Preparation Timeout and Residency Wait Timeout golden errors;
- route manifest ownership, mode, symlink, digest, and replacement drift;
- drain/restart with old live requests and a new route generation;
- bounded memory and file-descriptor use under maximum concurrent streams;
- fuzzed JSON, SSE, local frames, tokenizer inputs, and disconnect timing.

### 15.5 Evidence gates

Passing unit or fixture tests permits only the named profile-conformance claim.
It does not certify OpenAI-wide compatibility, model correctness, latency,
throughput, fairness, or multi-tenant security. A serving claim requires real
end-to-end runs with exact Gateway Build, profile, route, daemon build, Model
Revision, tokenizer/template, protocol descriptor, hardware/OS, workload, and
artifact hashes. Benchmark-project changes require a separate explicitly scoped
TurnVectorBenchmark task.

## 16. Implementation order and gates

Pure profile, route, and mapping work can begin before the daemon network phase,
using the in-memory Data Plane adapter. Production integration cannot begin
until the following P0-5 behavior is implemented and green:

- P02 Data Plane peer authentication;
- P05 Data Plane v1.0 descriptor plus the explicit
  `ACCEPTED_REQUEST_FOLLOW_V1` capability extension;
- P07 Data Plane negotiation;
- P09 through P12 ingress, history, and Direct Response capacity;
- P13 request lifecycle commands and effective Seed exposure;
- P14 Turn Output Reservation;
- P15 ordered bounded output publication;
- P16 backpressure, disconnect, and no-replay behavior.

Recommended delivery slices are:

| Slice | Outcome | Gate |
|---|---|---|
| G01 | Canonical compatibility profile, route schema, and golden external bytes | Deterministic identities and negative schema matrix |
| G02 | Deep gateway module with strict request mapping and in-memory Data Plane adapter | All pure mapping/default/identity tests |
| G03 | Exact template, tokenizer, stop, and incremental detokenizer | Cross-boundary and max-size codec tests |
| G04 | Unix Data Plane adapter and one-connection request state machine | P02/P05/P07/P09-P14 plus accept-and-follow integration |
| G05 | JSON/SSE StreamBridge and disconnect cancellation | P15/P16 ordering and slow-reader tests |
| G06 | HTTPS, fixed authenticator, rate limits, health, and launch configuration | Real UID/TLS/Control-denial tests |
| G07 | End-to-end fault, fuzz, resource-bound, and drain suite | All preceding gates green |
| G08 | Qualification adapter and evidence report | Separate Benchmark scope and claim review |

Each slice remains independently green. A production listener stays disabled
until G01 through G07 and all named daemon dependencies pass.

## 17. Upgrade and deployment

One gateway process supports one exact external profile major identity and one
Data Plane major. The external profile has no minors; the gateway may support a
complete explicitly locked non-revoked Data Plane minor matrix within that
major. No process translates between Data Plane majors.

Upgrade order is:

1. deploy a binary that supports the current daemon minor matrix;
2. verify readiness against the exact daemon and route identities;
3. stop external admission on the old gateway;
4. drain or cancel every old request within the fixed drain duration;
5. terminate the old process;
6. start the replacement with its fixed profile and route manifest;
7. enable external traffic only after fresh readiness.

Blue/green gateways may overlap only when each has independent bounded capacity
and the daemon Data Plane limits include their combined maximum. They never
share or transfer Request IDs.

## 18. Resolved decisions and deferred choices

Resolved here:

- separate process, Data Plane only;
- one deep gateway module, not a thin C shim;
- strict, major-only Chat Completions text profile first;
- immutable Revision routes, no alias submission;
- one local connection per external request;
- deterministic profile-owned defaults;
- no transparent retry, replay, resume, or Control proxy;
- bounded JSON, tokenization, buffering, SSE, and writes;
- explicit profile identity and coordinated drain upgrades.

Deployment-local choices that do not alter the module contract:

- bearer versus mutual-TLS authenticator;
- listener address and certificate source;
- principal route/rate assignments;
- concrete limits within binary maxima;
- concrete external route names and exact immutable route entries.

Any choice that changes accepted fields, mapping, defaults, response semantics,
request ownership, or failure behavior changes the Compatibility Profile and
requires the corresponding version and compatibility review.
