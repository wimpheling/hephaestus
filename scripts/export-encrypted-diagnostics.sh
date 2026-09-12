#!/usr/bin/env bash
# Validate an already downloaded private diagnostics archive, then encrypt it
# for an operator-supplied public X.509 recipient certificate.
set -Eeuo pipefail
umask 077

readonly MAX_ARCHIVE_BYTES=67108864
readonly MAX_CERT_BYTES=16384
readonly MAX_MEMBERS=1000
readonly MAX_MEMBER_BYTES=16777216
readonly MAX_EXPANDED_BYTES=134217728
readonly DEFAULT_RECIPIENT_DIR="${HEPH_DIAGNOSTICS_RECIPIENT_DIR:-${XDG_DATA_HOME:-${HOME:?}/.local/share}/hephaestus-gcp/diagnostics-recipient}"
readonly SCANNER_SCRIPT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)/check-browser-evidence.py"
cleanup_root=''

cleanup() {
  if [[ -n "$cleanup_root" ]]; then
    rm -rf -- "$cleanup_root"
    cleanup_root=''
  fi
}

trap cleanup EXIT

die() { printf 'export-encrypted-diagnostics: %s\n' "$*" >&2; exit 1; }

regular_file() {
  local path="$1" label="$2"
  [[ -f "$path" && ! -L "$path" ]] || die "$label must be a regular non-symlink file"
}

validate_recipient_cert() {
  local cert="$1" size
  regular_file "$cert" 'recipient certificate'
  size="$(stat -c '%s' -- "$cert")"
  [[ "$size" =~ ^[0-9]+$ && size -le MAX_CERT_BYTES ]] || die 'recipient certificate is too large'
  python3 - "$cert" <<'PY'
from pathlib import Path
import re
import sys

value = Path(sys.argv[1]).read_bytes()
try:
    text = value.decode("ascii")
except UnicodeDecodeError as error:
    raise SystemExit("recipient certificate is not ASCII PEM") from error
if "PRIVATE KEY" in text or "BEGIN RSA PRIVATE" in text or "BEGIN EC PRIVATE" in text:
    raise SystemExit("private-key material is not accepted")
pattern = r"\A-----BEGIN CERTIFICATE-----\r?\n[A-Za-z0-9+/=\r\n]+-----END CERTIFICATE-----\r?\n?\Z"
if re.fullmatch(pattern, text) is None:
    raise SystemExit("recipient input must be exactly one X.509 certificate PEM")
PY
  openssl x509 -in "$cert" -noout -checkend 0 >/dev/null 2>&1 ||
    die 'recipient certificate is invalid or expired'
}

validate_download_status() {
  local status_path="$1" archive="$2"
  regular_file "$status_path" 'diagnostics status'
  python3 - "$status_path" "$archive" <<'PY'
import hashlib
import json
from pathlib import Path
import sys

status_path, archive_path = sys.argv[1:]
try:
    status = json.loads(Path(status_path).read_text(encoding="utf-8"))
except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
    raise SystemExit("diagnostics download status is invalid") from error
if not isinstance(status, dict):
    raise SystemExit("diagnostics download status is not an object")
required = {
    "cleanup": "verified-absent",
    "upload": "verified-by-download",
    "download": "passed",
    "scan": "passed",
}
if any(status.get(field) != expected for field, expected in required.items()):
    raise SystemExit("diagnostics download prerequisites were not verified")
archive = Path(archive_path)
try:
    size = archive.stat().st_size
    if size > 64 * 1024 * 1024:
        raise SystemExit("diagnostics archive exceeds the 64 MiB bound")
    digestor = hashlib.sha256()
    with archive.open("rb") as input_file:
        for chunk in iter(lambda: input_file.read(1024 * 1024), b""):
            digestor.update(chunk)
    digest = digestor.hexdigest()
except OSError as error:
    raise SystemExit("diagnostics archive cannot be hashed") from error
if status.get("archiveBytes") != size or status.get("archiveSha256") != digest:
    raise SystemExit("diagnostics status does not match the archive")
PY
}

