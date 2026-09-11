//! Optional real cooking-app slice on the golden daemon fixture.

use super::{
    BrokeredFixture, BrokeredTlsUpstream, GatewayGoldenFixture, SeededInstance, secret_command_key,
};
use authz_postgres::PostgresMelangeAuthorizer;
use brokered_egress_domain::{
    BrokeredSecretRule, BrokeredSecretRuleId, ExactHttpsOrigin, HeaderName, HttpInjectionLocation,
};
use forge_domain::{OrganizationId, ProjectId};
use identity_domain::{AuthenticatedIdentity, RequestId, UserId};
use secret_application::{
    BindSecret, CreateSecret, DeclareBrokeredHttpsRule, GrantAndAcceptSecretImport, RotateSecret,
};
use secret_broker::{BrokeredHttpsAdapterRegistry, BrokeredHttpsRequest};
use secret_domain::{
    AgentSecretBindingId, DeliveryMode, ExecutionPhase, SecretAlias, SecretGrantId, SecretId,
    SecretImportId, SecretName, SecretOwner, SecretSlotKey, SecretTarget, SecretUsePolicy,
    SecretValue, SecretVersionId,
};
use secret_postgres::SecretService;
use secret_store::{EncryptedStore, LocalKeyProvider};
use std::{
    env, fs,
    io::Write as _,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};
use tokio_rustls::TlsAcceptor;

pub fn enabled() -> bool {
    env::var("HEPHAESTUS_APP_COOKING_E2E").as_deref() == Ok("1")
}

fn source_root() -> PathBuf {
    PathBuf::from(env::var("HEPHAESTUS_COOKING_SOURCE_ROOT").expect("cooking source root"))
}

pub fn agent_artifact() -> Option<Vec<u8>> {
    enabled().then(|| {
        std::fs::read(source_root().join("cooking-agent/cooking_agent.py"))
            .expect("exact cooking agent source")
    })
}

pub fn gateway_artifact() -> Option<Vec<u8>> {
    enabled().then(|| {
        std::fs::read(
            env::var("HEPHAESTUS_COOKING_GATEWAY_ARTIFACT").expect("cooking gateway artifact path"),
        )
        .expect("read built gateway")
    })
}

pub fn secret_slots() -> serde_json::Value {
    serde_json::json!([
        {"key":"model","purpose":"Cooking model fixture","required":true,"delivery_modes":["brokered"],"phases":["normal"],"destinations":["api.model.example"]},
        {"key":"telegram_relay","purpose":"External cooking relay","required":true,"delivery_modes":["brokered"],"phases":["normal"],"destinations":["relay.cooking.example"]}
    ])
}

pub async fn copy_blog(target: &Path) {
    let root = source_root().join("cooking-blog");
    let mut pending = vec![(root, target.to_path_buf())];
    while let Some((source, destination)) = pending.pop() {
        tokio::fs::create_dir_all(&destination)
            .await
            .expect("blog directory");
        for entry in std::fs::read_dir(source).expect("blog sources") {
            let entry = entry.expect("blog entry");
            let name = entry.file_name();
            if name.to_string_lossy().starts_with('.') || name == "public" || name == "__pycache__"
            {
                continue;
            }
            let destination = destination.join(name);
            if entry.file_type().expect("blog entry type").is_dir() {
                pending.push((entry.path(), destination));
            } else {
                tokio::fs::copy(entry.path(), destination)
                    .await
                    .expect("copy blog source");
            }
        }
    }
}

pub const MODEL_RULE: uuid::Uuid = uuid::uuid!("00000000-0000-0000-0000-000000000003");
pub const RELAY_RULE: uuid::Uuid = uuid::uuid!("00000000-0000-0000-0000-000000000004");
/// Rule IDs used only by the one-shot guest-crash release. They share the
/// canonical imported secret versions while remaining unambiguous in the
/// broker dispatcher.
pub const CRASH_MODEL_RULE: uuid::Uuid = uuid::uuid!("00000000-0000-0000-0000-000000000005");
pub const CRASH_RELAY_RULE: uuid::Uuid = uuid::uuid!("00000000-0000-0000-0000-000000000006");

/// Converts a preallocated binding spec into the immutable rule consumed by
/// the shared brokered TLS upstream factory.
pub fn brokered_rule_for_spec(
    spec: &super::cooking_adversarial_agent::BrokeredRuleSpec,
) -> BrokeredSecretRule {
    let expected_destination = match spec.slot {
        "model" => "api.model.example",
        "telegram_relay" => "relay.cooking.example",
        _ => panic!("unexpected cooking broker slot"),
    };
    assert_eq!(spec.destination, expected_destination);
    BrokeredSecretRule {
        id: BrokeredSecretRuleId::from_uuid(spec.rule_id),
        binding_id: spec.binding_id,
        instance_revision_id: spec.instance_revision_id,
        secret_version_id: spec.secret_version_id,
        destination: Some(
            ExactHttpsOrigin::parse(format!("https://{}", spec.destination))
                .expect("crash rule origin"),
        ),
        location: HttpInjectionLocation::OutboundHeaderPrefix {
            header: HeaderName::parse("authorization").expect("crash rule header"),
            prefix: String::from("Bearer "),
        },
        gateway_route_id: None,
    }
}
// The update slice adds the held/queued v1 drain pair, one post-rotation relay
// request, and one model-held active-revocation request. The relay endpoint
// must see no corresponding request for the last event.
const EXPECTED_COOKING_REQUESTS: usize = 10;
const BASE_COOKING_REQUESTS: usize = 6;
const MODEL_FAULT_UPDATE: u64 = 45;
const RELAY_FAULT_UPDATE: u64 = 46;

const fn expected_upstream_requests(crash_mode: bool, update_slice: bool, model: bool) -> usize {
    if crash_mode {
        // Crash updates 51..55 have six physical calls per adapter.  The
        // canonical update slice has a different ten/nine-call budget.
        6
    } else if update_slice {
        if model {
            EXPECTED_COOKING_REQUESTS
        } else {
            EXPECTED_COOKING_REQUESTS - 1
        }
    } else {
        BASE_COOKING_REQUESTS
    }
}

pub fn parameters() -> serde_json::Value {
    if enabled() {
        serde_json::json!({"model_rule_id":MODEL_RULE,"relay_rule_id":RELAY_RULE})
    } else {
        serde_json::json!({})
    }
}

// Keep the distinct credential grants and final immutable revision binding
// sequence together so fixture authority can be reviewed in one place.
#[allow(clippy::too_many_lines)]
pub async fn seed_brokered_fixture(
    pool: &sqlx::PgPool,
    actor: UserId,
    organization: OrganizationId,
    project: uuid::Uuid,
    instance: &SeededInstance,
) -> BrokeredFixture {
    sqlx::query("INSERT INTO project_secret_roles (project_id,user_id,role) VALUES ($1,$2,'secret_manager')")
        .bind(project).bind(actor.as_uuid()).execute(pool).await.expect("secret manager");
    let identity = AuthenticatedIdentity::new(
        actor,
        super::golden_issuer(),
        String::from("golden-subject"),
        serde_json::json!({}),
        RequestId::new(),
    );
    let service = SecretService::new(
        pool.clone(),
        EncryptedStore::new(
            LocalKeyProvider::new("golden/v1", [("golden/v1", [17_u8; 32])]).expect("key"),
        ),
        Arc::new(PostgresMelangeAuthorizer),
    );
    let mut revision = instance.revision;
    let mut rules = Vec::new();
    for (slot, host, sentinel) in [
        ("model", "api.model.example", MODEL_SENTINEL),
        ("telegram_relay", "relay.cooking.example", RELAY_SENTINEL),
    ] {
        let secret_id = SecretId::new();
        let version_id = SecretVersionId::new();
        service
            .create(
                &identity,
                CreateSecret {
                    command_key: secret_command_key("cooking-create", secret_id.as_uuid()),
                    secret_id,
                    version_id,
                    owner: SecretOwner::Organization(organization),
                    name: SecretName::parse(format!("cooking_{secret_id}")).expect("name"),
                    allowed_delivery_modes: vec![DeliveryMode::Brokered],
                    value: SecretValue::new(sentinel).expect("sentinel"),
                },
            )
            .await
            .expect("encrypted fixture credential");
        let import_id = SecretImportId::new();
        service
            .grant_and_accept_import(
                &identity,
                GrantAndAcceptSecretImport {
                    command_key: secret_command_key("cooking-grant", import_id.as_uuid()),
                    grant_id: SecretGrantId::new(),
                    secret_id,
                    target: SecretTarget::Project(ProjectId::from_uuid(project)),
                    policy: SecretUsePolicy {
                        delivery_modes: vec![DeliveryMode::Brokered],
                        phases: vec![ExecutionPhase::Normal],
                        destinations: vec![host.to_owned()],
                    },
                    expires_at: None,
                    import_id,
                    alias: SecretAlias::parse(slot).expect("alias"),
                },
            )
            .await
            .expect("grant and import");
        let binding_id = AgentSecretBindingId::new();
        let next = release_domain::AgentInstanceRevisionId::new();
        service
            .bind_secret(
                &identity,
                BindSecret {
                    command_key: secret_command_key("cooking-bind", binding_id.as_uuid()),
                    binding_id,
                    instance_id: release_domain::AgentInstanceId::from_uuid(instance.instance),
                    expected_revision_id: release_domain::AgentInstanceRevisionId::from_uuid(
                        revision,
                    ),
                    new_revision_id: next,
                    import_id,
                    slot: SecretSlotKey::parse(slot).expect("slot"),
                    mode: DeliveryMode::Brokered,
                    phases: vec![ExecutionPhase::Normal],
                    attachment_ids: vec![instance.attachment],
                    destinations: vec![host.to_owned()],
                },
            )
            .await
            .expect("bind cooking slot");
        revision = next.as_uuid();
        let rule_id = if slot == "model" {
            MODEL_RULE
        } else {
            RELAY_RULE
        };
        rules.push(BrokeredSecretRule {
            id: BrokeredSecretRuleId::from_uuid(rule_id),
            binding_id: binding_id.as_uuid(),
            instance_revision_id: revision,
            secret_version_id: version_id.as_uuid(),
            destination: Some(ExactHttpsOrigin::parse(format!("https://{host}")).expect("origin")),
            location: HttpInjectionLocation::OutboundHeaderPrefix {
                header: HeaderName::parse("authorization").expect("header"),
                prefix: String::from("Bearer "),
            },
            gateway_route_id: None,
        });
    }
    for rule in &mut rules {
        let slot = if rule.id.as_uuid() == MODEL_RULE {
            "model"
        } else {
            "telegram_relay"
        };
        rule.binding_id = sqlx::query_scalar(
            "SELECT id FROM agent_secret_bindings WHERE instance_revision_id=$1 AND slot_key=$2",
        )
        .bind(revision)
        .bind(slot)
        .fetch_one(pool)
        .await
        .expect("final immutable binding");
        rule.instance_revision_id = revision;
        let host = if slot == "model" {
            "api.model.example"
        } else {
            "relay.cooking.example"
        };
        service
            .declare_brokered_https_rule(
                &identity,
                DeclareBrokeredHttpsRule {
                    command_key: secret_command_key("cooking-rule", rule.id.as_uuid()),
                    rule_id: rule.id.as_uuid(),
                    binding_id: AgentSecretBindingId::from_uuid(rule.binding_id),
                    destination: format!("https://{host}"),
                    header: String::from("authorization"),
                    header_prefix: Some(String::from("Bearer ")),
                },
            )
            .await
            .expect("declare final cooking rule");
    }
    let upstream = cooking_upstreams(rules).await;
    let (import_id, version_id) =
        seed_inbound_secret(&service, &identity, organization, project).await;
    BrokeredFixture {
        upstream,
        import_id,
        version_id,
    }
}

