"""Offline classification tests for the guest-only denial probe."""

from __future__ import annotations

import importlib.util
import json
from pathlib import Path
import socket
import struct
import subprocess
import tempfile
import threading
import unittest
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
    def test_failed_authorized_control_cannot_be_hidden_by_denials(self):
        with (
            patch.object(PROBE, "_source_absent", return_value=True),
            patch.object(PROBE, "_broker_request_succeeds", return_value=False),
            patch.object(PROBE, "_other_repository_denied", return_value=(True, True)),
            patch.object(PROBE, "_prohibited_path_denied", return_value=True),
            patch.object(PROBE, "_broker_request_denied", return_value=True),
        ):
            checks = PROBE._run(
                "11111111-1111-4111-8111-111111111111",
                "22222222-2222-4222-8222-222222222222",
                Path("/workspace/git"),
            )
        self.assertFalse(checks["model_authorized_control"])
        self.assertFalse(all(checks.values()))

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
