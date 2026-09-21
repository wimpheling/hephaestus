"""Pure in-memory model of the release-owned session-chat protocol v1.

The module deliberately has no Hephaestus or Git dependency.  A release adapter
can translate these records to its own Git files and invoke the same validation
and retry rules at its boundary.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from datetime import datetime, timezone
import hashlib
import json
import re
from typing import Any, Iterable, Mapping
from types import MappingProxyType
from urllib.parse import quote
from uuid import UUID, uuid4


PROTOCOL = "heph.session-chat"
VERSION = 1
MAIN_REF = "refs/heads/main"
SESSION_ROOT = ".heph/session/v1"
MAX_TEXT_BYTES = 16 * 1024
MAX_CONTENT_BYTES = 1024 * 1024
MAX_CONTEXT_BYTES = 64 * 1024
MAX_ID_LENGTH = 64

_ID_RE = re.compile(r"^(release|agent|user):[A-Za-z0-9][A-Za-z0-9._:/-]{0,62}$")
_SHA256_RE = re.compile(r"^[0-9a-f]{64}$")
_MEDIA_RE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9!#$&^_.+-]{0,62}/[A-Za-z0-9][A-Za-z0-9!#$&^_.+-]{0,62}$")
_CONTEXT_KEY_RE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$")
_TIMESTAMP_RE = re.compile(r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d{1,6})?Z$")


class ProtocolError(ValueError):
    """Base class for records that cannot be accepted by this release."""


class MalformedRecord(ProtocolError):
    """A record has invalid shape or field values."""


class CompatibilityError(ProtocolError):
    """A record uses an unsupported protocol identifier or version."""


class Conflict(ProtocolError):
    """A mutation cannot be reconciled without changing its payload."""


class StaleParent(Conflict):
    """The expected Git parent is no longer the current main ref."""

    retry_requires_new_run = False


class StaleAgentParent(StaleParent):
    """An agent's stale publication must be retried by a fresh runtime run."""

    retry_requires_new_run = True


class NewRunRequired(StaleParent):
    """An agent tried to reuse a run that already observed a stale parent."""

    retry_requires_new_run = True


class IdempotencyConflict(Conflict):
    """A second response for one input has a different payload."""


def utc_now() -> str:
    """Return the canonical UTC timestamp used by examples and tests."""

    return datetime.now(timezone.utc).isoformat(timespec="microseconds").replace("+00:00", "Z")


def _uuid(value: str, field_name: str) -> str:
    if not isinstance(value, str):
        raise MalformedRecord(f"{field_name} must be a UUID string")
    try:
        parsed = UUID(value)
    except (ValueError, AttributeError) as exc:
        raise MalformedRecord(f"{field_name} must be a canonical UUID") from exc
    canonical = str(parsed)
    if value != canonical:
        raise MalformedRecord(f"{field_name} must use lowercase canonical UUID form")
    return canonical


def _id(value: str, field_name: str) -> str:
    if not isinstance(value, str) or len(value) > MAX_ID_LENGTH or not _ID_RE.fullmatch(value):
        raise MalformedRecord(f"{field_name} has an invalid bounded participant ID")
    if value.startswith("user:"):
        _uuid(value[5:], field_name)
    return value


def _timestamp(value: str) -> str:
    if not isinstance(value, str) or not _TIMESTAMP_RE.fullmatch(value):
        raise MalformedRecord("created_at must be a UTC RFC3339 timestamp ending in Z")
    try:
        datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError as exc:
        raise MalformedRecord("created_at is not a valid timestamp") from exc
    return value


def _bounded_text(value: str, field_name: str = "text") -> str:
    if not isinstance(value, str) or not value or "\x00" in value:
        raise MalformedRecord(f"{field_name} must be non-empty text without NUL")
    if len(value.encode("utf-8")) > MAX_TEXT_BYTES:
        raise MalformedRecord(f"{field_name} exceeds {MAX_TEXT_BYTES} UTF-8 bytes")
    return value


@dataclass(frozen=True)
class TextContent:
    text: str

    def __post_init__(self) -> None:
        _bounded_text(self.text)

    def to_dict(self) -> dict[str, str]:
        return {"kind": "text", "text": self.text}


