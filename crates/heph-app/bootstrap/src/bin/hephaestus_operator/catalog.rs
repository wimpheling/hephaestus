use registry_domain::{RegistryInventory, RegistryInventoryDocument, RegistryRetentionReport};
use registry_postgres::PgRegistryStore;
use serde_json::{Value, json};
use sqlx::{Postgres, Transaction, postgres::PgPool};
use std::{error::Error, fs, path::Path};
use uuid::Uuid;

use super::catalog_validation::parse_image_catalog_manifest;

pub async fn registry_retention_report(
    pool: &PgPool,
    arguments: &[String],
) -> Result<Value, Box<dyn Error>> {
    if arguments.len() != 2 {
        return Err(super::cli::usage().into());
    }
    let inventory = load_registry_inventory(Path::new(super::cli::argument(arguments, 1)?))?;
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
pub struct OciImageRecord {
    pub id: Uuid,
    pub key: String,
    pub display_name: String,
    pub image_reference: String,
    pub toolchains: Value,
    pub architectures: Vec<String>,
    pub availability_state: String,
    pub role: String,
    pub provenance: Value,
    pub signature_reference: Option<String>,
    pub sbom_reference: Option<String>,
    pub platform_policy_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OciImageCatalogManifest {
    pub schema_version: u64,
    pub images: Vec<OciImageRecord>,
}

pub async fn provision_image_catalog(
    pool: &PgPool,
    arguments: &[String],
) -> Result<Value, Box<dyn Error>> {
    let manifest_path = super::cli::argument(arguments, 1)?;
    let dry_run = arguments[2..]
        .iter()
        .try_fold(false, |dry_run, flag| match flag.as_str() {
            "--dry-run" if !dry_run => Ok(true),
            "--dry-run" => Err("--dry-run may only be supplied once"),
            _ => Err("unknown provision-image-catalog option"),
        })?;
    let contents = fs::read_to_string(manifest_path)?;
    let manifest = parse_image_catalog_manifest(&contents, Path::new(manifest_path))?;

    if dry_run {
        return Ok(json!({
            "mode": "dry_run",
            "schema_version": manifest.schema_version,
            "images": manifest.images.iter().map(|image| json!({
                "id": image.id,
                "key": image.key,
                "image_reference": image.image_reference,
                "availability_state": image.availability_state,
                "role": image.role,
            })).collect::<Vec<_>>(),
        }));
    }

    let mut transaction = pool.begin().await?;
    for image in &manifest.images {
        provision_oci_image(&mut transaction, image).await?;
    }
    transaction.commit().await?;

    Ok(json!({
        "mode": "upsert",
        "schema_version": manifest.schema_version,
        "upserted": manifest.images.len(),
        "keys": manifest.images.iter().map(|image| image.key.as_str()).collect::<Vec<_>>(),
    }))
}

async fn provision_oci_image(
    transaction: &mut Transaction<'_, Postgres>,
    image: &OciImageRecord,
) -> Result<(), Box<dyn Error>> {
    let changed = sqlx::query(
        "INSERT INTO oci_images
               (id, key, display_name, image_reference, toolchains,
                architectures, availability_state, role, provenance,
                signature_reference, sbom_reference,
                platform_policy_version)
             VALUES
               ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
             ON CONFLICT (key) DO UPDATE SET
               display_name = EXCLUDED.display_name,
               image_reference = EXCLUDED.image_reference,
               toolchains = EXCLUDED.toolchains,
               architectures = EXCLUDED.architectures,
               availability_state = EXCLUDED.availability_state,
               role = EXCLUDED.role,
               provenance = EXCLUDED.provenance,
               signature_reference = EXCLUDED.signature_reference,
               sbom_reference = EXCLUDED.sbom_reference,
               platform_policy_version = EXCLUDED.platform_policy_version,
               updated_at = now()
             WHERE oci_images.id = EXCLUDED.id",
    )
    .bind(image.id)
    .bind(&image.key)
    .bind(&image.display_name)
    .bind(&image.image_reference)
    .bind(&image.toolchains)
    .bind(&image.architectures)
    .bind(&image.availability_state)
    .bind(&image.role)
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
