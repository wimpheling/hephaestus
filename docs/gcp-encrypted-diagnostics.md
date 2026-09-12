# Encrypted GCP diagnostics export

The public repository keeps the retained diagnostics bundle private. The
`diagnostics-triage` workflow mode can optionally produce a short lived CMS
ciphertext artifact for local inspection. It uses the existing GitHub OIDC
identity and bucket access; it does not add IAM grants, service-account keys,
or a public plaintext artifact.

Generate a one-day local recipient key and certificate before dispatching the
workflow:

```sh
umask 077
recipient_dir="${XDG_DATA_HOME:-${HOME:-/home/a/.local/share}}/hephaestus-gcp/diagnostics-recipient"
scripts/export-encrypted-diagnostics.sh generate-recipient
cat "$recipient_dir/recipient-cert.pem"
```

Paste only the certificate into the optional `diagnostics_recipient_cert`
workflow input, together with the exact source run ID, attempt, commit SHA,
and selected `europe-west1` zone. The workflow accepts one bounded PEM X.509
certificate and rejects private-key PEM input. The existing no-VM download,
source checksum, archive-bound, credential-scan, and deletion checks still run
before encryption. The helper repeats the archive and credential validation
locally on the runner immediately before CMS encryption and also requires the
status manifest to prove the earlier no-VM download, deletion, archive hash,
and credential-scan checks. A triage acceptance failure therefore cannot by
itself bypass those prerequisites.

The workflow retains the safe status manifest as usual and uploads only
`gcp-diagnostics.tar.gz.cms` for one day when encryption succeeds. The local
private key stays under the mode-0700 recipient directory with mode 0600 and
never enters GitHub inputs, logs, artifacts, or GCP. The certificate is valid
for one day; the private key does not expire and remains local until you
remove it. The envelope uses OpenSSL CMS with authenticated AES-256-GCM;
changing the ciphertext causes local decryption to fail.

After downloading the ciphertext artifact, decrypt it only on the machine
holding the key, then revalidate the plaintext archive before inspection:

```sh
set -Eeuo pipefail
umask 077
destination="$PWD/gcp-diagnostics.tar.gz"
recipient_dir="${XDG_DATA_HOME:-${HOME:-/home/a/.local/share}}/hephaestus-gcp/diagnostics-recipient"
temporary="$(mktemp -d "${TMPDIR:-/tmp}/heph-diagnostics-decrypt.XXXXXX")"
trap 'rm -rf -- "$temporary"' EXIT
openssl cms -decrypt -inform DER \
  -in gcp-diagnostics.tar.gz.cms \
  -recip "$recipient_dir/recipient-cert.pem" \
  -inkey "$recipient_dir/recipient-key.pem" \
  -out "$temporary/decrypted.tar.gz"
scripts/export-encrypted-diagnostics.sh validate "$temporary/decrypted.tar.gz"
mv -- "$temporary/decrypted.tar.gz" "$destination"
```

Validation rejects unsafe archive members, manifest checksum mismatches,
missing credential-scan approval, symlinks, and browser evidence that fails
the repository scanner. The original collector excludes raw reports, payloads,
and credentials from its fixed bundle contract; the export only encrypts that
validated fixed bundle. Inspect it with bounded listing tools before extracting
it.
