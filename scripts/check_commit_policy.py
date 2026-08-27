#!/usr/bin/env python3
"""Validate TurnVector pull-request branch, title, and commit policy."""

import argparse
import fnmatch
import json
import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Dict, Iterable, List, Optional, Sequence, Set, Tuple

ALLOWED_TYPES = (
    "feat",
    "fix",
    "perf",
    "refactor",
    "docs",
    "test",
    "build",
    "ci",
    "chore",
    "revert",
)
TYPE_PATTERN = "|".join(ALLOWED_TYPES)
CONVENTIONAL_RE = re.compile(
    rf"^(?:{TYPE_PATTERN})(?:\([a-z0-9][a-z0-9._/-]*\))?!?: \S.+$"
)
BRANCH_RE = re.compile(rf"^(?:{TYPE_PATTERN})/[a-z0-9]+(?:-[a-z0-9]+)*$")
DOCUMENTATION_SUFFIXES = {".md", ".mdx", ".rst", ".adoc"}
DOCUMENTATION_NAMES = {"LICENSE", "NOTICE", "README"}
SIZE_LIMIT = 500
SIZE_EXCEPTION_LABEL = "commit-size-exception"
DIFF_OPTIONS = ("--find-renames=50%", "-l0", "--no-ext-diff", "--no-textconv",
                "--diff-algorithm=myers", "--no-indent-heuristic", "--no-color", "--text")
POLICY_MODES = {".commit-policy.json": "100644", ".github/workflows/contribution-policy.yml": "100644",
                "scripts/check_commit_policy.py": "100755", "tests/test_check_commit_policy.py": "100644"}
GIT_ENV = {"PATH": os.environ.get("PATH", "/usr/bin:/bin"), "LC_ALL": "C", "GIT_CONFIG_NOSYSTEM": "1", "GIT_LITERAL_PATHSPECS": "1", "GIT_NO_REPLACE_OBJECTS": "1"}


def git(*args: str, binary: bool = False, env=None) -> object:
    result = subprocess.run(
        ("git",) + args,
        check=True,
        env=GIT_ENV if env is None else env,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=not binary,
    )
    return result.stdout


def parse_config(content: bytes) -> Tuple[str, ...]:
    config = json.loads(content)
    globs = config.get("counted_documentation_globs") if isinstance(config, dict) else None
    if (not isinstance(config, dict) or set(config) != {"counted_documentation_globs"}
            or not isinstance(globs, list) or any(not isinstance(item, str) for item in globs)):
        raise ValueError("policy config must contain only a string-list counted_documentation_globs")
    return tuple(globs)


def is_documentation(path: str, counted_globs: Sequence[str]) -> bool:
    if any(fnmatch.fnmatch(path, pattern) for pattern in counted_globs):
        return False
    name = Path(path).name.upper()
    return Path(path).suffix.lower() in DOCUMENTATION_SUFFIXES or name in DOCUMENTATION_NAMES


def tree_entry(revision: str, path: str) -> Optional[Tuple[str, str]]:
    records = [item for item in git("ls-tree", "-z", revision, "--", path, binary=True).split(b"\0") if item]
    if not records:
        return None
    metadata, raw_path = records[0].split(b"\t", 1)
    mode, kind, blob = metadata.split()
    if len(records) != 1 or raw_path.decode("utf-8", "surrogateescape") != path or kind != b"blob":
        raise ValueError(f"invalid policy tree entry for {path!r}")
    return mode.decode(), blob.decode()


def config_history(base: str, head: str, proposed=None):
    merge_base = str(git("merge-base", base, head)).strip()
    revisions = [merge_base]
    for endpoint in (base, head):
        revisions.extend(commit for commit, _parents in commits_between(merge_base, endpoint) if commit not in revisions)
    globs, history = set(), []
    for revision in revisions:
        entry = tree_entry(revision, ".commit-policy.json")
        if entry is None or entry[0] != POLICY_MODES[".commit-policy.json"]:
            raise ValueError("policy config must be a 100644 Git blob")
        values = parse_config(git("cat-file", "blob", entry[1], binary=True))
        globs.update(values); history.append({"revision": revision, "blob": entry[1], "globs": sorted(values)})
    if proposed is not None:
        blob, content = proposed
        values = parse_config(content)
        globs.update(values); history.append({"revision": "WORKTREE", "blob": blob, "globs": sorted(values)})
    return tuple(sorted(globs)), history


def parse_changes(raw: bytes):
    fields = iter(raw.split(b"\0"))
    for metadata in fields:
        if not metadata:
            return
        old_mode, mode, _old_object, _object_id, status = metadata[1:].decode().split()
        old_path = next(fields).decode("utf-8", "surrogateescape") if status[0] in "RC" else None
        path = next(fields).decode("utf-8", "surrogateescape")
        yield status, path, old_path, None if set(old_mode) == {"0"} else old_mode, \
            None if set(mode) == {"0"} else mode


