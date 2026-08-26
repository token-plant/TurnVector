#!/usr/bin/env -S python3 -I -S -B
import argparse, contextlib, hashlib, json, os, shutil, stat, struct, subprocess, sys, tempfile, types
from pathlib import Path
if __name__ == "__main__" and not (sys.flags.isolated and sys.flags.no_site and sys.flags.dont_write_bytecode): raise SystemExit("runtime overhead binding error: binder requires Python -I -S -B\n")
_EXECUTED_SOURCE, _EXECUTED_LAUNCHER_SOURCE = (globals().get(name) for name in ("_EXECUTED_SOURCE", "_EXECUTED_LAUNCHER_SOURCE")); DESCRIPTOR, LOCK = "daemon-build-v1.json", "daemon-build-v1.lock.json"; CORE, CORE_LOCK, CATALOG, CATALOG_LOCK = "daemon-core-build-v1.json", "daemon-core-build-v1.lock.json", "runtime-overhead-catalog-v1.json", "runtime-overhead-catalog-v1.lock.json"
MAGIC, HEADER, HEADER_BYTES, FRAME_BYTES, MAX_PAYLOAD = b"TURNVECTORCATV1\0", struct.Struct(">16sIIIII32s32s32s"), 256, 4194560, 4194304; CORE_DOMAIN = b"turnvector:evidence:daemon-core-build\0"; CATALOG_DOMAIN = b"turnvector:evidence:runtime-overhead-catalog\0"; OUTER_DOMAIN = b"turnvector:evidence:outer-daemon-build\0"; TYPES = {"test_fixture_nonproduction": 1, "production": 2}; IDENTIFIER = "org.turnvector.daemon"
class CatalogIndex:
    __slots__ = ("_entries",)
    def __init__(self, catalog):
        entries = catalog.get("entries") if isinstance(catalog, dict) else None; result, previous = [], None
        if not isinstance(entries, list) or len(entries) > 256: raise ValueError("invalid Catalog lookup table")
        for entry in entries:
            encoded, text = canonical(entry), entry.get("key_sha256") if isinstance(entry, dict) else None
            try: digest = bytes.fromhex(text) if isinstance(text, str) else b""
            except ValueError: digest = b""
            key = canonical(entry.get("key")) if isinstance(entry, dict) else b""
            if len(encoded) > 16384 or len(digest) != 32 or digest.hex() != text or hashlib.sha256(key).digest() != digest or previous is not None and previous >= digest: raise ValueError("invalid Catalog lookup table")
            result.append((digest, key, encoded)); previous = digest
        object.__setattr__(self, "_entries", tuple(result))
    @property
    def entries(self): return self._entries
    def __setattr__(self, _name, _value): raise AttributeError("CatalogIndex is immutable")
