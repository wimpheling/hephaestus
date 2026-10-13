//! Audited caller rights after trusted shared owned-source resolution.

use authz_domain::{ObjectRef, ObjectType, Permission, Subject};
use authz_postgres::audit_decision;
use identity_domain::{RequestId, UserId};
use run_domain::{Run, RunKind};
use run_orchestrator::RunAuthorizationError;

use super::{PgRunLaunchAuthorizer, error, redacted};

impl PgRunLaunchAuthorizer {
    /// Audits fresh caller/source rights for an exact owned producer principal.
    ///
    /// The app first validates the complete live snapshot and obtains original
    /// actor/request facts from its trusted readonly source adapter. This helper
    /// rechecks their exact DB correlation before granting caller permissions.
    /// It grants no mount authority, dispatch support, issuance or renewal.
    ///
    /// # Errors
    ///
    /// Rejects non-worker calls, mismatched or inactive original actors, closed
    /// preissuance, stale live gates, missing issued session or denied rights.
    pub async fn authorize_canonical_source(
        &self,
        run: &Run,
        actor: UserId,
        request_id: RequestId,
        issued: bool,
    ) -> Result<(), RunAuthorizationError> {
        if run.kind == RunKind::Invocation {
            return Err(redacted("Invocation canonical launch is unsupported"));
        }
        let mut tx = self.pool.begin().await.map_err(error)?;
        let worker: bool = sqlx::query_scalar("SELECT current_user='hephaestus_worker'")
            .fetch_one(&mut *tx)
            .await
            .map_err(error)?;
        if !worker {
            return Err(redacted("canonical launch requires a worker"));
        }
        let exact: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM runs run
              JOIN agent_instances instance ON instance.id=run.instance_id
              JOIN users actor ON actor.id=$9 AND actor.status='active'
              WHERE ROW(run.id,run.instance_id,run.instance_revision_id,run.release_id,
                  run.release_agent_id,run.command_id,run.run_kind,run.attachment_id)
                IS NOT DISTINCT FROM ROW($1,$2,$3,$4,$5,$6,$7,$8)
                AND run.execution_profile='owned_canonical' AND run.vm_id=run.id::text
                AND run.vm_binding_kind='planned' AND run.vm_host_id IS NOT NULL
                AND run.vm_provider_namespace IS NOT NULL AND run.cancel_requested_at IS NULL
                AND NOT EXISTS(SELECT 1 FROM agent_instance_removal_requests WHERE instance_id=run.instance_id)
                AND NOT EXISTS(SELECT 1 FROM mailbox_delivery_attempts WHERE run_id=run.id)
                AND (($11 AND run.state IN ('provisioning','starting','running','succeeded','failed','cleaning_up')
                  AND EXISTS(SELECT 1 FROM runtime_authority_sessions session WHERE session.id=run.id
                    AND session.run_id=run.id AND session.instance_id=run.instance_id
                    AND session.instance_revision_id=run.instance_revision_id AND session.snapshot_id=run.id
                    AND session.status IN ('pending_handoff','active') AND session.revoked_at IS NULL
                    AND session.issued_at<=clock_timestamp() AND session.expires_at>clock_timestamp()))
                  OR (NOT $11 AND run.state IN ('leasing_volume','provisioning')
                    AND run.volume_acquisition_closed_at IS NULL))
                AND ((run.run_kind='normal' AND instance.state IN ('active','update_rejected')
                  AND instance.active_revision_id=run.instance_revision_id AND instance.run_gate_open
                  AND EXISTS(SELECT 1 FROM agent_attachments attachment WHERE attachment.id=run.attachment_id
                    AND attachment.instance_id=instance.id AND attachment.enabled AND attachment.removed_at IS NULL)
                  AND EXISTS(SELECT 1 FROM run_requests request WHERE request.run_id=run.id
                    AND request.command_id=run.command_id AND request.actor_id=$9 AND request.request_id=$10))
                  OR (run.run_kind='update' AND instance.state='updating'
                    AND EXISTS(SELECT 1 FROM run_vm_plan_admissions admission WHERE admission.run_id=run.id
                      AND admission.command_id=run.command_id AND admission.instance_id=run.instance_id
                      AND admission.instance_revision_id=run.instance_revision_id
                      AND admission.actor_id=$9 AND admission.request_id=$10
                      AND admission.consumption_kind='provision' AND admission.consumed_at IS NOT NULL
                      AND ROW(admission.vm_id,admission.vm_provider_namespace,admission.vm_host_id)
                        IS NOT DISTINCT FROM ROW(run.vm_id,run.vm_provider_namespace,run.vm_host_id)))))",
        )
        .bind(run.id.as_uuid()).bind(run.instance_id.as_uuid()).bind(run.instance_revision_id.as_uuid())
        .bind(run.release_id.as_uuid()).bind(run.release_agent_id.as_uuid()).bind(run.command_id.as_uuid())
        .bind(match run.kind { RunKind::Normal => "normal", RunKind::Update => "update", RunKind::Invocation => return Err(redacted("Invocation canonical launch is unsupported")) })
        .bind(run.attachment_id.map(uuid::Uuid::from))
        .bind(actor.as_uuid()).bind(request_id.as_uuid()).bind(issued)
        .fetch_one(&mut *tx).await.map_err(error)?;
        if !exact {
            return Err(redacted("exact canonical caller provenance unavailable"));
        }
        sqlx::query("SELECT set_config('hephaestus.actor_id',$1,true),set_config('hephaestus.subject_type','user',true),set_config('hephaestus.request_id',$2,true)")
            .bind(actor.to_string()).bind(request_id.to_string()).execute(&mut *tx).await.map_err(error)?;
        let caller = match run.kind {
            RunKind::Normal => (
                Permission::CanExecute,
                ObjectRef::new(
                    ObjectType::AgentAttachment,
                    run.attachment_id
                        .ok_or_else(|| redacted("normal attachment missing"))?
                        .as_uuid(),
                ),
            ),
            RunKind::Invocation => {
                return Err(redacted("Invocation canonical launch is unsupported"));
            }
            RunKind::Update => (
                Permission::CanUpdate,
                ObjectRef::new(ObjectType::AgentInstance, run.instance_id.as_uuid()),
            ),
        };
        for (permission, object) in [
            (
                Permission::CanUse,
                ObjectRef::new(ObjectType::ReleaseAgent, run.release_agent_id.as_uuid()),
            ),
            caller,
        ] {
            let decision = self
                .authorizer
                .check(&mut tx, Subject::User(actor), permission, object)
                .await
                .map_err(error)?;
            audit_decision(&mut tx, actor, permission, object, decision, request_id)
                .await
                .map_err(error)?;
            if !decision.is_allowed() {
                tx.commit().await.map_err(error)?;
                return Err(redacted("live canonical launch permission denied"));
            }
        }
        tx.commit().await.map_err(error)?;
        Ok(())
    }
}
