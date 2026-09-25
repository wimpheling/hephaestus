#!/usr/bin/env python3
"""Capture and reconcile the Heph workspace package inventory.

The baseline records the package facts that must survive the workspace move.
Manifest and target source paths are retained for reporting, but are not part
of the identity comparison because those paths are expected to change.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from pathlib import Path
from typing import Any


REPOSITORY_ROOT = Path(__file__).resolve().parents[1]
DEFAULT_BASELINE = (
    REPOSITORY_ROOT
    / "tasks/in-progress/structural/code_architecture/workspace-package-inventory.before.json"
)
EXPECTED_FACADES = {
    "heph-secret",
    "heph-runtime",
    "heph-run",
    "heph-forge",
    "heph-build",
}
EXPECTED_BASELINE_PACKAGE_COUNT = 85
EXPECTED_FACADE_TEST_TARGETS = {
    ("heph-build", "contracts"),
    ("heph-forge", "api"),
    ("heph-run", "api"),
    ("heph-runtime", "api"),
    ("heph-secret", "api"),
}
ALLOWED_ARCHITECTURE_METADATA_DELTA = {
    "package": "git-http",
    "field": "allow_cross_context_dependencies",
    "baseline_value": ["identity-oidc"],
}
ALLOWED_INTEGRATION_TEST_RELOCATIONS = (
    {
        "from_package": "volume-postgres",
        "from_target": "postgres",
        "to_package": "hephaestus-app",
        "to_target": "volume_postgres_local",
    },
    {
        "from_package": "workspace-postgres",
        "from_target": "postgres_git",
        "to_package": "hephaestus-app",
        "to_target": "workspace_postgres_git",
    },
    {
        "from_package": "run-postgres",
        "from_target": "phase1b_libkrun",
        "to_package": "hephaestus-app",
        "to_target": "run_postgres_phase1b_libkrun",
    },
    {
        "from_package": "forge-postgres",
        "from_target": "smart_http",
        "to_package": "hephaestus-app",
        "to_target": "forge_postgres_smart_http",
    },
    {
        "from_package": "forge-postgres",
        "from_target": "postgres",
        "to_package": "hephaestus-app",
        "to_target": "forge_postgres_receive",
    },
    {
        "from_package": "gateway-postgres",
        "from_target": "service_ownership",
        "to_package": "hephaestus-app",
        "to_target": "gateway_service_ownership",
    },
)


def relative_path(path: str) -> str:
    """Return a repository-relative path when Cargo reports an absolute path."""

    candidate = Path(path)
    try:
        return candidate.relative_to(REPOSITORY_ROOT).as_posix()
    except ValueError:
        return candidate.as_posix()


def run_cargo_metadata() -> dict[str, Any]:
    """Capture the current workspace metadata from Cargo."""

    result = subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--no-deps"],
        cwd=REPOSITORY_ROOT,
        check=True,
        capture_output=True,
        text=True,
    )
    return json.loads(result.stdout)


def load_json(path: Path) -> dict[str, Any]:
    """Load a JSON object from ``path`` with a useful error message."""

    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except OSError as error:
        raise SystemExit(f"cannot read {path}: {error}") from error
    except json.JSONDecodeError as error:
        raise SystemExit(f"invalid JSON in {path}: {error}") from error
    if not isinstance(value, dict):
        raise SystemExit(f"expected a JSON object in {path}")
    return value


def normalize_target(target: dict[str, Any]) -> dict[str, Any]:
    """Keep target identity and retain its path for reporting."""

    normalized = dict(target)
    src_path = normalized.pop("src_path", None)
    normalized["src_path"] = relative_path(src_path) if src_path else None
    return normalized


def normalize_package(package: dict[str, Any]) -> dict[str, Any]:
    """Select stable package facts and the current manifest path."""

    return {
        "name": package["name"],
        "version": package["version"],
        "manifest_path": package["manifest_path"],
        "manifest_path_relative": relative_path(package["manifest_path"]),
        "features": package.get("features", {}),
        "targets": sorted(
            (normalize_target(target) for target in package.get("targets", [])),
            key=lambda target: (target["name"], tuple(target["kind"])),
        ),
        "metadata_hephaestus": package.get("metadata", {}).get("hephaestus", {}),
    }


def inventory_from_metadata(metadata: dict[str, Any]) -> dict[str, Any]:
    """Build the versioned inventory representation from Cargo metadata."""

    packages = sorted(
        (normalize_package(package) for package in metadata.get("packages", [])),
        key=lambda package: package["name"],
    )
    names = [package["name"] for package in packages]
    workspace_members = metadata.get("workspace_members", [])
    return {
        "schema_version": 1,
        "workspace_root": metadata.get("workspace_root"),
        "workspace_member_count": len(workspace_members),
        "workspace_member_names": names,
        "package_count": len(packages),
        "packages": packages,
    }


def write_baseline(path: Path, inventory: dict[str, Any]) -> None:
    """Write a deterministic baseline artifact."""

    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(
        json.dumps(inventory, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )


def target_identity(target: dict[str, Any]) -> dict[str, Any]:
    """Exclude source paths, which necessarily change during the move."""

    return {key: value for key, value in target.items() if key != "src_path"}


def package_identity(package: dict[str, Any]) -> dict[str, Any]:
    """Return the package facts that must remain unchanged."""

    metadata = package["metadata_hephaestus"]
    if package["name"] == ALLOWED_ARCHITECTURE_METADATA_DELTA["package"]:
        metadata = dict(metadata)
        metadata.pop(ALLOWED_ARCHITECTURE_METADATA_DELTA["field"], None)
    return {
        "name": package["name"],
        "version": package["version"],
        "features": package["features"],
        # Integration tests can move to the composition root to remove core
        # development dependencies. They are reconciled separately below.
        "targets": [
            target_identity(target)
            for target in package["targets"]
            if "test" not in target["kind"]
        ],
        "metadata_hephaestus": metadata,
    }


def architecture_metadata_delta_is_allowed(
    package_name: str,
    baseline: dict[str, Any],
    current: dict[str, Any],
) -> bool:
    """Permit only git-http's documented stale allowlist removal."""

    delta = ALLOWED_ARCHITECTURE_METADATA_DELTA
    if package_name != delta["package"]:
        return baseline == current
    if baseline.get(delta["field"]) != delta["baseline_value"]:
        return baseline == current
    expected_current = dict(baseline)
    expected_current.pop(delta["field"])
    return current == expected_current


