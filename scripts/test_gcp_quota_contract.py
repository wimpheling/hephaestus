"""Provider-shaped tests for disposable GCP quota/admission arithmetic."""

from __future__ import annotations

import copy
import importlib.util
import sys
import unittest
from pathlib import Path

SPEC = importlib.util.spec_from_file_location(
    "gcp_quota_contract_under_test", Path(__file__).with_name("gcp_quota_contract.py")
)
assert SPEC is not None and SPEC.loader is not None
MODULE = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = MODULE
SPEC.loader.exec_module(MODULE)

DIAGNOSTIC_RUNNER = MODULE.DIAGNOSTIC_RUNNER
FULL_RUNNER = MODULE.FULL_RUNNER
FULL_RUNNER_TARGET = MODULE.FULL_RUNNER_TARGET
Resources = MODULE.Resources
controller_allocations = MODULE.controller_allocations
evaluate = MODULE.evaluate
parse_quota_snapshots = MODULE.parse_quota_snapshots
quota_shortfalls = MODULE.quota_shortfalls
required_additional_resources = MODULE.required_additional_resources


REGION = "europe-west1"
PROJECT = "hephaestus-508000"


def quota_snapshot(
    *,
    global_cpus: int | float | str,
    regional_n2: int | float | str,
    ssd: int | float | str,
    instances: int | float | str,
    addresses: int | float | str,
    regional_cpus: int | float | str = 0,
) -> tuple[dict[str, object], dict[str, object]]:
    def metric(limit: object, usage: object = 0) -> dict[str, object]:
        return {"limit": limit, "usage": usage}

    return (
        {"name": PROJECT, "quotas": [{"metric": "CPUS_ALL_REGIONS", **metric(global_cpus)}]},
        {
            "name": REGION,
            "quotas": [
                {"metric": "CPUS", **metric(regional_cpus)},
                {"metric": "N2_CPUS", **metric(regional_n2)},
                {"metric": "SSD_TOTAL_GB", **metric(ssd)},
                {"metric": "INSTANCES", **metric(instances)},
                {"metric": "IN_USE_ADDRESSES", **metric(addresses)},
            ],
        },
    )


def provider_instance(
    run_id: str,
    *,
    attempt: str = "1",
    status: str = "RUNNING",
    machine_type: str = "n2-standard-8",
    disk_name: str | None = None,
    zone: str = "europe-west1-d",
    purpose: str = "hephaestus-kvm-smoke",
    with_address: bool = True,
) -> dict[str, object]:
    name = f"heph-kvm-smoke-{run_id}-{attempt}"
    disk_name = disk_name or name
    access_configs = [{"type": "ONE_TO_ONE_NAT", "natIP": "203.0.113.10"}] if with_address else []
    return {
        "name": name,
        "status": status,
        "machineType": f"https://www.googleapis.com/compute/v1/projects/{PROJECT}/zones/{zone}/machineTypes/{machine_type}",
        "zone": f"https://www.googleapis.com/compute/v1/projects/{PROJECT}/zones/{zone}",
        "labels": {
            "purpose": purpose,
            "run_id": run_id,
            "run_attempt": attempt,
            "sha": "a" * 40,
        },
        "disks": [
            {
                "boot": True,
                "source": f"https://www.googleapis.com/compute/v1/projects/{PROJECT}/zones/{zone}/disks/{disk_name}",
            }
        ],
        "networkInterfaces": [{"accessConfigs": access_configs}],
    }


def provider_disk(name: str, *, size_gb: int = 150, zone: str = "europe-west1-d") -> dict[str, object]:
    return {
        "name": name,
        "status": "READY",
        "sizeGb": str(size_gb),
        "type": f"https://www.googleapis.com/compute/v1/projects/{PROJECT}/zones/{zone}/diskTypes/pd-balanced",
        "users": [f"https://www.googleapis.com/compute/v1/projects/{PROJECT}/zones/{zone}/instances/{name}"],
    }


