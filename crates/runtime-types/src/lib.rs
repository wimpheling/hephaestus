//! Stable identifiers shared by the run and volume domains.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
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

            /// Creates an identifier from its `UUID` representation.
            #[must_use]
            pub const fn from_uuid(value: Uuid) -> Self {
                Self(value)
            }

            /// Returns the `UUID` representation.
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

        impl From<Uuid> for $name {
            fn from(value: Uuid) -> Self {
                Self::from_uuid(value)
            }
        }

        impl From<$name> for Uuid {
            fn from(value: $name) -> Self {
                value.as_uuid()
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
    AgentInstanceId,
    "A stable identifier for a project-owned reusable agent instance."
);
identifier!(
    AgentInstanceRevisionId,
    "A stable identifier for an immutable agent-instance revision."
);
identifier!(
    AgentAttachmentId,
    "A stable identifier for a repository/ref attachment."
);
identifier!(ReleaseId, "A stable identifier for an immutable release.");
identifier!(
    ReleaseAgentId,
    "A stable identifier for one exported agent in a release."
);
identifier!(VolumeId, "A stable identifier for a persistent volume.");
identifier!(RunId, "A stable identifier for one agent execution.");
identifier!(
    CommandId,
    "An idempotency identifier for a durable command."
);
identifier!(EventId, "A stable identifier for a durable domain event.");
identifier!(LeaseId, "A stable identifier for a writable volume lease.");
