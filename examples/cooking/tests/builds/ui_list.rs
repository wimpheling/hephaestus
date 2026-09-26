// Reuse the parent facade so sibling fixture phases share one boundary context.
#[allow(unused_imports)]
use super::*;

// Each target is intentionally listed independently because the RPC filter is
// exact; keeping the four command results adjacent makes cross-target drift
// visible at the fixture boundary.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn list_reference_uis(
    context: &CookingBuildContext<'_>,
    organization_id: OrganizationId,
    static_release_id: Uuid,
    managed_release_id: Uuid,
    static_repository_id: RepositoryId,
    static_command: InstalledCookingUi,
    repository_static_command: InstalledCookingUi,
    global_static_command: InstalledCookingUi,
    managed_command: InstalledCookingUi,
) -> Result<
    (
        InstalledCookingUi,
        InstalledCookingUi,
        InstalledCookingUi,
        InstalledCookingUi,
    ),
    BuildError,
> {
    let client = rpc_release_client(
        context.running,
        context.identity.rpc_token,
        "/hephaestus.release.v1.ReleaseService/ListUiInstallations",
    )?;
    let static_ui = list_reference_ui_target(
        &client,
        organization_id,
        project_ui_target(context.project_id),
        "release-reference",
        static_release_id,
        static_command,
    )
    .await?;
    let repository_static_ui = list_reference_ui_target(
        &client,
        organization_id,
        repository_ui_target(static_repository_id),
        "release-reference-repository",
        static_release_id,
        repository_static_command,
    )
    .await?;
    let global_static_ui = list_reference_ui_target(
        &client,
        organization_id,
        global_ui_target(),
        "release-reference-global",
        static_release_id,
        global_static_command,
    )
    .await?;
    let managed_ui = list_reference_ui_target(
        &client,
        organization_id,
        project_ui_target(context.project_id),
        "managed-reference",
        managed_release_id,
        managed_command,
    )
    .await?;
    Ok((
        static_ui,
        repository_static_ui,
        global_static_ui,
        managed_ui,
    ))
}

// The target is cloned into the wire request and borrowed for response
// validation; value ownership keeps each exact target call self-contained.
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn list_reference_ui_target(
    client: &ReleaseServiceClient<connectrpc::client::HttpClient>,
    organization_id: OrganizationId,
    target: UiInstallationTarget,
    ui_key: &str,
    release_id: Uuid,
    command: InstalledCookingUi,
) -> Result<InstalledCookingUi, BuildError> {
    let response = client
        .list_ui_installations(ListUiInstallationsRequest {
            organization_id: opaque(organization_id.as_uuid()).into(),
            target: target.clone().into(),
            page: PageRequest {
                page_size: 100,
                ..Default::default()
            }
            .into(),
            ..Default::default()
        })
        .await?
        .into_owned();
    listed_reference_ui(
        &response.installations,
        ui_key,
        release_id,
        organization_id,
        &target,
        command,
    )
}

// The owner matrix is a small, fixed SQL proof for the four returned commands.
#[allow(clippy::too_many_lines)]
pub(crate) async fn assert_reference_ui_installation_rows(
    context: &CookingBuildContext<'_>,
    organization_id: OrganizationId,
    project_id: ProjectId,
    repository_id: RepositoryId,
    listed: (
        InstalledCookingUi,
        InstalledCookingUi,
        InstalledCookingUi,
        InstalledCookingUi,
    ),
) -> Result<(), BuildError> {
    let repository_owner_count: i64 = sqlx::query_scalar(
        "SELECT count(*)
           FROM repositories repository
           JOIN projects project ON project.id = repository.project_id
          WHERE repository.id = $1
            AND repository.project_id = $2
            AND project.organization_id = $3",
    )
    .bind(repository_id.as_uuid())
    .bind(project_id.as_uuid())
    .bind(organization_id.as_uuid())
    .fetch_one(context.pool)
    .await?;
    if repository_owner_count != 1 {
        return Err(invalid_state(
            "repository static UI target is outside the cooking organization/project",
        ));
    }

    let expected = [
        (
            listed.0,
            "project",
            "release-reference",
            None,
            Some(project_id.as_uuid()),
            None,
        ),
        (
            listed.1,
            "repository",
            "release-reference-repository",
            None,
            Some(project_id.as_uuid()),
            Some(repository_id.as_uuid()),
        ),
        (
            listed.2,
            "global",
            "release-reference-global",
            Some(organization_id.as_uuid()),
            None,
            None,
        ),
        (
            listed.3,
            "project",
            "managed-reference",
            None,
            Some(project_id.as_uuid()),
            None,
        ),
    ];
    for (command, scope, ui_key, expected_organization, expected_project, expected_repository) in
        expected
    {
        type InstallationOwnerRow = (
            Option<Uuid>,
            Option<Uuid>,
            Option<Uuid>,
            String,
            String,
            String,
            Uuid,
        );
        let row: Option<InstallationOwnerRow> = sqlx::query_as(
            "SELECT organization_id, project_id, repository_id,
                        scope, ui_key, lifecycle, current_generation_id
                   FROM ui_installations
                  WHERE id = $1",
        )
        .bind(command.installation_id)
        .fetch_optional(context.pool)
        .await?;
        let Some((
            stored_organization,
            stored_project,
            stored_repository,
            stored_scope,
            stored_key,
            lifecycle,
            generation,
        )) = row
        else {
            return Err(invalid_state(
                "ListUiInstallations returned an unknown installation",
            ));
        };
        if stored_organization != expected_organization
            || stored_project != expected_project
            || stored_repository != expected_repository
            || stored_scope != scope
            || stored_key != ui_key
            || lifecycle != "enabled"
            || generation != command.generation_id
        {
            return Err(invalid_state(&format!(
                "stored UI installation owner differs for {ui_key}"
            )));
        }
    }
    Ok(())
}