def policy_transition_errors(changes) -> List[str]:
    errors: List[str] = []
    changed = {value for _status, path, old_path, _old_mode, _mode in changes for value in (old_path, path) if value is not None}
    policy = changed.intersection(POLICY_MODES)
    if policy and changed - set(POLICY_MODES):
        errors.append("policy paths must change alone")
    for _status, path, old_path, _old_mode, mode in changes:
        owners = {value for value in (old_path, path) if value in POLICY_MODES}
        if any(old_path is not None and old_path != path or mode != POLICY_MODES[value]
               for value in owners):
            errors.append(f"policy path has invalid rename, deletion, or mode: {path!r}")
    return errors


def parse_numstat(raw: bytes) -> Iterable[Tuple[str, str, str, Optional[str]]]:
    fields = raw.split(b"\0")
    index = 0
    while index < len(fields) and fields[index]:
        added, deleted, path = fields[index].split(b"\t", 2)
        index += 1
        old_path = None
        if not path:
            old_path = fields[index].decode("utf-8", "surrogateescape")
            path = fields[index + 1]
            index += 2
        yield (
            added.decode("ascii"),
            deleted.decode("ascii"),
            path.decode("utf-8", "surrogateescape"),
            old_path,
        )


def changed_lines(
    parent: str, commit: str, counted_globs: Sequence[str]
) -> Tuple[int, List[str]]:
    raw = git("diff", "--numstat", "-z", *DIFF_OPTIONS, parent, commit, binary=True)
    total = 0
    binaries: List[str] = []
    for added, deleted, path, old_path in parse_numstat(raw):
        old_entry = tree_entry(parent, old_path or path)
        new_entry = tree_entry(commit, path)
        old_content = b"" if old_entry is None else git("cat-file", "blob", old_entry[1], binary=True)
        new_content = b"" if new_entry is None else git("cat-file", "blob", new_entry[1], binary=True)
        old_doc = old_entry is None or is_documentation(old_path or path, counted_globs)
        new_doc = new_entry is None or is_documentation(path, counted_globs)
        if b"\0" in old_content or b"\0" in new_content:
            binaries.append(path)
            continue
        if old_doc and new_doc:
            continue
        if old_doc != new_doc:
            total += text_lines(new_content if old_doc else old_content)
        else:
            total += int(added) + int(deleted)
    return total, binaries


def text_lines(content: bytes) -> int:
    return content.count(b"\n") + bool(content and not content.endswith(b"\n"))


def quote_alternate(path: Path) -> str:
    quoted = (chr(byte) if 32 <= byte < 127 and byte not in (34, 92)
              else f"\\{byte:03o}" for byte in os.fsencode(path))
    return '"' + "".join(quoted) + '"'


def clean_base_merge(base: str, commit: str, parents: Sequence[str]) -> bool:
    if len(parents) != 2 or subprocess.run(
        ("git", "merge-base", "--is-ancestor", parents[1], base), env=GIT_ENV,
        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
    ).returncode:
        return False
    with tempfile.TemporaryDirectory() as temporary:
        objects = Path(temporary) / "objects"
        objects.mkdir()
        common = Path(os.fsdecode(git("rev-parse", "--path-format=absolute", "--git-common-dir", binary=True).removesuffix(b"\n"))) / "objects"
        env = {**GIT_ENV, "GIT_OBJECT_DIRECTORY": str(objects), "GIT_ALTERNATE_OBJECT_DIRECTORIES": quote_alternate(common)}
        try:
            expected = str(git("merge-tree", "--write-tree", *parents, env=env)).splitlines()[0]
        except subprocess.CalledProcessError:
            return False
    return expected == str(git("show", "-s", "--format=%T", commit)).strip()


def pull_request_event(path: Optional[Path]) -> Dict[str, object]:
    if path is None:
        return {}
    with path.open(encoding="utf-8") as handle:
        event = json.load(handle)
    pull_request = event.get("pull_request", {})
    return {
        "base": pull_request.get("base", {}).get("sha"),
        "head": pull_request.get("head", {}).get("sha"),
        "branch": pull_request.get("head", {}).get("ref"),
        "title": pull_request.get("title"),
        "body": pull_request.get("body") or "",
        "labels": {
            label.get("name") for label in pull_request.get("labels", []) if label.get("name")
        },
    }


def commits_between(base: str, head: str) -> List[Tuple[str, List[str]]]:
    merge_base = str(git("merge-base", base, head)).strip()
    rows = str(git("rev-list", "--reverse", "--topo-order", "--parents", f"{merge_base}..{head}"))
    commits = []
    for row in rows.splitlines():
        parts = row.split()
        commits.append((parts[0], parts[1:]))
    return commits


