use crate::LocalGatewayReleaseRuntime;
use run_orchestrator::{RunRuntimeArtifact, RunRuntimeArtifactKind};
use sha2::{Digest, Sha256};
use std::{fs, os::unix::fs::PermissionsExt};
use uuid::Uuid;

#[test]
fn gateway_runtime_materializes_and_removes_the_exact_release_tree() {
    let fixture = tempfile::tempdir().expect("fixture");
    let runtime_root = fixture.path().join("runtime");
    let store_root = fixture.path().join("store");
    fs::create_dir(&runtime_root).expect("runtime root");
    fs::create_dir(&store_root).expect("store root");
    let key = Uuid::new_v4();
    let bytes = b"gateway release executable";
    fs::write(store_root.join(key.simple().to_string()), bytes).expect("object");
    let invocation = Uuid::new_v4();
    let runtime = LocalGatewayReleaseRuntime {
        runtime_root: runtime_root.clone(),
        release_artifact_root: store_root,
    };

    let mounts = runtime
            .prepare(
                invocation,
                &[RunRuntimeArtifact {
                    path: String::from("bin/handler"),
                    kind: RunRuntimeArtifactKind::Executable,
                    mode: 0o555,
                    content_hash: Sha256::digest(bytes).into(),
                    size_bytes: u64::try_from(bytes.len()).expect("length"),
                    storage_key: key,
                }],
                &serde_json::json!({"inbound_placeholder":"public-identifier","alice_provider_id":1001}),
            )
            .expect("materialize gateway release");
    let mount = &mounts[0];
    assert_eq!(mounts.len(), 2);
    let control = &mounts[1];
    assert_eq!(
        control.guest_path,
        std::path::PathBuf::from("/run/hephaestus")
    );
    assert!(control.read_only);
    let parameters: serde_json::Value = serde_json::from_slice(
        &fs::read(control.host_path.join("parameters.json")).expect("sealed parameters"),
    )
    .expect("parameter JSON");
    assert_eq!(parameters["alice_provider_id"], 1001);
    assert_eq!(parameters["inbound_placeholder"], "public-identifier");
    assert_eq!(
        fs::metadata(control.host_path.join("parameters.json"))
            .expect("parameter mode")
            .permissions()
            .mode()
            & 0o222,
        0,
        "ordinary parameters are sealed before guest launch"
    );
    assert_eq!(mount.guest_path, std::path::PathBuf::from("/release"));
    assert!(mount.read_only);
    assert_eq!(
        fs::read(mount.host_path.join("bin/handler")).expect("materialized executable"),
        bytes
    );

    runtime
        .destroy(invocation)
        .expect("destroy gateway release");
    assert!(
        !runtime_root
            .join("gateways")
            .join(invocation.to_string())
            .exists()
    );
}
