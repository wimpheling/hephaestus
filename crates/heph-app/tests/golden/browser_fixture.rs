use super::*;

pub async fn wait_for_caddy_configuration(admin_url: &str, required_route: &str) -> String {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .expect("bounded Caddy configuration client");
    let mut last_configuration = String::new();
    for _ in 0..40 {
        last_configuration = client
            .get(format!("{admin_url}/config/"))
            .send()
            .await
            .expect("load applied Caddy configuration")
            .error_for_status()
            .expect("Caddy configuration request succeeds")
            .text()
            .await
            .expect("read applied Caddy configuration");
        if last_configuration.contains(required_route) {
            return last_configuration;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    panic!(
        "Caddy did not contain {required_route} after bounded reconciliation: {last_configuration}"
    );
}

pub async fn seed_golden_browser_session(
    pool: &sqlx::PgPool,
    user_id: UserId,
    issuer: &str,
    subject: &str,
) -> BrowserSessionSid {
    let sid = BrowserSessionSid::new();
    let verified = AuthenticatedIdentity::new(
        user_id,
        issuer,
        subject,
        serde_json::Value::Null,
        RequestId::new(),
    );
    sqlx::query(
        "INSERT INTO human_browser_sessions
            (id, sid_digest, creation_idempotency_id, creation_request_id,
             identity_binding_digest, user_id, issued_at, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6, now(), now() + interval '12 hours')",
    )
    .bind(uuid::Uuid::new_v4())
    .bind(browser_session_sid_digest(sid).as_bytes().to_vec())
    .bind(uuid::Uuid::new_v4())
    .bind(uuid::Uuid::new_v4())
    .bind(
        browser_session_identity_binding_digest(&verified)
            .as_bytes()
            .to_vec(),
    )
    .bind(user_id.as_uuid())
    .execute(pool)
    .await
    .expect("seed active golden browser session");
    sid
}

pub fn signed_token(lifetime: Duration) -> String {
    let now = OffsetDateTime::now_utc().unix_timestamp();
    let lifetime_seconds = i64::try_from(lifetime.as_secs()).expect("bounded token lifetime");
    let issuer = golden_issuer();
    encode(
        &Header::new(Algorithm::HS256),
        &serde_json::json!({
            "iss": issuer,
            "sub": "golden-subject",
            "aud": AUDIENCE,
            "iat": now,
            "exp": now + lifetime_seconds,
            "email": "golden@example.invalid",
            "email_verified": true
        }),
        &EncodingKey::from_secret(SIGNING_SECRET),
    )
    .expect("sign golden bearer token")
}

pub fn installed_ui_fixture_enabled() -> bool {
    env::var("HEPHAESTUS_COOKING_INSTALLED_UI_FIXTURE").as_deref() == Ok("1")
        || env::var("HEPHAESTUS_APP_SESSION_CHAT_BROWSER_E2E").as_deref() == Ok("1")
}

pub fn installed_ui_platform_origin() -> String {
    let public =
        Url::parse(&env::var("HEPHAESTUS_CADDY_TEST_PUBLIC_URL").expect("joined Caddy public URL"))
            .expect("joined Caddy public URL");
    let port = public.port().expect("joined Caddy public port");
    format!("https://platform.localhost:{port}")
}

pub fn installed_ui_namespace() -> String {
    env::var("HEPHAESTUS_UI_NAMESPACE").unwrap_or_else(|_| String::from("ui.platform.localhost"))
}

pub fn reserve_installed_ui_listener() -> std::net::SocketAddr {
    std::net::TcpListener::bind("127.0.0.1:0")
        .expect("reserve installed UI origin listener")
        .local_addr()
        .expect("installed UI origin listener address")
}

pub fn installed_ui_origin_config(listener: std::net::SocketAddr) -> UiOriginConfig {
    let public =
        Url::parse(&env::var("HEPHAESTUS_CADDY_TEST_PUBLIC_URL").expect("joined Caddy public URL"))
            .expect("joined Caddy public URL");
    let public_port = UiPublicPort::parse(public.port().expect("joined Caddy public port"))
        .expect("joined Caddy public port is valid");
    UiOriginConfig::new(
        UiNamespace::parse(installed_ui_namespace()).expect("installed UI namespace"),
        public_port,
        installed_ui_platform_origin(),
    )
    .expect("installed UI origin configuration")
    .with_listener(listener)
}

pub fn caddy_configuration(admin_url: &str) -> Vec<u8> {
    let admin = reqwest::Url::parse(admin_url).expect("Caddy admin URL");
    let admin_listen = format!(
        "{}:{}",
        admin.host_str().expect("Caddy admin host"),
        admin.port().expect("Caddy admin port")
    );
    let mut routes = vec![
        serde_json::json!({
            "match": [{ "path": ["/platform/*"] }],
            "handle": [{ "handler": "static_response", "body": "platform-owned" }]
        }),
        serde_json::json!({
            "group": "hephaestus.gateway",
            "handle": [{ "handler": "subroute", "routes": [] }]
        }),
        serde_json::json!({ "handle": [{ "handler": "static_response", "status_code": 404 }] }),
    ];
    if installed_ui_fixture_enabled() {
        let web_port =
            env::var("HEPHAESTUS_E2E_EXTERNAL_WEB_PORT").unwrap_or_else(|_| "4000".into());
        let platform_host = Url::parse(&installed_ui_platform_origin())
            .expect("installed UI platform origin")
            .host_str()
            .expect("installed UI platform host")
            .to_owned();
        routes.insert(
            0,
            serde_json::json!({
                "group": "hephaestus.ui",
                "handle": [{ "handler": "subroute", "routes": [] }]
            }),
        );
        routes.insert(
            1,
            serde_json::json!({
                "match": [{ "host": [platform_host] }],
                "handle": [{
                    "handler": "reverse_proxy",
                    "upstreams": [{ "dial": format!("127.0.0.1:{web_port}") }]
                }],
                "terminal": true
            }),
        );
    }
    let mut configuration = serde_json::json!({
        "admin": { "listen": admin_listen },
        "apps": { "http": { "servers": { "shared": {
            "listen": [env::var("HEPHAESTUS_CADDY_TEST_LISTEN").expect("joined Caddy listen address")],
            "routes": routes
        } } } }
    });
    if env::var("HEPHAESTUS_CADDY_TEST_TLS").as_deref() == Ok("1") {
        let server = configuration
            .pointer_mut("/apps/http/servers/shared")
            .expect("shared Caddy server configuration");
        server["automatic_https"] = serde_json::json!({ "disable_redirects": true });
        server["tls_connection_policies"] = serde_json::json!([{}]);
        let subjects = if installed_ui_fixture_enabled() {
            serde_json::json!([
                "127.0.0.1",
                Url::parse(&installed_ui_platform_origin())
                    .expect("installed UI platform origin")
                    .host_str()
                    .expect("installed UI platform host")
                    .to_owned(),
                format!("*.{}", installed_ui_namespace())
            ])
        } else {
            serde_json::json!(["127.0.0.1"])
        };
        configuration["apps"]["tls"] = serde_json::json!({
            "certificates": { "automate": subjects },
            "automation": {
                "policies": [{
                    "subjects": subjects,
                    "issuers": [{ "module": "internal" }]
                }]
            }
        });
    }
    serde_json::to_vec(&configuration).expect("serialize Caddy configuration template")
}

pub const fn agent_config() -> &'static str {
    r#"
  version = 2
  [agent]
  name = "Golden Agent"
  key = "golden-agent"
  [build]
  command = "/bin/sh"
  arguments = ["-c", "mkdir -p /workspace/output/bin && cp /workspace/source/golden-agent.sh /workspace/output/bin/golden && chmod 0555 /workspace/output/bin/golden"]
  working_directory = "/workspace/source"
  image = { key = "golden-root" }
  triggers = ["refs/heads/main"]
  [build.resources]
  vcpus = 1
  memory_mib = 512
  [build.network]
  profile = "disabled"
  [[build.artifacts]]
  path = "bin/golden"
  kind = "executable"
  [guest]
  image = { key = "golden-root" }
  command = "bin/golden"
  arguments = []
  working_directory = "bin"
  [resources]
  vcpus = 1
  memory_mib = 512
  [workspace]
  mount = true
  path = "/workspace/repo"
  read_only = true
  [state_volume]
  enabled = true
  [network]
  profile = "disabled"
  [triggers]
  push = false
  refs = ["refs/heads/main"]
  [update_hook]
  command = "bin/golden"
  arguments = []
  timeout_seconds = 60
  [update_hook.resources]
  vcpus = 1
  memory_mib = 512
  "#
}
