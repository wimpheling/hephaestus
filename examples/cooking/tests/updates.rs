//! Bounded cooking update and recovery probes.
//!
//! The helper deliberately takes IDs returned by the real build/release path.
//! It does not insert a release, update, revision, or successful run row.  A
//! golden caller supplies ordinary published candidates (migration, explicit
//! rejection, and an abnormal hook) and may then send ingress while the gate
//! is closed before awaiting the assertions below.

use authz_postgres::PostgresMelangeAuthorizer;
use identity_domain::{AuthenticatedIdentity, RequestId, UserId};
use rpc_proto::{
    connect::hephaestus::instance::v1::AgentInstanceServiceClient,
    messages::hephaestus::{
        common::v1::{NetworkPolicy, OpaqueId, RequestContext, RuntimePolicy},
        instance::v1::{
            BrokeredRuleCopy, CreateUpdateRequest, RecoverUpdateRequest, RecoveryAction,
        },
    },
};
use secret_application::RotateSecret;
use secret_domain::{SecretCommandKey, SecretId, SecretValue, SecretVersionId};
use secret_postgres::SecretService;
use secret_store::{EncryptedStore, LocalKeyProvider};
use sqlx::PgPool;
use std::{path::Path, time::Duration};
use tokio::time::{Instant, sleep, timeout};
use uuid::Uuid;

#[path = "updates/operations.rs"]
mod operations;
#[path = "updates/recovery.rs"]
mod recovery;
#[path = "updates/rotation.rs"]
mod rotation;
#[path = "updates/sequence.rs"]
mod sequence;
#[path = "updates/sqlite.rs"]
mod sqlite;
#[path = "updates/tests.rs"]
mod tests;
#[path = "updates/timeout.rs"]
mod timeout;
#[path = "updates/types.rs"]
mod types;

pub(crate) use operations::*;
pub(crate) use recovery::*;
pub(crate) use rotation::*;
pub(crate) use sequence::*;
pub(crate) use sqlite::*;
pub(crate) use timeout::*;
pub(crate) use types::*;
