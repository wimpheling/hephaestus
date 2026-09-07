#!/usr/bin/env python3
"""Runtime-only bounded scanner for a cooking guest.

This module has no fixture values, CLI, test data, or host provisioning code.
It can be copied into a released diagnostic agent and called around its main
operation.  It returns metadata and fingerprint labels only.
"""

from __future__ import annotations

from dataclasses import dataclass
import errno
import hashlib
import os
from pathlib import Path
import re
import stat
import sys

CHUNK_SIZE = 16 * 1024
# Workspace materialization exposes the source tree at both /workspace/repo and
# /workspace/work. The current approved blog is about 151 MiB, so the aggregate
# scan needs room for both copies while remaining bounded.
MAX_TOTAL_BYTES = 512 * 1024 * 1024
MAX_FILES = 4096
EXPECTED_LABELS = (
    "inbound",
    "model",
    "relay",
    "inbound_rotated",
    "model_rotated",
    "relay_rotated",
)
EXPECTED_ENCODINGS = ("raw", "base64", "hex_lower", "hex_upper")
EXPECTED_SURFACE_CODES = (
    "release",
    "control",
    "state",
    "work",
    "repo",
    "secret-mount",
    "runtime-credential",
    "authority",
    "proc-env",
    "proc-argv",
    "unknown",
)
EXPECTED_ERRNO_CODES = ("permission", "missing", "not-directory", "loop", "io", "other", "none")


class ProbeError(RuntimeError):
    """A safe, non-sensitive diagnostic failure."""

    def __init__(
        self,
        message: str,
        *,
        surface_code: str = "unknown",
        errno_code: str = "none",
    ) -> None:
        super().__init__(message)
        self.surface_code = (
            surface_code if surface_code in EXPECTED_SURFACE_CODES else "unknown"
        )
        self.errno_code = errno_code if errno_code in EXPECTED_ERRNO_CODES else "other"


@dataclass
class ScanBudget:
    """Whole-invocation bounds shared by process and surface scans."""

    total_bytes: int = 0
    files: int = 0

    def consume_bytes(self, amount: int) -> None:
        self.total_bytes += amount
        if self.total_bytes > MAX_TOTAL_BYTES:
            raise ProbeError("probe invocation exceeds the bounded scan size")

    def consume_file(self) -> None:
        self.files += 1
        if self.files > MAX_FILES:
            raise ProbeError("probe invocation contains too many files")


def _validate_descriptors(records: list[dict[str, object]]) -> list[dict[str, object]]:
    if len(records) != len(EXPECTED_LABELS) * len(EXPECTED_ENCODINGS):
        raise ProbeError("descriptor set has an unexpected size")
    pairs: set[tuple[object, object]] = set()
    for record in records:
        if (
            set(record) != {"label", "encoding", "length", "sha256", "prefix4"}
            or record["label"] not in EXPECTED_LABELS
            or record["encoding"] not in EXPECTED_ENCODINGS
            or type(record["length"]) is not int
            or not 1 <= record["length"] <= CHUNK_SIZE
            or not isinstance(record["sha256"], str)
            or not re.fullmatch(r"sha256:[0-9a-f]{64}", record["sha256"])
            or not isinstance(record["prefix4"], str)
            or not re.fullmatch(r"[0-9a-f]{8}", record["prefix4"])
        ):
            raise ProbeError("descriptor set is malformed")
        pairs.add((record["label"], record["encoding"]))
    expected = {
        (label, encoding) for label in EXPECTED_LABELS for encoding in EXPECTED_ENCODINGS
    }
    if pairs != expected:
        raise ProbeError("descriptor set is incomplete or duplicated")
    return records


