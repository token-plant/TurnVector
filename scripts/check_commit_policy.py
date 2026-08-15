#!/usr/bin/env python3
"""Validate TurnVector pull-request branch, title, and commit policy."""

import argparse
import fnmatch
import json
import re
import subprocess
import sys
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
DOCUMENTATION_NAMES = ("README", "LICENSE", "NOTICE")
SIZE_LIMIT = 500
SIZE_EXCEPTION_LABEL = "commit-size-exception"


def git(*args: str, binary: bool = False) -> object:
    result = subprocess.run(
        ("git",) + args,
        check=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=not binary,
    )
    return result.stdout


def load_config(path: Path) -> Dict[str, object]:
    if not path.exists():
        return {"counted_documentation_globs": []}
    with path.open(encoding="utf-8") as handle:
        return json.load(handle)


def is_documentation(path: str, counted_globs: Sequence[str]) -> bool:
    if any(fnmatch.fnmatch(path, pattern) for pattern in counted_globs):
        return False
    name = Path(path).name.upper()
    return Path(path).suffix.lower() in DOCUMENTATION_SUFFIXES or any(
        name.startswith(prefix) for prefix in DOCUMENTATION_NAMES
    )


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
    raw = git("diff", "--numstat", "-z", "-M", parent, commit, binary=True)
    total = 0
    binaries: List[str] = []
    for added, deleted, path, old_path in parse_numstat(raw):
        paths = [path] if old_path is None else [old_path, path]
        if all(is_documentation(item, counted_globs) for item in paths):
            continue
        if added == "-" or deleted == "-":
            binaries.append(path)
            continue
        total += int(added) + int(deleted)
    return total, binaries


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
            print(f"EXEMPT merge commit {commit[:12]}")
            continue
        subject = str(git("show", "-s", "--format=%s", commit)).strip()
        if not CONVENTIONAL_RE.fullmatch(subject):
            errors.append(f"commit {commit[:12]} has non-conventional subject: {subject!r}")
        if not parents:
            errors.append(f"commit {commit[:12]} has no parent")
            continue
        line_count, binaries = changed_lines(parents[0], commit, counted_globs)
        print(f"COUNT {commit[:12]} {line_count} non-documentation changed lines")
        for path in binaries:
            print(f"REVIEW binary file in {commit[:12]}: {path}")
        if line_count > SIZE_LIMIT:
            oversized.append((commit, line_count))

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
    parser.add_argument("--config", type=Path, default=Path(".commit-policy.json"))
    return parser.parse_args(argv)


def main(argv: Optional[Sequence[str]] = None) -> int:
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

    config = load_config(args.config)
    errors = validate(
        str(base),
        str(head),
        str(branch),
        str(title),
        str(body),
        labels,
        list(config.get("counted_documentation_globs", [])),
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
