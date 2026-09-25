use std::{
    io::{self, ErrorKind, Read},
    net::{Shutdown, TcpListener, TcpStream},
    sync::{Arc, Mutex, mpsc},
    thread,
    time::{Duration, Instant},
};

use super::{
    routing::{response_for, write_response},
    types::{
        IO_TIMEOUT, MAX_HEADER_BYTES, QUEUE_CAPACITY, Response, StartupIdentity, WORKER_COUNT,
    },
};

pub(crate) fn run(listener: &TcpListener, identity: StartupIdentity) -> io::Result<()> {
    let (sender, receiver) = mpsc::sync_channel(QUEUE_CAPACITY);
    let receiver = Arc::new(Mutex::new(receiver));
    let identity = Arc::new(identity);
    for _ in 0..WORKER_COUNT {
        let receiver = Arc::clone(&receiver);
        let identity = Arc::clone(&identity);
        thread::Builder::new()
            .name(String::from("cooking-service-worker"))
            .spawn(move || {
                loop {
                    let connection = receiver
                        .lock()
                        .expect("service worker queue mutex is not poisoned")
                        .recv();
                    let Ok(stream) = connection else {
                        break;
                    };
                    let _ = serve_connection(stream, &identity);
                }
            })
            .map_err(io::Error::other)?;
    }

    loop {
        let (stream, _) = listener.accept()?;
        if sender.send(stream).is_err() {
            return Err(io::Error::new(
                ErrorKind::BrokenPipe,
                "service worker queue closed",
            ));
        }
    }
}

pub(crate) fn serve_connection(
    mut stream: TcpStream,
    identity: &StartupIdentity,
) -> io::Result<()> {
    stream.set_read_timeout(Some(IO_TIMEOUT))?;
    stream.set_write_timeout(Some(IO_TIMEOUT))?;
    let response = match read_request(&mut stream) {
        Ok(request) => response_for(request.as_slice(), identity),
        Err(_) => Response {
            status: 400,
            content_type: "text/plain; charset=utf-8",
            body: b"bad request".to_vec(),
        },
    };
    write_response(&mut stream, &response)?;
    stream.shutdown(Shutdown::Write)
}

fn read_request(stream: &mut TcpStream) -> io::Result<Vec<u8>> {
    read_request_with_budget(stream, IO_TIMEOUT)
}

pub(crate) fn read_request_with_budget(
    stream: &mut TcpStream,
    budget: Duration,
) -> io::Result<Vec<u8>> {
    let deadline = Instant::now() + budget;
    let mut request = Vec::with_capacity(1024);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(io::Error::new(
                ErrorKind::TimedOut,
                "request header deadline elapsed",
            ));
        }
        stream.set_read_timeout(Some(remaining))?;
        let mut chunk = [0_u8; 1024];
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            return Err(io::Error::new(
                ErrorKind::UnexpectedEof,
                "request ended before headers",
            ));
        }
        request.extend_from_slice(&chunk[..read]);
        if request.len() > MAX_HEADER_BYTES {
            return Err(io::Error::new(
                ErrorKind::InvalidData,
                "request headers exceed the service limit",
            ));
        }
        if let Some(end) = request.windows(4).position(|window| window == b"\r\n\r\n") {
            let end = end + 4;
            if request.len() != end {
                return Err(io::Error::new(
                    ErrorKind::InvalidData,
                    "request body is not supported",
                ));
            }
            request.truncate(end);
            return Ok(request);
        }
    }
}
