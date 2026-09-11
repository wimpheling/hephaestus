#!/usr/bin/env python3
"""Reject browser and cooking fixture credentials in retained files and ZIPs."""

import base64
import argparse
import hashlib
import io
import json
import os
import re
from pathlib import Path
import sys
import zipfile


# These are request-only fixture values, not production credentials. Scan the
# complete values so a trace's source-code references to their prefix are safe.
VALUES = (
    b"HEPHAESTUS_BROWSER_SECRET_4d7ccf_org",
    b"HEPHAESTUS_BROWSER_SECRET_4d7ccf_project",
    b"golden-brokered-provider-sentinel-5d1a",
    b"cooking-inbound-only-fixture-sentinel",
    b"cooking-model-only-fixture-sentinel-724c",
    b"cooking-relay-only-fixture-sentinel-819e",
    b"cooking-model-rotated-fixture-sentinel-936f",
    b"cooking-inbound-rotated-fixture-sentinel-157a",
    b"cooking-relay-rotated-fixture-sentinel-482b",
)
VALUE_RULES = (
    "browser-secret-org",
    "browser-secret-project",
    "golden-provider-sentinel",
    "cooking-inbound-sentinel",
    "cooking-model-sentinel",
    "cooking-relay-sentinel",
    "cooking-model-rotated-sentinel",
    "cooking-inbound-rotated-sentinel",
    "cooking-relay-rotated-sentinel",
)
assert len(VALUES) == len(VALUE_RULES)
# The execution stream is scanned before any human-readable redaction.  Keep
# encoded forms here because a guest or browser trace can serialize a secret
# as raw text, base64, or hexadecimal bytes.
PATTERN_RULES = tuple(
    (pattern, rule)
    for value, rule in zip(VALUES, VALUE_RULES)
    for pattern in (
        value,
        base64.b64encode(value),
        value.hex().encode("ascii"),
        value.hex().upper().encode("ascii"),
    )
)
STREAM_PATTERNS = tuple(
    pattern for pattern, _rule in PATTERN_RULES
)
STREAM_REPLACEMENT = b"[REDACTED]"
STREAM_CHUNK_BYTES = 8192
STREAM_OVERLAP_BYTES = max(len(pattern) for pattern in STREAM_PATTERNS)
STREAM_PATTERN_RE = re.compile(
    b"|".join(re.escape(pattern) for pattern in STREAM_PATTERNS)
)
# Retained evidence uses the same complete pattern set as the live stream.
PATTERNS = STREAM_PATTERNS
MAX_FILE_BYTES = 256 * 1024 * 1024
MAX_ARCHIVE_BYTES = 512 * 1024 * 1024


class ScanFailure(ValueError):
    """A scan failure with a stable, non-sensitive classification."""

    def __init__(self, message: str, rule: str, file_class: str = "unknown") -> None:
        super().__init__(message)
        self.rule = rule
        self.file_class = file_class


def check_bytes(data: bytes, label: str, depth: int = 0) -> None:
    for pattern, rule in PATTERN_RULES:
        if pattern in data:
            raise ScanFailure(f"fixture credential found in {label}", rule, "content")
    for encoded in re.findall(rb"data:application/zip;base64,([A-Za-z0-9+/=]+)", data):
        if depth >= 3:
            raise ScanFailure(
                f"embedded archive nesting exceeds scan limit: {label}",
                "archive-nesting-limit",
                "archive",
            )
        check_bytes(base64.b64decode(encoded, validate=True), label + ":embedded archive", depth + 1)
    if data.startswith(b"PK\x03\x04"):
        if depth >= 3:
            raise ScanFailure(f"archive nesting exceeds scan limit: {label}", "archive-nesting-limit", "archive")
        with zipfile.ZipFile(io.BytesIO(data)) as archive:
            members = archive.infolist()
            if sum(member.file_size for member in members) > MAX_ARCHIVE_BYTES:
                raise ScanFailure(f"archive exceeds scan limit: {label}", "archive-size-limit", "archive")
            for member in members:
                if member.file_size > MAX_FILE_BYTES:
                    raise ScanFailure(
                        f"archive member exceeds scan limit: {label}",
                        "archive-member-size-limit",
                        "archive",
                    )
                check_bytes(archive.read(member), f"{label}:{member.filename}", depth + 1)


