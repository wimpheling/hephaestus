//! Optional real cooking-app slice on the golden daemon fixture.

use super::{
    BROKERED_E2E_SENTINEL, BrokeredFixture, BrokeredTlsUpstream, GatewayGoldenFixture, ISSUER,
    SeededInstance, secret_command_key,
};
use authz_postgres::PostgresMelangeAuthorizer;
use brokered_egress_domain::{
    BrokeredSecretRule, BrokeredSecretRuleId, ExactHttpsOrigin, HeaderName, HttpInjectionLocation,
};
use forge_domain::{OrganizationId, ProjectId};
use identity_domain::{AuthenticatedIdentity, RequestId, UserId};
use secret_application::{
    BindSecret, CreateSecret, DeclareBrokeredHttpsRule, GrantAndAcceptSecretImport,
};
use secret_broker::BrokeredHttpsAdapterRegistry;
use secret_domain::{
    AgentSecretBindingId, DeliveryMode, ExecutionPhase, SecretAlias, SecretGrantId, SecretId,
    SecretImportId, SecretName, SecretOwner, SecretSlotKey, SecretTarget, SecretUsePolicy,
    SecretValue, SecretVersionId,
};
use secret_postgres::SecretService;
use secret_store::{EncryptedStore, LocalKeyProvider};
use std::{
    env,
    path::{Path, PathBuf},
    sync::{
        Arc,
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

const MODEL_RULE: uuid::Uuid = uuid::uuid!("00000000-0000-0000-0000-000000000003");
const RELAY_RULE: uuid::Uuid = uuid::uuid!("00000000-0000-0000-0000-000000000004");

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
        ISSUER,
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
    for (slot, host) in [
        ("model", "api.model.example"),
        ("telegram_relay", "relay.cooking.example"),
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
                    value: SecretValue::new(BROKERED_E2E_SENTINEL).expect("sentinel"),
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

const INBOUND_SENTINEL: &str = "cooking-inbound-only-fixture-sentinel";

struct CookingAdapters(Vec<Arc<dyn secret_application::BrokerAdapter>>);

#[async_trait::async_trait]
impl secret_application::BrokerAdapter for CookingAdapters {
    async fn invoke(
        &self,
        credential: &SecretValue,
        destination: &str,
        operation: &str,
        body: &[u8],
    ) -> Result<secret_application::BrokerResponse, secret_application::BrokerAdapterError> {
        let index = match destination {
            "api.model.example" => 0,
            "relay.cooking.example" => 1,
            _ => return Err(secret_application::BrokerAdapterError::Rejected),
        };
        self.0[index]
            .invoke(credential, destination, operation, body)
            .await
    }
}

async fn cooking_upstreams(rules: Vec<BrokeredSecretRule>) -> BrokeredTlsUpstream {
    let mut adapters = Vec::new();
    let mut servers = Vec::new();
    for rule in rules {
        let model = rule.id.as_uuid() == MODEL_RULE;
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
        adapters.push(Arc::new(
            BrokeredHttpsAdapterRegistry::test_only_local_trusted(
                rule,
                port,
                "127.0.0.1".parse().expect("loopback"),
                ca.pem().as_bytes(),
            )
            .expect("pinned TLS adapter"),
        ) as Arc<dyn secret_application::BrokerAdapter>);
        servers.push(tokio::spawn(async move {
            let (stream,_) = listener.accept().await.expect("broker TLS request");
            let mut stream = TlsAcceptor::from(Arc::new(tls)).accept(stream).await.expect("verified TLS");
            let request = read_http_request(&mut stream).await;
            let separator = request.windows(4).position(|part| part == b"\r\n\r\n").expect("HTTP headers");
            let headers = std::str::from_utf8(&request[..separator]).expect("HTTP header encoding");
            assert!(headers.contains(&format!("authorization: Bearer {BROKERED_E2E_SENTINEL}")), "host substituted credential");
            assert!(!headers.contains("heph-placeholder:"));
            let body: serde_json::Value = serde_json::from_slice(&request[separator+4..]).expect("application body");
            let response = if model {
                assert!(headers.starts_with("POST /v1/recipes HTTP/1.1"));
                assert_eq!(body["user_id"], "alice");
                serde_json::json!({"title":"Family pasta","summary":"A simple family recipe","ingredients":["pasta","tomatoes"],"steps":["Boil pasta","Add tomatoes"]})
            } else {
                assert!(headers.starts_with("POST /v1/messages HTTP/1.1"));
                invoke_relay(body).await
            };
            let body = serde_json::to_vec(&response).expect("response JSON");
            let header = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",body.len());
            stream.write_all(header.as_bytes()).await.expect("TLS response header");
            stream.write_all(&body).await.expect("TLS response body");
        }));
    }
    let observed = Arc::new(AtomicBool::new(false));
    let done = Arc::clone(&observed);
    let server = tokio::spawn(async move {
        for task in servers {
            task.await.expect("cooking upstream task");
        }
        done.store(true, Ordering::SeqCst);
    });
    BrokeredTlsUpstream {
        adapter: Arc::new(CookingAdapters(adapters)),
        observed,
        server,
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

async fn invoke_relay(body: serde_json::Value) -> serde_json::Value {
    let temporary = tempfile::tempdir().expect("external relay data");
    let script = "import importlib.util,json,sys\nspec=importlib.util.spec_from_file_location('relay',sys.argv[1]); module=importlib.util.module_from_spec(spec); spec.loader.exec_module(module)\nrequest=json.load(sys.stdin); relay=module.Relay(sys.argv[2],request['credential'].encode()); status,body=relay.deliver('Bearer '+request['credential'],json.dumps(request['body']).encode()); assert status==200; print(json.dumps(body))";
    let mut child = tokio::process::Command::new("python3")
        .arg("-c")
        .arg(script)
        .arg(source_root().join("telegram-relay/relay.py"))
        .arg(temporary.path().join("relay.sqlite3"))
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
            &serde_json::to_vec(
                &serde_json::json!({"credential":BROKERED_E2E_SENTINEL,"body":body}),
            )
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

async fn deliver_request(
    pool: &sqlx::PgPool,
    gateway: &GatewayGoldenFixture,
) -> runtime_types::RunId {
    let public = env::var("HEPHAESTUS_CADDY_TEST_PUBLIC_URL").expect("public Caddy URL");
    let url = format!("{public}/gateway/cooking/telegram");
    let client = reqwest::Client::new();
    let body = serde_json::json!({"update_id":42,"message":{"from":{"id":1001},"text":"pasta"}});
    for credential in [None, Some("wrong-inbound-value")] {
        let mut request = client.post(&url).json(&body);
        if let Some(value) = credential {
            request = request.header("x-telegram-bot-api-secret-token", value);
        }
        let response = request.send().await.expect("invalid verification");
        assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);
        assert!(response.bytes().await.expect("empty denial").is_empty());
    }
    let unknown = client
        .post(&url)
        .header("x-telegram-bot-api-secret-token", INBOUND_SENTINEL)
        .json(&serde_json::json!({"update_id":41,"message":{"from":{"id":9999},"text":"pasta"}}))
        .send()
        .await
        .expect("unknown identity");
    assert_eq!(unknown.status(), reqwest::StatusCode::FORBIDDEN);
    let accepted = client
        .post(&url)
        .header("x-telegram-bot-api-secret-token", INBOUND_SENTINEL)
        .json(&body)
        .send()
        .await
        .expect("cooking ingress");
    assert_eq!(accepted.status(), reqwest::StatusCode::OK);
    tokio::time::timeout(Duration::from_secs(90),async {
        loop {
            let row:Option<(uuid::Uuid,String,Option<String>)> = sqlx::query_as("SELECT run.id,run.state,run.outcome FROM mailbox_delivery_attempts attempt JOIN runs run ON run.id=attempt.run_id JOIN mailbox_events event ON event.id=attempt.event_id WHERE event.mailbox_id=$1 ORDER BY run.created_at DESC LIMIT 1")
                .bind(gateway.mailbox_id.as_uuid()).fetch_optional(pool).await.expect("cooking run");
            if let Some((id,state,outcome)) = row {
                if state == "cleaned_up" {
                    assert_eq!(outcome.as_deref(),Some("succeeded"),"cooking run failed: {id}");
                    break runtime_types::RunId::from_uuid(id);
                }
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }).await.expect("cooking mailbox run completes")
}

pub async fn exercise(
    pool: &sqlx::PgPool,
    running: &hephaestus_app::RunningHephaestus,
    instance: &SeededInstance,
    gateway: &GatewayGoldenFixture,
    root: &Path,
    repository: uuid::Uuid,
    input_commit: &str,
) {
    let run_id = deliver_request(pool, gateway).await;
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
    assert_eq!(
        super::git_output_bare(&bare, &["rev-parse", "refs/heads/main"]).await,
        input_commit,
        "canonical Git remains controlled"
    );
    let evidence:(uuid::Uuid,uuid::Uuid,i64)=sqlx::query_as("SELECT run.instance_revision_id,lease.id,delivery.dispatch_sequence FROM runs run JOIN agent_instance_volume_leases lease ON lease.run_id=run.id JOIN mailbox_delivery_attempts attempt ON attempt.run_id=run.id JOIN mailbox_deliveries delivery ON delivery.event_id=attempt.event_id WHERE run.id=$1").bind(run_id.as_uuid()).fetch_one(pool).await.expect("revision/state lease/dispatch provenance");
    assert_ne!(evidence.0, instance.revision, "run uses bound revision");
    assert!(evidence.2 > 0);
    super::cooking_inspection::inspect(pool, running, run_id.as_uuid()).await;
    assert_eq!(
        super::git_output_bare(&bare, &["rev-parse", "refs/heads/main"]).await,
        result_commit,
        "authorized host approval publishes the exact recipe result"
    );
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
        "cooking journey: mailbox={}, run={}, revision={}, lease={}, result_ref={}, result_commit={}",
        gateway.mailbox_id, run_id, evidence.0, evidence.1, result_ref, result_commit
    );
}
