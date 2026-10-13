//! Hephaestus single-node forge and agent-runtime daemon.

#[path = "hephaestusd/environment.rs"]
mod environment;
#[path = "hephaestusd/root_images.rs"]
mod root_images;
#[path = "hephaestusd/secrets.rs"]
mod secrets;
#[path = "hephaestusd/support.rs"]
mod support;

#[cfg(test)]
#[path = "hephaestusd/manifest_tests.rs"]
mod manifest_tests;
#[cfg(test)]
#[path = "hephaestusd/tests.rs"]
mod tests;

use environment::environment_config;
use hephaestus_app::{AppConfig, AppError, HephaestusApp, ManagedBackendSupervisor};
use std::{error::Error, time::Duration};

#[tokio::main]
// The owning handle is consumed by awaited shutdown; an extra drop would use a moved value.
#[allow(clippy::significant_drop_tightening)]
async fn main() -> Result<(), Box<dyn Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    let config = environment_config()?;
    let mut supervisor = ManagedBackendSupervisor::start();
    // Collect initialization/start/shutdown errors without discarding the root
    // service. A Held shutdown is never propagated out of this runtime owner.
    let outcome = Box::pin(run_daemon(config, &supervisor)).await;
    retain_until_stopped(&mut supervisor).await;
    if let Err(cause) = &outcome.0 {
        tracing::error!(error = %cause, "daemon application outcome after supervisor Stopped");
    }
    if let Err(cause) = &outcome.1 {
        tracing::error!(error = %cause, "daemon signal outcome after supervisor Stopped");
    }
    outcome.0?;
    outcome.1?;
    Ok(())
}

// The owning Running handle is consumed by shutdown; an extra drop is invalid.
#[allow(clippy::significant_drop_tightening)]
async fn run_daemon(
    config: AppConfig,
    supervisor: &ManagedBackendSupervisor,
) -> (Result<(), AppError>, std::io::Result<()>) {
    let app = match Box::pin(HephaestusApp::build_with_managed_supervisor(
        config, supervisor,
    ))
    .await
    {
        Ok(app) => app,
        Err(cause) => return (Err(cause), Ok(())),
    };
    let running = match Box::pin(app.start()).await {
        Ok(running) => running,
        Err(cause) => return (Err(cause), Ok(())),
    };
    eprintln!("hephaestusd ready at http://{}", running.http_addr());
    let signal = tokio::signal::ctrl_c().await;
    let shutdown = running.shutdown().await;
    // Observe both outcomes even when the signal wait fails.
    (shutdown, signal)
}

async fn retain_until_stopped(supervisor: &mut ManagedBackendSupervisor) {
    loop {
        match supervisor.shutdown(Duration::from_secs(10)).await {
            Ok(()) => return,
            Err(cause) => {
                tracing::error!(error = %cause, status = ?supervisor.status(),
                    "managed supervisor remains held; runtime and original custody retained");
            }
        }
        wait_for_managed_retry(supervisor).await;
    }
}

async fn wait_for_managed_retry(supervisor: &mut ManagedBackendSupervisor) {
    // No automatic acquisition retry. A subsequent operator interrupt asks
    // for one supported SAME-owner pass; unsupported held phases stay held.
    tokio::select! {
        changed = supervisor.changed() => {
            if let Err(cause) = changed {
                tracing::error!(error = %cause, "managed diagnostic wait failed");
                tokio::time::sleep(Duration::from_secs(30)).await;
            }
        }
        signal = tokio::signal::ctrl_c() => {
            handle_managed_retry_signal(supervisor, signal).await;
        }
        () = tokio::time::sleep(Duration::from_secs(30)) => {}
    }
}

async fn handle_managed_retry_signal(
    supervisor: &ManagedBackendSupervisor,
    signal: std::io::Result<()>,
) {
    match signal {
        Ok(()) => {
            if let Err(cause) = supervisor.retry_held().await {
                tracing::warn!(error = %cause, "explicit managed retry remains held");
            }
        }
        Err(cause) => {
            tracing::error!(error = %cause, "operator signal unavailable; retaining custody");
            tokio::time::sleep(Duration::from_secs(30)).await;
        }
    }
}
