//! Provider-neutral release command DTOs and workflow ports.

mod types;
mod ui_browser;
pub mod ui_browser_host;
pub mod ui_browser_serving;
mod ui_installation;
mod ui_installation_navigation;
mod ui_request_audit;

pub use release_domain::ui::UiCachePolicy;
pub use types::{
    BeginUpdateHook, BrokeredRuleCopy, CapabilityBindingSelection, CapabilityRevisionDiagnostic,
    CapabilityRevisionResult, CompleteBuild, CreateAttachment, CreateInstanceUpdate, ImportAgent,
    RecoverInstanceUpdate, ReleaseArtifactInput, RemoveAttachment, ReviseInstance,
    ReviseInstanceCapabilities, SetAttachmentEnabled, UpdateDecision, UpdateHookResult,
    UpdateRecoveryAction, UpdateRecoveryDecision,
};
pub use ui_browser::{
    AuthenticateUiBrowserSession, CreateUiBrowserHandoff, CreatedUiBrowserHandoff,
    CreatedUiBrowserSession, ExchangeUiBrowserHandoff, UiBrowserHandoffError,
    UiBrowserRequestRoute, UiBrowserSessionContext, UiBrowserSessionError, UiBrowserSessionStore,
};
pub use ui_browser_host::{
    UI_BOOTSTRAP_PATH, UI_CHILD_COOKIE, UI_HANDOFF_FRAGMENT_LENGTH, UI_RESERVED_PREFIX,
    UiGenerationHost, UiHostError, UiNamespace, UiPublicPort,
};
pub use ui_browser_serving::{
    ActiveUiGenerationHost, UiBrowserHttpPath, UiBrowserHttpPathError, UiBrowserHttpRequest,
    UiBrowserHttpServingProjection, UiBrowserRepositoryGitAuthorization, UiBrowserTargetContext,
    UiBrowserTargetContextProjection, UiGatewayRequestKind, UiGatewayRequestProjection,
    UiGenerationHostResolver, UiGitAuthorizationError, UiHostLookupError,
    UiRepositoryGitAuthorization, UiRepositoryGitOperation, UiServingError, UiServingProjection,
    UiStaticArtifactProjection, UiTargetContextError,
};
pub use ui_installation::{
    ActivateUiInstallation, DisableUiInstallation, InstallStaticUi, InstallStaticUiResult,
    InstallUi, InstallUiResult, RemoveUiInstallation, RollbackUiInstallation, UiInstallationError,
    UiInstallationGenerationResult, UiInstallationLifecycleResult, UiInstallationReceiptScope,
};
pub use ui_installation_navigation::{
    DEFAULT_UI_INSTALLATION_PAGE_SIZE, ListUiInstallations, MAX_UI_INSTALLATION_PAGE_SIZE,
    UiInstallationContentKind, UiInstallationNavigation, UiInstallationNavigationError,
    UiInstallationNavigationPage, UiInstallationNavigator, UiInstallationPage,
    UiInstallationTargetFilter,
};
pub use ui_request_audit::{
    NewUiRequestAuditEvent, UiRequestAuditContext, UiRequestAuditDecision, UiRequestAuditError,
    UiRequestAuditOutcome, UiRequestAuditReason, UiRequestAuditSink, UiRequestAuditSurface,
};
