//! Service materializer and reconfiguration helpers.

use authz_postgres::PostgresMelangeAuthorizer;
use gateway_domain::{
    GatewayEdgeError, GatewayServiceArtifact, GatewayServiceIdentity, GatewayServiceMaterializer,
};
use gateway_postgres::{
    ConfigureGatewayRequest, InstallGatewayManifest, PostgresGatewayInstaller,
    PostgresGatewayManagement,
};
use identity_domain::{AuthenticatedIdentity, RequestId};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use uuid::Uuid;
use vm_trait::VmMount;

#[derive(Default)]
pub struct RecordingServiceMaterializer {
    pub records: Mutex<Vec<MaterializerRecord>>,
    pub invalid_mounts: bool,
    pub destroyed: Mutex<Vec<GatewayServiceIdentity>>,
}

pub struct MaterializerRecord {
    pub identity: GatewayServiceIdentity,
    pub artifact_paths: Vec<String>,
    pub parameters: serde_json::Value,
}

impl GatewayServiceMaterializer for RecordingServiceMaterializer {
    fn prepare_service(
        &self,
        identity: GatewayServiceIdentity,
        artifacts: &[GatewayServiceArtifact],
        parameters: &serde_json::Value,
    ) -> Result<Vec<VmMount>, GatewayEdgeError> {
        self.records
            .lock()
            .expect("materializer records")
            .push(MaterializerRecord {
                identity,
                artifact_paths: artifacts
                    .iter()
                    .map(|artifact| artifact.path.clone())
                    .collect(),
                parameters: parameters.clone(),
            });
        let mounts = vec![
            VmMount {
                tag: String::from("service-release"),
                host_path: PathBuf::from("/tmp/service-release"),
                guest_path: PathBuf::from("/release"),
                read_only: true,
            },
            VmMount {
                tag: String::from("service-control"),
                host_path: PathBuf::from("/tmp/service-control"),
                guest_path: PathBuf::from("/run/hephaestus"),
                read_only: true,
            },
        ];
        if self.invalid_mounts {
            return Ok(mounts.into_iter().take(1).collect());
        }
        Ok(mounts)
    }

    fn destroy_service(&self, identity: GatewayServiceIdentity) -> Result<(), GatewayEdgeError> {
        self.destroyed
            .lock()
            .expect("destroyed identities")
            .push(identity);
        Ok(())
    }
}

pub async fn assert_reconfigure_after_reinstallation_creates_fresh_authority(
    pool: &sqlx::PgPool,
    installer: &PostgresGatewayInstaller,
    owner: &AuthenticatedIdentity,
    manifest: InstallGatewayManifest,
    gateway: gateway_postgres::InstalledGateway,
) {
    let management =
        PostgresGatewayManagement::new(pool.clone(), Arc::new(PostgresMelangeAuthorizer));
    let configure = || ConfigureGatewayRequest {
        gateway_id: gateway.gateway_id.as_uuid(),
        expected_revision_id: gateway.revision_id.as_uuid(),
        parameters: BTreeMap::new(),
        secret_selections: Vec::new(),
    };
    let original_identity = owner.clone().with_idempotency_id(RequestId::new());
    let configured = management
        .configure(&original_identity, configure())
        .await
        .expect("first configuration before reinstall");
    installer
        .install(
            &owner.clone().with_idempotency_id(RequestId::new()),
            manifest,
        )
        .await
        .expect("reinstall reactivates original declaration");
    let fresh_identity = owner.clone().with_idempotency_id(RequestId::new());
    let fresh = management
        .configure(&fresh_identity, configure())
        .await
        .expect("fresh configuration after reinstall");
    assert_ne!(fresh.revision_id, configured.revision_id);
    let replay = management
        .configure(&original_identity, configure())
        .await
        .expect("original successful command remains replayable");
    assert_eq!(replay.revision_id, configured.revision_id);
    let (active, revisions, bindings): (Uuid, i64, i64) = sqlx::query_as(
        "SELECT active_revision_id,
             (SELECT count(*) FROM gateway_revisions WHERE gateway_id = $1),
             (SELECT count(*) FROM gateway_mailbox_bindings WHERE gateway_revision_id = $2)
         FROM gateways WHERE id = $1",
    )
    .bind(gateway.gateway_id.as_uuid())
    .bind(fresh.revision_id)
    .fetch_one(pool)
    .await
    .expect("fresh revision and replay preserve independent authority");
    assert_eq!(
        active, fresh.revision_id,
        "old command replay must not reactivate history"
    );
    // The original release, the later release, and both configured revisions
    // are the only immutable revisions created by the installation fixture.
    assert_eq!(revisions, 4);
    assert_eq!(bindings, 0);
}
