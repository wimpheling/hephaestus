"""Deterministic child-process checks for the cooking guest crash variant."""
from __future__ import annotations

import argparse
import contextlib
import importlib.util
import io
import json
import os
from pathlib import Path
import signal
import sqlite3
import subprocess
import sys
import tempfile
import unittest
from urllib.parse import quote


STAGES = {
    51: "sqlite_before_commit",
    52: "sqlite_after_commit_before_model",
    53: "model_response_before_persist",
    54: "relay_return_before_persist",
    55: "proposal_ready_before_exit",
}
PHYSICAL_CALLS = {
    51: {"model": 1, "telegram_relay": 1},
    52: {"model": 1, "telegram_relay": 1},
    53: {"model": 2, "telegram_relay": 1},
    54: {"model": 1, "telegram_relay": 2},
    55: {"model": 1, "telegram_relay": 1},
}


def load_transformer():
    path = Path(__file__).resolve().with_name("guest_crash.py")
    spec = importlib.util.spec_from_file_location("cooking_guest_crash_transformer", path)
    if spec is None or spec.loader is None:
        raise RuntimeError("cannot load guest crash transformer")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def load_relay():
    path = Path(__file__).resolve().parents[1] / "telegram-relay" / "relay.py"
    spec = importlib.util.spec_from_file_location("cooking_guest_crash_relay", path)
    if spec is None or spec.loader is None:
        raise RuntimeError("cannot load deterministic relay")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class DeterministicUpstream:
    """Deterministic model plus the real sibling relay implementation."""

    def __init__(self, metrics_path: str, relay_path: str):
        self.metrics_path = metrics_path
        relay_module = load_relay()
        self.relay = relay_module.Relay(relay_path, b"guest-crash-fixture")

    def __call__(self, slot: str, body: dict[str, object]) -> dict[str, object]:
        identity = str(body["idempotency_key"])
        with sqlite3.connect(self.metrics_path) as db:
            db.execute(
                "INSERT INTO calls(slot, identity, body) VALUES(?,?,?)",
                (slot, identity, json.dumps(body, sort_keys=True)),
            )
        if slot == "model":
            return {
                "title": "Deterministic " + identity,
                "summary": "summary " + identity,
                "ingredients": ["ingredient " + identity],
                "steps": ["cook " + identity],
            }
        if slot != "telegram_relay":
            raise AssertionError("unexpected upstream slot")
        status, outcome = self.relay.deliver(
            "Bearer guest-crash-fixture",
            json.dumps(body, sort_keys=True, separators=(",", ":")).encode(),
        )
        if status != 200:
            raise AssertionError(f"deterministic relay status {status}")
        return outcome

    def close(self) -> None:
        self.relay.db.close()


def child(args: argparse.Namespace) -> int:
    spec = importlib.util.spec_from_file_location("guest_crash_agent", args.source)
    if spec is None or spec.loader is None:
        raise RuntimeError("cannot load transformed source")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    event = {
        "provider_update_id": args.update_id,
        "user_id": "alice",
        "command": "recipe",
        "text": "deterministic stage " + str(args.update_id),
    }
    db = module.connect(args.db)
    upstream = DeterministicUpstream(args.metrics, args.relay_db)
    try:
        identity = module.process(
            db,
            event,
            upstream,
            Path(args.work),
            check=False,
        )
        print(json.dumps({"result_identity": identity, "disposition": "proposal_ready"}))
    finally:
        upstream.close()
        db.close()
    return 0


def open_read_only(path: str) -> sqlite3.Connection:
    uri = "file:" + quote(path, safe="/") + "?mode=ro"
    return sqlite3.connect(uri, uri=True)


def state(path: str) -> dict[str, object]:
    with open_read_only(path) as db:
        processed = db.execute(
            "SELECT update_id,disposition,result_identity,event FROM processed_updates"
        ).fetchall()
        recipes = db.execute(
            "SELECT recipe_id,user_id,request_summary,publication_outcome,model_response,relay_outcome,context FROM recipes"
        ).fetchall()
        summaries = db.execute(
            "SELECT user_id,summary FROM conversation_summaries"
        ).fetchall()
    return {"processed": processed, "recipes": recipes, "summaries": summaries}


