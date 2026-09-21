-- Organization-owned global UI installations.
-- Migration 0087 intentionally remains unchanged; existing project and
-- repository rows keep organization_id NULL under the v88 owner-shape check.

ALTER TABLE ui_installations
    ADD COLUMN organization_id uuid REFERENCES organizations(id);
ALTER TABLE ui_installations
    ALTER COLUMN project_id DROP NOT NULL;

-- The 0087 checks were unnamed, so PostgreSQL assigned these stable names.
-- Replace them with explicit v88 constraints that make every owner shape
-- mutually exclusive and reject personal global ownership.
ALTER TABLE ui_installations
    DROP CONSTRAINT ui_installations_scope_check,
    DROP CONSTRAINT ui_installations_check;
ALTER TABLE ui_installations
    ADD CONSTRAINT ui_installations_scope_v88_check
        CHECK (scope IN ('global', 'project', 'repository')),
    ADD CONSTRAINT ui_installations_owner_shape_v88_check
        CHECK (
            (
                scope = 'global'
                AND organization_id IS NOT NULL
                AND project_id IS NULL
                AND repository_id IS NULL
            )
            OR (
                scope = 'project'
                AND organization_id IS NULL
                AND project_id IS NOT NULL
                AND repository_id IS NULL
            )
            OR (
                scope = 'repository'
                AND organization_id IS NULL
                AND project_id IS NOT NULL
                AND repository_id IS NOT NULL
            )
        );

DROP INDEX ui_installations_active_owner_key;
CREATE UNIQUE INDEX ui_installations_active_owner_key
    ON ui_installations (organization_id, project_id, repository_id, ui_key)
    NULLS NOT DISTINCT
    WHERE lifecycle <> 'removed';
CREATE INDEX ui_installations_organization_lifecycle
    ON ui_installations (organization_id, lifecycle, created_at DESC, id)
    WHERE organization_id IS NOT NULL;

ALTER TABLE ui_installation_generations
    DROP CONSTRAINT ui_installation_generations_ui_scope_check;
ALTER TABLE ui_installation_generations
    ADD CONSTRAINT ui_installation_generations_ui_scope_v88_check
        CHECK (ui_scope IN ('global', 'project', 'repository'));

CREATE OR REPLACE FUNCTION protect_ui_installation_identity() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF OLD.lifecycle = 'removed' THEN
        RAISE EXCEPTION 'removed UI installation is terminal'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    IF ROW(OLD.id, OLD.organization_id, OLD.project_id, OLD.repository_id,
           OLD.scope, OLD.ui_key, OLD.created_by, OLD.created_at)
       IS DISTINCT FROM
       ROW(NEW.id, NEW.organization_id, NEW.project_id, NEW.repository_id,
           NEW.scope, NEW.ui_key, NEW.created_by, NEW.created_at)
    THEN
        RAISE EXCEPTION 'UI installation owner and key are immutable'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    RETURN NEW;
END
$$;

-- The release and target installation have no direct organization composite
-- key: release ownership is derived through repository -> project -> org.
-- This deferred trigger protects worker writes for every installation scope;
-- the release adapter should perform the same check before mutation for a
-- typed error. Existing rows are checked before the trigger is installed.
DO $$
BEGIN
    IF EXISTS (
        SELECT 1
        FROM ui_installation_generations g
        JOIN ui_installations i ON i.id = g.installation_id
        JOIN releases r ON r.id = g.release_id
        JOIN repositories sr ON sr.id = r.repository_id
        JOIN projects sp ON sp.id = sr.project_id
        LEFT JOIN repositories tr ON tr.id = i.repository_id
        LEFT JOIN projects tp ON tp.id = COALESCE(tr.project_id, i.project_id)
        WHERE sp.organization_id IS DISTINCT FROM
            CASE
                WHEN i.scope = 'global' THEN i.organization_id
                ELSE tp.organization_id
            END
    ) THEN
        RAISE EXCEPTION 'existing UI installation generation crosses organization boundary'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
