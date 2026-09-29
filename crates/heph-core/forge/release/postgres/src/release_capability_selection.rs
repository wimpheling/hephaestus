use super::{
    AuthenticatedIdentity, CapabilityBinding, CapabilityOperation, CapabilityResourceKind,
    ObjectRef, ObjectType, Permission, Postgres, ReleaseService, ReleaseServiceError, Transaction,
    Uuid,
};

pub fn parse_capability_resource_kind(
    value: &str,
) -> Result<CapabilityResourceKind, ReleaseServiceError> {
    match value {
        "repository" => Ok(CapabilityResourceKind::Repository),
        "project" => Ok(CapabilityResourceKind::Project),
        "agent_instance" => Ok(CapabilityResourceKind::AgentInstance),
        "gateway" => Ok(CapabilityResourceKind::Gateway),
        "run" => Ok(CapabilityResourceKind::Run),
        "state_volume" => Ok(CapabilityResourceKind::StateVolume),
        "mailbox" => Ok(CapabilityResourceKind::Mailbox),
        _ => Err(ReleaseServiceError::InvalidStoredData),
    }
}

pub fn parse_capability_operation(value: &str) -> Result<CapabilityOperation, ReleaseServiceError> {
    match value {
        "inspect" => Ok(CapabilityOperation::Inspect),
        "configure" => Ok(CapabilityOperation::Configure),
        "execute" => Ok(CapabilityOperation::Execute),
        "update" => Ok(CapabilityOperation::Update),
        "pause" => Ok(CapabilityOperation::Pause),
        "recover" => Ok(CapabilityOperation::Recover),
        "cancel" => Ok(CapabilityOperation::Cancel),
        "attach" => Ok(CapabilityOperation::Attach),
        "restore" => Ok(CapabilityOperation::Restore),
        "git_read" => Ok(CapabilityOperation::GitRead),
        "create_ref" => Ok(CapabilityOperation::CreateRef),
        "update_ref" => Ok(CapabilityOperation::UpdateRef),
        "force_update_ref" => Ok(CapabilityOperation::ForceUpdateRef),
        "delete_ref" => Ok(CapabilityOperation::DeleteRef),
        "create_tag" => Ok(CapabilityOperation::CreateTag),
        "delete_tag" => Ok(CapabilityOperation::DeleteTag),
        "trigger_run" => Ok(CapabilityOperation::TriggerRun),
        "manage_attachments" => Ok(CapabilityOperation::ManageAttachments),
        "publish" => Ok(CapabilityOperation::Publish),
        _ => Err(ReleaseServiceError::InvalidStoredData),
    }
}

pub async fn capability_resource_is_in_project(
    tx: &mut Transaction<'_, Postgres>,
    project_id: Uuid,
    kind: CapabilityResourceKind,
    resource_id: Uuid,
) -> Result<bool, ReleaseServiceError> {
    let found = match kind {
        CapabilityResourceKind::Repository => {
            sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS (
                    SELECT 1 FROM repositories
                    WHERE id = $1 AND project_id = $2
                 )",
            )
            .bind(resource_id)
            .bind(project_id)
            .fetch_one(&mut **tx)
            .await?
        }
        CapabilityResourceKind::Project => resource_id == project_id,
        CapabilityResourceKind::AgentInstance => {
            sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS (
                    SELECT 1 FROM agent_instances
                    WHERE id = $1 AND project_id = $2 AND state <> 'removed'
                 )",
            )
            .bind(resource_id)
            .bind(project_id)
            .fetch_one(&mut **tx)
            .await?
        }
        CapabilityResourceKind::Run => {
            sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS (
                    SELECT 1 FROM runs AS run
                    JOIN agent_instances AS instance
                      ON instance.id = run.instance_id
                    WHERE run.id = $1 AND instance.project_id = $2
                 )",
            )
            .bind(resource_id)
            .bind(project_id)
            .fetch_one(&mut **tx)
            .await?
        }
        CapabilityResourceKind::StateVolume => {
            sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS (
                    SELECT 1 FROM agent_instance_state_volumes AS volume
                    JOIN agent_instances AS instance
                      ON instance.id = volume.instance_id
                    WHERE volume.id = $1 AND instance.project_id = $2
                 )",
            )
            .bind(resource_id)
            .bind(project_id)
            .fetch_one(&mut **tx)
            .await?
        }
        // Mailbox publication is a gateway-only capability. Agent-instance
        // bindings must not gain a mailbox target merely because the shared
        // vocabulary can represent one.
        CapabilityResourceKind::Gateway | CapabilityResourceKind::Mailbox => false,
    };
    Ok(found)
}

