#![allow(unused_imports)]
use super::*;
use serde_json::Value as JsonValue;
use sqlx::PgPool;
use std::{
    fs, io,
    os::unix::fs as unix_fs,
    path::{Path, PathBuf},
};
use uuid::Uuid;
pub(crate) const DENIAL_PROBE_CHECKS: [&str; 10] = [
    "source_checkout_absent",
    "model_authorized_control",
    "source_repository_read_denied",
    "source_repository_push_denied",
    "other_repository_read_denied",
    "other_repository_push_denied",
    "prohibited_path_push_denied",
    "model_destination_denied",
    "model_rule_denied",
    "runtime_git_credential_absent_from_surfaces",
];

pub(crate) async fn accepted_receive_count(pool: &PgPool, repository_id: Uuid) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*)
           FROM git_receives
          WHERE repository_id = $1 AND status = 'accepted'",
    )
    .bind(repository_id)
    .fetch_one(pool)
    .await
    .expect("count accepted denial-probe Git receives")
}

pub(crate) async fn canonical_main_ref(pool: &PgPool, repository_id: Uuid) -> Option<String> {
    sqlx::query_scalar(
        "SELECT commit_sha FROM git_refs
          WHERE repository_id = $1 AND git_ref = 'refs/heads/main'",
    )
    .bind(repository_id)
    .fetch_optional(pool)
    .await
    .expect("read denial-probe canonical main ref")
}

pub(crate) async fn assert_denial_probe_output(pool: &PgPool, run_id: Uuid) {
    let chunks: Vec<JsonValue> = sqlx::query_scalar(
        "SELECT payload->'bytes'
           FROM run_events
          WHERE run_id = $1 AND event_type = 'vm.log'
          ORDER BY sequence",
    )
    .bind(run_id)
    .fetch_all(pool)
    .await
    .expect("read denial-probe VM output");
    let mut bytes = Vec::new();
    for chunk in chunks {
        if let Some(values) = chunk.as_array() {
            bytes.extend(
                values
                    .iter()
                    .filter_map(JsonValue::as_u64)
                    .filter_map(|value| u8::try_from(value).ok()),
            );
        }
    }
    let output = String::from_utf8_lossy(&bytes);
    for check in DENIAL_PROBE_CHECKS {
        let marker = format!("HEPH_SESSION_CHAT_DENIAL_PROBE check={check} status=passed");
        assert_eq!(
            output.matches(&marker).count(),
            1,
            "denial probe must emit one passed marker for {check}"
        );
    }
    assert_eq!(
        output
            .matches("HEPH_SESSION_CHAT_DENIAL_PROBE check=")
            .count(),
        DENIAL_PROBE_CHECKS.len(),
        "denial probe must emit exactly ten fixed markers"
    );
}

fn copy_denial_probe_source(source: &Path, destination: &Path) -> io::Result<()> {
    if !source.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "session-chat denial source is not a directory",
        ));
    }
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let name = entry.file_name();
        if name == ".git" || name == "target" || name == "__pycache__" {
            continue;
        }
        let source_path = entry.path();
        let destination_path = destination.join(&name);
        let metadata = fs::symlink_metadata(&source_path)?;
        if metadata.is_dir() {
            copy_denial_probe_source(&source_path, &destination_path)?;
        } else if metadata.file_type().is_symlink() {
            unix_fs::symlink(fs::read_link(source_path)?, destination_path)?;
        } else if metadata.is_file() {
            fs::copy(source_path, destination_path)?;
        } else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "session-chat denial source contains unsupported file type",
            ));
        }
    }
    Ok(())
}

