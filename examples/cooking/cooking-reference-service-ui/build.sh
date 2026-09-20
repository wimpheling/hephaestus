#!/bin/sh
set -eu

source_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd -P)
output_root=${HEPHAESTUS_REFERENCE_SERVICE_UI_OUTPUT:-/workspace/output}
case "$output_root" in
  /*) ;;
  *) echo "HEPHAESTUS_REFERENCE_SERVICE_UI_OUTPUT must be absolute" >&2; exit 1 ;;
esac

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
html_path = source / "index.html"
manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
expected = {
    "schema_version": 1,
    "package": "@hephaestus/release-ui-kit",
    "version": "1.0.0",
    "css_file": "heph-ui-kit-v1.0.0.css",
}
if any(manifest.get(key) != value for key, value in expected.items()):
    raise SystemExit("managed reference UI kit manifest identity is invalid")
if hashlib.sha256(css_path.read_bytes()).hexdigest() != manifest.get("css_sha256"):
    raise SystemExit("managed reference UI kit CSS hash does not match its manifest")
html = html_path.read_text(encoding="utf-8")
if "<title>Managed release reference</title>" not in html:
    raise SystemExit("managed reference UI HTML title is missing")
if 'href="heph-ui-kit-v1.0.0.css"' not in html:
    raise SystemExit("managed reference UI stylesheet link is not relative to the service route")

bin_dir = output / "bin"
bin_dir.mkdir(parents=True, exist_ok=True)
shutil.copyfile(source / "reference-ui-service.py", bin_dir / "reference-ui-service")
shutil.copyfile(html_path, bin_dir / "index.html")
shutil.copyfile(css_path, bin_dir / "heph-ui-kit-v1.0.0.css")
(bin_dir / "reference-ui-service").chmod(0o755)
PY