def _scan_bytes(data: bytes, records: list[dict[str, object]]) -> set[tuple[str, str]]:
    if len(data) > MAX_TOTAL_BYTES:
        raise ProbeError("probe input exceeds the bounded scan size")
    matches: set[tuple[str, str]] = set()
    for record in records:
        label = str(record["label"])
        encoding = str(record["encoding"])
        length = int(record["length"])
        prefix = bytes.fromhex(str(record["prefix4"]))
        expected = str(record["sha256"])
        position = data.find(prefix)
        while position >= 0:
            end = position + length
            if end <= len(data):
                digest = "sha256:" + hashlib.sha256(data[position:end]).hexdigest()
                if digest == expected:
                    matches.add((label, encoding))
                    break
            position = data.find(prefix, position + 1)
    return matches


def _scan_reader(stream, records, budget: ScanBudget, digest_state=None):
    records = _validate_descriptors(records)
    maximum = max(int(record["length"]) for record in records)
    overlap = max(0, maximum - 1)
    carry = b""
    matches: set[tuple[str, str]] = set()
    total = 0
    while True:
        try:
            chunk = stream.read(CHUNK_SIZE)
        except OSError as error:
            raise ProbeError("probe could not read a guest surface") from None
        if not chunk:
            break
        if not isinstance(chunk, bytes):
            raise ProbeError("probe stream returned non-byte data")
        budget.consume_bytes(len(chunk))
        total += len(chunk)
        if digest_state is not None:
            digest_state.update(chunk)
        window = carry + chunk
        matches.update(_scan_bytes(window, records))
        carry = window[-overlap:] if overlap else b""
    return total, matches


def scan_stream(stream, records, budget: ScanBudget | None = None):
    """Scan a stream in bounded chunks and return fingerprint labels."""
    return _scan_reader(stream, records, budget or ScanBudget())[1]


def _allowed(path: Path, allowed_roots: tuple[str, ...]) -> bool:
    candidate = os.path.abspath(path)
    if not any(candidate == root or candidate.startswith(root + os.sep) for root in allowed_roots):
        return False
    resolved = os.path.realpath(candidate)
    return any(
        resolved == os.path.realpath(root)
        or resolved.startswith(os.path.realpath(root) + os.sep)
        for root in allowed_roots
    )


def _errno_code(error: OSError) -> str:
    """Reduce an OS error to a fixed, non-sensitive diagnostic class."""
    return {
        errno.EACCES: "permission",
        errno.ENOENT: "missing",
        errno.ENOTDIR: "not-directory",
        errno.ELOOP: "loop",
        errno.EIO: "io",
    }.get(error.errno, "other")


def _metadata(path: Path, size: int, fingerprint: str | None) -> dict[str, object]:
    try:
        info = os.lstat(path)
    except OSError as error:
        raise ProbeError("probe could not read surface metadata") from None
    if stat.S_ISDIR(info.st_mode):
        kind = "directory"
    elif stat.S_ISREG(info.st_mode):
        kind = "file"
    elif stat.S_ISSOCK(info.st_mode):
        kind = "socket"
    else:
        kind = "other"
    return {
        "path_fingerprint": "sha256:" + hashlib.sha256(os.fsencode(path)).hexdigest(),
        "kind": kind,
        "mode": stat.S_IMODE(info.st_mode),
        "uid": info.st_uid,
        "size": size,
        "fingerprint": fingerprint,
    }


def _scan_file(path: Path, records, budget: ScanBudget, expected_sockets: tuple[str, ...]):
    try:
        info = os.lstat(path)
    except OSError as error:
        raise ProbeError("probe could not read surface metadata") from None
    if stat.S_ISLNK(info.st_mode):
        raise ProbeError("probe encountered a symlink")
    budget.consume_file()
    if stat.S_ISSOCK(info.st_mode):
        if str(path) not in expected_sockets:
            raise ProbeError("probe encountered an unexpected socket")
        return _metadata(path, 0, None), set()
    if not stat.S_ISREG(info.st_mode):
        raise ProbeError("probe encountered an unexpected special file")
    digest_state = hashlib.sha256()
    try:
        with path.open("rb") as stream:
            total, matches = _scan_reader(stream, records, budget, digest_state)
    except ProbeError:
        raise
    except OSError as error:
        raise ProbeError("probe could not read a guest surface") from None
    return _metadata(path, total, "sha256:" + digest_state.hexdigest()), matches


