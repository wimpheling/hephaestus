use super::*;

#[test]
fn route_prefix_allows_dots_inside_segments_but_not_dot_segments_or_escapes() {
    for prefix in ["http.service.v1", "release-1.0/service.v1"] {
        let mut binding = route();
        binding.path_prefix = prefix.to_owned();
        binding.validate().expect("schema-valid dotted route");
    }
    for prefix in [
        ".",
        "..",
        "release/./service",
        "release/../service",
        "release%2Fservice",
    ] {
        let mut binding = route();
        binding.path_prefix = prefix.to_owned();
        assert!(binding.validate().is_err(), "unsafe route prefix: {prefix}");
    }
}

#[test]
fn ui_namespace_replaces_first_slot_and_preserves_gateway_routes() {
    let template = caddy_template_with_ui_slot()
        .with_ui_namespace(
            "ui.example",
            "127.0.0.1:19091".parse().expect("loopback upstream"),
        )
        .expect("valid UI namespace configuration");
    let rendered = template
        .render(
            &GatewayDesiredConfiguration {
                revision: GatewayConfigRevision::new(),
                routes: vec![route()],
            },
            "127.0.0.1:19090",
        )
        .expect("render UI and gateway routes");
    let configuration: Value = serde_json::from_slice(&rendered).expect("rendered JSON");
    let routes = configuration
        .pointer("/apps/http/servers/shared/routes")
        .and_then(Value::as_array)
        .expect("shared routes");
    assert_eq!(routes[0]["group"], "hephaestus.ui");
    assert_eq!(routes[0]["terminal"], true);
    assert_eq!(routes[0]["handle"][0]["handler"], "reverse_proxy");
    assert_eq!(
        routes[0]["handle"][0]["upstreams"][0]["dial"],
        "127.0.0.1:19091"
    );
    assert_eq!(routes[2]["group"], "hephaestus.gateway");
    assert_eq!(
        routes[2]["handle"][0]["routes"][0]["match"][0]["path"][0],
        "/gateway/echo"
    );
    assert_eq!(routes[0]["match"][0]["expression"]["name"], "ui_namespace");
    assert!(
        routes[0]["match"][0]["expression"]["expr"]
            .as_str()
            .expect("UI expression")
            .contains("header_regexp('Host'")
    );
}

#[test]
fn ui_namespace_requires_unique_first_slot_and_loopback_upstream() {
    let mut duplicate = serde_json::json!({
        "apps": { "http": { "servers": { "shared": {
            "routes": [
                { "group": "hephaestus.ui", "handle": [{ "handler": "subroute", "routes": [] }] },
                { "group": "hephaestus.ui", "handle": [{ "handler": "subroute", "routes": [] }] },
                { "group": "hephaestus.gateway", "handle": [{ "handler": "subroute", "routes": [] }] }
            ]
        } } } }
    });
    assert!(
        LocalCaddyConfigurationTemplate::new(
            duplicate.to_string().as_bytes(),
            String::from("shared")
        )
        .expect("gateway slot")
        .with_ui_namespace("ui.example", "127.0.0.1:19091".parse().expect("address"))
        .is_err()
    );

    duplicate["apps"]["http"]["servers"]["shared"]["routes"] = serde_json::json!([
        { "group": "hephaestus.gateway", "handle": [{ "handler": "subroute", "routes": [] }] },
        { "group": "hephaestus.ui", "handle": [{ "handler": "subroute", "routes": [] }] }
    ]);
    assert!(
        LocalCaddyConfigurationTemplate::new(
            duplicate.to_string().as_bytes(),
            String::from("shared")
        )
        .expect("gateway slot")
        .with_ui_namespace("ui.example", "127.0.0.1:19091".parse().expect("address"))
        .is_err()
    );

    assert!(
        caddy_template()
            .with_ui_namespace("ui.example", "127.0.0.1:19091".parse().expect("address"))
            .is_err()
    );

    let template = caddy_template_with_ui_slot();
    assert!(
        template
            .clone()
            .with_ui_namespace("ui..example", "127.0.0.1:19091".parse().expect("address"))
            .is_err()
    );
    assert!(
        template
            .clone()
            .with_ui_namespace("-ui.example", "127.0.0.1:19091".parse().expect("address"))
            .is_err()
    );
    assert!(
        template
            .clone()
            .with_ui_namespace("ui.example", "0.0.0.0:19091".parse().expect("address"))
            .is_err()
    );
    assert!(
        template
            .with_ui_namespace("ui.example", "[::1]:19091".parse().expect("IPv6 loopback"))
            .is_ok()
    );
}

#[test]
fn shared_caddy_template_requires_one_dedicated_gateway_subroute() {
    let missing = serde_json::json!({
        "apps": { "http": { "servers": { "shared": { "routes": [] } } } }
    })
    .to_string();
    assert!(matches!(
        LocalCaddyConfigurationTemplate::new(missing.as_bytes(), String::from("shared")),
        Err(GatewayEdgeError::InvalidCaddyConfiguration)
    ));
}

#[test]
fn reserved_authenticated_routes_are_not_publicly_rendered_or_admitted() {
    let public = route();
    let mut reserved = route();
    reserved.path_prefix = String::from("reserved");
    reserved.exposure = Exposure::HephAuthenticated;
    let rendered = caddy_gateway_routes(
        &GatewayDesiredConfiguration {
            revision: GatewayConfigRevision::new(),
            routes: vec![public, reserved.clone()],
        },
        "127.0.0.1:19090",
    );
    assert_eq!(rendered.len(), 1);
    assert_eq!(rendered[0]["match"][0]["path"][0], "/gateway/echo");
    assert!(validate_request(&reserved, &request("/gateway/reserved")).is_err());
}

#[tokio::test]
async fn dispatcher_returns_not_found_for_reserved_authenticated_route() {
    let mut reserved = route();
    reserved.exposure = Exposure::HephAuthenticated;
    let calls = Arc::new(AtomicUsize::new(0));
    let dispatcher = GatewayDispatcher::new(
        Resolver(reserved),
        CountingHandler(Arc::clone(&calls)),
        Recorder,
    );
    let response = dispatcher.dispatch(request("/gateway/echo")).await.response;
    assert_eq!(response.status, StatusCode::NOT_FOUND);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}
