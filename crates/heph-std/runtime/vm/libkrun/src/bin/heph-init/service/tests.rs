use super::transport::{bridge_streams, forward};
use super::{ServiceConfig, ServiceSupervisor, loopback_flags_up};
use std::{
    fs::File,
    io::{self, Read, Write},
    net::{TcpListener, TcpStream},
    os::fd::OwnedFd,
    os::unix::net::UnixStream,
    sync::mpsc,
    thread,
    time::Duration,
};
use uuid::Uuid;
use vm_libkrun::protocol::{
    PRIVATE_SERVICE_CHALLENGE_BYTES, PrivateHttpServiceMessage, PrivateServiceChallenge,
    PrivateServiceConnectionMessage,
};

#[test]
fn config_rejects_invalid_values() {
    let invalid_port = PrivateHttpServiceMessage {
        loopback_port: 80,
        max_connections: 1,
        connect_timeout_ms: 1,
    };
    assert!(ServiceConfig::try_from(&invalid_port).is_err());
    let invalid_limit = PrivateHttpServiceMessage {
        loopback_port: 8080,
        max_connections: 65,
        connect_timeout_ms: 1,
    };
    assert!(ServiceConfig::try_from(&invalid_limit).is_err());
    let invalid_timeout = PrivateHttpServiceMessage {
        loopback_port: 8080,
        max_connections: 1,
        connect_timeout_ms: 30_001,
    };
    assert!(ServiceConfig::try_from(&invalid_timeout).is_err());
}

#[test]
fn loopback_flag_update_preserves_existing_flags() {
    let iff_up = i16::try_from(libc::IFF_UP).expect("IFF_UP fits i16");
    assert_eq!(loopback_flags_up(0x10), 0x10 | iff_up);
    assert_eq!(loopback_flags_up(iff_up), iff_up);
}

#[test]
fn forwarding_preserves_half_close() {
    let (mut source_peer, source) = UnixStream::pair().unwrap();
    let (destination, mut destination_peer) = UnixStream::pair().unwrap();
    let bridge = thread::spawn(move || forward(source, destination));
    source_peer.write_all(b"request").unwrap();
    source_peer.shutdown(std::net::Shutdown::Write).unwrap();
    let mut request = Vec::new();
    destination_peer.read_to_end(&mut request).unwrap();
    assert_eq!(request, b"request");
    bridge.join().unwrap();
}

#[test]
fn host_close_wakes_idle_loopback_reader() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let address = listener.local_addr().unwrap();
    let (done_sender, done_receiver) = mpsc::channel();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        let mut request = [0_u8; 1];
        let read = stream.read(&mut request).unwrap();
        assert_eq!(read, 0, "host close should FIN the loopback write side");
        drop(stream);
        done_sender.send(()).unwrap();
    });
    let local = TcpStream::connect(address).unwrap();
    let (host_peer, host_side) = UnixStream::pair().unwrap();
    let host_fd: OwnedFd = host_side.into();
    let host = File::from(host_fd);
    let bridge = thread::spawn(move || bridge_streams(&local, &host));
    drop(host_peer);
    done_receiver
        .recv_timeout(Duration::from_secs(1))
        .expect("idle loopback reader did not receive EOF");
    bridge.join().unwrap();
    server.join().unwrap();
}

#[test]
fn admission_is_bounded_and_cancelled() {
    let supervisor = ServiceSupervisor::new(ServiceConfig {
        loopback_port: 8080,
        max_connections: 1,
        connect_timeout: Duration::from_millis(1),
    });
    let first = supervisor.reserve_pending().unwrap();
    supervisor
        .acquire_active(first, std::time::Instant::now() + Duration::from_secs(1))
        .unwrap();
    assert!(supervisor.reserve_pending().is_some());
    assert!(supervisor.reserve_pending().is_none());
    supervisor.cancel();
    assert!(supervisor.reserve_pending().is_none());
}

#[test]
fn pending_admission_waits_for_released_active_slot() {
    let supervisor = ServiceSupervisor::new(ServiceConfig {
        loopback_port: 8080,
        max_connections: 1,
        connect_timeout: Duration::from_secs(1),
    });
    let first = supervisor.reserve_pending().unwrap();
    supervisor
        .acquire_active(first, std::time::Instant::now() + Duration::from_secs(1))
        .unwrap();
    let second = supervisor.reserve_pending().unwrap();
    let waiter_supervisor = supervisor.clone();
    let waiter = thread::spawn(move || {
        waiter_supervisor.acquire_active(second, std::time::Instant::now() + Duration::from_secs(1))
    });
    thread::sleep(Duration::from_millis(25));
    assert!(!waiter.is_finished());
    supervisor.release(first);
    waiter.join().unwrap().unwrap();
    supervisor.release(second);
    supervisor.cancel();
}

#[test]
fn expired_admission_does_not_take_a_free_slot() {
    let supervisor = ServiceSupervisor::new(ServiceConfig {
        loopback_port: 8080,
        max_connections: 1,
        connect_timeout: Duration::from_secs(1),
    });
    let id = supervisor.reserve_pending().unwrap();
    let result = supervisor.acquire_active(
        id,
        std::time::Instant::now()
            .checked_sub(Duration::from_millis(1))
            .expect("test instant remains representable"),
    );
    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::TimedOut);
    let next = supervisor.reserve_pending().unwrap();
    supervisor
        .acquire_active(next, std::time::Instant::now() + Duration::from_secs(1))
        .unwrap();
    supervisor.release(next);
    supervisor.cancel();
}

