//! Canonical generation-host and bootstrap policy for UI serving.
//!
//! This module owns host grammar and platform bootstrap constants; HTTP
//! parsing, cookies, SQL, and listeners remain outside the service layer.

use release_domain::UiInstallationGenerationId;
use thiserror::Error;
use uuid::Uuid;

/// The Caddy namespace configured for the platform-owned UI origins.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiNamespace(String);

impl UiNamespace {
    /// Parses the canonical lowercase DNS namespace.
    ///
    /// # Errors
    ///
    /// Returns [`UiHostError::InvalidNamespace`] when the value is not a
    /// canonical DNS namespace.
    pub fn parse(value: impl Into<String>) -> Result<Self, UiHostError> {
        let value = value.into().to_ascii_lowercase();
        // `g-` + 32 hex digits + `.` are part of the generated label. Keep
        // the complete DNS host within the 253-byte authority limit.
        if value.is_empty() || value.len() > 218 || value.ends_with('.') {
            return Err(UiHostError::InvalidNamespace);
        }
        if value.split('.').any(|label| {
            label.is_empty()
                || label.len() > 63
                || label.starts_with('-')
                || label.ends_with('-')
                || !label
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        }) {
            return Err(UiHostError::InvalidNamespace);
        }
        Ok(Self(value))
    }

    /// Returns the canonical namespace.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for UiNamespace {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// The externally visible port for the UI origin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UiPublicPort(u16);

impl UiPublicPort {
    /// HTTPS's default port is used when configuration omits a value.
    #[must_use]
    pub const fn https_default() -> Self {
        Self(443)
    }

    /// Parses a configured public port.
    ///
    /// # Errors
    ///
    /// Returns [`UiHostError::InvalidPort`] for port zero.
    pub const fn parse(value: u16) -> Result<Self, UiHostError> {
        if value == 0 {
            Err(UiHostError::InvalidPort)
        } else {
            Ok(Self(value))
        }
    }

    /// Parses the canonical decimal spelling used in an authority/config.
    ///
    /// # Errors
    ///
    /// Returns [`UiHostError::InvalidPort`] for an invalid or non-canonical
    /// decimal port.
    pub fn parse_text(value: &str) -> Result<Self, UiHostError> {
        if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(UiHostError::InvalidPort);
        }
        let parsed = value.parse::<u16>().map_err(|_| UiHostError::InvalidPort)?;
        if parsed == 0 || parsed.to_string() != value {
            return Err(UiHostError::InvalidPort);
        }
        Ok(Self(parsed))
    }

    /// Returns the configured port.
    #[must_use]
    pub const fn get(self) -> u16 {
        self.0
    }
}

/// One canonical immutable generation host: `g-<32 lowercase hex>.<namespace>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UiGenerationHost {
    generation_id: UiInstallationGenerationId,
}

impl UiGenerationHost {
    /// Constructs a host value from an already canonical generation identity.
    #[must_use]
    pub const fn from_generation_id(generation_id: UiInstallationGenerationId) -> Self {
        Self { generation_id }
    }

    /// Parses a Host/authority value against the configured namespace and port.
    ///
    /// Case is folded for DNS comparison. A missing port means 443; explicit
    /// ports must equal the configured public port. Userinfo, trailing-dot
    /// aliases, IPv6 syntax, deep prefixes, and malformed labels are denied.
    ///
    /// # Errors
    ///
    /// Returns [`UiHostError::InvalidAuthority`] when the authority does not
    /// match the configured namespace and port.
    pub fn parse(
        authority: &str,
        namespace: &UiNamespace,
        public_port: UiPublicPort,
    ) -> Result<Self, UiHostError> {
        if authority.is_empty()
            || authority.bytes().any(|byte| {
                byte.is_ascii_control() || matches!(byte, b'@' | b'/' | b'?' | b'#' | b'\\')
            })
        {
            return Err(UiHostError::InvalidAuthority);
        }
        let (host, supplied_port) = match authority.rsplit_once(':') {
            Some((host, port)) if !host.contains(':') => {
                let parsed = UiPublicPort::parse_text(port)?.get();
                (host, Some(parsed))
            }
            Some(_) => return Err(UiHostError::InvalidAuthority),
            None => (authority, None),
        };
        let actual_port = supplied_port.unwrap_or(443);
        if actual_port != public_port.get() || host.is_empty() || host.ends_with('.') {
            return Err(UiHostError::InvalidAuthority);
        }
        let host = host.to_ascii_lowercase();
        let prefix = "g-";
        let suffix = format!(".{namespace}");
        let encoded = host
            .strip_prefix(prefix)
            .and_then(|value| value.strip_suffix(&suffix))
            .ok_or(UiHostError::UnknownGenerationHost)?;
        if encoded.len() != 32
            || !encoded
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            return Err(UiHostError::UnknownGenerationHost);
        }
        let uuid = Uuid::parse_str(encoded).map_err(|_| UiHostError::UnknownGenerationHost)?;
        Ok(Self {
            generation_id: UiInstallationGenerationId::from_uuid(uuid),
        })
    }

    /// Formats the canonical host authority, including a non-default port.
    #[must_use]
    pub fn authority(self, namespace: &UiNamespace, public_port: UiPublicPort) -> String {
        let host = format!("g-{}.{}", self.generation_id.as_uuid().simple(), namespace);
        debug_assert!(host.len() <= 253);
        if public_port.get() == 443 {
            host
        } else {
            format!("{host}:{}", public_port.get())
        }
    }

    /// Returns the encoded generation identity for the database lookup.
    #[must_use]
    pub const fn generation_id(self) -> UiInstallationGenerationId {
        self.generation_id
    }
}

