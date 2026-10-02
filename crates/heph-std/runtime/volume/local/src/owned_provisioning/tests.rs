//! Real host filesystem tests; injected memory ports make no `PostgreSQL` claims.

mod cancellation;
mod legacy;
mod metadata;
mod recovery;
mod safety;
mod support;

use support::Fixture;

#[tokio::test]
async fn native_first_birth_records_exact_clean_filesystem_once() {
    let fixture = Fixture::new();
    let context = fixture
        .store
        .provision_owned(&fixture.identity, &fixture.request)
        .await
        .expect("real sealed birth");
    let observation = context.observation.expect("physical proof");
    assert_eq!(
        observation.fields().phase,
        volume_trait::OwnedBackingPhase::Ready
    );
    assert_eq!(
        observation
            .fields()
            .filesystem
            .as_ref()
            .expect("ext4")
            .filesystem_uuid,
        fixture
            .request
            .purpose
            .receipt()
            .intent
            .registration()
            .filesystem_uuid()
    );
    assert_eq!(fixture.format_count(), 1);
    let bytes = std::fs::read(fixture.canonical()).expect("bytes");
    let repeated = fixture
        .store
        .provision_owned(&fixture.identity, &fixture.request)
        .await
        .expect("fresh authorized historical replay");
    assert_eq!(repeated.observation, Some(observation));
    assert_eq!(fixture.format_count(), 1);
    assert_eq!(std::fs::read(fixture.canonical()).expect("retained"), bytes);
}
