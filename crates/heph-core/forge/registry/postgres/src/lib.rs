//! `PostgreSQL` durability adapter for the forge-owned OCI registry control plane.
//!
//! The adapter persists ownership and approval decisions only. OCI content is
//! always read from Zot by the verifier before [`PgRegistryStore::record_verified`].

mod registry_store;

pub use registry_store::{
    ClaimedRegistryNotification, NewRegistryNotification, NotificationCompletion, PgRegistryStore,
    RegistryNotificationAction, RegistryNotificationReceipt, RegistryNotificationTarget,
    RegistryStoreError, connect,
};
