use crate::PostgresBrowserSessionStore;
use async_trait::async_trait;
use identity_application::{
    BrowserSessionAuthenticationError, BrowserSessionStore, CreateBrowserSession,
    CreateBrowserSessionError, CreatedBrowserSession, RevokeBrowserSession,
    RevokeBrowserSessionError, RevokedBrowserSession,
};
use identity_domain::{BrowserSessionMetadata, BrowserSessionSid, UserId};

#[async_trait]
impl BrowserSessionStore for PostgresBrowserSessionStore {
    async fn create_browser_session(
        &self,
        command: CreateBrowserSession,
    ) -> Result<CreatedBrowserSession, CreateBrowserSessionError> {
        Self::create_browser_session(self, command).await
    }

    async fn authenticate_browser_session(
        &self,
        user_id: UserId,
        sid: BrowserSessionSid,
    ) -> Result<BrowserSessionMetadata, BrowserSessionAuthenticationError> {
        Self::authenticate_browser_session(self, user_id, sid).await
    }

    async fn revoke_browser_session(
        &self,
        command: RevokeBrowserSession,
    ) -> Result<RevokedBrowserSession, RevokeBrowserSessionError> {
        Self::revoke_browser_session(self, command).await
    }
}
