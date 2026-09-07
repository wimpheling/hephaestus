#!/usr/bin/env python3
"""Reject browser and cooking fixture credentials in retained files and ZIPs."""

import base64
import io
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
# The execution stream is scanned before any human-readable redaction.  Keep
# encoded forms here because a guest or browser trace can serialize a secret
# as raw text, base64, or hexadecimal bytes.
STREAM_PATTERNS = tuple(
    pattern
    for value in VALUES
    for pattern in (
        value,
        base64.b64encode(value),
        value.hex().encode("ascii"),
        value.hex().upper().encode("ascii"),
    )
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


def check_bytes(data: bytes, label: str, depth: int = 0) -> None:
    if any(pattern in data for pattern in PATTERNS):
        raise ValueError(f"fixture credential found in {label}")
    for encoded in re.findall(rb"data:application/zip;base64,([A-Za-z0-9+/=]+)", data):
        if depth >= 3:
            raise ValueError(f"embedded archive nesting exceeds scan limit: {label}")
        check_bytes(base64.b64decode(encoded, validate=True), label + ":embedded archive", depth + 1)
    if data.startswith(b"PK\x03\x04"):
        if depth >= 3:
            raise ValueError(f"archive nesting exceeds scan limit: {label}")
        with zipfile.ZipFile(io.BytesIO(data)) as archive:
            members = archive.infolist()
            if sum(member.file_size for member in members) > MAX_ARCHIVE_BYTES:
                raise ValueError(f"archive exceeds scan limit: {label}")
            for member in members:
                if member.file_size > MAX_FILE_BYTES:
                    raise ValueError(f"archive member exceeds scan limit: {label}")
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


def main(paths: list[str]) -> int:
    if not paths:
        raise ValueError("at least one evidence directory is required")
    checked = 0
    for argument in paths:
        root = Path(argument)
        if not root.is_dir() or root.is_symlink():
            raise ValueError(f"evidence directory is missing or unsafe: {root}")
        for path in sorted(root.rglob("*")):
            if path.is_symlink():
                raise ValueError(f"evidence symlink is not scannable: {path}")
            if not path.is_file():
                continue
            if path.stat().st_size > MAX_FILE_BYTES:
                raise ValueError(f"evidence file exceeds scan limit: {path}")
            check_bytes(path.read_bytes(), str(path))
            checked += 1
    if checked == 0:
        raise ValueError("no retained evidence files were found")
    print(f"Browser/cooking evidence credential scan passed: {checked} files (including ZIP contents).")
    return 0


if __name__ == "__main__":
    try:
        if sys.argv[1:] == ["--stream"]:
            sys.exit(stream_main())
        sys.exit(main(sys.argv[1:]))
    except (OSError, ValueError, zipfile.BadZipFile, RuntimeError) as error:
        message = str(error)
        for pattern in PATTERNS:
            message = message.replace(pattern.decode("ascii"), "[REDACTED]")
        print(f"Browser evidence scan failed: {message}", file=sys.stderr)
        sys.exit(1)
