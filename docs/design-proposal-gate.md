# Design Proposal Gate

This gate applies whenever an Agent creates or materially revises a design,
regardless of subject. Examples include architecture, Module boundaries,
Interfaces, algorithms, scheduling, concurrency, persistence, performance,
capacity, benchmarks, evidence, implementation structure, and contributor or
Agent workflows. A material revision changes a decision, invariant, assumption,
bound, risk, or validation obligation. Calling substantive design content a
draft, option, sketch, or proposal does not avoid the gate.

The gate does not apply to a read-only description of an already accepted
design, a status report, or a mechanical execution checklist that introduces
no design decision. When applicability is uncertain, apply the gate.

The rule is prospective from the point at which this document and its trigger
first become governing Agent instructions. The one change that initially
introduces the rule records a bootstrap activation boundary before its first
frozen self-review bundle; earlier communication is not retroactively governed.
After activation, no task, revision, or policy edit may move or redeclare that
boundary.

Before the gate passes, the coordinating Agent may communicate scope questions,
progress, review findings, and blockers without disclosing substantive proposal
content. The Agent may report the design only after every completion condition
below is satisfied.

Bind every substantive report event to the exact Proposal Revision or revisions
whose new or materially revised content it reveals. Content not yet frozen into
a revision is `UNBOUND` and has no passing candidate. A read-only description
containing only already accepted content remains exempt; adding any unaccepted
content binds the event to the revision that introduces or changes that content.

Assign a stable Design Lineage ID before preparing the first proposal revision.
Every revision that retains any substantive decision, invariant, assumption,
bound, or conclusion from that design remains in the same lineage. A digest
change, peripheral edit, renamed proposal, or new round cannot reset lineage
state. A new lineage requires a genuinely independent user request that reuses
none of the prematurely disclosed substantive content; document that boundary.

## Review Bundle

Prepare one self-contained Review Bundle containing:

- the user request and acceptance criteria;
- the stable Design Lineage ID and prior revision disposition;
- the activation boundary when and only when bootstrapping this rule itself;
- the proposed decisions, alternatives considered, and rejected alternatives;
- affected boundaries, Interfaces, invariants, failure behavior, and rollout;
- the complete Mathematical Analysis required below;
- the provenance and evidence grade of every input;
- risks, unknowns, validation work, and claim limitations; and
- the relevant canonical repository documents and current revision identity.

Represent the complete bundle as canonical bytes and use their SHA-256 digest
as the Proposal Revision. A Git commit or tree ID may identify the bundle only
when that object content-addresses every bundle component. Otherwise include a
canonical manifest with the path or identity and SHA-256 digest of every
repository, local, or external component, and include the manifest itself in
the canonical bundle. The user request, Mathematical Analysis, evidence inputs,
and review instructions that affect a verdict are bundle components; none may
be supplied outside the revision binding.

Freeze the bundle before review and keep it unchanged while reviews are in
flight. A change to its decisions, equations, assumptions, inputs, conclusions,
risks, validation plan, or verdict-affecting instructions creates a new Proposal
Revision and invalidates every earlier verdict. Appending a review receipt that
only identifies the frozen revision does not change that revision.

## Mathematical Gate

Every design must include at least one decision-relevant equation, inequality,
logical invariant, set or state cardinality, or complexity relation and derive
a concrete design consequence from it. A qualitative design may use a formal
invariant or state-space proof, but it cannot declare the Mathematical Gate
inapplicable.

The Mathematical Analysis must also turn every material quantitative, scale,
or safety claim into an explicit derivation. It must:

1. Define every variable, unit, domain, and source.
2. Label each input as measured, specified, assumed, or symbolic.
3. Derive the relevant bounds for work or complexity and for each applicable
   resource dimension, such as latency, throughput, capacity, memory, storage,
   state cardinality, coverage, or failure exposure.
4. Show substitutions and checked arithmetic when numerical inputs exist.
5. Check dimensions, extrema, overflow or saturation behavior, and boundary
   conditions.
6. Include conservative or worst-case bounds and sensitivity or break-even
   analysis for assumptions that can change the decision.
7. Map each result to a design decision, threshold, or unresolved validation
   obligation.

Use symbolic inequalities or ranges when trustworthy numbers are unavailable.
State why an inapplicable resource dimension is excluded. Decorative formulas,
invented constants, and point estimates presented without uncertainty do not
pass. A derivation that depends on missing evidence remains theoretical and
must identify the experiment or benchmark needed to validate it.

The Mathematical Gate passes only when the derivation is dimensionally
consistent, reproducible from the stated inputs, covers every material claim,
and supports the stated conclusion without exceeding the evidence policy.

