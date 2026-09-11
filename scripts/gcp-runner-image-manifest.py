#!/usr/bin/env python3
"""Write the deterministic content manifest for a baked runner image."""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--repository-sha", required=True)
    parser.add_argument("--browser-lock-sha256", required=True)
    parser.add_argument("--browser-version", required=True)
    parser.add_argument("--recipe-sha256", required=True)
    parser.add_argument("--startup-sha256", required=True)
    parser.add_argument("--bake-sha256", required=True)
    parser.add_argument("--verifier-sha256", required=True)
    parser.add_argument("--manifest-generator-sha256", required=True)
    args = parser.parse_args()
    pins = {
        "rust_version": os.environ["HEPH_IMAGE_RUST_VERSION"],
        "libkrun_tag": os.environ["HEPH_IMAGE_LIBKRUN_TAG"],
        "libkrun_revision": os.environ["HEPH_IMAGE_LIBKRUN_REVISION"],
        "libkrunfw_tag": os.environ["HEPH_IMAGE_LIBKRUNFW_TAG"],
        "passt_revision": os.environ["HEPH_IMAGE_PASST_REVISION"],
        "node_version": os.environ["HEPH_IMAGE_NODE_VERSION"],
        "node_sha256": os.environ["HEPH_IMAGE_NODE_SHA256"],
        "playwright": os.environ["HEPH_IMAGE_PLAYWRIGHT_VERSION"],
        "oras_version": os.environ["HEPH_IMAGE_ORAS_VERSION"],
        "oras_sha256": os.environ["HEPH_IMAGE_ORAS_SHA256"],
    }
    document = {
        "schema": 1,
        "kind": "hephaestus-gcp-runner",
        "repository_sha": args.repository_sha,
        "browser_lock_sha256": args.browser_lock_sha256,
        "browser_version": args.browser_version,
        "recipe_sha256": args.recipe_sha256,
        "startup_sha256": args.startup_sha256,
        "bake_sha256": args.bake_sha256,
        "verifier_sha256": args.verifier_sha256,
        "manifest_generator_sha256": args.manifest_generator_sha256,
        "pins": pins,
        "required_paths": [
            "/usr/bin/passt",
            "/usr/local/lib64/libkrun.so.1",
            "/usr/local/lib64/libkrunfw.so.5",
            "/usr/local/bin/node",
            "/usr/local/bin/oras",
            "/srv/hephaestus/playwright-browsers",
            "/usr/local/libexec/hephaestus/gcp-runner-image-provision.sh",
            "/usr/local/libexec/hephaestus/gcp-runner-image-verify.py",
            "/usr/local/libexec/hephaestus/gcp-kvm-startup.sh",
            "/usr/local/libexec/hephaestus/gcp-runner-image-bake.sh",
            "/usr/local/libexec/hephaestus/gcp-runner-image-manifest.py",
        ],
    }
    canonical = json.dumps(document, sort_keys=True, separators=(",", ":"), ensure_ascii=True).encode()
    document["manifest_sha256"] = hashlib.sha256(canonical).hexdigest()
    args.output.write_text(json.dumps(document, sort_keys=True, indent=2) + "\n", encoding="utf-8")
    args.output.chmod(0o644)
    print(f"HEPH_GCP_RUNNER_IMAGE manifest_sha256={document['manifest_sha256']}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
