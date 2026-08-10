#!/usr/bin/env bash
# Manually smoke a reviewed platform-builder release through ephemeral Zot.
# This is an operator harness, intentionally not a commit-time test.
set -Eeuo pipefail

readonly repository_root="$(git rev-parse --show-toplevel)"
readonly container_engine="${CONTAINER_ENGINE:-podman}"
readonly postgres_image="${HEPHAESTUS_POSTGRES_TEST_IMAGE:-docker.io/library/postgres:17-alpine}"
readonly zot_image='ghcr.io/project-zot/zot@sha256:6f7bf2b8e43437c7c3a121bc80214845c85f27321e66f2ff4be6bf4220775fd7'
readonly layout_root="${HEPHAESTUS_PLATFORM_SMOKE_LAYOUT_ROOT:-/tmp/hephaestus-platform-smoke/layouts}"
readonly fixture_root="$(mktemp -d)"
readonly postgres_container="hephaestus-platform-smoke-postgres-$$"
readonly zot_container="hephaestus-platform-smoke-zot-$$"

die() { printf '%s\n' "$*" >&2; exit 65; }

require_executable() {
    local variable=$1
    local fallback=$2
    local value=${!variable:-}
    [[ -n $value ]] || value=$(command -v "$fallback" || true)
    [[ $value = /* && -x $value ]] || die "required executable is unavailable: $fallback (set $variable)"
    realpath -e -- "$value"
}

reserve_port() {
    python3 -c 'import socket; sock = socket.socket(); sock.bind(("127.0.0.1", 0)); print(sock.getsockname()[1]); sock.close()'
}

published_port() {
    "$container_engine" port "$1" "$2/tcp" | awk -F: '{print $NF}'
}

wait_for_registry() {
    for _attempt in {1..300}; do
        local status
        status=$(curl --silent --output /dev/null --write-out '%{http_code}' "$registry_origin/v2/" || true)
        [[ $status == 401 ]] && return
        sleep 0.1
    done
    die 'timed out waiting for authenticated Zot'
}

base64url() {
    openssl base64 -A | tr '+/' '-_' | tr -d '='
}

issue_pull_token() {
    local repository=$1 now header payload unsigned signature
    now=$(date +%s)
    header=$(printf '%s' '{"alg":"RS256","kid":"platform-smoke-v1","typ":"JWT"}' | base64url)
    payload=$(jq -cn --arg authority "$registry_authority" --arg repository "$repository" --argjson now "$now" \
        '{iss:"https://platform-smoke.invalid/v1/registry/token",aud:$authority,sub:"workload:platform-smoke",iat:$now,nbf:$now,exp:($now + 300),jti:"platform-smoke",access:[{type:"repository",name:$repository,actions:["pull"]}]}' | base64url)
    unsigned="$header.$payload"
    signature=$(printf '%s' "$unsigned" | openssl dgst -sha256 -sign "$fixture_root/registry-token-private.pem" | base64url)
    printf '%s.%s\n' "$unsigned" "$signature"
}

verify_pulled_builder() {
    local key=$1 image=$2
    case "$key" in
        ubuntu-native)
            "$container_engine" run --rm --network none "$image" /bin/sh -ec \
                'test "$(. /etc/os-release; printf "%s" "$VERSION_ID")" = 24.04; test "$(bash --version | head -n1 | awk "{print \$4}" | cut -d"(" -f1)" = 5.2.21; test "$(git --version | awk "{print \$3}")" = 2.43.0'
            ;;
        rust-ubuntu)
            "$container_engine" run --rm --network none "$image" /bin/sh -ec \
                'test "$(. /etc/os-release; printf "%s" "$VERSION_ID")" = 24.04; test "$(rustc --version | awk "{print \$2}")" = 1.88.0; test "$(cargo --version | awk "{print \$2}")" = 1.88.0'
            ;;
        typescript-node-ubuntu)
            "$container_engine" run --rm --network none "$image" /bin/sh -ec \
                'test "$(. /etc/os-release; printf "%s" "$VERSION_ID")" = 24.04; test "$(node --version)" = v24.19.0; test "$(pnpm --version)" = 11.20.0; test "$(tsc --version | awk "{print \$2}")" = 5.9.3'
            ;;
        python-ubuntu)
            "$container_engine" run --rm --network none "$image" /bin/sh -ec \
                'test "$(. /etc/os-release; printf "%s" "$VERSION_ID")" = 24.04; test "$(python3 --version | awk "{print \$2}")" = 3.13.5; test "$(python3 -m pip --version | awk "{print \$2}")" = 25.1.1'
            ;;
        *) die "unknown platform builder: $key" ;;
    esac
}

cleanup() {
    local status=$?
    if [[ $status -ne 0 ]]; then
        printf 'platform builder smoke retained failure evidence: %s\n' "$fixture_root" >&2
        [[ -f $fixture_root/zot.log ]] && tail -200 "$fixture_root/zot.log" >&2
        [[ -f $fixture_root/skopeo.log ]] && tail -200 "$fixture_root/skopeo.log" >&2
        "$container_engine" logs "$zot_container" >&2 2>/dev/null || true
    fi
    "$container_engine" rm --force "$zot_container" >/dev/null 2>&1 || true
    "$container_engine" rm --force "$postgres_container" >/dev/null 2>&1 || true
    [[ $status -eq 0 ]] && rm -rf -- "$fixture_root"
    return "$status"
}
trap cleanup EXIT

if [[ ${1:-} == --self-test ]]; then
    [[ $# == 1 ]] || die 'usage: smoke-platform-builder-release.sh [--self-test]'
    [[ $layout_root = /* ]]
    printf '%s\n' 'platform builder release smoke self-test passed'
    exit 0
fi
[[ $# == 0 ]] || die 'usage: smoke-platform-builder-release.sh [--self-test]'

for command in cargo curl git jq openssl python3 "$container_engine"; do
    command -v "$command" >/dev/null || die "required command is unavailable: $command"
done
[[ $layout_root = /* && -d $layout_root && ! -L $layout_root ]] || die "layout root must be an existing absolute non-symlink directory: $layout_root"
[[ -f $layout_root/.platform-builder-release.json ]] || die 'layout root is not a platform-builder release output'

readonly skopeo_binary="$(require_executable HEPHAESTUS_SKOPEO skopeo)"
readonly oras_binary="$(require_executable HEPHAESTUS_ORAS oras)"
readonly jq_binary="$(require_executable HEPHAESTUS_JQ jq)"

mkdir -p "$fixture_root/zot-storage" "$fixture_root/credentials" "$fixture_root/tools" \
    "$fixture_root/repositories" "$fixture_root/artifacts"
chmod 0700 "$fixture_root/credentials"
readonly zot_port="$(reserve_port)"
readonly registry_authority="127.0.0.1:$zot_port"
readonly registry_origin="http://$registry_authority"

openssl genpkey -algorithm RSA -pkeyopt rsa_keygen_bits:2048 \
    -out "$fixture_root/registry-token-private.pem" >/dev/null 2>&1
openssl req -new -x509 -key "$fixture_root/registry-token-private.pem" \
    -out "$fixture_root/registry-token-verification.crt" -days 1 \
    -subj '/CN=hephaestus-platform-smoke' >/dev/null 2>&1
chmod 0400 "$fixture_root/registry-token-private.pem" "$fixture_root/registry-token-verification.crt"

sed \
    -e 's|{{ zot.storage_root }}|/var/lib/registry|g' \
    -e 's|{{ zot.private_address }}|127.0.0.1|g' \
    -e "s|{{ zot.private_port }}|$zot_port|g" \
    -e 's|{{ hephaestus.registry_token_realm }}|https://platform-smoke.invalid/v1/registry/token|g' \
    -e "s|{{ hephaestus.registry_service }}|$registry_authority|g" \
    -e 's|{{ hephaestus.registry_notification_sink_url }}|http://127.0.0.1:1/unused|g' \
    -e 's|{{ hephaestus.registry_notification_callback_token }}|platform-smoke-unused-token|g' \
    "$repository_root/deploy/zot/zot-config.json.tera" >"$fixture_root/zot-config.json"

cat >"$fixture_root/tools/skopeo" <<WRAPPER
#!/bin/sh
if [ "\$1" = --version ]; then exec "$skopeo_binary" "\$@"; fi
if [ "\$1" != copy ]; then exit 64; fi
shift
exec "$skopeo_binary" copy --dest-tls-verify=false "\$@" >>"$fixture_root/skopeo.log" 2>&1
WRAPPER
cat >"$fixture_root/tools/oras" <<WRAPPER
#!/bin/sh
if [ "\$1" = --version ]; then exec "$oras_binary" version; fi
exec "$oras_binary" "\$@"
WRAPPER
chmod 0500 "$fixture_root/tools/skopeo" "$fixture_root/tools/oras"

"$container_engine" run --detach --rm --name "$postgres_container" \
    --env POSTGRES_PASSWORD=postgres --env POSTGRES_DB=hephaestus \
    --publish 127.0.0.1::5432 "$postgres_image" >/dev/null
for _attempt in {1..300}; do
    "$container_engine" exec "$postgres_container" pg_isready --quiet --username postgres --dbname hephaestus && break
    sleep 0.1
done
readonly postgres_port="$(published_port "$postgres_container" 5432)"
readonly database_url="postgres://postgres:postgres@127.0.0.1:$postgres_port/hephaestus?sslmode=disable"

"$container_engine" run --detach --rm --name "$zot_container" --network host \
    --read-only --tmpfs /tmp:rw,noexec,nosuid,nodev \
    --volume "$fixture_root/zot-config.json:/etc/zot/config.json:ro,Z" \
    --volume "$fixture_root/registry-token-verification.crt:/etc/zot/verification.crt:ro,Z" \
    --volume "$fixture_root/zot-storage:/var/lib/registry:rw,Z" \
    "$zot_image" serve /etc/zot/config.json >"$fixture_root/zot.log" 2>&1
wait_for_registry

cargo build -p bootstrap-postgres --bin hephaestus-e2e-seed --bin hephaestus-operator \
    -p registry-release --bin hephaestus-registry-release
HEPHAESTUS_DATABASE_URL="$database_url" \
HEPHAESTUS_REPOSITORY_ROOT="$fixture_root/repositories" \
HEPHAESTUS_ARTIFACT_ROOT="$fixture_root/artifacts" \
    "$repository_root/target/debug/hephaestus-e2e-seed" --schema-only

export HEPHAESTUS_FORGE_REGISTRY_AUTHORITY="$registry_authority"
export HEPHAESTUS_REGISTRY_SERVICE="$registry_authority"
export HEPHAESTUS_REGISTRY_PRIVATE_ORIGIN="$registry_origin/"
export HEPHAESTUS_DATABASE_URL="$database_url"
export HEPHAESTUS_REGISTRY_TOKEN_PRIVATE_KEY="$fixture_root/registry-token-private.pem"
export HEPHAESTUS_REGISTRY_TOKEN_ISSUER='https://platform-smoke.invalid/v1/registry/token'
export HEPHAESTUS_REGISTRY_TOKEN_KEY_ID='platform-smoke-v1'
export HEPHAESTUS_REGISTRY_TOKEN_LIFETIME_SECONDS='300'
export HEPHAESTUS_PLATFORM_CREDENTIAL_ROOT="$fixture_root/credentials"
export HEPHAESTUS_SKOPEO="$fixture_root/tools/skopeo"
export HEPHAESTUS_SKOPEO_VERSION="$("$skopeo_binary" --version | sed -n '1p')"
export HEPHAESTUS_ORAS="$fixture_root/tools/oras"
export HEPHAESTUS_ORAS_VERSION="$("$oras_binary" version | sed -n '1p')"
export HEPHAESTUS_JQ="$jq_binary"
export HEPHAESTUS_JQ_VERSION="$($jq_binary --version)"
export HEPHAESTUS_REGISTRY_RELEASE="$repository_root/target/debug/hephaestus-registry-release"
export HEPHAESTUS_REGISTRY_RELEASE_VERSION="$("$repository_root/target/debug/hephaestus-registry-release" --version)"

readonly review_output="$fixture_root/review.json"
readonly catalog_output="$fixture_root/platform-builder-catalog.json"
if ! publication_output=$("$repository_root/scripts/publish-platform-builders.sh" \
    --input-root "$layout_root" --review-output "$review_output" --catalog-output "$catalog_output" 2>&1); then
    printf '%s\n' "$publication_output" >&2
    die 'platform builder publication smoke failed'
fi
printf '%s\n' "$publication_output"
"$repository_root/target/debug/hephaestus-operator" provision-builder-catalog "$catalog_output" \
    >"$fixture_root/catalog-apply.json"

for key in ubuntu-native rust-ubuntu typescript-node-ubuntu python-ubuntu; do
    reference=$("$jq_binary" -er --arg key "$key" '.images[] | select(.key == $key) | .image_reference' "$catalog_output")
    token=$(issue_pull_token "platform/builders/$key")
    "$skopeo_binary" inspect --registry-token "$token" --tls-verify=false "docker://$reference" >/dev/null
    pulled_layout="$fixture_root/pulled/$key"
    mkdir -p -- "$pulled_layout"
    "$skopeo_binary" copy --src-registry-token "$token" --src-tls-verify=false \
        "docker://$reference" "oci:$pulled_layout:approved" >/dev/null
    verify_pulled_builder "$key" "oci:$pulled_layout:approved"
done

readonly retained_review="$(mktemp /tmp/hephaestus-platform-smoke-review.XXXXXX.json)"
readonly retained_catalog="$(mktemp /tmp/hephaestus-platform-smoke-catalog.XXXXXX.json)"
chmod 0600 "$retained_review" "$retained_catalog"
cp -- "$review_output" "$retained_review"
cp -- "$catalog_output" "$retained_catalog"
printf '%s\n' 'Platform builder release smoke passed: four images published, approved, cataloged, and pulled.' \
    "Review artifacts: $retained_review and $retained_catalog"
