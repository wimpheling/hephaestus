//! Actual configured source/build/import/authenticated publication returns.

use std::{fs, path::Path, sync::Arc};

use identity_domain::AuthenticatedIdentity;
use release_service::InstanceExecutionConfiguration;
use run_runtime_local::LocalRunRuntimeConfig;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use vm_trait::VmProvider;
use volume_local::{LocalVolumeConfig, VolumeRootOwner};

use super::{
    build_helpers::{BuildError, CookingBuildContext, PublishedCookingRepository, build_one},
    publication_backend::DrainProof,
    source_variants::{OwnedSourceVariant, VolumeVariant},
};

#[path = "publication_checks.rs"]
mod checks;
pub(super) use checks::{
    PublicationCheckpoint, PublicationPins, require_workspaces_absent, verify_pair_state,
};

// These must be retained from the actual backend before App::build. The stock
// golden RunningHephaestus does not expose provider/root ownership to this API.
pub struct ConfiguredBackend<'a> {
    pub build: &'a CookingBuildContext<'a>,
    pub runtime: &'a LocalRunRuntimeConfig,
    pub local: &'a LocalVolumeConfig,
    pub owner: Arc<VolumeRootOwner>,
    pub provider: Arc<dyn VmProvider>,
    pub configuration: InstanceExecutionConfiguration,
}
impl ConfiguredBackend<'_> {
    pub fn validate(&self) {
        validate_identity(
            self.local,
            &self.owner,
            self.provider.as_ref(),
            &self.configuration,
        );
        assert_eq!(
            fs::canonicalize(&self.runtime.release_artifact_root).unwrap(),
            self.runtime.release_artifact_root
        );
        assert!(
            self.runtime
                .release_artifact_root
                .starts_with(self.build.root)
        );
        assert_eq!(fs::canonicalize(self.build.root).unwrap(), self.build.root);
        assert!(self.runtime.runtime_root.starts_with(self.build.root));
        assert!(
            !self
                .runtime
                .runtime_root
                .starts_with(&self.runtime.release_artifact_root)
        );
        assert!(
            !self
                .runtime
                .release_artifact_root
                .starts_with(&self.runtime.runtime_root)
        );
    }
}

pub fn validate_identity(
    local: &LocalVolumeConfig,
    owner: &VolumeRootOwner,
    provider: &dyn VmProvider,
    configuration: &InstanceExecutionConfiguration,
) {
    owner.validate().unwrap();
    assert_eq!(
        provider.owner_scope().unwrap(),
        *configuration.provider_scope()
    );
    assert_eq!(local.host_id, configuration.provider_scope().host_id());
    assert_eq!(owner.host(), local.host_id);
    assert_eq!(owner.root(), local.volume_root);
    assert_eq!(
        configuration.volume_root(),
        local.volume_root.to_str().unwrap()
    );
    assert_eq!(configuration.volume_namespace(), owner.namespace_id());
    assert_eq!(
        fs::canonicalize(&local.volume_root).unwrap(),
        local.volume_root
    );
}

pub struct PublishedVariant {
    pub published: PublishedCookingRepository,
    pub variant: VolumeVariant,
    pub pool: PgPool,
    pub actor: AuthenticatedIdentity,
    pub runtime: LocalRunRuntimeConfig,
    pub local: LocalVolumeConfig,
    pub owner: Arc<VolumeRootOwner>,
    pub provider: Arc<dyn VmProvider>,
    pub configuration: InstanceExecutionConfiguration,
    project: forge_domain::ProjectId,
    source: OwnedSourceVariant,
}

/// A nonclone token backed only by the sealed, completed owned drain proof.
pub struct StoppedPublicationBackend {
    proof: Arc<DrainProof>,
    index: usize,
}
impl StoppedPublicationBackend {
    pub(super) fn from_drain(proof: DrainProof) -> [Self; 2] {
        let proof = Arc::new(proof);
        [
            Self {
                proof: proof.clone(),
                index: 0,
            },
            Self { proof, index: 1 },
        ]
    }
    pub(super) fn publication(&self) -> &Arc<PublishedVariant> {
        self.proof.publication(self.index)
    }
    pub(super) fn same_handoff(&self, other: &Self) -> bool {
        self.proof.same_handoff(&other.proof)
    }
    pub(super) fn validate_handoff(&self) -> Result<(), super::build_helpers::BuildError> {
        self.proof.validate_handoff(self.index)
    }
}
impl PublishedVariant {
    pub fn validate_identity(&self) {
        validate_identity(
            &self.local,
            &self.owner,
            self.provider.as_ref(),
            &self.configuration,
        );
    }
    pub fn source(&self) -> &Path {
        self.source.path()
    }
    pub const fn project_id(&self) -> forge_domain::ProjectId {
        self.project
    }
}

pub async fn publish(
    backend: &ConfiguredBackend<'_>,
    source: OwnedSourceVariant,
) -> PublishedVariant {
    backend.validate();
    assert!(source.path().starts_with(backend.build.root));
    let published = build_one(
        backend.build,
        source.variant.key(),
        source.path().into(),
        "Actual qualified Local SQLite source variant",
        false,
    )
    .await
    .expect("real configured build and authenticated publication");
    let publication = PublishedVariant {
        published,
        variant: source.variant,
        pool: backend.build.pool.clone(),
        actor: backend.build.identity.actor.clone(),
        runtime: backend.runtime.clone(),
        local: backend.local.clone(),
        owner: backend.owner.clone(),
        provider: backend.provider.clone(),
        configuration: backend.configuration.clone(),
        project: backend.build.project_id,
        source,
    };
    verify(backend, &publication).await;
    publication
}

