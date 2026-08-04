//! Authorization-aware read-only inspection and narrowly scoped recovery CLI.

use authz_domain::{AuthorizationDecision, ObjectRef, ObjectType, Permission, Subject};
use authz_postgres::{PostgresMelangeAuthorizer, audit_decision, begin_actor_transaction};
use identity_domain::{AuthenticatedIdentity, RequestId, UserId};
use registry_domain::{RegistryInventory, RegistryInventoryDocument, RegistryRetentionReport};
use registry_postgres::PgRegistryStore;
use release_domain::{AgentUpdateId, BuildRequestId, ReleaseCommandKey};
use release_postgres::{RecoverInstanceUpdate, ReleaseService, UpdateRecoveryAction};
use serde_json::{Map, Value, json};
use sqlx::{PgPool, Postgres, Transaction, postgres::PgPoolOptions};
use std::{collections::HashSet, env, error::Error, fs, path::Path, str::FromStr, sync::Arc};
use uuid::Uuid;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let arguments = env::args().skip(1).collect::<Vec<_>>();
    let database_url = env::var("HEPHAESTUS_DATABASE_URL")?;
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&database_url)
        .await?;
    let output = execute(&pool, &arguments).await?;
    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}

async fn execute(pool: &PgPool, arguments: &[String]) -> Result<Value, Box<dyn Error>> {
    let Some(command) = arguments.first().map(String::as_str) else {
        return Err(usage().into());
    };
    match command {
        "inspect-release" => {
            let (identity, object_id) = inspection_arguments(arguments)?;
            inspect(pool, &identity, InspectionTarget::Release, object_id).await
        }
        "inspect-instance" => {
            let (identity, object_id) = inspection_arguments(arguments)?;
            inspect(pool, &identity, InspectionTarget::AgentInstance, object_id).await
        }
        "inspect-secret" => {
            let (identity, object_id) = inspection_arguments(arguments)?;
            inspect(pool, &identity, InspectionTarget::Secret, object_id).await
        }
        "metrics" => {
            let identity = identity_argument(arguments, 1, Some(2))?;
            inspect_metrics(pool, &identity).await
        }
        "recover-update" => recover_update(pool, arguments).await,
        "abandon-build" => abandon_build(pool, arguments).await,
        "provision-builder-catalog" => provision_builder_catalog(pool, arguments).await,
        "registry-retention-report" => registry_retention_report(pool, arguments).await,
        _ => Err(usage().into()),
    }
}

async fn registry_retention_report(
    pool: &PgPool,
    arguments: &[String],
) -> Result<Value, Box<dyn Error>> {
    if arguments.len() != 2 {
        return Err(usage().into());
    }
    let inventory = load_registry_inventory(Path::new(argument(arguments, 1)?))?;
    let store = PgRegistryStore::new(pool.clone());
    let snapshot = store.retention_snapshot().await?;
    Ok(serde_json::to_value(RegistryRetentionReport::evaluate(
        snapshot, inventory,
    ))?)
}

