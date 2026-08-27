# Design Proposal Gate

This gate applies whenever an Agent creates or materially revises a design,
including architecture, Module boundaries, Interfaces, algorithms, scheduling,
concurrency, persistence, performance, capacity, benchmarks, evidence,
implementation structure, or contributor and Agent workflows. A material
revision changes a decision, invariant, assumption, bound, risk, or validation
obligation. Calling substantive content a draft, option, sketch, or proposal
does not avoid the gate.

The gate does not apply to a read-only description of an accepted design, a
status report, or a mechanical implementation checklist that introduces no
new design decision. When applicability is uncertain, apply the gate.

## Gate-Ready Authoring

Design iteration belongs before formal review. The coordinator may discuss
drafts, alternatives, tradeoffs, and substantive design content with the user
while authoring. **Pre-Gate disclosure is allowed** and does not invalidate a
design lineage. Only the final frozen version can become accepted authority.

Before formal review, remove unresolved choices and `TBD` values. Close the
proposed Interface, ownership, paths, invariants, failure behavior, capacity,
Work formulas, validation obligations, and delivery feasibility. Distinguish
specified requirements from examples and values that must be recomputed from
landing bytes. Use a disposable prototype or line-count measurement when it is
needed to establish feasibility.

## Frozen Review Bundle

Prepare one self-contained proposal Review Bundle. Except for the required
internal references allowed below, it must contain everything needed to review
the proposal without consulting the repository, user conversation, or another
source:

- the request, scope, acceptance criteria, and current base commit SHA;
- the complete proposed decisions and rejected alternatives;
- affected Interfaces, ownership, invariants, failure behavior, and rollout;
- the complete Mathematical Analysis required below;
- the provenance and evidence grade of material inputs;
- risks, limitations, validation work, and delivery feasibility; and
- the review instructions and canonical excerpts needed to decide the result.

Freeze the bundle as one canonical artifact and compute one SHA-256 over its
complete bytes. That bundle hash identifies the reviewed content. Do not create
separate component identities, per-file hashes, or a second publication
protocol around it.

The bundle is immutable during a formal round. A Review Bundle hash change
invalidates that round and requires fresh reviewers, but it does not permanently
invalidate the design lineage. Pre-distribution corrections carry no penalty:
correct the bundle, recompute its one hash, and launch a new round.

## Mathematical Gate

Every design includes at least one decision-relevant equation, inequality,
logical invariant, set or state cardinality, or complexity relation and derives
a concrete consequence from it. A qualitative design may use a formal
invariant or state-space proof, but it may not declare the Mathematical Gate
inapplicable.

Turn every material quantitative, scale, or safety claim into an explicit
derivation:

1. Define each variable, unit, domain, and source.
2. Label each input as measured, specified, assumed, or symbolic.
3. Derive work or complexity and every applicable resource bound, including
   latency, throughput, capacity, memory, storage, state cardinality, coverage,
   or failure exposure.
4. Show substitutions and checked arithmetic when numerical inputs exist.
5. Check units, extrema, overflow or saturation, and boundary conditions.
6. Include conservative bounds and sensitivity or break-even analysis for
   assumptions that may change the decision.
7. Map every result to a decision, threshold, or validation obligation.

Use symbolic inequalities or ranges when trustworthy numbers are unavailable.
State why an excluded resource dimension is inapplicable. Decorative formulas,
invented constants, and unsupported point estimates do not pass. A derivation
that depends on missing evidence remains theoretical and names the experiment
or benchmark needed to validate it.

The Mathematical Gate passes only when the derivation is dimensionally
consistent, reproducible from the stated inputs, covers every material claim,
and supports the conclusion within `docs/evidence-policy.md`.

## Launch Preflight

After the Mathematical Gate is complete, reserve exactly three fresh,
independent reviewer contexts. Reviewers may not receive proposal content before
formal launch. Create one Launch Record containing:

- a fresh Review Round ID;
- the base commit SHA and frozen Review Bundle hash;
- the three reviewer context IDs; and
- each requested reviewer model and reasoning effort.

