#!/usr/local/bin/python3
"""Guest-side negative-capability probe for the reference session-chat release.

This file is deliberately a test-only release entry point.  It exercises the
same runtime Git credential helper and broker ABI as ``agent.py`` with one
authorized control request followed by denied requests. The fixture supplies the authorized and comparison
repository IDs; no authority is inferred from a user-provided value.

The fixture invokes this entry point with two distinct canonical UUID options:
``--target-repository-id`` is the authorized runtime Git target and
``--other-repository-id`` is a different repository used for read and push
denials. Runtime control and secret paths remain the fixed production guest
paths, and ``HEPH_RUNTIME_GIT_HOST``/``HEPH_RUNTIME_GIT_PATH`` remain the
heph-init supplied credential-helper contract.
"""

from __future__ import annotations

import argparse
import contextlib
import io
import json
import os
from pathlib import Path
import socket
import struct
import subprocess
import sys
import tempfile
from typing import Sequence
from uuid import UUID, uuid4


CHECKS = (
    "source_checkout_absent",
    "model_authorized_control",
    "other_repository_read_denied",
    "other_repository_push_denied",
    "prohibited_path_push_denied",
    "model_destination_denied",
    "model_rule_denied",
)
GIT = "/usr/bin/git"
GIT_HOST = "127.0.0.1:19100"
GIT_TIMEOUT_SECONDS = 20
SOURCE_CHECKOUT = Path("/workspace/source")
WORKSPACE = Path("/workspace/git")
CONTROL = Path("/run/hephaestus")
SECRETS = Path("/run/hephaestus-secrets")
MODEL_RESPONSE_TEXT = "reference answer from deterministic model"
MODEL_SESSION_ID = "11111111-1111-4111-8111-111111111111"
MODEL_RECORD_ID = "22222222-2222-4222-8222-222222222222"
DESTINATION_RECORD_ID = "33333333-3333-4333-8333-333333333333"
RULE_RECORD_ID = "44444444-4444-4444-8444-444444444444"


def _uuid(value: str) -> str | None:
    try:
        parsed = UUID(value)
    except (AttributeError, ValueError):
        return None
    canonical = str(parsed)
    return canonical if canonical == value else None


def _git(arguments: Sequence[str], cwd: Path | None = None) -> subprocess.CompletedProcess[str]:
    command = [GIT]
    if cwd is not None:
        command.extend(("-C", str(cwd)))
    command.extend(arguments)
    environment = {**os.environ, "GIT_TERMINAL_PROMPT": "0"}
    return subprocess.run(
        command,
        cwd=None,
        env=environment,
        check=False,
        capture_output=True,
        text=True,
        timeout=GIT_TIMEOUT_SECONDS,
    )


def _remote(repository_id: str) -> str:
    return f"http://{GIT_HOST}/{repository_id}"


def _source_absent() -> bool:
    return not SOURCE_CHECKOUT.exists() and not SOURCE_CHECKOUT.is_symlink()


def _fixed_git_denial(result: subprocess.CompletedProcess[str]) -> bool:
    if result.returncode == 0:
        return False
    stderr = result.stderr.casefold()
    return (
        "heph_git_credential_error=target" in stderr
        or "authentication failed" in stderr
        or "could not read username" in stderr
        or "terminal prompts disabled" in stderr
    )


def _fixed_receive_denial(result: subprocess.CompletedProcess[str]) -> bool:
    return result.returncode != 0 and "runtime receive denied:" in result.stderr.casefold()


def _other_repository_denied(workspace: Path, other_repository_id: str) -> tuple[bool, bool]:
    remote = _remote(other_repository_id)
    read = _git(("ls-remote", "--heads", remote, "refs/heads/main"), cwd=workspace)
    push = _git(("push", remote, "HEAD:refs/heads/main"), cwd=workspace)
    return _fixed_git_denial(read), _fixed_git_denial(push)


def _prohibited_path_denied(target_repository_id: str) -> bool:
    with tempfile.TemporaryDirectory(prefix="heph-denied-probe-") as directory:
        checkout = Path(directory) / "target"
        cloned = _git(("clone", _remote(target_repository_id), str(checkout)))
        if cloned.returncode != 0:
            return False
        configured_name = _git(("config", "user.name", "Reference Chat Denial Probe"), cwd=checkout)
        configured_email = _git(("config", "user.email", "denial-probe@example.invalid"), cwd=checkout)
        if configured_name.returncode or configured_email.returncode:
            return False
        forbidden = checkout / ".heph" / "session" / "v1" / "records" / "human" / "denied.json"
        forbidden.parent.mkdir(parents=True, exist_ok=True)
        forbidden.write_text("denied probe\n", encoding="utf-8")
        added = _git(("add", "--", ".heph/session/v1/records/human/denied.json"), cwd=checkout)
        committed = _git(("commit", "-m", "denied path probe"), cwd=checkout)
        pushed = _git(("push", "origin", "HEAD:refs/heads/main"), cwd=checkout)
        return not added.returncode and not committed.returncode and _fixed_receive_denial(pushed)


class _CapturedSocket:
    """Proxy one real broker socket while retaining only its response bytes."""

    def __init__(self, *arguments: object) -> None:
        self._socket = socket.socket(*arguments)
        self.received = bytearray()

    def __enter__(self) -> "_CapturedSocket":
        return self

    def __exit__(self, *arguments: object) -> None:
        self._socket.close()

    def settimeout(self, timeout: float) -> None:
        self._socket.settimeout(timeout)

    def connect(self, address: object) -> None:
        self._socket.connect(address)

    def sendall(self, payload: bytes) -> None:
        self._socket.sendall(payload)

    def recv(self, length: int) -> bytes:
        payload = self._socket.recv(length)
        self.received.extend(payload)
        return payload


