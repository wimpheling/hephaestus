//! Dedicated managed acquisition and retained construction, without worker activation.

#[path = "managed_backend/acquire.rs"]
mod acquire;
#[path = "managed_backend/config.rs"]
mod config;
#[path = "managed_backend/custody.rs"]
mod custody;
#[path = "managed_backend/history.rs"]
mod history;
#[path = "managed_backend/paths.rs"]
mod paths;
#[path = "managed_backend/roots.rs"]
mod roots;
#[path = "managed_backend/services.rs"]
mod services;
#[path = "managed_backend/supervisor.rs"]
mod supervisor;
#[cfg(test)]
#[path = "managed_backend/tests.rs"]
mod tests;
#[path = "managed_backend/validation.rs"]
mod validation;

use crate::{AppConfig, AppError};
pub use config::{ManagedBackendConfig, ManagedRootIntent};
use control_plane_postgres::ControlPlanePool;

pub use supervisor::{
    ManagedBackendSupervisor, ManagedHeldReason, ManagedSupervisorPhase, ManagedSupervisorStatus,
    NodeLease,
};

pub use services::{OwnedTasks, Services};

impl NodeLease {
    pub fn require_recovered(&self) -> Result<(), AppError> {
        self.registry()?.require_recovered()
    }
    pub fn executor(&self) -> Result<std::sync::Arc<crate::ManagedRunCommandExecutor>, AppError> {
        let registry = self.registry()?;
        registry.require_recovered()?;
        let node = registry.node()?;
        custody::executor(&node)
    }
    pub fn original_invocation(
        &self,
    ) -> Result<std::sync::Arc<crate::ManagedInvocationController>, AppError> {
        let controller = self.executor()?.original_invocation();
        controller.validate_owners()?;
        Ok(controller)
    }
    pub fn fork_service_lease(&self) -> Self {
        self.fork()
    }
    pub async fn wait_stopped(&self, timeout: std::time::Duration) -> Result<(), AppError> {
        self.close();
        let registry = self.registry()?;
        tokio::time::timeout(timeout, async {
            let mut status = self.status_receiver();
            loop {
                let current = status.borrow().clone();
                if current.phase == ManagedSupervisorPhase::Stopped {
                    return Ok(());
                }
                if current.phase == ManagedSupervisorPhase::Held && !current.in_flight {
                    return Err(registry.failure());
                }
                status.changed().await.map_err(custody::failure)?;
            }
        })
        .await
        .map_err(|_| {
            AppError::Timeout(
                "managed closing is held/in flight; original services retained".into(),
            )
        })?
    }
}

pub fn start_owned(app: crate::HephaestusApp) -> Result<services::StartupTicket, AppError> {
    let registry = app
        .managed_backend
        .as_ref()
        .ok_or_else(|| invalid("managed original lease missing"))?
        .registry()?;
    registry.require_recovered()?;
    let node = registry.node()?;
    services::spawn(registry, node, app)
}

/// Registers one owned preparation before managed checks and awaits its controllers.
///
/// # Errors
/// Holds wrong configuration/history/roles or failed physical acquisition and
/// initialization. No worker readiness, cross-filesystem commit or cleanup is implied.
pub async fn prepare(
    app: AppConfig,
    admin: &ControlPlanePool,
    worker: &ControlPlanePool,
    application: &ControlPlanePool,
    service: Option<&ManagedBackendSupervisor>,
) -> Result<Option<NodeLease>, AppError> {
    if app.managed_backend.is_none() {
        return Ok(None);
    }
    let service = service
        .ok_or_else(|| invalid("managed configuration requires its process-owned supervisor"))?;
    // OWNED registration precedes every managed validation/history/acquisition.
    service
        .register(
            app,
            crate::ManagedInvocationPools {
                admin: admin.clone(),
                worker: worker.clone(),
                application: application.clone(),
            },
        )?
        .wait()
        .await
        .map(Some)
}

fn invalid(message: &str) -> AppError {
    AppError::Configuration(format!("managed backend preparation: {message}"))
}
