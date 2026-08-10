#!/usr/bin/env bash
set -euo pipefail

repository_root=$(git rev-parse --show-toplevel)
container_engine=${CONTAINER_ENGINE:-podman}
zot_image='ghcr.io/project-zot/zot@sha256:6f7bf2b8e43437c7c3a121bc80214845c85f27321e66f2ff4be6bf4220775fd7'
zot_binary='/usr/local/bin/zot-linux-amd64'
config_template="$repository_root/deploy/zot/zot-config.json.tera"
edge_template="$repository_root/deploy/zot/registry-edge.caddy.tera"
notification_fixture="$repository_root/deploy/zot/test-fixtures/notification-sink.py"
temporary_root=$(mktemp -d)
container_name="hephaestus-zot-smoke-${PPID}-${RANDOM}"
repository_path='platform/builders/smoke'
registry_authority='registry.smoke.invalid'
notification_callback_token='0123456789abcdefghi_jklmnopqrstuvwxyz-ABCDEFG'
notification_pid=''

cleanup() {
    if [[ -n $notification_pid ]]; then
        kill "$notification_pid" >/dev/null 2>&1 || true
        wait "$notification_pid" 2>/dev/null || true
    fi
    "$container_engine" rm --force "$container_name" >/dev/null 2>&1 || true
    rm -rf "$temporary_root"
}
trap cleanup EXIT

for command in curl openssl sed skopeo tr jq python3 awk wc timeout; do
    command -v "$command" >/dev/null || {
        printf 'required command is unavailable: %s\n' "$command" >&2
        exit 1
    }
done

test -f "$config_template"
test -f "$edge_template"
test -x "$notification_fixture"

reserve_port() {
    python3 -c 'import socket; sock = socket.socket(); sock.bind(("127.0.0.1", 0)); print(sock.getsockname()[1]); sock.close()'
}

zot_port=$(reserve_port)
notification_port=$(reserve_port)
registry_base="http://127.0.0.1:${zot_port}"
notification_sink_url="http://127.0.0.1:${notification_port}/internal/zot-notifications"
notification_events="$temporary_root/notification-events.jsonl"
notification_status="$temporary_root/notification-status"

start_notification_sink() {
    printf '200\n' >"$notification_status"
    python3 "$notification_fixture" \
        --port "$notification_port" \
        --output "$notification_events" \
        --status-file "$notification_status" \
        >"$temporary_root/notification-sink.log" 2>&1 &
    notification_pid=$!

    for _attempt in $(seq 1 30); do
        if curl --silent --fail "http://127.0.0.1:${notification_port}/healthz" >/dev/null; then
            return
        fi
        sleep 1
    done
    printf 'notification fixture did not become ready\n' >&2
    return 1
}

stop_notification_sink() {
    if [[ -n $notification_pid ]]; then
        kill "$notification_pid"
        wait "$notification_pid" 2>/dev/null || true
        notification_pid=''
    fi
}

render_config() {
    sed \
        -e 's|{{ zot.storage_root }}|/var/lib/registry|g' \
        -e 's|{{ zot.private_address }}|127.0.0.1|g' \
        -e "s|{{ zot.private_port }}|${zot_port}|g" \
        -e 's|{{ hephaestus.registry_token_realm }}|https://token.invalid/v1/registry/token|g' \
        -e "s|{{ hephaestus.registry_service }}|${registry_authority}|g" \
        -e "s|{{ hephaestus.registry_notification_sink_url }}|${notification_sink_url}|g" \
        -e "s|{{ hephaestus.registry_notification_callback_token }}|${notification_callback_token}|g" \
        "$config_template" >"$temporary_root/config.json"
}

start_zot() {
    "$container_engine" rm --force "$container_name" >/dev/null 2>&1 || true
    "$container_engine" run --rm --detach \
        --name "$container_name" \
        --network host \
        --read-only \
        --tmpfs /tmp:rw,noexec,nosuid,nodev \
        --volume "$temporary_root/config.json:/etc/zot/config.json:ro,Z" \
        --volume "$temporary_root/verification.crt:/etc/zot/verification.crt:ro,Z" \
        --volume "$temporary_root/storage:/var/lib/registry:rw,Z" \
        "$zot_image" serve /etc/zot/config.json >/dev/null

    for _attempt in $(seq 1 30); do
        response_headers="$temporary_root/response-headers"
        status=$(curl --silent --output /dev/null --dump-header "$response_headers" \
            --write-out '%{http_code}' "$registry_base/v2/" || true)
        if [[ $status == '401' ]] && grep -qi '^Www-Authenticate: Bearer ' "$response_headers"; then
            return
        fi
        sleep 1
    done
    "$container_engine" logs "$container_name" >&2 || true
    printf 'Zot did not become ready\n' >&2
    return 1
}

