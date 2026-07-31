-- Durable application events are PostgreSQL authority. The legacy outbox is
-- retained solely for internal commands and workflow signals; it can never be
-- interpreted as a client-facing product event.

ALTER TABLE outbox ADD COLUMN message_class text
GENERATED ALWAYS AS (
    CASE
        WHEN subject IN (
            'hephaestus.build.requested.v1',
            'hephaestus.instance.run.requested.v1',
            'hephaestus.run.start',
            'heph.run.command.start.v1',
            'heph.run.command.cancel.v1',
            'hephaestus.control.execute'
        ) THEN 'internal_command'
        ELSE 'internal_signal'
    END
) STORED;
ALTER TABLE outbox ADD CONSTRAINT outbox_internal_message_class
    CHECK (message_class IN ('internal_command', 'internal_signal'));

-- These legacy informational lifecycle messages are superseded by the
-- canonical product-event journal. Actionable commands and unrelated audit
-- signals remain pending and continue through their owning adapters.
UPDATE outbox
SET published_at = now(),
    attempts = attempts + 1,
    last_error = 'retired by canonical product-event migration'
WHERE published_at IS NULL AND subject IN (
    'hephaestus.build.completed.v1',
    'hephaestus.build.failed.v1',
    'hephaestus.git.receive.accepted',
    'hephaestus.git.agent_config.invalid',
    'heph.run.event.lifecycle.v1',
    'hephaestus.release.published.v1',
    'hephaestus.release.revoked.v1',
    'hephaestus.agent_instance.created.v1',
    'hephaestus.agent_instance.revised.v1',
    'hephaestus.agent_instance.attachment_changed.v1',
    'hephaestus.agent_instance.paused.v1',
    'hephaestus.agent_update.requested.v1',
    'hephaestus.agent_update.hook_started.v1',
    'hephaestus.agent_update.hook_committed.v1',
    'hephaestus.agent_update.completed.v1',
    'hephaestus.agent_update.rejected.v1',
    'hephaestus.agent_update.uncertain.v1',
    'hephaestus.agent_update.recovered.v1',
    'hephaestus.secret.created.v1',
    'hephaestus.secret.rotated.v1',
    'hephaestus.secret.granted.v1',
    'hephaestus.secret.imported.v1',
    'hephaestus.secret.bound.v1',
    'hephaestus.secret.runtime_authority_issued.v1',
    'hephaestus.secret.enabled.v1',
    'hephaestus.secret.disabled.v1',
    'hephaestus.secret.purged.v1',
    'hephaestus.secret.reconcile_revocation.v1'
);

CREATE TABLE application_event_scopes (
    scope_kind text NOT NULL CHECK (
        scope_kind IN (
            'identity', 'organization', 'project', 'repository',
            'run', 'agent_instance'
        )
    ),
    scope_id uuid NOT NULL,
    committed_cursor bigint NOT NULL DEFAULT 0
        CHECK (committed_cursor >= 0),
    retained_from_cursor bigint NOT NULL DEFAULT 1
        CHECK (retained_from_cursor > 0),
    retention_interval interval NOT NULL DEFAULT interval '30 days'
        CHECK (retention_interval >= interval '1 hour'),
    updated_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (scope_kind, scope_id),
    CHECK (retained_from_cursor <= committed_cursor + 1)
);

CREATE TABLE application_aggregate_versions (
    scope_kind text NOT NULL,
    scope_id uuid NOT NULL,
    aggregate_type text NOT NULL CHECK (
        length(aggregate_type) BETWEEN 1 AND 96
        AND aggregate_type ~ '^[a-z][a-z0-9_]*$'
    ),
    aggregate_id uuid NOT NULL,
    aggregate_version bigint NOT NULL CHECK (aggregate_version > 0),
    PRIMARY KEY (scope_kind, scope_id, aggregate_type, aggregate_id),
    FOREIGN KEY (scope_kind, scope_id)
        REFERENCES application_event_scopes(scope_kind, scope_id)
        ON DELETE CASCADE
);

CREATE TABLE application_events (
    id uuid PRIMARY KEY,
    occurrence_id uuid NOT NULL,
    scope_kind text NOT NULL,
    scope_id uuid NOT NULL,
    cursor bigint NOT NULL CHECK (cursor > 0),
    aggregate_type text NOT NULL CHECK (
        aggregate_type IN (
            'identity_profile', 'identity_organizations', 'organization',
            'project', 'repository', 'repository_ref', 'build', 'release',
            'agent_instance', 'run', 'review', 'secret_metadata',
            'secret_grant', 'secret_import', 'agent_secret_binding', 'artifact'
        )
    ),
    aggregate_id uuid NOT NULL,
    aggregate_version bigint NOT NULL CHECK (aggregate_version > 0),
    event_type text NOT NULL CHECK (
        length(event_type) BETWEEN 1 AND 128
        AND event_type ~ '^[a-z][a-z0-9_.]*$'
    ),
    schema_version integer NOT NULL DEFAULT 1 CHECK (schema_version > 0),
    change_kind text NOT NULL CHECK (
        change_kind IN ('created', 'updated', 'state_changed', 'removed')
    ),
    safe_state text CHECK (
        safe_state IS NULL OR safe_state IN (
            'pending', 'queued', 'running', 'active', 'paused', 'succeeded',
            'failed', 'published', 'revoked', 'disabled', 'rejected',
            'conflicted', 'removed'
        )
    ),
    related_id_one uuid,
    related_id_two uuid,
    actor_type text NOT NULL CHECK (actor_type IN ('user', 'system')),
    actor_id uuid,
    request_id uuid,
    occurred_at timestamptz NOT NULL DEFAULT now(),
    retained_until timestamptz NOT NULL,
    UNIQUE (scope_kind, scope_id, cursor),
    FOREIGN KEY (scope_kind, scope_id)
        REFERENCES application_event_scopes(scope_kind, scope_id),
    FOREIGN KEY (
        scope_kind, scope_id, aggregate_type, aggregate_id
    ) REFERENCES application_aggregate_versions(
        scope_kind, scope_id, aggregate_type, aggregate_id
    ),
    CHECK (
        (actor_type = 'user' AND actor_id IS NOT NULL)
        OR (actor_type = 'system' AND actor_id IS NULL)
    ),
    CHECK (
        (aggregate_type, event_type) IN (
            ('identity_profile', 'identity.profile_changed'),
            ('identity_organizations', 'identity.organizations_changed'),
            ('organization', 'organization.changed'),
            ('project', 'project.changed'),
            ('repository', 'repository.changed'),
            ('repository_ref', 'repository.ref_changed'),
            ('build', 'build.changed'),
            ('release', 'release.changed'),
            ('agent_instance', 'agent_instance.changed'),
            ('run', 'run.changed'),
            ('review', 'review.changed'),
            ('secret_metadata', 'secret_metadata.changed'),
            ('secret_grant', 'secret_grant.changed'),
            ('secret_import', 'secret_import.changed'),
            ('agent_secret_binding', 'agent_secret_binding.changed'),
            ('artifact', 'artifact.changed')
        )
    ),
    CHECK (
        CASE aggregate_type
            WHEN 'project' THEN related_id_one IS NOT NULL
            WHEN 'repository' THEN related_id_one IS NOT NULL
            WHEN 'build' THEN related_id_one IS NOT NULL
            WHEN 'release' THEN related_id_one IS NOT NULL
            WHEN 'review' THEN related_id_one IS NOT NULL
            WHEN 'secret_metadata' THEN related_id_one IS NOT NULL
            WHEN 'secret_grant' THEN related_id_one IS NOT NULL
                AND related_id_two IS NOT NULL
            WHEN 'secret_import' THEN related_id_one IS NOT NULL
                AND related_id_two IS NOT NULL
            WHEN 'agent_secret_binding' THEN related_id_one IS NOT NULL
                AND related_id_two IS NOT NULL
            WHEN 'artifact' THEN related_id_one IS NOT NULL
                AND related_id_two IS NOT NULL
            ELSE true
        END
    )
);
CREATE INDEX application_events_retention
    ON application_events (retained_until, scope_kind, scope_id, cursor);
