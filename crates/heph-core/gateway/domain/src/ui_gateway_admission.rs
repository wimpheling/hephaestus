//! Boundary for requests arriving from the trusted UI handler.
//!
//! This module deliberately contains no database or credential implementation.
//! The concrete provider belongs in `gateway-postgres` and must resolve the
//! route and gateway revision from the child session, never from UI input.

mod admission;
mod headers;

pub use admission::validate_ui_response;
pub use admission::{
    UiGatewayAdmission, UiGatewayAdmissionError, UiGatewayAdmissionProvider, UiGatewayAuthority,
    UiGatewayRequest, UiGatewayRequestKind, admission_failure_response, prepare_gateway_request,
};
pub use headers::{reject_ui_set_cookie, strip_ui_guest_headers};

#[cfg(test)]
#[path = "ui_gateway_admission/tests.rs"]
mod tests;
