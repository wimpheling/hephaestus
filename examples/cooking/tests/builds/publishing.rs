// Reuse the parent facade so sibling fixture phases share one boundary context.
#[allow(unused_imports)]
use super::*;
/// Ordinary published agent releases used by the update lifecycle scenario.
#[derive(Debug, Clone)]
pub(crate) struct PublishedCookingUpdateBuilds {
    /// Candidate whose hook runs the v1-to-v2 `SQLite` migration.
    pub migrate: PublishedCookingRepository,
    /// Candidate whose hook deliberately returns a nonzero exit after its
    /// transactional migration rollback.
    pub rollback: PublishedCookingRepository,
    /// Candidate whose hook terminates by signal, requiring operator recovery.
    pub abnormal: PublishedCookingRepository,
}

/// Copies, pushes, builds, versions, and publishes the canonical cooking
/// gateway and agent sources.
///
/// Repository metadata is created through the trusted fixture factory.  Build
/// and release rows are created by the production Git receive/build workers;
/// this helper only observes them and invokes the existing release RPCs.
pub(crate) async fn build_and_publish(
    context: CookingBuildContext<'_>,
) -> Result<PublishedCookingBuilds, BuildError> {
    let gateway = build_one(
        &context,
        "cooking-gateway",
        canonical_source(context.source_root, "cooking-gateway"),
        "Cooking gateway",
        false,
    )
    .await?;
    let agent = build_one(
        &context,
        "cooking-agent",
        canonical_source(context.source_root, "cooking-agent"),
        "Cooking agent",
        true,
    )
    .await?;
    Ok(PublishedCookingBuilds { gateway, agent })
}

/// Builds, publishes, and installs the two checked-in reference UIs through
/// the production Git/build/release and Release RPC boundaries.
///
/// The service declaration is installed before the managed UI so its gateway
/// metadata exists when the UI command validates the descriptor.  This helper
/// deliberately stops at durable installation metadata: instance startup,
/// browser handoffs, and child sessions belong to the later acceptance phase.
// The bounded owner matrix is kept together so its install/list/SQL checks
// remain visibly paired with the descriptors they prove.
#[allow(clippy::too_many_lines)]
pub(crate) async fn build_and_install_reference_uis(
    context: &CookingBuildContext<'_>,
    organization_id: OrganizationId,
) -> Result<InstalledCookingReferenceUis, BuildError> {
    let static_build = build_one(
        context,
        "cooking-reference-ui",
        canonical_source(context.source_root, "cooking-reference-ui"),
        "Cooking reference static UI",
        false,
    )
    .await?;
    let managed_build = build_one(
        context,
        "cooking-reference-service-ui",
        canonical_source(context.source_root, "cooking-reference-service-ui"),
        "Cooking reference managed UI",
        false,
    )
    .await?;

    let static_release = get_published_release(context, static_build.release_id).await?;
    validate_reference_ui_descriptor(
        &static_release,
        "release-reference",
        ReleaseUiScope::RELEASE_UI_SCOPE_PROJECT,
        ReleaseUiPresentation::RELEASE_UI_PRESENTATION_FULL_PAGE,
        UiInstallationContentKind::UI_INSTALLATION_CONTENT_KIND_STATIC,
        "reference",
        Some(&[
            "heph-ui-kit-v1.0.0.css",
            "heph-ui-kit-v1.0.0.js",
            "index.html",
        ]),
        None,
        None,
    )?;
    validate_reference_ui_descriptor(
        &static_release,
        "release-reference-repository",
        ReleaseUiScope::RELEASE_UI_SCOPE_REPOSITORY,
        ReleaseUiPresentation::RELEASE_UI_PRESENTATION_FULL_PAGE,
        UiInstallationContentKind::UI_INSTALLATION_CONTENT_KIND_STATIC,
        "reference-repository",
        Some(&[
            "heph-ui-kit-v1.0.0.css",
            "heph-ui-kit-v1.0.0.js",
            "index.html",
        ]),
        None,
        None,
    )?;
    validate_reference_ui_descriptor(
        &static_release,
        "release-reference-global",
        ReleaseUiScope::RELEASE_UI_SCOPE_GLOBAL,
        ReleaseUiPresentation::RELEASE_UI_PRESENTATION_FULL_PAGE,
        UiInstallationContentKind::UI_INSTALLATION_CONTENT_KIND_STATIC,
        "reference-global",
        Some(&[
            "heph-ui-kit-v1.0.0.css",
            "heph-ui-kit-v1.0.0.js",
            "index.html",
        ]),
        None,
        None,
    )?;
    let managed_release = get_published_release(context, managed_build.release_id).await?;
    validate_reference_ui_descriptor(
        &managed_release,
        "managed-reference",
        ReleaseUiScope::RELEASE_UI_SCOPE_PROJECT,
        ReleaseUiPresentation::RELEASE_UI_PRESENTATION_IFRAME,
        UiInstallationContentKind::UI_INSTALLATION_CONTENT_KIND_MANAGED_SERVICE,
        "managed-reference",
        None,
        Some(("cooking-reference-service-ui", "/reference")),
        Some(&[
            "/reference/header-policy",
            "/reference/identity",
            "/reference/probe-location",
            "/reference/probe-refresh",
            "/reference/probe-set-cookie",
        ]),
    )?;

    // The managed descriptor's gateway declaration is installed through the
    // existing authenticated gateway API.  No gateway or instance rows are
    // fabricated by this UI fixture helper.
    let installed_gateway = install_cooking_gateway(
        context,
        managed_build.release_id,
        managed_build.repository_id,
    )
    .await?;
    wait_for_cooking_gateway_active(context, installed_gateway).await?;

    let static_ui = install_reference_ui(
        context,
        organization_id,
        static_build.release_id,
        "release-reference",
        "static",
        project_ui_target(context.project_id),
    )
    .await?;
    let repository_static_ui = install_reference_ui(
        context,
        organization_id,
        static_build.release_id,
        "release-reference-repository",
        "repository-static",
        repository_ui_target(static_build.repository_id),
    )
    .await?;
    let global_static_ui = install_reference_ui(
        context,
        organization_id,
        static_build.release_id,
        "release-reference-global",
        "global-static",
        global_ui_target(),
    )
    .await?;
    let managed_ui = install_reference_ui(
        context,
        organization_id,
        managed_build.release_id,
        "managed-reference",
        "managed",
        project_ui_target(context.project_id),
    )
    .await?;
    let listed = list_reference_uis(
        context,
        organization_id,
        static_build.release_id,
        managed_build.release_id,
        static_build.repository_id,
        static_ui,
        repository_static_ui,
        global_static_ui,
        managed_ui,
    )
    .await?;

    assert_reference_ui_installation_rows(
        context,
        organization_id,
        context.project_id,
        static_build.repository_id,
        listed,
    )
    .await?;

    Ok(InstalledCookingReferenceUis {
        organization_id,
        project_id: context.project_id,
        repository_id: static_build.repository_id,
        managed_gateway_id: installed_gateway.gateway_id,
        managed_gateway_revision_id: installed_gateway.revision_id,
        managed_release_id: managed_build.release_id,
        static_ui: listed.0,
        repository_static_ui: listed.1,
        global_static_ui: listed.2,
        managed_ui: listed.3,
    })
}