def validate(
    base: str,
    head: str,
    branch: str,
    title: str,
    body: str,
    labels: Set[str],
    counted_globs: Sequence[str],
    enforce_size: bool = True,
) -> List[str]:
    errors: List[str] = []
    oversized: List[Tuple[str, int]] = []

    if not BRANCH_RE.fullmatch(branch):
        errors.append(f"branch '{branch}' must match <type>/<kebab-case-topic>")
    if not CONVENTIONAL_RE.fullmatch(title):
        errors.append(f"PR title is not conventional: {title!r}")

    commits = commits_between(base, head)
    if not commits:
        errors.append("pull request contains no commits")

    for commit, parents in commits:
        if len(parents) > 1:
            if clean_base_merge(base, commit, parents):
                print(f"EXEMPT clean base-sync merge commit {commit[:12]}")
            else:
                errors.append(f"merge commit contains non-base payload: {commit[:12]}")
            continue
        subject = str(git("show", "-s", "--format=%s", commit)).strip()
        if not CONVENTIONAL_RE.fullmatch(subject):
            errors.append(f"commit {commit[:12]} has non-conventional subject: {subject!r}")
        if not parents:
            errors.append(f"commit {commit[:12]} has no parent")
            continue
        raw = git("diff", "--raw", "-z", "--no-abbrev", *DIFF_OPTIONS,
                  parents[0], commit, "--", binary=True)
        changes = list(parse_changes(raw))
        errors.extend(policy_transition_errors(changes))
        line_count, binaries = changed_lines(parents[0], commit, counted_globs)
        print(f"COUNT {commit[:12]} {line_count} non-documentation changed lines")
        for path in binaries:
            print(f"REVIEW binary file in {commit[:12]}: {json.dumps(path, ensure_ascii=True)}")
        if line_count > SIZE_LIMIT:
            oversized.append((commit, line_count))

    if not enforce_size: oversized.clear()
    has_exception = SIZE_EXCEPTION_LABEL in labels
    if oversized and not has_exception:
        for commit, line_count in oversized:
            errors.append(
                f"commit {commit[:12]} changes {line_count} non-documentation lines; "
                f"limit is {SIZE_LIMIT} without '{SIZE_EXCEPTION_LABEL}'"
            )
    elif oversized:
        for commit, line_count in oversized:
            entry = re.search(rf"(?m)^[^\n]*\b{commit}\b[^\n]*:\s*\S.+$", body)
            if "## Policy Exceptions" not in body or entry is None:
                errors.append(
                    "Policy Exceptions must name oversized commit "
                    f"{commit} ({line_count} lines) and give a reason"
                )
            else:
                print(f"EXEMPT oversized commit {commit[:12]} ({line_count} lines)")
    elif has_exception:
        errors.append(f"label '{SIZE_EXCEPTION_LABEL}' is present but no commit exceeds {SIZE_LIMIT} lines")

    return errors


def parse_args(argv: Optional[Sequence[str]] = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base")
    parser.add_argument("--head")
    parser.add_argument("--branch")
    parser.add_argument("--title")
    parser.add_argument("--body", default="")
    parser.add_argument("--label", action="append", default=[])
    parser.add_argument("--event", type=Path)
    return parser.parse_args(argv)


def main(argv: Optional[Sequence[str]] = None) -> int:
    if not sys.flags.isolated:
        print("run this checker with python3 -I -B", file=sys.stderr)
        return 2
    args = parse_args(argv)
    event = pull_request_event(args.event)
    base = args.base or event.get("base")
    head = args.head or event.get("head") or "HEAD"
    branch = args.branch or event.get("branch")
    title = args.title or event.get("title")
    body = args.body or event.get("body") or ""
    labels = set(args.label) | set(event.get("labels", set()))

    missing = [name for name, value in (("base", base), ("branch", branch), ("title", title)) if not value]
    if missing:
        print(f"missing required input: {', '.join(missing)}", file=sys.stderr)
        return 2

    try:
        base = str(git("rev-parse", "--verify", "--end-of-options", f"{base}^{{commit}}")).strip()
        head = str(git("rev-parse", "--verify", "--end-of-options", f"{head}^{{commit}}")).strip()
        source = tree_entry(base, "scripts/check_commit_policy.py")
        if source is None or source[0] != "100755" or Path(__file__).read_bytes() != git(
                "cat-file", "blob", source[1], binary=True):
            raise ValueError("post-commit policy helper is not the explicit accepted base blob")
        globs, _history = config_history(base, head)
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        print(f"Policy check failed: {error}", file=sys.stderr)
        return 1
    errors = validate(
        base,
        head,
        str(branch),
        str(title),
        str(body),
        labels,
        globs,
    )
    if errors:
        print("Policy check failed:", file=sys.stderr)
        for error in errors:
            print(f"- {error}", file=sys.stderr)
        return 1
    print("Policy check passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
