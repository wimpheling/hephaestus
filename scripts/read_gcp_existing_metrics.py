#!/usr/bin/env python3
"""Read a bounded set of provider metrics for an existing Cooking run.

This command is deliberately read-only.  It validates the GitHub run before
deriving the VM name and time window, then uses Cloud Monitoring's
projects.timeSeries.list endpoint for a fixed metric allowlist.  The output is
safe to retain: arbitrary response labels and error bodies never leave this
process.
"""

from __future__ import annotations

import argparse
from dataclasses import dataclass
from datetime import datetime, timedelta, timezone
import json
import os
from pathlib import Path
import subprocess
import sys
from typing import Any, Mapping, Protocol
from urllib.error import HTTPError, URLError
from urllib.parse import quote, urlencode
from urllib.request import Request, urlopen


EXPECTED_REPOSITORY = "wimpheling/hephaestus"
EXPECTED_WORKFLOW = ".github/workflows/cooking-e2e.yml"
EXPECTED_EVENT = "workflow_dispatch"
PROJECT_ID = "hephaestus-508000"
ZONE = "europe-west1-d"
MAX_WINDOW = timedelta(hours=2)
SAMPLE_MARGIN = timedelta(minutes=5)
MAX_PAGES = 10
METRICS = (
    "compute.googleapis.com/instance/cpu/utilization",
    "compute.googleapis.com/instance/cpu/reserved_cores",
    "compute.googleapis.com/instance/disk/read_bytes_count",
    "compute.googleapis.com/instance/disk/read_ops_count",
    "compute.googleapis.com/instance/disk/write_bytes_count",
    "compute.googleapis.com/instance/disk/write_ops_count",
)


class MetricsError(Exception):
    """A safe, user-facing validation or transport failure."""


class Transport(Protocol):
    def get(self, url: str, headers: Mapping[str, str], timeout: float) -> tuple[int, bytes]:
        """Return an HTTP status and body for a GET request."""


class UrllibTransport:
    def get(self, url: str, headers: Mapping[str, str], timeout: float) -> tuple[int, bytes]:
        request = Request(url, headers=dict(headers), method="GET")
        try:
            with urlopen(request, timeout=timeout) as response:
                return int(response.status), response.read()
        except HTTPError as error:
            # Do not read or report the error body: it can contain data outside
            # the safe output schema.
            raise MetricsError(f"HTTP error status={error.code}") from None
        except (URLError, TimeoutError, OSError) as error:
            raise MetricsError(f"transport failure={type(error).__name__}") from None


@dataclass(frozen=True)
class RunWindow:
    run_id: int
    attempt: int
    source_sha: str
    created_at: str
    updated_at: str
    window_start: str
    window_end: str
    vm_name: str


def _parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--run-id", required=True, help="completed Cooking workflow run ID")
    parser.add_argument("--attempt", default="1", help="completed workflow run attempt")
    parser.add_argument("--repository", default=EXPECTED_REPOSITORY)
    parser.add_argument("--github-api-base", default="https://api.github.com")
    parser.add_argument("--monitoring-api-base", default="https://monitoring.googleapis.com")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--timeout", type=float, default=15.0)
    parser.add_argument(
        "--access-token",
        help=argparse.SUPPRESS,
    )
    return parser.parse_args()


def _integer(value: str, name: str, *, positive: bool = True) -> int:
    if not value.isascii() or not value.isdecimal():
        raise MetricsError(f"{name} must be numeric")
    parsed = int(value)
    if positive and parsed <= 0:
        raise MetricsError(f"{name} must be positive")
    return parsed


def _json_object(body: bytes, context: str) -> dict[str, Any]:
    try:
        value = json.loads(body)
    except (UnicodeDecodeError, json.JSONDecodeError):
        raise MetricsError(f"malformed {context} response") from None
    if not isinstance(value, dict):
        raise MetricsError(f"malformed {context} response")
    return value


def _get_json(
    transport: Transport,
    url: str,
    headers: Mapping[str, str],
    timeout: float,
    context: str,
) -> dict[str, Any]:
    try:
        status, body = transport.get(url, headers, timeout)
    except MetricsError:
        raise
    if status < 200 or status >= 300:
        raise MetricsError(f"{context} HTTP status={status}")
    return _json_object(body, context)


def _parse_timestamp(value: Any, field: str) -> datetime:
    if not isinstance(value, str):
        raise MetricsError(f"run metadata field {field} is invalid")
    try:
        parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError:
        raise MetricsError(f"run metadata field {field} is invalid") from None
    if parsed.tzinfo is None:
        raise MetricsError(f"run metadata field {field} is invalid")
    return parsed.astimezone(timezone.utc)


