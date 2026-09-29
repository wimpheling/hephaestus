// Reuse the parent facade so sibling fixture phases share one boundary context.
#[allow(unused_imports)]
use super::*;
pub(crate) async fn get_published_release(
    context: &CookingBuildContext<'_>,
    release_id: Uuid,
) -> Result<rpc_proto::messages::hephaestus::release::v1::Release, BuildError> {
    let client = rpc_release_client(
        context.running,
        context.identity.rpc_token,
        "/hephaestus.release.v1.ReleaseService/GetRelease",
    )?;
    client
        .get_release(GetReleaseRequest {
            release_id: opaque(release_id).into(),
            ..Default::default()
        })
        .await?
        .into_owned()
        .release
        .into_option()
        .ok_or_else(|| invalid_state("GetRelease returned no published release"))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn validate_reference_ui_descriptor(
    release: &rpc_proto::messages::hephaestus::release::v1::Release,
    expected_key: &str,
    expected_scope: ReleaseUiScope,
    expected_presentation: ReleaseUiPresentation,
    expected_kind: UiInstallationContentKind,
    expected_route_base: &str,
    expected_static_routes: Option<&[&str]>,
    expected_managed: Option<(&str, &str)>,
    expected_api_routes: Option<&[&str]>,
) -> Result<(), BuildError> {
    let descriptor = release
        .ui_descriptors
        .iter()
        .find(|descriptor| descriptor.key == expected_key)
        .ok_or_else(|| invalid_state(&format!("published release omitted UI {expected_key}")))?;
    if descriptor.scope.to_i32() != expected_scope as i32
        || descriptor.presentation.to_i32() != expected_presentation as i32
        || descriptor.route_base != expected_route_base
        || descriptor.entrypoint != "index.html"
    {
        return Err(invalid_state(&format!(
            "published UI {expected_key} has unexpected scope/presentation/route metadata"
        )));
    }
    match (expected_kind, descriptor.content.as_ref()) {
        (
            UiInstallationContentKind::UI_INSTALLATION_CONTENT_KIND_STATIC,
            Some(release_ui_descriptor::Content::StaticContent(content)),
        ) => {
            let Some(expected_routes) = expected_static_routes else {
                return Err(invalid_state("static reference UI has no expected routes"));
            };
            let actual_routes = content
                .files
                .iter()
                .map(|file| file.route.as_str())
                .collect::<Vec<_>>();
            if actual_routes != expected_routes {
                return Err(invalid_state(&format!(
                    "static UI {expected_key} files differ from checked-in declaration"
                )));
            }
        }
        (
            UiInstallationContentKind::UI_INSTALLATION_CONTENT_KIND_MANAGED_SERVICE,
            Some(release_ui_descriptor::Content::ManagedService(content)),
        ) => {
            let Some((expected_gateway, expected_route)) = expected_managed else {
                return Err(invalid_state(
                    "managed reference UI has no gateway expectation",
                ));
            };
            if content.gateway_name != expected_gateway || content.route != expected_route {
                return Err(invalid_state(&format!(
                    "managed UI {expected_key} gateway route differs from checked-in declaration"
                )));
            }
        }
        _ => {
            return Err(invalid_state(&format!(
                "published UI {expected_key} has unexpected content kind"
            )));
        }
    }
    if let Some(expected_routes) = expected_api_routes {
        let actual_routes = descriptor
            .apis
            .iter()
            .map(|api| api.route.as_str())
            .collect::<Vec<_>>();
        if actual_routes != expected_routes {
            return Err(invalid_state(&format!(
                "managed UI {expected_key} APIs differ from checked-in declaration"
            )));
        }
    }
    Ok(())
}