async fn seed_inbound_secret(
    service: &SecretService<LocalKeyProvider>,
    identity: &AuthenticatedIdentity,
    organization: OrganizationId,
    project: uuid::Uuid,
) -> (uuid::Uuid, uuid::Uuid) {
    let secret_id = SecretId::new();
    let version_id = SecretVersionId::new();
    service
        .create(
            identity,
            CreateSecret {
                command_key: secret_command_key("cooking-inbound", secret_id.as_uuid()),
                secret_id,
                version_id,
                owner: SecretOwner::Organization(organization),
                name: SecretName::parse(format!("cooking_inbound_{secret_id}")).expect("name"),
                allowed_delivery_modes: vec![DeliveryMode::Brokered],
                value: SecretValue::new(INBOUND_SENTINEL).expect("inbound sentinel"),
            },
        )
        .await
        .expect("separate inbound secret");
    let import_id = SecretImportId::new();
    service
        .grant_and_accept_import(
            identity,
            GrantAndAcceptSecretImport {
                command_key: secret_command_key("cooking-inbound-import", import_id.as_uuid()),
                grant_id: SecretGrantId::new(),
                secret_id,
                target: SecretTarget::Project(ProjectId::from_uuid(project)),
                policy: SecretUsePolicy {
                    delivery_modes: vec![DeliveryMode::Brokered],
                    phases: vec![ExecutionPhase::Normal],
                    destinations: vec![],
                },
                expires_at: None,
                import_id,
                alias: SecretAlias::parse("telegram_inbound").expect("inbound alias"),
            },
        )
        .await
        .expect("inbound-only import");
    (import_id.as_uuid(), version_id.as_uuid())
}

pub const INBOUND_SENTINEL: &str = "cooking-inbound-only-fixture-sentinel";
pub const MODEL_SENTINEL: &str = "cooking-model-only-fixture-sentinel-724c";
pub const RELAY_SENTINEL: &str = "cooking-relay-only-fixture-sentinel-819e";
/// Replacement model credential used only after the held v1 lease is pinned.
pub const MODEL_ROTATED_SENTINEL: &str = "cooking-model-rotated-fixture-sentinel-936f";
/// Replacement inbound credential used by the next immutable gateway revision.
pub const INBOUND_ROTATED_SENTINEL: &str = "cooking-inbound-rotated-fixture-sentinel-157a";
/// Replacement relay credential selected after the old lease has been pinned.
pub const RELAY_ROTATED_SENTINEL: &str = "cooking-relay-rotated-fixture-sentinel-482b";
/// All fixture credential sentinels that must remain outside persisted or
/// externally visible evidence.
pub const FIXTURE_CREDENTIAL_SENTINELS: &[&str] = &[
    INBOUND_SENTINEL,
    MODEL_SENTINEL,
    RELAY_SENTINEL,
    MODEL_ROTATED_SENTINEL,
    INBOUND_ROTATED_SENTINEL,
    RELAY_ROTATED_SENTINEL,
];

pub fn assert_no_credentials(value: &str) {
    for sentinel in FIXTURE_CREDENTIAL_SENTINELS {
        assert!(!value.contains(sentinel), "fixture credential exposed");
    }
}

/// Opaque version identities returned by a supported secret rotation.  The
/// values are intentionally kept separate from plaintext credentials so the
/// caller can prove lease pinning without putting secret material in logs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CredentialRotation {
    pub pinned_version_id: uuid::Uuid,
    pub rotated_version_id: uuid::Uuid,
}

/// Rotates the source secret used by one brokered rule after an already
/// admitted run has acquired its lease.  This uses the same authenticated
/// `SecretService` boundary as the application and leaves the old immutable
/// version available for the historical lease query.
pub async fn rotate_brokered_credential(
    pool: &sqlx::PgPool,
    actor: UserId,
    held_run_id: uuid::Uuid,
    rule_id: uuid::Uuid,
    replacement: &str,
) -> CredentialRotation {
    let (secret_id, pinned_version_id): (uuid::Uuid, uuid::Uuid) = sqlx::query_as(
        "SELECT imported.secret_id, snapshot.secret_version_id
           FROM brokered_secret_lease_snapshots snapshot
           JOIN brokered_secret_rules rule ON rule.id = snapshot.rule_id
           JOIN agent_secret_bindings binding ON binding.id = snapshot.binding_id
           JOIN secret_imports imported ON imported.id = binding.import_id
          WHERE snapshot.run_id = $1 AND rule.id = $2",
    )
    .bind(held_run_id)
    .bind(rule_id)
    .fetch_one(pool)
    .await
    .expect("brokered lease source secret");
    let identity = AuthenticatedIdentity::new(
        actor,
        super::golden_issuer(),
        String::from("golden-subject"),
        serde_json::json!({}),
        RequestId::new(),
    );
    let service = SecretService::new(
        pool.clone(),
        EncryptedStore::new(
            LocalKeyProvider::new("golden/v1", [("golden/v1", [17_u8; 32])])
                .expect("cooking rotation key"),
        ),
        Arc::new(PostgresMelangeAuthorizer),
    );
    let rotated_version_id = SecretVersionId::new();
    service
        .rotate(
            &identity,
            RotateSecret {
                command_key: secret_command_key("cooking-rotate-brokered", uuid::Uuid::new_v4()),
                secret_id: SecretId::from_uuid(secret_id),
                expected_active_version_id: SecretVersionId::from_uuid(pinned_version_id),
                new_version_id: rotated_version_id,
                value: SecretValue::new(replacement).expect("replacement sentinel"),
            },
        )
        .await
        .expect("rotate brokered cooking credential");
    let active_version: uuid::Uuid =
        sqlx::query_scalar("SELECT active_version_id FROM secrets WHERE id = $1")
            .bind(secret_id)
            .fetch_one(pool)
            .await
            .expect("rotated brokered active version");
    assert_eq!(active_version, rotated_version_id.as_uuid());
    CredentialRotation {
        pinned_version_id,
        rotated_version_id: active_version,
    }
}

/// Returns the project import selected by a durable brokered rule. This keeps
/// callers on the persisted rule/binding graph when they subsequently revoke
/// the source through `SecretService`.
pub async fn import_id_for_brokered_rule(pool: &sqlx::PgPool, rule_id: uuid::Uuid) -> uuid::Uuid {
    sqlx::query_scalar(
        "SELECT imported.id
           FROM brokered_secret_rules rule
           JOIN agent_secret_bindings binding ON binding.id = rule.binding_id
           JOIN secret_imports imported ON imported.id = binding.import_id
          WHERE rule.id = $1",
    )
    .bind(rule_id)
    .fetch_one(pool)
    .await
    .expect("brokered rule import")
}

/// Rotates the inbound secret selected by a gateway revision.  Gateway
/// reconfiguration is deliberately a separate call: an immutable revision
/// must retain the old selected version while the caller configures a new
/// revision through the normal Gateway RPC.
pub async fn rotate_inbound_credential(
    pool: &sqlx::PgPool,
    actor: UserId,
    import_id: uuid::Uuid,
    expected_version_id: uuid::Uuid,
) -> CredentialRotation {
    let (secret_id, active_version_id): (uuid::Uuid, uuid::Uuid) = sqlx::query_as(
        "SELECT secret_id, active_version_id FROM secret_imports
          JOIN secrets ON secrets.id = secret_imports.secret_id
         WHERE secret_imports.id = $1",
    )
    .bind(import_id)
    .fetch_one(pool)
    .await
    .expect("inbound source secret");
    assert_eq!(active_version_id, expected_version_id);
    rotate_secret_value(
        pool,
        actor,
        secret_id,
        expected_version_id,
        INBOUND_ROTATED_SENTINEL,
        "cooking-rotate-inbound",
    )
    .await
}

async fn rotate_secret_value(
    pool: &sqlx::PgPool,
    actor: UserId,
    secret_id: uuid::Uuid,
    pinned_version_id: uuid::Uuid,
    replacement: &str,
    operation: &str,
) -> CredentialRotation {
    let identity = AuthenticatedIdentity::new(
        actor,
        super::golden_issuer(),
        String::from("golden-subject"),
        serde_json::json!({}),
        RequestId::new(),
    );
    let service = SecretService::new(
        pool.clone(),
        EncryptedStore::new(
            LocalKeyProvider::new("golden/v1", [("golden/v1", [17_u8; 32])])
                .expect("inbound rotation key"),
        ),
        Arc::new(PostgresMelangeAuthorizer),
    );
    let rotated_version_id = SecretVersionId::new();
    service
        .rotate(
            &identity,
            RotateSecret {
                command_key: secret_command_key(operation, uuid::Uuid::new_v4()),
                secret_id: SecretId::from_uuid(secret_id),
                expected_active_version_id: SecretVersionId::from_uuid(pinned_version_id),
                new_version_id: rotated_version_id,
                value: SecretValue::new(replacement).expect("replacement sentinel"),
            },
        )
        .await
        .expect("rotate cooking credential");
    CredentialRotation {
        pinned_version_id,
        rotated_version_id: rotated_version_id.as_uuid(),
    }
}

/// Revokes an imported cooking source through the authenticated secret service
/// and verifies the source/import/binding lifecycle was durably fenced.
pub async fn revoke_imported_credential(pool: &sqlx::PgPool, actor: UserId, import_id: uuid::Uuid) {
    let secret_id: uuid::Uuid =
        sqlx::query_scalar("SELECT secret_id FROM secret_imports WHERE id = $1")
            .bind(import_id)
            .fetch_one(pool)
            .await
            .expect("credential import source");
    let identity = AuthenticatedIdentity::new(
        actor,
        super::golden_issuer(),
        String::from("golden-subject"),
        serde_json::json!({}),
        RequestId::new(),
    );
    let service = SecretService::new(
        pool.clone(),
        EncryptedStore::new(
            LocalKeyProvider::new("golden/v1", [("golden/v1", [17_u8; 32])])
                .expect("credential revoke key"),
        ),
        Arc::new(PostgresMelangeAuthorizer),
    );
    service
        .revoke_secret(
            &identity,
            secret_command_key("cooking-revoke-imported", import_id),
            SecretId::from_uuid(secret_id),
        )
        .await
        .expect("revoke cooking credential");
    let states: (String, String) = sqlx::query_as(
        "SELECT secrets.status, secret_imports.status
           FROM secrets JOIN secret_imports ON secret_imports.secret_id = secrets.id
          WHERE secret_imports.id = $1",
    )
    .bind(import_id)
    .fetch_one(pool)
    .await
    .expect("revoked credential state");
    assert_eq!(states, (String::from("revoked"), String::from("revoked")));
}

/// Compares immutable lease snapshots for an old and a later dispatch of the
/// same rule. This is the durable proof that rotation changes future leases
/// without rewriting the in-flight run's selected version.
pub async fn assert_rotated_brokered_lease(
    pool: &sqlx::PgPool,
    held_run_id: uuid::Uuid,
    later_event_id: uuid::Uuid,
    held_rule_id: uuid::Uuid,
    later_rule_id: uuid::Uuid,
    rotation: CredentialRotation,
) {
    let held: uuid::Uuid = sqlx::query_scalar(
        "SELECT secret_version_id FROM brokered_secret_lease_snapshots
          WHERE run_id = $1 AND rule_id = $2",
    )
    .bind(held_run_id)
    .bind(held_rule_id)
    .fetch_one(pool)
    .await
    .expect("held brokered lease version");
    let later: uuid::Uuid = sqlx::query_scalar(
        "SELECT snapshot.secret_version_id
           FROM mailbox_delivery_attempts attempt
           JOIN brokered_secret_lease_snapshots snapshot
             ON snapshot.run_id = attempt.run_id AND snapshot.rule_id = $2
           WHERE attempt.event_id = $1
          ORDER BY attempt.attempt_number DESC LIMIT 1",
    )
    .bind(later_event_id)
    .bind(later_rule_id)
    .fetch_one(pool)
    .await
    .expect("later brokered lease version");
    assert_eq!(held, rotation.pinned_version_id);
    assert_eq!(later, rotation.rotated_version_id);
    assert_ne!(held, later);
}