def _rfc3339(value: datetime) -> str:
    return value.astimezone(timezone.utc).isoformat(timespec="seconds").replace("+00:00", "Z")


def _metadata_window(
    metadata: Mapping[str, Any],
    run_id: int,
    attempt: int,
    repository: str,
) -> RunWindow:
    if metadata.get("id") != run_id or metadata.get("run_attempt") != attempt:
        raise MetricsError("GitHub run identity mismatch")
    if metadata.get("path") != EXPECTED_WORKFLOW:
        raise MetricsError("GitHub run workflow path is not Cooking E2E")
    if metadata.get("event") != EXPECTED_EVENT:
        raise MetricsError("GitHub run event is not workflow_dispatch")
    if metadata.get("head_branch") != "main":
        raise MetricsError("GitHub run is not from main")
    repository_info = metadata.get("repository")
    if not isinstance(repository_info, dict) or repository_info.get("full_name") != repository:
        raise MetricsError("GitHub run repository mismatch")
    if metadata.get("status") != "completed" or metadata.get("conclusion") != "success":
        raise MetricsError("GitHub run is not a successful completed run")
    head_sha = metadata.get("head_sha")
    if (
        not isinstance(head_sha, str)
        or len(head_sha) != 40
        or any(c not in "0123456789abcdef" for c in head_sha.lower())
    ):
        raise MetricsError("GitHub run source SHA is invalid")
    started = _parse_timestamp(
        metadata.get("run_started_at") or metadata.get("created_at"), "run_started_at"
    )
    created = _parse_timestamp(metadata.get("created_at"), "created_at")
    updated = _parse_timestamp(metadata.get("updated_at"), "updated_at")
    if updated < started or updated < created:
        raise MetricsError("GitHub run timestamps are inconsistent")
    window_start = started - SAMPLE_MARGIN
    window_end = updated + SAMPLE_MARGIN
    if window_end <= window_start or window_end - window_start > MAX_WINDOW:
        raise MetricsError("derived metric window exceeds two-hour bound")
    return RunWindow(
        run_id=run_id,
        attempt=attempt,
        source_sha=head_sha,
        created_at=_rfc3339(created),
        updated_at=_rfc3339(updated),
        window_start=_rfc3339(window_start),
        window_end=_rfc3339(window_end),
        vm_name=f"heph-kvm-smoke-{run_id}-{attempt}",
    )


def _token_from_gcloud() -> str:
    try:
        result = subprocess.run(
            ["gcloud", "auth", "print-access-token"],
            check=True,
            capture_output=True,
            text=True,
            timeout=30,
        )
    except (OSError, subprocess.SubprocessError):
        raise MetricsError("gcloud access token unavailable") from None
    token = result.stdout.strip()
    if not token or any(character.isspace() for character in token):
        raise MetricsError("gcloud access token unavailable")
    return token


def _github_url(base: str, repository: str, run_id: int, attempt: int) -> str:
    encoded_repository = quote(repository, safe="/")
    return f"{base.rstrip('/')}/repos/{encoded_repository}/actions/runs/{run_id}/attempts/{attempt}"


def _monitoring_url(base: str, metric: str, window: RunWindow, page_token: str | None) -> str:
    filter_value = " AND ".join(
        (
            'resource.type="gce_instance"',
            f'resource.labels.project_id="{PROJECT_ID}"',
            f'resource.labels.zone="{ZONE}"',
            f'metric.type="{metric}"',
            f'metric.labels.instance_name="{window.vm_name}"',
        )
    )
    query = {
        "filter": filter_value,
        "interval.startTime": window.window_start,
        "interval.endTime": window.window_end,
        "view": "FULL",
        "pageSize": "1000",
    }
    if page_token is not None:
        query["pageToken"] = page_token
    return f"{base.rstrip('/')}/v3/projects/{PROJECT_ID}/timeSeries?{urlencode(query)}"


def _number(value: Any, key: str) -> int | float:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        if key == "int64Value" and isinstance(value, str) and value.isascii() and value.isdecimal():
            return int(value)
        raise MetricsError("metric point value is not numeric")
    return value