@dataclass(frozen=True)
class ContentReference:
    content_id: str
    media_type: str
    byte_length: int
    sha256: str

    def __post_init__(self) -> None:
        if not isinstance(self.content_id, str) or not self.content_id.startswith("content:"):
            raise MalformedRecord("content_id must use the content:<uuid> form")
        _uuid(self.content_id.removeprefix("content:"), "content_id")
        if not isinstance(self.media_type, str) or not _MEDIA_RE.fullmatch(self.media_type):
            raise MalformedRecord("media_type must be a bounded media type")
        if isinstance(self.byte_length, bool) or not isinstance(self.byte_length, int) or not 0 < self.byte_length <= MAX_CONTENT_BYTES:
            raise MalformedRecord("byte_length is outside the allowed content bound")
        if not isinstance(self.sha256, str) or not _SHA256_RE.fullmatch(self.sha256):
            raise MalformedRecord("sha256 must be a lowercase SHA-256 value")

    def to_dict(self) -> dict[str, Any]:
        return {
            "kind": "reference",
            "content_id": self.content_id,
            "media_type": self.media_type,
            "byte_length": self.byte_length,
            "sha256": self.sha256,
        }


Content = TextContent | ContentReference


def _content_from_dict(value: Any) -> Content:
    if not isinstance(value, dict) or not isinstance(value.get("kind"), str):
        raise MalformedRecord("content must be a text or reference object")
    if value["kind"] == "text":
        if set(value) != {"kind", "text"}:
            raise MalformedRecord("text content has the wrong fields")
        return TextContent(value["text"])
    if value["kind"] == "reference":
        if set(value) != {"kind", "content_id", "media_type", "byte_length", "sha256"}:
            raise MalformedRecord("reference content has the wrong fields")
        return ContentReference(
            value["content_id"], value["media_type"], value["byte_length"], value["sha256"]
        )
    raise MalformedRecord("unknown content kind")


def _content_equal(left: Content, right: Content) -> bool:
    return left.to_dict() == right.to_dict()