pub struct CookingAdapters(
    pub  Arc<
        Mutex<std::collections::HashMap<uuid::Uuid, Arc<dyn secret_application::BrokerAdapter>>>,
    >,
);

/// Explicit fixture-only alias for a copied immutable broker rule.  The
/// production broker still receives the candidate rule ID; this adapter only
/// translates it to the pre-started TLS listener after the exact mapping was
/// registered by the update fixture.
struct RuleCopyAdapter {
    source_rule_id: uuid::Uuid,
    candidate_rule_id: uuid::Uuid,
    source: Arc<dyn secret_application::BrokerAdapter>,
}

#[async_trait::async_trait]
impl secret_application::BrokerAdapter for RuleCopyAdapter {
    async fn invoke(
        &self,
        credential: &SecretValue,
        destination: &str,
        operation: &str,
        body: &[u8],
    ) -> Result<secret_application::BrokerResponse, secret_application::BrokerAdapterError> {
        let body = rewrite_rule_copy_request(body, self.source_rule_id, self.candidate_rule_id)?;
        self.source
            .invoke(credential, destination, operation, &body)
            .await
    }
}

/// Rewrites the two identifier-bearing fields that a copied rule presents to
/// the fixture's source adapter. The production broker has already checked
/// the candidate rule and substituted the credential; this narrow translation
/// preserves the source adapter's strict placeholder check without accepting
/// an arbitrary rule or header value.
fn rewrite_rule_copy_request(
    body: &[u8],
    source_rule_id: uuid::Uuid,
    candidate_rule_id: uuid::Uuid,
) -> Result<Vec<u8>, secret_application::BrokerAdapterError> {
    let mut request: serde_json::Value = serde_json::from_slice(body)
        .map_err(|_| secret_application::BrokerAdapterError::Rejected)?;
    let requested = request
        .get("rule_id")
        .and_then(serde_json::Value::as_str)
        .and_then(|value| value.parse::<uuid::Uuid>().ok())
        .ok_or(secret_application::BrokerAdapterError::Rejected)?;
    if requested != candidate_rule_id {
        return Err(secret_application::BrokerAdapterError::Rejected);
    }
    let headers = request
        .get_mut("headers")
        .and_then(serde_json::Value::as_array_mut)
        .ok_or(secret_application::BrokerAdapterError::Rejected)?;
    let candidate_placeholder = format!("Bearer heph-placeholder:v1:{candidate_rule_id}");
    let source_placeholder = format!("Bearer heph-placeholder:v1:{source_rule_id}");
    let mut replacements = 0_u8;
    for header in headers {
        if header.get("name").and_then(serde_json::Value::as_str) == Some("authorization")
            && header.get("value").and_then(serde_json::Value::as_str)
                == Some(candidate_placeholder.as_str())
        {
            header["value"] = serde_json::Value::String(source_placeholder.clone());
            replacements = replacements.saturating_add(1);
        }
    }
    if replacements != 1 {
        return Err(secret_application::BrokerAdapterError::Rejected);
    }
    request["rule_id"] = serde_json::Value::String(source_rule_id.to_string());
    serde_json::to_vec(&request).map_err(|_| secret_application::BrokerAdapterError::Rejected)
}

impl CookingAdapters {
    pub fn new(
        adapters: std::collections::HashMap<uuid::Uuid, Arc<dyn secret_application::BrokerAdapter>>,
    ) -> Self {
        Self(Arc::new(Mutex::new(adapters)))
    }

    /// Registers only the source/candidate pairs carried by one `CreateUpdate`
    /// request.  A missing source is a fixture construction error; no rule or
    /// origin fallback is permitted.
    pub fn register_rule_copies(&self, copies: &[(uuid::Uuid, uuid::Uuid)]) {
        let mut adapters = self.0.lock().expect("cooking adapter registry");
        for (source_rule_id, candidate_rule_id) in copies {
            let source = adapters
                .get(source_rule_id)
                .cloned()
                .expect("source broker rule adapter registered");
            assert_ne!(source_rule_id, candidate_rule_id);
            assert!(
                !adapters.contains_key(candidate_rule_id),
                "candidate broker rule ID must be allocated once"
            );
            adapters.insert(
                *candidate_rule_id,
                Arc::new(RuleCopyAdapter {
                    source_rule_id: *source_rule_id,
                    candidate_rule_id: *candidate_rule_id,
                    source,
                }),
            );
        }
    }

    pub fn snapshot(
        &self,
    ) -> std::collections::HashMap<uuid::Uuid, Arc<dyn secret_application::BrokerAdapter>> {
        self.0.lock().expect("cooking adapter registry").clone()
    }
}

#[async_trait::async_trait]
impl secret_application::BrokerAdapter for CookingAdapters {
    async fn invoke(
        &self,
        credential: &SecretValue,
        destination: &str,
        operation: &str,
        body: &[u8],
    ) -> Result<secret_application::BrokerResponse, secret_application::BrokerAdapterError> {
        let request: BrokeredHttpsRequest = serde_json::from_slice(body)
            .map_err(|_| secret_application::BrokerAdapterError::Rejected)?;
        let adapter = self
            .0
            .lock()
            .expect("cooking adapter registry")
            .get(&request.rule_id)
            .cloned()
            .ok_or(secret_application::BrokerAdapterError::Rejected)?;
        adapter
            .invoke(credential, destination, operation, body)
            .await
    }
}

// The upstream fixture intentionally keeps TLS, request validation, and the
// bounded request ledger together so the test proves the whole provider edge.
#[allow(clippy::too_many_lines)]
async fn cooking_upstreams(rules: Vec<BrokeredSecretRule>) -> BrokeredTlsUpstream {
    cooking_upstreams_mode(rules, false).await
}

/// Starts the same TLS/ledger upstream factory for a transformed guest
/// release. Crash rules are keyed by immutable rule UUIDs, so canonical and
/// crash listeners can safely coexist on the same daemon adapter.
pub async fn cooking_crash_upstreams(rules: Vec<BrokeredSecretRule>) -> BrokeredTlsUpstream {
    cooking_upstreams_mode(rules, true).await
}

fn credential_class(value: &str) -> &'static str {
    match value {
        MODEL_SENTINEL => "model_initial",
        RELAY_SENTINEL => "relay_initial",
        MODEL_ROTATED_SENTINEL => "model_rotated",
        RELAY_ROTATED_SENTINEL => "relay_rotated",
        _ => "unknown",
    }
}

fn observed_credential_class(headers: &str) -> &'static str {
    for line in headers.lines() {
        let Some(value) = line.strip_prefix("authorization: Bearer ") else {
            continue;
        };
        return credential_class(value);
    }
    "unknown"
}

