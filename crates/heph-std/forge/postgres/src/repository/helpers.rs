use forge_service::ForgeRepositoryError;

pub fn storage(error: impl std::error::Error + Send + Sync + 'static) -> ForgeRepositoryError {
    ForgeRepositoryError::Storage(Box::new(error))
}

pub fn git(error: impl std::fmt::Display) -> ForgeRepositoryError {
    ForgeRepositoryError::GitInspection(error.to_string())
}

pub const fn serialization(error: serde_json::Error) -> ForgeRepositoryError {
    ForgeRepositoryError::Serialization(error)
}
