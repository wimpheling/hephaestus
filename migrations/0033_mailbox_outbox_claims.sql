-- Mailbox command publication is at-least-once, but concurrent publishers must
-- not both take the same pending record.  A short-lived claim is distinct from
-- broker acknowledgement: `published_at` changes only after JetStream confirms
-- the stable Nats-Msg-Id.

CREATE TABLE mailbox_outbox_claims (
    outbox_id uuid PRIMARY KEY REFERENCES outbox(id) ON DELETE CASCADE,
    claim_token uuid NOT NULL,
    claimed_at timestamptz NOT NULL DEFAULT now(),
    expires_at timestamptz NOT NULL,
    CHECK (expires_at > claimed_at)
);
CREATE INDEX mailbox_outbox_claims_expiry ON mailbox_outbox_claims (expires_at, outbox_id);

ALTER TABLE mailbox_outbox_claims ENABLE ROW LEVEL SECURITY;
ALTER TABLE mailbox_outbox_claims FORCE ROW LEVEL SECURITY;
CREATE POLICY mailbox_outbox_claims_worker ON mailbox_outbox_claims
    TO hephaestus_worker USING (true) WITH CHECK (true);
GRANT SELECT, INSERT, UPDATE, DELETE ON mailbox_outbox_claims TO hephaestus_worker;
