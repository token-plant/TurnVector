#!/usr/bin/env -S python3 -I -B
"""Freeze and validate the exact unstaged TurnVector worktree."""

import argparse, base64, fnmatch, hashlib, json, os, stat, subprocess, sys, tempfile
from pathlib import Path

if not sys.flags.isolated:
    print("run this checker with python3 -I -B", file=sys.stderr)
    sys.exit(2)

DIFF_OPTIONS = ("--find-renames=50%", "-l0", "--no-ext-diff", "--no-textconv",
                "--diff-algorithm=myers", "--no-indent-heuristic", "--no-color", "--text")
DOCUMENTATION_NAMES = {"LICENSE", "NOTICE", "README"}
DOCUMENTATION_SUFFIXES = {".adoc", ".md", ".mdx", ".rst"}
GIT_SELECTORS = {"GIT_ALTERNATE_OBJECT_DIRECTORIES", "GIT_ATTR_SOURCE", "GIT_CEILING_DIRECTORIES",
                 "GIT_COMMON_DIR", "GIT_DIR", "GIT_INDEX_FILE", "GIT_NAMESPACE",
                 "GIT_OBJECT_DIRECTORY", "GIT_PREFIX", "GIT_REPLACE_REF_BASE", "GIT_WORK_TREE"}
GIT_ENV = {**{key: value for key, value in os.environ.items() if key not in GIT_SELECTORS
             and not key.startswith("GIT_CONFIG_")}, "GIT_NO_REPLACE_OBJECTS": "1"}
SELF_SHA256 = hashlib.sha256(Path(__file__).read_bytes()).hexdigest()

def git(root: Path, *args: str, env=None) -> bytes:
    return subprocess.run(
        ("git",) + args, cwd=root, check=True, env=GIT_ENV if env is None else env,
        stdout=subprocess.PIPE, stderr=subprocess.PIPE,
    ).stdout

def parse_config(content: bytes):
    config = json.loads(content)
    globs = config.get("counted_documentation_globs") if isinstance(config, dict) else None
    if not isinstance(config, dict) or set(config) != {"counted_documentation_globs"} or not isinstance(globs, list):
        raise ValueError("policy config must contain only counted_documentation_globs")
    if any(not isinstance(item, str) for item in globs):
        raise ValueError("counted_documentation_globs must contain only strings")
    return tuple(globs)

def is_documentation(path: str, globs) -> bool:
    if any(fnmatch.fnmatch(path, pattern) for pattern in globs):
        return False
    name = Path(path).name.upper()
    return Path(path).suffix.lower() in DOCUMENTATION_SUFFIXES or name in DOCUMENTATION_NAMES

def text_lines(content: bytes) -> int:
    return content.count(b"\n") + bool(content and not content.endswith(b"\n"))

def parse_changes(raw: bytes):
    fields = iter(raw.split(b"\0"))
    for metadata in fields:
        if not metadata:
            return
        old_mode, mode, old_object, object_id, status = metadata[1:].decode("ascii").split()
        old_path = next(fields).decode("utf-8", "surrogateescape") if status[0] in "RC" else None
        path = next(fields).decode("utf-8", "surrogateescape")
        values = [None if set(value) == {"0"} else value
                  for value in (old_mode, mode, old_object, object_id)]
        yield status, path, old_path, *values

def tree_state(raw: bytes, index: bool):
    rows = (record.split(b"\t", 1) for record in raw.split(b"\0") if record)
    return {(meta.split()[0], meta.split()[1 if index else 2], path) for meta, path in rows}

def quote_alternate(path):
    quoted = (chr(byte) if 32 <= byte < 127 and byte not in (34, 92)
              else f"\\{byte:03o}" for byte in os.fsencode(path))
    return '"' + "".join(quoted) + '"'

def stat_identity(value):
    return (value.st_mode, value.st_dev, value.st_ino, value.st_size,
            value.st_mtime_ns, value.st_ctime_ns)

def raw_worktree_identity(root: Path):
    raw_paths = git(root, "ls-files", "--cached", "--others", "--exclude-standard", "-z")
    identity = []
    for raw_path in sorted(item for item in raw_paths.split(b"\0") if item):
        path = root / os.fsdecode(raw_path)
        try:
            before = path.lstat()
        except FileNotFoundError:
            identity.append((raw_path, None, None))
            continue
        if stat.S_ISLNK(before.st_mode):
            content = os.fsencode(os.readlink(path))
        elif stat.S_ISREG(before.st_mode):
            with open(path, "rb", opener=lambda name, flags: os.open(name, flags | os.O_NOFOLLOW)) as handle:
                opened = os.fstat(handle.fileno())
                content = handle.read()
                read = os.fstat(handle.fileno())
            if stat_identity(opened) != stat_identity(read):
                raise RuntimeError(f"worktree changed during read of {os.fsdecode(raw_path)!r}")
        else:
            raise ValueError(f"unsupported worktree type for {os.fsdecode(raw_path)!r}")
        after = path.lstat()
        if stat_identity(before) != stat_identity(after):
            raise RuntimeError(f"worktree changed during read of {os.fsdecode(raw_path)!r}")
        if stat.S_ISREG(before.st_mode) and stat_identity(before) != stat_identity(opened):
            raise RuntimeError(f"worktree changed during read of {os.fsdecode(raw_path)!r}")
        identity.append((raw_path, stat_identity(after), content))
    return tuple(identity)