## Unanimous Review Gate

After the Mathematical Gate is complete, assign a fresh Review Round ID that
has never identified another attempt for this proposal. Reserve exactly three
distinct, read-only sub-agent contexts without giving them the Review Bundle or
substantive proposal content. Configure each context explicitly with:

- model: `gpt-5.6-sol`;
- reasoning effort: `max`.

Use explicit model and effort overrides rather than inherited defaults.

After the tool returns the three context IDs, create a canonical Launch Record
containing the Design Lineage ID, Proposal Revision, Review Round ID, complete
context-ID set, and each requested model and reasoning effort. Compute its
SHA-256 before any context receives the Review Bundle. Then send the identical
frozen Review
Bundle, Proposal Revision, Review Round ID, canonical Launch Record, and Launch
Record SHA-256 to exactly those three contexts.

An inherited, default, substituted, or unavailable model configuration does not
satisfy the gate. Run the reviews concurrently when possible. Each recorded
context receives exactly one complete-review invocation for that revision and
round. Reviewers must not edit repository files, delegate any part of the
review, see another reviewer's verdict or a synthesis of peer findings before
returning their own response, or divide the proposal into partial reviews. The
coordinating Agent must not transmit such peer material while the round is in
flight and must preserve the message chronology needed to audit that condition.
All verdict-affecting instructions must already be in the frozen bundle. From
the delivery of a context's complete-review invocation until that context's raw
response returns, the coordinating Agent must send it no follow-up message.
Clarification, coaching, classification guidance, correction, or any other
in-flight instruction is a protocol violation even when it is not labeled a
review invocation.

The three contexts in the Launch Record are the complete set that may receive
that bundle for the round. Sending it to another context, invoking one recorded
context more than once, or soliciting a replacement response under the same
round invalidates the Proposal Revision. Each reviewer verifies the Launch
Record digest, exact three-context membership, own membership, model, and effort
and matching Design Lineage ID before reviewing the proposal. Each context must
remain read-only. The coordinating Agent records repository status and all
bundle-component digests
immediately before distribution and after all responses, and each reviewer
attests that it made no repository edit. These records establish auditable
process evidence; they do not claim to prove unobservable internal behavior
cryptographically.
Each snapshot must enumerate every entry in the Review Bundle's Component
Manifest; a subset, a later reconstruction, or a current-state substitution
does not satisfy the pre-distribution observation.

Every reviewer examines the complete proposal, independently recomputes all
numerical substitutions, and applies every lens in the common checklist:

1. mathematical correctness, units, assumptions, bounds, and sensitivity;
2. architecture, repository authority, invariants, failure behavior, and
   implementability; and
3. adversarial counterexamples, evidence scope, operability, testability, and
   claim discipline.

The lenses are emphases, not divisible assignments. No reviewer may rely on a
peer to cover a section, equation, check, or lens.

Each reviewer returns this structured receipt:

```yaml
proposal_revision: <revision>
design_lineage_id: <stable-lineage>
review_round: <round>
launch_record_sha256: <sha256>
reviewer_id: <distinct-context-id>
model: gpt-5.6-sol
reasoning_effort: max
verdict: PASS | FAIL | BLOCKED
mathematical_check: PASS | FAIL | BLOCKED
architecture_check: PASS | FAIL | BLOCKED
evidence_check: PASS | FAIL | BLOCKED
independence_check: PASS | FAIL | BLOCKED
complete_review_check: PASS | FAIL | BLOCKED
instruction_isolation_check: PASS | FAIL | BLOCKED
read_only_check: PASS | FAIL | BLOCKED
peer_verdicts_observed: false
delegated: false
in_flight_coordinator_messages_observed: 0
repository_edits_made: false
proposal_sections_reviewed: ALL
failure_class: NONE | DESIGN | EVIDENCE | PROTOCOL | INFRASTRUCTURE
blocking_findings: []
non_blocking_findings: []
```

`PASS` requires every check to pass, every attestation to have the exact value
shown, `proposal_sections_reviewed: ALL`, and `blocking_findings` to be empty. A
passing receipt uses `failure_class: NONE`. A missing or malformed
receipt, duplicate reviewer context, timeout, tool failure, wrong model or
effort, lineage, revision, or round mismatch, `FAIL`, or `BLOCKED` prevents that
round from passing. The coordinating Agent's own review cannot replace any of
the three receipts, and receipts from different rounds cannot be combined.

