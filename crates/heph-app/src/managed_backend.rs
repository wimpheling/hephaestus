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
