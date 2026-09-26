//! Parent-owned durable pumping for one service instance's application logs.
//!
//! The writer retains at most one 64-record/4 MiB batch. The append call gets
//! a bounded clone of that batch, so a transient cancellation temporarily uses
//! at most two such payload copies. The parent must settle the worker's event
//! collector before calling [`ServiceLogWriter::final_flush`], and must keep
//! lease supervision active until the flush result is handled.

mod types;
mod writer;

#[cfg(test)]
#[path = "service_log_writer/tests.rs"]
mod tests;

pub use types::{
    DEFAULT_SERVICE_LOG_APPEND_TIMEOUT, DEFAULT_SERVICE_LOG_FINAL_FLUSH_TIMEOUT,
    DEFAULT_SERVICE_LOG_MAX_RETRY_INTERVAL, DEFAULT_SERVICE_LOG_RETRY_INTERVAL,
    ServiceLogWriterError, ServiceLogWriterFlush, ServiceLogWriterPolicy, ServiceLogWriterPoll,
};
pub use writer::ServiceLogWriter;
