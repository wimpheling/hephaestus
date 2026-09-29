use serde::{Deserialize, Serialize};

use crate::{GitCapabilityError, MAX_OBJECTS, MAX_PACK_BYTES, MAX_REF_UPDATES, MAX_REQUEST_BYTES};

/// Bounded smart-HTTP request and object-transfer limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransferLimits {
    request_bytes: u64,
    pack_bytes: u64,
    object_count: u32,
    ref_updates: u16,
}

impl TransferLimits {
    /// Creates non-zero limits within the grammar's hard ceilings.
    ///
    /// # Errors
    ///
    /// Returns an error when any limit is zero or exceeds its hard ceiling.
    pub const fn new(
        request_bytes: u64,
        pack_bytes: u64,
        object_count: u32,
        ref_updates: u16,
    ) -> Result<Self, GitCapabilityError> {
        if request_bytes == 0
            || request_bytes > MAX_REQUEST_BYTES
            || pack_bytes == 0
            || pack_bytes > MAX_PACK_BYTES
            || object_count == 0
            || object_count > MAX_OBJECTS
            || ref_updates == 0
            || ref_updates > MAX_REF_UPDATES
        {
            return Err(GitCapabilityError::InvalidTransferLimits);
        }
        Ok(Self {
            request_bytes,
            pack_bytes,
            object_count,
            ref_updates,
        })
    }

    /// Maximum encoded request bytes.
    #[must_use]
    pub const fn request_bytes(self) -> u64 {
        self.request_bytes
    }

    /// Maximum accepted pack bytes.
    #[must_use]
    pub const fn pack_bytes(self) -> u64 {
        self.pack_bytes
    }

    /// Maximum accepted object count.
    #[must_use]
    pub const fn object_count(self) -> u32 {
        self.object_count
    }

    /// Maximum atomic ref updates in one receive.
    #[must_use]
    pub const fn ref_updates(self) -> u16 {
        self.ref_updates
    }
}
