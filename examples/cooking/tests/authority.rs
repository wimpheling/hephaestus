//! Authenticated retirement proof for the cooking gateway publication grant.

use super::GatewayGoldenFixture;
use connectrpc::{client::ClientConfig, error::ErrorCode};
use identity_domain::UserId;
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use rpc_proto::{
    connect::hephaestus::gateway::v1::GatewayServiceClient,
    messages::hephaestus::{
        common::v1::{OpaqueId, PageRequest, RequestContext},
        gateway::v1::{
            ConfigureGatewayRequest, CreateMailboxBindingRequest, GatewaySecretSelection,
            GetGatewayRequest, ListMailboxPublicationsRequest, RevokeMailboxBindingGrantRequest,
        },
    },
};
use sqlx::PgPool;
use std::{error::Error, time::Duration};
use uuid::Uuid;

type AuthorityError = Box<dyn Error + Send + Sync>;

/// Publishes a new immutable gateway revision selecting the rotated inbound
/// version, then creates the ordinary mailbox binding for that revision. The
/// previous revision and binding remain in history; callers should use the
/// returned fixture for subsequent ingress assertions.
pub async fn configure_rotated_inbound_gateway(
    pool: &PgPool,
    running: &hephaestus_app::RunningHephaestus,
    previous: &GatewayGoldenFixture,
    token_factory: &(dyn Fn(&str) -> String + Send + Sync),
    inbound_import_id: Uuid,
    rotated_version_id: Uuid,
    mailbox_id: Uuid,
) -> Result<GatewayGoldenFixture, AuthorityError> {
    let gateway_id = gateway_id(pool, previous).await?;
    let expected_revision_id: Uuid =
        sqlx::query_scalar("SELECT active_revision_id FROM gateways WHERE id = $1")
            .bind(gateway_id)
            .fetch_one(pool)
            .await?;
    let old_version_id: Uuid = sqlx::query_scalar(
        "SELECT secret_version_id FROM gateway_secret_bindings
          WHERE gateway_id = $1 AND gateway_revision_id = $2 AND slot_key = 'webhook'",
    )
    .bind(gateway_id)
    .bind(expected_revision_id)
    .fetch_one(pool)
    .await?;
    let configure = gateway_client(
        running,
        token_factory,
        "/hephaestus.gateway.v1.GatewayService/ConfigureGateway",
    )?;
    let inbound_placeholder =
        crate::cooking_builds::cooking_inbound_placeholder(rotated_version_id);
    let response = configure
        .configure_gateway(ConfigureGatewayRequest {
            context: mutation_context("cooking-rotate-inbound-gateway", Uuid::new_v4()).into(),
            gateway_id: opaque(gateway_id).into(),
            expected_revision_id: opaque(expected_revision_id).into(),
            parameters: crate::cooking_builds::cooking_gateway_parameters(
                &inbound_placeholder,
                1001,
                1002,
            ),
            secret_selections: vec![GatewaySecretSelection {
                slot_key: String::from("webhook"),
                import_id: opaque(inbound_import_id).into(),
                secret_version_id: opaque(rotated_version_id).into(),
                route_path: String::from("/cooking/telegram"),
                header_name: String::from("x-telegram-bot-api-secret-token"),
                ..Default::default()
            }],
            ..Default::default()
        })
        .await?
        .into_owned();
    let revision_id = response
        .revision_id
        .into_option()
        .ok_or("rotated ConfigureGateway returned no revision")?
        .value
        .parse::<Uuid>()?;
    // Mailbox producer identity is also the deduplication scope and is
    // unique across immutable bindings for one mailbox. Scope this rotation
    // to its new revision so retained history cannot collide with it.
    let producer_id = format!("cooking-gateway-{revision_id}");
    let binding_client = gateway_client(
        running,
        token_factory,
        "/hephaestus.gateway.v1.GatewayService/CreateMailboxBinding",
    )?;
    let binding = binding_client
        .create_mailbox_binding(CreateMailboxBindingRequest {
            context: mutation_context("cooking-rotate-inbound-mailbox", Uuid::new_v4()).into(),
            gateway_revision_id: opaque(revision_id).into(),
            slot_key: String::from("cooking_requests"),
            mailbox_id: opaque(mailbox_id).into(),
            producer_id,
            ..Default::default()
        })
        .await?
        .into_owned();
    let binding = binding
        .binding
        .into_option()
        .ok_or("rotated CreateMailboxBinding returned no binding")?;
    let grant_id = binding
        .grant_id
        .into_option()
        .ok_or("rotated CreateMailboxBinding returned no grant")?
        .value
        .parse::<Uuid>()?;
    let retained_versions: Vec<Uuid> = sqlx::query_scalar(
        "SELECT secret_version_id FROM gateway_secret_bindings
          WHERE gateway_id = $1 AND slot_key = 'webhook'
          ORDER BY secret_version_id",
    )
    .bind(gateway_id)
    .fetch_all(pool)
    .await?;
    assert!(retained_versions.contains(&old_version_id));
    assert!(retained_versions.contains(&rotated_version_id));
    Ok(GatewayGoldenFixture {
        mailbox_id: previous.mailbox_id,
        grant_id,
    })
}

