//! Real import completion then current typed bindings through app RLS.
use sqlx::PgPool;
use std::ops::Deref;

pub struct Bootstrap(crate::authority_bootstrap::Bootstrap);
impl Bootstrap {
    pub async fn new(pool: &PgPool, count: usize, sibling: bool) -> Self {
        Self(
            Box::pin(crate::authority_bootstrap::Bootstrap::new(
                pool, count, sibling,
            ))
            .await,
        )
    }
    pub async fn complete(&self, claim: &recipe_application::EffectClaim) {
        self.0.complete(claim).await;
        crate::qualified_invocation::bindings::establish(&self.0).await;
    }
}
impl Deref for Bootstrap {
    type Target = crate::authority_bootstrap::Bootstrap;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
