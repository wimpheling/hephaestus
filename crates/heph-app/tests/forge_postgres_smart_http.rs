//! Opt-in native smart-HTTP and `PostgreSQL` integration coverage.

use async_trait::async_trait;
use authz_postgres::PostgresMelangeAuthorizer;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use capability_domain::{RuntimeCredentialGeneration, RuntimeSessionId};
use forge_domain::{GitRef, OrganizationId};
use forge_postgres::PgForgeRepository;
use forge_service::{CreateRepository, GitStorage, RUN_START_SUBJECT};
use git_capability_domain::{
    BoundGitCapability, BranchRefPolicy, BranchUpdatePolicy, ChangedPathGlob, GitCapabilityCeiling,
    GitCapabilityCeilingInput, GitOperation as CapabilityGitOperation, RefGlob,
    RefMutationPermission, RefNamespacePolicy, RefUpdatePolicy,
    RepositoryId as CapabilityRepositoryId, TransferLimits,
};
use git_http::{
    AuthenticationError, AuthorizationError, AuthorizationRequest, CompositeGitAuthenticator,
    GitAuthenticator, GitAuthorizer, GitHttpLimits, GitHttpService, GitOperation,
    PostgresGitAuthorizer, Principal, RuntimeGitHttpAuthenticator,
};
use identity_domain::{AuthenticatedIdentity, RequestId, UserId};
use pat_domain::{PersonalAccessTokenLabel, PersonalAccessTokenScope};
use pat_postgres::{CreatePersonalAccessToken, PostgresPersonalAccessTokenService};
use runtime_git_authority::{RuntimeGitCredential, RuntimeGitCredentialIssuer};
use runtime_git_authority_postgres::PgRuntimeGitCredentialRepository;
use runtime_handoff_local::EncryptedFileRuntimeGitHandoffStore;
use serde_json::json;
use serial_test::serial;
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::{process::Command, sync::Mutex};
use uuid::Uuid;

#[path = "forge_postgres_smart_http/support/mod.rs"]
mod support;
use support::*;

#[path = "forge_postgres_smart_http/authorization.rs"]
mod authorization;
use authorization::*;
#[path = "forge_postgres_smart_http/clone_http.rs"]
mod clone_http;
