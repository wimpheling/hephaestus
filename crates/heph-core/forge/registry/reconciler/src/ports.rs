//! Adapter-neutral ports used by registry reconciliation.

use crate::model::{
    ClaimedNotification, NotificationCompletion, ReconciliationAction, ReconciliationPortError,
    ZotInspection,
};
use async_trait::async_trait;
use registry_domain::{ImmutableManifestReference, PublicationIntent, RegistryNamespace};
use std::time::Duration;

/// Durable inbox access required by the notification worker.
#[async_trait]
pub trait NotificationInbox: Send + Sync + 'static {
    /// Claims one notification using the caller's bounded lease duration.
    ///
    /// # Errors
    ///
    /// Returns an opaque storage failure. The observation is left claimed so
    /// it can be retried after its lease expires.
    async fn claim(
        &self,
        lease: Duration,
    ) -> Result<Option<ClaimedNotification>, ReconciliationPortError>;

    /// Marks a claim terminal after its observation was reduced.
    ///
    /// # Errors
    ///
    /// Returns an opaque storage failure when the lease was lost or storage is
    /// unavailable. This method must not mutate publication lifecycle state.
    async fn complete(
        &self,
        claim: &ClaimedNotification,
        completion: NotificationCompletion,
    ) -> Result<(), ReconciliationPortError>;
}

/// Durable source of publication intents for one authoritative pass.
#[async_trait]
pub trait PublicationIntents: Send + Sync + 'static {
    /// Lists every retained intent for a namespace in stable identity order.
    ///
    /// # Errors
    ///
    /// Returns an opaque persistence failure.
    async fn for_namespace(
        &self,
        namespace: &RegistryNamespace,
    ) -> Result<Vec<PublicationIntent>, ReconciliationPortError>;

    /// Lists every retained publication intent in stable identity order.
    ///
    /// This is the missed-event safety net: it runs independently of Zot's
    /// best-effort notification delivery.
    ///
    /// # Errors
    ///
    /// Returns an opaque persistence failure.
    async fn all(&self) -> Result<Vec<PublicationIntent>, ReconciliationPortError>;
}
/// Zot read boundary used by notification and scheduled reconciliation.
#[async_trait]
pub trait ZotRegistry: Send + Sync + 'static {
    /// Reads Zot by the exact immutable reference, never by a tag.
    ///
    /// Implementations must obtain both the referrer list and every
    /// referrer's subject from Zot rather than trusting a callback body.
    ///
    /// # Errors
    ///
    /// Returns an opaque transport or Zot service failure. `Missing` is a
    /// successful authoritative response and is represented by
    /// [`ZotInspection::Missing`].
    async fn inspect(
        &self,
        reference: &ImmutableManifestReference,
    ) -> Result<ZotInspection, ReconciliationPortError>;
}

/// Authorized lifecycle executor for reconciliation actions.
///
/// Implementations normally call the registry PostgreSQL adapter, whose
/// transactions append product events through the committed outbox. Diagnostic
/// actions may be recorded or logged, but must never become approval.
#[async_trait]
pub trait ReconciliationActionExecutor: Send + Sync + 'static {
    /// Applies one idempotent action before the originating inbox claim is
    /// completed.
    ///
    /// # Errors
    ///
    /// Returns an opaque persistence failure. The claim remains leased and is
    /// retried after expiry.
    async fn apply(&self, action: &ReconciliationAction) -> Result<(), ReconciliationPortError>;
}
