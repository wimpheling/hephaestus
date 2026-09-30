# Purpose

`registry-release` is the trusted operator binary for publishing and approving
one reviewed platform OCI image. It reads an OCI layout and its SBOM,
provenance, scan, and optional signature evidence, then records a Forge-owned
publication intent and drives it through registry verification to approval.

# Responsibilities

The command parses a platform image key and policy version, derives the
expected manifest descriptor from the layout, claims the platform namespace,
and creates or resumes the matching publication intent in PostgreSQL. It
issues a short-lived pull/push token for that exact namespace, invokes the
controlled publisher, records read-back evidence, and emits a JSON result with
publication ID, immutable reference, digest, and evidence references.

# When

Run `hephaestus-registry-release publish-platform-image` after a reviewed
platform image and evidence files are available. Supply `--key`, `--layout`,
`--sbom`, `--provenance`, and `--scan`; `--signature` supplies optional
signature evidence under the current command policy. The command is retryable
because it resumes the existing intent.
