import importlib.util
import json
import os
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts" / "check_commit_policy.py"
WORKTREE_SCRIPT = ROOT / "scripts" / "check_worktree_policy.py"
SPEC = importlib.util.spec_from_file_location("check_commit_policy", SCRIPT)
POLICY = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(POLICY)


class PolicyUnitTests(unittest.TestCase):
    def test_conventional_and_branch_formats(self):
        self.assertIsNotNone(POLICY.CONVENTIONAL_RE.fullmatch("feat(core)!: add contract"))
        self.assertIsNotNone(POLICY.CONVENTIONAL_RE.fullmatch("docs: explain policy"))
        self.assertIsNone(POLICY.CONVENTIONAL_RE.fullmatch("Update policy"))
        self.assertIsNotNone(POLICY.BRANCH_RE.fullmatch("perf/turn-latency"))
        self.assertIsNone(POLICY.BRANCH_RE.fullmatch("codex/turn-latency"))

    def test_documentation_allowlist_and_counted_override(self):
        overrides = ["docs/generated/**"]
        self.assertTrue(POLICY.is_documentation("AGENTS.md", overrides))
        self.assertTrue(POLICY.is_documentation("docs/design.rst", overrides))
        self.assertTrue(POLICY.is_documentation("NOTICE", overrides))
        self.assertFalse(POLICY.is_documentation("docs/generated/api.md", overrides))
        self.assertFalse(POLICY.is_documentation("docs/sample.rs", overrides))

    def test_numstat_parser_preserves_rename_paths(self):
        rows = list(POLICY.parse_numstat(b"0\t1\t\0old.rs\0new.rs\0"))
        self.assertEqual(rows, [("0", "1", "new.rs", "old.rs")])
        self.assertTrue(POLICY.policy_transition_errors([("D", "scripts/check_commit_policy.py", None, "100755", None)]) and POLICY.policy_transition_errors([("R100", "other.py", "scripts/check_commit_policy.py", "100755", "100755")]))

    def test_retired_worktree_auditor_is_not_policy_owned(self):
        path = "scripts/check_worktree_policy.py"
        self.assertNotIn(path, POLICY.POLICY_MODES)
        self.assertEqual(
            POLICY.policy_transition_errors([("D", path, None, "100755", None)]),
            [],
        )


