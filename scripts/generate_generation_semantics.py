#!/usr/bin/env python3
import argparse
import hashlib
import json
from pathlib import Path


DESCRIPTOR_NAME, LOCK_NAME = "generation-semantics-v1.json", "generation-semantics-v1.lock.json"
HASH_DOMAIN = b"turnvector:evidence:generation-semantics\0"
HASH_SCHEMA_VERSION = 1

DESCRIPTOR = {
    "descriptor_schema_version": 1,
    "generation_parameters": {
        "fields": {
            "sampling_mode": {
                "encoding": "closed_enum",
                "presence": "required",
                "values": {"UNSPECIFIED": 0, "GREEDY": 1, "CATEGORICAL": 2},
                "valid_values": ["GREEDY", "CATEGORICAL"],
                "invalid_values": ["UNSPECIFIED"],
            },
            "temperature": {"encoding": "ieee754_binary32_bits", "presence": "required", "preserve_bits": True},
            "top_p": {"encoding": "ieee754_binary32_bits", "presence": "required", "preserve_bits": True},
            "top_k": {"encoding": "u32", "presence": "required"},
        },
        "common_binary32_rejections": ["negative_zero", "nan", "positive_infinity", "negative_infinity"],
        "greedy_domain": {"sampling_mode": "GREEDY", "temperature_bits": "0x00000000", "top_p_bits": "0x3f800000", "top_k": 0},
        "categorical_domain": {
            "sampling_mode": "CATEGORICAL",
            "temperature": {"lower_exclusive_bits": "0x00000000", "upper_inclusive_bits": "0x40000000"},
            "top_p": {"lower_exclusive_bits": "0x00000000", "upper_inclusive_bits": "0x3f800000"},
            "top_k": {"disabled": 0, "enabled_max": "registered_vocabulary_size-1"},
        },
        "unsupported": ["logits_bias", "repetition_penalty", "presence_penalty", "frequency_penalty",
                        "min_p", "xtc", "backend_native_parameters"],
    },
    "greedy": {"logit_precondition": "nonfinite model or binary32-cast logit fails before output",
               "selection": "stable_argmax", "tie_break": "lower_token_id", "filters": "none", "rng_splits_per_token": 0},
    "categorical": {
        "logit_precondition": "nonfinite model or binary32-cast logit fails before split,state,output",
        "tensor_dtype": "binary32",
        "tensor_steps": ["centered=logits-max(logits)", "weights=exp(centered)", "normalizer=sum(weights)",
                         "log_probabilities=centered-log(normalizer)", "probabilities=exp(log_probabilities)"],
        "filter_order": ["top_p", "top_k"],
        "top_p": {
            "one": "retain_all", "order": "ascending(log_probability,token_id)",
            "cumulative_input": "gather(probabilities_from_exp(log_probabilities),order)",
            "cumulative": "inclusive_binary32", "mask": "cumulative<=binary32(1-top_p)",
            "retained": "first_strict_crossing_and_later",
            "no_crossing": "retain_only_greatest_log_probability;tie=lower_token_id",
        },
        "top_k": {"zero": "disabled", "input": "top_p_survivors", "count": "min(top_k,surviving_count)",
                  "ranking": "greatest_log_probability", "tie_break": "lower_token_id"},
        "compaction_order": "ascending_token_id",
        "sampling_logits": "(compacted_log_probability-max_compacted)/temperature",
        "draw_key": "split(state).second",
        "draw": "mlx_random_categorical_on_nonempty_compact_vector;zero_uniform_cannot_select_masked",
        "result": "map_compact_index_to_token_id",
        "masked_tokens_reach_draw": False,
    },
    "sampling_rng": {
        "seed": "u64 including zero", "initial_state": "mlx::core::random::key(seed)=uint32[2]{high32,low32}",
        "categorical_transition": "split(state);next=first;draw_key=second", "splits_per_sampled_token": 1,
        "hidden_or_ambiguous_stop_token_advances": True, "greedy_transition": "no_split",
        "batch_or_turn_shared_state": False,
    },
}


def canonical_json(value) -> bytes:
    return (json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n").encode()


def evidence_digest(payload: bytes) -> str:
    version = HASH_SCHEMA_VERSION.to_bytes(4, "big")
    return hashlib.sha256(HASH_DOMAIN + version + payload).hexdigest()


def lock_for(payload: bytes) -> dict:
    return {"algorithm": "sha256", "descriptor": DESCRIPTOR_NAME,
            "digest": evidence_digest(payload), "domain": HASH_DOMAIN[:-1].decode(),
            "hash_schema_version": HASH_SCHEMA_VERSION, "kind": "generation_semantics",
            "preimage": "utf8(domain)||0x00||u32be(hash_schema_version)||descriptor_bytes"}


def generate(output: Path) -> None:
    payload = canonical_json(DESCRIPTOR)
    output.mkdir(parents=True, exist_ok=True)
    (output / DESCRIPTOR_NAME).write_bytes(payload)
    (output / LOCK_NAME).write_bytes(canonical_json(lock_for(payload)))


def object_without_duplicates(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f"duplicate descriptor key: {key}")
        result[key] = value
    return result


def read_json(payload: bytes):
    return json.loads(payload, object_pairs_hook=object_without_duplicates)


def validate_shape(value, reference, path="descriptor") -> None:
    if type(value) is not type(reference):
        raise ValueError(f"{path} has the wrong value type")
    if isinstance(reference, dict):
        if value.keys() != reference.keys():
            raise ValueError(f"{path} has missing or unknown keys")
        for key in reference:
            validate_shape(value[key], reference[key], f"{path}.{key}")
    elif isinstance(reference, list):
        if len(value) != len(reference):
            raise ValueError(f"{path} has missing or unknown entries")
        for index, expected in enumerate(reference):
            validate_shape(value[index], expected, f"{path}[{index}]")


def validated_descriptor(payload: bytes) -> dict:
    descriptor = read_json(payload)
    validate_shape(descriptor, DESCRIPTOR)
    if descriptor["descriptor_schema_version"] != DESCRIPTOR["descriptor_schema_version"]:
        raise ValueError("unknown descriptor schema version")
    if payload != canonical_json(descriptor):
        raise ValueError("descriptor is not canonically encoded")
    return descriptor


def check(output: Path) -> None:
    payload = (output / DESCRIPTOR_NAME).read_bytes()
    descriptor = validated_descriptor(payload)
    if descriptor != DESCRIPTOR:
        raise ValueError("descriptor was not generated by this build")

    lock_payload = (output / LOCK_NAME).read_bytes()
    lock = read_json(lock_payload)
    expected_lock = lock_for(payload)
    validate_shape(lock, expected_lock, "lock")
    if lock_payload != canonical_json(lock) or lock != expected_lock:
        raise ValueError("descriptor Evidence Hash lock does not match")


def main() -> None:
    parser = argparse.ArgumentParser()
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--output", type=Path)
    mode.add_argument("--check", type=Path)
    mode.add_argument("--identity", type=Path)
    args = parser.parse_args()
    try:
        if args.output:
            generate(args.output)
        elif args.check:
            check(args.check)
        else:
            payload = args.identity.read_bytes()
            validated_descriptor(payload)
            print(evidence_digest(payload))
    except (OSError, ValueError, json.JSONDecodeError) as error:
        parser.exit(1, f"generation semantics error: {error}\n")


if __name__ == "__main__":
    main()
