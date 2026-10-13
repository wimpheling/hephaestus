//! Global original lookup before configured owner/profile comparison.

use super::super::{
    agent_repository::PgRuntimeSessionRepository,
    common::{SessionRow, storage},
};
use capability_domain::RuntimeSessionId;
use runtime_authority::{LiveRunSnapshotPins, RuntimeAuthorityError};
use sqlx::{Postgres, Transaction};
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(sqlx::FromRow)]
pub(super) struct Original {
    pub(super) id: Uuid,
    command_id: Uuid,
    pub(super) instance_id: Uuid,
    pub(super) revision_id: Uuid,
    release_id: Uuid,
    pub(super) release_agent_id: Uuid,
    pub(super) attachment: Option<Uuid>,
    kind: String,
    profile: String,
    legacy_required: bool,
    plan_required: bool,
    vm_id: Option<String>,
    namespace: Option<String>,
    host: Option<String>,
    binding: Option<String>,
}
impl Original {
    pub(super) fn pins(&self) -> Result<LiveRunSnapshotPins, RuntimeAuthorityError> {
        LiveRunSnapshotPins::new(
            self.id,
            self.instance_id,
            self.revision_id,
            self.release_id,
            self.release_agent_id,
            self.command_id,
        )
    }
    fn strict_legacy(&self) -> bool {
        self.profile == "legacy_scalar"
            && self.legacy_required
            && !self.plan_required
            && self.namespace.is_none()
            && self.host.is_none()
            && self.binding.is_none()
            && matches!(self.kind.as_str(), "normal" | "update")
    }
}

pub(super) async fn load(
    tx: &mut Transaction<'_, Postgres>,
    run: Uuid,
) -> Result<Original, RuntimeAuthorityError> {
    let original: Original = sqlx::query_as(
        "SELECT id,command_id,instance_id,instance_revision_id AS revision_id,
                release_id,release_agent_id,attachment_id AS attachment,run_kind AS kind,
                execution_profile AS profile,legacy_vm_placement_required AS legacy_required,
                vm_plan_admission_required AS plan_required,vm_id,
                vm_provider_namespace AS namespace,vm_host_id AS host,vm_binding_kind AS binding
         FROM public.runs WHERE id=$1",
    )
    .bind(run)
    .fetch_optional(&mut **tx)
    .await
    .map_err(storage)?
    .ok_or(RuntimeAuthorityError::NotFound)?;
    original.pins()?;
    Ok(original)
}

pub(super) async fn qualify(
    repository: &PgRuntimeSessionRepository,
    tx: &mut Transaction<'_, Postgres>,
    original: &Original,
    session: RuntimeSessionId,
) -> Result<(), RuntimeAuthorityError> {
    if original.kind == "invocation" {
        invocation(repository, tx, original, session).await
    } else {
        legacy(repository, tx, original).await
    }
}

#[derive(sqlx::FromRow)]
struct Placement {
    namespace: Uuid,
    host: String,
    vm_id: String,
    consumed_at: Option<OffsetDateTime>,
    consumption_kind: Option<String>,
    exact: bool,
    conflicting: bool,
}

