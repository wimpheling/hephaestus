use super::assert_bytes_have_no_credentials;
use futures_util::TryStreamExt as _;
use std::collections::BTreeSet;

/// Scan every retained `JetStream` message after the fixture daemon has stopped.
/// Acknowledged work-queue messages may already be deleted; this establishes
/// confinement for retained messages, not for previously consumed payloads.
pub async fn assert_nats_has_no_credentials(nats_url: &str) {
    let client = async_nats::connect(nats_url)
        .await
        .expect("connect for retained message scan");
    let context = async_nats::jetstream::new(client);
    let mut streams = context.streams();
    let mut surfaces = BTreeSet::new();
    let mut total = 0_u64;
    while let Some(info) = streams
        .try_next()
        .await
        .expect("enumerate retained streams")
    {
        let stream = context
            .get_stream(&info.config.name)
            .await
            .expect("open retained stream");
        let mut count = 0_u64;
        assert!(
            info.state.last_sequence < 100_000,
            "unexpected fixture stream size"
        );
        if info.state.messages > 0 {
            for sequence in info.state.first_sequence..=info.state.last_sequence {
                let message = match stream.get_raw_message(sequence).await {
                    Ok(message) => message,
                    Err(error)
                        if error.kind()
                            == async_nats::jetstream::stream::RawMessageErrorKind::NoMessageFound =>
                    {
                        continue;
                    }
                    Err(error) => panic!("cannot inspect retained stream message: {:?}", error.kind()),
                };
                assert_bytes_have_no_credentials(message.subject.as_bytes());
                assert_bytes_have_no_credentials(&message.payload);
                for (name, values) in message.headers.iter() {
                    let name: &str = name.as_ref();
                    assert_bytes_have_no_credentials(name.as_bytes());
                    for value in values {
                        assert_bytes_have_no_credentials(value.as_str().as_bytes());
                    }
                }
                count += 1;
            }
        }
        assert_eq!(
            count, info.state.messages,
            "retained stream changed during scan"
        );
        total += count;
        surfaces.insert(info.config.name);
    }
    for required in ["HEPH_RUN_COMMANDS", "HEPHAESTUS_PRODUCT_EVENTS"] {
        assert!(
            surfaces.contains(required),
            "missing required retained stream"
        );
    }
    assert!(total > 0, "retained message scan must not be empty");
    eprintln!(
        "Cooking retained NATS credential scan: {} streams, {total} messages",
        surfaces.len()
    );
}