def line_delta(root, before, after, old_content, content, env):
    if before is None:
        return text_lines(content), 0
    if after is None:
        return 0, text_lines(old_content)
    if before[1] == after[1]:
        return 0, 0
    raw = git(root, "diff", "--numstat", *DIFF_OPTIONS, before[1], after[1], env=env)
    added, deleted, _path = raw.split(b"\t", 2)
    return int(added), int(deleted)

def account_change(root, change, globs, env):
    status, path, old_path, old_mode, mode, old_object, object_id = change
    supported = {None, "100644", "100755", "120000"}
    if old_mode not in supported or mode not in supported:
        raise ValueError(f"unsupported Git mode for {path!r}")
    old_content = None if old_object is None else git(root, "cat-file", "blob", old_object, env=env)
    content = None if object_id is None else git(root, "cat-file", "blob", object_id, env=env)
    old_doc = None if old_object is None else is_documentation(old_path or path, globs)
    new_doc = None if object_id is None else is_documentation(path, globs)
    documentation = all(value for value in (old_doc, new_doc) if value is not None)
    binary = any(b"\0" in value for value in (old_content, content) if value is not None)
    before = None if old_object is None else (old_mode, old_object)
    after = None if object_id is None else (mode, object_id)
    added, deleted = (0, 0) if binary else line_delta(
        root, before, after, old_content, content, env,
    )
    counted = 0 if binary or documentation else added + deleted
    if not binary and old_path is not None and old_doc != new_doc:
        counted = (0 if old_doc else text_lines(old_content or b""))
        counted += 0 if new_doc else text_lines(content or b"")
    row = {"added": None if binary else added, "base_blob": old_object, "binary": binary,
           "content_base64": None if content is None else base64.b64encode(content).decode(),
           "counted_loc": counted, "deleted": None if binary else deleted,
           "documentation": documentation, "git_blob": object_id, "mode": mode, "path": path,
           "sha256": None if content is None else hashlib.sha256(content).hexdigest(), "status": status}
    if old_path is not None:
        row["old_path"] = old_path
    return row

def proposed_config(root, env):
    raw = git(root, "ls-files", "-s", "-z", "--", ".commit-policy.json", env=env)
    if not raw:
        raise ValueError("policy config must be a regular file")
    mode, object_id, stage = raw.split(b"\t", 1)[0].split()
    if (mode, stage) != (b"100644", b"0"):
        raise ValueError("policy config must be a regular file")
    return parse_config(git(root, "cat-file", "blob", object_id.decode(), env=env))

def capture(root: Path, base: str):
    raw_untracked = git(root, "ls-files", "--others", "--exclude-standard", "-z")
    untracked = {item.decode("utf-8", "surrogateescape") for item in raw_untracked.split(b"\0") if item}
    with tempfile.TemporaryDirectory() as temporary:
        directory = Path(temporary)
        objects = directory / "objects"
        objects.mkdir()
        common = Path(os.fsdecode(git(
            root, "rev-parse", "--path-format=absolute", "--git-common-dir",
        ).removesuffix(b"\n"))) / "objects"
        env = {**GIT_ENV, "GIT_INDEX_FILE": str(directory / "index"),
               "GIT_OBJECT_DIRECTORY": str(objects),
               "GIT_ALTERNATE_OBJECT_DIRECTORIES": quote_alternate(common)}
        git(root, "read-tree", base, env=env)
        git(root, "add", "-A", "--", ".", env=env)
        globs = proposed_config(root, env)
        raw = git(root, "diff", "--cached", "--raw", "-z", "--no-abbrev",
                  *DIFF_OPTIONS, base, "--", env=env)
        rows = [account_change(root, change, globs, env) for change in parse_changes(raw)]
        for row in rows:
            if row["status"] == "A" and row["path"] in untracked:
                row["status"] = "?"
        clean = subprocess.run(
            ("git", "diff", "--quiet", "--no-ext-diff", "--no-textconv", "--"),
            cwd=root, env=env,
        ).returncode == 0
        final_untracked = git(root, "ls-files", "--others", "--exclude-standard", "-z")
        if not clean or raw_untracked != final_untracked:
            raise RuntimeError("worktree changed during staged snapshot")
    rows.sort(key=lambda row: (row["path"], row["status"]))
    canonical = json.dumps(rows, sort_keys=True, separators=(",", ":")).encode()
    return tuple(sorted(globs)), rows, hashlib.sha256(canonical).hexdigest()