CREATE INDEX application_events_occurrence
    ON application_events (occurrence_id, scope_kind, scope_id);

CREATE TABLE product_event_outbox (
    event_id uuid PRIMARY KEY REFERENCES application_events(id) ON DELETE CASCADE,
    subject text NOT NULL DEFAULT 'hephaestus.product.event.v1'
        CHECK (subject = 'hephaestus.product.event.v1'),
    created_at timestamptz NOT NULL DEFAULT now(),
    published_at timestamptz,
    dead_lettered_at timestamptz,
    attempts integer NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    last_error text CHECK (last_error IS NULL OR octet_length(last_error) <= 2048),
    terminal_reason text CHECK (
        terminal_reason IS NULL OR octet_length(terminal_reason) <= 2048
    ),
    CHECK (num_nonnulls(published_at, dead_lettered_at) <= 1),
    CHECK ((dead_lettered_at IS NULL) = (terminal_reason IS NULL))
);
CREATE INDEX unpublished_product_event_outbox
    ON product_event_outbox (created_at, event_id)
    WHERE published_at IS NULL AND dead_lettered_at IS NULL;

-- Terminal projection failures retain non-disclosing diagnostics even after
-- the ordinary application-event retention window deletes the source row.
CREATE TABLE product_event_dead_letters (
    event_id uuid PRIMARY KEY,
    scope_kind text NOT NULL,
    scope_id uuid NOT NULL,
    cursor bigint NOT NULL CHECK (cursor > 0),
    aggregate_type text NOT NULL,
    aggregate_id uuid NOT NULL,
    aggregate_version bigint NOT NULL CHECK (aggregate_version > 0),
    terminal_reason text NOT NULL CHECK (octet_length(terminal_reason) <= 2048),
    dead_lettered_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (scope_kind, scope_id, cursor)
);

CREATE FUNCTION enqueue_product_event() RETURNS trigger
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = public, pg_temp
AS $$
BEGIN
    INSERT INTO product_event_outbox (event_id) VALUES (NEW.id);
    RETURN NEW;
END
$$;
REVOKE ALL ON FUNCTION enqueue_product_event() FROM PUBLIC;
CREATE TRIGGER application_event_product_outbox
AFTER INSERT ON application_events
FOR EACH ROW EXECUTE FUNCTION enqueue_product_event();

-- Converts all durable source-table states to the finite safe lifecycle
-- vocabulary in ProductEvent. Unknown state text aborts the state mutation.
CREATE FUNCTION normalize_application_event_state(p_state text) RETURNS text
LANGUAGE plpgsql
IMMUTABLE
AS $$
DECLARE
    v_normalized text;
BEGIN
    IF p_state IS NULL OR p_state = '' THEN RETURN NULL; END IF;
    v_normalized := CASE
        WHEN p_state IN ('pending', 'draft', 'preparing', 'uninitialized', 'open')
            THEN 'pending'
        WHEN p_state IN ('queued', 'candidate', 'deferred', 'claimed', 'dispatched')
            THEN 'queued'
        WHEN p_state IN (
            'running', 'importing', 'processing', 'provisioning', 'starting',
            'leasing_volume', 'update_draining', 'updating', 'recovering',
            'approval_requested', 'hook_running', 'activation_recovery',
            'draining', 'finalize_requested'
        ) THEN 'running'
        WHEN p_state IN (
            'active', 'accepted', 'ready', 'attached', 'materialized',
            'valid', 'prepared', 'sealed', 'imported', 'drafted',
            'hook_committed', 'ref_published'
        ) THEN 'active'
        WHEN p_state IN ('paused_unknown_state', 'paused_activation_recovery', 'recovery_required')
            THEN 'paused'
        WHEN p_state IN (
            'succeeded', 'completed', 'approved', 'activated', 'released',
            'cleaned', 'cleaned_up'
        ) THEN 'succeeded'
        WHEN p_state IN (
            'failed', 'denied', 'cancelled', 'materialization_failed',
            'seal_failed', 'abandoned'
        ) THEN 'failed'
        WHEN p_state = 'published' THEN 'published'
        WHEN p_state IN ('revoked', 'expired') THEN 'revoked'
        WHEN p_state IN ('disabled', 'suspended') THEN 'disabled'
        WHEN p_state IN (
            'rejected', 'update_rejected', 'agent_rejected', 'invalid',
            'import_rejected'
        ) THEN 'rejected'
        WHEN p_state IN ('conflicted', 'compatibility_unknown', 'superseded')
            THEN 'conflicted'
        WHEN p_state IN (
            'removed', 'tombstoned', 'purged', 'cleaning_up', 'destroyed'
        ) THEN 'removed'
        ELSE NULL
    END CASE;
    IF v_normalized IS NULL THEN
        RAISE EXCEPTION 'unknown application event state %', p_state;
    END IF;
    RETURN v_normalized;
