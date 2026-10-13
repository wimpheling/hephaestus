use crate::resume_fixture;
use recipe_application::{ExistingInstanceClosureRequest, RemoveResumeConfiguration};
use release_postgres::PostgresPreparedInstanceSafetyRepository;
use sqlx::PgPool;
use vm_trait::VmProviderOwnerScope;

pub struct Fixture {
    pub seed: resume_fixture::Fixture,
    pub request: ExistingInstanceClosureRequest,
}
impl Fixture {
    pub async fn new(pool: &PgPool) -> Self {
        let seed = resume_fixture::Fixture::new(pool).await;
        seed.withdraw_original().await;
        let admitted = seed
            .repository()
            .admit_remove_resume(&seed.manager, seed.remove)
            .await
            .unwrap();
        let closure = admitted.continuations()[0].closure.clone().unwrap();
        let request = ExistingInstanceClosureRequest::new(
            seed.remove.command,
            seed.closure.original.base.intent.clone(),
            seed.closure.original.target.clone(),
            closure,
        )
        .unwrap();
        sqlx::raw_sql(include_str!(
            "../../../../../../../migrations/0130_existing_instance_closure.sql"
        ))
        .execute(pool)
        .await
        .unwrap();
        Self { seed, request }
    }
    pub fn service(&self) -> PostgresPreparedInstanceSafetyRepository {
        self.seed
            .closure
            .original
            .service()
            .with_remove_resume(self.seed.config.clone())
            .unwrap()
    }
    pub fn foreign(&self) -> PostgresPreparedInstanceSafetyRepository {
        // Checked labels are fixture comparison data, not an actual VM owner proof.
        let scope =
            VmProviderOwnerScope::new("foreign-owner".into(), self.seed.config.host().into())
                .unwrap();
        let config = RemoveResumeConfiguration::new(
            scope.namespace().into(),
            scope.host_id().into(),
            self.seed.config.platform().clone(),
            self.seed.config.profile(),
        )
        .unwrap();
        PostgresPreparedInstanceSafetyRepository::new(
            self.seed.closure.original.base.app.clone(),
            self.seed.closure.original.base.worker.clone(),
            &scope,
            self.seed.config.platform().clone(),
        )
        .with_remove_resume(config)
        .unwrap()
    }
}
