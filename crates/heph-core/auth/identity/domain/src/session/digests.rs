use super::super::AuthenticatedIdentity;
use super::{BrowserSessionDigest, BrowserSessionIdentityBindingDigest, BrowserSessionSid};
use sha2::{Digest, Sha256};

const BROWSER_SESSION_SID_DOMAIN: &[u8] = b"hephaestus-human-browser-session-sid-v1\0";
const BROWSER_SESSION_IDENTITY_DOMAIN: &[u8] = b"hephaestus-human-browser-session-identity-v1\0";

/// Hashes a session ID with a domain separator before durable storage.
#[must_use]
pub fn browser_session_sid_digest(session_id: BrowserSessionSid) -> BrowserSessionDigest {
    let mut digest = Sha256::new();
    digest.update(BROWSER_SESSION_SID_DOMAIN);
    digest.update(session_id.as_uuid().as_bytes());
    BrowserSessionDigest(digest.finalize().into())
}

/// Binds a session to the exact verified OIDC issuer and subject.
///
/// Each UTF-8 field is length-prefixed before hashing so different issuer and
/// subject pairs cannot become the same byte sequence through concatenation.
#[must_use]
pub fn browser_session_identity_binding_digest(
    identity: &AuthenticatedIdentity,
) -> BrowserSessionIdentityBindingDigest {
    let mut digest = Sha256::new();
    digest.update(BROWSER_SESSION_IDENTITY_DOMAIN);
    update_hash_field(&mut digest, identity.issuer.as_bytes());
    update_hash_field(&mut digest, identity.subject.as_bytes());
    BrowserSessionIdentityBindingDigest(digest.finalize().into())
}

fn update_hash_field(digest: &mut Sha256, value: &[u8]) {
    let length = u64::try_from(value.len()).expect("UTF-8 field length fits in u64");
    digest.update(length.to_be_bytes());
    digest.update(value);
}
