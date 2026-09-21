"""Offline tests for the release-owned session-chat protocol."""

from __future__ import annotations

import json
import sys
from pathlib import Path
import unittest
from uuid import UUID

sys.path.insert(0, str(Path(__file__).parent))

from protocol import (  # noqa: E402
    CompatibilityError,
    ContentReference,
    Conflict,
    ContextEntry,
    IdempotencyConflict,
    MalformedRecord,
    NewRunRequired,
    Record,
    SessionRepo,
    StaleAgentParent,
    StaleParent,
    TextContent,
    utc_now,
)


SESSION = "11111111-1111-4111-8111-111111111111"
OTHER_SESSION = "22222222-2222-4222-8222-222222222222"
USER_A = "user:aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"
USER_B = "user:bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb"
RELEASE = "release:reference-chat"
AGENT = "agent:reference-chat"


def user_message(record_id: str, actor: str, text: str) -> Record:
    return Record(record_id, "user_message", actor, "human", actor, utc_now(), content=TextContent(text))


def assistant_message(record_id: str, input_id: str, text: str, correlation: str) -> Record:
    return Record(
        record_id,
        "assistant_message",
        AGENT,
        "agent",
        AGENT,
        utc_now(),
        content=TextContent(text),
        in_reply_to=input_id,
        correlation_id=correlation,
    )


