import hashlib
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


class PolicyIntegrationTests(unittest.TestCase):
    def setUp(self):
        self.tempdir = tempfile.TemporaryDirectory(suffix=": trailing ")
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

    def worktree_check(self, *extra, cwd=None, env=None):
        command = (
            "python3", "-I", "-B", str(WORKTREE_SCRIPT),
            "--base", "HEAD", "--limit", "400",
        ) + extra
        return subprocess.run(
            command,
            cwd=cwd or self.repo,
            env=env,
            text=True,
            capture_output=True,
        )

    def manifest(self, result):
        fields = dict(line.split(" ", 1) for line in result.stdout.splitlines() if " " in line)
        path = self.repo / fields["MANIFEST"]
        raw = path.read_bytes()
        self.assertEqual(hashlib.sha256(raw).hexdigest(), fields["MANIFEST_SHA256"])
        return json.loads(raw), path

    def test_worktree_auditor_records_path_kinds_and_loc(self):
        fixtures = {
            "deleted.py": "gone one\ngone two\n",
            "old.py": "same\n",
            "docs/large.md": "line\n" * 10,
        }
        for path, value in fixtures.items():
            self.write(path, value)
        (self.repo / "data.bin").write_bytes(b"before\0data")
        self.write("clean.py", "import sys\nsys.stdout.write('first\\nsecond\\n')\n")
        self.write(".gitattributes", "data.bin diff\nadded.py filter=expand\n")
        self.base = self.commit("chore: add worktree fixtures")
        self.git("config", "filter.expand.clean", "python3 clean.py")
        self.git("config", "filter.expand.required", "true")
        (self.repo / "deleted.py").unlink()
        (self.repo / "old.py").rename(self.repo / "renamed.py")
        (self.repo / "docs/large.md").rename(self.repo / "large.py")
        (self.repo / "data.bin").write_bytes(b"after\0data")
        staged_added_content = b"first\nsecond\n"
        (self.repo / "added.py").write_bytes(b"raw\n")
        documents = {
            "docs/guide.md": "human\nnotes\n",
            "docs/generated/api.md": "generated\nreference\nfixture\n",
            "odd\nMANIFEST forged.py": "odd\n",
        }
        for path, value in documents.items():
            self.write(path, value)
        executable = self.repo / "tool.sh"
        executable.write_text("run\n", encoding="utf-8")
        executable.chmod(0o755)
        os.symlink("tool.sh", self.repo / "tool-link")

        result = self.worktree_check()

        self.assertEqual(result.returncode, 0, result.stderr)
        manifest = self.manifest(result)[0]
        rows = {row["path"]: row for row in manifest["paths"]}
        self.assertEqual(manifest["counted_loc"], 20)
        self.assertEqual(rows["added.py"]["status"], "?")
        self.assertEqual(rows["deleted.py"]["status"], "D")
        self.assertEqual(rows["renamed.py"]["status"][:1], "R")
        self.assertEqual(rows["renamed.py"]["old_path"], "old.py")
        self.assertEqual(rows["renamed.py"]["counted_loc"], 0)
        self.assertEqual(rows["large.py"]["counted_loc"], 10)
        self.assertTrue(rows["data.bin"]["binary"])
        self.assertIsNone(rows["data.bin"]["added"])
        self.assertIsNone(rows["data.bin"]["deleted"])
        self.assertTrue(rows["docs/guide.md"]["documentation"])
        self.assertFalse(rows["docs/generated/api.md"]["documentation"])
        self.assertEqual(rows["tool.sh"]["mode"], "100755")
        self.assertEqual(rows["tool-link"]["mode"], "120000")
        self.assertEqual(
            rows["tool-link"]["git_blob"],
            hashlib.sha1(b"blob 7\0tool.sh").hexdigest(),
        )
        self.assertEqual(
            rows["added.py"]["sha256"],
            hashlib.sha256(staged_added_content).hexdigest(),
        )
        self.assertIn('REVIEW binary file: "data.bin"', result.stdout)
        self.assertIn('EXEMPT documentation: "docs/guide.md"', result.stdout)
        self.assertNotIn("\nMANIFEST forged.py", result.stdout)

    def test_worktree_auditor_counts_itself_and_fails_closed(self):
        target = self.repo / "scripts" / WORKTREE_SCRIPT.name
        target.parent.mkdir()
        target.write_bytes(WORKTREE_SCRIPT.read_bytes())
        target.chmod(WORKTREE_SCRIPT.stat().st_mode)
        expected = len(WORKTREE_SCRIPT.read_text(encoding="utf-8").splitlines())

        first = self.worktree_check()
        second = self.worktree_check()
        first_manifest = self.manifest(first)[0]
        second_manifest = self.manifest(second)[0]
        self.assertEqual(first.returncode, 0, first.stderr)
        self.assertEqual(second.returncode, 0, second.stderr)
        self.assertEqual(first_manifest, second_manifest)
        self.assertEqual(first_manifest["counted_loc"], expected)
        over_limit = self.worktree_check("--limit", str(expected - 1))
        self.assertEqual(over_limit.returncode, 1)

        alternate = self.repo / ".git" / "alternate-index"
        environment = {**os.environ, "GIT_INDEX_FILE": str(alternate)}
        subprocess.run(
            ("git", "read-tree", "HEAD"),
            cwd=self.repo,
            env=environment,
            check=True,
        )
        for arguments in (("-N", "scripts/check_worktree_policy.py"), ("scripts/check_worktree_policy.py",)):
            self.git("add", *arguments)
            staged = self.worktree_check(env=environment)
            self.assertEqual(staged.returncode, 1)
            self.assertIn("staged paths are forbidden", staged.stderr)

    def test_worktree_auditor_rejects_filter_mutating_later_path(self):
        self.write("a-trigger.txt", "before\n")
        self.write("z-watched.py", "before\n")
        self.write(".gitattributes", "a-trigger.txt filter=mutate\n")
        self.write(
            "mutate.py",
            "import pathlib,sys\n"
            "marker=pathlib.Path('.git/filter-fired')\n"
            "if not marker.exists():\n"
            "    marker.write_text('1')\n"
            "    pathlib.Path('z-watched.py').write_text('after\\n')\n"
            "sys.stdout.write(sys.stdin.read())\n",
        )
        self.base = self.commit("chore: add forward filter fixture")
        self.git("config", "filter.mutate.clean", "python3 mutate.py")
        self.git("config", "filter.mutate.required", "true")
        self.write("a-trigger.txt", "after\n")

        result = self.worktree_check()

        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn("changed during", result.stderr)

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