def canonical(value) -> bytes: return (json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n").encode()
def unique(pairs):
    result = dict(pairs)
    if len(result) != len(pairs): raise ValueError("duplicate JSON key")
    return result
def decode(payload: bytes): return json.loads(payload, object_pairs_hook=unique)
def sha256(payload: bytes) -> str: return hashlib.sha256(payload).hexdigest()
def evidence(domain: bytes, payload: bytes) -> str: return sha256(domain + (1).to_bytes(4, "big") + payload)
def catalog_identity(payload: bytes) -> str: return evidence(CATALOG_DOMAIN, payload)
def outer_identity(core: str, catalog: str) -> str: return sha256(OUTER_DOMAIN + (1).to_bytes(4, "big") + bytes.fromhex(core) + bytes.fromhex(catalog))
def walk_keys(value):
    if isinstance(value, dict):
        for key, child in value.items(): yield key; yield from walk_keys(child)
    elif isinstance(value, list):
        for child in value: yield from walk_keys(child)
def captured(path: Path, expected_mode=0o755) -> bytes:
    handle = os.fdopen(os.open(path, os.O_RDONLY | os.O_NOFOLLOW), "rb"); before = os.fstat(handle.fileno()); payload = handle.read(); after = os.fstat(handle.fileno()); handle.close()
    identity = lambda value: (value.st_dev, value.st_ino, value.st_mode, value.st_size, value.st_mtime_ns, value.st_ctime_ns)
    if not stat.S_ISREG(before.st_mode) or stat.S_IMODE(before.st_mode) != expected_mode or identity(before) != identity(after): raise ValueError(f"invalid or changing build input: {path.name}")
    return payload
def read_canonical(path: Path):
    payload = captured(path, 0o644); value = decode(payload)
    if canonical(value) != payload: raise ValueError(f"noncanonical JSON: {path.name}")
    return value, payload
def expected_lock(descriptor: str, digest: str, domain: str, kind: str): return {"algorithm": "sha256", "descriptor": descriptor, "digest": digest, "domain": domain, "hash_schema_version": 1, "kind": kind, "preimage": "utf8(domain)||0x00||u32be(hash_schema_version)||descriptor_bytes"}
def build_frame(payload: bytes, core: str, catalog: str, catalog_type: str, test_only: bool) -> bytes:
    value = decode(payload)
    if canonical(value) != payload or len(payload) > MAX_PAYLOAD or catalog_identity(payload) != catalog or catalog_type not in TYPES or test_only != (catalog_type != "production"): raise ValueError("invalid Catalog frame input")
    catalog_index(value)
    prefix = HEADER.pack(MAGIC, 1, HEADER_BYTES, len(payload), TYPES[catalog_type], int(test_only), bytes.fromhex(core), bytes.fromhex(catalog), bytes.fromhex(outer_identity(core, catalog)))
    return prefix + bytes(HEADER_BYTES - len(prefix)) + payload + bytes(MAX_PAYLOAD - len(payload))
def parse_frame(frame: bytes):
    if len(frame) != FRAME_BYTES: raise ValueError("invalid Catalog frame length")
    magic, version, header_bytes, length, kind, flags, core, catalog, outer = HEADER.unpack_from(frame)
    if magic != MAGIC or version != 1 or header_bytes != HEADER_BYTES or length > MAX_PAYLOAD or kind not in TYPES.values() or flags not in (0, 1) or flags != (kind == TYPES["test_fixture_nonproduction"]) or any(frame[HEADER.size:HEADER_BYTES]): raise ValueError("invalid Catalog frame header")
    payload, padding = frame[HEADER_BYTES:HEADER_BYTES + length], frame[HEADER_BYTES + length:]
    if any(padding): raise ValueError("nonzero Catalog frame padding")
    value = decode(payload); core_hex, catalog_hex = core.hex(), catalog.hex()
    if canonical(value) != payload or catalog_identity(payload) != catalog_hex or outer_identity(core_hex, catalog_hex) != outer.hex() or value.get("catalog_type", next(name for name, code in TYPES.items() if code == kind)) != next(name for name, code in TYPES.items() if code == kind): raise ValueError("Catalog frame tuple or payload mismatch")
    return {"catalog_identity": catalog_hex, "catalog_type": next(name for name, code in TYPES.items() if code == kind), "core_identity": core_hex, "outer_identity": outer.hex(), "payload_bytes": length, "test_only": bool(flags)}, value, catalog_index(value)
def catalog_index(catalog: dict):
    return CatalogIndex(catalog)
def compare_key(left: bytes, right: bytes):
    for offset, (left_byte, right_byte) in enumerate(zip(left, right)):
        if left_byte != right_byte: return (-1 if left_byte < right_byte else 1), offset + 1
    return 0, 32
def lookup(index: CatalogIndex, key: dict):
    if type(index) is not CatalogIndex: raise ValueError("lookup requires a validated Catalog index")
    key_payload = canonical(key)
    if len(key_payload) > 16384: raise ValueError("Catalog lookup key exceeds the fixed bound")
    entries, target = index.entries, hashlib.sha256(key_payload).digest(); low, high, comparisons, compared = 0, len(entries), 0, 0
    while low < high:
        middle = (low + high) // 2; relation, work = compare_key(target, entries[middle][0]); comparisons += 1; compared += work
        if relation == 0:
            if entries[middle][1] != key_payload: raise ValueError("Catalog key digest collision")
            if comparisons > 9 or compared > 288 or len(entries[middle][2]) > 16384: raise ValueError("Catalog lookup work exceeds the fixed bound")
            return decode(entries[middle][2]), {"entry_bytes_validated": len(entries[middle][2]), "key_bytes_compared": compared, "key_comparisons": comparisons}
        if relation > 0: low = middle + 1
        else: high = middle
    if comparisons > 9 or compared > 288: raise ValueError("Catalog lookup work exceeds the fixed bound")
    return None, {"entry_bytes_validated": 0, "key_bytes_compared": compared, "key_comparisons": comparisons}
def load_inputs(root: Path, catalog_source: Path, allow_fixture: bool):
    root = root.resolve(); schemas = root / "schemas"; launcher = root / "scripts/run_runtime_overhead_binding.py"
    if not all(isinstance(value, bytes) for value in (_EXECUTED_SOURCE, _EXECUTED_LAUNCHER_SOURCE)) or captured(Path(__file__).resolve()) != _EXECUTED_SOURCE or captured(launcher) != _EXECUTED_LAUNCHER_SOURCE: raise ValueError("executed binding source does not match captured bytes")
    core, core_payload = read_canonical(schemas / CORE); core_lock, core_lock_payload = read_canonical(schemas / CORE_LOCK); catalog, catalog_payload = read_canonical(schemas / CATALOG); catalog_lock, catalog_lock_payload = read_canonical(schemas / CATALOG_LOCK)
    core_digest, catalog_digest = evidence(CORE_DOMAIN, core_payload), catalog_identity(catalog_payload)
    if core_lock != expected_lock(CORE, core_digest, CORE_DOMAIN[:-1].decode(), "daemon_core_build") or catalog_lock != expected_lock(CATALOG, catalog_digest, CATALOG_DOMAIN[:-1].decode(), "runtime_overhead_catalog") or catalog.get("daemon_core_build_identity") != core_digest: raise ValueError("Core or Catalog Evidence Hash lock mismatch")
    catalog_type = catalog.get("catalog_type")
    if catalog_type not in TYPES or catalog_type != "production" and not allow_fixture: raise ValueError("nonproduction Catalog requires an explicit test-only target")
    frame = core.get("catalog", {}).get("frame", {}); section = core.get("section_identities", {}).get("catalog_payload", {})
    if frame != {"encoding": "fixed_binary_tuple_v1", "header_bytes": HEADER_BYTES, "section_bytes": FRAME_BYTES} or section.get("byte_length") != FRAME_BYTES or section.get("section") != "__tvcatalog" or section.get("segment") != "__DATA_CONST" or section.get("executable") is not False or section.get("writable") is not False: raise ValueError("Core Catalog frame contract mismatch")
    compiler = root / "scripts/compile_runtime_overhead_catalog.py"; compiler_payload = captured(compiler); namespace = {"__name__": "runtime_overhead_catalog_dependency", "__file__": str(compiler)}; exec(compile(compiler_payload, str(compiler), "exec"), namespace); compiled = namespace["compile_catalog"](schemas, catalog_source.resolve())
    if compiled != catalog: raise ValueError("Catalog was not generated by the current compiler and source")
    namespace["check"](schemas, compiled)
    return {"catalog": catalog, "catalog_compiler_sha256": sha256(compiler_payload), "catalog_digest": catalog_digest, "catalog_payload": catalog_payload, "catalog_type": catalog_type, "core": core, "core_digest": core_digest, "service_ready": False, "test_only": catalog_type != "production"}
def load_core(root: Path, inputs: dict):
    source, launcher = root / "scripts/generate_daemon_core_build.py", root / "scripts/run_daemon_core_build.py"; source_payload, launcher_payload = captured(source), captured(launcher); records = {item["path"]: item for item in inputs["core"]["source_closure"]["files"]}
    if records[source.relative_to(root).as_posix()]["sha256"] != sha256(source_payload) or records[launcher.relative_to(root).as_posix()]["sha256"] != sha256(launcher_payload): raise ValueError("Core generator source record mismatch")
    namespace = {"__name__": "daemon_core_build_dependency", "__file__": str(source), "_EXECUTED_SOURCE": source_payload, "_EXECUTED_LAUNCHER_SOURCE": launcher_payload}; exec(compile(source_payload, str(source), "exec"), namespace); return types.SimpleNamespace(**namespace)
def command_bytes(payload: bytes):
    if len(payload) < 32 or struct.unpack_from("<I", payload)[0] != 0xfeedfacf: raise ValueError("invalid Mach-O artifact")
    count, length = struct.unpack_from("<II", payload, 16); end, offset = 32 + length, 32; commands = bytearray(payload[32:end])
    for _ in range(count):
        command, size = struct.unpack_from("<II", payload, offset); relative = offset - 32
        if command == 0x1B: commands[relative + 8:relative + 24] = bytes(16)
        elif command == 0x1D: commands[relative + 8:relative + 16] = bytes(8)
        elif command == 0x19 and payload[offset + 8:offset + 24].rstrip(b"\0") == b"__LINKEDIT": commands[relative + 32:relative + 40] = bytes(8); commands[relative + 48:relative + 56] = bytes(8)
        offset += size
    if offset != end: raise ValueError("invalid Mach-O load command table")
    return bytes(payload[:32] + commands)
def signing_metadata(payload: bytes):
    tool = Path("/usr/bin/codesign"); before = captured(tool, 0o755); environment = {"PATH": "/usr/bin:/bin", "LANG": "C", "LC_ALL": "C"}; fields = {}
    with tempfile.TemporaryDirectory() as directory:
        path = Path(directory) / "turnvector-daemon"; path.write_bytes(payload); os.chmod(path, 0o755); subprocess.run([tool, "--verify", "--strict", path], check=True, capture_output=True, env=environment); result = subprocess.run([tool, "--display", "--verbose=1", path], check=True, capture_output=True, text=True, env=environment)
        for line in (result.stdout + result.stderr).splitlines():
            for name in ("Identifier", "Signature"):
                if line.startswith(f"{name}="):
                    if name in fields: raise ValueError("duplicate code-signing metadata")
                    fields[name] = line.split("=", 1)[1]
    if fields != {"Identifier": IDENTIFIER, "Signature": "adhoc"} or captured(tool, 0o755) != before: raise ValueError("artifact code-signing metadata mismatch")
    return {"identifier": fields["Identifier"], "kind": fields["Signature"], "tool_sha256": sha256(before)}
def codesign(path: Path):
    tool = Path("/usr/bin/codesign"); before = captured(tool, 0o755); subprocess.run([tool, "--force", "--sign", "-", "--identifier", IDENTIFIER, "--timestamp=none", path], check=True, capture_output=True)
    if captured(tool, 0o755) != before: raise ValueError("codesign tool changed during binding")
    return signing_metadata(captured(path))
def build_artifact(root: Path, inputs: dict, destination: Path):
    core = load_core(root, inputs); core.check(root, root / "schemas"); before = core.tree_identity(root); paths = core.resolve_tools(root)
    with tempfile.TemporaryDirectory() as directory:
        temporary, snapshot = Path(directory).resolve(), Path(directory).resolve() / "source"; shutil.copytree(root, snapshot, symlinks=True, ignore=lambda _path, names: sorted(set(names) & core.IGNORED))
        if core.tree_identity(root) != before or core.tree_identity(snapshot) != before: raise ValueError("source tree changed while freezing binding inputs")
        placeholder = temporary / "catalog-placeholder.bin"; placeholder.write_bytes(bytes(FRAME_BYTES)); flags = lambda sdk: [f'-Clinker={paths["clang"]}', "-Clink-arg=-isysroot", f"-Clink-arg={sdk}", "-Clink-arg=-t", f"-Clink-arg=-Wl,-sectcreate,__DATA_CONST,__tvcatalog,{placeholder}"]
        env = {"PATH": ":".join(dict.fromkeys(str(paths[name].parent) for name in ("cargo", "rustc", "clang", "ld"))) + ":/usr/bin:/bin", "HOME": str(temporary / "home"), "CARGO_HOME": str(temporary / "cargo-home"), "CARGO_TARGET_DIR": str(temporary / "discovery"), "CARGO_INCREMENTAL": "0", "RUSTC": str(paths["rustc"]), "CARGO_ENCODED_RUSTFLAGS": "\x1f".join(flags(paths["sdk"])), "DEVELOPER_DIR": str(paths["developer"]), "SDKROOT": str(paths["sdk"]), "MACOSX_DEPLOYMENT_TARGET": "11.0", "LANG": "C", "LC_ALL": "C"}; cargo = str(paths["cargo"]); common = ["--offline", "--locked", "--manifest-path", str(snapshot / "Cargo.toml"), "--release", "--package", "turnvector-daemon", "--bin", "turnvector-daemon"]
        trace = core.run([cargo, "build", *common], Path("/"), env, True); linked = core.sdk_link_inputs(trace, paths["sdk"]); expected = inputs["core"]["toolchain"]["native_link"]["link_inputs"]
        if core.tree_artifact(paths["sdk"], "sdk-link-inputs", linked) != expected: raise ValueError("binding linker inputs differ from the Core descriptor")
        frozen_sdk = temporary / "frozen-sdk"; frozen = core.freeze_sdk(paths["sdk"], linked, frozen_sdk); env.update({"CARGO_TARGET_DIR": str(temporary / "target"), "CARGO_ENCODED_RUSTFLAGS": "\x1f".join(flags(frozen_sdk)), "SDKROOT": str(frozen_sdk)}); final_trace = core.run([cargo, "build", *common], Path("/"), env, True)
        if core.sdk_link_inputs(final_trace, frozen_sdk) != frozen or core.tree_identity(snapshot) != before or core.tree_identity(root) != before: raise ValueError("binding build inputs changed")
        current_tools = core.toolchain(paths, root); current_tools["native_link"]["link_inputs"] = core.tree_artifact(paths["sdk"], "sdk-link-inputs", linked)
        if current_tools != inputs["core"]["toolchain"]: raise ValueError("binding toolchain changed")
        artifact = temporary / "target/release/turnvector-daemon"; original = artifact.read_bytes(); baseline = core.macho_text(artifact, inputs["core"]["toolchain"]["rustc"]["host"])
        if baseline != inputs["core"]["section_identities"]["executable_text"]: raise ValueError("binding artifact does not match the Core executable identity")
        frame = build_frame(inputs["catalog_payload"], inputs["core_digest"], inputs["catalog_digest"], inputs["catalog_type"], inputs["test_only"]); section = baseline["catalog_section"]; handle = os.open(artifact, os.O_WRONLY | os.O_NOFOLLOW); os.pwrite(handle, frame, section["file_offset"]); os.fsync(handle); os.close(handle); signing = codesign(artifact); final = artifact.read_bytes()
        if final[section["file_offset"]:section["file_offset"] + FRAME_BYTES] != frame or command_bytes(original) != command_bytes(final): raise ValueError("binding changed bytes outside the permitted signing layout")
        zeroed = bytearray(final); zeroed[section["file_offset"]:section["file_offset"] + FRAME_BYTES] = bytes(FRAME_BYTES); inspect = temporary / "inspect"; inspect.write_bytes(zeroed); bound = core.macho_text(inspect, baseline["architecture"]); normalized = json.loads(json.dumps(bound)); normalized["loader_commands"]["normalized_sha256"] = baseline["loader_commands"]["normalized_sha256"]; normalized["sha256"] = baseline["sha256"]
        if normalized != baseline: raise ValueError("binding changed a Core executable semantic outside the permitted signing layout")
        destination.parent.mkdir(parents=True, exist_ok=True); shutil.copy2(artifact, destination); os.chmod(destination, 0o755); return baseline, bound, signing
def descriptor_for(root: Path, catalog_source: Path, allow_fixture: bool, artifact: Path):
    inputs = load_inputs(root, catalog_source, allow_fixture); baseline, bound, signing = build_artifact(root, inputs, artifact); frame = parse_frame(captured(artifact)[baseline["catalog_section"]["file_offset"]:baseline["catalog_section"]["file_offset"] + FRAME_BYTES])[0]
    if load_inputs(root, catalog_source, allow_fixture) != inputs: raise ValueError("binding inputs changed")
    generator = {"binder_sha256": sha256(_EXECUTED_SOURCE), "catalog_compiler_sha256": inputs["catalog_compiler_sha256"], "launcher_sha256": sha256(_EXECUTED_LAUNCHER_SOURCE), "protocol": "external_single_read_captured_source_v1"}; identities = {"daemon_core_build": inputs["core_digest"], "outer_daemon_build": outer_identity(inputs["core_digest"], inputs["catalog_digest"]), "runtime_overhead_catalog": inputs["catalog_digest"]}
    return {"catalog": {"canonical_payload_bytes": len(inputs["catalog_payload"]), "catalog_type": inputs["catalog_type"], "capacity": inputs["core"]["catalog"]["capacity"], "frame": inputs["core"]["catalog"]["frame"], "lookup": inputs["core"]["catalog"]["lookup"]}, "daemon_build_schema_version": 1, "embedding": {"artifact": "turnvector-daemon", "format": "mach_o_64", "section": baseline["catalog_section"], "service_ready": inputs["service_ready"], "target": "test_only" if inputs["test_only"] else "production", "tuple": frame}, "executable_provenance": {"allowed_loader_delta": ["uuid_payload", "code_signature_range", "linkedit_mapping_sizes"], "architecture": baseline["architecture"], "bound_loader_normalized_sha256": bound["loader_commands"]["normalized_sha256"], "code_sections": baseline["sections"], "core_executable_identity": baseline["sha256"], "signing": signing}, "generator_execution": generator, "identities": identities}
def lock_for(payload: bytes, descriptor: dict): return {"algorithm": "sha256", "descriptor": DESCRIPTOR, "descriptor_sha256": sha256(payload), "digest": descriptor["identities"]["outer_daemon_build"], "domain": OUTER_DOMAIN[:-1].decode(), "hash_schema_version": 1, "inputs": ["daemon_core_build", "runtime_overhead_catalog"], "kind": "outer_daemon_build", "preimage": "utf8(domain)||0x00||u32be(hash_schema_version)||core_digest_bytes||catalog_digest_bytes"}
def publish(output: Path, pairs):
    output.mkdir(parents=True, exist_ok=True); directory = os.open(output, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW); temporary = f".daemon-build.{os.urandom(16).hex()}"; os.mkdir(temporary, 0o700, dir_fd=directory)
    try:
        if not os.path.samestat(os.stat(output, follow_symlinks=False), os.fstat(directory)): raise ValueError("daemon build output directory changed")
        for name, payload in pairs:
            descriptor = os.open(f"{temporary}/{name}", os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600, dir_fd=directory)
            with os.fdopen(descriptor, "wb") as handle: handle.write(payload); os.fchmod(handle.fileno(), 0o644)
        for name, _payload in pairs: os.replace(f"{temporary}/{name}", name, src_dir_fd=directory, dst_dir_fd=directory)
        if not os.path.samestat(os.stat(output, follow_symlinks=False), os.fstat(directory)): raise ValueError("daemon build output directory changed")
    finally:
        for name, _payload in pairs:
            with contextlib.suppress(FileNotFoundError): os.unlink(f"{temporary}/{name}", dir_fd=directory)
        os.rmdir(temporary, dir_fd=directory); os.close(directory)
def output_pair(output: Path):
    directory = os.open(output, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW); result = []; identity = lambda value: (value.st_dev, value.st_ino, value.st_mode, value.st_size, value.st_mtime_ns, value.st_ctime_ns)
    try:
        for name in (DESCRIPTOR, LOCK):
            descriptor = os.open(name, os.O_RDONLY | os.O_NOFOLLOW, dir_fd=directory)
            with os.fdopen(descriptor, "rb") as handle: before = os.fstat(handle.fileno()); payload = handle.read(); current = os.stat(name, dir_fd=directory, follow_symlinks=False)
            if not stat.S_ISREG(before.st_mode) or stat.S_IMODE(before.st_mode) != 0o644 or identity(before) != identity(current): raise ValueError("daemon build output changed or has an invalid mode")
            result.append(payload)
        if not os.path.samestat(os.stat(output, follow_symlinks=False), os.fstat(directory)): raise ValueError("daemon build output directory changed")
        return tuple(result)
    finally: os.close(directory)
def publish_artifact(source: Path, destination: Path):
    payload = captured(source); destination.parent.mkdir(parents=True, exist_ok=True); directory = os.open(destination.parent, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW); temporary = f".{destination.name}.{os.urandom(16).hex()}"
    try:
        if not os.path.samestat(os.stat(destination.parent, follow_symlinks=False), os.fstat(directory)): raise ValueError("artifact output directory changed")
        descriptor = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o700, dir_fd=directory)
        with os.fdopen(descriptor, "wb") as handle: handle.write(payload); os.fchmod(handle.fileno(), 0o755); os.fsync(handle.fileno())
        os.replace(temporary, destination.name, src_dir_fd=directory, dst_dir_fd=directory)
        if not os.path.samestat(os.stat(destination.parent, follow_symlinks=False), os.fstat(directory)): raise ValueError("artifact output directory changed")
    finally:
        with contextlib.suppress(FileNotFoundError): os.unlink(temporary, dir_fd=directory)
        os.close(directory)
    if captured(destination) != payload: raise ValueError("published artifact changed")