fn load_registry_inventory(path: &Path) -> Result<RegistryInventory, Box<dyn Error>> {
    let contents = fs::read_to_string(path)?;
    let document =
        serde_json::from_str::<RegistryInventoryDocument>(&contents).map_err(|error| {
            format!(
                "{} is not a valid registry inventory document: {error}",
                path.display()
            )
        })?;
    RegistryInventory::try_from(document).map_err(Into::into)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BuilderCatalogRecord {
    id: Uuid,
    key: String,
    display_name: String,
    image_reference: String,
    toolchains: Value,
    architectures: Vec<String>,
    preparation_state: String,
    availability_state: String,
    network_ceiling: String,
    max_vcpus: i64,
    max_memory_mib: i64,
    dependency_policy: String,
    provenance: Value,
    signature_reference: Option<String>,
    sbom_reference: Option<String>,
    platform_policy_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BuilderCatalogManifest {
    schema_version: u64,
    images: Vec<BuilderCatalogRecord>,
}

async fn provision_builder_catalog(
    pool: &PgPool,
    arguments: &[String],
) -> Result<Value, Box<dyn Error>> {
    let manifest_path = argument(arguments, 1)?;
    let dry_run = arguments[2..]
        .iter()
        .try_fold(false, |dry_run, flag| match flag.as_str() {
            "--dry-run" if !dry_run => Ok(true),
            "--dry-run" => Err("--dry-run may only be supplied once"),
            _ => Err("unknown provision-builder-catalog option"),
        })?;
    let contents = fs::read_to_string(manifest_path)?;
    let manifest = parse_builder_catalog_manifest(&contents, Path::new(manifest_path))?;

    if dry_run {
        return Ok(json!({
            "mode": "dry_run",
            "schema_version": manifest.schema_version,
            "images": manifest.images.iter().map(|image| json!({
                "id": image.id,
                "key": image.key,
                "image_reference": image.image_reference,
                "preparation_state": image.preparation_state,
                "availability_state": image.availability_state,
            })).collect::<Vec<_>>(),
        }));
    }

    let mut transaction = pool.begin().await?;
    for image in &manifest.images {
        provision_catalog_image(&mut transaction, image).await?;
    }
    transaction.commit().await?;

    Ok(json!({
        "mode": "upsert",
        "schema_version": manifest.schema_version,
        "upserted": manifest.images.len(),
        "keys": manifest.images.iter().map(|image| image.key.as_str()).collect::<Vec<_>>(),
    }))
}

async fn provision_catalog_image(
    transaction: &mut Transaction<'_, Postgres>,
    image: &BuilderCatalogRecord,
) -> Result<(), Box<dyn Error>> {
    let approved = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (
                SELECT 1
                FROM registry_publications publication
                JOIN registry_namespaces namespace ON namespace.id = publication.namespace_id
                WHERE publication.owner_kind = 'platform_builder'
                  AND publication.platform_builder_key = $1
                  AND publication.state = 'approved'
                  AND publication.registry_authority || '/' || namespace.repository_path
                      || '@' || publication.expected_digest = $2
            )",
    )
    .bind(&image.key)
    .bind(&image.image_reference)
    .fetch_one(&mut **transaction)
    .await?;
    if !approved {
        return Err(format!(
            "catalog image {} does not match an approved forge registry publication",
            image.key
        )
        .into());
    }
    let changed = sqlx::query(
        "INSERT INTO builder_images
               (id, key, display_name, image_reference, toolchains,
                architectures, preparation_state, availability_state,
                network_ceiling, max_vcpus, max_memory_mib, dependency_policy,
                provenance, signature_reference, sbom_reference,
                platform_policy_version)
             VALUES
               ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14,
                $15, $16)
             ON CONFLICT (key) DO UPDATE SET
               display_name = EXCLUDED.display_name,
               image_reference = EXCLUDED.image_reference,
               toolchains = EXCLUDED.toolchains,
               architectures = EXCLUDED.architectures,
               preparation_state = EXCLUDED.preparation_state,
               availability_state = EXCLUDED.availability_state,
               network_ceiling = EXCLUDED.network_ceiling,
               max_vcpus = EXCLUDED.max_vcpus,
               max_memory_mib = EXCLUDED.max_memory_mib,
               dependency_policy = EXCLUDED.dependency_policy,
               provenance = EXCLUDED.provenance,
               signature_reference = EXCLUDED.signature_reference,
               sbom_reference = EXCLUDED.sbom_reference,
               platform_policy_version = EXCLUDED.platform_policy_version,
               updated_at = now()
             WHERE builder_images.id = EXCLUDED.id",
    )
    .bind(image.id)
    .bind(&image.key)
    .bind(&image.display_name)
    .bind(&image.image_reference)
    .bind(&image.toolchains)
    .bind(&image.architectures)
    .bind(&image.preparation_state)
    .bind(&image.availability_state)
    .bind(&image.network_ceiling)
    .bind(image.max_vcpus)
    .bind(image.max_memory_mib)
    .bind(&image.dependency_policy)
    .bind(&image.provenance)
    .bind(image.signature_reference.as_deref())
    .bind(image.sbom_reference.as_deref())
    .bind(&image.platform_policy_version)
    .execute(&mut **transaction)
    .await?;
    if changed.rows_affected() != 1 {
        return Err(format!(
            "catalog key {} exists with a different stable id; refusing to replace it",
            image.key
        )
        .into());
    }
    Ok(())
}

