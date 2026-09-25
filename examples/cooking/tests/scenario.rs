//! Optional real cooking-app slice on the golden daemon fixture.

use super::{
    BrokeredFixture, BrokeredTlsUpstream, GatewayGoldenFixture, SeededInstance, secret_command_key,
};
use authz_postgres::PostgresMelangeAuthorizer;
use brokered_egress_domain::{
    BrokeredSecretRule, BrokeredSecretRuleId, ExactHttpsOrigin, HeaderName, HttpInjectionLocation,
};
use forge_domain::{OrganizationId, ProjectId};
use identity_domain::{AuthenticatedIdentity, BrowserSessionSid, RequestId, UserId};
use secret_application::{
    BindSecret, CreateSecret, DeclareBrokeredHttpsRule, GrantAndAcceptSecretImport, RotateSecret,
};
use secret_broker::{BrokeredHttpsAdapterRegistry, BrokeredHttpsRequest};
use secret_domain::{
    AgentSecretBindingId, DeliveryMode, ExecutionPhase, SecretAlias, SecretGrantId, SecretId,
    SecretImportId, SecretName, SecretOwner, SecretSlotKey, SecretTarget, SecretUsePolicy,
    SecretValue, SecretVersionId,
};
use secret_postgres::SecretService;
use secret_store::{EncryptedStore, LocalKeyProvider};
use std::{
    env, fs,
    io::Write as _,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};
use tokio_rustls::TlsAcceptor;

#[path = "scenario/adapters.rs"]
mod adapters;
#[path = "scenario/checkpoint.rs"]
mod checkpoint;
#[path = "scenario/config.rs"]
mod config;
#[path = "scenario/credentials.rs"]
mod credentials;
#[path = "scenario/diagnostics.rs"]
mod diagnostics;
#[path = "scenario/event_wait.rs"]
mod event_wait;
#[path = "scenario/exercise_follow_up.rs"]
mod exercise_follow_up;
#[path = "scenario/exercise_initial.rs"]
mod exercise_initial;
#[path = "scenario/http.rs"]
mod http;
#[path = "scenario/retry.rs"]
mod retry;
#[path = "scenario/seed.rs"]
mod seed;
#[path = "scenario/tests.rs"]
mod tests;
#[path = "scenario/upstream.rs"]
mod upstream;

pub(crate) use adapters::*;
pub(crate) use checkpoint::*;
pub(crate) use config::*;
pub(crate) use credentials::*;
pub(crate) use diagnostics::*;
pub(crate) use event_wait::*;
pub(crate) use exercise_follow_up::*;
pub(crate) use exercise_initial::*;
pub(crate) use http::*;
pub(crate) use retry::*;
pub(crate) use seed::*;
pub(crate) use upstream::*;
