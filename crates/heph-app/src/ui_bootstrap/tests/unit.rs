use super::super::UiBootstrapConfig;
use super::super::helpers::{Theme, ThemeOption, bootstrap_html, parse_theme};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use release_service::ui_browser_host::{UI_CHILD_COOKIE, UiNamespace, UiPublicPort};

#[test]
fn theme_is_allowlisted_and_not_reflected() {
    assert_eq!(
        parse_theme(Some("heph_theme=light")),
        Ok(ThemeOption(Some(Theme::Light)))
    );
    assert!(parse_theme(Some("heph_theme=anything")).is_err());
    assert!(parse_theme(Some("heph_theme=light&heph_theme=dark")).is_err());
    assert_eq!(
        parse_theme(Some("redirect=https://evil")),
        Ok(ThemeOption(None))
    );
}

#[test]
fn bootstrap_script_clears_fragment_before_post_and_requires_host_relative_route() {
    let html = bootstrap_html("nonce", "https://app.example");
    assert!(html.contains("history.replaceState(null,\"\",location.pathname+initialSearch)"));
    assert!(html.contains("fetch(\"/_heph/bootstrap\"+bootstrapQuery"));
    assert!(html.contains("heph_theme_origin"));
    assert!(html.contains("body:fragment"));
    assert!(html.contains("location.replace(destination.pathname"));
    assert!(!html.contains("parent_session_id"));
    assert!(html.contains("heph-platform-origin"));
}

#[test]
fn cookie_policy_has_no_domain_or_platform_cookie() {
    let value = URL_SAFE_NO_PAD.encode([7_u8; 32]);
    let cookie =
        format!("{UI_CHILD_COOKIE}={value}; Path=/; Max-Age=60; Secure; HttpOnly; SameSite=Strict");
    assert!(cookie.starts_with("__Host-hephaestus_ui="));
    assert!(!cookie.contains("Domain="));
    assert!(!cookie.contains("platform"));
}

#[test]
fn csp_uses_exact_platform_frame_ancestor() {
    let html = bootstrap_html("nonce", "https://app.example");
    assert!(html.contains("nonce=\"nonce\""));
    assert!(html.contains("heph-platform-origin"));
    assert!(!html.contains("g-*"));
}

#[test]
fn origin_configuration_rejects_paths_and_sibling_namespaces() {
    let namespace = UiNamespace::parse("ui.app.example").expect("test namespace");
    let port = UiPublicPort::https_default();
    assert!(UiBootstrapConfig::new(namespace.clone(), port, "https://app.example").is_ok());
    assert!(UiBootstrapConfig::new(namespace.clone(), port, "https://app.example/path").is_err());
    assert!(UiBootstrapConfig::new(namespace, port, "https://other.example").is_err());
}

#[test]
fn exact_origin_rejects_ambiguous_authorities() {
    let namespace = UiNamespace::parse("ui.app.example").expect("namespace");
    let port = UiPublicPort::https_default();
    assert!(UiBootstrapConfig::new(namespace.clone(), port, "https://app.example").is_ok());
    assert!(UiBootstrapConfig::new(namespace.clone(), port, "https://app.example/path").is_err());
    assert!(UiBootstrapConfig::new(namespace.clone(), port, "https://user@app.example").is_err());
    assert!(UiBootstrapConfig::new(namespace, port, "https://app.example:443:444").is_err());
}
