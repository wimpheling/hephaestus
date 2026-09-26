//! Opt-in shared-Caddy ingress proof.
//!
//! `scripts/test-gateway-caddy-smoke.sh` owns the disposable Caddy container
//! and supplies its loopback-only admin and public listener URLs.

use gateway_edge::{
    GatewayConfigRevision, GatewayDesiredConfiguration, GatewayProvider, LocalCaddyAdministration,
    LocalCaddyConfigurationTemplate, LocalCaddyGatewayProvider,
};
use std::{env, time::Duration};
use tokio::{net::TcpListener, time::timeout};

#[path = "caddy_ingress/fixtures.rs"]
mod fixtures;
use fixtures::{
    NoopDispatcher, caddy_configuration, caddy_configuration_with_ui_slot, desired, desired_path,
    serve_requests, serve_ui_requests,
};

#[tokio::test(flavor = "multi_thread")]
#[allow(clippy::too_many_lines)] // Keep the ordered load/reload precedence proof together.
async fn rendered_ui_namespace_survives_caddy_load_and_never_falls_through() {
    if env::var("HEPHAESTUS_CADDY_TEST_UI").as_deref() != Ok("1") {
        return;
    }
    let admin_url = env::var("HEPHAESTUS_CADDY_TEST_ADMIN_URL")
        .expect("Caddy admin URL supplied for UI ingress proof");
    let public_url = env::var("HEPHAESTUS_CADDY_TEST_PUBLIC_URL")
        .expect("Caddy public URL supplied for UI ingress proof");

    let dispatcher = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind private dispatcher fixture");
    let dispatcher_address = dispatcher.local_addr().expect("read dispatcher address");
    let dispatcher_task = tokio::spawn(serve_requests(dispatcher, 2));
    let ui = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind private UI fixture");
    let ui_address = ui.local_addr().expect("read UI address");
    let ui_task = tokio::spawn(serve_ui_requests(ui, 4));

    let template = LocalCaddyConfigurationTemplate::new(
        &caddy_configuration_with_ui_slot(&admin_url),
        String::from("shared"),
    )
    .expect("valid complete shared Caddy configuration")
    .with_ui_namespace("ui.example", ui_address)
    .expect("valid loopback UI upstream");
    let provider = LocalCaddyGatewayProvider::new(
        LocalCaddyAdministration::new(&admin_url).expect("loopback Caddy admin client"),
        NoopDispatcher,
    )
    .with_dispatcher_upstream(dispatcher_address.to_string())
    .with_configuration_template(template);

    timeout(Duration::from_secs(5), provider.reconcile(&desired()))
        .await
        .expect("Caddy load before deadline")
        .expect("load rendered UI and gateway routes");

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .expect("bounded Caddy client");
    let platform = client
        .get(format!("{public_url}/platform/status"))
        .send()
        .await
        .expect("outside UI namespace platform request");
    assert_eq!(platform.status(), reqwest::StatusCode::OK);
    assert_eq!(
        platform.text().await.expect("platform body"),
        "platform-owned"
    );

    let known = client
        .get(format!("{public_url}/ui/known"))
        .header(reqwest::header::HOST, "ui.example")
        .send()
        .await
        .expect("known UI namespace request");
    assert_eq!(known.status(), reqwest::StatusCode::OK);
    assert_eq!(known.text().await.expect("known UI body"), "ui-upstream");

    let unknown = client
        .get(format!("{public_url}/platform/status"))
        .header(reqwest::header::HOST, "malformed..ui.example")
        .send()
        .await
        .expect("unknown UI namespace request");
    assert_eq!(unknown.status(), reqwest::StatusCode::NOT_FOUND);
    assert_eq!(
        unknown.text().await.expect("unknown UI body"),
        "ui-upstream-unknown"
    );

    let gateway = client
        .post(format!("{public_url}/gateway/proof"))
        .body("gateway-before-reload")
        .send()
        .await
        .expect("gateway request before reload");
    assert_eq!(gateway.status(), reqwest::StatusCode::CREATED);

    timeout(
        Duration::from_secs(5),
        provider.reconcile(&desired_path("changed")),
    )
    .await
    .expect("Caddy reload before deadline")
    .expect("reload updated gateway routes while preserving UI route");

    let known_after_reload = client
        .get(format!("{public_url}/ui/after-reload"))
        .header(reqwest::header::HOST, "deep.ui.example")
        .send()
        .await
        .expect("UI request after gateway reload");
    assert_eq!(known_after_reload.status(), reqwest::StatusCode::OK);
    assert_eq!(
        known_after_reload
            .text()
            .await
            .expect("UI body after reload"),
        "ui-upstream"
    );

    let unknown_after_reload = client
        .get(format!("{public_url}/ui/after-reload"))
        .header(reqwest::header::HOST, "deep.malformed..ui.example")
        .send()
        .await
        .expect("unknown UI request after gateway reload");
    assert_eq!(
        unknown_after_reload.status(),
        reqwest::StatusCode::NOT_FOUND
    );
    assert_eq!(
        unknown_after_reload
            .text()
            .await
            .expect("unknown UI body after reload"),
        "ui-upstream-unknown"
    );

    let gateway_after_reload = client
        .post(format!("{public_url}/gateway/changed"))
        .body("gateway-after-reload")
        .send()
        .await
        .expect("updated gateway request");
    assert_eq!(gateway_after_reload.status(), reqwest::StatusCode::CREATED);

    timeout(Duration::from_secs(5), ui_task)
        .await
        .expect("UI fixture completed before deadline")
        .expect("private UI task");
    timeout(Duration::from_secs(5), dispatcher_task)
        .await
        .expect("dispatcher fixture completed before deadline")
        .expect("private dispatcher task");
}

