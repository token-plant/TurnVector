import copy
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
GENERATOR = ROOT / "scripts" / "generate_generation_semantics.py"
SCHEMAS = ROOT / "schemas"
DESCRIPTOR_NAME, LOCK_NAME = "generation-semantics-v1.json", "generation-semantics-v1.lock.json"


def canonical(value) -> bytes:
    return (json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n").encode()


class GenerationSemanticsTests(unittest.TestCase):
    def run_generator(self, *args: str) -> subprocess.CompletedProcess:
        return subprocess.run(
            [sys.executable, "-I", "-B", str(GENERATOR), *args], cwd=ROOT,
            check=False, capture_output=True, text=True,
        )

    def generate(self, output: Path) -> None:
        result = self.run_generator("--output", str(output))
        self.assertEqual(result.returncode, 0, result.stderr)

    def assert_rejected(self, descriptor, lock) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            if descriptor is not None:
                (root / DESCRIPTOR_NAME).write_bytes(descriptor)
            if lock is not None:
                (root / LOCK_NAME).write_bytes(lock)
            self.assertNotEqual(self.run_generator("--check", str(root)).returncode, 0)

    def test_double_generation_is_byte_identical(self):
        with tempfile.TemporaryDirectory() as directory:
            outputs = [Path(directory) / name for name in ("first", "second")]
            for output in outputs:
                self.generate(output)
            for name in (DESCRIPTOR_NAME, LOCK_NAME):
                generated = (outputs[0] / name).read_bytes()
                self.assertEqual(generated, (outputs[1] / name).read_bytes())
                self.assertEqual(generated, (SCHEMAS / name).read_bytes())

    def test_check_rejects_every_fail_closed_input_class(self):
        descriptor_payload = (SCHEMAS / DESCRIPTOR_NAME).read_bytes()
        lock_payload = (SCHEMAS / LOCK_NAME).read_bytes()
        descriptor = json.loads(descriptor_payload)
        lock = json.loads(lock_payload)

        missing_key = copy.deepcopy(descriptor)
        del missing_key["greedy"]
        unknown_key = copy.deepcopy(descriptor)
        unknown_key["future_semantics"] = {}
        unknown_version = copy.deepcopy(descriptor)
        unknown_version["descriptor_schema_version"] = 2
        missing_lock_key = copy.deepcopy(lock)
        del missing_lock_key["kind"]
        unknown_lock_key = copy.deepcopy(lock)
        unknown_lock_key["future_hash"] = "unknown"

        drift = copy.deepcopy(descriptor)
        drift["greedy"]["selection"] = "unstable_argmax"
        drift_payload = canonical(drift)
        with tempfile.TemporaryDirectory() as directory:
            drift_path = Path(directory) / DESCRIPTOR_NAME
            drift_path.write_bytes(drift_payload)
            identity = self.run_generator("--identity", str(drift_path))
        self.assertEqual(identity.returncode, 0, identity.stderr)
        matching_drift_lock = copy.deepcopy(lock)
        matching_drift_lock["digest"] = identity.stdout.strip()

        cases = (
            ("missing descriptor", None, lock_payload),
            ("missing lock", descriptor_payload, None),
            ("duplicate descriptor", b'{"x":1,"x":2}\n', lock_payload),
            ("noncanonical descriptor", b" " + descriptor_payload, lock_payload),
            ("missing descriptor key", canonical(missing_key), lock_payload),
            ("unknown descriptor key", canonical(unknown_key), lock_payload),
            ("unknown descriptor version", canonical(unknown_version), lock_payload),
            ("duplicate lock", descriptor_payload, b'{"x":1,"x":2}\n'),
            ("noncanonical lock", descriptor_payload, b" " + lock_payload),
            ("missing lock key", descriptor_payload, canonical(missing_lock_key)),
            ("unknown lock key", descriptor_payload, canonical(unknown_lock_key)),
            ("matching lock for build drift", drift_payload, canonical(matching_drift_lock)),
        )
        for name, candidate_descriptor, candidate_lock in cases:
            with self.subTest(name=name):
                self.assert_rejected(candidate_descriptor, candidate_lock)

    def test_every_semantic_category_changes_the_evidence_hash(self):
        descriptor = json.loads((SCHEMAS / DESCRIPTOR_NAME).read_bytes())
        mutations = []
        for path, value in (
            (("generation_parameters", "fields", "sampling_mode", "invalid_values", 0), "GREEDY"),
            (("generation_parameters", "fields", "sampling_mode", "encoding"), "enum_i32"),
            (("generation_parameters", "categorical_domain", "top_p", "upper_inclusive_bits"), "0x3f7fffff"),
            (("greedy", "selection"), "unstable_argmax"),
            (("categorical", "filter_order", 0), "top_k"),
            (("categorical", "top_p", "cumulative_input"), "weights/normalizer"),
            (("categorical", "top_p", "mask"), "cumulative<binary32(1-top_p)"),
            (("categorical", "top_k", "input"), "full_vocabulary"),
            (("categorical", "top_k", "ranking"), "token_id"),
            (("categorical", "draw_key"), "split(state).first"),
            (("sampling_rng", "categorical_transition"), "draw_key=first;next=second"),
        ):
            mutation = copy.deepcopy(descriptor)
            target = mutation
            for key in path[:-1]:
                target = target[key]
            target[path[-1]] = value
            mutations.append(mutation)

        identities = []
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / DESCRIPTOR_NAME
            for value in (descriptor, *mutations):
                path.write_bytes(canonical(value))
                result = self.run_generator("--identity", str(path))
                self.assertEqual(result.returncode, 0, result.stderr)
                identities.append(result.stdout.strip())
        self.assertEqual(len(identities), len(set(identities)))


if __name__ == "__main__":
    unittest.main()
