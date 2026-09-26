mod mapping;
mod notification;
mod parsing;
mod rows;
mod store;
mod transitions;

mod prelude {
    pub use authz_domain::{ObjectRef, ObjectType, Permission, Subject};
    pub use identity_domain::AuthenticatedIdentity;
    pub use registry_domain::{
        ImmutableManifestReference, NamespaceClaim, OciDescriptor, OciMediaType,
        PlatformDescriptor, PlatformImageKey, PolicyVersion, PublicationIntent,
        PublicationIntentId, PublicationLifecycleError, PublicationState, RegistryAuthority,
        RegistryNamespace, RegistryNotificationBacklog, RegistryOperationalMetrics, RegistryOwner,
        RegistryRetentionSnapshot, RegistryValueError, Sha256Digest, SupplyChainEvidence,
        SupplyChainPolicy, SupplyChainReferrer, SupplyChainReferrerKind, VerifiedPublication,
    };
    pub use sqlx::{FromRow, PgPool, Postgres, Transaction, postgres::PgPoolOptions};
    pub use std::time::Duration;
    pub use time::OffsetDateTime;
    pub use uuid::Uuid;
}

pub use notification::{
    ClaimedRegistryNotification, NewRegistryNotification, NotificationCompletion,
    RegistryNotificationAction, RegistryNotificationReceipt, RegistryNotificationTarget,
    RegistryStoreError,
};
pub use store::{PgRegistryStore, connect};
