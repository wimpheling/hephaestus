"""Fail-closed quota and admission arithmetic for disposable GCP runners.

The shell controller owns gcloud authentication and VM creation. This module
owns only pure parsing and arithmetic so callers can pass real project-info,
regional, instance, and disk JSON snapshots without shell arithmetic or
double-counting an already admitted runner.
"""

from __future__ import annotations

import argparse
from dataclasses import dataclass
import json
import math
from pathlib import Path
import re
from typing import Any, Mapping, Sequence


CONTROLLER_PURPOSE = "hephaestus-kvm-smoke"
FULL_MACHINE_TYPE = "n2-standard-8"
FULL_DISK_GB = 150
DIAGNOSTIC_MACHINE_TYPE = "e2-small"
DIAGNOSTIC_DISK_GB = 20
FULL_RUNNER_SLOTS = 2
ACTIVE_STATUSES = frozenset(
    {"PROVISIONING", "STAGING", "RUNNING", "REPAIRING", "STOPPING", "SUSPENDING", "SUSPENDED"}
)
SHA_RE = re.compile(r"^[0-9a-f]{40}$")
RUN_ID_RE = re.compile(r"^[1-9][0-9]*$")
RUN_ATTEMPT_RE = re.compile(r"^[1-9][0-9]*$")


@dataclass(frozen=True)
class Resources:
    """Resources consumed by one valid runner allocation."""

    global_cpus: int = 0
    regional_cpus: int = 0
    regional_n2_cpus: int = 0
    ssd_gb: int = 0
    instances: int = 0
    addresses: int = 0

    def __add__(self, other: Resources) -> Resources:
        return Resources(
            global_cpus=self.global_cpus + other.global_cpus,
            regional_cpus=self.regional_cpus + other.regional_cpus,
            regional_n2_cpus=self.regional_n2_cpus + other.regional_n2_cpus,
            ssd_gb=self.ssd_gb + other.ssd_gb,
            instances=self.instances + other.instances,
            addresses=self.addresses + other.addresses,
        )

    def subtract_floor_zero(self, other: Resources) -> Resources:
        """Subtract observed allocations without producing negative needs."""

        return Resources(
            global_cpus=max(0, self.global_cpus - other.global_cpus),
            regional_cpus=max(0, self.regional_cpus - other.regional_cpus),
            regional_n2_cpus=max(0, self.regional_n2_cpus - other.regional_n2_cpus),
            ssd_gb=max(0, self.ssd_gb - other.ssd_gb),
            instances=max(0, self.instances - other.instances),
            addresses=max(0, self.addresses - other.addresses),
        )

    def as_dict(self) -> dict[str, int]:
        return {
            "global_cpus": self.global_cpus,
            "regional_cpus": self.regional_cpus,
            "regional_n2_cpus": self.regional_n2_cpus,
            "ssd_gb": self.ssd_gb,
            "instances": self.instances,
            "addresses": self.addresses,
        }


# n2-standard-8 with a 150 GB balanced boot disk, as used by full Cooking and
# session-chat runs. The two-runner aggregate is the admission ceiling.
FULL_RUNNER = Resources(global_cpus=8, regional_n2_cpus=8, ssd_gb=150, instances=1, addresses=1)
FULL_RUNNER_TARGET = Resources(global_cpus=16, regional_n2_cpus=16, ssd_gb=300, instances=2, addresses=2)

# diagnostic uses e2-small and its 20 GB disk. It deliberately does not
# consume N2_CPUS, preserving the existing mode's actual machine size.
DIAGNOSTIC_RUNNER = Resources(global_cpus=2, regional_cpus=2, ssd_gb=20, instances=1, addresses=1)


def diagnostic_runner(disk_gb: int = DIAGNOSTIC_DISK_GB) -> Resources:
    """Return diagnostic resources for stock (20 GB) or custom-image (150 GB) boots."""

    if disk_gb not in (DIAGNOSTIC_DISK_GB, FULL_DISK_GB):
        raise ValueError("diagnostic disk size must be 20 GB or 150 GB")
    return Resources(global_cpus=2, regional_cpus=2, ssd_gb=disk_gb, instances=1, addresses=1)