async fn legacy(
    repository: &PgRuntimeSessionRepository,
    tx: &mut Transaction<'_, Postgres>,
    original: &Original,
) -> Result<(), RuntimeAuthorityError> {
    let scope = repository
        .expired_legacy_scope
        .as_ref()
        .ok_or(RuntimeAuthorityError::Unsupported)?;
    if !original.strict_legacy() {
        return Err(RuntimeAuthorityError::Unsupported);
    }
    let placements: Vec<Placement> = sqlx::query_as(
        "SELECT plan.provider_namespace AS namespace,plan.host_id AS host,plan.vm_id,
                plan.consumed_at,plan.consumption_kind,
                public.legacy_vm_placement_matches(execution,plan) IS TRUE AS exact,
                (EXISTS(SELECT 1 FROM public.run_vm_plan_admissions WHERE run_id=execution.id)
                 OR EXISTS(SELECT 1 FROM public.agent_instance_execution_invocations WHERE run_id=execution.id)
                 OR EXISTS(SELECT 1 FROM public.run_invocation_consumptions WHERE run_id=execution.id)) AS conflicting
         FROM public.run_legacy_vm_placements plan
         JOIN public.runs execution ON execution.id=plan.run_id WHERE plan.run_id=$1 LIMIT 2")
        .bind(original.id).fetch_all(&mut **tx).await.map_err(storage)?;
    let [plan] = placements.as_slice() else {
        return Err(RuntimeAuthorityError::IdentityMismatch);
    };
    if !placement_matches(
        original,
        plan,
        scope.provider_scope().namespace(),
        scope.provider_scope().host_id(),
    ) {
        return Err(RuntimeAuthorityError::IdentityMismatch);
    }
    Ok(())
}

fn placement_matches(original: &Original, plan: &Placement, namespace: &str, host: &str) -> bool {
    plan.exact
        && !plan.conflicting
        && !plan.namespace.is_nil()
        && plan.namespace.to_string() == namespace
        && plan.host == host
        && plan.vm_id == original.id.to_string()
        && original.vm_id.as_ref().is_none_or(|id| id == &plan.vm_id)
        && plan.consumed_at.is_some()
        && plan.consumption_kind.as_deref() == Some("provision")
}

async fn invocation(
    repository: &PgRuntimeSessionRepository,
    tx: &mut Transaction<'_, Postgres>,
    original: &Original,
    session: RuntimeSessionId,
) -> Result<(), RuntimeAuthorityError> {
    let config = repository
        .invocation_configuration
        .as_ref()
        .ok_or(RuntimeAuthorityError::Unsupported)?;
    let scope = config.provider_scope();
    if original.profile != "owned_canonical"
        || original.legacy_required
        || original.plan_required
        || original.attachment.is_some()
        || session.as_uuid() != original.id
        || original.vm_id.as_deref() != Some(original.id.to_string().as_str())
        || original.binding.as_deref() != Some("planned")
        || original.namespace.as_deref() != Some(scope.namespace())
        || original.host.as_deref() != Some(scope.host_id())
    {
        return Err(RuntimeAuthorityError::IdentityMismatch);
    }
    // Source124's worker-only helper checks immutable birth, consumed config and
    // full historical session graph. It explicitly permits Expired without live auth.
    let rows: Vec<SessionRow> = sqlx::query_as(
        "SELECT id,snapshot_id,identity_hash,issuance_generation,status,issued_at,
                expires_at,acknowledged_at,revoked_at
         FROM public.find_invocation119_session($1,$2)",
    )
    .bind(original.id)
    .bind(repository.invocation_config_bytes()?)
    .fetch_all(&mut **tx)
    .await
    .map_err(storage)?;
    let [row] = rows.as_slice() else {
        return Err(RuntimeAuthorityError::NotFound);
    };
    if row.id != session.as_uuid() || row.status != "expired" {
        return Err(RuntimeAuthorityError::IdentityMismatch);
    }
    Ok(())
}

#[cfg(test)]
#[path = "original_tests.rs"]
mod tests;

#[cfg(test)]
pub(super) fn controlled_original() -> Original {
    Original {
        id: Uuid::from_u128(1),
        command_id: Uuid::from_u128(2),
        instance_id: Uuid::from_u128(3),
        revision_id: Uuid::from_u128(4),
        release_id: Uuid::from_u128(5),
        release_agent_id: Uuid::from_u128(6),
        attachment: Some(Uuid::from_u128(7)),
        kind: "normal".into(),
        profile: "legacy_scalar".into(),
        legacy_required: true,
        plan_required: false,
        vm_id: None,
        namespace: None,
        host: None,
        binding: None,
    }
}