@dataclass(frozen=True)
class Record:
    record_id: str
    kind: str
    actor_id: str
    actor_role: str
    participant_id: str
    created_at: str
    content: Content | None = None
    in_reply_to: str | None = None
    correlation_id: str | None = None
    tombstone_of: str | None = None
    data: Mapping[str, str] = field(default_factory=dict)
    protocol: str = PROTOCOL
    version: int = VERSION

    def __post_init__(self) -> None:
        object.__setattr__(self, "data", MappingProxyType(dict(self.data)))
        self.validate()

    def validate(self) -> None:
        _uuid(self.record_id, "record_id")
        _timestamp(self.created_at)
        _id(self.actor_id, "actor_id")
        _id(self.participant_id, "participant_id")
        if self.protocol != PROTOCOL or isinstance(self.version, bool) or not isinstance(self.version, int) or self.version != VERSION:
            raise CompatibilityError(f"unsupported protocol {self.protocol!r} version {self.version!r}")
        if self.actor_role not in {"release", "agent", "human"}:
            raise MalformedRecord("unknown actor role")
        expected_actor_prefix = {"release": "release:", "agent": "agent:", "human": "user:"}[self.actor_role]
        if not self.actor_id.startswith(expected_actor_prefix):
            raise MalformedRecord("actor ID does not match actor role")
        if not isinstance(self.data, Mapping) or any(
            not isinstance(key, str) or not isinstance(value, str) for key, value in self.data.items()
        ):
            raise MalformedRecord("data keys and values must be strings")
        if any(len(key) > 64 or len(value.encode("utf-8")) > MAX_TEXT_BYTES for key, value in self.data.items()):
            raise MalformedRecord("data contains an unbounded key or value")
        if self.kind == "session_manifest":
            if self.actor_role != "release" or self.actor_id != self.participant_id:
                raise MalformedRecord("manifest must be release-authored")
            if set(self.data) - {"session_id", "release_id", "agent_id", "ref", "forked_from_session_id"}:
                raise MalformedRecord("manifest contains unknown data fields")
            for key in ("session_id", "release_id", "agent_id", "ref"):
                if key not in self.data:
                    raise MalformedRecord(f"manifest requires data.{key}")
            _uuid(self.data["session_id"], "data.session_id")
            _id(self.data["release_id"], "data.release_id")
            _id(self.data["agent_id"], "data.agent_id")
            if self.data["release_id"] != self.actor_id or not self.data["agent_id"].startswith("agent:"):
                raise MalformedRecord("manifest participant identities do not match its actor")
            if "forked_from_session_id" in self.data:
                _uuid(self.data["forked_from_session_id"], "data.forked_from_session_id")
            if self.data["ref"] != MAIN_REF:
                raise MalformedRecord("manifest ref must be refs/heads/main")
            if self.content is not None:
                raise MalformedRecord("manifest cannot carry content")
        elif self.kind == "participant":
            if self.actor_role != "release" or self.content is not None:
                raise MalformedRecord("participant declarations are release-written without content")
            declared_role = self.data.get("role")
            if declared_role not in {"release", "agent", "human"}:
                raise MalformedRecord("participant requires a known data.role")
            prefix = {"release": "release:", "agent": "agent:", "human": "user:"}[declared_role]
            if not self.participant_id.startswith(prefix):
                raise MalformedRecord("participant ID does not match data.role")
        elif self.kind == "user_message":
            if self.actor_role != "human" or self.actor_id != self.participant_id or self.content is None:
                raise MalformedRecord("user messages require a human actor and content")
            if self.in_reply_to is not None or self.correlation_id is not None or self.tombstone_of is not None:
                raise MalformedRecord("user messages cannot carry response or tombstone fields")
        elif self.kind == "assistant_message":
            if self.actor_role != "agent" or self.actor_id != self.participant_id or self.content is None:
                raise MalformedRecord("assistant messages require an agent actor and content")
            if self.in_reply_to is None or self.correlation_id is None:
                raise MalformedRecord("assistant messages require reply and correlation IDs")
            _uuid(self.in_reply_to, "in_reply_to")
            _uuid(self.correlation_id, "correlation_id")
            if self.tombstone_of is not None:
                raise MalformedRecord("assistant messages cannot tombstone records")
        elif self.kind == "tombstone":
            if self.actor_role != "release" or self.content is not None or self.tombstone_of is None:
                raise MalformedRecord("tombstones require a release actor and target")
            _uuid(self.tombstone_of, "tombstone_of")
        else:
            raise MalformedRecord(f"unknown record kind {self.kind!r}")

    def to_dict(self) -> dict[str, Any]:
        result: dict[str, Any] = {
            "protocol": self.protocol,
            "version": self.version,
            "record_id": self.record_id,
            "kind": self.kind,
            "actor": {"id": self.actor_id, "role": self.actor_role},
            "participant_id": self.participant_id,
            "created_at": self.created_at,
        }
        if self.content is not None:
            result["content"] = self.content.to_dict()
        if self.in_reply_to is not None:
            result["in_reply_to"] = self.in_reply_to
        if self.correlation_id is not None:
            result["correlation_id"] = self.correlation_id
        if self.tombstone_of is not None:
            result["tombstone_of"] = self.tombstone_of
        if self.data:
            result["data"] = dict(self.data)
        return result

    def canonical_json(self) -> str:
        return json.dumps(self.to_dict(), sort_keys=True, separators=(",", ":"), ensure_ascii=False)

    @classmethod
    def from_dict(cls, value: Any) -> "Record":
        if not isinstance(value, dict):
            raise MalformedRecord("record must be a JSON object")
        required = {"protocol", "version", "record_id", "kind", "actor", "participant_id", "created_at"}
        if not required.issubset(value):
            raise MalformedRecord("record is missing required fields")
        allowed = required | {"content", "in_reply_to", "correlation_id", "tombstone_of", "data"}
        if set(value) - allowed:
            raise MalformedRecord("record contains unknown fields")
        actor = value["actor"]
        if not isinstance(actor, dict) or set(actor) != {"id", "role"}:
            raise MalformedRecord("actor must contain only id and role")
        content = _content_from_dict(value["content"]) if "content" in value else None
        data = value.get("data", {})
        if not isinstance(data, dict):
            raise MalformedRecord("data must be a JSON object")
        return cls(
            record_id=value["record_id"],
            kind=value["kind"],
            actor_id=actor["id"],
            actor_role=actor["role"],
            participant_id=value["participant_id"],
            created_at=value["created_at"],
            content=content,
            in_reply_to=value.get("in_reply_to"),
            correlation_id=value.get("correlation_id"),
            tombstone_of=value.get("tombstone_of"),
            data=data,
            protocol=value["protocol"],
            version=value["version"],
        )

    @classmethod
    def from_json(cls, value: str) -> "Record":
        try:
            decoded = json.loads(value)
        except json.JSONDecodeError as exc:
            raise MalformedRecord("record is not valid JSON") from exc
        return cls.from_dict(decoded)