fn parse_builder_catalog_manifest(
    contents: &str,
    path: &Path,
) -> Result<BuilderCatalogManifest, Box<dyn Error>> {
    let document = serde_json::from_str::<Value>(contents)
        .map_err(|error| format!("{} is not valid JSON: {error}", path.display()))?;
    let object = document
        .as_object()
        .ok_or_else(|| manifest_error("builder catalog manifest must be a JSON object"))?;
    let schema_version = required_u64(object, "schema_version")?;
    if schema_version != 1 {
        return Err(manifest_error(format!(
            "unsupported builder catalog manifest schema_version {schema_version}; expected 1"
        )));
    }
    let images = required_array(object, "images")?;
    if images.is_empty() {
        return Err(manifest_error(
            "builder catalog manifest must contain at least one image",
        ));
    }

    let mut records = Vec::with_capacity(images.len());
    let mut ids = HashSet::with_capacity(images.len());
    let mut keys = HashSet::with_capacity(images.len());
    let mut references = HashSet::with_capacity(images.len());
    for (index, image) in images.iter().enumerate() {
        let record = parse_builder_catalog_record(image, index)?;
        if !ids.insert(record.id) {
            return Err(manifest_error(format!(
                "images[{index}] duplicates stable id {}",
                record.id
            )));
        }
        if !keys.insert(record.key.clone()) {
            return Err(manifest_error(format!(
                "images[{index}] duplicates key {}",
                record.key
            )));
        }
        if !references.insert(record.image_reference.clone()) {
            return Err(manifest_error(format!(
                "images[{index}] duplicates image reference {}",
                record.image_reference
            )));
        }
        records.push(record);
    }
    Ok(BuilderCatalogManifest {
        schema_version,
        images: records,
    })
}

// Keep all field validation in one reviewable record parser so malformed
// catalog entries cannot bypass a validation branch by taking another path.
#[allow(clippy::too_many_lines)]
fn parse_builder_catalog_record(
    value: &Value,
    index: usize,
) -> Result<BuilderCatalogRecord, Box<dyn Error>> {
    let object = value
        .as_object()
        .ok_or_else(|| manifest_error(format!("images[{index}] must be a JSON object")))?;
    let id = Uuid::parse_str(required_string(object, "id")?.as_str())
        .map_err(|error| manifest_error(format!("images[{index}].id is invalid: {error}")))?;
    let key = required_string(object, "key")?;
    validate_key(&key, index)?;
    let display_name = required_string(object, "display_name")?;
    if display_name.trim().is_empty() || display_name.len() > 200 {
        return Err(manifest_error(format!(
            "images[{index}].display_name must contain 1..=200 non-whitespace bytes"
        )));
    }
    let image_reference = required_string(object, "image_reference")?;
    validate_image_reference(&image_reference, index)?;
    let toolchains = required_value(object, "toolchains")?.clone();
    validate_toolchains(&toolchains, index)?;
    let architectures = string_array(object, "architectures", index)?;
    if architectures.is_empty()
        || architectures.iter().any(|architecture| {
            architecture.is_empty()
                || architecture.len() > 32
                || architecture
                    .bytes()
                    .any(|byte| byte.is_ascii_control() || byte.is_ascii_whitespace())
        })
    {
        return Err(manifest_error(format!(
            "images[{index}].architectures must contain non-empty, bounded values"
        )));
    }
    let preparation_state = required_string(object, "preparation_state")?;
    if !matches!(preparation_state.as_str(), "ready" | "preparing" | "failed") {
        return Err(manifest_error(format!(
            "images[{index}].preparation_state is invalid"
        )));
    }
    let availability_state = required_string(object, "availability_state")?;
    if !matches!(
        availability_state.as_str(),
        "available" | "unavailable" | "retired"
    ) {
        return Err(manifest_error(format!(
            "images[{index}].availability_state is invalid"
        )));
    }
    let network_ceiling = required_string(object, "network_ceiling")?;
    if !matches!(
        network_ceiling.as_str(),
        "disabled" | "broker_only" | "egress"
    ) {
        return Err(manifest_error(format!(
            "images[{index}].network_ceiling is invalid"
        )));
    }
    let max_vcpus = required_i64(object, "max_vcpus", index)?;
    if !(1..=64).contains(&max_vcpus) {
        return Err(manifest_error(format!(
            "images[{index}].max_vcpus must be between 1 and 64"
        )));
    }
    let max_memory_mib = required_i64(object, "max_memory_mib", index)?;
    if !(128..=1_048_576).contains(&max_memory_mib) {
        return Err(manifest_error(format!(
            "images[{index}].max_memory_mib must be between 128 and 1048576"
        )));
    }
    let dependency_policy = required_string(object, "dependency_policy")?;
    if !matches!(
        dependency_policy.as_str(),
        "vendored_offline" | "read_only_platform_cache" | "constrained_registry_egress"
    ) {
        return Err(manifest_error(format!(
            "images[{index}].dependency_policy is invalid"
        )));
    }
    let provenance = required_value(object, "provenance")?.clone();
    let provenance_object = provenance.as_object().ok_or_else(|| {
        manifest_error(format!("images[{index}].provenance must be a JSON object"))
    })?;
    let source = required_string(provenance_object, "source")?;
    if source.trim().is_empty() {
        return Err(manifest_error(format!(
            "images[{index}].provenance.source must not be empty"
        )));
    }
    let signature_reference = optional_string(provenance_object, "signature")?;
    let sbom_reference = optional_string(provenance_object, "sbom")?;
    let platform_policy_version = required_string(object, "platform_policy_version")?;
    if platform_policy_version.trim().is_empty() || platform_policy_version.len() > 128 {
        return Err(manifest_error(format!(
            "images[{index}].platform_policy_version must contain 1..=128 bytes"
        )));
    }

    Ok(BuilderCatalogRecord {
        id,
        key,
        display_name,
        image_reference,
        toolchains,
        architectures,
        preparation_state,
        availability_state,
        network_ceiling,
        max_vcpus,
        max_memory_mib,
        dependency_policy,
        provenance,
        signature_reference,
        sbom_reference,
        platform_policy_version,
    })
}

