#!/bin/sh
set -eu

source_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd -P)
output_root=${HEPHAESTUS_REFERENCE_UI_OUTPUT:-/workspace/output}
case "$output_root" in
  /*) ;;
  *) echo "HEPHAESTUS_REFERENCE_UI_OUTPUT must be absolute" >&2; exit 1 ;;
esac

mkdir -p "$output_root/dist" "$output_root/bin"

SOURCE_DIR="$source_dir" OUTPUT_ROOT="$output_root" python3 - <<'PY'
import hashlib
import json
import os
from pathlib import Path
import shutil

source = Path(os.environ["SOURCE_DIR"])
output = Path(os.environ["OUTPUT_ROOT"])
manifest_path = source / "vendor/release-ui-kit/v1.0.0/dist/manifest.json"
css_path = source / "vendor/release-ui-kit/v1.0.0/dist/heph-ui-kit-v1.0.0.css"
manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
expected = {
    "schema_version": 1,
    "package": "@hephaestus/release-ui-kit",
    "version": "1.0.0",
    "css_file": "heph-ui-kit-v1.0.0.css",
}
if any(manifest.get(key) != value for key, value in expected.items()):
    raise SystemExit("vendored release UI kit manifest identity is invalid")
actual_hash = hashlib.sha256(css_path.read_bytes()).hexdigest()
if manifest.get("css_sha256") != actual_hash:
    raise SystemExit("vendored release UI kit CSS hash does not match its manifest")
if manifest.get("token_sha256") != "aaeee8eff870a2418fd936893ab26a52ead2b995b726d6af65d0358c535652d6":
    raise SystemExit("vendored release UI kit token provenance is invalid")
if manifest.get("component_sha256") != "5ef3128841c58acd1b984e374de16983c2d41baf6d387ea3adcdd0ad72c97376":
    raise SystemExit("vendored release UI kit component provenance is invalid")
shutil.copyfile(source / "index.html", output / "dist/index.html")
shutil.copyfile(css_path, output / "dist/heph-ui-kit-v1.0.0.css")
shutil.copyfile(source / "bin/reference-ui-check", output / "bin/reference-ui-check")
os.chmod(output / "bin/reference-ui-check", 0o755)
PY
