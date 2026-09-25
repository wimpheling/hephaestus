//! Dependency-free long-lived HTTP service fixture for the persistent gateway.
//!
//! The process deliberately owns no Hephaestus authority. It listens only on
//! the declared guest-loopback port and serves bounded ordinary HTTP responses.

mod isolation;
mod routing;
mod server;
mod types;

#[cfg(test)]
mod tests;

use std::net::TcpListener;

use types::{SERVICE_ADDRESS, StartupIdentity};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let listener = TcpListener::bind(SERVICE_ADDRESS)?;
    let identity = StartupIdentity::new()?;
    println!(
        "cooking-service ready on {}:{} pid={}",
        SERVICE_ADDRESS.0, SERVICE_ADDRESS.1, identity.pid
    );
    server::run(&listener, identity)?;
    Ok(())
}
