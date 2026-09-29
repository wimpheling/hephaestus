use super::{
    CURSOR_TAG_LENGTH, OrganizationId, RpcError, UiInstallationId, UiInstallationTarget,
    UiInstallationTargetFilter, UserId,
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use hmac::{Hmac, Mac as _};
use sha2::Sha256;
use uuid::Uuid;
/// Signed cursor bound to actor, organization, exact target, method, and order.
#[derive(Clone)]
pub struct UiInstallationCursorCodec {
    key: [u8; 32],
}

impl UiInstallationCursorCodec {
    pub const fn new(key: [u8; 32]) -> Self {
        Self { key }
    }
    pub fn encode(
        &self,
        id: UiInstallationId,
        actor: UserId,
        organization: OrganizationId,
        target: UiInstallationTarget,
    ) -> String {
        let (kind, target_id) = cursor_scope(target);
        let payload = format!(
            "v1|{actor}|{organization}|{kind}|{target_id}|{}|created_at-desc-id-desc",
            id.as_uuid()
        );
        let tag = self.tag(payload.as_bytes());
        let mut bytes = payload.into_bytes();
        bytes.push(b'|');
        bytes.extend_from_slice(&tag);
        URL_SAFE_NO_PAD.encode(bytes)
    }
    pub fn decode(
        &self,
        token: &str,
        actor: UserId,
        organization: OrganizationId,
        target: UiInstallationTarget,
    ) -> Result<UiInstallationId, RpcError> {
        if token.is_empty() || token.len() > 512 || !token.is_ascii() {
            return Err(RpcError::InvalidArgument);
        }
        let bytes = URL_SAFE_NO_PAD
            .decode(token)
            .map_err(|_| RpcError::InvalidArgument)?;
        let delimiter = bytes
            .len()
            .checked_sub(CURSOR_TAG_LENGTH + 1)
            .ok_or(RpcError::InvalidArgument)?;
        if bytes.get(delimiter) != Some(&b'|') {
            return Err(RpcError::InvalidArgument);
        }
        let payload = &bytes[..delimiter];
        let supplied = &bytes[delimiter + 1..];
        if !self.verify_tag(payload, supplied) {
            return Err(RpcError::InvalidArgument);
        }
        let fields: Vec<&str> = std::str::from_utf8(payload)
            .map_err(|_| RpcError::InvalidArgument)?
            .split('|')
            .collect();
        let (kind, target_id) = cursor_scope(target);
        if fields.len() != 7
            || fields[0] != "v1"
            || fields[1] != actor.to_string()
            || fields[2] != organization.to_string()
            || fields[3] != kind
            || fields[4] != target_id.to_string()
            || fields[6] != "created_at-desc-id-desc"
        {
            return Err(RpcError::InvalidArgument);
        }
        Uuid::parse_str(fields[5])
            .map(UiInstallationId::from_uuid)
            .map_err(|_| RpcError::InvalidArgument)
    }
    pub fn tag(&self, payload: &[u8]) -> [u8; 32] {
        let mut mac =
            Hmac::<Sha256>::new_from_slice(&self.key).expect("HMAC accepts a 32-byte key");
        mac.update(b"hephaestus-ui-installation-cursor-v1\0");
        mac.update(payload);
        mac.finalize().into_bytes().into()
    }

    pub fn verify_tag(&self, payload: &[u8], supplied: &[u8]) -> bool {
        let mut mac =
            Hmac::<Sha256>::new_from_slice(&self.key).expect("HMAC accepts a 32-byte key");
        mac.update(b"hephaestus-ui-installation-cursor-v1\0");
        mac.update(payload);
        mac.verify_slice(supplied).is_ok()
    }
}
pub const fn cursor_scope(target: UiInstallationTarget) -> (&'static str, Uuid) {
    match target {
        UiInstallationTarget::Organization(id) => ("global", id.as_uuid()),
        UiInstallationTarget::Project(id) => ("project", id.as_uuid()),
        UiInstallationTarget::Repository(id) => ("repository", id.as_uuid()),
    }
}

pub trait TargetReceiptScope {
    fn aggregate_type(self) -> &'static str;
    fn primary_scope_kind(self) -> &'static str;
    fn filter(self) -> UiInstallationTargetFilter;
}

impl TargetReceiptScope for UiInstallationTarget {
    fn aggregate_type(self) -> &'static str {
        match self {
            Self::Organization(_) => "organization",
            Self::Project(_) => "project",
            Self::Repository(_) => "repository",
        }
    }
    fn primary_scope_kind(self) -> &'static str {
        match self {
            Self::Repository(_) => "project",
            Self::Organization(_) | Self::Project(_) => self.aggregate_type(),
        }
    }
    fn filter(self) -> UiInstallationTargetFilter {
        match self {
            Self::Organization(id) => {
                let _ = id;
                UiInstallationTargetFilter::Global
            }
            Self::Project(id) => UiInstallationTargetFilter::Project(id),
            Self::Repository(id) => UiInstallationTargetFilter::Repository(id),
        }
    }
}