def ledger_state(metrics_path: str, relay_path: str) -> dict[str, object]:
    with sqlite3.connect(metrics_path) as db:
        calls = db.execute(
            "SELECT slot,identity,body FROM calls ORDER BY rowid"
        ).fetchall()
    with sqlite3.connect(relay_path) as db:
        relay = db.execute(
            "SELECT key,request,outcome FROM deliveries ORDER BY rowid"
        ).fetchall()
    return {"calls": calls, "relay": relay}


def run_child(
    source: str,
    update_id: int,
    db: str,
    metrics: str,
    relay_db: str,
    marker: str,
    work: str,
) -> subprocess.CompletedProcess[str]:
    env = os.environ.copy()
    env["HEPHAESTUS_COOKING_CRASH_MARKER_ROOT"] = marker
    command = [
        sys.executable,
        str(Path(__file__).resolve()),
        "--child",
        "--source",
        source,
        "--update-id",
        str(update_id),
        "--db",
        db,
        "--metrics",
        metrics,
        "--relay-db",
        relay_db,
        "--marker",
        marker,
        "--work",
        work,
    ]
    return subprocess.run(
        command,
        env=env,
        text=True,
        capture_output=True,
        timeout=15,
        check=False,
    )


def assert_pre(update_id: int, current: dict[str, object], ledger: dict[str, object]) -> None:
    processed = current["processed"]
    recipes = current["recipes"]
    summaries = current["summaries"]
    relay = ledger["relay"]
    if update_id == 51:
        if processed or recipes or summaries or relay:
            raise AssertionError((update_id, current, ledger))
        return
    if update_id == 55:
        if len(processed) != 1 or processed[0][0:2] != (update_id, "completed"):
            raise AssertionError((update_id, processed))
        if len(recipes) != 1 or len(summaries) != 1 or len(relay) != 1:
            raise AssertionError((update_id, current, ledger))
        recipe = recipes[0]
        if recipe[0:4] != (
            "recipe-" + str(update_id),
            "alice",
            "deterministic stage " + str(update_id),
            "proposal_ready",
        ):
            raise AssertionError((update_id, recipe))
        if recipe[4] is None or recipe[5] is None:
            raise AssertionError((update_id, recipe))
        if json.loads(recipe[6]) != {"recent_recipes": [], "summary": ""}:
            raise AssertionError((update_id, recipe[6]))
        return
    if len(processed) != 1 or processed[0][0:2] != (update_id, "pending"):
        raise AssertionError((update_id, processed))
    if len(recipes) != 1 or len(summaries) != 1:
        raise AssertionError((update_id, current))
    recipe = recipes[0]
    if recipe[0:4] != (
        "recipe-" + str(update_id),
        "alice",
        "deterministic stage " + str(update_id),
        "pending",
    ):
        raise AssertionError((update_id, recipe))
    if json.loads(recipe[6]) != {"recent_recipes": [], "summary": ""}:
        raise AssertionError((update_id, recipe[6]))
    if update_id in (52, 53):
        if recipe[4] is not None or recipe[5] is not None or relay:
            raise AssertionError((update_id, recipe, relay))
    elif update_id == 54:
        if recipe[4] is None or recipe[5] is not None or len(relay) != 1:
            raise AssertionError((update_id, recipe, relay))


def assert_post(update_id: int, current: dict[str, object]) -> None:
    processed = current["processed"]
    recipes = current["recipes"]
    summaries = current["summaries"]
    if len(processed) != 1 or processed[0][0:3] != (
        update_id,
        "completed",
        "recipe-" + str(update_id),
    ):
        raise AssertionError((update_id, processed))
    if len(recipes) != 1 or len(summaries) != 1:
        raise AssertionError((update_id, current))
    recipe = recipes[0]
    if recipe[0:4] != (
        "recipe-" + str(update_id),
        "alice",
        "deterministic stage " + str(update_id),
        "proposal_ready",
    ):
        raise AssertionError((update_id, recipe))
    if recipe[4] is None or recipe[5] is None:
        raise AssertionError((update_id, recipe))
    if json.loads(recipe[6]) != {"recent_recipes": [], "summary": ""}:
        raise AssertionError((update_id, recipe[6]))
    if summaries != [("alice", "summary recipe-" + str(update_id))]:
        raise AssertionError((update_id, summaries))


