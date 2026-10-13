-- Fixture-only restrictive fault injection; never applied by the public migrator.
CREATE FUNCTION fixture_reject_revision() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'fixture revision storage failure' USING ERRCODE='23000'; END $$;
