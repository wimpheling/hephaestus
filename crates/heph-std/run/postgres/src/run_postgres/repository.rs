use async_trait::async_trait;
use run_domain::{CancelRun, FreshRunExecutionProfile, Run, StartRun};
use run_orchestrator::{CreateRunResult, RepositoryError, RunRepository, StoredVmEvent};
use runtime_types::{EventId, RunId};
use serde_json::Value;
use sqlx::{PgPool, Postgres, Transaction};
use time::OffsetDateTime;

use super::errors::storage;
use super::model::RunRow;

/// `PostgreSQL` implementation of [`RunRepository`].
#[derive(Clone)]
pub struct PgRunRepository {
    pub(crate) pool: PgPool,
    pub(crate) cleanup_wake_scope: Option<run_domain::RunCleanupHostId>,
    pub(crate) vm_scope: Option<run_domain::RunCleanupHostId>,
    pub(crate) execution_profile: Option<FreshRunExecutionProfile>,
    pub(crate) invocation_lease_runtime: bool,
    pub(crate) invocation_configuration: Option<release_service::InstanceExecutionConfiguration>,
    pub(crate) legacy_vm_scope: Option<run_domain::LegacyVmPlacementScope>,
}

impl PgRunRepository {
    /// Creates a repository using an existing connection pool.
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self {
            pool,
            vm_scope: None,
            cleanup_wake_scope: None,
            execution_profile: None,
            legacy_vm_scope: None,
            invocation_configuration: None,
            invocation_lease_runtime: false,
        }
    }

    /// Selects explicit fresh producer SQL for the reviewed profile schema.
    ///
    /// The ordinary constructor retains the public107 SQL shape. This option
    /// never reclassifies a persisted Run. Owned admission also requires the
    /// exact provider scope configured by [`Self::with_vm_owner_scope`].
    #[must_use]
    pub const fn with_execution_profile(mut self, profile: FreshRunExecutionProfile) -> Self {
        self.execution_profile = Some(profile);
        self
    }

    /// Configures the exact persistent VM owner for atomic worker admission.
    #[must_use]
    pub fn with_vm_owner_scope(mut self, scope: run_domain::RunCleanupHostId) -> Self {
        self.vm_scope = Some(scope);
        self
    }

    /// Configures the actual managed provider for prospective Legacy placement.
    /// Explicit `LegacyScalar` is required; no existing Run can be adopted.
    #[must_use]
    pub fn with_legacy_vm_placement(mut self, scope: run_domain::LegacyVmPlacementScope) -> Self {
        self.legacy_vm_scope = Some(scope);
        self
    }

    /// Selects private117 consumption of preexisting qualified115 Invocation runs.
    ///
    /// Default OFF; no schema probe or profile promotion is performed. Requires
    /// explicit owned canonical profile and matching actual provider composition.
    /// This metadata option grants no source, mount or physical provider authority.
    #[must_use]
    pub fn with_qualified_invocation_configuration(
        mut self,
        configuration: release_service::InstanceExecutionConfiguration,
    ) -> Self {
        self.invocation_configuration = Some(configuration);
        self
    }

    /// Selects private119 Invocation lease/progress metadata explicitly.
    ///
    /// Defaults off. Requires the existing checked configuration and actual
    /// configured provider scope; it confers no caller/physical authority.
    /// Private117-only composition stays queued-only. Starting/Running still
    /// require a later qualified session group and remain unsupported here.
    #[must_use]
    pub const fn with_qualified_invocation_lease_runtime(mut self) -> Self {
        self.invocation_lease_runtime = true;
        self
    }

    /// Enables exact original-removal cleanup metadata for the configured owner.
    ///
    /// Defaults off. This private-schema option requires an explicit canonical
    /// execution profile and matching actual VM scope. It grants no provider IO.
    #[must_use]
    pub fn with_qualified_cleanup_wake(mut self, scope: run_domain::RunCleanupHostId) -> Self {
        self.cleanup_wake_scope = Some(scope);
        self
    }

    pub(super) async fn locked_run(
        transaction: &mut Transaction<'_, Postgres>,
        run_id: RunId,
    ) -> Result<RunRow, RepositoryError> {
        sqlx::query_as::<_, RunRow>("SELECT * FROM runs WHERE id = $1 FOR UPDATE")
            .bind(run_id.as_uuid())
            .fetch_optional(&mut **transaction)
            .await
            .map_err(storage)?
            .ok_or(RepositoryError::NotFound(run_id))
    }

    pub(super) async fn append_event_tx(
        transaction: &mut Transaction<'_, Postgres>,
        run_id: RunId,
        event_type: &str,
        payload: Value,
        occurred_at: OffsetDateTime,
    ) -> Result<(), RepositoryError> {
        let sequence: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(sequence), 0) + 1 FROM run_events WHERE run_id = $1",
        )
        .bind(run_id.as_uuid())
        .fetch_one(&mut **transaction)
        .await
        .map_err(storage)?;
        let event_id = EventId::new();
        sqlx::query(
            "INSERT INTO run_events
             (id, run_id, sequence, event_type, payload, occurred_at)
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(event_id.as_uuid())
        .bind(run_id.as_uuid())
        .bind(sequence)
        .bind(event_type)
        .bind(&payload)
        .bind(occurred_at)
        .execute(&mut **transaction)
        .await
        .map_err(storage)?;
        Ok(())
    }
}

