//! Dynamic libkrun FFI boundary and safe VM configuration.

#[path = "ffi/api.rs"]
mod api;
#[path = "ffi/context.rs"]
mod context;
#[path = "ffi/errors.rs"]
mod errors;
#[cfg(test)]
#[path = "ffi/tests.rs"]
mod tests;

pub use context::Context;
pub use errors::FfiError;

#[cfg(test)]
pub use api::KrunApi;
#[cfg(test)]
pub use errors::{deterministic_mac, path_cstring, status};
#[cfg(test)]
use std::sync::Arc;

#[cfg(test)]
fn context_from_api(api: Arc<dyn api::KrunApi>) -> Result<Context, FfiError> {
    context::Context::from_api(api)
}