def verify_artifact(root: Path, catalog_source: Path, artifact: Path, descriptor: dict, allow_fixture: bool):
    before = captured(artifact); signing = signing_metadata(before); payload = captured(artifact)
    if payload != before: raise ValueError("bound artifact changed during verification")
    inputs = load_inputs(root, catalog_source, allow_fixture); section = descriptor["embedding"]["section"]; header, catalog, _index = parse_frame(payload[section["file_offset"]:section["file_offset"] + FRAME_BYTES]); identities = {"daemon_core_build": inputs["core_digest"], "outer_daemon_build": outer_identity(inputs["core_digest"], inputs["catalog_digest"]), "runtime_overhead_catalog": inputs["catalog_digest"]}
    if header != descriptor["embedding"]["tuple"] or descriptor["identities"] != identities or canonical(catalog) != inputs["catalog_payload"]: raise ValueError("bound artifact tuple mismatch")
    core = load_core(root, inputs); baseline = inputs["core"]["section_identities"]["executable_text"]; zeroed = bytearray(payload); zeroed[section["file_offset"]:section["file_offset"] + FRAME_BYTES] = bytes(FRAME_BYTES)
    with tempfile.TemporaryDirectory() as directory:
        path = Path(directory) / "inspect"; path.write_bytes(zeroed); inspected = core.macho_text(path, descriptor["executable_provenance"]["architecture"])
    normalized = json.loads(json.dumps(inspected)); normalized["loader_commands"]["normalized_sha256"] = baseline["loader_commands"]["normalized_sha256"]; normalized["sha256"] = baseline["sha256"]; provenance = descriptor["executable_provenance"]
    if normalized != baseline or provenance["architecture"] != baseline["architecture"] or provenance["code_sections"] != baseline["sections"] or provenance["core_executable_identity"] != baseline["sha256"] or provenance["bound_loader_normalized_sha256"] != inspected["loader_commands"]["normalized_sha256"] or section != baseline["catalog_section"] or provenance["signing"] != signing: raise ValueError("bound artifact executable provenance mismatch")
