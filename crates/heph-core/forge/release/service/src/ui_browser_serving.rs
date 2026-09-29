//! Provider-neutral UI host and resource read ports.
//!
//! Implementations belong in `release-postgres`; HTTP handlers receive typed
//! ports and projections rather than SQL pools.

mod authorization;
mod http;
mod projection;
#[cfg(test)]
mod tests;

pub use authorization::{
    UiBrowserRepositoryGitAuthorization, UiBrowserTargetContext, UiBrowserTargetContextProjection,
    UiGitAuthorizationError, UiRepositoryGitAuthorization, UiRepositoryGitOperation,
    UiTargetContextError,
};
pub use http::{
    UiBrowserHttpPath, UiBrowserHttpPathError, UiBrowserHttpRequest, UiGatewayRequestKind,
    UiGatewayRequestProjection,
};
pub use projection::{
    ActiveUiGenerationHost, UiBrowserHttpServingProjection, UiGenerationHostResolver,
    UiHostLookupError, UiServingError, UiServingProjection, UiStaticArtifactProjection,
};
