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
        self.assertTrue(POLICY.policy_transition_errors([("D", "scripts/check_commit_policy.py", None, "100755", None)]) and POLICY.policy_transition_errors([("R100", "other.py", "scripts/check_commit_policy.py", "100755", "100755")]))


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
            target = self.repo / "scripts" / source.name; target.parent.mkdir(exist_ok=True)
            target.write_bytes(source.read_bytes()); target.chmod(0o755)
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

    def worktree_check(self, *extra, cwd=None, env=None, script=WORKTREE_SCRIPT, policy_base=True):
        command = ("python3", "-I", "-B", str(script), "--base", "HEAD") + (("--policy-base", self.base) if policy_base else ()) + ("--limit", "400") + extra
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
        target.write_bytes(WORKTREE_SCRIPT.read_bytes() + b"# candidate change\n")
        target.chmod(WORKTREE_SCRIPT.stat().st_mode)
        expected = 1

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
        self.write("candidate.py", "candidate\n")
        subprocess.run(
            ("git", "read-tree", "HEAD"),
            cwd=self.repo,
            env=environment,
            check=True,
        )
        for arguments in (("-N", "candidate.py"), ("candidate.py",)):
            self.git("add", *arguments)
            staged = self.worktree_check(env=environment)
            self.assertEqual(staged.returncode, 1)
            self.assertIn("staged paths are forbidden", staged.stderr)

        self.git("reset")
        work = self.repo / ".work"; work.rename(self.repo / "saved-work")
        os.symlink(self.repo / "saved-work", work)
        self.assertEqual(self.worktree_check().returncode, 1)

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
        self.write("z-watched.py", "before\n"); next_remote = self.git("commit-tree", self.git("rev-parse", "HEAD^{tree}").strip(), "-p", self.base, "-m", "chore: advance remote during scan").strip(); self.git("update-ref", "refs/remotes/origin/main", self.base); self.git("config", "filter.mutate.clean", f"git update-ref refs/remotes/origin/main {next_remote} && cat"); self.write("a-trigger.txt", "remote\n"); drift = self.worktree_check(); self.assertEqual(drift.returncode, 1, drift.stdout + drift.stderr); self.assertIn("changed during", drift.stderr)

    def test_worktree_auditor_binds_config_history_and_policy_scope(self):
        self.write(".commit-policy.json", json.dumps({"counted_documentation_globs": ["special/**"]}))
        self.commit("build: count special documentation")
        self.write(".commit-policy.json", json.dumps({"counted_documentation_globs": ["docs/generated/**"]}))
        self.commit("build: restore documentation policy")
        self.write("special/generated.md", "line\n" * 401)
        self.assertIn("changes 401 counted lines", self.worktree_check().stderr)
        (self.repo / "special/generated.md").unlink()
        self.write("scripts/check_commit_policy.py", "raise SystemExit(0)\n")
        self.assertEqual(self.manifest(self.worktree_check())[0]["policy_helper_sha256"], hashlib.sha256(SCRIPT.read_bytes()).hexdigest())
        self.write("src/payload.py", "payload\n")
        self.assertIn("policy paths must change alone", self.worktree_check().stderr)
        self.git("reset", "--hard", "HEAD")
        self.git("rm", "scripts/check_commit_policy.py")
        self.commit("build: remove accepted helper")
        self.assertIn("accepted policy helper is missing", self.worktree_check().stderr)
        helper = self.repo / "scripts/check_commit_policy.py"
        helper.write_bytes(b"")
        helper.chmod(0o755)
        self.commit("build: add empty accepted helper")
        self.assertIn("empty or has the wrong mode", self.worktree_check().stderr)

    def test_config_history_includes_divergent_base(self):
        self.git("switch", "-c", "stale"); self.write("special/generated.md", "line\n" * 600); stale = self.commit("build: add stale generated output")
        self.git("switch", "main"); self.write(".commit-policy.json", json.dumps({"counted_documentation_globs": ["special/**"]}))
        tightened = self.commit("build: count special documentation")
        self.assertIn("changes 600", self.check(base=tightened, head=stale).stderr)
        self.git("update-ref", "refs/remotes/origin/main", tightened); self.git("switch", "stale"); self.write("special/worktree.md", "line\n" * 401)
        self.assertIn("changes 401 counted lines", self.worktree_check().stderr)

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
        plan = (ROOT / "docs/plans/2026-08-16-p0-runtime-implementation.md").read_text(encoding="utf-8"); block = plan.split("After a commit is signed, its one-commit policy check is:\n\n```sh\n", 1)[1].split("\n```", 1)[0]
        self.assertEqual(plan.split("After signed T02 installation,", 1)[1].split("```sh\n", 1)[1].split("\n   ```", 1)[0].count("refs/remotes/origin/main^{commit}"), 2)
        self.assertNotEqual(checked.returncode, 0)
        self.assertNotEqual(subprocess.run(("/bin/sh", "-c", block), cwd=self.repo).returncode, 0)
        self.assertNotIn("python3 -I -B scripts/check_worktree_policy.py --base HEAD --limit 420", plan)
        self.git("config", "diff.renames", "false")
        self.git("replace", self.base, "HEAD")
        marker = self.repo / "hostile-marker"; startup = self.repo / "startup"; startup.mkdir()
        (startup / "sitecustomize.py").write_text(f"open({str(marker)!r},'w').write('bad')\n")
        environment = {**os.environ, "GIT_DIR": str(self.repo / "missing-git-dir"),
                       "GIT_CONFIG_COUNT": "1", "GIT_CONFIG_KEY_0": "diff.renames",
                       "GIT_CONFIG_VALUE_0": "false", "PYTHONPATH": str(startup)}
        sanitized = self.check(env=environment)
        self.assertEqual(sanitized.returncode, 0, sanitized.stderr)
        self.assertEqual(self.worktree_check(env=environment).returncode, 0)
        self.assertFalse(marker.exists())

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
        block = (ROOT / "docs/plans/2026-08-16-p0-runtime-implementation.md").read_text(encoding="utf-8").split("After a commit is signed, its one-commit policy check is:\n\n```sh\n", 1)[1].split("\n```", 1)[0]; lines = block.splitlines(); verified = next(i for i, line in enumerate(lines) if "verify-commit --raw" in line); lines[verified - 1:verified + 1] = ["true"]; self.assertEqual(subprocess.run(("/bin/sh", "-c", "\n".join(lines)), cwd=self.repo).returncode, 0); self.assertEqual(subprocess.run(("/bin/zsh", "-c", "\n".join(lines)), cwd=self.repo).returncode, 0)
        self.write("src/merge-payload.py", "payload\n"); self.git("add", "."); self.git("commit", "--amend", "--no-edit")
        self.assertIn("merge commit contains non-base payload", self.check().stderr)
        feature = self.git("rev-parse", "HEAD^1").strip(); initial_base = self.git("rev-parse", "HEAD^1^").strip(); self.git("reset", "--hard", feature); self.git("switch", "-c", "unrelated", initial_base); self.write("unrelated.txt", "unrelated\n"); self.commit("chore: advance unrelated branch")
        self.git("switch", "feat/policy-check"); self.git("merge", "unrelated", "--no-ff", "-m", "Merge unrelated"); self.assertNotEqual(subprocess.run(("/bin/sh", "-c", "\n".join(lines)), cwd=self.repo).returncode, 0)

    def test_squashed_policy_installation_blocks_t02_replay(self):
        owners = (".github/workflows/contribution-policy.yml", POLICY.PLAN, "tests/test_check_commit_policy.py")
        for path in owners: self.write(path, "base\n")
        helper = self.repo / "scripts/check_commit_policy.py"; t01_source = subprocess.check_output(("git", "-C", str(ROOT), "cat-file", "blob", POLICY.T01_HELPER_BLOB))
        helper.write_bytes(t01_source); worktree = self.repo / "scripts/check_worktree_policy.py"; worktree.unlink(); self.write(POLICY.PLAN, "pre-t01\n"); self.write("tests/test_check_commit_policy.py", "pre-t01\n"); self.git("add", "-A"); root_tree = self.git("write-tree").strip(); root = self.git("commit-tree", root_tree, "-m", "build: prepare pre-policy root").strip()
        self.write(POLICY.PLAN, "base\n"); self.write("tests/test_check_commit_policy.py", "base\n"); worktree.write_bytes(WORKTREE_SCRIPT.read_bytes()); worktree.chmod(0o755); self.git("add", "."); parent = self.git("commit-tree", self.git("write-tree").strip(), "-p", root, "-m", "build: audit unstaged commit scope").strip(); remote = self.git("commit-tree", root_tree, "-p", root, "-m", "chore: advance pre-policy remote").strip(); self.git("switch", "-C", "bootstrap", parent)
        source = SCRIPT.read_text(encoding="utf-8").replace(POLICY.T01_PARENT, root).replace(POLICY.T01_COMMIT, parent)
        def install(marker):
            helper.write_text(source, encoding="utf-8")
            for path in (*owners, "scripts/check_worktree_policy.py"):
                target = self.repo / path; target.write_bytes(target.read_bytes() + f"# {marker}\n".encode())
        install("installed"); installed = self.commit("build: bind contribution-policy authority")
        helper.write_bytes(t01_source); restored = self.commit("build: restore bootstrap helper")
        self.git("switch", "-c", "replay", parent); install("replay"); replay = self.commit("build: bind contribution-policy authority")
        self.assertEqual(self.check(script=helper, base=parent, head=replay).returncode, 0); self.git("switch", "--detach", installed)
        self.assertTrue("cannot restore bootstrap helper" in self.check(script=helper, base=installed, head=restored).stderr and "reviewed one-time predecessor" in self.check(script=helper, base=installed, head=replay).stderr)
        self.base = installed; self.git("update-ref", "refs/remotes/origin/main", installed); helper.write_bytes(t01_source); self.assertIn("cannot restore bootstrap helper", self.worktree_check(script=self.repo / "scripts/check_worktree_policy.py").stderr); self.git("reset", "--hard", installed); self.git("merge", remote, "--no-ff", "-m", "Merge pre-policy remote"); self.git("update-ref", "refs/remotes/origin/main", remote); continued = self.worktree_check(script=self.repo / "scripts/check_worktree_policy.py", policy_base=False); self.assertEqual(continued.returncode, 0, continued.stderr)


if __name__ == "__main__":
    unittest.main()