pub(crate) fn listed_reference_ui(
    installations: &[UiInstallationNavigation],
    ui_key: &str,
    release_id: Uuid,
    organization_id: OrganizationId,
    target: &UiInstallationTarget,
    command: InstalledCookingUi,
) -> Result<InstalledCookingUi, BuildError> {
    let entry = installations
        .iter()
        .find(|entry| {
            entry.ui_key == ui_key
                && entry
                    .release_id
                    .as_option()
                    .is_some_and(|id| id.value == release_id.to_string())
        })
        .ok_or_else(|| invalid_state(&format!("ListUiInstallations omitted {ui_key}")))?;
    if entry
        .organization_id
        .as_option()
        .is_none_or(|id| id.value != organization_id.as_uuid().to_string())
        || !matches_installation_target(entry, target)
        || entry.lifecycle.to_i32()
            != UiInstallationLifecycle::UI_INSTALLATION_LIFECYCLE_ENABLED as i32
        || !entry.launchable
    {
        return Err(invalid_state(&format!(
            "listed {ui_key} is not enabled and launchable for the requested owner"
        )));
    }
    let listed = InstalledCookingUi {
        installation_id: response_id(
            entry.installation_id.as_option().cloned(),
            "ListUiInstallations installation",
        )?,
        generation_id: response_id(
            entry.generation_id.as_option().cloned(),
            "ListUiInstallations generation",
        )?,
    };
    if listed.installation_id != command.installation_id
        || listed.generation_id != command.generation_id
    {
        return Err(invalid_state(&format!(
            "ListUiInstallations changed the fresh {ui_key} command result"
        )));
    }
    Ok(listed)
}

pub(crate) fn project_ui_target(project_id: ProjectId) -> UiInstallationTarget {
    UiInstallationTarget {
        target: Some(ui_installation_target::Target::ProjectId(
            opaque(project_id.as_uuid()).into(),
        )),
        ..Default::default()
    }
}

pub(crate) fn repository_ui_target(repository_id: RepositoryId) -> UiInstallationTarget {
    UiInstallationTarget {
        target: Some(ui_installation_target::Target::RepositoryId(
            opaque(repository_id.as_uuid()).into(),
        )),
        ..Default::default()
    }
}

pub(crate) fn global_ui_target() -> UiInstallationTarget {
    UiInstallationTarget {
        target: Some(ui_installation_target::Target::Global(Box::default())),
        ..Default::default()
    }
}

pub(crate) fn matches_installation_target(
    entry: &UiInstallationNavigation,
    expected: &UiInstallationTarget,
) -> bool {
    let Some(target) = entry.target.as_option() else {
        return false;
    };
    match expected.target.as_ref() {
        Some(ui_installation_target::Target::Global(_)) => {
            matches!(
                target.target.as_ref(),
                Some(ui_installation_target::Target::Global(_))
            )
        }
        Some(ui_installation_target::Target::ProjectId(id)) => matches!(
            target.target.as_ref(),
            Some(ui_installation_target::Target::ProjectId(actual))
                if actual.value == id.value
        ),
        Some(ui_installation_target::Target::RepositoryId(id)) => matches!(
            target.target.as_ref(),
            Some(ui_installation_target::Target::RepositoryId(actual))
                if actual.value == id.value
        ),
        None => false,
    }
}
