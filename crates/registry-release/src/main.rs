//! Trusted operator composition for forge-owned registry releases.

use clap::{Parser, Subcommand};
use registry_domain::{
    ImmutableManifestReference, NamespaceClaim, OciDescriptor, OciMediaType, PlatformBuilderKey,
    PolicyVersion, PublicationIntent, PublicationIntentId, PublicationState, RegistryAuthority,
    RegistryOwner, Sha256Digest, SupplyChainPolicy, VerifiedPublication,
};
use registry_postgres::{PgRegistryStore, connect as connect_registry};
use registry_publisher::{
    ControlledOciPublisher, PublicationEvidenceFiles, PublicationMaterial, PublisherConfiguration,
    SystemCommandRunner,
};
use registry_token::{
    AuthorizationDecision, KeyId, RegistryService, RegistryTokenIssuer, RepositoryActions,
    RepositoryName, ScopeRequest, SigningKey, TokenIssuer, TokenLifetime, TokenSubject,
    UnixTimestamp,
};
use serde::{Deserialize, Serialize};
use std::{env, error::Error, fs, path::PathBuf, sync::Arc};
use time::OffsetDateTime;
use zeroize::Zeroizing;

#[derive(Parser)]
#[command(name = "hephaestus-registry-release", version)]
struct Arguments {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Publish and approve one reviewed platform builder.
    PublishPlatformBuilder {
        /// Stable platform builder key.
        #[arg(long)]
        key: String,
        /// Administrator-owned OCI image layout.
        #[arg(long)]
        layout: PathBuf,
        /// SPDX JSON evidence file under the configured layout root.
        #[arg(long)]
        sbom: PathBuf,
        /// In-toto JSON provenance file under the configured layout root.
        #[arg(long)]
        provenance: PathBuf,
        /// Successful vulnerability scan JSON under the configured layout root.
        #[arg(long)]
        scan: PathBuf,
        /// Optional verified signature/approval artifact.
        #[arg(long)]
        signature: Option<PathBuf>,
        /// Reviewed policy revision.
        #[arg(long, default_value = "builder/v1")]
        policy_version: String,
    },
}