END
$$;
REVOKE ALL ON FUNCTION normalize_application_event_state(text) FROM PUBLIC;

-- Allocates both cursor and aggregate version while holding the corresponding
-- scope rows. Callers invoke this inside the same transaction as state change.
CREATE FUNCTION append_application_event(
    p_occurrence_id uuid,
    p_scope_kind text,
    p_scope_id uuid,
    p_aggregate_type text,
    p_aggregate_id uuid,
    p_event_type text,
    p_change_kind text,
    p_safe_state text DEFAULT NULL,
    p_related_id_one uuid DEFAULT NULL,
    p_related_id_two uuid DEFAULT NULL
) RETURNS TABLE (event_id uuid, cursor bigint, aggregate_version bigint)
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = public, pg_temp
AS $$
DECLARE
    v_actor_text text;
    v_request_text text;
    v_actor_type text := 'system';
    v_actor_id uuid;
    v_request_id uuid;
    v_retention interval;
BEGIN
    IF p_occurrence_id IS NULL OR p_scope_id IS NULL OR p_aggregate_id IS NULL THEN
        RAISE EXCEPTION 'application event identity is required';
    END IF;

    INSERT INTO application_event_scopes (scope_kind, scope_id)
    VALUES (p_scope_kind, p_scope_id)
    ON CONFLICT (scope_kind, scope_id) DO NOTHING;

    UPDATE application_event_scopes
    SET committed_cursor = committed_cursor + 1, updated_at = now()
    WHERE scope_kind = p_scope_kind AND scope_id = p_scope_id
    RETURNING committed_cursor, retention_interval
    INTO cursor, v_retention;

    INSERT INTO application_aggregate_versions (
        scope_kind, scope_id, aggregate_type, aggregate_id, aggregate_version
    ) VALUES (
        p_scope_kind, p_scope_id, p_aggregate_type, p_aggregate_id, 1
    )
    ON CONFLICT (scope_kind, scope_id, aggregate_type, aggregate_id)
    DO UPDATE SET aggregate_version =
        application_aggregate_versions.aggregate_version + 1
    RETURNING application_aggregate_versions.aggregate_version
    INTO aggregate_version;

    v_actor_text := NULLIF(current_setting('hephaestus.actor_id', true), '');
    v_request_text := NULLIF(current_setting('hephaestus.request_id', true), '');
    IF current_setting('hephaestus.subject_type', true) = 'user'
       AND v_actor_text ~* '^[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$'
    THEN
        v_actor_type := 'user';
        v_actor_id := v_actor_text::uuid;
    END IF;
    IF v_request_text ~* '^[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$'
    THEN
        v_request_id := v_request_text::uuid;
    END IF;

    event_id := gen_random_uuid();
    INSERT INTO application_events (
        id, occurrence_id, scope_kind, scope_id, cursor,
        aggregate_type, aggregate_id, aggregate_version, event_type,
        change_kind, safe_state, related_id_one, related_id_two,
        actor_type, actor_id, request_id,
        retained_until
    ) VALUES (
        event_id, p_occurrence_id, p_scope_kind, p_scope_id, cursor,
        p_aggregate_type, p_aggregate_id, aggregate_version, p_event_type,
        p_change_kind, normalize_application_event_state(p_safe_state),
        p_related_id_one, p_related_id_two,
        v_actor_type, v_actor_id, v_request_id,
        now() + v_retention
    );
    RETURN NEXT;
END
$$;
REVOKE ALL ON FUNCTION append_application_event(
    uuid, text, uuid, text, uuid, text, text, text, uuid, uuid
) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION append_application_event(
    uuid, text, uuid, text, uuid, text, text, text, uuid, uuid
) TO hephaestus_app, hephaestus_worker;

-- Retention never reuses a cursor. The retained lower bound is advanced in
-- the same transaction that deletes expired records, so resume gaps are exact.
CREATE FUNCTION prune_application_events(p_limit integer DEFAULT 1000)
RETURNS integer
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = public, pg_temp
AS $$
DECLARE
    v_count integer;
BEGIN
    IF p_limit < 1 OR p_limit > 10000 THEN
        RAISE EXCEPTION 'invalid application event prune limit';
    END IF;
    WITH expired AS (
        SELECT id, scope_kind, scope_id, cursor
        FROM application_events
        WHERE retained_until <= now()
          AND NOT EXISTS (
              SELECT 1 FROM product_event_outbox outbox
              WHERE outbox.event_id = application_events.id
                AND outbox.published_at IS NULL
                AND outbox.dead_lettered_at IS NULL
          )
        ORDER BY retained_until, scope_kind, scope_id, cursor
        LIMIT p_limit
        FOR UPDATE SKIP LOCKED
    ), deleted AS (
        DELETE FROM application_events event
        USING expired
        WHERE event.id = expired.id
        RETURNING expired.scope_kind, expired.scope_id, expired.cursor
    ), bounds AS (
        SELECT scope_kind, scope_id, max(cursor) + 1 AS retained_from_cursor
        FROM deleted GROUP BY scope_kind, scope_id
    ), updated_bounds AS (
        UPDATE application_event_scopes scope
        SET retained_from_cursor = greatest(
                scope.retained_from_cursor, bounds.retained_from_cursor
            ),
            updated_at = now()
        FROM bounds
        WHERE scope.scope_kind = bounds.scope_kind
          AND scope.scope_id = bounds.scope_id
        RETURNING scope.scope_id
    )
    SELECT count(*)::integer INTO v_count FROM deleted
    WHERE EXISTS (SELECT 1 FROM updated_bounds);
    RETURN v_count;
END
$$;
REVOKE ALL ON FUNCTION prune_application_events(integer) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION prune_application_events(integer)
    TO hephaestus_worker;