def _finite_nonnegative(value: object, *, field: str) -> float:
    if isinstance(value, bool):
        raise ValueError(f"invalid {field}: boolean")
    try:
        number = float(value)
    except (TypeError, ValueError) as error:
        raise ValueError(f"invalid {field}: not numeric") from error
    if not math.isfinite(number) or number < 0:
        raise ValueError(f"invalid {field}: must be finite and non-negative")
    return number


def _positive_integer(value: object, *, field: str) -> int:
    number = _finite_nonnegative(value, field=field)
    if not number.is_integer() or number < 1:
        raise ValueError(f"invalid {field}: must be a positive integer")
    return int(number)


def _quota_map(payload: Mapping[str, Any], *, scope: str) -> dict[str, dict[str, float]]:
    quotas = payload.get("quotas")
    if not isinstance(quotas, list):
        raise ValueError(f"{scope} quota snapshot has no quotas list")
    result: dict[str, dict[str, float]] = {}
    for item in quotas:
        if not isinstance(item, Mapping) or not isinstance(item.get("metric"), str):
            raise ValueError(f"{scope} quota snapshot has an invalid metric")
        metric = item["metric"]
        if metric in result:
            raise ValueError(f"{scope} quota snapshot repeats metric {metric}")
        result[metric] = {
            "limit": _finite_nonnegative(item.get("limit"), field=f"{scope}.{metric}.limit"),
            "usage": _finite_nonnegative(item.get("usage"), field=f"{scope}.{metric}.usage"),
        }
    return result


def parse_quota_snapshots(
    project_info: Mapping[str, Any], regional_info: Mapping[str, Any]
) -> dict[str, dict[str, dict[str, float]]]:
    """Parse the exact shapes returned by gcloud ``--format=json``."""

    return {
        "global": _quota_map(project_info, scope="global"),
        "regional": _quota_map(regional_info, scope="regional"),
    }


def _resource_name(url: object, *, kind: str) -> str:
    if not isinstance(url, str):
        raise ValueError(f"missing {kind} resource URL")
    name = url.rstrip("/").rsplit("/", 1)[-1]
    if not name or "/" not in url:
        raise ValueError(f"invalid {kind} resource URL")
    return name


def _instance_region(instance: Mapping[str, Any]) -> str:
    zone = _resource_name(instance.get("zone"), kind="zone")
    if "-" not in zone:
        raise ValueError("invalid instance zone")
    return zone.rsplit("-", 1)[0]


def _controller_labels(instance: Mapping[str, Any]) -> bool:
    labels = instance.get("labels")
    if not isinstance(labels, Mapping):
        return False
    return (
        labels.get("purpose") == CONTROLLER_PURPOSE
        and isinstance(labels.get("run_id"), str)
        and RUN_ID_RE.fullmatch(labels["run_id"]) is not None
        and isinstance(labels.get("run_attempt"), str)
        and RUN_ATTEMPT_RE.fullmatch(labels["run_attempt"]) is not None
        and isinstance(labels.get("sha"), str)
        and SHA_RE.fullmatch(labels["sha"]) is not None
    )


def _boot_disk_name(instance: Mapping[str, Any]) -> str:
    disks = instance.get("disks")
    if not isinstance(disks, list):
        raise ValueError("controller instance has no disk list")
    boots = [item for item in disks if isinstance(item, Mapping) and item.get("boot") is True]
    if len(boots) != 1:
        raise ValueError("controller instance must have exactly one boot disk")
    return _resource_name(boots[0].get("source"), kind="boot disk")


def _has_ephemeral_address(instance: Mapping[str, Any]) -> bool:
    interfaces = instance.get("networkInterfaces")
    if not isinstance(interfaces, list) or not interfaces:
        raise ValueError("controller instance has no network interface")
    return any(
        isinstance(interface, Mapping)
        and isinstance(interface.get("accessConfigs"), list)
        and any(
            isinstance(config, Mapping) and config.get("type") == "ONE_TO_ONE_NAT"
            for config in interface["accessConfigs"]
        )
        for interface in interfaces
    )


