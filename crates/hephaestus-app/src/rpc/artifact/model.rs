use crate::application::artifact::{ArtifactError, ArtifactMetadata};
use rpc_proto::messages::hephaestus::{
    artifact::v1::{Artifact, ArtifactProvenance},
    common::v1::OpaqueId,
};
use uuid::Uuid;

pub(super) fn artifact(value: &ArtifactMetadata) -> Artifact {
    Artifact {
        id: opaque(value.id).into(),
        path: value.path.clone(),
        kind: value.kind.clone(),
        mode: value.mode,
        sha256: value.sha256.clone(),
        size_bytes: value.size_bytes,
        media_type: value.media_type.clone(),
        provenance: ArtifactProvenance {
            build_id: opaque(value.build_id).into(),
            release_id: opaque(value.release_id).into(),
            source_commit: value.source_commit.clone(),
            ..Default::default()
        }
        .into(),
        ..Default::default()
    }
}

fn opaque(id: Uuid) -> OpaqueId {
    OpaqueId {
        value: id.to_string(),
        ..Default::default()
    }
}

// A single exhaustive match keeps every storage and transport category stable.
#[allow(clippy::cognitive_complexity)]
pub(super) fn application_error(error: ArtifactError) -> super::super::RpcError {
    use super::super::RpcError;

    match error {
        ArtifactError::NotFound => RpcError::NotFound,
        ArtifactError::InvalidArgument => RpcError::InvalidArgument,
        ArtifactError::ResourceExhausted => RpcError::ResourceExhausted,
        ArtifactError::InvalidUtf8 => RpcError::FailedPrecondition,
        ArtifactError::InvalidStoredData => {
            tracing::error!(%error, "artifact metadata did not match canonical storage");
            RpcError::Internal
        }
        ArtifactError::Persistence(source) => {
            tracing::error!(error = %source, "artifact authorization query failed");
            RpcError::Unavailable
        }
        ArtifactError::Storage(source) => {
            tracing::error!(error = %source, "canonical artifact resolution failed");
            RpcError::Unavailable
        }
        ArtifactError::Io(source) => {
            tracing::error!(error = %source, "canonical artifact read failed");
            RpcError::Unavailable
        }
    }
}