GRANT SELECT ON application_event_scopes, application_events
    TO hephaestus_app, hephaestus_worker;
GRANT SELECT ON application_aggregate_versions
    TO hephaestus_app, hephaestus_worker;
GRANT SELECT, UPDATE ON product_event_outbox
    TO hephaestus_app, hephaestus_worker;
GRANT SELECT, INSERT ON product_event_dead_letters
    TO hephaestus_app, hephaestus_worker;

-- Generic direct route. Arguments are aggregate type, aggregate-id column,
-- scope kind, scope-id column, event type, and optional safe-state column.
-- It intentionally projects no row JSON into the durable event.
CREATE FUNCTION capture_direct_application_event() RETURNS trigger
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = public, pg_temp
AS $$
DECLARE
    v_row jsonb := CASE WHEN TG_OP = 'DELETE' THEN to_jsonb(OLD) ELSE to_jsonb(NEW) END;
    v_change_kind text := CASE TG_OP
        WHEN 'INSERT' THEN 'created'
        WHEN 'UPDATE' THEN 'updated'
        ELSE 'removed'
    END;
    v_aggregate_id uuid;
    v_scope_id uuid;
    v_safe_state text;
    v_related_id_one uuid;
    v_related_id_two uuid;
    v_occurrence uuid := COALESCE(
        NULLIF(current_setting('hephaestus.occurrence_id', true), '')::uuid,
        gen_random_uuid()
    );
BEGIN
    v_aggregate_id := (v_row ->> TG_ARGV[1])::uuid;
    v_scope_id := (v_row ->> TG_ARGV[3])::uuid;
    IF TG_NARGS > 5 AND TG_ARGV[5] <> '' THEN
        v_safe_state := left(v_row ->> TG_ARGV[5], 128);
        IF TG_OP = 'UPDATE'
           AND to_jsonb(OLD) ->> TG_ARGV[5]
               IS DISTINCT FROM to_jsonb(NEW) ->> TG_ARGV[5]
        THEN
            v_change_kind := 'state_changed';
        END IF;
    END IF;
    IF TG_NARGS > 6 AND TG_ARGV[6] <> '' THEN
        v_related_id_one := (v_row ->> TG_ARGV[6])::uuid;
    END IF;
    IF TG_NARGS > 7 AND TG_ARGV[7] <> '' THEN
        v_related_id_two := (v_row ->> TG_ARGV[7])::uuid;
    END IF;
    -- Child-table invalidations inherit the typed parent identifiers required
    -- by ProductEvent payloads. No parent row content is copied.
    IF TG_ARGV[0] = 'project' AND v_related_id_one IS NULL THEN
        SELECT organization_id INTO v_related_id_one
        FROM projects WHERE id = v_aggregate_id;
    ELSIF TG_ARGV[0] = 'repository' AND v_related_id_one IS NULL THEN
        SELECT project_id INTO v_related_id_one
        FROM repositories WHERE id = v_aggregate_id;
    ELSIF TG_ARGV[0] = 'agent_instance' AND v_related_id_one IS NULL THEN
        SELECT project_id INTO v_related_id_one
        FROM agent_instances WHERE id = v_aggregate_id;
    ELSIF TG_ARGV[0] = 'run' THEN
        IF v_related_id_one IS NULL THEN
            SELECT instance.project_id INTO v_related_id_one
            FROM runs run JOIN agent_instances instance ON instance.id = run.instance_id
            WHERE run.id = v_aggregate_id;
        END IF;
        IF v_related_id_two IS NULL THEN
            SELECT repository_id INTO v_related_id_two
            FROM run_requests WHERE run_id = v_aggregate_id;
        END IF;
    END IF;
    PERFORM append_application_event(
        v_occurrence, TG_ARGV[2], v_scope_id, TG_ARGV[0],
        v_aggregate_id, TG_ARGV[4], v_change_kind, v_safe_state,
        v_related_id_one, v_related_id_two
    );
    RETURN CASE WHEN TG_OP = 'DELETE' THEN OLD ELSE NEW END;
END
$$;
REVOKE ALL ON FUNCTION capture_direct_application_event() FROM PUBLIC;

