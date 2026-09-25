use run_orchestrator::RunRuntimeError;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

use crate::{GATEWAY_SERVICE_SCHEMA_VERSION, filesystem::runtime_error};

/// Filesystem roots used for per-run runtime materialization.
#[derive(Debug, Clone)]
pub struct LocalRunRuntimeConfig {
    /// Transient administrator-owned root containing active run trees.
    pub runtime_root: PathBuf,
    /// Durable administrator-owned opaque release-object store.
    pub release_artifact_root: PathBuf,
}

/// Local exact-runtime lifecycle manager over a provider-neutral catalog.
#[derive(Clone)]
pub struct LocalRunRuntimeManager {
    pub(crate) catalog: std::sync::Arc<dyn run_orchestrator::RunRuntimeCatalog>,
    pub(crate) config: LocalRunRuntimeConfig,
}

/// Local immutable release tree used by a one-shot gateway invocation.
///
/// Gateway invocations share the same verified object store and artifact
/// materialization rules as runs, but have no run-shaped control context.
/// Keeping this narrow adapter here prevents gateway dispatch from mounting
/// opaque object-store files directly as a release filesystem.
#[derive(Clone)]
pub struct LocalGatewayReleaseRuntime {
    pub(crate) runtime_root: PathBuf,
    pub(crate) release_artifact_root: PathBuf,
}

/// Immutable identity of one host-owned persistent gateway service instance.
///
/// A new launch attempt receives a new `instance_id`; the gateway and revision
/// IDs remain sealed in the materialized tree for recovery and cleanup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GatewayServiceIdentity {
    /// Unique host-owned launch-attempt identity.
    pub instance_id: Uuid,
    /// Durable gateway aggregate identity.
    pub gateway_id: Uuid,
    /// Immutable gateway revision selected for this instance.
    pub revision_id: Uuid,
}

/// One validated persistent-service tree discovered during recovery.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewayServiceInstanceRecord {
    /// Sealed identity read from the tree metadata.
    pub identity: GatewayServiceIdentity,
    /// Host path below the dedicated service namespace.
    pub path: PathBuf,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GatewayServiceIdentityFile {
    schema_version: u8,
    instance_id: Uuid,
    gateway_id: Uuid,
    revision_id: Uuid,
}

impl From<GatewayServiceIdentity> for GatewayServiceIdentityFile {
    fn from(identity: GatewayServiceIdentity) -> Self {
        Self {
            schema_version: GATEWAY_SERVICE_SCHEMA_VERSION,
            instance_id: identity.instance_id,
            gateway_id: identity.gateway_id,
            revision_id: identity.revision_id,
        }
    }
}

impl TryFrom<GatewayServiceIdentityFile> for GatewayServiceIdentity {
    type Error = RunRuntimeError;
    fn try_from(metadata: GatewayServiceIdentityFile) -> Result<Self, Self::Error> {
        if metadata.schema_version != GATEWAY_SERVICE_SCHEMA_VERSION
            || metadata.instance_id.is_nil()
            || metadata.gateway_id.is_nil()
            || metadata.revision_id.is_nil()
        {
            return Err(runtime_error(
                "gateway service identity metadata is invalid",
            ));
        }
        Ok(Self {
            instance_id: metadata.instance_id,
            gateway_id: metadata.gateway_id,
            revision_id: metadata.revision_id,
        })
    }
}
