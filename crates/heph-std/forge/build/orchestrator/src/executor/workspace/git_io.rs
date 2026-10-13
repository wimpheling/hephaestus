//! Original pipe/reader thread custody. Errors/diagnostics stay private.
use super::super::owner::lock;
use std::{
    any::Any,
    io::{self, Read},
    sync::{Arc, Mutex},
    thread::JoinHandle,
};
struct State {
    pipe: Option<Box<dyn Read + Send>>,
    task: Option<JoinHandle<io::Result<Vec<u8>>>>,
    joined: Option<Result<(), Arc<io::Error>>>,
    bytes: Option<Vec<u8>>,
    panic: Option<Box<dyn Any + Send>>,
}
pub(super) struct Reader(Mutex<State>);
impl Reader {
    pub(super) fn new(pipe: impl Read + Send + 'static) -> Arc<Self> {
        Arc::new(Self(Mutex::new(State {
            pipe: Some(Box::new(pipe)),
            task: None,
            joined: None,
            bytes: None,
            panic: None,
        })))
    }
    pub(super) fn start(&self) {
        let mut state = lock(&self.0);
        let Some(mut pipe) = state.pipe.take() else {
            return;
        };
        // The original FD is either in this retained state or this exact
        // thread. No caller/observer owns an abort handle.
        match std::thread::Builder::new()
            .name("heph-build-git-io".into())
            .spawn(move || {
                let mut bytes = Vec::new();
                pipe.read_to_end(&mut bytes)?;
                Ok(bytes)
            }) {
            Ok(task) => state.task = Some(task),
            Err(error) => state.joined = Some(Err(Arc::new(error))),
        }
    }
    pub(super) fn join(&self) -> Result<(), Arc<io::Error>> {
        self.start();
        let mut state = lock(&self.0);
        if let Some(joined) = &state.joined {
            return joined.clone();
        }
        let Some(task) = state.task.take() else {
            return Err(Arc::new(io::Error::other("original Git IO owner unknown")));
        };
        let result = match task.join() {
            Ok(Ok(bytes)) => {
                state.bytes = Some(bytes);
                Ok(())
            }
            Ok(Err(error)) => Err(Arc::new(error)),
            Err(payload) => {
                state.panic = Some(payload);
                Err(Arc::new(io::Error::other("original Git IO owner panicked")))
            }
        };
        state.joined = Some(result.clone());
        result
    }
    pub(super) fn settled(&self) -> bool {
        lock(&self.0).joined.is_some()
    }
    pub(super) fn take_bytes(&self) -> Result<Vec<u8>, Arc<io::Error>> {
        self.join()?;
        lock(&self.0)
            .bytes
            .take()
            .ok_or_else(|| Arc::new(io::Error::other("original Git output already consumed")))
    }
}
