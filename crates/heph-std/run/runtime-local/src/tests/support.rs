use crate::{GatewayServiceIdentity, LocalGatewayReleaseRuntime};
use run_orchestrator::{RunRuntimeArtifact, RunRuntimeArtifactKind};
use sha2::{Digest, Sha256};
use std::fs;
use uuid::Uuid;

pub(super) fn service_fixture() -> (
    tempfile::TempDir,
    LocalGatewayReleaseRuntime,
    RunRuntimeArtifact,
) {
    let fixture = tempfile::tempdir().expect("fixture");
    let runtime_root = fixture.path().join("runtime");
    let store_root = fixture.path().join("store");
    fs::create_dir(&runtime_root).expect("runtime root");
    fs::create_dir(&store_root).expect("store root");
    let key = Uuid::new_v4();
    let bytes = b"persistent gateway service executable";
    fs::write(store_root.join(key.simple().to_string()), bytes).expect("object");
    let runtime = LocalGatewayReleaseRuntime {
        runtime_root,
        release_artifact_root: store_root,
    };
    let artifact = RunRuntimeArtifact {
        path: String::from("bin/server"),
        kind: RunRuntimeArtifactKind::Executable,
        mode: 0o555,
        content_hash: Sha256::digest(bytes).into(),
        size_bytes: u64::try_from(bytes.len()).expect("length"),
        storage_key: key,
    };
    (fixture, runtime, artifact)
}

pub(super) fn service_identity() -> GatewayServiceIdentity {
    GatewayServiceIdentity {
        instance_id: Uuid::new_v4(),
        gateway_id: Uuid::new_v4(),
        revision_id: Uuid::new_v4(),
    }
}
