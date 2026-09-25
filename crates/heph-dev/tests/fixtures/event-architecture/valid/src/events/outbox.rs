use async_nats::Client;

async fn publish_committed_outbox(client: &Client) {
    client.publish("product.changed", "typed".into()).await.unwrap();
}
