"""Focused tests for the bounded Cooking evidence collector."""

import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch


SPEC = importlib.util.spec_from_file_location(
    "cooking_diagnostics", Path(__file__).with_name("collect-cooking-diagnostics.py")
)
COLLECTOR = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(COLLECTOR)


class CookingDiagnosticsTests(unittest.TestCase):
    def test_collects_timeout_output_and_metadata_only_lineage(self):
        with tempfile.TemporaryDirectory() as root:
            root_path = Path(root)
            source = root_path / "serial.log"
            source.write_text(
                "HEPH_GCP_COOKING: FAIL exit=124\n"
                "HEPH_GCP_COOKING request=UNKNOWN_REQUEST body=UNKNOWN_BODY rc=0\n"
                "HEPH_GCP_COOKING {\"BODY\":\"UNKNOWN_JSON_BODY\"}\n"
                "HEPH_GCP_COOKING event=phase-pass phase=smoke rc=0\n",
                encoding="utf-8",
            )
            runtime = root_path / "runtime-structured.json"
            runtime.write_text(
                json.dumps(
                    {
                        "test_result": 42,
                        "diagnostics_result": "ready",
                        "phase": "smoke",
                        "revision": "a" * 40,
                        "request": {"body": "UNKNOWN_BODY"},
                    }
                )
                + "\n",
                encoding="utf-8",
            )
            snapshot = root_path / "lineage.jsonl"
            snapshot.write_text(
                json.dumps(
                    {
                        "event_id": "00000000-0000-0000-0000-000000000001",
                        "attempt_id": "00000000-0000-0000-0000-000000000002",
                        "attempt_run_id": "00000000-0000-0000-0000-000000000003",
                        "attempt_number": 1,
                        "attempt_state": "failed",
                        "attempt_completed_at": None,
                        "run_state": "cleaned_up",
                        "run_outcome": None,
                        "disposition": "retryable",
                        "next_eligible_at": None,
                        "terminal_at": None,
                        "sampled_at": "2026-09-11T00:00:00Z",
                    }
                )
                + "\n",
                encoding="utf-8",
            )
            snapshot_status = root_path / "lineage-status.json"
            snapshot_status.write_text(
                json.dumps(
                    {
                        "schema": 1,
                        "status": "query_timeout",
                        "sampled_at": "2026-09-11T00:00:00Z",
                        "mailbox_id": "00000000-0000-0000-0000-000000000004",
                        "event_id": None,
                        "rows": 1,
                    }
                ),
                encoding="utf-8",
            )
            output = root_path / "bundle"
            archive = root_path / "bundle.tar.gz"
            self.assertEqual(
                COLLECTOR.collect(
                    output,
                    [f"serial={source}"],
                    snapshot,
                    snapshot_status,
                    archive,
                ),
                0,
            )
            manifest = json.loads((output / "manifest.json").read_text(encoding="utf-8"))
            self.assertEqual(manifest["credentialScan"], "passed")
            self.assertEqual(manifest["collectionErrors"], [])
            self.assertTrue(archive.is_file())
            self.assertNotIn("payload", (output / "lineage.jsonl").read_text(encoding="utf-8"))
            status = json.loads((output / "lineage-status.json").read_text(encoding="utf-8"))
            self.assertEqual(status["status"], "query_timeout")
            runtime_output = root_path / "runtime-output"
            self.assertEqual(
                COLLECTOR.collect(
                    runtime_output,
                    [f"runtime-structured={runtime}"],
                    None,
                    None,
                    None,
                ),
                0,
            )
            retained_runtime = (runtime_output / "sources/runtime-structured").read_text()
            self.assertIn('"test_result":42', retained_runtime)
            self.assertNotIn("UNKNOWN_BODY", retained_runtime)

    def test_projects_strict_rust_test_results_and_keeps_panic_location_informational(self):
        with tempfile.TemporaryDirectory() as root:
            root_path = Path(root)
            source = root_path / "test-output.log"
            source.write_text(
                "test examples::cooking::tests::scenario::mailbox_retry ... FAILED\n"
                "thread 'scenario::mailbox_retry' panicked at examples/cooking/tests/scenario.rs:1903:17: secret body\n"
                "test examples::cooking::tests::scenario::healthy ... ok\n"
                "test examples::cooking::tests::scenario::ignored_case ... ignored\n"
                "thread 'caught' panicked at examples/cooking/tests/scenario.rs:42:3\n",
                encoding="utf-8",
            )
            output = root_path / "bundle"
            self.assertEqual(
                COLLECTOR.collect(output, [f"test-output={source}"], None, None, None),
                0,
            )
            retained = (output / "sources/test-output").read_text(encoding="utf-8")
            self.assertIn(
                "HEPH_GCP_TEST test=examples::cooking::tests::scenario::mailbox_retry status=failed",
                retained,
            )
            self.assertIn(
                "HEPH_GCP_TEST test=examples::cooking::tests::scenario::healthy status=passed",
                retained,
            )
            self.assertIn(
                "HEPH_GCP_TEST test=examples::cooking::tests::scenario::ignored_case status=ignored",
                retained,
            )
            self.assertIn("location=examples/cooking/tests/scenario.rs:1903:17", retained)
            self.assertIn("location=examples/cooking/tests/scenario.rs:42:3", retained)
            self.assertNotIn("panicked", retained)
            self.assertNotIn("secret body", retained)

            panic_only = root_path / "panic-only.log"
            panic_only.write_text(
                "thread 'caught' panicked at examples/cooking/tests/scenario.rs:7:9: detail\n",
                encoding="utf-8",
            )
            panic_output = root_path / "panic-bundle"
            self.assertEqual(
                COLLECTOR.collect(panic_output, [f"test-output={panic_only}"], None, None, None),
                0,
            )
            panic_retained = (panic_output / "sources/test-output").read_text(encoding="utf-8")
            self.assertEqual(panic_retained, "location=examples/cooking/tests/scenario.rs:7:9\n")

    def test_rejects_symlink_and_raw_browser_trace(self):
        with tempfile.TemporaryDirectory() as root:
            root_path = Path(root)
            target = root_path / "safe.log"
            target.write_text("safe\n", encoding="utf-8")
            link = root_path / "trace.network"
            link.symlink_to(target)
            with self.assertRaises(COLLECTOR.CollectionError):
                COLLECTOR.collect(root_path / "bundle", [f"serial={link}"], None, None, None)
            raw = root_path / "trace.log"
            raw.write_text("safe\n", encoding="utf-8")
            with self.assertRaises(COLLECTOR.CollectionError):
                COLLECTOR.collect(root_path / "bundle-raw", [f"serial={raw}"], None, None, None)
            nested = root_path / "nested"
            nested.mkdir()
            ancestor = root_path / "ancestor"
            ancestor.symlink_to(nested, target_is_directory=True)
            with self.assertRaises(COLLECTOR.CollectionError):
                COLLECTOR.collect(
                    root_path / "bundle-ancestor",
                    [f"serial={ancestor / 'missing.log'}"],
                    None,
                    None,
                    None,
                )

    def test_rejects_oversize_and_sensitive_snapshot_fields(self):
        with tempfile.TemporaryDirectory() as root:
            root_path = Path(root)
            source = root_path / "serial.log"
            source.write_text("x\n", encoding="utf-8")
            snapshot = root_path / "lineage.jsonl"
            snapshot.write_text(json.dumps({"event_id": "payload"}) + "\n", encoding="utf-8")
            with self.assertRaises(COLLECTOR.CollectionError):
                COLLECTOR.collect(root_path / "bundle", [f"serial={source}"], snapshot, None, None)
            old_limit = COLLECTOR.MAX_SOURCE_BYTES
            try:
                COLLECTOR.MAX_SOURCE_BYTES = 1
                with self.assertRaises(COLLECTOR.CollectionError):
                    COLLECTOR.collect(root_path / "bundle-large", [f"serial={source}"], None, None, None)
            finally:
                COLLECTOR.MAX_SOURCE_BYTES = old_limit

    def test_fails_closed_on_fixture_credential_and_missing_source(self):
        with tempfile.TemporaryDirectory() as root:
            root_path = Path(root)
            secret = root_path / "serial.log"
            secret.write_bytes(COLLECTOR.EVIDENCE.VALUES[0] + b"\n")
            with self.assertRaises(ValueError):
                COLLECTOR.collect(root_path / "bundle", [f"serial={secret}"], None, None, None)
            self.assertFalse((root_path / "bundle").exists())
            with self.assertRaises(COLLECTOR.CollectionError):
                COLLECTOR.collect(
                    root_path / "missing-bundle",
                    [f"serial={root_path / 'missing.log'}"],
                    None,
                    None,
                    None,
                )

    def test_records_missing_optional_source_without_retaining_it(self):
        with tempfile.TemporaryDirectory() as root:
            root_path = Path(root)
            source = root_path / "serial.log"
            source.write_text("safe\n", encoding="utf-8")
            output = root_path / "bundle"
            self.assertEqual(
                COLLECTOR.collect(
                    output,
                    [
                        f"serial={source}",
                        f"runtime-log={root_path / 'killed-before-log'}",
                    ],
                    None,
                    None,
                    None,
                ),
                0,
            )
            manifest = json.loads((output / "manifest.json").read_text(encoding="utf-8"))
            self.assertEqual(
                manifest["collectionErrors"],
                [{"label": "runtime-log", "status": "missing"}],
            )

    def test_quarantines_unsafe_source_and_keeps_safe_partial_bundle(self):
        with tempfile.TemporaryDirectory() as root:
            root_path = Path(root)
            unsafe = root_path / "serial.log"
            unsafe.write_bytes(COLLECTOR.EVIDENCE.VALUES[0] + b"\n")
            snapshot = root_path / "lineage.jsonl"
            snapshot.write_text(
                json.dumps(
                    {
                        "event_id": "00000000-0000-0000-0000-000000000001",
                        "attempt_id": "00000000-0000-0000-0000-000000000002",
                        "attempt_run_id": "00000000-0000-0000-0000-000000000003",
                        "attempt_number": 1,
                        "attempt_state": "failed",
                        "run_state": "failed",
                        "run_outcome": "failed",
                    }
                )
                + "\n",
                encoding="utf-8",
            )
            snapshot_status = root_path / "lineage-status.json"
            snapshot_status.write_text(
                '{"schema":1,"status":"ok","rows":1}\n', encoding="utf-8"
            )
            output = root_path / "bundle"
            archive = root_path / "bundle.tar.gz"
            self.assertEqual(
                COLLECTOR.collect(
                    output,
                    [f"serial={unsafe}"],
                    snapshot,
                    snapshot_status,
                    archive,
                ),
                0,
            )
            manifest = json.loads((output / "manifest.json").read_text(encoding="utf-8"))
            self.assertEqual(manifest["collectionStatus"], "partial")
            self.assertEqual(
                manifest["rejectedSources"],
                [{"label": "serial", "reason": "credential-scan-rejected", "status": "rejected"}],
            )
            self.assertFalse((output / "sources/serial").exists())
            self.assertTrue((output / "lineage.jsonl").is_file())
            self.assertTrue((output / "lineage-status.json").is_file())
            self.assertTrue(archive.is_file())
            self.assertNotIn(COLLECTOR.EVIDENCE.VALUES[0], archive.read_bytes())

    def test_quarantines_symlink_ancestor_and_keeps_safe_partial_bundle(self):
        with tempfile.TemporaryDirectory() as root:
            root_path = Path(root)
            safe = root_path / "safe.log"
            safe.write_text("HEPH_GCP_KVM_STARTUP event=ready\n", encoding="utf-8")
            target = root_path / "target"
            target.mkdir()
            unsafe = target / "runtime.log"
            unsafe.write_text("HEPH_GCP_KVM_STARTUP event=unsafe\n", encoding="utf-8")
            linked = root_path / "linked"
            linked.symlink_to(target, target_is_directory=True)
            output = root_path / "bundle"

            self.assertEqual(
                COLLECTOR.collect(
                    output,
                    [f"serial={safe}", f"runtime-log={linked / unsafe.name}"],
                    None,
                    None,
                    None,
                ),
                0,
            )
            manifest = json.loads((output / "manifest.json").read_text(encoding="utf-8"))
            self.assertEqual(manifest["collectionStatus"], "partial")
            self.assertEqual(
                manifest["rejectedSources"],
                [{"label": "runtime-log", "reason": "source-policy-rejected", "status": "rejected"}],
            )
            self.assertTrue((output / "sources/serial").is_file())
            self.assertFalse((output / "sources/runtime-log").exists())

    def test_snapshot_status_reads_through_safe_bounded_open(self):
        with tempfile.TemporaryDirectory() as root:
            root_path = Path(root)
            source = root_path / "lineage-status.json"
            source.write_text('{"schema":1,"status":"ok","rows":0}\n', encoding="utf-8")
            destination = root_path / "retained-status.json"

            with patch.object(Path, "read_text", side_effect=AssertionError("unsafe text read")):
                size, digest = COLLECTOR._canonical_snapshot_status(source, destination)

            retained = destination.read_bytes()
            self.assertEqual(size, len(retained))
            self.assertEqual(digest, COLLECTOR.hashlib.sha256(retained).hexdigest())
            self.assertEqual(json.loads(retained), {"schema": 1, "status": "ok", "rows": 0})

    def test_rejects_duplicate_labels_and_projects_browser_summary(self):
        with tempfile.TemporaryDirectory() as root:
            root_path = Path(root)
            first = root_path / "one.log"
            second = root_path / "two.log"
            first.write_text("HEPH_GCP_KVM_STARTUP event=one\n", encoding="utf-8")
            second.write_text("HEPH_GCP_KVM_STARTUP event=two\n", encoding="utf-8")
            with self.assertRaises(COLLECTOR.CollectionError):
                COLLECTOR.collect(
                    root_path / "duplicate",
                    [f"serial={first}", f"serial={second}"],
                    None,
                    None,
                    None,
                )
            browser = root_path / "browser.json"
            browser.write_text(
                json.dumps(
                    {
                        "status": "failed",
                        "test": "browser smoke",
                        "error": "assertion failed",
                        "stack": "at test.js:1",
                        "request_body": "must be rejected",
                    }
                ),
                encoding="utf-8",
            )
            with self.assertRaises(COLLECTOR.CollectionError):
                COLLECTOR.collect(
                    root_path / "unsafe-browser",
                    [f"browser-summary={browser}"],
                    None,
                    None,
                    None,
                )
            browser.write_text(
                json.dumps(
                    {
                        "status": "failed",
                        "test": "browser smoke",
                        "error": 'assertion failed {"request":{"body":"UNKNOWN_BODY"}}',
                        "stack": 'at test.js:1\n{"response":{"body":"UNKNOWN_RESPONSE"}}',
                    }
                ),
                encoding="utf-8",
            )
            output = root_path / "browser-bundle"
            self.assertEqual(
                COLLECTOR.collect(
                    output,
                    [f"browser-summary={browser}"],
                    None,
                    None,
                    None,
                ),
                0,
            )
            self.assertNotIn("request", (output / "sources/browser-summary").read_text())

            browser.write_text(
                json.dumps(
                    {
                        "status": "not-run",
                        "suite": "cooking-playwright",
                        "test": "browser-journey",
                        "phase": "browser",
                        "component": "browser-e2e",
                        "result_origin": "no-browser-report",
                    }
                ),
                encoding="utf-8",
            )
            no_browser_output = root_path / "browser-not-run-bundle"
            self.assertEqual(
                COLLECTOR.collect(
                    no_browser_output,
                    [f"browser-summary={browser}"],
                    None,
                    None,
                    None,
                ),
                0,
            )
            retained_no_browser = json.loads(
                (no_browser_output / "sources/browser-summary").read_text(encoding="utf-8")
            )
            self.assertEqual(retained_no_browser["status"], "not-run")
            self.assertEqual(retained_no_browser["result_origin"], "no-browser-report")

    def test_canonicalizes_quoted_ansi_denial_fields(self):
        with tempfile.TemporaryDirectory() as root:
            root_path = Path(root)
            source = root_path / "broker.log"
            source.write_text(
                '\x1b[2m2026 INFO secret_broker '
                'run_id="00000000-0000-0000-0000-000000000001" '
                'slot="model" denial_stage="session-authentication" '
                'denial_class="authentication_denied" message="UNKNOWN"\x1b[0m\n',
                encoding="utf-8",
            )
            output = root_path / "bundle"
            self.assertEqual(
                COLLECTOR.collect(output, [f"runtime-log={source}"], None, None, None), 0
            )
            retained = (output / "sources/runtime-log").read_text(encoding="utf-8")
            self.assertIn("run_id=00000000-0000-0000-0000-000000000001", retained)
            self.assertIn("denial_stage=session-authentication", retained)
            self.assertIn("denial_class=authentication_denied", retained)
            self.assertNotIn("UNKNOWN", retained)

    def test_text_projection_drops_unknown_payloads_and_multiline_http_data(self):
        with tempfile.TemporaryDirectory() as root:
            root_path = Path(root)
            source = root_path / "runtime.log"
            source.write_text(
                "HEPH_GCP_KVM_STARTUP event=ready run_id=00000000-0000-0000-0000-000000000001 "
                "reason_class=clean-exit\n"
                'ERROR: {"request":{"body":"UNKNOWN_REQUEST"}}\n'
                "Response: UNKNOWN_RESPONSE\n"
                "body: UNKNOWN_BODY\n"
                "at worker.py:42\n",
                encoding="utf-8",
            )
            output = root_path / "bundle"
            self.assertEqual(
                COLLECTOR.collect(output, [f"runtime-log={source}"], None, None, None), 0
            )
            retained = (output / "sources/runtime-log").read_text(encoding="utf-8")
            self.assertIn("event=ready", retained)
            self.assertIn("reason_class=clean-exit", retained)
            self.assertIn("location=worker.py:42", retained)
            self.assertNotIn("at worker.py:42", retained)
            self.assertNotIn("UNKNOWN_", retained)

    def test_retains_safe_vm_lifecycle_and_broker_denial_fields(self):
        with tempfile.TemporaryDirectory() as root:
            root_path = Path(root)
            source = root_path / "serial.log"
            source.write_text(
                # Extracted from /tmp/gcp-cooking-34573078719-artifact/.../
                # gcp-kvm-smoke.log; only approved fields are projected below.
                "\x1b[2m2026-09-11T07:32:43.510902Z\x1b[0m INFO "
                "\x1b[2mvm_libkrun::provider\x1b[0m: provisioning VM resources "
                "\x1b[3mvm_id\x1b[0m=\x1b[0mbuild-84b48799-7754-4058-9747-d6e891417b64 "
                "\x1b[3mvcpus\x1b[0m=\x1b[0m1\n"
                "\x1b[2m2026-09-11T07:32:43.518567Z\x1b[0m INFO "
                "\x1b[2mvm_libkrun::provider\x1b[0m: starting libkrun worker "
                "\x1b[3mvm_id\x1b[0m=\x1b[0mbuild-84b48799-7754-4058-9747-d6e891417b64\x1b[0m\n"
                "\x1b[2m2026-09-11T07:32:44.198250Z\x1b[0m INFO "
                "\x1b[2mvm_libkrun::provider\x1b[0m: guest ready "
                "\x1b[3mvm_id\x1b[0m=\x1b[0mbuild-84b48799-7754-4058-9747-d6e891417b64\x1b[0m\n"
                "\x1b[2m2026-09-11T07:32:44.315519Z\x1b[0m INFO "
                "\x1b[2mvm_libkrun::provider\x1b[0m: worker reaped "
                "\x1b[3mworker_pid\x1b[0m=\x1b[0m70720 \x1b[3mcode\x1b[0m=\x1b[0mNone "
                "\x1b[3msignal\x1b[0m=\x1b[0mSome(9)\n"
                "\x1b[2m2026-09-11T07:32:44.802141Z\x1b[0m INFO "
                "\x1b[2mvm_libkrun::provider\x1b[0m: worker reaped "
                "\x1b[3mworker_pid\x1b[0m=\x1b[0m70752 \x1b[3mcode\x1b[0m=\x1b[0mSome(0) "
                "\x1b[3msignal\x1b[0m=\x1b[0mNone\n"
                "\x1b[2m2026-09-11T07:46:25.458115Z\x1b[0m WARN "
                "\x1b[2msecret_broker\x1b[0m: secret broker request failed "
                "\x1b[3mrun_id\x1b[0m=\x1b[0m6872e5df-61fb-4e75-b6d2-fca5724a793c "
                "\x1b[3mslot\x1b[0m=\x1b[0mmodel \x1b[3merror_class\x1b[0m=\x1b[0m\"denied\"\n",
                encoding="utf-8",
            )
            output = root_path / "bundle"
            self.assertEqual(
                COLLECTOR.collect(output, [f"serial={source}"], None, None, None), 0
            )
            retained = (output / "sources/serial").read_text(encoding="utf-8")
            self.assertIn("timestamp=2026-09-11T07:32:43.510902Z", retained)
            self.assertIn("event=provision operation=provision", retained)
            self.assertIn("run_id=build-84b48799-7754-4058-9747-d6e891417b64", retained)
            self.assertIn("event=start operation=start", retained)
            self.assertIn("event=guestready operation=guestready", retained)
            self.assertIn("pid=70720 exit_signal=9", retained)
            self.assertIn("event=reap operation=reap", retained)
            self.assertIn("pid=70752 exit_code=0", retained)
            self.assertIn("run_id=6872e5df-61fb-4e75-b6d2-fca5724a793c", retained)
            self.assertIn("slot=model error_class=denied", retained)
            self.assertNotIn("vcpus=1", retained)

    def test_free_form_error_payload_is_dropped_but_safe_assertion_is_canonical(self):
        with tempfile.TemporaryDirectory() as root:
            root_path = Path(root)
            source = root_path / "runtime.log"
            source.write_text(
                'ERROR: {"ingredients":["UNKNOWN_PAYLOAD"]}\n'
                "AssertionError: expected guest-ready\n",
                encoding="utf-8",
            )
            output = root_path / "bundle"
            self.assertEqual(
                COLLECTOR.collect(output, [f"runtime-log={source}"], None, None, None), 0
            )
            retained = (output / "sources/runtime-log").read_text(encoding="utf-8")
            self.assertNotIn("UNKNOWN_PAYLOAD", retained)
            self.assertNotIn('{"ingredients"', retained)
            self.assertIn("assertion=expected guest-ready", retained)

    def test_rejects_generic_secret_assignment_before_projection(self):
        with tempfile.TemporaryDirectory() as root:
            root_path = Path(root)
            source = root_path / "runtime.log"
            source.write_text("HEPH_GCP_KVM_STARTUP token=generated-secret\n", encoding="utf-8")
            with self.assertRaises(COLLECTOR.CollectionError):
                COLLECTOR.collect(root_path / "bundle", [f"runtime-log={source}"], None, None, None)

    def test_projects_typed_evidence_scan_result_and_rejects_raw_fields(self):
        with tempfile.TemporaryDirectory() as root:
            root_path = Path(root)
            scan = root_path / "evidence-scan.json"
            scan.write_text(
                json.dumps(
                    {
                        "schema": 1,
                        "status": "failed",
                        "rule": "browser-secret-org",
                        "file_class": "content",
                        "path_sha256": "a" * 64,
                        "checked_files": 12,
                        "checked_bytes": 4096,
                    }
                )
                + "\n",
                encoding="utf-8",
            )
            output = root_path / "bundle"
            self.assertEqual(
                COLLECTOR.collect(output, [f"evidence-scan={scan}"], None, None, None), 0
            )
            retained = json.loads((output / "sources/evidence-scan").read_text(encoding="utf-8"))
            self.assertEqual(retained["rule"], "browser-secret-org")
            self.assertEqual(retained["checked_files"], 12)
            self.assertNotIn("path", retained)

            unsafe = root_path / "unsafe-scan.json"
            unsafe.write_text(
                json.dumps(
                    {
                        "schema": 1,
                        "status": "failed",
                        "rule": "fixture-credential",
                        "file_class": "content",
                        "path_sha256": "a" * 64,
                        "checked_files": 1,
                        "checked_bytes": 2,
                        "context": "raw credential context",
                    }
                ),
                encoding="utf-8",
            )
            with self.assertRaises(COLLECTOR.CollectionError):
                COLLECTOR.collect(root_path / "unsafe-bundle", [f"evidence-scan={unsafe}"], None, None, None)

    def test_projects_explicit_unavailable_evidence_scan_result(self):
        with tempfile.TemporaryDirectory() as root:
            root_path = Path(root)
            scan = root_path / "evidence-scan.json"
            scan.write_text(
                '{"schema":1,"status":"unavailable","rule":"scanner-unavailable",'
                '"file_class":"unknown","path_sha256":null,"checked_files":0,"checked_bytes":0}\n',
                encoding="utf-8",
            )
            output = root_path / "bundle"
            self.assertEqual(
                COLLECTOR.collect(output, [f"evidence-scan={scan}"], None, None, None), 0
            )
            retained = json.loads((output / "sources/evidence-scan").read_text(encoding="utf-8"))
            self.assertEqual(retained["status"], "unavailable")
            self.assertEqual(retained["rule"], "scanner-unavailable")

    def test_retains_gate_markers_at_end_of_bounded_large_log(self):
        with tempfile.TemporaryDirectory() as root:
            root_path = Path(root)
            source = root_path / "runtime.log"
            source.write_bytes(
                b"x" * (15 * 1024 * 1024)
                + b"\n"
                + b"HEPH_GCP_COOKING event=workload-result operation=cooking-workload "
                + b"phase=cooking status=failed exit_code=7\n"
            )
            output = root_path / "bundle"
            self.assertEqual(
                COLLECTOR.collect(output, [f"runtime-log={source}"], None, None, None), 0
            )
            retained = (output / "sources/runtime-log").read_text(encoding="utf-8")
            self.assertIn("event=workload-result", retained)
            self.assertIn("exit_code=7", retained)

    def test_normalizes_prefixed_gate_markers_before_generic_projection(self):
        with tempfile.TemporaryDirectory() as root:
            root_path = Path(root)
            source = root_path / "runtime.log"
            source.write_text(
                "INFO cooking status=passed "
                "HEPH_GCP_COOKING event=workload-result operation=cooking-workload "
                "phase=cooking status=failed exit_code=7\n"
                "worker HEPH_GCP_COOKING event=evidence-scan operation=evidence-scan phase=evidence "
                "status=failed exit_code=1 report_status=failed rule=browser-secret-org "
                "file_class=content path_sha256=" + "d" * 64 + " checked_files=2 checked_bytes=3\n"
                "cooking HEPH_GCP_COOKING event=browser-report-validation "
                "operation=browser-report-validation phase=evidence status=passed "
                "report_state=complete reason=complete exit_code=0\n",
                encoding="utf-8",
            )
            output = root_path / "bundle"
            self.assertEqual(
                COLLECTOR.collect(output, [f"runtime-log={source}"], None, None, None), 0
            )
            retained = (output / "sources/runtime-log").read_text(encoding="utf-8")
            self.assertIn("HEPH_GCP_COOKING event=workload-result", retained)
            self.assertIn("HEPH_GCP_COOKING event=evidence-scan", retained)
            self.assertIn("HEPH_GCP_COOKING event=browser-report-validation", retained)
            self.assertNotIn("INFO cooking status=passed", retained)


if __name__ == "__main__":
    unittest.main()
