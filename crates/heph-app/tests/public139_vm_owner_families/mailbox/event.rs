//! Bounded opaque external event; no user publish or gateway authority is claimed.

use mailbox_domain::{
    BodyReference, BodyReferenceId, ContentMetadata, DeduplicationKey, EnvelopeMethod,
    EnvelopeRoute, MailboxEnvelope, MailboxEvent, MailboxEventId, MailboxId, ProducerId,
};
use runtime_types::AgentInstanceId;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use time::OffsetDateTime;

pub(super) const BODY: &[u8] = b"ordinary139 mailbox owner history";

pub(super) fn external(mailbox: MailboxId, instance: AgentInstanceId) -> MailboxEvent {
    MailboxEvent {
        id: MailboxEventId::new(),
        mailbox_id: mailbox,
        instance_id: instance,
        producer_id: ProducerId::parse("ordinary139-history-external").unwrap(),
        deduplication_key: DeduplicationKey::parse("first").unwrap(),
        envelope: MailboxEnvelope::new(
            EnvelopeMethod::parse("POST").unwrap(),
            EnvelopeRoute::parse("/mailbox/history").unwrap(),
            BTreeMap::new(),
            ContentMetadata::new(
                BodyReference::new(
                    BodyReferenceId::new(),
                    u32::try_from(BODY.len()).unwrap(),
                    Sha256::digest(BODY).into(),
                )
                .unwrap(),
                Some("application/octet-stream".into()),
                Some("identity".into()),
            )
            .unwrap(),
            OffsetDateTime::now_utc(),
            None,
        )
        .unwrap(),
    }
}
