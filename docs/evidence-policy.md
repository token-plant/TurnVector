# Evidence Policy

This policy applies to research inputs, benchmarks, experiments, models, traces, raw results, and product claims derived from technical evidence.

## Evidence Sources

- Treat each report as a historical snapshot of its recorded environment, method, results, limitations, and decision.
- Use the reports marked `Required: always` by the local `.internal/reference/INDEX.md` as the source for current P-1 gate status, architecture evidence, and permitted product scope. If those local prerequisites are unavailable, do not make or change evidence-backed claims.
- Preserve the difference between a supporting control, a partial result, and a passed gate. Product wording must not exceed the strongest completed evidence gate.
- Publish a dated report for a rerun or changed decision instead of silently rewriting historical evidence. Mark factual corrections explicitly and record why the correction was needed.

## Git Boundary

Version product source and compact review artifacts that are necessary to understand or reproduce a decision, such as schemas, manifests, decisions, checksums, and conclusion reports.

Keep durable internal references under ignored `.internal/reference/`. Keep the following outside Git or under ignored `.internal/` or `.work/` paths:

- source PDFs and other research inputs;
- extracted text, page renders, and screenshots;
- model weights and model caches;
- raw JSONL, CSV, benchmark, profiler, and Instruments output;
- traces, toolchains, build products, and bulky generated intermediates.

An implementation request does not authorize changing ignore rules, moving local internal references into tracked paths, or force-adding excluded evidence. Any exception requires explicit user approval before the files enter Git.

## Reproduction

- Reproduce archived experiments in a disposable worktree based on the Git baseline named by the source report.
- Keep the current product worktree intact; archived overlays are inputs to a reproduction, not updates to the product repository.
- Record repository revision, dependency and model revisions, hardware and operating system, input identity, seed, commands or protocol, and integrity checks for durable reruns.
- Keep TurnVectorBenchmark source, baselines, fixtures, dependencies, lockfiles, generated files, and Git state unchanged during TurnVector verification.

## Decision Record

A durable evidence report must state:

- the question and gate being evaluated;
- the fixed environment and inputs;
- the method, thresholds, and exclusions;
- observed results and integrity checks;
- limitations and unresolved failures;
- the resulting status and exactly what product claim it permits.

Evidence work is complete only when the stored report, external artifacts, and stated product scope agree.
