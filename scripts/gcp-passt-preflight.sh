#!/usr/bin/env bash
# Run the exact passt preflight used by the libkrun worker before its kernel
# smoke.  This intentionally exercises only passt's Unix control socket and
# one forge-owned accepted stream.
set -Eeuo pipefail
umask 077

readonly probe_root="${HEPHAESTUS_LIBKRUN_TMP_ROOT:-/tmp/hephaestus-libkrun}"
readonly passt_binary="${HEPH_GCP_PASST_PREFLIGHT_BINARY:-/usr/bin/passt}"
readonly probe_user='forge'
readonly probe_uid=10001
readonly probe_gid=10001
readonly wait_seconds=5
readonly accept_wait_seconds=3

probe_fixture_root=''
probe_dir=''
probe_pid=''
client_pid=''
socket_path=''
pid_path=''
log_path=''
stderr_path=''
stdout_path=''
probe_apparmor_profile=''

require_command() {
    command -v "$1" >/dev/null 2>&1 || {
        echo "gcp-passt-preflight: required command is missing: $1" >&2
        exit 2
    }
}

stop_probe() {
    if [[ -n "$client_pid" ]] && kill -0 "$client_pid" 2>/dev/null; then
        kill -TERM "$client_pid" 2>/dev/null || true
        wait "$client_pid" 2>/dev/null || true
    fi
    client_pid=''
    if [[ -n "$probe_pid" ]] && kill -0 "$probe_pid" 2>/dev/null; then
        kill -TERM "$probe_pid" 2>/dev/null || true
        for _ in {1..20}; do
            kill -0 "$probe_pid" 2>/dev/null || break
            sleep 0.1
        done
        kill -KILL "$probe_pid" 2>/dev/null || true
    fi
}

cleanup() {
    local status=$?
    trap - EXIT INT TERM
    stop_probe
    [[ -n "$probe_pid" ]] && wait "$probe_pid" 2>/dev/null || true
    if [[ -n "$probe_fixture_root" ]]; then
        rm -rf -- "$probe_fixture_root"
    fi
    exit "$status"
}

on_signal() {
    local signal="$1"
    local status=130
    [[ "$signal" == TERM ]] && status=143
    trap - EXIT INT TERM
    stop_probe
    [[ -n "$probe_pid" ]] && wait "$probe_pid" 2>/dev/null || true
    [[ -n "$probe_fixture_root" ]] && rm -rf -- "$probe_fixture_root"
    exit "$status"
}

bounded_failure_diagnostics() {
    {
        echo '--- gcp-passt-preflight diagnostics ---'
        echo "binary=$passt_binary"
        dpkg-query -W -f='package=${Package} version=${Version} status=${Status}\n' passt 2>&1 || true
        realpath -e "$passt_binary" 2>&1 || true
        id "$probe_user" 2>&1 || true
        for path in "$probe_root" "$probe_fixture_root" "$probe_dir" "$socket_path" "$pid_path" "$log_path"; do
            if [[ -e "$path" || -L "$path" ]]; then
                stat -c 'stat=%A owner=%U:%G mode=%a path=%n' -- "$path" 2>&1 || true
                namei -om -- "$path" 2>&1 || true
            fi
        done
        echo '--- passt stdout (first 80 lines) ---'
        sed -n '1,80p' "$stdout_path" 2>/dev/null || true
        echo '--- passt stderr (first 120 lines) ---'
        sed -n '1,120p' "$stderr_path" 2>/dev/null || true
        echo '--- passt log (first 160 lines) ---'
        sed -n '1,160p' "$log_path" 2>/dev/null || true
        echo '--- runuser launcher AppArmor profile (passthrough child not inferred) ---'
        if [[ -n "$probe_apparmor_profile" ]]; then
            printf '%s\n' "$probe_apparmor_profile"
        else
            echo 'unavailable (runuser launcher exited before capture)'
        fi
        echo '--- passt AppArmor status ---'
        if command -v aa-status >/dev/null 2>&1; then
            aa-status --enabled 2>&1 || true
            aa-status --profiled 2>/dev/null | grep -Ei '(^|/)(usr\.bin\.)?passt([[:space:]]|$)' || true
        else
            echo 'aa-status unavailable'
        fi
        profile_list="$(dpkg-query -L passt 2>/dev/null \
            | grep -E '^/etc/apparmor\.d/' | head -20 || true)"
        if [[ -n "$profile_list" ]]; then
            while IFS= read -r profile; do
                if [[ -f "$profile" ]]; then
                    echo "--- $profile (first 240 lines) ---"
                    sed -n '1,240p' "$profile" 2>&1 || true
                fi
            done <<<"$profile_list"
        else
            echo 'no AppArmor profile files listed by dpkg-query -L passt'
        fi
        echo '--- bounded kernel AppArmor audit ---'
        journalctl -k -b --no-pager -n 300 2>/dev/null \
            | grep -Ei 'apparmor=.*(passt|/usr/bin/passt)|profile=.*(passt|/usr/bin/passt)' \
            | tail -40 || true
        echo '--- end diagnostics ---'
    } >&2
}

require_command getent
require_command id
require_command install
require_command namei
require_command realpath
require_command runuser
require_command sed
require_command stat
require_command timeout
require_command tr
require_command chown
require_command grep
require_command tail
require_command dpkg-query
require_command journalctl
require_command python3

