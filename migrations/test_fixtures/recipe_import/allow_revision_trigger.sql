-- Fixture-only restrictive fault injection; never applied by the public migrator.
DROP TRIGGER fixture_reject_revision ON agent_instance_revisions;
