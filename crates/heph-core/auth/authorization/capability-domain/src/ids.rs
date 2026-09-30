use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};
use uuid::Uuid;

macro_rules! identifier {
    ($name:ident, $documentation:literal) => {
        #[doc = $documentation]
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(Uuid);

        impl $name {
            /// Creates a random version 4 identifier.
            #[must_use]
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }

            /// Creates an identifier from its UUID representation.
            #[must_use]
            pub const fn from_uuid(value: Uuid) -> Self {
                Self(value)
            }

            /// Returns the UUID representation.
            #[must_use]
            pub const fn as_uuid(self) -> Uuid {
                self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }

        impl FromStr for $name {
            type Err = uuid::Error;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Uuid::parse_str(value).map(Self)
            }
        }
    };
}

identifier!(
    CapabilityRequirementId,
    "A stable identifier for one symbolic release capability requirement."
);
identifier!(
    CapabilityBindingId,
    "A stable identifier for one exact immutable capability binding."
);
identifier!(
    AuthorizationSnapshotId,
    "A stable identifier for one immutable dispatch-time authority snapshot."
);
identifier!(
    RuntimeSessionId,
    "A stable identifier for one short-lived authenticated runtime session."
);
identifier!(
    GatewayInvocationId,
    "A stable identifier for one exact gateway HTTP invocation."
);