def run_mode(root: Path, catalog_source: Path, output: Path, allow_fixture: bool, artifact: Path, checking: bool):
    with tempfile.TemporaryDirectory() as directory:
        root = root.resolve(); built = Path(directory) / "turnvector-daemon"; descriptor = descriptor_for(root, catalog_source, allow_fixture, built); verify_artifact(root, catalog_source, built, descriptor, allow_fixture); payload = canonical(descriptor); lock = canonical(lock_for(payload, descriptor)); pair = (payload, lock)
        if checking:
            if output_pair(output) != pair: raise ValueError("daemon build output does not match current inputs")
        else: publish(output, tuple(zip((DESCRIPTOR, LOCK), pair)))
        if output_pair(output) != pair: raise ValueError("daemon build output changed")
        if artifact is not None: publish_artifact(built, artifact); verify_artifact(root, catalog_source, artifact, descriptor, allow_fixture)
        verify_artifact(root, catalog_source, built, descriptor, allow_fixture)
def main():
    parser = argparse.ArgumentParser(); parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[1]); parser.add_argument("--catalog-source", type=Path, required=True); parser.add_argument("--allow-test-fixture", action="store_true"); parser.add_argument("--artifact", type=Path); mode = parser.add_mutually_exclusive_group(required=True); mode.add_argument("--output", type=Path); mode.add_argument("--check", type=Path); args = parser.parse_args()
    try: run_mode(args.root, args.catalog_source, args.check or args.output, args.allow_test_fixture, args.artifact, args.check is not None)
    except (KeyError, OSError, ValueError, json.JSONDecodeError, subprocess.SubprocessError, struct.error) as error: parser.exit(1, f"runtime overhead binding error: {error}\n")
if __name__ == "__main__": main()
