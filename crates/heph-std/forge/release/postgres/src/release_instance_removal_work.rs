//! Durable cancellation requests and grant withdrawal within closure admission.

use authz_postgres::AUTHORIZATION_MODEL_VERSION;
use identity_domain::AuthenticatedIdentity;
use release_service::RequestInstanceRemoval;
use run_domain::CancelRun;
use runtime_types::{CommandId, RunId};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::{ReleaseServiceError, append_event};

pub async fn admit_cancellation(
    tx: &mut Transaction<'_, Postgres>,
    identity: &AuthenticatedIdentity,
    command: &RequestInstanceRemoval,
) -> Result<(), ReleaseServiceError> {
    let grants:Vec<Uuid>=sqlx::query_scalar("SELECT mount_grant.id FROM agent_instance_volume_mount_grants mount_grant WHERE instance_id=$1 AND NOT EXISTS(SELECT 1 FROM agent_instance_volume_mount_revocations revoked WHERE revoked.grant_id=mount_grant.id) ORDER BY mount_grant.id")
        .bind(command.instance_id.as_uuid()).fetch_all(&mut **tx).await?;
    for grant in grants {
        let revoke = Uuid::new_v5(
            &command.removal_id.as_uuid(),
            format!("volume-grant:{grant}").as_bytes(),
        );
        sqlx::query("INSERT INTO agent_instance_volume_mount_revocations(id,grant_id,created_by,request_id,authorization_model_version) VALUES($1,$2,$3,$4,$5) ON CONFLICT(grant_id) DO NOTHING")
            .bind(revoke).bind(grant).bind(identity.user_id.as_uuid()).bind(identity.request_id.as_uuid()).bind(AUTHORIZATION_MODEL_VERSION).execute(&mut **tx).await?;
    }
    let runs: Vec<Uuid> = sqlx::query_scalar("SELECT close_instance_launches($1)")
        .bind(command.removal_id.as_uuid())
        .fetch_all(&mut **tx)
        .await?;
    for run in runs {
        let cancel = CancelRun {
            command_id: CommandId::from_uuid(Uuid::new_v5(
                &command.removal_id.as_uuid(),
                format!("cancel-run:{run}").as_bytes(),
            )),
            run_id: RunId::from_uuid(run),
            reason: String::from("instance removal requested"),
        };
        append_event(
            tx,
            run,
            "heph.run.command.cancel.v1",
            "run.cancel_requested.v1",
            serde_json::to_value(cancel)?,
        )
        .await?;
    }
    Ok(())
}
