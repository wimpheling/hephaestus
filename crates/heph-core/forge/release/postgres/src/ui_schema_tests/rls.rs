use super::{
    fixtures::ReleaseFixture,
    support::{assert_sqlstate, role_pool},
};

pub async fn assert_rls_isolation(url: &str, fixture: &ReleaseFixture) {
    let owner = role_pool(url, "hephaestus_app", fixture.owner).await;
    let visible: (i64, i64, i64, i64, i64) = sqlx::query_as(
        "SELECT
            (SELECT count(*) FROM release_ui_source_snapshots WHERE release_id = $1),
            (SELECT count(*) FROM release_ui_descriptors WHERE release_id = $1),
            (SELECT count(*) FROM release_ui_static_files WHERE release_id = $1),
            (SELECT count(*) FROM release_ui_managed_services WHERE release_id = $1),
            (SELECT count(*) FROM release_ui_api_bindings WHERE release_id = $1)",
    )
    .bind(fixture.release)
    .fetch_one(&owner)
    .await
    .expect("authorized release UI read");
    assert_eq!(visible, (1, 3, 1, 1, 1));

    let outsider = role_pool(url, "hephaestus_app", fixture.outsider).await;
    let hidden: (i64, i64, i64, i64, i64) = sqlx::query_as(
        "SELECT
            (SELECT count(*) FROM release_ui_source_snapshots WHERE release_id = $1),
            (SELECT count(*) FROM release_ui_descriptors WHERE release_id = $1),
            (SELECT count(*) FROM release_ui_static_files WHERE release_id = $1),
            (SELECT count(*) FROM release_ui_managed_services WHERE release_id = $1),
            (SELECT count(*) FROM release_ui_api_bindings WHERE release_id = $1)",
    )
    .bind(fixture.release)
    .fetch_one(&outsider)
    .await
    .expect("unauthorized release UI read is filtered");
    assert_eq!(hidden, (0, 0, 0, 0, 0));
    let denied = sqlx::query(
        "INSERT INTO release_ui_descriptors
         (release_id, ui_key, scope, label, icon, presentation, route_base,
          entrypoint, ui_kit_version, cache, content_kind)
         VALUES ($1, 'outsider', 'global', 'Outsider', 'app', 'iframe',
                 'outsider', 'index.html', 1, 'no_store', 'static')",
    )
    .bind(fixture.release)
    .execute(&outsider)
    .await;
    // The draft guard runs before the INSERT policy and sees no release row
    // through outsider RLS, so it fails closed with the same integrity error.
    assert_sqlstate(denied, "23000");
}
