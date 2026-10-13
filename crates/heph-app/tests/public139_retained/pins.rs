use super::fixture::Fixture;
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Clone)]
pub struct Pins {
    closure: Uuid,
    operation: Uuid,
    generation: i64,
    purpose: Vec<u8>,
    namespace: String,
    host: String,
    retention: Uuid,
}
impl Pins {
    pub fn genuine(f: &Fixture) -> Self {
        let fields = f.retained.retained.closure.fields();
        Self {
            closure: fields.id,
            operation: f.original.claim.operation_id,
            generation: i64::try_from(f.original.claim.generation).unwrap(),
            purpose: f.original.claim.purpose.hash().to_vec(),
            namespace: fields.vm_provider_namespace.clone(),
            host: fields.vm_host_id.clone(),
            retention: f.retained.retention_operation_id,
        }
    }
    pub fn substitute(&mut self, slot: usize) {
        match slot {
            0 => self.closure = Uuid::new_v4(),
            1 => self.operation = Uuid::new_v4(),
            2 => self.generation += 1,
            3 => self.purpose[0] ^= 1,
            4 => self.namespace.push('x'),
            5 => self.host.push('x'),
            6 => self.retention = Uuid::new_v4(),
            _ => panic!("unknown pin"),
        }
    }
}

pub async fn check(worker: &PgPool, pins: &Pins, accepted: bool) {
    let mut tx = worker.begin().await.unwrap();
    let result = async {
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *tx)
            .await?;
        let role: String = sqlx::query_scalar("SELECT current_user::text")
            .fetch_one(&mut *tx)
            .await?;
        let outcome =
            sqlx::query("SELECT public.assert_retained_owned_reuse_context($1,$2,$3,$4,$5,$6,$7)")
                .bind(pins.closure)
                .bind(pins.operation)
                .bind(pins.generation)
                .bind(&pins.purpose)
                .bind(&pins.namespace)
                .bind(&pins.host)
                .bind(pins.retention)
                .execute(&mut *tx)
                .await;
        Ok::<_, sqlx::Error>((role, outcome))
    }
    .await;
    let rollback = tx.rollback().await;
    let (role, outcome) = result.unwrap();
    rollback.unwrap();
    assert_eq!(role, "hephaestus_worker");
    if accepted {
        outcome.unwrap();
    } else {
        denied(&outcome.unwrap_err());
    }
}

pub async fn old121_rejects(worker: &PgPool, pins: &Pins) {
    let mut tx = worker.begin().await.unwrap();
    let result = async {
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *tx)
            .await?;
        Ok::<_, sqlx::Error>(
            sqlx::query("SELECT public.assert_owned_ready_retention($1,$2,$3,$4,$5,$6)")
                .bind(pins.closure)
                .bind(pins.operation)
                .bind(pins.generation)
                .bind(&pins.purpose)
                .bind(&pins.namespace)
                .bind(&pins.host)
                .execute(&mut *tx)
                .await,
        )
    }
    .await;
    let rollback = tx.rollback().await;
    let outcome = result.unwrap();
    rollback.unwrap();
    denied(&outcome.unwrap_err());
}

fn denied(error: &sqlx::Error) {
    assert_eq!(
        error.as_database_error().unwrap().code().as_deref(),
        Some("23000")
    );
}