/// Confirms that gateway runtime leases retain both the old revision's
/// selected inbound version and the later rotated revision's version.
pub async fn assert_inbound_lease_history(
    pool: &PgPool,
    gateway: &GatewayGoldenFixture,
    old_version_id: Uuid,
    rotated_version_id: Uuid,
) -> Result<(), AuthorityError> {
    let gateway_id = gateway_id(pool, gateway).await?;
    let versions: Vec<Uuid> = sqlx::query_scalar(
        "SELECT DISTINCT lease.secret_version_id
           FROM gateway_secret_leases lease
           JOIN gateway_secret_bindings binding ON binding.id = lease.binding_id
          WHERE binding.gateway_id = $1",
    )
    .bind(gateway_id)
    .fetch_all(pool)
    .await?;
    assert!(
        versions.contains(&old_version_id),
        "old inbound lease is retained"
    );
    assert!(
        versions.contains(&rotated_version_id),
        "rotated inbound lease is retained"
    );
    assert_ne!(old_version_id, rotated_version_id);
    Ok(())
}

/// Revokes the current cooking publication grant through authenticated RPC,
/// then proves replay and live Caddy denial preserve the earlier evidence.
pub async fn retire_cooking_gateway_grant(
    pool: &PgPool,
    running: &hephaestus_app::RunningHephaestus,
    gateway: &GatewayGoldenFixture,
    token_factory: &(dyn Fn(&str) -> String + Send + Sync),
    inbound_credential: &str,
    outsider_id: UserId,
) -> Result<(), AuthorityError> {
    let active_binding: (Uuid, Uuid) = sqlx::query_as(
        "SELECT binding.id, binding_grant.id
           FROM gateway_mailbox_bindings binding
           JOIN gateway_mailbox_binding_grants binding_grant ON binding_grant.binding_id = binding.id
           JOIN gateways gateway ON gateway.id = binding.gateway_id
          WHERE binding.mailbox_id = $1 AND binding_grant.id = $2
            AND binding_grant.status = 'active'
            AND gateway.active_revision_id = binding.gateway_revision_id",
    )
    .bind(gateway.mailbox_id.as_uuid())
    .bind(gateway.grant_id)
    .fetch_one(pool)
    .await?;
    assert_eq!(active_binding.1, gateway.grant_id);

    let audience = "/hephaestus.gateway.v1.GatewayService/RevokeMailboxBindingGrant";
    let client = gateway_client(running, token_factory, audience)?;
    let revoke_key = "cooking-authority-revoke-gateway-mailbox";
    let revoked = client
        .revoke_mailbox_binding_grant(RevokeMailboxBindingGrantRequest {
            context: mutation_context(revoke_key, Uuid::new_v4()).into(),
            binding_id: opaque(active_binding.0).into(),
            ..Default::default()
        })
        .await?
        .into_owned();
    let revoked_binding = revoked
        .binding
        .as_option()
        .ok_or("revoke returned no binding")?;
    let revoke_receipt = revoked
        .receipt
        .as_option()
        .ok_or("revoke returned no receipt")?;
    assert_eq!(
        revoked_binding
            .grant_id
            .as_option()
            .map(|id| id.value.clone()),
        Some(active_binding.1.to_string())
    );

    // Revoke itself is idempotent across transport retries.
    let replay_context = mutation_context(revoke_key, Uuid::new_v4());
    let replay = client
        .revoke_mailbox_binding_grant(RevokeMailboxBindingGrantRequest {
            context: replay_context.clone().into(),
            binding_id: opaque(active_binding.0).into(),
            ..Default::default()
        })
        .await?
        .into_owned();
    assert_eq!(replay.receipt.as_option(), Some(revoke_receipt));
    assert_eq!(
        replay
            .binding
            .as_option()
            .map(|binding| binding.grant_status.as_str()),
        Some("revoked")
    );

    // CreateMailboxBinding's exact receipt and replay are proved by the
    // build helper before browser reconfiguration. Retiring the current
    // browser-selected binding must not reuse that earlier revision key.
    let status: String =
        sqlx::query_scalar("SELECT status FROM gateway_mailbox_binding_grants WHERE id = $1")
            .bind(active_binding.1)
            .fetch_one(pool)
            .await?;
    assert_eq!(status, "revoked");

    assert_revoked_gateway_history(pool, running, gateway, token_factory, outsider_id).await?;
    assert_revoked_ingress(pool, gateway, inbound_credential).await?;
    Ok(())
}

