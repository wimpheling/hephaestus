// Shared crate-visible imports and protocol bounds for the private split modules.
//! Bounded semantic secret-broker protocol over a host Unix socket that
//! libkrun exposes through a dedicated vsock port.

pub(super) use async_trait::async_trait;
pub(super) use brokered_egress_client::{WireBrokerRequest, WireBrokerResponse, WireBrokerStatus};
pub(super) use brokered_egress_domain::{
    BrokeredEgressError, BrokeredSecretRule, HttpInjectionLocation, InjectionDirection,
};
pub(super) use secret_application::{
    BrokerAdapter, BrokerAdapterError, BrokerRequest, BrokerResponse, BrokerStatus,
    SecretRuntimeResolver, VerifiedBrokeredHttpsRule,
};
pub(super) use secret_domain::{OpaqueRuntimeCredential, SecretSlotKey, SecretValue};
pub(super) use std::{
    collections::HashMap,
    fs,
    net::{IpAddr, SocketAddr},
    os::unix::fs::{FileTypeExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
pub(super) use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpStream, UnixListener, UnixStream},
};
pub(super) use tokio_util::sync::CancellationToken;

pub(super) const MAX_FRAME_BYTES: usize = 1024 * 1024;
pub(super) const MAX_CREDENTIAL_BYTES: usize = 256;
pub(super) const MAX_ADAPTER_BODY_BYTES: usize = 64 * 1024;
pub(super) const MAX_UPSTREAM_RESPONSE_BYTES: u64 = 64 * 1024;
pub(super) const UPSTREAM_TIMEOUT: Duration = Duration::from_secs(5);
pub(super) const MAX_HTTPS_HEADERS: usize = 32;
pub(super) const MAX_HTTPS_PATH_BYTES: usize = 4_096;
