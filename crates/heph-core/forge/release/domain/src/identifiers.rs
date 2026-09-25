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
    ReleaseArtifactId,
    "A stable identifier for one immutable release artifact."
);
identifier!(
    AgentFamilyId,
    "A source-repository-scoped exported-agent family identifier."
);
identifier!(
    BuildRequestId,
    "A stable identifier for one exact idempotent build request."
);
identifier!(
    AgentUpdateId,
    "A stable identifier for one instance update transaction."
);
identifier!(
    DeferredTriggerId,
    "A stable identifier for a trigger received behind a closed run gate."
);
identifier!(
    UiInstallationId,
    "Stable identity for one project, repository, or organization-owned global UI installation."
);
identifier!(
    UiInstallationGenerationId,
    "Opaque identity for one immutable UI activation generation."
);
