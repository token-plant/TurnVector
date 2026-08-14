# Repository Boundaries

TurnVector and TurnVectorBenchmark are independent repositories. TurnVectorBenchmark is the paired benchmark project used to verify TurnVector changes.

For TurnVector work:

- Scope edits, Git operations, dependency updates, and generated files to the TurnVector repository root.
- Treat the TurnVectorBenchmark checkout as read-only unless the task explicitly requests a benchmark project change.
- Benchmark verification may run TurnVectorBenchmark against the current TurnVector checkout; keep benchmark source, baselines, fixtures, dependencies, lockfiles, and Git state unchanged.
- When verification uses both repositories, check each repository's `git status --short` before and after the run. Completion requires TurnVectorBenchmark to have no new changes.
- Keep branches, commits, issues, and pull requests scoped to the repository they change.
