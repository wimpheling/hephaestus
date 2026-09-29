use crate::{
    application::event::{AggregateVersion, ApplicationEvent, EventScope as AppScope, ScopeKind},
    event_cursor::EventCursorCodec,
    rpc::RpcError,
};
use rpc_proto::messages::hephaestus::{
    common::v1::{Cursor, OpaqueId},
    event::v1::{
        AgentInstanceChanged, AgentSecretBindingChanged, AggregateType, AggregateVersionReference,
        ArtifactChanged, BuildChanged, ChangeKind, EventProvenance, EventScope, EventScopeKind,
        GatewayChanged, IdentityOrganizationsChanged, IdentityProfileChanged, LifecycleState,
        OrganizationChanged, ProductEvent, ProjectChanged, RegistryPublicationChanged,
        ReleaseChanged, RepositoryChanged, RepositoryRefChanged, ReviewChanged, RunChanged,
        SecretGrantChanged, SecretImportChanged, SecretMetadataChanged, product_event,
    },
};
use time::OffsetDateTime;
use uuid::Uuid;

// Exhaustive typed projection is intentionally kept in one auditable match.
#[allow(clippy::too_many_lines, clippy::redundant_pub_crate)]
pub(crate) fn event(
    codec: &EventCursorCodec,
    scope: AppScope,
    value: &ApplicationEvent,
) -> Result<ProductEvent, RpcError> {
    let aggregate_type = aggregate_type(&value.aggregate_type)?;
    let change = change(&value.change_kind)?;
    let state = lifecycle(value.safe_state.as_deref())?;
    let related_one = value.related_id_one.map(opaque);
    let related_two = value.related_id_two.map(opaque);
    let payload = match (aggregate_type, value.event_type.as_str()) {
        (AggregateType::IdentityProfile, "identity.profile_changed") => {
            product_event::Payload::IdentityProfileChanged(Box::new(IdentityProfileChanged {
                change: change.into(),
                state: state.into(),
                ..Default::default()
            }))
        }
        (AggregateType::IdentityOrganizations, "identity.organizations_changed") => {
            product_event::Payload::IdentityOrganizationsChanged(Box::new(
                IdentityOrganizationsChanged {
                    organization_id: required(related_one)?.into(),
                    change: change.into(),
                    state: state.into(),
                    ..Default::default()
                },
            ))
        }
        (AggregateType::Organization, "organization.changed") => {
            product_event::Payload::OrganizationChanged(Box::new(OrganizationChanged {
                change: change.into(),
                state: state.into(),
                ..Default::default()
            }))
        }
        (AggregateType::Project, "project.changed") => {
            product_event::Payload::ProjectChanged(Box::new(ProjectChanged {
                organization_id: required(related_one)?.into(),
                change: change.into(),
                state: state.into(),
                ..Default::default()
            }))
        }
        (AggregateType::Repository, "repository.changed") => {
            product_event::Payload::RepositoryChanged(Box::new(RepositoryChanged {
                project_id: required(related_one)?.into(),
                change: change.into(),
                state: state.into(),
                ..Default::default()
            }))
        }
        (AggregateType::RepositoryRef, "repository.ref_changed") => {
            product_event::Payload::RepositoryRefChanged(Box::new(RepositoryRefChanged {
                change: change.into(),
                state: state.into(),
                ..Default::default()
            }))
        }
        (AggregateType::Build, "build.changed") => {
            product_event::Payload::BuildChanged(Box::new(BuildChanged {
                repository_id: required(related_one)?.into(),
                change: change.into(),
                state: state.into(),
                ..Default::default()
            }))
        }
        (AggregateType::Release, "release.changed") => {
            product_event::Payload::ReleaseChanged(Box::new(ReleaseChanged {
                repository_id: required(related_one)?.into(),
                change: change.into(),
                state: state.into(),
                ..Default::default()
            }))
        }
        (AggregateType::AgentInstance, "agent_instance.changed") => {
            product_event::Payload::AgentInstanceChanged(Box::new(AgentInstanceChanged {
                project_id: related_one.into(),
                change: change.into(),
                state: state.into(),
                ..Default::default()
            }))
        }
        (AggregateType::Run, "run.changed") => {
            product_event::Payload::RunChanged(Box::new(RunChanged {
                project_id: related_one.into(),
                repository_id: related_two.into(),
                change: change.into(),
                state: state.into(),
                ..Default::default()
            }))
        }
        (AggregateType::Review, "review.changed") => {
            product_event::Payload::ReviewChanged(Box::new(ReviewChanged {
                run_id: required(related_one)?.into(),
                change: change.into(),
                state: state.into(),
                ..Default::default()
            }))
        }
        (AggregateType::SecretMetadata, "secret_metadata.changed") => {
            product_event::Payload::SecretMetadataChanged(Box::new(SecretMetadataChanged {
                owner_id: required(related_one)?.into(),
                change: change.into(),
                state: state.into(),
                ..Default::default()
            }))
        }
        (AggregateType::SecretGrant, "secret_grant.changed") => {
            product_event::Payload::SecretGrantChanged(Box::new(SecretGrantChanged {
                secret_id: required(related_one)?.into(),
                target_id: required(related_two)?.into(),
                change: change.into(),
                state: state.into(),
                ..Default::default()
            }))
        }
        (AggregateType::SecretImport, "secret_import.changed") => {
            product_event::Payload::SecretImportChanged(Box::new(SecretImportChanged {
                secret_id: required(related_one)?.into(),
                target_id: required(related_two)?.into(),
                change: change.into(),
                state: state.into(),
                ..Default::default()
            }))
        }
        (AggregateType::AgentSecretBinding, "agent_secret_binding.changed") => {
            product_event::Payload::AgentSecretBindingChanged(Box::new(AgentSecretBindingChanged {
                agent_instance_id: required(related_one)?.into(),
                secret_import_id: required(related_two)?.into(),
                change: change.into(),
                state: state.into(),
                ..Default::default()
            }))
        }
        (AggregateType::Artifact, "artifact.changed") => {
            product_event::Payload::ArtifactChanged(Box::new(ArtifactChanged {
                release_id: required(related_one)?.into(),
                build_id: required(related_two)?.into(),
                change: change.into(),
                state: state.into(),
                ..Default::default()
            }))
        }
        (AggregateType::RegistryPublication, "registry.publication_changed") => {
            product_event::Payload::RegistryPublicationChanged(Box::new(
                RegistryPublicationChanged {
                    change: change.into(),
                    state: state.into(),
                    ..Default::default()
                },
            ))
        }
        (AggregateType::Gateway, "gateway.changed") => {
            product_event::Payload::GatewayChanged(Box::new(GatewayChanged {
                change: change.into(),
                state: state.into(),
                ..Default::default()
            }))
        }
        _ => return Err(RpcError::Internal),
    };
    Ok(ProductEvent {
        event_id: opaque(value.id).into(),
        cursor: cursor(codec, scope, value.cursor)?.into(),
        scope: proto_scope(scope).into(),
        aggregate_type: aggregate_type.into(),
        aggregate_id: opaque(value.aggregate_id).into(),
        aggregate_version: u64::try_from(value.aggregate_version)
            .map_err(|_| RpcError::Internal)?,
        occurred_at: timestamp(value.occurred_at).into(),
        provenance: EventProvenance {
            actor_id: value.actor_id.map(opaque).into(),
            request_id: value.request_id.map(opaque).into(),
            ..Default::default()
        }
        .into(),
        schema_version: u32::try_from(value.schema_version).map_err(|_| RpcError::Internal)?,
        payload: Some(payload),
        ..Default::default()
    })
}

