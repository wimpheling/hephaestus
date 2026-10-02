use crate::support::Fixture;
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

pub async fn seed(pool: &PgPool, fixture: &Fixture, lease: Uuid) -> Uuid {
    let mailbox = Uuid::new_v4();
    sqlx::query("INSERT INTO mailboxes(id,project_id,instance_id,state) VALUES($1,$2,$3,'active')")
        .bind(mailbox)
        .bind(fixture.consuming_project)
        .bind(fixture.instance)
        .execute(pool)
        .await
        .unwrap();
    let leased = event(pool, fixture, mailbox).await;
    event(pool, fixture, mailbox).await;
    sqlx::query("UPDATE mailbox_deliveries SET disposition='leased' WHERE event_id=$1")
        .bind(leased)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO mailbox_delivery_attempts(id,event_id,mailbox_id,attempt_number,state,command_id,instance_id,instance_revision_id,run_id,state_volume_id,lease_id,lease_fencing_token,state_access_outcome) SELECT gen_random_uuid(),$1,$2,1,'leased',gen_random_uuid(),instance_id,instance_revision_id,id,$4,$5,19,'no_state' FROM runs WHERE id=$3")
        .bind(leased).bind(mailbox).bind(fixture.run).bind(fixture.volume).bind(lease).execute(pool).await.expect("historical exact mailbox scalar triple");
    mailbox
}

async fn event(pool: &PgPool, fixture: &Fixture, mailbox: Uuid) -> Uuid {
    let event = Uuid::new_v4();
    let body = Uuid::new_v4();
    sqlx::query("INSERT INTO mailbox_payloads(id,mailbox_id,project_id,encoded_body,encoded_length,decoded_length,integrity_hash) VALUES($1,$2,$3,'x'::bytea,1,1,$4)").bind(body).bind(mailbox).bind(fixture.consuming_project).bind(Sha256::digest(b"x").to_vec()).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO mailbox_events(id,mailbox_id,project_id,instance_id,body_id,producer_kind,producer_id,deduplication_scope,deduplication_key,method,route,received_at) VALUES($1,$2,$3,$4,$5,'system','fixture','fixture',$6,'POST','/',now())").bind(event).bind(mailbox).bind(fixture.consuming_project).bind(fixture.instance).bind(body).bind(event.to_string()).execute(pool).await.unwrap();
    event
}

pub async fn attempt_snapshot(pool: &PgPool, mailbox: Uuid) -> Value {
    sqlx::query_scalar("SELECT COALESCE(jsonb_agg(to_jsonb(attempt) ORDER BY attempt.id),'[]'::jsonb) FROM mailbox_delivery_attempts attempt WHERE mailbox_id=$1").bind(mailbox).fetch_one(pool).await.unwrap()
}

pub async fn assert_closed(pool: &PgPool, fixture: &Fixture, mailbox: Uuid, before: &Value) {
    assert_eq!(
        attempt_snapshot(pool, mailbox).await,
        *before,
        "running/leased evidence and scalar triple retained until supervised drain"
    );
    let dispositions: Vec<String> = sqlx::query_scalar(
        "SELECT disposition FROM mailbox_deliveries WHERE mailbox_id=$1 ORDER BY disposition",
    )
    .bind(mailbox)
    .fetch_all(pool)
    .await
    .unwrap();
    assert_eq!(
        dispositions,
        vec![String::from("cancelled"), String::from("leased")]
    );
    let state: String = sqlx::query_scalar("SELECT state FROM mailboxes WHERE id=$1")
        .bind(mailbox)
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(state, "paused");
    // Even a trusted worker restoring the mailbox's scheduling state cannot
    // bypass the independent, immutable instance closure admission.
    sqlx::query("UPDATE mailboxes SET state='active' WHERE id=$1")
        .bind(mailbox)
        .execute(pool)
        .await
        .unwrap();
    let mut tx = pool.begin().await.unwrap();
    let body = Uuid::new_v4();
    sqlx::query("INSERT INTO mailbox_payloads(id,mailbox_id,project_id,encoded_body,encoded_length,decoded_length,integrity_hash) VALUES($1,$2,$3,'x'::bytea,1,1,$4)").bind(body).bind(mailbox).bind(fixture.consuming_project).bind(Sha256::digest(b"x").to_vec()).execute(&mut *tx).await.unwrap();
    assert!(sqlx::query("INSERT INTO mailbox_events(id,mailbox_id,project_id,instance_id,body_id,producer_kind,producer_id,deduplication_scope,deduplication_key,method,route,received_at) VALUES(gen_random_uuid(),$1,$2,$3,$4,'system','fixture','closed','closed','POST','/',now())").bind(mailbox).bind(fixture.consuming_project).bind(fixture.instance).bind(body).execute(&mut *tx).await.is_err(),"mailbox admission rejects permanent closure");
    tx.rollback().await.unwrap();
}
