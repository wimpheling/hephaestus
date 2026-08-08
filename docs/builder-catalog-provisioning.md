# Platform OCI image catalog provisioning

The platform OCI image catalog is operator-owned metadata. The application and
daemon do not seed catalog rows at startup, and migrations intentionally do not
contain default image records.

## Provisioning command

Build the active operator binary from the bootstrap package and validate a
manifest without changing the database:

```sh
cargo run -p bootstrap-postgres --bin hephaestus-operator -- \
  provision-image-catalog path/to/image-catalog.json --dry-run
```

After the manifest has been reviewed and its image artifacts have been built,
scanned, approved, signed where required, and recorded with their real OCI
digests, apply it with:

```sh
HEPHAESTUS_DATABASE_URL=... \
cargo run -p bootstrap-postgres --bin hephaestus-operator -- \
  provision-image-catalog path/to/image-catalog.json
```

The command validates the complete manifest before opening a transaction. Each
record is then inserted or updated by its stable catalog key. An existing key
must keep the same stable UUID; changing it is rejected. The upsert updates
metadata and lifecycle state, but never turns a mutable tag into an accepted
image reference. A failed record or database error rolls back the complete
manifest.

## Manifest contract

The file is JSON with `schema_version: 1` and a non-empty `images` array. Each
image record must provide:

| Field | Contract |
| --- | --- |
| `id` | Stable UUID chosen by the reviewed platform record. |
| `key` | Lowercase catalog key, for example `rust-ubuntu`. |
| `display_name` | Human-readable name. |
| `image_reference` | OCI reference ending in a lowercase 64-character `sha256` digest. Tags are rejected. |
| `toolchains` | At least one `{ "name", "version" }` record with exact image contents. |
| `architectures` | One or more supported architectures. |
| `preparation_state` | `ready`, `preparing`, or `failed`. |
| `availability_state` | `available`, `unavailable`, or `retired`. |
| `provenance` | Object with a non-empty `source`; optional `signature` and `sbom` references are retained. |
| `platform_policy_version` | Non-empty reviewed image-policy identifier. |

Network, resource, dependency, secret, mount, and state policy are not image
metadata. They are independently declared and validated for the build or guest
execution contract that selects the image.

A shape-only template is shown below. The placeholder is intentionally invalid
and must be replaced with a real digest from the reviewed artifact record; this
repository does not claim or invent production image digests.

```json
{
  "schema_version": 1,
  "images": [
    {
      "id": "REPLACE_WITH_REVIEWED_STABLE_UUID",
      "key": "ubuntu-native",
      "display_name": "Ubuntu native",
      "image_reference": "registry.forge.example/platform/images/ubuntu-native@sha256:REPLACE_WITH_64_LOWERCASE_HEX_DIGEST",
      "toolchains": [
        { "name": "shell", "version": "REPLACE_WITH_IMAGE_VERSION" }
      ],
      "architectures": ["x86_64"],
      "preparation_state": "ready",
      "availability_state": "available",
      "provenance": {
        "source": "REPLACE_WITH_BUILD_OR_ATTESTATION_REFERENCE",
        "signature": null,
        "sbom": null
      },
      "platform_policy_version": "image/v1"
    }
  ]
}
```

The initial catalog release defines separate records
for `ubuntu-native`, `rust-ubuntu`, `typescript-node-ubuntu`, and
`python-ubuntu`. Their Ubuntu Dockerfiles, pinned upstream toolchains, scan,
SBOM/provenance attestation, and digest-manifest generator live in
[`platform/builders`](../platform/builders) and the platform-image catalog
writer.

The images are published by the reviewed platform release operation into the
forge-owned Zot namespace `platform/images/<image-key>`. The publisher
uploads the exact image plus digest-pinned provenance, SBOM, scan, and optional
signature referrers, then reads them back from Zot before producing
`platform-image-catalog.json`. A platform operator reviews that evidence and
the generated manifest before applying it with `provision-image-catalog`.
GitHub Actions and GHCR are not part of this trust path. This checkout
intentionally does not claim a production digest until the internal release
operation has completed.

This command only provisions platform-owned catalog rows. Project-owned OCI
image definitions and their produced immutable images follow the same image
lifecycle. Once approved and materialized, they may be selected by any
execution contract under normal project authorization.

Every approved image is materialized into the daemon-local image cache by its
digest-pinned reference. A successfully published but unmaterialized image is
not selectable; the caller receives that clear availability state before an
execution is created.