def test_targets(inventory: dict[str, Any]) -> dict[tuple[str, str], dict[str, Any]]:
    """Index full test target identities by package and target name."""

    indexed: dict[tuple[str, str], dict[str, Any]] = {}
    for package in inventory.get("packages", []):
        for target in package["targets"]:
            if "test" in target["kind"]:
                indexed[(package["name"], target["name"])] = target_identity(target)
    return indexed


def reconcile(
    baseline: dict[str, Any],
    current: dict[str, Any],
    *,
    baseline_only: bool,
) -> tuple[list[str], list[str], list[str], list[str]]:
    """Return package errors, stable fact errors, and allowed relocations."""

    baseline_packages = {
        package["name"]: package for package in baseline.get("packages", [])
    }
    current_packages = {
        package["name"]: package for package in current.get("packages", [])
    }
    missing = sorted(set(baseline_packages) - set(current_packages))
    expected_names = set(baseline_packages)
    if not baseline_only:
        expected_names |= EXPECTED_FACADES
    unexpected = sorted(set(current_packages) - expected_names)
    changed: list[str] = []
    for name in sorted(set(baseline_packages) & set(current_packages)):
        if package_identity(baseline_packages[name]) != package_identity(
            current_packages[name]
        ) or not architecture_metadata_delta_is_allowed(
            name,
            baseline_packages[name]["metadata_hephaestus"],
            current_packages[name]["metadata_hephaestus"],
        ):
            changed.append(name)

    baseline_tests = test_targets(baseline)
    current_tests = test_targets(current)
    relocations: list[str] = []
    removed_tests = set(baseline_tests) - set(current_tests)
    added_tests = set(current_tests) - set(baseline_tests)
    for package, target in sorted(set(baseline_tests) & set(current_tests)):
        if baseline_tests[(package, target)] != current_tests[(package, target)]:
            changed.append(f"changed integration-test target {package}::{target}")
    for relocation in ALLOWED_INTEGRATION_TEST_RELOCATIONS:
        source = (relocation["from_package"], relocation["from_target"])
        destination = (relocation["to_package"], relocation["to_target"])
        if source in removed_tests and destination in added_tests:
            source_identity = baseline_tests[source]
            destination_identity = current_tests[destination]
            if not (
                source_identity.get("crate_types") == ["bin"]
                and source_identity.get("kind") == ["test"]
            ):
                changed.append(f"{source[0]}::{source[1]} integration-test identity")
            if not (
                destination_identity.get("crate_types") == ["bin"]
                and destination_identity.get("kind") == ["test"]
            ):
                changed.append(
                    f"{destination[0]}::{destination[1]} integration-test identity"
                )
            relocations.append(
                f"{source[0]}::{source[1]} -> {destination[0]}::{destination[1]}"
            )
            removed_tests.remove(source)
            added_tests.remove(destination)
    if removed_tests:
        changed.extend(
            f"removed integration-test target {package}::{target}"
            for package, target in sorted(removed_tests)
        )
    for target in EXPECTED_FACADE_TEST_TARGETS & added_tests:
        identity = current_tests[target]
        if identity.get("crate_types") == ["bin"] and identity.get("kind") == ["test"]:
            added_tests.remove(target)
    if added_tests:
        changed.extend(
            f"added integration-test target {package}::{target}"
            for package, target in sorted(added_tests)
        )

    expected_count = len(baseline_packages) + (0 if baseline_only else len(EXPECTED_FACADES))
    if current.get("package_count") != expected_count:
        changed.append(
            f"<package-count> expected {expected_count}, got {current.get('package_count')}"
        )
    if current.get("workspace_member_count") != expected_count:
        changed.append(
            "<workspace-member-count> "
            f"expected {expected_count}, got {current.get('workspace_member_count')}"
        )
    return missing, unexpected, changed, relocations