pub async fn authorize_capability_selection(
    service: &ReleaseService,
    tx: &mut Transaction<'_, Postgres>,
    identity: &AuthenticatedIdentity,
    binding: &CapabilityBinding,
) -> Result<(), ReleaseServiceError> {
    let object_type = match binding.resource().kind {
        CapabilityResourceKind::Repository => ObjectType::Repository,
        CapabilityResourceKind::Project => ObjectType::Project,
        CapabilityResourceKind::AgentInstance => ObjectType::AgentInstance,
        CapabilityResourceKind::Run => ObjectType::Run,
        CapabilityResourceKind::StateVolume => ObjectType::StateVolume,
        CapabilityResourceKind::Gateway | CapabilityResourceKind::Mailbox => {
            return Err(ReleaseServiceError::CapabilityResourceUnavailable);
        }
    };
    let object = ObjectRef::new(object_type, binding.resource().id);
    service
        .require(tx, identity, Permission::CanGrantAgentCapability, object)
        .await?;
    let mut checked = Vec::new();
    for operation in binding.granted_operations() {
        let permission = capability_grant_permission(binding.resource().kind, operation)
            .ok_or(ReleaseServiceError::InvalidCapabilityBinding)?;
        if !checked.contains(&permission) {
            service.require(tx, identity, permission, object).await?;
            checked.push(permission);
        }
    }
    Ok(())
}

pub const fn capability_grant_permission(
    resource_kind: CapabilityResourceKind,
    operation: CapabilityOperation,
) -> Option<Permission> {
    match (resource_kind, operation) {
        (
            CapabilityResourceKind::Repository,
            CapabilityOperation::Inspect | CapabilityOperation::GitRead,
        )
        | (
            CapabilityResourceKind::Project
            | CapabilityResourceKind::AgentInstance
            | CapabilityResourceKind::Run
            | CapabilityResourceKind::StateVolume,
            CapabilityOperation::Inspect,
        ) => Some(Permission::CanRead),
        (
            CapabilityResourceKind::Repository,
            CapabilityOperation::CreateRef
            | CapabilityOperation::UpdateRef
            | CapabilityOperation::ForceUpdateRef
            | CapabilityOperation::DeleteRef
            | CapabilityOperation::CreateTag
            | CapabilityOperation::DeleteTag
            | CapabilityOperation::TriggerRun
            | CapabilityOperation::ManageAttachments,
        )
        | (CapabilityResourceKind::Project, CapabilityOperation::Execute) => {
            Some(Permission::CanWrite)
        }
        (
            CapabilityResourceKind::Project,
            CapabilityOperation::Configure
            | CapabilityOperation::Update
            | CapabilityOperation::Pause
            | CapabilityOperation::Recover,
        )
        | (
            CapabilityResourceKind::AgentInstance,
            CapabilityOperation::Configure | CapabilityOperation::Pause,
        ) => Some(Permission::CanManage),
        (CapabilityResourceKind::AgentInstance, CapabilityOperation::Execute) => {
            Some(Permission::CanExecute)
        }
        (CapabilityResourceKind::AgentInstance, CapabilityOperation::Update) => {
            Some(Permission::CanUpdate)
        }
        (
            CapabilityResourceKind::AgentInstance | CapabilityResourceKind::Run,
            CapabilityOperation::Recover,
        ) => Some(Permission::CanRecover),
        (CapabilityResourceKind::Run, CapabilityOperation::Cancel) => Some(Permission::CanCancel),
        (CapabilityResourceKind::StateVolume, CapabilityOperation::Attach) => {
            Some(Permission::CanAttach)
        }
        (CapabilityResourceKind::StateVolume, CapabilityOperation::Restore) => {
            Some(Permission::CanRestore)
        }
        // Construction validates the compatibility matrix before this
        // mapping is evaluated. A defensive fallback still denies malformed
        // stored input instead of manufacturing broader authority.
        _ => None,
    }
}
