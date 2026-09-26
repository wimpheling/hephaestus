use std::io;
use std::time::Duration;

use vm_libkrun::protocol::PrivateHttpServiceMessage;

use super::MAX_SERVICE_TIMEOUT;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ServiceConfig {
    pub loopback_port: u16,
    pub max_connections: usize,
    pub connect_timeout: Duration,
}

impl TryFrom<&PrivateHttpServiceMessage> for ServiceConfig {
    type Error = io::Error;

    fn try_from(message: &PrivateHttpServiceMessage) -> Result<Self, Self::Error> {
        if !(1024..=u16::MAX).contains(&message.loopback_port) {
            return Err(invalid("loopback port must be between 1024 and 65535"));
        }
        if !(1..=64).contains(&message.max_connections) {
            return Err(invalid("service connection limit must be between 1 and 64"));
        }
        let timeout = Duration::from_millis(message.connect_timeout_ms);
        if timeout.is_zero() || timeout > MAX_SERVICE_TIMEOUT {
            return Err(invalid(
                "service connect timeout must be between 1ms and 30s",
            ));
        }
        Ok(Self {
            loopback_port: message.loopback_port,
            max_connections: message.max_connections as usize,
            connect_timeout: timeout,
        })
    }
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}
