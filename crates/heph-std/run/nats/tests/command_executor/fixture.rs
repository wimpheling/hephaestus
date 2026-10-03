use std::time::Duration;

use async_nats::{
    Subscriber,
    jetstream::{
        self,
        consumer::PullConsumer,
        stream::{Config, RetentionPolicy, StorageType},
    },
};
use futures_util::StreamExt;
use run_nats::{CANCEL_RUN_SUBJECT, START_RUN_SUBJECT};
use runtime_types::CommandId;
use tokio::sync::{Mutex, MutexGuard};

// Supported subjects are fixed; serialize these test streams on the dedicated
// disposable account instead of accepting messages from another test fixture.
static ACCOUNT: Mutex<()> = Mutex::const_new(());

pub struct Fixture {
    context: jetstream::Context,
    pub client: async_nats::Client,
    pub consumer: PullConsumer,
    stream: jetstream::stream::Stream,
    name: String,
    _account: MutexGuard<'static, ()>,
}
impl Fixture {
    pub async fn open(ack_wait: Duration) -> Self {
        let account = ACCOUNT.lock().await;
        let url = std::env::var("HEPHAESTUS_NATS_COMMAND_EXECUTOR_TEST_URL")
            .expect("explicit dedicated disposable JetStream URL");
        let client = async_nats::connect(url).await.unwrap();
        let context = jetstream::new(client.clone());
        let name = format!("HEPH_EXECUTOR_TEST_{}", CommandId::new());
        let stream = context
            .create_stream(Config {
                name: name.clone(),
                subjects: vec![START_RUN_SUBJECT.into(), CANCEL_RUN_SUBJECT.into()],
                retention: RetentionPolicy::WorkQueue,
                storage: StorageType::Memory,
                ..Default::default()
            })
            .await
            .unwrap();
        let consumer = stream
            .create_consumer(jetstream::consumer::pull::Config {
                durable_name: Some("controlled-executor".into()),
                ack_wait,
                ..Default::default()
            })
            .await
            .unwrap();
        Self {
            context,
            client,
            consumer,
            stream,
            name,
            _account: account,
        }
    }
    pub async fn publish(&self, subject: &'static str, bytes: Vec<u8>) {
        self.context
            .publish(subject, bytes.into())
            .await
            .unwrap()
            .await
            .unwrap();
    }
    pub async fn next(&self) -> jetstream::Message {
        let mut batch = self
            .consumer
            .fetch()
            .max_messages(1)
            .expires(Duration::from_secs(5))
            .messages()
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), batch.next())
            .await
            .expect("actual delivery")
            .expect("one item")
            .unwrap()
    }
    pub async fn counts(&mut self) -> (usize, u64) {
        let pending = self.consumer.info().await.unwrap().num_ack_pending;
        let stored = self.stream.info().await.unwrap().state.messages;
        (pending, stored)
    }
    pub async fn wait_counts(&mut self, expected: (usize, u64)) {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if self.counts().await == expected {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("broker acknowledges exactly the completed commands");
    }
    pub async fn acknowledgements(&self) -> Subscriber {
        let subscription = self.client.subscribe("$JS.ACK.>").await.unwrap();
        self.client.flush().await.unwrap();
        subscription
    }
    pub fn own_ack(&self, message: &async_nats::Message) -> bool {
        message.subject.split('.').any(|part| part == self.name)
    }
    pub async fn close(&self) {
        self.context.delete_stream(&self.name).await.unwrap();
    }
}