-- Identity events deliberately contain only aggregate identity, change kind,
-- and normalized state. Profile and provider PII never enters this journal.
CREATE TRIGGER users_application_event
AFTER INSERT OR UPDATE OR DELETE ON users
FOR EACH ROW EXECUTE FUNCTION capture_direct_application_event(
    'identity_profile', 'id', 'identity', 'id',
    'identity.profile_changed', ''
);
CREATE TRIGGER user_profiles_application_event
AFTER INSERT OR UPDATE OR DELETE ON user_profiles
FOR EACH ROW EXECUTE FUNCTION capture_direct_application_event(
    'identity_profile', 'user_id', 'identity', 'user_id',
    'identity.profile_changed', ''
);
CREATE TRIGGER external_identities_application_event
AFTER INSERT OR UPDATE OR DELETE ON external_identities
FOR EACH ROW EXECUTE FUNCTION capture_direct_application_event(
    'identity_profile', 'user_id', 'identity', 'user_id',
    'identity.profile_changed', ''
);
CREATE TRIGGER organizations_application_event
AFTER INSERT OR UPDATE OR DELETE ON organizations
FOR EACH ROW EXECUTE FUNCTION capture_direct_application_event(
    'organization', 'id', 'organization', 'id', 'organization.changed', ''
);
CREATE TRIGGER organization_members_identity_application_event
AFTER INSERT OR UPDATE OR DELETE ON organization_members
FOR EACH ROW EXECUTE FUNCTION capture_direct_application_event(
    'identity_organizations', 'organization_id', 'identity', 'user_id',
    'identity.organizations_changed', '', 'organization_id'
);
CREATE TRIGGER organization_members_organization_application_event
AFTER INSERT OR UPDATE OR DELETE ON organization_members
FOR EACH ROW EXECUTE FUNCTION capture_direct_application_event(
    'organization', 'organization_id', 'organization', 'organization_id',
    'organization.changed', ''
);
CREATE TRIGGER organization_secret_managers_organization_application_event
AFTER INSERT OR UPDATE OR DELETE ON organization_secret_managers
FOR EACH ROW EXECUTE FUNCTION capture_direct_application_event(
    'organization', 'organization_id', 'organization', 'organization_id',
    'organization.changed', ''
);
CREATE TRIGGER organization_secret_managers_identity_application_event
AFTER INSERT OR UPDATE OR DELETE ON organization_secret_managers
FOR EACH ROW EXECUTE FUNCTION capture_direct_application_event(
    'identity_organizations', 'organization_id', 'identity', 'user_id',
    'identity.organizations_changed', '', 'organization_id'
);
CREATE TRIGGER projects_project_application_event
AFTER INSERT OR UPDATE OR DELETE ON projects
FOR EACH ROW EXECUTE FUNCTION capture_direct_application_event(
    'project', 'id', 'project', 'id', 'project.changed', '', 'organization_id'
);
CREATE TRIGGER projects_organization_application_event
AFTER INSERT OR UPDATE OR DELETE ON projects
FOR EACH ROW EXECUTE FUNCTION capture_direct_application_event(
    'project', 'id', 'organization', 'organization_id', 'project.changed', '', 'organization_id'
);
CREATE TRIGGER project_maintainers_application_event
AFTER INSERT OR UPDATE OR DELETE ON project_maintainers
FOR EACH ROW EXECUTE FUNCTION capture_direct_application_event(
    'project', 'project_id', 'project', 'project_id', 'project.changed', ''
);
CREATE TRIGGER project_secret_roles_application_event
AFTER INSERT OR UPDATE OR DELETE ON project_secret_roles
FOR EACH ROW EXECUTE FUNCTION capture_direct_application_event(
    'project', 'project_id', 'project', 'project_id', 'project.changed', ''
);
CREATE TRIGGER repositories_repository_application_event
AFTER INSERT OR UPDATE OR DELETE ON repositories
FOR EACH ROW EXECUTE FUNCTION capture_direct_application_event(
    'repository', 'id', 'repository', 'id', 'repository.changed', '', 'project_id'
);
CREATE TRIGGER repositories_project_application_event
AFTER INSERT OR UPDATE OR DELETE ON repositories
FOR EACH ROW EXECUTE FUNCTION capture_direct_application_event(
    'repository', 'id', 'project', 'project_id', 'repository.changed', '', 'project_id'
);
CREATE TRIGGER repository_managers_application_event
AFTER INSERT OR UPDATE OR DELETE ON repository_managers
FOR EACH ROW EXECUTE FUNCTION capture_direct_application_event(
    'repository', 'repository_id', 'repository', 'repository_id',
    'repository.changed', ''
);
CREATE TRIGGER repository_secret_roles_application_event
AFTER INSERT OR UPDATE OR DELETE ON repository_secret_roles
FOR EACH ROW EXECUTE FUNCTION capture_direct_application_event(
    'repository', 'repository_id', 'repository', 'repository_id',
    'repository.changed', ''
);
CREATE TRIGGER agent_families_application_event
AFTER INSERT OR UPDATE OR DELETE ON agent_families
FOR EACH ROW EXECUTE FUNCTION capture_direct_application_event(
    'repository', 'repository_id', 'repository', 'repository_id',
    'repository.changed', ''
);

-- Repository, build, and release state paths.
CREATE TRIGGER git_receives_application_event
AFTER INSERT OR UPDATE OR DELETE ON git_receives
FOR EACH ROW EXECUTE FUNCTION capture_direct_application_event(
    'repository', 'repository_id', 'repository', 'repository_id',
    'repository.changed', 'status'
);
CREATE TRIGGER git_refs_application_event
AFTER INSERT OR UPDATE OR DELETE ON git_refs
FOR EACH ROW EXECUTE FUNCTION capture_direct_application_event(
    'repository_ref', 'repository_id', 'repository', 'repository_id',
    'repository.ref_changed', ''
);
CREATE TRIGGER agent_config_revisions_application_event
AFTER INSERT OR UPDATE OR DELETE ON agent_config_revisions
FOR EACH ROW EXECUTE FUNCTION capture_direct_application_event(
    'repository', 'repository_id', 'repository', 'repository_id',
    'repository.changed', 'status'
);
CREATE TRIGGER build_requests_application_event
AFTER INSERT OR UPDATE OR DELETE ON build_requests
FOR EACH ROW EXECUTE FUNCTION capture_direct_application_event(
    'build', 'id', 'repository', 'repository_id', 'build.changed', 'state', 'repository_id'
);
CREATE TRIGGER releases_application_event
AFTER INSERT OR UPDATE OR DELETE ON releases
FOR EACH ROW EXECUTE FUNCTION capture_direct_application_event(
    'release', 'id', 'repository', 'repository_id', 'release.changed', 'state', 'repository_id'
);

