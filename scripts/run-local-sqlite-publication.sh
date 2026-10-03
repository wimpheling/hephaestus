#!/usr/bin/env bash
# Real configured Python guest build and authenticated publication; no instance execution.
set -Eeuo pipefail
script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
repo_root="$(cd -- "${script_dir}/.." && pwd -P)"
[[ $# == 1 && "$1" = /* && ! -e "$1" ]] || {
    printf 'usage: %s NEW_ABSOLUTE_PROOF_DIRECTORY\n' "$0" >&2
    exit 2
}
mkdir -m 0700 -- "$1"
export HEPHAESTUS_SQLITE_PUBLICATION_OUTPUT="$1"
readonly revision=581b939d5ad5e5a81e77ad01ad8931487a8d2bcf
readonly image='localhost:55000/platform/images/python-ubuntu@sha256:24b78e523e8cf1732243dc8945d5c75145b5e21d23081a90ecc6b591d4bb820e'
export HEPHAESTUS_SQLITE_PYTHON_LAYOUT="${repo_root}/.local/hephaestus/platform-images/releases/${revision}/python-ubuntu/image"
python3 - "${repo_root}/.local/hephaestus/platform-images/installations/${revision}/catalog.json" \
    "${HEPHAESTUS_SQLITE_PYTHON_LAYOUT}" "${image}" "$1/image-input.json" <<'PY'
import hashlib, json, pathlib, sys
catalog, root, reference, output = sys.argv[1:]
layout = pathlib.Path(root)
entry, = [x for x in json.load(open(catalog))["images"] if x["key"] == "python-ubuntu"]
assert entry["image_reference"] == reference and entry["availability_state"] == "available"
assert {"name": "CPython", "version": "3.13.5"} in entry["toolchains"]
index = json.loads((layout / "index.json").read_bytes())
assert [x["digest"] for x in index["manifests"]] == [reference.split("@")[1]]
for blob in (layout / "blobs" / "sha256").iterdir():
    digest = hashlib.file_digest(blob.open("rb"), "sha256").hexdigest()
    assert digest == blob.name, str(blob)
pathlib.Path(output).write_text(json.dumps({"catalog_entry": entry, "layout": root,
    "index_sha256": hashlib.sha256((layout / "index.json").read_bytes()).hexdigest(),
    "provenance_boundary": "locally reviewed fixture; not production release attestation"}, indent=2))
PY
export HEPHAESTUS_LIBKRUN_UBUNTU_IMAGE="${image}"
export HEPHAESTUS_APP_SQLITE_PUBLICATION=1 HEPHAESTUS_APP_LIBKRUN_E2E=1
export HEPHAESTUS_LIBKRUN_DIAGNOSTICS_DIR="$1/diagnostics"
# The golden cleanup owns its NATS subjects, so use the launcher's dedicated server.
exec env -u HEPHAESTUS_NATS_TEST_URL -u HEPHAESTUS_APP_COOKING_BUILD_PROOF \
    -u HEPHAESTUS_APP_SESSION_CHAT_E2E "${script_dir}/run-libkrun-integration.sh"