class StreamRedactor:
    """Incrementally redact one logical byte stream with bounded lookahead."""

    def __init__(self) -> None:
        self.pending = b""
        self.matched = False

    def feed(self, chunk: bytes) -> bytes:
        """Consume one contiguous chunk and return bytes safe to emit."""

        self.pending += chunk
        output = bytearray()
        while len(self.pending) > STREAM_OVERLAP_BYTES:
            safe_limit = len(self.pending) - STREAM_OVERLAP_BYTES
            match = STREAM_PATTERN_RE.search(self.pending)
            if match is not None and match.start() < safe_limit:
                output.extend(self.pending[: match.start()])
                output.extend(STREAM_REPLACEMENT)
                self.pending = self.pending[match.end() :]
                self.matched = True
                continue
            output.extend(self.pending[:safe_limit])
            self.pending = self.pending[safe_limit:]
        return bytes(output)

    def finish(self) -> bytes:
        """Flush the final lookahead and complete the redaction."""

        redacted, count = STREAM_PATTERN_RE.subn(STREAM_REPLACEMENT, self.pending)
        self.pending = b""
        self.matched = self.matched or count > 0
        return redacted


def redact_stream(input_stream, output_stream, chunk_bytes: int = STREAM_CHUNK_BYTES) -> bool:
    """Copy stdin to stdout while redacting fixture values across chunk edges.

    At most ``chunk_bytes + STREAM_OVERLAP_BYTES`` are retained in memory. A
    full overlap is held back so a value beginning at the end of one input
    chunk cannot be emitted before its remaining bytes arrive.
    """

    if chunk_bytes < 1:
        raise ValueError("stream chunk size must be positive")
    redactor = StreamRedactor()
    read_chunk = getattr(input_stream, "read1", None)
    if read_chunk is None:
        read_chunk = input_stream.read
    while True:
        chunk = read_chunk(chunk_bytes)
        if not chunk:
            break
        output_stream.write(redactor.feed(chunk))
        output_stream.flush()
    output_stream.write(redactor.finish())
    output_stream.flush()
    return redactor.matched


def stream_main() -> int:
    """Scan and redact a raw execution stream, failing only after EOF."""

    matched = redact_stream(sys.stdin.buffer, sys.stdout.buffer)
    sys.stdout.buffer.flush()
    if matched:
        print(
            "Browser evidence raw execution stream contained a fixture credential; "
            "matching bytes were redacted.",
            file=sys.stderr,
        )
        return 1
    return 0


def _file_class(path: Path, data: bytes | None = None) -> str:
    suffix = path.suffix.lower()
    if suffix in {".zip", ".gz", ".tgz", ".tar", ".zst", ".xz"} or (data is not None and data.startswith(b"PK\x03\x04")):
        return "archive"
    if suffix in {".json", ".jsonl", ".ndjson"}:
        return "structured"
    if suffix in {".log", ".txt", ".out", ".err"}:
        return "text"
    return "binary"


def _path_digest(path: Path | None, roots: list[Path]) -> str | None:
    if path is None:
        return None
    for root in roots:
        try:
            relative = path.relative_to(root).as_posix()
        except ValueError:
            continue
        return hashlib.sha256(relative.encode("utf-8")).hexdigest()
    return None


def _status_report(
    *,
    status: str,
    rule: str,
    file_class: str,
    path: Path | None,
    roots: list[Path],
    checked_files: int,
    checked_bytes: int,
) -> dict[str, object]:
    return {
        "schema": 1,
        "status": status,
        "rule": rule,
        "file_class": file_class,
        "path_sha256": _path_digest(path, roots),
        "checked_files": checked_files,
        "checked_bytes": checked_bytes,
    }


