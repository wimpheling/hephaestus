# Purpose

This directory contains the PostgreSQL provider for durable application events.
It gives command handlers a committed mutation receipt and gives workers a
reliable path from database outbox rows to product-event delivery.

# Responsibilities

The provider reads receipts by exact occurrence, actor, aggregate, and scope,
then publishes committed product events from their transactional outbox. It
records publication attempts and terminal failures so retries preserve event
identity and operators can see when delivery needs recovery.

# When

Use the receipt reader when a transport needs to replay a committed mutation.
Run the outbox publisher as a worker after the event transaction commits, and
create the release event stream topology before publishing records to
JetStream.
