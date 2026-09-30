use control_plane_postgres::{build::BuildApplication, connect_app};
use identity_domain::{AuthenticatedIdentity, UserId};
use sqlx::{PgPool, postgres::PgPoolOptions};
use uuid::Uuid;

use crate::support::{CONFIG, identity, seed_identity, seed_images};

#[path = "scenario/invalid.rs"]
mod invalid;
#[path = "scenario/legacy.rs"]
mod legacy;
#[path = "scenario/valid.rs"]
mod valid;

pub struct Context {
    pub bootstrap: PgPool,
    pub application: BuildApplication,
    pub owner: UserId,
    pub repository: Uuid,
    pub configuration: serde_json::Value,
    pub source_hash: agent_config::ConfigHash,
    pub normalized_config_hash: agent_config::ConfigHash,
    pub base_hash: [u8; 32],
    pub ui_hash: [u8; 32],
    pub identity: AuthenticatedIdentity,
    pub duplicate_identity: AuthenticatedIdentity,
    pub parallel_identity: AuthenticatedIdentity,
}

impl Context {
    async fn initialize(database_url: &str) -> Self {
        let bootstrap = PgPoolOptions::new()
            .max_connections(4)
            .connect(database_url)
            .await
            .expect("connect PostgreSQL bootstrap pool");
        sqlx::migrate!("../../../../migrations")
            .run(&bootstrap)
            .await
            .expect("apply migrations");

        let owner = UserId::new();
        let repository = Uuid::new_v4();
        let project = Uuid::new_v4();
        let organization = Uuid::new_v4();
        seed_identity(&bootstrap, owner, organization, project, repository).await;
        seed_images(&bootstrap).await;

        let parsed = agent_config::parse(CONFIG.as_bytes());
        let config = parsed.config.expect("valid test configuration");
        let normalized_config_hash = parsed
            .normalized_hash
            .expect("normalized configuration hash");
        let configuration = serde_json::to_value(&config).expect("serialize configuration");
        let build = config.build.as_ref().expect("build declaration");
        let base_hash = agent_config::build_identity::base_build_definition_hash(build)
            .expect("base build hash");
        let ui_hash = [4_u8; 32];
        let parallel_identity = identity(owner);
        let duplicate_identity = identity(owner);
        let identity = identity(owner);
        let app_pool = connect_app(database_url, 4).await.expect("app pool");

        Self {
            bootstrap,
            application: BuildApplication::new(app_pool),
            owner,
            repository,
            configuration,
            source_hash: parsed.hash,
            normalized_config_hash,
            base_hash,
            ui_hash,
            identity,
            duplicate_identity,
            parallel_identity,
        }
    }
}

#[tokio::test]
async fn manual_build_ui_basic_matrix_uses_application_role() {
    let Ok(database_url) = std::env::var("HEPHAESTUS_POSTGRES_TEST_URL") else {
        eprintln!("skipping manual UI build matrix: test URL is unset");
        return;
    };
    let context = Context::initialize(&database_url).await;
    valid::run(&context).await;
    invalid::run(&context).await;
    legacy::run(&context).await;
}
