use async_nats::Client;

async fn publish_committed_command(client: &Client) {
    client
        .publish("heph.run.command.cancel.v1", "typed".into())
        .await
        .unwrap();
}
