"""Offline tests against ordinary temporary Git repositories."""

from __future__ import annotations

import subprocess
import sys
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).parent))

from git_adapter import LocalGitError, LocalGitSession, _git, _reason  # noqa: E402
from protocol import (  # noqa: E402
    ContextEntry,
    NewRunRequired,
    ProtocolError,
    Record,
    StaleAgentParent,
    TextContent,
    utc_now,
)


SESSION = "11111111-1111-4111-8111-111111111111"
FORK_SESSION = "22222222-2222-4222-8222-222222222222"
USER_A = "user:aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"
USER_B = "user:bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb"
AGENT = "agent:reference-chat"


def user_message(record_id: str, actor: str, text: str) -> Record:
    return Record(record_id, "user_message", actor, "human", actor, utc_now(), content=TextContent(text))


def assistant_message(record_id: str, input_id: str) -> Record:
    return Record(
        record_id,
        "assistant_message",
        AGENT,
        "agent",
        AGENT,
        utc_now(),
        content=TextContent("answer"),
        in_reply_to=input_id,
        correlation_id="33333333-3333-4333-8333-333333333333",
    )


class LocalGitAdapterTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory(prefix="session-chat-git-")
        self.root = Path(self.temp.name)
        self.remote = self.root / "remote.git"
        subprocess.run(["git", "init", "--bare", str(self.remote)], check=True, capture_output=True, text=True)
        seed_path = self.root / "seed"
        seed = LocalGitSession.initialize(seed_path, SESSION, human_ids=(USER_A, USER_B))
        subprocess.run(["git", "-C", str(seed_path), "remote", "add", "origin", str(self.remote)], check=True)
        seed.push()

    def tearDown(self) -> None:
        self.temp.cleanup()

    def test_git_error_exposes_only_allowlisted_metadata(self) -> None:
        secret = "Bearer session-chat-secret-value"
        helper = self.root / "fake-git"
        helper.write_text(f"#!/bin/sh\nprintf '%s\\n' '{secret}' >&2\nexit 1\n", encoding="utf-8")
        helper.chmod(0o755)
        with patch("git_adapter.GIT_BINARY", str(helper)):
            with self.assertRaises(LocalGitError) as raised:
                _git(self.root, "push", "origin", "refs/heads/main")
        error = raised.exception
        self.assertEqual((error.operation, error.reason, error.returncode), ("push", "command_failed", 1))
        self.assertNotIn(secret, str(error))

    def test_helper_failure_code_precedes_generic_git_auth_classification(self) -> None:
        self.assertEqual(
            _reason("heph_git_credential_error=authority_protection\n", 128),
            "helper_authority_protection",
        )
        self.assertEqual(
            _reason(
                "heph_git_credential_error=target\nfatal: authentication failed\n",
                128,
            ),
            "helper_target",
        )

    def test_open_reestablishes_release_identity_for_materialized_workspaces(self) -> None:
        checkout = self.clone("materialized")
        isolated_git_env = {
            "GIT_CONFIG_GLOBAL": "/dev/null",
            "GIT_CONFIG_NOSYSTEM": "1",
            "PATH": os.environ.get("PATH", ""),
        }
        for key in ("user.name", "user.email"):
            subprocess.run(
                ["git", "-C", str(checkout.path), "config", "--local", "--unset", key],
                check=False,
                capture_output=True,
                text=True,
                env=isolated_git_env,
            )
        with patch.dict(os.environ, isolated_git_env, clear=True):
            reopened = LocalGitSession.open(checkout.path)
            record = user_message("99999999-9999-4999-8999-999999999999", USER_A, "identity")
            reopened.append_human(record, reopened.head)
        self.assertEqual(reopened.visible_transcript()[-1], record)

    def clone(self, name: str) -> LocalGitSession:
        return LocalGitSession.clone(self.remote, self.root / name)

    def test_initialization_paths_and_real_history_are_deterministic(self) -> None:
        checkout = self.clone("reader")
        self.assertEqual(checkout.session_id, SESSION)
        baseline = checkout.history()
        self.assertEqual(
            [record.kind for record in baseline],
            ["session_manifest", "participant", "participant", "participant", "participant"],
        )
        paths = subprocess.run(
            ["git", "-C", str(checkout.path), "diff-tree", "--root", "--no-commit-id", "--name-only", "-r", checkout.head],
            check=True,
            capture_output=True,
            text=True,
        ).stdout.splitlines()
        self.assertEqual(paths, sorted(paths))
        self.assertIn("participants/agent%3Areference-chat.json", "\n".join(paths))

    def test_two_clones_reconcile_concurrent_human_commits(self) -> None:
        first = self.clone("first")
        second = self.clone("second")
        parent = second.head
        first_record = user_message("44444444-4444-4444-8444-444444444444", USER_A, "first")
        second_record = user_message("55555555-5555-4555-8555-555555555555", USER_B, "second")
        first.append_human(first_record, first.head)
        first.push()
        second.append_human_retry(second_record, parent)
        reader = self.clone("reader")
        self.assertEqual(
            [record.record_id for record in reader.visible_transcript()],
            [first_record.record_id, second_record.record_id],
        )
        self.assertEqual(len(reader.history()), 7)

    def test_stale_agent_publication_requires_new_run_and_real_ref_sync(self) -> None:
        human = self.clone("human")
        agent = self.clone("agent")
        old_parent = agent.head
        incoming = user_message("66666666-6666-4666-8666-666666666666", USER_A, "question")
        human.append_human(incoming, human.head)
        human.push()
        response = assistant_message("77777777-7777-4777-8777-777777777777", incoming.record_id)
        stale_run = "88888888-8888-4888-8888-888888888888"
        with self.assertRaises(StaleAgentParent):
            agent.publish_agent_response(response, old_parent, stale_run)
        with self.assertRaises(NewRunRequired):
            agent.publish_agent_response(response, old_parent, stale_run)
        agent.sync()
        agent.publish_agent_response(response, agent.head, "99999999-9999-4999-8999-999999999999")
        agent.push()
        response_paths = subprocess.run(
            ["git", "-C", str(agent.path), "diff-tree", "--no-commit-id", "--name-only", "-r", agent.head],
            check=True,
            capture_output=True,
            text=True,
        ).stdout.splitlines()
        self.assertEqual(response_paths, [".heph/session/v1/records/agent/agent%3Areference-chat/77777777-7777-4777-8777-777777777777.json"])
        reader = self.clone("reader")
        self.assertEqual(reader.visible_transcript()[-1], response)

    def test_fork_retains_reachable_records_and_drops_remote_authority(self) -> None:
        source = self.clone("source")
        incoming = user_message("aaaaaaaa-1111-4111-8111-aaaaaaaaaaaa", USER_A, "fork me")
        source.append_human(incoming, source.head)
        source.push()
        forked = source.fork(self.root / "fork", FORK_SESSION)
        self.assertEqual(forked.session_id, FORK_SESSION)
        self.assertIn(incoming, forked.history())
        self.assertEqual(len([record for record in forked.history() if record.kind == "session_manifest"]), 2)
        self.assertEqual(forked._remote_head("origin"), None)
        self.assertEqual(forked.visible_transcript()[-1], incoming)

    def test_context_reload_rebuilds_and_rejects_history_deletion(self) -> None:
        checkout = self.clone("context")
        incoming = user_message("bbbbbbbb-1111-4111-8111-bbbbbbbbbbbb", USER_A, "context")
        checkout.append_human(incoming, checkout.head)
        checkout.push()
        response = assistant_message("cccccccc-1111-4111-8111-cccccccccccc", incoming.record_id)
        entry = ContextEntry(AGENT, "summary", "bounded", utc_now())
        checkout.publish_agent_responses(
            (response,),
            checkout.head,
            "dddddddd-1111-4111-8111-dddddddddddd",
            context_entries=(entry,),
        )
        checkout._context[(AGENT, "ghost")] = entry
        checkout._load()
        self.assertEqual(tuple(item.key for item in checkout.model_context()), ("summary",))

        path = ".heph/session/v1/context/agent%3Areference-chat/summary.json"
        subprocess.run(["git", "-C", str(checkout.path), "rm", path], check=True, capture_output=True, text=True)
        subprocess.run(
            ["git", "-C", str(checkout.path), "commit", "-m", "malformed context deletion"],
            check=True,
            capture_output=True,
            text=True,
        )
        with self.assertRaises(ProtocolError):
            LocalGitSession.open(checkout.path)


if __name__ == "__main__":
    unittest.main()
