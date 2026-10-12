mod errors;
mod guest;
mod runtime;
mod types;

#[cfg(test)]
mod path_tests;

#[cfg(test)]
#[path = "worker/tests.rs"]
mod tests;

pub use types::{
    WireError, WireErrorKind, WireLogStream, WorkerCommand, WorkerConfiguration, WorkerEvent,
    WorkerMessage, WorkerRequest,
};

use crate::framing::{read_sync, write_sync};
use runtime::WorkerRuntime;

use std::{
    error::Error,
    fs::File,
    io,
    os::unix::net::UnixStream,
    path::PathBuf,
    sync::{Arc, Mutex, MutexGuard},
};

pub fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    let (socket_path, runtime_directory) = parse_launch_arguments(std::env::args_os())?;
    let stream = UnixStream::connect(socket_path)?;
    let reader = stream.try_clone()?;
    let writer = Arc::new(Mutex::new(stream));
    let outcome = run(reader, &writer, runtime_directory.as_ref());
    drop(runtime_directory);
    outcome
}

fn run(
    mut reader: UnixStream,
    writer: &Arc<Mutex<UnixStream>>,
    runtime_directory: Option<&Arc<File>>,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let mut runtime: Option<WorkerRuntime> = None;
    loop {
        let request: WorkerRequest = read_sync(&mut reader)?;
        match request.command {
            WorkerCommand::Configure {
                config,
                spec,
                runtime_dir,
            } => {
                let result = if runtime.is_some() {
                    Err(WireError::invalid_state("worker is already configured"))
                } else {
                    WorkerRuntime::configure(config, *spec, runtime_dir, runtime_directory.cloned())
                        .map(|configured| runtime = Some(configured))
                        .map_err(WireError::from)
                };
                send_response(writer, request.request_id, result)?;
            }
            WorkerCommand::Start => {
                let result = runtime
                    .as_mut()
                    .ok_or_else(|| WireError::invalid_state("worker is not configured"))
                    .and_then(WorkerRuntime::start);
                match result {
                    Ok(started) => {
                        send_response(writer, request.request_id, Ok(()))?;
                        send_message(writer, &WorkerMessage::Event(started.event.clone()))?;
                        started.spawn(Arc::clone(writer));
                    }
                    Err(error) => {
                        send_response(writer, request.request_id, Err(error))?;
                    }
                }
            }
            WorkerCommand::Cancel { timeout_ms } => {
                let result = runtime
                    .as_ref()
                    .ok_or_else(|| WireError::invalid_state("worker is not configured"))
                    .and_then(|configured| configured.cancel(timeout_ms));
                send_response(writer, request.request_id, result)?;
            }
            WorkerCommand::Health { nonce } => {
                let result = runtime
                    .as_ref()
                    .ok_or_else(|| WireError::invalid_state("worker is not configured"))
                    .and_then(|configured| configured.health(nonce));
                send_response(writer, request.request_id, result)?;
            }
            WorkerCommand::InvokePrivateHttp {
                request_id,
                request: private_request,
            } => {
                let result = runtime
                    .as_ref()
                    .ok_or_else(|| WireError::invalid_state("worker is not configured"))
                    .and_then(|configured| configured.private_http(request_id, private_request));
                send_response(writer, request.request_id, result)?;
            }
            WorkerCommand::OpenPrivateServiceConnection { connection } => {
                let result = runtime
                    .as_ref()
                    .ok_or_else(|| WireError::invalid_state("worker is not configured"))
                    .and_then(|configured| configured.open_private_service_connection(connection));
                send_response(writer, request.request_id, result)?;
            }
            WorkerCommand::Destroy => {
                send_response(writer, request.request_id, Ok(()))?;
                drop(runtime.take());
                return Ok(());
            }
        }
    }
}
fn send_response(
    writer: &Arc<Mutex<UnixStream>>,
    request_id: u64,
    result: Result<(), WireError>,
) -> io::Result<()> {
    send_message(writer, &WorkerMessage::Response { request_id, result })
}

fn send_message(writer: &Arc<Mutex<UnixStream>>, message: &WorkerMessage) -> io::Result<()> {
    write_sync(&mut *lock(writer), message)
}

fn parse_launch_arguments(
    mut arguments: impl Iterator<Item = std::ffi::OsString>,
) -> Result<(PathBuf, Option<Arc<File>>), io::Error> {
    let _binary = arguments.next();
    let flag = arguments.next();
    if flag.as_deref() == Some(std::ffi::OsStr::new("--runtime-stdin")) {
        if arguments.next().is_some() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "unexpected runtime arguments",
            ));
        }
        let runtime = Arc::new(inherited_runtime_directory()?);
        let socket =
            crate::directory::fd_path(&runtime).join(crate::protocol::SUPERVISOR_SOCKET_NAME);
        return Ok((socket, Some(runtime)));
    }
    let path = arguments.next();
    if flag.as_deref() != Some(std::ffi::OsStr::new("--socket"))
        || path.is_none()
        || arguments.next().is_some()
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "usage: hephaestus-vm-libkrun-worker --socket PATH | --runtime-stdin",
        ));
    }
    Ok((PathBuf::from(path.expect("checked above")), None))
}

fn inherited_runtime_directory() -> io::Result<File> {
    use std::os::unix::fs::MetadataExt;
    let runtime: File = rustix::io::fcntl_dupfd_cloexec(std::io::stdin(), 3)?.into();
    let metadata = runtime.metadata()?;
    if !metadata.is_dir()
        || metadata.uid() != rustix::process::geteuid().as_raw()
        || metadata.mode() & 0o077 != 0
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "inherited runtime must be an owned private directory",
        ));
    }
    Ok(runtime)
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
