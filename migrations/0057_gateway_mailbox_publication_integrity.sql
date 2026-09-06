-- A producer identity is a mailbox-local idempotency principal.  Reusing it
-- through a second gateway binding would let that binding observe or collide
-- with the first binding's deduplication scope, so make that ambiguity
-- unrepresentable at the persistence boundary.
ALTER TABLE gateway_mailbox_bindings
    ADD CONSTRAINT gateway_mailbox_bindings_mailbox_producer_unique
        UNIQUE (mailbox_id, producer_id);