A mathematical, design, architecture, evidence, feasibility, or claim-scope
objection uses `DESIGN` or `EVIDENCE`. A reused Round ID, post-distribution
Launch Record change, record-digest mismatch, wrong requested configuration,
incomplete or extra distribution after distribution begins, unrecorded
recipient, repeated invocation, in-flight coordinator message, peer-verdict
exposure, delegation, partial review, reviewer repository mutation, premature
proposal disclosure, malformed receipt, hidden response, or reclassification
uses `PROTOCOL`. `INFRASTRUCTURE` is limited to an external tool, access,
model-availability, or execution-environment failure that prevents the review
from being completed after the protocol has been followed; it cannot accompany
a substantive blocking finding. The coordinating Agent preserves every raw
response as returned and cannot omit, downgrade, or reclassify it.

For Design Lineage `d`, Proposal Revision `r`, Review Round `j`, and event time
`t`, define `U` as an unestablished check value caused by unavailable evidence:

```text
MathPass(r) in {U, 0, 1} = whether all Mathematical Gate obligations pass for r
LaunchSet(r, j, h) = the context set in the canonical Launch Record with hash h
DistributionSet(r, j) = every context that actually received the bundle
ResponseSet(r, j) = every launched context that returned any raw response
InvocationCount_c(r, j) = complete-review invocations sent to context c
InFlightMessageCount_c(r, j) = coordinator messages sent to c after its bundle
                                delivery and before its raw response
ContentPass_c(r, j, h) in {U, 0, 1} = valid content result with no blocker
Independent_c(r, j, h) in {U, 0, 1} = no delegation or peer-verdict exposure
Complete_c(r, j, h) in {U, 0, 1} = every section, equation, check, and lens read
InstructionIsolated_c(r, j, h) in {U, 0, 1} = no in-flight or outside instruction
ReadOnly_c(r, j, h) in {U, 0, 1} = receipt and snapshots establish read-only work

ReviewPass_c(r, j, h) = 1 iff ContentPass_c(r, j, h) = 1
                              AND Independent_c(r, j, h) = 1
                              AND Complete_c(r, j, h) = 1
                              AND InstructionIsolated_c(r, j, h) = 1
                              AND ReadOnly_c(r, j, h) = 1

RoundPass(r, j, h) = |LaunchSet(r, j, h)| = 3
                     AND DistributionSet(r, j) = LaunchSet(r, j, h)
                     AND ResponseSet(r, j) = LaunchSet(r, j, h)
                     AND every c in LaunchSet(r, j, h)
                         has InvocationCount_c(r, j) = 1
                     AND every c in LaunchSet(r, j, h)
                         has InFlightMessageCount_c(r, j) = 0
                     AND every c in LaunchSet(r, j, h)
                         satisfies ReviewPass_c(r, j, h)

SubstantiveFailure_t(r) = by time t, any response from a context given r
                          contains a substantive blocking finding, even if its
                          receipt is malformed or has mismatched metadata

ProcessViolation_t(r) = by time t, any non-infrastructure breach for r violates
                        reservation, fresh identity, pre-distribution record
                        hashing, exact distribution, single invocation,
                        instruction isolation, configuration, reviewer
                        independence, complete review, read-only operation,
                        receipt conformance, response preservation, or
                        classification; or a pre-launch failure lacks its
                        required Abort Record

GateCandidate_t(d, r) = r is bound to d
                     AND MathPass(r) = 1
                     AND NOT SubstantiveFailure_t(r)
                     AND NOT ProcessViolation_t(r)
                     AND exists fresh j and pre-distribution h completed by t
                         such that RoundPass(r, j, h)

DisclosureTargets(u, d) = exact revisions in d whose new or materially revised
                          substantive content event u reports, or UNBOUND when
                          that content has no frozen revision

GateCandidate_u(d, UNBOUND) = false

PrematureDisclosure_t(d) = by time t, there exists a report event u and target r
                           in DisclosureTargets(u, d) for which
                           GateCandidate_u(d, r) is false; permitted scope
                           questions, progress, review findings, blockers, and
                           accepted-content-only descriptions are excluded

ProtocolViolation_t(d, r) = ProcessViolation_t(r) OR PrematureDisclosure_t(d)

Rejected_t(d, r) = SubstantiveFailure_t(r) OR ProtocolViolation_t(d, r)

Ready_t(d, r) = GateCandidate_t(d, r) AND NOT PrematureDisclosure_t(d)
```

