use sha2::{Digest, Sha256};
use std::fmt;
use uuid::Uuid;

const HANDOFF_SECRET_DOMAIN: &[u8] = b"hephaestus-ui-browser-handoff-v1\0";
const SESSION_SECRET_DOMAIN: &[u8] = b"hephaestus-ui-browser-session-v1\0";

macro_rules! bearer_secret {
    (
        $secret:ident,
        $digest_type:ident,
        $domain:ident,
        $secret_doc:literal,
        $digest_doc:literal
    ) => {
        #[doc = $secret_doc]
        pub struct $secret([u8; 32]);

        impl $secret {
            /// Generates a 32-byte bearer value from two platform UUID-v4
            /// values. UUID-v4 encodes 244 bits of entropy in this 32-byte
            /// representation.
            #[must_use]
            pub fn random() -> Self {
                let first = Uuid::new_v4();
                let second = Uuid::new_v4();
                let mut bytes = [0; 32];
                bytes[..16].copy_from_slice(first.as_bytes());
                bytes[16..].copy_from_slice(second.as_bytes());
                Self(bytes)
            }

            /// Restores a secret supplied at its explicit transport boundary.
            /// Callers must not log or serialize the returned value.
            #[must_use]
            pub const fn from_bytes(bytes: [u8; 32]) -> Self {
                Self(bytes)
            }

            /// Returns the raw bytes for this secret's explicit transport
            /// boundary: a sensitive Phoenix request for a handoff, or an
            /// explicit `Set-Cookie` for a child session.
            #[must_use]
            pub const fn as_bytes(&self) -> &[u8; 32] {
                &self.0
            }

            /// Computes the domain-separated one-way storage digest.
            #[must_use]
            pub fn digest(&self) -> $digest_type {
                let mut digest = Sha256::new();
                digest.update($domain);
                digest.update(self.0);
                $digest_type(digest.finalize().into())
            }
        }

        impl fmt::Debug for $secret {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(concat!(stringify!($secret), "(REDACTED)"))
            }
        }

        #[doc = $digest_doc]
        #[derive(Clone, Copy, PartialEq, Eq, Hash)]
        pub struct $digest_type([u8; 32]);

        impl $digest_type {
            /// Returns digest bytes for a parameterized persistence adapter.
            #[must_use]
            pub const fn as_bytes(self) -> [u8; 32] {
                self.0
            }
        }

        impl fmt::Debug for $digest_type {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(concat!(stringify!($digest_type), "(REDACTED)"))
            }
        }

        impl fmt::Display for $digest_type {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("[redacted]")
            }
        }
    };
}

bearer_secret!(
    UiBrowserHandoffSecret,
    UiBrowserHandoffDigest,
    HANDOFF_SECRET_DOMAIN,
    "Raw one-time handoff bearer secret. It has no `Display` or `Serialize` implementation.",
    "One-way digest of a UI handoff secret."
);
bearer_secret!(
    UiBrowserSessionSecret,
    UiBrowserSessionDigest,
    SESSION_SECRET_DOMAIN,
    "Raw child-session bearer secret. It has no `Display` or `Serialize` implementation.",
    "One-way digest of a UI child-session secret."
);