def controller_allocations(
    instances: Sequence[Mapping[str, Any]],
    disks: Sequence[Mapping[str, Any]],
    *,
    region: str,
) -> list[dict[str, object]]:
    """Extract only active, controller-owned VM allocations.

    An instance with controller labels but malformed machine/disk identity
    fails closed. VMs from other products, regions, or labels are ignored and
    cannot be subtracted from the full-runner target.
    """

    disk_by_name: dict[str, Mapping[str, Any]] = {}
    for disk in disks:
        if not isinstance(disk, Mapping) or not isinstance(disk.get("name"), str):
            raise ValueError("disk snapshot contains an invalid disk")
        if disk["name"] in disk_by_name:
            raise ValueError(f"disk snapshot repeats {disk['name']}")
        disk_by_name[disk["name"]] = disk

    allocations: list[dict[str, object]] = []
    for instance in instances:
        if not isinstance(instance, Mapping) or not _controller_labels(instance):
            continue
        if instance.get("status") not in ACTIVE_STATUSES:
            continue
        if _instance_region(instance) != region:
            continue
        machine_type = _resource_name(instance.get("machineType"), kind="machine type")
        disk = disk_by_name.get(_boot_disk_name(instance))
        if disk is None:
            raise ValueError(f"controller instance {instance.get('name')} boot disk is absent")
        disk_type = _resource_name(disk.get("type"), kind="disk type")
        disk_gb = _positive_integer(disk.get("sizeGb"), field="controller boot disk sizeGb")
        if not _has_ephemeral_address(instance):
            raise ValueError(f"controller instance {instance.get('name')} has no ephemeral address")

        if machine_type == FULL_MACHINE_TYPE and disk_type == "pd-balanced" and disk_gb == FULL_DISK_GB:
            resources, mode = FULL_RUNNER, "full"
        elif machine_type == DIAGNOSTIC_MACHINE_TYPE and disk_type == "pd-balanced" and disk_gb in (
            DIAGNOSTIC_DISK_GB,
            FULL_DISK_GB,
        ):
            resources, mode = diagnostic_runner(disk_gb), "diagnostic"
        else:
            raise ValueError(f"controller instance {instance.get('name')} has unsupported machine or disk size")
        allocations.append({"mode": mode, **resources.as_dict(), "name": instance.get("name", "")})
    return allocations


def _resources_from_allocation(allocation: Mapping[str, object]) -> Resources:
    def integer(name: str) -> int:
        number = _finite_nonnegative(allocation.get(name, 0), field=f"allocation.{name}")
        if not number.is_integer():
            raise ValueError(f"invalid allocation.{name}: must be an integer")
        return int(number)

    return Resources(
        global_cpus=integer("global_cpus"),
        regional_cpus=integer("regional_cpus"),
        regional_n2_cpus=integer("regional_n2_cpus"),
        ssd_gb=integer("ssd_gb"),
        instances=integer("instances"),
        addresses=integer("addresses"),
    )


def required_additional_resources(
    mode: str,
    allocations: Sequence[Mapping[str, object]] = (),
    *,
    diagnostic_disk_gb: int = DIAGNOSTIC_DISK_GB,
) -> Resources:
    """Return resources still needed for this admission."""

    if mode == "full":
        current_full = sum(
            (_resources_from_allocation(item) for item in allocations if item.get("mode") == "full"),
            Resources(),
        )
        return FULL_RUNNER_TARGET.subtract_floor_zero(current_full)
    if mode == "diagnostic":
        return diagnostic_runner(diagnostic_disk_gb)
    raise ValueError(f"unsupported runner mode: {mode}")


def _quota_value(quota: Mapping[str, Mapping[str, float]], key: str) -> float:
    value = quota.get(key)
    if not isinstance(value, Mapping):
        raise ValueError(f"missing quota metric {key}")
    limit = _finite_nonnegative(value.get("limit"), field=f"quota.{key}.limit")
    usage = _finite_nonnegative(value.get("usage"), field=f"quota.{key}.usage")
    return limit - usage


