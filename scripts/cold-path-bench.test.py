"""Negative controls for the benchmark's input-equivalence gate (CI only)."""

import copy
import importlib.util
from pathlib import Path
import unittest


spec = importlib.util.spec_from_file_location("cold_path_bench", Path(__file__).with_name("cold-path-bench.py"))
bench = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bench)


def row():
    return {
        "input_tree": "same tree", "stdout_hash": "same stdout", "stderr_hash": "same stderr",
        "outputs_hash": "same outputs", "root_outputs_hash": "same root outputs",
        "step_key": "same key", "image": "same image", "manifest_matches_input": True,
        "_manifest": {"v": 3, "step_key": "same key", "roots": ["/workspace"],
                      "entries": [{"p": "cloud/source.ts", "k": "f", "h": "source bytes", "m": 420}]},
    }


class Controls(unittest.TestCase):
    def test_accepts_extra_valid_absence_probes_without_deleting_them(self):
        left, right = row(), row()
        right["_manifest"]["entries"].append({"p": "cloud/app/invite/node_modules", "k": "a"})
        original = copy.deepcopy(right)
        bench.validate_controls([left, right])
        self.assertEqual(right, original)

    def test_rejects_a_manifest_that_disagrees_with_its_actual_input(self):
        left, right = row(), row()
        right["manifest_matches_input"] = False
        with self.assertRaisesRegex(AssertionError, "disagrees with actual input"):
            bench.validate_controls([left, right])

    def test_rejects_changed_positive_reads_even_with_unchanged_summary_hashes(self):
        for field, value in (("h", "other content"), ("m", 493), ("p", "cloud/other.ts")):
            with self.subTest(field=field):
                left, right = row(), row()
                right["_manifest"]["entries"][0][field] = value
                with self.assertRaisesRegex(AssertionError, "positive reads changed"):
                    bench.validate_controls([left, right])

    def test_rejects_changed_complete_inputs_outputs_keys_and_images(self):
        for field in ("input_tree", "stdout_hash", "stderr_hash", "outputs_hash",
                      "root_outputs_hash", "step_key", "image"):
            with self.subTest(field=field):
                left, right = row(), row()
                right[field] = "changed"
                with self.assertRaisesRegex(AssertionError, field + " changed"):
                    bench.validate_controls([left, right])

    def test_rejects_changed_manifest_identity_fields(self):
        for field, value in (("v", 4), ("step_key", "other key"),
                             ("roots", ["/other"]), ("volatile", ["UNKEYED"])):
            with self.subTest(field=field):
                left, right = row(), row()
                right["_manifest"][field] = value
                with self.assertRaisesRegex(AssertionError, "manifest frame changed"):
                    bench.validate_controls([left, right])

    def test_rejects_missing_controls(self):
        with self.assertRaisesRegex(AssertionError, "missing comparison controls"):
            bench.validate_controls([row()])

    def test_rejects_manifests_for_a_different_step_even_if_they_match_each_other(self):
        left, right = row(), row()
        for value in (left, right):
            value["_manifest"]["step_key"] = "other step"
        with self.assertRaisesRegex(AssertionError, "belongs to another step"):
            bench.validate_controls([left, right])


if __name__ == "__main__":
    unittest.main()