#[tokio::test(flavor = "multi_thread")]
async fn preserves_platform_routes_and_removes_tombstoned_gateway_routes() {
    let Ok(admin_url) = env::var("HEPHAESTUS_CADDY_TEST_ADMIN_URL") else {
        return;
    };
    let public_url = env::var("HEPHAESTUS_CADDY_TEST_PUBLIC_URL")
        .expect("Caddy public listener URL supplied with admin URL");

    let upstream = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind private dispatcher fixture");
    let upstream_address = upstream.local_addr().expect("read dispatcher address");
    let responder = tokio::spawn(serve_requests(upstream, 2));
    let template = LocalCaddyConfigurationTemplate::new(
        &caddy_configuration(&admin_url),
        String::from("shared"),
    )
    .expect("valid complete shared Caddy configuration");
    let provider = LocalCaddyGatewayProvider::new(
        LocalCaddyAdministration::new(&admin_url).expect("loopback Caddy admin client"),
        NoopDispatcher,
    )
    .with_dispatcher_upstream(upstream_address.to_string())
    .with_configuration_template(template);
    provider
        .reconcile(&desired())
        .await
        .expect("atomically merge the gateway subroute into shared Caddy");

    let response = reqwest::Client::new()
        .post(format!("{public_url}/gateway/proof?source=caddy"))
        .header("x-forwarded-for", "spoofed")
        .body("gateway-caddy-request")
        .send()
        .await
        .expect("Caddy public gateway request");
    assert_eq!(response.status(), reqwest::StatusCode::CREATED);
    assert_eq!(
        response.headers().get("x-gateway-proof"),
        Some(&reqwest::header::HeaderValue::from_static("shared-caddy"))
    );
    assert_eq!(
        response.bytes().await.expect("bounded response body"),
        "gateway-caddy-response"
    );

    provider
        .reconcile(&desired_path("changed"))
        .await
        .expect("atomically update the gateway subroute");
    let old = reqwest::get(format!("{public_url}/gateway/proof"))
        .await
        .expect("removed route response after gateway update");
    assert_eq!(old.status(), reqwest::StatusCode::NOT_FOUND);
    let updated = reqwest::Client::new()
        .post(format!("{public_url}/gateway/changed"))
        .body("updated-gateway-request")
        .send()
        .await
        .expect("updated Caddy gateway request");
    assert_eq!(updated.status(), reqwest::StatusCode::CREATED);
    let platform = reqwest::get(format!("{public_url}/platform/status"))
        .await
        .expect("platform route survives gateway reconciliation");
    assert_eq!(platform.status(), reqwest::StatusCode::OK);
    assert_eq!(
        platform.text().await.expect("platform body"),
        "platform-owned"
    );

    provider
        .reconcile(&GatewayDesiredConfiguration {
            revision: GatewayConfigRevision::new(),
            routes: Vec::new(),
        })
        .await
        .expect("atomically remove tombstoned gateway routes");
    let removed = reqwest::get(format!("{public_url}/gateway/proof"))
        .await
        .expect("tombstoned gateway route response");
    assert_eq!(removed.status(), reqwest::StatusCode::NOT_FOUND);
    let platform_after = reqwest::get(format!("{public_url}/platform/status"))
        .await
        .expect("platform route after tombstone");
    assert_eq!(platform_after.status(), reqwest::StatusCode::OK);
    responder.await.expect("private dispatcher task");
}
