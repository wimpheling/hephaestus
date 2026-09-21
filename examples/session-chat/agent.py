#!/usr/bin/env python3
"""Reference release agent for the Git-backed session-chat protocol.

The entry point reads one capability-scoped repository checkout, resolves every
pending human turn, and publishes all responses in one expected-parent commit
and one Git push. Model traffic uses the private brokered-egress vsock ABI; the
released process has no ambient HTTPS fallback.
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path
import socket
import struct
import sys
from typing import Any, Callable
from uuid import UUID, uuid4

from git_adapter import LocalGitSession
from protocol import ContextEntry, MAIN_REF, Record, TextContent, utc_now


CONTROL_ROOT = Path("/run/hephaestus")
SECRETS_ROOT = Path("/run/hephaestus-secrets")
BROKER_CID = 2
BROKER_PORT = 19001
MAX_FRAME_BYTES = 1024 * 1024
MAX_MODEL_RESPONSE_BYTES = 16 * 1024
MODEL_DESTINATION = "api.model.example"
MODEL_PATH = "/v1/chat"
MODEL_SLOT = "model"
MODEL_PLACEHOLDER_PREFIX = "heph-placeholder:v1:"


class AgentError(RuntimeError):
    """The reference agent cannot safely complete this run."""


def _encoded(value: Any) -> bytes:
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode()


def _read_json(path: Path) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        raise AgentError(f"invalid control document: {path.name}") from exc
    if not isinstance(value, dict):
        raise AgentError(f"control document is not an object: {path.name}")
    return value


def _canonical_uuid(value: Any, field: str) -> str:
    if not isinstance(value, str):
        raise AgentError(f"{field} is not a UUID")
    try:
        canonical = str(UUID(value))
    except (AttributeError, ValueError) as exc:
        raise AgentError(f"{field} is not a UUID") from exc
    if canonical != value:
        raise AgentError(f"{field} is not canonical")
    return canonical


def _read_exact(stream: Any, length: int) -> bytes:
    data = bytearray()
    while len(data) < length:
        part = stream.recv(length - len(data))
        if not part:
            raise AgentError("broker response was truncated")
        data.extend(part)
    return bytes(data)


def _read_broker_response(stream: Any) -> dict[str, Any]:
    length = struct.unpack("!I", _read_exact(stream, 4))[0]
    if not 0 < length <= MAX_FRAME_BYTES:
        raise AgentError("broker response frame is outside its bound")
    try:
        response = json.loads(_read_exact(stream, length))
    except json.JSONDecodeError as exc:
        raise AgentError("broker response is not JSON") from exc
    if not isinstance(response, dict) or set(response) != {"status", "body"}:
        raise AgentError("broker response has the wrong shape")
    return response


class BrokeredModel:
    """Model adapter over the existing host-authorized broker protocol."""

    def __init__(
        self,
        control_root: Path = CONTROL_ROOT,
        secrets_root: Path = SECRETS_ROOT,
        socket_factory: Callable[..., Any] = socket.socket,
    ) -> None:
        context = _read_json(control_root / "context.json")
        self.run_id = _canonical_uuid(context.get("run_id"), "run_id")
        parameters = _read_json(control_root / "parameters.json")
        rule_id = _canonical_uuid(parameters.get("model_rule_id"), "model_rule_id")
        try:
            credential = (secrets_root / ".runtime-credential").read_bytes()
        except OSError as exc:
            raise AgentError("runtime broker credential is unavailable") from exc
        if len(credential) != 32:
            raise AgentError("runtime broker credential has the wrong length")
        self._credential = credential
        self._rule_id = rule_id
        self._socket_factory = socket_factory

    def __call__(self, request: dict[str, Any]) -> str:
        payload = {
            "idempotency_key": request["idempotency_key"],
            "session_id": request["session_id"],
            "messages": request["messages"],
            "context": request["context"],
        }
        placeholder = MODEL_PLACEHOLDER_PREFIX + self._rule_id
        outbound = {
            "rule_id": self._rule_id,
            "method": "post",
            "path_and_query": MODEL_PATH,
            "headers": [
                {"name": "authorization", "value": "Bearer " + placeholder},
                {"name": "content-type", "value": "application/json"},
            ],
            "body": list(_encoded(payload)),
        }
        wire = {
            "credential": list(self._credential),
            "run_id": self.run_id,
            "slot": MODEL_SLOT,
            "destination": MODEL_DESTINATION,
            "operation": "https_v1",
            "body": list(_encoded(outbound)),
        }
        encoded = _encoded(wire)
        if not 0 < len(encoded) <= MAX_FRAME_BYTES:
            raise AgentError("broker request frame is outside its bound")
        try:
            with self._socket_factory(socket.AF_VSOCK, socket.SOCK_STREAM) as stream:
                stream.settimeout(15)
                stream.connect((BROKER_CID, BROKER_PORT))
                stream.sendall(struct.pack("!I", len(encoded)) + encoded)
                response = _read_broker_response(stream)
        except (OSError, struct.error) as exc:
            raise AgentError("broker model request failed") from exc
        if response["status"] != "succeeded" or not isinstance(response["body"], list):
            raise AgentError("broker model request was denied or retryable")
        try:
            body = bytes(response["body"])
        except (TypeError, ValueError) as exc:
            raise AgentError("broker model response body is invalid") from exc
        if len(body) > MAX_MODEL_RESPONSE_BYTES:
            raise AgentError("model response exceeds its bound")
        try:
            decoded = json.loads(body)
        except json.JSONDecodeError as exc:
            raise AgentError("model response is not JSON") from exc
        if not isinstance(decoded, dict) or set(decoded) != {"text"}:
            raise AgentError("model response has the wrong shape")
        text = decoded["text"]
        if not isinstance(text, str) or not text or "\x00" in text or len(text.encode()) > MAX_MODEL_RESPONSE_BYTES:
            raise AgentError("model response text is invalid")
        return text


def _message_for_record(record: Record) -> dict[str, Any] | None:
    if record.content is None:
        return None
    return {
        "record_id": record.record_id,
        "role": "assistant" if record.kind == "assistant_message" else "user",
        "content": record.content.to_dict(),
    }


def run_once(
    workspace: Path = Path("/workspace/git"),
    control_root: Path = CONTROL_ROOT,
    secrets_root: Path = SECRETS_ROOT,
    remote: str = "origin",
    model: Callable[[dict[str, Any]], str] | None = None,
) -> dict[str, Any]:
    """Resolve all pending turns in one run, commit once, and push once."""

    session = LocalGitSession.open(workspace)
    context = _read_json(control_root / "context.json")
    if context.get("git_ref") != MAIN_REF:
        raise AgentError("runtime Git ref is not refs/heads/main")
    expected_parent = context.get("commit_sha")
    if not isinstance(expected_parent, str) or session.head != expected_parent:
        raise AgentError("runtime checkout is not the authorized expected commit")
    pending = session.pending_human_messages()
    if not pending:
        return {"disposition": "idle", "responses": 0, "commit_sha": session.head}
    run_id = _canonical_uuid(context.get("run_id"), "run_id")
    call_model = model or BrokeredModel(control_root, secrets_root)
    messages: list[dict[str, Any]] = []
    model_context = {entry.key: entry.value for entry in session.model_context()}
    responses: list[Record] = []
    context_entries: list[ContextEntry] = []
    pending_ids = {human.record_id for human in pending}
    for record in session.visible_transcript():
        message = _message_for_record(record)
        if message is not None:
            messages.append(message)
        if record.kind != "user_message" or record.record_id not in pending_ids:
            continue
        request_id = f"{session.session_id}:{record.record_id}"
        text = call_model(
            {
                "idempotency_key": request_id,
                "session_id": session.session_id,
                "messages": list(messages),
                "context": dict(model_context),
            }
        )
        response = Record(
            record_id=str(uuid4()),
            kind="assistant_message",
            actor_id=session.agent_id,
            actor_role="agent",
            participant_id=session.agent_id,
            created_at=utc_now(),
            content=TextContent(text),
            in_reply_to=record.record_id,
            correlation_id=str(uuid4()),
        )
        responses.append(response)
        messages.append({"record_id": response.record_id, "role": "assistant", "content": response.content.to_dict()})
        model_context["last_response"] = text
    context_entries.append(ContextEntry(session.agent_id, "last_response", messages[-1]["content"]["text"], utc_now()))
    commit_sha = session.publish_agent_responses(
        responses,
        expected_parent,
        run_id,
        context_entries=context_entries,
        remote=remote,
    )
    session.push(remote)
    return {"disposition": "completed", "responses": len(responses), "commit_sha": commit_sha}


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--workspace", default=str(Path("/workspace/git")))
    parser.add_argument("--control", default=str(CONTROL_ROOT))
    parser.add_argument("--secrets", default=str(SECRETS_ROOT))
    parser.add_argument("--remote", default="origin")
    args = parser.parse_args(argv)
    try:
        result = run_once(Path(args.workspace), Path(args.control), Path(args.secrets), args.remote)
    except Exception as error:  # noqa: BLE001 - the guest emits only a typed-safe failure class.
        print(f"session-chat agent failed: {type(error).__name__}", file=sys.stderr)
        return 1
    print(json.dumps(result, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
