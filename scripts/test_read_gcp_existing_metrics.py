"""Real-script transport fixtures for the read-only GCE metrics query."""

from __future__ import annotations

import importlib.util
import json
from pathlib import Path
from types import SimpleNamespace
import sys
import unittest
from unittest.mock import patch
from urllib.parse import parse_qs, urlparse


ROOT = Path(__file__).parent
SPEC = importlib.util.spec_from_file_location(
    "read_gcp_existing_metrics", ROOT / "read_gcp_existing_metrics.py"
)
assert SPEC is not None and SPEC.loader is not None
QUERY = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = QUERY
SPEC.loader.exec_module(QUERY)


RUN_ID = 34747950421
ATTEMPT = 1
SOURCE_SHA = "127f57e3707b687d4083c365de5537014abe7098"
INSTANCE_ID = "1234567890123456789"


class FixtureTransport:
    """Cloud and GitHub transport substitute; no GCE API is exposed."""

    def __init__(self, *, malformed: bool = False, status: int = 200) -> None:
        self.malformed = malformed
        self.status = status
        self.calls: list[tuple[str, str]] = []

    def get(self, url: str, headers: dict[str, str], timeout: float) -> tuple[int, bytes]:
        del headers, timeout
        parsed = urlparse(url)
        self.calls.append(("GET", url))
        if "/actions/runs/" in parsed.path:
            return 200, json.dumps(
                {
                    "id": RUN_ID,
                    "run_attempt": ATTEMPT,
                    "path": ".github/workflows/cooking-e2e.yml",
                    "event": "workflow_dispatch",
                    "head_branch": "main",
                    "head_sha": SOURCE_SHA,
                    "status": "completed",
                    "conclusion": "success",
                    "created_at": "2026-09-13T08:32:40Z",
                    "run_started_at": "2026-09-13T08:32:55Z",
                    "updated_at": "2026-09-13T08:55:20Z",
                    "repository": {"full_name": "wimpheling/hephaestus"},
                }
            ).encode()
        if self.status != 200:
            return self.status, b'{"error":"private failure detail"}'
        if self.malformed:
            return 200, b'{"timeSeries":[{"resource":null}]}'
        query = parse_qs(parsed.query)
        metric = query["filter"][0].split('metric.type="', 1)[1].split('"', 1)[0]
        value: dict[str, object]
        if metric.endswith("reserved_cores"):
            value = {"int64Value": "8"}
        else:
            value = {"doubleValue": 0.25}
        return 200, json.dumps(
            {
                "timeSeries": [
                    {
                        "resource": {
                            "type": "gce_instance",
                            "labels": {
                                "project_id": "hephaestus-508000",
                                "zone": "europe-west1-d",
                                "instance_id": INSTANCE_ID,
                            },
                        },
                        "points": [
                            {"interval": {"endTime": "2026-09-13T08:45:00Z"}, "value": value}
                        ],
                    }
                ]
            }
        ).encode()


def arguments(output: Path) -> SimpleNamespace:
    return SimpleNamespace(
        run_id=str(RUN_ID),
        attempt=str(ATTEMPT),
        repository="wimpheling/hephaestus",
        github_api_base="https://api.github.com",
        monitoring_api_base="https://monitoring.googleapis.com",
        output=output,
        timeout=5.0,
        access_token=None,
    )


