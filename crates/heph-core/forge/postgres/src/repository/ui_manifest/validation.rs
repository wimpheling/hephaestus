use agent_config::{ConfigHash, Diagnostic};

use super::types::{
    GatewayManifestInspection, UiManifestEntryKind, UiManifestInspection, UiManifestStatus,
};
use super::{MAX_DIAGNOSTIC_BYTES, MAX_DIAGNOSTICS};

pub(super) const fn entry_kind(mode: gix::objs::tree::EntryMode) -> Option<UiManifestEntryKind> {
    if mode.is_blob() {
        Some(UiManifestEntryKind::Regular)
    } else if mode.is_link() {
        Some(UiManifestEntryKind::Symlink)
    } else if mode.is_tree() {
        Some(UiManifestEntryKind::Tree)
    } else if mode.is_commit() {
        Some(UiManifestEntryKind::Gitlink)
    } else {
        None
    }
}

pub(super) fn invalid_inspection(
    entry_kind: UiManifestEntryKind,
    object_id: gix::ObjectId,
    actual_size: Option<u64>,
    requires_gateways: bool,
    code: &'static str,
    message: &'static str,
) -> UiManifestInspection {
    UiManifestInspection {
        status: UiManifestStatus::Invalid,
        requires_gateways,
        entry_kind,
        object_id,
        actual_size,
        source_hash: None,
        normalized_hash: None,
        config: None,
        gateway_object_id: None,
        gateway_actual_size: None,
        gateway_source_hash: None,
        gateway_normalized_hash: None,
        gateway_config: None,
        diagnostics: vec![diagnostic(code, message)],
    }
}

pub(super) fn diagnostic(code: &'static str, message: &'static str) -> Diagnostic {
    Diagnostic {
        code: code.to_owned(),
        path: Some(String::from("manifest")),
        message: message.to_owned(),
    }
}

pub(super) fn redact_diagnostics(
    diagnostics: &[Diagnostic],
    fallback_code: &'static str,
) -> Vec<Diagnostic> {
    let mut result = Vec::new();
    let mut used = 0usize;
    for diagnostic in diagnostics.iter().take(MAX_DIAGNOSTICS) {
        let code = stable_code(&diagnostic.code).unwrap_or_else(|| fallback_code.to_owned());
        let value = Diagnostic {
            code,
            path: Some(String::from("manifest")),
            message: String::from("repository manifest is invalid"),
        };
        let bytes = value.code.len() + value.message.len() + 8;
        if used.saturating_add(bytes) > MAX_DIAGNOSTIC_BYTES {
            break;
        }
        used += bytes;
        result.push(value);
    }
    if result.is_empty() && !diagnostics.is_empty() {
        result.push(diagnostic(fallback_code, "repository manifest is invalid"));
    }
    result
}

fn stable_code(value: &str) -> Option<String> {
    (!value.is_empty()
        && value.len() <= 96
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_'))
    .then(|| value.to_owned())
}

impl UiManifestInspection {
    pub(super) fn with_source_hash(mut self, source_hash: Option<ConfigHash>) -> Self {
        self.source_hash = source_hash;
        self.diagnostics = redact_diagnostics(&self.diagnostics, "invalid_repository_ui_manifest");
        self
    }

    pub(super) fn with_gateway_observation(&mut self, gateway: &GatewayManifestInspection) {
        self.gateway_object_id = gateway.object_id;
        self.gateway_actual_size = gateway.actual_size;
        self.gateway_source_hash.clone_from(&gateway.source_hash);
        self.gateway_normalized_hash = None;
        self.gateway_config = None;
    }
}