-- Reusable instance state paths.
CREATE TRIGGER agent_instances_instance_application_event
AFTER INSERT OR UPDATE OR DELETE ON agent_instances
FOR EACH ROW EXECUTE FUNCTION capture_direct_application_event(
    'agent_instance', 'id', 'agent_instance', 'id', 'agent_instance.changed', 'state', 'project_id'
);
CREATE TRIGGER agent_instances_project_application_event
AFTER INSERT OR UPDATE OR DELETE ON agent_instances
FOR EACH ROW EXECUTE FUNCTION capture_direct_application_event(
    'agent_instance', 'id', 'project', 'project_id', 'agent_instance.changed', 'state', 'project_id'
);
CREATE TRIGGER agent_instance_revisions_application_event
AFTER INSERT OR DELETE ON agent_instance_revisions
FOR EACH ROW EXECUTE FUNCTION capture_direct_application_event(
    'agent_instance', 'instance_id', 'agent_instance', 'instance_id',
    'agent_instance.changed', ''
);
CREATE TRIGGER agent_attachments_instance_application_event
AFTER INSERT OR UPDATE OR DELETE ON agent_attachments
FOR EACH ROW EXECUTE FUNCTION capture_direct_application_event(
    'agent_instance', 'instance_id', 'agent_instance', 'instance_id',
    'agent_instance.changed', ''
);
CREATE TRIGGER agent_attachments_repository_application_event
AFTER INSERT OR UPDATE OR DELETE ON agent_attachments
FOR EACH ROW EXECUTE FUNCTION capture_direct_application_event(
    'repository', 'repository_id', 'repository', 'repository_id',
    'repository.changed', ''
);
CREATE TRIGGER agent_updates_application_event
AFTER INSERT OR UPDATE OR DELETE ON agent_updates
FOR EACH ROW EXECUTE FUNCTION capture_direct_application_event(
    'agent_instance', 'instance_id', 'agent_instance', 'instance_id',
    'agent_instance.changed', 'state'
);
CREATE TRIGGER agent_instance_state_volumes_application_event
AFTER INSERT OR UPDATE OR DELETE ON agent_instance_state_volumes
FOR EACH ROW EXECUTE FUNCTION capture_direct_application_event(
    'agent_instance', 'instance_id', 'agent_instance', 'instance_id',
    'agent_instance.changed', 'state'
);
CREATE TRIGGER deferred_agent_triggers_application_event
AFTER INSERT OR UPDATE OR DELETE ON deferred_agent_triggers
FOR EACH ROW EXECUTE FUNCTION capture_direct_application_event(
    'agent_instance', 'instance_id', 'agent_instance', 'instance_id',
    'agent_instance.changed', 'state'
);
CREATE TRIGGER agent_instance_events_application_event
AFTER INSERT ON agent_instance_events
FOR EACH ROW EXECUTE FUNCTION capture_direct_application_event(
    'agent_instance', 'instance_id', 'agent_instance', 'instance_id',
    'agent_instance.changed', ''
);
CREATE TRIGGER agent_instance_volume_leases_application_event
AFTER INSERT OR UPDATE OR DELETE ON agent_instance_volume_leases
FOR EACH ROW EXECUTE FUNCTION capture_direct_application_event(
    'agent_instance', 'instance_id', 'agent_instance', 'instance_id',
    'agent_instance.changed', 'state'
);

-- Run result state paths. `run_events` is deliberately excluded: logs and
-- metrics remain on their separately bounded stream.
CREATE TRIGGER run_requests_run_application_event
AFTER INSERT OR UPDATE OR DELETE ON run_requests
FOR EACH ROW EXECUTE FUNCTION capture_direct_application_event(
    'run', 'run_id', 'run', 'run_id', 'run.changed', 'dispatch_state'
);
CREATE TRIGGER run_requests_repository_application_event
AFTER INSERT OR UPDATE OR DELETE ON run_requests
FOR EACH ROW EXECUTE FUNCTION capture_direct_application_event(
    'repository', 'repository_id', 'repository', 'repository_id',
    'repository.changed', 'dispatch_state'
);
CREATE TRIGGER run_results_run_application_event
AFTER INSERT OR UPDATE OR DELETE ON run_results
FOR EACH ROW EXECUTE FUNCTION capture_direct_application_event(
    'run', 'run_id', 'run', 'run_id', 'run.changed', 'state'
);
CREATE TRIGGER run_results_repository_application_event
AFTER INSERT OR UPDATE OR DELETE ON run_results
FOR EACH ROW EXECUTE FUNCTION capture_direct_application_event(
    'repository', 'repository_id', 'repository', 'repository_id',
    'repository.changed', 'state'
);
CREATE TRIGGER run_workspaces_application_event
AFTER INSERT OR UPDATE OR DELETE ON run_workspaces
FOR EACH ROW EXECUTE FUNCTION capture_direct_application_event(
    'run', 'run_id', 'run', 'run_id', 'run.changed', 'state', '', 'repository_id'
);
CREATE TRIGGER run_instance_provenance_application_event
AFTER INSERT OR UPDATE OR DELETE ON run_instance_provenance
FOR EACH ROW EXECUTE FUNCTION capture_direct_application_event(
    'run', 'run_id', 'run', 'run_id', 'run.changed', '', '', 'target_repository_id'
);
CREATE TRIGGER run_secret_provenance_application_event
AFTER INSERT OR UPDATE OR DELETE ON run_secret_provenance
FOR EACH ROW EXECUTE FUNCTION capture_direct_application_event(
    'run', 'run_id', 'run', 'run_id', 'run.changed', ''
);
CREATE TRIGGER secret_runtime_sessions_application_event
AFTER INSERT OR UPDATE OR DELETE ON secret_runtime_sessions
FOR EACH ROW EXECUTE FUNCTION capture_direct_application_event(
    'run', 'run_id', 'run', 'run_id', 'run.changed', 'status'
);
CREATE TRIGGER secret_runtime_mounts_application_event
AFTER INSERT OR UPDATE OR DELETE ON secret_runtime_mounts
FOR EACH ROW EXECUTE FUNCTION capture_direct_application_event(
    'run', 'run_id', 'run', 'run_id', 'run.changed', 'state'
);
CREATE TRIGGER review_proposals_run_application_event
AFTER INSERT OR UPDATE OR DELETE ON review_proposals
FOR EACH ROW EXECUTE FUNCTION capture_direct_application_event(
    'review', 'id', 'run', 'run_id', 'review.changed', 'state', 'run_id'
);
CREATE TRIGGER review_proposals_repository_application_event
AFTER INSERT OR UPDATE OR DELETE ON review_proposals
FOR EACH ROW EXECUTE FUNCTION capture_direct_application_event(
    'review', 'id', 'repository', 'repository_id',
    'review.changed', 'state', 'run_id'
);

-- Routes whose scope or typed related IDs are owned by a parent row. The
-- durable event contains only identifiers and normalized lifecycle state.
CREATE FUNCTION capture_parented_application_event() RETURNS trigger
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = public, pg_temp
AS $$
DECLARE
    v_row jsonb := CASE WHEN TG_OP = 'DELETE' THEN to_jsonb(OLD) ELSE to_jsonb(NEW) END;
    v_change text := CASE TG_OP
        WHEN 'INSERT' THEN 'created' WHEN 'UPDATE' THEN 'updated' ELSE 'removed'
    END;
    v_occurrence uuid := COALESCE(
        NULLIF(current_setting('hephaestus.occurrence_id', true), '')::uuid,
        gen_random_uuid()
    );
    v_id uuid;
    v_parent uuid;
    v_scope uuid;
    v_related uuid;
    v_project uuid;
    v_repository uuid;
    v_state text;
    v_kind text;
