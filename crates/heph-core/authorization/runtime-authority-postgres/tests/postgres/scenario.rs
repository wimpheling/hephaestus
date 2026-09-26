use serial_test::serial;

use super::{guest, host, race, support};

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn gateway_host_mediated_sessions_are_distinct_and_lifecycle_bound() {
    let Some(context) = support::setup().await else {
        return;
    };
    let (_service, service_request) = host::exercise(&context).await;
    race::exercise(&context, &service_request).await;
    guest::exercise(&context).await;
}
