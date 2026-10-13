//! Minimal real fixture identities and image metadata; publication creates releases.

use std::{
    fs::{self, File},
    os::unix::fs::MetadataExt,
    path::Path,
    sync::Arc,
};

use forge_domain::{OrganizationId, ProjectId};
use forge_postgres::PgForgeRepository;
use forge_storage::GitStorage;
use identity_domain::{
    AuthenticatedIdentity, BrowserSessionSid, RequestId, UserId,
    browser_session_identity_binding_digest, browser_session_sid_digest,
};
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use time::OffsetDateTime;

use super::super::build_helpers::BuildError;
use super::PublicationInputs;

pub(in super::super) const GIT_AUDIENCE: &str = "hephaestus-git";
const SUBJECT: &str = "qualified-local-publication";
const PYTHON_DIGEST: &str =
    "@sha256:24b78e523e8cf1732243dc8945d5c75145b5e21d23081a90ecc6b591d4bb820e";

/// Retains only actual base identities and the trusted repository factory.
pub(in super::super) struct PublicationSeed {
    pub(in super::super) actor: AuthenticatedIdentity,
    pub(in super::super) project: ProjectId,
    pub(in super::super) repositories: PgForgeRepository,
    session: BrowserSessionSid,
}

/// Reads the immutable reviewed input once, before allocating backend resources.
pub(in super::super) fn image_input(inputs: &PublicationInputs) -> Result<Value, BuildError> {
    let bytes = fs::read(&inputs.image_input)?;
    assert_eq!(
        Sha256::digest(&bytes).to_vec(),
        inputs.image_input_sha256.to_vec()
    );
    let input: Value = serde_json::from_slice(&bytes)?;
    let entry = &input["catalog_entry"];
    assert_eq!(entry["key"], "python-ubuntu");
    assert!(
        entry["image_reference"]
            .as_str()
            .unwrap()
            .ends_with(PYTHON_DIGEST)
    );
    assert_eq!(entry["architectures"], json!(["x86_64"]));
    assert!(
        !entry["platform_policy_version"]
            .as_str()
            .unwrap()
            .is_empty()
    );
    assert!(
        !entry["id"]
            .as_str()
            .unwrap()
            .parse::<uuid::Uuid>()?
            .is_nil()
    );
    Ok(input)
}

/// Checks the retained configured image directory identity, not its guest bytes.
pub(in super::super) fn validate_rootfs(path: &Path, pin: &File) -> Result<(), BuildError> {
    assert_eq!(fs::canonicalize(path)?, path);
    let actual = fs::symlink_metadata(path)?;
    let retained = pin.metadata()?;
    assert!(actual.is_dir() && !actual.file_type().is_symlink());
    assert_eq!(
        (actual.dev(), actual.ino()),
        (retained.dev(), retained.ino())
    );
    Ok(())
}

impl PublicationSeed {
    /// Seeds only the user, organization, rights, browser binding, project and image.
    pub(in super::super) async fn create(
        pool: &PgPool,
        inputs: &PublicationInputs,
        root: &Path,
        image: &Value,
    ) -> Result<Self, BuildError> {
        let user = UserId::new();
        let organization = OrganizationId::new();
        sqlx::query("INSERT INTO users(id,display_name) VALUES($1,'Qualified Local publisher')")
            .bind(user.as_uuid())
            .execute(pool)
            .await?;
        sqlx::query("INSERT INTO organizations(id,name) VALUES($1,$2)")
            .bind(organization.as_uuid())
            .bind(format!("qualified-local-{organization}"))
            .execute(pool)
            .await?;
        sqlx::query(
            "INSERT INTO organization_members(organization_id,user_id,role) VALUES($1,$2,'owner')",
        )
        .bind(organization.as_uuid())
        .bind(user.as_uuid())
        .execute(pool)
        .await?;
        sqlx::query("INSERT INTO external_identities(user_id,issuer,subject,provider_metadata) VALUES($1,$2,$3,'{}')")
            .bind(user.as_uuid()).bind(&inputs.oidc_issuer).bind(SUBJECT).execute(pool).await?;
        let actor = AuthenticatedIdentity::new(
            user,
            &inputs.oidc_issuer,
            SUBJECT,
            Value::Null,
            RequestId::new(),
        );
        let session = BrowserSessionSid::new();
        sqlx::query("INSERT INTO human_browser_sessions(id,sid_digest,creation_idempotency_id,creation_request_id,identity_binding_digest,user_id,issued_at,expires_at) VALUES($1,$2,$3,$4,$5,$6,now(),now()+interval '12 hours')")
            .bind(uuid::Uuid::new_v4()).bind(browser_session_sid_digest(session).as_bytes().to_vec())
            .bind(uuid::Uuid::new_v4()).bind(uuid::Uuid::new_v4())
            .bind(browser_session_identity_binding_digest(&actor).as_bytes().to_vec())
            .bind(user.as_uuid()).execute(pool).await?;
        let storage = Arc::new(GitStorage::initialize(&root.join("repositories")).await?);
        let repositories = PgForgeRepository::new(pool.clone(), storage);
        let project = repositories
            .create_project_trusted(organization, "qualified-local")
            .await?;
        sqlx::query("INSERT INTO project_maintainers(project_id,user_id) VALUES($1,$2)")
            .bind(project.id.as_uuid())
            .bind(user.as_uuid())
            .execute(pool)
            .await?;
        let entry = &image["catalog_entry"];
        let architectures = entry["architectures"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_str().unwrap().to_owned())
            .collect::<Vec<_>>();
        sqlx::query("INSERT INTO oci_images(id,key,display_name,image_reference,toolchains,architectures,availability_state,provenance,platform_policy_version) VALUES($1,'python-ubuntu',$2,$3,$4,$5,'available',$6,$7)")
            .bind(entry["id"].as_str().unwrap().parse::<uuid::Uuid>()?)
            .bind(entry["display_name"].as_str().unwrap())
            .bind(entry["image_reference"].as_str().unwrap()).bind(&entry["toolchains"])
            .bind(architectures).bind(&entry["provenance"])
            .bind(entry["platform_policy_version"].as_str().unwrap()).execute(pool).await?;
        Ok(Self {
            actor,
            project: project.id,
            repositories,
            session,
        })
    }

    /// Generates the ordinary OIDC bearer accepted by the real Git transport.
    pub(in super::super) fn git_token(&self, inputs: &PublicationInputs) -> String {
        let now = OffsetDateTime::now_utc().unix_timestamp();
        encode(
            &Header::new(Algorithm::HS256),
            &json!({
                "iss":self.actor.issuer.as_str(),"sub":self.actor.subject.as_str(),"aud":GIT_AUDIENCE,
                "iat":now,"exp":now+300,
            }),
            &EncodingKey::from_secret(&inputs.oidc_signing_secret),
        )
        .unwrap()
    }

    /// Generates an actual browser-bound mediator assertion for this RPC method.
    pub(in super::super) fn rpc_token(&self, inputs: &PublicationInputs, audience: &str) -> String {
        let now = OffsetDateTime::now_utc().unix_timestamp();
        encode(
            &Header::new(Algorithm::HS256),
            &json!({
                "iss":"hephaestus-web-mediator","sub":self.actor.user_id.to_string(),
                "aud":audience,"iat":now,"nbf":now,"exp":now+25,
                "jti":uuid::Uuid::new_v4().to_string(),"sid":self.session.to_protocol_string(),
            }),
            &EncodingKey::from_secret(&inputs.rpc_mediator_signing_key),
        )
        .unwrap()
    }
}
