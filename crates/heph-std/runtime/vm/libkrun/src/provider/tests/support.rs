// Shared fixtures used by the libkrun provider lifecycle tests.
pub(super) use crate::provider::LibkrunProvider;
pub(super) use crate::provider::http::private_http_response;
pub(super) use crate::provider::{common::*, helpers::*};
pub(super) use crate::{
    config::LibkrunConfig,
    protocol::{
        PRIVATE_SERVICE_CHALLENGE_BYTES, PRIVATE_SERVICE_HANDSHAKE_MAGIC,
        PRIVATE_SERVICE_HANDSHAKE_VERSION, PrivateHttpResponseMessage,
        PrivateServiceConnectionMessage,
    },
    service_transport::ServiceBroker,
    worker::{WireError, WireErrorKind, WorkerCommand, WorkerEvent},
};
pub(super) use async_trait::async_trait;
pub(super) use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    time::Duration,
};
pub(super) use tempfile::TempDir;
pub(super) use tokio::io::{AsyncReadExt, AsyncWriteExt};
pub(super) use tokio::net::UnixStream;
pub(super) use tokio::sync::{Mutex, Notify, Semaphore, broadcast, watch};
pub(super) use vm_trait::{
    DiskFormat, GuestCommand, NetworkMode, PrivateHttpRequest, RootFilesystem, StopMode, VmDisk,
    VmError, VmEvent, VmId, VmInstance, VmProvider, VmResources, VmSpec,
};

#[path = "fixtures.rs"]
mod fixtures;
#[path = "mock_worker.rs"]
mod mock_worker;
pub(super) use fixtures::*;
pub(super) use mock_worker::*;
