use run_domain::{VmOwnerHistory, VmOwnerHistoryConflict, VmOwnerHistoryUnknown};
use run_orchestrator::RepositoryError;
use uuid::Uuid;
use vm_trait::VmProviderOwnerScope;

#[derive(sqlx::FromRow)]
pub(super) struct Original {
    pub(super) execution_profile: String,
    pub(super) namespace: Option<String>,
    pub(super) host: Option<String>,
    pub(super) qualified: bool,
}

pub(super) fn namespace(text: &str) -> Result<Uuid, RepositoryError> {
    let id = Uuid::parse_str(text)
        .map_err(|_| RepositoryError::InvalidData("VM namespace is not a UUID"))?;
    if id.is_nil() || id.to_string() != text {
        return Err(RepositoryError::InvalidData(
            "VM namespace is not canonical nonnil UUID",
        ));
    }
    Ok(id)
}

pub(super) fn classify(originals: &[Original], expected: &VmProviderOwnerScope) -> VmOwnerHistory {
    let mut exact = false;
    let mut conflict = false;
    let mut unknown = None;
    for original in originals {
        if original.execution_profile == "historical_unproven" {
            unknown.get_or_insert(VmOwnerHistoryUnknown::HistoricalUnproven);
            continue;
        }
        if !original.qualified {
            unknown.get_or_insert(VmOwnerHistoryUnknown::UnqualifiedOriginal);
            continue;
        }
        let (Some(namespace_text), Some(host)) = (&original.namespace, &original.host) else {
            unknown.get_or_insert(VmOwnerHistoryUnknown::MissingOwnerIdentity);
            continue;
        };
        if namespace(namespace_text).is_err()
            || VmProviderOwnerScope::new(namespace_text.clone(), host.clone()).is_err()
        {
            unknown.get_or_insert(VmOwnerHistoryUnknown::MissingOwnerIdentity);
            continue;
        }
        if host.as_str() == expected.host_id() {
            if namespace_text.as_str() == expected.namespace() {
                exact = true;
            } else {
                conflict = true;
            }
        }
    }
    unknown.map_or_else(
        || {
            if conflict {
                VmOwnerHistory::Conflict(VmOwnerHistoryConflict::NamespaceMismatch)
            } else if exact {
                VmOwnerHistory::Exact
            } else {
                VmOwnerHistory::NoHistoryForConfiguredHost
            }
        },
        VmOwnerHistory::Unknown,
    )
}
