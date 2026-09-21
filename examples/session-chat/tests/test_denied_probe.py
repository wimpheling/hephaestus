"""Offline classification tests for the guest-only denial probe."""

from __future__ import annotations

import base64
import importlib.util
import io
import json
from pathlib import Path
import socket
import struct
import subprocess
import sys
import tempfile
import threading
import types
import unittest
from contextlib import redirect_stdout
from unittest.mock import patch


PROBE_PATH = Path(__file__).with_name("denied_probe.py")
SPEC = importlib.util.spec_from_file_location("session_chat_denied_probe", PROBE_PATH)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("cannot load denial probe")
PROBE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PROBE)


def wire_frame(status: str) -> bytes:
    payload = json.dumps({"status": status, "body": []}, separators=(",", ":")).encode()
    return struct.pack("!I", len(payload)) + payload


def result(returncode: int, stderr: str) -> subprocess.CompletedProcess[str]:
    return subprocess.CompletedProcess([], returncode, "", stderr)


class DeniedProbeTests(unittest.TestCase):
    def test_credential_surface_observer_scans_each_surface_without_logging_values(self):
        token = PROBE.GIT_TOKEN_PREFIX + ("a" * 43)
        clean = PROBE._CredentialSurfaceObserver(token)
        clean.inspect_process(
            [PROBE.GIT, "-C", "/workspace/git", "push"],
            {"GIT_TERMINAL_PROMPT": "0"},
            stdin="url=http://127.0.0.1:19100/target\n\n",
            stdout="ordinary output",
            stderr="ordinary error",
        )
        self.assertTrue(clean.clean)

        surfaces = {
            "argv": ([PROBE.GIT, token], {}, "", ""),
            "environment": ([PROBE.GIT], {"LEAK": token}, "", ""),
            "stdout": ([PROBE.GIT], {}, token, ""),
            "stderr": ([PROBE.GIT], {}, "", token),
        }
        for name, (argv, environment, stdout, stderr) in surfaces.items():
            with self.subTest(surface=name):
                observer = PROBE._CredentialSurfaceObserver(token)
                observer.inspect_process(argv, environment, stdout=stdout, stderr=stderr)
                self.assertFalse(observer.clean)

        basic = base64.b64encode(f"{PROBE.GIT_USERNAME}:{token}".encode()).decode()
        observer = PROBE._CredentialSurfaceObserver(token)
        observer.inspect_process([PROBE.GIT], {}, stdout=f"Authorization: Basic {basic}")
        self.assertFalse(observer.clean)

    def test_credential_surface_observer_exempts_only_helper_password_stdout(self):
        token = PROBE.GIT_TOKEN_PREFIX + ("b" * 43)
        observer = PROBE._CredentialSurfaceObserver(token)
        observer.inspect_process(
            [PROBE.GIT, "credential", "fill"],
            {"HEPH_RUNTIME_GIT_PATH": "target"},
            stdout=f"username={PROBE.GIT_USERNAME}\npassword={token}\n",
            stderr="",
            include_stdout=False,
        )
        self.assertTrue(observer.clean)

        observer.inspect_process(
            [PROBE.GIT, "credential", "fill"],
            {"HEPH_RUNTIME_GIT_PATH": "target"},
            stdout=f"password={token}\n",
            stderr="",
        )
        self.assertFalse(observer.clean)

    def test_agent_git_proxy_returns_clean_completion_and_restores_subprocess(self):
        class FakeSubprocess:
            def run(self, command, **_kwargs):
                return subprocess.CompletedProcess(command, 0, "ordinary output", "ordinary error")

        original_subprocess = FakeSubprocess()
        module = types.SimpleNamespace(subprocess=original_subprocess)
        observer = PROBE._CredentialSurfaceObserver(
            PROBE.GIT_TOKEN_PREFIX + ("c" * 43)
        )
        with patch.dict(sys.modules, {"git_adapter": module}):
            with PROBE.observe_agent_git(observer):
                result = module.subprocess.run([PROBE.GIT, "status"], capture_output=True)
                self.assertEqual(result.returncode, 0)
                self.assertTrue(observer.clean)
            self.assertIs(module.subprocess, original_subprocess)

    def test_agent_git_proxy_scans_effective_environment_and_restores_subprocess(self):
        token = PROBE.GIT_TOKEN_PREFIX + ("c" * 43)
        calls = []

        class FakeSubprocess:
            def run(self, command, **_kwargs):
                calls.append(command)
                return subprocess.CompletedProcess(command, 0, "ordinary output", "ordinary error")

        original_subprocess = FakeSubprocess()
        module = types.SimpleNamespace(subprocess=original_subprocess)
        observer = PROBE._CredentialSurfaceObserver(token)
        with (
            patch.dict(sys.modules, {"git_adapter": module}),
            patch.dict(PROBE.os.environ, {"LEAKED_RUNTIME_TOKEN": token}),
        ):
            with self.assertRaisesRegex(RuntimeError, "^credential surface observation failed$"):
                with PROBE.observe_agent_git(observer):
                    self.assertIsNot(module.subprocess, original_subprocess)
                    module.subprocess.run([PROBE.GIT, "status"], capture_output=True)
            self.assertIs(module.subprocess, original_subprocess)
        self.assertEqual(calls, [])
        self.assertFalse(observer.clean)

    def test_agent_git_proxy_rejects_contaminated_completion_without_token_error(self):
        token = PROBE.GIT_TOKEN_PREFIX + ("e" * 43)

        class FakeSubprocess:
            def run(self, command, **_kwargs):
                return subprocess.CompletedProcess(command, 1, token, "ordinary error")

        original_subprocess = FakeSubprocess()
        module = types.SimpleNamespace(subprocess=original_subprocess)
        observer = PROBE._CredentialSurfaceObserver(token)
        with patch.dict(sys.modules, {"git_adapter": module}):
            with self.assertRaisesRegex(RuntimeError, "^credential surface observation failed$") as failure:
                with PROBE.observe_agent_git(observer):
                    module.subprocess.run([PROBE.GIT, "status"])
        self.assertNotIn(token, str(failure.exception))
        self.assertFalse(observer.clean)
        self.assertIs(module.subprocess, original_subprocess)

    def test_agent_git_proxy_scans_exception_surfaces_and_raises_fixed_error(self):
        token = PROBE.GIT_TOKEN_PREFIX + ("d" * 43)

        class RaisingSubprocess:
            def run(self, command, **_kwargs):
                raise subprocess.TimeoutExpired(command, 1, output=token, stderr=token)

        original_subprocess = RaisingSubprocess()
        module = types.SimpleNamespace(subprocess=original_subprocess)
        observer = PROBE._CredentialSurfaceObserver(token)
        with patch.dict(sys.modules, {"git_adapter": module}):
            with self.assertRaisesRegex(RuntimeError, "^observed Git subprocess failed$") as failure:
                with PROBE.observe_agent_git(observer):
                    module.subprocess.run([PROBE.GIT, "status"])
        self.assertNotIn(token, str(failure.exception))
        self.assertFalse(observer.clean)
        self.assertIs(module.subprocess, original_subprocess)

    def test_source_checkout_absent_accepts_missing_or_empty_real_directory_only(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            missing = root / "missing"
            with patch.object(PROBE, "SOURCE_CHECKOUT", missing):
                self.assertTrue(PROBE._source_absent())

            empty = root / "empty"
            empty.mkdir()
            with patch.object(PROBE, "SOURCE_CHECKOUT", empty):
                self.assertTrue(PROBE._source_absent())
                (empty / "checkout-file").write_text("source", encoding="utf-8")
                self.assertFalse(PROBE._source_absent())

            symlink = root / "symlink"
            symlink.symlink_to(empty, target_is_directory=True)
            with patch.object(PROBE, "SOURCE_CHECKOUT", symlink):
                self.assertFalse(PROBE._source_absent())

            regular_file = root / "file"
            regular_file.write_text("source", encoding="utf-8")
            with patch.object(PROBE, "SOURCE_CHECKOUT", regular_file):
                self.assertFalse(PROBE._source_absent())

            mount_candidate = root / "mount-candidate"
            mount_candidate.mkdir()
            with (
                patch.object(PROBE, "SOURCE_CHECKOUT", mount_candidate),
                patch.object(PROBE.os.path, "ismount", return_value=True),
            ):
                self.assertFalse(PROBE._source_absent())

    def test_failed_authorized_control_cannot_be_hidden_by_denials(self):
        with (
            patch.object(PROBE, "_source_absent", return_value=True),
            patch.object(PROBE, "_broker_request_succeeds", return_value=False),
            patch.object(PROBE, "_repository_denied", return_value=(True, True)),
            patch.object(PROBE, "_prohibited_path_denied", return_value=True),
            patch.object(PROBE, "_broker_request_denied", return_value=True),
        ):
            checks = PROBE._run(
                "11111111-1111-4111-8111-111111111111",
                "22222222-2222-4222-8222-222222222222",
                "33333333-3333-4333-8333-333333333333",
                Path("/workspace/git"),
            )
        self.assertFalse(checks["model_authorized_control"])
        self.assertFalse(all(checks.values()))

    def test_cli_requires_three_distinct_repositories_and_dispatches_source_and_other(self):
        target = "11111111-1111-4111-8111-111111111111"
        source = "a2222222-b222-4222-8222-222222222222"
        other = "c3333333-d333-4333-8333-333333333333"
        valid_prefix = [
            "--target-repository-id",
            target,
            "--source-repository-id",
            source,
            "--other-repository-id",
            other,
        ]
        observer = PROBE._CredentialSurfaceObserver(PROBE.GIT_TOKEN_PREFIX + ("f" * 43))
        expected = {check: True for check in PROBE.CHECKS}
        with (
            patch.object(PROBE, "runtime_credential_observer", return_value=observer),
            patch.object(observer, "scan_config", return_value=True) as scan_config,
            patch.object(PROBE, "_run", return_value=expected) as run,
        ):
            output = io.StringIO()
            with redirect_stdout(output):
                self.assertEqual(
                    PROBE.main(
                        [
                            "--target-repository-id",
                            target,
                            "--source-repository-id",
                            source,
                            "--other-repository-id",
                            other,
                            "--workspace",
                            "/workspace/git",
                        ]
                    ),
                    0,
                )
        run.assert_called_once_with(target, source, other, Path("/workspace/git"), observer)
        scan_config.assert_called_once_with(Path("/workspace/git"))
        self.assertIn("check=source_repository_read_denied status=passed", output.getvalue())

        dirty_observer = PROBE._CredentialSurfaceObserver(PROBE.GIT_TOKEN_PREFIX + ("g" * 43))
        dirty_observer.inspect_process([PROBE.GIT, "status"], {"LEAK": PROBE.GIT_TOKEN_PREFIX + ("g" * 43)})
        with (
            patch.object(PROBE, "runtime_credential_observer", return_value=dirty_observer),
            patch.object(dirty_observer, "scan_config", return_value=True),
            patch.object(PROBE, "_run", return_value=expected),
        ):
            output = io.StringIO()
            with redirect_stdout(output):
                self.assertEqual(PROBE.main(valid_prefix), 1)
        self.assertIn(f"check={PROBE.FINAL_CHECK} status=failed", output.getvalue())

        with (
            patch.object(PROBE, "runtime_credential_observer", side_effect=RuntimeError("hidden")),
            patch.object(PROBE, "_run") as run,
        ):
            output = io.StringIO()
            with redirect_stdout(output):
                self.assertEqual(PROBE.main(valid_prefix), 1)
        run.assert_not_called()
        self.assertEqual(output.getvalue().count("status=failed\n"), len(PROBE.CHECKS))

        invalid_cases = {
            "missing-target": [
                "--source-repository-id",
                source,
                "--other-repository-id",
                other,
            ],
            "missing-source": [
                "--target-repository-id",
                target,
                "--other-repository-id",
                other,
            ],
            "missing-other": [
                "--target-repository-id",
                target,
                "--source-repository-id",
                source,
            ],
            "noncanonical-source": [
                *valid_prefix[:3],
                source.upper(),
                *valid_prefix[4:],
            ],
            "duplicate-target-source": [
                "--target-repository-id",
                target,
                "--source-repository-id",
                target,
                "--other-repository-id",
                other,
            ],
            "duplicate-target-other": [
                "--target-repository-id",
                target,
                "--source-repository-id",
                source,
                "--other-repository-id",
                target,
            ],
            "duplicate-source-other": [
                "--target-repository-id",
                target,
                "--source-repository-id",
                source,
                "--other-repository-id",
                source,
            ],
        }
        for name, arguments in invalid_cases.items():
            with self.subTest(name=name), patch.object(PROBE, "_run") as run:
                output = io.StringIO()
                with redirect_stdout(output):
                    self.assertEqual(PROBE.main(arguments), 2)
                run.assert_not_called()
                self.assertEqual(output.getvalue().count("status=failed\n"), len(PROBE.CHECKS))

    def test_broker_adapter_classifies_real_model_wire_status(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            control = root / "control"
            secrets = root / "secrets"
            control.mkdir()
            secrets.mkdir()
            (control / "context.json").write_text(
                json.dumps({"run_id": "44444444-4444-4444-8444-444444444444"}), encoding="utf-8"
            )
            (control / "parameters.json").write_text(
                json.dumps({"model_rule_id": "55555555-5555-4555-8555-555555555555"}), encoding="utf-8"
            )
            (secrets / ".runtime-credential").write_bytes(b"c" * 32)

            def call(status: str) -> str | None:
                client, server = socket.socketpair()
                client.settimeout(2)
                server.settimeout(2)
                response = wire_frame(status)
                server_errors: list[BaseException] = []

                def recv_exact(length: int) -> bytes:
                    payload = bytearray()
                    while len(payload) < length:
                        chunk = server.recv(length - len(payload))
                        if not chunk:
                            raise EOFError("broker test socket closed")
                        payload.extend(chunk)
                    return bytes(payload)

                def serve() -> None:
                    try:
                        header = recv_exact(4)
                        length = struct.unpack("!I", header)[0]
                        payload = recv_exact(length)
                        if not payload:
                            raise ValueError("broker test payload was empty")
                        server.sendall(response)
                    except BaseException as error:  # propagate below the thread boundary
                        server_errors.append(error)
                    finally:
                        server.close()

                thread = threading.Thread(target=serve)
                thread.start()

                class FakeCapturedSocket:
                    def __init__(self, *_arguments: object) -> None:
                        self.received = bytearray()

                    def __enter__(self) -> "FakeCapturedSocket":
                        return self

                    def __exit__(self, *_arguments: object) -> None:
                        client.close()

                    def settimeout(self, _timeout: float) -> None:
                        client.settimeout(_timeout)

                    def connect(self, _address: object) -> None:
                        return None

                    def sendall(self, payload: bytes) -> None:
                        client.sendall(payload)

                    def recv(self, length: int) -> bytes:
                        payload = client.recv(length)
                        self.received.extend(payload)
                        return payload

                try:
                    with patch.object(PROBE, "CONTROL", control), patch.object(PROBE, "SECRETS", secrets), patch.object(
                        PROBE, "_CapturedSocket", FakeCapturedSocket
                    ):
                        actual = PROBE._broker_request_status(destination="api.undeclared.invalid")
                finally:
                    client.close()
                    thread.join(timeout=3)
                self.assertFalse(thread.is_alive())
                if server_errors:
                    raise AssertionError("broker socket fixture failed") from server_errors[0]
                return actual

            self.assertEqual(call("denied"), "denied")
            self.assertEqual(call("retryable"), "retryable")
            self.assertNotEqual(call("retryable"), "denied")

    def test_only_wire_denied_is_authorization_denial(self):
        self.assertEqual(PROBE._wire_status(wire_frame("denied")), "denied")
        self.assertNotEqual(PROBE._wire_status(wire_frame("succeeded")), "denied")
        self.assertNotEqual(PROBE._wire_status(wire_frame("retryable")), "denied")
        self.assertIsNone(PROBE._wire_status(wire_frame("unknown")))
        self.assertIsNone(PROBE._wire_status(b"\x00\x00\x00\x05{}"))
        self.assertIsNone(PROBE._wire_status(b""))

    def test_git_denial_requires_fixed_helper_or_auth_reason(self):
        self.assertTrue(PROBE._fixed_git_denial(result(1, "heph_git_credential_error=target\n")))
        self.assertTrue(PROBE._fixed_git_denial(result(128, "fatal: authentication failed\n")))
        self.assertFalse(PROBE._fixed_git_denial(result(1, "connection reset by peer\n")))
        self.assertFalse(PROBE._fixed_git_denial(result(0, "heph_git_credential_error=target\n")))

    def test_receive_denial_requires_host_hook_prefix(self):
        self.assertTrue(PROBE._fixed_receive_denial(result(1, "remote: runtime receive denied: path\n")))
        self.assertFalse(PROBE._fixed_receive_denial(result(1, "remote: connection reset\n")))
        self.assertFalse(PROBE._fixed_receive_denial(result(0, "runtime receive denied: path\n")))


if __name__ == "__main__":
    unittest.main()
