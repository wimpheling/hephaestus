//! Exact frozen private28 inputs; genuine fixture loads the first16 in order.
use sqlx::PgPool;

const SOURCES: [(&str, &str); 28] = [
    (
        "/var/tmp/0108_run_volume_leases.profile.sql",
        "359f2192a87cbcab91576c260c6c043c263f0d9c43aaaefee3e4f77423e0224b",
    ),
    (
        "/var/tmp/0108_run_cleanup_fragment.profile.sql",
        "1783cc49e81b0ed042829df640e1f7092f5830dc77b0d2edb4693d708c73be8a",
    ),
    (
        "/var/tmp/0108_legacy_scalar_compatibility.sql",
        "0f91300933fe4a15ba4e6fa509ec66d9a43839c3f3881094a83788961c3f15f0",
    ),
    (
        "/var/tmp/0108_run_profile_lifecycle.sql",
        "d3232b6bf068376ca39fe4abd6de1b46941ff36d9d67edff8971a8a076f9f320",
    ),
    (
        "/var/tmp/0108_legacy_vm_placement.sql",
        "891a8a08079bfc033d5360ca07a894619f86a6bd91ef9c3af10f72d6b34ed795",
    ),
    (
        "/var/tmp/heph-recipe-0109_external_observation_retirement.sql",
        "831ce0fde3f50006f41eef546513954b807c3657174827f4e3f56c774a562f1f",
    ),
    (
        "/var/tmp/0110_owned_volume_registration.sql",
        "17d8779d5a714d3dee760ea6313f91fe5389e0db9306dc8fa215094b216f4e40",
    ),
    (
        "/var/tmp/0111_owned_volume_backing.sql",
        "d70f9a4b4f13a311edc1e1683f7964998b83553111e8449401d59fb5edd2a2a9",
    ),
    (
        "/var/tmp/0112_owned_partial_retention.atomic.sql",
        "b237fe5f5aadd2c1c8e66d6b0da9a38d0de3bef8cfdb999a1ef365f80040270e",
    ),
    (
        "/var/tmp/0113_deployment_execution_preparation.sql",
        "168064715f3a40297acd324b322a937f1acad2834345e4906ddd20b48afcda25",
    ),
    (
        "/var/tmp/0113_owned_first_ready_lease_exclusion.sql",
        "1ea5bffa139bdcf2d906b730ad02177b530806f34974e106d0e779e0cc687088",
    ),
    (
        "/var/tmp/0114_qualified_instance_activation.corrected.sql",
        "6c70cc50ba4894845eb1b69d540aef91826e2541c76f9badce1b3c8304d3bd61",
    ),
    (
        "/var/tmp/0115_qualified_instance_invocation.sql",
        "57a1e5cdb68b7dfae3b3e7e09bea48ea894ec26dcfc41aa40b3c40648b0a9193",
    ),
    (
        "/var/tmp/0116_owned_ready_retention.sql",
        "e15aff0c14d9ae5e2e585f4b781acc7c0259224ebbc6f6332c1510961aa054d9",
    ),
    (
        "/var/tmp/0117_qualified_invocation_consumption.nowait.sql",
        "4d6fb43f06737ec28b8a078aac896387ae9462b4601d9668e8f85a7ac92b0183",
    ),
    (
        "/var/tmp/0119_qualified_invocation_leases.sql",
        "1e8cfbdb81b9eede747442434ee96103e33ed18641d6ea1d35b50f9fd5eaf0f4",
    ),
    (
        "/var/tmp/0119_qualified_invocation_authority.sql",
        "0d116862074139fd0de88518e4952bb0a70fb5167259c55625e0c09752555bc9",
    ),
    (
        "/var/tmp/0119_qualified_invocation_empty_secret.sql",
        "ead5c2814cddf4c59045e1223b20da1e35bc41396231e13c4518deab6fd9acf3",
    ),
    (
        "/var/tmp/0118_prepared_instance_safety.sql",
        "9d3012d3c716743f43d9e8ed98c68d6f67ecda89ecb61a7869bea7cd9b10abb9",
    ),
    (
        "/var/tmp/0118_prepared_instance_closure.sql",
        "19e731c25e71b148c487f01222948d41c9cc57b60e3f93baaca7265ae80d553e",
    ),
    (
        "/var/tmp/0118_qualified_remove_resume.sql",
        "a262644ec57b5096cd02fd720d96f85fa74a3f7ccae2393cf062a2b1e843f6c9",
    ),
    (
        "/var/tmp/0118_existing_instance_closure.sql",
        "991a24e80494f9aaac0a70fdfee68174657808001e478d69ee2131bc89e0e71c",
    ),
    (
        "/var/tmp/0118_run_cleanup_wake_qualification.sql",
        "fb699a684129798319b333c1ec0a31a100f395818edb2cf9055a5cf73b2f375f",
    ),
    (
        "/var/tmp/0119_qualified_invocation_remove_resume.sql",
        "0fea3368d4fe7d7312b7886d7de45ba5bc4be23a80021dad0ab6a518cc694671",
    ),
    (
        "/var/tmp/0119_queued_invocation_cleanup.sql",
        "14e83b1746041802a923eb9698b036f0dcf5439db150f405e530695c0152eb8d",
    ),
    (
        "/var/tmp/0119_queued_invocation_cancel_delivery.sql",
        "931f675e922b14b796282733c316adc11e9dbb5332a144413843cd49c5773077",
    ),
    (
        "/var/tmp/0119_queued_invocation_cleanup_context.sql",
        "c2b827cc14e776b07dbec223f4c8ccc8eaa610ec12176c417336724b37c0fdb3",
    ),
    (
        "/var/tmp/0119_queued_invocation_cleanup_transaction.sql",
        "ff4d9809d0915e036e4043eb639da06e7608060e3a3776d8ab49662b6a573db5",
    ),
];

pub fn assert_pins() {
    for (path, expected) in SOURCES {
        let bytes = std::fs::read(path).unwrap();
        let actual = release_domain::ContentHash::digest(&bytes);
        let expected = (0..32)
            .map(|index| u8::from_str_radix(&expected[index * 2..index * 2 + 2], 16).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            actual.as_bytes().as_slice(),
            expected,
            "immutable source {path}"
        );
    }
}

pub async fn remaining(pool: &PgPool) {
    assert_pins();
    // Bootstrap + schema::upgrade(119) loaded exact original1-16, without reruns.
    for (path, _) in &SOURCES[16..] {
        sqlx::raw_sql(&std::fs::read_to_string(path).unwrap())
            .execute(pool)
            .await
            .unwrap();
    }
}
