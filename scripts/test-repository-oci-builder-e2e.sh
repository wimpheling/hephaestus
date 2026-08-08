#!/usr/bin/env bash
# Exercise repository-owned OCI preparation against ephemeral PostgreSQL and Zot.

set -Eeuo pipefail

readonly repository_root="$(git rev-parse --show-toplevel)"
readonly container_engine="${CONTAINER_ENGINE:-podman}"
readonly postgres_image="${HEPHAESTUS_POSTGRES_TEST_IMAGE:-docker.io/library/postgres:17-alpine}"
readonly nats_image="${HEPHAESTUS_NATS_TEST_IMAGE:-docker.io/library/nats:2.11-alpine}"
readonly zot_image='ghcr.io/project-zot/zot@sha256:6f7bf2b8e43437c7c3a121bc80214845c85f27321e66f2ff4be6bf4220775fd7'
readonly alpine_image='docker.io/library/alpine@sha256:4bcff63911fcb4448bd4fdacec207030997caf25e9bea4045fa6c8c44de311d1'
readonly fixture_root="$(mktemp -d)"
readonly secret_runtime_root="$(mktemp -d /dev/shm/hephaestus-oci-e2e-secrets.XXXXXX)"
readonly postgres_container="hephaestus-oci-postgres-$$"
readonly nats_container="hephaestus-oci-nats-$$"
readonly zot_container="hephaestus-oci-zot-$$"
readonly runtime_image="localhost/hephaestus-oci-runtime-e2e:$$"
readonly syft_fixture="$repository_root/deploy/zot/test-fixtures/syft-success"
readonly trivy_fixture="$repository_root/deploy/zot/test-fixtures/trivy-success"
daemon_pid=''
oidc_pid=''

cleanup() {
    local status=$?
    if [[ $status -ne 0 ]]; then
        [[ -f $fixture_root/daemon.log ]] && tail -200 "$fixture_root/daemon.log" >&2
        [[ -f $fixture_root/buildah.log ]] && tail -200 "$fixture_root/buildah.log" >&2
        [[ -f $fixture_root/skopeo.log ]] && tail -200 "$fixture_root/skopeo.log" >&2
        [[ -f $fixture_root/oras.log ]] && tail -200 "$fixture_root/oras.log" >&2
        "$container_engine" logs "$zot_container" >&2 2>/dev/null || true
        "$container_engine" exec "$postgres_container" psql --username postgres \
            --dbname hephaestus --command \
            "SELECT definition.id, definition.status, definition.failure_reason, preparation.state, preparation.failure_reason FROM project_builder_definitions definition LEFT JOIN project_builder_preparation_jobs preparation ON preparation.builder_id = definition.id ORDER BY definition.created_at" \
            >&2 2>/dev/null || true
    fi
    if [[ -n $daemon_pid ]]; then
        kill "$daemon_pid" >/dev/null 2>&1 || true
        wait "$daemon_pid" 2>/dev/null || true
    fi
    if [[ -n $oidc_pid ]]; then
        kill "$oidc_pid" >/dev/null 2>&1 || true
        wait "$oidc_pid" 2>/dev/null || true
    fi
    "$container_engine" rm --force "$zot_container" >/dev/null 2>&1 || true
    "$container_engine" rm --force "$nats_container" >/dev/null 2>&1 || true
    "$container_engine" rm --force "$postgres_container" >/dev/null 2>&1 || true
    "$container_engine" rmi "$runtime_image" >/dev/null 2>&1 || true
    rm -rf -- "$fixture_root" "$secret_runtime_root"
    return "$status"
}
trap cleanup EXIT