def _path_component(value: str) -> str:
    return quote(value, safe="")


def path_for_record(record: Record) -> str:
    """Return the locked release-owned path for one immutable record."""

    if record.kind == "session_manifest":
        return f"{SESSION_ROOT}/manifest.json"
    if record.kind == "participant":
        return f"{SESSION_ROOT}/participants/{_path_component(record.participant_id)}.json"
    if record.kind == "user_message":
        return f"{SESSION_ROOT}/records/human/{_path_component(record.record_id)}.json"
    if record.kind == "assistant_message":
        return f"{SESSION_ROOT}/records/agent/{_path_component(record.actor_id)}/{_path_component(record.record_id)}.json"
    if record.kind == "tombstone":
        return f"{SESSION_ROOT}/records/tombstones/{_path_component(record.tombstone_of or '')}.json"
    raise MalformedRecord(f"no path mapping for {record.kind!r}")


def path_for_context(agent_id: str, key: str) -> str:
    _id(agent_id, "agent_id")
    if not isinstance(key, str) or not _CONTEXT_KEY_RE.fullmatch(key):
        raise MalformedRecord("context key is not a safe path component")
    return f"{SESSION_ROOT}/context/{_path_component(agent_id)}/{_path_component(key)}.json"


def path_for_content(content_id: str) -> str:
    if not isinstance(content_id, str) or not content_id.startswith("content:"):
        raise MalformedRecord("content_id must use the content:<uuid> form")
    _uuid(content_id.removeprefix("content:"), "content_id")
    return f"{SESSION_ROOT}/content/{_path_component(content_id)}.json"


@dataclass(frozen=True)
class Commit:
    commit_id: str
    parents: tuple[str, ...]
    records: tuple[Record, ...]
    message: str


@dataclass(frozen=True)
class ContextEntry:
    agent_id: str
    key: str
    value: str
    updated_at: str

    def __post_init__(self) -> None:
        _id(self.agent_id, "agent_id")
        _bounded_text(self.key, "context key")
        _bounded_text(self.value, "context value")
        if len(self.value.encode("utf-8")) > MAX_CONTEXT_BYTES:
            raise MalformedRecord("context value exceeds its bound")
        if not _CONTEXT_KEY_RE.fullmatch(self.key):
            raise MalformedRecord("context key is not a safe path component")
        _timestamp(self.updated_at)


def _commit_id(parents: Iterable[str], records: Iterable[Record], message: str) -> str:
    payload = json.dumps(
        {"parents": list(parents), "records": [record.to_dict() for record in records], "message": message},
        sort_keys=True,
        separators=(",", ":"),
        ensure_ascii=False,
    ).encode("utf-8")
    return hashlib.sha256(payload).hexdigest()


