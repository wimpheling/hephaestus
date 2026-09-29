use super::{
    Arc, CancellationToken, ClaimedNotification, Duration, GitAuthenticator, InboxDisposition,
    NewRegistryNotification, NotificationAction, NotificationCompletion, NotificationInbox,
    NotificationObservation, OciWorkerError, PgNotificationCompletion, PgRegistryStore,
    PublicationIntents, ReconciliationAction, ReconciliationActionExecutor,
    ReconciliationPortError, RegistryAction, RegistryAuthorizationDecision,
    RegistryAuthorizationError, RegistryInboxError, RegistryNamespace, RegistryNotificationAction,
    RegistryNotificationInbox, RegistryNotificationTarget, RegistryPublisherTokenIssuer,
    RegistryPullTokenProvider, RepositoryActions, RepositoryName, ScopeRequest, TokenSubject,
    UnixTimestamp, ZotClientError, async_trait,
};
use registry_http::RegistryScopeAuthorizer;
use registry_reconciler::{ObservedTarget, RegistryReconciler};
use registry_token::IssuedToken;
use registry_zot::ZotHttpRegistry;

#[derive(Clone)]
pub struct InternalRegistryTokens {
    pub issuer: Arc<registry_token::RegistryTokenIssuer>,
}

impl InternalRegistryTokens {
    fn issue(
        &self,
        namespace: &RegistryNamespace,
        actions: RepositoryActions,
        action_text: &str,
        subject: &str,
    ) -> Result<IssuedToken, ()> {
        let repository = namespace
            .as_str()
            .parse::<RepositoryName>()
            .map_err(|_| ())?;
        let request = ScopeRequest::parse(
            self.issuer.service().as_str(),
            &format!("repository:{repository}:{action_text}"),
        )
        .map_err(|_| ())?;
        let mut authorization = RegistryAuthorizationDecision::deny_all();
        authorization.grant(repository, actions);
        let now =
            u64::try_from(time::OffsetDateTime::now_utc().unix_timestamp()).map_err(|_| ())?;
        self.issuer
            .issue(
                subject.parse::<TokenSubject>().map_err(|_| ())?,
                &request,
                &authorization,
                UnixTimestamp::new(now),
            )
            .map_err(|_| ())
    }
}

#[async_trait]
impl RegistryPublisherTokenIssuer for InternalRegistryTokens {
    async fn issue_pull_push(
        &self,
        intent: &registry_domain::PublicationIntent,
    ) -> Result<IssuedToken, OciWorkerError> {
        self.issue(
            intent.reference().namespace(),
            RepositoryActions::pull_push(),
            "pull,push",
            "workload:repository-builder",
        )
        .map_err(|()| OciWorkerError::RegistryPublication)
    }
}

#[async_trait]
impl RegistryPullTokenProvider for InternalRegistryTokens {
    async fn issue_pull_token(
        &self,
        namespace: &RegistryNamespace,
    ) -> Result<IssuedToken, ZotClientError> {
        self.issue(
            namespace,
            RepositoryActions::pull(),
            "pull",
            "workload:registry-reconciler",
        )
        .map_err(|()| ZotClientError::Unavailable)
    }
}

#[derive(Clone)]
pub struct PostgresRegistryReconciliation {
    pub store: PgRegistryStore,
}

#[async_trait]
impl NotificationInbox for PostgresRegistryReconciliation {
    async fn claim(
        &self,
        lease: Duration,
    ) -> Result<Option<ClaimedNotification>, ReconciliationPortError> {
        self.store
            .claim_notification(lease)
            .await
            .map(|claim| {
                claim.map(|claim| ClaimedNotification {
                    id: claim.id,
                    lease_token: claim.claim_token,
                    repository_path: claim.repository_path,
                    namespace: claim.namespace,
                    target: claim.target.map(|target| ObservedTarget {
                        digest: target.digest,
                        media_type: target.media_type,
                    }),
                })
            })
            .map_err(|_| ReconciliationPortError)
    }

    async fn complete(
        &self,
        claim: &ClaimedNotification,
        completion: NotificationCompletion,
    ) -> Result<(), ReconciliationPortError> {
        let completion = match completion {
            NotificationCompletion::Processed => PgNotificationCompletion::Processed,
            NotificationCompletion::Rejected { failure_code } => {
                PgNotificationCompletion::Rejected { failure_code }
            }
        };
        self.store
            .complete_notification(claim.id, claim.lease_token, completion)
            .await
            .map_err(|_| ReconciliationPortError)
    }
}

#[async_trait]
impl PublicationIntents for PostgresRegistryReconciliation {
    async fn for_namespace(
        &self,
        namespace: &RegistryNamespace,
    ) -> Result<Vec<registry_domain::PublicationIntent>, ReconciliationPortError> {
        self.store
            .list_for_namespace(namespace)
            .await
            .map_err(|_| ReconciliationPortError)
    }

    async fn all(
        &self,
    ) -> Result<Vec<registry_domain::PublicationIntent>, ReconciliationPortError> {
        self.store
            .list_all()
            .await
            .map_err(|_| ReconciliationPortError)
    }
}

