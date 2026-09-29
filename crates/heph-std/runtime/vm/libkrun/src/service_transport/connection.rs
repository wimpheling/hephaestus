use super::{ConnectionState, closed_stream_error, lock};
use std::{
    io,
    pin::Pin,
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll},
};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::UnixStream;

impl ConnectionState {
    pub(super) const fn new(stream: UnixStream) -> Self {
        Self {
            stream: Mutex::new(Some(stream)),
            closed: AtomicBool::new(false),
            read_waker: Mutex::new(None),
            write_waker: Mutex::new(None),
        }
    }

    pub(super) fn poll_read(
        &self,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if self.closed.load(Ordering::Acquire) {
            return Poll::Ready(Ok(()));
        }
        {
            let mut waker = lock(&self.read_waker);
            if self.closed.load(Ordering::Acquire) {
                return Poll::Ready(Ok(()));
            }
            *waker = Some(context.waker().clone());
        }
        if self.closed.load(Ordering::Acquire) {
            return Poll::Ready(Ok(()));
        }
        let mut stream_guard = lock(&self.stream);
        let Some(stream) = stream_guard.as_mut() else {
            return Poll::Ready(Ok(()));
        };
        let result = Pin::new(&mut *stream).poll_read(context, buffer);
        drop(stream_guard);
        if result.is_ready() {
            lock(&self.read_waker).take();
        }
        result
    }

    pub(super) fn poll_write(
        &self,
        context: &mut Context<'_>,
        buffer: &[u8],
    ) -> Poll<io::Result<usize>> {
        if self.closed.load(Ordering::Acquire) {
            return Poll::Ready(Err(closed_stream_error()));
        }
        {
            let mut waker = lock(&self.write_waker);
            if self.closed.load(Ordering::Acquire) {
                return Poll::Ready(Err(closed_stream_error()));
            }
            *waker = Some(context.waker().clone());
        }
        if self.closed.load(Ordering::Acquire) {
            return Poll::Ready(Err(closed_stream_error()));
        }
        let mut stream_guard = lock(&self.stream);
        let Some(stream) = stream_guard.as_mut() else {
            return Poll::Ready(Err(closed_stream_error()));
        };
        let result = Pin::new(&mut *stream).poll_write(context, buffer);
        drop(stream_guard);
        if result.is_ready() {
            lock(&self.write_waker).take();
        }
        result
    }

    pub(super) fn poll_flush(&self, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        if self.closed.load(Ordering::Acquire) {
            return Poll::Ready(Err(closed_stream_error()));
        }
        {
            let mut waker = lock(&self.write_waker);
            if self.closed.load(Ordering::Acquire) {
                return Poll::Ready(Err(closed_stream_error()));
            }
            *waker = Some(context.waker().clone());
        }
        if self.closed.load(Ordering::Acquire) {
            return Poll::Ready(Err(closed_stream_error()));
        }
        let mut stream_guard = lock(&self.stream);
        let Some(stream) = stream_guard.as_mut() else {
            return Poll::Ready(Err(closed_stream_error()));
        };
        let result = Pin::new(&mut *stream).poll_flush(context);
        drop(stream_guard);
        if result.is_ready() {
            lock(&self.write_waker).take();
        }
        result
    }

    pub(super) fn poll_shutdown(&self, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        if self.closed.load(Ordering::Acquire) {
            return Poll::Ready(Ok(()));
        }
        {
            let mut waker = lock(&self.write_waker);
            if self.closed.load(Ordering::Acquire) {
                return Poll::Ready(Ok(()));
            }
            *waker = Some(context.waker().clone());
        }
        if self.closed.load(Ordering::Acquire) {
            return Poll::Ready(Ok(()));
        }
        let mut stream_guard = lock(&self.stream);
        let Some(stream) = stream_guard.as_mut() else {
            return Poll::Ready(Ok(()));
        };
        let result = Pin::new(&mut *stream).poll_shutdown(context);
        drop(stream_guard);
        if result.is_ready() {
            lock(&self.write_waker).take();
        }
        result
    }

    pub(super) fn close(&self) {
        self.closed.store(true, Ordering::Release);
        drop(lock(&self.stream).take());
        let read_waker = lock(&self.read_waker).take();
        let write_waker = lock(&self.write_waker).take();
        if let Some(waker) = read_waker {
            waker.wake();
        }
        if let Some(waker) = write_waker {
            waker.wake();
        }
    }
}
