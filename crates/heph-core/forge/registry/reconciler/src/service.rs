//! Reconciliation orchestration over the durable and remote ports.

use crate::{
    model::{
        AuthoritativeReconciliation, ClaimedNotification, IntentReconciliation,
        NotificationCompletion, NotificationReduction, ReconciliationAction,
        ReconciliationPortError,
    },
    ports::{NotificationInbox, PublicationIntents, ReconciliationActionExecutor, ZotRegistry},
    reducer::{orphan_reduction, reduce_intent},
};
use registry_domain::PublicationIntent;
use std::time::Duration;

/// Reducer and authoritative reconciliation application service.
pub struct RegistryReconciler<I, P, Z> {
    inbox: I,
    intents: P,
    zot: Z,
}

impl<I, P, Z> RegistryReconciler<I, P, Z> {
    /// Creates the service from explicit storage and Zot ports.
    #[must_use]
    pub const fn new(inbox: I, intents: P, zot: Z) -> Self {
        Self {
            inbox,
            intents,
            zot,
        }
    }
}

impl<I, P, Z> RegistryReconciler<I, P, Z>
where
    I: NotificationInbox,
    P: PublicationIntents,
    Z: ZotRegistry,
{
    /// Claims and reduces at most one notification, then completes its inbox
    /// claim. Publication actions are returned rather than executed.
    ///
    /// # Errors
    ///
    /// Returns when a port is unavailable. In that case the notification is
    /// deliberately not completed and may be retried after its lease expires.
    pub async fn process_next(
        &self,
        lease: Duration,
    ) -> Result<Option<NotificationReduction>, ReconciliationPortError> {
        let Some(claim) = self.inbox.claim(lease).await? else {
            return Ok(None);
        };
        let reduction = self.reduce_claimed(&claim).await?;
        self.inbox
            .complete(&claim, reduction.completion.clone())
            .await?;
        Ok(Some(reduction))
    }

    /// Claims, authoritatively reduces, and applies one observation before
    /// completing its durable inbox claim.
    ///
    /// This is the production path. [`Self::process_next`] remains useful to
    /// consumers that deliberately persist the returned action batch in their
    /// own transaction boundary.
    ///
    /// # Errors
    ///
    /// Returns when a port or action executor is unavailable. No inbox
    /// completion is attempted after an action failure, preserving retry.
    pub async fn process_next_and_apply<E>(
        &self,
        lease: Duration,
        executor: &E,
    ) -> Result<bool, ReconciliationPortError>
    where
        E: ReconciliationActionExecutor,
    {
        let Some(claim) = self.inbox.claim(lease).await? else {
            return Ok(false);
        };
        let reduction = self.reduce_claimed(&claim).await?;
        for intent in &reduction.intents {
            for action in &intent.actions {
                executor.apply(action).await?;
            }
        }
        for action in &reduction.actions {
            executor.apply(action).await?;
        }
        self.inbox.complete(&claim, reduction.completion).await?;
        Ok(true)
    }

    /// Reduces one already claimed notification into unapplied typed actions.
    ///
    /// # Errors
    ///
    /// Returns when durable intents or Zot cannot be read. It does not complete
    /// the claim, allowing the caller to retain retry semantics.
    pub async fn reduce_claimed(
        &self,
        claim: &ClaimedNotification,
    ) -> Result<NotificationReduction, ReconciliationPortError> {
        let Some(namespace) = &claim.namespace else {
            return Ok(orphan_reduction(claim));
        };
        let intents = self.intents.for_namespace(namespace).await?;
        if intents.is_empty() {
            return Ok(orphan_reduction(claim));
        }
        let observed_target_matches = claim.target.as_ref().is_none_or(|target| {
            intents.iter().any(|intent| {
                target.digest == *intent.reference().digest()
                    && target.media_type == *intent.expected_manifest().media_type()
            })
        });
        let mut actions = Vec::new();
        if !observed_target_matches {
            actions.push(ReconciliationAction::ObservedDifferentTarget {
                namespace: namespace.clone(),
            });
        }
        let mut reduced = Vec::with_capacity(intents.len());
        for intent in intents {
            reduced.push(self.reconcile_intent(intent).await?);
        }
        Ok(NotificationReduction {
            notification_id: claim.id,
            completion: NotificationCompletion::Processed,
            intents: reduced,
            actions,
        })
    }

    /// Inspects every durable intent, independent of notifications.
    ///
    /// # Errors
    ///
    /// Returns when durable intents or Zot cannot be read. The caller should
    /// retry the scheduled pass; no lifecycle mutation occurs here.
    pub async fn reconcile_all(
        &self,
    ) -> Result<AuthoritativeReconciliation, ReconciliationPortError> {
        let intents = self.intents.all().await?;
        let mut reduced = Vec::with_capacity(intents.len());
        for intent in intents {
            reduced.push(self.reconcile_intent(intent).await?);
        }
        Ok(AuthoritativeReconciliation { intents: reduced })
    }

    /// Performs a full missed-event reconciliation and applies every proposed
    /// lifecycle action through the authorized executor.
    ///
    /// # Errors
    ///
    /// Returns on the first inspection or action failure. Actions are
    /// idempotent, so the next scheduled pass safely retries the full set.
    pub async fn reconcile_all_and_apply<E>(
        &self,
        executor: &E,
    ) -> Result<AuthoritativeReconciliation, ReconciliationPortError>
    where
        E: ReconciliationActionExecutor,
    {
        let report = self.reconcile_all().await?;
        for intent in &report.intents {
            for action in &intent.actions {
                executor.apply(action).await?;
            }
        }
        Ok(report)
    }

    /// Inspects one intent by its exact immutable Zot digest.
    ///
    /// # Errors
    ///
    /// Returns when Zot cannot be read. A missing digest is returned as a
    /// normal authoritative result and is not an error.
    pub async fn reconcile_intent(
        &self,
        intent: PublicationIntent,
    ) -> Result<IntentReconciliation, ReconciliationPortError> {
        let inspection = self.zot.inspect(intent.reference()).await?;
        Ok(reduce_intent(&intent, inspection))
    }
}
