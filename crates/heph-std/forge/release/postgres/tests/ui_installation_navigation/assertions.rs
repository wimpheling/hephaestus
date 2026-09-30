//! Application-role navigation and authorization assertions.

use identity_domain::UserId;
use identity_domain::{AuthenticatedIdentity, RequestId};
use release_domain::UiInstallationState;
use release_postgres::PgUiInstallationNavigator;
use release_service::{
    ListUiInstallations, UiInstallationNavigator, UiInstallationPage, UiInstallationTargetFilter,
};
use serde_json::json;
use sqlx::PgPool;

use super::seed::Seeded;

#[allow(clippy::too_many_lines)] // Keep the complete target, cursor, and visibility matrix together.
#[allow(clippy::cognitive_complexity)] // Keep each authorization transition adjacent to its observable projection.
pub(super) async fn verify(bootstrap: &PgPool, app_pool: &PgPool, fixture: &Seeded) {
    let Seeded {
        actor,
        organization,
        project,
        repository,
        source_project,
        second_organization,
        release,
    } = fixture;
    let navigator = PgUiInstallationNavigator::new(app_pool.clone());
    let identity = identity(*actor);
    let invalid_zero_page = navigator
        .list_ui_installations(
            &identity,
            ListUiInstallations {
                organization_id: *organization,
                target: UiInstallationTargetFilter::Project(*project),
                page: UiInstallationPage {
                    size: 0,
                    after: None,
                },
            },
        )
        .await;
    assert!(matches!(
        invalid_zero_page,
        Err(release_service::UiInstallationNavigationError::InvalidPage)
    ));
    let invalid_large_page = navigator
        .list_ui_installations(
            &identity,
            ListUiInstallations {
                organization_id: *organization,
                target: UiInstallationTargetFilter::Project(*project),
                page: UiInstallationPage {
                    size: 101,
                    after: None,
                },
            },
        )
        .await;
    assert!(matches!(
        invalid_large_page,
        Err(release_service::UiInstallationNavigationError::InvalidPage)
    ));
    let project_page = navigator
        .list_ui_installations(
            &identity,
            ListUiInstallations {
                organization_id: *organization,
                target: UiInstallationTargetFilter::Project(*project),
                page: UiInstallationPage {
                    size: 1,
                    after: None,
                },
            },
        )
        .await
        .expect("first project navigation page");
    assert_eq!(project_page.installations.len(), 1);
    assert_eq!(
        project_page.installations[0].lifecycle,
        UiInstallationState::Enabled
    );
    let project_second_page = navigator
        .list_ui_installations(
            &identity,
            ListUiInstallations {
                organization_id: *organization,
                target: UiInstallationTargetFilter::Project(*project),
                page: UiInstallationPage {
                    size: 1,
                    after: project_page.next,
                },
            },
        )
        .await
        .expect("second project navigation page");
    assert_eq!(project_second_page.installations.len(), 1);
    assert!(project_second_page.next.is_none());
    assert!(
        [
            project_page.installations[0].ui_key.as_str(),
            project_second_page.installations[0].ui_key.as_str(),
        ]
        .into_iter()
        .all(|key| key != "project-ui")
    );

    let repository_page = navigator
        .list_ui_installations(
            &identity,
            ListUiInstallations {
                organization_id: *organization,
                target: UiInstallationTargetFilter::Repository(*repository),
                page: UiInstallationPage::default(),
            },
        )
        .await
        .expect("repository navigation projection");
    assert_eq!(repository_page.installations.len(), 1);
    assert_eq!(
        repository_page.installations[0].lifecycle,
        UiInstallationState::Enabled
    );
    assert!(repository_page.installations[0].launchable);

    let global_page = navigator
        .list_ui_installations(
            &identity,
            ListUiInstallations {
                organization_id: *organization,
                target: UiInstallationTargetFilter::Global,
                page: UiInstallationPage::default(),
            },
        )
        .await
        .expect("global navigation projection");
    assert_eq!(global_page.installations.len(), 1);
    assert_eq!(
        global_page.installations[0].lifecycle,
        UiInstallationState::Disabled
    );
    assert!(!global_page.installations[0].launchable);

    let second_global_page = navigator
        .list_ui_installations(
            &identity,
            ListUiInstallations {
                organization_id: *second_organization,
                target: UiInstallationTargetFilter::Global,
                page: UiInstallationPage::default(),
            },
        )
        .await
        .expect("second organization navigation projection");
    assert_eq!(second_global_page.installations.len(), 1);
    assert_eq!(
        second_global_page.installations[0].lifecycle,
        UiInstallationState::Enabled
    );

    let foreign_target_page = navigator
        .list_ui_installations(
            &identity,
            ListUiInstallations {
                organization_id: *second_organization,
                target: UiInstallationTargetFilter::Repository(*repository),
                page: UiInstallationPage::default(),
            },
        )
        .await;
    assert!(matches!(
        foreign_target_page,
        Err(release_service::UiInstallationNavigationError::InvalidPage)
    ));

    let foreign_cursor = navigator
        .list_ui_installations(
            &identity,
            ListUiInstallations {
                organization_id: *organization,
                target: UiInstallationTargetFilter::Repository(*repository),
                page: UiInstallationPage {
                    size: 1,
                    after: Some(global_page.installations[0].installation_id),
                },
            },
        )
        .await;
    assert!(matches!(
        foreign_cursor,
        Err(release_service::UiInstallationNavigationError::InvalidPage)
    ));

    sqlx::query("UPDATE releases SET state = 'revoked', revoked_at = now() WHERE id = $1")
        .bind(release.as_uuid())
        .execute(bootstrap)
        .await
        .expect("revoke source release use permission");
    let source_use_revoked = navigator
        .list_ui_installations(
            &identity,
            ListUiInstallations {
                organization_id: *organization,
                target: UiInstallationTargetFilter::Repository(*repository),
                page: UiInstallationPage::default(),
            },
        )
        .await
        .expect("source use revocation remains a safe projection");
    assert_eq!(source_use_revoked.installations.len(), 1);
    assert!(!source_use_revoked.installations[0].launchable);

    sqlx::query("DELETE FROM project_maintainers WHERE project_id = $1 AND user_id = $2")
        .bind(source_project.as_uuid())
        .bind(actor.as_uuid())
        .execute(bootstrap)
        .await
        .expect("revoke source project read permission");
    sqlx::query("DELETE FROM organization_members WHERE organization_id = $1 AND user_id = $2")
        .bind(organization.as_uuid())
        .bind(actor.as_uuid())
        .execute(bootstrap)
        .await
        .expect("revoke current organization read permission");
    let revoked_page = navigator
        .list_ui_installations(
            &identity,
            ListUiInstallations {
                organization_id: *organization,
                target: UiInstallationTargetFilter::Repository(*repository),
                page: UiInstallationPage::default(),
            },
        )
        .await
        .expect("revoked navigation remains a successful empty read");
    assert!(revoked_page.installations.is_empty());
    let other_org_remains_visible = navigator
        .list_ui_installations(
            &identity,
            ListUiInstallations {
                organization_id: *second_organization,
                target: UiInstallationTargetFilter::Global,
                page: UiInstallationPage::default(),
            },
        )
        .await
        .expect("other organization remains independently visible");
    assert_eq!(other_org_remains_visible.installations.len(), 1);
    println!(
        "REAL_UI_NAVIGATION=1 app_role=1 explicit_targets=1 project_repo_global=1 dual_org=1 pagination=1 invalid_pages=1 removed_excluded=1 source_use_launchable_false=1 source_read_hidden=1 owner_revocation_other_org_visible=1"
    );
}

fn identity(user_id: UserId) -> AuthenticatedIdentity {
    AuthenticatedIdentity::new(
        user_id,
        "https://ui-navigation-test.example",
        format!("subject-{user_id}"),
        json!({"email_verified": true}),
        RequestId::new(),
    )
}
