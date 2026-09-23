"""Focused offline tests for the ordinary reference chat release agent."""

from __future__ import annotations

import json
from pathlib import Path
import socket
import struct
import subprocess
import sys
import tempfile
import threading
import unittest
from contextlib import redirect_stderr
from io import StringIO
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).parent))

import agent  # noqa: E402
from git_adapter import LocalGitError, LocalGitSession  # noqa: E402
from protocol import MAIN_REF, Record, StaleAgentParent, TextContent, utc_now  # noqa: E402


SESSION = "11111111-1111-4111-8111-111111111111"
HUMAN = "user:aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"
AGENT = "agent:reference-chat"


def user_message(record_id: str, text: str) -> Record:
    return Record(record_id, "user_message", HUMAN, "human", HUMAN, utc_now(), content=TextContent(text))


class ReferenceAgentTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory(prefix="session-chat-agent-")
        self.root = Path(self.temp.name)
        self.remote = self.root / "remote.git"
        subprocess.run(["git", "init", "--bare", str(self.remote)], check=True, capture_output=True, text=True)
        seed = LocalGitSession.initialize(self.root / "seed", SESSION, human_ids=(HUMAN,))
        subprocess.run(["git", "-C", str(seed.path), "remote", "add", "origin", str(self.remote)], check=True)
        seed.push()
        self.control = self.root / "control"
        self.secrets = self.root / "secrets"
        self.control.mkdir()
        self.secrets.mkdir()
        (self.secrets / ".runtime-credential").write_bytes(b"c" * 32)

    def tearDown(self) -> None:
        self.temp.cleanup()

    def test_main_logs_only_typed_git_failure_metadata(self) -> None:
        secret = "https://user:password@example.invalid/private.git"
        stderr = StringIO()
        with patch.object(
            agent,
            "run_once",
            side_effect=LocalGitError("push", 1, "auth"),
        ), redirect_stderr(stderr):
            self.assertEqual(agent.main([]), 1)
        self.assertEqual(
            stderr.getvalue(),
            "session-chat agent failed: git operation=push reason=auth returncode=1\n",
        )
        self.assertNotIn(secret, stderr.getvalue())

    def context_for(self, commit_sha: str) -> None:
        (self.control / "context.json").write_text(
            json.dumps(
                {
                    "run_id": "44444444-4444-4444-8444-444444444444",
                    "git_ref": MAIN_REF,
                    "commit_sha": commit_sha,
                }
            ),
            encoding="utf-8",
        )
        (self.control / "parameters.json").write_text(
            json.dumps({"model_rule_id": "55555555-5555-4555-8555-555555555555"}), encoding="utf-8"
        )

    def add_human(self, name: str, record: Record) -> LocalGitSession:
        human = LocalGitSession.clone(self.remote, self.root / name)
        human.append_human(record, human.head)
        human.push()
        return human

    def test_pending_turns_are_modelled_and_pushed_in_one_batch(self) -> None:
        self.add_human("human-a", user_message("22222222-2222-4222-8222-222222222222", "first"))
        self.add_human("human-b", user_message("33333333-3333-4333-8333-333333333333", "second"))
        checkout = LocalGitSession.clone(self.remote, self.root / "agent")
        self.context_for(checkout.head)
        calls: list[dict[str, object]] = []

        def model(request: dict[str, object]) -> str:
            calls.append(request)
            return "bounded answer"

        result = agent.run_once(checkout.path, self.control, self.secrets, model=model)
        self.assertEqual(result["disposition"], "completed")
        self.assertEqual(result["responses"], 2)
        self.assertEqual(len(calls), 2)
        self.assertEqual(
            [message["record_id"] for message in calls[0]["messages"]],
            ["22222222-2222-4222-8222-222222222222"],
        )
        self.assertEqual(
            calls[1]["messages"][0]["record_id"],
            "22222222-2222-4222-8222-222222222222",
        )
        self.assertEqual(len(calls[1]["messages"]), 3)
        self.assertEqual(calls[1]["messages"][1]["role"], "assistant")
        self.assertEqual(calls[1]["messages"][2]["record_id"], "33333333-3333-4333-8333-333333333333")
        self.assertEqual(calls[1]["messages"][-1]["content"]["text"], "second")
        reader = LocalGitSession.clone(self.remote, self.root / "reader")
        answers = [record for record in reader.history() if record.kind == "assistant_message"]
        self.assertEqual(len(answers), 2)
        self.assertEqual(len(reader.model_context()), 1)
        self.assertNotIn("bounded answer", json.dumps([record.to_dict() for record in reader.history() if record.kind == "user_message"]))
        commits = subprocess.run(
            ["git", "-C", str(reader.path), "rev-list", "--first-parent", "--count", MAIN_REF],
            check=True,
            capture_output=True,
            text=True,
        ).stdout.strip()
        self.assertEqual(commits, "4")

    def test_stale_parent_fails_before_agent_commit(self) -> None:
        self.add_human("human-first", user_message("66666666-6666-4666-8666-666666666666", "first"))
        checkout = LocalGitSession.clone(self.remote, self.root / "agent")
        self.context_for(checkout.head)
        self.add_human("human-late", user_message("77777777-7777-4777-8777-777777777777", "late human"))
        with self.assertRaises(StaleAgentParent):
            agent.run_once(checkout.path, self.control, self.secrets, model=lambda _request: "answer")
        self.assertEqual(
            len([record for record in LocalGitSession.clone(self.remote, self.root / "reader").history() if record.kind == "assistant_message"]),
            0,
        )

    def test_brokered_model_uses_private_wire_and_no_fallback(self) -> None:
        checkout = LocalGitSession.clone(self.remote, self.root / "agent")
        self.context_for(checkout.head)
        response = json.dumps({"text": "broker answer"}, separators=(",", ":")).encode()
        wire_response = json.dumps({"status": "succeeded", "body": list(response)}, separators=(",", ":")).encode()

        client, server = socket.socketpair()
        observed: dict[str, object] = {}

        def serve() -> None:
            with server:
                header = server.recv(4)
                length = struct.unpack("!I", header)[0]
                payload = bytearray()
                while len(payload) < length:
                    payload.extend(server.recv(length - len(payload)))
                observed.update(json.loads(payload))
                server.sendall(struct.pack("!I", len(wire_response)) + wire_response)

        thread = threading.Thread(target=serve)
        thread.start()
        class ConnectedStream:
            def __init__(self, stream: socket.socket) -> None:
                self.stream = stream

            def __enter__(self) -> "ConnectedStream":
                return self

            def __exit__(self, *_args: object) -> None:
                self.stream.close()

            def settimeout(self, timeout: float) -> None:
                self.stream.settimeout(timeout)

            def connect(self, _address: tuple[int, int]) -> None:
                return None

            def sendall(self, data: bytes) -> None:
                self.stream.sendall(data)

            def recv(self, length: int) -> bytes:
                return self.stream.recv(length)

        model = agent.BrokeredModel(
            self.control,
            self.secrets,
            socket_factory=lambda *_args: ConnectedStream(client),
        )
        self.assertEqual(model({"idempotency_key": "key", "session_id": SESSION, "messages": [], "context": {}}), "broker answer")
        thread.join(timeout=2)
        wire = observed
        self.assertEqual(wire["run_id"], "44444444-4444-4444-8444-444444444444")
        self.assertEqual(wire["slot"], "model")
        self.assertEqual(wire["destination"], "api.model.example")
        self.assertEqual(wire["operation"], "https_v1")
        self.assertEqual(wire["credential"], list(b"c" * 32))
        request = json.loads(bytes(wire["body"]))
        self.assertEqual(request["rule_id"], "55555555-5555-4555-8555-555555555555")
        self.assertEqual(request["method"], "post")
        self.assertEqual(request["headers"][0]["value"], "Bearer heph-placeholder:v1:55555555-5555-4555-8555-555555555555")
        self.assertEqual(request["path_and_query"], "/v1/chat")

    def test_broker_failure_is_not_replaced_by_a_fabricated_response(self) -> None:
        self.add_human("human", user_message("77777777-7777-4777-8777-777777777777", "needs model"))
        checkout = LocalGitSession.clone(self.remote, self.root / "agent")
        self.context_for(checkout.head)
        with self.assertRaises(agent.AgentError):
            agent.run_once(checkout.path, self.control, self.secrets, model=lambda _request: (_ for _ in ()).throw(agent.AgentError("broker down")))
        reader = LocalGitSession.clone(self.remote, self.root / "reader")
        self.assertFalse(any(record.kind == "assistant_message" for record in reader.history()))


if __name__ == "__main__":
    unittest.main()