#[derive(Deserialize)]
struct LayoutIndex {
    manifests: Vec<LayoutDescriptor>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LayoutDescriptor {
    media_type: String,
    digest: String,
    size: u64,
}

#[derive(Serialize)]
struct ReleaseOutput {
    publication_id: String,
    state: &'static str,
    reference: String,
    manifest_digest: String,
    evidence: Vec<EvidenceOutput>,
}

#[derive(Serialize)]
struct EvidenceOutput {
    kind: &'static str,
    reference: String,
}

struct Runtime {
    authority: RegistryAuthority,
    issuer: Arc<RegistryTokenIssuer>,
    store: PgRegistryStore,
    publisher: ControlledOciPublisher<SystemCommandRunner>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let arguments = Arguments::parse();
    let runtime = runtime().await?;
    let output = match arguments.command {
        Command::PublishPlatformBuilder {
            key,
            layout,
            sbom,
            provenance,
            scan,
            signature,
            policy_version,
        } => {
            runtime
                .publish_platform(PublishPlatform {
                    key,
                    material: PublicationMaterial {
                        layout,
                        evidence: PublicationEvidenceFiles {
                            sbom,
                            provenance,
                            scan,
                            signature,
                        },
                    },
                    policy_version,
                })
                .await?
        }
    };
    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}

struct PublishPlatform {
    key: String,
    material: PublicationMaterial,
    policy_version: String,
}

impl Runtime {
    async fn publish_platform(
        &self,
        request: PublishPlatform,
    ) -> Result<ReleaseOutput, Box<dyn Error>> {
        let key = PlatformBuilderKey::parse(request.key)?;
        let expected = layout_descriptor(&request.material.layout)?;
        let owner = RegistryOwner::PlatformBuilder { builder_key: key };
        let claim = NamespaceClaim::new(owner);
        let reference = ImmutableManifestReference::new(
            self.authority.clone(),
            claim.namespace().clone(),
            expected.digest().clone(),
        );
        let proposed = PublicationIntent::new(
            PublicationIntentId::new(),
            claim,
            reference,
            expected,
            PolicyVersion::parse(request.policy_version)?,
            SupplyChainPolicy::without_signature(),
        )?;
        let mut intent = self.store.create_intent(&proposed).await?;
        match intent.state() {
            PublicationState::Approved => return release_output(&intent),
            PublicationState::Verified => {
                intent = self.store.approve(intent.id()).await?;
                return release_output(&intent);
            }
            PublicationState::Publishing => {
                intent = self.store.retry(intent.id()).await?;
            }
            PublicationState::Pending => {}
            PublicationState::Missing | PublicationState::Retired => {
                return Err("existing publication is not retryable".into());
            }
        }
        intent = self.store.begin_publishing(intent.id()).await?;
        let token = issue_publish_token(&self.issuer, intent.claim().namespace())?;
        let verification = match self
            .publisher
            .publish(&intent, &request.material, token.token())
        {
            Ok(value) => value,
            Err(error) => {
                let _ignored = self.store.retry(intent.id()).await;
                return Err(Box::new(error));
            }
        };
        intent = self
            .store
            .record_verified(intent.id(), verification)
            .await?;
        intent = self.store.approve(intent.id()).await?;
        release_output(&intent)
    }
}

fn issue_publish_token(
    issuer: &RegistryTokenIssuer,
    namespace: &registry_domain::RegistryNamespace,
) -> Result<registry_token::IssuedToken, Box<dyn Error>> {
    let repository = namespace.as_str().parse::<RepositoryName>()?;
    let request = ScopeRequest::parse(
        issuer.service().as_str(),
        &format!("repository:{repository}:pull,push"),
    )?;
    let mut authorization = AuthorizationDecision::deny_all();
    authorization.grant(repository, RepositoryActions::pull_push());
    let now = u64::try_from(OffsetDateTime::now_utc().unix_timestamp())?;
    Ok(issuer.issue(
        "workload:platform-release".parse::<TokenSubject>()?,
        &request,
        &authorization,
        UnixTimestamp::new(now),
    )?)
}

fn release_output(intent: &PublicationIntent) -> Result<ReleaseOutput, Box<dyn Error>> {
    let verification = intent
        .verification()
        .ok_or("approved publication has no verification")?;
    Ok(output(intent, verification))
}

fn output(intent: &PublicationIntent, verification: &VerifiedPublication) -> ReleaseOutput {
    let namespace = intent.reference().namespace();
    let authority = intent.reference().authority();
    let evidence = verification
        .evidence()
        .referrers()
        .iter()
        .map(|referrer| EvidenceOutput {
            kind: match referrer.kind() {
                registry_domain::SupplyChainReferrerKind::Sbom => "sbom",
                registry_domain::SupplyChainReferrerKind::Provenance => "provenance",
                registry_domain::SupplyChainReferrerKind::Scan => "scan",
                registry_domain::SupplyChainReferrerKind::Signature => "signature",
            },
            reference: format!("{authority}/{namespace}@{}", referrer.descriptor().digest()),
        })
        .collect();
    ReleaseOutput {
        publication_id: intent.id().to_string(),
        state: "approved",
        reference: intent.reference().to_string(),
        manifest_digest: intent.reference().digest().to_string(),
        evidence,
    }
}

fn layout_descriptor(layout: &std::path::Path) -> Result<OciDescriptor, Box<dyn Error>> {
    let index: LayoutIndex = serde_json::from_slice(&fs::read(layout.join("index.json"))?)?;
    let [descriptor] = index.manifests.as_slice() else {
        return Err("OCI layout must contain exactly one top-level descriptor".into());
    };
    Ok(OciDescriptor::new(
        Sha256Digest::parse(descriptor.digest.clone())?,
        descriptor.size,
        OciMediaType::parse(descriptor.media_type.clone())?,
    )?)
}

async fn runtime() -> Result<Runtime, Box<dyn Error>> {
    let authority = RegistryAuthority::parse(required("HEPHAESTUS_FORGE_REGISTRY_AUTHORITY")?)?;
    let service = required("HEPHAESTUS_REGISTRY_SERVICE")?.parse::<RegistryService>()?;
    if service.as_str() != authority.as_str() {
        return Err("registry service and authority must match".into());
    }
    let private_key = Zeroizing::new(fs::read(required(
        "HEPHAESTUS_REGISTRY_TOKEN_PRIVATE_KEY",
    )?)?);
    let issuer = Arc::new(RegistryTokenIssuer::new(
        required("HEPHAESTUS_REGISTRY_TOKEN_ISSUER")?.parse::<TokenIssuer>()?,
        service,
        SigningKey::rs256_pem(
            required("HEPHAESTUS_REGISTRY_TOKEN_KEY_ID")?.parse::<KeyId>()?,
            &private_key,
        )?,
        TokenLifetime::new(
            env::var("HEPHAESTUS_REGISTRY_TOKEN_LIFETIME_SECONDS")
                .unwrap_or_else(|_| String::from("300"))
                .parse()?,
        )?,
    ));
    let database_url = required("HEPHAESTUS_DATABASE_URL")?;
    let store = connect_registry(&database_url).await?;
    let config = PublisherConfiguration::new(
        authority.clone(),
        &absolute_path("HEPHAESTUS_REGISTRY_LAYOUT_ROOT")?,
        &absolute_path("HEPHAESTUS_REGISTRY_CREDENTIAL_ROOT")?,
        &absolute_path("HEPHAESTUS_SKOPEO")?,
        &absolute_path("HEPHAESTUS_ORAS")?,
    )?;
    Ok(Runtime {
        authority,
        issuer,
        store,
        publisher: ControlledOciPublisher::new(config, SystemCommandRunner),
    })
}

fn required(name: &str) -> Result<String, Box<dyn Error>> {
    env::var(name).map_err(|_| format!("{name} is required").into())
}

fn absolute_path(name: &str) -> Result<PathBuf, Box<dyn Error>> {
    let path = PathBuf::from(required(name)?);
    if !path.is_absolute() {
        return Err(format!("{name} must be absolute").into());
    }
    Ok(path)
}