class GcpQuotaContractTests(unittest.TestCase):
    def test_provider_snapshot_extracts_only_controller_owned_full_vm(self) -> None:
        controller = provider_instance("34599999991")
        unrelated = provider_instance("34599999992", purpose="other-product")
        disks = [provider_disk(controller["name"]), provider_disk(unrelated["name"])]
        allocations = controller_allocations([controller, unrelated], disks, region=REGION)
        self.assertEqual(len(allocations), 1)
        self.assertEqual(allocations[0]["mode"], "full")
        self.assertEqual(allocations[0]["ssd_gb"], 150)

    def test_one_existing_full_runner_requires_one_additional_runner(self) -> None:
        controller = provider_instance("34599999991")
        allocations = controller_allocations([controller], [provider_disk(controller["name"])], region=REGION)
        self.assertEqual(required_additional_resources("full", allocations), FULL_RUNNER)

        global_info, regional_info = quota_snapshot(global_cpus=32, regional_n2=200, ssd=500, instances=24, addresses=8)
        global_info["quotas"][0]["usage"] = 8
        for quota in regional_info["quotas"]:
            quota["usage"] = {"N2_CPUS": 8, "SSD_TOTAL_GB": 150, "INSTANCES": 1, "IN_USE_ADDRESSES": 1}.get(quota["metric"], 0)
        quotas = parse_quota_snapshots(global_info, regional_info)
        result = evaluate("full", quotas, allocations)
        self.assertTrue(result["admissible"])
        self.assertEqual(result["required_additional"]["global_cpus"], 8)

    def test_two_admitted_full_runners_reject_third_even_with_zero_need(self) -> None:
        first = provider_instance("34599999991")
        second = provider_instance("34599999992")
        disks = [provider_disk(first["name"]), provider_disk(second["name"])]
        allocations = controller_allocations([first, second], disks, region=REGION)
        global_info, regional_info = quota_snapshot(global_cpus=32, regional_n2=200, ssd=500, instances=24, addresses=8)
        global_info["quotas"][0]["usage"] = 16
        for quota in regional_info["quotas"]:
            quota["usage"] = {"N2_CPUS": 16, "SSD_TOTAL_GB": 300, "INSTANCES": 2, "IN_USE_ADDRESSES": 2}.get(quota["metric"], 0)
        result = evaluate("full", parse_quota_snapshots(global_info, regional_info), allocations)
        self.assertFalse(result["admissible"])
        self.assertIn("full-runner-admission-limit-reached", result["errors"])
        self.assertEqual(result["required_additional"]["instances"], 0)

    def test_diagnostic_keeps_e2_small_and_20gb_contract(self) -> None:
        diagnostic = provider_instance("34599999993", machine_type="e2-small")
        disk = provider_disk(diagnostic["name"], size_gb=20)
        allocations = controller_allocations([diagnostic], [disk], region=REGION)
        self.assertEqual(allocations[0]["mode"], "diagnostic")
        self.assertEqual(required_additional_resources("diagnostic", allocations), DIAGNOSTIC_RUNNER)
        global_info, regional_info = quota_snapshot(global_cpus=2, regional_n2=0, regional_cpus=2, ssd=20, instances=1, addresses=1)
        self.assertEqual(quota_shortfalls("diagnostic", parse_quota_snapshots(global_info, regional_info), allocations), {})

    def test_diagnostic_custom_image_keeps_150gb_boot_contract(self) -> None:
        diagnostic = provider_instance("34599999997", machine_type="e2-small")
        disk = provider_disk(diagnostic["name"], size_gb=150)
        allocations = controller_allocations([diagnostic], [disk], region=REGION)
        self.assertEqual(allocations[0]["mode"], "diagnostic")
        self.assertEqual(required_additional_resources("diagnostic", diagnostic_disk_gb=150).ssd_gb, 150)

    def test_wrong_region_and_unrelated_machine_do_not_reduce_full_need(self) -> None:
        wrong_region = provider_instance("34599999994", zone="europe-west4-d")
        unrelated_machine = provider_instance(
            "34599999995", machine_type="n2-standard-16", disk_name="other-disk", purpose="other-product"
        )
        allocations = controller_allocations(
            [wrong_region, unrelated_machine],
            [provider_disk(wrong_region["name"], zone="europe-west4-d"), provider_disk("other-disk", size_gb=300)],
            region=REGION,
        )
        self.assertEqual(allocations, [])
        self.assertEqual(required_additional_resources("full", allocations), FULL_RUNNER_TARGET)

    def test_malformed_controller_identity_fails_closed(self) -> None:
        instance = provider_instance("34599999996", with_address=False)
        with self.assertRaisesRegex(ValueError, "no ephemeral address"):
            controller_allocations([instance], [provider_disk(instance["name"])], region=REGION)

    def test_nan_infinity_and_negative_quota_values_fail_closed(self) -> None:
        for bad in ("NaN", "Infinity", -1):
            global_info, regional_info = quota_snapshot(global_cpus=32, regional_n2=200, ssd=500, instances=24, addresses=8)
            global_info["quotas"][0]["limit"] = bad
            with self.assertRaises(ValueError):
                parse_quota_snapshots(global_info, regional_info)

    def test_duplicate_quota_metric_fails_closed(self) -> None:
        global_info, regional_info = quota_snapshot(global_cpus=32, regional_n2=200, ssd=500, instances=24, addresses=8)
        global_info["quotas"].append(copy.deepcopy(global_info["quotas"][0]))
        with self.assertRaisesRegex(ValueError, "repeats metric"):
            parse_quota_snapshots(global_info, regional_info)

    def test_full_contract_exact_limits(self) -> None:
        global_info, regional_info = quota_snapshot(global_cpus=16, regional_n2=16, ssd=300, instances=2, addresses=2)
        self.assertEqual(quota_shortfalls("full", parse_quota_snapshots(global_info, regional_info)), {})


if __name__ == "__main__":
    unittest.main()
