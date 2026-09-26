//! Guest-side private service loopback and vsock bridge.

use std::time::Duration;

const MAX_SERVICE_TIMEOUT: Duration = Duration::from_secs(30);
const SERVICE_IDLE_TIMEOUT: Duration = Duration::from_secs(30);
const JOIN_TIMEOUT: Duration = Duration::from_secs(2);

#[path = "service/config.rs"]
mod config;
#[path = "service/network.rs"]
mod network;
#[path = "service/supervisor.rs"]
mod supervisor;
#[path = "service/transport.rs"]
mod transport;

pub use config::ServiceConfig;
pub use network::bring_up_loopback;
pub use supervisor::ServiceSupervisor;
pub const SERVICE_HOST: &str = "127.0.0.1";
pub const SERVICE_HOST_ENV: &str = "HEPH_SERVICE_HOST";
pub const SERVICE_PORT_ENV: &str = "HEPH_SERVICE_PORT";

#[cfg(test)]
#[path = "service/tests.rs"]
mod tests;

#[cfg(test)]
pub use network::loopback_flags_up;