END
$$;

CREATE FUNCTION reject_ui_project_organization_move()
RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.organization_id IS DISTINCT FROM OLD.organization_id
       AND (
           EXISTS (
               SELECT 1 FROM ui_installations
               WHERE project_id = OLD.id
           )
           OR EXISTS (
               SELECT 1
               FROM ui_installations installation
               JOIN repositories repository
                   ON repository.id = installation.repository_id
               WHERE repository.project_id = OLD.id
           )
           OR EXISTS (
               SELECT 1
               FROM ui_installation_generations generation
               JOIN releases release_record
                   ON release_record.id = generation.release_id
               JOIN repositories repository
                   ON repository.id = release_record.repository_id
               WHERE repository.project_id = OLD.id
           )
       )
    THEN
        RAISE EXCEPTION 'project organization cannot move after UI publication'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    RETURN NEW;
END
$$;
REVOKE ALL ON FUNCTION reject_ui_project_organization_move() FROM PUBLIC;

CREATE FUNCTION reject_ui_repository_project_move()
RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.project_id IS DISTINCT FROM OLD.project_id
       AND (
           EXISTS (
               SELECT 1 FROM ui_installations
               WHERE repository_id = OLD.id
           )
           OR EXISTS (
               SELECT 1
               FROM ui_installation_generations generation
               JOIN releases release_record
                   ON release_record.id = generation.release_id
               WHERE release_record.repository_id = OLD.id
           )
       )
    THEN
        RAISE EXCEPTION 'repository project cannot move after UI publication'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    RETURN NEW;
END
$$;
REVOKE ALL ON FUNCTION reject_ui_repository_project_move() FROM PUBLIC;

CREATE FUNCTION reject_ui_release_repository_move()
RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.repository_id IS DISTINCT FROM OLD.repository_id
       AND EXISTS (
           SELECT 1 FROM ui_installation_generations
           WHERE release_id = OLD.id
       )
    THEN
        RAISE EXCEPTION 'release repository cannot move after UI publication'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    RETURN NEW;
END
$$;
REVOKE ALL ON FUNCTION reject_ui_release_repository_move() FROM PUBLIC;

-- These guards cover retained UI history only. Ordinary parent edits remain
-- available while no installation or generation references the row.
CREATE TRIGGER ui_project_ui_organization_immutable
BEFORE UPDATE OF organization_id ON projects
FOR EACH ROW EXECUTE FUNCTION reject_ui_project_organization_move();
CREATE TRIGGER ui_repository_ui_project_immutable
BEFORE UPDATE OF project_id ON repositories
FOR EACH ROW EXECUTE FUNCTION reject_ui_repository_project_move();
CREATE TRIGGER ui_release_ui_repository_immutable
BEFORE UPDATE OF repository_id ON releases
FOR EACH ROW EXECUTE FUNCTION reject_ui_release_repository_move();

CREATE FUNCTION enforce_ui_installation_generation_organization()
RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
    initial_target_project_id uuid;
    initial_target_repository_id uuid;
    initial_source_release_id uuid;
    initial_source_project_id uuid;
    initial_source_repository_id uuid;
    target_project_id uuid;
    target_repository_id uuid;
    source_project_id uuid;
    source_repository_id uuid;
    source_release_id uuid;
    target_organization_id uuid;
    source_organization_id uuid;
