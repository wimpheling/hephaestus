use crate::{fixtures, seed, support};
use identity_domain::{AuthenticatedIdentity, RequestId};
use recipe_application::{
    AttemptProvenance, CommandIdentity, DeploymentAttemptId, DeploymentIntent, EffectClaim,
    EffectEvidence, EffectOutcome, ResourceAction, ResourceEffect,
};
use recipe_postgres::{
    PostgresDeploymentRepository, PostgresEffectVerificationRecorder, VerifiedOutcome,
};
use sqlx::{PgPool, postgres::PgPoolOptions};
use uuid::Uuid;

pub struct Harness {
    pub app: PgPool,
    pub worker: PgPool,
    pub repository: PostgresDeploymentRepository,
    pub identity: AuthenticatedIdentity,
    pub command: CommandIdentity,
    pub intent: DeploymentIntent,
    pub fixture: support::Fixture,
}

impl Harness {
    pub async fn new(pool: &PgPool, instances: bool, external: bool) -> Self {
        let fixture = seed::seed(pool).await;
        let app = fixtures::app_pool(pool).await;
        let worker = worker_pool(pool).await;
        let repository = PostgresDeploymentRepository::new(app.clone());
        let identity = support::identity(fixture.maintainer);
        let intent = if instances {
            fixtures::intent(&fixture, external)
        } else {
            let full = fixtures::source(&fixture, external);
            let source = full
                .split("[[resources]]\nkind = \"instance\"")
                .next()
                .expect("volume declaration");
            fixtures::build(
                &fixture,
                source,
                recipe_application::DeploymentId::from_uuid(Uuid::new_v4()).expect("id"),
                "execution",
                &std::collections::BTreeMap::new(),
                &[],
                external,
            )
        };
        let install = fixtures::install(&identity, intent.clone());
        let command = install.command;
        repository
            .admit_install(&identity, install)
            .await
            .expect("durable admission");
        Self {
            app,
            worker,
            repository,
            identity,
            command,
            intent,
            fixture,
        }
    }

    pub fn request(&self) -> AuthenticatedIdentity {
        let mut identity = self.identity.clone();
        identity.request_id = RequestId::new();
        identity
    }

    pub async fn effect(
        &self,
        identity: &AuthenticatedIdentity,
        command: CommandIdentity,
        name: &str,
        action: ResourceAction,
    ) -> ResourceEffect {
        let snapshot = self
            .repository
            .inspect(identity, self.intent.id())
            .await
            .expect("current CAS");
        ResourceEffect {
            command,
            deployment_id: self.intent.id(),
            resource: fixtures::key(name),
            expected_deployment_version: snapshot.version,
            expected_resource_version: snapshot.resources[&fixtures::key(name)].version,
            action,
            provenance: AttemptProvenance::new(identity, attempt()).expect("provenance"),
        }
    }

    pub async fn proof(
        &self,
        claim: &EffectClaim,
        verification: DeploymentAttemptId,
        outcome: VerifiedOutcome,
    ) {
        PostgresEffectVerificationRecorder::new(self.worker.clone())
            .record(claim, verification, outcome)
            .await
            .expect("trusted worker ledger fixture");
    }

    pub async fn apply(
        &self,
        identity: &AuthenticatedIdentity,
        command: CommandIdentity,
        name: &str,
        action: ResourceAction,
    ) -> EffectClaim {
        let effect = self.effect(identity, command, name, action).await;
        let claim = self
            .repository
            .claim_resource_effect(identity, effect)
            .await
            .expect("claim");
        self.proof(
            &claim,
            claim.provenance.attempt_id,
            VerifiedOutcome::Applied,
        )
        .await;
        self.repository
            .complete_resource_effect(identity, &claim, evidence(&claim))
            .await
            .expect("verified completion");
        claim
    }
}

pub fn attempt() -> DeploymentAttemptId {
    DeploymentAttemptId::from_uuid(Uuid::new_v4()).expect("attempt")
}
pub const fn evidence(claim: &EffectClaim) -> EffectEvidence {
    EffectEvidence {
        identity: claim.identity,
        input_hash: claim.input_hash,
        outcome: EffectOutcome::Applied,
    }
}

pub async fn worker_pool(pool: &PgPool) -> PgPool {
    PgPoolOptions::new()
        .max_connections(4)
        .after_connect(|connection, _| {
            Box::pin(async move {
                sqlx::query("SET ROLE hephaestus_worker")
                    .execute(connection)
                    .await?;
                Ok(())
            })
        })
        .connect_with(pool.connect_options().as_ref().clone())
        .await
        .expect("worker pool")
}
