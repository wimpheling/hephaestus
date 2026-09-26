//! Release UI authorization assertions through the application role.

use authz_postgres::begin_actor_transaction;
use control_plane_postgres::release::{ReleaseApplication, ReleaseError};
use release_domain::ui::UiMediaType;
use sqlx::PgPool;

use super::helpers::identity;
use super::seed::Seeded;

#[allow(clippy::too_many_lines)] // One role-scoped assertion phase covers the complete reader matrix.
#[allow(clippy::cognitive_complexity)] // Keep malformed-row and authorization outcomes adjacent for auditability.
pub(super) async fn verify(app_pool: &PgPool, scenarios: &Seeded) {
    let Seeded {
        owner,
        outsider,
        valid,
        legacy,
        missing_managed,
        non_html,
        overflow,
        file_overflow,
        api_overflow,
        html_artifact,
        agent_id,
        ..
    } = scenarios;
    let (current_user, is_superuser, bypasses_rls): (String, bool, bool) = sqlx::query_as(
        "SELECT current_user, rolsuper, rolbypassrls
         FROM pg_roles WHERE rolname = current_user",
    )
    .fetch_one(app_pool)
    .await
    .expect("inspect application database role");
    assert_eq!(current_user, "hephaestus_app");
    assert!(!is_superuser);
    assert!(!bypasses_rls);

    let application = ReleaseApplication::new(app_pool.clone());
    let owner_identity = identity(*owner);
    let outsider_identity = identity(*outsider);
    let detail = application
        .get_release(&owner_identity, valid.release_id)
        .await
        .expect("owner release UI detail");
    assert_eq!(detail.ui_descriptors.len(), 2);
    assert_eq!(detail.ui_descriptors[0].key.as_str(), "docs");
    assert_eq!(detail.ui_descriptors[1].key.as_str(), "service");
    let control_plane_postgres::release::ui::ReleaseUiContent::Static {
        files: static_files,
    } = &detail.ui_descriptors[0].content
    else {
        panic!("expected static UI");
    };
    assert_eq!(static_files[0].artifact_id, *html_artifact);
    assert_eq!(static_files[0].media_type, UiMediaType::TextHtml);
    let control_plane_postgres::release::ui::ReleaseUiContent::ManagedService {
        release_agent_id: managed,
        ..
    } = &detail.ui_descriptors[1].content
    else {
        panic!("expected managed UI");
    };
    assert_eq!(*managed, *agent_id);
    assert_eq!(detail.ui_descriptors[1].apis[0].key.as_str(), "health");
    assert_eq!(detail.ui_descriptors[1].apis[0].release_agent_id, *agent_id);

    let legacy_detail = application
        .get_release(&owner_identity, legacy.release_id)
        .await
        .expect("legacy release detail");
    assert!(legacy_detail.ui_descriptors.is_empty());
    assert!(matches!(
        application
            .get_release(&outsider_identity, valid.release_id)
            .await,
        Err(ReleaseError::NotFound)
    ));
    assert!(matches!(
        application
            .get_release(&owner_identity, missing_managed.release_id)
            .await,
        Err(ReleaseError::InvalidStoredData)
    ));
    assert!(matches!(
        application
            .get_release(&owner_identity, non_html.release_id)
            .await,
        Err(ReleaseError::InvalidStoredData)
    ));
    assert!(matches!(
        application
            .get_release(&owner_identity, overflow.release_id)
            .await,
        Err(ReleaseError::InvalidStoredData)
    ));
    assert!(matches!(
        application
            .get_release(&owner_identity, file_overflow.release_id)
            .await,
        Err(ReleaseError::InvalidStoredData)
    ));
    assert!(matches!(
        application
            .get_release(&owner_identity, api_overflow.release_id)
            .await,
        Err(ReleaseError::InvalidStoredData)
    ));

    let mut transaction = begin_actor_transaction(app_pool, &owner_identity)
        .await
        .expect("owner actor transaction");
    let visible_owner: i64 =
        sqlx::query_scalar("SELECT count(*) FROM release_ui_descriptors WHERE release_id = $1")
            .bind(valid.release_id)
            .fetch_one(&mut *transaction)
            .await
            .expect("owner application-role UI descriptor visibility");
    assert_eq!(visible_owner, 2);
    transaction
        .rollback()
        .await
        .expect("rollback actor transaction");

    let mut outsider_transaction = begin_actor_transaction(app_pool, &outsider_identity)
        .await
        .expect("outsider actor transaction");
    let visible_outsider: i64 =
        sqlx::query_scalar("SELECT count(*) FROM release_ui_descriptors WHERE release_id = $1")
            .bind(valid.release_id)
            .fetch_one(&mut *outsider_transaction)
            .await
            .expect("outsider application-role UI descriptor visibility");
    assert_eq!(visible_outsider, 0);
    outsider_transaction
        .rollback()
        .await
        .expect("rollback outsider actor transaction");
    app_pool.close().await;
}