async fn assert_revoked_gateway_history(
    pool: &PgPool,
    running: &hephaestus_app::RunningHephaestus,
    gateway: &GatewayGoldenFixture,
    token_factory: &(dyn Fn(&str) -> String + Send + Sync),
    outsider_id: UserId,
) -> Result<(), AuthorityError> {
    let browser_gateway_id = gateway_id(pool, gateway).await?;
    let current = gateway_client(
        running,
        token_factory,
        "/hephaestus.gateway.v1.GatewayService/GetGateway",
    )?
    .get_gateway(GetGatewayRequest {
        gateway_id: opaque(browser_gateway_id).into(),
        ..Default::default()
    })
    .await?
    .into_owned();
    assert!(current.gateway.as_option().is_some());

    let history = gateway_client(
        running,
        token_factory,
        "/hephaestus.gateway.v1.GatewayService/ListMailboxPublications",
    )?
    .list_mailbox_publications(ListMailboxPublicationsRequest {
        gateway_id: opaque(browser_gateway_id).into(),
        page: PageRequest {
            page_size: 64,
            ..Default::default()
        }
        .into(),
        ..Default::default()
    })
    .await?
    .into_owned();
    assert!(
        history
            .publications
            .iter()
            .any(|publication| publication.run_id.is_set()),
        "prior publication/run history must remain inspectable"
    );

    let outsider = gateway_client_with_token(
        running,
        &outsider_token(
            "/hephaestus.gateway.v1.GatewayService/GetGateway",
            outsider_id,
        ),
    )?;
    let outsider_error = outsider
        .get_gateway(GetGatewayRequest {
            gateway_id: opaque(browser_gateway_id).into(),
            ..Default::default()
        })
        .await
        .expect_err("an outsider must not inspect the cooking gateway");
    // Gateway reads check CanRead before loading the resource, so an
    // authenticated outsider receives the generic authorization denial.
    assert_eq!(outsider_error.code, ErrorCode::PermissionDenied);

    let outsider_history = gateway_client_with_token(
        running,
        &outsider_token(
            "/hephaestus.gateway.v1.GatewayService/ListMailboxPublications",
            outsider_id,
        ),
    )?;
    let outsider_history_error = outsider_history
        .list_mailbox_publications(ListMailboxPublicationsRequest {
            gateway_id: opaque(browser_gateway_id).into(),
            page: PageRequest {
                page_size: 64,
                ..Default::default()
            }
            .into(),
            ..Default::default()
        })
        .await
        .expect_err("an outsider must not inspect gateway publication history");
    assert_eq!(outsider_history_error.code, ErrorCode::PermissionDenied);
    Ok(())
}

