use crate::VmError;

/// Exact persistent provider ownership; unrelated to resource or recipe scopes.
///
/// Shape validation grants no authority. The provider must bind this namespace
/// to its actual root/account and host before reporting scoped absence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VmProviderOwnerScope {
    namespace: String,
    host_id: String,
}

impl VmProviderOwnerScope {
    /// Constructs checked opaque ownership identifiers.
    ///
    /// # Errors
    ///
    /// Rejects empty, excessive or unsafe identifiers.
    pub fn new(namespace: String, host_id: String) -> Result<Self, VmError> {
        for value in [&namespace, &host_id] {
            if value.is_empty()
                || value.len() > 128
                || matches!(value.as_str(), "." | "..")
                || !value.bytes().all(|byte| {
                    byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-')
                })
            {
                return Err(VmError::InvalidSpec {
                    field: "owner_scope".into(),
                    reason: "ownership identifiers require 1..128 safe ASCII bytes".into(),
                });
            }
        }
        Ok(Self { namespace, host_id })
    }

    /// Returns the persistent provider owner namespace.
    #[must_use]
    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    /// Returns the configured owning host.
    #[must_use]
    pub fn host_id(&self) -> &str {
        &self.host_id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owner_identifiers_are_bounded_and_path_free() {
        for invalid in [
            String::new(),
            "..".into(),
            "a/b".into(),
            "a\0b".into(),
            "é".into(),
            "a".repeat(129),
        ] {
            assert!(VmProviderOwnerScope::new(invalid.clone(), "host-1".into()).is_err());
            assert!(VmProviderOwnerScope::new("owner-1".into(), invalid).is_err());
        }
        let scope = VmProviderOwnerScope::new("owner-1".into(), "host-1".into()).unwrap();
        assert_eq!(scope.namespace(), "owner-1");
        assert_eq!(scope.host_id(), "host-1");
    }
}