def _protected_metadata(
    error: OSError,
    expected_paths: tuple[str, ...],
    scanned_root: Path,
) -> dict[str, object] | None:
    """Accept only the ext4 metadata directory explicitly protected by init.

    A freshly formatted state volume contains ``lost+found`` as root-owned
    mode 0700.  The agent must not enumerate that directory, but the state
    root and its application files must remain readable.  This narrow rule
    records the denied metadata instead of treating every permission error as
    harmless.
    """
    if error.errno != errno.EACCES or not error.filename:
        return None
    candidate = Path(os.fsdecode(error.filename)).absolute()
    expected = {Path(path).absolute() for path in expected_paths}
    if candidate not in expected or candidate.parent != scanned_root.absolute():
        return None
    try:
        info = os.lstat(candidate)
    except OSError:
        return None
    if not stat.S_ISDIR(info.st_mode) or info.st_uid != 0 or stat.S_IMODE(info.st_mode) != 0o700:
        return None
    return {
        "path_fingerprint": "sha256:" + hashlib.sha256(os.fsencode(candidate)).hexdigest(),
        "kind": "directory",
        "mode": stat.S_IMODE(info.st_mode),
        "uid": info.st_uid,
        "access": "denied",
    }


def scan_surface(
    path: Path,
    records: list[dict[str, object]],
    budget: ScanBudget | None = None,
    *,
    allowed_roots: tuple[str, ...],
    expected_sockets: tuple[str, ...] = (),
    expected_protected_metadata: tuple[str, ...] = (),
    required: bool = True,
    surface_code: str = "unknown",
):
    """Scan one actual guest file/tree, including path-name bytes."""
    if not allowed_roots or not _allowed(path, allowed_roots):
        raise ProbeError("probe target is outside the approved guest surfaces")
    budget = budget or ScanBudget()
    path_fingerprint = "sha256:" + hashlib.sha256(os.fsencode(path)).hexdigest()
    try:
        path_info = os.lstat(path)
    except FileNotFoundError:
        if required:
            raise ProbeError("required probe surface is missing")
        return {"path_fingerprint": path_fingerprint, "present": False, "files": 0, "matches": []}
    except OSError as error:
        raise ProbeError("probe could not inspect a guest surface") from None
    if stat.S_ISLNK(path_info.st_mode):
        raise ProbeError("probe encountered a symlink")
    budget.consume_bytes(len(os.fsencode(path)))
    matches = _scan_bytes(os.fsencode(path), records)
    files: list[dict[str, object]] = []
    protected_metadata: list[dict[str, object]] = []
    if stat.S_ISREG(path_info.st_mode) or stat.S_ISSOCK(path_info.st_mode):
        metadata, found = _scan_file(path, records, budget, expected_sockets)
        files.append(metadata)
        matches.update(found)
    elif stat.S_ISDIR(path_info.st_mode):
        def walk_error(error):
            metadata = _protected_metadata(error, expected_protected_metadata, path)
            if metadata is not None:
                protected_metadata.append(metadata)
                return
            raise ProbeError(
                "probe could not enumerate a guest surface",
                surface_code=surface_code,
                errno_code=_errno_code(error),
            )

        for current, directories, names in os.walk(
            path, topdown=True, onerror=walk_error, followlinks=False
        ):
            directories.sort()
            names.sort()
            current_path = Path(current)
            for name in directories + names:
                child = current_path / name
                try:
                    child_info = os.lstat(child)
                except OSError as error:
                    raise ProbeError("probe could not inspect a guest surface") from None
                if stat.S_ISLNK(child_info.st_mode):
                    raise ProbeError("probe encountered a symlink")
                budget.consume_bytes(len(os.fsencode(child)))
                matches.update(_scan_bytes(os.fsencode(child), records))
            for name in names:
                metadata, found = _scan_file(
                    current_path / name, records, budget, expected_sockets
                )
                files.append(metadata)
                matches.update(found)
    else:
        raise ProbeError("probe target is not a regular file, directory, or socket")
    return {
        "path_fingerprint": path_fingerprint,
        "present": True,
        "files": len(files),
        "inventory": files,
        "protected_metadata": protected_metadata,
        "matches": [f"{label}:{encoding}" for label, encoding in sorted(matches)],
    }