class ReadExistingMetricsTests(unittest.TestCase):
    def test_workflow_is_main_only_and_reuses_read_only_auth_contract(self) -> None:
        workflow = (ROOT.parent / ".github/workflows/gcp-read-existing-metrics.yml").read_text(
            encoding="utf-8"
        )
        self.assertIn("github.repository == 'wimpheling/hephaestus'", workflow)
        self.assertIn("github.ref == 'refs/heads/main'", workflow)
        self.assertIn("actions: read", workflow)
        self.assertIn("id-token: write", workflow)
        self.assertIn(
            "projects/84572286146/locations/global/workloadIdentityPools/github-actions/providers/github",
            workflow,
        )
        self.assertIn("scripts/read_gcp_existing_metrics.py", workflow)
        self.assertIn("actions/upload-artifact@v4", workflow)
        self.assertNotIn("gcp-kvm-smoke.sh", workflow)

    def test_real_query_path_projects_allowlisted_points_and_exact_filter(self) -> None:
        transport = FixtureTransport()
        with patch.dict("os.environ", {"GH_TOKEN": "github-fixture-token"}, clear=False):
            result = QUERY.run(
                arguments(Path("unused.json")), transport=transport, token="gcp-fixture-token"
            )

        self.assertEqual(result["provenance"]["run_id"], RUN_ID)
        self.assertEqual(result["provenance"]["vm_name"], f"heph-kvm-smoke-{RUN_ID}-{ATTEMPT}")
        self.assertEqual(len(result["metrics"]), len(QUERY.METRICS))
        self.assertEqual(result["metrics"][1]["series"][0]["instance_id"], int(INSTANCE_ID))
        self.assertEqual(result["metrics"][1]["series"][0]["points"][0]["value"], 8)
        self.assertTrue(all(method == "GET" for method, _url in transport.calls))
        monitoring_calls = [
            url for _method, url in transport.calls if "monitoring.googleapis.com" in url
        ]
        self.assertEqual(len(monitoring_calls), len(QUERY.METRICS))
        for url in monitoring_calls:
            query = parse_qs(urlparse(url).query)
            self.assertIn('metric.type="', query["filter"][0])
            self.assertIn('metric.labels.instance_name="heph-kvm-smoke-', query["filter"][0])
            self.assertIn('resource.labels.zone="europe-west1-d"', query["filter"][0])
            self.assertEqual(query["view"], ["FULL"])
        safe_json = json.dumps(result)
        self.assertNotIn("instance_name", safe_json)
        self.assertNotIn("private failure detail", safe_json)

    def test_malformed_monitoring_response_fails_closed(self) -> None:
        transport = FixtureTransport(malformed=True)
        with patch.dict("os.environ", {"GH_TOKEN": "github-fixture-token"}, clear=False):
            with self.assertRaisesRegex(QUERY.MetricsError, "malformed Monitoring"):
                QUERY.run(
                    arguments(Path("unused.json")), transport=transport, token="gcp-fixture-token"
                )

    def test_http_error_exposes_status_without_response_body(self) -> None:
        transport = FixtureTransport(status=403)
        with patch.dict("os.environ", {"GH_TOKEN": "github-fixture-token"}, clear=False):
            with self.assertRaisesRegex(
                QUERY.MetricsError, r"Monitoring HTTP status=403"
            ) as raised:
                QUERY.run(
                    arguments(Path("unused.json")), transport=transport, token="gcp-fixture-token"
                )
        self.assertNotIn("private failure detail", str(raised.exception))

    def test_run_metadata_rejects_wrong_workflow_without_monitoring_call(self) -> None:
        transport = FixtureTransport()
        original_get = transport.get

        def wrong_workflow(url: str, headers: dict[str, str], timeout: float) -> tuple[int, bytes]:
            status, body = original_get(url, headers, timeout)
            if "/actions/runs/" in url:
                metadata = json.loads(body)
                metadata["path"] = ".github/workflows/other.yml"
                return status, json.dumps(metadata).encode()
            return status, body

        transport.get = wrong_workflow  # type: ignore[method-assign]
        with patch.dict("os.environ", {"GH_TOKEN": "github-fixture-token"}, clear=False):
            with self.assertRaisesRegex(QUERY.MetricsError, "workflow path"):
                QUERY.run(
                    arguments(Path("unused.json")), transport=transport, token="gcp-fixture-token"
                )
        self.assertEqual(len(transport.calls), 1)


if __name__ == "__main__":
    unittest.main()
