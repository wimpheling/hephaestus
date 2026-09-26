use run_orchestrator::RepositoryError;

pub fn storage(error: impl std::error::Error + Send + Sync + 'static) -> RepositoryError {
    RepositoryError::Storage(Box::new(error))
}
