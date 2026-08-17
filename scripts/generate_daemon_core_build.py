#!/usr/bin/env -S python3 -I -S -B
import argparse, contextlib, hashlib, json, os, shlex, shutil, stat, struct, subprocess, sys, tempfile
from pathlib import Path
if not (sys.flags.isolated and sys.flags.no_site and sys.flags.dont_write_bytecode): raise SystemExit("daemon core build error: generator requires Python -I -S -B\n")
_EXECUTED_SOURCE, _EXECUTED_LAUNCHER_SOURCE = (globals().get(name) for name in ("_EXECUTED_SOURCE", "_EXECUTED_LAUNCHER_SOURCE"))
if not all(isinstance(value, bytes) for value in (_EXECUTED_SOURCE, _EXECUTED_LAUNCHER_SOURCE)): raise SystemExit("daemon core build error: generator requires the captured-source launcher\n")
DESCRIPTOR, LOCK, LAUNCHER = "daemon-core-build-v1.json", "daemon-core-build-v1.lock.json", "scripts/run_daemon_core_build.py"; HASH_DOMAIN = b"turnvector:evidence:daemon-core-build\0"; RUNTIME_INPUTS = ("crates/turnvector-core/src/lib.rs", "crates/turnvector-daemon/src/main.rs"); BOOTSTRAP = 'import os,stat,sys\np=sys.argv[1];sys.argv=sys.argv[1:];h=os.fdopen(os.open(p,os.O_RDONLY|os.O_NOFOLLOW),"rb");a=os.fstat(h.fileno());b=h.read();c=os.fstat(h.fileno());h.close()\ni=lambda x:(x.st_dev,x.st_ino,x.st_mode,x.st_size,x.st_mtime_ns,x.st_ctime_ns)\nif not stat.S_ISREG(a.st_mode) or stat.S_IMODE(a.st_mode)!=0o755 or i(a)!=i(c):raise ValueError("captured launcher changed or has an invalid type or mode")\nexec(compile(b,p,"exec"),{"__name__":"__main__","__file__":p,"_EXECUTED_LAUNCHER_SOURCE":b})'
STATIC_INPUTS = ("Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "crates/turnvector-core/Cargo.toml", "crates/turnvector-daemon/Cargo.toml", *RUNTIME_INPUTS, "schemas/generation-semantics-v1.json", "schemas/generation-semantics-v1.lock.json", "scripts/generate_generation_semantics.py", "scripts/generate_daemon_core_build.py", LAUNCHER)
IGNORED = {".git", ".internal", ".work", "target", "__pycache__", ".DS_Store", DESCRIPTOR, LOCK}; FORBIDDEN_KEYS = {"catalog_payload_bytes", "catalog_identity", "outer_daemon_build_identity", "final_binary_sha256"}; EMPTY_SHA = hashlib.sha256(b"").hexdigest()
CONTRACT = json.loads(r'''{"build_variants":{"all_features":false,"default_features":true,"deployment_target":"11.0","generator_execution":{"command":"<bound-python> -I -S -B -c <bootstrap_source> scripts/run_daemon_core_build.py","protocol":"external_single_read_captured_source_v2"},"linker_flags":"-Clinker=<bound-clang> -Clink-arg=-isysroot -Clink-arg=<bound-sdk> -Clink-arg=-t","profiles":["dev","release"],"selected_features":[]},"cardinality_inputs":{"encoding":"u16","ingress":{"connections":64,"global_active":1024,"global_warming":256,"per_connection_active":64,"per_connection_warming":16},"model_registry":256},"catalog":{"capacity":{"max_canonical_bytes":4194304,"max_entries":256,"max_entry_bytes":16384},"lookup":{"algorithm":"sorted_sha256_key_binary_search_then_canonical_equality_v1","key_bytes":32},"schema_version":1,"worst_case_work":{"entry_bytes_validated":16384,"key_bytes_compared":288,"key_comparisons":9}},"event_registry":{"mandatory_crossing_max":1024,"max_entries":4096,"max_kinds":256,"schema_version":1},"excluded_identity_inputs":["runtime_overhead_catalog_payload_bytes","runtime_overhead_catalog_identity","outer_daemon_build_identity","request_certification_case_bound_tables","lifecycle_overhead_qualification","final_binary_sha256"],"native_inputs":{"adapter_schema_version":1,"files":[],"interface_revision":1},"prepared_carry":{"mandatory_suballocation_max":8192,"nonborrowable":true,"safety_suballocation_max":8192,"slots":1},"support":{"funding_claim":{"max_claims_per_obligation":256,"nonempty":true,"schema_version":1,"variants":["ordinary_reservation","admission_initial","entitlement_vector","lifecycle_reserve"]},"operations":["describe_model","describe_request","materialize_request","release_request","form_candidates","observe_turn_receipt","sample_backend_resources"],"outstanding_credit_vector":{"axes":["operation","pool","horizon"],"credit_encoding":"u16","max_dimensions":168,"schema_version":1},"pools":["ordinary","mandatory_completion","safety_sampling"],"records":{"active_and_retained":8192,"conditional_obligations":4096,"entitlement_tombstones":2048,"funding_claims":1048576,"lifecycle_reserves":2048,"ordinary_claims":4096,"pending_obligations":4096,"total_operation_obligations":16384},"start_count":{"count_encoding":"u32","max_cells":168,"max_horizons":8,"max_physical_credits":16384,"schema_version":1}}}'''); CONTRACT["build_variants"]["generator_execution"].update({"bootstrap_source": BOOTSTRAP, "bootstrap_sha256": hashlib.sha256(BOOTSTRAP.encode()).hexdigest()})
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
def resolve_tools() -> dict:
    values = {"python": sys.executable, "cargo": shutil.which("cargo"), "rustc": shutil.which("rustc")}
    if any(value is None for value in values.values()): raise ValueError("required build tool not found")
    paths = {name: Path(value).resolve() for name, value in values.items()}; paths["xcrun"] = Path("/usr/bin/xcrun")
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
def toolchain(paths: dict) -> dict:
    env, root = {"PATH": "/usr/bin:/bin", "LANG": "C", "LC_ALL": "C"}, Path("/")
    sysroot = Path(run([paths["rustc"], "--print", "sysroot"], root, env).strip()).resolve(); target = Path(run([paths["rustc"], "--print", "target-libdir"], root, env).strip()).resolve()
    files = sorted([*sysroot.glob("lib/librustc_driver*.dylib"), *(path for path in target.rglob("*") if path.is_file())])
    def identity(name, args): return {"version": run([paths[name], *args], root, env).strip(), "binary_sha256": sha256(paths[name].read_bytes())}
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
    identities, packages, roots = {}, [], []
    for package_id in runtime:
        package, manifest = by_id[package_id], Path(by_id[package_id]["manifest_path"]).resolve()
        if not manifest.is_relative_to(root): raise ValueError(f"undeclared local dependency outside repository: {manifest}")
        relative = manifest.relative_to(root).as_posix()
        if package["source"] is not None or (package_id not in workspace and not relative.startswith("vendor/")): raise ValueError("only repository vendored path dependencies are allowed")
        kinds = {kind for target in package["targets"] for kind in target["kind"]}
        if "custom-build" in kinds or "proc-macro" in kinds: raise ValueError("runtime packages may not use build scripts or proc-macro targets")
        roots.append(manifest.parent); identities[package_id] = f'{package["name"]}@{package["version"]}|{relative}'
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
                    if not any(parent == resolved or parent in resolved.parents for parent in package_roots): raise ValueError(f"undeclared runtime input: {relative}")
                    observed.add(relative)
    missing = sorted(set(RUNTIME_INPUTS) - observed)
    if missing: raise ValueError(f"declared runtime input was not dependency-traced: {missing[0]}")
    return sorted(observed), sorted(environment)
def generation_registry(root: Path, env: dict) -> dict:
    try:
        run([sys.executable, "-I", "-S", "-B", root / "scripts/generate_generation_semantics.py", "--check", root / "schemas"], root, env)
    except ValueError as error: raise ValueError(f"current Generation Semantics verification failed: {error}")
    descriptor = (root / "schemas/generation-semantics-v1.json").read_bytes(); lock = read_json((root / "schemas/generation-semantics-v1.lock.json").read_bytes()); return {"kind": "generation_semantics", "identity": lock["digest"], "descriptor_sha256": sha256(descriptor)}
def macho_text(path: Path, architecture: str) -> dict:
    data = path.read_bytes(); expected_cpu = {"aarch64-apple-darwin": (0x0100000C, 0), "x86_64-apple-darwin": (0x01000007, 3)}.get(architecture)
    if len(data) < 32 or struct.unpack_from("<I", data)[0] != 0xfeedfacf: raise ValueError("daemon executable is not a thin little-endian Mach-O 64 artifact")
    cpu_type, cpu_subtype = struct.unpack_from("<II", data, 4)
    if expected_cpu is None or cpu_type != expected_cpu[0]: raise ValueError("daemon executable architecture does not match the Rust host")
    if cpu_subtype & 0x00FFFFFF != expected_cpu[1]: raise ValueError("daemon executable CPU subtype does not match the Rust host")
    count, offset, sections = struct.unpack_from("<I", data, 16)[0], 32, []
    for _ in range(count):
        command, size = struct.unpack_from("<II", data, offset)
        if size < 8 or offset + size > len(data): raise ValueError("invalid Mach-O load command")
        if command == 0x19:
            section_count = struct.unpack_from("<I", data, offset + 64)[0]
            if size < 72 + section_count * 80: raise ValueError("invalid Mach-O section table")
            for index in range(section_count):
                values = struct.unpack_from("<16s16sQQIIIIIIII", data, offset + 72 + index * 80); section, segment = (value.rstrip(b"\0").decode("ascii") for value in values[:2])
                if values[8] & (0x80000000 | 0x00000400):
                    length, start = values[3:5]
                    if start + length > len(data) or not length: raise ValueError("invalid executable instruction section range")
                    sections.append({"byte_length": length, "section": section, "segment": segment, "sha256": sha256(data[start:start + length])})
        offset += size
    sections.sort(key=canonical)
    if not any((item["segment"], item["section"]) == ("__TEXT", "__text") for item in sections): raise ValueError("daemon executable has no __TEXT,__text section")
    return {"artifact": "turnvector-daemon", "architecture": architecture, "byte_length": sum(item["byte_length"] for item in sections), "cpu_subtype": cpu_subtype, "cpu_type": cpu_type, "format": "mach_o_64", "present": True, "sections": sections, "sha256": sha256(canonical(sections))}
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
    sections, keys = value.get("section_identities", {}), {"artifact", "architecture", "byte_length", "cpu_subtype", "cpu_type", "format", "present", "sections", "sha256"}; native = {"artifacts": [], "byte_length": 0, "present": False, "sha256": EMPTY_SHA}; catalog = {"executable": False, "format": "mach_o_64", "section": "__tvcatalog", "segment": "__DATA_CONST", "writable": False}
    if sections.get("native_text") != native or sections.get("catalog_payload") != catalog or set(sections.get("executable_text", {})) != keys: raise ValueError("section identity mismatch")
    return value
def build_descriptor(root: Path) -> tuple:
    root = root.resolve(); source, launcher = root / "scripts/generate_daemon_core_build.py", root / LAUNCHER
    if Path(__file__).resolve() != source: raise ValueError("executing generator is not the generator declared by --root")
    paths, before = resolve_tools(), tree_identity(root)
    if source.read_bytes() != _EXECUTED_SOURCE or launcher.read_bytes() != _EXECUTED_LAUNCHER_SOURCE: raise ValueError("executing generator or launcher bytes do not match the declared build input")
    with tempfile.TemporaryDirectory() as directory:
        temporary, snapshot = Path(directory).resolve(), Path(directory).resolve() / "source"; shutil.copytree(root, snapshot, symlinks=True, ignore=lambda _path, names: sorted(set(names) & IGNORED))
        if tree_identity(root) != before or tree_identity(snapshot) != before: raise ValueError("source tree changed while freezing build inputs")
        if any((base / ".cargo" / name).exists() for base in (snapshot, Path("/")) for name in ("config", "config.toml")): raise ValueError("Cargo configuration is not an admitted build input")
        if b'\nsource = "' in (snapshot / "Cargo.lock").read_bytes(): raise ValueError("only repository vendored path dependencies are allowed")
        selected = toolchain(paths); flags = lambda sdk: [f'-Clinker={paths["clang"]}', "-Clink-arg=-isysroot", f'-Clink-arg={sdk}', "-Clink-arg=-t"]
        env = {"PATH": ":".join(dict.fromkeys(str(paths[name].parent) for name in ("cargo", "rustc", "clang", "ld"))) + ":/usr/bin:/bin", "HOME": str(temporary / "home"), "CARGO_HOME": str(temporary / "cargo-home"), "CARGO_TARGET_DIR": str(temporary / "discovery-target"), "CARGO_INCREMENTAL": "0", "RUSTC": str(paths["rustc"]), "CARGO_ENCODED_RUSTFLAGS": "\x1f".join(flags(paths["sdk"])), "DEVELOPER_DIR": str(paths["developer"]), "SDKROOT": str(paths["sdk"]), "MACOSX_DEPLOYMENT_TARGET": "11.0", "LANG": "C", "LC_ALL": "C"}
        registry, cargo = generation_registry(snapshot, env), str(paths["cargo"]); common = ["--offline", "--locked", "--manifest-path", str(snapshot / "Cargo.toml")]
        if tree_identity(snapshot) != before: raise ValueError("source snapshot changed during Generation Semantics verification")
        metadata = read_json(run([cargo, "metadata", *common, "--format-version", "1", "--filter-platform", selected["rustc"]["host"]], Path("/"), env).encode()); graph, package_roots = dependency_graph(snapshot, metadata)
        for profile in ([], ["--release"]):
            run([cargo, "check", *common, "--package", "turnvector-daemon", "--bin", "turnvector-daemon", *profile], Path("/"), env); frozen(snapshot, before, "during daemon check")
        trace = run([cargo, "build", *common, "--release", "--package", "turnvector-daemon", "--bin", "turnvector-daemon"], Path("/"), env, True)
        if tree_identity(snapshot) != before: raise ValueError("source snapshot changed during daemon build")
        linked, current_tools = sdk_link_inputs(trace, paths["sdk"]), toolchain(paths)
        if current_tools != selected: raise ValueError("toolchain changed after discovery")
        selected["native_link"]["link_inputs"] = tree_artifact(paths["sdk"], "sdk-link-inputs", linked); frozen_sdk = temporary / "frozen-sdk"; frozen_linked = freeze_sdk(paths["sdk"], linked, frozen_sdk); final_paths = {**paths, "sdk": frozen_sdk}
        env.update({"CARGO_TARGET_DIR": str(temporary / "target"), "CARGO_ENCODED_RUSTFLAGS": "\x1f".join(flags(frozen_sdk)), "SDKROOT": str(frozen_sdk)}); final_trace = run([cargo, "build", *common, "--release", "--package", "turnvector-daemon", "--bin", "turnvector-daemon"], Path("/"), env, True)
        if sdk_link_inputs(final_trace, frozen_sdk) != frozen_linked or sdk_link_inputs(final_trace, paths["sdk"], False): raise ValueError("linker input set changed after discovery")
        frozen(snapshot, before, "during final daemon build"); current_tools = toolchain(final_paths); current_tools["native_link"]["link_inputs"] = tree_artifact(frozen_sdk, "sdk-link-inputs", frozen_linked)
        if current_tools != selected: raise ValueError("toolchain or linker input changed during final daemon build")
        dev_traced, dev_environment = traced_inputs(snapshot, temporary / "discovery-target", package_roots); release_traced, release_environment = traced_inputs(snapshot, temporary / "target", package_roots); traced, environment = sorted({*dev_traced, *release_traced}), sorted({*dev_environment, *release_environment}); names = source_names(snapshot, graph, traced); executable, records = macho_text(temporary / "target/release/turnvector-daemon", selected["rustc"]["host"]), source_records(before, names)
        if tree_identity(snapshot) != before: raise ValueError("source snapshot changed before descriptor publication")
        descriptor = {"descriptor_schema_version": 1, "source_closure": {"algorithm": "isolated_snapshot_plus_dev_and_release_daemon_dep_info_v1", "dependency_policy": "reachable_normal_repository_path_dependencies_without_build_code_v1", "runtime_inputs": traced, "environment_inputs": environment, "files": records}, "toolchain": selected, "dependency_graph": graph, "registries": {"protocol": [], "domain": [registry]}, "section_identities": {"executable_text": executable, "native_text": {"artifacts": [], "byte_length": 0, "present": False, "sha256": EMPTY_SHA}, "catalog_payload": {"executable": False, "format": "mach_o_64", "section": "__tvcatalog", "segment": "__DATA_CONST", "writable": False}}, **CONTRACT}
    return descriptor, (before, paths, selected, linked)
def stable(root: Path, guard: tuple) -> None:
    before, paths, selected, linked = guard; current = toolchain(paths); current["native_link"]["link_inputs"] = tree_artifact(paths["sdk"], "sdk-link-inputs", linked)
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