def _wire_status(payload: bytes) -> str | None:
    if len(payload) < 4:
        return None
    frame_length = struct.unpack("!I", payload[:4])[0]
    if frame_length == 0 or frame_length != len(payload) - 4:
        return None
    try:
        response = json.loads(payload[4:].decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError):
        return None
    if not isinstance(response, dict) or set(response) != {"status", "body"}:
        return None
    status = response.get("status")
    return status if status in {"succeeded", "denied", "retryable"} else None


def _model_request(record_id: str) -> dict[str, object]:
    return {
        "idempotency_key": f"{MODEL_SESSION_ID}:{record_id}",
        "session_id": MODEL_SESSION_ID,
        "messages": [
            {
                "record_id": record_id,
                "role": "user",
                "content": {"kind": "text", "text": "denial probe control"},
            }
        ],
        "context": {},
    }


def _broker_request_status(*, destination: str | None = None, rule_id: str | None = None) -> str | None:
    # Import the ordinary release adapter so this probe uses the production
    # control files, credential shape, frame ABI, and broker socket contract.
    sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
    import agent  # pylint: disable=import-outside-toplevel

    captured: _CapturedSocket | None = None

    def socket_factory(*arguments: object) -> _CapturedSocket:
        nonlocal captured
        captured = _CapturedSocket(*arguments)
        return captured

    model = agent.BrokeredModel(CONTROL, SECRETS, socket_factory=socket_factory)
    original_destination = agent.MODEL_DESTINATION
    record_id = DESTINATION_RECORD_ID if destination is not None else RULE_RECORD_ID
    try:
        if destination is not None:
            agent.MODEL_DESTINATION = destination
        if rule_id is not None:
            model._rule_id = rule_id  # test-only malformed capability request
        model(_model_request(record_id))
    except agent.AgentError:
        # A broker transport failure or malformed response is not evidence of
        # authorization denial; only the fixed production wire status counts.
        return _wire_status(bytes(captured.received)) if captured is not None else None
    except Exception:  # noqa: BLE001 - output must stay fixed and credential-free.
        return None
    finally:
        agent.MODEL_DESTINATION = original_destination
    return _wire_status(bytes(captured.received)) if captured is not None else None


def _broker_request_denied(*, destination: str | None = None, rule_id: str | None = None) -> bool:
    return _broker_request_status(destination=destination, rule_id=rule_id) == "denied"


def _broker_request_succeeds() -> bool:
    """Prove the same runtime credential and binding can make the declared call."""
    sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
    import agent  # pylint: disable=import-outside-toplevel

    try:
        response = agent.BrokeredModel(CONTROL, SECRETS)(_model_request(MODEL_RECORD_ID))
    except Exception:  # noqa: BLE001 - output must stay fixed and credential-free.
        return False
    return response == MODEL_RESPONSE_TEXT


def _run(target_repository_id: str, other_repository_id: str, workspace: Path) -> dict[str, bool]:
    checks: dict[str, bool] = {"source_checkout_absent": _source_absent()}
    try:
        checks["model_authorized_control"] = _broker_request_succeeds()
    except Exception:  # noqa: BLE001 - retain only fixed check status.
        checks["model_authorized_control"] = False
    try:
        other_read, other_push = _other_repository_denied(workspace, other_repository_id)
    except Exception:  # noqa: BLE001 - retain only fixed check status.
        other_read, other_push = False, False
    checks["other_repository_read_denied"] = other_read
    checks["other_repository_push_denied"] = other_push
    try:
        checks["prohibited_path_push_denied"] = _prohibited_path_denied(target_repository_id)
    except Exception:  # noqa: BLE001 - retain only fixed check status.
        checks["prohibited_path_push_denied"] = False
    try:
        checks["model_destination_denied"] = _broker_request_denied(destination="api.undeclared.invalid")
        checks["model_rule_denied"] = _broker_request_denied(rule_id=str(uuid4()))
    except Exception:  # noqa: BLE001 - retain only fixed check status.
        checks["model_destination_denied"] = False
        checks["model_rule_denied"] = False
    return checks


def main(argv: Sequence[str] | None = None) -> int:
    class QuietParser(argparse.ArgumentParser):
        def error(self, _message: str) -> None:
            raise ValueError("invalid probe arguments")

    parser = QuietParser(add_help=False)
    parser.add_argument("--target-repository-id")
    parser.add_argument("--other-repository-id")
    parser.add_argument("--workspace", default=str(WORKSPACE))
    try:
        with contextlib.redirect_stderr(io.StringIO()):
            args, unknown = parser.parse_known_args(argv)
    except (SystemExit, ValueError):
        for check in CHECKS:
            print(f"check={check} status=failed")
        return 2
    target = _uuid(args.target_repository_id or "")
    other = _uuid(args.other_repository_id or "")
    if unknown or target is None or other is None or target == other:
        for check in CHECKS:
            print(f"check={check} status=failed")
        return 2
    results = _run(target, other, Path(args.workspace))
    for check in CHECKS:
        print(f"check={check} status={'passed' if results.get(check) is True else 'failed'}")
    return 0 if all(results.get(check) is True for check in CHECKS) else 1


if __name__ == "__main__":
    raise SystemExit(main())
