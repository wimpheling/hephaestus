//! Controlled checked models; no persisted native143 or physical proof.
use super::{super::fixture as identities, original::DeleteFixture};
use crate::{
    CommandIdentity, DeploymentAdmission, DeploymentError, DeploymentLifecycle,
    DeploymentOperation, DeploymentSnapshot, EffectClaim, InstallProgress,
    PreparedInstanceDeleteHistory, PreparedInstanceDeleteRequest, PreparedInstanceDetachRequest,
    PreparedInstanceTerminalProgress, PreparedInstanceTerminalPublisher, RemovalProgress,
    ResourceAction, ResourceProgress,
};
use std::collections::BTreeMap;
use uuid::Uuid;

pub(super) fn exact(action: ResourceAction) -> DeleteFixture {
    let mut exact = DeleteFixture::new(action);
    if action == ResourceAction::Delete {
        exact.effect.generation = 4;
        exact.effect.resource_version = 7;
    }
    exact.current = current();
    exact
}
pub(super) fn current() -> CommandIdentity {
    CommandIdentity::from_identity(&identities::identity(15, 16), DeploymentOperation::Remove)
        .unwrap()
}
pub(super) fn delete(exact: &DeleteFixture) -> PreparedInstanceDeleteRequest {
    exact.delete(8).unwrap()
}
pub(super) fn detach(exact: &DeleteFixture) -> PreparedInstanceDetachRequest {
    PreparedInstanceDetachRequest::new(
        exact.intent.clone(),
        exact.target.clone(),
        exact.closure.clone(),
        exact.effect.clone(),
        exact.current,
        exact.configuration.clone(),
    )
    .unwrap()
}
pub(super) fn progress(effect: &EffectClaim) -> PreparedInstanceTerminalProgress {
    let recovering = effect.action == ResourceAction::Detach;
    PreparedInstanceTerminalProgress::new(
        ResourceProgress {
            install: InstallProgress::Ready,
            removal: if recovering {
                RemovalProgress::RecoveryRequired
            } else {
                RemovalProgress::Detached
            },
            version: effect.resource_version + u64::from(recovering),
            active_attempt: Some(effect.provenance.attempt_id),
            diagnostic: recovering.then_some(crate::DiagnosticCode::ProviderOutcomeUnknown),
        },
        effect.generation,
        Some(if recovering {
            ResourceAction::Drain
        } else {
            ResourceAction::Detach
        }),
    )
    .unwrap()
}
pub(super) fn applied(effect: &EffectClaim) -> PreparedInstanceTerminalProgress {
    PreparedInstanceTerminalProgress::new(
        ResourceProgress {
            install: InstallProgress::Ready,
            removal: RemovalProgress::Deleted,
            version: effect.resource_version + 1,
            active_attempt: None,
            diagnostic: None,
        },
        effect.generation,
        Some(ResourceAction::Delete),
    )
    .unwrap()
}
pub(super) fn publisher(exact: &DeleteFixture) -> PreparedInstanceTerminalPublisher {
    let actor = identities::identity(13, 14);
    let command = CommandIdentity::from_identity(&actor, DeploymentOperation::Remove).unwrap();
    PreparedInstanceTerminalPublisher::new(
        &exact.effect,
        command,
        command.id(),
        actor.user_id,
        identities::identity(13, 21).request_id,
        Some(command.id()),
    )
    .unwrap()
}
pub(super) fn committed(
    exact: &DeleteFixture,
    observed: PreparedInstanceTerminalProgress,
) -> Result<PreparedInstanceDeleteHistory, DeploymentError> {
    PreparedInstanceDeleteHistory::from_observed_parts(
        delete(exact),
        observed,
        Some(Uuid::from_u128(90)),
        Some(9),
        Some(publisher(exact)),
    )
}
pub(super) fn admission(
    exact: &DeleteFixture,
    observed: &PreparedInstanceTerminalProgress,
) -> DeploymentAdmission {
    let lifecycle = if observed.progress().removal == RemovalProgress::RecoveryRequired {
        DeploymentLifecycle::RecoveryRequired
    } else {
        DeploymentLifecycle::Removing
    };
    let mut resources: BTreeMap<_, _> = exact
        .intent
        .resources()
        .keys()
        .map(|name| {
            (
                name.clone(),
                ResourceProgress {
                    install: InstallProgress::Ready,
                    removal: RemovalProgress::Pending,
                    version: 1,
                    active_attempt: None,
                    diagnostic: None,
                },
            )
        })
        .collect();
    resources.insert(exact.effect.resource.clone(), observed.progress().clone());
    DeploymentAdmission {
        disposition: crate::AdmissionDisposition::Resume,
        snapshot: DeploymentSnapshot {
            intent: exact.intent.clone(),
            lifecycle,
            version: 20,
            resources,
        },
        receipt: crate::CommandReceipt {
            command: exact.current,
            deployment_id: exact.intent.id(),
            input_hash: release_domain::ContentHash::digest(b"controlled admission"),
            lifecycle,
            deployment_version: 20,
            event: event_application::CommittedMutation {
                event_id: Uuid::from_u128(91),
                scope_kind: "project".into(),
                scope_id: exact.intent.project_id().as_uuid(),
                cursor: 1,
                aggregate_version: 1,
            },
        },
    }
}
pub(super) fn active(exact: &DeleteFixture) -> crate::RemoveContinuation {
    crate::RemoveContinuation {
        claim: exact.effect.clone(),
        closure: Some(exact.closure.clone()),
    }
}
