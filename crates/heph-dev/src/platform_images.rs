//! Explicit local release operations for the reviewed platform builder images.

mod operations;
mod receipt;
mod registry;
mod support;
mod validation;

pub use operations::{build, clean, import_base, publish, status};

#[cfg(test)]
mod tests {
    use super::{
        receipt::{
            IMPORT_RECEIPT_VERSION, ImportedBaseReceipt, UBUNTU_BASE, ubuntu_base_digest,
            validate_base_receipt,
        },
        validation::validate_revision,
    };

    #[test]
    fn accepts_only_immutable_lowercase_revisions() {
        assert!(validate_revision(&"a".repeat(40)).is_ok());
        assert!(validate_revision(&"b".repeat(64)).is_ok());
        assert!(validate_revision(&"A".repeat(40)).is_err());
        assert!(validate_revision("../unsafe").is_err());
    }

    #[test]
    fn rejects_base_import_receipts_that_change_the_reviewed_digest() {
        let receipt = ImportedBaseReceipt {
            version: IMPORT_RECEIPT_VERSION,
            upstream_reference: UBUNTU_BASE.into(),
            upstream_manifest_digest: ubuntu_base_digest().into(),
            local_reference: "localhost:55000/platform/bases/ubuntu:reviewed".into(),
            local_manifest_digest: "sha256:deadbeef".into(),
        };
        assert!(validate_base_receipt(&receipt).is_err());
    }
}
