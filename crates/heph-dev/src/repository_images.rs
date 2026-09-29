//! Explicit local enablement for repository-owned OCI image preparation.

#[path = "repository_images/catalog.rs"]
mod catalog;
#[path = "repository_images/workflow.rs"]
mod workflow;

const WORKFLOW_VERSION: u8 = 1;
const BUILDER_KEY: &str = "oci-builder-ubuntu";
const VERIFIER_KEY: &str = "oci-verifier-ubuntu";

pub use workflow::{clean, disable, enable, status};

#[cfg(test)]
mod tests {
    use super::catalog::{Catalog, execution_role};

    #[test]
    fn legacy_catalog_omitted_role_defaults_to_execution() {
        let catalog: Catalog = serde_json::from_str(
            r#"{"images":[{"key":"ubuntu-native","image_reference":"registry.example/ubuntu@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}]}"#,
        )
        .expect("legacy catalog should deserialize");

        assert_eq!(catalog.images[0].role, execution_role());
    }

    #[test]
    fn operational_catalog_role_remains_distinct() {
        let catalog: Catalog = serde_json::from_str(
            r#"{"images":[{"key":"oci-builder-ubuntu","image_reference":"registry.example/builder@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","role":"platform_operation"}]}"#,
        )
        .expect("operational catalog should deserialize");

        assert_eq!(catalog.images[0].role, "platform_operation");
    }
}
