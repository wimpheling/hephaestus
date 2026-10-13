//! Separate bounded ordinary inspection; existing Invocation qualification is unchanged.

mod legacy;
mod projection;
mod records;
mod unborn;

use crate::errors::storage;
pub use projection::{
    QualifiedOrdinaryEvidence, QualifiedOrdinaryGitSource, QualifiedOrdinaryRoute,
    QualifiedOrdinaryRoutingDecision, QualifiedOrdinaryRoutingHoldReason, QualifiedOrdinaryUnborn,
};
use records::{Inbox, OriginalRun, Request};
use release_service::InstanceExecutionConfiguration;
use run_domain::StartRun;
use run_orchestrator::RepositoryError;
use runtime_types::RunId;
use sqlx::{Postgres, Transaction};

pub(super) async fn inspect(
    tx: &mut Transaction<'_, Postgres>,
    expected: &InstanceExecutionConfiguration,
    run: RunId,
    supplied: Option<&StartRun>,
) -> Result<QualifiedOrdinaryRoutingDecision, RepositoryError> {
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .execute(&mut **tx)
        .await
        .map_err(storage)?;
    sqlx::query("SET LOCAL search_path = pg_catalog, public, pg_temp")
        .execute(&mut **tx)
        .await
        .map_err(storage)?;
    let role: String = sqlx::query_scalar("SELECT current_user::text")
        .fetch_one(&mut **tx)
        .await
        .map_err(storage)?;
    if role != "hephaestus_worker" {
        return Ok(held(QualifiedOrdinaryRoutingHoldReason::WorkerRoleRequired));
    }
    // Global original first: no expected-owner, profile, kind or current-dispatch filter.
    let original:Option<OriginalRun>=sqlx::query_as(
        "SELECT id,command_id,instance_id,instance_revision_id,release_id,release_agent_id,
                attachment_id,run_kind,requires_state,execution_profile,legacy_vm_placement_required,
                vm_id,vm_provider_namespace,vm_host_id,vm_binding_kind,vm_plan_admission_required,
                state,cancel_requested_at IS NOT NULL AS cancellation_requested,
                volume_acquisition_closed_at IS NOT NULL AS acquisition_closed
         FROM public.runs WHERE id=$1")
        .bind(run.as_uuid()).fetch_optional(&mut **tx).await.map_err(storage)?;
    let Some(original) = original else {
        return unborn::inspect(tx, run, supplied).await;
    };
    // Exclusion only: this ordinary API never replaces Invocation qualification.
    if original.is_invocation() {
        return Ok(held(
            QualifiedOrdinaryRoutingHoldReason::UnsupportedProducer,
        ));
    }
    match original.execution_profile.as_str() {
        "legacy_scalar" => legacy::inspect(tx, expected, original, supplied).await,
        "owned_canonical" => Ok(held(
            QualifiedOrdinaryRoutingHoldReason::CanonicalOriginalFacadeRequired,
        )),
        _ => Ok(held(QualifiedOrdinaryRoutingHoldReason::UnsupportedProfile)),
    }
}

async fn requests(
    tx: &mut Transaction<'_, Postgres>,
    run: RunId,
    command: Option<uuid::Uuid>,
) -> Result<Vec<Request>, RepositoryError> {
    // OR exposes command collisions with another Run instead of prefiltering them away.
    sqlx::query_as(
        "SELECT id,run_id,command_id,instance_id,instance_revision_id,release_id,
        release_agent_id,attachment_id,requires_state,request_kind,repository_id,commit_sha,
        git_ref,receive_id,trigger_command_id,actor_id,request_id,retry_of_run_id,attempt,
        platform_policy_version FROM public.run_requests WHERE run_id=$1 OR command_id=$2 LIMIT 2",
    )
    .bind(run.as_uuid())
    .bind(command)
    .fetch_all(&mut **tx)
    .await
    .map_err(storage)
}

async fn inbox(
    tx: &mut Transaction<'_, Postgres>,
    start: &StartRun,
) -> Result<Option<Inbox>, RepositoryError> {
    sqlx::query_as(
        "SELECT command_type,payload,processed_at IS NOT NULL AS processed
        FROM public.command_inbox WHERE command_id=$1",
    )
    .bind(start.command_id.as_uuid())
    .fetch_optional(&mut **tx)
    .await
    .map_err(storage)
}

pub(super) async fn incompatible_producers(
    tx: &mut Transaction<'_, Postgres>,
    run: RunId,
) -> Result<bool, RepositoryError> {
    // These actual receipt families cannot supply the supported Git Normal original.
    sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM public.mailbox_delivery_attempts WHERE run_id=$1)
        OR EXISTS(SELECT 1 FROM public.agent_updates WHERE hook_run_id=$1)
        OR EXISTS(SELECT 1 FROM public.run_vm_plan_admissions WHERE run_id=$1)
        OR EXISTS(SELECT 1 FROM public.agent_instance_execution_invocations WHERE run_id=$1)
        OR EXISTS(SELECT 1 FROM public.run_invocation_consumptions WHERE run_id=$1)",
    )
    .bind(run.as_uuid())
    .fetch_one(&mut **tx)
    .await
    .map_err(storage)
}

pub(super) const fn held(
    reason: QualifiedOrdinaryRoutingHoldReason,
) -> QualifiedOrdinaryRoutingDecision {
    QualifiedOrdinaryRoutingDecision::Held(reason)
}