validate_archive() {
  local archive="$1" extract_root="$2" archive_bytes
  regular_file "$archive" 'diagnostics archive'
  archive_bytes="$(stat -c '%s' -- "$archive")"
  [[ "$archive_bytes" =~ ^[0-9]+$ && archive_bytes -le MAX_ARCHIVE_BYTES ]] ||
    die 'diagnostics archive exceeds the 64 MiB bound'
  python3 - "$archive" <<'PY'
from pathlib import Path, PurePosixPath
import tarfile
import sys

with tarfile.open(sys.argv[1], mode="r|gz") as archive:
    names = set()
    count = 0
    expanded = 0
    for member in archive:
        count += 1
        if count > 1000:
            raise SystemExit("diagnostics archive member budget exceeded")
        path = PurePosixPath(member.name)
        if (path.is_absolute() or ".." in path.parts or
                not member.name.startswith("cooking-diagnostics/") or
                not member.isfile() or member.size > 16 * 1024 * 1024):
            raise SystemExit("diagnostics archive member is unsafe or unbounded")
        if member.name in names:
            raise SystemExit("diagnostics archive contains duplicate members")
        names.add(member.name)
        expanded += member.size
    if expanded > 128 * 1024 * 1024:
        raise SystemExit("diagnostics archive expanded size exceeds its bound")
PY
  mkdir -m 700 -- "$extract_root"
  tar -xzf "$archive" -C "$extract_root" --no-same-owner --no-same-permissions ||
    die 'diagnostics archive extraction failed'
  python3 - "$extract_root/cooking-diagnostics" "$archive" <<'PY'
import hashlib
import json
from pathlib import Path, PurePosixPath
import tarfile
import sys

root = Path(sys.argv[1])
if not root.is_dir() or root.is_symlink():
    raise SystemExit("diagnostics root is missing")
manifest_path = root / "manifest.json"
if not manifest_path.is_file() or manifest_path.is_symlink():
    raise SystemExit("diagnostics manifest is missing or unsafe")
manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
if (
    not isinstance(manifest, dict)
    or manifest.get("schema") != 1
    or manifest.get("credentialScan") != "passed"
    or not isinstance(manifest.get("sources", []), list)
    or not manifest.get("sources")
):
    raise SystemExit("archive was not credential-scanned as passed")
expected = {"cooking-diagnostics/manifest.json"}
labels = set()
for record in manifest.get("sources", []):
    if not isinstance(record, dict) or not isinstance(record.get("path"), str):
        raise SystemExit("manifest source record is invalid")
    label = record.get("label")
    if not isinstance(label, str) or label in labels:
        raise SystemExit("manifest source label is duplicated or invalid")
    labels.add(label)
    relative = PurePosixPath(record["path"])
    if (
        relative.is_absolute()
        or ".." in relative.parts
        or not record["path"]
        or relative.as_posix() != record["path"]
        or relative.as_posix() == "manifest.json"
        or relative.as_posix() in {".", ".."}
    ):
        raise SystemExit("manifest source path is unsafe")
    member_name = "cooking-diagnostics/" + relative.as_posix()
    if member_name in expected:
        raise SystemExit("manifest source member is duplicated")
    expected.add(member_name)
    path = root / relative
    if not path.is_file() or path.is_symlink():
        raise SystemExit("manifest source is missing or symlinked")
    if type(record.get("bytes")) is not int or record["bytes"] != path.stat().st_size:
        raise SystemExit("manifest source size mismatch")
    if hashlib.sha256(path.read_bytes()).hexdigest() != record.get("sha256"):
        raise SystemExit("manifest source checksum mismatch")
for path in root.rglob("*"):
    if path.is_symlink():
        raise SystemExit("diagnostics archive contains a symlink")
    if path.is_file() and "cooking-diagnostics/" + path.relative_to(root).as_posix() not in expected:
        raise SystemExit("diagnostics archive contains an unmanifested member")
with tarfile.open(sys.argv[2], mode="r|gz") as archive:
    names = set()
    count = 0
    for member in archive:
        count += 1
        if count > 1000 or member.name in names:
            raise SystemExit("diagnostics archive member list is invalid")
        names.add(member.name)
    if names != expected or count != len(expected):
        raise SystemExit("diagnostics archive members do not match the manifest")
PY
  [[ -f "$SCANNER_SCRIPT" ]] || die 'credential scanner is unavailable'
  python3 -B "$SCANNER_SCRIPT" "$extract_root/cooking-diagnostics" >/dev/null 2>&1 ||
    die 'credential scan failed during encrypted export'
}

