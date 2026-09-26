use crate::support::parse_reference;
use heph_build::{ClaimedMaterializationJob, MaterializedRoot, OciWorkerStoreError};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, FromRow)]
pub struct ProductionJobRow {
    pub id: Uuid,
    pub definition_id: Uuid,
}

#[derive(Debug, FromRow)]
pub struct ProductionInputRow {
    pub source_repository_id: Uuid,
    pub source_revision: String,
    pub project_id: Uuid,
    pub context_digest: String,
    pub dockerfile_path: String,
    pub context_path: String,
    pub base_image_reference: String,
}
#[derive(Debug, FromRow)]
pub struct MaterializationJobRow {
    pub id: Uuid,
    pub image_reference: String,
}

impl MaterializationJobRow {
    pub fn try_into_job(self) -> Result<ClaimedMaterializationJob, OciWorkerStoreError> {
        Ok(ClaimedMaterializationJob {
            id: self.id,
            image_reference: parse_reference(self.image_reference)?,
        })
    }
}

#[derive(Debug, FromRow)]
pub struct MaterializedRootRow {
    pub image_reference: String,
    pub root_path: String,
}

impl MaterializedRootRow {
    pub fn try_into_root(self) -> Result<MaterializedRoot, OciWorkerStoreError> {
        let root_path = std::path::PathBuf::from(self.root_path);
        if !root_path.is_absolute() {
            return Err(OciWorkerStoreError::Conflict);
        }
        Ok(MaterializedRoot {
            image_reference: parse_reference(self.image_reference)?,
            root_path,
        })
    }
}