BEGIN
    IF TG_OP = 'UPDATE' AND (
        (v_row ? 'state' AND to_jsonb(OLD) ->> 'state'
            IS DISTINCT FROM to_jsonb(NEW) ->> 'state')
        OR (v_row ? 'status' AND to_jsonb(OLD) ->> 'status'
            IS DISTINCT FROM to_jsonb(NEW) ->> 'status')
    ) THEN
        v_change := 'state_changed';
    END IF;
    CASE TG_TABLE_NAME
        WHEN 'git_ref_updates' THEN
            SELECT repository_id INTO v_scope FROM git_receives
            WHERE id = (v_row ->> 'receive_id')::uuid;
            PERFORM append_application_event(
                v_occurrence, 'repository', v_scope, 'repository_ref', v_scope,
                'repository.ref_changed', v_change, NULL
            );
        WHEN 'build_request_sources' THEN
            v_id := (v_row ->> 'build_request_id')::uuid;
            SELECT repository_id INTO v_scope FROM build_requests WHERE id = v_id;
            PERFORM append_application_event(
                v_occurrence, 'repository', v_scope, 'build', v_id,
                'build.changed', v_change, NULL, v_scope
            );
        WHEN 'build_executions' THEN
            v_id := (v_row ->> 'build_request_id')::uuid;
            SELECT repository_id INTO v_scope FROM build_requests WHERE id = v_id;
            PERFORM append_application_event(
                v_occurrence, 'repository', v_scope, 'build', v_id,
                'build.changed', v_change, left(v_row ->> 'state', 128), v_scope
            );
        WHEN 'release_agents' THEN
            v_parent := (v_row ->> 'release_id')::uuid;
            SELECT repository_id INTO v_scope FROM releases WHERE id = v_parent;
            PERFORM append_application_event(
                v_occurrence, 'repository', v_scope, 'release', v_parent,
                'release.changed', v_change, NULL, v_scope
            );
        WHEN 'release_artifacts' THEN
            v_id := (v_row ->> 'id')::uuid;
            v_parent := (v_row ->> 'release_id')::uuid;
            SELECT repository_id, build_request_id INTO v_scope, v_related
            FROM releases WHERE id = v_parent;
            PERFORM append_application_event(
                v_occurrence, 'repository', v_scope, 'artifact', v_id,
                'artifact.changed', v_change, NULL, v_parent, v_related
            );
        WHEN 'runs' THEN
            v_id := (v_row ->> 'id')::uuid;
            v_parent := (v_row ->> 'instance_id')::uuid;
            SELECT project_id INTO v_project
            FROM agent_instances WHERE id = v_parent;
            SELECT repository_id INTO v_repository
            FROM run_requests WHERE run_id = v_id;
            PERFORM append_application_event(
                v_occurrence, 'run', v_id, 'run', v_id,
                'run.changed', v_change, left(v_row ->> 'state', 128),
                v_project, v_repository
            );
            PERFORM append_application_event(
                v_occurrence, 'agent_instance', v_parent, 'run', v_id,
                'run.changed', v_change, left(v_row ->> 'state', 128),
                v_project, v_repository
            );
            PERFORM append_application_event(
                v_occurrence, 'project', v_project, 'run', v_id,
                'run.changed', v_change, left(v_row ->> 'state', 128),
                v_project, v_repository
            );
        WHEN 'result_artifacts' THEN
            v_id := (v_row ->> 'result_id')::uuid;
            SELECT run_id, repository_id INTO v_scope, v_repository
            FROM run_results WHERE id = v_id;
            SELECT instance.project_id INTO v_project
            FROM runs run JOIN agent_instances instance ON instance.id = run.instance_id
            WHERE run.id = v_scope;
            PERFORM append_application_event(
                v_occurrence, 'run', v_scope, 'run', v_scope,
                'run.changed', v_change, NULL, v_project, v_repository
            );
        WHEN 'secrets' THEN
            v_id := (v_row ->> 'id')::uuid;
            v_scope := (v_row ->> 'owner_organization_id')::uuid;
            PERFORM append_application_event(
                v_occurrence, 'organization', v_scope, 'secret_metadata', v_id,
                'secret_metadata.changed', v_change,
                left(v_row ->> 'status', 128), v_scope
            );
            IF v_row ->> 'project_id' IS NOT NULL THEN
                v_project := (v_row ->> 'project_id')::uuid;
                PERFORM append_application_event(
                    v_occurrence, 'project', v_project, 'secret_metadata', v_id,
                    'secret_metadata.changed', v_change,
                    left(v_row ->> 'status', 128), v_scope
                );
            END IF;
        WHEN 'secret_versions' THEN
            v_id := (v_row ->> 'secret_id')::uuid;
            SELECT owner_organization_id, project_id
            INTO v_scope, v_project FROM secrets WHERE id = v_id;
            PERFORM append_application_event(
                v_occurrence, 'organization', v_scope, 'secret_metadata', v_id,
                'secret_metadata.changed', v_change,
                left(v_row ->> 'status', 128), v_scope
            );
            IF v_project IS NOT NULL THEN
                PERFORM append_application_event(
                    v_occurrence, 'project', v_project, 'secret_metadata', v_id,
                    'secret_metadata.changed', v_change,
                    left(v_row ->> 'status', 128), v_scope
                );
            END IF;
        WHEN 'secret_grants' THEN
            v_id := (v_row ->> 'id')::uuid;
            v_scope := (v_row ->> 'owner_organization_id')::uuid;
            v_project := (v_row ->> 'target_project_id')::uuid;
            v_related := (v_row ->> 'target_id')::uuid;
            PERFORM append_application_event(
                v_occurrence, 'organization', v_scope, 'secret_grant', v_id,
                'secret_grant.changed', v_change,
                left(v_row ->> 'status', 128),
                (v_row ->> 'secret_id')::uuid, v_related
            );
            PERFORM append_application_event(
                v_occurrence, 'project', v_project, 'secret_grant', v_id,
                'secret_grant.changed', v_change,
                left(v_row ->> 'status', 128),
                (v_row ->> 'secret_id')::uuid, v_related
            );
        WHEN 'secret_imports' THEN
            v_id := (v_row ->> 'id')::uuid;
            SELECT owner_organization_id, target_project_id
            INTO v_scope, v_project FROM secret_grants
            WHERE id = (v_row ->> 'grant_id')::uuid;
            v_related := (v_row ->> 'target_id')::uuid;
            PERFORM append_application_event(
                v_occurrence, 'organization', v_scope, 'secret_import', v_id,
                'secret_import.changed', v_change,
                left(v_row ->> 'status', 128),
                (v_row ->> 'secret_id')::uuid, v_related
            );
            PERFORM append_application_event(
                v_occurrence, 'project', v_project, 'secret_import', v_id,
                'secret_import.changed', v_change,
                left(v_row ->> 'status', 128),
                (v_row ->> 'secret_id')::uuid, v_related
            );
        WHEN 'agent_secret_bindings' THEN
            v_id := (v_row ->> 'id')::uuid;
            SELECT instance_id INTO v_scope FROM agent_instance_revisions
            WHERE id = (v_row ->> 'instance_revision_id')::uuid;
            PERFORM append_application_event(
                v_occurrence, 'agent_instance', v_scope,
                'agent_secret_binding', v_id,
                'agent_secret_binding.changed', v_change,
                left(v_row ->> 'status', 128),
                v_scope, (v_row ->> 'import_id')::uuid
            );
        WHEN 'secret_leases' THEN
            IF v_row ->> 'run_id' IS NOT NULL THEN
                v_scope := (v_row ->> 'run_id')::uuid;
                PERFORM append_application_event(
                    v_occurrence, 'run', v_scope, 'run', v_scope,
                    'run.changed', v_change, left(v_row ->> 'status', 128)
                );
            ELSE
                SELECT instance_id INTO v_scope FROM agent_updates
                WHERE id = (v_row ->> 'update_id')::uuid;
                PERFORM append_application_event(
                    v_occurrence, 'agent_instance', v_scope,
                    'agent_instance', v_scope, 'agent_instance.changed',
                    v_change, left(v_row ->> 'status', 128)
                );
            END IF;
        WHEN 'control_requests' THEN
            IF v_row ->> 'run_id' IS NOT NULL THEN
                v_scope := (v_row ->> 'run_id')::uuid;
                SELECT instance.project_id, request.repository_id
                INTO v_project, v_repository
                FROM runs run
                JOIN agent_instances instance ON instance.id = run.instance_id
                LEFT JOIN run_requests request ON request.run_id = run.id
                WHERE run.id = v_scope;
                PERFORM append_application_event(
                    v_occurrence, 'run', v_scope, 'run', v_scope,
                    'run.changed', v_change, left(v_row ->> 'state', 128),
                    v_project, v_repository
                );
            ELSE
                v_id := (v_row ->> 'proposal_id')::uuid;
                SELECT run_id, repository_id INTO v_scope, v_repository
                FROM review_proposals WHERE id = v_id;
                PERFORM append_application_event(
                    v_occurrence, 'run', v_scope, 'review', v_id,
                    'review.changed', v_change, left(v_row ->> 'state', 128),
                    v_scope
                );
                PERFORM append_application_event(
                    v_occurrence, 'repository', v_repository, 'review', v_id,
                    'review.changed', v_change, left(v_row ->> 'state', 128),
                    v_scope
                );
            END IF;
        ELSE
            RAISE EXCEPTION 'unrouted application event table %', TG_TABLE_NAME;
    END CASE;
    RETURN CASE WHEN TG_OP = 'DELETE' THEN OLD ELSE NEW END;