def _write_status(path: Path, report: dict[str, object]) -> None:
    if not path.is_absolute() or path.exists() or path.is_symlink():
        raise ValueError("scanner status output is not a fresh absolute file")
    if any(parent.is_symlink() for parent in path.parents if parent.exists()):
        raise ValueError("scanner status output parent is unsafe")
    path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    temporary = path.with_name(f".{path.name}.tmp-{os.getpid()}")
    if temporary.exists() or temporary.is_symlink():
        raise ValueError("scanner status staging path already exists")
    try:
        temporary.write_text(json.dumps(report, sort_keys=True, separators=(",", ":")) + "\n", encoding="utf-8")
        temporary.chmod(0o600)
        temporary.replace(path)
    except Exception:
        temporary.unlink(missing_ok=True)
        raise


def main(paths: list[str], status_output: Path | None = None) -> int:
    if not paths:
        raise ValueError("at least one evidence directory is required")
    roots = [Path(argument) for argument in paths]
    checked = 0
    checked_bytes = 0
    current_path: Path | None = None
    current_data: bytes | None = None
    try:
        for root in roots:
            if not root.is_dir() or root.is_symlink():
                raise ScanFailure("evidence directory is missing or unsafe", "evidence-root", "directory")
            for path in sorted(root.rglob("*")):
                current_path = path
                current_data = None
                if path.is_symlink():
                    raise ScanFailure("evidence symlink is not scannable", "symlink", "filesystem")
                if not path.is_file():
                    continue
                if path.stat().st_size > MAX_FILE_BYTES:
                    raise ScanFailure("evidence file exceeds scan limit", "file-size-limit", _file_class(path))
                current_data = path.read_bytes()
                checked_bytes += len(current_data)
                check_bytes(current_data, str(path))
                checked += 1
        if checked == 0:
            raise ScanFailure("no retained evidence files were found", "no-files", "directory")
    except Exception as error:
        if status_output is not None:
            if isinstance(error, ScanFailure):
                rule = error.rule
                file_class = error.file_class
                if file_class == "content" and current_path is not None:
                    file_class = _file_class(current_path, current_data)
            elif isinstance(error, zipfile.BadZipFile):
                rule = "archive-invalid"
                file_class = "archive"
            elif isinstance(error, (OSError, UnicodeError)):
                rule = "read-error"
                file_class = _file_class(current_path or Path("unknown"), current_data)
            else:
                rule = "scan-error"
                file_class = _file_class(current_path or Path("unknown"), current_data)
            _write_status(
                status_output,
                _status_report(
                    status="failed",
                    rule=rule,
                    file_class=file_class,
                    path=current_path,
                    roots=roots,
                    checked_files=checked,
                    checked_bytes=checked_bytes,
                ),
            )
        raise
    if status_output is not None:
        _write_status(
            status_output,
            _status_report(
                status="passed",
                rule="none",
                file_class="none",
                path=None,
                roots=roots,
                checked_files=checked,
                checked_bytes=checked_bytes,
            ),
        )
    print(f"Browser/cooking evidence credential scan passed: {checked} files (including ZIP contents).")
    return 0


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("paths", nargs="+")
    parser.add_argument("--status-output", type=Path)
    try:
        if sys.argv[1:] == ["--stream"]:
            sys.exit(stream_main())
        arguments = parser.parse_args()
        sys.exit(main(arguments.paths, arguments.status_output))
    except (OSError, ValueError, zipfile.BadZipFile, RuntimeError) as error:
        message = str(error)
        for pattern in PATTERNS:
            message = message.replace(pattern.decode("ascii"), "[REDACTED]")
        print(f"Browser evidence scan failed: {message}", file=sys.stderr)
        sys.exit(1)