def quota_shortfalls(
    mode: str,
    quotas: Mapping[str, Mapping[str, Mapping[str, object]]],
    allocations: Sequence[Mapping[str, object]] = (),
    *,
    diagnostic_disk_gb: int = DIAGNOSTIC_DISK_GB,
) -> dict[str, float]:
    """Return unavailable quota amounts, or an empty mapping when admissible."""

    required = required_additional_resources(mode, allocations, diagnostic_disk_gb=diagnostic_disk_gb)
    metrics = {
        "CPUS_ALL_REGIONS": ("global", required.global_cpus),
        "SSD_TOTAL_GB": ("regional", required.ssd_gb),
        "INSTANCES": ("regional", required.instances),
        "IN_USE_ADDRESSES": ("regional", required.addresses),
    }
    if required.regional_n2_cpus:
        metrics["N2_CPUS"] = ("regional", required.regional_n2_cpus)
    if required.regional_cpus:
        metrics["CPUS"] = ("regional", required.regional_cpus)

    shortfalls: dict[str, float] = {}
    for metric, (scope, need) in metrics.items():
        if need <= 0:
            continue
        available = _quota_value(quotas.get(scope, {}), metric)
        if available < need:
            shortfalls[metric] = need - available
    return shortfalls


def evaluate(
    mode: str,
    quotas: Mapping[str, Mapping[str, Mapping[str, object]]],
    allocations: Sequence[Mapping[str, object]] = (),
    *,
    diagnostic_disk_gb: int = DIAGNOSTIC_DISK_GB,
) -> dict[str, object]:
    """Return a safe admission result suitable for a controller error message."""

    full_count = sum(1 for item in allocations if item.get("mode") == "full")
    errors: list[str] = []
    if mode == "full" and full_count >= FULL_RUNNER_SLOTS:
        errors.append("full-runner-admission-limit-reached")
    shortfalls = quota_shortfalls(mode, quotas, allocations, diagnostic_disk_gb=diagnostic_disk_gb)
    if shortfalls:
        errors.append("quota-insufficient")
    return {
        "admissible": not errors,
        "mode": mode,
        "active_full_allocations": full_count,
        "required_additional": required_additional_resources(
            mode, allocations, diagnostic_disk_gb=diagnostic_disk_gb
        ).as_dict(),
        "shortfalls": shortfalls,
        "errors": errors,
    }


def _read_json(path: Path) -> Any:
    try:
        with path.open(encoding="utf-8") as handle:
            return json.load(handle)
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
        raise ValueError(f"cannot read JSON snapshot {path}") from error


def _cli() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--mode", choices=("full", "diagnostic"), required=True)
    parser.add_argument("--region", required=True)
    parser.add_argument("--project-info", type=Path, required=True)
    parser.add_argument("--regional-info", type=Path, required=True)
    parser.add_argument("--instances", type=Path, required=True)
    parser.add_argument("--disks", type=Path, required=True)
    parser.add_argument("--diagnostic-disk-gb", type=int, default=DIAGNOSTIC_DISK_GB)
    args = parser.parse_args()
    try:
        project_info = _read_json(args.project_info)
        regional_info = _read_json(args.regional_info)
        instances = _read_json(args.instances)
        disks = _read_json(args.disks)
        if not isinstance(project_info, Mapping) or not isinstance(regional_info, Mapping):
            raise ValueError("quota snapshots must be JSON objects")
        if not isinstance(instances, list) or not isinstance(disks, list):
            raise ValueError("instance and disk snapshots must be JSON arrays")
        quotas = parse_quota_snapshots(project_info, regional_info)
        allocations = controller_allocations(instances, disks, region=args.region)
        result = evaluate(args.mode, quotas, allocations, diagnostic_disk_gb=args.diagnostic_disk_gb)
    except ValueError as error:
        result = {"admissible": False, "errors": [f"invalid-quota-snapshot: {error}"]}
    print(json.dumps(result, sort_keys=True, separators=(",", ":")))
    return 0 if result["admissible"] else 1


if __name__ == "__main__":
    raise SystemExit(_cli())