class SessionRepo:
    """An in-memory first-parent main ref with release-owned mutation rules."""

    def __init__(self) -> None:
        self._commits: dict[str, Commit] = {}
        self._head: str | None = None
        self._session_id: str | None = None
        self._release_id: str | None = None
        self._agent_id: str | None = None
        self._participants: dict[str, str] = {}
        self._stale_runs: set[str] = set()
        self._context: dict[tuple[str, str], ContextEntry] = {}
        self.platform_authority: None = None

    @classmethod
    def initialize(
        cls,
        session_id: str,
        release_id: str = "release:reference-chat",
        agent_id: str = "agent:reference-chat",
        human_ids: Iterable[str] = (),
        created_at: str | None = None,
    ) -> "SessionRepo":
        session_id = _uuid(session_id, "session_id")
        _id(release_id, "release_id")
        _id(agent_id, "agent_id")
        if not release_id.startswith("release:") or not agent_id.startswith("agent:"):
            raise MalformedRecord("initial release and agent IDs have wrong role prefixes")
        timestamp = created_at or utc_now()
        repo = cls()
        repo._session_id, repo._release_id, repo._agent_id = session_id, release_id, agent_id
        participants = [(release_id, "release"), (agent_id, "agent")]
        participants.extend((_id(value, "human_id"), "human") for value in human_ids)
        if len({value for value, _ in participants}) != len(participants):
            raise MalformedRecord("initial participants must be unique")
        manifest = Record(
            record_id=str(uuid4()),
            kind="session_manifest",
            actor_id=release_id,
            actor_role="release",
            participant_id=release_id,
            created_at=timestamp,
            data={"session_id": session_id, "release_id": release_id, "agent_id": agent_id, "ref": MAIN_REF},
        )
        records = [manifest]
        for participant_id, role in participants:
            records.append(
                Record(
                    record_id=str(uuid4()),
                    kind="participant",
                    actor_id=release_id,
                    actor_role="release",
                    participant_id=participant_id,
                    created_at=timestamp,
                    data={"role": role},
                )
            )
        repo._commit(tuple(records), "session initialization")
        return repo

    @property
    def head(self) -> str:
        if self._head is None:
            raise ProtocolError("repository is not initialized")
        return self._head

    @property
    def session_id(self) -> str:
        if self._session_id is None:
            raise ProtocolError("repository is not initialized")
        return self._session_id

    @property
    def baseline(self) -> Commit:
        commit = self._commits[self._first_commit_id()]
        return commit

    def _first_commit_id(self) -> str:
        current = self.head
        while self._commits[current].parents:
            current = self._commits[current].parents[0]
        return current

    def _commit(self, records: tuple[Record, ...], message: str) -> str:
        records = tuple(sorted(records, key=lambda record: path_for_record(record).encode("utf-8")))
        parent = () if self._head is None else (self._head,)
        commit_id = _commit_id(parent, records, message)
        commit = Commit(commit_id, parent, records, message)
        self._commits[commit_id] = commit
        self._head = commit_id
        for record in records:
            if record.kind == "participant":
                self._participants[record.participant_id] = record.data["role"]
        return commit_id

    def _assert_parent(self, expected_parent: str) -> None:
        if expected_parent != self.head:
            raise StaleParent(f"expected parent {expected_parent}, current head is {self.head}")

    def commits_in_order(self) -> tuple[Commit, ...]:
        commits: list[Commit] = []
        current = self.head
        while True:
            commit = self._commits[current]
            commits.append(commit)
            if not commit.parents:
                return tuple(reversed(commits))
            current = commit.parents[0]

    def history(self) -> tuple[Record, ...]:
        return tuple(record for commit in self.commits_in_order() for record in commit.records)

    def _record_map(self) -> dict[str, Record]:
        result: dict[str, Record] = {}
        for record in self.history():
            previous = result.get(record.record_id)
            if previous is not None and previous.canonical_json() != record.canonical_json():
                raise MalformedRecord("reachable history contains conflicting record IDs")
            result[record.record_id] = record
        return result

    def _assert_actor(self, record: Record) -> None:
        if self._participants.get(record.actor_id) != record.actor_role:
            raise ProtocolError("record actor is not a registered participant with that role")

    def append_human(self, record: Record, expected_parent: str) -> str:
        if record.kind != "user_message" or record.actor_role != "human":
            raise ProtocolError("append_human accepts only user messages")
        self._assert_actor(record)
        existing = self._record_map().get(record.record_id)
        if existing is not None:
            if existing.canonical_json() == record.canonical_json():
                return self._commit_for(record.record_id)
            raise Conflict("record ID already exists with a different payload")
        self._assert_parent(expected_parent)
        return self._commit((record,), "user message")

    def append_human_retry(self, record: Record, expected_parent: str) -> str:
        """Retry a stale human append on current main without losing either record."""

        existing = self._record_map().get(record.record_id)
        if existing is not None:
            if existing.canonical_json() == record.canonical_json():
                return self._commit_for(record.record_id)
            raise Conflict("record ID already exists with a different payload")
        if expected_parent == self.head:
            return self.append_human(record, expected_parent)
        return self.append_human(record, self.head)

    def publish_agent_response(self, record: Record, expected_parent: str, run_id: str) -> str:
        """Publish one response; a stale run may never retry under its old ID."""

        run_id = _uuid(run_id, "run_id")
        if run_id in self._stale_runs:
            raise NewRunRequired("stale runtime must retry with a new run")
        if record.kind != "assistant_message" or record.actor_role != "agent":
            raise ProtocolError("publish_agent_response accepts only assistant messages")
        self._assert_actor(record)
        records = self._record_map()
        input_record = records.get(record.in_reply_to or "")
        if input_record is None or input_record.kind != "user_message":
            raise ProtocolError("assistant response must reference an existing user message")
        if expected_parent != self.head:
            self._stale_runs.add(run_id)
            raise StaleAgentParent("runtime expected parent is stale; start a new run")
        prior_response = next(
            (value for value in records.values() if value.kind == "assistant_message" and value.in_reply_to == record.in_reply_to),
            None,
        )
        if prior_response is not None:
            if (
                prior_response.actor_id == record.actor_id
                and prior_response.content is not None
                and record.content is not None
                and _content_equal(prior_response.content, record.content)
            ):
                return self._commit_for(prior_response.record_id)
            raise IdempotencyConflict("input already has a different assistant response")
        existing = records.get(record.record_id)
        if existing is not None:
            if existing.canonical_json() == record.canonical_json():
                return self._commit_for(record.record_id)
            raise Conflict("record ID already exists with a different payload")
        return self._commit((record,), "assistant response")

    def append_tombstone(self, target_id: str, expected_parent: str, release_id: str | None = None, reason: str = "retention") -> str:
        _uuid(target_id, "target_id")
        release_id = release_id or self._release_id
        if release_id is None:
            raise ProtocolError("repository has no release participant")
        record = Record(
            record_id=str(uuid4()),
            kind="tombstone",
            actor_id=release_id,
            actor_role="release",
            participant_id=release_id,
            created_at=utc_now(),
            tombstone_of=target_id,
            data={"reason": _bounded_text(reason, "reason")},
        )
        self._assert_actor(record)
        if target_id not in self._record_map():
            raise ProtocolError("tombstone target is not reachable")
        self._assert_parent(expected_parent)
        return self._commit((record,), "tombstone")

    def visible_transcript(self) -> tuple[Record, ...]:
        records = self.history()
        tombstoned = {record.tombstone_of for record in records if record.kind == "tombstone"}
        return tuple(
            record
            for record in records
            if record.kind in {"user_message", "assistant_message"} and record.record_id not in tombstoned
        )

    def write_model_context(self, entry: ContextEntry) -> None:
        if self._participants.get(entry.agent_id) != "agent":
            raise ProtocolError("model context must belong to a registered agent")
        self._context[(entry.agent_id, entry.key)] = entry

    def model_context(self, agent_id: str) -> tuple[ContextEntry, ...]:
        _id(agent_id, "agent_id")
        return tuple(value for (owner, _), value in sorted(self._context.items()) if owner == agent_id)

    def _commit_for(self, record_id: str) -> str:
        for commit in self.commits_in_order():
            if any(record.record_id == record_id for record in commit.records):
                return commit.commit_id
        raise ProtocolError("record is not reachable")

    def fork(self, new_session_id: str) -> "SessionRepo":
        new_session_id = _uuid(new_session_id, "new_session_id")
        if new_session_id == self.session_id:
            raise Conflict("fork must have a new session ID")
        forked = SessionRepo()
        forked._commits = dict(self._commits)
        forked._head = self._head
        forked._session_id = new_session_id
        forked._release_id = self._release_id
        forked._agent_id = self._agent_id
        forked._participants = dict(self._participants)
        forked.platform_authority = None
        manifest = Record(
            record_id=str(uuid4()),
            kind="session_manifest",
            actor_id=self._release_id or "release:reference-chat",
            actor_role="release",
            participant_id=self._release_id or "release:reference-chat",
            created_at=utc_now(),
            data={
                "session_id": new_session_id,
                "release_id": self._release_id or "release:reference-chat",
                "agent_id": self._agent_id or "agent:reference-chat",
                "ref": MAIN_REF,
                "forked_from_session_id": self.session_id,
            },
        )
        forked._commit((manifest,), "fork session")
        return forked
