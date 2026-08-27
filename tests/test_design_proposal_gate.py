import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


def normalized(path):
    return " ".join(path.read_text(encoding="utf-8").split())


GATE = normalized(ROOT / "docs" / "design-proposal-gate.md")
AGENT_INSTRUCTIONS = normalized(ROOT / "AGENTS.md")
DELIVERY_AUTHORITY = " ".join(
    normalized(ROOT / "docs" / "plans" / name)
    for name in (
        "2026-08-16-p0-runtime-implementation.md",
        "2026-08-19-p0-parallel-module-delivery.md",
    )
)
TRACKED_AUTHORITY = f"{AGENT_INSTRUCTIONS} {GATE} {DELIVERY_AUTHORITY}"


class DesignProposalGateTests(unittest.TestCase):
    def test_pre_gate_disclosure_does_not_invalidate_lineage(self):
        self.assertIn("Pre-Gate disclosure is allowed", GATE)
        self.assertIn("does not invalidate a design lineage", GATE)

    def test_temporary_verification_writes_may_be_fully_restored(self):
        self.assertIn("Temporary verification writes are allowed", GATE)
        self.assertIn("`/tmp` or a disposable clone", GATE)
        self.assertIn("must not stage, commit, push, or change refs", GATE)
        self.assertIn("restored to its exact frozen state", GATE)

    def test_bundle_change_invalidates_only_the_round(self):
        self.assertIn("A Review Bundle hash change invalidates that round", GATE)
        self.assertIn("does not permanently invalidate the design lineage", GATE)

    def test_only_required_internal_references_may_be_read_outside_bundle(self):
        self.assertIn(
            "Formal Design Proposal Gate reviewers read those required internal references directly",
            AGENT_INSTRUCTIONS,
        )
        self.assertIn(
            "Those required internal references are the only permitted bundle-external content",
            GATE,
        )
        self.assertIn(
            "The required internal references are its only permitted external review context",
            AGENT_INSTRUCTIONS,
        )
        self.assertIn(
            "Reading any other bundle-external content invalidates the round",
            GATE,
        )
        self.assertIn(
            "Design Proposal Gate reviewers read the frozen bundle plus",
            DELIVERY_AUTHORITY,
        )
        self.assertNotIn(
            "rereads repository-required internal references",
            DELIVERY_AUTHORITY,
        )
        self.assertNotIn("runs or requests relevant tests", DELIVERY_AUTHORITY)

    def test_invocation_is_derived_from_the_launch_record(self):
        self.assertIn("The Launch Record is the sole invocation identity", GATE)
        self.assertIn("generated directly from the Launch Record", GATE)
        self.assertIn("byte-for-byte identical", GATE)

    def test_retired_delivery_protocol_is_absent(self):
        retired = (
            "accepted-object auditor",
            "accepted auditor",
            "auditor-at-",
            "accepted immediate-parent auditor",
            "Component Manifest",
            "formal pre/post snapshot",
            "bounded auditor",
            "read-count promise",
        )
        for term in retired:
            self.assertNotIn(term, TRACKED_AUTHORITY)
        self.assertFalse((ROOT / "scripts" / "check_worktree_policy.py").exists())


if __name__ == "__main__":
    unittest.main()
