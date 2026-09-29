//! Production-path cooking agent authority probe.
//!
//! This helper is intentionally separate from canonical fixture setup. A
//! caller builds/publishes the returned source through the ordinary Git/build
//! path, imports it into a distinct instance, and then runs one policy-denied
//! ingress after the canonical recipe-42 positive control has completed.

use authz_postgres::PostgresMelangeAuthorizer;
use connectrpc::client::ClientConfig;
use forge_domain::RepositoryId;
use hephaestus_app::RunningHephaestus;
use rpc_proto::{
    connect::hephaestus::instance::v1::AgentInstanceServiceClient,
    messages::hephaestus::{
        common::v1::{OpaqueId, ParameterValue, RequestContext},
        instance::v1::BindSecretRequest,
        secret::v1::{DeliveryMode, DeliveryPhase},
    },
};
use secret_application::DeclareBrokeredHttpsRule;
use secret_domain::{AgentSecretBindingId, SecretCommandKey};
use secret_postgres::SecretService;
use secret_store::{EncryptedStore, LocalKeyProvider};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use std::{error::Error, fmt::Write as _, sync::Arc, time::Duration};
use uuid::Uuid;

use super::cooking_builds::{self, CookingBuildContext, PreparedCookingInstance};

#[path = "adversarial_agent/declare.rs"]
mod declare;
#[path = "adversarial_agent/ingress.rs"]
mod ingress;
#[path = "adversarial_agent/prepare.rs"]
mod prepare;
#[path = "adversarial_agent/probe.rs"]
mod probe;
#[path = "adversarial_agent/runtime.rs"]
mod runtime;
#[path = "adversarial_agent/tests.rs"]
mod tests;
#[path = "adversarial_agent/types.rs"]
mod types;

pub(crate) use declare::*;
pub(crate) use ingress::*;
pub(crate) use prepare::*;
pub(crate) use probe::*;
pub(crate) use runtime::*;
pub(crate) use types::*;
