#!/usr/bin/env bash
set -euo pipefail

usage() {
    printf '%s\n' \
        'usage: write-platform-builder-catalog.sh --output path --registry registry/repository \' \
        '  --ubuntu-digest sha256:... --rust-digest sha256:... \' \
        '  --typescript-digest sha256:... --python-digest sha256:...' >&2
    exit 64
}

output=''
registry=''
ubuntu_digest=''
rust_digest=''
typescript_digest=''
python_digest=''

while [[ $# -gt 0 ]]; do
    case "$1" in
        --output) output=${2:-}; shift 2 ;;
        --registry) registry=${2:-}; shift 2 ;;
        --ubuntu-digest) ubuntu_digest=${2:-}; shift 2 ;;
        --rust-digest) rust_digest=${2:-}; shift 2 ;;
        --typescript-digest) typescript_digest=${2:-}; shift 2 ;;
        --python-digest) python_digest=${2:-}; shift 2 ;;
        *) usage ;;
    esac
done

[[ -n "$output" && -n "$registry" && -n "$ubuntu_digest" && -n "$rust_digest" \
    && -n "$typescript_digest" && -n "$python_digest" ]] || usage

[[ "$registry" =~ ^[a-z0-9][a-z0-9.-]*(:[0-9]{1,5})?/platform/builders$ ]] || {
    printf 'registry must be a forge authority followed by /platform/builders\n' >&2
    exit 65
}

for digest in "$ubuntu_digest" "$rust_digest" "$typescript_digest" "$python_digest"; do
    [[ "$digest" =~ ^sha256:[0-9a-f]{64}$ ]] || {
        printf 'invalid OCI digest: %s\n' "$digest" >&2
        exit 65
    }
done

provenance_source=${HEPHAESTUS_PLATFORM_PROVENANCE_SOURCE:-}
ubuntu_signature=${HEPHAESTUS_UBUNTU_SIGNATURE_REFERENCE:-}
ubuntu_sbom=${HEPHAESTUS_UBUNTU_SBOM_REFERENCE:-}
rust_signature=${HEPHAESTUS_RUST_SIGNATURE_REFERENCE:-}
rust_sbom=${HEPHAESTUS_RUST_SBOM_REFERENCE:-}
typescript_signature=${HEPHAESTUS_TYPESCRIPT_SIGNATURE_REFERENCE:-}
typescript_sbom=${HEPHAESTUS_TYPESCRIPT_SBOM_REFERENCE:-}
python_signature=${HEPHAESTUS_PYTHON_SIGNATURE_REFERENCE:-}
python_sbom=${HEPHAESTUS_PYTHON_SBOM_REFERENCE:-}
absolute_uri_pattern='^[a-zA-Z][a-zA-Z0-9+.-]*://[^[:space:]"\\]+$'
evidence_pattern='^[^[:space:]"\\]+@sha256:[0-9a-f]{64}$'

[[ "$provenance_source" =~ $absolute_uri_pattern ]] || {
    printf 'HEPHAESTUS_PLATFORM_PROVENANCE_SOURCE must be a safe absolute URI\n' >&2
    exit 65
}

for evidence in "$ubuntu_sbom" "$rust_sbom" "$typescript_sbom" "$python_sbom"; do
    [[ "$evidence" =~ $evidence_pattern ]] || {
        printf 'supply-chain evidence must be a digest-pinned OCI reference: %s\n' \
            "$evidence" >&2
        exit 65
    }
done

for evidence in "$ubuntu_signature" "$rust_signature" "$typescript_signature" "$python_signature"; do
    [[ -z "$evidence" || "$evidence" =~ $evidence_pattern ]] || {
        printf 'optional signature evidence must be a digest-pinned OCI reference: %s\n' \
            "$evidence" >&2
        exit 65
    }
done

json_optional_reference() {
    if [[ -n "$1" ]]; then
        printf '"%s"' "$1"
    else
        printf 'null'
    fi
}

