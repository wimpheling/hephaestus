#!/usr/bin/env python3
"""Maintain the bounded, root-owned Cooking gate result sidecar.

The sidecar is deliberately small and typed.  It is written atomically so a
startup collector can distinguish a complete result from a process that died
while producing evidence.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import tempfile
from typing import Any


SCHEMA = 1
GATES = ("workload", "evidence-scan", "browser-validation")
STATES = frozenset({"pending", "running", "passed", "failed", "timed-out", "unknown"})
REASONS = frozenset(
    {
        "none",
        "unknown",
        "unfinished",
        "workload-failed",
        "evidence-scan-failed",
        "evidence-scan-report-invalid",
        "browser-validation-failed",
        "browser-report-invalid",
        "browser-tests-not-passed",
        "timeout",
        "supervisor-failed",
    }
)
FAILED_REASONS = frozenset(
    {
        "workload-failed",
        "evidence-scan-failed",
        "evidence-scan-report-invalid",
        "browser-validation-failed",
        "browser-report-invalid",
        "browser-tests-not-passed",
        "supervisor-failed",
    }
)
REVISION_RE = re.compile(r"[0-9a-f]{40}\Z")
SHA256_RE = re.compile(r"[0-9a-f]{64}\Z")
MAX_SIDECAR_BYTES = 16 * 1024


class GateError(ValueError):
    """A safe, user-facing sidecar contract error."""


def _exit_code(value: Any, field: str) -> None:
    if value is not None and (type(value) is not int or not 0 <= value <= 255):
        raise GateError(f"{field} must be null or an exit code from 0 through 255")


def validate_document(value: Any) -> dict[str, Any]:
    if not isinstance(value, dict) or set(value) != {
        "schema",
        "revision",
        "script_sha256",
        "test_mode",
        "overall_exit_code",
        "supervisor_exit_code",
        "finalized",
        "gates",
    }:
        raise GateError("sidecar fields are invalid")
    if type(value["schema"]) is not int or value["schema"] != SCHEMA:
        raise GateError("sidecar schema is unsupported")
    if not isinstance(value["revision"], str) or REVISION_RE.fullmatch(value["revision"]) is None:
        raise GateError("sidecar revision is invalid")
    if not isinstance(value["script_sha256"], str) or SHA256_RE.fullmatch(value["script_sha256"]) is None:
        raise GateError("sidecar script hash is invalid")
    if value["test_mode"] not in {"gcp-cooking", "diagnostic"}:
        raise GateError("sidecar test mode is invalid")
    _exit_code(value["overall_exit_code"], "overall_exit_code")
    _exit_code(value["supervisor_exit_code"], "supervisor_exit_code")
    if type(value["finalized"]) is not bool:
        raise GateError("sidecar finalized flag is invalid")
    gates = value["gates"]
    if not isinstance(gates, dict) or set(gates) != set(GATES):
        raise GateError("sidecar gate set is invalid")
    for name in GATES:
        gate = gates[name]
        if not isinstance(gate, dict) or set(gate) != {"state", "exit_code", "reason_class"}:
            raise GateError(f"sidecar {name} gate fields are invalid")
        if gate["state"] not in STATES:
            raise GateError(f"sidecar {name} state is invalid")
        _exit_code(gate["exit_code"], f"{name}.exit_code")
        if gate["reason_class"] not in REASONS:
            raise GateError(f"sidecar {name} reason class is invalid")
        if gate["state"] == "passed" and gate["exit_code"] != 0:
            raise GateError(f"sidecar {name} passed gate must have exit code 0")
        if gate["state"] == "timed-out" and gate["exit_code"] != 124:
            raise GateError(f"sidecar {name} timed-out gate must have exit code 124")
        if gate["state"] in {"pending", "running", "unknown"} and gate["exit_code"] is not None:
            raise GateError(f"sidecar {name} unfinished gate must have a null exit code")
        if gate["state"] == "passed" and gate["reason_class"] != "none":
            raise GateError(f"sidecar {name} passed gate must have reason class none")
        if gate["state"] == "failed" and gate["reason_class"] not in FAILED_REASONS:
            raise GateError(f"sidecar {name} failed gate reason class is invalid")
        if gate["state"] == "failed" and (gate["exit_code"] is None or gate["exit_code"] == 0):
            raise GateError(f"sidecar {name} failed gate must have a nonzero exit code")
        if gate["state"] == "timed-out" and gate["reason_class"] != "timeout":
            raise GateError(f"sidecar {name} timed-out gate reason class is invalid")
        if gate["state"] == "unknown" and gate["reason_class"] not in {"unknown", "unfinished"}:
            raise GateError(f"sidecar {name} unknown gate reason class is invalid")
    if value["finalized"]:
        if value["overall_exit_code"] is None:
            raise GateError("finalized sidecar must have an overall exit code")
        if any(value["gates"][name]["state"] in {"pending", "running"} for name in GATES):
            raise GateError("finalized sidecar contains an unfinished gate")
    return value


def _parent_fd(path: Path) -> int:
    parent = path.parent
    try:
        info = parent.stat()
    except OSError as exc:
        raise GateError("sidecar parent is unavailable") from exc
    if not stat.S_ISDIR(info.st_mode) or parent.is_symlink() or info.st_mode & 0o777 != 0o700:
        raise GateError("sidecar parent must be a non-symlink 0700 directory")
    if info.st_uid != os.getuid():
        raise GateError("sidecar parent owner is invalid")
    try:
        return os.open(parent, os.O_RDONLY | os.O_DIRECTORY | getattr(os, "O_NOFOLLOW", 0))
    except OSError as exc:
        raise GateError("sidecar parent cannot be opened safely") from exc


def _read(path: Path) -> dict[str, Any]:
    fd = _parent_fd(path)
    try:
        try:
            file_fd = os.open(path.name, os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0), dir_fd=fd)
        except OSError as exc:
            raise GateError("sidecar is unavailable") from exc
        try:
            info = os.fstat(file_fd)
            if not stat.S_ISREG(info.st_mode) or info.st_mode & 0o777 != 0o600 or info.st_uid != os.getuid():
                raise GateError("sidecar file mode or owner is invalid")
            if info.st_size > MAX_SIDECAR_BYTES:
                raise GateError("sidecar exceeds its bounded size")
            try:
                raw = os.read(file_fd, MAX_SIDECAR_BYTES + 1)
                if len(raw) > MAX_SIDECAR_BYTES:
                    raise GateError("sidecar exceeds its bounded size")
                value = json.loads(raw.decode("utf-8"))
            except GateError:
                raise
            except (OSError, UnicodeDecodeError, json.JSONDecodeError) as exc:
                raise GateError("sidecar JSON is invalid") from exc
        finally:
            os.close(file_fd)
    finally:
        os.close(fd)
    return validate_document(value)


def _write(path: Path, value: dict[str, Any]) -> None:
    validate_document(value)
    fd = _parent_fd(path)
    temporary_name: str | None = None
    try:
        try:
            existing = os.stat(path.name, dir_fd=fd, follow_symlinks=False)
        except FileNotFoundError:
            existing = None
        if existing is not None:
            if stat.S_ISLNK(existing.st_mode):
                raise GateError("refusing to replace a symlinked sidecar")
            if not stat.S_ISREG(existing.st_mode):
                raise GateError("sidecar target is not a regular file")
        temporary_fd, temporary_path = tempfile.mkstemp(prefix=f".{path.name}.", dir=path.parent)
        temporary_name = Path(temporary_path).name
        os.fchmod(temporary_fd, 0o600)
        encoded = (json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n").encode()
        with os.fdopen(temporary_fd, "wb") as stream:
            stream.write(encoded)
            stream.flush()
            os.fsync(stream.fileno())
        os.rename(temporary_name, path.name, src_dir_fd=fd, dst_dir_fd=fd)
        temporary_name = None
        os.fsync(fd)
    except OSError as exc:
        raise GateError("atomic sidecar write failed") from exc
    finally:
        if temporary_name is not None:
            try:
                os.unlink(temporary_name, dir_fd=fd)
            except OSError:
                pass
        os.close(fd)


def _load_args(args: argparse.Namespace) -> tuple[Path, dict[str, Any]]:
    path = Path(args.path)
    return path, _read(path)


def _cmd_init(args: argparse.Namespace) -> int:
    helper = Path(args.script_file)
    if helper.is_symlink() or not helper.is_file():
        raise GateError("script file is unavailable")
    actual_hash = hashlib.sha256(helper.read_bytes()).hexdigest()
    if actual_hash != args.script_sha256:
        raise GateError("script hash does not match the helper")
    path = Path(args.path)
    fd = _parent_fd(path)
    try:
        try:
            os.stat(path.name, dir_fd=fd, follow_symlinks=False)
        except FileNotFoundError:
            pass
        else:
            raise GateError("sidecar already exists")
    except OSError as exc:
        raise GateError("sidecar target cannot be checked safely") from exc
    finally:
        os.close(fd)
    value = {
        "schema": SCHEMA,
        "revision": args.revision,
        "script_sha256": args.script_sha256,
        "test_mode": args.test_mode,
        "overall_exit_code": None,
        "supervisor_exit_code": None,
        "finalized": False,
        "gates": {name: {"state": "pending", "exit_code": None, "reason_class": "unknown"} for name in GATES},
    }
    _write(path, value)
    return 0


def _cmd_begin(args: argparse.Namespace) -> int:
    path, value = _load_args(args)
    if value["finalized"]:
        raise GateError("sidecar is already finalized")
    gate = value["gates"][args.gate]
    if gate["state"] != "pending":
        raise GateError(f"{args.gate} gate is not pending")
    gate.update(state="running", exit_code=None, reason_class="unknown")
    _write(path, value)
    return 0


def _cmd_complete(args: argparse.Namespace) -> int:
    path, value = _load_args(args)
    if value["finalized"]:
        raise GateError("sidecar is already finalized")
    gate = value["gates"][args.gate]
    if gate["state"] != "running":
        raise GateError(f"{args.gate} gate is not running")
    if args.state == "passed" and args.exit_code != 0:
        raise GateError("passed gate requires exit code 0")
    if args.state == "failed" and args.exit_code == 0:
        raise GateError("failed gate requires a nonzero exit code")
    if args.state == "timed-out" and args.exit_code != 124:
        raise GateError("timed-out gate requires exit code 124")
    if args.state in {"unknown", "pending", "running"}:
        raise GateError("complete requires a terminal gate state")
    if args.state == "passed" and args.reason_class != "none":
        raise GateError("passed gate requires reason class none")
    if args.state == "failed" and args.reason_class not in FAILED_REASONS:
        raise GateError("failed gate reason class is invalid")
    if args.state == "timed-out" and args.reason_class != "timeout":
        raise GateError("timed-out gate reason class is invalid")
    gate.update(state=args.state, exit_code=args.exit_code, reason_class=args.reason_class)
    _write(path, value)
    return 0


def _cmd_finalize(args: argparse.Namespace) -> int:
    path, value = _load_args(args)
    if value["finalized"]:
        # The Cooking child exits before startup observes its process status.
        # Permit startup's one follow-up write to add that observation while
        # preserving the runtime aggregate and all already-known gate states.
        if args.supervisor_exit_code is None or value["supervisor_exit_code"] is not None:
            raise GateError("sidecar is already finalized")
        value["supervisor_exit_code"] = args.supervisor_exit_code
        _write(path, value)
        return 0
    for gate in value["gates"].values():
        if gate["state"] in {"pending", "running"}:
            gate.update(state="unknown", exit_code=None, reason_class="unfinished")
    value["overall_exit_code"] = args.overall_exit_code
    value["supervisor_exit_code"] = args.supervisor_exit_code
    value["finalized"] = True
    _write(path, value)
    return 0


def _cmd_validate(args: argparse.Namespace) -> int:
    path, value = _load_args(args)
    helper = Path(args.script_file)
    if helper.is_symlink() or not helper.is_file():
        raise GateError("script file is unavailable")
    actual_hash = hashlib.sha256(helper.read_bytes()).hexdigest()
    if actual_hash != value["script_sha256"]:
        raise GateError("sidecar script hash does not match the helper")
    print("cooking gate sidecar: valid")
    return 0


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--path", required=True)
    parser.add_argument("--script-file", default=__file__)
    sub = parser.add_subparsers(dest="command", required=True)
    init = sub.add_parser("init")
    init.add_argument("--revision", required=True)
    init.add_argument("--script-sha256", required=True)
    init.add_argument("--test-mode", required=True)
    begin = sub.add_parser("begin")
    begin.add_argument("gate", choices=GATES)
    complete = sub.add_parser("complete")
    complete.add_argument("gate", choices=GATES)
    complete.add_argument("--state", choices=("passed", "failed", "timed-out"), required=True)
    complete.add_argument("--exit-code", type=int, required=True)
    complete.add_argument("--reason-class", choices=sorted(REASONS), required=True)
    finalize = sub.add_parser("finalize")
    finalize.add_argument("--overall-exit-code", type=int, required=True)
    finalize.add_argument("--supervisor-exit-code", type=int)
    sub.add_parser("validate")
    return parser


def main(argv: list[str] | None = None) -> int:
    args = _parser().parse_args(argv)
    try:
        if args.command == "init":
            if REVISION_RE.fullmatch(args.revision) is None or SHA256_RE.fullmatch(args.script_sha256) is None:
                raise GateError("init revision or script hash is invalid")
            if args.test_mode not in {"gcp-cooking", "diagnostic"}:
                raise GateError("init test mode is invalid")
            return _cmd_init(args)
        if args.command == "begin":
            return _cmd_begin(args)
        if args.command == "complete":
            if not 0 <= args.exit_code <= 255:
                raise GateError("complete exit code is invalid")
            return _cmd_complete(args)
        if args.command == "finalize":
            _exit_code(args.overall_exit_code, "overall_exit_code")
            _exit_code(args.supervisor_exit_code, "supervisor_exit_code")
            if args.overall_exit_code is None:
                raise GateError("finalize requires an overall exit code")
            return _cmd_finalize(args)
        return _cmd_validate(args)
    except GateError as exc:
        print(f"cooking gate sidecar: {exc}", file=os.sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
