//! Exact normal-run typed mount checks, separate from generic snapshot ceilings.

use runtime_authority::RuntimeAuthorityError;
use runtime_types::RunId;
use volume_domain::VolumeMountScope;

use super::{PgRuntimeSessionRepository, common::storage};

impl PgRuntimeSessionRepository {
    /// Rechecks an exact active normal-run revision and explicit mount grant.
    ///
    /// Callers must invoke this before lease acquisition, before VM start, and
    /// during heartbeat. A denial requires bounded VM stop/fencing before lease
    /// release; this query does not stop or unmount a running VM.
    ///
    /// # Errors
    ///
    /// Returns a safe persistence failure on storage error, and `false` for any
    /// stale binding, mode, release hash, source authority, or revoked grant.
    pub async fn volume_mount_authorized(
        &self,
        run_id: RunId,
        scope: &VolumeMountScope,
    ) -> Result<bool, RuntimeAuthorityError> {
        sqlx::query_scalar(
            "SELECT EXISTS(
                SELECT 1 FROM agent_instance_volume_mount_grants mount_grant
                JOIN agent_instance_revision_volume_bindings binding
                  ON binding.instance_revision_id=mount_grant.instance_revision_id
                 AND binding.slot_key=mount_grant.slot_key
                 AND binding.volume_id=mount_grant.volume_id
                 AND binding.access_mode=mount_grant.access_mode
                JOIN agent_instance_revisions revision
                  ON revision.id=mount_grant.instance_revision_id
                 AND revision.instance_id=mount_grant.instance_id
                 AND revision.release_agent_id=mount_grant.release_agent_id
                JOIN agent_instances instance ON instance.id=revision.instance_id
                JOIN release_agents agent ON agent.id=revision.release_agent_id
                JOIN releases release ON release.id=agent.release_id
                JOIN runs run ON run.instance_id=instance.id AND run.instance_revision_id=revision.id
                WHERE run.id=$1 AND mount_grant.instance_id=$2
                  AND mount_grant.instance_revision_id=$3 AND mount_grant.release_agent_id=$4
                  AND mount_grant.slot_key=$5 AND mount_grant.volume_id=$6
                  AND mount_grant.access_mode=$7 AND mount_grant.release_contract_hash=$8
                  AND agent.runtime_contract_hash=mount_grant.release_contract_hash
                  AND run.release_agent_id=agent.id AND run.release_id=release.id
                  AND run.run_kind='normal' AND run.state IN ('leasing_volume','provisioning','starting','running')
                  AND instance.state IN ('active','update_rejected')
                  AND instance.active_revision_id=revision.id
                  AND revision.runnable AND release.state='published'
                  AND NOT EXISTS(SELECT 1 FROM agent_instance_volume_mount_revocations revoked
                      WHERE revoked.grant_id=mount_grant.id)
                  AND private_volume_mount_binding_is_declared(revision.id,binding.slot_key)
                  AND private_volume_mount_source_is_live(mount_grant.created_by,mount_grant.volume_id)
                  AND check_permission('agent_instance',instance.id::text,'agent_attach',
                                       'state_volume',mount_grant.volume_id::text)=1
             )",
        ).bind(run_id.as_uuid()).bind(scope.instance_id().as_uuid())
        .bind(scope.revision_id().as_uuid()).bind(scope.release_agent_id().as_uuid())
        .bind(scope.slot().as_str()).bind(scope.volume_id().as_uuid())
        .bind(scope.access_mode().as_str()).bind(scope.release_contract_hash().as_bytes().as_slice())
        .fetch_one(&self.pool).await.map_err(storage)
    }
}
