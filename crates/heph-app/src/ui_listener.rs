use crate::{
    AppError, GitHttpService, HephaestusApp, PgUiBrowserServingStore, PgUiBrowserSessionStore,
    PgUiGenerationHostResolver, PgUiRequestAuditRepository, UiBrowserRepositoryGitAuthorization,
    UiBrowserSessionStore, UiGenerationHostResolver, component,
};
use crate::{ui_bootstrap, ui_browser_content, ui_context, ui_origin_wiring, ui_repository_git};
use axum::Router;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Semaphore;

pub async fn build_ui_listener(
    app: &HephaestusApp,
    git: Arc<GitHttpService>,
) -> Result<Option<(tokio::net::TcpListener, Router)>, AppError> {
    let Some(gateway) = &app.gateway_edge else {
        return Ok(None);
    };
    let Some(ui) = &gateway.ui_origin else {
        return Ok(None);
    };
    let listen = ui.listener().ok_or_else(|| {
        AppError::Configuration(String::from(
            "UI origin listener is missing from validated configuration",
        ))
    })?;
    let listener = tokio::net::TcpListener::bind(listen)
        .await
        .map_err(component("UI origin listener"))?;
    let origin = ui_bootstrap::UiBootstrapConfig::new(
        ui.namespace().clone(),
        ui.public_port(),
        ui.platform_origin().to_owned(),
    )
    .map_err(component("UI origin configuration"))?;
    let sessions: Arc<dyn UiBrowserSessionStore> = Arc::new(PgUiBrowserSessionStore::new(
        app.service_log_pool.clone(),
        app.application_pool.clone(),
    ));
    let audit_sink: Arc<dyn release_service::UiRequestAuditSink> = Arc::new(
        PgUiRequestAuditRepository::new(app.service_log_pool.clone()),
    );
    let host_resolver: Arc<dyn UiGenerationHostResolver> = Arc::new(
        PgUiGenerationHostResolver::new(app.application_pool.clone()),
    );
    let bootstrap = Arc::new(ui_bootstrap::UiBootstrapState::new(
        Arc::clone(&host_resolver),
        sessions,
        origin,
        Arc::clone(&audit_sink),
    ));
    let serving_store = Arc::new(PgUiBrowserServingStore::new(app.application_pool.clone()));
    let serving: Arc<dyn release_service::UiBrowserHttpServingProjection> = serving_store.clone();
    let git_authority: Arc<dyn UiBrowserRepositoryGitAuthorization> = serving_store.clone();
    let gateway = gateway.ui_dispatcher.clone().ok_or_else(|| {
        AppError::Configuration(String::from("UI origin requires a real gateway dispatcher"))
    })?;
    let content = Arc::new(
        ui_browser_content::UiContentState::new(
            Arc::clone(&host_resolver),
            serving,
            Arc::new(app.artifact_store.clone()),
            gateway,
            ui.namespace().clone(),
            ui.public_port(),
            ui.platform_origin().to_owned(),
            Arc::clone(&audit_sink),
        )
        .map_err(component("UI content configuration"))?,
    );
    let git = Arc::new(ui_repository_git::UiRepositoryGitState::new(
        host_resolver.clone(),
        git_authority,
        git,
        ui.namespace().clone(),
        ui.public_port(),
        Arc::clone(&audit_sink),
    ));
    let context_state = Arc::new(ui_context::UiContextState::new(
        Arc::clone(&host_resolver),
        serving_store,
        ui.namespace().clone(),
        ui.public_port(),
        Arc::clone(&audit_sink),
    ));
    let router = ui_origin_wiring::bounded_ui_router_with_audit(
        ui_bootstrap::router(bootstrap)
            .merge(ui_context::router(context_state))
            .merge(ui_repository_git::router(git))
            .merge(ui_browser_content::router(content)),
        Arc::new(Semaphore::new(128)),
        Duration::from_secs(30),
        Arc::clone(&audit_sink),
    );
    Ok(Some((listener, router)))
}
