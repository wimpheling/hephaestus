use run_domain::FreshRunExecutionProfile;
use run_orchestrator::RepositoryError;
use runtime_types::RunId;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::{PgRunRepository, errors::storage};

/// Why a persisted Run cannot authorize automated physical recovery.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfiledRunRecoveryHoldReason {
    /// A historical or different profile cannot be promoted by this worker.
    ProfileMismatch,
    /// The Run has no recorded VM identity; no UUID fallback is permitted.
    MissingVmIdentity,
    /// A scalar state volume's host does not prove the VM provider placement.
    LegacyProviderPlacementUnproven,
    /// A zero-volume scalar VM has no persisted physical placement evidence.
    LegacyStatelessPlacementUnproven,
    /// Canonical ownership is missing or differs from the configured worker.
    OwnedProviderScopeUnresolved,
}

/// Safe inspection of held recovery work; this grants no cleanup authority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfiledRunRecoveryHold {
    /// Stable Run identifier; no workload or provider path is exposed.
    pub run_id: RunId,
    /// Closed domain reason for retaining resources and fences.
    pub reason: ProfiledRunRecoveryHoldReason,
}

#[derive(sqlx::FromRow)]
struct Candidate {
    id: Uuid,
    execution_profile: String,
    vm_id: Option<String>,
    requires_state: bool,
    vm_provider_namespace: Option<String>,
    vm_host_id: Option<String>,
    vm_binding_kind: Option<String>,
}

impl PgRunRepository {
    /// Lists held profiled recovery work without guessing physical placement.
    ///
    /// This inspection does not guard the old core scalar stale-lease loop.
    /// Profile-aware recovery composition remains required before activation.
    ///
    /// # Errors
    /// Rejects the public107 constructor or a pool without the worker role.
    pub async fn profiled_recovery_holds(
        &self,
    ) -> Result<Vec<ProfiledRunRecoveryHold>, RepositoryError> {
        let configured = self.execution_profile.ok_or(RepositoryError::InvalidData(
            "profile recovery inspection requires explicit configuration",
        ))?;
        let mut tx = self.profile_inventory_transaction().await?;
        let rows:Vec<Candidate>=sqlx::query_as("SELECT id,execution_profile,vm_id,requires_state,vm_provider_namespace,vm_host_id,vm_binding_kind FROM runs WHERE state<>'cleaned_up' ORDER BY created_at,id")
            .fetch_all(&mut *tx).await.map_err(storage)?;
        tx.commit().await.map_err(storage)?;
        Ok(rows
            .into_iter()
            .filter_map(|row| {
                self.hold_reason(&row, configured)
                    .map(|reason| ProfiledRunRecoveryHold {
                        run_id: RunId::from_uuid(row.id),
                        reason,
                    })
            })
            .collect())
    }

    pub(crate) async fn profile_inventory_transaction(
        &self,
    ) -> Result<Transaction<'_, Postgres>, RepositoryError> {
        let mut tx = self.pool.begin().await.map_err(storage)?;
        let role: String = sqlx::query_scalar("SELECT current_user::text")
            .fetch_one(&mut *tx)
            .await
            .map_err(storage)?;
        if role != "hephaestus_worker" {
            return Err(RepositoryError::InvalidData(
                "profile inventory requires worker role",
            ));
        }
        Ok(tx)
    }

    fn hold_reason(
        &self,
        row: &Candidate,
        configured: FreshRunExecutionProfile,
    ) -> Option<ProfiledRunRecoveryHoldReason> {
        use ProfiledRunRecoveryHoldReason as Reason;
        if row.execution_profile != configured.as_str() {
            return Some(Reason::ProfileMismatch);
        }
        if row.vm_id.is_none() {
            return Some(Reason::MissingVmIdentity);
        }
        if configured == FreshRunExecutionProfile::LegacyScalar {
            return Some(if row.requires_state {
                Reason::LegacyProviderPlacementUnproven
            } else {
                Reason::LegacyStatelessPlacementUnproven
            });
        }
        let Some(scope) = self.vm_scope.as_ref() else {
            return Some(Reason::OwnedProviderScopeUnresolved);
        };
        if row.vm_provider_namespace.as_deref() != Some(scope.provider_namespace())
            || row.vm_host_id.as_deref() != Some(scope.host_id())
            || row.vm_binding_kind.as_deref() != Some("planned")
            || row.vm_id.as_deref() != Some(row.id.to_string().as_str())
        {
            return Some(Reason::OwnedProviderScopeUnresolved);
        }
        None
    }
}