[[ "$(id -u)" == 0 ]] || {
    echo 'gcp-passt-preflight: run as root so the forge-owned runtime boundary is checked exactly' >&2
    exit 2
}
[[ -x "$passt_binary" ]] || {
    echo "gcp-passt-preflight: passt is not executable: $passt_binary" >&2
    exit 2
}
getent passwd "$probe_user" >/dev/null || {
    echo "gcp-passt-preflight: required user is missing: $probe_user" >&2
    exit 2
}
actual_uid="$(id -u "$probe_user")"
actual_gid="$(id -g "$probe_user")"
[[ "$actual_uid" == "$probe_uid" && "$actual_gid" == "$probe_gid" ]] || {
    echo "gcp-passt-preflight: $probe_user must be uid:gid ${probe_uid}:${probe_gid}, got ${actual_uid}:${actual_gid}" >&2
    exit 2
}
[[ -d "$probe_root" ]] || {
    echo "gcp-passt-preflight: runtime root is missing: $probe_root" >&2
    exit 2
}
root_mode="$(stat -c '%a' -- "$probe_root")"
root_owner="$(stat -c '%u:%g' -- "$probe_root")"
[[ "$root_mode" == 700 && "$root_owner" == "${probe_uid}:${probe_gid}" ]] || {
    echo "gcp-passt-preflight: runtime root must be mode 700 and owned by ${probe_uid}:${probe_gid}; got mode=$root_mode owner=$root_owner" >&2
    exit 2
}

trap 'on_signal INT' INT
trap 'on_signal TERM' TERM
trap cleanup EXIT

probe_fixture_root="$(mktemp -d "$probe_root/h.XXXXXX")"
chown "$probe_uid:$probe_gid" "$probe_fixture_root"
chmod 700 "$probe_fixture_root"
install -d -o "$probe_uid" -g "$probe_gid" -m 700 \
    "$probe_fixture_root/runtime" "$probe_fixture_root/runtime/integration-primary"
probe_dir="$probe_fixture_root/runtime/integration-primary"
socket_path="$probe_dir/passt.sock"
pid_path="$probe_dir/passt.pid"
log_path="$probe_dir/passt.log"
stderr_path="$probe_dir/passt.stderr"
stdout_path="$probe_dir/passt.stdout"
install -o "$probe_uid" -g "$probe_gid" -m 600 /dev/null "$stderr_path"
install -o "$probe_uid" -g "$probe_gid" -m 600 /dev/null "$stdout_path"

# Keep the control-socket arguments aligned with crates/vm-libkrun/src/network.rs.
# Debug is intentional here so the accepted-connection event is observable;
# no forwarded ports means passt cannot expose or connect a test service.
runuser -u "$probe_user" -- env HOME=/home/forge "$passt_binary" \
    --foreground --one-off --debug \
    --socket "$socket_path" \
    --pid "$pid_path" \
    --log-file "$log_path" \
    --log-size 1048576 \
    --runas "$probe_uid:$probe_gid" \
    --udp-ports none \
    --tcp-ports none \
    >"$stdout_path" 2>"$stderr_path" &
probe_pid=$!
if [[ -r "/proc/${probe_pid}/attr/current" ]]; then
    probe_apparmor_profile="$(tr -d '\0' <"/proc/${probe_pid}/attr/current")" || true
fi

socket_seen=false
deadline=$((SECONDS + wait_seconds))
while (( SECONDS < deadline )); do
    if [[ -S "$socket_path" ]]; then
        socket_seen=true
        break
    fi
    if ! kill -0 "$probe_pid" 2>/dev/null; then
        break
    fi
    sleep 0.1
done

if [[ "$socket_seen" == true ]]; then
    # A socket appearing only proves that passt created its listener.  Keep a
    # real forge client connected briefly so the debug log must show that
    # accept4() completed; this catches the cloud-only EACCES failure before
    # the expensive kernel build.
    set +e
    runuser -u "$probe_user" -- python3 - "$socket_path" <<'PY' &
import socket
import sys
import time

with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as client:
    client.settimeout(2.0)
    client.connect(sys.argv[1])
    time.sleep(1.0)
PY
    client_pid=$!
    set -e

    accepted=false
    accept_error=false
    server_alive_while_connected=false
    deadline=$((SECONDS + accept_wait_seconds))
    while (( SECONDS < deadline )); do
        if grep -Fq 'Error accepting tap client:' "$log_path" 2>/dev/null ||
            grep -Fq 'Error accepting tap client:' "$stderr_path" 2>/dev/null; then
            accept_error=true
            break
        fi
        if grep -Fq 'accepted connection from PID' "$log_path" 2>/dev/null ||
            grep -Fq 'accepted connection from PID' "$stderr_path" 2>/dev/null; then
            accepted=true
            if kill -0 "$probe_pid" 2>/dev/null; then
                server_alive_while_connected=true
            fi
            break
        fi
        if ! kill -0 "$probe_pid" 2>/dev/null; then
            break
        fi
        sleep 0.1
    done

    set +e
    wait "$client_pid"
    client_status=$?
    set -e
    client_pid=''

    if [[ "$accept_error" == true || "$accepted" != true ||
        "$server_alive_while_connected" != true || "$client_status" != 0 ]]; then
        stop_probe
        echo "HEPH_GCP_PASST_PREFLIGHT FAIL accept socket=$socket_path accepted=$accepted server_alive_while_connected=$server_alive_while_connected client_status=$client_status" >&2
        bounded_failure_diagnostics
        exit 1
    fi

    stop_probe
    wait "$probe_pid" 2>/dev/null || true
    probe_pid=''
    echo "HEPH_GCP_PASST_PREFLIGHT PASS socket=$socket_path accepted=true"
    exit 0
fi

stop_probe
set +e
wait "$probe_pid"
probe_status=$?
set -e
probe_pid=''
(( probe_status == 0 )) && probe_status=1
echo "HEPH_GCP_PASST_PREFLIGHT FAIL status=$probe_status socket=$socket_path" >&2
bounded_failure_diagnostics
exit "$probe_status"