fn validate_key(value: &str, index: usize) -> Result<(), Box<dyn Error>> {
    let valid = (1..=64).contains(&value.len())
        && value.bytes().enumerate().all(|(position, byte)| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || ((byte == b'_' || byte == b'-') && position > 0)
        });
    valid.then_some(()).ok_or_else(|| {
        manifest_error(format!(
            "images[{index}].key must be a lowercase catalog identifier"
        ))
    })
}

fn validate_image_reference(value: &str, index: usize) -> Result<(), Box<dyn Error>> {
    let Some((repository, digest)) = value.rsplit_once("@sha256:") else {
        return Err(manifest_error(format!(
            "images[{index}].image_reference must be digest-pinned"
        )));
    };
    let valid_repository = !repository.is_empty()
        && repository == repository.trim()
        && !repository
            .bytes()
            .any(|byte| byte.is_ascii_control() || byte.is_ascii_whitespace());
    let valid_digest = digest.len() == 64
        && digest
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase());
    if valid_repository && valid_digest {
        Ok(())
    } else {
        Err(manifest_error(format!(
            "images[{index}].image_reference must end in a lowercase 64-character sha256 digest"
        )))
    }
}

fn validate_toolchains(value: &Value, index: usize) -> Result<(), Box<dyn Error>> {
    let toolchains = value
        .as_array()
        .ok_or_else(|| manifest_error(format!("images[{index}].toolchains must be an array")))?;
    if toolchains.is_empty() {
        return Err(manifest_error(format!(
            "images[{index}].toolchains must contain at least one toolchain"
        )));
    }
    for (toolchain_index, toolchain) in toolchains.iter().enumerate() {
        let object = toolchain.as_object().ok_or_else(|| {
            manifest_error(format!(
                "images[{index}].toolchains[{toolchain_index}] must be an object"
            ))
        })?;
        let name = required_string(object, "name")?;
        let version = required_string(object, "version")?;
        if name.trim().is_empty()
            || name.len() > 64
            || version.trim().is_empty()
            || version.len() > 128
        {
            return Err(manifest_error(format!(
                "images[{index}].toolchains[{toolchain_index}] has invalid name or version"
            )));
        }
    }
    Ok(())
}

fn required_value<'a>(
    object: &'a Map<String, Value>,
    key: &str,
) -> Result<&'a Value, Box<dyn Error>> {
    object
        .get(key)
        .ok_or_else(|| manifest_error(format!("missing required field {key}")))
}

fn required_string(object: &Map<String, Value>, key: &str) -> Result<String, Box<dyn Error>> {
    required_value(object, key)?
        .as_str()
        .map(String::from)
        .ok_or_else(|| manifest_error(format!("field {key} must be a string")))
}

fn optional_string(
    object: &Map<String, Value>,
    key: &str,
) -> Result<Option<String>, Box<dyn Error>> {
    match object.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_str()
            .map(|value| Some(String::from(value)))
            .ok_or_else(|| manifest_error(format!("field {key} must be a string or null"))),
    }
}