BEGIN
    -- Lock the source release before its repository and project parents, then
    -- use the same repository-before-project order for all parent rows. These
    -- locks serialize the validation with concurrent provenance moves.
    SELECT target_installation.repository_id,
           COALESCE(target_repository.project_id, target_installation.project_id),
           source_release.id,
           source_repository.id,
           source_repository.project_id
    INTO initial_target_repository_id, initial_target_project_id,
         initial_source_release_id, initial_source_repository_id,
         initial_source_project_id
    FROM ui_installations target_installation
    JOIN releases source_release
        ON source_release.id = NEW.release_id
    JOIN repositories source_repository
        ON source_repository.id = source_release.repository_id
    LEFT JOIN repositories target_repository
        ON target_repository.id = target_installation.repository_id
    WHERE target_installation.id = NEW.installation_id;

    PERFORM 1
    FROM releases
    WHERE id = initial_source_release_id
    FOR SHARE;
    PERFORM 1
    FROM repositories
    WHERE id IN (initial_source_repository_id, initial_target_repository_id)
    ORDER BY id
    FOR SHARE;
    PERFORM 1
    FROM projects
    WHERE id IN (initial_source_project_id, initial_target_project_id)
    ORDER BY id
    FOR SHARE;

    -- Re-read after the locks. If a parent pointer changed before the lock was
    -- acquired, fail closed rather than accepting an incompletely locked view.
    SELECT target_installation.repository_id,
           COALESCE(target_repository.project_id, target_installation.project_id),
           source_release.id,
           source_repository.id,
           source_repository.project_id,
           CASE
               WHEN target_installation.scope = 'global'
                   THEN target_installation.organization_id
               ELSE target_project.organization_id
           END,
           source_project.organization_id
    INTO target_repository_id, target_project_id, source_release_id,
         source_repository_id, source_project_id, target_organization_id,
         source_organization_id
    FROM ui_installations target_installation
    JOIN releases source_release
        ON source_release.id = NEW.release_id
    JOIN repositories source_repository
        ON source_repository.id = source_release.repository_id
    JOIN projects source_project
        ON source_project.id = source_repository.project_id
    LEFT JOIN repositories target_repository
        ON target_repository.id = target_installation.repository_id
    LEFT JOIN projects target_project
        ON target_project.id = COALESCE(
            target_repository.project_id, target_installation.project_id
        )
    WHERE target_installation.id = NEW.installation_id;

    IF source_release_id IS DISTINCT FROM initial_source_release_id
       OR target_repository_id IS DISTINCT FROM initial_target_repository_id
       OR target_project_id IS DISTINCT FROM initial_target_project_id
       OR source_repository_id IS DISTINCT FROM initial_source_repository_id
       OR source_project_id IS DISTINCT FROM initial_source_project_id
    THEN
        RAISE EXCEPTION 'UI installation parent changed during generation'
            USING ERRCODE = 'serialization_failure';
    END IF;

    IF target_organization_id IS NULL
       OR source_organization_id IS NULL
       OR target_organization_id IS DISTINCT FROM source_organization_id
    THEN
        RAISE EXCEPTION 'UI installation release crosses organization boundary'
            USING ERRCODE = 'integrity_constraint_violation';
    END IF;
    RETURN NEW;
END
$$;
REVOKE ALL ON FUNCTION enforce_ui_installation_generation_organization() FROM PUBLIC;

CREATE CONSTRAINT TRIGGER ui_installation_generation_organization
AFTER INSERT ON ui_installation_generations
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION enforce_ui_installation_generation_organization();

DROP POLICY ui_installations_app_read ON ui_installations;
CREATE POLICY ui_installations_app_read
    ON ui_installations FOR SELECT TO hephaestus_app
    USING (
        (
            organization_id IS NOT NULL
            AND project_id IS NULL
            AND repository_id IS NULL
            AND check_permission('user', hephaestus_actor_id(), 'can_read',
                'organization', organization_id::text) = 1
        )
        OR (
            organization_id IS NULL
            AND project_id IS NOT NULL
            AND check_permission('user', hephaestus_actor_id(), 'can_read',
                'project', project_id::text) = 1
            AND (
                repository_id IS NULL
                OR check_permission('user', hephaestus_actor_id(), 'can_read',
                    'repository', repository_id::text) = 1
            )
        )
    );
