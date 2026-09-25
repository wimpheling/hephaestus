// Reuse the parent facade so sibling fixture phases share one boundary context.
#[allow(unused_imports)]
use super::*;
/// Provenance and IDs returned for one published cooking source repository.
#[derive(Debug, Clone)]
pub(crate) struct PublishedCookingRepository {
    /// Repository metadata ID.
    pub repository_id: RepositoryId,
    /// Authenticated actor which owns the created repository metadata.
    pub actor_id: UserId,
    /// Exact commit pushed through Git HTTP.
    pub source_commit: String,
    /// Build request created by the accepted push.
    pub build_request_id: Uuid,
    /// Release generated from the successful isolated build.
    pub release_id: Uuid,
    /// Agent identity inside the published release used by instance updates.
    pub release_agent_id: Uuid,
    /// Version assigned through `SetDraftVersion`.
    pub version: String,
    /// Canonical hash returned by the Build service.
    pub build_definition_hash: String,
    /// Normalized configuration hash returned by the Build service.
    pub configuration_hash: String,
    /// Release manifest hash returned by the Release service.
    pub manifest_hash: String,
    /// Temporary checkout retained under the fixture root until test cleanup.
    pub source_path: PathBuf,
    /// Mutable Git checkout used when publishing successive same-family
    /// update commits.  Callers should use [`Self::source_path`] for
    /// provenance because this path changes for each variant.
    pub working_path: PathBuf,
}

/// Results for both canonical cooking repositories.
#[derive(Debug, Clone)]
pub(crate) struct PublishedCookingBuilds {
    /// Rust gateway source and release.
    pub gateway: PublishedCookingRepository,
    /// Python agent source and release.
    pub agent: PublishedCookingRepository,
}

/// IDs returned by the production instance commands for a published cooking
/// agent and its separate blog repository attachment.
#[allow(clippy::struct_field_names)]
#[derive(Debug, Clone, Copy)]
pub(crate) struct PreparedCookingInstance {
    /// Instance created by `ImportAgent`.
    pub instance_id: Uuid,
    /// Immutable revision created by `ImportAgent`.
    pub revision_id: Uuid,
    /// Push attachment created against the blog repository.
    pub attachment_id: Uuid,
    /// Mailbox created by the instance service for the cooking run.
    pub mailbox_id: Uuid,
}

/// Separate Git repository attached to the cooking instance and its exact
/// pushed input commit.
#[derive(Debug, Clone)]
pub(crate) struct PreparedCookingBlog {
    /// Repository metadata ID.
    pub repository_id: RepositoryId,
    /// Exact commit accepted by the production Git receive path.
    pub source_commit: String,
}

/// Gateway identifiers returned after installing its released declaration and
/// reading it back through the query API.
#[derive(Debug, Clone, Copy)]
pub(crate) struct InstalledCookingGateway {
    /// Gateway metadata identifier.
    pub gateway_id: Uuid,
    /// Latest immutable declaration revision accepted by `ConfigureGateway`.
    pub revision_id: Uuid,
}

/// Safe identifiers returned by the installed-reference-UI fixture.  The
/// later browser phase needs only the owner target and current immutable
/// generations; release provenance, tokens, and handoff material stay inside
/// the disposable fixture.
#[derive(Debug, Clone, Copy)]
pub(crate) struct InstalledCookingUi {
    /// Installation command identity returned by the Release service.
    pub installation_id: Uuid,
    /// Current generation selected by the installation projection.
    pub generation_id: Uuid,
}

