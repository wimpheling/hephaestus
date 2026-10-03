use uuid::Uuid;

use super::InstanceExecutionError;

macro_rules! identifier {
    ($name:ident, $documentation:literal) => {
        #[doc = $documentation]
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(Uuid);

        impl $name {
            /// Checks a non-nil durable data identifier.
            ///
            /// This grants no caller authority or provider/commit proof.
            ///
            /// # Errors
            /// Returns `InvalidIdentifier` for a nil value.
            pub const fn from_uuid(value: Uuid) -> Result<Self, InstanceExecutionError> {
                if value.is_nil() {
                    Err(InstanceExecutionError::InvalidIdentifier)
                } else {
                    Ok(Self(value))
                }
            }

            /// Returns the stable `UUID` data value.
            #[must_use]
            pub const fn as_uuid(self) -> Uuid {
                self.0
            }
        }
    };
}

identifier!(
    InstanceActivationId,
    r"Immutable one-time activation data identity, separate from Invocation.

Activation and Invocation identities are distinct checked data types:

```compile_fail
use release_service::{InstanceActivationId, InstanceInvocationId};
fn invocation_identity(value: InstanceActivationId) -> InstanceInvocationId { value }
```"
);
identifier!(
    InstanceInvocationId,
    "Immutable Invocation request data identity, separate from middleware requests."
);

pub const fn require_ids(values: &[Uuid]) -> Result<(), InstanceExecutionError> {
    let mut index = 0;
    while index < values.len() {
        if values[index].is_nil() {
            return Err(InstanceExecutionError::InvalidIdentifier);
        }
        index += 1;
    }
    Ok(())
}
