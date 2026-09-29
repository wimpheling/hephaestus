use serial_test::serial;
use std::env;

use super::acceptance_recovery::run as run_recovery;
use super::acceptance_setup::setup;
use super::acceptance_transport::run as run_transport;

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn acceptance_deduplication_outbox_jetstream_redelivery_and_recovery_are_durable() {
    let (Ok(database_url), Ok(nats_url)) = (
        env::var("HEPHAESTUS_POSTGRES_TEST_URL"),
        env::var("HEPHAESTUS_NATS_TEST_URL"),
    ) else {
        return;
    };
    let state = setup(&database_url).await;
    let run_id = run_transport(&state, &nats_url).await;
    run_recovery(&state, run_id).await;
}
