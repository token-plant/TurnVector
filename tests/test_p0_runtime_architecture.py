import ast, copy, hashlib
import json
import re
import unittest
from collections import Counter
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
MANIFEST = ROOT / "schemas/p0-runtime-architecture-v1.jsonl"
MANIFEST_SHA256 = "5d72f57bd0ba58fcd197508d06142ee4b3b2fbbbe54edb5d4d8227beef3ba5c2"
ROW = re.compile(r"([A-Z]+)(\d+)([a-z]?)\Z")
KIND_COUNTS = {"meta": 1, "crate": 3, "seam": 2, "adapter": 2, "schema": 9, "module": 41}
MODULE_ORDER = """admission audit_journal backend_contract closure_control core_gate control_store control_plane certification_tooling certification daemon_custody data_plane device_executor event_loop fault_gate fake_backend lifecycle_gate model_descriptor model_registry native_build native_runtime native_turns protocol_authority qualification_core_adapters qualification_lifecycle_adapters qualification_integration qualification_system_adapters release_identity request_book resource_ledger resource_evidence resource_governor residency_coordinator runtime_carry runtime_measurement runtime_qualification scheduling_gate scheduler support_ledger transition_coordinator turn_plans volume_qualification""".split()
MODULE_IDS = set(MODULE_ORDER)
IMPLEMENTED = {"certification", "model_descriptor", "model_registry", "request_book", "support_ledger"}
IMPLEMENTED_DEPENDENCIES = {"certification": ["model_registry", "request_book"], "model_descriptor": [], "model_registry": ["model_descriptor"], "request_book": ["model_registry"], "support_ledger": []}
SCHEMAS = {
    "audit_journal_v1": ("S10", "module", "audit_journal"),
    "backend_owned_values_v1": ("E01", "module", "backend_contract"),
    "build_identity_v1": ("B03", "tool", "build_identity"),
    "control_plane_wire_v1": ("P06", "crate", "turnvector-protocol"),
    "control_store_v1": ("S03", "module", "control_store"),
    "core_domain_v1": ("C01", "crate", "turnvector-core"),
    "core_event_transition_v1": ("C05", "crate", "turnvector-core"),
    "data_plane_wire_v1": ("P05", "crate", "turnvector-protocol"),
    "qualification_subject_v1": ("Q01", "module", "qualification_core_adapters"),
}
CORE_OPERATIONS = [("handle", ["CoreEvent"], "CoreTransition")]
BACKEND_OPERATIONS = [
    ("initialize", ["ControlView"], "BackendInitialization"),
    ("describe_model", ["ModelRegistration"], "RawModelDescriptor"),
    ("describe_request", ["RequestDescriptionInput"], "RequestDescription"),
    ("materialize_request", ["RequestDescription", "ResourceReservation"], "MaterializationResult"),
    ("release_request", ["RequestId", "ReleaseReason", "ControlView"], "RequestReleaseResult"),
    ("form_candidates", ["EligibleRequests", "HardConstraints"], "FormationResult"),
    ("execute_turn", ["TurnPlan", "ControlView"], "TurnResult"),
    ("observe_turn_receipt", ["TurnReceipt"], "CostProfileUpdate"),
    ("execute_exclusive", ["ExclusiveOperation", "ControlView"], "ExclusiveResult"),
    ("transition_residency", ["ResidencyOperation", "ControlView"], "ResidencyResult"),
    ("sample_backend_resources", [], "BackendResourceSample"),
    ("shutdown", ["ControlView"], "ShutdownResult"),
]
NATIVE_RUNTIME_OPERATIONS = {"initialize", "describe_model", "describe_request", "materialize_request", "release_request", "form_candidates", "observe_turn_receipt", "transition_residency", "sample_backend_resources", "shutdown"}
MODULE_KEYS = {"adapter_memberships", "compile_target", "consumes_seams", "contribution_rows", "coordinates_seams", "crate", "dependencies", "existing_implementation_sources", "id", "kind", "layer", "operations", "primary_rows", "provides_seams", "rust_module", "source", "status"}
PROTOCOL_ROOT = b"//! Contract-only Protocol crate; DTO and conversion implementation begins at P04.\n"
FIXTURE_ONLY_PATHS = ("crates/turnvector-protocol/Cargo.toml", "crates/turnvector-protocol/src/lib.rs", "crates/turnvector-daemon/src/release_identity.rs")
FROZEN_TOPOLOGY_SHA256 = {
    "Cargo.toml": "d718f6f9dce78fcdebf72931b2d0f03fbfa2cf3772623cc7a544503ccfe9595a",
    "crates/turnvector-core/Cargo.toml": "5a4a358ec3b14544dbfc3a6ee4e8cc1e56674e19d66e210e2249a7c806cd83b6",
    "crates/turnvector-core/src/lib.rs": "6bfb0028a47c707d56eb0ed6bc39055524f8afb12a8898e0a849f9832ddd52e6",
    "crates/turnvector-daemon/Cargo.toml": "1dec90d7c04004eaa25ab4bf3aacea7989a898a24df29629be6c93ad6d203f0a",
    "crates/turnvector-daemon/src/main.rs": "f9e7bb4b97fbe08292028ab4ada11199df3a3629a18899cfb57a46683230622e",
    "tests/test_daemon_core_build.py": "36127c653cd5b43afc21fc6aa6ed1ea012db0b93d18a716466ea512a989a6cdd",
}
PROTOCOL_CARGO = b'''[package]\nname = "turnvector-protocol"\nversion.workspace = true\nedition.workspace = true\nrust-version.workspace = true\nlicense.workspace = true\npublish = false\n\n[lints]\nworkspace = true\n'''