def run_case(source: str, update_id: int, root: Path) -> dict[str, object]:
    root.mkdir(parents=True, exist_ok=True)
    db_path = root / "cooking.sqlite3"
    metrics_path = root / "metrics.sqlite3"
    relay_path = root / "relay.sqlite3"
    marker_path = root / "markers"
    work_path = root / "work"
    with sqlite3.connect(metrics_path) as db:
        db.executescript(
            """
            CREATE TABLE calls(slot TEXT NOT NULL, identity TEXT NOT NULL, body TEXT NOT NULL);
            """
        )
    first = run_child(
        source, update_id, str(db_path), str(metrics_path), str(relay_path),
        str(marker_path), str(work_path),
    )
    if first.returncode != -signal.SIGKILL or first.stdout:
        raise AssertionError((update_id, first.returncode, first.stdout, first.stderr))
    before = state(str(db_path))
    before_ledger = ledger_state(str(metrics_path), str(relay_path))
    assert_pre(update_id, before, before_ledger)
    marker = marker_path / (str(update_id) + "-" + STAGES[update_id])
    if not marker.is_file() or marker.read_text() != str(update_id) + "\n" + STAGES[update_id] + "\n":
        raise AssertionError((update_id, marker))
    retry = run_child(
        source, update_id, str(db_path), str(metrics_path), str(relay_path),
        str(marker_path), str(work_path),
    )
    if retry.returncode != 0:
        raise AssertionError((update_id, retry.returncode, retry.stderr))
    if json.loads(retry.stdout)["result_identity"] != "recipe-" + str(update_id):
        raise AssertionError((update_id, retry.stdout))
    after = state(str(db_path))
    after_ledger = ledger_state(str(metrics_path), str(relay_path))
    assert_post(update_id, after)
    calls = after_ledger["calls"]
    counts = {
        slot: sum(1 for call in calls if call[0] == slot)
        for slot in ("model", "telegram_relay")
    }
    if counts != PHYSICAL_CALLS[update_id] or len(after_ledger["relay"]) != 1:
        raise AssertionError((update_id, counts, after_ledger))
    return {
        "update_id": update_id,
        "stage": STAGES[update_id],
        "first_returncode": first.returncode,
        "retry_returncode": retry.returncode,
        "model_calls": counts["model"],
        "relay_calls": counts["telegram_relay"],
        "relay_ledger_rows": len(after_ledger["relay"]),
        "recipes": len(after["recipes"]),
    }