stop_zot() {
    "$container_engine" stop "$container_name" >/dev/null
}

base64url() {
    openssl base64 -A | tr '+/' '-_' | tr -d '='
}

issue_token() {
    local actions=$1
    local access issued_at expires_at token_header token_payload token_signing_input token_signature
    access=$(jq -cn --arg repository "$repository_path" --arg actions "$actions" \
        '{type: "repository", name: $repository, actions: ($actions | split(","))}')
    issued_at=$(date -u +%s)
    expires_at=$((issued_at + 300))
    token_header=$(printf '%s' '{"alg":"RS256","typ":"JWT","kid":"smoke-v1"}' | base64url)
    token_payload=$(jq -cn \
        --argjson issued_at "$issued_at" \
        --argjson expires_at "$expires_at" \
        --argjson access "$access" \
        '{iss: "https://token.invalid/v1/registry/token", aud: "registry.smoke.invalid", sub: "workload:zot-smoke", iat: $issued_at, nbf: $issued_at, exp: $expires_at, jti: "123e4567-e89b-12d3-a456-426614174000", access: [$access]}' \
        | base64url)
    token_signing_input="${token_header}.${token_payload}"
    token_signature=$(printf '%s' "$token_signing_input" | openssl dgst -sha256 \
        -sign "$temporary_root/verification.key" -binary | base64url)
    registry_token="${token_signing_input}.${token_signature}"
}

write_authfile() {
    local token=$1
    local destination=$2
    jq -cn --arg authority "127.0.0.1:${zot_port}" --arg token "$token" \
        '{auths: {($authority): {identitytoken: $token}}}' >"$destination"
}

curl_status() {
    curl --silent --show-error --output /dev/null --write-out '%{http_code}' "$@"
}

notification_count() {
    if [[ ! -s $notification_events ]]; then
        printf '0\n'
        return
    fi
    jq -s --arg path '/internal/zot-notifications' \
        '[.[] | select(.path == $path)] | length' "$notification_events"
}

wait_for_notifications() {
    local expected=$1
    local observed
    for _attempt in $(seq 1 30); do
        observed=$(notification_count)
        if (( observed >= expected )); then
            return
        fi
        sleep 1
    done
    printf 'expected at least %s Zot notifications, received %s\n' "$expected" "$observed" >&2
    "$container_engine" logs "$container_name" >&2 || true
    return 1
}