// Keep the generated guest entrypoint beside its source staging so the release
// fixture remains auditable as one bounded denial-probe setup.
#[allow(clippy::too_many_lines)]
pub(crate) fn prepare_denial_probe_source(source_root: &Path, root: &Path) -> PathBuf {
    let destination = root.join(format!("session-chat-denial-source-{}", Uuid::new_v4()));
    copy_denial_probe_source(source_root, &destination).expect("copy denial-probe source");
    let denied_probe_destination = destination.join("tests");
    fs::create_dir_all(&denied_probe_destination).expect("create denial-probe package");
    fs::copy(
        source_root.join("tests/denied_probe.py"),
        denied_probe_destination.join("denied_probe.py"),
    )
    .expect("stage denial-probe module");
    fs::write(
        destination.join("denied_probe_entry.py"),
        r#"#!/usr/local/bin/python3
import importlib.util
import json
from pathlib import Path
import sys

import agent

_PROBE_SPEC = importlib.util.spec_from_file_location(
    "session_chat_denied_probe", Path(__file__).with_name("tests") / "denied_probe.py"
)
if _PROBE_SPEC is None or _PROBE_SPEC.loader is None:
    raise SystemExit(1)
denied_probe = importlib.util.module_from_spec(_PROBE_SPEC)
_PROBE_SPEC.loader.exec_module(denied_probe)

def main():
    context = json.loads((Path("/run/hephaestus") / "context.json").read_text(encoding="utf-8"))
    parameters = json.loads((Path("/run/hephaestus") / "parameters.json").read_text(encoding="utf-8"))
    target = context.get("repository_id")
    source = parameters.get("denial_source_repository_id")
    other = parameters.get("denial_other_repository_id")
    if not all(isinstance(value, str) for value in (target, source, other)):
        return 1
    try:
        observer = denied_probe.runtime_credential_observer(target)
    except Exception:  # noqa: BLE001 - output must stay fixed and credential-free.
        observer = None
    try:
        results = denied_probe._run(
            target,
            source,
            other,
            Path("/workspace/git"),
            observer,
        )
    except Exception:
        results = {check: False for check in denied_probe.CHECKS}
    final_check = denied_probe.FINAL_CHECK
    for check in denied_probe.CHECKS:
        if check == final_check:
            continue
        status = "passed" if results.get(check) is True else "failed"
        print(
            f"HEPH_SESSION_CHAT_DENIAL_PROBE check={check} status={status}",
            file=sys.stderr,
            flush=True,
        )
    failed = [
        index for index, check in enumerate(denied_probe.CHECKS)
        if check != final_check and results.get(check) is not True
    ]
    if failed:
        return 40 + failed[0]
    try:
        with denied_probe.observe_agent_git(observer):
            agent.run_once()
    except Exception:  # noqa: BLE001 - output must stay fixed and credential-free.
        results[final_check] = False
    else:
        try:
            results[final_check] = observer is not None and observer.scan_config(Path("/workspace/git"))
        except Exception:  # noqa: BLE001 - output must stay fixed and credential-free.
            results[final_check] = False
    status = "passed" if results[final_check] is True else "failed"
    print(
        f"HEPH_SESSION_CHAT_DENIAL_PROBE check={final_check} status={status}",
        file=sys.stderr,
        flush=True,
    )
    return 0 if results[final_check] is True else 49


if __name__ == "__main__":
    raise SystemExit(main())
"#,
    )
    .expect("write denial-probe entrypoint");
    let build_script = destination.join("build.sh");
    let mut build = fs::read_to_string(&build_script).expect("read session-chat build script");
    build.push_str(
        "\npython3 - <<'PY'\nfrom pathlib import Path\nfor source in (\"tests/denied_probe.py\", \"denied_probe_entry.py\"):\n    path = Path(source)\n    compile(path.read_text(encoding=\"utf-8\"), str(path), \"exec\", dont_inherit=True)\nPY\ninstall -m 0755 denied_probe_entry.py \"$output_root/bin/session-chat/session-chat-agent\"\ninstall -m 0644 agent.py \"$output_root/bin/session-chat/agent.py\"\ninstall -d -m 0755 \"$output_root/bin/session-chat/tests\"\ninstall -m 0644 tests/denied_probe.py \"$output_root/bin/session-chat/tests/denied_probe.py\"\n",
    );
    let agent_config = destination.join("agent.toml");
    let mut config = fs::read_to_string(&agent_config).expect("read denial-probe agent config");
    config.push_str(
        r#"

[[parameters]]
name = "denial_source_repository_id"
type = "string"
minimum_length = 36
maximum_length = 36
required = true

[[parameters]]
name = "denial_other_repository_id"
type = "string"
minimum_length = 36
maximum_length = 36
required = true
"#,
    );
    fs::write(agent_config, config).expect("write denial-probe agent config");
    fs::write(build_script, build).expect("write denial-probe build script");
    destination
}
