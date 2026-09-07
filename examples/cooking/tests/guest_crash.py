#!/usr/bin/env python3
"""Create the one-shot guest crash variant for the cooking acceptance run.

This transformer deliberately only edits a copied cooking-agent source tree.
It does not alter the canonical source or any PostgreSQL/SQLite lifecycle row.
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path


HELPER = r'''

# Test-only fault seam. The release variant is used only by the explicit
# guest-crash acceptance branch; the canonical release has no such calls.
_CRASH_STAGE_BY_UPDATE = {
    51: "sqlite_before_commit",
    52: "sqlite_after_commit_before_model",
    53: "model_response_before_persist",
    54: "relay_return_before_persist",
    55: "proposal_ready_before_exit",
}
_CRASH_MARKER_ROOT = Path(os.environ.get(
    "HEPHAESTUS_COOKING_CRASH_MARKER_ROOT",
    "/var/lib/hephaestus/.cooking-crash-markers",
))


def _crash_once(update_id, stage):
    """SIGKILL once for the selected update/stage after durable marker fsync."""
    if _CRASH_STAGE_BY_UPDATE.get(update_id) != stage:
        return
    try:
        _CRASH_MARKER_ROOT.mkdir(mode=0o700, parents=False, exist_ok=True)
    except OSError as error:
        raise RuntimeError("guest crash marker directory is unavailable") from error
    if not _CRASH_MARKER_ROOT.is_dir():
        raise RuntimeError("guest crash marker path is not a directory")
    parent_fd = os.open(
        _CRASH_MARKER_ROOT.parent,
        os.O_RDONLY | getattr(os, "O_DIRECTORY", 0),
    )
    try:
        os.fsync(parent_fd)
    finally:
        os.close(parent_fd)
    marker = _CRASH_MARKER_ROOT / (str(update_id) + "-" + stage)
    try:
        marker_fd = os.open(
            marker,
            os.O_WRONLY | os.O_CREAT | os.O_EXCL,
            0o600,
        )
    except FileExistsError:
        return
    with os.fdopen(marker_fd, "wb") as marker_file:
        marker_file.write((str(update_id) + "\n" + stage + "\n").encode("ascii"))
        marker_file.flush()
        os.fsync(marker_file.fileno())
    directory_fd = os.open(
        _CRASH_MARKER_ROOT,
        os.O_RDONLY | getattr(os, "O_DIRECTORY", 0),
    )
    try:
        os.fsync(directory_fd)
    finally:
        os.close(directory_fd)
    os.kill(os.getpid(), signal.SIGKILL)
'''

ANCHORS = {
    "sqlite_before_commit": (
        "        db.execute('INSERT OR REPLACE INTO conversation_summaries VALUES(?,?)', (event['user_id'], event['text'][:512]))\n",
        "        db.execute('INSERT OR REPLACE INTO conversation_summaries VALUES(?,?)', (event['user_id'], event['text'][:512]))\n        _crash_once(update_id, 'sqlite_before_commit')\n",
    ),
    "sqlite_after_commit_before_model": (
        "    row = db.execute('SELECT model_response,relay_outcome,context FROM recipes WHERE recipe_id=?', (identity,)).fetchone()\n",
        "    _crash_once(update_id, 'sqlite_after_commit_before_model')\n    row = db.execute('SELECT model_response,relay_outcome,context FROM recipes WHERE recipe_id=?', (identity,)).fetchone()\n",
    ),
    "model_response_before_persist": (
        "    validate_recipe(recipe)\n    version = db.execute('SELECT version FROM schema_meta').fetchone()[0]\n",
        "    validate_recipe(recipe)\n    _crash_once(update_id, 'model_response_before_persist')\n    version = db.execute('SELECT version FROM schema_meta').fetchone()[0]\n",
    ),
    "relay_return_before_persist": (
        "        outcome = call('telegram_relay', {'idempotency_key': identity, 'user_id': event['user_id'], 'text': f\"{recipe['title']}: content/recipes/{identity}.md\"})\n        if set(outcome) != {'status', 'message_id'} or outcome['status'] != 'delivered':\n",
        "        outcome = call('telegram_relay', {'idempotency_key': identity, 'user_id': event['user_id'], 'text': f\"{recipe['title']}: content/recipes/{identity}.md\"})\n        _crash_once(update_id, 'relay_return_before_persist')\n        if set(outcome) != {'status', 'message_id'} or outcome['status'] != 'delivered':\n",
    ),
    "proposal_ready_before_exit": (
        "        db.execute('UPDATE processed_updates SET disposition=? WHERE update_id=?', ('completed', update_id))\n    return identity\n",
        "        db.execute('UPDATE processed_updates SET disposition=? WHERE update_id=?', ('completed', update_id))\n    _crash_once(update_id, 'proposal_ready_before_exit')\n    return identity\n",
    ),
}


PROBE_SURFACES = (
    ("release", "/release"),
    ("control", "/run/hephaestus"),
    ("state", "/var/lib/hephaestus"),
    ("work", "/workspace/work"),
    ("repo", "/workspace/repo"),
    ("secret-mount", "/run/hephaestus-secrets"),
    ("runtime-credential", "/run/hephaestus-secrets/.runtime-credential"),
    ("authority", None),
    ("proc-env", "/proc/self/environ"),
    ("proc-argv", "/proc/self/cmdline"),
)
# ext4 creates this root-owned 0700 directory. It is intentionally
# inaccessible to the agent; the probe records that bounded metadata fact
# while still requiring the state root and application files to be readable.
PROBE_EXPECTED_PROTECTED_METADATA = ("/var/lib/hephaestus/lost+found",)


def probe_source(runtime_source: Path, descriptor_source: Path) -> str:
    runtime = runtime_source.read_text(encoding="utf-8")
    descriptor_text = descriptor_source.read_text(encoding="utf-8")
    descriptor = json.loads(descriptor_text)
    if descriptor.get("version") != 1 or not isinstance(descriptor.get("descriptors"), list):
        raise ValueError("probe descriptor document is malformed")
    surfaces = ",".join(label for label, _ in PROBE_SURFACES)
    surface_codes = tuple(label for label, _ in PROBE_SURFACES)
    authority_index = next(
        index for index, (label, _) in enumerate(PROBE_SURFACES) if label == "authority"
    )
    path_literals = [repr(path) for _, path in PROBE_SURFACES if path is not None]
    path_literals.insert(authority_index, "os.environ['HEPH_RUNTIME_AUTHORITY_PATH']")
    paths = "(" + ", ".join(path_literals) + ",)"
    reason_codes = {
        "probe invocation exceeds the bounded scan size": "budget_total",
        "probe input exceeds the bounded scan size": "budget_input",
        "probe invocation contains too many files": "file_count",
        "descriptor set has an unexpected size": "descriptor_size",
        "descriptor set is malformed": "descriptor_shape",
        "descriptor set is incomplete or duplicated": "descriptor_set",
        "probe could not read a guest surface": "read",
        "probe could not read surface metadata": "metadata",
        "probe encountered a symlink": "symlink",
        "probe encountered an unexpected socket": "socket",
        "probe encountered an unexpected special file": "special_file",
        "probe could not enumerate a guest surface": "enumeration",
        "probe could not inspect a guest surface": "inspect",
        "probe target is outside the approved guest surfaces": "outside_root",
        "required probe surface is missing": "missing_surface",
        "probe target is not a regular file, directory, or socket": "file_type",
        "runtime probe requires at least one actual surface": "no_surfaces",
        "runtime probe found no files or expected sockets": "empty_surface",
        "fixture fingerprint found in a guest-visible surface": "credential_match",
        "runtime probe before-evidence is empty": "empty_before",
        "runtime probe after-evidence is empty": "empty_after",
    }
    return f'''\n\n# Test-only runtime confinement probe. The descriptor contains fingerprints only;
# fixture plaintext is generated on the host and is never embedded in this release.
exec({runtime!r}, globals())
_PROBE_RECORDS = json.loads({descriptor_text!r})["descriptors"]
_PROBE_SURFACES = {paths}
_PROBE_SURFACE_LABELS = {surfaces!r}
_PROBE_SURFACE_CODES = {surface_codes!r}
_PROBE_REASON_CODES = {reason_codes!r}
_PROBE_ERRNO_CODES = ('permission', 'missing', 'not-directory', 'loop', 'io', 'other', 'none')
_PROBE_SURFACE_CODE_SET = _PROBE_SURFACE_CODES + ('unknown',)


def _emit_probe(stage):
    try:
        evidence = runtime_probe(
            _PROBE_RECORDS,
            _PROBE_SURFACES,
            argv=__import__('sys').argv,
            surface_labels=_PROBE_SURFACE_CODES,
            expected_protected_metadata={PROBE_EXPECTED_PROTECTED_METADATA!r},
        )
    except ProbeError as error:
        stage_code = stage if stage in ('before', 'after') else 'unknown'
        reason_code = _PROBE_REASON_CODES.get(str(error), 'unknown')
        surface_code = getattr(error, 'surface_code', 'unknown')
        if surface_code not in _PROBE_SURFACE_CODE_SET:
            surface_code = 'unknown'
        errno_code = getattr(error, 'errno_code', 'other')
        if errno_code not in _PROBE_ERRNO_CODES:
            errno_code = 'other'
        safe_error = ProbeError(
            'probe failed: stage=' + stage_code + ';reason=' + reason_code,
            surface_code=surface_code,
            errno_code=errno_code,
        )
        print(
            'cooking probe failed: stage=' + stage_code + ';reason=' + reason_code
            + ';surface=' + surface_code + ';errno=' + errno_code,
            file=__import__('sys').stderr,
            flush=True,
        )
        raise safe_error from None
    print(
        "cooking probe " + stage + ": surfaces=" + _PROBE_SURFACE_LABELS
        + ";files=" + str(evidence["scanned_files"])
        + ";bytes=" + str(evidence["scanned_bytes"]),
        file=__import__('sys').stderr,
        flush=True,
    )
'''


def transform(
    source: Path,
    destination: Path,
    runtime_source: Path | None = None,
    descriptor_source: Path | None = None,
) -> None:
    text = source.read_text(encoding="utf-8")
    if "_CRASH_STAGE_BY_UPDATE" in text or "_crash_once(update_id" in text:
        raise ValueError("source already contains guest crash hooks")
    imports = "import json\n"
    if text.count(imports) != 1:
        raise ValueError("expected exactly one json import")
    text = text.replace(imports, "import json\nimport os\nimport signal\n", 1)
    function_anchor = "\n\ndef encoded(value):\n"
    if text.count(function_anchor) != 1:
        raise ValueError("encoded function anchor is missing or duplicated")
    embedded_probe = ""
    if (runtime_source is None) != (descriptor_source is None):
        raise ValueError("runtime and descriptor sources must be supplied together")
    if runtime_source is not None and descriptor_source is not None:
        embedded_probe = probe_source(runtime_source, descriptor_source)
    text = text.replace(function_anchor, HELPER + embedded_probe + function_anchor, 1)
    for stage, (before, after) in ANCHORS.items():
        if text.count(before) != 1:
            raise ValueError(f"{stage}: expected one exact source anchor")
        text = text.replace(before, after, 1)
    if embedded_probe:
        main_anchor = (
            "    parameters = json.loads((control / 'parameters.json').read_bytes())\n"
            "    identity = process(db, json.loads(raw), Broker(parameters), Path('/workspace/work'))\n"
            "    print(json.dumps({'result_identity': identity, 'disposition': 'proposal_ready'}))\n"
        )
        main_replacement = (
            "    parameters = json.loads((control / 'parameters.json').read_bytes())\n"
            "    _emit_probe('before')\n"
            "    identity = process(db, json.loads(raw), Broker(parameters), Path('/workspace/work'))\n"
            "    _emit_probe('after')\n"
            "    print(json.dumps({'result_identity': identity, 'disposition': 'proposal_ready'}))\n"
        )
        if text.count(main_anchor) != 1:
            raise ValueError("cooking main probe anchor is missing or duplicated")
        text = text.replace(main_anchor, main_replacement, 1)
    destination.parent.mkdir(parents=True, exist_ok=True)
    destination.write_text(text, encoding="utf-8")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    parser.add_argument("destination", type=Path)
    parser.add_argument("--runtime-source", type=Path)
    parser.add_argument("--descriptor-source", type=Path)
    args = parser.parse_args()
    transform(args.source, args.destination, args.runtime_source, args.descriptor_source)
    print("wrote one-shot guest crash variant with stages 51,52,53,54,55")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