def report(
    baseline: dict[str, Any],
    current: dict[str, Any],
    *,
    baseline_path: Path,
    baseline_only: bool,
) -> int:
    """Print reconciliation results and return a process status."""

    baseline_names = [
        package["name"] for package in baseline.get("packages", [])
    ]
    if (
        len(baseline_names) != EXPECTED_BASELINE_PACKAGE_COUNT
        or len(set(baseline_names)) != EXPECTED_BASELINE_PACKAGE_COUNT
    ):
        print(
            "Baseline validation failed: expected exactly "
            f"{EXPECTED_BASELINE_PACKAGE_COUNT} unique packages, got "
            f"{len(baseline_names)}"
        )
        return 1

    missing, unexpected, changed, relocations = reconcile(
        baseline, current, baseline_only=baseline_only
    )
    baseline_names = set(baseline_names)
    current_names = {package["name"] for package in current.get("packages", [])}
    path_changes = sum(
        baseline_package["manifest_path_relative"]
        != next(
            package["manifest_path_relative"]
            for package in current.get("packages", [])
            if package["name"] == baseline_package["name"]
        )
        for baseline_package in baseline.get("packages", [])
        if baseline_package["name"] in current_names
    )
    facades = sorted(current_names - baseline_names)

    print(f"Baseline: {baseline_path}")
    print(f"Current packages: {current.get('package_count')}")
    print(f"Current workspace members: {current.get('workspace_member_count')}")
    print(f"Baseline packages compared: {len(baseline_names)}")
    print(f"Manifest path changes: {path_changes}")
    print(f"Additional packages: {', '.join(facades) if facades else 'none'}")
    if relocations:
        print("Integration-test target relocations:")
        for relocation in relocations:
            print(f"  {relocation}")

    if baseline_only:
        expected_facades = set()
    else:
        expected_facades = EXPECTED_FACADES
    if set(facades) != expected_facades:
        changed.append(
            "<facades> expected "
            f"{', '.join(sorted(expected_facades)) or 'none'}, got "
            f"{', '.join(facades) or 'none'}"
        )
    if missing or unexpected or changed:
        if missing:
            print(f"Missing packages: {', '.join(missing)}")
        if unexpected:
            print(f"Unexpected packages: {', '.join(unexpected)}")
        if changed:
            print(f"Changed package facts: {', '.join(changed)}")
        return 1

    print("Inventory reconciliation: clean")
    return 0


def parse_args() -> argparse.Namespace:
    """Parse command-line options."""

    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--baseline",
        type=Path,
        default=DEFAULT_BASELINE,
        help=f"baseline artifact (default: {DEFAULT_BASELINE})",
    )
    parser.add_argument(
        "--metadata",
        type=Path,
        help="raw cargo metadata JSON; otherwise invoke cargo metadata",
    )
    parser.add_argument(
        "--write-baseline",
        action="store_true",
        help="write the current metadata inventory to --baseline",
    )
    parser.add_argument(
        "--baseline-only",
        action="store_true",
        help="verify only the 85 baseline packages; permit no facade packages yet",
    )
    args = parser.parse_args()
    if args.write_baseline and args.baseline_only:
        parser.error("--write-baseline and --baseline-only are mutually exclusive")
    return args


def main() -> int:
    """Run baseline generation or reconciliation."""

    args = parse_args()
    metadata = load_json(args.metadata) if args.metadata else run_cargo_metadata()
    inventory = inventory_from_metadata(metadata)
    if args.write_baseline:
        write_baseline(args.baseline, inventory)
        print(
            f"Wrote {inventory['package_count']} package baseline to {args.baseline}"
        )
        return 0

    baseline = load_json(args.baseline)
    return report(
        baseline,
        inventory,
        baseline_path=args.baseline,
        baseline_only=args.baseline_only,
    )


if __name__ == "__main__":
    sys.exit(main())