def canonical(value):
    return (json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n").encode()


def unique_object(pairs):
    value = {}
    for key, item in pairs:
        if key in value:
            raise ValueError(f"duplicate JSON key: {key}")
        value[key] = item
    return value


def load_manifest(payload=None):
    lines = (MANIFEST.read_bytes() if payload is None else payload).splitlines(keepends=True)
    if not lines or any(not line.endswith(b"\n") for line in lines):
        raise ValueError("manifest must be nonempty newline-terminated JSONL")
    records = []
    for line in lines:
        record = json.loads(line, object_pairs_hook=unique_object)
        if canonical(record) != line:
            raise ValueError("manifest record is not canonical")
        records.append(record)
    return records


def row_parts(value):
    match = ROW.fullmatch(value)
    if match is None:
        raise ValueError(f"invalid ledger row: {value}")
    return match.group(1), match.group(2), match.group(3)


def expand(specification):
    if "-" not in specification:
        row_parts(specification)
        return [specification]
    first, last = specification.split("-", 1)
    left, right = row_parts(first), row_parts(last)
    if left[0] != right[0]:
        raise ValueError("cross-series row range")
    if left[1] == right[1] and left[2] and right[2]:
        if ord(left[2]) > ord(right[2]):
            raise ValueError("descending suffix range")
        return [f"{left[0]}{left[1]}{chr(value)}" for value in range(ord(left[2]), ord(right[2]) + 1)]
    if left[2] or right[2] or int(left[1]) > int(right[1]):
        raise ValueError("invalid numeric row range")
    width = max(len(left[1]), len(right[1]))
    return [f"{left[0]}{value:0{width}d}" for value in range(int(left[1]), int(right[1]) + 1)]


def expand_all(values):
    return [row for value in values for row in expand(value)]


def operation_tuples(operations):
    for operation in operations:
        if set(operation) != {"inputs", "name", "output"} or not operation["name"] or not operation["output"] or any(not value for value in operation["inputs"]):
            raise ValueError("malformed operation")
    result = [(item["name"], item["inputs"], item["output"]) for item in operations]
    if len({item[0] for item in result}) != len(result):
        raise ValueError("duplicate operation")
    return result


def shell_bytes(module):
    rows = ", ".join(module["primary_rows"])
    return f"//! Contract-only final path for the `{module['id']}` Module; ledger ownership: {rows}.\n".encode()


def read_path(relative, overrides):
    return overrides.get(relative, (ROOT / relative).read_bytes())


def input_custody(source):
    tree = ast.parse(source)
    launcher = None
    for statement in tree.body:
        if isinstance(statement, ast.Assign) and len(statement.targets) == 1 and isinstance(statement.targets[0], ast.Tuple) and isinstance(statement.value, ast.Tuple):
            for target, value in zip(statement.targets[0].elts, statement.value.elts):
                if isinstance(target, ast.Name) and target.id == "LAUNCHER" and isinstance(value, ast.Constant) and isinstance(value.value, str):
                    launcher = value.value
    if launcher != "scripts/run_daemon_core_build.py":
        raise ValueError("daemon-core launcher drift")

    def literal_tuple(node):
        if not isinstance(node, ast.Tuple):
            raise ValueError("input custody must use literal tuples")
        values = []
        for item in node.elts:
            if isinstance(item, ast.Constant) and isinstance(item.value, str):
                values.append(item.value)
            elif isinstance(item, ast.Name) and item.id == "LAUNCHER":
                values.append(launcher)
            else:
                raise ValueError("input custody contains a nonliteral path")
        return tuple(values)

    values, writes = {}, Counter()
    for statement in tree.body:
        target = statement.targets[0] if isinstance(statement, ast.Assign) and len(statement.targets) == 1 else statement.target if isinstance(statement, ast.AugAssign) else None
        if not isinstance(target, ast.Name) or target.id not in {"INPUTS", "FIXTURE_ONLY_INPUTS"}:
            continue
        name = target.id; writes[name] += 1
        if isinstance(statement, ast.Assign) and name not in values:
            values[name] = literal_tuple(statement.value)
        elif isinstance(statement, ast.AugAssign) and name == "INPUTS" and isinstance(statement.op, ast.Add) and name in values:
            values[name] += literal_tuple(statement.value)
        else:
            raise ValueError("invalid input custody writer")
    stores = Counter(node.id for node in ast.walk(tree) if isinstance(node, ast.Name) and isinstance(node.ctx, ast.Store) and node.id in {"INPUTS", "FIXTURE_ONLY_INPUTS"})
    if stores != writes or set(values) != {"INPUTS", "FIXTURE_ONLY_INPUTS"} or len(values["INPUTS"]) != len(set(values["INPUTS"])):
        raise ValueError("input custody is incomplete, duplicated, or indirect")
    return values["INPUTS"], values["FIXTURE_ONLY_INPUTS"]


def validate(records, overrides=None):
    overrides = overrides or {}
    if any(hashlib.sha256(read_path(path, overrides)).hexdigest() != digest for path, digest in FROZEN_TOPOLOGY_SHA256.items()):
        raise ValueError("frozen architecture topology drift")
    if hashlib.sha256(b"".join(canonical(record) for record in records)).hexdigest() != MANIFEST_SHA256:
        raise ValueError("architecture manifest drift")
    if Counter(record.get("kind") for record in records) != Counter(KIND_COUNTS):
        raise ValueError("wrong record cardinality")
    expected_order = ["meta"] + [kind for kind in ("crate", "seam", "adapter", "schema", "module") for _ in range(KIND_COUNTS[kind])]
    if [record["kind"] for record in records] != expected_order:
        raise ValueError("wrong record order")
    grouped = {kind: [record for record in records if record["kind"] == kind] for kind in KIND_COUNTS}
    if any([record["id"] for record in grouped[kind]] != sorted(record["id"] for record in grouped[kind]) for kind in ("crate", "seam", "adapter")) or [record["id"] for record in grouped["module"]] != MODULE_ORDER:
        raise ValueError("records are not ordered by id")
    if [record["id"] for record in grouped["schema"]] != ["audit_journal_v1", "backend_owned_values_v1", "build_identity_v1", "control_plane_wire_v1", "core_domain_v1", "core_event_transition_v1", "control_store_v1", "data_plane_wire_v1", "qualification_subject_v1"]:
        raise ValueError("schema records are not in canonical order")
    identifiers = [record["id"] for record in records if "id" in record]
    if len(identifiers) != len(set(identifiers)):
        raise ValueError("duplicate record id")
    meta = grouped["meta"][0]
    if (meta["format_version"], meta["module_count"], meta["primary_ledger_row_count"], meta["schema_family_count"], meta["module_visibility_default"], meta["status_scope"]) != (2, 41, 193, 9, "private", "declared_source_path_only"):
        raise ValueError("wrong architecture metadata")

    crates = {record["id"]: record for record in grouped["crate"]}
    if set(crates) != {"turnvector-core", "turnvector-daemon", "turnvector-protocol"}:
        raise ValueError("wrong crate set")
    if crates["turnvector-core"]["dependencies"] or crates["turnvector-core"]["runtime_linkage"] != "production":
        raise ValueError("Core crate dependency drift")
    if crates["turnvector-daemon"]["dependencies"] != [{"activation": "current_tree", "id": "turnvector-core", "status": "active"}, {"activation": "P04", "id": "turnvector-protocol", "status": "reserved"}]:
        raise ValueError("daemon crate dependency drift")
    if crates["turnvector-protocol"]["dependencies"] != [{"activation": "P04", "id": "turnvector-core", "status": "reserved"}] or crates["turnvector-protocol"]["status"] != "contract_only":
        raise ValueError("Protocol crate dependency drift")

    modules = {record["id"]: record for record in grouped["module"]}
    if set(modules) != MODULE_IDS or any(set(module) != MODULE_KEYS for module in modules.values()):
        raise ValueError("wrong module inventory or shape")
    if {name for name, module in modules.items() if module["status"] == "implemented"} != IMPLEMENTED or any(module["status"] not in {"implemented", "contract_only"} for module in modules.values()):
        raise ValueError("implemented status drift")
    if {name: modules[name]["dependencies"] for name in IMPLEMENTED} != IMPLEMENTED_DEPENDENCIES:
        raise ValueError("implemented dependency drift")
    if Counter(module["compile_target"] for module in modules.values()) != Counter({"production": 32, "test": 8, "offline_tool": 1}):
        raise ValueError("compile-target drift")
    edge_count = 0
    for module in modules.values():
        if module["crate"] not in {"turnvector-core", "turnvector-daemon"} or module["source"] != f"crates/{module['crate']}/src/{module['rust_module']}.rs" or not read_path(module["source"], overrides) or any(not read_path(source, overrides) for source in module["existing_implementation_sources"]):
            raise ValueError("module source or crate drift")
        operation_tuples(module["operations"])
        for dependency in module["dependencies"]:
            edge_count += 1
            if dependency not in modules or module["layer"] <= modules[dependency]["layer"]:
                raise ValueError("cyclic or invalid module dependency")
        if set(expand_all(module["contribution_rows"])) - set(expand_all(meta["ledger_series"])):
            raise ValueError("unknown contribution row")
    if edge_count != 77 or max(module["layer"] for module in modules.values()) != 13:
        raise ValueError("module graph drift")

    ledger = expand_all(meta["ledger_series"])
    if len(ledger) != 193 or len(set(ledger)) != 193:
        raise ValueError("ledger cardinality drift")
    owners = Counter(row for module in modules.values() for row in expand_all(module["primary_rows"]))
    dynamic = meta["dynamic_primary_rows"]
    if dynamic != [{"owner_selector": "affected_module_owner", "row": "Q15"}]:
        raise ValueError("dynamic owner drift")
    owners.update(item["row"] for item in dynamic)
    if owners != Counter({row: 1 for row in ledger}):
        raise ValueError("primary ownership is not exact")

    schemas = {record["id"]: record for record in grouped["schema"]}
    observed_schemas = {name: (record["definition_row"], record["owner"]["kind"], record["owner"]["id"]) for name, record in schemas.items()}
    if observed_schemas != SCHEMAS:
        raise ValueError("schema ownership drift")
    for record in schemas.values():
        owner = record["owner"]
        if owner["kind"] == "module" and owner["id"] not in modules or owner["kind"] == "crate" and owner["id"] not in crates or owner["kind"] == "tool" and owner["id"] != "build_identity":
            raise ValueError("unresolved schema owner")

    seams = {record["id"]: record for record in grouped["seam"]}
    if set(seams) != {"backend_interface_v1", "core_transition_v1"}:
        raise ValueError("wrong seam set")
    backend, core = seams["backend_interface_v1"], seams["core_transition_v1"]
    if operation_tuples(backend["operations"]) != BACKEND_OPERATIONS or operation_tuples(core["operations"]) != CORE_OPERATIONS:
        raise ValueError("seam operation drift")
    if (backend["provider"], backend["consumers"], backend["adapters"], backend["visibility"]) != ({"id": "backend_contract", "kind": "module"}, ["device_executor"], ["fake_backend_adapter", "native_backend_adapter"], "crate_private"):
        raise ValueError("Backend seam drift")
    if (core["provider"], core["consumers"], core["coordination_owner"], core["visibility"]) != ({"id": "turnvector-core", "kind": "crate"}, ["event_loop"], "transition_coordinator", "public"):
        raise ValueError("Core seam drift")

    adapters = {record["id"]: record for record in grouped["adapter"]}
    all_backend_operations = {item[0] for item in BACKEND_OPERATIONS}
    fake = adapters["fake_backend_adapter"]["operation_owners"]
    native = adapters["native_backend_adapter"]["operation_owners"]
    if fake != [{"module": "fake_backend", "operations": [item[0] for item in BACKEND_OPERATIONS]}]:
        raise ValueError("Fake adapter drift")
    native_map = {item["module"]: set(item["operations"]) for item in native}
    if native_map != {"native_runtime": NATIVE_RUNTIME_OPERATIONS, "native_turns": {"execute_turn", "execute_exclusive"}} or set.union(*native_map.values()) != all_backend_operations or set.intersection(*native_map.values()):
        raise ValueError("Native adapter is incomplete or overlapping")
    for seam in seams.values():
        if seam["provider"]["kind"] == "module" and seam["id"] not in modules[seam["provider"]["id"]]["provides_seams"]:
            raise ValueError("seam provider inverse missing")
        if any(seam["id"] not in modules[consumer]["consumes_seams"] for consumer in seam["consumers"]):
            raise ValueError("seam consumer inverse missing")
    if core["id"] not in modules[core["coordination_owner"]]["coordinates_seams"]:
        raise ValueError("seam coordinator inverse missing")

    contract_modules = [module for module in modules.values() if module["status"] == "contract_only"]
    for module in contract_modules:
        if read_path(module["source"], overrides) != shell_bytes(module):
            raise ValueError(f"contract shell contains implementation: {module['id']}")
    if read_path("crates/turnvector-protocol/src/lib.rs", overrides) != PROTOCOL_ROOT or read_path("crates/turnvector-protocol/Cargo.toml", overrides) != PROTOCOL_CARGO:
        raise ValueError("Protocol crate is not item-free and exact")
    core_source = read_path("crates/turnvector-core/src/lib.rs", overrides).decode()
    daemon_source = read_path("crates/turnvector-daemon/src/main.rs", overrides).decode(); core_lines, daemon_lines = set(core_source.splitlines()), set(daemon_source.splitlines())
    for module in modules.values():
        declaration = f"mod {module['rust_module']};"
        if module["compile_target"] == "production" and declaration not in (core_lines if module["crate"] == "turnvector-core" else daemon_lines):
            raise ValueError("production module is not private and declared")
        if module["compile_target"] == "test" and f"#[cfg(test)]\n{declaration}" not in daemon_source:
            raise ValueError("test module is not private and gated")
    owner_reexport = any(re.search(rf"(?ms)^\s*pub\s+use\s+[^;]*\b{re.escape(module['rust_module'])}\b", core_source + daemon_source) for module in modules.values())
    if any(f"pub mod {module['rust_module']};" in core_source + daemon_source for module in modules.values()) or owner_reexport or "#[path" in core_source + daemon_source or "include!(" in core_source + daemon_source or "mod release_identity;" in daemon_source or "turnvector-protocol" in read_path("crates/turnvector-daemon/Cargo.toml", overrides).decode():
        raise ValueError("public owner or early offline-tool linkage")
    workspace = read_path("Cargo.toml", overrides).decode()
    if 'members = ["crates/turnvector-core", "crates/turnvector-daemon", "crates/turnvector-protocol"]' not in workspace:
        raise ValueError("Protocol crate is not a workspace member")
    fixture_test = read_path("tests/test_daemon_core_build.py", overrides).decode(); production_inputs, fixture_inputs = input_custody(fixture_test)
    production_paths = {module["source"] for module in contract_modules if module["compile_target"] == "production"}; test_paths = {module["source"] for module in modules.values() if module["compile_target"] == "test"}
    if not production_paths <= set(production_inputs) or set(production_inputs) & (test_paths | set(FIXTURE_ONLY_PATHS)):
        raise ValueError("production shell missing from daemon-core inputs")
    if fixture_inputs != FIXTURE_ONLY_PATHS:
        raise ValueError("unlinked contract missing from exact fixture custody")
    for path, marker in (("CONTEXT.md", "**Architecture Contract Baseline**:"), ("docs/plans/2026-08-16-p0-runtime-implementation.md", "ADR 0048 establishes"), ("docs/plans/2026-08-19-p0-parallel-module-delivery.md", "## Architecture Contract Baseline"), ("docs/adr/0048-freeze-p0-runtime-architecture-contracts.md", "# Freeze P0 Runtime Architecture Contracts Before Behavior")):
        if marker not in read_path(path, overrides).decode():
            raise ValueError("architecture authority amendment missing")


class RuntimeArchitectureTests(unittest.TestCase):
    def test_current_architecture_is_canonical_complete_and_item_free(self):
        validate(load_manifest())

    def test_malformed_duplicate_incomplete_cyclic_public_and_implemented_inputs_fail_closed(self):
        baseline = load_manifest()
        mutations = []
        malformed = copy.deepcopy(baseline); malformed[0]["format_version"] = 99; mutations.append(malformed)
        duplicate = copy.deepcopy(baseline); duplicate.append(copy.deepcopy(duplicate[-1])); mutations.append(duplicate)
        incomplete = [copy.deepcopy(record) for record in baseline if record.get("id") != "admission"]; mutations.append(incomplete)
        cyclic = copy.deepcopy(baseline); next(record for record in cyclic if record.get("id") == "admission")["dependencies"].append("transition_coordinator"); mutations.append(cyclic)
        graph = copy.deepcopy(baseline); next(record for record in graph if record.get("id") == "admission")["dependencies"][0] = "audit_journal"; mutations.append(graph)
        status = copy.deepcopy(baseline); next(record for record in status if record.get("id") == "admission")["status"] = "reserved"; mutations.append(status)
        public = copy.deepcopy(baseline); public[0]["module_visibility_default"] = "public"; mutations.append(public)
        adapter_gap = copy.deepcopy(baseline); next(record for record in adapter_gap if record.get("id") == "fake_backend_adapter")["operation_owners"][0]["operations"].pop(); mutations.append(adapter_gap)
        for records in mutations:
            with self.assertRaises(ValueError):
                validate(records)
        core_cargo = (ROOT / "crates/turnvector-core/Cargo.toml").read_bytes(); core_source = (ROOT / "crates/turnvector-core/src/lib.rs").read_bytes(); daemon_source = (ROOT / "crates/turnvector-daemon/src/main.rs").read_bytes(); fixture_source = (ROOT / "tests/test_daemon_core_build.py").read_bytes(); admission = b"crates/turnvector-core/src/admission.rs"
        missing_input = fixture_source.replace(b'"' + admission + b'", ', b"", 1).replace(b"\nFIXTURE_ONLY_INPUTS", b'\n# "' + admission + b'"\nFIXTURE_ONLY_INPUTS', 1)
        hidden_input = fixture_source.replace(b"\nFIXTURE_ONLY_INPUTS", b'\nINPUTS += ("crates/turnvector-daemon/src/" + "release_identity.rs",)\nFIXTURE_ONLY_INPUTS', 1)
        sources = (("crates/turnvector-core/src/admission.rs", b"fn placeholder() {}\n"), ("crates/turnvector-core/Cargo.toml", core_cargo + b'\n[lib]\npath = "src/admission.rs"\n'), ("crates/turnvector-core/src/lib.rs", core_source.replace(b"mod certification;", b"pub mod certification;")), ("crates/turnvector-core/src/lib.rs", core_source.replace(b"mod admission;", b'#[cfg_attr(all(), path = "certification.rs")]\nmod admission;')), ("crates/turnvector-core/src/lib.rs", core_source + b"\nuse support as owner_alias; pub use owner_alias::*;\n"), ("crates/turnvector-daemon/src/main.rs", daemon_source.replace(b"#[cfg(test)]\nmod fault_gate;", b"#[cfg(not(test))]\nmod fault_gate;")), ("crates/turnvector-daemon/Cargo.toml", (ROOT / "crates/turnvector-daemon/Cargo.toml").read_bytes() + b"turnvector-protocol = { path = \"../turnvector-protocol\" }\n"), ("tests/test_daemon_core_build.py", fixture_source.replace(b"INPUTS += (", b'INPUTS += ("crates/turnvector-daemon/src/fault_gate.rs", ', 1)), ("tests/test_daemon_core_build.py", missing_input), ("tests/test_daemon_core_build.py", hidden_input), ("tests/test_daemon_core_build.py", fixture_source.replace(b"\nFIXTURE_ONLY_INPUTS", b'\nglobals()["INPUTS"] += ("crates/turnvector-daemon/src/release_identity.rs",)\nFIXTURE_ONLY_INPUTS', 1)))
        for path, payload in sources:
            with self.assertRaises(ValueError): validate(baseline, {path: payload})


if __name__ == "__main__":
    unittest.main()
