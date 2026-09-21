"""Small real-local-Git adapter for the release-owned session-chat protocol.

This is deliberately a release example. It shells out to the ordinary Git
executable, stores only the locked protocol paths, and does not grant or
emulate Hephaestus runtime authority.
"""

from __future__ import annotations

from pathlib import Path
import os
import subprocess
from typing import Iterable
from uuid import UUID, uuid4

from protocol import (
    Conflict,
    MAIN_REF,
    MalformedRecord,
    NewRunRequired,
    ProtocolError,
    Record,
    SessionRepo,
    StaleAgentParent,
    StaleParent,
    path_for_record,
    utc_now,
)


class LocalGitError(RuntimeError):
    """An ordinary Git command failed in the example repository."""


def _git(path: Path, *arguments: str, check: bool = True) -> str:
    completed = subprocess.run(
        ["git", "-C", str(path), *arguments],
        check=False,
        capture_output=True,
        text=True,
        env={**os.environ, "GIT_TERMINAL_PROMPT": "0"},
    )
    if check and completed.returncode:
        detail = completed.stderr.strip() or completed.stdout.strip()
        raise LocalGitError(f"git {' '.join(arguments)} failed: {detail}")
    return completed.stdout.strip()


def _write(path: Path, value: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(value + "\n", encoding="utf-8")


class LocalGitSession:
    """Protocol-aware view over one ordinary local Git checkout."""

    def __init__(self, path: Path) -> None:
        self.path = path.resolve()
        self._records: tuple[Record, ...] = ()
        self._by_id: dict[str, Record] = {}
        self._participants: dict[str, str] = {}
        self._stale_runs: set[str] = set()
        self._session_id = ""
        self._release_id = ""
        self._agent_id = ""

    @classmethod
    def initialize(
        cls,
        path: Path,
        session_id: str,
        release_id: str = "release:reference-chat",
        agent_id: str = "agent:reference-chat",
        human_ids: Iterable[str] = (),
    ) -> "LocalGitSession":
        path = path.resolve()
        path.mkdir(parents=True, exist_ok=False)
        _git(path, "init", "-b", "main")
        _git(path, "config", "user.name", "Reference Chat Release")
        _git(path, "config", "user.email", "reference-chat@example.invalid")
        model = SessionRepo.initialize(session_id, release_id, agent_id, human_ids)
        adapter = cls(path)
        adapter._write_commit(model.baseline.records, "session initialization")
        return cls.open(path)

    @classmethod
    def clone(cls, remote: Path, destination: Path) -> "LocalGitSession":
        destination = destination.resolve()
        subprocess.run(
            ["git", "clone", "--branch", "main", str(remote.resolve()), str(destination)],
            check=True,
            capture_output=True,
            text=True,
            env={**os.environ, "GIT_TERMINAL_PROMPT": "0"},
        )
        _git(destination, "config", "user.name", "Reference Chat Release")
        _git(destination, "config", "user.email", "reference-chat@example.invalid")
        return cls.open(destination)

    @classmethod
    def open(cls, path: Path) -> "LocalGitSession":
        adapter = cls(path)
        adapter._load()
        return adapter

    @property
    def head(self) -> str:
        return _git(self.path, "rev-parse", MAIN_REF)

    @property
    def session_id(self) -> str:
        return self._session_id

    @property
    def release_id(self) -> str:
        return self._release_id

    @property
    def agent_id(self) -> str:
        return self._agent_id

    def history(self) -> tuple[Record, ...]:
        return self._records

    def visible_transcript(self) -> tuple[Record, ...]:
        tombstoned = {record.tombstone_of for record in self._records if record.kind == "tombstone"}
        return tuple(
            record
            for record in self._records
            if record.kind in {"user_message", "assistant_message"} and record.record_id not in tombstoned
        )

    def _load(self) -> None:
        if _git(self.path, "symbolic-ref", "-q", "HEAD") != MAIN_REF:
            raise ProtocolError("session checkout must have refs/heads/main checked out")
        commit_ids = _git(self.path, "rev-list", "--first-parent", "--reverse", MAIN_REF).splitlines()
        if not commit_ids:
            raise ProtocolError("session repository has no main history")
        records: list[Record] = []
        seen_paths: dict[str, str] = {}
        for commit_id in commit_ids:
            paths = _git(self.path, "diff-tree", "--root", "--no-commit-id", "--name-only", "-r", commit_id).splitlines()
            if paths != sorted(set(paths)):
                raise ProtocolError("commit changed paths are not unique and sorted")
            for path in paths:
                if not path.startswith(".heph/session/v1/") or not path.endswith(".json"):
                    continue
                value = _git(self.path, "show", f"{commit_id}:{path}")
                if path.endswith("/context.json") or "/context/" in path or "/content/" in path:
                    continue
                record = Record.from_json(value)
                if path_for_record(record) != path:
                    raise MalformedRecord(f"record {record.record_id} is stored at the wrong path")
                previous = seen_paths.get(path)
                if previous is None:
                    records.append(record)
                elif previous != record.canonical_json():
                    if path != ".heph/session/v1/manifest.json":
                        raise ProtocolError(f"immutable record path changed: {path}")
                    records.append(record)
                seen_paths[path] = record.canonical_json()
        self._records = tuple(records)
        self._by_id = {}
        for record in self._records:
            previous = self._by_id.get(record.record_id)
            if previous is not None and previous.canonical_json() != record.canonical_json():
                raise MalformedRecord("history contains conflicting record IDs")
            self._by_id[record.record_id] = record
        participant_records = [record for record in self._records if record.kind == "participant"]
        self._participants = {record.participant_id: record.data["role"] for record in participant_records}
        manifests = [record for record in self._records if record.kind == "session_manifest"]
        if not manifests:
            raise ProtocolError("session has no manifest")
        latest = manifests[-1]
        self._session_id = latest.data["session_id"]
        self._release_id = latest.data["release_id"]
        self._agent_id = latest.data["agent_id"]
        for record in self._records:
            if record.kind in {"session_manifest", "participant"}:
                continue
            if self._participants.get(record.actor_id) != record.actor_role:
                raise ProtocolError("record actor is not a registered participant")
            if record.kind == "assistant_message":
                target = self._by_id.get(record.in_reply_to or "")
                if target is None or target.kind != "user_message":
                    raise ProtocolError("assistant response has no user-message target")

    def _write_commit(self, records: Iterable[Record], message: str) -> str:
        ordered = tuple(sorted(records, key=lambda record: path_for_record(record).encode("utf-8")))
        paths: list[str] = []
        for record in ordered:
            relative = path_for_record(record)
            _write(self.path / relative, record.canonical_json())
            paths.append(relative)
        _git(self.path, "add", "--", *paths)
        _git(self.path, "commit", "--no-edit", "-m", message)
        changed = _git(self.path, "diff-tree", "--root", "--no-commit-id", "--name-only", "-r", self.head).splitlines()
        if changed != sorted(set(changed)):
            raise ProtocolError("Git commit changed paths are not sorted")
        if changed != paths:
            raise ProtocolError(f"unexpected changed paths: {changed!r}")
        return self.head

    def _assert_parent(self, expected_parent: str) -> None:
        if self.head != expected_parent:
            raise StaleParent(f"expected {expected_parent}, current local head is {self.head}")

    def append_human(self, record: Record, expected_parent: str) -> str:
        if record.kind != "user_message" or record.actor_role != "human":
            raise ProtocolError("append_human accepts only user messages")
        if self._participants.get(record.actor_id) != "human":
            raise ProtocolError("human actor is not registered")
        previous = self._by_id.get(record.record_id)
        if previous is not None:
            if previous.canonical_json() == record.canonical_json():
                return self._commit_for(record.record_id)
            raise Conflict("record ID already exists with a different payload")
        self._assert_parent(expected_parent)
        commit = self._write_commit((record,), "user message")
        self._load()
        return self.head

    def append_human_retry(self, record: Record, expected_parent: str, remote: str = "origin") -> str:
        """Commit a human record, rebase onto a concurrent remote, and push."""

        self.append_human(record, expected_parent)
        self.push_reconciled(remote)
        return self.head

    def publish_agent_response(self, record: Record, expected_parent: str, run_id: str, remote: str = "origin") -> str:
        try:
            canonical_run_id = str(UUID(run_id))
        except (ValueError, AttributeError) as exc:
            raise ProtocolError("run_id must be a canonical UUID") from exc
        if run_id != canonical_run_id:
            raise ProtocolError("run_id must use lowercase canonical UUID form")
        if run_id in self._stale_runs:
            raise NewRunRequired("stale runtime must retry with a new run")
        if record.kind != "assistant_message" or record.actor_id != self._agent_id:
            raise ProtocolError("agent publication must use this session's agent")
        remote_head = self._remote_head(remote)
        if self.head != expected_parent or (remote_head is not None and remote_head != expected_parent):
            self._stale_runs.add(run_id)
            raise StaleAgentParent("runtime expected parent is stale; start a new run")
        if self._participants.get(record.actor_id) != "agent":
            raise ProtocolError("agent actor is not registered")
        target = self._by_id.get(record.in_reply_to or "")
        if target is None or target.kind != "user_message":
            raise ProtocolError("assistant response has no user-message target")
        prior = next(
            (value for value in self._records if value.kind == "assistant_message" and value.in_reply_to == record.in_reply_to),
            None,
        )
        if prior is not None:
            if prior.content is not None and record.content is not None and prior.content.to_dict() == record.content.to_dict():
                return self._commit_for(prior.record_id)
            raise Conflict("input already has a different assistant response")
        commit = self._write_commit((record,), "assistant response")
        self._load()
        return commit

    def push(self, remote: str = "origin") -> None:
        _git(self.path, "push", remote, f"{MAIN_REF}:{MAIN_REF}")

    def fetch(self, remote: str = "origin") -> str:
        _git(self.path, "fetch", remote, "main")
        return _git(self.path, "rev-parse", f"refs/remotes/{remote}/main")

    def _remote_head(self, remote: str) -> str | None:
        result = _git(self.path, "ls-remote", remote, MAIN_REF, check=False)
        if not result:
            return None
        return result.split()[0]

    def push_reconciled(self, remote: str = "origin") -> None:
        remote_head = self.fetch(remote)
        if self.head != remote_head:
            _git(self.path, "rebase", f"refs/remotes/{remote}/main")
            self._load()
        self.push(remote)

    def sync(self, remote: str = "origin") -> str:
        remote_head = self.fetch(remote)
        if self.head != remote_head:
            _git(self.path, "merge", "--ff-only", f"refs/remotes/{remote}/main")
            self._load()
        return self.head

    def _commit_for(self, record_id: str) -> str:
        for commit_id in _git(self.path, "rev-list", "--first-parent", "--reverse", MAIN_REF).splitlines():
            paths = _git(self.path, "diff-tree", "--root", "--no-commit-id", "--name-only", "-r", commit_id).splitlines()
            for path in paths:
                if path.endswith(".json") and "/records/" in path:
                    record = Record.from_json(_git(self.path, "show", f"{commit_id}:{path}"))
                    if record.record_id == record_id:
                        return commit_id
        raise ProtocolError("record is not reachable")

    def fork(self, destination: Path, new_session_id: str) -> "LocalGitSession":
        destination = destination.resolve()
        subprocess.run(
            ["git", "clone", "--local", str(self.path), str(destination)],
            check=True,
            capture_output=True,
            text=True,
            env={**os.environ, "GIT_TERMINAL_PROMPT": "0"},
        )
        forked = self.open(destination)
        if _git(destination, "remote", check=False):
            _git(destination, "remote", "remove", "origin")
        manifest = Record(
            record_id=str(uuid4()),
            kind="session_manifest",
            actor_id=forked.release_id,
            actor_role="release",
            participant_id=forked.release_id,
            created_at=utc_now(),
            data={
                "session_id": new_session_id,
                "release_id": forked.release_id,
                "agent_id": forked.agent_id,
                "ref": MAIN_REF,
                "forked_from_session_id": forked.session_id,
            },
        )
        forked._write_commit((manifest,), "fork session")
        return self.open(destination)