#[test]
fn pending_admission_honors_setup_deadline() {
    let supervisor = ServiceSupervisor::new(ServiceConfig {
        loopback_port: 8080,
        max_connections: 1,
        connect_timeout: Duration::from_secs(1),
    });
    let first = supervisor.reserve_pending().unwrap();
    supervisor
        .acquire_active(first, std::time::Instant::now() + Duration::from_secs(1))
        .unwrap();
    let second = supervisor.reserve_pending().unwrap();
    let result = supervisor.acquire_active(
        second,
        std::time::Instant::now() + Duration::from_millis(25),
    );
    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::TimedOut);
    supervisor.release(first);
    supervisor.cancel();
}

#[test]
fn pending_admission_cancels_without_sleep_polling() {
    let supervisor = ServiceSupervisor::new(ServiceConfig {
        loopback_port: 8080,
        max_connections: 1,
        connect_timeout: Duration::from_secs(1),
    });
    let first = supervisor.reserve_pending().unwrap();
    supervisor
        .acquire_active(first, std::time::Instant::now() + Duration::from_secs(1))
        .unwrap();
    let second = supervisor.reserve_pending().unwrap();
    let waiter_supervisor = supervisor.clone();
    let waiter = thread::spawn(move || {
        waiter_supervisor.acquire_active(second, std::time::Instant::now() + Duration::from_secs(1))
    });
    thread::sleep(Duration::from_millis(25));
    supervisor.cancel();
    assert_eq!(
        waiter.join().unwrap().unwrap_err().kind(),
        io::ErrorKind::Interrupted
    );
    supervisor.release(first);
}

#[test]
fn failed_connect_releases_admission() {
    let supervisor = ServiceSupervisor::new(ServiceConfig {
        loopback_port: 8080,
        max_connections: 1,
        connect_timeout: Duration::from_millis(1),
    });
    supervisor.open(PrivateServiceConnectionMessage {
        connection_id: Uuid::new_v4(),
        challenge: PrivateServiceChallenge([0; PRIVATE_SERVICE_CHALLENGE_BYTES]),
    });
    thread::sleep(Duration::from_millis(250));
    let next = supervisor.reserve_pending().unwrap();
    supervisor
        .acquire_active(next, std::time::Instant::now() + Duration::from_secs(1))
        .unwrap();
    supervisor.release(next);
    supervisor.cancel();
    supervisor.join_all();
}

#[test]
fn cancellation_closes_owned_descriptor_snapshot() {
    let supervisor = ServiceSupervisor::new(ServiceConfig {
        loopback_port: 8080,
        max_connections: 1,
        connect_timeout: Duration::from_secs(1),
    });
    let (mut peer, socket) = UnixStream::pair().unwrap();
    let id = supervisor.reserve_pending().unwrap();
    supervisor
        .acquire_active(id, std::time::Instant::now() + Duration::from_secs(1))
        .unwrap();
    let owned: OwnedFd = socket.try_clone().unwrap().into();
    assert!(supervisor.register_fds(id, vec![std::sync::Arc::new(owned)]));
    drop(socket);
    supervisor.cancel();
    let mut bytes = Vec::new();
    peer.read_to_end(&mut bytes).unwrap();
    assert!(bytes.is_empty());
    supervisor.release(id);
}

#[test]
fn completed_handle_registry_is_reaped_under_churn() {
    let supervisor = ServiceSupervisor::new(ServiceConfig {
        loopback_port: 8080,
        max_connections: 1,
        connect_timeout: Duration::from_millis(1),
    });
    for _ in 0..32 {
        supervisor.open(PrivateServiceConnectionMessage {
            connection_id: Uuid::new_v4(),
            challenge: PrivateServiceChallenge([0; PRIVATE_SERVICE_CHALLENGE_BYTES]),
        });
        thread::sleep(Duration::from_millis(5));
    }
    let deadline = std::time::Instant::now() + Duration::from_secs(1);
    loop {
        supervisor.reap_finished();
        let active_count = supervisor.inner.state.lock().unwrap().active.len();
        let handle_count = supervisor.handles.lock().unwrap().len();
        if active_count == 0 && handle_count <= 1 {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "service workers did not settle before the bounded deadline: active={active_count} handles={handle_count}"
        );
        thread::sleep(Duration::from_millis(5));
    }
    assert!(supervisor.inner.state.lock().unwrap().active.is_empty());
    assert!(supervisor.handles.lock().unwrap().len() <= 1);
    supervisor.cancel();
    supervisor.join_all();
}

#[test]
fn open_keeps_existing_live_handle_registered() {
    let supervisor = ServiceSupervisor::new(ServiceConfig {
        loopback_port: 8080,
        max_connections: 1,
        connect_timeout: Duration::from_millis(1),
    });
    let blocker = thread::spawn(|| thread::sleep(Duration::from_millis(100)));
    supervisor.handles.lock().unwrap().push(blocker);
    supervisor.open(PrivateServiceConnectionMessage {
        connection_id: Uuid::new_v4(),
        challenge: PrivateServiceChallenge([0; PRIVATE_SERVICE_CHALLENGE_BYTES]),
    });
    assert!(!supervisor.handles.lock().unwrap().is_empty());
    supervisor.cancel();
    supervisor.join_all();
}