class ProtocolRecordTests(unittest.TestCase):
    def test_canonical_round_trip_and_ordering(self) -> None:
        repo = SessionRepo.initialize(SESSION, human_ids=(USER_A, USER_B))
        first = user_message("33333333-3333-4333-8333-333333333333", USER_A, "first")
        second = user_message("44444444-4444-4444-8444-444444444444", USER_B, "second")
        repo.append_human(first, repo.head)
        repo.append_human_retry(second, repo.baseline.commit_id)
        self.assertEqual([record.record_id for record in repo.visible_transcript()], [first.record_id, second.record_id])
        self.assertEqual(Record.from_json(first.canonical_json()), first)
        self.assertEqual(first.to_dict()["kind"], "user_message")
        with self.assertRaises(TypeError):
            first.data["mutated"] = "nope"

    def test_malformed_records_are_rejected(self) -> None:
        with self.assertRaises(MalformedRecord):
            Record.from_json("{")
        value = user_message("55555555-5555-4555-8555-555555555555", USER_A, "hello").to_dict()
        value["record_id"] = "not-a-uuid"
        with self.assertRaises(MalformedRecord):
            Record.from_dict(value)
        value = user_message("66666666-6666-4666-8666-666666666666", USER_A, "hello").to_dict()
        value["content"] = {"kind": "text", "text": ""}
        with self.assertRaises(MalformedRecord):
            Record.from_dict(value)
        value = user_message("77777777-7777-4777-8777-777777777777", USER_A, "hello").to_dict()
        value["unknown"] = True
        with self.assertRaises(MalformedRecord):
            Record.from_dict(value)
        with self.assertRaises(MalformedRecord):
            ContentReference("content:not-a-uuid", "text/plain", 1, "0" * 64)
        with self.assertRaises(MalformedRecord):
            ContentReference("content:11111111-1111-4111-8111-111111111111", 42, 1, "0" * 64)
        with self.assertRaises(MalformedRecord):
            ContentReference("content:11111111-1111-4111-8111-111111111111", "text/plain", True, "0" * 64)
        with self.assertRaises(MalformedRecord):
            TextContent("x" * (16 * 1024 + 1))

    def test_unsupported_version_is_not_guessed(self) -> None:
        value = user_message("88888888-8888-4888-8888-888888888888", USER_A, "hello").to_dict()
        value["version"] = 2
        with self.assertRaises(CompatibilityError):
            Record.from_dict(value)
        value["version"] = True
        with self.assertRaises(CompatibilityError):
            Record.from_dict(value)

    def test_response_correlation_and_idempotency(self) -> None:
        repo = SessionRepo.initialize(SESSION, human_ids=(USER_A,))
        incoming = user_message("99999999-9999-4999-8999-999999999999", USER_A, "question")
        repo.append_human(incoming, repo.head)
        response = assistant_message("aaaaaaaa-1111-4111-8111-aaaaaaaaaaaa", incoming.record_id, "answer", "bbbbbbbb-1111-4111-8111-bbbbbbbbbbbb")
        first_commit = repo.publish_agent_response(response, repo.head, "cccccccc-1111-4111-8111-cccccccccccc")
        duplicate = assistant_message("dddddddd-1111-4111-8111-dddddddddddd", incoming.record_id, "answer", "eeeeeeee-1111-4111-8111-eeeeeeeeeeee")
        self.assertEqual(repo.publish_agent_response(duplicate, repo.head, "ffffffff-1111-4111-8111-ffffffffffff"), first_commit)
        different = assistant_message("11111111-aaaa-4aaa-8aaa-111111111111", incoming.record_id, "different", "22222222-aaaa-4aaa-8aaa-222222222222")
        with self.assertRaises(IdempotencyConflict):
            repo.publish_agent_response(different, repo.head, "33333333-aaaa-4aaa-8aaa-333333333333")

    def test_concurrent_human_retry_preserves_both_records(self) -> None:
        repo = SessionRepo.initialize(SESSION, human_ids=(USER_A, USER_B))
        parent = repo.head
        first = user_message("44444444-aaaa-4aaa-8aaa-444444444444", USER_A, "one")
        second = user_message("55555555-aaaa-4aaa-8aaa-555555555555", USER_B, "two")
        repo.append_human(first, parent)
        repo.append_human_retry(second, parent)
        self.assertEqual({record.record_id for record in repo.visible_transcript()}, {first.record_id, second.record_id})
        self.assertEqual(repo.append_human_retry(second, parent), repo._commit_for(second.record_id))

    def test_stale_agent_run_requires_new_run(self) -> None:
        repo = SessionRepo.initialize(SESSION, human_ids=(USER_A,))
        incoming = user_message("66666666-aaaa-4aaa-8aaa-666666666666", USER_A, "question")
        parent = repo.head
        repo.append_human(incoming, parent)
        response = assistant_message("77777777-aaaa-4aaa-8aaa-777777777777", incoming.record_id, "answer", "88888888-aaaa-4aaa-8aaa-888888888888")
        stale_parent = parent
        other = user_message("99999999-aaaa-4aaa-8aaa-999999999999", USER_A, "concurrent")
        repo.append_human(other, repo.head)
        stale_run = "aaaaaaaa-bbbb-4bbb-8bbb-aaaaaaaaaaaa"
        with self.assertRaises(StaleAgentParent) as raised:
            repo.publish_agent_response(response, stale_parent, stale_run)
        self.assertTrue(raised.exception.retry_requires_new_run)
        with self.assertRaises(NewRunRequired):
            repo.publish_agent_response(response, repo.head, stale_run)
        accepted = repo.publish_agent_response(response, repo.head, "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb")
        self.assertIn(accepted, {commit.commit_id for commit in repo.commits_in_order()})

    def test_tombstone_hides_view_but_retains_history(self) -> None:
        repo = SessionRepo.initialize(SESSION, human_ids=(USER_A,))
        incoming = user_message("cccccccc-aaaa-4aaa-8aaa-cccccccccccc", USER_A, "remove me")
        repo.append_human(incoming, repo.head)
        repo.append_tombstone(incoming.record_id, repo.head, reason="retention")
        self.assertEqual(repo.visible_transcript(), ())
        self.assertIn(incoming, repo.history())
        self.assertTrue(any(record.kind == "tombstone" for record in repo.history()))

    def test_fork_retains_history_without_platform_authority(self) -> None:
        repo = SessionRepo.initialize(SESSION, human_ids=(USER_A,))
        incoming = user_message("dddddddd-aaaa-4aaa-8aaa-dddddddddddd", USER_A, "keep history")
        repo.append_human(incoming, repo.head)
        forked = repo.fork(OTHER_SESSION)
        self.assertEqual(forked.session_id, OTHER_SESSION)
        self.assertIn(incoming, forked.history())
        self.assertIsNone(forked.platform_authority)
        self.assertNotEqual(repo.session_id, forked.session_id)
        fork_manifest = [record for record in forked.history() if record.kind == "session_manifest"][-1]
        self.assertEqual(fork_manifest.data["session_id"], OTHER_SESSION)
        self.assertEqual(fork_manifest.data["forked_from_session_id"], SESSION)

    def test_model_context_is_separate_from_visible_transcript(self) -> None:
        repo = SessionRepo.initialize(SESSION, human_ids=(USER_A,))
        repo.write_model_context(ContextEntry(AGENT, "summary", "private model state", utc_now()))
        self.assertEqual(repo.visible_transcript(), ())
        self.assertEqual(repo.model_context(AGENT)[0].value, "private model state")
        self.assertNotIn("private model state", json.dumps([record.to_dict() for record in repo.history()]))

    def test_record_ids_are_uuid_values(self) -> None:
        repo = SessionRepo.initialize(SESSION)
        for record in repo.history():
            UUID(record.record_id)


if __name__ == "__main__":
    unittest.main()