def scan_process(records, argv=None, budget: ScanBudget | None = None):
    """Scan actual environment/argv values without returning their bytes."""
    budget = budget or ScanBudget()
    env_bytes = b"\0".join(
        key.encode() + b"=" + value.encode() for key, value in sorted(os.environ.items())
    )
    argv_bytes = b"\0".join(value.encode() for value in (argv or []))
    budget.consume_bytes(len(env_bytes))
    budget.consume_bytes(len(argv_bytes))
    return {
        "environment": [f"{a}:{b}" for a, b in sorted(_scan_bytes(env_bytes, records))],
        "argv": [f"{a}:{b}" for a, b in sorted(_scan_bytes(argv_bytes, records))],
    }


def runtime_probe(
    records,
    surfaces,
    argv=None,
    expected_sockets=(),
    surface_labels=(),
    expected_protected_metadata=(),
):
    """Run in an agent and fail closed on empty/missing/matching surfaces."""
    records = _validate_descriptors(records)
    surfaces = tuple(surfaces)
    surface_labels = tuple(surface_labels)
    if surface_labels and len(surface_labels) != len(surfaces):
        raise ProbeError("probe surface labels are malformed")
    if not surface_labels:
        surface_labels = ("unknown",) * len(surfaces)
    if not surfaces:
        raise ProbeError("runtime probe requires at least one actual surface")
    budget = ScanBudget()
    process = scan_process(records, argv if argv is not None else sys.argv, budget)
    scanned_surfaces = []
    for index, surface in enumerate(surfaces):
        try:
            scanned_surfaces.append(
                scan_surface(
                    Path(surface),
                    records,
                    budget,
                    allowed_roots=surfaces,
                    expected_sockets=tuple(expected_sockets),
                    expected_protected_metadata=tuple(expected_protected_metadata)
                    if surface_labels[index] == "state"
                    else (),
                    required=True,
                    surface_code=surface_labels[index],
                )
            )
        except ProbeError as error:
            if error.surface_code == "unknown":
                error.surface_code = (
                    surface_labels[index]
                    if surface_labels[index] in EXPECTED_SURFACE_CODES
                    else "unknown"
                )
            raise
    result = {
        "version": 1,
        "process": process,
        "surfaces": scanned_surfaces,
        "scanned_files": budget.files,
        "scanned_bytes": budget.total_bytes,
    }
    if budget.files == 0:
        raise ProbeError("runtime probe found no files or expected sockets")
    matches = process["environment"] + process["argv"]
    matches += [match for surface in result["surfaces"] for match in surface["matches"]]
    if matches:
        raise ProbeError("fixture fingerprint found in a guest-visible surface")
    return result


def run_with_probe(records, surfaces, operation, argv=None, expected_sockets=()):
    """Run an agent operation between required nonempty before/after scans."""
    before = runtime_probe(records, surfaces, argv, expected_sockets)
    if before["scanned_files"] == 0:
        raise ProbeError("runtime probe before-evidence is empty")
    try:
        result = operation()
    finally:
        after = runtime_probe(records, surfaces, argv, expected_sockets)
        if after["scanned_files"] == 0:
            raise ProbeError("runtime probe after-evidence is empty")
    return result, before, after
