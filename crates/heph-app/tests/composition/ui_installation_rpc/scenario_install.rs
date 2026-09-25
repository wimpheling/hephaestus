use super::{
    ui_installation_scenario_state::{InstallScenarioState, ReleaseClient},
    ui_installation_seed::Fixture,
    ui_installation_transport::{
        ACTIVATE_AUDIENCE, LIST_AUDIENCE, UI_AUDIENCE, assert_receipt_scope, authorization, opaque,
        organization_target, project_target, repository_target, request_context, session_token,
        session_token_for_lifetime,
    },
};
use connectrpc::error::ErrorCode;
use identity_domain::BrowserSessionSid;
use rpc_proto::messages::hephaestus::release::v1::{ActivateUiRequest, InstallUiRequest};
use sqlx::PgPool;

pub(crate) async fn run(
    pool: &PgPool,
    release: &ReleaseClient,
    fixture: &Fixture,
) -> InstallScenarioState {
    let target = project_target(fixture.project_id);
    let install_token = session_token(fixture.user_id, fixture.sid, UI_AUDIENCE);
    let install_request = InstallUiRequest {
        context: request_context("ui-install").into(),
        organization_id: opaque(fixture.organization_id).into(),
        target: Some(target.clone()).into(),
        release_id: opaque(fixture.release_id).into(),
        ui_key: String::from("assistant"),
        ..Default::default()
    };
    let wrong_audience = release
        .install_ui_with_options(
            install_request.clone(),
            authorization(&session_token(fixture.user_id, fixture.sid, LIST_AUDIENCE)),
        )
        .await
        .expect_err("wrong procedure audience must be rejected");
    assert_eq!(wrong_audience.code, ErrorCode::Unauthenticated);
    let expired = release
        .install_ui_with_options(
            install_request.clone(),
            authorization(&session_token_for_lifetime(
                fixture.user_id,
                fixture.sid,
                UI_AUDIENCE,
                -1,
            )),
        )
        .await
        .expect_err("expired parent assertion must be rejected");
    assert_eq!(expired.code, ErrorCode::Unauthenticated);
    let inactive_parent = release
        .install_ui_with_options(
            install_request.clone(),
            authorization(&session_token(
                fixture.user_id,
                BrowserSessionSid::new(),
                UI_AUDIENCE,
            )),
        )
        .await
        .expect_err("signed assertion without an active parent must be rejected");
    assert_eq!(inactive_parent.code, ErrorCode::Unauthenticated);
    let wrong_organization = release
        .install_ui_with_options(
            InstallUiRequest {
                context: request_context("cross-org-target").into(),
                organization_id: opaque(fixture.foreign_organization_id).into(),
                ..install_request.clone()
            },
            authorization(&session_token(fixture.user_id, fixture.sid, UI_AUDIENCE)),
        )
        .await
        .expect_err("target organization mismatch must be rejected");
    assert_eq!(wrong_organization.code, ErrorCode::InvalidArgument);
    let denied_installations: i64 = sqlx::query_scalar("SELECT count(*) FROM ui_installations")
        .fetch_one(pool)
        .await
        .expect("count installations after denied requests");
    assert_eq!(denied_installations, 0);
    let installed = release
        .install_ui_with_options(install_request.clone(), authorization(&install_token))
        .await
        .expect("authenticated UI install")
        .into_owned();
    let receipt = installed.receipt.as_option().expect("install receipt");
    assert_receipt_scope(pool, receipt, "project", "project").await;
    let replay = release
        .install_ui_with_options(install_request, authorization(&install_token))
        .await
        .expect("exact install replay")
        .into_owned();
    assert_eq!(replay.installation_id, installed.installation_id);
    assert_eq!(replay.generation_id, installed.generation_id);
    assert_eq!(replay.receipt, installed.receipt);

    let organization = organization_target();
    let organization_installed = release
        .install_ui_with_options(
            InstallUiRequest {
                context: request_context("ui-install-organization").into(),
                organization_id: opaque(fixture.organization_id).into(),
                target: Some(organization.clone()).into(),
                release_id: opaque(fixture.global_release_id).into(),
                ui_key: String::from("assistant"),
                ..Default::default()
            },
            authorization(&install_token),
        )
        .await
        .expect("authenticated organization UI install")
        .into_owned();
    assert_receipt_scope(
        pool,
        organization_installed
            .receipt
            .as_option()
            .expect("organization receipt"),
        "organization",
        "organization",
    )
    .await;
    let repository = repository_target(fixture.repository_id);
    let repository_installed = release
        .install_ui_with_options(
            InstallUiRequest {
                context: request_context("ui-install-repository").into(),
                organization_id: opaque(fixture.organization_id).into(),
                target: Some(repository.clone()).into(),
                release_id: opaque(fixture.repository_release_id).into(),
                ui_key: String::from("assistant"),
                ..Default::default()
            },
            authorization(&install_token),
        )
        .await
        .expect("authenticated repository UI install")
        .into_owned();
    assert_receipt_scope(
        pool,
        repository_installed
            .receipt
            .as_option()
            .expect("repository receipt"),
        "repository",
        "project",
    )
    .await;
    let global_console = release
        .install_ui_with_options(
            InstallUiRequest {
                context: request_context("ui-install-global-console").into(),
                organization_id: opaque(fixture.organization_id).into(),
                target: Some(organization.clone()).into(),
                release_id: opaque(fixture.global_release_id).into(),
                ui_key: String::from("console"),
                ..Default::default()
            },
            authorization(&install_token),
        )
        .await
        .expect("second organization UI install for cursor paging")
        .into_owned();
    assert_receipt_scope(
        pool,
        global_console
            .receipt
            .as_option()
            .expect("global console receipt"),
        "organization",
        "organization",
    )
    .await;

    let organization_activated = release
        .activate_ui_with_options(
            ActivateUiRequest {
                context: request_context("ui-activate-organization").into(),
                installation_id: organization_installed.installation_id.clone(),
                expected_generation_id: organization_installed.generation_id.clone(),
                release_id: opaque(fixture.global_release_id).into(),
                ui_key: String::from("assistant"),
                ..Default::default()
            },
            authorization(&session_token(
                fixture.user_id,
                fixture.sid,
                ACTIVATE_AUDIENCE,
            )),
        )
        .await
        .expect("activate organization UI")
        .into_owned();
    assert_receipt_scope(
        pool,
        organization_activated
            .receipt
            .as_option()
            .expect("organization activation receipt"),
        "organization",
        "organization",
    )
    .await;
    let repository_activated = release
        .activate_ui_with_options(
            ActivateUiRequest {
                context: request_context("ui-activate-repository").into(),
                installation_id: repository_installed.installation_id.clone(),
                expected_generation_id: repository_installed.generation_id.clone(),
                release_id: opaque(fixture.repository_release_id).into(),
                ui_key: String::from("assistant"),
                ..Default::default()
            },
            authorization(&session_token(
                fixture.user_id,
                fixture.sid,
                ACTIVATE_AUDIENCE,
            )),
        )
        .await
        .expect("activate repository UI")
        .into_owned();
    assert_receipt_scope(
        pool,
        repository_activated
            .receipt
            .as_option()
            .expect("repository activation receipt"),
        "repository",
        "project",
    )
    .await;
    InstallScenarioState {
        target,
        installed,
        list_token: session_token(
            fixture.user_id,
            fixture.sid,
            super::ui_installation_transport::LIST_AUDIENCE,
        ),
        handoff_token: session_token(
            fixture.user_id,
            fixture.sid,
            super::ui_installation_transport::HANDOFF_AUDIENCE,
        ),
    }
}
