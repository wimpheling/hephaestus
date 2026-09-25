use super::super::{UiBootstrapConfig, UiBootstrapState};
use async_trait::async_trait;
use release_domain::{
    UiInstallationGenerationId, UiInstallationId,
    ui_browser::{UiBrowserRoute, UiBrowserSessionId},
};
use release_service::ui_browser_host::{UiGenerationHost, UiNamespace, UiPublicPort};
use release_service::ui_browser_serving::{
    ActiveUiGenerationHost, UiGenerationHostResolver, UiHostLookupError,
};
use release_service::{UiBrowserHandoffError, UiBrowserSessionStore};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Clone)]
pub(super) struct FakeHostResolver {
    pub(super) generation_id: UiInstallationGenerationId,
    pub(super) calls: Arc<AtomicUsize>,
}

#[async_trait]
impl UiGenerationHostResolver for FakeHostResolver {
    async fn resolve_active_generation_host(
        &self,
        host: UiGenerationHost,
    ) -> Result<Option<ActiveUiGenerationHost>, UiHostLookupError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(
            (host.generation_id() == self.generation_id).then_some(ActiveUiGenerationHost {
                generation_id: self.generation_id,
            }),
        )
    }
}

pub(super) struct FakeSessions {
    pub(super) exchanges: Arc<AtomicUsize>,
    pub(super) reject: bool,
}

#[async_trait]
impl UiBrowserSessionStore for FakeSessions {
    async fn create_ui_browser_handoff(
        &self,
        _command: release_service::CreateUiBrowserHandoff,
    ) -> Result<release_service::CreatedUiBrowserHandoff, UiBrowserHandoffError> {
        panic!("bootstrap test must not issue handoffs")
    }

    async fn exchange_ui_browser_handoff(
        &self,
        command: release_service::ExchangeUiBrowserHandoff,
    ) -> Result<release_service::CreatedUiBrowserSession, UiBrowserHandoffError> {
        self.exchanges.fetch_add(1, Ordering::SeqCst);
        if self.reject {
            return Err(UiBrowserHandoffError::InvalidOrExpired);
        }
        Ok(release_service::CreatedUiBrowserSession {
            context: release_service::UiBrowserSessionContext {
                session_id: UiBrowserSessionId::from_uuid(Uuid::new_v4()),
                parent_session_id: identity_domain::BrowserSessionId::from_uuid(Uuid::new_v4()),
                actor_id: identity_domain::UserId::from_uuid(Uuid::new_v4()),
                organization_id: forge_domain::OrganizationId::from_uuid(Uuid::new_v4()),
                installation_id: UiInstallationId::from_uuid(Uuid::new_v4()),
                generation_id: command.expected_generation_id,
                route: UiBrowserRoute::parse("schema-ui").expect("route"),
                expires_at: OffsetDateTime::now_utc() + time::Duration::hours(1),
            },
        })
    }

    async fn authenticate_ui_browser_session(
        &self,
        _command: release_service::AuthenticateUiBrowserSession,
    ) -> Result<release_service::UiBrowserSessionContext, release_service::UiBrowserSessionError>
    {
        panic!("bootstrap test must not authenticate content")
    }
}

pub(super) fn bootstrap_state(
    generation_id: UiInstallationGenerationId,
    host_calls: Arc<AtomicUsize>,
    exchange_calls: Arc<AtomicUsize>,
) -> (Arc<UiBootstrapState>, String) {
    bootstrap_state_with_rejection(generation_id, host_calls, exchange_calls, false)
}

pub(super) fn bootstrap_state_with_rejection(
    generation_id: UiInstallationGenerationId,
    host_calls: Arc<AtomicUsize>,
    exchange_calls: Arc<AtomicUsize>,
    reject: bool,
) -> (Arc<UiBootstrapState>, String) {
    let namespace = UiNamespace::parse("ui.app.example").expect("namespace");
    let port = UiPublicPort::https_default();
    let host = UiGenerationHost::from_generation_id(generation_id);
    let authority = host.authority(&namespace, port);
    let state = UiBootstrapState::new(
        Arc::new(FakeHostResolver {
            generation_id,
            calls: host_calls,
        }),
        Arc::new(FakeSessions {
            exchanges: exchange_calls,
            reject,
        }),
        UiBootstrapConfig::new(namespace, port, "https://app.example").expect("config"),
        Arc::new(crate::ui_audit::NoopAuditSink),
    );
    (Arc::new(state), authority)
}