class GuestCrashRegression(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.temp = tempfile.TemporaryDirectory(prefix="heph-guest-crash-")
        cls.addClassCleanup(cls.temp.cleanup)
        cls.root = Path(cls.temp.name)
        cls.source = cls.root / "variant" / "cooking_agent.py"
        canonical = Path(__file__).resolve().parents[1] / "cooking-agent" / "cooking_agent.py"
        transformer = load_transformer()
        transformer.transform(canonical, cls.source)
        generated = cls.source.read_text(encoding="utf-8")
        initial_summary = "INSERT OR REPLACE INTO conversation_summaries VALUES(?,?)', (event['user_id'], event['text'][:512]))"
        if generated.count(initial_summary) != 1:
            raise AssertionError("generated variant lost or duplicated the initial summary write")
        hook_count = sum(
            generated.count(f"_crash_once(update_id, '{stage}')")
            for stage in STAGES.values()
        )
        if hook_count != 5:
            raise AssertionError(f"expected five guest crash hooks, found {hook_count}")

    def run_stage(self, update_id: int):
        result = run_case(self.source.as_posix(), update_id, self.root / str(update_id))
        self.assertEqual(result["first_returncode"], -signal.SIGKILL)
        self.assertEqual(result["retry_returncode"], 0)
        self.assertEqual(result["recipes"], 1)

    def test_probe_failure_emits_only_allowlisted_stage_and_reason(self):
        transformer = load_transformer()
        runtime_source = Path(__file__).with_name("guest_confinement.py")
        canonical = Path(__file__).resolve().parents[1] / "cooking-agent" / "cooking_agent.py"
        with tempfile.TemporaryDirectory(prefix="heph-guest-probe-classifier-") as temporary:
            root = Path(temporary)
            descriptor = root / "descriptors.json"
            descriptor.write_text(json.dumps({"version": 1, "descriptors": []}))
            variant = root / "cooking_agent.py"
            transformer.transform(canonical, variant, runtime_source, descriptor)
            spec = importlib.util.spec_from_file_location("guest_probe_classifier", variant)
            if spec is None or spec.loader is None:
                raise RuntimeError("cannot load generated probe variant")
            module = importlib.util.module_from_spec(spec)
            sys.modules[spec.name] = module
            authority_path = os.environ.get("HEPH_RUNTIME_AUTHORITY_PATH")
            os.environ["HEPH_RUNTIME_AUTHORITY_PATH"] = str(root / "authority")
            try:
                spec.loader.exec_module(module)
            finally:
                if authority_path is None:
                    os.environ.pop("HEPH_RUNTIME_AUTHORITY_PATH", None)
                else:
                    os.environ["HEPH_RUNTIME_AUTHORITY_PATH"] = authority_path

            def fail_with_known_reason(*_args, **_kwargs):
                raise module.ProbeError("probe invocation exceeds the bounded scan size")

            module.runtime_probe = fail_with_known_reason
            output = io.StringIO()
            with contextlib.redirect_stderr(output):
                with self.assertRaises(module.ProbeError) as failure:
                    module._emit_probe("before")
            self.assertEqual(
                output.getvalue(),
                "cooking probe failed: stage=before;reason=budget_total;surface=unknown;errno=none\n",
            )
            self.assertEqual(str(failure.exception), "probe failed: stage=before;reason=budget_total")

            def fail_with_unknown_reason(*_args, **_kwargs):
                raise module.ProbeError("raw path and secret must stay hidden")

            module.runtime_probe = fail_with_unknown_reason
            output = io.StringIO()
            with contextlib.redirect_stderr(output):
                with self.assertRaises(module.ProbeError) as failure:
                    module._emit_probe("unexpected")
            self.assertEqual(
                output.getvalue(),
                "cooking probe failed: stage=unknown;reason=unknown;surface=unknown;errno=none\n",
            )
            self.assertEqual(str(failure.exception), "probe failed: stage=unknown;reason=unknown")

            def fail_with_enumeration_reason(*_args, **_kwargs):
                raise module.ProbeError(
                    "probe could not enumerate a guest surface",
                    surface_code="proc-env",
                    errno_code="permission",
                )

            module.runtime_probe = fail_with_enumeration_reason
            output = io.StringIO()
            with contextlib.redirect_stderr(output):
                with self.assertRaises(module.ProbeError) as failure:
                    module._emit_probe("after")
            self.assertEqual(
                output.getvalue(),
                "cooking probe failed: stage=after;reason=enumeration;surface=proc-env;errno=permission\n",
            )
            self.assertEqual(str(failure.exception), "probe failed: stage=after;reason=enumeration")

    def test_sqlite_before_commit_sigkill_and_retry(self):
        self.run_stage(51)

    def test_sqlite_after_commit_before_model_sigkill_and_retry(self):
        self.run_stage(52)

    def test_model_response_before_persist_sigkill_and_retry(self):
        self.run_stage(53)

    def test_relay_return_before_persist_sigkill_and_retry(self):
        self.run_stage(54)

    def test_proposal_ready_before_exit_sigkill_and_retry(self):
        self.run_stage(55)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--child", action="store_true")
    parser.add_argument("--source")
    parser.add_argument("--update-id", type=int)
    parser.add_argument("--db")
    parser.add_argument("--metrics")
    parser.add_argument("--relay-db")
    parser.add_argument("--marker")
    parser.add_argument("--work")
    args = parser.parse_args()
    if args.child:
        required = ("source", "update_id", "db", "metrics", "relay_db", "marker", "work")
        missing = [name for name in required if getattr(args, name, None) is None]
        if missing:
            parser.error("child mode missing: " + ", ".join(missing))
        return child(args)
    unittest.main(argv=[sys.argv[0]], verbosity=2)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