#[async_trait]
impl RunRepository for PgRunRepository {
    async fn load_cleanup_wake(
        &self,
        request: &run_orchestrator::RunCleanupWakeRequest,
        scope: &run_domain::RunCleanupHostId,
    ) -> Result<run_orchestrator::RunCleanupWakeSnapshot, RepositoryError> {
        self.read_qualified_cleanup_wake(request, scope).await
    }

    async fn prepare_cleanup_wake(
        &self,
        request: &run_orchestrator::RunCleanupWakeRequest,
        scope: &run_domain::RunCleanupHostId,
    ) -> Result<run_orchestrator::RunCleanupWakeSnapshot, RepositoryError> {
        self.prepare_qualified_cleanup_wake(request, scope).await
    }

    async fn create_run(&self, command: &StartRun) -> Result<CreateRunResult, RepositoryError> {
        if self.legacy_vm_scope.is_some() {
            return Err(RepositoryError::InvalidData(
                "configured Legacy placement requires explicit planned-create port",
            ));
        }
        self.create_run_impl(command).await
    }

    async fn create_run_with_legacy_placement(
        &self,
        command: &StartRun,
        scope: &run_domain::LegacyVmPlacementScope,
    ) -> Result<CreateRunResult, RepositoryError> {
        self.create_legacy_placed_run(command, scope).await
    }

    async fn consume_legacy_vm_placement(
        &self,
        command: &StartRun,
        scope: &run_domain::LegacyVmPlacementScope,
        kind: run_domain::LegacyVmPlacementConsumption,
    ) -> Result<run_domain::LegacyVmPlacement, RepositoryError> {
        self.consume_legacy_placement(command, scope, kind).await
    }

    async fn legacy_vm_placement(
        &self,
        run_id: RunId,
        scope: &run_domain::LegacyVmPlacementScope,
    ) -> Result<run_domain::LegacyVmPlacement, RepositoryError> {
        self.load_legacy_placement(run_id, scope).await
    }

    async fn close_legacy_vm_acquisition(
        &self,
        run_id: RunId,
        scope: &run_domain::LegacyVmPlacementScope,
    ) -> Result<run_domain::LegacyVmPlacement, RepositoryError> {
        self.close_legacy_acquisition(run_id, scope).await
    }

    async fn legacy_vm_placement_inventory(
        &self,
        scope: &run_domain::LegacyVmPlacementScope,
    ) -> Result<run_domain::LegacyVmPlacementInventory, RepositoryError> {
        self.inspect_legacy_placements(scope).await
    }

    async fn assert_legacy_vm_placement_open(
        &self,
        run: RunId,
        scope: &run_domain::LegacyVmPlacementScope,
    ) -> Result<run_domain::LegacyVmPlacement, RepositoryError> {
        self.assert_legacy_open(run, scope).await
    }
    async fn legacy_vm_cleanup_completed(
        &self,
        run: RunId,
        scope: &run_domain::LegacyVmPlacementScope,
    ) -> Result<Run, RepositoryError> {
        self.read_legacy_terminal(run, scope).await
    }
    async fn create_run_with_vm_plan(
        &self,
        command: &StartRun,
        scope: &run_domain::RunCleanupHostId,
        vm_id: &vm_trait::VmId,
    ) -> Result<CreateRunResult, RepositoryError> {
        self.create_planned_run(command, scope, vm_id).await
    }

    async fn ensure_runtime_git_provenance(&self, run: &Run) -> Result<(), RepositoryError> {
        self.ensure_runtime_git_provenance_impl(run).await
    }

    async fn get(&self, run_id: RunId) -> Result<Run, RepositoryError> {
        self.load_run(run_id).await
    }

    async fn bind_resources(
        &self,
        run_id: RunId,
        volume_id: Option<runtime_types::VolumeId>,
        lease_id: Option<runtime_types::LeaseId>,
        lease_fencing_token: Option<i64>,
        vm_id: &str,
    ) -> Result<Run, RepositoryError> {
        self.reject_default_invocation(run_id).await?;
        self.bind_resources_impl(run_id, volume_id, lease_id, lease_fencing_token, vm_id)
            .await
    }

    async fn transition(
        &self,
        run_id: RunId,
        next: run_domain::RunState,
        exit: Option<&vm_trait::VmExit>,
        failure: Option<&str>,
    ) -> Result<Run, RepositoryError> {
        self.reject_default_invocation(run_id).await?;
        self.transition_impl(run_id, next, exit, failure).await
    }

    async fn append_vm_event(
        &self,
        run_id: RunId,
        event: StoredVmEvent,
    ) -> Result<(), RepositoryError> {
        self.append_vm_event_impl(run_id, event).await
    }

    async fn classify_invocation_cancel(
        &self,
        run: RunId,
        scope: &vm_trait::VmProviderOwnerScope,
    ) -> Result<run_orchestrator::InvocationCancelEvidence, RepositoryError> {
        self.classify_invocation_cancel_impl(run, scope).await
    }

    async fn prepare_invocation_cancel_cleanup(
        &self,
        run: RunId,
        scope: &vm_trait::VmProviderOwnerScope,
    ) -> Result<run_orchestrator::InvocationCancelPreparation, RepositoryError> {
        self.prepare_invocation_cancel_cleanup_impl(run, scope)
            .await
    }

    async fn request_cancel(&self, command: &CancelRun) -> Result<bool, RepositoryError> {
        self.request_cancel_impl(command).await
    }

    async fn recoverable_runs(&self) -> Result<Vec<Run>, RepositoryError> {
        self.recoverable_runs_impl().await
    }
}
