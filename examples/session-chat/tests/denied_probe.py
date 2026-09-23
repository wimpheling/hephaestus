#!/usr/local/bin/python3
"""Guest-side negative-capability probe for the reference session-chat release.

This file is deliberately a test-only release entry point.  It exercises the
same runtime Git credential helper and broker ABI as ``agent.py`` with one
authorized control request followed by denied requests. The fixture supplies
the authorized target, source, and comparison repository IDs; no authority is
inferred from a user-provided value.

The fixture invokes this entry point with three distinct canonical UUID options:
``--target-repository-id`` is the authorized runtime Git target and
``--source-repository-id`` is a distinct source repository used for read and
push denials. ``--other-repository-id`` is another repository used for the
same denials. Runtime control and secret paths remain the fixed production
guest paths, and ``HEPH_RUNTIME_GIT_HOST``/``HEPH_RUNTIME_GIT_PATH`` remain the
heph-init supplied credential-helper contract.

``/workspace/source`` may be absent or an empty real directory in the base
image. A symlink, file, mount, or nonempty directory at that path is treated as
an unexpected source checkout.
"""

from __future__ import annotations

import argparse
import base64
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
    "source_repository_read_denied",
    "source_repository_push_denied",
    "other_repository_read_denied",
    "other_repository_push_denied",
    "prohibited_path_push_denied",
    "model_destination_denied",
    "model_rule_denied",
    "runtime_git_credential_absent_from_surfaces",
)
FINAL_CHECK = "runtime_git_credential_absent_from_surfaces"
GIT = "/usr/bin/git"
GIT_HOST = "127.0.0.1:19100"
GIT_TIMEOUT_SECONDS = 20
GIT_USERNAME = "heph-runtime"
GIT_TOKEN_PREFIX = "heph_git_v1_"
SOURCE_CHECKOUT = Path("/workspace/source")
WORKSPACE = Path("/workspace/git")
CONTROL = Path("/run/hephaestus")
SECRETS = Path("/run/hephaestus-secrets")
MODEL_RESPONSE_TEXT = "reference answer from deterministic model"
MODEL_SESSION_ID = "11111111-1111-4111-8111-111111111111"
MODEL_RECORD_ID = "22222222-2222-4222-8222-222222222222"
DESTINATION_RECORD_ID = "33333333-3333-4333-8333-333333333333"
RULE_RECORD_ID = "44444444-4444-4444-8444-444444444444"


class _CredentialSurfaceObserver:
    """Keep the real Git token guest-local while scanning fixed surfaces."""

    def __init__(self, token: str) -> None:
        encoded_basic = base64.b64encode(f"{GIT_USERNAME}:{token}".encode("ascii"))
        self._patterns = (token.encode("ascii"), encoded_basic, b"Basic " + encoded_basic)
        self._matched = False
        self._invalid = False

    @property
    def clean(self) -> bool:
        return not self._matched and not self._invalid

    def _scan(self, value: object) -> None:
        if value is None:
            return
        if isinstance(value, bytes):
            payload = value
        elif isinstance(value, str):
            payload = value.encode("utf-8", errors="replace")
        elif isinstance(value, dict):
            for key, item in value.items():
                self._scan(key)
                self._scan(item)
            return
        elif isinstance(value, (list, tuple)):
            for item in value:
                self._scan(item)
            return
        else:
            self._invalid = True
            return
        if any(pattern in payload for pattern in self._patterns):
            self._matched = True

    def inspect_process(
        self,
        argv: object,
        environment: object,
        *,
        stdin: object = None,
        stdout: object = None,
        stderr: object = None,
        include_stdout: bool = True,
    ) -> None:
        self._scan(argv)
        self._scan(environment)
        self._scan(stdin)
        if include_stdout:
            self._scan(stdout)
        self._scan(stderr)

    def scan_config(self, workspace: Path) -> bool:
        result = _git(("config", "--show-origin", "--null", "--list"), cwd=workspace, observer=self)
        return result.returncode == 0 and self.clean


def _canonical_runtime_git_token(value: str) -> bool:
    encoded = value.removeprefix(GIT_TOKEN_PREFIX)
    return (
        len(encoded) == 43
        and all(character.isascii() and (character.isalnum() or character in "_-") for character in encoded)
    )


