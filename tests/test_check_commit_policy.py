import importlib.util
import json
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts" / "check_commit_policy.py"
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


class PolicyIntegrationTests(unittest.TestCase):
    def setUp(self):
        self.tempdir = tempfile.TemporaryDirectory()
        self.repo = Path(self.tempdir.name)
        self.git("init", "-b", "main")
        self.git("config", "user.name", "Policy Test")
        self.git("config", "user.email", "policy@example.com")
        self.write("seed.txt", "seed\n")
        self.write(
            ".commit-policy.json",
            json.dumps({"counted_documentation_globs": ["docs/generated/**"]}),
        )
        self.git("add", ".")
        self.git("commit", "-m", "chore: initialize fixture")
        self.base = self.git("rev-parse", "HEAD").strip()

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

    def check(self, branch="feat/policy-check", title="feat: enforce policy", *extra):
        return subprocess.run(
            (
                "python3",
                str(SCRIPT),
                "--base",
                self.base,
                "--head",
                "HEAD",
                "--branch",
                branch,
                "--title",
                title,
                "--config",
                str(self.repo / ".commit-policy.json"),
            )
            + extra,
            cwd=self.repo,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )

    def test_valid_commit_and_large_document_pass(self):
        self.write("src/policy.rs", "line\n")
        self.write("docs/policy.md", "doc\n" * 700)
        self.commit("feat: add policy")
        result = self.check()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("1 non-documentation changed lines", result.stdout)

    def test_large_code_commit_requires_documented_exception(self):
        self.write("src/large.rs", "line\n" * 501)
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

    def test_base_sync_merge_commit_is_exempt(self):
        self.git("switch", "-c", "feat/policy-check")
        self.write("src/policy.rs", "feature\n")
        self.commit("feat: add policy")

        self.git("switch", "main")
        self.write("base.txt", "advance\n")
        self.commit("chore: advance base")
        self.base = self.git("rev-parse", "HEAD").strip()

        self.git("switch", "feat/policy-check")
        self.git("merge", "main", "--no-ff", "-m", "Merge main")
        result = self.check()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("EXEMPT merge commit", result.stdout)


if __name__ == "__main__":
    unittest.main()
