#!/usr/bin/env python3
"""Strict compatibility validation for reviewed immutable runner images."""
from __future__ import annotations

import argparse
import json
import re
from pathlib import Path
from typing import Any


HEX256 = re.compile(r"[0-9a-f]{64}")
RECORD_KEYS = frozenset(
    {
        "manifest_sha256",
        "image_fingerprint",
        "baked_recipe_sha256",
        "baked_verifier_sha256",
        "baked_startup_sha256",
        "allowed_runtime_startup_sha256",
    }
)
ROOT_KEYS = frozenset({"schema", "kind", "records"})
IMAGE_KEYS = frozenset({"name", "status", "labels", "description"})


class ValidationError(ValueError):
    """Raised when compatibility or image metadata is not exact."""


def reject_duplicate_keys(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    value: dict[str, Any] = {}
    for key, item in pairs:
        if key in value:
            raise ValidationError(f"duplicate JSON field: {key}")
        value[key] = item
    return value


def load_json(path: Path) -> Any:
    try:
        return json.loads(path.read_text(encoding="utf-8"), object_pairs_hook=reject_duplicate_keys)
    except (OSError, UnicodeError, json.JSONDecodeError, ValidationError) as error:
        raise ValidationError(f"compatibility JSON is unreadable: {error}") from error


def require_exact_keys(value: dict[str, Any], expected: frozenset[str], label: str) -> None:
    unknown = set(value) - expected
    missing = expected - set(value)
    if unknown:
        raise ValidationError(f"{label} has unknown fields: {','.join(sorted(unknown))}")
    if missing:
        raise ValidationError(f"{label} is missing fields: {','.join(sorted(missing))}")


def sha256_field(value: Any, label: str) -> str:
    if not isinstance(value, str) or HEX256.fullmatch(value) is None:
        raise ValidationError(f"{label} must be a lowercase SHA-256")
    return value


def load_records(path: Path) -> dict[str, dict[str, str]]:
    document = load_json(path)
    if not isinstance(document, dict):
        raise ValidationError("compatibility document must be an object")
    require_exact_keys(document, ROOT_KEYS, "compatibility document")
    if type(document["schema"]) is not int or document["schema"] != 1 or document["kind"] != "hephaestus-gcp-runner-compatibility":
        raise ValidationError("compatibility document schema/kind mismatch")
    records = document["records"]
    if not isinstance(records, list) or not records:
        raise ValidationError("compatibility records must be a non-empty array")
    result: dict[str, dict[str, str]] = {}
    fingerprints: set[str] = set()
    for index, item in enumerate(records):
        label = f"compatibility record {index}"
        if not isinstance(item, dict):
            raise ValidationError(f"{label} must be an object")
        require_exact_keys(item, RECORD_KEYS, label)
        normalized = {key: sha256_field(item[key], f"{label}.{key}") for key in RECORD_KEYS}
        if normalized["manifest_sha256"] != normalized["image_fingerprint"]:
            raise ValidationError(f"{label} fingerprint does not equal manifest")
        manifest = normalized["manifest_sha256"]
        if manifest in result or normalized["image_fingerprint"] in fingerprints:
            raise ValidationError(f"duplicate compatibility record: {manifest}")
        result[manifest] = normalized
        fingerprints.add(normalized["image_fingerprint"])
    return result


def description_anchors(description: Any) -> dict[str, str]:
    if not isinstance(description, str):
        raise ValidationError("image description is invalid")
    result: dict[str, str] = {}
    for key in ("manifest_sha256", "recipe_sha256", "verifier_sha256", "startup_sha256"):
        # Do not consume the delimiter after a match: otherwise a repeated
        # key separated by one comma or space can hide from the next search.
        matches = re.findall(rf"(?:^|[ ,]){key}=([^ ,]*)", description)
        if len(matches) != 1 or HEX256.fullmatch(matches[0]) is None:
            raise ValidationError(f"image description anchor is missing or duplicated: {key}")
        result[key] = matches[0]
    return result


def record_for(records: dict[str, dict[str, str]], manifest: str, runtime: str) -> dict[str, str]:
    record = records.get(manifest)
    if record is None:
        raise ValidationError(f"unknown runner image compatibility record: {manifest}")
    if runtime != record["allowed_runtime_startup_sha256"]:
        raise ValidationError("runtime startup SHA is not allowed for this image")
    return record


def normal_record(manifest: str, recipe: str, verifier: str, startup: str) -> dict[str, str]:
    """Represent a newly-built image whose baked and runtime startup agree."""
    return {
        "manifest_sha256": manifest,
        "image_fingerprint": manifest,
        "baked_recipe_sha256": recipe,
        "baked_verifier_sha256": verifier,
        "baked_startup_sha256": startup,
        "allowed_runtime_startup_sha256": startup,
    }


def validate_controller(
    metadata: Any,
    image_name: str,
    records: dict[str, dict[str, str]],
    expected_recipe: str,
    expected_verifier: str,
    runtime_startup: str,
) -> dict[str, str]:
    expected_recipe = sha256_field(expected_recipe, "current recipe SHA")
    expected_verifier = sha256_field(expected_verifier, "current verifier SHA")
    runtime_startup = sha256_field(runtime_startup, "current runtime startup SHA")
    if not isinstance(metadata, dict):
        raise ValidationError("image metadata must be an object")
    require_exact_keys(metadata, IMAGE_KEYS, "image metadata")
    if metadata["name"] != image_name:
        raise ValidationError("custom runner image name does not match the requested image")
    labels = metadata["labels"]
    if not isinstance(labels, dict):
        raise ValidationError("image labels are invalid")
    if metadata["status"] != "READY":
        raise ValidationError("custom runner image is not READY")
    fingerprint = labels.get("fingerprint")
    if not isinstance(fingerprint, str) or re.fullmatch(r"[0-9a-f]{32}", fingerprint) is None:
        raise ValidationError("custom runner image lacks the immutable runner-image labels")
    if labels.get("purpose") != "hephaestus-runner-image":
        raise ValidationError("custom runner image lacks the immutable runner-image labels")
    anchors = description_anchors(metadata["description"])
    manifest = anchors["manifest_sha256"]
    if image_name != f"hephaestus-runner-{manifest[:32]}" or fingerprint != manifest[:32]:
        raise ValidationError("custom runner image name does not match its manifest fingerprint")
    repository_sha = labels.get("repository_sha")
    if not isinstance(repository_sha, str) or re.fullmatch(r"[0-9a-f]{40}", repository_sha) is None:
        raise ValidationError("custom runner image repository provenance is invalid")
    if anchors["startup_sha256"] == runtime_startup:
        # Preserve the existing path for newly-built images.  A compatibility
        # record is required only when the immutable baked startup diverges
        # from the current trusted runtime startup.
        record = normal_record(manifest, anchors["recipe_sha256"], anchors["verifier_sha256"], anchors["startup_sha256"])
    else:
        record = record_for(records, manifest, runtime_startup)
    if anchors["recipe_sha256"] != record["baked_recipe_sha256"] or anchors["recipe_sha256"] != expected_recipe:
        raise ValidationError("runner image recipe does not match the reviewed compatibility record")
    if anchors["verifier_sha256"] != record["baked_verifier_sha256"] or anchors["verifier_sha256"] != expected_verifier:
        raise ValidationError("runner image verifier does not match the reviewed compatibility record")
    if anchors["startup_sha256"] != record["baked_startup_sha256"]:
        raise ValidationError("runner image baked startup does not match the reviewed compatibility record")
    return {
        "manifest_sha256": manifest,
        "recipe_sha256": record["baked_recipe_sha256"],
        "verifier_sha256": record["baked_verifier_sha256"],
        "startup_sha256": record["baked_startup_sha256"],
        "runtime_startup_sha256": record["allowed_runtime_startup_sha256"],
    }


def validate_guest(
    manifest_path: Path,
    records: dict[str, dict[str, str]],
    expected_manifest: str,
    expected_recipe: str,
    expected_verifier: str,
    expected_baked_startup: str,
    expected_runtime_startup: str,
) -> dict[str, str]:
    expected_manifest = sha256_field(expected_manifest, "expected manifest SHA")
    expected_recipe = sha256_field(expected_recipe, "expected recipe SHA")
    expected_verifier = sha256_field(expected_verifier, "expected verifier SHA")
    expected_baked_startup = sha256_field(expected_baked_startup, "expected baked startup SHA")
    expected_runtime_startup = sha256_field(expected_runtime_startup, "expected runtime startup SHA")
    document = load_json(manifest_path)
    if not isinstance(document, dict):
        raise ValidationError("runner image manifest must be an object")
    for key in ("manifest_sha256", "recipe_sha256", "verifier_sha256", "startup_sha256"):
        sha256_field(document.get(key), f"runner image manifest.{key}")
    manifest = document["manifest_sha256"]
    if manifest != expected_manifest:
        raise ValidationError("runner image manifest does not match the external anchor")
    if document["recipe_sha256"] != expected_recipe or document["verifier_sha256"] != expected_verifier:
        raise ValidationError("runner image recipe/verifier does not match the external anchor")
    if document["startup_sha256"] != expected_baked_startup:
        raise ValidationError("runner image baked startup does not match the external anchor")
    if expected_baked_startup == expected_runtime_startup:
        record = normal_record(manifest, document["recipe_sha256"], document["verifier_sha256"], document["startup_sha256"])
    else:
        record = record_for(records, manifest, expected_runtime_startup)
    if record["baked_recipe_sha256"] != document["recipe_sha256"]:
        raise ValidationError("runner image recipe does not match the reviewed compatibility record")
    if record["baked_verifier_sha256"] != document["verifier_sha256"]:
        raise ValidationError("runner image verifier does not match the reviewed compatibility record")
    if record["baked_startup_sha256"] != document["startup_sha256"]:
        raise ValidationError("runner image baked startup does not match the reviewed compatibility record")
    return {
        "manifest_sha256": manifest,
        "recipe_sha256": record["baked_recipe_sha256"],
        "verifier_sha256": record["baked_verifier_sha256"],
        "startup_sha256": record["baked_startup_sha256"],
        "runtime_startup_sha256": record["allowed_runtime_startup_sha256"],
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("command", choices=("controller", "guest"))
    parser.add_argument("--records-file", type=Path, required=True)
    parser.add_argument("--metadata-json")
    parser.add_argument("--image-name")
    parser.add_argument("--manifest-file", type=Path)
    parser.add_argument("--expected-manifest")
    parser.add_argument("--expected-recipe", required=True)
    parser.add_argument("--expected-verifier", required=True)
    parser.add_argument("--expected-baked-startup")
    parser.add_argument("--runtime-startup", required=True)
    args = parser.parse_args()
    try:
        records = load_records(args.records_file)
        if args.command == "controller":
            if args.metadata_json is None or args.image_name is None:
                raise ValidationError("controller image metadata arguments are missing")
            metadata = json.loads(args.metadata_json, object_pairs_hook=reject_duplicate_keys)
            result = validate_controller(
                metadata,
                args.image_name,
                records,
                args.expected_recipe,
                args.expected_verifier,
                args.runtime_startup,
            )
        else:
            if args.manifest_file is None or args.expected_manifest is None or args.expected_baked_startup is None:
                raise ValidationError("guest manifest arguments are missing")
            result = validate_guest(
                args.manifest_file,
                records,
                args.expected_manifest,
                args.expected_recipe,
                args.expected_verifier,
                args.expected_baked_startup,
                args.runtime_startup,
            )
    except (OSError, UnicodeError, json.JSONDecodeError, ValidationError) as error:
        raise SystemExit(f"runner image compatibility rejected: {error}") from error
    print(",".join(f"runner-image-{key.replace('_', '-') }={value}" for key, value in result.items()))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