class PolicyIntegrationTests(unittest.TestCase):
    def setUp(self):
        self.tempdir = tempfile.TemporaryDirectory(suffix=": trailing \n")
        self.repo = Path(self.tempdir.name)
        self.git("init", "-b", "main")
        self.git("config", "user.name", "Policy Test")
        self.git("config", "user.email", "policy@example.com")
        self.write("seed.txt", "seed\n")
        self.write(
            ".commit-policy.json",
            json.dumps({"counted_documentation_globs": ["docs/generated/**"]}),
        )
        self.write(".gitignore", ".work/\n__pycache__/\n")
        for source in (SCRIPT, WORKTREE_SCRIPT):
            target = self.repo / "scripts" / source.name
            target.parent.mkdir(exist_ok=True)
            target.write_bytes(source.read_bytes())
            target.chmod(0o755)
        self.git("add", ".")
        self.git("commit", "-m", "chore: initialize fixture")
        self.base = self.git("rev-parse", "HEAD").strip()
        self.git("update-ref", "refs/remotes/origin/main", self.base)

    def tearDown(self):
        self.tempdir.cleanup()

    def git(self, *args):
        result = subprocess.run(
            ("git",) + args,
            cwd=self.repo,
            check=True,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
        return result.stdout

    def write(self, path, content):
        target = self.repo / path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(content, encoding="utf-8")

    def commit(self, subject):
        self.git("add", ".")
        self.git("commit", "-m", subject)
        return self.git("rev-parse", "HEAD").strip()

    def check(self, branch="feat/policy-check", title="feat: enforce policy", *extra, env=None, script=SCRIPT, base=None, head="HEAD"):
        return subprocess.run(
            (
                "python3",
                "-I",
                "-B",
                str(script),
                "--base",
                base or self.base,
                "--head",
                head,
                "--branch",
                branch,
                "--title",
                title,
            )
            + extra,
            cwd=self.repo,
            env=env,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )

    def test_retired_worktree_auditor_uses_current_policy_api(self):
        self.write("candidate.py", "candidate\n")
        for policy_base in ((), ("--policy-base", self.base)):
            result = subprocess.run(
                (
                    "python3", "-I", "-B", str(WORKTREE_SCRIPT), "--base", "HEAD",
                    *policy_base, "--limit", "400",
                ),
                cwd=self.repo,
                text=True,
                capture_output=True,
            )
            self.assertEqual(result.returncode, 0, result.stderr)

    def test_config_history_includes_divergent_base(self):
        self.git("switch", "-c", "stale"); self.write("special/generated.md", "line\n" * 600); stale = self.commit("build: add stale generated output")
        self.git("switch", "main"); self.write(".commit-policy.json", json.dumps({"counted_documentation_globs": ["special/**"]}))
        tightened = self.commit("build: count special documentation")
        self.assertIn("changes 600", self.check(base=tightened, head=stale).stderr)

    def test_valid_commit_and_large_document_pass(self):
        self.write("src/policy.rs", "line\n")
        self.write("docs/policy.md", "doc\n" * 700)
        self.commit("feat: add policy")
        result = self.check()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("1 non-documentation changed lines", result.stdout)

        (self.repo / "docs/policy.md").rename(self.repo / "src/policy-doc.py"); self.commit("build: move documentation into code")
        self.assertIn("changes 700", self.check().stderr)

    def test_large_code_commit_requires_documented_exception(self):
        self.write(":(literal)hidden.py", "line\n" * 501)
        commit = self.commit("feat: add large policy")
        rejected = self.check()
        self.assertEqual(rejected.returncode, 1)
        self.assertIn("changes 501", rejected.stderr)

        accepted = self.check(
            "feat/policy-check",
            "feat: enforce policy",
            "--label",
            "commit-size-exception",
            "--body",
            f"## Policy Exceptions\n{commit}: approved fixture",
        )
        self.assertEqual(accepted.returncode, 0, accepted.stderr)

    def test_invalid_branch_title_and_commit_are_reported(self):
        self.write("src/policy.rs", "line\n")
        self.commit("Update policy")
        result = self.check("codex/policy", "Update policy")
        self.assertEqual(result.returncode, 1)
        self.assertIn("must match", result.stderr)
        self.assertIn("PR title is not conventional", result.stderr)
        self.assertIn("non-conventional subject", result.stderr)

    def test_post_commit_rejects_policy_payload_and_ignores_git_selectors(self):
        self.write(".github/workflows/contribution-policy.yml", "name: relaxed\n"); self.write("src/payload.py", "payload\n"); (self.repo / "binary\nPolicy check passed").write_bytes(b"\0")
        self.commit("build: mix policy and payload"); rejected = self.check(); self.assertIn("policy paths must change alone", rejected.stderr); self.assertEqual(rejected.stdout.splitlines().count("Policy check passed"), 0)
        candidate = self.repo / "scripts/check_commit_policy.py"; candidate.write_text("raise SystemExit(0)\n", encoding="utf-8")
        self.assertEqual(self.check(script=candidate).returncode, 0)
        workflow = (ROOT / ".github/workflows/contribution-policy.yml").read_text(encoding="utf-8"); self.assertTrue('cat-file blob "${base}:scripts/check_commit_policy.py"' in workflow and '--base "$base" --head "$head"' in workflow)
        self.git("reset", "--hard", "HEAD")
        self.git("switch", "-C", "selector-test", self.base)
        self.write("src/valid.py", "valid\n"); head = self.commit("build: add valid fixture")
        event = {"pull_request": {"base": {"sha": self.base}, "head": {"sha": head, "ref":
                 "feat/policy-check"}, "title": "feat: enforce policy", "body": "", "labels": []}}
        event_path = self.repo / "event.json"; event_path.write_text(json.dumps(event), encoding="utf-8")
        self.git("update-ref", "refs/remotes/policy/pr-head", head)
        self.git("switch", "--detach", self.base)
        run = workflow.split("      - name: Validate contribution policy\n", 1)[1].split("        run: |\n", 1)[1]
        run = "\n".join(line[10:] for line in run.splitlines())
        environment = {**os.environ, "RUNNER_TEMP": str(self.repo), "GITHUB_EVENT_PATH": str(event_path),
                       "EVENT_BASE_SHA": "0" * 40, "EVENT_HEAD_SHA": head}
        checked = subprocess.run(("/bin/sh", "-c", run), cwd=self.repo, env=environment)
        self.git("switch", "--detach", head)
        self.assertNotEqual(checked.returncode, 0)
        self.git("config", "diff.renames", "false")
        self.git("replace", self.base, "HEAD")
        marker = self.repo / "hostile-marker"; startup = self.repo / "startup"; startup.mkdir()
        (startup / "sitecustomize.py").write_text(f"open({str(marker)!r},'w').write('bad')\n")
        environment = {**os.environ, "GIT_DIR": str(self.repo / "missing-git-dir"),
                       "GIT_CONFIG_COUNT": "1", "GIT_CONFIG_KEY_0": "diff.renames",
                       "GIT_CONFIG_VALUE_0": "false", "PYTHONPATH": str(startup)}
        sanitized = self.check(env=environment)
        self.assertEqual(sanitized.returncode, 0, sanitized.stderr)
        self.assertFalse(marker.exists())

    def test_post_commit_authenticates_explicit_accepted_base(self):
        candidate = self.repo / "scripts/check_commit_policy.py"
        candidate.write_bytes(candidate.read_bytes() + b"# candidate helper\n")
        updated = self.commit("build: update contribution policy helper")

        accepted = self.check(script=SCRIPT, base=self.base, head=updated)
        self.assertEqual(accepted.returncode, 0, accepted.stderr)
        wrong_source = self.check(script=candidate, base=self.base, head=updated)
        self.assertEqual(wrong_source.returncode, 1)
        self.assertIn("explicit accepted base blob", wrong_source.stderr)

    def test_base_sync_merge_commit_is_exempt(self):
        self.git("switch", "-c", "feat/policy-check")
        self.write("src/policy.rs", "feature\n")
        self.commit("feat: add policy")

        self.git("switch", "main")
        self.write("base.txt", "advance\n")
        self.commit("chore: advance base")
        self.base = self.git("rev-parse", "HEAD").strip()
        self.git("update-ref", "refs/remotes/origin/main", self.base)

        self.git("switch", "feat/policy-check")
        self.git("merge", "main", "--no-ff", "-m", "Merge main")
        result = self.check()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("EXEMPT clean base-sync merge commit", result.stdout)
        self.write("src/merge-payload.py", "payload\n"); self.git("add", "."); self.git("commit", "--amend", "--no-edit")
        self.assertIn("merge commit contains non-base payload", self.check().stderr)


if __name__ == "__main__":
    unittest.main()
