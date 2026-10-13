-- Fixture-only restrictive fault injection; never applied by the public migrator.
CREATE TRIGGER fixture_reject_revision BEFORE INSERT ON agent_instance_revisions FOR EACH ROW EXECUTE FUNCTION fixture_reject_revision();