/// Reserved platform path prefix. Release declarations must not shadow it.
pub const UI_RESERVED_PREFIX: &str = "/_heph/";
/// Same-origin bootstrap endpoint for handoff exchange.
pub const UI_BOOTSTRAP_PATH: &str = "/_heph/bootstrap";
/// Host-only child cookie name. It is distinct from Phoenix's platform cookie.
pub const UI_CHILD_COOKIE: &str = "__Host-hephaestus_ui";
/// Fragment encoding of the 32-byte handoff value: base64url without padding.
pub const UI_HANDOFF_FRAGMENT_LENGTH: usize = 43;

/// Returns whether a path belongs to the reserved platform namespace.
#[must_use]
pub fn is_reserved_ui_path(path: &str) -> bool {
    path == UI_RESERVED_PREFIX.trim_end_matches('/') || path.starts_with(UI_RESERVED_PREFIX)
}

/// Returns whether a declaration-relative route would collide with the
/// platform namespace after the handler adds its leading slash.
#[must_use]
pub fn is_reserved_ui_route(route: &str) -> bool {
    route == UI_RESERVED_PREFIX.trim_matches('/')
        || route.starts_with(UI_RESERVED_PREFIX.trim_start_matches('/'))
}

/// Checks the transport spelling of the bootstrap fragment before the secret
/// boundary decodes it into the non-serializable handoff secret type.
#[must_use]
pub fn is_handoff_fragment(value: &str) -> bool {
    value.len() == UI_HANDOFF_FRAGMENT_LENGTH
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

/// Host policy failures are intentionally generic at the HTTP boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum UiHostError {
    /// The configured namespace is not a canonical DNS namespace.
    #[error("UI namespace is invalid")]
    InvalidNamespace,
    /// The configured or supplied port is invalid.
    #[error("UI public port is invalid")]
    InvalidPort,
    /// The authority contains unsupported URL syntax or a wrong port.
    #[error("UI host authority is invalid")]
    InvalidAuthority,
    /// The exact generation host is not present in the configured namespace.
    #[error("UI generation host is unknown")]
    UnknownGenerationHost,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn generation() -> UiInstallationGenerationId {
        UiInstallationGenerationId::from_uuid(
            Uuid::parse_str("01234567-89ab-cdef-0123-456789abcdef").expect("UUID"),
        )
    }

    #[test]
    fn generation_host_round_trips_default_and_explicit_ports() {
        let namespace = UiNamespace::parse("Ui.Example.Test").expect("namespace");
        let generation_host = UiGenerationHost::from_generation_id(generation());
        let default_authority =
            generation_host.authority(&namespace, UiPublicPort::https_default());
        assert_eq!(
            default_authority,
            "g-0123456789abcdef0123456789abcdef.ui.example.test"
        );
        assert_eq!(
            UiGenerationHost::parse(
                &default_authority,
                &namespace,
                UiPublicPort::https_default()
            )
            .expect("default host")
            .generation_id(),
            generation()
        );

        let port = UiPublicPort::parse(8443).expect("port");
        let explicit = generation_host.authority(&namespace, port);
        assert!(explicit.ends_with(":8443"));
        assert_eq!(
            UiGenerationHost::parse(&explicit, &namespace, port)
                .expect("explicit host")
                .generation_id(),
            generation()
        );
    }

    #[test]
    fn host_rejects_aliases_userinfo_and_noncanonical_ports() {
        let namespace = UiNamespace::parse("ui.example.test").expect("namespace");
        let host = "g-0123456789abcdef0123456789abcdef.ui.example.test";
        let port = UiPublicPort::https_default();
        for authority in [
            "G-0123456789abcdef0123456789abcdef.ui.example.test.",
            "user@g-0123456789abcdef0123456789abcdef.ui.example.test",
            "g-0123456789abcdef0123456789abcdef.ui.example.test:0443",
            "g-0123456789abcdef0123456789abcdef.ui.example.test:8443",
        ] {
            assert!(UiGenerationHost::parse(authority, &namespace, port).is_err());
        }
        assert!(UiGenerationHost::parse(host, &namespace, port).is_ok());
    }

    #[test]
    fn namespace_accounts_for_generated_label_and_reserved_paths_are_exact() {
        let max_namespace = format!(
            "{}.{}.{}.{}",
            "a".repeat(63),
            "b".repeat(63),
            "c".repeat(63),
            "d".repeat(26)
        );
        let oversized_namespace = format!(
            "{}.{}.{}.{}",
            "a".repeat(63),
            "b".repeat(63),
            "c".repeat(63),
            "d".repeat(27)
        );
        assert!(UiNamespace::parse(oversized_namespace).is_err());
        assert!(UiNamespace::parse(max_namespace).is_ok());
        assert!(is_reserved_ui_path("/_heph/anything"));
        assert!(is_reserved_ui_path("/_heph"));
        assert!(!is_reserved_ui_path("/_hepha/anything"));
        assert!(is_reserved_ui_route("_heph/bootstrap"));
        assert!(!is_reserved_ui_route("heph/bootstrap"));
    }

    #[test]
    fn handoff_fragment_has_exact_transport_shape() {
        assert!(is_handoff_fragment(&"a".repeat(UI_HANDOFF_FRAGMENT_LENGTH)));
        assert!(!is_handoff_fragment(
            "a".repeat(UI_HANDOFF_FRAGMENT_LENGTH - 1).as_str()
        ));
        assert!(!is_handoff_fragment(&format!(
            "{}=",
            "a".repeat(UI_HANDOFF_FRAGMENT_LENGTH)
        )));
    }
}