def _credential_fill(remote: str, observer: _CredentialSurfaceObserver | None = None) -> str | None:
    command = [GIT, "credential", "fill"]
    environment = {**os.environ, "GIT_TERMINAL_PROMPT": "0"}
    request = f"url={remote}\n\n"
    try:
        result = subprocess.run(
            command,
            env=environment,
            input=request,
            check=False,
            capture_output=True,
            text=True,
            timeout=GIT_TIMEOUT_SECONDS,
        )
    except Exception as error:
        if observer is not None:
            observer.inspect_process(
                command,
                environment,
                stdin=request,
                stdout=getattr(error, "stdout", None),
                stderr=getattr(error, "stderr", None),
                include_stdout=False,
            )
            raise RuntimeError("observed credential helper failed") from None
        raise
    if observer is not None:
        # The helper's password stdout is the intended private helper-to-Git
        # channel. Its argv, environment, request, and stderr remain scanned.
        observer.inspect_process(
            command,
            environment,
            stdin=request,
            stderr=result.stderr,
            include_stdout=False,
        )
    if result.returncode != 0:
        return None
    fields = dict(
        line.split("=", 1)
        for line in result.stdout.splitlines()
        if "=" in line
    )
    token = fields.get("password")
    if fields.get("username") != GIT_USERNAME or token is None:
        return None
    return token if token.startswith(GIT_TOKEN_PREFIX) and _canonical_runtime_git_token(token) else None


def runtime_credential_observer(target_repository_id: str) -> _CredentialSurfaceObserver | None:
    """Acquire the helper password in the guest and return a fixed-surface scanner."""
    remote = _remote(target_repository_id)
    token = _credential_fill(remote)
    if token is None:
        return None
    observer = _CredentialSurfaceObserver(token)
    # Inspect a second real helper invocation's non-password surfaces. The
    # first invocation is used only to obtain the guest-local token.
    if _credential_fill(remote, observer) != token:
        return None
    return observer


@contextlib.contextmanager
def _observe_agent_git(observer: _CredentialSurfaceObserver | None):
    """Observe release Git subprocesses without changing the release code."""
    if observer is None:
        yield
        return
    import git_adapter  # pylint: disable=import-outside-toplevel

    original_subprocess = git_adapter.subprocess

    class SubprocessProxy:
        def run(self, *arguments: object, **kwargs: object):
            command = arguments[0] if arguments else kwargs.get("args")
            environment = kwargs.get("env")
            if environment is None:
                environment = dict(os.environ)
            observer.inspect_process(command, environment, stdin=kwargs.get("input"), include_stdout=False)
            if not observer.clean:
                raise RuntimeError("credential surface observation failed")
            try:
                completed = original_subprocess.run(*arguments, **kwargs)
            except Exception as error:
                observer.inspect_process(
                    command,
                    environment,
                    stdin=kwargs.get("input"),
                    stdout=getattr(error, "stdout", None),
                    stderr=getattr(error, "stderr", None),
                )
                raise RuntimeError("observed Git subprocess failed") from None
            observer.inspect_process(
                command,
                environment,
                stdin=kwargs.get("input"),
                stdout=completed.stdout,
                stderr=completed.stderr,
            )
            if not observer.clean:
                raise RuntimeError("credential surface observation failed")
            return completed

    git_adapter.subprocess = SubprocessProxy()
    try:
        yield
    finally:
        git_adapter.subprocess = original_subprocess


@contextlib.contextmanager
def observe_agent_git(observer: _CredentialSurfaceObserver | None):
    with _observe_agent_git(observer):
        yield


def _uuid(value: str) -> str | None:
    try:
        parsed = UUID(value)
    except (AttributeError, ValueError):
        return None
    canonical = str(parsed)
    return canonical if canonical == value else None


def _git(
    arguments: Sequence[str],
    cwd: Path | None = None,
    observer: _CredentialSurfaceObserver | None = None,
) -> subprocess.CompletedProcess[str]:
    command = [GIT]
    if cwd is not None:
        command.extend(("-C", str(cwd)))
    command.extend(arguments)
    environment = {**os.environ, "GIT_TERMINAL_PROMPT": "0"}
    try:
        result = subprocess.run(
            command,
            cwd=None,
            env=environment,
            check=False,
            capture_output=True,
            text=True,
            timeout=GIT_TIMEOUT_SECONDS,
        )
    except Exception as error:
        if observer is not None:
            observer.inspect_process(
                command,
                environment,
                stdout=getattr(error, "stdout", None),
                stderr=getattr(error, "stderr", None),
            )
            raise RuntimeError("observed Git subprocess failed") from None
        raise
    if observer is not None:
        observer.inspect_process(command, environment, stdout=result.stdout, stderr=result.stderr)
    return result


def _remote(repository_id: str) -> str:
    return f"http://{GIT_HOST}/{repository_id}"


def _source_absent() -> bool:
    try:
        SOURCE_CHECKOUT.lstat()
    except FileNotFoundError:
        return True
    except OSError:
        return False
    if not SOURCE_CHECKOUT.is_dir() or SOURCE_CHECKOUT.is_symlink() or os.path.ismount(SOURCE_CHECKOUT):
        return False
    try:
        next(SOURCE_CHECKOUT.iterdir())
    except StopIteration:
        return True
    except OSError:
        return False
    return False


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


