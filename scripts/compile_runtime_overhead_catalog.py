#!/usr/bin/env -S python3 -I -S -B
import argparse, hashlib, json
from pathlib import Path
CATALOG, LOCK = "runtime-overhead-catalog-v1.json", "runtime-overhead-catalog-v1.lock.json"; CORE, CORE_LOCK = "daemon-core-build-v1.json", "daemon-core-build-v1.lock.json"
DOMAIN = b"turnvector:evidence:runtime-overhead-catalog\0"; TABLE_KINDS = ("lifecycle_operations", "sequenced_events", "local_stale")
SUPPORT = ("describe_model", "describe_request", "materialize_request", "release_request", "form_candidates", "observe_turn_receipt", "sample_backend_resources")
POOLS = ("ordinary", "mandatory_completion", "safety_sampling")
INTERFACE = {"initialize": "startup", "describe_model": "support", "describe_request": "support", "materialize_request": "support", "release_request": "support", "form_candidates": "support", "execute_turn": "turn_result", "observe_turn_receipt": "support",
             "execute_exclusive": "exclusive", "transition_residency": "residency", "sample_backend_resources": "support", "shutdown": "shutdown"}
STATES = {"describe_model": ["pre_ready", "ready", "evidence_degraded_or_recovery"], "describe_request": ["pre_ready", "ready", "evidence_degraded_or_recovery"], "materialize_request": ["ready"], "release_request": ["ready", "evidence_degraded_or_recovery", "drain"],
          "form_candidates": ["ready"], "observe_turn_receipt": ["ready", "evidence_degraded_or_recovery", "drain"], "sample_backend_resources": ["pre_ready", "ready", "evidence_degraded_or_recovery", "drain"]}
EVENTS = {"request_acceptance": "ordinary", "materialization_result": "ordinary", "candidate_formation_result": "ordinary", "receipt": "scheduling_trigger", "plan_rejection": "scheduling_trigger", "support_result": "scheduling_trigger",
          "idle_reentry": "scheduling_trigger", "cancellation": "mandatory_crossing", "disconnect": "mandatory_crossing", "critical": "mandatory_crossing", "device_executor_failure": "mandatory_crossing", "shutdown": "mandatory_crossing"}
