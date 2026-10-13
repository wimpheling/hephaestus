//! Complete selected held fences; an empty set is checked explicitly.

use std::collections::BTreeSet;
use volume_domain::VolumeSelectionOrigin;

use super::{OffsetDateTime, PgRunAuthorityManager, Run, RunAuthorityError, denied};

impl PgRunAuthorityManager {
    pub(in crate::runtime_authority_manager) async fn invocation_lease_expiries(
        &self,
        run: &Run,
    ) -> Result<Vec<OffsetDateTime>, RunAuthorityError> {
        let qualified = self
            .invocation
            .as_ref()
            .ok_or_else(|| denied("qualified Invocation composition is unavailable"))?;
        let selections = qualified
            .volumes
            .load_run_selections(run.id)
            .await
            .map_err(|error| denied(error.to_string()))?;
        let identity = selections.identity();
        if identity.run_id() != run.id
            || identity.instance_id() != run.instance_id
            || identity.revision_id() != run.instance_revision_id
            || identity.release_id() != run.release_id
            || identity.release_agent_id() != run.release_agent_id
            || selections
                .selections()
                .iter()
                .any(|item| item.origin() == VolumeSelectionOrigin::LegacyOrigin)
        {
            return Err(denied(
                "Invocation complete selections differ from the stored Run",
            ));
        }
        qualified
            .volumes
            .preflight_run(&selections)
            .await
            .map_err(|error| denied(error.to_string()))?;
        let leases = qualified
            .volumes
            .leases_for_run(run.id)
            .await
            .map_err(|error| denied(error.to_string()))?;
        let now = OffsetDateTime::now_utc();
        let mut lease_ids = BTreeSet::new();
        if leases.len() != selections.selections().len()
            || leases
                .iter()
                .zip(selections.selections())
                .any(|(held, selected)| {
                    let lease = held.lease();
                    held.selection() != Some(selected)
                        || lease.run_id != run.id
                        || lease.volume_id != selected.scope().volume_id()
                        || lease.host_id != qualified.configuration.provider_scope().host_id()
                        || lease.id.as_uuid().is_nil()
                        || lease.fencing_token <= 0
                        || lease.expires_at <= now
                        || !lease_ids.insert(lease.id.as_uuid())
                })
        {
            return Err(denied("Invocation requires every exact current held fence"));
        }
        // Zero selections must still load/preflight and match zero global held
        // rows. B1 live/writer independently proves the frozen empty snapshot.
        Ok(leases.iter().map(|held| held.lease().expires_at).collect())
    }
}
