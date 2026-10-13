//! Exact complete prefix, then canonical private17..29; no missing-migration relaxation.
use sha2::{Digest, Sha256};
use sqlx::PgPool;

const INPUTS: &[(&str, &[u8], &str)] = &[
    (
        "/var/tmp/0108_run_volume_leases.profile.sql",
        include_bytes!("/var/tmp/0108_run_volume_leases.profile.sql"),
        "359f2192a87cbcab91576c260c6c043c263f0d9c43aaaefee3e4f77423e0224b",
    ),
    (
        "/var/tmp/0108_run_cleanup_fragment.profile.sql",
        include_bytes!("/var/tmp/0108_run_cleanup_fragment.profile.sql"),
        "1783cc49e81b0ed042829df640e1f7092f5830dc77b0d2edb4693d708c73be8a",
    ),
    (
        "/var/tmp/0108_legacy_scalar_compatibility.sql",
        include_bytes!("/var/tmp/0108_legacy_scalar_compatibility.sql"),
        "0f91300933fe4a15ba4e6fa509ec66d9a43839c3f3881094a83788961c3f15f0",
    ),
    (
        "/var/tmp/0108_run_profile_lifecycle.sql",
        include_bytes!("/var/tmp/0108_run_profile_lifecycle.sql"),
        "d3232b6bf068376ca39fe4abd6de1b46941ff36d9d67edff8971a8a076f9f320",
    ),
    (
        "/var/tmp/0108_legacy_vm_placement.sql",
        include_bytes!("/var/tmp/0108_legacy_vm_placement.sql"),
        "891a8a08079bfc033d5360ca07a894619f86a6bd91ef9c3af10f72d6b34ed795",
    ),
    (
        "/var/tmp/heph-recipe-0109_external_observation_retirement.sql",
        include_bytes!("/var/tmp/heph-recipe-0109_external_observation_retirement.sql"),
        "831ce0fde3f50006f41eef546513954b807c3657174827f4e3f56c774a562f1f",
    ),
    (
        "/var/tmp/0110_owned_volume_registration.sql",
        include_bytes!("/var/tmp/0110_owned_volume_registration.sql"),
        "17d8779d5a714d3dee760ea6313f91fe5389e0db9306dc8fa215094b216f4e40",
    ),
    (
        "/var/tmp/0111_owned_volume_backing.sql",
        include_bytes!("/var/tmp/0111_owned_volume_backing.sql"),
        "d70f9a4b4f13a311edc1e1683f7964998b83553111e8449401d59fb5edd2a2a9",
    ),
    (
        "/var/tmp/0112_owned_partial_retention.atomic.sql",
        include_bytes!("/var/tmp/0112_owned_partial_retention.atomic.sql"),
        "b237fe5f5aadd2c1c8e66d6b0da9a38d0de3bef8cfdb999a1ef365f80040270e",
    ),
    (
        "/var/tmp/0113_deployment_execution_preparation.sql",
        include_bytes!("/var/tmp/0113_deployment_execution_preparation.sql"),
        "168064715f3a40297acd324b322a937f1acad2834345e4906ddd20b48afcda25",
    ),
    (
        "/var/tmp/0113_owned_first_ready_lease_exclusion.sql",
        include_bytes!("/var/tmp/0113_owned_first_ready_lease_exclusion.sql"),
        "1ea5bffa139bdcf2d906b730ad02177b530806f34974e106d0e779e0cc687088",
    ),
    (
        "/var/tmp/0114_qualified_instance_activation.corrected.sql",
        include_bytes!("/var/tmp/0114_qualified_instance_activation.corrected.sql"),
        "6c70cc50ba4894845eb1b69d540aef91826e2541c76f9badce1b3c8304d3bd61",
    ),
    (
        "/var/tmp/0115_qualified_instance_invocation.sql",
        include_bytes!("/var/tmp/0115_qualified_instance_invocation.sql"),
        "57a1e5cdb68b7dfae3b3e7e09bea48ea894ec26dcfc41aa40b3c40648b0a9193",
    ),
    (
        "/var/tmp/0116_owned_ready_retention.sql",
        include_bytes!("/var/tmp/0116_owned_ready_retention.sql"),
        "e15aff0c14d9ae5e2e585f4b781acc7c0259224ebbc6f6332c1510961aa054d9",
    ),
    (
        "/var/tmp/0117_qualified_invocation_consumption.nowait.sql",
        include_bytes!("/var/tmp/0117_qualified_invocation_consumption.nowait.sql"),
        "4d6fb43f06737ec28b8a078aac896387ae9462b4601d9668e8f85a7ac92b0183",
    ),
    (
        "/var/tmp/0119_qualified_invocation_leases.sql",
        include_bytes!("/var/tmp/0119_qualified_invocation_leases.sql"),
        "1e8cfbdb81b9eede747442434ee96103e33ed18641d6ea1d35b50f9fd5eaf0f4",
    ),
    (
        "/var/tmp/0119_qualified_invocation_authority.sql",
        include_bytes!("/var/tmp/0119_qualified_invocation_authority.sql"),
        "0d116862074139fd0de88518e4952bb0a70fb5167259c55625e0c09752555bc9",
    ),
    (
        "/var/tmp/0119_qualified_invocation_empty_secret.sql",
        include_bytes!("/var/tmp/0119_qualified_invocation_empty_secret.sql"),
        "ead5c2814cddf4c59045e1223b20da1e35bc41396231e13c4518deab6fd9acf3",
    ),
    (
        "/var/tmp/0118_prepared_instance_safety.sql",
        include_bytes!("/var/tmp/0118_prepared_instance_safety.sql"),
        "9d3012d3c716743f43d9e8ed98c68d6f67ecda89ecb61a7869bea7cd9b10abb9",
    ),
    (
        "/var/tmp/0118_prepared_instance_closure.sql",
        include_bytes!("/var/tmp/0118_prepared_instance_closure.sql"),
        "19e731c25e71b148c487f01222948d41c9cc57b60e3f93baaca7265ae80d553e",
    ),
    (
        "/var/tmp/0118_qualified_remove_resume.sql",
        include_bytes!("/var/tmp/0118_qualified_remove_resume.sql"),
        "a262644ec57b5096cd02fd720d96f85fa74a3f7ccae2393cf062a2b1e843f6c9",
    ),
    (
        "/var/tmp/0118_existing_instance_closure.sql",
        include_bytes!("/var/tmp/0118_existing_instance_closure.sql"),
        "991a24e80494f9aaac0a70fdfee68174657808001e478d69ee2131bc89e0e71c",
    ),
    (
        "/var/tmp/0118_run_cleanup_wake_qualification.sql",
        include_bytes!("/var/tmp/0118_run_cleanup_wake_qualification.sql"),
        "fb699a684129798319b333c1ec0a31a100f395818edb2cf9055a5cf73b2f375f",
    ),
    (
        "/var/tmp/0119_qualified_invocation_remove_resume.sql",
        include_bytes!("/var/tmp/0119_qualified_invocation_remove_resume.sql"),
        "0fea3368d4fe7d7312b7886d7de45ba5bc4be23a80021dad0ab6a518cc694671",
    ),
    (
        "/var/tmp/0119_queued_invocation_cleanup.sql",
        include_bytes!("/var/tmp/0119_queued_invocation_cleanup.sql"),
        "14e83b1746041802a923eb9698b036f0dcf5439db150f405e530695c0152eb8d",
    ),
    (
        "/var/tmp/0119_queued_invocation_cancel_delivery.sql",
        include_bytes!("/var/tmp/0119_queued_invocation_cancel_delivery.sql"),
        "931f675e922b14b796282733c316adc11e9dbb5332a144413843cd49c5773077",
    ),
    (
        "/var/tmp/0119_queued_invocation_cleanup_context.sql",
        include_bytes!("/var/tmp/0119_queued_invocation_cleanup_context.sql"),
        "c2b827cc14e776b07dbec223f4c8ccc8eaa610ec12176c417336724b37c0fdb3",
    ),
    (
        "/var/tmp/0119_queued_invocation_cleanup_transaction.sql",
        include_bytes!("/var/tmp/0119_queued_invocation_cleanup_transaction.sql"),
        "ff4d9809d0915e036e4043eb639da06e7608060e3a3776d8ab49662b6a573db5",
    ),
    (
        "/var/tmp/0119_qualified_invocation_terminal_success.empty_secret_cleanup.sql",
        include_bytes!(
            "/var/tmp/0119_qualified_invocation_terminal_success.empty_secret_cleanup.sql"
        ),
        "b4a2ea18e5d3df68cb63a16a5e8aca964480c98a9d39680da477271b7306ef8e",
    ),
];
pub fn verify_inputs() {
    assert_ne!(
        std::env::var("HEPHAESTUS_PRIVATE_RECIPE_REGRESSION_HEAD")
            .ok()
            .as_deref(),
        Some("111"),
        "this fixture requires the complete119 prefix"
    );
    for &(path, bytes, digest) in INPUTS {
        assert_eq!(std::fs::read(path).unwrap(), bytes);
        assert_eq!(format!("{:x}", Sha256::digest(bytes)), digest);
    }
}
pub async fn apply(pool: &PgPool) {
    verify_inputs();
    let before: Vec<(i64, Vec<u8>)> =
        sqlx::query_as("SELECT version,checksum FROM _sqlx_migrations ORDER BY version")
            .fetch_all(pool)
            .await
            .unwrap();
    let mut full = crate::preparation_schema::checkpoint(113);
    let mut migrations = full.iter().cloned().collect::<Vec<_>>();
    for (version, index) in [(114, 11), (115, 12), (116, 13), (117, 14), (119, 15)] {
        migrations.push(sqlx::migrate::Migration::new(
            version,
            "private_invocation_consumption".into(),
            sqlx::migrate::MigrationType::Simple,
            std::str::from_utf8(INPUTS[index].1)
                .unwrap()
                .to_owned()
                .into(),
            false,
        ));
    }
    assert_eq!(
        before,
        migrations
            .iter()
            .map(|m| (m.version, m.checksum.to_vec()))
            .collect::<Vec<_>>(),
        "all installed public107/private119 checksums equal the complete frozen prefix"
    );
    full.migrations = migrations.into();
    // DEFAULT keeps ignore_missing=false; this contains every installed migration.
    full.run(pool).await.unwrap();
    let mut tx = pool.begin().await.unwrap();
    for &(_, bytes, _) in &INPUTS[16..] {
        sqlx::raw_sql(std::str::from_utf8(bytes).unwrap())
            .execute(&mut *tx)
            .await
            .unwrap();
    }
    tx.commit().await.unwrap();
    let after: Vec<(i64, Vec<u8>)> =
        sqlx::query_as("SELECT version,checksum FROM _sqlx_migrations ORDER BY version")
            .fetch_all(pool)
            .await
            .unwrap();
    assert_eq!(
        after, before,
        "additive17..29 leaves installed prefix exact"
    );
}