**The Launch Record is the sole invocation identity.** The canonical invocation
JSON is generated directly from the Launch Record, never authored separately,
and is byte-for-byte identical for all three reviewers.

Before distribution, perform only this preflight:

1. every referenced file exists;
2. the base commit SHA is current for the proposed authority;
3. the bundle bytes reproduce the recorded bundle hash; and
4. all three reviewers receive the same generated invocation bytes.

Fix a preflight error before distribution without recording a failed lineage or
round. After distribution, an invocation mismatch invalidates that round. Use
fresh reviewers and a fresh Launch Record; an unchanged design need not be
re-authored.

## Independent Review Round

Reviewer independence begins at formal distribution. Each reviewer receives
the same frozen bundle and generated invocation exactly once. Reviewers read
the frozen Review Bundle plus `.internal/reference/INDEX.md` and every document
it marks `Required: always`. Those required internal references are the only
permitted bundle-external content. Reviewers must not read the user session,
other repository files, another reviewer's output, or an older review result.
They must not delegate the review. The coordinator must not send peer findings,
targeted corrections, or other verdict-shaping instructions while the round is
active. Reading any other bundle-external content invalidates the round.

Every reviewer examines the entire proposal and independently applies all of
these lenses:

1. mathematical correctness, units, assumptions, bounds, and sensitivity;
2. architecture, repository authority, ownership, invariants, failure behavior,
   and implementability; and
3. adversarial counterexamples, evidence scope, operability, testability, and
   claim discipline.

The lenses are emphases, not divisible assignments. No reviewer may rely on a
peer to cover a section, equation, or lens. Each reviewer returns `PASS`,
`FAIL`, or `BLOCKED`, plus blocking and non-blocking findings. A pass identifies
the Review Round ID and bundle hash and confirms complete, independent review.
No elaborate receipt or publication format is required.

### Temporary Verification Work

Temporary verification writes are allowed. Prefer `/tmp` or a disposable clone.
A reviewer may also create probes or ignored output in the candidate worktree
when it is fully removed or restored before the review ends.

Reviewers must not stage, commit, push, or change refs. At completion, the
Review Bundle bytes and hash must equal their launch values, no extra state may
remain, and the candidate worktree must be restored to its exact frozen state.
If the worktree began clean, it ends clean. If it began with an unstaged frozen
candidate, it ends with that exact candidate rather than an empty status.
Temporary writes that meet these conditions are not a failure. Changed reviewed
bytes, residual state, prohibited Git mutation, unauthorized bundle-external
reading, or peer information exposure invalidates the round.

## Decision Rule

For frozen bundle `b`, Launch Record `l`, and its three reviewers `R`, define:

```text
RoundPass(b, l) = MathPass(b)
                  AND BaseMatches(b, l)
                  AND HashMatches(b, l)
                  AND |R| = 3
                  AND SameInvocation(R, l)
                  AND BundleUnchanged(b)
                  AND every r in R has CompleteIndependentPass(r, b, l)
                  AND no IsolationOrResidueViolation(r)
```

The design is accepted only when `RoundPass(b, l)` is true. This is unanimity
within one round, not majority voting, divided review, reviewer shopping, or
receipt accumulation across rounds.

If a round fails, wait for all three reviewers, combine and deduplicate the
valid findings, and return to authoring. Apply the findings as one revision,
rerun the Mathematical Gate, freeze the whole bundle, and launch one new round
with three fresh reviewers. Do not repair one finding at a time while a formal
round is active. A content, identity, isolation, or cleanup failure invalidates
only the affected round; it does not permanently invalidate the design lineage.

## Final Report

An accepted design report records only the information needed to identify and
understand the decision:

- the base commit SHA, frozen Review Bundle hash, and Launch Record;
- the decisive equations, assumptions, bounds, and sensitivity result;
- the three reviewer identities, configurations, verdicts, and findings;
- the unanimous result and any remaining non-blocking limitations; and
- the exact claim that the analysis and reviews support.

The gate validates design coherence. It does not turn theoretical results into
measurements, certify runtime behavior, or override the evidence policy,
repository verification, benchmark gates, or user approval requirements.