async fn verify(backend: &ConfiguredBackend<'_>, publication: &PublishedVariant) {
    drop(
        PublicationPins::capture(publication, backend.build.root)
            .await
            .expect("real imported artifact and exact immutable publication provenance"),
    );
    let published = &publication.published;
    let audience = "/hephaestus.release.v1.ReleaseService/GetRelease";
    let response = reqwest::Client::new()
        .post(format!(
            "http://{}{audience}",
            backend.build.running.http_addr()
        ))
        .bearer_auth((backend.build.identity.rpc_token)(audience))
        .header("connect-protocol-version", "1")
        .json(&serde_json::json!({"releaseId":{"value":published.release_id.to_string()}}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .bytes()
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&response).unwrap();
    let release = &body["release"];
    assert_eq!(release["state"], "RELEASE_STATE_PUBLISHED");
    assert_eq!(release["id"]["value"], published.release_id.to_string());
    let exports = release["agents"].as_array().unwrap();
    assert_eq!(exports.len(), 1);
    assert_eq!(exports[0]["agentKey"], "local-sqlite");
    assert_eq!(
        exports[0]["id"]["value"],
        published.release_agent_id.to_string()
    );
    backend.validate();
}

type Artifact = (String, String, i32, uuid::Uuid, Vec<u8>, i64);

async fn publication_rows(p: &PublishedVariant) -> Result<Value, BuildError> {
    let published = &p.published;
    let artifacts: Vec<Artifact> = sqlx::query_as(
        "SELECT path,kind,mode,storage_key,content_hash,size_bytes FROM release_artifacts WHERE release_id=$1 ORDER BY path,id",
    ).bind(published.release_id).fetch_all(&p.pool).await?;
    if artifacts.len() != 1 {
        return Err(held("publication artifact set changed"));
    }
    let artifact = &artifacts[0];
    if (artifact.0.as_str(), artifact.1.as_str(), artifact.2)
        != ("bin/local-sqlite", "executable", 0o555)
    {
        return Err(held("publication artifact declaration differs"));
    }
    let row: Value = sqlx::query_scalar(
        "SELECT jsonb_build_object('build_state',b.state,'execution_state',e.state,'release_state',r.state,'definition',encode(r.build_definition_hash,'hex'),'configuration',encode(r.configuration_hash,'hex'),'manifest',encode(r.manifest_hash,'hex'),'project',repo.project_id,'actor',r.publication_actor_id,'agent',a.id,'key',a.agent_key,'requires_state',a.requires_state,'contract',a.runtime_contract,'contract_hash',encode(a.runtime_contract_hash,'hex'),'stored_config',r.configuration) FROM build_requests b JOIN build_executions e ON e.build_request_id=b.id JOIN releases r ON r.build_request_id=b.id JOIN repositories repo ON repo.id=r.repository_id JOIN release_agents a ON a.release_id=r.id WHERE b.id=$1 AND r.id=$2 AND r.repository_id=$3 AND r.source_commit=$4 AND a.id=$5",
    ).bind(published.build_request_id).bind(published.release_id).bind(published.repository_id.as_uuid())
        .bind(&published.source_commit).bind(published.release_agent_id).fetch_one(&p.pool).await?;
    let version: String = sqlx::query_scalar("SELECT version FROM releases WHERE id=$1")
        .bind(published.release_id)
        .fetch_one(&p.pool)
        .await?;
    if version != published.version {
        return Err(held("immutable release version changed"));
    }
    let slots: Vec<volume_domain::VolumeSlotDeclaration> = row["contract"]
        .get("volume_slots")
        .map(|v| serde_json::from_value(v.clone()))
        .transpose()?
        .unwrap_or_default();
    if published.actor_id != p.actor.user_id
        || row["build_state"] != "succeeded"
        || row["execution_state"] != "drafted"
        || row["release_state"] != "published"
        || row["definition"] != published.build_definition_hash
        || row["configuration"] != published.configuration_hash
        || row["manifest"] != published.manifest_hash
        || row["project"] != p.project_id().to_string()
        || row["actor"] != p.actor.user_id.to_string()
        || row["key"] != "local-sqlite"
        || row["requires_state"] != false
        || row["stored_config"] != serde_json::to_value(&p.source.configuration)?
        || row["contract"]["executable"] != "bin/local-sqlite"
        || row["contract"]["working_directory"] != "bin"
        || row["contract"]["publication_mode"] != "proposal"
        || slots != p.variant.slots()
        || row["contract_hash"]
            != format!(
                "{:x}",
                Sha256::digest(serde_json::to_vec(&row["contract"])?)
            )
    {
        return Err(held("genuine publication provenance differs"));
    }
    Ok(json!({"artifact":artifact,"provenance":row,"version":version}))
}

fn held(reason: &'static str) -> BuildError {
    std::io::Error::other(reason).into()
}
