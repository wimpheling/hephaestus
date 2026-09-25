use async_nats::Client;

const PRODUCT_EVENT_SUBJECT: &str = "hephaestus.product.event.v1";

struct EventPublisher;

impl EventPublisher {
    async fn publish_pending(&self, _client: &Client) {}

    async fn publish_elsewhere(&self, client: &Client) {
        client
            .publish(PRODUCT_EVENT_SUBJECT, "typed".into())
            .await
            .unwrap();
    }
}