pub(super) fn version(value: &AggregateVersion) -> Result<AggregateVersionReference, RpcError> {
    Ok(AggregateVersionReference {
        aggregate_type: aggregate_type(&value.kind)?.into(),
        aggregate_id: opaque(value.id).into(),
        aggregate_version: u64::try_from(value.version).map_err(|_| RpcError::Internal)?,
        ..Default::default()
    })
}

fn aggregate_type(value: &str) -> Result<AggregateType, RpcError> {
    match value {
        "identity_profile" => Ok(AggregateType::IdentityProfile),
        "identity_organizations" => Ok(AggregateType::IdentityOrganizations),
        "organization" => Ok(AggregateType::Organization),
        "project" => Ok(AggregateType::Project),
        "repository" => Ok(AggregateType::Repository),
        "repository_ref" => Ok(AggregateType::RepositoryRef),
        "build" => Ok(AggregateType::Build),
        "release" => Ok(AggregateType::Release),
        "agent_instance" => Ok(AggregateType::AgentInstance),
        "run" => Ok(AggregateType::Run),
        "review" => Ok(AggregateType::Review),
        "secret_metadata" => Ok(AggregateType::SecretMetadata),
        "secret_grant" => Ok(AggregateType::SecretGrant),
        "secret_import" => Ok(AggregateType::SecretImport),
        "agent_secret_binding" => Ok(AggregateType::AgentSecretBinding),
        "artifact" => Ok(AggregateType::Artifact),
        "registry_publication" => Ok(AggregateType::RegistryPublication),
        "gateway" => Ok(AggregateType::Gateway),
        _ => Err(RpcError::Internal),
    }
}

