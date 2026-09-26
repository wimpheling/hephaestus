use crate::{
    GatewayServiceArtifact, GatewayServiceArtifactKind, GatewayServiceIdentity,
    LocalGatewayReleaseMaterializer, LocalRunRuntimeConfig, LocalRunRuntimeManager,
};
use async_trait::async_trait;
use gateway_domain::GatewayServiceMaterializer;
use gateway_edge::GatewayEdgeError;
use heph_run::{Run, RunRuntimeCatalog, RunRuntimeCatalogError, RunRuntimeInput};
use runtime_types::RunId;
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::sync::Arc;
use uuid::Uuid;

struct EmptyRunRuntimeCatalog;

#[async_trait]
impl RunRuntimeCatalog for EmptyRunRuntimeCatalog {
    async fn load_runtime(&self, _run: &Run) -> Result<RunRuntimeInput, RunRuntimeCatalogError> {
        Err(RunRuntimeCatalogError::Unavailable)
    }

    async fn run_is_live(&self, _run_id: RunId) -> Result<bool, RunRuntimeCatalogError> {
        Ok(false)
    }
}

#[test]
fn app_materializer_bridges_service_identity_and_exact_cleanup() {
    let fixture = tempfile::tempdir().expect("temporary materializer roots");
    let runtime_root = fixture.path().join("runtime");
    let store_root = fixture.path().join("store");
    let key = Uuid::new_v4();
    let bytes = b"persistent service executable";
    std::fs::create_dir(&store_root).expect("store root");
    std::fs::write(store_root.join(key.simple().to_string()), bytes).expect("store object");
    let manager = LocalRunRuntimeManager::initialize(
        Arc::new(EmptyRunRuntimeCatalog),
        LocalRunRuntimeConfig {
            runtime_root: runtime_root.clone(),
            release_artifact_root: store_root,
        },
    )
    .expect("initialize runtime");
    let materializer = LocalGatewayReleaseMaterializer {
        runtime: manager.gateway_release_runtime(),
    };
    let identity = GatewayServiceIdentity {
        instance_id: Uuid::new_v4(),
        gateway_id: Uuid::new_v4(),
        revision_id: Uuid::new_v4(),
    };
    let mounts = materializer
        .prepare_service(
            identity,
            &[GatewayServiceArtifact {
                path: String::from("bin/server"),
                kind: GatewayServiceArtifactKind::Executable,
                mode: 0o555,
                content_hash: Sha256::digest(bytes).into(),
                size_bytes: u64::try_from(bytes.len()).expect("artifact length"),
                storage_key: key,
            }],
            &serde_json::json!({"port": 8080}),
        )
        .expect("materialize service");
    assert_eq!(mounts.len(), 2);
    assert!(mounts.iter().all(|mount| mount.read_only));
    assert_eq!(mounts[0].guest_path, PathBuf::from("/release"));
    assert_eq!(mounts[1].guest_path, PathBuf::from("/run/hephaestus"));

    let wrong_identity = GatewayServiceIdentity {
        gateway_id: Uuid::new_v4(),
        ..identity
    };
    assert!(matches!(
        materializer.destroy_service(wrong_identity),
        Err(GatewayEdgeError::HandlerUnavailable)
    ));
    assert!(runtime_service_path(&runtime_root, identity.instance_id).exists());
    materializer
        .destroy_service(identity)
        .expect("destroy exact service identity");
    assert!(!runtime_service_path(&runtime_root, identity.instance_id).exists());
}

fn runtime_service_path(runtime_root: &std::path::Path, instance_id: Uuid) -> PathBuf {
    runtime_root
        .join("gateway-services")
        .join(instance_id.to_string())
}