END
$$;
REVOKE ALL ON FUNCTION capture_parented_application_event() FROM PUBLIC;

CREATE TRIGGER build_executions_application_event
AFTER INSERT OR UPDATE OR DELETE ON build_executions
FOR EACH ROW EXECUTE FUNCTION capture_parented_application_event();
CREATE TRIGGER git_ref_updates_application_event
AFTER INSERT OR UPDATE OR DELETE ON git_ref_updates
FOR EACH ROW EXECUTE FUNCTION capture_parented_application_event();
CREATE TRIGGER build_request_sources_application_event
AFTER INSERT OR UPDATE OR DELETE ON build_request_sources
FOR EACH ROW EXECUTE FUNCTION capture_parented_application_event();
CREATE TRIGGER release_agents_application_event
AFTER INSERT OR UPDATE OR DELETE ON release_agents
FOR EACH ROW EXECUTE FUNCTION capture_parented_application_event();
CREATE TRIGGER release_artifacts_application_event
AFTER INSERT OR UPDATE OR DELETE ON release_artifacts
FOR EACH ROW EXECUTE FUNCTION capture_parented_application_event();
CREATE TRIGGER runs_application_event
AFTER INSERT OR UPDATE OR DELETE ON runs
FOR EACH ROW EXECUTE FUNCTION capture_parented_application_event();
CREATE TRIGGER result_artifacts_application_event
AFTER INSERT OR UPDATE OR DELETE ON result_artifacts
FOR EACH ROW EXECUTE FUNCTION capture_parented_application_event();
CREATE TRIGGER secrets_application_event
AFTER INSERT OR UPDATE OR DELETE ON secrets
FOR EACH ROW EXECUTE FUNCTION capture_parented_application_event();
CREATE TRIGGER secret_versions_application_event
AFTER INSERT OR UPDATE OR DELETE ON secret_versions
FOR EACH ROW EXECUTE FUNCTION capture_parented_application_event();
CREATE TRIGGER secret_grants_application_event
AFTER INSERT OR UPDATE OR DELETE ON secret_grants
FOR EACH ROW EXECUTE FUNCTION capture_parented_application_event();
CREATE TRIGGER secret_imports_application_event
AFTER INSERT OR UPDATE OR DELETE ON secret_imports
FOR EACH ROW EXECUTE FUNCTION capture_parented_application_event();
CREATE TRIGGER agent_secret_bindings_application_event
AFTER INSERT OR UPDATE OR DELETE ON agent_secret_bindings
FOR EACH ROW EXECUTE FUNCTION capture_parented_application_event();
CREATE TRIGGER secret_leases_application_event
AFTER INSERT OR UPDATE OR DELETE ON secret_leases
FOR EACH ROW EXECUTE FUNCTION capture_parented_application_event();
CREATE TRIGGER control_requests_application_event
AFTER INSERT OR UPDATE OR DELETE ON control_requests
FOR EACH ROW EXECUTE FUNCTION capture_parented_application_event();
