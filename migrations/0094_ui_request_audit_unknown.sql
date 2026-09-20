-- Extend migration 0093's closed audit vocabulary for operations whose
-- deadline may have elapsed after side effects began. This migration keeps
-- all existing pairs valid and permits exactly one additional pair:
-- decision=undetermined, outcome=unknown.
--
-- Migration 0093's unnamed checks have deterministic PostgreSQL names from
-- the table and column declarations. Do not discover constraints through
-- information_schema pattern matching: the guards below require those exact
-- names, exact constrained columns, and the old predicate tokens before any
-- constraint is dropped.

DO $$
DECLARE
    decision_attnum smallint;
    outcome_attnum smallint;
BEGIN
    SELECT attribute.attnum
      INTO decision_attnum
      FROM pg_catalog.pg_attribute AS attribute
     WHERE attribute.attrelid = 'public.ui_request_audit_events'::pg_catalog.regclass
       AND attribute.attname = 'decision'
       AND NOT attribute.attisdropped;

    SELECT attribute.attnum
      INTO outcome_attnum
      FROM pg_catalog.pg_attribute AS attribute
     WHERE attribute.attrelid = 'public.ui_request_audit_events'::pg_catalog.regclass
       AND attribute.attname = 'outcome'
       AND NOT attribute.attisdropped;

    IF decision_attnum IS NULL OR outcome_attnum IS NULL THEN
        RAISE EXCEPTION
            'migration 0094 requires decision/outcome columns from migration 0093';
    END IF;

    IF NOT EXISTS (
        SELECT 1
          FROM pg_catalog.pg_constraint AS constraint_row
         WHERE constraint_row.conrelid = 'public.ui_request_audit_events'::pg_catalog.regclass
           AND constraint_row.conname = 'ui_request_audit_events_decision_check'
           AND constraint_row.contype = 'c'
           AND constraint_row.conkey = ARRAY[decision_attnum]
           AND pg_catalog.pg_get_constraintdef(constraint_row.oid) LIKE
               '%decision%allowed%denied%'
           AND pg_catalog.pg_get_constraintdef(constraint_row.oid) NOT LIKE
               '%undetermined%'
    ) THEN
        RAISE EXCEPTION
            'migration 0094 found no exact migration 0093 decision CHECK';
    END IF;

    IF NOT EXISTS (
        SELECT 1
          FROM pg_catalog.pg_constraint AS constraint_row
         WHERE constraint_row.conrelid = 'public.ui_request_audit_events'::pg_catalog.regclass
           AND constraint_row.conname = 'ui_request_audit_events_outcome_check'
           AND constraint_row.contype = 'c'
           AND constraint_row.conkey = ARRAY[outcome_attnum]
           AND pg_catalog.pg_get_constraintdef(constraint_row.oid) LIKE
               '%outcome%succeeded%failed%not_attempted%'
           AND pg_catalog.pg_get_constraintdef(constraint_row.oid) NOT LIKE
               '%unknown%'
    ) THEN
        RAISE EXCEPTION
            'migration 0094 found no exact migration 0093 outcome CHECK';
    END IF;

    IF NOT EXISTS (
        SELECT 1
          FROM pg_catalog.pg_constraint AS constraint_row
         WHERE constraint_row.conrelid = 'public.ui_request_audit_events'::pg_catalog.regclass
           AND constraint_row.conname = 'ui_request_audit_events_check'
           AND constraint_row.contype = 'c'
           AND constraint_row.conkey = ARRAY[decision_attnum, outcome_attnum]
           AND pg_catalog.pg_get_constraintdef(constraint_row.oid) LIKE
               '%decision%denied%not_attempted%'
           AND pg_catalog.pg_get_constraintdef(constraint_row.oid) LIKE
               '%decision%allowed%succeeded%failed%'
           AND pg_catalog.pg_get_constraintdef(constraint_row.oid) NOT LIKE
               '%undetermined%'
           AND pg_catalog.pg_get_constraintdef(constraint_row.oid) NOT LIKE
               '%unknown%'
    ) THEN
        RAISE EXCEPTION
            'migration 0094 found no exact migration 0093 decision/outcome CHECK';
    END IF;
END
$$;

ALTER TABLE public.ui_request_audit_events
    DROP CONSTRAINT ui_request_audit_events_decision_check,
    DROP CONSTRAINT ui_request_audit_events_outcome_check,
    DROP CONSTRAINT ui_request_audit_events_check;

ALTER TABLE public.ui_request_audit_events
    ADD CONSTRAINT ui_request_audit_events_decision_check
    CHECK (decision IN ('allowed', 'denied', 'undetermined')),
    ADD CONSTRAINT ui_request_audit_events_outcome_check
    CHECK (outcome IN ('succeeded', 'failed', 'not_attempted', 'unknown')),
    ADD CONSTRAINT ui_request_audit_events_decision_outcome_check
    CHECK (
        (decision = 'denied' AND outcome = 'not_attempted')
        OR (decision = 'allowed' AND outcome IN ('succeeded', 'failed'))
        OR (decision = 'undetermined' AND outcome = 'unknown')
    );
