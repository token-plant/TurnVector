#!/usr/bin/env -S python3 -I -S -B
import argparse, concurrent.futures, contextlib, hashlib, json, os, shlex, shutil, stat, struct, subprocess, sys, tempfile
from pathlib import Path
if not (sys.flags.isolated and sys.flags.no_site and sys.flags.dont_write_bytecode): raise SystemExit("daemon core build error: generator requires Python -I -S -B\n")
_EXECUTED_SOURCE, _EXECUTED_LAUNCHER_SOURCE = (globals().get(name) for name in ("_EXECUTED_SOURCE", "_EXECUTED_LAUNCHER_SOURCE"))
if not all(isinstance(value, bytes) for value in (_EXECUTED_SOURCE, _EXECUTED_LAUNCHER_SOURCE)): raise SystemExit("daemon core build error: generator requires the captured-source launcher\n")
DESCRIPTOR, LOCK, LAUNCHER = "daemon-core-build-v1.json", "daemon-core-build-v1.lock.json", "scripts/run_daemon_core_build.py"; HASH_DOMAIN = b"turnvector:evidence:daemon-core-build\0"; RUNTIME_INPUTS = ("crates/turnvector-core/src/lib.rs", "crates/turnvector-daemon/src/main.rs"); BOOTSTRAP = 'import os,stat,sys\np=sys.argv[1];sys.argv=sys.argv[1:];h=os.fdopen(os.open(p,os.O_RDONLY|os.O_NOFOLLOW),"rb");a=os.fstat(h.fileno());b=h.read();c=os.fstat(h.fileno());h.close()\ni=lambda x:(x.st_dev,x.st_ino,x.st_mode,x.st_size,x.st_mtime_ns,x.st_ctime_ns)\nif not stat.S_ISREG(a.st_mode) or stat.S_IMODE(a.st_mode)!=0o755 or i(a)!=i(c):raise ValueError("captured launcher changed or has an invalid type or mode")\nexec(compile(b,p,"exec"),{"__name__":"__main__","__file__":p,"_EXECUTED_LAUNCHER_SOURCE":b})'
STATIC_INPUTS = ("Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "crates/turnvector-core/Cargo.toml", "crates/turnvector-core/build.rs", "crates/turnvector-daemon/Cargo.toml", *RUNTIME_INPUTS, "schemas/generation-semantics-v1.json", "schemas/generation-semantics-v1.lock.json", "scripts/generate_generation_semantics.py", "scripts/generate_daemon_core_build.py", LAUNCHER)
ALLOWED_BUILD_MANIFEST, ALLOWED_BUILD_SOURCE = "crates/turnvector-core/Cargo.toml", "crates/turnvector-core/build.rs"
IGNORED = {".git", ".internal", ".work", "target", "__pycache__", ".DS_Store", DESCRIPTOR, LOCK}; FORBIDDEN_KEYS = {"catalog_payload_bytes", "catalog_identity", "outer_daemon_build_identity", "final_binary_sha256"}; EMPTY_SHA = hashlib.sha256(b"").hexdigest()
CONTRACT = json.loads(r'''{"build_variants":{"all_features":false,"default_features":true,"deployment_target":"11.0","generator_execution":{"command":"<bound-python> -I -S -B -c <bootstrap_source> scripts/run_daemon_core_build.py","protocol":"external_single_read_captured_source_v2"},"linker_flags":"-Clinker=<bound-clang> -Clink-arg=-isysroot -Clink-arg=<bound-sdk> -Clink-arg=-t -Clink-arg=-Wl,-sectcreate,__DATA_CONST,__tvcatalog,<fixed-zero-placeholder>","profiles":["dev","release"],"selected_features":[]},"cardinality_inputs":{"encoding":"u16","ingress":{"connections":64,"global_active":1024,"global_warming":256,"per_connection_active":64,"per_connection_warming":16},"model_registry":256},"catalog":{"capacity":{"max_canonical_bytes":4194304,"max_entries":256,"max_entry_bytes":16384},"frame":{"encoding":"fixed_binary_tuple_v1","header_bytes":256,"section_bytes":4194560},"lookup":{"algorithm":"sorted_sha256_key_binary_search_then_canonical_equality_v1","key_bytes":32},"schema_version":1,"worst_case_work":{"entry_bytes_validated":16384,"key_bytes_compared":288,"key_comparisons":9}},"event_registry":{"mandatory_crossing_max":1024,"max_entries":4096,"max_kinds":256,"schema_version":1},"excluded_identity_inputs":["runtime_overhead_catalog_payload_bytes","runtime_overhead_catalog_identity","outer_daemon_build_identity","request_certification_case_bound_tables","lifecycle_overhead_qualification","final_binary_sha256"],"native_inputs":{"adapter_schema_version":1,"files":[],"interface_revision":1},"prepared_carry":{"mandatory_suballocation_max":8192,"nonborrowable":true,"safety_suballocation_max":8192,"slots":1},"support":{"funding_claim":{"max_claims_per_obligation":1024,"nonempty":true,"schema_version":1,"variants":["ordinary_reservation","admission_initial","entitlement_vector","lifecycle_reserve"]},"operations":["describe_model","describe_request","materialize_request","release_request","form_candidates","observe_turn_receipt","sample_backend_resources"],"outstanding_credit_vector":{"axes":["operation","pool","horizon"],"credit_encoding":"u16","max_dimensions":168,"schema_version":1},"pools":["ordinary","mandatory_completion","safety_sampling"],"records":{"active_and_retained":8192,"conditional_obligations":4096,"entitlement_tombstones":2048,"funding_claims":4194304,"lifecycle_reserves":4096,"ordinary_claims":4096,"pending_obligations":4096,"total_operation_obligations":32768},"start_count":{"count_encoding":"u32","max_cells":168,"max_horizons":8,"max_physical_credits":32768,"schema_version":1}}}'''); CONTRACT["build_variants"]["generator_execution"].update({"bootstrap_source": BOOTSTRAP, "bootstrap_sha256": hashlib.sha256(BOOTSTRAP.encode()).hexdigest()})
CATALOG_SECTION = {"byte_length": CONTRACT["catalog"]["frame"]["section_bytes"], "encoding": CONTRACT["catalog"]["frame"]["encoding"], "executable": False, "format": "mach_o_64", "header_bytes": CONTRACT["catalog"]["frame"]["header_bytes"], "padding_byte": 0, "section": "__tvcatalog", "segment": "__DATA_CONST", "writable": False}
def canonical(value) -> bytes: return (json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n").encode()
def sha256(payload: bytes) -> str: return hashlib.sha256(payload).hexdigest()
def unique(pairs):
    result = dict(pairs)
    if len(result) != len(pairs): raise ValueError("duplicate key")
    return result
def read_json(payload: bytes): return json.loads(payload, object_pairs_hook=unique)
def run(command, cwd: Path, env: dict, error_output=False) -> str:
    result = subprocess.run([str(item) for item in command], cwd=cwd, env=env, check=False, capture_output=True, text=True)
    if result.returncode: raise ValueError(result.stderr.strip() or result.stdout.strip() or f"command failed: {command[0]}")
    return result.stderr if error_output else result.stdout
def tree_identity(root: Path) -> list:
    records = []
    def visit(directory, prefix):
        for entry in sorted(os.scandir(directory), key=lambda item: os.fsencode(item.name)):
            relative = f"{prefix}/{entry.name}" if prefix else entry.name
            if entry.name in IGNORED: continue
            if entry.is_symlink(): records.append([relative, "link", os.readlink(entry.path)])
            elif entry.is_dir(follow_symlinks=False): visit(entry.path, relative)
            elif entry.is_file(follow_symlinks=False):
                mode = stat.S_IMODE(entry.stat(follow_symlinks=False).st_mode); records.append([relative, f"file:{mode:o}", sha256(Path(entry.path).read_bytes())])
            else: raise ValueError(f"unsupported source-tree entry: {relative}")
    visit(root, ""); return records
def source_names(root: Path, graph: dict, traced: list) -> list: return sorted({*STATIC_INPUTS, *traced, *(item["manifest"] for item in graph["packages"])})
def source_records(identity: list, names: list) -> list:
    frozen = {item[0]: item[1:] for item in identity}; records = []
    for name in names:
        kind, digest = frozen.get(name, (None, None))
        if not kind or not kind.startswith("file:"): raise ValueError(f"build input is not a regular file: {name}")
        records.append({"path": name, "mode": "100755" if int(kind[5:], 8) & 0o111 else "100644", "sha256": digest})
    return records
def frozen(snapshot: Path, identity: list, stage: str) -> None:
    if tree_identity(snapshot) != identity: raise ValueError(f"source snapshot changed {stage}")
def rust_tool(name: str, value: str, rustup, root: Path) -> Path:
    invocation, target = Path(os.path.abspath(value)), Path(value).resolve(); manager = Path(os.path.abspath(rustup)).resolve() if rustup is not None else None
    if target.name != "rustup" and (manager is None or not os.path.samefile(invocation, manager)): return invocation
    env = {"PATH": "/usr/bin:/bin", "LANG": "C", "LC_ALL": "C", **{key: os.environ[key] for key in ("HOME", "RUSTUP_HOME") if key in os.environ}}
    selected = Path(run([manager or target, "which", name], root, env).strip()).resolve()
    if not selected.is_file() or not os.access(selected, os.X_OK): raise ValueError(f"rustup did not select an executable {name}")
    return selected
def resolve_tools(root: Path) -> dict:
    values = {"python": sys.executable, "cargo": shutil.which("cargo"), "rustc": shutil.which("rustc")}
    if any(value is None for value in values.values()): raise ValueError("required build tool not found")
    rustup = shutil.which("rustup"); paths = {"python": Path(values["python"]).resolve(), **{name: rust_tool(name, values[name], rustup, root) for name in ("cargo", "rustc")}}; paths["xcrun"] = Path("/usr/bin/xcrun")
    for name in ("clang", "ld"):
        paths[name] = Path(subprocess.run([str(paths["xcrun"]), "--find", name], check=True, capture_output=True, text=True).stdout.rstrip("\n")).resolve()
    paths["developer"] = paths["clang"].parents[4]; selector_env = {"PATH": "/usr/bin:/bin", "DEVELOPER_DIR": str(paths["developer"])}
    paths["sdk"] = Path(subprocess.run([str(paths["xcrun"]), "--sdk", "macosx", "--show-sdk-path"], env=selector_env, check=True, capture_output=True, text=True).stdout.rstrip("\n")).resolve()
    paths["clang_resources"] = Path(subprocess.run([str(paths["clang"]), "--print-resource-dir"], env=selector_env, check=True, capture_output=True, text=True).stdout.rstrip("\n")).resolve()
    paths.update({"xcode_lib": paths["clang"].parents[1] / "lib", "xcode": paths["developer"].parent, "python_runtime": paths["python"].parents[1], "system_build": Path("/System/Library/CoreServices/SystemVersion.plist")})
    for name in ("sdk", "clang_resources", "xcode_lib", "xcode", "python_runtime", "system_build"):
        if not paths[name].exists(): raise ValueError(f"required toolchain closure is missing: {name}")
    return paths
def artifact(path: Path, label: str) -> dict:
    if path.is_symlink() or not path.is_file(): raise ValueError(f"toolchain artifact is not a regular file: {label}")
    return {"path": label, "sha256": sha256(path.read_bytes())}
def tree_artifact(root: Path, label: str, paths=None) -> dict:
    candidates = paths if paths is not None else root.rglob("*"); rows, total = [], 0
    for path in sorted(set(candidates), key=lambda item: os.fsencode(item.relative_to(root).as_posix())):
        if not path.is_file(): continue
        relative, payload = path.relative_to(root).as_posix(), path.read_bytes(); total += len(payload); rows.append([relative, f"{stat.S_IMODE(path.stat().st_mode):o}", sha256(payload)])
    if not rows: raise ValueError(f"toolchain closure is empty: {label}")
    return {"algorithm": "canonical_path_mode_sha256_records_v1", "path": label, "file_count": len(rows), "byte_length": total, "sha256": sha256(canonical(rows)), "files": rows}
def sdk_link_inputs(trace: str, sdk: Path, required=True) -> list:
    result = set()
    for path in (Path(line.strip()) for line in trace.splitlines()):
        parent = next((item for item in path.parents if item.resolve() == sdk), None) if path.is_absolute() and path.is_file() and path.resolve().is_relative_to(sdk) else None
        if parent is not None: result.add(sdk / path.relative_to(parent))
    if required and not result: raise ValueError("linker did not report any SDK inputs")
    return sorted(result)
def freeze_sdk(sdk: Path, linked: list, destination: Path) -> list:
    for source in {*linked, *sdk.glob("*.plist"), *sdk.glob("*.json")}:
        target = destination / source.relative_to(sdk); target.parent.mkdir(parents=True, exist_ok=True); shutil.copy2(source, target)
    frozen = [destination / source.relative_to(sdk) for source in linked]
    if tree_artifact(sdk, "sdk-link-inputs", linked) != tree_artifact(destination, "sdk-link-inputs", frozen): raise ValueError("frozen SDK linker inputs do not match discovery")
    return frozen
def rust_sysroot(paths: dict, root: Path) -> Path: return Path(run([paths["rustc"], "--print", "sysroot"], root, {"PATH": "/usr/bin:/bin", "LANG": "C", "LC_ALL": "C"}).strip()).resolve()
def toolchain(paths: dict, root: Path) -> dict:
    sysroot, command_root = rust_sysroot(paths, root), Path("/"); env = {"PATH": "/usr/bin:/bin", "LANG": "C", "LC_ALL": "C", "RUSTUP_TOOLCHAIN": str(sysroot)}
    target = Path(run([paths["rustc"], "--print", "target-libdir"], command_root, env).strip()).resolve()
    files = sorted([*sysroot.glob("lib/librustc_driver*.dylib"), *(path for path in target.rglob("*") if path.is_file())])
    def identity(name, args): return {"version": run([paths[name], *args], command_root, env).strip(), "binary_sha256": sha256(paths[name].read_bytes())}
    rustc = identity("rustc", ["-vV"]); rustc["host"] = next(line.split(": ", 1)[1] for line in rustc["version"].splitlines() if line.startswith("host: ")); rustc["driver_and_target_files"] = [artifact(path, path.relative_to(sysroot).as_posix()) for path in files]
    python = identity("python", ["--version"]); python_files = [path for path in paths["python_runtime"].rglob("*") if path.is_file() and "__pycache__" not in path.parts and "site-packages" not in path.parts and path.suffix != ".pyc"]
    python.update({"flags": {"dont_write_bytecode": True, "isolated": True, "no_site": True}, "runtime_files": tree_artifact(paths["python_runtime"], "python-runtime", python_files)})
    native = {name: identity(name, ["--version"] if name == "clang" else ["-version_details"]) for name in ("clang", "ld")}
    sdk_files = [*paths["sdk"].glob("*.plist"), *paths["sdk"].glob("*.json")]; libraries = [paths["xcode_lib"] / name for name in ("libLTO.dylib", "libtapi.dylib", "libcodedirectory.dylib", "libswiftDemangle.dylib")]; native.update({"deployment_target": "11.0", "sdk": tree_artifact(paths["sdk"], "macos-sdk", sdk_files), "clang_resources": tree_artifact(paths["clang_resources"], "clang-resources"), "linker_libraries": tree_artifact(paths["xcode_lib"], "xcode-linker-libraries", libraries), "xcode": tree_artifact(paths["xcode"], "xcode", paths["xcode"].glob("*.plist")), "system_build": artifact(paths["system_build"], "system-build")})
    return {"python": python, "cargo": identity("cargo", ["--version"]), "rustc": rustc, "native_link": native, "selector": artifact(paths["xcrun"], "xcrun")}
def dependency_graph(root: Path, metadata: dict) -> tuple:
    workspace, by_id, nodes = set(metadata["workspace_members"]), {item["id"]: item for item in metadata["packages"]}, {item["id"]: item for item in metadata["resolve"]["nodes"]}
    candidates = [item["id"] for item in metadata["packages"] if item["id"] in workspace and item["name"] == "turnvector-daemon"]
    if len(candidates) != 1: raise ValueError("runtime graph must contain exactly one turnvector-daemon package")
    runtime, pending = set(), candidates[:]
    while pending:
        package_id = pending.pop()
        if package_id in runtime: continue
        runtime.add(package_id)
        for dependency in nodes[package_id]["deps"]:
            kinds = {item["kind"] for item in dependency["dep_kinds"]}
            if "build" in kinds: raise ValueError("runtime packages may not use build dependencies")
            if None in kinds: pending.append(dependency["pkg"])
    identities, packages, roots, custom_builds = {}, [], [], []
    for package_id in runtime:
        package, manifest = by_id[package_id], Path(by_id[package_id]["manifest_path"]).resolve()
        if not manifest.is_relative_to(root): raise ValueError(f"undeclared local dependency outside repository: {manifest}")
        relative = manifest.relative_to(root).as_posix()
        if package["source"] is not None or (package_id not in workspace and not relative.startswith("vendor/")): raise ValueError("only repository vendored path dependencies are allowed")
        kinds = {kind for target in package["targets"] for kind in target["kind"]}
        if "proc-macro" in kinds: raise ValueError("runtime packages may not use proc-macro targets")
        for target in package["targets"]:
            if "custom-build" not in target["kind"]: continue
            source = Path(target["src_path"]).resolve()
            if relative != ALLOWED_BUILD_MANIFEST or source != (root / ALLOWED_BUILD_SOURCE).resolve(): raise ValueError(f"runtime build scripts may contain only {ALLOWED_BUILD_SOURCE} as the sole custom build")
            custom_builds.append((relative, source))
        roots.append(manifest.parent); identities[package_id] = f'{package["name"]}@{package["version"]}|{relative}'
    if len(custom_builds) != 1: raise ValueError(f"runtime graph must contain exactly one {ALLOWED_BUILD_SOURCE} build script")
    for package_id in runtime:
        package, node, root_package = by_id[package_id], nodes[package_id], package_id == candidates[0]
        targets = [item for item in package["targets"] if (root_package and item["name"] == "turnvector-daemon" and "bin" in item["kind"]) or (not root_package and "lib" in item["kind"])]
        if not targets: raise ValueError(f"runtime package has no selected target: {package['name']}")
        edges = [{"name": item["name"], "package": identities[item["pkg"]], "targets": sorted({kind["target"] for kind in item["dep_kinds"] if kind["kind"] is None}, key=lambda value: value or "")} for item in node["deps"] if any(kind["kind"] is None for kind in item["dep_kinds"])]
        relative = Path(package["manifest_path"]).resolve().relative_to(root).as_posix(); packages.append({"identity": identities[package_id], "manifest": relative, "dependencies": sorted(edges, key=canonical), "selected_features": sorted(node["features"]), "targets": sorted(({"name": item["name"], "kind": sorted(item["kind"]), "crate_types": sorted(item["crate_types"]), "edition": item["edition"]} for item in targets), key=canonical)})
    resolved = [{"package": identities[item["id"]], "features": sorted(item["features"]), "dependencies": sorted(identities[dependency["pkg"]] for dependency in item["deps"] if any(kind["kind"] is None for kind in dependency["dep_kinds"]))} for item in nodes.values() if item["id"] in runtime]
    return {"packages": sorted(packages, key=lambda item: item["identity"]), "resolved": sorted(resolved, key=lambda item: item["package"])}, roots
def traced_inputs(root: Path, target: Path, package_roots: list) -> tuple:
    observed, environment = set(), set()
    for depinfo in target.rglob("*.d"):
        for line in depinfo.read_text(errors="strict").replace("\\\n", "").splitlines():
            if line.startswith("# env-dep:"):
                value = line[len("# env-dep:"):]
                if not value.split("=", 1)[0].startswith("CARGO_PKG_"): raise ValueError(f"undeclared environment input: {value.split('=', 1)[0]}")
                environment.add(value.replace(str(root), "<repository>"))
            elif ":" in line and not line.lstrip().startswith("#"):
                for item in shlex.split(line.split(":", 1)[1]):
                    path = Path(item) if Path(item).is_absolute() else root / item
                    if not path.is_file(): continue
                    resolved = path.resolve()
                    if not resolved.is_relative_to(root): raise ValueError(f"undeclared runtime input: {resolved}")
                    relative = resolved.relative_to(root).as_posix()
                    if relative not in STATIC_INPUTS and not any(parent == resolved or parent in resolved.parents for parent in package_roots): raise ValueError(f"undeclared runtime input: {relative}")
                    observed.add(relative)
    missing = sorted(set(RUNTIME_INPUTS) - observed)
    if missing: raise ValueError(f"declared runtime input was not dependency-traced: {missing[0]}")
    return sorted(observed), sorted(environment)
def generation_registry(root: Path, env: dict) -> dict:
    try:
        run([sys.executable, "-I", "-S", "-B", root / "scripts/generate_generation_semantics.py", "--check", root / "schemas"], root, env)
    except ValueError as error: raise ValueError(f"current Generation Semantics verification failed: {error}")
    descriptor = (root / "schemas/generation-semantics-v1.json").read_bytes(); lock = read_json((root / "schemas/generation-semantics-v1.lock.json").read_bytes()); return {"kind": "generation_semantics", "identity": lock["digest"], "descriptor_sha256": sha256(descriptor)}
def leb128(payload: bytes, index: int, signed=False) -> tuple:
    value, shift = 0, 0
    while index < len(payload) and shift < 64:
        byte = payload[index]; index += 1; value |= (byte & 0x7F) << shift; shift += 7
        if not byte & 0x80:
            if shift > 64 and ((not signed and byte & 0x7E) or (signed and byte not in (0, 0x7F))): raise ValueError("overflowed Mach-O dyld LEB128 operand")
            if signed and byte & 0x40: value -= 1 << shift
            return value, index
    raise ValueError("invalid Mach-O dyld LEB128 operand")
def dyld_locations(payload: bytes, kind: str, segments: list, linkedit: int, dylibs: int, execute_mask: int) -> list:
    index, segment, address, value_type, symbol, ordinal, records, stopped, lazy_bound = 0, None, 0, (0 if kind in ("rebase", "bind") else 1), None, (-3 if kind == "weak" else None), [], False, False
    def emit(count, step=8):
        nonlocal address, lazy_bound
        width = 8 if value_type == 1 else 4 if value_type in (2, 3) else 0
        if not width or count > 1_000_000 or segment is None or not 0 <= segment < linkedit or (kind != "rebase" and (symbol is None or ordinal is None or not -3 <= ordinal <= dylibs)): raise ValueError("invalid Mach-O dyld fixup state")
        current = segments[segment]; protections = (current["max_protection"], current["initial_protection"]); writable = all(value & 2 and not value & execute_mask for value in protections); executable = all(value & execute_mask and not value & 2 for value in protections)
        if (value_type == 1 and not writable) or (value_type in (2, 3) and (not executable or kind != "rebase")): raise ValueError("Mach-O dyld fixup type does not match segment protections")
        for _ in range(count):
            location = current["vm_address"] + address
            if address > current["vm_size"] or 8 > current["vm_size"] - address: raise ValueError("Mach-O dyld fixup is outside its segment")
            records.append(["rebase" if kind == "rebase" else "bind", location, width]); address += step
        lazy_bound = kind == "lazy"
    while index < len(payload):
        byte = payload[index]; index += 1; opcode, immediate = byte & 0xF0, byte & 0x0F
        if kind == "rebase":
            if opcode == 0: stopped = True; break
            if opcode == 0x10: value_type = immediate
            elif opcode == 0x20: segment = immediate; address, index = leb128(payload, index)
            elif opcode == 0x30: value, index = leb128(payload, index); address += value
            elif opcode == 0x40: address += immediate * 8
            elif opcode in (0x50, 0x60): count, index = (immediate, index) if opcode == 0x50 else leb128(payload, index); emit(count)
            elif opcode == 0x70: value, index = leb128(payload, index); emit(1, 8 + value)
            elif opcode == 0x80: count, index = leb128(payload, index); skip, index = leb128(payload, index); emit(count, 8 + skip)
            else: raise ValueError("unsupported Mach-O rebase opcode")
            continue
        if opcode == 0:
            if kind != "lazy": stopped = True; break
            segment, address, symbol, ordinal, lazy_bound = None, 0, None, None, False; continue
        if opcode in (0x10, 0x20, 0x30):
            if kind == "weak": raise ValueError("unexpected dylib ordinal in weak bind")
            ordinal, index = (immediate, index) if opcode == 0x10 else (leb128(payload, index) if opcode == 0x20 else ((0 if immediate == 0 else immediate - 16), index)); continue
        if opcode == 0x40:
            end = payload.find(b"\0", index)
            if end <= index: raise ValueError("invalid Mach-O bind symbol")
            symbol, index = payload[index:end], end + 1
        elif opcode == 0x50:
            if kind == "lazy": raise ValueError("unsupported Mach-O lazy bind opcode")
            value_type = immediate
        elif opcode == 0x60: _, index = leb128(payload, index, True)
        elif opcode == 0x70: segment = immediate; address, index = leb128(payload, index)
        elif opcode == 0x80:
            if kind == "lazy": raise ValueError("unsupported Mach-O lazy bind opcode")
            value, index = leb128(payload, index); address += value
        elif opcode == 0x90: emit(1)
        elif opcode in (0xA0, 0xB0, 0xC0):
            if kind == "lazy": raise ValueError("unsupported Mach-O lazy bind opcode")
            if opcode == 0xA0: skip, index = leb128(payload, index); count = 1
            else: count, skip = (1, immediate * 8) if opcode == 0xB0 else (None, None)
            if opcode == 0xC0: count, index = leb128(payload, index); skip, index = leb128(payload, index)
            emit(count, 8 + skip)
        else: raise ValueError("unsupported Mach-O bind opcode")
    if kind == "lazy" and lazy_bound or stopped and (len(payload) - index > 15 or any(payload[index:])): raise ValueError("unterminated or trailing Mach-O dyld opcodes")
    return records
def macho_text(path: Path, architecture: str) -> dict:
    data = path.read_bytes(); expected_cpu = {"aarch64-apple-darwin": (0x0100000C, 0), "x86_64-apple-darwin": (0x01000007, 3)}.get(architecture); execute_mask = 0x04
    if len(data) < 32 or struct.unpack_from("<I", data)[0] != 0xfeedfacf: raise ValueError("daemon executable is not a thin little-endian Mach-O 64 artifact")
    cpu_type, cpu_subtype, file_type, count, command_bytes, header_flags, reserved = struct.unpack_from("<IIIIIII", data, 4); command_end = 32 + command_bytes
    if expected_cpu is None or cpu_type != expected_cpu[0]: raise ValueError("daemon executable architecture does not match the Rust host")
    if cpu_subtype & 0x00FFFFFF != expected_cpu[1]: raise ValueError("daemon executable CPU subtype does not match the Rust host")
    if file_type != 2 or header_flags & 0x200085 != 0x200085 or header_flags & 0x20112 or reserved or command_end > len(data) or command_bytes < count * 8: raise ValueError("daemon artifact is not a valid Mach-O executable")
    offset, sections, catalog, instruction_ranges, segments, dyld, sources = 32, [], [], [], [], [], []
    normalized_commands, entry_points, dylinkers, dependent_dylibs = bytearray(data[32:command_end]), [], [], []
    contained = lambda start, length, outer_start, outer_length: start >= outer_start and length <= outer_length and start - outer_start <= outer_length - length
    def load_path(command_offset, size, name_offset, label):
        if name_offset < 12 or name_offset >= size: raise ValueError(f"invalid Mach-O {label} path offset")
        payload = data[command_offset + name_offset:command_offset + size]; end = payload.find(b"\0")
        if end <= 0 or any(payload[end + 1:]): raise ValueError(f"invalid Mach-O {label} path")
        value = payload[:end].decode("utf-8")
        if not value.startswith("/"): raise ValueError(f"invalid Mach-O {label} path")
        return value
    for _ in range(count):
        if offset + 8 > command_end: raise ValueError("invalid Mach-O load command table")
        command, size = struct.unpack_from("<II", data, offset)
        if size < 8 or size % 8 or offset + size > command_end: raise ValueError("invalid Mach-O load command")
        if command == 0x80000034: raise ValueError("Mach-O chained fixups are not supported by the Catalog frame proof")
        if command not in {0x19, 0x80000022, 0x2, 0xB, 0xE, 0x1B, 0x32, 0x2A, 0x80000028, 0xC, 0x26, 0x29, 0x1D}: raise ValueError("unsupported Mach-O load command")
        relative = offset - 32
        if command == 0x1B:
            if size != 24: raise ValueError("invalid Mach-O UUID command")
            normalized_commands[relative + 8:relative + 24] = b"\0" * 16
        if command == 0x80000028:
            if size != 24: raise ValueError("invalid Mach-O entry point command")
            entry_offset, stack_size = struct.unpack_from("<QQ", data, offset + 8); entry_points.append({"file_offset": entry_offset, "stack_size": stack_size})
        if command == 0xE:
            if size < 16: raise ValueError("invalid Mach-O dynamic linker command")
            dylinkers.append(load_path(offset, size, struct.unpack_from("<I", data, offset + 8)[0], "dynamic linker"))
        if command == 0xC:
            if size < 32: raise ValueError("invalid Mach-O dependent library command")
            name_offset, timestamp, current_version, compatibility_version = struct.unpack_from("<4I", data, offset + 8); dependent_dylibs.append({"compatibility_version": compatibility_version, "current_version": current_version, "path": load_path(offset, size, name_offset, "dependent library"), "timestamp": timestamp})
        if command in (0x22, 0x80000022):
            if command != 0x80000022 or size != 48: raise ValueError("unsupported Mach-O dyld info command")
            dyld.append(struct.unpack_from("<10I", data, offset + 8))
        if command == 0x2:
            if size != 24: raise ValueError("invalid Mach-O symbol table command")
            symbol_offset, symbol_count, string_offset, string_size = struct.unpack_from("<4I", data, offset + 8); sources.extend((("symbol_table", symbol_offset, symbol_count * 16), ("string_table", string_offset, string_size)))
        if command == 0xB:
            if size != 80: raise ValueError("invalid Mach-O dynamic symbol table command")
            values = struct.unpack_from("<18I", data, offset + 8)
            for name, pair, width in (("table_of_contents", 6, 8), ("module_table", 8, 56), ("external_references", 10, 4), ("indirect_symbols", 12, 4), ("external_relocations", 14, 8), ("local_relocations", 16, 8)): sources.append((name, values[pair], values[pair + 1] * width))
        if command in (0x26, 0x29, 0x1D):
            if size != 16: raise ValueError("invalid Mach-O linkedit data command")
            start, length = struct.unpack_from("<II", data, offset + 8); sources.append(({0x26: "function_starts", 0x29: "data_in_code", 0x1D: "code_signature"}[command], start, length))
        if command == 0x19:
            if size < 72: raise ValueError("invalid Mach-O segment command")
            segment_name, vm_address, vm_size, file_offset, file_size, max_protection, initial_protection, section_count, segment_flags = struct.unpack_from("<16sQQQQiiII", data, offset + 8); segment_name = segment_name.rstrip(b"\0").decode("ascii")
            if size != 72 + section_count * 80: raise ValueError("invalid Mach-O section table")
            segment_record = {"file_offset": file_offset, "file_size": file_size, "flags": segment_flags, "initial_protection": initial_protection, "max_protection": max_protection, "segment": segment_name, "vm_address": vm_address, "vm_size": vm_size}; segments.append(segment_record)
            segment_file_valid = file_offset <= len(data) and file_size <= len(data) - file_offset
            for index in range(section_count):
                values = struct.unpack_from("<16s16sQQIIIIIIII", data, offset + 72 + index * 80); section, segment = (value.rstrip(b"\0").decode("ascii") for value in values[:2]); address, length, start, alignment, relocation_offset, relocation_count, flags, reserved1, reserved2, reserved3 = values[2:]
                file_contained = segment_file_valid and contained(start, length, file_offset, file_size) and contained(start, length, 0, len(data)) and start >= command_end; vm_contained = contained(address, length, vm_address, vm_size); congruent = address - vm_address == start - file_offset
                layout = {"address": address, "alignment": alignment, "byte_length": length, "file_offset": start, "flags": flags, "relocation_count": relocation_count, "relocation_offset": relocation_offset, "reserved1": reserved1, "reserved2": reserved2, "reserved3": reserved3, "section": section, "segment": segment}
                if (segment, section) == ("__DATA_CONST", "__tvcatalog"):
                    readable_nonexecuting = all(protection & 1 and not protection & ~3 for protection in (max_protection, initial_protection))
                    if segment_name != segment or flags != 0 or relocation_offset or relocation_count or reserved1 or reserved2 or reserved3 or not segment_flags & 0x10 or not readable_nonexecuting or not file_contained or not vm_contained or not congruent or length != CATALOG_SECTION["byte_length"] or any(data[start:start + length]): raise ValueError("invalid daemon Catalog placeholder section")
                    catalog.append(layout)
                if flags & (0x80000000 | 0x00000400):
                    readable_executing = all(protection & 1 and protection & execute_mask and not protection & 2 and not protection & ~(1 | execute_mask) for protection in (max_protection, initial_protection))
                    if segment_name != segment or not file_contained or not vm_contained or not congruent or not length or not readable_executing: raise ValueError("invalid executable instruction section range")
                    instruction_ranges.append((start, length, address, length)); sections.append({**layout, "sha256": sha256(data[start:start + length])})
        offset += size
    if offset != command_end or len(dyld) != 1 or len(entry_points) != 1 or len(dylinkers) != 1 or not dependent_dylibs: raise ValueError("invalid Mach-O load command table")
    names = [item["segment"] for item in segments]; expected = ["__TEXT", "__DATA_CONST", "__DATA", "__LINKEDIT"]
    if names not in (expected, ["__PAGEZERO", *expected]): raise ValueError("invalid Mach-O segment order")
    linkedit_index = names.index("__LINKEDIT"); linkedit = segments[linkedit_index]
    if linkedit_index != len(segments) - 1 or linkedit["max_protection"] != 1 or linkedit["initial_protection"] != 1: raise ValueError("invalid Mach-O __LINKEDIT segment")
    for item in segments:
        if item["file_size"] > item["vm_size"] or item["file_offset"] + item["file_size"] > len(data) or item["file_offset"] + item["file_size"] > (1 << 64) or item["vm_address"] + item["vm_size"] > (1 << 64): raise ValueError("invalid Mach-O segment range")
        if item["segment"] == "__PAGEZERO" and (item["file_size"] or item["max_protection"] or item["initial_protection"] or item["vm_address"]): raise ValueError("invalid Mach-O __PAGEZERO segment")
        if item["segment"] != "__PAGEZERO" and any(not protection & 1 or protection & 2 and protection & execute_mask or protection & ~(3 | execute_mask) for protection in (item["max_protection"], item["initial_protection"])): raise ValueError("invalid Mach-O segment protections")
    text_segment = segments[names.index("__TEXT")]
    if text_segment["file_offset"] or not contained(0, command_end, 0, text_segment["file_size"]): raise ValueError("Mach-O header and load commands are not in __TEXT")
    mapped = [item for item in segments if item["segment"] != "__PAGEZERO"]
    if mapped != sorted(mapped, key=lambda item: item["file_offset"]) or mapped != sorted(mapped, key=lambda item: item["vm_address"]): raise ValueError("Mach-O segment mappings are out of load-command order")
    overlap = lambda left, left_length, right, right_length: max(left, right) < min(left + left_length, right + right_length)
    for index, left in enumerate(segments):
        for right in segments[index + 1:]:
            if left["file_size"] and right["file_size"] and overlap(left["file_offset"], left["file_size"], right["file_offset"], right["file_size"]) or left["vm_size"] and right["vm_size"] and overlap(left["vm_address"], left["vm_size"], right["vm_address"], right["vm_size"]): raise ValueError("overlapping Mach-O segment mappings")
    stream_names, streams, stream_proof = ("rebase", "bind", "weak_bind", "lazy_bind", "export"), [], []
    for index, name in enumerate(stream_names):
        start, length = dyld[0][index * 2:index * 2 + 2]
        if (not length and start) or (length and not contained(start, length, linkedit["file_offset"], linkedit["file_size"])): raise ValueError("invalid Mach-O dyld info range")
        payload = data[start:start + length]; streams.append(payload); stream_proof.append({"byte_length": length, "file_offset": start, "kind": name, "sha256": sha256(payload)})
        sources.append((name, start, length))
    bound_sources, occupied_sources = [], []
    for name, start, length in sources:
        if length and not contained(start, length, linkedit["file_offset"], linkedit["file_size"]): raise ValueError("Mach-O loader source is outside __LINKEDIT")
        if length: occupied_sources.append((name, start, length)); bound_sources.append({"byte_length": length, "file_offset": start, "kind": name, "sha256": sha256(data[start:start + length])} if name != "code_signature" else None)
    bound_sources = sorted((item for item in bound_sources if item is not None), key=canonical)
    for index, (_, start, length) in enumerate(occupied_sources):
        for _, other_start, other_length in occupied_sources[index + 1:]:
            if overlap(start, length, other_start, other_length): raise ValueError("overlapping Mach-O loader sources")
    fixups = dyld_locations(streams[0], "rebase", segments, linkedit_index, len(dependent_dylibs), execute_mask) + sum((dyld_locations(stream, kind, segments, linkedit_index, len(dependent_dylibs), execute_mask) for stream, kind in zip(streams[1:4], ("bind", "weak", "lazy"))), [])
    sections.sort(key=canonical); layouts = [item for item in segments if item["segment"] != "__LINKEDIT"]
    if not any((item["segment"], item["section"]) == ("__TEXT", "__text") for item in sections) or len({(item["segment"], item["section"]) for item in sections}) != len(sections): raise ValueError("daemon executable has missing or duplicate instruction section identities")
    if len(catalog) != 1: raise ValueError("daemon executable must contain exactly one Catalog placeholder section")
    catalog_file, catalog_vm, catalog_length = catalog[0]["file_offset"], catalog[0]["address"], catalog[0]["byte_length"]
    entry_file = text_segment["file_offset"] + entry_points[0]["file_offset"]
    if not any(start <= entry_file < start + length for start, length, _, _ in instruction_ranges): raise ValueError("Mach-O entry point is outside executable instructions")
    if any(overlap(start, length, catalog_file, catalog_length) or overlap(address, vm_length, catalog_vm, catalog_length) for start, length, address, vm_length in instruction_ranges): raise ValueError("daemon executable code overlaps the Catalog placeholder section")
    if any(overlap(address, width, catalog_vm, catalog_length) for _, address, width in fixups): raise ValueError("daemon Catalog placeholder section contains a dyld fixup")
    linkedit_proof = {key: linkedit[key] for key in ("file_offset", "flags", "initial_protection", "max_protection", "segment", "vm_address")}; proof = {"algorithm": "classic_dyld_info_loader_state_v2", "bind_count": sum(item[0] == "bind" for item in fixups), "catalog_target_count": 0, "chained_fixups": False, "linkedit": linkedit_proof, "linkedit_sources": bound_sources, "rebase_count": sum(item[0] == "rebase" for item in fixups), "sha256": sha256(canonical(sorted(fixups))), "streams": stream_proof}; loader = {"dependent_dylibs": dependent_dylibs, "dynamic_linker": dylinkers[0], "entry_point": entry_points[0], "normalized_sha256": sha256(bytes(normalized_commands))}; identity = {"catalog_section": catalog[0], "fixup_proof": proof, "header": {"file_type": file_type, "flags": header_flags}, "loader_commands": loader, "sections": sections, "segments": layouts}
    return {"artifact": "turnvector-daemon", "architecture": architecture, "byte_length": sum(item["byte_length"] for item in sections), "catalog_section": catalog[0], "cpu_subtype": cpu_subtype, "cpu_type": cpu_type, "fixup_proof": proof, "format": "mach_o_64", "header": identity["header"], "loader_commands": loader, "present": True, "sections": sections, "segments": layouts, "sha256": sha256(canonical(identity))}
def reject_forbidden(value) -> None:
    if isinstance(value, dict):
        for key, child in value.items():
            if key in FORBIDDEN_KEYS: raise ValueError("final binary self hash is forbidden" if key == "final_binary_sha256" else f"forbidden identity input: {key}")
            reject_forbidden(child)
    elif isinstance(value, list):
        for child in value: reject_forbidden(child)
def validated_descriptor(payload: bytes) -> dict:
    value = read_json(payload); reject_forbidden(value)
    if canonical(value) != payload or value.get("descriptor_schema_version") != 1: raise ValueError("descriptor is not canonical version 1")
    for key, expected in CONTRACT.items():
        if value.get(key) != expected: raise ValueError(f"{key} contract mismatch")
    sections, keys = value.get("section_identities", {}), {"artifact", "architecture", "byte_length", "catalog_section", "cpu_subtype", "cpu_type", "fixup_proof", "format", "header", "loader_commands", "present", "sections", "segments", "sha256"}; native = {"artifacts": [], "byte_length": 0, "present": False, "sha256": EMPTY_SHA}
    if sections.get("native_text") != native or sections.get("catalog_payload") != CATALOG_SECTION or set(sections.get("executable_text", {})) != keys: raise ValueError("section identity mismatch")
    return value
def build_descriptor(root: Path) -> tuple:
    root = root.resolve(); source, launcher = root / "scripts/generate_daemon_core_build.py", root / LAUNCHER
    if Path(__file__).resolve() != source: raise ValueError("executing generator is not the generator declared by --root")
    paths, before = resolve_tools(root), tree_identity(root)
    if source.read_bytes() != _EXECUTED_SOURCE or launcher.read_bytes() != _EXECUTED_LAUNCHER_SOURCE: raise ValueError("executing generator or launcher bytes do not match the declared build input")
    with tempfile.TemporaryDirectory() as directory:
        temporary, snapshot = Path(directory).resolve(), Path(directory).resolve() / "source"; shutil.copytree(root, snapshot, symlinks=True, ignore=lambda _path, names: sorted(set(names) & IGNORED))
        if tree_identity(root) != before or tree_identity(snapshot) != before: raise ValueError("source tree changed while freezing build inputs")
        if any((base / ".cargo" / name).exists() for base in (snapshot, Path("/")) for name in ("config", "config.toml")): raise ValueError("Cargo configuration is not an admitted build input")
        if b'\nsource = "' in (snapshot / "Cargo.lock").read_bytes(): raise ValueError("only repository vendored path dependencies are allowed")
        placeholder = temporary / "catalog-placeholder.bin"
        with placeholder.open("wb") as handle: handle.truncate(CATALOG_SECTION["byte_length"])
        selected = toolchain(paths, root); flags = lambda sdk: [f'-Clinker={paths["clang"]}', "-Clink-arg=-isysroot", f'-Clink-arg={sdk}', "-Clink-arg=-t", f"-Clink-arg=-Wl,-sectcreate,__DATA_CONST,__tvcatalog,{placeholder}"]
        env = {"PATH": ":".join(dict.fromkeys(str(paths[name].parent) for name in ("cargo", "rustc", "clang", "ld"))) + ":/usr/bin:/bin", "HOME": str(temporary / "home"), "CARGO_HOME": str(temporary / "cargo-home"), "CARGO_TARGET_DIR": str(temporary / "discovery-target"), "CARGO_INCREMENTAL": "0", "RUSTC": str(paths["rustc"]), "RUSTUP_TOOLCHAIN": str(rust_sysroot(paths, snapshot)), "CARGO_ENCODED_RUSTFLAGS": "\x1f".join(flags(paths["sdk"])), "DEVELOPER_DIR": str(paths["developer"]), "SDKROOT": str(paths["sdk"]), "MACOSX_DEPLOYMENT_TARGET": "11.0", "LANG": "C", "LC_ALL": "C"}
        registry, cargo = generation_registry(snapshot, env), str(paths["cargo"]); common = ["--offline", "--locked", "--manifest-path", str(snapshot / "Cargo.toml")]
        if tree_identity(snapshot) != before: raise ValueError("source snapshot changed during Generation Semantics verification")
        metadata = read_json(run([cargo, "metadata", *common, "--format-version", "1", "--filter-platform", selected["rustc"]["host"]], Path("/"), env).encode()); graph, package_roots = dependency_graph(snapshot, metadata)
        check_env = {**env, "CARGO_TARGET_DIR": str(temporary / "check-target")}
        with concurrent.futures.ThreadPoolExecutor(max_workers=1) as pool:
            check = pool.submit(run, [cargo, "check", *common, "--package", "turnvector-daemon", "--bin", "turnvector-daemon"], Path("/"), check_env)
            trace = run([cargo, "build", *common, "--release", "--package", "turnvector-daemon", "--bin", "turnvector-daemon"], Path("/"), env, True); check.result()
        frozen(snapshot, before, "during daemon check")
        if tree_identity(snapshot) != before: raise ValueError("source snapshot changed during daemon build")
        linked, current_tools = sdk_link_inputs(trace, paths["sdk"]), toolchain(paths, root)
        if current_tools != selected: raise ValueError("toolchain changed after discovery")
        selected["native_link"]["link_inputs"] = tree_artifact(paths["sdk"], "sdk-link-inputs", linked); frozen_sdk = temporary / "frozen-sdk"; frozen_linked = freeze_sdk(paths["sdk"], linked, frozen_sdk); final_paths = {**paths, "sdk": frozen_sdk}
        env.update({"CARGO_TARGET_DIR": str(temporary / "target"), "CARGO_ENCODED_RUSTFLAGS": "\x1f".join(flags(frozen_sdk)), "SDKROOT": str(frozen_sdk)}); final_trace = run([cargo, "build", *common, "--release", "--package", "turnvector-daemon", "--bin", "turnvector-daemon"], Path("/"), env, True)
        if sdk_link_inputs(final_trace, frozen_sdk) != frozen_linked or sdk_link_inputs(final_trace, paths["sdk"], False): raise ValueError("linker input set changed after discovery")
        frozen(snapshot, before, "during final daemon build"); current_tools = toolchain(final_paths, root); current_tools["native_link"]["link_inputs"] = tree_artifact(frozen_sdk, "sdk-link-inputs", frozen_linked)
        if current_tools != selected: raise ValueError("toolchain or linker input changed during final daemon build")
        dev_traced, dev_environment = traced_inputs(snapshot, temporary / "check-target", package_roots); discovery_traced, discovery_environment = traced_inputs(snapshot, temporary / "discovery-target", package_roots); release_traced, release_environment = traced_inputs(snapshot, temporary / "target", package_roots); traced, environment = sorted({*dev_traced, *discovery_traced, *release_traced}), sorted({*dev_environment, *discovery_environment, *release_environment}); names = source_names(snapshot, graph, traced); executable, records = macho_text(temporary / "target/release/turnvector-daemon", selected["rustc"]["host"]), source_records(before, names)
        if tree_identity(snapshot) != before: raise ValueError("source snapshot changed before descriptor publication")
        descriptor = {"descriptor_schema_version": 1, "source_closure": {"algorithm": "isolated_snapshot_plus_dev_and_release_daemon_dep_info_v1", "dependency_policy": "reachable_normal_repository_path_dependencies_with_single_turnvector_core_build_rs_v1", "runtime_inputs": traced, "environment_inputs": environment, "files": records}, "toolchain": selected, "dependency_graph": graph, "registries": {"protocol": [], "domain": [registry]}, "section_identities": {"executable_text": executable, "native_text": {"artifacts": [], "byte_length": 0, "present": False, "sha256": EMPTY_SHA}, "catalog_payload": CATALOG_SECTION}, **CONTRACT}
    return descriptor, (before, paths, selected, linked)
def stable(root: Path, guard: tuple) -> None:
    before, paths, selected, linked = guard; current = toolchain(paths, root); current["native_link"]["link_inputs"] = tree_artifact(paths["sdk"], "sdk-link-inputs", linked)
    if tree_identity(root.resolve()) != before or current != selected: raise ValueError("source or toolchain changed before descriptor publication")
def output_pair(directory: int) -> tuple:
    result = []; identity = lambda item: (item.st_dev, item.st_ino, item.st_mode, item.st_size, item.st_mtime_ns, item.st_ctime_ns)
    for name in (DESCRIPTOR, LOCK):
        descriptor = os.open(name, os.O_RDONLY | os.O_NOFOLLOW, dir_fd=directory)
        with os.fdopen(descriptor, "rb") as handle: before = os.fstat(handle.fileno()); payload = handle.read(); current = os.stat(name, dir_fd=directory, follow_symlinks=False)
        if not stat.S_ISREG(before.st_mode) or stat.S_IMODE(before.st_mode) != 0o644 or identity(before) != identity(current): raise ValueError("descriptor output pair changed or has an invalid type or mode")
        result.append(payload)
    return tuple(result)
def publish_pair(directory: int, pairs: tuple) -> None:
    temporary = f".daemon-core-build.{os.urandom(16).hex()}"; os.mkdir(temporary, 0o700, dir_fd=directory)
    try:
        for name, payload in pairs:
            descriptor = os.open(f"{temporary}/{name}", os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600, dir_fd=directory)
            with os.fdopen(descriptor, "wb") as handle: handle.write(payload); os.fchmod(handle.fileno(), 0o644)
        for name, _payload in pairs: os.replace(f"{temporary}/{name}", name, src_dir_fd=directory, dst_dir_fd=directory)
    finally:
        for name, _payload in pairs:
            with contextlib.suppress(FileNotFoundError): os.unlink(f"{temporary}/{name}", dir_fd=directory)
        os.rmdir(temporary, dir_fd=directory)
def lock_for(payload: bytes) -> dict: return {"algorithm": "sha256", "descriptor": DESCRIPTOR, "digest": sha256(HASH_DOMAIN + (1).to_bytes(4, "big") + payload), "domain": HASH_DOMAIN[:-1].decode(), "hash_schema_version": 1, "kind": "daemon_core_build", "preimage": "utf8(domain)||0x00||u32be(hash_schema_version)||descriptor_bytes"}
def generate(root: Path, output: Path) -> None:
    output.mkdir(parents=True, exist_ok=True); directory = os.open(output, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        descriptor, guard = build_descriptor(root); payload = canonical(descriptor); lock_payload = canonical(lock_for(payload))
        stable(root, guard)
        if not os.path.samestat(os.stat(output, follow_symlinks=False), os.fstat(directory)): raise ValueError("output directory changed before publication")
        publish_pair(directory, ((DESCRIPTOR, payload), (LOCK, lock_payload))); stable(root, guard)
        if not os.path.samestat(os.stat(output, follow_symlinks=False), os.fstat(directory)) or output_pair(directory) != (payload, lock_payload): raise ValueError("descriptor output pair or directory changed after publication")
    finally: os.close(directory)
def check(root: Path, output: Path) -> None:
    directory = os.open(output, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        pair = output_pair(directory); payload, lock_payload = pair; candidate = validated_descriptor(payload)
        if canonical(read_json(lock_payload)) != lock_payload or read_json(lock_payload) != lock_for(payload): raise ValueError("daemon Core Build Evidence Hash lock does not match")
        expected, guard = build_descriptor(root)
        if candidate != expected: raise ValueError("descriptor was not generated by this build")
        stable(root, guard)
        if not os.path.samestat(os.stat(output, follow_symlinks=False), os.fstat(directory)) or output_pair(directory) != pair: raise ValueError("descriptor output pair or directory changed during verification")
    finally: os.close(directory)
def main() -> None:
    parser = argparse.ArgumentParser(); parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[1]); mode = parser.add_mutually_exclusive_group(required=True); mode.add_argument("--output", type=Path); mode.add_argument("--check", type=Path); args = parser.parse_args()
    try: generate(args.root, args.output) if args.output else check(args.root, args.check)
    except (OSError, ValueError, KeyError, json.JSONDecodeError, subprocess.SubprocessError, struct.error) as error: parser.exit(1, f"daemon core build error: {error}\n")
if __name__ == "__main__": main()
