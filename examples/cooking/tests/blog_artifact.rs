//! Production build and artifact retrieval proof for the generated cooking blog.
//!
//! The caller supplies the commit produced by the authorized result
//! publication.  This helper asks the Build service to build that exact Git
//! object, publishes the resulting immutable release, and reads the generated
//! HTML through the authenticated Artifact service.

use super::cooking_builds::{BuildError, CookingBuildContext, PreparedCookingBlog};

#[path = "blog_artifact/access.rs"]
mod access;
#[path = "blog_artifact/build.rs"]
mod build;
#[path = "blog_artifact/polling.rs"]
mod polling;
#[path = "blog_artifact/rpc.rs"]
mod rpc;
#[cfg(test)]
#[path = "blog_artifact/tests.rs"]
mod tests;
#[path = "blog_artifact/types.rs"]
mod types;

pub(crate) use access::assert_outsider_cannot_read_release;
pub use build::build_publish_and_verify;
pub(crate) use polling::{wait_for_blog_configuration, wait_for_build};
pub(crate) use rpc::{
    invalid_state, opaque, opaque_value, outsider_assertion, request_context, response_id,
    rpc_artifact_client, rpc_build_client, rpc_release_client,
};
pub use types::PublishedCookingBlogArtifact;