validate_local_archive() {
  local archive="$1" temp_root
  [[ -n "$archive" ]] || die 'archive is required'
  temp_root="$(mktemp -d "${TMPDIR:-/tmp}/heph-diagnostics-validate.XXXXXX")"
  cleanup_root="$temp_root"
  validate_archive "$archive" "$temp_root/extract"
  printf 'Diagnostics archive validation passed: %s\n' "$archive"
}

generate_recipient() {
  local directory="${1:-$DEFAULT_RECIPIENT_DIR}"
  [[ "$directory" = /* ]] || die 'recipient directory must be an absolute path'
  [[ ! -L "$directory" ]] || die 'recipient directory must not be a symlink'
  mkdir -p -- "$directory"
  chmod 700 -- "$directory"
  local key="$directory/recipient-key.pem" cert="$directory/recipient-cert.pem"
  [[ ! -e "$key" && ! -e "$cert" ]] || die 'recipient files already exist; choose a new directory'
  openssl genpkey -algorithm RSA -pkeyopt rsa_keygen_bits:3072 -out "$key" >/dev/null 2>&1 ||
    die 'could not generate recipient private key'
  chmod 600 -- "$key"
  if ! openssl req -new -x509 -sha256 -days 1 -key "$key" \
    -subj '/CN=hephaestus-diagnostics-recipient' -out "$cert" >/dev/null 2>&1; then
    rm -f -- "$key" "$cert"
    die 'could not generate recipient certificate'
  fi
  chmod 644 -- "$cert"
  printf 'Generated ephemeral recipient certificate: %s\n' "$cert"
  printf 'Private key is local-only: %s\n' "$key"
}

export_encrypted() {
  local archive="$1" cert="$2" output="$3" status_path="${4:-}"
  [[ -n "$archive" && -n "$cert" && -n "$output" ]] || die 'archive, certificate and output are required'
  [[ "$output" = /* ]] || die 'encrypted output must be an absolute path'
  [[ ! -e "$output" && ! -L "$output" ]] || die 'encrypted output already exists or is a symlink'
  validate_recipient_cert "$cert"
  [[ -n "$status_path" ]] || die 'verified diagnostics status is required for export'
  validate_download_status "$status_path" "$archive" || die 'diagnostics download prerequisites are not verified'
  local temp_root temp_output
  temp_root="$(mktemp -d "${TMPDIR:-/tmp}/heph-diagnostics-export.XXXXXX")"
  cleanup_root="$temp_root"
  validate_archive "$archive" "$temp_root/extract"
  mkdir -p -- "$(dirname -- "$output")"
  temp_output="$temp_root/encrypted.cms"
  openssl cms -encrypt -binary -aes-256-gcm -outform DER \
    -in "$archive" -out "$temp_output" "$cert" >/dev/null 2>&1 ||
    die 'CMS AES-256-GCM encryption failed'
  chmod 600 -- "$temp_output"
  mv -f -- "$temp_output" "$output"
  chmod 600 -- "$output"
  printf 'Encrypted diagnostics written: %s bytes=%s sha256=%s\n' \
    "$output" "$(stat -c '%s' -- "$output")" "$(sha256sum -- "$output" | awk '{print $1}')"
}

case "${1:-}" in
  generate-recipient)
    [[ "$#" -le 2 ]] || die 'usage: generate-recipient [directory]'
    generate_recipient "${2:-$DEFAULT_RECIPIENT_DIR}"
    ;;
  export)
    [[ "$#" -eq 5 ]] || die 'usage: export archive.tar.gz recipient-cert.pem output.cms status.json'
    export_encrypted "$2" "$3" "$4" "$5"
  ;;
  validate)
    [[ "$#" -eq 2 ]] || die 'usage: validate archive.tar.gz'
    validate_local_archive "$2"
    ;;
  *)
    die 'usage: generate-recipient [directory] | export archive.tar.gz recipient-cert.pem output.cms status.json | validate archive.tar.gz'
    ;;
esac