STALE_BRANCHES = ("formation_required", "typed_impossible_close")
def canonical(value) -> bytes: return (json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n").encode()
def unique(pairs):
    value = dict(pairs)
    if len(value) != len(pairs): raise ValueError("duplicate JSON key")
    return value
def read_canonical(path: Path):
    payload = path.read_bytes(); value = json.loads(payload, object_pairs_hook=unique)
    if payload != canonical(value): raise ValueError(f"noncanonical JSON: {path.name}")
    return value, payload
def evidence(payload: bytes, domain: bytes = DOMAIN) -> str: return hashlib.sha256(domain + (1).to_bytes(4, "big") + payload).hexdigest()
def exact(value: dict, keys, label: str) -> None:
    if type(value) is not dict or set(value) != set(keys): raise ValueError(f"{label} has missing or unknown keys")
def positive(value, label: str, maximum=(1 << 64) - 1) -> int:
    if type(value) is not int or not 0 < value <= maximum: raise ValueError(f"{label} must be a positive bounded integer")
    return value
def hex_identity(value, label: str) -> str:
    if type(value) is not str or len(value) != 64 or any(character not in "0123456789abcdef" for character in value): raise ValueError(f"{label} must be a lowercase SHA-256 identity")
    return value
def reject_forbidden(value, names) -> None:
    if isinstance(value, dict):
        for key, item in value.items():
            if key in names:
                raise ValueError(f"forbidden Catalog input: {key}")
            reject_forbidden(item, names)
    elif isinstance(value, list):
        for item in value:
            reject_forbidden(item, names)
    elif isinstance(value, str) and any(name in value for name in names):
        raise ValueError("forbidden Catalog input reference")
def bounded_sum(values, label: str) -> int:
    total = 0
    for value in values:
        if type(value) is not int or value < 0 or total + value > (1 << 64) - 1: raise ValueError(f"{label} overflows u64")
        total += value
    return total
def core_identity(directory: Path) -> tuple:
    descriptor, payload = read_canonical(directory / CORE)
    lock, _ = read_canonical(directory / CORE_LOCK)
    exact(lock, {"algorithm", "descriptor", "digest", "domain", "hash_schema_version", "kind", "preimage"}, "Daemon Core Build lock")
    domain = lock["domain"].encode() + b"\0"
    if (lock["algorithm"], lock["descriptor"], lock["domain"], lock["hash_schema_version"], lock["kind"], lock["preimage"]) != (
            "sha256", CORE, "turnvector:evidence:daemon-core-build", 1, "daemon_core_build",
            "utf8(domain)||0x00||u32be(hash_schema_version)||descriptor_bytes") or lock["digest"] != evidence(payload, domain):
        raise ValueError("Daemon Core Build Evidence Hash lock does not match")
    support, catalog, event, cardinality, carry = (descriptor[name] for name in ("support", "catalog", "event_registry", "cardinality_inputs", "prepared_carry"))
    capacity, lookup, work = (catalog[name] for name in ("capacity", "lookup", "worst_case_work")); funding, vector, start, records = (support[name] for name in ("funding_claim", "outstanding_credit_vector", "start_count", "records"))
    exact(catalog, {"schema_version", "capacity", "lookup", "worst_case_work"}, "Core Catalog"); exact(capacity, {"max_canonical_bytes", "max_entries", "max_entry_bytes"}, "Core Catalog capacity"); exact(lookup, {"algorithm", "key_bytes"}, "Core Catalog lookup"); exact(work, {"entry_bytes_validated", "key_bytes_compared", "key_comparisons"}, "Core Catalog work")
    exact(support, {"operations", "pools", "funding_claim", "outstanding_credit_vector", "records", "start_count"}, "Core support"); exact(funding, {"max_claims_per_obligation", "nonempty", "schema_version", "variants"}, "Core funding claim"); exact(vector, {"axes", "credit_encoding", "max_dimensions", "schema_version"}, "Core credit vector")
    exact(start, {"count_encoding", "max_cells", "max_horizons", "max_physical_credits", "schema_version"}, "Core start count"); exact(records, {"active_and_retained", "conditional_obligations", "entitlement_tombstones", "funding_claims", "lifecycle_reserves", "ordinary_claims", "pending_obligations", "total_operation_obligations"}, "Core records"); exact(event, {"mandatory_crossing_max", "max_entries", "max_kinds", "schema_version"}, "Core event registry")
    exact(cardinality, {"encoding", "ingress", "model_registry"}, "Core cardinalities"); exact(cardinality["ingress"], {"connections", "global_active", "global_warming", "per_connection_active", "per_connection_warming"}, "Core ingress cardinalities"); exact(carry, {"mandatory_suballocation_max", "nonborrowable", "safety_suballocation_max", "slots"}, "Core Prepared Carry")
    for value in (*capacity.values(), event["mandatory_crossing_max"], event["max_entries"], event["max_kinds"], carry["mandatory_suballocation_max"], carry["safety_suballocation_max"], funding["max_claims_per_obligation"], vector["max_dimensions"], *records.values(), start["max_cells"], start["max_horizons"], start["max_physical_credits"]): positive(value, "Core Catalog capacity", (1 << 32) - 1)
    comparisons = capacity["max_entries"].bit_length(); schemas = (descriptor.get("descriptor_schema_version"), catalog["schema_version"], event["schema_version"], funding["schema_version"], vector["schema_version"], start["schema_version"])
    if (any(type(version) is not int or version != 1 for version in schemas) or cardinality["encoding"] != "u16" or support["operations"] != list(SUPPORT) or support["pools"] != list(POOLS) or
            vector["axes"] != ["operation", "pool", "horizon"] or vector["credit_encoding"] != "u16" or start["count_encoding"] != "u32" or
            funding["nonempty"] is not True or funding["variants"] != ["ordinary_reservation", "admission_initial", "entitlement_vector", "lifecycle_reserve"] or
            lookup != {"algorithm": "sorted_sha256_key_binary_search_then_canonical_equality_v1", "key_bytes": 32} or work != {"entry_bytes_validated": capacity["max_entry_bytes"], "key_bytes_compared": 32 * comparisons, "key_comparisons": comparisons} or
            carry["nonborrowable"] is not True or type(carry["slots"]) is not int or carry["slots"] != 1):
        raise ValueError("unsupported Daemon Core Build Catalog contract")
    ingress = cardinality["ingress"]
    for name, value in {**ingress, "model_registry": descriptor["cardinality_inputs"]["model_registry"]}.items():
        positive(value, name, (1 << 16) - 1)
    if ingress["global_warming"] > ingress["global_active"] or ingress["global_active"] > ingress["connections"] * ingress["per_connection_active"] or ingress["global_warming"] > ingress["connections"] * ingress["per_connection_warming"]:
        raise ValueError("Ingress cardinalities exceed their connection backing")
    return descriptor, lock["digest"]
def indexed_cases(table: dict, kind: str) -> dict:
    cases = {case["id"]: case for case in table["cases"]}
    if len(cases) != len(table["cases"]) or not cases: raise ValueError(f"duplicate or missing {kind} evidence case")
    return cases
def validate_table(table: dict, kind: str, catalog_type: str, identity: str, event_limit: dict) -> dict:
    common = {"schema_version", "table_kind", "evidence_type", "daemon_core_build_identity", "provenance", "cases"}
    extras = {"operation_classifications", "lifecycle_states"} if kind == "lifecycle_operations" else ({"event_classes"} if kind == "sequenced_events" else set())
    exact(table, common | extras, f"{kind} table")
    if type(table["schema_version"]) is not int or table["schema_version"] != 1 or table["table_kind"] != kind or table["evidence_type"] != catalog_type or table["daemon_core_build_identity"] != identity:
        raise ValueError(f"{kind} table type or Core binding mismatch")
    exact(table["provenance"], {"kind", "artifact_sha256"}, f"{kind} provenance")
    hex_identity(table["provenance"]["artifact_sha256"], f"{kind} provenance")
    expected_provenance = {"production": "lifecycle_evidence_artifact", "test_fixture_nonproduction": "synthetic_fixture"}[catalog_type]
    if table["provenance"]["kind"] != expected_provenance: raise ValueError("fixture and production evidence types cannot substitute")
    cases = indexed_cases(table, kind)
    if kind == "lifecycle_operations":
        if table["operation_classifications"] != INTERFACE or table["lifecycle_states"] != STATES:
            raise ValueError("unclassified Backend operation or lifecycle state")
        for case in cases.values():
            exact(case, {"id", "complete_envelope_us", "direct_call_us"}, "lifecycle case")
            if set(case["complete_envelope_us"]) != set(SUPPORT) or set(case["direct_call_us"]) != set(SUPPORT):
                raise ValueError("unclassified support operation")
            for operation in SUPPORT:
                complete = positive(case["complete_envelope_us"][operation], f"{operation} envelope")
                if positive(case["direct_call_us"][operation], f"{operation} direct call") > complete:
                    raise ValueError("direct-call bound exceeds its complete envelope")
    elif kind == "sequenced_events":
        if table["event_classes"] != EVENTS:
            raise ValueError("unclassified sequenced event")
        if len(EVENTS) > event_limit["max_kinds"] or len(EVENTS) * len(cases) > event_limit["max_entries"]:
            raise ValueError("sequenced event registry exceeds Core capacity")
        for case in cases.values():
            exact(case, {"id", "complete_envelope_us", "max_per_cut", "mandatory_crossing_allowance"}, "event case")
            if set(case["complete_envelope_us"]) != set(EVENTS) or set(case["max_per_cut"]) != set(EVENTS):
                raise ValueError("unclassified sequenced event case")
            for event in EVENTS:
                positive(case["complete_envelope_us"][event], f"{event} envelope")
                positive(case["max_per_cut"][event], f"{event} count", (1 << 32) - 1)
            positive(case["mandatory_crossing_allowance"], "mandatory crossing allowance", event_limit["mandatory_crossing_max"])
    else:
        for case in cases.values():
            exact(case, {"id", "complete_envelope_us", "successor_support_ceiling_us", "branches"}, "local-stale case")
            if set(case["branches"]) != set(STALE_BRANCHES):
                raise ValueError("unclassified local-stale branch")
            complete = positive(case["complete_envelope_us"], "local-stale envelope")
            positive(case["successor_support_ceiling_us"], "local-stale successor ceiling")
            if any(positive(value, "local-stale branch") > complete for value in case["branches"].values()):
                raise ValueError("local-stale branch exceeds its complete envelope")
    return cases
def count_bounds(core: dict, closure: dict, horizons: list) -> tuple:
    exact(closure, {"max_batch_members", "max_consecutive_turns_per_request", "max_safety_samples",
                    "max_sequential_request_churn", "max_terminal_membership_changes",
                    "optional_ordinary_starts_per_operation"}, "finite request closure")
    for key, value in closure.items():
        positive(value, key, (1 << 32) - 1)
    support, ingress, registry = core["support"], core["cardinality_inputs"]["ingress"], core["cardinality_inputs"]["model_registry"]
    accepted, warming = ingress["global_active"], ingress["global_warming"]
    claim_scope = {operation: accepted if operation == "form_candidates" else closure["max_batch_members"] if operation == "observe_turn_receipt" else 1 for operation in SUPPORT}
    if max(claim_scope.values()) > support["funding_claim"]["max_claims_per_obligation"]: raise ValueError("affected-request claims exceed the funding-claim bound")
    churn = closure["max_sequential_request_churn"]; population = accepted + churn; turns = population * closure["max_consecutive_turns_per_request"]
    mandatory = {"describe_model": registry, "describe_request": max(population, warming + registry),
                 "materialize_request": population, "release_request": population,
                 "form_candidates": population + turns + closure["max_terminal_membership_changes"],
                 "observe_turn_receipt": turns, "sample_backend_resources": 1}
    rows = []
    for operation in SUPPORT:
        for pool in POOLS:
            count = (closure["optional_ordinary_starts_per_operation"] if pool == "ordinary" else
                     mandatory[operation] if pool == "mandatory_completion" else
                     closure["max_safety_samples"] if operation == "sample_backend_resources" else 1)
            positive(count, f"{operation}/{pool} count", (1 << 32) - 1)
            rows.extend({"horizon_us": horizon, "max_starts": count, "operation": operation, "pool": pool} for horizon in horizons)
    vector = []
    for operation, count in (("form_candidates", closure["max_consecutive_turns_per_request"] + 1),
                             ("observe_turn_receipt", closure["max_consecutive_turns_per_request"])):
        positive(count, f"{operation} vector", (1 << 16) - 1)
        vector.extend({"horizon_us": horizon, "max_outstanding": count,
                       "operation": operation, "pool": "mandatory_completion"} for horizon in horizons)
    if len(rows) > support["start_count"]["max_cells"] or len(vector) > support["outstanding_credit_vector"]["max_dimensions"]:
        raise ValueError("count or vector dimensions exceed Core capacity")
    return rows, vector, claim_scope
def table_case(cases: dict, name, label: str) -> dict:
    if name not in cases: raise ValueError(f"unknown {label} evidence case")
    return cases[name]
def compile_entry(core: dict, raw: dict, horizons: list, references: dict, cases: dict) -> dict:
    exact(raw, {"backend_bootstrap_manifest_identity", "configuration_snapshot_identity", "event_case",
                "finite_request_closure", "lifecycle_case", "lifecycle_schema_version", "local_stale_case",
                "returned_backend_descriptors", "span_schema_version", "stable_platform_envelope_identity",
                "support_horizons_us"}, "Catalog source entry")
    for key in ("backend_bootstrap_manifest_identity", "configuration_snapshot_identity", "stable_platform_envelope_identity"):
        hex_identity(raw[key], key)
    exact(raw["returned_backend_descriptors"], {"adapter", "mlx", "interface", "capability", "resource_signal", "operation_bound_set"}, "returned Backend descriptors")
    for key, value in raw["returned_backend_descriptors"].items():
        hex_identity(value, key)
    if any(type(raw[name]) is not int or raw[name] != 1 for name in ("span_schema_version", "lifecycle_schema_version")): raise ValueError("unsupported span or lifecycle schema version")
    if raw["support_horizons_us"] != sorted(set(raw["support_horizons_us"])) or not raw["support_horizons_us"]:
        raise ValueError("support horizons must be positive, sorted, and unique")
    for horizon in raw["support_horizons_us"]:
        positive(horizon, "support horizon")
    lifecycle = table_case(cases["lifecycle_operations"], raw["lifecycle_case"], "lifecycle")
    event = table_case(cases["sequenced_events"], raw["event_case"], "event")
    stale = table_case(cases["local_stale"], raw["local_stale_case"], "local-stale")
    if stale["successor_support_ceiling_us"] < lifecycle["complete_envelope_us"]["form_candidates"]: raise ValueError("local-stale successor ceiling is below Candidate Formation envelope")
    counts, vector, claim_scope = count_bounds(core, raw["finite_request_closure"], horizons)
    retained = bounded_sum((item["max_starts"] for item in counts if item["horizon_us"] == horizons[-1]), "retained credits")
    interference = [{"horizon_us": horizon, "microseconds": bounded_sum((item["max_starts"] * lifecycle["complete_envelope_us"][item["operation"]] for item in counts if item["horizon_us"] == horizon), "support interference")} for horizon in raw["support_horizons_us"]]
    crossing = max(event["complete_envelope_us"][name] for name, kind in EVENTS.items() if kind == "mandatory_crossing")
    event_value = bounded_sum((event["complete_envelope_us"][name] * event["max_per_cut"][name] for name in EVENTS), "event interference")
    event_value = bounded_sum((event_value, event["mandatory_crossing_allowance"] * crossing), "event interference")
    evidence_refs = {kind: {"case_id": raw[{"lifecycle_operations": "lifecycle_case", "sequenced_events": "event_case", "local_stale": "local_stale_case"}[kind]], "identity": references[kind]["identity"]} for kind in TABLE_KINDS}
    key = {name: raw[name] for name in ("backend_bootstrap_manifest_identity", "configuration_snapshot_identity", "lifecycle_schema_version", "returned_backend_descriptors", "span_schema_version", "stable_platform_envelope_identity")}
    key.update({"lifecycle_evidence": evidence_refs, "support_horizons_us": raw["support_horizons_us"]})
    return {"key": key, "key_sha256": hashlib.sha256(canonical(key)).hexdigest(),
            "finite_request_closure": raw["finite_request_closure"],
            "owner_thread_support_budget": {"complete_envelope_us": lifecycle["complete_envelope_us"],
                "direct_call_us": lifecycle["direct_call_us"], "support_horizons_us": raw["support_horizons_us"],
                "count_horizons_us": horizons, "max_funding_claims_per_start": claim_scope, "support_start_count_bounds": counts,
                "support_interference_us": interference, "physical_start_credit_capacity": retained,
                "outstanding_credit_vector": {"dimensions": len(vector), "entries": vector}},
            "sequenced_event_interference_bound": {"complete_envelope_us": event["complete_envelope_us"],
                "event_kind_count": len(EVENTS), "mandatory_crossing_allowance": event["mandatory_crossing_allowance"],
                "max_per_cut": event["max_per_cut"],
                "interference_us": [{"horizon_us": horizon, "microseconds": event_value} for horizon in raw["support_horizons_us"]]},
            "stale_plan_disposition_bound": {"branches": stale["branches"],
                "complete_envelope_us": stale["complete_envelope_us"],
                "successor_support_ceiling_us": stale["successor_support_ceiling_us"]}}
def compile_catalog(core: Path, source_path: Path) -> dict:
    descriptor, identity = core_identity(core)
    source, _ = read_canonical(source_path)
    exact(source, {"source_schema_version", "catalog_type", "tables", "entries"}, "Catalog source")
    if type(source["source_schema_version"]) is not int or source["source_schema_version"] != 1 or source["catalog_type"] not in {"production", "test_fixture_nonproduction"}:
        raise ValueError("unknown Catalog source type or schema")
    exact(source["tables"], TABLE_KINDS, "Lifecycle Evidence Table registry")
    reject_forbidden(source, set(descriptor["excluded_identity_inputs"]))
    references, cases = {}, {}
    for kind in TABLE_KINDS:
        filename = source["tables"][kind]
        if type(filename) is not str or Path(filename).name != filename:
            raise ValueError("Lifecycle Evidence Table path must be one local filename")
        table, payload = read_canonical(source_path.parent / filename)
        reject_forbidden(table, set(descriptor["excluded_identity_inputs"]))
        cases[kind] = validate_table(table, kind, source["catalog_type"], identity, descriptor["event_registry"])
        references[kind] = {"case_ids": sorted(cases[kind]), "evidence_type": table["evidence_type"],
                            "identity": evidence(payload, f"turnvector:evidence:{kind}\0".encode()),
                            "provenance": table["provenance"]}
    if type(source["entries"]) is not list or not source["entries"] or len(source["entries"]) > descriptor["catalog"]["capacity"]["max_entries"]:
        raise ValueError("Catalog entry count is outside the Core capacity")
    horizons = sorted({item for entry in source["entries"] for item in entry["support_horizons_us"]})
    if len(horizons) > descriptor["support"]["start_count"]["max_horizons"]:
        raise ValueError("Catalog horizon count exceeds Core capacity")
    entries = sorted((compile_entry(descriptor, entry, horizons, references, cases) for entry in source["entries"]), key=lambda item: item["key_sha256"])
    if len({entry["key_sha256"] for entry in entries}) != len(entries):
        raise ValueError("duplicate Catalog entry key")
    for entry in entries:
        if len(canonical(entry)) > descriptor["catalog"]["capacity"]["max_entry_bytes"]:
            raise ValueError("Catalog entry exceeds Core byte capacity")
    retained_counts = {entry["key_sha256"]: {(item["operation"], item["pool"]): item["max_starts"] for item in entry["owner_thread_support_budget"]["support_start_count_bounds"] if item["horizon_us"] == horizons[-1]} for entry in entries}
    joint_counts = {cell: max(counts[cell] for counts in retained_counts.values()) for cell in next(iter(retained_counts.values()))}; active = bounded_sum(joint_counts.values(), "activation-history credits")
    accepted, ingress = descriptor["cardinality_inputs"]["ingress"]["global_active"], descriptor["cardinality_inputs"]["ingress"]
    pending, conditional = accepted * 3, accepted * 4
    vector_dimensions = max(entry["owner_thread_support_budget"]["outstanding_credit_vector"]["dimensions"] for entry in entries)
    max_batch = max(entry["finite_request_closure"]["max_batch_members"] for entry in entries); claim_scope = {operation: max(entry["owner_thread_support_budget"]["max_funding_claims_per_start"][operation] for entry in entries) for operation in SUPPORT}
    ordinary = bounded_sum((count for (operation, pool), count in joint_counts.items() if pool == "ordinary"), "ordinary claims"); active_claims = bounded_sum((count * claim_scope[operation] for (operation, pool), count in joint_counts.items()), "activation-history funding claims")
    max_churn = max(entry["finite_request_closure"]["max_sequential_request_churn"] for entry in entries); population = accepted + max_churn; description_items = max(accepted + 1, ingress["global_warming"] + descriptor["cardinality_inputs"]["model_registry"]); lifecycle = description_items + accepted + 1
    funding = {"active_and_retained": active_claims, "conditional_obligations": conditional, "entitlement_vector_slots": accepted * vector_dimensions, "lifecycle_reserves": lifecycle, "pending_obligations": pending}; joint = active + pending + conditional + lifecycle
    required = {"active_and_retained": active, "conditional_obligations": conditional,
                "entitlement_tombstones": population,
                "funding_claims": bounded_sum(funding.values(), "funding claims"),
                "lifecycle_reserves": lifecycle, "ordinary_claims": ordinary, "pending_obligations": pending,
                "physical_start_credits": joint,
                "total_operation_obligations": joint, "vector_dimensions": vector_dimensions}
    records, support = descriptor["support"]["records"], descriptor["support"]
    available = {"active_and_retained": records["active_and_retained"], "conditional_obligations": records["conditional_obligations"],
                 "entitlement_tombstones": records["entitlement_tombstones"], "funding_claims": records["funding_claims"],
                 "lifecycle_reserves": records["lifecycle_reserves"], "ordinary_claims": records["ordinary_claims"],
                 "pending_obligations": records["pending_obligations"],
                 "physical_start_credits": support["start_count"]["max_physical_credits"],
                 "total_operation_obligations": records["total_operation_obligations"],
                 "vector_dimensions": support["outstanding_credit_vector"]["max_dimensions"]}
    for name in required:
        if required[name] > available[name]:
            raise ValueError(f"Catalog exceeds Core {name} capacity")
    pool_capacity = {pool: bounded_sum((count for (operation, cell_pool), count in joint_counts.items() if cell_pool == pool), f"{pool} activation capacity") for pool in POOLS}; mandatory, safety = pool_capacity["mandatory_completion"], pool_capacity["safety_sampling"]
    activations = [{"active_and_retained": active, "active_and_retained_funding_claims": active_claims, "history_reset": False,
                    "mandatory_suballocation": mandatory, "ordinary_retained": ordinary, "predecessor": before["key_sha256"], "safety_suballocation": safety, "successor": after["key_sha256"]} for before in entries for after in entries]
    carry = {"activation_sequences": activations, "mandatory_suballocation": mandatory, "nonborrowable": True,
             "safety_suballocation": safety, "slots": 1}
    if carry["slots"] > descriptor["prepared_carry"]["slots"] or mandatory > descriptor["prepared_carry"]["mandatory_suballocation_max"] or safety > descriptor["prepared_carry"]["safety_suballocation_max"]:
        raise ValueError("Catalog exceeds Prepared Carry capacity")
    result = {"catalog_schema_version": 1, "catalog_type": source["catalog_type"],
            "daemon_core_build_identity": identity, "catalog_retention_horizon_us": horizons[-1],
            "interface_operation_classifications": INTERFACE, "lifecycle_evidence_tables": references,
            "entries": entries, "capacity_proof": {"available": available, "required": required, "basis": {"accepted_requests": accepted, "funding_claims": funding, "global_warming": ingress["global_warming"], "lifecycle_reserves": {"post_load_description_claims": description_items, "post_observation_description_claims": accepted, "safety_schedule": 1}, "max_batch_members": max_batch, "max_sequential_request_churn": max_churn, "model_registry": descriptor["cardinality_inputs"]["model_registry"], "per_connection_active": ingress["per_connection_active"], "per_connection_warming": ingress["per_connection_warming"], "request_population": population}},
            "prepared_carry_proof": carry,
            "conservation_rules": {"active_scope": "frozen", "conditional_successors": ["pending", "typed_impossible_close"],
                "carry_in_credit_conservation": {"carry_in_credits": 1, "future_start_reduction": 1, "retained_until_horizon_us": horizons[-1], "scope": "same_operation_pool_horizon"},
                "funding_claims_per_affected_request": 1, "ordinary_claim_required": True,
                "physical_credits_per_call": 1, "pool_borrowing": False,
                "prestart_mutations": ["split", "merge", "rebind", "typed_impossible_close"],
                "terminal_entitlement": "retained_tombstone_until_catalog_horizon"},
            "lifecycle_capacity_rules": {
                "actual_call_credits": {batch: {branch: 1 for branch in ("conditional_continuation", "nonreceipt_formation", "receipt_observation")} for batch in ("B1", "B4")},
                "candidate_funding": ["admission_initial", "entitlement_vector", "mixed_initial_and_entitlement"],
                "conditional_occupies": ["physical_credit", "funding_claims", "vector_slots", "storage", "all_horizons"],
                "conditional_successors": ["pending", "typed_impossible_close"],
                "description_reserves": ["post_load_before_effect", "post_observation_before_observation"],
                "half_open_window": "[t,t+H);carry_and_crossing_count_once",
                "max_funding_claims_per_start": claim_scope, "new_member": "joins_funding_before_call_scope",
                "prestart_membership_changes": ["split", "merge", "rebind", "cancel"],
                "safety_reserves": ["first_before_readiness", "next_before_expiry"],
                "started_membership": "frozen",
                "terminal_entitlement": "tombstone_until_catalog_retention"}}
    if len(canonical(result)) > descriptor["catalog"]["capacity"]["max_canonical_bytes"]:
        raise ValueError("Catalog exceeds Core canonical byte capacity")
    return result
def lock_for(payload: bytes) -> dict: return {"algorithm": "sha256", "descriptor": CATALOG, "digest": evidence(payload), "domain": DOMAIN[:-1].decode(), "hash_schema_version": 1, "kind": "runtime_overhead_catalog", "preimage": "utf8(domain)||0x00||u32be(hash_schema_version)||descriptor_bytes"}
def write(output: Path, value: dict) -> None:
    payload = canonical(value); output.mkdir(parents=True, exist_ok=True)
    (output / CATALOG).write_bytes(payload); (output / LOCK).write_bytes(canonical(lock_for(payload)))
def check(output: Path, value: dict) -> None:
    catalog, payload = read_canonical(output / CATALOG); lock, lock_payload = read_canonical(output / LOCK)
    if catalog != value or payload != canonical(value): raise ValueError("Catalog output does not match the current inputs")
    expected = lock_for(payload)
    if lock != expected or lock_payload != canonical(expected): raise ValueError("Catalog Evidence Hash lock does not match")
def main() -> None:
    parser = argparse.ArgumentParser(); parser.add_argument("--core", type=Path, required=True); parser.add_argument("--source", type=Path, required=True)
    mode = parser.add_mutually_exclusive_group(required=True); mode.add_argument("--output", type=Path); mode.add_argument("--check", type=Path)
    args = parser.parse_args()
    try:
        value = compile_catalog(args.core, args.source)
        write(args.output, value) if args.output else check(args.check, value)
    except (KeyError, OSError, ValueError, json.JSONDecodeError) as error:
        parser.exit(1, f"runtime overhead catalog error: {error}\n")
if __name__ == "__main__":
    main()