/// Checked-in reference UIs installed across project, repository, and global
/// owners, plus the managed project UI used by browser authority coverage.
#[derive(Debug, Clone, Copy)]
pub(crate) struct InstalledCookingReferenceUis {
    /// Organization owning the project target.
    pub organization_id: OrganizationId,
    /// Project target receiving both installations.
    pub project_id: ProjectId,
    /// Repository target receiving the repository-scoped static installation.
    pub repository_id: RepositoryId,
    /// Managed reference gateway identifier retained for post-restart readiness.
    pub managed_gateway_id: Uuid,
    /// Immutable managed reference gateway revision installed by the fixture.
    pub managed_gateway_revision_id: Uuid,
    /// Published managed reference release retained for later reactivation.
    pub managed_release_id: Uuid,
    /// Static full-page reference UI.
    pub static_ui: InstalledCookingUi,
    /// Repository-scoped static reference UI.
    pub repository_static_ui: InstalledCookingUi,
    /// Organization-global static reference UI.
    pub global_static_ui: InstalledCookingUi,
    /// Managed iframe reference UI.
    pub managed_ui: InstalledCookingUi,
}

/// Result of configuring the gateway and creating its ordinary mailbox grant.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ConfiguredCookingGateway {
    /// New immutable configured gateway revision.
    pub revision_id: Uuid,
    /// Revocable grant created by `CreateMailboxBinding`.
    pub grant_id: Uuid,
}

/// Slot used only by the joined authority probe. It is deliberately absent
/// from the released gateway declaration while a real second mailbox exists.
pub const FOREIGN_PUBLICATION_SLOT: &str = "cooking_foreign_requests";

/// Returns the cooking agent parameter payload used by the production import
/// and update RPC helpers.
pub(crate) fn cooking_agent_parameters() -> Vec<ParameterValue> {
    cooking_agent_parameters_for_rules(
        super::super::cooking::MODEL_RULE,
        super::super::cooking::RELAY_RULE,
    )
}

/// Returns typed parameters for an instance whose immutable broker rules were
/// allocated before import. This keeps transformed guest releases on the
/// ordinary ImportAgent/build path while allowing a separate rule namespace.
pub(crate) fn cooking_agent_parameters_for_rules(
    model_rule_id: uuid::Uuid,
    relay_rule_id: uuid::Uuid,
) -> Vec<ParameterValue> {
    vec![
        ParameterValue {
            name: String::from("model_rule_id"),
            value: Some(
                rpc_proto::messages::hephaestus::common::v1::parameter_value::Value::StringValue(
                    model_rule_id.to_string(),
                ),
            ),
            ..Default::default()
        },
        ParameterValue {
            name: String::from("relay_rule_id"),
            value: Some(
                rpc_proto::messages::hephaestus::common::v1::parameter_value::Value::StringValue(
                    relay_rule_id.to_string(),
                ),
            ),
            ..Default::default()
        },
    ]
}

/// Returns the host-to-guest inbound placeholder for the selected secret
/// version. The gateway edge rewrites the inbound credential using this
/// version UUID before the released guest receives the request.
pub(crate) fn cooking_inbound_placeholder(secret_version_id: Uuid) -> String {
    format!("heph-placeholder:v1:{secret_version_id}")
}

/// Returns the exact typed gateway parameter payload for a cooking fixture.
pub(crate) fn cooking_gateway_parameters(
    inbound_placeholder: &str,
    alice_provider_id: i64,
    bob_provider_id: i64,
) -> Vec<ParameterValue> {
    vec![
        ParameterValue {
            name: String::from("inbound_placeholder"),
            value: Some(
                rpc_proto::messages::hephaestus::common::v1::parameter_value::Value::StringValue(
                    inbound_placeholder.to_owned(),
                ),
            ),
            ..Default::default()
        },
        ParameterValue {
            name: String::from("alice_provider_id"),
            value: Some(
                rpc_proto::messages::hephaestus::common::v1::parameter_value::Value::IntegerValue(
                    alice_provider_id,
                ),
            ),
            ..Default::default()
        },
        ParameterValue {
            name: String::from("bob_provider_id"),
            value: Some(
                rpc_proto::messages::hephaestus::common::v1::parameter_value::Value::IntegerValue(
                    bob_provider_id,
                ),
            ),
            ..Default::default()
        },
    ]
}
