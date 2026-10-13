use crate::{
    AppConfig, AppError, HephaestusApp, ManagedBackendSupervisor, RunningHephaestus, app_build,
    startup,
};

impl HephaestusApp {
    /// Validates configuration and constructs every production dependency.
    ///
    /// This does not bind listeners or activate command/background workers.
    /// Managed preparation requires the explicit process-owned supervisor.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid configuration, incompatible migrations, or
    /// an unavailable required dependency.
    pub async fn build(config: AppConfig) -> Result<Self, AppError> {
        app_build::build(config, None).await
    }

    /// Builds with the daemon-owned managed supervisor already retained.
    ///
    /// Readiness remains pending until qualified recovery and lifecycle wiring.
    /// Caller abandonment releases its lease, never the service's original tuple.
    ///
    /// # Errors
    /// Returns configuration, database, acquisition or construction failures while
    /// the service retains any original managed custody for closing/retry.
    pub async fn build_with_managed_supervisor(
        config: AppConfig,
        service: &ManagedBackendSupervisor,
    ) -> Result<Self, AppError> {
        app_build::build(config, Some(service)).await
    }

    /// Binds HTTP, establishes durable NATS topology, and starts supervised
    /// background workers.
    ///
    /// This returns only after HTTP, command consumption, and outbox
    /// publication have crossed their readiness barriers.
    ///
    /// # Errors
    ///
    /// Returns an error when startup or readiness fails. Already-started tasks
    /// are cancelled and reaped before the error is returned.
    pub async fn start(self) -> Result<RunningHephaestus, AppError> {
        // Keep the large startup state machine off the caller's stack.
        Box::pin(self.start_inner()).await
    }

    async fn start_inner(self) -> Result<RunningHephaestus, AppError> {
        startup::start(self).await
    }
}