require_executable() {
    local variable=$1
    local fallback=$2
    local value=${!variable:-}
    if [[ -z $value ]]; then
        value=$(command -v "$fallback" || true)
    fi
    [[ -n $value && $value == /* && -x $value ]] || {
        printf 'required OCI E2E executable is unavailable: %s (set %s)\n' "$fallback" "$variable" >&2
        exit 1
    }
    printf '%s\n' "$value"
}

reserve_port() {
    python3 -c 'import socket; sock = socket.socket(); sock.bind(("127.0.0.1", 0)); print(sock.getsockname()[1]); sock.close()'
}

published_port() {
    "$container_engine" port "$1" "$2/tcp" | awk -F: '{print $NF}'
}

wait_for_url() {
    local url=$1
    local log=$2
    for _attempt in {1..600}; do
        if curl --fail --silent "$url" >/dev/null 2>&1; then
            return
        fi
        sleep 0.1
    done
    [[ -f $log ]] && tail -200 "$log" >&2
    printf 'timed out waiting for %s\n' "$url" >&2
    exit 1
}

wait_for_registry() {
    for _attempt in {1..300}; do
        local status
        status=$(curl --silent --output /dev/null --write-out '%{http_code}' \
            "$registry_origin/v2/" || true)
        [[ $status == 401 ]] && return
        sleep 0.1
    done
    printf 'timed out waiting for authenticated Zot\n' >&2
    exit 1
}

sql_value() {
    "$container_engine" exec "$postgres_container" psql --username postgres \
        --dbname hephaestus --tuples-only --no-align --command "$1" | tr -d '\r'
}

for command in cargo curl git jq node npm openssl python3 sed tar "$container_engine"; do
    command -v "$command" >/dev/null || {
        printf 'required command is unavailable: %s\n' "$command" >&2
        exit 1
    }
done
test -x "$syft_fixture"
test -x "$trivy_fixture"

readonly buildah_binary="$(require_executable HEPHAESTUS_BUILDAH_BINARY buildah)"
readonly skopeo_binary="$(require_executable HEPHAESTUS_SKOPEO skopeo)"
readonly oras_binary="$(require_executable HEPHAESTUS_ORAS oras)"
readonly umoci_binary="$(require_executable HEPHAESTUS_UMOCI_BINARY umoci)"
readonly git_binary="$(require_executable HEPHAESTUS_GIT_BINARY git)"
readonly tar_binary="$(require_executable HEPHAESTUS_TAR_BINARY tar)"

mkdir -p \
    "$fixture_root/repositories" \
    "$fixture_root/artifacts" \
    "$fixture_root/volumes" \
    "$fixture_root/workspaces" \
    "$fixture_root/runtime" \
    "$fixture_root/root-image" \
    "$fixture_root/zot-storage" \
    "$fixture_root/base-layout" \
    "$fixture_root/builder-checkouts" \
    "$fixture_root/builder-output" \
    "$fixture_root/builder-rootfs" \
    "$fixture_root/registry-credentials" \
    "$fixture_root/secret-keys" \
    "$fixture_root/test-tools"
chmod 0700 "$fixture_root/registry-credentials" "$fixture_root/secret-keys" "$secret_runtime_root"
head -c 32 /dev/zero | tr '\0' '\127' >"$fixture_root/secret-keys/e2e-v1"
chmod 0400 "$fixture_root/secret-keys/e2e-v1"

readonly oidc_port="$(reserve_port)"
readonly daemon_port="$(reserve_port)"
readonly zot_port="$(reserve_port)"
readonly oidc_url="http://127.0.0.1:$oidc_port"
readonly daemon_url="http://127.0.0.1:$daemon_port"
readonly registry_authority="127.0.0.1:$zot_port"
readonly registry_origin="http://$registry_authority"

openssl genpkey -algorithm RSA -pkeyopt rsa_keygen_bits:2048 \
    -out "$fixture_root/registry-token-private.pem" >/dev/null 2>&1
openssl req -new -x509 -key "$fixture_root/registry-token-private.pem" \
    -out "$fixture_root/registry-token-verification.crt" -days 1 \
    -subj '/CN=hephaestus-repository-oci-e2e' >/dev/null 2>&1
printf '%s\n' 'repository-oci-e2e-notification-token-0123456789abcdef' \
    >"$fixture_root/registry-notification-token"
chmod 0400 "$fixture_root/registry-token-private.pem" \
    "$fixture_root/registry-token-verification.crt" \
    "$fixture_root/registry-notification-token"

sed \
    -e 's|{{ zot.storage_root }}|/var/lib/registry|g' \
    -e 's|{{ zot.private_address }}|127.0.0.1|g' \
    -e "s|{{ zot.private_port }}|$zot_port|g" \
    -e "s|{{ hephaestus.registry_token_realm }}|$daemon_url/v1/registry/token|g" \
    -e "s|{{ hephaestus.registry_service }}|$registry_authority|g" \
    -e "s|{{ hephaestus.registry_notification_sink_url }}|$daemon_url/internal/zot-notifications|g" \
    -e 's|{{ hephaestus.registry_notification_callback_token }}|repository-oci-e2e-notification-token-0123456789abcdef|g' \
    "$repository_root/deploy/zot/zot-config.json.tera" >"$fixture_root/zot-config.json"

"$container_engine" run --detach --rm --name "$postgres_container" \
    --env POSTGRES_PASSWORD=postgres --env POSTGRES_DB=hephaestus \
    --publish 127.0.0.1::5432 "$postgres_image" >/dev/null
"$container_engine" run --detach --rm --name "$nats_container" \
    --publish 127.0.0.1::4222 "$nats_image" --jetstream --store_dir /tmp/nats >/dev/null
for _attempt in {1..300}; do
    "$container_engine" exec "$postgres_container" pg_isready --quiet \
        --username postgres --dbname hephaestus && break
    sleep 0.1
done
readonly postgres_port="$(published_port "$postgres_container" 5432)"
readonly nats_port="$(published_port "$nats_container" 4222)"
readonly database_url="postgres://postgres:postgres@127.0.0.1:$postgres_port/hephaestus?sslmode=disable"

"$container_engine" run --detach --rm --name "$zot_container" --network host \
    --read-only --tmpfs /tmp:rw,noexec,nosuid,nodev \
    --volume "$fixture_root/zot-config.json:/etc/zot/config.json:ro,Z" \
    --volume "$fixture_root/registry-token-verification.crt:/etc/zot/verification.crt:ro,Z" \
    --volume "$fixture_root/zot-storage:/var/lib/registry:rw,Z" \
    "$zot_image" serve /etc/zot/config.json >/dev/null
wait_for_registry

if [[ ! -d $repository_root/e2e/playwright/node_modules/jose ]]; then
    npm --prefix "$repository_root/e2e/playwright" ci --ignore-scripts
fi
HEPHAESTUS_E2E_OIDC_PORT="$oidc_port" HEPHAESTUS_E2E_WEB_URL='http://127.0.0.1:1' \
    node "$repository_root/e2e/playwright/oidc-provider.mjs" >"$fixture_root/oidc.log" 2>&1 &
oidc_pid=$!
wait_for_url "$oidc_url/.well-known/openid-configuration" "$fixture_root/oidc.log"

cargo build -p hephaestus-app --bin hephaestusd -p bootstrap-postgres --bin hephaestus-e2e-seed \
    -p git-http --bin pre-receive
HEPHAESTUS_DATABASE_URL="$database_url" \
HEPHAESTUS_REPOSITORY_ROOT="$fixture_root/repositories" \
HEPHAESTUS_ARTIFACT_ROOT="$fixture_root/artifacts" \
HEPHAESTUS_BROWSER_OIDC_ISSUER="$oidc_url" \
    "$repository_root/target/debug/hephaestus-e2e-seed" >"$fixture_root/seed.json"
readonly project_id="$(jq -r '.project_id' "$fixture_root/seed.json")"
readonly repository_id="$(jq -r '.repository_id' "$fixture_root/seed.json")"

"$skopeo_binary" copy --override-os linux --override-arch amd64 \
    "docker://$alpine_image" "oci:$fixture_root/base-layout:base" >/dev/null
readonly base_digest="$(jq -r '.manifests[] | select(.annotations["org.opencontainers.image.ref.name"] == "base") | .digest' "$fixture_root/base-layout/index.json")"
[[ $base_digest =~ ^sha256:[0-9a-f]{64}$ ]]
readonly base_reference="$registry_authority/platform/builders/repository-e2e-base@$base_digest"
"$container_engine" exec -i "$postgres_container" psql --username postgres \
    --dbname hephaestus >/dev/null <<SQL
INSERT INTO builder_images
    (id, key, display_name, image_reference, toolchains, architectures,
     preparation_state, availability_state, network_ceiling, max_vcpus,
     max_memory_mib, dependency_policy, provenance, platform_policy_version)
VALUES
    (gen_random_uuid(), 'repository-e2e-base', 'Repository E2E base',
     '$base_reference', '[{"name":"shell","version":"alpine-3.22.1"}]'::jsonb,
     ARRAY['amd64'], 'ready', 'available', 'disabled', 2, 512,
     'vendored_offline', '{"source":"pinned-e2e-fixture"}'::jsonb, 'e2e/v1');
SQL
jq -n --arg reference "$base_reference" --arg path "$fixture_root/base-layout" \
    '{($reference): $path}' >"$fixture_root/base-layouts.json"
printf '%s\n' '{}' >"$fixture_root/syft.yaml"

cat >"$fixture_root/test-tools/skopeo" <<WRAPPER
#!/bin/sh
if [ "\$1" != copy ]; then
    printf '%s\\n' 'repository OCI E2E Skopeo wrapper only permits copy' >&2
    exit 64
fi
shift
exec "$skopeo_binary" copy --dest-tls-verify=false "\$@" >>"$fixture_root/skopeo.log" 2>&1
WRAPPER
cat >"$fixture_root/test-tools/buildah" <<WRAPPER
#!/bin/sh
exec "$buildah_binary" "\$@" >>"$fixture_root/buildah.log" 2>&1
WRAPPER
cat >"$fixture_root/test-tools/oras" <<WRAPPER
#!/bin/sh
case "\$1" in
    attach|discover)
        command=\$1
        shift
        exec "$oras_binary" "\$command" --plain-http "\$@" >>"$fixture_root/oras.log" 2>&1
        ;;
    manifest)
        shift
        subcommand=\$1
        shift
        exec "$oras_binary" manifest "\$subcommand" --plain-http "\$@" >>"$fixture_root/oras.log" 2>&1
        ;;
    *) exit 64 ;;
esac
WRAPPER
chmod 0500 "$fixture_root/test-tools/buildah" "$fixture_root/test-tools/skopeo" \
    "$fixture_root/test-tools/oras"

readonly root_image_manifest="$fixture_root/root-image-manifest.json"
printf '{"version":1,"roots":{"fixture-root@sha256:%s":{"kind":"directory","path":"%s"}}}\n' \
    "$(printf 'a%.0s' {1..64})" "$fixture_root/root-image" >"$root_image_manifest"
export HEPHAESTUS_DATABASE_URL="$database_url"
export HEPHAESTUS_NATS_URL="nats://127.0.0.1:$nats_port"
export HEPHAESTUS_HTTP_LISTEN="127.0.0.1:$daemon_port"
export HEPHAESTUS_REPOSITORY_ROOT="$fixture_root/repositories"
export HEPHAESTUS_GIT_HTTP_BACKEND="$(git --exec-path)/git-http-backend"
export HEPHAESTUS_GIT_PRE_RECEIVE_HOOK="$repository_root/target/debug/pre-receive"
export HEPHAESTUS_OIDC_ISSUER="$oidc_url"
export HEPHAESTUS_OIDC_AUDIENCE='hephaestus-git'
export HEPHAESTUS_OIDC_ALGORITHM='HS256'
export HEPHAESTUS_OIDC_HS256_SECRET='e2e-signing-secret-with-sufficient-entropy'
export HEPHAESTUS_VOLUME_ROOT="$fixture_root/volumes"
export HEPHAESTUS_WORKSPACE_ROOT="$fixture_root/workspaces"
export HEPHAESTUS_ARTIFACT_ROOT="$fixture_root/artifacts"
export HEPHAESTUS_RUNTIME_ROOT="$fixture_root/runtime"
export HEPHAESTUS_SECRET_RUNTIME_ROOT="$secret_runtime_root"
export HEPHAESTUS_SECRET_KEY_DIRECTORY="$fixture_root/secret-keys"
export HEPHAESTUS_SECRET_KEY_REFERENCE='e2e-v1'
export HEPHAESTUS_RPC_MEDIATOR_SECRET='e2e-rpc-mediator-secret-with-sufficient-entropy'
export HEPHAESTUS_REGISTRY_TOKEN_PRIVATE_KEY="$fixture_root/registry-token-private.pem"
export HEPHAESTUS_REGISTRY_TOKEN_ISSUER="$daemon_url/v1/registry/token"
export HEPHAESTUS_REGISTRY_SERVICE="$registry_authority"
export HEPHAESTUS_REGISTRY_PRIVATE_ORIGIN="$registry_origin/"
export HEPHAESTUS_REGISTRY_TOKEN_KEY_ID='repository-oci-e2e-v1'
export HEPHAESTUS_REGISTRY_TOKEN_LIFETIME_SECONDS='300'
export HEPHAESTUS_REGISTRY_NOTIFICATION_CALLBACK_TOKEN_FILE="$fixture_root/registry-notification-token"
export HEPHAESTUS_REGISTRY_RECONCILIATION_INTERVAL_MILLISECONDS='250'
export HEPHAESTUS_REGISTRY_CREDENTIAL_ROOT="$fixture_root/registry-credentials"
export HEPHAESTUS_ROOT_IMAGE_MANIFEST="$root_image_manifest"
export HEPHAESTUS_VM_BACKEND='fixture'
export HEPHAESTUS_HOST_ID='repository-oci-e2e'
export HEPHAESTUS_MKFS_EXT4="$(command -v mkfs.ext4)"
export HEPHAESTUS_RUNTIME_POLICY_VERSION='repository-oci-e2e/v1'
export HEPHAESTUS_RUNTIME_MAX_VCPUS='2'
export HEPHAESTUS_RUNTIME_MAX_MEMORY_MIB='1024'
export HEPHAESTUS_RUNTIME_ALLOW_BROKER_ONLY='true'
export HEPHAESTUS_RUNTIME_ALLOW_EGRESS='false'
export HEPHAESTUS_OCI_BUILDER_ROOTFS_ROOT="$fixture_root/builder-rootfs"
export HEPHAESTUS_OCI_BUILDER_BASE_LAYOUT_MANIFEST="$fixture_root/base-layouts.json"
export HEPHAESTUS_OCI_BUILDER_CHECKOUT_ROOT="$fixture_root/builder-checkouts"
export HEPHAESTUS_OCI_BUILDER_OUTPUT_ROOT="$fixture_root/builder-output"
export HEPHAESTUS_OCI_BUILDER_ROOT_MANIFEST="$fixture_root/repository-builder-roots.json"
export HEPHAESTUS_OCI_BUILDER_POLL_MILLISECONDS='100'
export HEPHAESTUS_BUILDAH_BINARY="$fixture_root/test-tools/buildah"
export HEPHAESTUS_TRIVY_BINARY="$trivy_fixture"
export HEPHAESTUS_SYFT_BINARY="$syft_fixture"
export HEPHAESTUS_SYFT_CONFIG="$fixture_root/syft.yaml"
export HEPHAESTUS_UMOCI_BINARY="$umoci_binary"
export HEPHAESTUS_SKOPEO="$fixture_root/test-tools/skopeo"
export HEPHAESTUS_ORAS="$fixture_root/test-tools/oras"
export HEPHAESTUS_GIT_BINARY="$git_binary"
export HEPHAESTUS_TAR_BINARY="$tar_binary"
export RUST_LOG='hephaestus_app=debug,oci_builder_worker=debug,registry_publisher=debug'

"$repository_root/target/debug/hephaestusd" >"$fixture_root/daemon.log" 2>&1 &
daemon_pid=$!
wait_for_url "$daemon_url/healthz" "$fixture_root/daemon.log"

readonly git_token="$(curl --fail --silent "$oidc_url/test/git-token")"
mkdir "$fixture_root/source"
git init --initial-branch=main "$fixture_root/source" >/dev/null
git -C "$fixture_root/source" config user.name 'Repository OCI E2E'
git -C "$fixture_root/source" config user.email 'oci-e2e@example.invalid'
mkdir -p "$fixture_root/source/builders/repository-e2e"
printf '%s\n' 'repository builder reached materialized execution' \
    >"$fixture_root/source/builders/repository-e2e/marker.txt"
cat >"$fixture_root/source/builders/repository-e2e/Dockerfile" <<'DOCKERFILE'
FROM heph-base
COPY marker.txt /etc/hephaestus-repository-builder-e2e
DOCKERFILE
cat >"$fixture_root/source/heph.builders.toml" <<'MANIFEST'
version = 1

[[builders]]
key = "repository-e2e"
display_name = "Repository OCI E2E"

[builders.oci]
dockerfile = "builders/repository-e2e/Dockerfile"
context = "builders/repository-e2e"
base = "repository-e2e-base"
MANIFEST
git -C "$fixture_root/source" add .
git -C "$fixture_root/source" commit -m 'Add repository OCI builder' >/dev/null
readonly source_commit="$(git -C "$fixture_root/source" rev-parse HEAD)"
git -C "$fixture_root/source" remote add origin "$daemon_url/$repository_id"
git -C "$fixture_root/source" -c "http.extraHeader=Authorization: Bearer $git_token" \
    push origin HEAD:refs/heads/main >/dev/null

builder_id=''
for _attempt in {1..900}; do
    builder_id=$(sql_value "SELECT id FROM project_builder_definitions WHERE source_repository_id = '$repository_id' AND source_revision = '$source_commit' AND key = 'repository-e2e'")
    state=$(sql_value "SELECT definition.status || ':' || COALESCE(materialization.state, 'none') FROM project_builder_definitions definition LEFT JOIN project_builder_root_materialization_jobs materialization ON materialization.builder_id = definition.id AND materialization.worker_name = 'oci-rootfs-repository-oci-e2e' WHERE definition.id = NULLIF('$builder_id', '')::uuid")
    [[ $state == 'ready:materialized' ]] && break
    [[ $state == failed:* ]] && {
        printf 'repository builder failed: %s\n' "$state" >&2
        exit 1
    }
    sleep 0.2
done
[[ -n $builder_id && $state == 'ready:materialized' ]]

readonly repository_path="projects/$project_id/repository-builders/$builder_id"
readonly image_reference="$(sql_value "SELECT oci_image_reference FROM project_builder_definitions WHERE id = '$builder_id'")"
readonly image_digest="$(sql_value "SELECT oci_image_digest FROM project_builder_definitions WHERE id = '$builder_id'")"
readonly root_path="$(sql_value "SELECT root_path FROM project_builder_root_materialization_jobs WHERE builder_id = '$builder_id' AND state = 'materialized'")"
[[ $image_reference == "$registry_authority/$repository_path@$image_digest" ]]
[[ -f $root_path/etc/hephaestus-repository-builder-e2e ]]
grep -Fxq 'repository builder reached materialized execution' \
    "$root_path/etc/hephaestus-repository-builder-e2e"

readonly publication_state="$(sql_value "SELECT publication.state || ':' || count(evidence.id)::text FROM registry_publications publication JOIN registry_namespaces namespace ON namespace.id = publication.namespace_id LEFT JOIN registry_publication_evidence evidence ON evidence.publication_id = publication.id WHERE namespace.repository_path = '$repository_path' GROUP BY publication.state")"
[[ $publication_state == 'approved:3' ]]

readonly pull_token="$(curl --fail --silent --get \
    --header "Authorization: Bearer $git_token" \
    --data-urlencode "service=$registry_authority" \
    --data-urlencode "scope=repository:$repository_path:pull" \
    "$daemon_url/v1/registry/token" | jq -r '.token')"
"$skopeo_binary" inspect --registry-token "$pull_token" --tls-verify=false \
    "docker://$image_reference" >/dev/null

readonly denied_path="projects/00000000-0000-4000-8000-000000000099/repository-builders/$builder_id"
readonly denied_token="$(curl --fail --silent --get \
    --header "Authorization: Bearer $git_token" \
    --data-urlencode "service=$registry_authority" \
    --data-urlencode "scope=repository:$denied_path:pull" \
    "$daemon_url/v1/registry/token" | jq -r '.token')"
readonly denied_payload="$(printf '%s' "$denied_token" | cut -d. -f2 | tr '_-' '/+' | awk '{ padding=(4-length($0)%4)%4; printf "%s", $0; while (padding--) printf "=" }' | openssl base64 -d -A)"
[[ $(jq '.access | length' <<<"$denied_payload") == 0 ]]
readonly denied_status="$(curl --silent --output /dev/null --write-out '%{http_code}' \
    --header "Authorization: Bearer $denied_token" \
    "$registry_origin/v2/$denied_path/manifests/$image_digest")"
[[ $denied_status == 401 || $denied_status == 403 ]]

tar --create --file "$fixture_root/materialized-rootfs.tar" --directory "$root_path" .
"$container_engine" import "$fixture_root/materialized-rootfs.tar" "$runtime_image" >/dev/null
"$container_engine" run --rm --network none "$runtime_image" /bin/sh -eu -c \
    'test "$(cat /etc/hephaestus-repository-builder-e2e)" = "repository builder reached materialized execution"'

printf 'Repository OCI builder E2E passed: commit discovery; isolated build; real Zot publication/referrers; durable approval; materialization/execution; cross-project denial.\n'
