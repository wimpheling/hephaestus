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
use hephaestus_app::HephaestusApp;
use std::error::Error;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    let config = environment_config()?;
    let running = HephaestusApp::build(config).await?.start().await?;
    eprintln!("hephaestusd ready at http://{}", running.http_addr());
    tokio::signal::ctrl_c().await?;
    running.shutdown().await?;
    Ok(())
}