#[allow(clippy::too_many_lines)]
async fn cooking_upstreams_mode(
    rules: Vec<BrokeredSecretRule>,
    crash_mode: bool,
) -> BrokeredTlsUpstream {
    let update_slice = env::var("HEPHAESTUS_COOKING_UPDATE_E2E").as_deref() == Ok("1");
    let mut adapters = std::collections::HashMap::new();
    let mut servers = Vec::new();
    let relay_directory = tempfile::tempdir().expect("external relay data");
    let relay_database = relay_directory.path().join("relay.sqlite3");
    let update_barrier = update_slice.then(|| Arc::new(super::CookingUpdateBarrier::new()));
    let revocation_barrier = update_slice.then(|| Arc::new(super::CookingUpdateBarrier::new()));
    for rule in rules {
        let rule_id = rule.id.as_uuid();
        let model = rule_id == MODEL_RULE || rule_id == CRASH_MODEL_RULE;
        let expected_requests = expected_upstream_requests(crash_mode, update_slice, model);
        let host = if model {
            "api.model.example"
        } else {
            "relay.cooking.example"
        };
        let _installed = rustls::crypto::ring::default_provider().install_default();
        let mut parameters = rcgen::CertificateParams::default();
        parameters.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        let ca_key = rcgen::KeyPair::generate().expect("CA key");
        let ca = parameters.self_signed(&ca_key).expect("CA");
        let key = rcgen::KeyPair::generate().expect("TLS key");
        let leaf = rcgen::CertificateParams::new(vec![host.to_owned()])
            .expect("TLS name")
            .signed_by(&key, &ca, &ca_key)
            .expect("TLS certificate");
        let tls = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(
                vec![rustls::pki_types::CertificateDer::from(leaf.der().to_vec())],
                rustls::pki_types::PrivateKeyDer::Pkcs8(key.serialize_der().into()),
            )
            .expect("TLS configuration");
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("upstream listener");
        let port = listener.local_addr().expect("upstream address").port();
        let adapter = Arc::new(
            BrokeredHttpsAdapterRegistry::test_only_local_trusted(
                rule,
                port,
                "127.0.0.1".parse().expect("loopback"),
                ca.pem().as_bytes(),
            )
            .expect("pinned TLS adapter"),
        ) as Arc<dyn secret_application::BrokerAdapter>;
        adapters.insert(rule_id, Arc::clone(&adapter));
        let relay_database = relay_database.clone();
        let model_barrier = model.then(|| update_barrier.clone()).flatten();
        let revocation_barrier = model.then(|| revocation_barrier.clone()).flatten();
        servers.push(tokio::spawn(async move {
            let mut requests = 0;
            let calls = Mutex::new(std::collections::BTreeMap::<String, usize>::new());
            for _ in 0..expected_requests {
                let (stream, _) = listener.accept().await.expect("broker TLS request");
                let mut stream = TlsAcceptor::from(Arc::new(tls.clone()))
                    .accept(stream)
                    .await
                    .expect("verified TLS");
                let request = read_http_request(&mut stream).await;
                let separator = request
                    .windows(4)
                    .position(|part| part == b"\r\n\r\n")
                    .expect("HTTP headers");
                let headers =
                    std::str::from_utf8(&request[..separator]).expect("HTTP header encoding");
                assert!(!headers.contains("heph-placeholder:"));
                let body: serde_json::Value = serde_json::from_slice(&request[separator + 4..])
                    .expect("application body");
                let identity = body["idempotency_key"]
                    .as_str()
                    .expect("outbound idempotency key")
                    .to_owned();
                let expected_credential = if !model
                    && matches!(identity.as_str(), "recipe-48" | "recipe-49")
                {
                    RELAY_ROTATED_SENTINEL
                } else if model
                    && update_slice
                    && matches!(identity.as_str(), "recipe-48" | "recipe-49" | "recipe-50")
                {
                    MODEL_ROTATED_SENTINEL
                } else if model {
                    MODEL_SENTINEL
                } else {
                    RELAY_SENTINEL
                };
                let expected_class = credential_class(expected_credential);
                let observed_class = observed_credential_class(headers);
                assert!(
                    observed_class == expected_class,
                    "host substituted credential: identity={identity} expected_class={expected_class} observed_class={observed_class}"
                );
                let call_number = {
                    let mut calls = calls.lock().expect("upstream call ledger");
                    let entry = calls.entry(identity.clone()).or_default();
                    *entry += 1;
                    let call_number = *entry;
                    drop(calls);
                    call_number
                };
                let response = if model {
                    assert!(headers.starts_with("POST /v1/recipes HTTP/1.1"));
                    let user = body["user_id"].as_str().expect("model user");
                    assert!(matches!(user, "alice" | "bob"));
                    let text = body["text"].as_str().expect("model request text");
                    if identity == "recipe-47" {
                        let barrier = model_barrier
                            .as_ref()
                            .expect("update barrier for deferred recipe");
                        barrier.mark_entered();
                        barrier.release.notified().await;
                    }
                    if identity == "recipe-50" {
                        let barrier = revocation_barrier
                            .as_ref()
                            .expect("revocation barrier for active operation");
                        barrier.mark_entered();
                        barrier.release.notified().await;
                    }
                    if user == "alice" && text == "salad" {
                        assert_eq!(
                            body["context"]["summary"],
                            "A simple family recipe",
                            "later Alice request must use persisted SQLite context"
                        );
                    }
                    if !crash_mode
                        && identity == format!("recipe-{MODEL_FAULT_UPDATE}")
                        && call_number == 1
                    {
                        // A valid HTTP response with an invalid application shape
                        // exercises the model-response validation and retry path.
                        serde_json::json!({"fault": "malformed-model-response"})
                    } else {
                        serde_json::json!({
                            "title": format!("Family {text}"),
                            "summary": "A simple family recipe",
                            "ingredients": [text, "tomatoes"],
                            "steps": ["Prepare ingredients", "Serve the family"]
                        })
                    }
                } else {
                    assert!(headers.starts_with("POST /v1/messages HTTP/1.1"));
                    let response = invoke_relay(body, &relay_database, expected_credential).await;
                    if !crash_mode
                        && identity == format!("recipe-{RELAY_FAULT_UPDATE}")
                        && call_number == 1
                    {
                        // invoke_relay has committed its SQLite ledger entry. Closing
                        // before the HTTP response makes the caller observe loss.
                        drop(stream);
                        requests += 1;
                        continue;
                    }
                    response
                };
                let body = serde_json::to_vec(&response).expect("response JSON");
                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                stream
                    .write_all(header.as_bytes())
                    .await
                    .expect("TLS response header");
                stream
                    .write_all(&body)
                    .await
                    .expect("TLS response body");
                requests += 1;
            }
            assert_eq!(
                requests,
                expected_requests,
                "each declared outbound binding receives the planned physical call count"
            );
            let calls = calls.into_inner().expect("upstream call ledger");
            let mut expected = if crash_mode {
                std::collections::BTreeMap::from([
                    (String::from("recipe-51"), 1),
                    (String::from("recipe-52"), 1),
                    (String::from("recipe-53"), if model { 2 } else { 1 }),
                    (String::from("recipe-54"), if model { 1 } else { 2 }),
                    (String::from("recipe-55"), 1),
                ])
            } else {
                std::collections::BTreeMap::from([
                    (String::from("recipe-42"), 1),
                    (String::from("recipe-43"), 1),
                    (String::from("recipe-44"), 1),
                    (String::from("recipe-45"), if model { 2 } else { 1 }),
                    (String::from("recipe-46"), if model { 1 } else { 2 }),
                ])
            };
            if update_slice && !crash_mode {
                expected.insert(String::from("recipe-47"), 1);
                expected.insert(String::from("recipe-48"), 1);
                expected.insert(String::from("recipe-49"), 1);
                if model {
                    expected.insert(String::from("recipe-50"), 1);
                }
            }
            assert_eq!(calls, expected, "bounded outbound calls are recorded per event");
            if !model {
                assert_relay_ledger(&relay_database, update_slice, crash_mode).await;
                if update_slice {
                    assert!(
                        tokio::time::timeout(Duration::from_secs(2), listener.accept())
                            .await
                            .is_err(),
                        "revoked relay operation must not reach the external endpoint"
                    );
                }
            }
        }));
    }
    let observed = Arc::new(AtomicBool::new(false));
    let done = Arc::clone(&observed);
    let server = tokio::spawn(async move {
        // Keep the relay directory alive until all upstream requests and the
        // ledger census complete. Dropping this guard also cleans up on a
        // failing upstream assertion or task panic.
        let _relay_directory = relay_directory;
        for task in servers {
            task.await.expect("cooking upstream task");
        }
        done.store(true, Ordering::SeqCst);
    });
    let registry = Arc::new(CookingAdapters::new(adapters.clone()));
    BrokeredTlsUpstream {
        adapter: Arc::clone(&registry) as Arc<dyn secret_application::BrokerAdapter>,
        cooking_registry: Some(registry),
        rule_adapters: Arc::new(adapters),
        observed,
        server,
        update_barrier,
        revocation_barrier,
    }
}

async fn read_http_request(
    stream: &mut tokio_rustls::server::TlsStream<tokio::net::TcpStream>,
) -> Vec<u8> {
    let mut request = Vec::new();
    loop {
        let mut chunk = [0_u8; 2048];
        let length = stream
            .read(&mut chunk)
            .await
            .expect("read bounded HTTP request");
        assert_ne!(length, 0);
        request.extend_from_slice(&chunk[..length]);
        assert!(request.len() <= 32768);
        if let Some(separator) = request.windows(4).position(|part| part == b"\r\n\r\n") {
            let headers = std::str::from_utf8(&request[..separator]).expect("headers");
            let length: usize = headers
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length: ")
                        .map(str::to_owned)
                })
                .expect("content length")
                .parse()
                .expect("bounded length");
            if request.len() == separator + 4 + length {
                return request;
            }
        }
    }
}

async fn invoke_relay(
    body: serde_json::Value,
    database: &Path,
    credential: &str,
) -> serde_json::Value {
    let script = "import importlib.util,json,sys\nspec=importlib.util.spec_from_file_location('relay',sys.argv[1]); module=importlib.util.module_from_spec(spec); spec.loader.exec_module(module)\nrequest=json.load(sys.stdin); relay=module.Relay(sys.argv[2],request['credential'].encode()); status,body=relay.deliver('Bearer '+request['credential'],json.dumps(request['body']).encode()); assert status==200; print(json.dumps(body))";
    let mut child = tokio::process::Command::new("python3")
        .arg("-c")
        .arg(script)
        .arg(source_root().join("telegram-relay/relay.py"))
        .arg(database)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("external relay application");
    child
        .stdin
        .take()
        .expect("relay input")
        .write_all(
            &serde_json::to_vec(&serde_json::json!({"credential":credential,"body":body}))
                .expect("relay input JSON"),
        )
        .await
        .expect("relay input");
    let output = child
        .wait_with_output()
        .await
        .expect("relay application outcome");
    assert!(
        output.status.success(),
        "external relay rejected bounded request"
    );
    serde_json::from_slice(&output.stdout).expect("relay redacted response")
}

const fn expected_relay_ledger(update_slice: bool, crash_mode: bool) -> &'static str {
    if crash_mode {
        "['recipe-51','recipe-52','recipe-53','recipe-54','recipe-55']"
    } else if update_slice {
        "['recipe-42','recipe-43','recipe-44','recipe-45','recipe-46','recipe-47','recipe-48','recipe-49']"
    } else {
        "['recipe-42','recipe-43','recipe-44','recipe-45','recipe-46']"
    }
}

async fn assert_relay_ledger(database: &Path, update_slice: bool, crash_mode: bool) {
    let expected = expected_relay_ledger(update_slice, crash_mode);
    let script = format!(
        "import sqlite3,sys\nrows=sqlite3.connect(sys.argv[1]).execute('SELECT key FROM deliveries ORDER BY key').fetchall()\nassert [row[0] for row in rows] == {expected}, rows"
    );
    let output = tokio::process::Command::new("python3")
        .arg("-c")
        .arg(script)
        .arg(database)
        .output()
        .await
        .expect("relay ledger census");
    assert!(
        output.status.success(),
        "relay ledger must retain exactly one row per logical recipe"
    );
}

#[derive(Clone, Copy)]
pub struct CookingRun {
    pub event_id: uuid::Uuid,
    pub run_id: runtime_types::RunId,
}

type CookingRunRow = (
    uuid::Uuid,
    uuid::Uuid,
    String,
    Option<String>,
    Option<String>,
);

type CookingRunSuccessRow = (uuid::Uuid, uuid::Uuid, String, Option<String>);

#[derive(Clone, Copy)]
struct CookingRunObservation {
    event_id: uuid::Uuid,
    run_id: uuid::Uuid,
    state: &'static str,
    outcome: &'static str,
}

fn safe_run_state(value: &str) -> &'static str {
    match value {
        "queued" => "queued",
        "leasing_volume" => "leasing_volume",
        "provisioning" => "provisioning",
        "starting" => "starting",
        "running" => "running",
        "succeeded" => "succeeded",
        "failed" => "failed",
        "cancelled" => "cancelled",
        "cleaned_up" => "cleaned_up",
        _ => "unknown",
    }
}

fn safe_run_outcome(value: Option<&str>) -> &'static str {
    match value {
        None => "none",
        Some("succeeded") => "succeeded",
        Some("failed") => "failed",
        Some("cancelled") => "cancelled",
        Some(_) => "unknown",
    }
}

/// Runs admitted before the daemon restart; the follow-up reuses this frozen
/// provenance after a fresh supervisor boot.
pub struct CookingCheckpoint {
    alice: CookingRun,
    bob: CookingRun,
}

impl CookingCheckpoint {
    /// Returns the exact canonical operations whose broker effects form the
    /// post-ingress positive-control baseline.
    pub const fn run_ids(&self) -> [uuid::Uuid; 2] {
        [self.alice.run_id.as_uuid(), self.bob.run_id.as_uuid()]
    }
}

