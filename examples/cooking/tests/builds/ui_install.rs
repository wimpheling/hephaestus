// Reuse the parent facade so sibling fixture phases share one boundary context.
#[allow(unused_imports)]
use super::*;
pub(crate) async fn install_reference_ui(
    context: &CookingBuildContext<'_>,
    organization_id: OrganizationId,
    release_id: Uuid,
    ui_key: &str,
    operation: &str,
    target: UiInstallationTarget,
) -> Result<InstalledCookingUi, BuildError> {
    let client = rpc_release_client(
        context.running,
        context.identity.rpc_token,
        "/hephaestus.release.v1.ReleaseService/InstallUi",
    )?;
    let response = client
        .install_ui(InstallUiRequest {
            context: mutation_context(&format!("install-reference-ui-{operation}")).into(),
            organization_id: opaque(organization_id.as_uuid()).into(),
            target: target.into(),
            release_id: opaque(release_id).into(),
            ui_key: ui_key.to_owned(),
            ..Default::default()
        })
        .await?
        .into_owned();
    if response.lifecycle.to_i32()
        != UiInstallationLifecycle::UI_INSTALLATION_LIFECYCLE_ENABLED as i32
    {
        return Err(invalid_state(&format!("InstallUi did not enable {ui_key}")));
    }
    Ok(InstalledCookingUi {
        installation_id: response_id(
            response.installation_id.into_option(),
            "InstallUi installation",
        )?,
        generation_id: response_id(response.generation_id.into_option(), "InstallUi generation")?,
    })
}

/// Disables the managed reference installation through the owner-authorized
/// Release RPC.  The caller supplies the generation observed at installation
/// time so the lifecycle transition remains a real compare-and-set operation.
pub(crate) async fn disable_installed_ui(
    running: &RunningHephaestus,
    token_factory: &(dyn Fn(&str) -> String + Send + Sync),
    installed_ui: InstalledCookingUi,
) -> Result<InstalledCookingUi, BuildError> {
    let client = rpc_release_client(
        running,
        token_factory,
        "/hephaestus.release.v1.ReleaseService/DisableUi",
    )?;
    let response = client
        .disable_ui(DisableUiRequest {
            context: mutation_context("installed-ui-disable-lifecycle").into(),
            installation_id: opaque(installed_ui.installation_id).into(),
            expected_generation_id: opaque(installed_ui.generation_id).into(),
            ..Default::default()
        })
        .await?
        .into_owned();
    if response.lifecycle.to_i32()
        != UiInstallationLifecycle::UI_INSTALLATION_LIFECYCLE_DISABLED as i32
    {
        return Err(invalid_state("DisableUi did not disable the managed UI"));
    }
    let returned_installation_id = response_id(
        response.installation_id.into_option(),
        "DisableUi installation",
    )?;
    let returned_generation_id =
        response_id(response.generation_id.into_option(), "DisableUi generation")?;
    if returned_installation_id != installed_ui.installation_id {
        return Err(invalid_state("DisableUi returned a different installation"));
    }
    if returned_generation_id != installed_ui.generation_id {
        return Err(invalid_state("DisableUi returned a different generation"));
    }
    Ok(InstalledCookingUi {
        installation_id: returned_installation_id,
        generation_id: returned_generation_id,
    })
}

/// Activates a fresh managed-reference generation after a lifecycle disable.
/// The old generation is supplied as the compare-and-set expectation so a
/// concurrent lifecycle change cannot silently relaunch stale UI state.
pub(crate) async fn activate_installed_ui(
    running: &RunningHephaestus,
    token_factory: &(dyn Fn(&str) -> String + Send + Sync),
    installed_ui: InstalledCookingUi,
    release_id: Uuid,
) -> Result<InstalledCookingUi, BuildError> {
    let client = rpc_release_client(
        running,
        token_factory,
        "/hephaestus.release.v1.ReleaseService/ActivateUi",
    )?;
    let response = client
        .activate_ui(ActivateUiRequest {
            context: mutation_context("installed-ui-reactivate-lifecycle").into(),
            installation_id: opaque(installed_ui.installation_id).into(),
            expected_generation_id: opaque(installed_ui.generation_id).into(),
            release_id: opaque(release_id).into(),
            ui_key: String::from("managed-reference"),
            ..Default::default()
        })
        .await?
        .into_owned();
    if response.lifecycle.to_i32()
        != UiInstallationLifecycle::UI_INSTALLATION_LIFECYCLE_ENABLED as i32
    {
        return Err(invalid_state("ActivateUi did not enable the managed UI"));
    }
    let returned_installation_id = response_id(
        response.installation_id.into_option(),
        "ActivateUi installation",
    )?;
    let returned_generation_id = response_id(
        response.generation_id.into_option(),
        "ActivateUi generation",
    )?;
    if returned_installation_id != installed_ui.installation_id {
        return Err(invalid_state(
            "ActivateUi returned a different installation",
        ));
    }
    if returned_generation_id == installed_ui.generation_id {
        return Err(invalid_state("ActivateUi reused the disabled generation"));
    }
    Ok(InstalledCookingUi {
        installation_id: returned_installation_id,
        generation_id: returned_generation_id,
    })
}

/// Removes the reactivated managed reference installation with its current
/// generation as the compare-and-set expectation.
pub(crate) async fn remove_installed_ui(
    running: &RunningHephaestus,
    token_factory: &(dyn Fn(&str) -> String + Send + Sync),
    installed_ui: InstalledCookingUi,
) -> Result<InstalledCookingUi, BuildError> {
    let client = rpc_release_client(
        running,
        token_factory,
        "/hephaestus.release.v1.ReleaseService/RemoveUi",
    )?;
    let response = client
        .remove_ui(RemoveUiRequest {
            context: mutation_context("installed-ui-remove-lifecycle").into(),
            installation_id: opaque(installed_ui.installation_id).into(),
            expected_generation_id: opaque(installed_ui.generation_id).into(),
            ..Default::default()
        })
        .await?
        .into_owned();
    if response.lifecycle.to_i32()
        != UiInstallationLifecycle::UI_INSTALLATION_LIFECYCLE_REMOVED as i32
    {
        return Err(invalid_state("RemoveUi did not remove the managed UI"));
    }
    let returned_installation_id = response_id(
        response.installation_id.into_option(),
        "RemoveUi installation",
    )?;
    let returned_generation_id =
        response_id(response.generation_id.into_option(), "RemoveUi generation")?;
    if returned_installation_id != installed_ui.installation_id {
        return Err(invalid_state("RemoveUi returned a different installation"));
    }
    if returned_generation_id != installed_ui.generation_id {
        return Err(invalid_state("RemoveUi returned a different generation"));
    }
    Ok(InstalledCookingUi {
        installation_id: returned_installation_id,
        generation_id: returned_generation_id,
    })
}
