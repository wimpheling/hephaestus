//! Mailbox publication request and actor-role helpers.

use super::fixture::Fixture;
use gateway_postgres::GatewayMailboxPublicationRequest;
use mailbox_domain::{
    BodyReference, BodyReferenceId, ContentMetadata, DeduplicationKey, EnvelopeMethod,
    EnvelopeRoute, MailboxEnvelope,
};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use time::OffsetDateTime;
use uuid::Uuid;

pub async fn set_actor_app_role(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    actor: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "SELECT set_config('hephaestus.actor_id', $1, true),
                set_config('hephaestus.subject_type', 'user', true)",
    )
    .bind(actor.to_string())
    .execute(&mut **transaction)
    .await?;
    sqlx::query("SET LOCAL ROLE hephaestus_app")
        .execute(&mut **transaction)
        .await?;
    Ok(())
}

pub fn request(fixture: &Fixture, slot: &str, key: &str) -> GatewayMailboxPublicationRequest {
    let body = b"gateway-postgres-proof".to_vec();
    let body_reference = BodyReference::new(
        BodyReferenceId::new(),
        u32::try_from(body.len()).expect("body fits bounded envelope"),
        Sha256::digest(&body).into(),
    )
    .expect("body reference");
    GatewayMailboxPublicationRequest {
        runtime_session_id: fixture.session,
        invocation_id: fixture.invocation,
        slot_key: slot.to_owned(),
        deduplication_key: DeduplicationKey::parse(key).expect("deduplication key"),
        envelope: MailboxEnvelope::new(
            EnvelopeMethod::parse("POST").expect("method"),
            EnvelopeRoute::parse("/telegram/update").expect("route"),
            BTreeMap::default(),
            ContentMetadata::new(
                body_reference,
                Some("application/json".to_owned()),
                Some("identity".to_owned()),
            )
            .expect("content"),
            OffsetDateTime::now_utc(),
            None,
        )
        .expect("envelope"),
        decoded_length: u32::try_from(body.len()).expect("body length"),
        encoded_body: body,
    }
}
