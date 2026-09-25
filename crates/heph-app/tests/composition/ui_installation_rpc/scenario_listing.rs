use super::{
    ui_installation_scenario_state::{InstallScenarioState, ReleaseClient},
    ui_installation_seed::Fixture,
    ui_installation_transport::{
        LIST_AUDIENCE, authorization, opaque, organization_target, session_token,
    },
};
use connectrpc::error::ErrorCode;
use rpc_proto::messages::hephaestus::common::v1::PageRequest;
use rpc_proto::messages::hephaestus::release::v1::ListUiInstallationsRequest;

pub(super) async fn run(release: &ReleaseClient, fixture: &Fixture, state: &InstallScenarioState) {
    let list_token = state.list_token.clone();
    let target = state.target.clone();
    list_project(release, &list_token, fixture, target.clone()).await;
    let cursor = list_organization(release, &list_token, fixture).await;
    assert_cursor_scope(release, &list_token, fixture, target, cursor).await;
}

async fn list_project(
    release: &ReleaseClient,
    list_token: &str,
    fixture: &Fixture,
    target: rpc_proto::messages::hephaestus::release::v1::UiInstallationTarget,
) {
    let list_request = ListUiInstallationsRequest {
        organization_id: opaque(fixture.organization_id).into(),
        target: Some(target).into(),
        page: PageRequest {
            page_size: 50,
            ..Default::default()
        }
        .into(),
        ..Default::default()
    };
    let listed = release
        .list_ui_installations_with_options(list_request.clone(), authorization(list_token))
        .await
        .expect("list installed UI")
        .into_owned();
    assert_eq!(listed.installations.len(), 1);
    assert!(listed.installations[0].launchable);
    assert!(listed.page.as_option().is_some());
    let tampered_cursor = release
        .list_ui_installations_with_options(
            ListUiInstallationsRequest {
                page: PageRequest {
                    page_token: String::from("tampered-cursor"),
                    ..Default::default()
                }
                .into(),
                ..list_request
            },
            authorization(list_token),
        )
        .await
        .expect_err("tampered installation cursor must be rejected");
    assert_eq!(tampered_cursor.code, ErrorCode::InvalidArgument);
}

async fn list_organization(release: &ReleaseClient, list_token: &str, fixture: &Fixture) -> String {
    let organization_list_request = organization_list_request(fixture);
    let organization_page = release
        .list_ui_installations_with_options(organization_list_request, authorization(list_token))
        .await
        .expect("list organization UI installations")
        .into_owned();
    assert_eq!(organization_page.installations.len(), 1);
    let cursor = organization_page
        .page
        .as_option()
        .expect("organization page response")
        .next_page_token
        .clone();
    assert!(
        !cursor.is_empty(),
        "organization list must produce a cursor"
    );
    cursor
}

fn organization_list_request(fixture: &Fixture) -> ListUiInstallationsRequest {
    ListUiInstallationsRequest {
        organization_id: opaque(fixture.organization_id).into(),
        target: Some(organization_target()).into(),
        page: PageRequest {
            page_size: 1,
            ..Default::default()
        }
        .into(),
        ..Default::default()
    }
}

async fn assert_cursor_scope(
    release: &ReleaseClient,
    list_token: &str,
    fixture: &Fixture,
    target: rpc_proto::messages::hephaestus::release::v1::UiInstallationTarget,
    cursor: String,
) {
    let second_actor_cursor = release
        .list_ui_installations_with_options(
            ListUiInstallationsRequest {
                page: PageRequest {
                    page_token: cursor.clone(),
                    ..Default::default()
                }
                .into(),
                ..organization_list_request(fixture)
            },
            authorization(&session_token(
                fixture.second_user_id,
                fixture.second_sid,
                LIST_AUDIENCE,
            )),
        )
        .await
        .expect_err("cursor must reject a different actor");
    assert_eq!(second_actor_cursor.code, ErrorCode::InvalidArgument);
    let second_organization_cursor = release
        .list_ui_installations_with_options(
            ListUiInstallationsRequest {
                organization_id: opaque(fixture.foreign_organization_id).into(),
                target: Some(organization_target()).into(),
                page: PageRequest {
                    page_token: cursor.clone(),
                    ..Default::default()
                }
                .into(),
                ..Default::default()
            },
            authorization(list_token),
        )
        .await
        .expect_err("cursor must reject a different organization");
    assert_eq!(second_organization_cursor.code, ErrorCode::InvalidArgument);
    let second_target_cursor = release
        .list_ui_installations_with_options(
            ListUiInstallationsRequest {
                target: Some(target).into(),
                page: PageRequest {
                    page_token: cursor,
                    ..Default::default()
                }
                .into(),
                ..organization_list_request(fixture)
            },
            authorization(list_token),
        )
        .await
        .expect_err("cursor must reject a different target");
    assert_eq!(second_target_cursor.code, ErrorCode::InvalidArgument);
}