def review_directory(root: Path):
    descriptor = os.open(root, os.O_RDONLY | os.O_DIRECTORY)
    directory = root
    for part in (".work", "reviews", "worktree"):
        try:
            os.mkdir(part, dir_fd=descriptor)
        except FileExistsError:
            pass
        child = os.open(part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=descriptor)
        os.close(descriptor)
        descriptor, directory = child, directory / part
    return directory, descriptor

def atomic_write(descriptor, name, content):
    temporary = f".{name}.{os.getpid()}"
    flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW
    with os.fdopen(os.open(temporary, flags, 0o600, dir_fd=descriptor), "wb") as handle:
        handle.write(content)
    os.replace(temporary, name, src_dir_fd=descriptor, dst_dir_fd=descriptor)

def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base", default="HEAD")
    parser.add_argument("--limit", type=int, required=True)
    args = parser.parse_args(argv)
    try:
        root = Path(os.fsdecode(
            git(Path.cwd(), "rev-parse", "--show-toplevel").removesuffix(b"\n"),
        ))
        os.chdir(root)
        head = git(root, "rev-parse", "HEAD").decode().strip()
        base = head if args.base == "HEAD" else git(
            root, "rev-parse", "--verify", "--end-of-options", f"{args.base}^{{commit}}",
        ).decode().strip()
        index = git(root, "ls-files", "-s", "-z")
        head_tree = git(root, "ls-tree", "-r", "-z", head)
        if tree_state(index, True) != tree_state(head_tree, False):
            raise RuntimeError("staged paths are forbidden during worktree review")
        if git(root, "diff", "--cached", "--name-only", "-z"):
            raise RuntimeError("staged paths are forbidden during worktree review")
        entry = raw_worktree_identity(root)
        first = capture(root, base)
        if entry != raw_worktree_identity(root):
            raise RuntimeError("worktree changed during review scan")
        second = capture(root, base)
        if first != second or entry != raw_worktree_identity(root):
            raise RuntimeError("worktree changed during review scan")
        if head != git(root, "rev-parse", "HEAD").decode().strip() or index != git(root, "ls-files", "-s", "-z"):
            raise RuntimeError("worktree changed during review scan")
        globs, rows, diff_digest = second
        counted = sum(row["counted_loc"] for row in rows)
        payload = {"auditor_source": "t01-bootstrap", "auditor_source_sha256": SELF_SHA256,
                   "base": base, "counted_documentation_globs": globs, "counted_loc": counted,
                   "diff_sha256": diff_digest, "head": head, "limit": args.limit,
                   "paths": rows, "schema": "turnvector-worktree-review-v1"}
        serialized = (json.dumps(payload, sort_keys=True, separators=(",", ":")) + "\n").encode()
        digest = hashlib.sha256(serialized).hexdigest()
        directory, descriptor = review_directory(root)
        relative_manifest = directory.relative_to(root) / f"{digest}.json"
        try:
            ignored = subprocess.run(
                ("git", "check-ignore", "-q", "--", str(relative_manifest)),
                cwd=root, env=GIT_ENV,
            ).returncode == 0
            if not ignored:
                raise ValueError(f"manifest path is not ignored: {relative_manifest}")
            if entry != raw_worktree_identity(root):
                raise RuntimeError("worktree changed during review scan")
            atomic_write(descriptor, relative_manifest.name, serialized)
            if not os.path.samestat(os.stat(directory, follow_symlinks=False), os.fstat(descriptor)):
                raise RuntimeError("review directory changed during manifest write")
        finally:
            os.close(descriptor)
    except (RuntimeError, OSError, ValueError, subprocess.CalledProcessError) as error:
        print(f"worktree policy check failed: {error}", file=sys.stderr)
        return 1
    for row in rows:
        shown = json.dumps(row["path"])
        print(f"PATH {row['status']} {shown} {row['counted_loc']} counted lines")
        if row["binary"]:
            print(f"REVIEW binary file: {shown}")
        elif row["documentation"]:
            print(f"EXEMPT documentation: {shown}")
    print(f"COUNT {counted} non-documentation changed lines (limit {args.limit})")
    print(f"DIFF_SHA256 {diff_digest}")
    print(f"MANIFEST {relative_manifest}")
    print(f"MANIFEST_SHA256 {digest}")
    if counted > args.limit:
        print(f"worktree changes {counted} counted lines; limit is {args.limit}", file=sys.stderr)
        return 1
    return 0

if __name__ == "__main__":
    sys.exit(main())
