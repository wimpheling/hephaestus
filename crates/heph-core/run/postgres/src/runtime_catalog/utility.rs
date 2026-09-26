use run_domain::RunKind;
use run_orchestrator::RunRuntimeCatalogError;

pub(super) const fn run_kind_name(kind: RunKind) -> &'static str {
    match kind {
        RunKind::Normal => "normal",
        RunKind::Update => "update",
    }
}

pub(super) fn storage(error: sqlx::Error) -> RunRuntimeCatalogError {
    RunRuntimeCatalogError::Storage(Box::new(error))
}
