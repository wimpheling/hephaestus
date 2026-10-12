//! Broker clones retain original descriptor paths through late shutdown/Drop.
use super::ServiceBroker;
use crate::{directory::ChildDirectory, protocol::PRIVATE_SERVICE_SOCKET_NAME};
use std::{fs, sync::Arc, time::Duration};

#[tokio::test]
async fn retained_broker_clone_keeps_directory_alive_and_never_removes_replacement_socket() {
    let temp = tempfile::tempdir().unwrap();
    let directory = ChildDirectory::create(
        temp.path(),
        "vm",
        rustix::fs::Mode::from_bits_truncate(0o700),
        |_| {},
    )
    .unwrap();
    let file = directory.file().unwrap();
    let retained_file = Arc::downgrade(&file);
    let broker =
        Arc::new(ServiceBroker::bind_pinned(file.clone(), 1, Duration::from_secs(1)).unwrap());
    let caller = broker.clone();
    let original = temp.path().join("original");
    fs::rename(temp.path().join("vm"), &original).unwrap();
    fs::create_dir(temp.path().join("vm")).unwrap();
    let foreign = temp.path().join("vm").join(PRIVATE_SERVICE_SOCKET_NAME);
    fs::write(&foreign, b"foreign socket bytes").unwrap();
    drop(directory);
    drop(file);
    drop(broker);
    assert!(
        retained_file.upgrade().is_some(),
        "external broker owns the original descriptor"
    );
    caller.shutdown().await;
    assert!(!original.join(PRIVATE_SERVICE_SOCKET_NAME).exists());
    assert_eq!(fs::read(&foreign).unwrap(), b"foreign socket bytes");
    drop(caller);
    assert!(retained_file.upgrade().is_none());
    assert_eq!(fs::read(foreign).unwrap(), b"foreign socket bytes");
}
