//! Strict Legacy inventory reconciliation never performs global resource sweeps.
use super::{OrchestratorError, RunOrchestrator, canonical_cleanup::invalid};
use run_domain::{LegacyVmPlacement, LegacyVmPlacementInventory, RunState};
use std::collections::BTreeSet;
use time::OffsetDateTime;
use volume_trait::ScalarLeaseHistory;

fn check_inventory(inventory: &LegacyVmPlacementInventory) -> Result<(), OrchestratorError> {
    let mut placed = BTreeSet::new();
    for plan in &inventory.placements {
        if !placed.insert(plan.command().run_id) {
            return Err(invalid("duplicate Legacy inventory Run"));
        }
    }
    let mut closed = BTreeSet::new();
    for run in &inventory.closed_runs {
        if !placed.contains(run) || !closed.insert(*run) {
            return Err(invalid(
                "Legacy closure inventory contradicts checked placements",
            ));
        }
    }
    for held in &inventory.held {
        if placed.contains(&held.run_id) {
            return Err(invalid("Legacy inventory marks a placed Run Held"));
        }
    }
    Ok(())
}

impl RunOrchestrator {
    pub(super) async fn recover_legacy_runs(
        &self,
        stale_only: bool,
    ) -> Result<usize, OrchestratorError> {
        let inventory = self
            .repository
            .legacy_vm_placement_inventory(self.legacy_owner()?)
            .await?;
        check_inventory(&inventory)?;
        let mut count = 0;
        for plan in &inventory.placements {
            // A live start owns guest execution; another recovery cannot destroy it.
            let claim = match self.operation_guards.claim_start(plan.command().run_id) {
                Ok(claim) => claim,
                Err(OrchestratorError::RunInProgress(_)) => continue,
                Err(error) => return Err(error),
            };
            let closed = inventory.closed_runs.contains(&plan.command().run_id);
            if self.recover_legacy_plan(plan, closed, stale_only).await? {
                count += 1;
            }
            drop(claim);
        }
        // Held history is preserved and remains inspectable through the port.
        // No global recover() callback is invoked, even when inventory is empty.
        Ok(count)
    }
    async fn recover_legacy_plan(
        &self,
        plan: &LegacyVmPlacement,
        closed: bool,
        stale_only: bool,
    ) -> Result<bool, OrchestratorError> {
        if plan.scope() != self.legacy_owner()? {
            return Err(invalid("foreign Legacy placement appeared as actionable"));
        }
        let run = self.repository.get(plan.command().run_id).await?;
        self.check_legacy_run_pins(&run, plan)?;
        let previous = {
            let plans = self.legacy_plans.lock().await;
            let previous = plans.get(&run.id).cloned();
            drop(plans);
            previous
        };
        if let Some(previous) = previous {
            self.check_legacy_plan_return(run.id, &previous, plan)?;
        }
        self.legacy_plans.lock().await.insert(run.id, plan.clone());
        if run.state == RunState::CleanedUp {
            if stale_only {
                return Ok(false);
            }
            self.legacy_terminal(run.id).await?;
            return Ok(true);
        }
        let history = self.volumes.scalar_lease_history(run.id).await?;
        if let ScalarLeaseHistory::Held(lease)
        | ScalarLeaseHistory::HeldRecovering(lease)
        | ScalarLeaseHistory::Released(lease) = &history
        {
            self.check_legacy_lease(&run, lease)?;
        }
        if stale_only {
            match &history {
                ScalarLeaseHistory::Held(lease)
                    if lease.expires_at <= OffsetDateTime::now_utc() =>
                {
                    self.volumes.begin_recovery(lease).await?;
                }
                ScalarLeaseHistory::HeldRecovering(_) => {}
                _ => return Ok(false),
            }
        } else if !closed
            && matches!(history, ScalarLeaseHistory::NoHistory)
            && matches!(run.state, RunState::Queued | RunState::LeasingVolume)
        {
            // Positive open no-IO eligibility remains available for redelivery.
            if plan.consumption().is_none()
                || self
                    .repository
                    .assert_legacy_vm_placement_open(run.id, self.legacy_owner()?)
                    .await
                    .is_ok()
            {
                return Ok(false);
            }
        }
        self.cleanup_legacy(
            run.id,
            None,
            Some(("supervisor restarted while resources were active", None)),
        )
        .await?;
        Ok(true)
    }
}
