use async_nats::Client;

async fn mutate_and_publish(client: &Client) {
    client.publish("product.changed", "{}".into()).await.unwrap();
}