upload_blob() {
    local file=$1
    local digest=$2
    local headers location upload_url separator
    headers="$temporary_root/upload-headers-${RANDOM}"
    curl --silent --show-error --fail --dump-header "$headers" --output /dev/null \
        --header "Authorization: Bearer ${push_token}" \
        --request POST "$registry_base/v2/${repository_path}/blobs/uploads/"
    location=$(awk 'tolower($1) == "location:" {print $2}' "$headers" | tr -d '\r' | tail -n 1)
    test -n "$location"
    case "$location" in
        http://*|https://*) upload_url=$location ;;
        /*) upload_url="${registry_base}${location}" ;;
        *) upload_url="${registry_base}/${location}" ;;
    esac
    if [[ $upload_url == *\?* ]]; then separator='&'; else separator='?'; fi
    curl --silent --show-error --fail --output /dev/null \
        --header "Authorization: Bearer ${push_token}" \
        --header 'Content-Type: application/octet-stream' \
        --request PUT --data-binary "@${file}" \
        "${upload_url}${separator}digest=${digest}"
}

push_manifest() {
    local reference=$1
    local file=$2
    local media_type=$3
    curl --silent --show-error --fail --output /dev/null \
        --header "Authorization: Bearer ${push_token}" \
        --header "Content-Type: ${media_type}" \
        --request PUT --data-binary "@${file}" \
        "$registry_base/v2/${repository_path}/manifests/${reference}"
}

make_oci_layout() {
    local layout=$1
    local config config_digest config_size manifest manifest_digest manifest_size
    mkdir -p "$layout/blobs/sha256"
    printf '%s\n' '{"imageLayoutVersion":"1.0.0"}' >"$layout/oci-layout"
    config='{"architecture":"amd64","os":"linux","rootfs":{"type":"layers","diff_ids":[]}}'
    config_size=$(printf '%s' "$config" | wc -c | tr -d ' ')
    config_digest=$(printf '%s' "$config" | openssl dgst -sha256 -r | awk '{print $1}')
    printf '%s' "$config" >"$layout/blobs/sha256/${config_digest}"
    manifest=$(jq -cn --arg digest "sha256:${config_digest}" --argjson size "$config_size" \
        '{schemaVersion: 2, mediaType: "application/vnd.oci.image.manifest.v1+json", config: {mediaType: "application/vnd.oci.image.config.v1+json", digest: $digest, size: $size}, layers: []}')
    manifest_size=$(printf '%s' "$manifest" | wc -c | tr -d ' ')
    manifest_digest=$(printf '%s' "$manifest" | openssl dgst -sha256 -r | awk '{print $1}')
    printf '%s' "$manifest" >"$layout/blobs/sha256/${manifest_digest}"
    jq -cn --arg digest "sha256:${manifest_digest}" --argjson size "$manifest_size" \
        '{schemaVersion: 2, manifests: [{mediaType: "application/vnd.oci.image.manifest.v1+json", digest: $digest, size: $size, annotations: {"org.opencontainers.image.ref.name": "smoke"}}]}' \
        >"$layout/index.json"
}

sed \
    -e 's|{{ registry.authority }}|registry.smoke.invalid|g' \
    -e 's|{{ registry.maximum_request_bytes }}|1GB|g' \
    -e 's|{{ registry.upload_timeout }}|5m|g' \
    -e 's|{{ zot.private_port }}|5000|g' \
    "$edge_template" >"$temporary_root/Caddyfile"
grep -Fq '@distribution path /v2/*' "$temporary_root/Caddyfile"
grep -Fq 'reverse_proxy 127.0.0.1:5000' "$temporary_root/Caddyfile"
grep -Fq 'X-Forwarded-Proto https' "$temporary_root/Caddyfile"
grep -Fq 'respond "not found" 404' "$temporary_root/Caddyfile"
! grep -Fq '{{' "$temporary_root/Caddyfile"

version=$("$container_engine" run --rm --entrypoint "$zot_binary" "$zot_image" --version 2>&1)
grep -q '"commit":"v2.1.18-7bb211bcd4352b90f3e99752607fbd1f050bf7ca"' <<<"$version"

umask 077
mkdir -p "$temporary_root/storage"
openssl req -x509 -newkey rsa:2048 -nodes \
    -keyout "$temporary_root/verification.key" \
    -out "$temporary_root/verification.crt" \
    -days 1 \
    -subj '/CN=hephaestus-zot-smoke' \
    >/dev/null 2>&1
render_config

grep -Fq '"events"' "$temporary_root/config.json"
grep -Fq '"enable": true' "$temporary_root/config.json"
grep -Fq "$notification_sink_url" "$temporary_root/config.json"
grep -Fq "$notification_callback_token" "$temporary_root/config.json"

"$container_engine" run --rm --entrypoint "$zot_binary" \
    --volume "$temporary_root/config.json:/etc/zot/config.json:ro,Z" \
    --volume "$temporary_root/verification.crt:/etc/zot/verification.crt:ro,Z" \
    "$zot_image" verify /etc/zot/config.json

printf 'not a PEM certificate\n' >"$temporary_root/invalid-verification.crt"
set +e
timeout 10 "$container_engine" run --rm --entrypoint "$zot_binary" \
    --volume "$temporary_root/config.json:/etc/zot/config.json:ro,Z" \
    --volume "$temporary_root/invalid-verification.crt:/etc/zot/verification.crt:ro,Z" \
    "$zot_image" serve /etc/zot/config.json >/dev/null 2>&1
invalid_certificate_status=$?
set -e
if (( invalid_certificate_status == 0 || invalid_certificate_status == 124 )); then
    printf 'Zot did not promptly reject an invalid Bearer verification certificate\n' >&2
    exit 1
fi
jq '.http.port = "not-a-port"' "$temporary_root/config.json" >"$temporary_root/invalid-config.json"
if "$container_engine" run --rm --entrypoint "$zot_binary" \
    --volume "$temporary_root/invalid-config.json:/etc/zot/config.json:ro,Z" \
    --volume "$temporary_root/verification.crt:/etc/zot/verification.crt:ro,Z" \
    "$zot_image" verify /etc/zot/config.json >/dev/null 2>&1; then
    printf 'Zot accepted a configuration with a non-numeric listener port\n' >&2
    exit 1
fi

start_notification_sink
start_zot

test "$status" = '401'
grep -qi '^Www-Authenticate: Bearer ' "$response_headers"
grep -qi 'realm="https://token.invalid/v1/registry/token"' "$response_headers"
grep -qi "service=\"${registry_authority}\"" "$response_headers"

issue_token pull
pull_token=$registry_token
pull_status=$(curl_status --header "Authorization: Bearer ${pull_token}" "$registry_base/v2/")
test "$pull_status" = '200'
write_authfile "$pull_token" "$temporary_root/pull-auth.json"
if ! skopeo list-tags --authfile "$temporary_root/pull-auth.json" --tls-verify=false \
    "docker://127.0.0.1:${zot_port}/${repository_path}" \
    >"$temporary_root/skopeo-output" 2>"$temporary_root/skopeo-error"; then
    if grep -Eqi 'unauthorized|authentication required|invalid credentials' "$temporary_root/skopeo-error"; then
        printf 'Skopeo rejected the direct identity token\n' >&2
        exit 1
    fi
fi
pull_only_upload_status=$(curl_status --header "Authorization: Bearer ${pull_token}" \
    --request POST "$registry_base/v2/${repository_path}/blobs/uploads/")
case "$pull_only_upload_status" in
    401|403) ;;
    *) printf 'pull-only token unexpectedly started an upload: HTTP %s\n' "$pull_only_upload_status" >&2; exit 1 ;;
esac

issue_token pull,push
push_token=$registry_token
write_authfile "$push_token" "$temporary_root/push-auth.json"
layout="$temporary_root/oci-layout"
make_oci_layout "$layout"
skopeo copy --dest-registry-token "$push_token" --dest-tls-verify=false \
    "oci:${layout}:smoke" "docker://127.0.0.1:${zot_port}/${repository_path}:subject" \
    >"$temporary_root/skopeo-push.log"

subject_digest=$(skopeo inspect --registry-token "$pull_token" --tls-verify=false \
    --format '{{.Digest}}' "docker://127.0.0.1:${zot_port}/${repository_path}:subject")
[[ $subject_digest =~ ^sha256:[0-9a-f]{64}$ ]]
skopeo copy --src-registry-token "$pull_token" --src-tls-verify=false \
    "docker://127.0.0.1:${zot_port}/${repository_path}@${subject_digest}" \
    "oci:${temporary_root}/pulled:subject" >"$temporary_root/skopeo-pull.log"
wait_for_notifications 1

jq -se --arg token "Bearer ${notification_callback_token}" '
    [ .[]
      | select(.path == "/internal/zot-notifications")
      | .headers
      | to_entries
      | map({key: (.key | ascii_downcase), value})
      | from_entries
    ]
    | length > 0
      and all(.authorization == $token
              and .["ce-specversion"] == "1.0"
              and (.["ce-id"] | type == "string" and length > 0)
              and (.["ce-type"] | type == "string" and length > 0))
' "$notification_events" >/dev/null

subject_raw="$temporary_root/subject-manifest.json"
curl --silent --show-error --fail --header "Authorization: Bearer ${pull_token}" \
    --header 'Accept: application/vnd.oci.image.manifest.v1+json' \
    "$registry_base/v2/${repository_path}/manifests/${subject_digest}" >"$subject_raw"
subject_size=$(wc -c <"$subject_raw" | tr -d ' ')
empty_config="$temporary_root/empty-config.json"
printf '{}' >"$empty_config"
empty_digest=$(openssl dgst -sha256 -r "$empty_config" | awk '{print $1}')
upload_blob "$empty_config" "sha256:${empty_digest}"
artifact_manifest="$temporary_root/sbom-manifest.json"
jq -cn --arg config_digest "sha256:${empty_digest}" \
    --arg subject_digest "$subject_digest" \
    --argjson subject_size "$subject_size" \
    '{schemaVersion: 2,
      mediaType: "application/vnd.oci.image.manifest.v1+json",
      artifactType: "application/spdx+json",
      config: {mediaType: "application/vnd.oci.empty.v1+json", digest: $config_digest, size: 2},
      layers: [],
      subject: {mediaType: "application/vnd.oci.image.manifest.v1+json", digest: $subject_digest, size: $subject_size}}' \
    >"$artifact_manifest"
artifact_digest="sha256:$(openssl dgst -sha256 -r "$artifact_manifest" | awk '{print $1}')"
push_manifest smoke-sbom "$artifact_manifest" 'application/vnd.oci.image.manifest.v1+json'
referrers="$temporary_root/referrers.json"
curl --silent --show-error --fail --header "Authorization: Bearer ${pull_token}" \
    --header 'Accept: application/vnd.oci.image.index.v1+json' \
    "$registry_base/v2/${repository_path}/referrers/${subject_digest}" >"$referrers"
jq -e --arg digest "$artifact_digest" --arg artifact_type 'application/spdx+json' \
    '[.manifests[] | select(.digest == $digest and .artifactType == $artifact_type)] | length == 1' \
    "$referrers" >/dev/null

unknown_digest="sha256:$(printf '%064d' 0)"
test "$(curl_status --header "Authorization: Bearer ${pull_token}" \
    "$registry_base/v2/${repository_path}/blobs/${unknown_digest}")" = '404'
test "$(curl_status --header "Authorization: Bearer ${pull_token}" \
    "$registry_base/v2/${repository_path}/manifests/${unknown_digest}")" = '404'

missing_manifest="$temporary_root/missing-content-manifest.json"
jq -cn --arg missing "$unknown_digest" \
    '{schemaVersion: 2, mediaType: "application/vnd.oci.image.manifest.v1+json", config: {mediaType: "application/vnd.oci.image.config.v1+json", digest: $missing, size: 1}, layers: []}' \
    >"$missing_manifest"
missing_push_status=$(curl_status --header "Authorization: Bearer ${push_token}" \
    --header 'Content-Type: application/vnd.oci.image.manifest.v1+json' \
    --request PUT --data-binary "@${missing_manifest}" \
    "$registry_base/v2/${repository_path}/manifests/missing-content")
case "$missing_push_status" in
    400|404) ;;
    201|202)
        if skopeo copy --src-registry-token "$pull_token" --src-tls-verify=false \
            "docker://127.0.0.1:${zot_port}/${repository_path}:missing-content" \
            "oci:${temporary_root}/missing-pull:missing-content" \
            >"$temporary_root/missing-content.log" 2>&1; then
            printf 'an OCI manifest with a missing config blob was pullable\n' >&2
            exit 1
        fi
        ;;
    *) printf 'unexpected missing-content manifest response: HTTP %s\n' "$missing_push_status" >&2; exit 1 ;;
esac

metrics_status=$(curl --silent --show-error --output "$temporary_root/metrics" \
    --write-out '%{http_code}' "$registry_base/metrics")
test "$metrics_status" = '200'
grep -q '^#' "$temporary_root/metrics"
for disabled_path in / /v2/_zot/ext/search /v2/_zot/ext/ui /v2/_zot/ext/mgmt; do
    disabled_status=$(curl_status "$registry_base${disabled_path}" || true)
    case "$disabled_status" in
        2??|3??)
            printf 'disabled Zot surface unexpectedly responded at %s: HTTP %s\n' \
                "$disabled_path" "$disabled_status" >&2
            exit 1
            ;;
    esac
done

delivered_before_outage=$(notification_count)
stop_notification_sink
skopeo copy --dest-registry-token "$push_token" --dest-tls-verify=false \
    "oci:${layout}:smoke" "docker://127.0.0.1:${zot_port}/${repository_path}:during-outage" \
    >"$temporary_root/skopeo-outage-push.log"
start_notification_sink
skopeo copy --dest-registry-token "$push_token" --dest-tls-verify=false \
    "oci:${layout}:smoke" "docker://127.0.0.1:${zot_port}/${repository_path}:after-outage" \
    >"$temporary_root/skopeo-recovery-push.log"
wait_for_notifications $((delivered_before_outage + 1))

notifications_before_restart=$(notification_count)
stop_zot
start_zot
skopeo inspect --registry-token "$pull_token" --tls-verify=false \
    --format '{{.Digest}}' "docker://127.0.0.1:${zot_port}/${repository_path}:subject" \
    | grep -Fx "$subject_digest"
skopeo copy --src-registry-token "$pull_token" --src-tls-verify=false \
    "docker://127.0.0.1:${zot_port}/${repository_path}@${subject_digest}" \
    "oci:${temporary_root}/pulled-after-restart:subject" >"$temporary_root/skopeo-restart-pull.log"
curl --silent --show-error --fail --header "Authorization: Bearer ${pull_token}" \
    --header 'Accept: application/vnd.oci.image.index.v1+json' \
    "$registry_base/v2/${repository_path}/referrers/${subject_digest}" >"$temporary_root/referrers-after-restart.json"
jq -e --arg digest "$artifact_digest" '[.manifests[] | select(.digest == $digest)] | length == 1' \
    "$temporary_root/referrers-after-restart.json" >/dev/null
sleep 2
test "$(notification_count)" = "$notifications_before_restart"

printf '%s\n' 'Zot smoke test passed: pinned config validation; invalid key/config rejection; Bearer challenge and scoped identity-token pull/push; Skopeo push/pull; OCI referrers; callback delivery plus non-blocking sink outage; restart persistence; missing-content pull failure; private metrics and disabled Zot surfaces; rendered edge deny policy.'
