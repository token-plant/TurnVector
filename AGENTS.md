# Repository Boundaries

TurnVector and TurnVectorBenchmark are independent repositories. TurnVectorBenchmark is the paired benchmark project used to verify TurnVector changes.

# Required Internal References

Before every TurnVector task, read `.internal/reference/INDEX.md` completely, then read every document it marks `Required: always` completely. Treat these local-only documents as design and evidence context, while revalidating facts that can drift against the current repository, runtime, hardware, and dependency revisions.

If the index or a required document is missing, report the missing local prerequisite before making architecture, runtime, benchmark, or evidence claims. Keep `.internal/` local and outside Git.

For TurnVector work:

- Scope edits, Git operations, dependency updates, and generated files to the TurnVector repository root.
- Treat the TurnVectorBenchmark checkout as read-only unless the task explicitly requests a benchmark project change.
- Benchmark verification may run TurnVectorBenchmark against the current TurnVector checkout; keep benchmark source, baselines, fixtures, dependencies, lockfiles, and Git state unchanged.
- When verification uses both repositories, check each repository's `git status --short` before and after the run. Completion requires TurnVectorBenchmark to have no new changes.
- Keep branches, commits, issues, and pull requests scoped to the repository they change.