fn required_u64(object: &Map<String, Value>, key: &str) -> Result<u64, Box<dyn Error>> {
    required_value(object, key)?
        .as_u64()
        .ok_or_else(|| manifest_error(format!("field {key} must be a positive integer")))
}

fn required_i64(
    object: &Map<String, Value>,
    key: &str,
    index: usize,
) -> Result<i64, Box<dyn Error>> {
    required_value(object, key)?
        .as_i64()
        .ok_or_else(|| manifest_error(format!("images[{index}].{key} must be an integer")))
}

fn required_array<'a>(
    object: &'a Map<String, Value>,
    key: &str,
) -> Result<&'a Vec<Value>, Box<dyn Error>> {
    required_value(object, key)?
        .as_array()
        .ok_or_else(|| manifest_error(format!("field {key} must be an array")))
}

fn string_array(
    object: &Map<String, Value>,
    key: &str,
    index: usize,
) -> Result<Vec<String>, Box<dyn Error>> {
    required_array(object, key)?
        .iter()
        .enumerate()
        .map(|(array_index, value)| {
            value.as_str().map(String::from).ok_or_else(|| {
                manifest_error(format!(
                    "images[{index}].{key}[{array_index}] must be a string"
                ))
            })
        })
        .collect()
}

fn manifest_error(message: impl Into<String>) -> Box<dyn Error> {
    message.into().into()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InspectionTarget {
    Release,
    AgentInstance,
    Secret,
}

impl InspectionTarget {
    const fn object_type(self) -> ObjectType {
        match self {
            Self::Release => ObjectType::Release,
            Self::AgentInstance => ObjectType::AgentInstance,
            Self::Secret => ObjectType::Secret,
        }
    }

    const fn permission(self) -> Permission {
        match self {
            Self::Release | Self::AgentInstance => Permission::CanRead,
            Self::Secret => Permission::InspectMetadata,
        }
    }
}

fn inspection_arguments(
    arguments: &[String],
) -> Result<(AuthenticatedIdentity, Uuid), Box<dyn Error>> {
    Ok((
        identity_argument(arguments, 1, Some(3))?,
        Uuid::parse_str(argument(arguments, 2)?)?,
    ))
}

fn identity_argument(
    arguments: &[String],
    user_index: usize,
    request_index: Option<usize>,
) -> Result<AuthenticatedIdentity, Box<dyn Error>> {
    let user_id = UserId::from_uuid(Uuid::parse_str(argument(arguments, user_index)?)?);
    let request_id = request_index
        .and_then(|index| arguments.get(index))
        .map(|value| Uuid::parse_str(value))
        .transpose()?
        .map_or_else(RequestId::new, RequestId::from_uuid);
    Ok(AuthenticatedIdentity::new(
        user_id,
        "hephaestus-operator",
        user_id.to_string(),
        json!({"interface": "operator_cli"}),
        request_id,
    ))
}

fn argument(arguments: &[String], index: usize) -> Result<&str, Box<dyn Error>> {
    arguments
        .get(index)
        .map(String::as_str)
        .ok_or_else(|| usage().into())
}

async fn inspect(
    pool: &PgPool,
    identity: &AuthenticatedIdentity,
    target: InspectionTarget,
    object_id: Uuid,
) -> Result<Value, Box<dyn Error>> {
    let mut transaction = begin_actor_transaction(pool, identity).await?;
    let authorizer = PostgresMelangeAuthorizer;
    let object_type = target.object_type();
    let object = ObjectRef::new(object_type, object_id);
    let permission = target.permission();
    let decision = authorizer
        .check(
            &mut transaction,
            Subject::User(identity.user_id),
            permission,
            object,
        )
        .await?;
    audit_decision(
        &mut transaction,
        identity.user_id,
        permission,
        object,
        decision,
        identity.request_id,
    )
    .await?;
    if decision == AuthorizationDecision::Deny {
        transaction.commit().await?;
        return Err("inspection authorization denied".into());
    }
    let value = load_inspection(&mut transaction, target, object_id)
        .await?
        .ok_or("object is unavailable")?;
    if target == InspectionTarget::Secret {
        audit_secret_inspection(&mut transaction, identity, object_id).await?;
    }
    transaction.commit().await?;
    Ok(value)
}

async fn load_inspection(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    target: InspectionTarget,
    object_id: Uuid,
) -> Result<Option<Value>, sqlx::Error> {
    match target {
        InspectionTarget::Release => {
            sqlx::query_scalar::<_, Value>(include_str!(
                "../../../hephaestus-app/sql/operator/release.sql"
            ))
            .bind(object_id)
            .fetch_optional(&mut **transaction)
            .await
        }
        InspectionTarget::AgentInstance => {
            sqlx::query_scalar::<_, Value>(include_str!(
                "../../../hephaestus-app/sql/operator/agent_instance.sql"
            ))
            .bind(object_id)
            .fetch_optional(&mut **transaction)
            .await
        }
        InspectionTarget::Secret => {
            sqlx::query_scalar::<_, Value>(include_str!(
                "../../../hephaestus-app/sql/operator/secret.sql"
            ))
            .bind(object_id)
            .fetch_optional(&mut **transaction)
            .await
        }
    }
}

async fn audit_secret_inspection(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    identity: &AuthenticatedIdentity,
    secret_id: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO secret_audit_events
           (id, owner_organization_id, requester_id, secret_id, operation,
            permission, decision, outcome, request_id, command_id,
            authorization_model_version, policy_version)
           SELECT $1, owner_organization_id, $2, id, 'inspect_metadata',
                  'secret.inspect_metadata', 'allow', 'metadata_returned',
                  $3, $3, $4, 'operator/v1'
           FROM secrets WHERE id = $5",
    )
    .bind(Uuid::new_v4())
    .bind(identity.user_id.as_uuid())
    .bind(identity.request_id.as_uuid())
    .bind(authz_postgres::AUTHORIZATION_MODEL_VERSION)
    .bind(secret_id)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

async fn inspect_metrics(
    pool: &PgPool,
    identity: &AuthenticatedIdentity,
) -> Result<Value, Box<dyn Error>> {
    let mut transaction = begin_actor_transaction(pool, identity).await?;
    let value = sqlx::query_scalar::<_, Value>(
        "SELECT jsonb_build_object(
               'release', COALESCE(
                   (SELECT to_jsonb(metric) FROM release_operation_metrics metric),
                   '{}'::jsonb
               ),
               'instance', COALESCE(
                   (SELECT to_jsonb(metric) FROM instance_operation_metrics metric),
                   '{}'::jsonb
               ),
               'secret', COALESCE(
                   (SELECT to_jsonb(metric) FROM secret_operation_metrics metric),
                   '{}'::jsonb
               )
           )",
    )
    .fetch_one(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(value)
}

async fn recover_update(pool: &PgPool, arguments: &[String]) -> Result<Value, Box<dyn Error>> {
    let identity = identity_argument(arguments, 1, Some(4))?;
    let update_id = AgentUpdateId::from_str(argument(arguments, 2)?)?;
    let action_name = argument(arguments, 3)?;
    let action = match action_name {
        "retry" => UpdateRecoveryAction::RetryHook,
        "reject" => UpdateRecoveryAction::RejectCandidate,
        "resume" => UpdateRecoveryAction::ResumeActivation,
        _ => return Err("recovery action must be retry, reject, or resume".into()),
    };
    let command_key = ReleaseCommandKey::derive(
        "operator-recover-update",
        &[
            update_id.as_uuid().as_bytes(),
            action_name.as_bytes(),
            identity.request_id.as_uuid().as_bytes(),
        ],
    );
    let service = ReleaseService::new(pool.clone(), Arc::new(PostgresMelangeAuthorizer));
    let decision = service
        .recover_update(
            &identity,
            RecoverInstanceUpdate {
                command_key,
                update_id,
                action,
            },
        )
        .await?;
    Ok(json!({
        "update_id": update_id,
        "action": action_name,
        "decision": format!("{decision:?}"),
        "request_id": identity.request_id,
    }))
}

async fn abandon_build(pool: &PgPool, arguments: &[String]) -> Result<Value, Box<dyn Error>> {
    let identity = identity_argument(arguments, 1, Some(3))?;
    let build_id = BuildRequestId::from_str(argument(arguments, 2)?)?;
    let object = ObjectRef::new(ObjectType::Build, build_id.as_uuid());
    let authorizer = PostgresMelangeAuthorizer;
    let mut transaction = begin_actor_transaction(pool, &identity).await?;
    let decision = authorizer
        .check(
            &mut transaction,
            Subject::User(identity.user_id),
            Permission::CanCancel,
            object,
        )
        .await?;
    audit_decision(
        &mut transaction,
        identity.user_id,
        Permission::CanCancel,
        object,
        decision,
        identity.request_id,
    )
    .await?;
    if decision == AuthorizationDecision::Deny {
        transaction.commit().await?;
        return Err("build recovery authorization denied".into());
    }
    let changed = sqlx::query(
        "UPDATE build_requests
           SET state = 'cancelled', completed_at = now(),
               diagnostics = jsonb_build_array(jsonb_build_object(
                   'code', 'operator_abandoned',
                   'request_id', $2::text
               ))
           WHERE id = $1 AND state IN ('queued', 'running', 'importing')",
    )
    .bind(build_id.as_uuid())
    .bind(identity.request_id.as_uuid())
    .execute(&mut *transaction)
    .await?;
    if changed.rows_affected() != 1 {
        return Err("build is unavailable or already terminal".into());
    }
    sqlx::query(
        "UPDATE build_executions
           SET state = 'failed', failure_code = 'operator_abandoned',
               completed_at = now(), updated_at = now()
           WHERE build_request_id = $1 AND state <> 'drafted'",
    )
    .bind(build_id.as_uuid())
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(json!({
        "build_request_id": build_id,
        "state": "cancelled",
        "request_id": identity.request_id,
    }))
}

const fn usage() -> &'static str {
    "usage: hephaestus-operator <inspect-release|inspect-instance|inspect-secret> \
       <actor-uuid> <object-uuid> [request-uuid]\n\
       hephaestus-operator metrics <actor-uuid> [request-uuid]\n\
       hephaestus-operator recover-update <actor-uuid> <update-uuid> \
       <retry|reject|resume> [request-uuid]\n\
       hephaestus-operator abandon-build <actor-uuid> <build-uuid> [request-uuid]\n\
       hephaestus-operator provision-builder-catalog <manifest.json> [--dry-run]\n\
       hephaestus-operator registry-retention-report <inventory.json>"
}