/// Waits until both canonical positive-control operations have reached their
/// terminal cleanup state before an audit baseline is captured.
pub async fn wait_for_checkpoint_runs(
    pool: &sqlx::PgPool,
    checkpoint: &CookingCheckpoint,
    timeout: Duration,
) {
    let run_ids = checkpoint.run_ids();
    tokio::time::timeout(timeout, async {
        loop {
            let settled: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM runs
                  WHERE id = ANY($1)
                    AND state = 'cleaned_up'
                    AND outcome = 'succeeded'",
            )
            .bind(run_ids.to_vec())
            .fetch_one(pool)
            .await
            .expect("canonical cooking runs settle");
            if settled == i64::try_from(run_ids.len()).expect("checkpoint size fits i64") {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("canonical cooking runs cleanup");
}

pub async fn send_update(
    client: &reqwest::Client,
    url: &str,
    update_id: u64,
    provider_id: u64,
    text: &str,
) -> reqwest::Response {
    send_update_with_credential(client, url, update_id, provider_id, text, INBOUND_SENTINEL).await
}

/// Sends one normalized cooking update with an explicitly selected inbound
/// credential. The credential parameter exists only for the rotation probe;
/// ordinary scenario requests continue through [`send_update`].
pub async fn send_update_with_credential(
    client: &reqwest::Client,
    url: &str,
    update_id: u64,
    provider_id: u64,
    text: &str,
    credential: &str,
) -> reqwest::Response {
    client
        .post(url)
        .header("x-telegram-bot-api-secret-token", credential)
        .json(&serde_json::json!({
            "update_id": update_id,
            "message": {"from": {"id": provider_id}, "text": text}
        }))
        .send()
        .await
        .expect("cooking ingress request")
}

async fn send_raw(
    client: &reqwest::Client,
    url: &str,
    body: Vec<u8>,
    credential: Option<&str>,
) -> reqwest::Response {
    let mut request = client.post(url).body(body);
    if let Some(value) = credential {
        request = request.header("x-telegram-bot-api-secret-token", value);
    }
    request.send().await.expect("cooking ingress request")
}

async fn mailbox_counts(pool: &sqlx::PgPool, gateway: &GatewayGoldenFixture) -> (i64, i64, i64) {
    sqlx::query_as(
        "SELECT count(*) FILTER (WHERE outcome = 'accepted'),
                count(*) FILTER (WHERE outcome = 'duplicate'),
                (SELECT count(*) FROM mailbox_events WHERE mailbox_id = $1)
           FROM gateway_mailbox_publications
          WHERE mailbox_id = $1",
    )
    .bind(gateway.mailbox_id.as_uuid())
    .fetch_one(pool)
    .await
    .expect("cooking mailbox publication counts")
}

async fn assert_normalized_mailbox_body(
    pool: &sqlx::PgPool,
    event_id: uuid::Uuid,
    update_id: u64,
    user_id: &str,
    text: &str,
) {
    let (method, route, headers, content_type, body): (
        String,
        String,
        serde_json::Value,
        Option<String>,
        Vec<u8>,
    ) = sqlx::query_as(
        "SELECT event.method, event.route, event.selected_headers,
                event.content_type, payload.encoded_body
           FROM mailbox_events event
           JOIN mailbox_payloads payload ON payload.id = event.body_id
          WHERE event.id = $1",
    )
    .bind(event_id)
    .fetch_one(pool)
    .await
    .expect("normalized mailbox envelope");
    assert_eq!(method, "POST");
    assert_eq!(route, "/telegram/updates");
    assert_eq!(
        headers,
        serde_json::json!({
            "x-cooking-event-kind": "cooking.telegram.received.v1"
        })
    );
    assert_eq!(content_type.as_deref(), Some("application/json"));
    let normalized: serde_json::Value =
        serde_json::from_slice(&body).expect("normalized cooking event JSON");
    assert_eq!(
        normalized,
        serde_json::json!({
            "provider_update_id": update_id,
            "user_id": user_id,
            "command": "recipe",
            "text": text,
        })
    );
    assert!(
        !normalized
            .to_string()
            .contains("telegram-bot-api-secret-token")
    );
    assert_no_credentials(&normalized.to_string());
}

pub async fn wait_for_event_run(
    pool: &sqlx::PgPool,
    gateway: &GatewayGoldenFixture,
    update_id: u64,
) -> CookingRun {
    let key = format!("telegram-update-{update_id}");
    let mut latest_observation = None;
    let mut next_lineage_snapshot = tokio::time::Instant::now();
    let result = tokio::time::timeout(Duration::from_secs(90), async {
        loop {
            if tokio::time::Instant::now() >= next_lineage_snapshot {
                write_cooking_lineage_snapshot(pool, gateway.mailbox_id.as_uuid(), None).await;
                next_lineage_snapshot = tokio::time::Instant::now() + Duration::from_secs(1);
            }
            let row: Option<CookingRunSuccessRow> = sqlx::query_as(
                "SELECT publication.event_id, run.id, run.state, run.outcome
                   FROM gateway_mailbox_publications publication
                   JOIN mailbox_events event ON event.id = publication.event_id
                   JOIN mailbox_delivery_attempts attempt ON attempt.event_id = event.id
                   JOIN runs run ON run.id = attempt.run_id
                  WHERE publication.mailbox_id = $1
                    AND publication.deduplication_key = $2
                    AND publication.outcome IN ('accepted', 'duplicate')
                  ORDER BY attempt.attempt_number DESC
                  LIMIT 1",
            )
            .bind(gateway.mailbox_id.as_uuid())
            .bind(&key)
            .fetch_optional(pool)
            .await
            .expect("cooking run by exact event key");
            if let Some((event_id, run_id, state, outcome)) = row {
                latest_observation = Some(CookingRunObservation {
                    event_id,
                    run_id,
                    state: safe_run_state(&state),
                    outcome: safe_run_outcome(outcome.as_deref()),
                });
                if state == "cleaned_up" && outcome.as_deref() == Some("succeeded") {
                    break CookingRun {
                        event_id,
                        run_id: runtime_types::RunId::from_uuid(run_id),
                    };
                }
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await;
    result.unwrap_or_else(|_| {
        let latest = latest_observation.map_or_else(
            || String::from("none"),
            |observation| {
                format!(
                    "event_id={} run_id={} state={} outcome={}",
                    observation.event_id,
                    observation.run_id,
                    observation.state,
                    observation.outcome
                )
            },
        );
        panic!(
            "cooking mailbox run completes: mailbox_id={} update_id={} latest={latest}",
            gateway.mailbox_id.as_uuid(),
            update_id
        );
    })
}

/// Waits for one exact mailbox event to fail after its run has cleaned up.
/// The returned failure is retained so the caller can assert the typed
/// broker-denial category without exposing provider values in diagnostics.
pub async fn wait_for_failed_event_run(
    pool: &sqlx::PgPool,
    gateway: &GatewayGoldenFixture,
    update_id: u64,
) -> CookingRun {
    let key = format!("telegram-update-{update_id}");
    let mut next_lineage_snapshot = tokio::time::Instant::now();
    tokio::time::timeout(Duration::from_secs(90), async {
        loop {
            if tokio::time::Instant::now() >= next_lineage_snapshot {
                write_cooking_lineage_snapshot(pool, gateway.mailbox_id.as_uuid(), None).await;
                next_lineage_snapshot = tokio::time::Instant::now() + Duration::from_secs(1);
            }
            let row: Option<CookingRunRow> = sqlx::query_as(
                "SELECT publication.event_id, run.id, run.state, run.outcome, run.failure
                   FROM gateway_mailbox_publications publication
                   JOIN mailbox_events event ON event.id = publication.event_id
                   JOIN mailbox_delivery_attempts attempt ON attempt.event_id = event.id
                   JOIN runs run ON run.id = attempt.run_id
                  WHERE publication.mailbox_id = $1
                    AND publication.deduplication_key = $2
                    AND publication.outcome = 'accepted'
                  ORDER BY attempt.attempt_number DESC LIMIT 1",
            )
            .bind(gateway.mailbox_id.as_uuid())
            .bind(&key)
            .fetch_optional(pool)
            .await
            .expect("failed cooking run by exact event key");
            if let Some((event_id, run_id, state, outcome, _failure)) = row {
                if state == "cleaned_up" && outcome.as_deref() == Some("failed") {
                    break CookingRun {
                        event_id,
                        run_id: runtime_types::RunId::from_uuid(run_id),
                    };
                }
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("revoked relay cooking run completes")
}

/// Holds a real model operation, revokes the relay source while its run and
/// lease are active, then releases the model. The run must fail at broker
/// admission before any relay request or relay ledger mutation occurs.
pub async fn exercise_active_relay_revocation(
    pool: &sqlx::PgPool,
    gateway: &GatewayGoldenFixture,
    actor: UserId,
    upstream: &BrokeredTlsUpstream,
    relay_rule_id: uuid::Uuid,
) {
    let public = env::var("HEPHAESTUS_CADDY_TEST_PUBLIC_URL").expect("public Caddy URL");
    let url = format!("{public}/gateway/cooking/telegram");
    let client = reqwest::Client::new();
    let accepted = send_update_with_credential(
        &client,
        &url,
        50,
        1001,
        "revoked-relay",
        INBOUND_ROTATED_SENTINEL,
    )
    .await;
    assert_eq!(accepted.status(), reqwest::StatusCode::OK);
    accepted
        .bytes()
        .await
        .expect("active revocation acknowledgement");
    upstream.wait_revocation_entered().await;
    let active = wait_for_active_event_run(
        pool,
        gateway.mailbox_id.as_uuid(),
        Duration::from_secs(90),
        50,
    )
    .await;
    let active_relay_leases: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM brokered_secret_lease_snapshots snapshot
          JOIN secret_leases lease ON lease.id = snapshot.lease_id
         WHERE snapshot.run_id = $1 AND snapshot.rule_id = $2
           AND lease.status = 'active' AND lease.expires_at > now()",
    )
    .bind(active.run_id.as_uuid())
    .bind(relay_rule_id)
    .fetch_one(pool)
    .await
    .expect("active relay lease before revocation");
    assert_eq!(
        active_relay_leases, 1,
        "relay lease is pinned before revoke"
    );
    let relay_import = import_id_for_brokered_rule(pool, relay_rule_id).await;
    revoke_imported_credential(pool, actor, relay_import).await;
    upstream.release_revocation();
    let failed = wait_for_failed_event_run(pool, gateway, 50).await;
    assert_eq!(failed.event_id, active.event_id);
    assert_eq!(
        failed.run_id, active.run_id,
        "relay denial must settle the held operation rather than create a replacement run"
    );
    let denied_decisions: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM brokered_secret_audit_events
          WHERE run_id = $1 AND rule_id = $2
            AND event_kind = 'authorization_decision' AND decision = 'deny'",
    )
    .bind(failed.run_id.as_uuid())
    .bind(relay_rule_id)
    .fetch_one(pool)
    .await
    .expect("typed relay broker denial audit");
    assert_eq!(denied_decisions, 1, "relay denial is durably typed");
    let relay_uses: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM brokered_secret_audit_events
          WHERE run_id = $1 AND rule_id = $2 AND event_kind = 'substitution_use'",
    )
    .bind(failed.run_id.as_uuid())
    .bind(relay_rule_id)
    .fetch_one(pool)
    .await
    .expect("revoked relay substitution audit");
    assert_eq!(
        relay_uses, 0,
        "denied relay has no logical substitution use"
    );
    let relay_attempts: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM brokered_secret_lease_snapshots
          WHERE run_id = $1 AND rule_id = $2",
    )
    .bind(failed.run_id.as_uuid())
    .bind(relay_rule_id)
    .fetch_one(pool)
    .await
    .expect("revoked relay lease history");
    assert_eq!(
        relay_attempts, 1,
        "the denied run retains one pinned relay lease"
    );
}

pub async fn wait_for_event_id(
    pool: &sqlx::PgPool,
    mailbox_id: uuid::Uuid,
    timeout: Duration,
    update_id: u64,
) -> uuid::Uuid {
    let key = format!("telegram-update-{update_id}");
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let event_id: Option<uuid::Uuid> = sqlx::query_scalar(
            "SELECT event_id FROM gateway_mailbox_publications
              WHERE mailbox_id = $1
                AND deduplication_key = $2 AND outcome = 'accepted'
              ORDER BY accepted_at DESC LIMIT 1",
        )
        .bind(mailbox_id)
        .bind(&key)
        .fetch_optional(pool)
        .await
        .expect("deferred cooking event ID");
        if let Some(event_id) = event_id {
            return event_id;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "cooking event publication timed out"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

pub async fn wait_for_active_event_run(
    pool: &sqlx::PgPool,
    mailbox_id: uuid::Uuid,
    timeout: Duration,
    update_id: u64,
) -> CookingRun {
    let key = format!("telegram-update-{update_id}");
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let row: Option<(uuid::Uuid, uuid::Uuid, String)> = sqlx::query_as(
            "SELECT publication.event_id, run.id, run.state
               FROM gateway_mailbox_publications publication
               JOIN mailbox_delivery_attempts attempt ON attempt.event_id = publication.event_id
               JOIN runs run ON run.id = attempt.run_id
              WHERE publication.mailbox_id = $1
                AND publication.deduplication_key = $2
                AND publication.outcome = 'accepted'
              ORDER BY attempt.attempt_number DESC LIMIT 1",
        )
        .bind(mailbox_id)
        .bind(&key)
        .fetch_optional(pool)
        .await
        .expect("active cooking event run");
        if let Some((event_id, run_id, state)) = row {
            if [
                "queued",
                "leasing_volume",
                "provisioning",
                "starting",
                "running",
            ]
            .contains(&state.as_str())
            {
                return CookingRun {
                    event_id,
                    run_id: runtime_types::RunId::from_uuid(run_id),
                };
            }
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "active cooking event run timed out"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

async fn deliver_request(
    pool: &sqlx::PgPool,
    gateway: &GatewayGoldenFixture,
    client: &reqwest::Client,
    url: &str,
    update_id: u64,
    provider_id: u64,
    text: &str,
) -> CookingRun {
    let accepted = send_update(client, url, update_id, provider_id, text).await;
    assert_eq!(accepted.status(), reqwest::StatusCode::OK);
    accepted
        .bytes()
        .await
        .expect("cooking acknowledgement body");
    wait_for_event_run(pool, gateway, update_id).await
}

async fn wait_for_retried_attempt_completion(
    pool: &sqlx::PgPool,
    mailbox_id: uuid::Uuid,
    event_id: uuid::Uuid,
    run_id: uuid::Uuid,
) {
    // `wait_for_event_run` observes the run cleanup transaction. The mailbox
    // completion observer settles its exact attempt in a following transaction,
    // so this assertion may briefly see the returned successful run's attempt
    // as leased or running. Wait only for that known attempt to reach completed.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let state: Option<String> = sqlx::query_scalar(
            "SELECT state FROM mailbox_delivery_attempts
              WHERE event_id = $1 AND attempt_number = 2 AND run_id = $2",
        )
        .bind(event_id)
        .bind(run_id)
        .fetch_optional(pool)
        .await
        .expect("durable retry attempt projection");
        match state.as_deref() {
            Some("completed") => return,
            Some("leased" | "running") => {}
            Some(other) => {
                write_retry_failure_evidence(pool, mailbox_id, event_id, run_id, Some(other)).await;
                panic!("retry attempt entered unexpected state: {other}");
            }
            None => {
                write_retry_failure_evidence(pool, mailbox_id, event_id, run_id, None).await;
                panic!("successful retry attempt identity is absent");
            }
        }
        let within_deadline = tokio::time::Instant::now() < deadline;
        if !within_deadline {
            write_retry_failure_evidence(pool, mailbox_id, event_id, run_id, Some("timeout")).await;
        }
        assert!(
            within_deadline,
            "successful retry attempt completion projection timed out"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

async fn write_retry_failure_evidence(
    pool: &sqlx::PgPool,
    mailbox_id: uuid::Uuid,
    event_id: uuid::Uuid,
    run_id: uuid::Uuid,
    observed_state: Option<&str>,
) {
    // Flush the exact mailbox lineage before the panic is propagated.  The
    // periodic snapshot can predate the retry's terminal transition, so this
    // final, event-scoped sample captures the latest state before the panic.
    write_cooking_lineage_snapshot(pool, mailbox_id, Some(event_id)).await;

    let lookup = tokio::time::timeout(
        Duration::from_secs(2),
        sqlx::query_as::<_, RetryAttemptRow>(
            "SELECT attempt.id, attempt.attempt_number, attempt.state,
                    run.state, run.outcome, run.exit_code, run.exit_signal
               FROM mailbox_delivery_attempts attempt
               JOIN runs run ON run.id = attempt.run_id
              WHERE attempt.event_id = $1 AND attempt.run_id = $2
              ORDER BY attempt.attempt_number DESC LIMIT 1",
        )
        .bind(event_id)
        .bind(run_id)
        .fetch_optional(pool),
    )
    .await;
    let evidence = match lookup {
        Ok(Ok(Some(row))) => RetryAttemptEvidence::from_row(&row, observed_state, "ok"),
        Ok(Ok(None)) => RetryAttemptEvidence::missing(observed_state),
        Ok(Err(_)) => RetryAttemptEvidence::unavailable(observed_state, "query-failed"),
        Err(_) => RetryAttemptEvidence::unavailable(observed_state, "query-timeout"),
    };
    let attempt_id = evidence
        .attempt_id
        .map_or_else(|| String::from("unknown"), |value| value.to_string());
    let attempt_number = evidence
        .attempt_number
        .map_or_else(|| String::from("unknown"), |value| value.to_string());
    let exit_code = evidence
        .exit_code
        .map_or_else(|| String::from("none"), |value| value.to_string());
    let exit_signal = evidence
        .exit_signal
        .map_or_else(|| String::from("none"), |value| value.to_string());
    let classification = match observed_state {
        Some("timeout") => "retry-completion-timeout",
        _ => match evidence.attempt_state {
            "failed" => "retry-terminal-failed",
            "uncertain" => "retry-terminal-uncertain",
            _ => "retry-terminal-unresolved",
        },
    };
    let marker = format!(
        concat!(
            "HEPH_COOKING_RETRY event=terminal classification={classification} ",
            "lookup_status={lookup_status} event_id={event_id} attempt_id={attempt_id} ",
            "attempt_number={attempt_number} run_id={run_id} attempt_state={attempt_state} ",
            "run_state={run_state} run_outcome={run_outcome} exit_code={exit_code} ",
            "exit_signal={exit_signal}"
        ),
        classification = classification,
        lookup_status = evidence.lookup_status,
        event_id = event_id,
        attempt_id = attempt_id,
        attempt_number = attempt_number,
        run_id = run_id,
        attempt_state = evidence.attempt_state,
        run_state = evidence.run_state,
        run_outcome = evidence.run_outcome,
        exit_code = exit_code,
        exit_signal = exit_signal,
    );
    let mut stderr = std::io::stderr();
    let _ = writeln!(stderr, "{marker}");
    let _ = stderr.flush();
}

type RetryAttemptRow = (
    uuid::Uuid,
    i32,
    String,
    String,
    Option<String>,
    Option<i32>,
    Option<i32>,
);

struct RetryAttemptEvidence {
    attempt_id: Option<uuid::Uuid>,
    attempt_number: Option<i32>,
    attempt_state: &'static str,
    run_state: &'static str,
    run_outcome: &'static str,
    exit_code: Option<i32>,
    exit_signal: Option<i32>,
    lookup_status: &'static str,
}

impl RetryAttemptEvidence {
    fn from_row(
        row: &RetryAttemptRow,
        observed_state: Option<&str>,
        lookup_status: &'static str,
    ) -> Self {
        Self {
            attempt_id: Some(row.0),
            attempt_number: Some(row.1),
            attempt_state: safe_retry_attempt_state(Some(row.2.as_str()), observed_state),
            run_state: safe_retry_run_state(Some(row.3.as_str())),
            run_outcome: safe_retry_run_outcome(row.4.as_deref()),
            exit_code: row.5,
            exit_signal: row.6,
            lookup_status,
        }
    }

    fn missing(observed_state: Option<&str>) -> Self {
        Self {
            attempt_id: None,
            attempt_number: None,
            attempt_state: safe_retry_attempt_state(None, observed_state),
            run_state: "unknown",
            run_outcome: "none",
            exit_code: None,
            exit_signal: None,
            lookup_status: "missing",
        }
    }

    fn unavailable(observed_state: Option<&str>, lookup_status: &'static str) -> Self {
        Self {
            attempt_id: None,
            attempt_number: None,
            attempt_state: safe_retry_attempt_state(None, observed_state),
            run_state: "unknown",
            run_outcome: "none",
            exit_code: None,
            exit_signal: None,
            lookup_status,
        }
    }
}

fn safe_retry_attempt_state(row_state: Option<&str>, observed_state: Option<&str>) -> &'static str {
    match row_state.or(observed_state) {
        Some("leased") => "leased",
        Some("running") => "running",
        Some("completed") => "completed",
        Some("failed") => "failed",
        Some("uncertain") => "uncertain",
        _ => "unknown",
    }
}

fn safe_retry_run_state(value: Option<&str>) -> &'static str {
    match value {
        Some("queued") => "queued",
        Some("leasing_volume") => "leasing_volume",
        Some("provisioning") => "provisioning",
        Some("starting") => "starting",
        Some("running") => "running",
        Some("succeeded") => "succeeded",
        Some("failed") => "failed",
        Some("cancelled") => "cancelled",
        Some("cleaning_up") => "cleaning_up",
        Some("cleaned_up") => "cleaned_up",
        _ => "unknown",
    }
}

fn safe_retry_run_outcome(value: Option<&str>) -> &'static str {
    match value {
        None => "none",
        Some("succeeded") => "succeeded",
        Some("failed") => "failed",
        Some("cancelled") => "cancelled",
        _ => "unknown",
    }
}

async fn assert_retried_delivery(
    pool: &sqlx::PgPool,
    mailbox_id: uuid::Uuid,
    successful_run: CookingRun,
) {
    wait_for_retried_attempt_completion(
        pool,
        mailbox_id,
        successful_run.event_id,
        successful_run.run_id.as_uuid(),
    )
    .await;

    let logical_attempts: i32 = sqlx::query_scalar(
        "SELECT logical_attempt_count FROM mailbox_deliveries WHERE event_id = $1",
    )
    .bind(successful_run.event_id)
    .fetch_one(pool)
    .await
    .expect("durable outbound fault retry count");
    assert_eq!(
        logical_attempts, 2,
        "fault recovery creates one explicit retry"
    );

    let attempts: Vec<(i32, uuid::Uuid, String, String, Option<String>)> = sqlx::query_as(
        "SELECT attempt.attempt_number, attempt.run_id, attempt.state,
                    run.state, run.outcome
               FROM mailbox_delivery_attempts attempt
               JOIN runs run ON run.id = attempt.run_id
              WHERE attempt.event_id = $1
              ORDER BY attempt.attempt_number",
    )
    .bind(successful_run.event_id)
    .fetch_all(pool)
    .await
    .expect("durable outbound fault retry attempts");
    assert_eq!(attempts.len(), 2, "fault recovery retains both attempts");
    assert_eq!(attempts[0].0, 1, "first fault attempt is numbered one");
    assert_eq!(attempts[1].0, 2, "retry attempt is numbered two");
    assert_ne!(
        attempts[0].1, attempts[1].1,
        "fault recovery creates distinct runs"
    );
    assert!(
        matches!(attempts[0].2.as_str(), "failed" | "uncertain"),
        "first fault attempt remains inspectable with an explicit terminal state: {}",
        attempts[0].2
    );
    assert_eq!(attempts[0].3, "cleaned_up");
    assert_eq!(
        attempts[0].4.as_deref(),
        Some("failed"),
        "first fault run is durably failed"
    );
    assert_eq!(
        attempts[1].2, "completed",
        "only the retry completes the attempt"
    );
    assert_eq!(attempts[1].3, "cleaned_up");
    assert_eq!(
        attempts[1].4.as_deref(),
        Some("succeeded"),
        "only the retry completes the run"
    );
    assert_eq!(
        attempts[1].1,
        successful_run.run_id.as_uuid(),
        "returned run is the durable successful retry"
    );
}

// This is an acceptance scenario whose branches each assert a separate
// boundary; keeping the sequence visible makes failures directly actionable.
#[allow(clippy::cognitive_complexity)]
pub async fn exercise_initial(
    pool: &sqlx::PgPool,
    gateway: &GatewayGoldenFixture,
) -> CookingCheckpoint {
    let public = env::var("HEPHAESTUS_CADDY_TEST_PUBLIC_URL").expect("public Caddy URL");
    let url = format!("{public}/gateway/cooking/telegram");
    let client = reqwest::Client::new();
    let baseline = mailbox_counts(pool, gateway).await;
    for credential in [None, Some("wrong-inbound-value")] {
        let response = send_raw(
            &client,
            &url,
            serde_json::to_vec(&serde_json::json!({
                "update_id": 40,
                "message": {"from": {"id": 1001}, "text": "pasta"}
            }))
            .expect("invalid verification body"),
            credential,
        )
        .await;
        assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);
        assert!(response.bytes().await.expect("empty denial").is_empty());
        assert_eq!(mailbox_counts(pool, gateway).await, baseline);
    }
    let unknown = send_raw(
        &client,
        &url,
        serde_json::to_vec(&serde_json::json!({
            "update_id": 41,
            "message": {"from": {"id": 9999}, "text": "pasta"}
        }))
        .expect("unknown identity body"),
        Some(INBOUND_SENTINEL),
    )
    .await;
    assert_eq!(unknown.status(), reqwest::StatusCode::FORBIDDEN);
    unknown.bytes().await.expect("unknown identity response");
    assert_eq!(mailbox_counts(pool, gateway).await, baseline);

    let malformed = send_raw(&client, &url, b"{".to_vec(), Some(INBOUND_SENTINEL)).await;
    assert_eq!(malformed.status(), reqwest::StatusCode::BAD_REQUEST);
    malformed.bytes().await.expect("malformed response");
    assert_eq!(mailbox_counts(pool, gateway).await, baseline);
    let oversized = send_raw(&client, &url, vec![b'x'; 16_385], Some(INBOUND_SENTINEL)).await;
    assert_eq!(oversized.status(), reqwest::StatusCode::BAD_REQUEST);
    oversized.bytes().await.expect("oversized response");
    assert_eq!(mailbox_counts(pool, gateway).await, baseline);

    let proxy = super::cooking_ingress_loss::IngressLossProxy::bind(&public)
        .await
        .expect("bind post-commit ingress response-loss proxy");
    let (alice, bob) = super::cooking_ingress_loss::exercise(
        &super::cooking_ingress_loss::IngressLossContext {
            pool,
            mailbox_id: gateway.mailbox_id.as_uuid(),
            caddy_public_url: &public,
            inbound_credential: INBOUND_SENTINEL,
            timeout: Duration::from_secs(90),
        },
        proxy,
    )
    .await
    .expect("post-commit ingress response-loss and replay");
    assert_ne!(alice.event_id, bob.event_id);
    assert_normalized_mailbox_body(pool, alice.event_id, 42, "alice", "pasta").await;
    assert_normalized_mailbox_body(pool, bob.event_id, 43, "bob", "soup").await;

    let duplicate = send_update(&client, &url, 42, 1001, "pasta").await;
    assert_eq!(duplicate.status(), reqwest::StatusCode::OK);
    duplicate.bytes().await.expect("duplicate acknowledgement");
    let duplicate_count = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let count: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM gateway_mailbox_publications
                  WHERE mailbox_id = $1 AND deduplication_key = 'telegram-update-42'
                    AND outcome = 'duplicate'",
            )
            .bind(gateway.mailbox_id.as_uuid())
            .fetch_one(pool)
            .await
            .expect("duplicate ingress publication");
            if count == 2 {
                break count;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("duplicate ingress settles");
    assert_eq!(duplicate_count, 2);
    assert_eq!(mailbox_counts(pool, gateway).await, (2, 2, 2));

    CookingCheckpoint { alice, bob }
}

// The follow-up retains the full provenance and approval assertions in one
// reviewable post-restart slice.
#[allow(
    clippy::cognitive_complexity,
    clippy::too_many_arguments,
    clippy::too_many_lines
)]
pub async fn exercise_follow_up(
    pool: &sqlx::PgPool,
    running: &hephaestus_app::RunningHephaestus,
    instance: &SeededInstance,
    gateway: &GatewayGoldenFixture,
    root: &Path,
    repository: uuid::Uuid,
    input_commit: &str,
    checkpoint: CookingCheckpoint,
    _upstream: &BrokeredTlsUpstream,
) -> String {
    let CookingCheckpoint { alice, bob } = checkpoint;
    let public = env::var("HEPHAESTUS_CADDY_TEST_PUBLIC_URL").expect("public Caddy URL");
    let url = format!("{public}/gateway/cooking/telegram");
    let client = reqwest::Client::new();
    // This request is deliberately sent only after the supervisor restart;
    // its model request must carry Alice's persisted SQLite summary.
    let follow_up = deliver_request(pool, gateway, &client, &url, 44, 1001, "salad").await;
    assert_ne!(follow_up.event_id, alice.event_id);
    assert_normalized_mailbox_body(pool, follow_up.event_id, 44, "alice", "salad").await;
    // The response-loss helper already records one durable replay of update
    // 42; the explicit duplicate in exercise_initial records the second.
    assert_eq!(mailbox_counts(pool, gateway).await, (3, 2, 3));
    let cooking_run_count: i64 = sqlx::query_scalar(
        "SELECT count(DISTINCT attempt.run_id)
           FROM mailbox_delivery_attempts attempt
           JOIN mailbox_events event ON event.id = attempt.event_id
          WHERE event.mailbox_id = $1",
    )
    .bind(gateway.mailbox_id.as_uuid())
    .fetch_one(pool)
    .await
    .expect("cooking run count");
    if cooking_run_count != 3 {
        // Keep the exact response-loss event's attempt lineage available even
        // when the assertion aborts the test before the next periodic sample.
        write_cooking_lineage_snapshot(pool, gateway.mailbox_id.as_uuid(), Some(alice.event_id))
            .await;
    }
    assert_eq!(
        cooking_run_count, 3,
        "duplicate ingress must not start a run"
    );
    let leases: Vec<(time::OffsetDateTime, time::OffsetDateTime)> = sqlx::query_as(
        "SELECT lease.acquired_at, lease.released_at
           FROM agent_instance_volume_leases lease
          WHERE lease.run_id = ANY($1)
          ORDER BY lease.acquired_at",
    )
    .bind(vec![
        alice.run_id.as_uuid(),
        bob.run_id.as_uuid(),
        follow_up.run_id.as_uuid(),
    ])
    .fetch_all(pool)
    .await
    .expect("cooking lease windows");
    assert_eq!(leases.len(), 3, "each logical recipe gets one state lease");
    for pair in leases.windows(2) {
        assert!(
            pair[0].1 <= pair[1].0,
            "state leases for cooking runs must not overlap"
        );
    }

    let run_id = alice.run_id;
    running
        .wait_for_run_event(
            run_id,
            hephaestus_app::RunEventKind::ResultCompleted,
            Duration::from_secs(30),
        )
        .await
        .expect("controlled cooking result");
    let (result_ref, result_commit): (String, String) = sqlx::query_as(
        "SELECT result_ref,result_commit FROM run_results WHERE run_id=$1 AND state='completed'",
    )
    .bind(run_id.as_uuid())
    .fetch_one(pool)
    .await
    .expect("controlled result provenance");
    let bare = root.join("repositories").join(format!("{repository}.git"));
    assert_eq!(
        super::git_output_bare(&bare, &["rev-parse", &format!("{result_commit}^")]).await,
        input_commit
    );
    let recipe = super::git_output_bare(
        &bare,
        &[
            "show",
            &format!("{result_commit}:content/recipes/recipe-42.md"),
        ],
    )
    .await;
    assert!(recipe.contains("Family pasta"));
    for (run, recipe_name, expected_title) in [
        (bob.run_id, "recipe-43.md", "Family soup"),
        (follow_up.run_id, "recipe-44.md", "Family salad"),
    ] {
        running
            .wait_for_run_event(
                run,
                hephaestus_app::RunEventKind::ResultCompleted,
                Duration::from_secs(30),
            )
            .await
            .expect("controlled cooking result");
        let commit: String = sqlx::query_scalar(
            "SELECT result_commit FROM run_results
              WHERE run_id = $1 AND state = 'completed'",
        )
        .bind(run.as_uuid())
        .fetch_one(pool)
        .await
        .expect("controlled result commit");
        assert_eq!(
            super::git_output_bare(&bare, &["rev-parse", &format!("{commit}^")]).await,
            input_commit,
            "competing cooking result retains its frozen Git target"
        );
        let page = super::git_output_bare(
            &bare,
            &["show", &format!("{commit}:content/recipes/{recipe_name}")],
        )
        .await;
        assert!(page.contains(expected_title));
    }
    let bob_view = super::cooking_inspection::inspect(
        pool,
        running,
        bob.run_id.as_uuid(),
        bob.event_id,
        false,
    )
    .await;
    super::cooking_inspection::inspect(
        pool,
        running,
        follow_up.run_id.as_uuid(),
        follow_up.event_id,
        false,
    )
    .await;
    assert_eq!(
        super::git_output_bare(&bare, &["rev-parse", "refs/heads/main"]).await,
        input_commit,
        "canonical Git remains controlled"
    );
    let evidence:(uuid::Uuid,uuid::Uuid,i64)=sqlx::query_as("SELECT run.instance_revision_id,lease.id,delivery.dispatch_sequence FROM runs run JOIN agent_instance_volume_leases lease ON lease.run_id=run.id JOIN mailbox_delivery_attempts attempt ON attempt.run_id=run.id JOIN mailbox_deliveries delivery ON delivery.event_id=attempt.event_id WHERE run.id=$1").bind(run_id.as_uuid()).fetch_one(pool).await.expect("revision/state lease/dispatch provenance");
    assert_ne!(evidence.0, instance.revision, "run uses bound revision");
    assert!(evidence.2 > 0);
    super::cooking_inspection::inspect(pool, running, run_id.as_uuid(), alice.event_id, true).await;
    assert_eq!(
        super::git_output_bare(&bare, &["rev-parse", "refs/heads/main"]).await,
        result_commit,
        "authorized host approval publishes the exact recipe result"
    );
    // Bob's proposal was created against the frozen input commit. Once Alice
    // advances canonical main, approving Bob must settle as a Git conflict and
    // retain the competing proposal history without moving the canonical ref.
    super::cooking_inspection::approve_for_test(pool, running, &bob_view).await;
    let bob_proposal: uuid::Uuid =
        sqlx::query_scalar("SELECT id FROM review_proposals WHERE run_id = $1")
            .bind(bob.run_id.as_uuid())
            .fetch_one(pool)
            .await
            .expect("Bob competing proposal");
    let bob_proposal_state: String =
        sqlx::query_scalar("SELECT state FROM review_proposals WHERE id = $1")
            .bind(bob_proposal)
            .fetch_one(pool)
            .await
            .expect("Bob proposal disposition");
    assert_eq!(bob_proposal_state, "conflicted");
    let conflict_events: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM run_events
          WHERE run_id = $1 AND event_type = 'review.conflicted'",
    )
    .bind(bob.run_id.as_uuid())
    .fetch_one(pool)
    .await
    .expect("Bob conflict history");
    assert_eq!(conflict_events, 1, "stale approval remains durable history");
    assert_eq!(
        super::git_output_bare(&bare, &["rev-parse", "refs/heads/main"]).await,
        result_commit,
        "stale competing approval cannot move canonical Git"
    );
    let resolved_head = super::cooking_conflicts::resolve(
        pool,
        running,
        root,
        repository,
        bob.run_id.as_uuid(),
        &result_commit,
    )
    .await;
    assert_ne!(resolved_head, result_commit);

    // These two requests exercise retryable provider faults after the daemon
    // restart. Each has one durable logical recipe and two physical attempts;
    // the relay fault commits before dropping its response, so its retry must
    // resolve the existing deterministic ledger row.
    let model_fault = deliver_request(
        pool,
        gateway,
        &client,
        &url,
        MODEL_FAULT_UPDATE,
        1001,
        "pancakes",
    )
    .await;
    assert_retried_delivery(pool, gateway.mailbox_id.as_uuid(), model_fault).await;
    super::cooking_inspection::inspect_with_https_uses(
        pool,
        running,
        model_fault.run_id.as_uuid(),
        model_fault.event_id,
        false,
        4,
    )
    .await;
    let relay_fault = deliver_request(
        pool,
        gateway,
        &client,
        &url,
        RELAY_FAULT_UPDATE,
        1002,
        "waffles",
    )
    .await;
    assert_retried_delivery(pool, gateway.mailbox_id.as_uuid(), relay_fault).await;
    super::cooking_inspection::inspect_with_https_uses(
        pool,
        running,
        relay_fault.run_id.as_uuid(),
        relay_fault.event_id,
        false,
        2,
    )
    .await;
    // Update 42 has two deliberate duplicate publications: the response-loss
    // helper's direct replay and the explicit replay in exercise_initial.
    // The two fault requests add accepted publications without duplicates.
    assert_eq!(mailbox_counts(pool, gateway).await, (5, 2, 5));
    if let Ok(hugo) = env::var("HEPHAESTUS_COOKING_HUGO") {
        let checkout = root.join("cooking-site-build");
        super::git(
            root,
            &[
                "clone",
                bare.to_str().expect("bare path"),
                checkout.to_str().expect("build path"),
            ],
        )
        .await;
        let output = tokio::process::Command::new(hugo)
            .arg("--source")
            .arg(&checkout)
            .output()
            .await
            .expect("pinned Hugo build");
        assert!(
            output.status.success(),
            "generated recipe passes Hugo build"
        );
        assert!(
            checkout
                .join("public/recipes/recipe-42/index.html")
                .is_file()
        );
    }
    eprintln!(
        "cooking journey: mailbox={}, runs=[{},{},{}], revision={}, lease={}, result_ref={}, result_commit={}",
        gateway.mailbox_id,
        alice.run_id,
        bob.run_id,
        follow_up.run_id,
        evidence.0,
        evidence.1,
        result_ref,
        result_commit
    );
    write_cooking_lineage_snapshot(pool, gateway.mailbox_id.as_uuid(), None).await;
    resolved_head
}

type CookingLineageRow = (
    uuid::Uuid,
    uuid::Uuid,
    i32,
    uuid::Uuid,
    String,
    time::OffsetDateTime,
    Option<time::OffsetDateTime>,
    String,
    Option<String>,
    Option<i32>,
    Option<i32>,
    String,
    Option<time::OffsetDateTime>,
    Option<time::OffsetDateTime>,
    time::OffsetDateTime,
    time::OffsetDateTime,
);

fn cooking_lineage_row_json(
    row: CookingLineageRow,
    mailbox_id: uuid::Uuid,
    sampled_at: &str,
) -> String {
    let (
        event_id,
        attempt_id,
        attempt_number,
        attempt_run_id,
        attempt_state,
        attempt_created_at,
        attempt_completed_at,
        run_state,
        run_outcome,
        run_exit_code,
        run_exit_signal,
        disposition,
        next_eligible_at,
        terminal_at,
        run_created_at,
        run_updated_at,
    ) = row;
    serde_json::json!({
        "sampled_at": sampled_at,
        "mailbox_id": mailbox_id,
        "event_id": event_id,
        "attempt_id": attempt_id,
        "attempt_number": attempt_number,
        "attempt_run_id": attempt_run_id,
        "attempt_state": attempt_state,
        "attempt_created_at": attempt_created_at.to_string(),
        "attempt_completed_at": attempt_completed_at.map(|value| value.to_string()),
        "run_state": run_state,
        "run_outcome": run_outcome,
        "exit_code": run_exit_code,
        "exit_signal": run_exit_signal,
        "disposition": disposition,
        "next_eligible_at": next_eligible_at.map(|value| value.to_string()),
        "terminal_at": terminal_at.map(|value| value.to_string()),
        "run_created_at": run_created_at.to_string(),
        "run_updated_at": run_updated_at.to_string(),
    })
    .to_string()
}

/// Periodically exports only current Cooking delivery/run lifecycle metadata.
/// The file is atomically replaced so a killed test leaves the last complete
/// sample for the external diagnostics collector. Payloads, failures, and
/// credentials are intentionally outside this query.
async fn write_cooking_lineage_snapshot(
    pool: &sqlx::PgPool,
    mailbox_id: uuid::Uuid,
    event_id: Option<uuid::Uuid>,
) {
    let Some(directory) = env::var_os("HEPHAESTUS_COOKING_DIAGNOSTICS_DIR") else {
        return;
    };
    let directory = PathBuf::from(directory);
    if !directory.is_absolute() || directory.is_symlink() || !directory.is_dir() {
        return;
    }
    let rows = sqlx::query_as::<_, CookingLineageRow>(
        "SELECT event.id, attempt.id, attempt.attempt_number, attempt.run_id,
                attempt.state, attempt.created_at, attempt.completed_at,
                run.state, run.outcome, run.exit_code, run.exit_signal,
                delivery.disposition,
                delivery.next_eligible_at, delivery.terminal_at,
                run.created_at, run.updated_at
           FROM mailbox_events event
           JOIN mailbox_delivery_attempts attempt ON attempt.event_id = event.id
           JOIN mailbox_deliveries delivery ON delivery.event_id = event.id
           JOIN runs run ON run.id = attempt.run_id
          WHERE event.mailbox_id = $1
            AND ($2::uuid IS NULL OR event.id = $2)
          ORDER BY event.id, attempt.attempt_number, attempt.id
          LIMIT 5000",
    )
    .bind(mailbox_id)
    .bind(event_id)
    .fetch_all(pool);
    let rows = match tokio::time::timeout(Duration::from_secs(2), rows).await {
        Ok(Ok(rows)) => rows,
        Ok(Err(_)) => {
            write_cooking_lineage_status(&directory, mailbox_id, event_id, "query_failed", 0);
            return;
        }
        Err(_) => {
            write_cooking_lineage_status(&directory, mailbox_id, event_id, "query_timeout", 0);
            return;
        }
    };
    let sampled_at = time::OffsetDateTime::now_utc().to_string();
    let lines = rows
        .into_iter()
        .map(|row| cooking_lineage_row_json(row, mailbox_id, &sampled_at))
        .collect::<Vec<_>>();
    let contents = if lines.is_empty() {
        String::new()
    } else {
        format!("{}\n", lines.join("\n"))
    };
    if !write_cooking_diagnostic_file(&directory, "cooking-lineage.jsonl", contents.as_bytes()) {
        write_cooking_lineage_status(
            &directory,
            mailbox_id,
            event_id,
            "write_failed",
            lines.len(),
        );
        return;
    }
    write_cooking_lineage_status(&directory, mailbox_id, event_id, "ok", lines.len());
}

fn write_cooking_diagnostic_file(directory: &Path, name: &str, contents: &[u8]) -> bool {
    let destination = directory.join(name);
    let temporary = directory.join(format!(".{name}.tmp.{}", uuid::Uuid::new_v4()));
    fs::write(&temporary, contents)
        .and_then(|()| fs::rename(temporary, destination))
        .is_ok()
}

fn write_cooking_lineage_status(
    directory: &Path,
    mailbox_id: uuid::Uuid,
    event_id: Option<uuid::Uuid>,
    status: &str,
    rows: usize,
) {
    let value = serde_json::json!({
        "schema": 1,
        "status": status,
        "sampled_at": time::OffsetDateTime::now_utc().to_string(),
        "mailbox_id": mailbox_id,
        "event_id": event_id,
        "rows": rows,
    });
    let _ = write_cooking_diagnostic_file(
        directory,
        "cooking-lineage-status.json",
        format!("{value}\n").as_bytes(),
    );
}

#[cfg(test)]
mod rule_copy_tests {
    use super::rewrite_rule_copy_request;

    #[test]
    fn copied_rule_rewrites_candidate_placeholder_after_rotation() {
        let source = "11111111-1111-4111-8111-111111111111"
            .parse()
            .expect("source rule UUID");
        let candidate = "22222222-2222-4222-8222-222222222222"
            .parse()
            .expect("candidate rule UUID");
        let body = serde_json::json!({
            "rule_id": candidate,
            "method": "post",
            "path_and_query": "/v1/recipes",
            "headers": [
                {"name": "authorization", "value": format!("Bearer heph-placeholder:v1:{candidate}")},
                {"name": "content-type", "value": "application/json"}
            ],
            "body": []
        });

        let rewritten = rewrite_rule_copy_request(
            &serde_json::to_vec(&body).expect("request JSON"),
            source,
            candidate,
        )
        .expect("candidate request maps to source");
        let rewritten: serde_json::Value =
            serde_json::from_slice(&rewritten).expect("rewritten JSON");
        assert_eq!(rewritten["rule_id"], source.to_string());
        assert_eq!(
            rewritten["headers"][0]["value"],
            format!("Bearer heph-placeholder:v1:{source}")
        );
    }

    #[test]
    fn copied_rule_rejects_a_placeholder_for_another_rule() {
        let source = "11111111-1111-4111-8111-111111111111"
            .parse()
            .expect("source rule UUID");
        let candidate = "22222222-2222-4222-8222-222222222222"
            .parse()
            .expect("candidate rule UUID");
        let body = serde_json::json!({
            "rule_id": candidate,
            "headers": [{
                "name": "authorization",
                "value": "Bearer heph-placeholder:v1:33333333-3333-4333-8333-333333333333"
            }]
        });
        assert!(matches!(
            rewrite_rule_copy_request(
                &serde_json::to_vec(&body).expect("request JSON"),
                source,
                candidate,
            ),
            Err(secret_application::BrokerAdapterError::Rejected)
        ));
    }
}

#[cfg(test)]
mod crash_upstream_tests {
    use super::{BASE_COOKING_REQUESTS, EXPECTED_COOKING_REQUESTS};
    use super::{
        MODEL_ROTATED_SENTINEL, MODEL_SENTINEL, credential_class, expected_relay_ledger,
        expected_upstream_requests,
    };

    #[test]
    fn crash_mode_completion_budget_is_six_for_both_adapters() {
        assert_eq!(expected_upstream_requests(true, true, true), 6);
        assert_eq!(expected_upstream_requests(true, true, false), 6);
        assert_eq!(expected_upstream_requests(true, false, true), 6);
        assert_eq!(expected_upstream_requests(true, false, false), 6);
        assert_eq!(
            expected_upstream_requests(false, true, true),
            EXPECTED_COOKING_REQUESTS
        );
        assert_eq!(
            expected_upstream_requests(false, true, false),
            EXPECTED_COOKING_REQUESTS - 1
        );
        assert_eq!(
            expected_upstream_requests(false, false, true),
            BASE_COOKING_REQUESTS
        );
        assert_eq!(
            expected_upstream_requests(false, false, false),
            BASE_COOKING_REQUESTS
        );
    }

    #[test]
    fn credential_class_diagnostic_distinguishes_rotation_without_exposing_values() {
        assert_eq!(credential_class(MODEL_SENTINEL), "model_initial");
        assert_eq!(credential_class(MODEL_ROTATED_SENTINEL), "model_rotated");
        assert_ne!(
            credential_class(MODEL_SENTINEL),
            credential_class(MODEL_ROTATED_SENTINEL)
        );
    }

    #[test]
    fn crash_mode_relay_ledger_tracks_all_five_logical_recipes() {
        assert_eq!(
            expected_relay_ledger(true, true),
            "['recipe-51','recipe-52','recipe-53','recipe-54','recipe-55']"
        );
        assert_eq!(
            expected_relay_ledger(false, false),
            "['recipe-42','recipe-43','recipe-44','recipe-45','recipe-46']"
        );
        assert_eq!(
            expected_relay_ledger(true, false),
            "['recipe-42','recipe-43','recipe-44','recipe-45','recipe-46','recipe-47','recipe-48','recipe-49']"
        );
    }
}