def _repository_denied(
    workspace: Path,
    repository_id: str,
    observer: _CredentialSurfaceObserver | None = None,
) -> tuple[bool, bool]:
    remote = _remote(repository_id)
    read = _git(("ls-remote", "--heads", remote, "refs/heads/main"), cwd=workspace, observer=observer)
    push = _git(("push", remote, "HEAD:refs/heads/main"), cwd=workspace, observer=observer)
    return _fixed_git_denial(read), _fixed_git_denial(push)


def _prohibited_path_denied(
    target_repository_id: str,
    observer: _CredentialSurfaceObserver | None = None,
) -> bool:
    with tempfile.TemporaryDirectory(prefix="heph-denied-probe-") as directory:
        checkout = Path(directory) / "target"
        cloned = _git(("clone", _remote(target_repository_id), str(checkout)), observer=observer)
        if cloned.returncode != 0:
            return False
        configured_name = _git(
            ("config", "user.name", "Reference Chat Denial Probe"), cwd=checkout, observer=observer
        )
        configured_email = _git(
            ("config", "user.email", "denial-probe@example.invalid"), cwd=checkout, observer=observer
        )
        if configured_name.returncode or configured_email.returncode:
            return False
        if observer is not None and not observer.scan_config(checkout):
            return False
        forbidden = checkout / ".heph" / "session" / "v1" / "records" / "human" / "denied.json"
        forbidden.parent.mkdir(parents=True, exist_ok=True)
        forbidden.write_text("denied probe\n", encoding="utf-8")
        added = _git(
            ("add", "--", ".heph/session/v1/records/human/denied.json"),
            cwd=checkout,
            observer=observer,
        )
        committed = _git(("commit", "-m", "denied path probe"), cwd=checkout, observer=observer)
        pushed = _git(
            ("push", "origin", "HEAD:refs/heads/main"), cwd=checkout, observer=observer
        )
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


def _run(
    target_repository_id: str,
    source_repository_id: str,
    other_repository_id: str,
    workspace: Path,
    observer: _CredentialSurfaceObserver | None = None,
) -> dict[str, bool]:
    checks: dict[str, bool] = {
        "source_checkout_absent": _source_absent(),
        FINAL_CHECK: observer is not None and observer.clean,
    }
    try:
        checks["model_authorized_control"] = _broker_request_succeeds()
    except Exception:  # noqa: BLE001 - retain only fixed check status.
        checks["model_authorized_control"] = False
    try:
        source_read, source_push = _repository_denied(workspace, source_repository_id, observer)
    except Exception:  # noqa: BLE001 - retain only fixed check status.
        source_read, source_push = False, False
    checks["source_repository_read_denied"] = source_read
    checks["source_repository_push_denied"] = source_push
    try:
        other_read, other_push = _repository_denied(workspace, other_repository_id, observer)
    except Exception:  # noqa: BLE001 - retain only fixed check status.
        other_read, other_push = False, False
    checks["other_repository_read_denied"] = other_read
    checks["other_repository_push_denied"] = other_push
    try:
        checks["prohibited_path_push_denied"] = _prohibited_path_denied(
            target_repository_id, observer
        )
    except Exception:  # noqa: BLE001 - retain only fixed check status.
        checks["prohibited_path_push_denied"] = False
    try:
        checks["model_destination_denied"] = _broker_request_denied(destination="api.undeclared.invalid")
        checks["model_rule_denied"] = _broker_request_denied(rule_id=str(uuid4()))
    except Exception:  # noqa: BLE001 - retain only fixed check status.
        checks["model_destination_denied"] = False
        checks["model_rule_denied"] = False
    checks[FINAL_CHECK] = observer is not None and observer.clean
    return checks


def main(argv: Sequence[str] | None = None) -> int:
    class QuietParser(argparse.ArgumentParser):
        def error(self, _message: str) -> None:
            raise ValueError("invalid probe arguments")

    parser = QuietParser(add_help=False)
    parser.add_argument("--target-repository-id")
    parser.add_argument("--source-repository-id")
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
    source = _uuid(args.source_repository_id or "")
    other = _uuid(args.other_repository_id or "")
    if unknown or target is None or source is None or other is None or len({target, source, other}) != 3:
        for check in CHECKS:
            print(f"check={check} status=failed")
        return 2
    workspace = Path(args.workspace)
    try:
        observer = runtime_credential_observer(target)
    except Exception:  # noqa: BLE001 - retain only fixed check status.
        observer = None
    if observer is None:
        results = {check: False for check in CHECKS}
    else:
        results = _run(target, source, other, workspace, observer)
        try:
            results[FINAL_CHECK] = observer.scan_config(workspace) and observer.clean
        except Exception:  # noqa: BLE001 - retain only fixed check status.
            results[FINAL_CHECK] = False
    for check in CHECKS:
        print(f"check={check} status={'passed' if results.get(check) is True else 'failed'}")
    return 0 if all(results.get(check) is True for check in CHECKS) else 1


if __name__ == "__main__":
    raise SystemExit(main())