#[async_trait]
impl ReconciliationActionExecutor for PostgresRegistryReconciliation {
    async fn apply(&self, action: &ReconciliationAction) -> Result<(), ReconciliationPortError> {
        match action {
            ReconciliationAction::RecordVerified {
                intent_id,
                verification,
            } => {
                self.store
                    .record_verified(*intent_id, verification.clone())
                    .await
                    .map_err(|_| ReconciliationPortError)?;
            }
            ReconciliationAction::MarkMissing { intent_id, reason } => {
                self.store
                    .mark_missing(*intent_id)
                    .await
                    .map_err(|_| ReconciliationPortError)?;
                tracing::warn!(publication_id = %intent_id, ?reason, "registry publication failed closed");
            }
            ReconciliationAction::RestoreVerified {
                intent_id,
                verification,
            } => {
                self.store
                    .restore_verified(*intent_id, verification)
                    .await
                    .map_err(|_| ReconciliationPortError)?;
            }
            ReconciliationAction::ObservedDifferentTarget { namespace } => {
                tracing::warn!(namespace = %namespace, "Zot notification target did not match a publication intent");
            }
            ReconciliationAction::OrphanNamespace { repository_path } => {
                tracing::warn!(
                    repository_path,
                    "Zot notification addressed an unowned namespace"
                );
            }
            ReconciliationAction::Investigate { intent_id, reason } => {
                tracing::warn!(publication_id = %intent_id, ?reason, "registry publication requires investigation");
            }
        }
        Ok(())
    }
}

pub struct PostgresRegistryScopeAuthorizer {
    pub store: PgRegistryStore,
}

#[async_trait]
impl RegistryScopeAuthorizer for PostgresRegistryScopeAuthorizer {
    async fn authorize(
        &self,
        identity: &identity_domain::AuthenticatedIdentity,
        request: &registry_token::ScopeRequest,
    ) -> Result<RegistryAuthorizationDecision, RegistryAuthorizationError> {
        let mut decision = RegistryAuthorizationDecision::deny_all();
        for scope in request.scopes() {
            if !scope.actions().contains(RegistryAction::Pull) {
                continue;
            }
            let Ok(namespace) = RegistryNamespace::parse(scope.repository().as_str().to_owned())
            else {
                continue;
            };
            if self
                .store
                .authorize_user_pull(identity, &namespace)
                .await
                .map_err(|_| RegistryAuthorizationError)?
            {
                // Human token exchange is deliberately pull-only. Trusted
                // publishers receive push grants through the worker boundary.
                decision.grant(
                    scope.repository().clone(),
                    registry_token::RepositoryActions::pull(),
                );
            }
        }
        Ok(decision)
    }
}

pub struct PostgresRegistryNotificationInbox {
    pub store: PgRegistryStore,
}

#[async_trait]
impl RegistryNotificationInbox for PostgresRegistryNotificationInbox {
    async fn ingest(
        &self,
        observation: NotificationObservation,
    ) -> Result<InboxDisposition, RegistryInboxError> {
        let target =
            observation
                .digest()
                .zip(observation.media_type())
                .map(|(digest, media_type)| RegistryNotificationTarget {
                    digest: digest.clone(),
                    media_type: media_type.clone(),
                });
        let receipt = self
            .store
            .ingest_notification(NewRegistryNotification {
                event_key: observation.idempotency_key().as_str().to_owned(),
                repository_path: observation.repository().as_str().to_owned(),
                action: match observation.action() {
                    NotificationAction::Push => RegistryNotificationAction::Push,
                    NotificationAction::Delete => RegistryNotificationAction::Delete,
                },
                target,
                occurred_at: observation.occurred_at(),
                payload_sha256: *observation.payload_sha256().as_bytes(),
            })
            .await
            .map_err(|_| RegistryInboxError)?;
        Ok(if receipt.duplicate {
            InboxDisposition::Duplicate
        } else {
            InboxDisposition::Accepted
        })
    }
}

pub async fn registry_caller_authentication(
    axum::extract::State(authenticator): axum::extract::State<Arc<dyn GitAuthenticator>>,
    mut request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let credential = request
        .headers()
        .get(http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let principal = authenticator
        .authenticate(credential.as_deref(), identity_domain::RequestId::new())
        .await;
    let Ok(principal) = principal else {
        let mut response = axum::response::Response::new(axum::body::Body::empty());
        *response.status_mut() = http::StatusCode::UNAUTHORIZED;
        return response;
    };
    let Some(identity) = principal.human_identity().cloned() else {
        let mut response = axum::response::Response::new(axum::body::Body::empty());
        *response.status_mut() = http::StatusCode::UNAUTHORIZED;
        return response;
    };
    request.headers_mut().remove(http::header::AUTHORIZATION);
    request.extensions_mut().insert(identity);
    next.run(request).await
}

pub async fn registry_reconciliation_loop(
    reconciler: RegistryReconciler<
        PostgresRegistryReconciliation,
        PostgresRegistryReconciliation,
        ZotHttpRegistry<InternalRegistryTokens>,
    >,
    executor: PostgresRegistryReconciliation,
    lease: Duration,
    poll_interval: Duration,
    cancellation: CancellationToken,
) {
    let mut interval = tokio::time::interval(poll_interval);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            () = cancellation.cancelled() => break,
            _ = interval.tick() => {
                if let Err(error) = reconciler.process_next_and_apply(lease, &executor).await {
                    tracing::warn!(%error, "registry notification reconciliation pass failed");
                }
                if let Err(error) = reconciler.reconcile_all_and_apply(&executor).await {
                    // Zot availability is intentionally not forge readiness:
                    // approved consumers remain fail-closed from durable state.
                    tracing::warn!(%error, "registry authoritative reconciliation pass failed");
                }
            }
        }
    }
}
