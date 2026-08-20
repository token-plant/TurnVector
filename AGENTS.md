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

# Evidence-Sensitive Work

Before changing evidence policy or creating or revising research, benchmark, experiment, model, trace, or product-claim artifacts, read `docs/evidence-policy.md` in addition to the required internal references.

# Design Proposal Gate

Before creating or materially revising a design, read
`docs/design-proposal-gate.md` completely. Before reporting any substantive
design content, the same frozen Review Bundle must pass both its Mathematical
Gate and one three-reviewer Unanimous Review Round. Until then, report only
scope questions, progress, review findings, or blockers without disclosing the
proposal itself.

# Change Delivery

Analysis, research, review, and discussion are read-only unless the user explicitly requests repository changes. Unless the user requests `local-only`, `no commit`, or `no PR`, a task that changes repository files is complete only after a ready pull request has been opened and verified.

1. **Prepare.** Inspect the worktree, fetch `origin/main`, and compare ancestry before editing. Preserve existing work and intentional commits. For a new task, work from current `origin/main` on a branch named `<type>/<kebab-case-topic>`; establish the branch first when the checkout is detached. Continue on the current branch when updating an existing pull request.
2. **Change.** Keep edits within task scope and add or update tests for changed behavior. Update an existing pull request instead of opening a duplicate. After the first push, add follow-up commits rather than rewriting shared history or force-pushing.
3. **Verify.** Run the repository-native formatting, lint, test, and relevant benchmark checks. Inspect the final diff and repository status. Record every required check that could not run. A change-caused failure must be fixed before delivery; an external blocker belongs in a draft pull request and remains incomplete until resolved.
4. **Commit.** Use verified signed commits with subjects matching `type(scope)!: description` or `type: description`. Allowed types are `feat`, `fix`, `perf`, `refactor`, `docs`, `test`, `build`, `ci`, `chore`, and `revert`. Each non-merge pull-request commit must cover one aspect and contain at most 500 non-documentation additions plus deletions. Human-authored documentation is exempt; generated references, fixtures, embedded production source, data, migrations, snapshots, and lockfiles remain counted. True merge commits used only to synchronize the base branch are exempt. An oversized atomic commit requires explicit user approval, the `commit-size-exception` label, and a PR entry naming the commit and reason.
5. **Deliver.** Verify the active GitHub account and target repository, push through the configured remote, and use `gh` to open a pull request targeting `main`. Complete the repository pull-request template with concrete verification, benchmark and evidence impact, limitations, and any policy exception. Use a ready pull request only after available required checks pass; otherwise keep it as a draft.
6. **Confirm.** Verify the remote head SHA, base branch, final diff, review state, and check results. A ready pull request with all reported checks passing, or with no checks configured, is the delivery endpoint.
7. **Merge.** Merge only when the user explicitly requests a merge; general completion language is not merge authorization. Squash merge by default unless another method is required. The resulting base-branch squash commit is exempt from the pull-request commit rules. Verify the merged state and remote head-branch cleanup.