ubuntu_signature_json=$(json_optional_reference "$ubuntu_signature")
rust_signature_json=$(json_optional_reference "$rust_signature")
typescript_signature_json=$(json_optional_reference "$typescript_signature")
python_signature_json=$(json_optional_reference "$python_signature")

mkdir -p "$(dirname "$output")"
cat >"$output" <<EOF
{
  "schema_version": 1,
  "images": [
    {
      "id": "056a8bce-60f7-4b79-bf05-4c9d04c14f39",
      "key": "ubuntu-native",
      "display_name": "Ubuntu native builder",
      "image_reference": "${registry}/ubuntu-native@${ubuntu_digest}",
      "toolchains": [{"name":"Ubuntu","version":"24.04"},{"name":"Bash","version":"5.2.21"},{"name":"Git","version":"2.43.0"}],
      "architectures": ["x86_64"],
      "preparation_state": "ready",
      "availability_state": "available",
      "network_ceiling": "disabled",
      "max_vcpus": 4,
      "max_memory_mib": 2048,
      "dependency_policy": "vendored_offline",
      "provenance": {"source":"${provenance_source}","signature":${ubuntu_signature_json},"sbom":"${ubuntu_sbom}"},
      "platform_policy_version": "builder/v1"
    },
    {
      "id": "a90d44bf-9571-4a6f-a7d0-461aa2968a79",
      "key": "rust-ubuntu",
      "display_name": "Rust on Ubuntu",
      "image_reference": "${registry}/rust-ubuntu@${rust_digest}",
      "toolchains": [{"name":"Ubuntu","version":"24.04"},{"name":"Rust","version":"1.88.0"},{"name":"Cargo","version":"1.88.0"}],
      "architectures": ["x86_64"],
      "preparation_state": "ready",
      "availability_state": "available",
      "network_ceiling": "disabled",
      "max_vcpus": 4,
      "max_memory_mib": 4096,
      "dependency_policy": "vendored_offline",
      "provenance": {"source":"${provenance_source}","signature":${rust_signature_json},"sbom":"${rust_sbom}"},
      "platform_policy_version": "builder/v1"
    },
    {
      "id": "370e7d5f-22f3-4a9a-a0f9-a65dd230e6af",
      "key": "typescript-node-ubuntu",
      "display_name": "TypeScript and Node on Ubuntu",
      "image_reference": "${registry}/typescript-node-ubuntu@${typescript_digest}",
      "toolchains": [{"name":"Ubuntu","version":"24.04"},{"name":"Node","version":"24.19.0"},{"name":"pnpm","version":"11.20.0"},{"name":"TypeScript","version":"5.9.3"}],
      "architectures": ["x86_64"],
      "preparation_state": "ready",
      "availability_state": "available",
      "network_ceiling": "disabled",
      "max_vcpus": 4,
      "max_memory_mib": 4096,
      "dependency_policy": "vendored_offline",
      "provenance": {"source":"${provenance_source}","signature":${typescript_signature_json},"sbom":"${typescript_sbom}"},
      "platform_policy_version": "builder/v1"
    },
    {
      "id": "5106cc73-041d-4e93-84d3-24cfc6b78565",
      "key": "python-ubuntu",
      "display_name": "Python on Ubuntu",
      "image_reference": "${registry}/python-ubuntu@${python_digest}",
      "toolchains": [{"name":"Ubuntu","version":"24.04"},{"name":"CPython","version":"3.13.5"},{"name":"pip","version":"25.1.1"}],
      "architectures": ["x86_64"],
      "preparation_state": "ready",
      "availability_state": "available",
      "network_ceiling": "disabled",
      "max_vcpus": 4,
      "max_memory_mib": 4096,
      "dependency_policy": "vendored_offline",
      "provenance": {"source":"${provenance_source}","signature":${python_signature_json},"sbom":"${python_sbom}"},
      "platform_policy_version": "builder/v1"
    }
  ]
}
EOF
