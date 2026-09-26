//! Post-commit ingress response-loss probe for the cooking acceptance path.
//!
//! The proxy forwards one exact bounded request to real Caddy, waits for the
//! complete response, then closes the client socket before sending response
//! bytes. SQL is read-only evidence for durable publication and delivery.

#[path = "ingress_loss/exercise.rs"]
mod exercise;
#[path = "ingress_loss/http.rs"]
mod http;
#[path = "ingress_loss/proxy.rs"]
mod proxy;
#[cfg(test)]
#[path = "ingress_loss/tests.rs"]
mod tests;

pub use exercise::exercise;
pub(crate) use http::{caddy_host, read_http_message};
pub(crate) use proxy::MAX_HTTP_MESSAGE_BYTES;
pub use proxy::{IngressLossContext, IngressLossProxy};
