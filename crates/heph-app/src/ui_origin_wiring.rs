//! Composition seams for the dedicated loopback UI-origin listener.

mod bridge;
mod router;

#[cfg(test)]
#[path = "ui_origin_wiring/tests.rs"]
mod tests;

pub use bridge::RealUiGatewayDispatcher;
pub use router::bounded_ui_router_with_audit;