def _read_metric(
    transport: Transport,
    base: str,
    token: str,
    metric: str,
    window: RunWindow,
    timeout: float,
) -> dict[str, Any]:
    headers = {"Authorization": f"Bearer {token}", "Accept": "application/json"}
    series: list[dict[str, Any]] = []
    page_token: str | None = None
    for _page in range(MAX_PAGES):
        response = _get_json(
            transport,
            _monitoring_url(base, metric, window, page_token),
            headers,
            timeout,
            "Monitoring",
        )
        time_series = response.get("timeSeries")
        if not isinstance(time_series, list):
            raise MetricsError("malformed Monitoring response")
        for item in time_series:
            if not isinstance(item, dict):
                raise MetricsError("malformed Monitoring response")
            resource = item.get("resource")
            points = item.get("points")
            if not isinstance(resource, dict) or resource.get("type") != "gce_instance":
                raise MetricsError("malformed Monitoring resource")
            resource_labels = resource.get("labels")
            if not isinstance(resource_labels, dict):
                raise MetricsError("malformed Monitoring resource labels")
            instance_id = resource_labels.get("instance_id")
            if (
                not isinstance(instance_id, str)
                or not instance_id.isascii()
                or not instance_id.isdecimal()
            ):
                raise MetricsError("malformed Monitoring instance ID")
            if not isinstance(points, list):
                raise MetricsError("malformed Monitoring points")
            safe_points: list[dict[str, Any]] = []
            for point in points:
                if not isinstance(point, dict):
                    raise MetricsError("malformed Monitoring point")
                interval = point.get("interval")
                value = point.get("value")
                if not isinstance(interval, dict) or not isinstance(value, dict):
                    raise MetricsError("malformed Monitoring point")
                timestamp = interval.get("endTime")
                if not isinstance(timestamp, str):
                    raise MetricsError("malformed Monitoring timestamp")
                numeric_values = [
                    (key, candidate)
                    for key, candidate in value.items()
                    if key in ("doubleValue", "int64Value")
                ]
                if len(numeric_values) != 1:
                    raise MetricsError("malformed Monitoring numeric value")
                key, candidate = numeric_values[0]
                safe_points.append({"timestamp": timestamp, "value": _number(candidate, key)})
            series.append({"instance_id": int(instance_id), "points": safe_points})
        page_token_value = response.get("nextPageToken")
        if page_token_value is None or page_token_value == "":
            break
        if not isinstance(page_token_value, str) or len(page_token_value) > 4096:
            raise MetricsError("malformed Monitoring page token")
        page_token = page_token_value
    else:
        raise MetricsError("Monitoring pagination limit exceeded")
    return {"metric_type": metric, "series": series}


def run(
    args: argparse.Namespace,
    transport: Transport | None = None,
    token: str | None = None,
) -> dict[str, Any]:
    if args.repository != EXPECTED_REPOSITORY:
        raise MetricsError("repository is not the trusted Hephaestus repository")
    run_id = _integer(args.run_id, "run ID")
    attempt = _integer(args.attempt, "attempt")
    if args.timeout <= 0 or args.timeout > 60:
        raise MetricsError("timeout must be between one and 60 seconds")
    transport = transport or UrllibTransport()
    github_token = os.environ.get("GH_TOKEN")
    if not github_token:
        raise MetricsError("GitHub read token unavailable")
    metadata = _get_json(
        transport,
        _github_url(args.github_api_base, args.repository, run_id, attempt),
        {"Authorization": f"Bearer {github_token}", "Accept": "application/vnd.github+json"},
        args.timeout,
        "GitHub",
    )
    window = _metadata_window(metadata, run_id, attempt, args.repository)
    access_token = token or args.access_token or _token_from_gcloud()
    metrics = [
        _read_metric(transport, args.monitoring_api_base, access_token, metric, window, args.timeout)
        for metric in METRICS
    ]
    return {
        "schema_version": 1,
        "provenance": {
            "repository": args.repository,
            "workflow_path": EXPECTED_WORKFLOW,
            "event": EXPECTED_EVENT,
            "run_id": window.run_id,
            "run_attempt": window.attempt,
            "status": "completed",
            "conclusion": "success",
            "source_sha": window.source_sha,
            "project_id": PROJECT_ID,
            "zone": ZONE,
            "vm_name": window.vm_name,
            "created_at": window.created_at,
            "updated_at": window.updated_at,
            "window_start": window.window_start,
            "window_end": window.window_end,
        },
        "metrics": metrics,
    }


def main() -> int:
    try:
        args = _parse_args()
        result = run(args)
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(
            json.dumps(result, sort_keys=True, separators=(",", ":")) + "\n", encoding="utf-8"
        )
    except MetricsError as error:
        print(f"metrics query failed: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