fn change(value: &str) -> Result<ChangeKind, RpcError> {
    match value {
        "created" => Ok(ChangeKind::Created),
        "updated" => Ok(ChangeKind::Updated),
        "state_changed" => Ok(ChangeKind::StateChanged),
        "removed" => Ok(ChangeKind::Removed),
        _ => Err(RpcError::Internal),
    }
}

fn lifecycle(value: Option<&str>) -> Result<LifecycleState, RpcError> {
    match value {
        None | Some("") => Ok(LifecycleState::Unspecified),
        Some("pending") => Ok(LifecycleState::Pending),
        Some("queued") => Ok(LifecycleState::Queued),
        Some("running") => Ok(LifecycleState::Running),
        Some("active") => Ok(LifecycleState::Active),
        Some("paused") => Ok(LifecycleState::Paused),
        Some("succeeded") => Ok(LifecycleState::Succeeded),
        Some("failed") => Ok(LifecycleState::Failed),
        Some("published") => Ok(LifecycleState::Published),
        Some("revoked") => Ok(LifecycleState::Revoked),
        Some("disabled") => Ok(LifecycleState::Disabled),
        Some("rejected") => Ok(LifecycleState::Rejected),
        Some("conflicted") => Ok(LifecycleState::Conflicted),
        Some("removed") => Ok(LifecycleState::Removed),
        Some(_) => Err(RpcError::Internal),
    }
}

pub(super) fn proto_scope(value: AppScope) -> EventScope {
    EventScope {
        kind: match value.kind {
            ScopeKind::Identity => EventScopeKind::Identity,
            ScopeKind::Organization => EventScopeKind::Organization,
            ScopeKind::Project => EventScopeKind::Project,
            ScopeKind::Repository => EventScopeKind::Repository,
            ScopeKind::Run => EventScopeKind::Run,
            ScopeKind::AgentInstance => EventScopeKind::AgentInstance,
        }
        .into(),
        id: opaque(value.id).into(),
        ..Default::default()
    }
}

fn required(value: Option<OpaqueId>) -> Result<OpaqueId, RpcError> {
    value.ok_or(RpcError::Internal)
}

fn opaque(value: Uuid) -> OpaqueId {
    OpaqueId {
        value: value.to_string(),
        ..Default::default()
    }
}

pub(super) fn cursor(
    codec: &EventCursorCodec,
    scope: AppScope,
    value: i64,
) -> Result<Cursor, RpcError> {
    if value < 0 {
        return Err(RpcError::Internal);
    }
    Ok(Cursor {
        value: codec.encode(scope.kind.as_str(), scope.id, value),
        ..Default::default()
    })
}

pub(super) fn timestamp(value: OffsetDateTime) -> buffa_types::google::protobuf::Timestamp {
    buffa_types::google::protobuf::Timestamp {
        seconds: value.unix_timestamp(),
        nanos: i32::try_from(value.nanosecond()).unwrap_or_default(),
        ..Default::default()
    }
}
