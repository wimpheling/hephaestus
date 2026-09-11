#!/usr/bin/env python3
"""Fail-closed validation for an immutable Hephaestus runner image manifest."""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("manifest", type=Path)
    parser.add_argument("--root", type=Path, default=Path("/"))
    for name in ("rust-version", "libkrun-tag", "libkrun-revision", "libkrunfw-tag", "passt-revision", "node-version", "playwright-version", "oras-version", "oras-sha256"):
        parser.add_argument(f"--{name}", required=True)
    args = parser.parse_args()
    try:
        document = json.loads(args.manifest.read_text(encoding="utf-8"))
    except (OSError, ValueError) as error:
        raise SystemExit(f"runner image manifest unreadable: {error}") from error
    if document.get("schema") != 1 or document.get("kind") != "hephaestus-gcp-runner":
        raise SystemExit("runner image manifest schema/kind mismatch")
    fingerprint = document.get("manifest_sha256")
    if not isinstance(fingerprint, str) or len(fingerprint) != 64 or any(c not in "0123456789abcdef" for c in fingerprint):
        raise SystemExit("runner image manifest fingerprint is invalid")
    unsigned = dict(document)
    del unsigned["manifest_sha256"]
    canonical = json.dumps(unsigned, sort_keys=True, separators=(",", ":"), ensure_ascii=True).encode()
    if hashlib.sha256(canonical).hexdigest() != fingerprint:
        raise SystemExit("runner image manifest fingerprint mismatch")
    expected = {
        "rust_version": args.rust_version,
        "libkrun_tag": args.libkrun_tag,
        "libkrun_revision": args.libkrun_revision,
        "libkrunfw_tag": args.libkrunfw_tag,
        "passt_revision": args.passt_revision,
        "node_version": args.node_version,
        "playwright": args.playwright_version,
        "oras_version": args.oras_version,
        "oras_sha256": args.oras_sha256,
    }
    pins = document.get("pins")
    if not isinstance(pins, dict) or any(pins.get(key) != value for key, value in expected.items()):
        raise SystemExit("runner image dependency pins do not match startup contract")
    lock_sha = document.get("browser_lock_sha256")
    if not isinstance(lock_sha, str) or len(lock_sha) != 64 or any(c not in "0123456789abcdef" for c in lock_sha):
        raise SystemExit("runner image browser lock fingerprint is invalid")
    browser_version = document.get("browser_version")
    if not isinstance(browser_version, str) or not browser_version.strip() or any(c in browser_version for c in "\r\n\x00"):
        raise SystemExit("runner image browser version is invalid")
    for path in document.get("required_paths", []):
        target = args.root / path.lstrip("/")
        # Standard loader and Node installations use root-owned symlinks (for
        # example libkrun.so.1 and /usr/local/bin/node); reject only dangling
        # links while retaining the manifest's absolute-path contract.
        if not target.exists():
            raise SystemExit(f"runner image required path is missing: {path}")
        if target.is_symlink():
            try:
                resolved = target.resolve(strict=True)
            except OSError as error:
                raise SystemExit(f"runner image required symlink is invalid: {path}") from error
            if path == "/usr/local/bin/node":
                allowed = args.root / "opt/hephaestus" / f"node-{args.node_version}"
            elif path.startswith("/usr/local/lib64/"):
                allowed = args.root / "usr/local/lib64"
            else:
                raise SystemExit(f"runner image required path must not be a symlink: {path}")
            try:
                resolved.relative_to(allowed)
            except ValueError as error:
                raise SystemExit(f"runner image required symlink escapes its install root: {path}") from error
    root_hashes = {
        "recipe_sha256": args.root / "usr/local/libexec/hephaestus/gcp-runner-image-provision.sh",
        "verifier_sha256": args.root / "usr/local/libexec/hephaestus/gcp-runner-image-verify.py",
        "startup_sha256": args.root / "usr/local/libexec/hephaestus/gcp-kvm-startup.sh",
        "bake_sha256": args.root / "usr/local/libexec/hephaestus/gcp-runner-image-bake.sh",
        "manifest_generator_sha256": args.root / "usr/local/libexec/hephaestus/gcp-runner-image-manifest.py",
    }
    for field, target in root_hashes.items():
        expected_hash = document.get(field)
        if not isinstance(expected_hash, str) or len(expected_hash) != 64:
            raise SystemExit(f"runner image {field} is invalid")
        if target.exists() and hashlib.sha256(target.read_bytes()).hexdigest() != expected_hash:
            raise SystemExit(f"runner image {field} does not match installed recipe")
    if not (args.root / "etc/hephaestus/runner-image-required").is_file():
        raise SystemExit("runner image selection marker is missing")
    print(f"HEPH_GCP_RUNNER_IMAGE verified manifest_sha256={fingerprint}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