async fn assert_revoked_ingress(
    pool: &PgPool,
    gateway: &GatewayGoldenFixture,
    inbound_credential: &str,
) -> Result<(), AuthorityError> {
    let before: i64 =
        sqlx::query_scalar("SELECT count(*) FROM mailbox_events WHERE mailbox_id = $1")
            .bind(gateway.mailbox_id.as_uuid())
            .fetch_one(pool)
            .await?;
    let before_deliveries: i64 =
        sqlx::query_scalar("SELECT count(*) FROM mailbox_deliveries WHERE mailbox_id = $1")
            .bind(gateway.mailbox_id.as_uuid())
            .fetch_one(pool)
            .await?;
    let before_runs: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM runs WHERE instance_id = (
        SELECT instance_id FROM mailboxes WHERE id = $1
    )",
    )
    .bind(gateway.mailbox_id.as_uuid())
    .fetch_one(pool)
    .await?;
    let public = std::env::var("HEPHAESTUS_CADDY_TEST_PUBLIC_URL")?;
    let response = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()?
        .post(format!("{public}/gateway/cooking/telegram"))
        .header("x-telegram-bot-api-secret-token", inbound_credential)
        .json(&serde_json::json!({
            "update_id": 9001,
            "message": {"from": {"id": 1001}, "text": "retired"}
        }))
        .send()
        .await?;
    assert_eq!(response.status(), reqwest::StatusCode::BAD_GATEWAY);
    response.bytes().await?;
    let after: i64 =
        sqlx::query_scalar("SELECT count(*) FROM mailbox_events WHERE mailbox_id = $1")
            .bind(gateway.mailbox_id.as_uuid())
            .fetch_one(pool)
            .await?;
    assert_eq!(after, before);
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM mailbox_deliveries WHERE mailbox_id = $1",
        )
        .bind(gateway.mailbox_id.as_uuid())
        .fetch_one(pool)
        .await?,
        before_deliveries,
        "revoked ingress must not create a delivery attempt"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM runs WHERE instance_id = (
            SELECT instance_id FROM mailboxes WHERE id = $1
        )"
        )
        .bind(gateway.mailbox_id.as_uuid())
        .fetch_one(pool)
        .await?,
        before_runs,
        "revoked ingress must not create a run"
    );
    Ok(())
}

fn gateway_client(
    running: &hephaestus_app::RunningHephaestus,
    token_factory: &(dyn Fn(&str) -> String + Send + Sync),
    audience: &str,
) -> Result<GatewayServiceClient<connectrpc::client::HttpClient>, AuthorityError> {
    gateway_client_with_token(running, &token_factory(audience))
}

fn gateway_client_with_token(
    running: &hephaestus_app::RunningHephaestus,
    token: &str,
) -> Result<GatewayServiceClient<connectrpc::client::HttpClient>, AuthorityError> {
    let uri = format!("http://{}", running.http_addr()).parse()?;
    let config = ClientConfig::new(uri)
        .with_default_header(
            http::header::AUTHORIZATION,
            http::HeaderValue::from_str(&format!("Bearer {token}"))?,
        )
        .with_default_timeout(Duration::from_secs(30));
    Ok(GatewayServiceClient::new(
        connectrpc::client::HttpClient::plaintext(),
        config,
    ))
}

fn outsider_token(audience: &str, outsider_id: UserId) -> String {
    let now = time::OffsetDateTime::now_utc().unix_timestamp();
    encode(
        &Header::new(Algorithm::HS256),
        &serde_json::json!({
            "iss": "hephaestus-web-mediator",
            "sub": outsider_id.to_string(),
            "aud": audience,
            "iat": now,
            "nbf": now,
            "exp": now + 25,
            "jti": Uuid::new_v4().to_string()
        }),
        &EncodingKey::from_secret(&hephaestus_app::rpc::mediator_signing_key(
            b"golden-internal-command-token-with-sufficient-entropy",
        )),
    )
    .expect("sign outsider mediator token")
}

fn mutation_context(key: &str, request_id: Uuid) -> RequestContext {
    RequestContext {
        request_id: opaque(request_id).into(),
        idempotency_key: key.to_owned(),
        ..Default::default()
    }
}

fn opaque(id: Uuid) -> OpaqueId {
    OpaqueId {
        value: id.to_string(),
        ..Default::default()
    }
}

async fn gateway_id(pool: &PgPool, gateway: &GatewayGoldenFixture) -> Result<Uuid, AuthorityError> {
    Ok(sqlx::query_scalar(
        "SELECT binding.gateway_id FROM gateway_mailbox_bindings binding
         JOIN gateway_mailbox_binding_grants binding_grant ON binding_grant.binding_id = binding.id
         WHERE binding.mailbox_id = $1 AND binding_grant.id = $2",
    )
    .bind(gateway.mailbox_id.as_uuid())
    .bind(gateway.grant_id)
    .fetch_one(pool)
    .await?)
}
