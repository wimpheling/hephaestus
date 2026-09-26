//! Provider-neutral secret command, runtime, and broker contracts.

mod broker;
mod commands;
mod errors;
mod traits;

pub use broker::{
    BrokerAdapter, BrokerAdapterError, BrokerRequest, BrokerResponse, BrokerStatus,
    VerifiedBrokeredHttpsRule,
};
pub use commands::{
    AcceptSecretImport, BindSecret, CreateSecret, DeclareBrokeredHttpsRule,
    GrantAndAcceptSecretImport, GrantSecret, IssuedSecretLease, ResolveRunSecrets,
    ResolvedRawSecret, RotateSecret, RuntimeSecretAuthority,
};
pub use errors::{CreatedSecret, SecretServiceError};
pub use traits::{SecretCommandService, SecretDispatchResolver, SecretRuntimeResolver};