For three reviewers, there are one `MathPass` and five per-reviewer check values,
or `1 + 5 * 3 = 16` values. Including `U`, they admit
`3^16 = 43,046,721` information states. Once all evidence is established, they
admit `2^16 = 65,536` Boolean states. Only the all-`1` state can possibly become
ready; structural and rejection guards can only remove eligibility. With `n`
reviewers the counts are `3^(1 + 5n)` total information states and
`2^(1 + 5n)` fully established Boolean states. An infrastructure-only missing
response leaves checks at `U`, keeps the round non-passing, and need not set a
rejection latch. Therefore missing evidence, a majority, omitted lens, delegated
section, peer-influenced response, coached response, or mutable-workspace review
cannot be accepted as an equivalent state.

Equivalently, one round for the same revision must distribute the bundle to
exactly the three contexts named in a pre-distribution content-addressed Launch
Record and receive one independent, complete, matching digest-bound pass from
every one, with no fourth reviewer and no missing, hidden, malformed, repeated,
peer-influenced, coached, mutating, or substituted response. The proposal must
also remain undisclosed outside the review process until this state is reached,
and premature disclosure remains latched across every substantively overlapping
revision in the same Design Lineage.

This is unanimity within one round, not majority voting, divided review,
reviewer shopping, or accumulation across retries.

## Revision Loop

If any response contains a substantive blocking finding, or any round has a
`PROTOCOL` failure, that Proposal Revision is rejected permanently. Consolidate
the findings or incident into a changed canonical bundle, repeat the
Mathematical Gate, freeze a new Proposal Revision, and submit it to a new round.
Earlier passes never carry forward. A `PASS` receipt may retain non-blocking
findings; those findings are reported as limitations and do not require
revision.

A changed bundle remains in the same Design Lineage when it retains any
substantive content from the design. A premature-disclosure protocol failure
rejects the entire lineage permanently; changing a digest or making a
peripheral edit cannot remediate it. Report that blocker instead of the design.
Only a genuinely independent user request with no reused prematurely disclosed
content may establish a new lineage.

Before any context receives the bundle, an external failure during reservation,
record hashing, tool access, or exact-model acquisition is an
`INFRASTRUCTURE` failure and permits a fresh round against the same unchanged
Proposal Revision. Close that Review Round ID with an immutable Pre-Launch Abort
Record containing the Design Lineage ID, Proposal Revision, Round ID, failure
stage and raw error, any reserved context IDs and requested configurations, and
evidence that the Distribution Set is empty. Include available partial Launch
Record bytes or hashes, but do not require artifacts whose creation was the
failed operation. Never reuse the aborted Round ID. After the first bundle
delivery, failure to complete distribution to exactly the Launch Set is
`PROTOCOL`, regardless of its claimed cause; partial distribution is never an
infrastructure retry.

After complete exact distribution, a round that fails only because an external
reviewer service, tool, access path, exact model, or execution environment is
unavailable, with no substantive finding or protocol breach, is an
`INFRASTRUCTURE` failure. Preserve the round record and every response. The
coordinating Agent may start a new round with a fresh unique Review Round ID
against the same unchanged Proposal Revision; no response carries across
rounds.

Continue while the findings can be resolved within the user's authority and
task scope. When exact reviewers or required evidence are unavailable, report
the gate as blocked with the unmet conditions and unresolved findings. Do not
report the proposal as ready or silently substitute a weaker review process.

## Final Report

A ready design report must include:

- the stable Design Lineage ID, frozen Proposal Revision, and bootstrap
  activation boundary when applicable;
- the decisive equations, inputs, assumptions, bounds, and sensitivity result;
- the passing Review Round ID and the complete three structured receipts,
  including distinct reviewer IDs, exact model and effort, per-check results,
  independence, complete-review, instruction-isolation, and read-only
  attestations, empty blocking findings, and matching lineage, revision, round,
  and Launch Record SHA-256;
- for every launched Review Round ID for that revision, its complete immutable
  canonical Launch Record bytes and pre-distribution SHA-256, distribution
  record, invocation count, message chronology sufficient to show that no peer
  verdict or in-flight coordinator instruction was exposed before each response,
  complete pre/post Component Manifest digests, repository status, disposition,
  and every raw response or missing-response status;
- for every Review Round ID aborted before launch, its immutable Pre-Launch Abort
  Record, empty Distribution Set evidence, available partial launch artifacts,
  and disposition;
- the pre-gate communication record establishing that only permitted scope
  questions, progress, review findings, or blockers were reported before
  readiness, plus the exact revision binding for each substantive report event;
- remaining non-blocking limitations and unresolved empirical validation; and
- the exact claim the analysis and reviews permit.

The review gate validates the internal coherence of a design. It does not turn
theoretical results into measurements, certify runtime behavior, or override
`docs/evidence-policy.md`, repository verification, benchmark gates, or user
approval requirements.
