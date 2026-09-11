"""Focused tests for the finalized GCP Cooking gate-results sidecar."""

from __future__ import annotations

import importlib.util
import json
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).parent
HELPER = ROOT / "cooking-gate-results.py"


def _load(name: str, path: Path):
    spec = importlib.util.spec_from_file_location(name, path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"cannot load {path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


COLLECTOR = _load("cooking_gate_collector", ROOT / "collect-cooking-diagnostics.py")
TRIAGE = _load("cooking_gate_summary", ROOT / "summarize-cooking-diagnostics.py")


class GateResultsTests(unittest.TestCase):
    def _invoke(self, path: Path, *args: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            ["python3", str(HELPER), "--path", str(path), *args],
            text=True,
            capture_output=True,
            check=False,
        )

    def _sidecar(
        self,
        states: tuple[str, str, str],
        exit_code: int,
        supervisor_exit: int | None = None,
    ) -> dict:
        if supervisor_exit is None:
            supervisor_exit = exit_code
        gates = {}
        for name, state in zip(COLLECTOR.GATE_NAMES, states):
            reason = (
                "none" if state == "passed" else
                "timeout" if state == "timed-out" else
                "unfinished" if state == "unknown" else
                "workload-failed" if name == "workload" else
                "evidence-scan-failed" if name == "evidence-scan" else
                "browser-validation-failed"
            )
            gate_exit = (
                0 if state == "passed" else
                124 if state == "timed-out" else
                None if state == "unknown" else exit_code
            )
            gates[name] = {"state": state, "exit_code": gate_exit, "reason_class": reason}
        return {
            "schema": 1,
            "revision": "a" * 40,
            "script_sha256": "b" * 64,
            "test_mode": "gcp-cooking",
            "overall_exit_code": exit_code,
            "supervisor_exit_code": supervisor_exit,
            "finalized": True,
            "gates": gates,
        }

    def _helper_sidecar(self, root: Path, states: tuple[str, str, str], exit_code: int) -> dict:
        """Create the source with the production helper, then read its JSON."""

        sidecar = root / "helper" / "cooking-gate-results.json"
        sidecar.parent.mkdir(mode=0o700)
        digest = subprocess.run(
            ["sha256sum", str(HELPER)], text=True, capture_output=True, check=True
        ).stdout.split()[0]
        result = self._invoke(
            sidecar,
            "--script-file", str(HELPER),
            "init", "--revision", "a" * 40, "--script-sha256", digest, "--test-mode", "gcp-cooking",
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        for name, state in zip(COLLECTOR.GATE_NAMES, states):
            result = self._invoke(sidecar, "begin", name)
            self.assertEqual(result.returncode, 0, result.stderr)
            if state == "unknown":
                continue
            reason = (
                "none" if state == "passed" else
                "timeout" if state == "timed-out" else
                "workload-failed" if name == "workload" else
                "evidence-scan-failed" if name == "evidence-scan" else
                "browser-validation-failed"
            )
            gate_exit = 0 if state == "passed" else 124 if state == "timed-out" else exit_code
            result = self._invoke(
                sidecar, "complete", name, "--state", state,
                "--exit-code", str(gate_exit), "--reason-class", reason,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
        result = self._invoke(sidecar, "finalize", "--overall-exit-code", str(exit_code))
        self.assertEqual(result.returncode, 0, result.stderr)
        # Startup observes the child's exit separately and appends it after
        # the runtime has recorded its aggregate result.
        result = self._invoke(
            sidecar, "finalize", "--overall-exit-code", str(exit_code),
            "--supervisor-exit-code", "124",
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        return json.loads(sidecar.read_text(encoding="utf-8"))

    def _summarize_sidecar(self, value: dict):
        with tempfile.TemporaryDirectory(prefix="heph-gcp-gates-") as directory:
            root = Path(directory)
            helper_output = root / "helper-gate-results.json"
            helper_output.write_text(json.dumps(value) + "\n", encoding="utf-8")
            copied = root / "input" / "gate-results.json"
            copied.parent.mkdir(mode=0o700)
            # This models startup's bounded copy into its collector input.
            shutil.copyfile(helper_output, copied)
            bundle = root / "bundle"
            self.assertEqual(COLLECTOR.collect(bundle, [f"gate-results={copied}"], None, None, None), 0)
            return TRIAGE.summarize(bundle)

    def test_production_helper_output_copies_through_collector_and_summary(self):
        with tempfile.TemporaryDirectory(prefix="heph-gcp-gates-") as directory:
            value = self._helper_sidecar(Path(directory), ("failed", "passed", "passed"), 7)
        triage = self._summarize_sidecar(value)
        self.assertEqual(triage["gateResults"]["test_mode"], "gcp-cooking")
        self.assertEqual(triage["gateResults"]["gates"]["evidence-scan"]["state"], "passed")
        self.assertNotIn("payload", triage["gateResults"])

    def test_timeout_and_unknown_states_are_typed(self):
        timeout = self._summarize_sidecar(self._sidecar(("timed-out", "unknown", "unknown"), 124))
        self.assertEqual(timeout["gateResults"]["gates"]["workload"]["reason_class"], "timeout")
        unknown = self._summarize_sidecar(self._sidecar(("unknown", "unknown", "unknown"), 1))
        self.assertEqual(unknown["gateResults"]["gates"]["workload"]["reason_class"], "unfinished")

    def test_runtime_and_startup_exit_codes_are_preserved_independently(self):
        value = self._sidecar(("failed", "passed", "passed"), 1, supervisor_exit=124)
        triage = self._summarize_sidecar(value)
        self.assertEqual(triage["gateResults"]["overall_exit_code"], 1)
        self.assertEqual(triage["gateResults"]["supervisor_exit_code"], 124)

    def test_missing_legacy_gate_source_is_explicitly_unavailable(self):
        with tempfile.TemporaryDirectory(prefix="heph-gcp-gates-legacy-") as directory:
            root = Path(directory)
            source = root / "serial.log"
            source.write_text("HEPH_GCP_COOKING event=phase-pass phase=smoke\n", encoding="utf-8")
            bundle = root / "bundle"
            self.assertEqual(COLLECTOR.collect(bundle, [f"serial={source}"], None, None, None), 0)
            self.assertEqual(
                TRIAGE.summarize(bundle)["gateResults"],
                {"schema": 1, "status": "unavailable", "reason": "missing-result"},
            )

    def test_secret_and_extra_fields_are_rejected_without_retention(self):
        for mutation in (
            lambda value: value.update({"payload": "secret"}),
            lambda value: value.update({"secret": "token=leak"}),
        ):
            with self.subTest(mutation=mutation):
                value = self._sidecar(("passed", "passed", "passed"), 0)
                mutation(value)
                with tempfile.TemporaryDirectory(prefix="heph-gcp-gates-invalid-") as directory:
                    root = Path(directory)
                    source = root / "gate-results.json"
                    source.write_text(json.dumps(value) + "\n", encoding="utf-8")
                    serial = root / "serial.log"
                    serial.write_text("safe\n", encoding="utf-8")
                    bundle = root / "bundle"
                    self.assertEqual(COLLECTOR.collect(bundle, [f"serial={serial}", f"gate-results={source}"], None, None, None), 0)
                    manifest = json.loads((bundle / "manifest.json").read_text(encoding="utf-8"))
                    self.assertEqual(manifest["rejectedSources"][0]["label"], "gate-results")
                    self.assertFalse((bundle / "sources/gate-results").exists())

    def test_stale_provenance_and_invalid_transitions_are_rejected(self):
        cases = []
        stale = self._sidecar(("passed", "passed", "passed"), 0)
        stale["finalized"] = False
        cases.append(stale)
        malformed_provenance = self._sidecar(("passed", "passed", "passed"), 0)
        malformed_provenance["revision"] = "A" * 40
        cases.append(malformed_provenance)
        invalid = self._sidecar(("failed", "passed", "passed"), 0)
        cases.append(invalid)
        for value in cases:
            with self.subTest(value=value):
                with tempfile.TemporaryDirectory(prefix="heph-gcp-gates-invalid-") as directory:
                    root = Path(directory)
                    source = root / "gate-results.json"
                    source.write_text(json.dumps(value) + "\n", encoding="utf-8")
                    serial = root / "serial.log"
                    serial.write_text("safe\n", encoding="utf-8")
                    bundle = root / "bundle"
                    self.assertEqual(COLLECTOR.collect(bundle, [f"serial={serial}", f"gate-results={source}"], None, None, None), 0)
                    manifest = json.loads((bundle / "manifest.json").read_text(encoding="utf-8"))
                    self.assertEqual(manifest["rejectedSources"][0]["label"], "gate-results")


if __name__ == "__main__":
    unittest.main()