#[cfg(test)]
mod tests {
    use super::{InspectionTarget, ObjectType, Permission, parse_builder_catalog_manifest};
    use std::path::Path;

    const TEST_DIGEST: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    #[test]
    fn inspection_targets_have_closed_authorization_policies() {
        assert_eq!(InspectionTarget::Release.object_type(), ObjectType::Release);
        assert_eq!(InspectionTarget::Release.permission(), Permission::CanRead);
        assert_eq!(
            InspectionTarget::AgentInstance.object_type(),
            ObjectType::AgentInstance
        );
        assert_eq!(
            InspectionTarget::AgentInstance.permission(),
            Permission::CanRead
        );
        assert_eq!(InspectionTarget::Secret.object_type(), ObjectType::Secret);
        assert_eq!(
            InspectionTarget::Secret.permission(),
            Permission::InspectMetadata
        );
    }

    fn manifest_json(reference: &str) -> String {
        r#"{
                "schema_version": 1,
                "images": [{
                    "id": "20000000-0000-4000-8000-000000000010",
                    "key": "ubuntu-native",
                    "display_name": "Ubuntu native builder",
                    "image_reference": "__REFERENCE__",
                    "toolchains": [{"name":"shell","version":"ubuntu-24.04"}],
                    "architectures": ["x86_64"],
                    "preparation_state": "ready",
                    "availability_state": "available",
                    "network_ceiling": "disabled",
                    "max_vcpus": 4,
                    "max_memory_mib": 1024,
                    "dependency_policy": "vendored_offline",
                    "provenance": {"source":"attestation://test/ubuntu-native"},
                    "platform_policy_version": "builder/v1"
                }]
            }"#
        .replace("__REFERENCE__", reference)
    }

    #[test]
    fn catalog_manifest_requires_explicit_digest_pinned_records() {
        let reference = format!("registry.example/ubuntu@sha256:{TEST_DIGEST}");
        let manifest = manifest_json(&reference);
        let parsed = parse_builder_catalog_manifest(&manifest, Path::new("test.json"))
            .expect("manifest should be valid");
        assert_eq!(parsed.schema_version, 1);
        assert_eq!(parsed.images[0].key, "ubuntu-native");
        assert_eq!(parsed.images[0].image_reference, reference);
    }

    #[test]
    fn catalog_manifest_rejects_unpinned_references() {
        let tagged = manifest_json("registry.example/ubuntu:24.04");
        assert!(parse_builder_catalog_manifest(&tagged, Path::new("test.json")).is_err());
    }
}
