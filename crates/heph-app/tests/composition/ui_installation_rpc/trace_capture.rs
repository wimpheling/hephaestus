use std::io::{self, Write};
use std::sync::{Arc, Mutex, OnceLock};

use tracing_subscriber::fmt::MakeWriter;

#[derive(Clone)]
struct SharedWriterFactory(Arc<Mutex<Vec<u8>>>);

struct SharedWriter(Arc<Mutex<Vec<u8>>>);

impl<'a> MakeWriter<'a> for SharedWriterFactory {
    type Writer = SharedWriter;

    fn make_writer(&'a self) -> Self::Writer {
        SharedWriter(self.0.clone())
    }
}

impl Write for SharedWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .expect("UI RPC trace buffer lock")
            .extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(super) struct TraceCapture {
    bytes: Arc<Mutex<Vec<u8>>>,
}

pub(super) fn install() -> TraceCapture {
    static BUFFER: OnceLock<Arc<Mutex<Vec<u8>>>> = OnceLock::new();
    let bytes = BUFFER
        .get_or_init(|| Arc::new(Mutex::new(Vec::new())))
        .clone();
    bytes.lock().expect("clear UI RPC trace buffer").clear();

    // Do not consult RUST_LOG: the non-disclosure assertion must observe the
    // events emitted by app and adapter tasks even when the caller narrows its
    // normal test logging policy.
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_writer(SharedWriterFactory(bytes.clone()))
        // This is the only opt-in test in this binary, so losing ownership
        // would make the security assertion unverifiable.
        .try_init()
        .expect("UI RPC test binary must own its global tracing subscriber");

    TraceCapture { bytes }
}

impl TraceCapture {
    pub(super) fn assert_absent(&self, secret: &str) -> (usize, usize) {
        let bytes = self.bytes.lock().expect("read UI RPC trace buffer");
        let logs = String::from_utf8_lossy(&bytes);
        assert!(
            !logs.contains(secret),
            "UI RPC trace output contained a sensitive handoff value"
        );
        let events = logs.lines().count();
        (bytes.len(), events)
    }
}
