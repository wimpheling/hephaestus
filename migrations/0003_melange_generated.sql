-- Melange Migration (UP)
-- Melange version: 0.8.5
-- Schema checksum: 4a71ce11770ac14b15711b5e56fcb2007ec2945014a2fabfb759042c9faea822
-- Codegen version: 0.8.5

-- ============================================================
-- Check Functions (31 functions)
-- ============================================================

-- Generated check function for agent.project
-- Features: Direct
CREATE OR REPLACE FUNCTION "public"."check_agent_project"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[]
) RETURNS INTEGER AS $$
DECLARE
    v_userset_check INTEGER := 0;
BEGIN
    -- Userset subject handling
    IF position('#' in p_subject_id) > 0 THEN
        -- Case 1: Self-referential userset check
        IF (p_subject_type = 'agent' AND substring(p_subject_id from 1 for position('#' in p_subject_id) - 1) = p_object_id) THEN
        SELECT INTO v_userset_check 1
    WHERE substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('project')
    LIMIT 1;
        IF v_userset_check = 1 THEN
        RETURN 1;
    END IF;
    END IF;
    END IF;
    IF EXISTS (
    SELECT 1
    FROM melange_tuples
    WHERE (object_type = 'agent' AND relation IN ('project') AND object_id = p_object_id AND subject_type IN ('project') AND subject_type = p_subject_type AND (subject_id = p_subject_id AND NOT (subject_id = '*')))
    LIMIT 1
    ) THEN
        RETURN 1;
    ELSE
        RETURN 0;
    END IF;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';


-- Generated check function for organization.owner
-- Features: Direct
CREATE OR REPLACE FUNCTION "public"."check_organization_owner"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[]
) RETURNS INTEGER AS $$
DECLARE
    v_userset_check INTEGER := 0;
BEGIN
    -- Userset subject handling
    IF position('#' in p_subject_id) > 0 THEN
        -- Case 1: Self-referential userset check
        IF (p_subject_type = 'organization' AND substring(p_subject_id from 1 for position('#' in p_subject_id) - 1) = p_object_id) THEN
        SELECT INTO v_userset_check 1
    WHERE substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('owner')
    LIMIT 1;
        IF v_userset_check = 1 THEN
        RETURN 1;
    END IF;
    END IF;
    END IF;
    IF EXISTS (
    SELECT 1
    FROM melange_tuples
    WHERE (object_type = 'organization' AND relation IN ('owner') AND object_id = p_object_id AND subject_type IN ('user') AND subject_type = p_subject_type AND (subject_id = p_subject_id AND NOT (subject_id = '*')))
    LIMIT 1
    ) THEN
        RETURN 1;
    ELSE
        RETURN 0;
    END IF;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';


-- Generated check function for project.maintainer
-- Features: Direct
CREATE OR REPLACE FUNCTION "public"."check_project_maintainer"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[]
) RETURNS INTEGER AS $$
DECLARE
    v_userset_check INTEGER := 0;
BEGIN
    -- Userset subject handling
    IF position('#' in p_subject_id) > 0 THEN
        -- Case 1: Self-referential userset check
        IF (p_subject_type = 'project' AND substring(p_subject_id from 1 for position('#' in p_subject_id) - 1) = p_object_id) THEN
        SELECT INTO v_userset_check 1
    WHERE substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('maintainer')
    LIMIT 1;
        IF v_userset_check = 1 THEN
        RETURN 1;
    END IF;
    END IF;
    END IF;
    IF EXISTS (
    SELECT 1
    FROM melange_tuples
    WHERE (object_type = 'project' AND relation IN ('maintainer') AND object_id = p_object_id AND subject_type IN ('user') AND subject_type = p_subject_type AND (subject_id = p_subject_id AND NOT (subject_id = '*')))
    LIMIT 1
    ) THEN
        RETURN 1;
    ELSE
        RETURN 0;
    END IF;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';


-- Generated check function for project.organization
-- Features: Direct
CREATE OR REPLACE FUNCTION "public"."check_project_organization"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[]
) RETURNS INTEGER AS $$
DECLARE
    v_userset_check INTEGER := 0;
BEGIN
    -- Userset subject handling
    IF position('#' in p_subject_id) > 0 THEN
        -- Case 1: Self-referential userset check
        IF (p_subject_type = 'project' AND substring(p_subject_id from 1 for position('#' in p_subject_id) - 1) = p_object_id) THEN
        SELECT INTO v_userset_check 1
    WHERE substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('organization')
    LIMIT 1;
        IF v_userset_check = 1 THEN
        RETURN 1;
    END IF;
    END IF;
    END IF;
    IF EXISTS (
    SELECT 1
    FROM melange_tuples
    WHERE (object_type = 'project' AND relation IN ('organization') AND object_id = p_object_id AND subject_type IN ('organization') AND subject_type = p_subject_type AND (subject_id = p_subject_id AND NOT (subject_id = '*')))
    LIMIT 1
    ) THEN
        RETURN 1;
    ELSE
        RETURN 0;
    END IF;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';


-- Generated check function for repository.project
-- Features: Direct
CREATE OR REPLACE FUNCTION "public"."check_repository_project"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[]
) RETURNS INTEGER AS $$
DECLARE
    v_userset_check INTEGER := 0;
BEGIN
    -- Userset subject handling
    IF position('#' in p_subject_id) > 0 THEN
        -- Case 1: Self-referential userset check
        IF (p_subject_type = 'repository' AND substring(p_subject_id from 1 for position('#' in p_subject_id) - 1) = p_object_id) THEN
        SELECT INTO v_userset_check 1
    WHERE substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('project')
    LIMIT 1;
        IF v_userset_check = 1 THEN
        RETURN 1;
    END IF;
    END IF;
    END IF;
    IF EXISTS (
    SELECT 1
    FROM melange_tuples
    WHERE (object_type = 'repository' AND relation IN ('project') AND object_id = p_object_id AND subject_type IN ('project') AND subject_type = p_subject_type AND (subject_id = p_subject_id AND NOT (subject_id = '*')))
    LIMIT 1
    ) THEN
        RETURN 1;
    ELSE
        RETURN 0;
    END IF;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';


-- Generated check function for repository.public
-- Features: Direct+Wildcard
CREATE OR REPLACE FUNCTION "public"."check_repository_public"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[]
) RETURNS INTEGER AS $$
DECLARE
    v_userset_check INTEGER := 0;
BEGIN
    -- Userset subject handling
    IF position('#' in p_subject_id) > 0 THEN
        -- Case 1: Self-referential userset check
        IF (p_subject_type = 'repository' AND substring(p_subject_id from 1 for position('#' in p_subject_id) - 1) = p_object_id) THEN
        SELECT INTO v_userset_check 1
    WHERE substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('public')
    LIMIT 1;
        IF v_userset_check = 1 THEN
        RETURN 1;
    END IF;
    END IF;
    END IF;
    IF EXISTS (
    SELECT 1
    FROM melange_tuples
    WHERE (object_type = 'repository' AND relation IN ('public') AND object_id = p_object_id AND subject_type IN ('user') AND subject_type = p_subject_type AND (subject_id = p_subject_id OR subject_id = '*'))
    LIMIT 1
    ) THEN
        RETURN 1;
    ELSE
        RETURN 0;
    END IF;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';


-- Generated check function for run.agent
-- Features: Direct
CREATE OR REPLACE FUNCTION "public"."check_run_agent"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[]
) RETURNS INTEGER AS $$
DECLARE
    v_userset_check INTEGER := 0;
BEGIN
    -- Userset subject handling
    IF position('#' in p_subject_id) > 0 THEN
        -- Case 1: Self-referential userset check
        IF (p_subject_type = 'run' AND substring(p_subject_id from 1 for position('#' in p_subject_id) - 1) = p_object_id) THEN
        SELECT INTO v_userset_check 1
    WHERE substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('agent')
    LIMIT 1;
        IF v_userset_check = 1 THEN
        RETURN 1;
    END IF;
    END IF;
    END IF;
    IF EXISTS (
    SELECT 1
    FROM melange_tuples
    WHERE (object_type = 'run' AND relation IN ('agent') AND object_id = p_object_id AND subject_type IN ('agent') AND subject_type = p_subject_type AND (subject_id = p_subject_id AND NOT (subject_id = '*')))
    LIMIT 1
    ) THEN
        RETURN 1;
    ELSE
        RETURN 0;
    END IF;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';


-- Generated check function for state_volume.agent
-- Features: Direct
CREATE OR REPLACE FUNCTION "public"."check_state_volume_agent"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[]
) RETURNS INTEGER AS $$
DECLARE
    v_userset_check INTEGER := 0;
BEGIN
    -- Userset subject handling
    IF position('#' in p_subject_id) > 0 THEN
        -- Case 1: Self-referential userset check
        IF (p_subject_type = 'state_volume' AND substring(p_subject_id from 1 for position('#' in p_subject_id) - 1) = p_object_id) THEN
        SELECT INTO v_userset_check 1
    WHERE substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('agent')
    LIMIT 1;
        IF v_userset_check = 1 THEN
        RETURN 1;
    END IF;
    END IF;
    END IF;
    IF EXISTS (
    SELECT 1
    FROM melange_tuples
    WHERE (object_type = 'state_volume' AND relation IN ('agent') AND object_id = p_object_id AND subject_type IN ('agent') AND subject_type = p_subject_type AND (subject_id = p_subject_id AND NOT (subject_id = '*')))
    LIMIT 1
    ) THEN
        RETURN 1;
    ELSE
        RETURN 0;
    END IF;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';


-- Generated check function for organization.admin
-- Features: Direct+Implied
CREATE OR REPLACE FUNCTION "public"."check_organization_admin"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[]
) RETURNS INTEGER AS $$
DECLARE
    v_userset_check INTEGER := 0;
BEGIN
    -- Userset subject handling
    IF position('#' in p_subject_id) > 0 THEN
        -- Case 1: Self-referential userset check
        IF (p_subject_type = 'organization' AND substring(p_subject_id from 1 for position('#' in p_subject_id) - 1) = p_object_id) THEN
        SELECT INTO v_userset_check 1
    WHERE substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('admin', 'owner')
    LIMIT 1;
        IF v_userset_check = 1 THEN
        RETURN 1;
    END IF;
    END IF;
    END IF;
    IF EXISTS (
    SELECT 1
    FROM melange_tuples
    WHERE (object_type = 'organization' AND relation IN ('admin', 'owner') AND object_id = p_object_id AND subject_type IN ('user') AND subject_type = p_subject_type AND (subject_id = p_subject_id AND NOT (subject_id = '*')))
    LIMIT 1
    ) THEN
        RETURN 1;
    ELSE
        RETURN 0;
    END IF;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';


-- Generated check function for organization.can_delete
-- Features: Implied
CREATE OR REPLACE FUNCTION "public"."check_organization_can_delete"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[]
) RETURNS INTEGER AS $$
DECLARE
    v_userset_check INTEGER := 0;
BEGIN
    -- Userset subject handling
    IF position('#' in p_subject_id) > 0 THEN
        -- Case 1: Self-referential userset check
        IF (p_subject_type = 'organization' AND substring(p_subject_id from 1 for position('#' in p_subject_id) - 1) = p_object_id) THEN
        SELECT INTO v_userset_check 1
    WHERE substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('can_delete', 'owner')
    LIMIT 1;
        IF v_userset_check = 1 THEN
        RETURN 1;
    END IF;
    END IF;
    END IF;
    IF EXISTS (
    SELECT 1
    FROM melange_tuples
    WHERE (object_type = 'organization' AND relation IN ('can_delete', 'owner') AND object_id = p_object_id AND subject_type IN ('user') AND subject_type = p_subject_type AND (subject_id = p_subject_id AND NOT (subject_id = '*')))
    LIMIT 1
    ) THEN
        RETURN 1;
    ELSE
        RETURN 0;
    END IF;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';


-- Generated check function for organization.can_manage
-- Features: Implied
CREATE OR REPLACE FUNCTION "public"."check_organization_can_manage"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[]
) RETURNS INTEGER AS $$
DECLARE
    v_userset_check INTEGER := 0;
BEGIN
    -- Userset subject handling
    IF position('#' in p_subject_id) > 0 THEN
        -- Case 1: Self-referential userset check
        IF (p_subject_type = 'organization' AND substring(p_subject_id from 1 for position('#' in p_subject_id) - 1) = p_object_id) THEN
        SELECT INTO v_userset_check 1
    WHERE substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('can_manage', 'owner')
    LIMIT 1;
        IF v_userset_check = 1 THEN
        RETURN 1;
    END IF;
    END IF;
    END IF;
    IF EXISTS (
    SELECT 1
    FROM melange_tuples
    WHERE (object_type = 'organization' AND relation IN ('can_manage', 'owner') AND object_id = p_object_id AND subject_type IN ('user') AND subject_type = p_subject_type AND (subject_id = p_subject_id AND NOT (subject_id = '*')))
    LIMIT 1
    ) THEN
        RETURN 1;
    ELSE
        RETURN 0;
    END IF;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';


-- Generated check function for project.can_delete
-- Features: Recursive
CREATE OR REPLACE FUNCTION "public"."check_project_can_delete"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[]
) RETURNS INTEGER AS $$
DECLARE
    v_has_access BOOLEAN := FALSE;
    v_key TEXT := 'project:' || p_object_id || ':can_delete';
    v_userset_check INTEGER := 0;
BEGIN
    -- Cycle detection
    IF v_key = ANY(p_visited) THEN
        RETURN 0;
    END IF;
    IF array_length(p_visited, 1) >= 25 THEN
        RAISE EXCEPTION 'resolution too complex' USING ERRCODE = 'M2002';
    END IF;
    v_has_access := FALSE;
    -- Userset subject handling
    IF position('#' in p_subject_id) > 0 THEN
        -- Case 1: Self-referential userset check
        IF (p_subject_type = 'project' AND substring(p_subject_id from 1 for position('#' in p_subject_id) - 1) = p_object_id) THEN
        SELECT INTO v_userset_check 1
    WHERE substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('can_delete')
    LIMIT 1;
        IF v_userset_check = 1 THEN
        RETURN 1;
    END IF;
    END IF;
    END IF;
    IF NOT (v_has_access) THEN
        -- Recursive access path via organization -> owner
        IF EXISTS (
    SELECT 1
    FROM melange_tuples AS link
    WHERE (link.object_type = 'project' AND link.relation IN ('organization') AND link.object_id = p_object_id AND "public"."check_permission_internal"(p_subject_type, p_subject_id, 'owner', link.subject_type, link.subject_id, p_visited || ARRAY[v_key]) = 1 AND link.subject_type IN ('organization'))
    ) THEN
        v_has_access := TRUE;
    END IF;
    END IF;
    IF v_has_access THEN
        RETURN 1;
    END IF;
    RETURN 0;
END;
$$ LANGUAGE plpgsql STABLE COST 1000
SET search_path = 'public';


-- Generated check function for project.can_manage
-- Features: Implied
CREATE OR REPLACE FUNCTION "public"."check_project_can_manage"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[]
) RETURNS INTEGER AS $$
DECLARE
    v_userset_check INTEGER := 0;
BEGIN
    -- Userset subject handling
    IF position('#' in p_subject_id) > 0 THEN
        -- Case 1: Self-referential userset check
        IF (p_subject_type = 'project' AND substring(p_subject_id from 1 for position('#' in p_subject_id) - 1) = p_object_id) THEN
        SELECT INTO v_userset_check 1
    WHERE substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('can_manage', 'maintainer')
    LIMIT 1;
        IF v_userset_check = 1 THEN
        RETURN 1;
    END IF;
    END IF;
    END IF;
    IF EXISTS (
    SELECT 1
    FROM melange_tuples
    WHERE (object_type = 'project' AND relation IN ('can_manage', 'maintainer') AND object_id = p_object_id AND subject_type IN ('user') AND subject_type = p_subject_type AND (subject_id = p_subject_id AND NOT (subject_id = '*')))
    LIMIT 1
    ) THEN
        RETURN 1;
    ELSE
        RETURN 0;
    END IF;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';


-- Generated check function for project.can_write
-- Features: Implied
CREATE OR REPLACE FUNCTION "public"."check_project_can_write"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[]
) RETURNS INTEGER AS $$
DECLARE
    v_userset_check INTEGER := 0;
BEGIN
    -- Userset subject handling
    IF position('#' in p_subject_id) > 0 THEN
        -- Case 1: Self-referential userset check
        IF (p_subject_type = 'project' AND substring(p_subject_id from 1 for position('#' in p_subject_id) - 1) = p_object_id) THEN
        SELECT INTO v_userset_check 1
    WHERE substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('can_write', 'maintainer')
    LIMIT 1;
        IF v_userset_check = 1 THEN
        RETURN 1;
    END IF;
    END IF;
    END IF;
    IF EXISTS (
    SELECT 1
    FROM melange_tuples
    WHERE (object_type = 'project' AND relation IN ('can_write', 'maintainer') AND object_id = p_object_id AND subject_type IN ('user') AND subject_type = p_subject_type AND (subject_id = p_subject_id AND NOT (subject_id = '*')))
    LIMIT 1
    ) THEN
        RETURN 1;
    ELSE
        RETURN 0;
    END IF;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';


-- Generated check function for organization.can_create_project
-- Features: Implied
CREATE OR REPLACE FUNCTION "public"."check_organization_can_create_project"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[]
) RETURNS INTEGER AS $$
DECLARE
    v_userset_check INTEGER := 0;
BEGIN
    -- Userset subject handling
    IF position('#' in p_subject_id) > 0 THEN
        -- Case 1: Self-referential userset check
        IF (p_subject_type = 'organization' AND substring(p_subject_id from 1 for position('#' in p_subject_id) - 1) = p_object_id) THEN
        SELECT INTO v_userset_check 1
    WHERE substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('admin', 'can_create_project', 'owner')
    LIMIT 1;
        IF v_userset_check = 1 THEN
        RETURN 1;
    END IF;
    END IF;
    END IF;
    IF EXISTS (
    SELECT 1
    FROM melange_tuples
    WHERE (object_type = 'organization' AND relation IN ('can_create_project', 'admin', 'owner') AND object_id = p_object_id AND subject_type IN ('user') AND subject_type = p_subject_type AND (subject_id = p_subject_id AND NOT (subject_id = '*')))
    LIMIT 1
    ) THEN
        RETURN 1;
    ELSE
        RETURN 0;
    END IF;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';


-- Generated check function for organization.can_manage_members
-- Features: Implied
CREATE OR REPLACE FUNCTION "public"."check_organization_can_manage_members"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[]
) RETURNS INTEGER AS $$
DECLARE
    v_userset_check INTEGER := 0;
BEGIN
    -- Userset subject handling
    IF position('#' in p_subject_id) > 0 THEN
        -- Case 1: Self-referential userset check
        IF (p_subject_type = 'organization' AND substring(p_subject_id from 1 for position('#' in p_subject_id) - 1) = p_object_id) THEN
        SELECT INTO v_userset_check 1
    WHERE substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('admin', 'can_manage_members', 'owner')
    LIMIT 1;
        IF v_userset_check = 1 THEN
        RETURN 1;
    END IF;
    END IF;
    END IF;
    IF EXISTS (
    SELECT 1
    FROM melange_tuples
    WHERE (object_type = 'organization' AND relation IN ('can_manage_members', 'admin', 'owner') AND object_id = p_object_id AND subject_type IN ('user') AND subject_type = p_subject_type AND (subject_id = p_subject_id AND NOT (subject_id = '*')))
    LIMIT 1
    ) THEN
        RETURN 1;
    ELSE
        RETURN 0;
    END IF;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';


-- Generated check function for organization.member
-- Features: Direct+Implied
CREATE OR REPLACE FUNCTION "public"."check_organization_member"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[]
) RETURNS INTEGER AS $$
DECLARE
    v_userset_check INTEGER := 0;
BEGIN
    -- Userset subject handling
    IF position('#' in p_subject_id) > 0 THEN
        -- Case 1: Self-referential userset check
        IF (p_subject_type = 'organization' AND substring(p_subject_id from 1 for position('#' in p_subject_id) - 1) = p_object_id) THEN
        SELECT INTO v_userset_check 1
    WHERE substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('admin', 'member', 'owner')
    LIMIT 1;
        IF v_userset_check = 1 THEN
        RETURN 1;
    END IF;
    END IF;
    END IF;
    IF EXISTS (
    SELECT 1
    FROM melange_tuples
    WHERE (object_type = 'organization' AND relation IN ('member', 'admin', 'owner') AND object_id = p_object_id AND subject_type IN ('user') AND subject_type = p_subject_type AND (subject_id = p_subject_id AND NOT (subject_id = '*')))
    LIMIT 1
    ) THEN
        RETURN 1;
    ELSE
        RETURN 0;
    END IF;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';


-- Generated check function for repository.can_delete
-- Features: Recursive
CREATE OR REPLACE FUNCTION "public"."check_repository_can_delete"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[]
) RETURNS INTEGER AS $$
DECLARE
    v_has_access BOOLEAN := FALSE;
    v_key TEXT := 'repository:' || p_object_id || ':can_delete';
    v_userset_check INTEGER := 0;
BEGIN
    -- Cycle detection
    IF v_key = ANY(p_visited) THEN
        RETURN 0;
    END IF;
    IF array_length(p_visited, 1) >= 25 THEN
        RAISE EXCEPTION 'resolution too complex' USING ERRCODE = 'M2002';
    END IF;
    v_has_access := FALSE;
    -- Userset subject handling
    IF position('#' in p_subject_id) > 0 THEN
        -- Case 1: Self-referential userset check
        IF (p_subject_type = 'repository' AND substring(p_subject_id from 1 for position('#' in p_subject_id) - 1) = p_object_id) THEN
        SELECT INTO v_userset_check 1
    WHERE substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('can_delete')
    LIMIT 1;
        IF v_userset_check = 1 THEN
        RETURN 1;
    END IF;
    END IF;
    END IF;
    IF NOT (v_has_access) THEN
        -- Recursive access path via project -> can_delete
        IF EXISTS (
    SELECT 1
    FROM melange_tuples AS link
    WHERE (link.object_type = 'repository' AND link.relation IN ('project') AND link.object_id = p_object_id AND "public"."check_permission_internal"(p_subject_type, p_subject_id, 'can_delete', link.subject_type, link.subject_id, p_visited || ARRAY[v_key]) = 1 AND link.subject_type IN ('project'))
    ) THEN
        v_has_access := TRUE;
    END IF;
    END IF;
    IF v_has_access THEN
        RETURN 1;
    END IF;
    RETURN 0;
END;
$$ LANGUAGE plpgsql STABLE COST 1000
SET search_path = 'public';


-- Generated check function for agent.can_manage
-- Features: Recursive
CREATE OR REPLACE FUNCTION "public"."check_agent_can_manage"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[]
) RETURNS INTEGER AS $$
DECLARE
    v_has_access BOOLEAN := FALSE;
    v_key TEXT := 'agent:' || p_object_id || ':can_manage';
    v_userset_check INTEGER := 0;
BEGIN
    -- Cycle detection
    IF v_key = ANY(p_visited) THEN
        RETURN 0;
    END IF;
    IF array_length(p_visited, 1) >= 25 THEN
        RAISE EXCEPTION 'resolution too complex' USING ERRCODE = 'M2002';
    END IF;
    v_has_access := FALSE;
    -- Userset subject handling
    IF position('#' in p_subject_id) > 0 THEN
        -- Case 1: Self-referential userset check
        IF (p_subject_type = 'agent' AND substring(p_subject_id from 1 for position('#' in p_subject_id) - 1) = p_object_id) THEN
        SELECT INTO v_userset_check 1
    WHERE substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('can_manage')
    LIMIT 1;
        IF v_userset_check = 1 THEN
        RETURN 1;
    END IF;
    END IF;
    END IF;
    IF NOT (v_has_access) THEN
        -- Recursive access path via project -> can_manage
        IF EXISTS (
    SELECT 1
    FROM melange_tuples AS link
    WHERE (link.object_type = 'agent' AND link.relation IN ('project') AND link.object_id = p_object_id AND "public"."check_permission_internal"(p_subject_type, p_subject_id, 'can_manage', link.subject_type, link.subject_id, p_visited || ARRAY[v_key]) = 1 AND link.subject_type IN ('project'))
    ) THEN
        v_has_access := TRUE;
    END IF;
    END IF;
    IF v_has_access THEN
        RETURN 1;
    END IF;
    RETURN 0;
END;
$$ LANGUAGE plpgsql STABLE COST 1000
SET search_path = 'public';


-- Generated check function for agent.can_execute
-- Features: Recursive
CREATE OR REPLACE FUNCTION "public"."check_agent_can_execute"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[]
) RETURNS INTEGER AS $$
DECLARE
    v_has_access BOOLEAN := FALSE;
    v_key TEXT := 'agent:' || p_object_id || ':can_execute';
    v_userset_check INTEGER := 0;
BEGIN
    -- Cycle detection
    IF v_key = ANY(p_visited) THEN
        RETURN 0;
    END IF;
    IF array_length(p_visited, 1) >= 25 THEN
        RAISE EXCEPTION 'resolution too complex' USING ERRCODE = 'M2002';
    END IF;
    v_has_access := FALSE;
    -- Userset subject handling
    IF position('#' in p_subject_id) > 0 THEN
        -- Case 1: Self-referential userset check
        IF (p_subject_type = 'agent' AND substring(p_subject_id from 1 for position('#' in p_subject_id) - 1) = p_object_id) THEN
        SELECT INTO v_userset_check 1
    WHERE substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('can_execute')
    LIMIT 1;
        IF v_userset_check = 1 THEN
        RETURN 1;
    END IF;
    END IF;
    END IF;
    IF NOT (v_has_access) THEN
        -- Recursive access path via project -> can_write
        IF EXISTS (
    SELECT 1
    FROM melange_tuples AS link
    WHERE (link.object_type = 'agent' AND link.relation IN ('project') AND link.object_id = p_object_id AND "public"."check_permission_internal"(p_subject_type, p_subject_id, 'can_write', link.subject_type, link.subject_id, p_visited || ARRAY[v_key]) = 1 AND link.subject_type IN ('project'))
    ) THEN
        v_has_access := TRUE;
    END IF;
    END IF;
    IF v_has_access THEN
        RETURN 1;
    END IF;
    RETURN 0;
END;
$$ LANGUAGE plpgsql STABLE COST 1000
SET search_path = 'public';


-- Generated check function for repository.can_write
-- Features: Recursive
CREATE OR REPLACE FUNCTION "public"."check_repository_can_write"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[]
) RETURNS INTEGER AS $$
DECLARE
    v_has_access BOOLEAN := FALSE;
    v_key TEXT := 'repository:' || p_object_id || ':can_write';
    v_userset_check INTEGER := 0;
BEGIN
    -- Cycle detection
    IF v_key = ANY(p_visited) THEN
        RETURN 0;
    END IF;
    IF array_length(p_visited, 1) >= 25 THEN
        RAISE EXCEPTION 'resolution too complex' USING ERRCODE = 'M2002';
    END IF;
    v_has_access := FALSE;
    -- Userset subject handling
    IF position('#' in p_subject_id) > 0 THEN
        -- Case 1: Self-referential userset check
        IF (p_subject_type = 'repository' AND substring(p_subject_id from 1 for position('#' in p_subject_id) - 1) = p_object_id) THEN
        SELECT INTO v_userset_check 1
    WHERE substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('can_write')
    LIMIT 1;
        IF v_userset_check = 1 THEN
        RETURN 1;
    END IF;
    END IF;
    END IF;
    IF NOT (v_has_access) THEN
        -- Recursive access path via project -> can_write
        IF EXISTS (
    SELECT 1
    FROM melange_tuples AS link
    WHERE (link.object_type = 'repository' AND link.relation IN ('project') AND link.object_id = p_object_id AND "public"."check_permission_internal"(p_subject_type, p_subject_id, 'can_write', link.subject_type, link.subject_id, p_visited || ARRAY[v_key]) = 1 AND link.subject_type IN ('project'))
    ) THEN
        v_has_access := TRUE;
    END IF;
    END IF;
    IF v_has_access THEN
        RETURN 1;
    END IF;
    RETURN 0;
END;
$$ LANGUAGE plpgsql STABLE COST 1000
SET search_path = 'public';


-- Generated check function for organization.can_read
-- Features: Implied
CREATE OR REPLACE FUNCTION "public"."check_organization_can_read"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[]
) RETURNS INTEGER AS $$
DECLARE
    v_userset_check INTEGER := 0;
BEGIN
    -- Userset subject handling
    IF position('#' in p_subject_id) > 0 THEN
        -- Case 1: Self-referential userset check
        IF (p_subject_type = 'organization' AND substring(p_subject_id from 1 for position('#' in p_subject_id) - 1) = p_object_id) THEN
        SELECT INTO v_userset_check 1
    WHERE substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('admin', 'can_read', 'member', 'owner')
    LIMIT 1;
        IF v_userset_check = 1 THEN
        RETURN 1;
    END IF;
    END IF;
    END IF;
    IF EXISTS (
    SELECT 1
    FROM melange_tuples
    WHERE (object_type = 'organization' AND relation IN ('can_read', 'admin', 'member', 'owner') AND object_id = p_object_id AND subject_type IN ('user') AND subject_type = p_subject_type AND (subject_id = p_subject_id AND NOT (subject_id = '*')))
    LIMIT 1
    ) THEN
        RETURN 1;
    ELSE
        RETURN 0;
    END IF;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';


-- Generated check function for state_volume.can_manage
-- Features: Recursive
CREATE OR REPLACE FUNCTION "public"."check_state_volume_can_manage"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[]
) RETURNS INTEGER AS $$
DECLARE
    v_has_access BOOLEAN := FALSE;
    v_key TEXT := 'state_volume:' || p_object_id || ':can_manage';
    v_userset_check INTEGER := 0;
BEGIN
    -- Cycle detection
    IF v_key = ANY(p_visited) THEN
        RETURN 0;
    END IF;
    IF array_length(p_visited, 1) >= 25 THEN
        RAISE EXCEPTION 'resolution too complex' USING ERRCODE = 'M2002';
    END IF;
    v_has_access := FALSE;
    -- Userset subject handling
    IF position('#' in p_subject_id) > 0 THEN
        -- Case 1: Self-referential userset check
        IF (p_subject_type = 'state_volume' AND substring(p_subject_id from 1 for position('#' in p_subject_id) - 1) = p_object_id) THEN
        SELECT INTO v_userset_check 1
    WHERE substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('can_manage')
    LIMIT 1;
        IF v_userset_check = 1 THEN
        RETURN 1;
    END IF;
    END IF;
    END IF;
    IF NOT (v_has_access) THEN
        -- Recursive access path via agent -> can_manage
        IF EXISTS (
    SELECT 1
    FROM melange_tuples AS link
    WHERE (link.object_type = 'state_volume' AND link.relation IN ('agent') AND link.object_id = p_object_id AND "public"."check_permission_internal"(p_subject_type, p_subject_id, 'can_manage', link.subject_type, link.subject_id, p_visited || ARRAY[v_key]) = 1 AND link.subject_type IN ('agent'))
    ) THEN
        v_has_access := TRUE;
    END IF;
    END IF;
    IF v_has_access THEN
        RETURN 1;
    END IF;
    RETURN 0;
END;
$$ LANGUAGE plpgsql STABLE COST 1000
SET search_path = 'public';


-- Generated check function for state_volume.can_restore
-- Features: Recursive
CREATE OR REPLACE FUNCTION "public"."check_state_volume_can_restore"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[]
) RETURNS INTEGER AS $$
DECLARE
    v_has_access BOOLEAN := FALSE;
    v_key TEXT := 'state_volume:' || p_object_id || ':can_restore';
    v_userset_check INTEGER := 0;
BEGIN
    -- Cycle detection
    IF v_key = ANY(p_visited) THEN
        RETURN 0;
    END IF;
    IF array_length(p_visited, 1) >= 25 THEN
        RAISE EXCEPTION 'resolution too complex' USING ERRCODE = 'M2002';
    END IF;
    v_has_access := FALSE;
    -- Userset subject handling
    IF position('#' in p_subject_id) > 0 THEN
        -- Case 1: Self-referential userset check
        IF (p_subject_type = 'state_volume' AND substring(p_subject_id from 1 for position('#' in p_subject_id) - 1) = p_object_id) THEN
        SELECT INTO v_userset_check 1
    WHERE substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('can_restore')
    LIMIT 1;
        IF v_userset_check = 1 THEN
        RETURN 1;
    END IF;
    END IF;
    END IF;
    IF NOT (v_has_access) THEN
        -- Recursive access path via agent -> can_manage
        IF EXISTS (
    SELECT 1
    FROM melange_tuples AS link
    WHERE (link.object_type = 'state_volume' AND link.relation IN ('agent') AND link.object_id = p_object_id AND "public"."check_permission_internal"(p_subject_type, p_subject_id, 'can_manage', link.subject_type, link.subject_id, p_visited || ARRAY[v_key]) = 1 AND link.subject_type IN ('agent'))
    ) THEN
        v_has_access := TRUE;
    END IF;
    END IF;
    IF v_has_access THEN
        RETURN 1;
    END IF;
    RETURN 0;
END;
$$ LANGUAGE plpgsql STABLE COST 1000
SET search_path = 'public';


-- Generated check function for run.can_cancel
-- Features: Recursive
CREATE OR REPLACE FUNCTION "public"."check_run_can_cancel"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[]
) RETURNS INTEGER AS $$
DECLARE
    v_has_access BOOLEAN := FALSE;
    v_key TEXT := 'run:' || p_object_id || ':can_cancel';
    v_userset_check INTEGER := 0;
BEGIN
    -- Cycle detection
    IF v_key = ANY(p_visited) THEN
        RETURN 0;
    END IF;
    IF array_length(p_visited, 1) >= 25 THEN
        RAISE EXCEPTION 'resolution too complex' USING ERRCODE = 'M2002';
    END IF;
    v_has_access := FALSE;
    -- Userset subject handling
    IF position('#' in p_subject_id) > 0 THEN
        -- Case 1: Self-referential userset check
        IF (p_subject_type = 'run' AND substring(p_subject_id from 1 for position('#' in p_subject_id) - 1) = p_object_id) THEN
        SELECT INTO v_userset_check 1
    WHERE substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('can_cancel')
    LIMIT 1;
        IF v_userset_check = 1 THEN
        RETURN 1;
    END IF;
    END IF;
    END IF;
    IF NOT (v_has_access) THEN
        -- Recursive access path via agent -> can_execute
        IF EXISTS (
    SELECT 1
    FROM melange_tuples AS link
    WHERE (link.object_type = 'run' AND link.relation IN ('agent') AND link.object_id = p_object_id AND "public"."check_permission_internal"(p_subject_type, p_subject_id, 'can_execute', link.subject_type, link.subject_id, p_visited || ARRAY[v_key]) = 1 AND link.subject_type IN ('agent'))
    ) THEN
        v_has_access := TRUE;
    END IF;
    END IF;
    IF NOT (v_has_access) THEN
        -- Recursive access path via agent -> can_manage
        IF EXISTS (
    SELECT 1
    FROM melange_tuples AS link
    WHERE (link.object_type = 'run' AND link.relation IN ('agent') AND link.object_id = p_object_id AND "public"."check_permission_internal"(p_subject_type, p_subject_id, 'can_manage', link.subject_type, link.subject_id, p_visited || ARRAY[v_key]) = 1 AND link.subject_type IN ('agent'))
    ) THEN
        v_has_access := TRUE;
    END IF;
    END IF;
    IF v_has_access THEN
        RETURN 1;
    END IF;
    RETURN 0;
END;
$$ LANGUAGE plpgsql STABLE COST 1000
SET search_path = 'public';


-- Generated check function for state_volume.can_attach
-- Features: Recursive
CREATE OR REPLACE FUNCTION "public"."check_state_volume_can_attach"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[]
) RETURNS INTEGER AS $$
DECLARE
    v_has_access BOOLEAN := FALSE;
    v_key TEXT := 'state_volume:' || p_object_id || ':can_attach';
    v_userset_check INTEGER := 0;
BEGIN
    -- Cycle detection
    IF v_key = ANY(p_visited) THEN
        RETURN 0;
    END IF;
    IF array_length(p_visited, 1) >= 25 THEN
        RAISE EXCEPTION 'resolution too complex' USING ERRCODE = 'M2002';
    END IF;
    v_has_access := FALSE;
    -- Userset subject handling
    IF position('#' in p_subject_id) > 0 THEN
        -- Case 1: Self-referential userset check
        IF (p_subject_type = 'state_volume' AND substring(p_subject_id from 1 for position('#' in p_subject_id) - 1) = p_object_id) THEN
        SELECT INTO v_userset_check 1
    WHERE substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('can_attach')
    LIMIT 1;
        IF v_userset_check = 1 THEN
        RETURN 1;
    END IF;
    END IF;
    END IF;
    IF NOT (v_has_access) THEN
        -- Recursive access path via agent -> can_execute
        IF EXISTS (
    SELECT 1
    FROM melange_tuples AS link
    WHERE (link.object_type = 'state_volume' AND link.relation IN ('agent') AND link.object_id = p_object_id AND "public"."check_permission_internal"(p_subject_type, p_subject_id, 'can_execute', link.subject_type, link.subject_id, p_visited || ARRAY[v_key]) = 1 AND link.subject_type IN ('agent'))
    ) THEN
        v_has_access := TRUE;
    END IF;
    END IF;
    IF v_has_access THEN
        RETURN 1;
    END IF;
    RETURN 0;
END;
$$ LANGUAGE plpgsql STABLE COST 1000
SET search_path = 'public';


-- Generated check function for project.can_read
-- Features: Implied+Recursive
CREATE OR REPLACE FUNCTION "public"."check_project_can_read"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[]
) RETURNS INTEGER AS $$
DECLARE
    v_has_access BOOLEAN := FALSE;
    v_key TEXT := 'project:' || p_object_id || ':can_read';
    v_userset_check INTEGER := 0;
BEGIN
    -- Cycle detection
    IF v_key = ANY(p_visited) THEN
        RETURN 0;
    END IF;
    IF array_length(p_visited, 1) >= 25 THEN
        RAISE EXCEPTION 'resolution too complex' USING ERRCODE = 'M2002';
    END IF;
    v_has_access := FALSE;
    -- Userset subject handling
    IF position('#' in p_subject_id) > 0 THEN
        -- Case 1: Self-referential userset check
        IF (p_subject_type = 'project' AND substring(p_subject_id from 1 for position('#' in p_subject_id) - 1) = p_object_id) THEN
        SELECT INTO v_userset_check 1
    WHERE substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('can_read', 'maintainer')
    LIMIT 1;
        IF v_userset_check = 1 THEN
        RETURN 1;
    END IF;
    END IF;
    END IF;
    -- Direct/Implied access path
    IF EXISTS (
    SELECT 1
    FROM melange_tuples
    WHERE (object_type = 'project' AND relation IN ('can_read', 'maintainer') AND object_id = p_object_id AND subject_type IN ('user') AND subject_type = p_subject_type AND (subject_id = p_subject_id AND NOT (subject_id = '*')))
    LIMIT 1
    ) THEN
        v_has_access := TRUE;
    END IF;
    IF NOT (v_has_access) THEN
        -- Recursive access path via organization -> can_read
        IF EXISTS (
    SELECT 1
    FROM melange_tuples AS link
    WHERE (link.object_type = 'project' AND link.relation IN ('organization') AND link.object_id = p_object_id AND "public"."check_permission_internal"(p_subject_type, p_subject_id, 'can_read', link.subject_type, link.subject_id, p_visited || ARRAY[v_key]) = 1 AND link.subject_type IN ('organization'))
    ) THEN
        v_has_access := TRUE;
    END IF;
    END IF;
    IF v_has_access THEN
        RETURN 1;
    END IF;
    RETURN 0;
END;
$$ LANGUAGE plpgsql STABLE COST 1000
SET search_path = 'public';


-- Generated check function for agent.can_read
-- Features: Recursive
CREATE OR REPLACE FUNCTION "public"."check_agent_can_read"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[]
) RETURNS INTEGER AS $$
DECLARE
    v_has_access BOOLEAN := FALSE;
    v_key TEXT := 'agent:' || p_object_id || ':can_read';
    v_userset_check INTEGER := 0;
BEGIN
    -- Cycle detection
    IF v_key = ANY(p_visited) THEN
        RETURN 0;
    END IF;
    IF array_length(p_visited, 1) >= 25 THEN
        RAISE EXCEPTION 'resolution too complex' USING ERRCODE = 'M2002';
    END IF;
    v_has_access := FALSE;
    -- Userset subject handling
    IF position('#' in p_subject_id) > 0 THEN
        -- Case 1: Self-referential userset check
        IF (p_subject_type = 'agent' AND substring(p_subject_id from 1 for position('#' in p_subject_id) - 1) = p_object_id) THEN
        SELECT INTO v_userset_check 1
    WHERE substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('can_read')
    LIMIT 1;
        IF v_userset_check = 1 THEN
        RETURN 1;
    END IF;
    END IF;
    END IF;
    IF NOT (v_has_access) THEN
        -- Recursive access path via project -> can_read
        IF EXISTS (
    SELECT 1
    FROM melange_tuples AS link
    WHERE (link.object_type = 'agent' AND link.relation IN ('project') AND link.object_id = p_object_id AND "public"."check_permission_internal"(p_subject_type, p_subject_id, 'can_read', link.subject_type, link.subject_id, p_visited || ARRAY[v_key]) = 1 AND link.subject_type IN ('project'))
    ) THEN
        v_has_access := TRUE;
    END IF;
    END IF;
    IF v_has_access THEN
        RETURN 1;
    END IF;
    RETURN 0;
END;
$$ LANGUAGE plpgsql STABLE COST 1000
SET search_path = 'public';


-- Generated check function for repository.can_read
-- Features: Implied+Wildcard+Recursive
CREATE OR REPLACE FUNCTION "public"."check_repository_can_read"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[]
) RETURNS INTEGER AS $$
DECLARE
    v_has_access BOOLEAN := FALSE;
    v_key TEXT := 'repository:' || p_object_id || ':can_read';
    v_userset_check INTEGER := 0;
BEGIN
    -- Cycle detection
    IF v_key = ANY(p_visited) THEN
        RETURN 0;
    END IF;
    IF array_length(p_visited, 1) >= 25 THEN
        RAISE EXCEPTION 'resolution too complex' USING ERRCODE = 'M2002';
    END IF;
    v_has_access := FALSE;
    -- Userset subject handling
    IF position('#' in p_subject_id) > 0 THEN
        -- Case 1: Self-referential userset check
        IF (p_subject_type = 'repository' AND substring(p_subject_id from 1 for position('#' in p_subject_id) - 1) = p_object_id) THEN
        SELECT INTO v_userset_check 1
    WHERE substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('can_read', 'public')
    LIMIT 1;
        IF v_userset_check = 1 THEN
        RETURN 1;
    END IF;
    END IF;
    END IF;
    -- Direct/Implied access path
    IF EXISTS (
    SELECT 1
    FROM melange_tuples
    WHERE (object_type = 'repository' AND relation IN ('can_read', 'public') AND object_id = p_object_id AND subject_type IN ('user') AND subject_type = p_subject_type AND (subject_id = p_subject_id OR subject_id = '*'))
    LIMIT 1
    ) THEN
        v_has_access := TRUE;
    END IF;
    IF NOT (v_has_access) THEN
        -- Recursive access path via project -> can_read
        IF EXISTS (
    SELECT 1
    FROM melange_tuples AS link
    WHERE (link.object_type = 'repository' AND link.relation IN ('project') AND link.object_id = p_object_id AND "public"."check_permission_internal"(p_subject_type, p_subject_id, 'can_read', link.subject_type, link.subject_id, p_visited || ARRAY[v_key]) = 1 AND link.subject_type IN ('project'))
    ) THEN
        v_has_access := TRUE;
    END IF;
    END IF;
    IF v_has_access THEN
        RETURN 1;
    END IF;
    RETURN 0;
END;
$$ LANGUAGE plpgsql STABLE COST 1000
SET search_path = 'public';


-- Generated check function for run.can_read
-- Features: Recursive
CREATE OR REPLACE FUNCTION "public"."check_run_can_read"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[]
) RETURNS INTEGER AS $$
DECLARE
    v_has_access BOOLEAN := FALSE;
    v_key TEXT := 'run:' || p_object_id || ':can_read';
    v_userset_check INTEGER := 0;
BEGIN
    -- Cycle detection
    IF v_key = ANY(p_visited) THEN
        RETURN 0;
    END IF;
    IF array_length(p_visited, 1) >= 25 THEN
        RAISE EXCEPTION 'resolution too complex' USING ERRCODE = 'M2002';
    END IF;
    v_has_access := FALSE;
    -- Userset subject handling
    IF position('#' in p_subject_id) > 0 THEN
        -- Case 1: Self-referential userset check
        IF (p_subject_type = 'run' AND substring(p_subject_id from 1 for position('#' in p_subject_id) - 1) = p_object_id) THEN
        SELECT INTO v_userset_check 1
    WHERE substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('can_read')
    LIMIT 1;
        IF v_userset_check = 1 THEN
        RETURN 1;
    END IF;
    END IF;
    END IF;
    IF NOT (v_has_access) THEN
        -- Recursive access path via agent -> can_read
        IF EXISTS (
    SELECT 1
    FROM melange_tuples AS link
    WHERE (link.object_type = 'run' AND link.relation IN ('agent') AND link.object_id = p_object_id AND "public"."check_permission_internal"(p_subject_type, p_subject_id, 'can_read', link.subject_type, link.subject_id, p_visited || ARRAY[v_key]) = 1 AND link.subject_type IN ('agent'))
    ) THEN
        v_has_access := TRUE;
    END IF;
    END IF;
    IF v_has_access THEN
        RETURN 1;
    END IF;
    RETURN 0;
END;
$$ LANGUAGE plpgsql STABLE COST 1000
SET search_path = 'public';


-- Generated check function for state_volume.can_read
-- Features: Recursive
CREATE OR REPLACE FUNCTION "public"."check_state_volume_can_read"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[]
) RETURNS INTEGER AS $$
DECLARE
    v_has_access BOOLEAN := FALSE;
    v_key TEXT := 'state_volume:' || p_object_id || ':can_read';
    v_userset_check INTEGER := 0;
BEGIN
    -- Cycle detection
    IF v_key = ANY(p_visited) THEN
        RETURN 0;
    END IF;
    IF array_length(p_visited, 1) >= 25 THEN
        RAISE EXCEPTION 'resolution too complex' USING ERRCODE = 'M2002';
    END IF;
    v_has_access := FALSE;
    -- Userset subject handling
    IF position('#' in p_subject_id) > 0 THEN
        -- Case 1: Self-referential userset check
        IF (p_subject_type = 'state_volume' AND substring(p_subject_id from 1 for position('#' in p_subject_id) - 1) = p_object_id) THEN
        SELECT INTO v_userset_check 1
    WHERE substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('can_read')
    LIMIT 1;
        IF v_userset_check = 1 THEN
        RETURN 1;
    END IF;
    END IF;
    END IF;
    IF NOT (v_has_access) THEN
        -- Recursive access path via agent -> can_read
        IF EXISTS (
    SELECT 1
    FROM melange_tuples AS link
    WHERE (link.object_type = 'state_volume' AND link.relation IN ('agent') AND link.object_id = p_object_id AND "public"."check_permission_internal"(p_subject_type, p_subject_id, 'can_read', link.subject_type, link.subject_id, p_visited || ARRAY[v_key]) = 1 AND link.subject_type IN ('agent'))
    ) THEN
        v_has_access := TRUE;
    END IF;
    END IF;
    IF v_has_access THEN
        RETURN 1;
    END IF;
    RETURN 0;
END;
$$ LANGUAGE plpgsql STABLE COST 1000
SET search_path = 'public';


-- ============================================================
-- No-Wildcard Check Functions (31 functions)
-- ============================================================

-- Generated check function for agent.project
-- Features: Direct
CREATE OR REPLACE FUNCTION "public"."check_agent_project_nw"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[]
) RETURNS INTEGER AS $$
DECLARE
    v_userset_check INTEGER := 0;
BEGIN
    -- Userset subject handling
    IF position('#' in p_subject_id) > 0 THEN
        -- Case 1: Self-referential userset check
        IF (p_subject_type = 'agent' AND substring(p_subject_id from 1 for position('#' in p_subject_id) - 1) = p_object_id) THEN
        SELECT INTO v_userset_check 1
    WHERE substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('project')
    LIMIT 1;
        IF v_userset_check = 1 THEN
        RETURN 1;
    END IF;
    END IF;
    END IF;
    IF EXISTS (
    SELECT 1
    FROM melange_tuples
    WHERE (object_type = 'agent' AND relation IN ('project') AND object_id = p_object_id AND subject_type IN ('project') AND subject_type = p_subject_type AND (subject_id = p_subject_id AND NOT (subject_id = '*')))
    LIMIT 1
    ) THEN
        RETURN 1;
    ELSE
        RETURN 0;
    END IF;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';


-- Generated check function for organization.owner
-- Features: Direct
CREATE OR REPLACE FUNCTION "public"."check_organization_owner_nw"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[]
) RETURNS INTEGER AS $$
DECLARE
    v_userset_check INTEGER := 0;
BEGIN
    -- Userset subject handling
    IF position('#' in p_subject_id) > 0 THEN
        -- Case 1: Self-referential userset check
        IF (p_subject_type = 'organization' AND substring(p_subject_id from 1 for position('#' in p_subject_id) - 1) = p_object_id) THEN
        SELECT INTO v_userset_check 1
    WHERE substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('owner')
    LIMIT 1;
        IF v_userset_check = 1 THEN
        RETURN 1;
    END IF;
    END IF;
    END IF;
    IF EXISTS (
    SELECT 1
    FROM melange_tuples
    WHERE (object_type = 'organization' AND relation IN ('owner') AND object_id = p_object_id AND subject_type IN ('user') AND subject_type = p_subject_type AND (subject_id = p_subject_id AND NOT (subject_id = '*')))
    LIMIT 1
    ) THEN
        RETURN 1;
    ELSE
        RETURN 0;
    END IF;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';


-- Generated check function for project.maintainer
-- Features: Direct
CREATE OR REPLACE FUNCTION "public"."check_project_maintainer_nw"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[]
) RETURNS INTEGER AS $$
DECLARE
    v_userset_check INTEGER := 0;
BEGIN
    -- Userset subject handling
    IF position('#' in p_subject_id) > 0 THEN
        -- Case 1: Self-referential userset check
        IF (p_subject_type = 'project' AND substring(p_subject_id from 1 for position('#' in p_subject_id) - 1) = p_object_id) THEN
        SELECT INTO v_userset_check 1
    WHERE substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('maintainer')
    LIMIT 1;
        IF v_userset_check = 1 THEN
        RETURN 1;
    END IF;
    END IF;
    END IF;
    IF EXISTS (
    SELECT 1
    FROM melange_tuples
    WHERE (object_type = 'project' AND relation IN ('maintainer') AND object_id = p_object_id AND subject_type IN ('user') AND subject_type = p_subject_type AND (subject_id = p_subject_id AND NOT (subject_id = '*')))
    LIMIT 1
    ) THEN
        RETURN 1;
    ELSE
        RETURN 0;
    END IF;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';


-- Generated check function for project.organization
-- Features: Direct
CREATE OR REPLACE FUNCTION "public"."check_project_organization_nw"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[]
) RETURNS INTEGER AS $$
DECLARE
    v_userset_check INTEGER := 0;
BEGIN
    -- Userset subject handling
    IF position('#' in p_subject_id) > 0 THEN
        -- Case 1: Self-referential userset check
        IF (p_subject_type = 'project' AND substring(p_subject_id from 1 for position('#' in p_subject_id) - 1) = p_object_id) THEN
        SELECT INTO v_userset_check 1
    WHERE substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('organization')
    LIMIT 1;
        IF v_userset_check = 1 THEN
        RETURN 1;
    END IF;
    END IF;
    END IF;
    IF EXISTS (
    SELECT 1
    FROM melange_tuples
    WHERE (object_type = 'project' AND relation IN ('organization') AND object_id = p_object_id AND subject_type IN ('organization') AND subject_type = p_subject_type AND (subject_id = p_subject_id AND NOT (subject_id = '*')))
    LIMIT 1
    ) THEN
        RETURN 1;
    ELSE
        RETURN 0;
    END IF;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';


-- Generated check function for repository.project
-- Features: Direct
CREATE OR REPLACE FUNCTION "public"."check_repository_project_nw"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[]
) RETURNS INTEGER AS $$
DECLARE
    v_userset_check INTEGER := 0;
BEGIN
    -- Userset subject handling
    IF position('#' in p_subject_id) > 0 THEN
        -- Case 1: Self-referential userset check
        IF (p_subject_type = 'repository' AND substring(p_subject_id from 1 for position('#' in p_subject_id) - 1) = p_object_id) THEN
        SELECT INTO v_userset_check 1
    WHERE substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('project')
    LIMIT 1;
        IF v_userset_check = 1 THEN
        RETURN 1;
    END IF;
    END IF;
    END IF;
    IF EXISTS (
    SELECT 1
    FROM melange_tuples
    WHERE (object_type = 'repository' AND relation IN ('project') AND object_id = p_object_id AND subject_type IN ('project') AND subject_type = p_subject_type AND (subject_id = p_subject_id AND NOT (subject_id = '*')))
    LIMIT 1
    ) THEN
        RETURN 1;
    ELSE
        RETURN 0;
    END IF;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';


-- Generated check function for repository.public
-- Features: Direct+Wildcard
CREATE OR REPLACE FUNCTION "public"."check_repository_public_nw"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[]
) RETURNS INTEGER AS $$
DECLARE
    v_userset_check INTEGER := 0;
BEGIN
    -- Userset subject handling
    IF position('#' in p_subject_id) > 0 THEN
        -- Case 1: Self-referential userset check
        IF (p_subject_type = 'repository' AND substring(p_subject_id from 1 for position('#' in p_subject_id) - 1) = p_object_id) THEN
        SELECT INTO v_userset_check 1
    WHERE substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('public')
    LIMIT 1;
        IF v_userset_check = 1 THEN
        RETURN 1;
    END IF;
    END IF;
    END IF;
    IF EXISTS (
    SELECT 1
    FROM melange_tuples
    WHERE (object_type = 'repository' AND relation IN ('public') AND object_id = p_object_id AND subject_type IN ('user') AND subject_type = p_subject_type AND (subject_id = p_subject_id AND NOT (subject_id = '*')))
    LIMIT 1
    ) THEN
        RETURN 1;
    ELSE
        RETURN 0;
    END IF;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';


-- Generated check function for run.agent
-- Features: Direct
CREATE OR REPLACE FUNCTION "public"."check_run_agent_nw"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[]
) RETURNS INTEGER AS $$
DECLARE
    v_userset_check INTEGER := 0;
BEGIN
    -- Userset subject handling
    IF position('#' in p_subject_id) > 0 THEN
        -- Case 1: Self-referential userset check
        IF (p_subject_type = 'run' AND substring(p_subject_id from 1 for position('#' in p_subject_id) - 1) = p_object_id) THEN
        SELECT INTO v_userset_check 1
    WHERE substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('agent')
    LIMIT 1;
        IF v_userset_check = 1 THEN
        RETURN 1;
    END IF;
    END IF;
    END IF;
    IF EXISTS (
    SELECT 1
    FROM melange_tuples
    WHERE (object_type = 'run' AND relation IN ('agent') AND object_id = p_object_id AND subject_type IN ('agent') AND subject_type = p_subject_type AND (subject_id = p_subject_id AND NOT (subject_id = '*')))
    LIMIT 1
    ) THEN
        RETURN 1;
    ELSE
        RETURN 0;
    END IF;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';


-- Generated check function for state_volume.agent
-- Features: Direct
CREATE OR REPLACE FUNCTION "public"."check_state_volume_agent_nw"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[]
) RETURNS INTEGER AS $$
DECLARE
    v_userset_check INTEGER := 0;
BEGIN
    -- Userset subject handling
    IF position('#' in p_subject_id) > 0 THEN
        -- Case 1: Self-referential userset check
        IF (p_subject_type = 'state_volume' AND substring(p_subject_id from 1 for position('#' in p_subject_id) - 1) = p_object_id) THEN
        SELECT INTO v_userset_check 1
    WHERE substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('agent')
    LIMIT 1;
        IF v_userset_check = 1 THEN
        RETURN 1;
    END IF;
    END IF;
    END IF;
    IF EXISTS (
    SELECT 1
    FROM melange_tuples
    WHERE (object_type = 'state_volume' AND relation IN ('agent') AND object_id = p_object_id AND subject_type IN ('agent') AND subject_type = p_subject_type AND (subject_id = p_subject_id AND NOT (subject_id = '*')))
    LIMIT 1
    ) THEN
        RETURN 1;
    ELSE
        RETURN 0;
    END IF;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';


-- Generated check function for organization.admin
-- Features: Direct+Implied
CREATE OR REPLACE FUNCTION "public"."check_organization_admin_nw"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[]
) RETURNS INTEGER AS $$
DECLARE
    v_userset_check INTEGER := 0;
BEGIN
    -- Userset subject handling
    IF position('#' in p_subject_id) > 0 THEN
        -- Case 1: Self-referential userset check
        IF (p_subject_type = 'organization' AND substring(p_subject_id from 1 for position('#' in p_subject_id) - 1) = p_object_id) THEN
        SELECT INTO v_userset_check 1
    WHERE substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('admin', 'owner')
    LIMIT 1;
        IF v_userset_check = 1 THEN
        RETURN 1;
    END IF;
    END IF;
    END IF;
    IF EXISTS (
    SELECT 1
    FROM melange_tuples
    WHERE (object_type = 'organization' AND relation IN ('admin', 'owner') AND object_id = p_object_id AND subject_type IN ('user') AND subject_type = p_subject_type AND (subject_id = p_subject_id AND NOT (subject_id = '*')))
    LIMIT 1
    ) THEN
        RETURN 1;
    ELSE
        RETURN 0;
    END IF;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';


-- Generated check function for organization.can_delete
-- Features: Implied
CREATE OR REPLACE FUNCTION "public"."check_organization_can_delete_nw"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[]
) RETURNS INTEGER AS $$
DECLARE
    v_userset_check INTEGER := 0;
BEGIN
    -- Userset subject handling
    IF position('#' in p_subject_id) > 0 THEN
        -- Case 1: Self-referential userset check
        IF (p_subject_type = 'organization' AND substring(p_subject_id from 1 for position('#' in p_subject_id) - 1) = p_object_id) THEN
        SELECT INTO v_userset_check 1
    WHERE substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('can_delete', 'owner')
    LIMIT 1;
        IF v_userset_check = 1 THEN
        RETURN 1;
    END IF;
    END IF;
    END IF;
    IF EXISTS (
    SELECT 1
    FROM melange_tuples
    WHERE (object_type = 'organization' AND relation IN ('can_delete', 'owner') AND object_id = p_object_id AND subject_type IN ('user') AND subject_type = p_subject_type AND (subject_id = p_subject_id AND NOT (subject_id = '*')))
    LIMIT 1
    ) THEN
        RETURN 1;
    ELSE
        RETURN 0;
    END IF;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';


-- Generated check function for organization.can_manage
-- Features: Implied
CREATE OR REPLACE FUNCTION "public"."check_organization_can_manage_nw"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[]
) RETURNS INTEGER AS $$
DECLARE
    v_userset_check INTEGER := 0;
BEGIN
    -- Userset subject handling
    IF position('#' in p_subject_id) > 0 THEN
        -- Case 1: Self-referential userset check
        IF (p_subject_type = 'organization' AND substring(p_subject_id from 1 for position('#' in p_subject_id) - 1) = p_object_id) THEN
        SELECT INTO v_userset_check 1
    WHERE substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('can_manage', 'owner')
    LIMIT 1;
        IF v_userset_check = 1 THEN
        RETURN 1;
    END IF;
    END IF;
    END IF;
    IF EXISTS (
    SELECT 1
    FROM melange_tuples
    WHERE (object_type = 'organization' AND relation IN ('can_manage', 'owner') AND object_id = p_object_id AND subject_type IN ('user') AND subject_type = p_subject_type AND (subject_id = p_subject_id AND NOT (subject_id = '*')))
    LIMIT 1
    ) THEN
        RETURN 1;
    ELSE
        RETURN 0;
    END IF;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';


-- Generated check function for project.can_delete
-- Features: Recursive
CREATE OR REPLACE FUNCTION "public"."check_project_can_delete_nw"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[]
) RETURNS INTEGER AS $$
DECLARE
    v_has_access BOOLEAN := FALSE;
    v_key TEXT := 'project:' || p_object_id || ':can_delete';
    v_userset_check INTEGER := 0;
BEGIN
    -- Cycle detection
    IF v_key = ANY(p_visited) THEN
        RETURN 0;
    END IF;
    IF array_length(p_visited, 1) >= 25 THEN
        RAISE EXCEPTION 'resolution too complex' USING ERRCODE = 'M2002';
    END IF;
    v_has_access := FALSE;
    -- Userset subject handling
    IF position('#' in p_subject_id) > 0 THEN
        -- Case 1: Self-referential userset check
        IF (p_subject_type = 'project' AND substring(p_subject_id from 1 for position('#' in p_subject_id) - 1) = p_object_id) THEN
        SELECT INTO v_userset_check 1
    WHERE substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('can_delete')
    LIMIT 1;
        IF v_userset_check = 1 THEN
        RETURN 1;
    END IF;
    END IF;
    END IF;
    IF NOT (v_has_access) THEN
        -- Recursive access path via organization -> owner
        IF EXISTS (
    SELECT 1
    FROM melange_tuples AS link
    WHERE (link.object_type = 'project' AND link.relation IN ('organization') AND link.object_id = p_object_id AND "public"."check_permission_internal"(p_subject_type, p_subject_id, 'owner', link.subject_type, link.subject_id, p_visited || ARRAY[v_key]) = 1 AND link.subject_type IN ('organization'))
    ) THEN
        v_has_access := TRUE;
    END IF;
    END IF;
    IF v_has_access THEN
        RETURN 1;
    END IF;
    RETURN 0;
END;
$$ LANGUAGE plpgsql STABLE COST 1000
SET search_path = 'public';


-- Generated check function for project.can_manage
-- Features: Implied
CREATE OR REPLACE FUNCTION "public"."check_project_can_manage_nw"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[]
) RETURNS INTEGER AS $$
DECLARE
    v_userset_check INTEGER := 0;
BEGIN
    -- Userset subject handling
    IF position('#' in p_subject_id) > 0 THEN
        -- Case 1: Self-referential userset check
        IF (p_subject_type = 'project' AND substring(p_subject_id from 1 for position('#' in p_subject_id) - 1) = p_object_id) THEN
        SELECT INTO v_userset_check 1
    WHERE substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('can_manage', 'maintainer')
    LIMIT 1;
        IF v_userset_check = 1 THEN
        RETURN 1;
    END IF;
    END IF;
    END IF;
    IF EXISTS (
    SELECT 1
    FROM melange_tuples
    WHERE (object_type = 'project' AND relation IN ('can_manage', 'maintainer') AND object_id = p_object_id AND subject_type IN ('user') AND subject_type = p_subject_type AND (subject_id = p_subject_id AND NOT (subject_id = '*')))
    LIMIT 1
    ) THEN
        RETURN 1;
    ELSE
        RETURN 0;
    END IF;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';


-- Generated check function for project.can_write
-- Features: Implied
CREATE OR REPLACE FUNCTION "public"."check_project_can_write_nw"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[]
) RETURNS INTEGER AS $$
DECLARE
    v_userset_check INTEGER := 0;
BEGIN
    -- Userset subject handling
    IF position('#' in p_subject_id) > 0 THEN
        -- Case 1: Self-referential userset check
        IF (p_subject_type = 'project' AND substring(p_subject_id from 1 for position('#' in p_subject_id) - 1) = p_object_id) THEN
        SELECT INTO v_userset_check 1
    WHERE substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('can_write', 'maintainer')
    LIMIT 1;
        IF v_userset_check = 1 THEN
        RETURN 1;
    END IF;
    END IF;
    END IF;
    IF EXISTS (
    SELECT 1
    FROM melange_tuples
    WHERE (object_type = 'project' AND relation IN ('can_write', 'maintainer') AND object_id = p_object_id AND subject_type IN ('user') AND subject_type = p_subject_type AND (subject_id = p_subject_id AND NOT (subject_id = '*')))
    LIMIT 1
    ) THEN
        RETURN 1;
    ELSE
        RETURN 0;
    END IF;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';


-- Generated check function for organization.can_create_project
-- Features: Implied
CREATE OR REPLACE FUNCTION "public"."check_organization_can_create_project_nw"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[]
) RETURNS INTEGER AS $$
DECLARE
    v_userset_check INTEGER := 0;
BEGIN
    -- Userset subject handling
    IF position('#' in p_subject_id) > 0 THEN
        -- Case 1: Self-referential userset check
        IF (p_subject_type = 'organization' AND substring(p_subject_id from 1 for position('#' in p_subject_id) - 1) = p_object_id) THEN
        SELECT INTO v_userset_check 1
    WHERE substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('admin', 'can_create_project', 'owner')
    LIMIT 1;
        IF v_userset_check = 1 THEN
        RETURN 1;
    END IF;
    END IF;
    END IF;
    IF EXISTS (
    SELECT 1
    FROM melange_tuples
    WHERE (object_type = 'organization' AND relation IN ('can_create_project', 'admin', 'owner') AND object_id = p_object_id AND subject_type IN ('user') AND subject_type = p_subject_type AND (subject_id = p_subject_id AND NOT (subject_id = '*')))
    LIMIT 1
    ) THEN
        RETURN 1;
    ELSE
        RETURN 0;
    END IF;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';


-- Generated check function for organization.can_manage_members
-- Features: Implied
CREATE OR REPLACE FUNCTION "public"."check_organization_can_manage_members_nw"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[]
) RETURNS INTEGER AS $$
DECLARE
    v_userset_check INTEGER := 0;
BEGIN
    -- Userset subject handling
    IF position('#' in p_subject_id) > 0 THEN
        -- Case 1: Self-referential userset check
        IF (p_subject_type = 'organization' AND substring(p_subject_id from 1 for position('#' in p_subject_id) - 1) = p_object_id) THEN
        SELECT INTO v_userset_check 1
    WHERE substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('admin', 'can_manage_members', 'owner')
    LIMIT 1;
        IF v_userset_check = 1 THEN
        RETURN 1;
    END IF;
    END IF;
    END IF;
    IF EXISTS (
    SELECT 1
    FROM melange_tuples
    WHERE (object_type = 'organization' AND relation IN ('can_manage_members', 'admin', 'owner') AND object_id = p_object_id AND subject_type IN ('user') AND subject_type = p_subject_type AND (subject_id = p_subject_id AND NOT (subject_id = '*')))
    LIMIT 1
    ) THEN
        RETURN 1;
    ELSE
        RETURN 0;
    END IF;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';


-- Generated check function for organization.member
-- Features: Direct+Implied
CREATE OR REPLACE FUNCTION "public"."check_organization_member_nw"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[]
) RETURNS INTEGER AS $$
DECLARE
    v_userset_check INTEGER := 0;
BEGIN
    -- Userset subject handling
    IF position('#' in p_subject_id) > 0 THEN
        -- Case 1: Self-referential userset check
        IF (p_subject_type = 'organization' AND substring(p_subject_id from 1 for position('#' in p_subject_id) - 1) = p_object_id) THEN
        SELECT INTO v_userset_check 1
    WHERE substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('admin', 'member', 'owner')
    LIMIT 1;
        IF v_userset_check = 1 THEN
        RETURN 1;
    END IF;
    END IF;
    END IF;
    IF EXISTS (
    SELECT 1
    FROM melange_tuples
    WHERE (object_type = 'organization' AND relation IN ('member', 'admin', 'owner') AND object_id = p_object_id AND subject_type IN ('user') AND subject_type = p_subject_type AND (subject_id = p_subject_id AND NOT (subject_id = '*')))
    LIMIT 1
    ) THEN
        RETURN 1;
    ELSE
        RETURN 0;
    END IF;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';


-- Generated check function for repository.can_delete
-- Features: Recursive
CREATE OR REPLACE FUNCTION "public"."check_repository_can_delete_nw"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[]
) RETURNS INTEGER AS $$
DECLARE
    v_has_access BOOLEAN := FALSE;
    v_key TEXT := 'repository:' || p_object_id || ':can_delete';
    v_userset_check INTEGER := 0;
BEGIN
    -- Cycle detection
    IF v_key = ANY(p_visited) THEN
        RETURN 0;
    END IF;
    IF array_length(p_visited, 1) >= 25 THEN
        RAISE EXCEPTION 'resolution too complex' USING ERRCODE = 'M2002';
    END IF;
    v_has_access := FALSE;
    -- Userset subject handling
    IF position('#' in p_subject_id) > 0 THEN
        -- Case 1: Self-referential userset check
        IF (p_subject_type = 'repository' AND substring(p_subject_id from 1 for position('#' in p_subject_id) - 1) = p_object_id) THEN
        SELECT INTO v_userset_check 1
    WHERE substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('can_delete')
    LIMIT 1;
        IF v_userset_check = 1 THEN
        RETURN 1;
    END IF;
    END IF;
    END IF;
    IF NOT (v_has_access) THEN
        -- Recursive access path via project -> can_delete
        IF EXISTS (
    SELECT 1
    FROM melange_tuples AS link
    WHERE (link.object_type = 'repository' AND link.relation IN ('project') AND link.object_id = p_object_id AND "public"."check_permission_internal"(p_subject_type, p_subject_id, 'can_delete', link.subject_type, link.subject_id, p_visited || ARRAY[v_key]) = 1 AND link.subject_type IN ('project'))
    ) THEN
        v_has_access := TRUE;
    END IF;
    END IF;
    IF v_has_access THEN
        RETURN 1;
    END IF;
    RETURN 0;
END;
$$ LANGUAGE plpgsql STABLE COST 1000
SET search_path = 'public';


-- Generated check function for agent.can_manage
-- Features: Recursive
CREATE OR REPLACE FUNCTION "public"."check_agent_can_manage_nw"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[]
) RETURNS INTEGER AS $$
DECLARE
    v_has_access BOOLEAN := FALSE;
    v_key TEXT := 'agent:' || p_object_id || ':can_manage';
    v_userset_check INTEGER := 0;
BEGIN
    -- Cycle detection
    IF v_key = ANY(p_visited) THEN
        RETURN 0;
    END IF;
    IF array_length(p_visited, 1) >= 25 THEN
        RAISE EXCEPTION 'resolution too complex' USING ERRCODE = 'M2002';
    END IF;
    v_has_access := FALSE;
    -- Userset subject handling
    IF position('#' in p_subject_id) > 0 THEN
        -- Case 1: Self-referential userset check
        IF (p_subject_type = 'agent' AND substring(p_subject_id from 1 for position('#' in p_subject_id) - 1) = p_object_id) THEN
        SELECT INTO v_userset_check 1
    WHERE substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('can_manage')
    LIMIT 1;
        IF v_userset_check = 1 THEN
        RETURN 1;
    END IF;
    END IF;
    END IF;
    IF NOT (v_has_access) THEN
        -- Recursive access path via project -> can_manage
        IF EXISTS (
    SELECT 1
    FROM melange_tuples AS link
    WHERE (link.object_type = 'agent' AND link.relation IN ('project') AND link.object_id = p_object_id AND "public"."check_permission_internal"(p_subject_type, p_subject_id, 'can_manage', link.subject_type, link.subject_id, p_visited || ARRAY[v_key]) = 1 AND link.subject_type IN ('project'))
    ) THEN
        v_has_access := TRUE;
    END IF;
    END IF;
    IF v_has_access THEN
        RETURN 1;
    END IF;
    RETURN 0;
END;
$$ LANGUAGE plpgsql STABLE COST 1000
SET search_path = 'public';


-- Generated check function for agent.can_execute
-- Features: Recursive
CREATE OR REPLACE FUNCTION "public"."check_agent_can_execute_nw"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[]
) RETURNS INTEGER AS $$
DECLARE
    v_has_access BOOLEAN := FALSE;
    v_key TEXT := 'agent:' || p_object_id || ':can_execute';
    v_userset_check INTEGER := 0;
BEGIN
    -- Cycle detection
    IF v_key = ANY(p_visited) THEN
        RETURN 0;
    END IF;
    IF array_length(p_visited, 1) >= 25 THEN
        RAISE EXCEPTION 'resolution too complex' USING ERRCODE = 'M2002';
    END IF;
    v_has_access := FALSE;
    -- Userset subject handling
    IF position('#' in p_subject_id) > 0 THEN
        -- Case 1: Self-referential userset check
        IF (p_subject_type = 'agent' AND substring(p_subject_id from 1 for position('#' in p_subject_id) - 1) = p_object_id) THEN
        SELECT INTO v_userset_check 1
    WHERE substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('can_execute')
    LIMIT 1;
        IF v_userset_check = 1 THEN
        RETURN 1;
    END IF;
    END IF;
    END IF;
    IF NOT (v_has_access) THEN
        -- Recursive access path via project -> can_write
        IF EXISTS (
    SELECT 1
    FROM melange_tuples AS link
    WHERE (link.object_type = 'agent' AND link.relation IN ('project') AND link.object_id = p_object_id AND "public"."check_permission_internal"(p_subject_type, p_subject_id, 'can_write', link.subject_type, link.subject_id, p_visited || ARRAY[v_key]) = 1 AND link.subject_type IN ('project'))
    ) THEN
        v_has_access := TRUE;
    END IF;
    END IF;
    IF v_has_access THEN
        RETURN 1;
    END IF;
    RETURN 0;
END;
$$ LANGUAGE plpgsql STABLE COST 1000
SET search_path = 'public';


-- Generated check function for repository.can_write
-- Features: Recursive
CREATE OR REPLACE FUNCTION "public"."check_repository_can_write_nw"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[]
) RETURNS INTEGER AS $$
DECLARE
    v_has_access BOOLEAN := FALSE;
    v_key TEXT := 'repository:' || p_object_id || ':can_write';
    v_userset_check INTEGER := 0;
BEGIN
    -- Cycle detection
    IF v_key = ANY(p_visited) THEN
        RETURN 0;
    END IF;
    IF array_length(p_visited, 1) >= 25 THEN
        RAISE EXCEPTION 'resolution too complex' USING ERRCODE = 'M2002';
    END IF;
    v_has_access := FALSE;
    -- Userset subject handling
    IF position('#' in p_subject_id) > 0 THEN
        -- Case 1: Self-referential userset check
        IF (p_subject_type = 'repository' AND substring(p_subject_id from 1 for position('#' in p_subject_id) - 1) = p_object_id) THEN
        SELECT INTO v_userset_check 1
    WHERE substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('can_write')
    LIMIT 1;
        IF v_userset_check = 1 THEN
        RETURN 1;
    END IF;
    END IF;
    END IF;
    IF NOT (v_has_access) THEN
        -- Recursive access path via project -> can_write
        IF EXISTS (
    SELECT 1
    FROM melange_tuples AS link
    WHERE (link.object_type = 'repository' AND link.relation IN ('project') AND link.object_id = p_object_id AND "public"."check_permission_internal"(p_subject_type, p_subject_id, 'can_write', link.subject_type, link.subject_id, p_visited || ARRAY[v_key]) = 1 AND link.subject_type IN ('project'))
    ) THEN
        v_has_access := TRUE;
    END IF;
    END IF;
    IF v_has_access THEN
        RETURN 1;
    END IF;
    RETURN 0;
END;
$$ LANGUAGE plpgsql STABLE COST 1000
SET search_path = 'public';


-- Generated check function for organization.can_read
-- Features: Implied
CREATE OR REPLACE FUNCTION "public"."check_organization_can_read_nw"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[]
) RETURNS INTEGER AS $$
DECLARE
    v_userset_check INTEGER := 0;
BEGIN
    -- Userset subject handling
    IF position('#' in p_subject_id) > 0 THEN
        -- Case 1: Self-referential userset check
        IF (p_subject_type = 'organization' AND substring(p_subject_id from 1 for position('#' in p_subject_id) - 1) = p_object_id) THEN
        SELECT INTO v_userset_check 1
    WHERE substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('admin', 'can_read', 'member', 'owner')
    LIMIT 1;
        IF v_userset_check = 1 THEN
        RETURN 1;
    END IF;
    END IF;
    END IF;
    IF EXISTS (
    SELECT 1
    FROM melange_tuples
    WHERE (object_type = 'organization' AND relation IN ('can_read', 'admin', 'member', 'owner') AND object_id = p_object_id AND subject_type IN ('user') AND subject_type = p_subject_type AND (subject_id = p_subject_id AND NOT (subject_id = '*')))
    LIMIT 1
    ) THEN
        RETURN 1;
    ELSE
        RETURN 0;
    END IF;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';


-- Generated check function for state_volume.can_manage
-- Features: Recursive
CREATE OR REPLACE FUNCTION "public"."check_state_volume_can_manage_nw"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[]
) RETURNS INTEGER AS $$
DECLARE
    v_has_access BOOLEAN := FALSE;
    v_key TEXT := 'state_volume:' || p_object_id || ':can_manage';
    v_userset_check INTEGER := 0;
BEGIN
    -- Cycle detection
    IF v_key = ANY(p_visited) THEN
        RETURN 0;
    END IF;
    IF array_length(p_visited, 1) >= 25 THEN
        RAISE EXCEPTION 'resolution too complex' USING ERRCODE = 'M2002';
    END IF;
    v_has_access := FALSE;
    -- Userset subject handling
    IF position('#' in p_subject_id) > 0 THEN
        -- Case 1: Self-referential userset check
        IF (p_subject_type = 'state_volume' AND substring(p_subject_id from 1 for position('#' in p_subject_id) - 1) = p_object_id) THEN
        SELECT INTO v_userset_check 1
    WHERE substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('can_manage')
    LIMIT 1;
        IF v_userset_check = 1 THEN
        RETURN 1;
    END IF;
    END IF;
    END IF;
    IF NOT (v_has_access) THEN
        -- Recursive access path via agent -> can_manage
        IF EXISTS (
    SELECT 1
    FROM melange_tuples AS link
    WHERE (link.object_type = 'state_volume' AND link.relation IN ('agent') AND link.object_id = p_object_id AND "public"."check_permission_internal"(p_subject_type, p_subject_id, 'can_manage', link.subject_type, link.subject_id, p_visited || ARRAY[v_key]) = 1 AND link.subject_type IN ('agent'))
    ) THEN
        v_has_access := TRUE;
    END IF;
    END IF;
    IF v_has_access THEN
        RETURN 1;
    END IF;
    RETURN 0;
END;
$$ LANGUAGE plpgsql STABLE COST 1000
SET search_path = 'public';


-- Generated check function for state_volume.can_restore
-- Features: Recursive
CREATE OR REPLACE FUNCTION "public"."check_state_volume_can_restore_nw"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[]
) RETURNS INTEGER AS $$
DECLARE
    v_has_access BOOLEAN := FALSE;
    v_key TEXT := 'state_volume:' || p_object_id || ':can_restore';
    v_userset_check INTEGER := 0;
BEGIN
    -- Cycle detection
    IF v_key = ANY(p_visited) THEN
        RETURN 0;
    END IF;
    IF array_length(p_visited, 1) >= 25 THEN
        RAISE EXCEPTION 'resolution too complex' USING ERRCODE = 'M2002';
    END IF;
    v_has_access := FALSE;
    -- Userset subject handling
    IF position('#' in p_subject_id) > 0 THEN
        -- Case 1: Self-referential userset check
        IF (p_subject_type = 'state_volume' AND substring(p_subject_id from 1 for position('#' in p_subject_id) - 1) = p_object_id) THEN
        SELECT INTO v_userset_check 1
    WHERE substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('can_restore')
    LIMIT 1;
        IF v_userset_check = 1 THEN
        RETURN 1;
    END IF;
    END IF;
    END IF;
    IF NOT (v_has_access) THEN
        -- Recursive access path via agent -> can_manage
        IF EXISTS (
    SELECT 1
    FROM melange_tuples AS link
    WHERE (link.object_type = 'state_volume' AND link.relation IN ('agent') AND link.object_id = p_object_id AND "public"."check_permission_internal"(p_subject_type, p_subject_id, 'can_manage', link.subject_type, link.subject_id, p_visited || ARRAY[v_key]) = 1 AND link.subject_type IN ('agent'))
    ) THEN
        v_has_access := TRUE;
    END IF;
    END IF;
    IF v_has_access THEN
        RETURN 1;
    END IF;
    RETURN 0;
END;
$$ LANGUAGE plpgsql STABLE COST 1000
SET search_path = 'public';


-- Generated check function for run.can_cancel
-- Features: Recursive
CREATE OR REPLACE FUNCTION "public"."check_run_can_cancel_nw"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[]
) RETURNS INTEGER AS $$
DECLARE
    v_has_access BOOLEAN := FALSE;
    v_key TEXT := 'run:' || p_object_id || ':can_cancel';
    v_userset_check INTEGER := 0;
BEGIN
    -- Cycle detection
    IF v_key = ANY(p_visited) THEN
        RETURN 0;
    END IF;
    IF array_length(p_visited, 1) >= 25 THEN
        RAISE EXCEPTION 'resolution too complex' USING ERRCODE = 'M2002';
    END IF;
    v_has_access := FALSE;
    -- Userset subject handling
    IF position('#' in p_subject_id) > 0 THEN
        -- Case 1: Self-referential userset check
        IF (p_subject_type = 'run' AND substring(p_subject_id from 1 for position('#' in p_subject_id) - 1) = p_object_id) THEN
        SELECT INTO v_userset_check 1
    WHERE substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('can_cancel')
    LIMIT 1;
        IF v_userset_check = 1 THEN
        RETURN 1;
    END IF;
    END IF;
    END IF;
    IF NOT (v_has_access) THEN
        -- Recursive access path via agent -> can_execute
        IF EXISTS (
    SELECT 1
    FROM melange_tuples AS link
    WHERE (link.object_type = 'run' AND link.relation IN ('agent') AND link.object_id = p_object_id AND "public"."check_permission_internal"(p_subject_type, p_subject_id, 'can_execute', link.subject_type, link.subject_id, p_visited || ARRAY[v_key]) = 1 AND link.subject_type IN ('agent'))
    ) THEN
        v_has_access := TRUE;
    END IF;
    END IF;
    IF NOT (v_has_access) THEN
        -- Recursive access path via agent -> can_manage
        IF EXISTS (
    SELECT 1
    FROM melange_tuples AS link
    WHERE (link.object_type = 'run' AND link.relation IN ('agent') AND link.object_id = p_object_id AND "public"."check_permission_internal"(p_subject_type, p_subject_id, 'can_manage', link.subject_type, link.subject_id, p_visited || ARRAY[v_key]) = 1 AND link.subject_type IN ('agent'))
    ) THEN
        v_has_access := TRUE;
    END IF;
    END IF;
    IF v_has_access THEN
        RETURN 1;
    END IF;
    RETURN 0;
END;
$$ LANGUAGE plpgsql STABLE COST 1000
SET search_path = 'public';


-- Generated check function for state_volume.can_attach
-- Features: Recursive
CREATE OR REPLACE FUNCTION "public"."check_state_volume_can_attach_nw"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[]
) RETURNS INTEGER AS $$
DECLARE
    v_has_access BOOLEAN := FALSE;
    v_key TEXT := 'state_volume:' || p_object_id || ':can_attach';
    v_userset_check INTEGER := 0;
BEGIN
    -- Cycle detection
    IF v_key = ANY(p_visited) THEN
        RETURN 0;
    END IF;
    IF array_length(p_visited, 1) >= 25 THEN
        RAISE EXCEPTION 'resolution too complex' USING ERRCODE = 'M2002';
    END IF;
    v_has_access := FALSE;
    -- Userset subject handling
    IF position('#' in p_subject_id) > 0 THEN
        -- Case 1: Self-referential userset check
        IF (p_subject_type = 'state_volume' AND substring(p_subject_id from 1 for position('#' in p_subject_id) - 1) = p_object_id) THEN
        SELECT INTO v_userset_check 1
    WHERE substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('can_attach')
    LIMIT 1;
        IF v_userset_check = 1 THEN
        RETURN 1;
    END IF;
    END IF;
    END IF;
    IF NOT (v_has_access) THEN
        -- Recursive access path via agent -> can_execute
        IF EXISTS (
    SELECT 1
    FROM melange_tuples AS link
    WHERE (link.object_type = 'state_volume' AND link.relation IN ('agent') AND link.object_id = p_object_id AND "public"."check_permission_internal"(p_subject_type, p_subject_id, 'can_execute', link.subject_type, link.subject_id, p_visited || ARRAY[v_key]) = 1 AND link.subject_type IN ('agent'))
    ) THEN
        v_has_access := TRUE;
    END IF;
    END IF;
    IF v_has_access THEN
        RETURN 1;
    END IF;
    RETURN 0;
END;
$$ LANGUAGE plpgsql STABLE COST 1000
SET search_path = 'public';


-- Generated check function for project.can_read
-- Features: Implied+Recursive
CREATE OR REPLACE FUNCTION "public"."check_project_can_read_nw"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[]
) RETURNS INTEGER AS $$
DECLARE
    v_has_access BOOLEAN := FALSE;
    v_key TEXT := 'project:' || p_object_id || ':can_read';
    v_userset_check INTEGER := 0;
BEGIN
    -- Cycle detection
    IF v_key = ANY(p_visited) THEN
        RETURN 0;
    END IF;
    IF array_length(p_visited, 1) >= 25 THEN
        RAISE EXCEPTION 'resolution too complex' USING ERRCODE = 'M2002';
    END IF;
    v_has_access := FALSE;
    -- Userset subject handling
    IF position('#' in p_subject_id) > 0 THEN
        -- Case 1: Self-referential userset check
        IF (p_subject_type = 'project' AND substring(p_subject_id from 1 for position('#' in p_subject_id) - 1) = p_object_id) THEN
        SELECT INTO v_userset_check 1
    WHERE substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('can_read', 'maintainer')
    LIMIT 1;
        IF v_userset_check = 1 THEN
        RETURN 1;
    END IF;
    END IF;
    END IF;
    -- Direct/Implied access path
    IF EXISTS (
    SELECT 1
    FROM melange_tuples
    WHERE (object_type = 'project' AND relation IN ('can_read', 'maintainer') AND object_id = p_object_id AND subject_type IN ('user') AND subject_type = p_subject_type AND (subject_id = p_subject_id AND NOT (subject_id = '*')))
    LIMIT 1
    ) THEN
        v_has_access := TRUE;
    END IF;
    IF NOT (v_has_access) THEN
        -- Recursive access path via organization -> can_read
        IF EXISTS (
    SELECT 1
    FROM melange_tuples AS link
    WHERE (link.object_type = 'project' AND link.relation IN ('organization') AND link.object_id = p_object_id AND "public"."check_permission_internal"(p_subject_type, p_subject_id, 'can_read', link.subject_type, link.subject_id, p_visited || ARRAY[v_key]) = 1 AND link.subject_type IN ('organization'))
    ) THEN
        v_has_access := TRUE;
    END IF;
    END IF;
    IF v_has_access THEN
        RETURN 1;
    END IF;
    RETURN 0;
END;
$$ LANGUAGE plpgsql STABLE COST 1000
SET search_path = 'public';


-- Generated check function for agent.can_read
-- Features: Recursive
CREATE OR REPLACE FUNCTION "public"."check_agent_can_read_nw"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[]
) RETURNS INTEGER AS $$
DECLARE
    v_has_access BOOLEAN := FALSE;
    v_key TEXT := 'agent:' || p_object_id || ':can_read';
    v_userset_check INTEGER := 0;
BEGIN
    -- Cycle detection
    IF v_key = ANY(p_visited) THEN
        RETURN 0;
    END IF;
    IF array_length(p_visited, 1) >= 25 THEN
        RAISE EXCEPTION 'resolution too complex' USING ERRCODE = 'M2002';
    END IF;
    v_has_access := FALSE;
    -- Userset subject handling
    IF position('#' in p_subject_id) > 0 THEN
        -- Case 1: Self-referential userset check
        IF (p_subject_type = 'agent' AND substring(p_subject_id from 1 for position('#' in p_subject_id) - 1) = p_object_id) THEN
        SELECT INTO v_userset_check 1
    WHERE substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('can_read')
    LIMIT 1;
        IF v_userset_check = 1 THEN
        RETURN 1;
    END IF;
    END IF;
    END IF;
    IF NOT (v_has_access) THEN
        -- Recursive access path via project -> can_read
        IF EXISTS (
    SELECT 1
    FROM melange_tuples AS link
    WHERE (link.object_type = 'agent' AND link.relation IN ('project') AND link.object_id = p_object_id AND "public"."check_permission_internal"(p_subject_type, p_subject_id, 'can_read', link.subject_type, link.subject_id, p_visited || ARRAY[v_key]) = 1 AND link.subject_type IN ('project'))
    ) THEN
        v_has_access := TRUE;
    END IF;
    END IF;
    IF v_has_access THEN
        RETURN 1;
    END IF;
    RETURN 0;
END;
$$ LANGUAGE plpgsql STABLE COST 1000
SET search_path = 'public';


-- Generated check function for repository.can_read
-- Features: Implied+Wildcard+Recursive
CREATE OR REPLACE FUNCTION "public"."check_repository_can_read_nw"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[]
) RETURNS INTEGER AS $$
DECLARE
    v_has_access BOOLEAN := FALSE;
    v_key TEXT := 'repository:' || p_object_id || ':can_read';
    v_userset_check INTEGER := 0;
BEGIN
    -- Cycle detection
    IF v_key = ANY(p_visited) THEN
        RETURN 0;
    END IF;
    IF array_length(p_visited, 1) >= 25 THEN
        RAISE EXCEPTION 'resolution too complex' USING ERRCODE = 'M2002';
    END IF;
    v_has_access := FALSE;
    -- Userset subject handling
    IF position('#' in p_subject_id) > 0 THEN
        -- Case 1: Self-referential userset check
        IF (p_subject_type = 'repository' AND substring(p_subject_id from 1 for position('#' in p_subject_id) - 1) = p_object_id) THEN
        SELECT INTO v_userset_check 1
    WHERE substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('can_read', 'public')
    LIMIT 1;
        IF v_userset_check = 1 THEN
        RETURN 1;
    END IF;
    END IF;
    END IF;
    -- Direct/Implied access path
    IF EXISTS (
    SELECT 1
    FROM melange_tuples
    WHERE (object_type = 'repository' AND relation IN ('can_read', 'public') AND object_id = p_object_id AND subject_type IN ('user') AND subject_type = p_subject_type AND (subject_id = p_subject_id AND NOT (subject_id = '*')))
    LIMIT 1
    ) THEN
        v_has_access := TRUE;
    END IF;
    IF NOT (v_has_access) THEN
        -- Recursive access path via project -> can_read
        IF EXISTS (
    SELECT 1
    FROM melange_tuples AS link
    WHERE (link.object_type = 'repository' AND link.relation IN ('project') AND link.object_id = p_object_id AND "public"."check_permission_internal"(p_subject_type, p_subject_id, 'can_read', link.subject_type, link.subject_id, p_visited || ARRAY[v_key]) = 1 AND link.subject_type IN ('project'))
    ) THEN
        v_has_access := TRUE;
    END IF;
    END IF;
    IF v_has_access THEN
        RETURN 1;
    END IF;
    RETURN 0;
END;
$$ LANGUAGE plpgsql STABLE COST 1000
SET search_path = 'public';


-- Generated check function for run.can_read
-- Features: Recursive
CREATE OR REPLACE FUNCTION "public"."check_run_can_read_nw"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[]
) RETURNS INTEGER AS $$
DECLARE
    v_has_access BOOLEAN := FALSE;
    v_key TEXT := 'run:' || p_object_id || ':can_read';
    v_userset_check INTEGER := 0;
BEGIN
    -- Cycle detection
    IF v_key = ANY(p_visited) THEN
        RETURN 0;
    END IF;
    IF array_length(p_visited, 1) >= 25 THEN
        RAISE EXCEPTION 'resolution too complex' USING ERRCODE = 'M2002';
    END IF;
    v_has_access := FALSE;
    -- Userset subject handling
    IF position('#' in p_subject_id) > 0 THEN
        -- Case 1: Self-referential userset check
        IF (p_subject_type = 'run' AND substring(p_subject_id from 1 for position('#' in p_subject_id) - 1) = p_object_id) THEN
        SELECT INTO v_userset_check 1
    WHERE substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('can_read')
    LIMIT 1;
        IF v_userset_check = 1 THEN
        RETURN 1;
    END IF;
    END IF;
    END IF;
    IF NOT (v_has_access) THEN
        -- Recursive access path via agent -> can_read
        IF EXISTS (
    SELECT 1
    FROM melange_tuples AS link
    WHERE (link.object_type = 'run' AND link.relation IN ('agent') AND link.object_id = p_object_id AND "public"."check_permission_internal"(p_subject_type, p_subject_id, 'can_read', link.subject_type, link.subject_id, p_visited || ARRAY[v_key]) = 1 AND link.subject_type IN ('agent'))
    ) THEN
        v_has_access := TRUE;
    END IF;
    END IF;
    IF v_has_access THEN
        RETURN 1;
    END IF;
    RETURN 0;
END;
$$ LANGUAGE plpgsql STABLE COST 1000
SET search_path = 'public';


-- Generated check function for state_volume.can_read
-- Features: Recursive
CREATE OR REPLACE FUNCTION "public"."check_state_volume_can_read_nw"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[]
) RETURNS INTEGER AS $$
DECLARE
    v_has_access BOOLEAN := FALSE;
    v_key TEXT := 'state_volume:' || p_object_id || ':can_read';
    v_userset_check INTEGER := 0;
BEGIN
    -- Cycle detection
    IF v_key = ANY(p_visited) THEN
        RETURN 0;
    END IF;
    IF array_length(p_visited, 1) >= 25 THEN
        RAISE EXCEPTION 'resolution too complex' USING ERRCODE = 'M2002';
    END IF;
    v_has_access := FALSE;
    -- Userset subject handling
    IF position('#' in p_subject_id) > 0 THEN
        -- Case 1: Self-referential userset check
        IF (p_subject_type = 'state_volume' AND substring(p_subject_id from 1 for position('#' in p_subject_id) - 1) = p_object_id) THEN
        SELECT INTO v_userset_check 1
    WHERE substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('can_read')
    LIMIT 1;
        IF v_userset_check = 1 THEN
        RETURN 1;
    END IF;
    END IF;
    END IF;
    IF NOT (v_has_access) THEN
        -- Recursive access path via agent -> can_read
        IF EXISTS (
    SELECT 1
    FROM melange_tuples AS link
    WHERE (link.object_type = 'state_volume' AND link.relation IN ('agent') AND link.object_id = p_object_id AND "public"."check_permission_internal"(p_subject_type, p_subject_id, 'can_read', link.subject_type, link.subject_id, p_visited || ARRAY[v_key]) = 1 AND link.subject_type IN ('agent'))
    ) THEN
        v_has_access := TRUE;
    END IF;
    END IF;
    IF v_has_access THEN
        RETURN 1;
    END IF;
    RETURN 0;
END;
$$ LANGUAGE plpgsql STABLE COST 1000
SET search_path = 'public';


-- ============================================================
-- Explain Functions (31 functions)
-- ============================================================

-- Generated explain function for agent.project
-- Features: Direct
CREATE OR REPLACE FUNCTION "public"."explain_agent_project"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[],
    p_max_nodes INTEGER DEFAULT NULL
) RETURNS JSONB AS $$
DECLARE
    v_key TEXT := 'agent:' || p_object_id || ':project';
    v_node_count INTEGER := 0;
    v_evidence_tuple RECORD;
    v_root JSONB;
    v_attempts JSONB := '[]'::JSONB;
    v_userset_check INTEGER := 0;
    v_max_nodes INTEGER := COALESCE(p_max_nodes, current_setting('melange.max_explain_nodes', true)::INTEGER, 100);
    v_truncated BOOLEAN := FALSE;
BEGIN
    -- Cycle detection
    IF v_key = ANY(p_visited) THEN
        v_root := jsonb_build_object('type', 'cycle', 'label', v_key);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('agent' || ':' || p_object_id),
        'relation', 'project',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
    IF array_length(p_visited, 1) >= 25 THEN
        RAISE EXCEPTION 'resolution too complex' USING ERRCODE = 'M2002';
    END IF;
    IF v_node_count >= v_max_nodes THEN
        v_root := jsonb_build_object('type', 'truncated');
        v_node_count := v_node_count + 1;
        v_truncated := TRUE;
        RETURN jsonb_build_object(
        'object', ('agent' || ':' || p_object_id),
        'relation', 'project',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
    -- Userset subject handling (subject is itself a userset reference)
    IF position('#' in p_subject_id) > 0 THEN
        -- Case 1: self-referential userset (subject's userset resolves to this object)
        IF (p_subject_type = 'agent' AND split_part(p_subject_id, '#', 1) = p_object_id) THEN
        SELECT INTO v_userset_check 1
    WHERE substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('project')
    LIMIT 1;
        IF v_userset_check = 1 THEN
        v_root := jsonb_build_object('type', 'userset', 'label', 'self-referential userset matches relation closure', 'result', true);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('agent' || ':' || p_object_id),
        'relation', 'project',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', true,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
    END IF;
    END IF;
    -- Direct/Implied grant attempt
    SELECT INTO v_evidence_tuple t.subject_type, t.subject_id, t.relation, t.object_type, t.object_id
    FROM melange_tuples AS t
    WHERE (t.object_type = 'agent' AND t.relation IN ('project') AND t.object_id = p_object_id AND t.subject_type = p_subject_type AND (t.subject_id = p_subject_id AND NOT (t.subject_id = '*')) AND t.subject_type IN ('project'))
    LIMIT 1;
    IF FOUND THEN
        v_root := (CASE WHEN v_evidence_tuple.subject_id = '*' THEN jsonb_build_object('type', 'wildcard', 'users', jsonb_build_array(jsonb_build_object('type', v_evidence_tuple.subject_type, 'id', '*')), 'result', true) ELSE jsonb_build_object('type', 'direct', 'label', 'direct grant', 'evidence', jsonb_build_array(jsonb_build_object('subject_type', v_evidence_tuple.subject_type, 'subject_id', v_evidence_tuple.subject_id, 'relation', v_evidence_tuple.relation, 'object_type', v_evidence_tuple.object_type, 'object_id', v_evidence_tuple.object_id)), 'result', true) END);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('agent' || ':' || p_object_id),
        'relation', 'project',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', true,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    ELSE
        v_node_count := v_node_count + 1;
        v_attempts := v_attempts || jsonb_build_array(jsonb_build_object('type', 'direct', 'label', 'no direct grant', 'result', false));
    END IF;
    IF v_node_count >= v_max_nodes THEN
        v_truncated := TRUE;
    END IF;
    -- All recorded attempts failed
    v_root := jsonb_build_object('type', 'union', 'children', v_attempts, 'result', false);
    v_node_count := v_node_count + 1;
    RETURN jsonb_build_object(
        'object', ('agent' || ':' || p_object_id),
        'relation', 'project',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';


-- Generated explain function for organization.owner
-- Features: Direct
CREATE OR REPLACE FUNCTION "public"."explain_organization_owner"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[],
    p_max_nodes INTEGER DEFAULT NULL
) RETURNS JSONB AS $$
DECLARE
    v_key TEXT := 'organization:' || p_object_id || ':owner';
    v_node_count INTEGER := 0;
    v_evidence_tuple RECORD;
    v_root JSONB;
    v_attempts JSONB := '[]'::JSONB;
    v_userset_check INTEGER := 0;
    v_max_nodes INTEGER := COALESCE(p_max_nodes, current_setting('melange.max_explain_nodes', true)::INTEGER, 100);
    v_truncated BOOLEAN := FALSE;
BEGIN
    -- Cycle detection
    IF v_key = ANY(p_visited) THEN
        v_root := jsonb_build_object('type', 'cycle', 'label', v_key);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('organization' || ':' || p_object_id),
        'relation', 'owner',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
    IF array_length(p_visited, 1) >= 25 THEN
        RAISE EXCEPTION 'resolution too complex' USING ERRCODE = 'M2002';
    END IF;
    IF v_node_count >= v_max_nodes THEN
        v_root := jsonb_build_object('type', 'truncated');
        v_node_count := v_node_count + 1;
        v_truncated := TRUE;
        RETURN jsonb_build_object(
        'object', ('organization' || ':' || p_object_id),
        'relation', 'owner',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
    -- Userset subject handling (subject is itself a userset reference)
    IF position('#' in p_subject_id) > 0 THEN
        -- Case 1: self-referential userset (subject's userset resolves to this object)
        IF (p_subject_type = 'organization' AND split_part(p_subject_id, '#', 1) = p_object_id) THEN
        SELECT INTO v_userset_check 1
    WHERE substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('owner')
    LIMIT 1;
        IF v_userset_check = 1 THEN
        v_root := jsonb_build_object('type', 'userset', 'label', 'self-referential userset matches relation closure', 'result', true);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('organization' || ':' || p_object_id),
        'relation', 'owner',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', true,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
    END IF;
    END IF;
    -- Direct/Implied grant attempt
    SELECT INTO v_evidence_tuple t.subject_type, t.subject_id, t.relation, t.object_type, t.object_id
    FROM melange_tuples AS t
    WHERE (t.object_type = 'organization' AND t.relation IN ('owner') AND t.object_id = p_object_id AND t.subject_type = p_subject_type AND (t.subject_id = p_subject_id AND NOT (t.subject_id = '*')) AND t.subject_type IN ('user'))
    LIMIT 1;
    IF FOUND THEN
        v_root := (CASE WHEN v_evidence_tuple.subject_id = '*' THEN jsonb_build_object('type', 'wildcard', 'users', jsonb_build_array(jsonb_build_object('type', v_evidence_tuple.subject_type, 'id', '*')), 'result', true) ELSE jsonb_build_object('type', 'direct', 'label', 'direct grant', 'evidence', jsonb_build_array(jsonb_build_object('subject_type', v_evidence_tuple.subject_type, 'subject_id', v_evidence_tuple.subject_id, 'relation', v_evidence_tuple.relation, 'object_type', v_evidence_tuple.object_type, 'object_id', v_evidence_tuple.object_id)), 'result', true) END);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('organization' || ':' || p_object_id),
        'relation', 'owner',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', true,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    ELSE
        v_node_count := v_node_count + 1;
        v_attempts := v_attempts || jsonb_build_array(jsonb_build_object('type', 'direct', 'label', 'no direct grant', 'result', false));
    END IF;
    IF v_node_count >= v_max_nodes THEN
        v_truncated := TRUE;
    END IF;
    -- All recorded attempts failed
    v_root := jsonb_build_object('type', 'union', 'children', v_attempts, 'result', false);
    v_node_count := v_node_count + 1;
    RETURN jsonb_build_object(
        'object', ('organization' || ':' || p_object_id),
        'relation', 'owner',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';


-- Generated explain function for project.maintainer
-- Features: Direct
CREATE OR REPLACE FUNCTION "public"."explain_project_maintainer"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[],
    p_max_nodes INTEGER DEFAULT NULL
) RETURNS JSONB AS $$
DECLARE
    v_key TEXT := 'project:' || p_object_id || ':maintainer';
    v_node_count INTEGER := 0;
    v_evidence_tuple RECORD;
    v_root JSONB;
    v_attempts JSONB := '[]'::JSONB;
    v_userset_check INTEGER := 0;
    v_max_nodes INTEGER := COALESCE(p_max_nodes, current_setting('melange.max_explain_nodes', true)::INTEGER, 100);
    v_truncated BOOLEAN := FALSE;
BEGIN
    -- Cycle detection
    IF v_key = ANY(p_visited) THEN
        v_root := jsonb_build_object('type', 'cycle', 'label', v_key);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('project' || ':' || p_object_id),
        'relation', 'maintainer',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
    IF array_length(p_visited, 1) >= 25 THEN
        RAISE EXCEPTION 'resolution too complex' USING ERRCODE = 'M2002';
    END IF;
    IF v_node_count >= v_max_nodes THEN
        v_root := jsonb_build_object('type', 'truncated');
        v_node_count := v_node_count + 1;
        v_truncated := TRUE;
        RETURN jsonb_build_object(
        'object', ('project' || ':' || p_object_id),
        'relation', 'maintainer',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
    -- Userset subject handling (subject is itself a userset reference)
    IF position('#' in p_subject_id) > 0 THEN
        -- Case 1: self-referential userset (subject's userset resolves to this object)
        IF (p_subject_type = 'project' AND split_part(p_subject_id, '#', 1) = p_object_id) THEN
        SELECT INTO v_userset_check 1
    WHERE substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('maintainer')
    LIMIT 1;
        IF v_userset_check = 1 THEN
        v_root := jsonb_build_object('type', 'userset', 'label', 'self-referential userset matches relation closure', 'result', true);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('project' || ':' || p_object_id),
        'relation', 'maintainer',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', true,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
    END IF;
    END IF;
    -- Direct/Implied grant attempt
    SELECT INTO v_evidence_tuple t.subject_type, t.subject_id, t.relation, t.object_type, t.object_id
    FROM melange_tuples AS t
    WHERE (t.object_type = 'project' AND t.relation IN ('maintainer') AND t.object_id = p_object_id AND t.subject_type = p_subject_type AND (t.subject_id = p_subject_id AND NOT (t.subject_id = '*')) AND t.subject_type IN ('user'))
    LIMIT 1;
    IF FOUND THEN
        v_root := (CASE WHEN v_evidence_tuple.subject_id = '*' THEN jsonb_build_object('type', 'wildcard', 'users', jsonb_build_array(jsonb_build_object('type', v_evidence_tuple.subject_type, 'id', '*')), 'result', true) ELSE jsonb_build_object('type', 'direct', 'label', 'direct grant', 'evidence', jsonb_build_array(jsonb_build_object('subject_type', v_evidence_tuple.subject_type, 'subject_id', v_evidence_tuple.subject_id, 'relation', v_evidence_tuple.relation, 'object_type', v_evidence_tuple.object_type, 'object_id', v_evidence_tuple.object_id)), 'result', true) END);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('project' || ':' || p_object_id),
        'relation', 'maintainer',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', true,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    ELSE
        v_node_count := v_node_count + 1;
        v_attempts := v_attempts || jsonb_build_array(jsonb_build_object('type', 'direct', 'label', 'no direct grant', 'result', false));
    END IF;
    IF v_node_count >= v_max_nodes THEN
        v_truncated := TRUE;
    END IF;
    -- All recorded attempts failed
    v_root := jsonb_build_object('type', 'union', 'children', v_attempts, 'result', false);
    v_node_count := v_node_count + 1;
    RETURN jsonb_build_object(
        'object', ('project' || ':' || p_object_id),
        'relation', 'maintainer',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';


-- Generated explain function for project.organization
-- Features: Direct
CREATE OR REPLACE FUNCTION "public"."explain_project_organization"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[],
    p_max_nodes INTEGER DEFAULT NULL
) RETURNS JSONB AS $$
DECLARE
    v_key TEXT := 'project:' || p_object_id || ':organization';
    v_node_count INTEGER := 0;
    v_evidence_tuple RECORD;
    v_root JSONB;
    v_attempts JSONB := '[]'::JSONB;
    v_userset_check INTEGER := 0;
    v_max_nodes INTEGER := COALESCE(p_max_nodes, current_setting('melange.max_explain_nodes', true)::INTEGER, 100);
    v_truncated BOOLEAN := FALSE;
BEGIN
    -- Cycle detection
    IF v_key = ANY(p_visited) THEN
        v_root := jsonb_build_object('type', 'cycle', 'label', v_key);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('project' || ':' || p_object_id),
        'relation', 'organization',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
    IF array_length(p_visited, 1) >= 25 THEN
        RAISE EXCEPTION 'resolution too complex' USING ERRCODE = 'M2002';
    END IF;
    IF v_node_count >= v_max_nodes THEN
        v_root := jsonb_build_object('type', 'truncated');
        v_node_count := v_node_count + 1;
        v_truncated := TRUE;
        RETURN jsonb_build_object(
        'object', ('project' || ':' || p_object_id),
        'relation', 'organization',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
    -- Userset subject handling (subject is itself a userset reference)
    IF position('#' in p_subject_id) > 0 THEN
        -- Case 1: self-referential userset (subject's userset resolves to this object)
        IF (p_subject_type = 'project' AND split_part(p_subject_id, '#', 1) = p_object_id) THEN
        SELECT INTO v_userset_check 1
    WHERE substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('organization')
    LIMIT 1;
        IF v_userset_check = 1 THEN
        v_root := jsonb_build_object('type', 'userset', 'label', 'self-referential userset matches relation closure', 'result', true);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('project' || ':' || p_object_id),
        'relation', 'organization',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', true,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
    END IF;
    END IF;
    -- Direct/Implied grant attempt
    SELECT INTO v_evidence_tuple t.subject_type, t.subject_id, t.relation, t.object_type, t.object_id
    FROM melange_tuples AS t
    WHERE (t.object_type = 'project' AND t.relation IN ('organization') AND t.object_id = p_object_id AND t.subject_type = p_subject_type AND (t.subject_id = p_subject_id AND NOT (t.subject_id = '*')) AND t.subject_type IN ('organization'))
    LIMIT 1;
    IF FOUND THEN
        v_root := (CASE WHEN v_evidence_tuple.subject_id = '*' THEN jsonb_build_object('type', 'wildcard', 'users', jsonb_build_array(jsonb_build_object('type', v_evidence_tuple.subject_type, 'id', '*')), 'result', true) ELSE jsonb_build_object('type', 'direct', 'label', 'direct grant', 'evidence', jsonb_build_array(jsonb_build_object('subject_type', v_evidence_tuple.subject_type, 'subject_id', v_evidence_tuple.subject_id, 'relation', v_evidence_tuple.relation, 'object_type', v_evidence_tuple.object_type, 'object_id', v_evidence_tuple.object_id)), 'result', true) END);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('project' || ':' || p_object_id),
        'relation', 'organization',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', true,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    ELSE
        v_node_count := v_node_count + 1;
        v_attempts := v_attempts || jsonb_build_array(jsonb_build_object('type', 'direct', 'label', 'no direct grant', 'result', false));
    END IF;
    IF v_node_count >= v_max_nodes THEN
        v_truncated := TRUE;
    END IF;
    -- All recorded attempts failed
    v_root := jsonb_build_object('type', 'union', 'children', v_attempts, 'result', false);
    v_node_count := v_node_count + 1;
    RETURN jsonb_build_object(
        'object', ('project' || ':' || p_object_id),
        'relation', 'organization',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';


-- Generated explain function for repository.project
-- Features: Direct
CREATE OR REPLACE FUNCTION "public"."explain_repository_project"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[],
    p_max_nodes INTEGER DEFAULT NULL
) RETURNS JSONB AS $$
DECLARE
    v_key TEXT := 'repository:' || p_object_id || ':project';
    v_node_count INTEGER := 0;
    v_evidence_tuple RECORD;
    v_root JSONB;
    v_attempts JSONB := '[]'::JSONB;
    v_userset_check INTEGER := 0;
    v_max_nodes INTEGER := COALESCE(p_max_nodes, current_setting('melange.max_explain_nodes', true)::INTEGER, 100);
    v_truncated BOOLEAN := FALSE;
BEGIN
    -- Cycle detection
    IF v_key = ANY(p_visited) THEN
        v_root := jsonb_build_object('type', 'cycle', 'label', v_key);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('repository' || ':' || p_object_id),
        'relation', 'project',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
    IF array_length(p_visited, 1) >= 25 THEN
        RAISE EXCEPTION 'resolution too complex' USING ERRCODE = 'M2002';
    END IF;
    IF v_node_count >= v_max_nodes THEN
        v_root := jsonb_build_object('type', 'truncated');
        v_node_count := v_node_count + 1;
        v_truncated := TRUE;
        RETURN jsonb_build_object(
        'object', ('repository' || ':' || p_object_id),
        'relation', 'project',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
    -- Userset subject handling (subject is itself a userset reference)
    IF position('#' in p_subject_id) > 0 THEN
        -- Case 1: self-referential userset (subject's userset resolves to this object)
        IF (p_subject_type = 'repository' AND split_part(p_subject_id, '#', 1) = p_object_id) THEN
        SELECT INTO v_userset_check 1
    WHERE substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('project')
    LIMIT 1;
        IF v_userset_check = 1 THEN
        v_root := jsonb_build_object('type', 'userset', 'label', 'self-referential userset matches relation closure', 'result', true);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('repository' || ':' || p_object_id),
        'relation', 'project',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', true,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
    END IF;
    END IF;
    -- Direct/Implied grant attempt
    SELECT INTO v_evidence_tuple t.subject_type, t.subject_id, t.relation, t.object_type, t.object_id
    FROM melange_tuples AS t
    WHERE (t.object_type = 'repository' AND t.relation IN ('project') AND t.object_id = p_object_id AND t.subject_type = p_subject_type AND (t.subject_id = p_subject_id AND NOT (t.subject_id = '*')) AND t.subject_type IN ('project'))
    LIMIT 1;
    IF FOUND THEN
        v_root := (CASE WHEN v_evidence_tuple.subject_id = '*' THEN jsonb_build_object('type', 'wildcard', 'users', jsonb_build_array(jsonb_build_object('type', v_evidence_tuple.subject_type, 'id', '*')), 'result', true) ELSE jsonb_build_object('type', 'direct', 'label', 'direct grant', 'evidence', jsonb_build_array(jsonb_build_object('subject_type', v_evidence_tuple.subject_type, 'subject_id', v_evidence_tuple.subject_id, 'relation', v_evidence_tuple.relation, 'object_type', v_evidence_tuple.object_type, 'object_id', v_evidence_tuple.object_id)), 'result', true) END);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('repository' || ':' || p_object_id),
        'relation', 'project',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', true,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    ELSE
        v_node_count := v_node_count + 1;
        v_attempts := v_attempts || jsonb_build_array(jsonb_build_object('type', 'direct', 'label', 'no direct grant', 'result', false));
    END IF;
    IF v_node_count >= v_max_nodes THEN
        v_truncated := TRUE;
    END IF;
    -- All recorded attempts failed
    v_root := jsonb_build_object('type', 'union', 'children', v_attempts, 'result', false);
    v_node_count := v_node_count + 1;
    RETURN jsonb_build_object(
        'object', ('repository' || ':' || p_object_id),
        'relation', 'project',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';


-- Generated explain function for repository.public
-- Features: Direct+Wildcard
CREATE OR REPLACE FUNCTION "public"."explain_repository_public"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[],
    p_max_nodes INTEGER DEFAULT NULL
) RETURNS JSONB AS $$
DECLARE
    v_key TEXT := 'repository:' || p_object_id || ':public';
    v_node_count INTEGER := 0;
    v_evidence_tuple RECORD;
    v_root JSONB;
    v_attempts JSONB := '[]'::JSONB;
    v_userset_check INTEGER := 0;
    v_max_nodes INTEGER := COALESCE(p_max_nodes, current_setting('melange.max_explain_nodes', true)::INTEGER, 100);
    v_truncated BOOLEAN := FALSE;
BEGIN
    -- Cycle detection
    IF v_key = ANY(p_visited) THEN
        v_root := jsonb_build_object('type', 'cycle', 'label', v_key);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('repository' || ':' || p_object_id),
        'relation', 'public',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
    IF array_length(p_visited, 1) >= 25 THEN
        RAISE EXCEPTION 'resolution too complex' USING ERRCODE = 'M2002';
    END IF;
    IF v_node_count >= v_max_nodes THEN
        v_root := jsonb_build_object('type', 'truncated');
        v_node_count := v_node_count + 1;
        v_truncated := TRUE;
        RETURN jsonb_build_object(
        'object', ('repository' || ':' || p_object_id),
        'relation', 'public',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
    -- Userset subject handling (subject is itself a userset reference)
    IF position('#' in p_subject_id) > 0 THEN
        -- Case 1: self-referential userset (subject's userset resolves to this object)
        IF (p_subject_type = 'repository' AND split_part(p_subject_id, '#', 1) = p_object_id) THEN
        SELECT INTO v_userset_check 1
    WHERE substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('public')
    LIMIT 1;
        IF v_userset_check = 1 THEN
        v_root := jsonb_build_object('type', 'userset', 'label', 'self-referential userset matches relation closure', 'result', true);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('repository' || ':' || p_object_id),
        'relation', 'public',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', true,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
    END IF;
    END IF;
    -- Direct/Implied grant attempt
    SELECT INTO v_evidence_tuple t.subject_type, t.subject_id, t.relation, t.object_type, t.object_id
    FROM melange_tuples AS t
    WHERE (t.object_type = 'repository' AND t.relation IN ('public') AND t.object_id = p_object_id AND t.subject_type = p_subject_type AND (t.subject_id = p_subject_id OR t.subject_id = '*') AND t.subject_type IN ('user'))
    LIMIT 1;
    IF FOUND THEN
        v_root := (CASE WHEN v_evidence_tuple.subject_id = '*' THEN jsonb_build_object('type', 'wildcard', 'users', jsonb_build_array(jsonb_build_object('type', v_evidence_tuple.subject_type, 'id', '*')), 'result', true) ELSE jsonb_build_object('type', 'direct', 'label', 'direct grant', 'evidence', jsonb_build_array(jsonb_build_object('subject_type', v_evidence_tuple.subject_type, 'subject_id', v_evidence_tuple.subject_id, 'relation', v_evidence_tuple.relation, 'object_type', v_evidence_tuple.object_type, 'object_id', v_evidence_tuple.object_id)), 'result', true) END);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('repository' || ':' || p_object_id),
        'relation', 'public',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', true,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    ELSE
        v_node_count := v_node_count + 1;
        v_attempts := v_attempts || jsonb_build_array(jsonb_build_object('type', 'direct', 'label', 'no direct grant', 'result', false));
    END IF;
    IF v_node_count >= v_max_nodes THEN
        v_truncated := TRUE;
    END IF;
    -- All recorded attempts failed
    v_root := jsonb_build_object('type', 'union', 'children', v_attempts, 'result', false);
    v_node_count := v_node_count + 1;
    RETURN jsonb_build_object(
        'object', ('repository' || ':' || p_object_id),
        'relation', 'public',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';


-- Generated explain function for run.agent
-- Features: Direct
CREATE OR REPLACE FUNCTION "public"."explain_run_agent"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[],
    p_max_nodes INTEGER DEFAULT NULL
) RETURNS JSONB AS $$
DECLARE
    v_key TEXT := 'run:' || p_object_id || ':agent';
    v_node_count INTEGER := 0;
    v_evidence_tuple RECORD;
    v_root JSONB;
    v_attempts JSONB := '[]'::JSONB;
    v_userset_check INTEGER := 0;
    v_max_nodes INTEGER := COALESCE(p_max_nodes, current_setting('melange.max_explain_nodes', true)::INTEGER, 100);
    v_truncated BOOLEAN := FALSE;
BEGIN
    -- Cycle detection
    IF v_key = ANY(p_visited) THEN
        v_root := jsonb_build_object('type', 'cycle', 'label', v_key);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('run' || ':' || p_object_id),
        'relation', 'agent',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
    IF array_length(p_visited, 1) >= 25 THEN
        RAISE EXCEPTION 'resolution too complex' USING ERRCODE = 'M2002';
    END IF;
    IF v_node_count >= v_max_nodes THEN
        v_root := jsonb_build_object('type', 'truncated');
        v_node_count := v_node_count + 1;
        v_truncated := TRUE;
        RETURN jsonb_build_object(
        'object', ('run' || ':' || p_object_id),
        'relation', 'agent',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
    -- Userset subject handling (subject is itself a userset reference)
    IF position('#' in p_subject_id) > 0 THEN
        -- Case 1: self-referential userset (subject's userset resolves to this object)
        IF (p_subject_type = 'run' AND split_part(p_subject_id, '#', 1) = p_object_id) THEN
        SELECT INTO v_userset_check 1
    WHERE substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('agent')
    LIMIT 1;
        IF v_userset_check = 1 THEN
        v_root := jsonb_build_object('type', 'userset', 'label', 'self-referential userset matches relation closure', 'result', true);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('run' || ':' || p_object_id),
        'relation', 'agent',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', true,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
    END IF;
    END IF;
    -- Direct/Implied grant attempt
    SELECT INTO v_evidence_tuple t.subject_type, t.subject_id, t.relation, t.object_type, t.object_id
    FROM melange_tuples AS t
    WHERE (t.object_type = 'run' AND t.relation IN ('agent') AND t.object_id = p_object_id AND t.subject_type = p_subject_type AND (t.subject_id = p_subject_id AND NOT (t.subject_id = '*')) AND t.subject_type IN ('agent'))
    LIMIT 1;
    IF FOUND THEN
        v_root := (CASE WHEN v_evidence_tuple.subject_id = '*' THEN jsonb_build_object('type', 'wildcard', 'users', jsonb_build_array(jsonb_build_object('type', v_evidence_tuple.subject_type, 'id', '*')), 'result', true) ELSE jsonb_build_object('type', 'direct', 'label', 'direct grant', 'evidence', jsonb_build_array(jsonb_build_object('subject_type', v_evidence_tuple.subject_type, 'subject_id', v_evidence_tuple.subject_id, 'relation', v_evidence_tuple.relation, 'object_type', v_evidence_tuple.object_type, 'object_id', v_evidence_tuple.object_id)), 'result', true) END);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('run' || ':' || p_object_id),
        'relation', 'agent',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', true,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    ELSE
        v_node_count := v_node_count + 1;
        v_attempts := v_attempts || jsonb_build_array(jsonb_build_object('type', 'direct', 'label', 'no direct grant', 'result', false));
    END IF;
    IF v_node_count >= v_max_nodes THEN
        v_truncated := TRUE;
    END IF;
    -- All recorded attempts failed
    v_root := jsonb_build_object('type', 'union', 'children', v_attempts, 'result', false);
    v_node_count := v_node_count + 1;
    RETURN jsonb_build_object(
        'object', ('run' || ':' || p_object_id),
        'relation', 'agent',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';


-- Generated explain function for state_volume.agent
-- Features: Direct
CREATE OR REPLACE FUNCTION "public"."explain_state_volume_agent"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[],
    p_max_nodes INTEGER DEFAULT NULL
) RETURNS JSONB AS $$
DECLARE
    v_key TEXT := 'state_volume:' || p_object_id || ':agent';
    v_node_count INTEGER := 0;
    v_evidence_tuple RECORD;
    v_root JSONB;
    v_attempts JSONB := '[]'::JSONB;
    v_userset_check INTEGER := 0;
    v_max_nodes INTEGER := COALESCE(p_max_nodes, current_setting('melange.max_explain_nodes', true)::INTEGER, 100);
    v_truncated BOOLEAN := FALSE;
BEGIN
    -- Cycle detection
    IF v_key = ANY(p_visited) THEN
        v_root := jsonb_build_object('type', 'cycle', 'label', v_key);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('state_volume' || ':' || p_object_id),
        'relation', 'agent',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
    IF array_length(p_visited, 1) >= 25 THEN
        RAISE EXCEPTION 'resolution too complex' USING ERRCODE = 'M2002';
    END IF;
    IF v_node_count >= v_max_nodes THEN
        v_root := jsonb_build_object('type', 'truncated');
        v_node_count := v_node_count + 1;
        v_truncated := TRUE;
        RETURN jsonb_build_object(
        'object', ('state_volume' || ':' || p_object_id),
        'relation', 'agent',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
    -- Userset subject handling (subject is itself a userset reference)
    IF position('#' in p_subject_id) > 0 THEN
        -- Case 1: self-referential userset (subject's userset resolves to this object)
        IF (p_subject_type = 'state_volume' AND split_part(p_subject_id, '#', 1) = p_object_id) THEN
        SELECT INTO v_userset_check 1
    WHERE substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('agent')
    LIMIT 1;
        IF v_userset_check = 1 THEN
        v_root := jsonb_build_object('type', 'userset', 'label', 'self-referential userset matches relation closure', 'result', true);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('state_volume' || ':' || p_object_id),
        'relation', 'agent',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', true,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
    END IF;
    END IF;
    -- Direct/Implied grant attempt
    SELECT INTO v_evidence_tuple t.subject_type, t.subject_id, t.relation, t.object_type, t.object_id
    FROM melange_tuples AS t
    WHERE (t.object_type = 'state_volume' AND t.relation IN ('agent') AND t.object_id = p_object_id AND t.subject_type = p_subject_type AND (t.subject_id = p_subject_id AND NOT (t.subject_id = '*')) AND t.subject_type IN ('agent'))
    LIMIT 1;
    IF FOUND THEN
        v_root := (CASE WHEN v_evidence_tuple.subject_id = '*' THEN jsonb_build_object('type', 'wildcard', 'users', jsonb_build_array(jsonb_build_object('type', v_evidence_tuple.subject_type, 'id', '*')), 'result', true) ELSE jsonb_build_object('type', 'direct', 'label', 'direct grant', 'evidence', jsonb_build_array(jsonb_build_object('subject_type', v_evidence_tuple.subject_type, 'subject_id', v_evidence_tuple.subject_id, 'relation', v_evidence_tuple.relation, 'object_type', v_evidence_tuple.object_type, 'object_id', v_evidence_tuple.object_id)), 'result', true) END);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('state_volume' || ':' || p_object_id),
        'relation', 'agent',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', true,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    ELSE
        v_node_count := v_node_count + 1;
        v_attempts := v_attempts || jsonb_build_array(jsonb_build_object('type', 'direct', 'label', 'no direct grant', 'result', false));
    END IF;
    IF v_node_count >= v_max_nodes THEN
        v_truncated := TRUE;
    END IF;
    -- All recorded attempts failed
    v_root := jsonb_build_object('type', 'union', 'children', v_attempts, 'result', false);
    v_node_count := v_node_count + 1;
    RETURN jsonb_build_object(
        'object', ('state_volume' || ':' || p_object_id),
        'relation', 'agent',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';


-- Generated explain function for organization.admin
-- Features: Direct+Implied
CREATE OR REPLACE FUNCTION "public"."explain_organization_admin"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[],
    p_max_nodes INTEGER DEFAULT NULL
) RETURNS JSONB AS $$
DECLARE
    v_key TEXT := 'organization:' || p_object_id || ':admin';
    v_node_count INTEGER := 0;
    v_evidence_tuple RECORD;
    v_root JSONB;
    v_attempts JSONB := '[]'::JSONB;
    v_userset_check INTEGER := 0;
    v_max_nodes INTEGER := COALESCE(p_max_nodes, current_setting('melange.max_explain_nodes', true)::INTEGER, 100);
    v_truncated BOOLEAN := FALSE;
BEGIN
    -- Cycle detection
    IF v_key = ANY(p_visited) THEN
        v_root := jsonb_build_object('type', 'cycle', 'label', v_key);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('organization' || ':' || p_object_id),
        'relation', 'admin',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
    IF array_length(p_visited, 1) >= 25 THEN
        RAISE EXCEPTION 'resolution too complex' USING ERRCODE = 'M2002';
    END IF;
    IF v_node_count >= v_max_nodes THEN
        v_root := jsonb_build_object('type', 'truncated');
        v_node_count := v_node_count + 1;
        v_truncated := TRUE;
        RETURN jsonb_build_object(
        'object', ('organization' || ':' || p_object_id),
        'relation', 'admin',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
    -- Userset subject handling (subject is itself a userset reference)
    IF position('#' in p_subject_id) > 0 THEN
        -- Case 1: self-referential userset (subject's userset resolves to this object)
        IF (p_subject_type = 'organization' AND split_part(p_subject_id, '#', 1) = p_object_id) THEN
        SELECT INTO v_userset_check 1
    WHERE substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('admin', 'owner')
    LIMIT 1;
        IF v_userset_check = 1 THEN
        v_root := jsonb_build_object('type', 'userset', 'label', 'self-referential userset matches relation closure', 'result', true);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('organization' || ':' || p_object_id),
        'relation', 'admin',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', true,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
    END IF;
    END IF;
    -- Direct/Implied grant attempt
    SELECT INTO v_evidence_tuple t.subject_type, t.subject_id, t.relation, t.object_type, t.object_id
    FROM melange_tuples AS t
    WHERE (t.object_type = 'organization' AND t.relation IN ('admin', 'owner') AND t.object_id = p_object_id AND t.subject_type = p_subject_type AND (t.subject_id = p_subject_id AND NOT (t.subject_id = '*')) AND t.subject_type IN ('user'))
    LIMIT 1;
    IF FOUND THEN
        v_root := (CASE WHEN v_evidence_tuple.subject_id = '*' THEN jsonb_build_object('type', 'wildcard', 'users', jsonb_build_array(jsonb_build_object('type', v_evidence_tuple.subject_type, 'id', '*')), 'result', true) ELSE jsonb_build_object('type', 'direct', 'label', ('direct or implied grant via ' || v_evidence_tuple.relation), 'evidence', jsonb_build_array(jsonb_build_object('subject_type', v_evidence_tuple.subject_type, 'subject_id', v_evidence_tuple.subject_id, 'relation', v_evidence_tuple.relation, 'object_type', v_evidence_tuple.object_type, 'object_id', v_evidence_tuple.object_id)), 'result', true) END);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('organization' || ':' || p_object_id),
        'relation', 'admin',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', true,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    ELSE
        v_node_count := v_node_count + 1;
        v_attempts := v_attempts || jsonb_build_array(jsonb_build_object('type', 'direct', 'label', 'no direct grant', 'result', false));
    END IF;
    IF v_node_count >= v_max_nodes THEN
        v_truncated := TRUE;
    END IF;
    -- All recorded attempts failed
    v_root := jsonb_build_object('type', 'union', 'children', v_attempts, 'result', false);
    v_node_count := v_node_count + 1;
    RETURN jsonb_build_object(
        'object', ('organization' || ':' || p_object_id),
        'relation', 'admin',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';


-- Generated explain function for organization.can_delete
-- Features: Implied
CREATE OR REPLACE FUNCTION "public"."explain_organization_can_delete"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[],
    p_max_nodes INTEGER DEFAULT NULL
) RETURNS JSONB AS $$
DECLARE
    v_key TEXT := 'organization:' || p_object_id || ':can_delete';
    v_node_count INTEGER := 0;
    v_evidence_tuple RECORD;
    v_root JSONB;
    v_attempts JSONB := '[]'::JSONB;
    v_userset_check INTEGER := 0;
    v_max_nodes INTEGER := COALESCE(p_max_nodes, current_setting('melange.max_explain_nodes', true)::INTEGER, 100);
    v_truncated BOOLEAN := FALSE;
BEGIN
    -- Cycle detection
    IF v_key = ANY(p_visited) THEN
        v_root := jsonb_build_object('type', 'cycle', 'label', v_key);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('organization' || ':' || p_object_id),
        'relation', 'can_delete',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
    IF array_length(p_visited, 1) >= 25 THEN
        RAISE EXCEPTION 'resolution too complex' USING ERRCODE = 'M2002';
    END IF;
    IF v_node_count >= v_max_nodes THEN
        v_root := jsonb_build_object('type', 'truncated');
        v_node_count := v_node_count + 1;
        v_truncated := TRUE;
        RETURN jsonb_build_object(
        'object', ('organization' || ':' || p_object_id),
        'relation', 'can_delete',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
    -- Userset subject handling (subject is itself a userset reference)
    IF position('#' in p_subject_id) > 0 THEN
        -- Case 1: self-referential userset (subject's userset resolves to this object)
        IF (p_subject_type = 'organization' AND split_part(p_subject_id, '#', 1) = p_object_id) THEN
        SELECT INTO v_userset_check 1
    WHERE substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('can_delete', 'owner')
    LIMIT 1;
        IF v_userset_check = 1 THEN
        v_root := jsonb_build_object('type', 'userset', 'label', 'self-referential userset matches relation closure', 'result', true);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('organization' || ':' || p_object_id),
        'relation', 'can_delete',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', true,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
    END IF;
    END IF;
    -- Direct/Implied grant attempt
    SELECT INTO v_evidence_tuple t.subject_type, t.subject_id, t.relation, t.object_type, t.object_id
    FROM melange_tuples AS t
    WHERE (t.object_type = 'organization' AND t.relation IN ('can_delete', 'owner') AND t.object_id = p_object_id AND t.subject_type = p_subject_type AND (t.subject_id = p_subject_id AND NOT (t.subject_id = '*')) AND t.subject_type IN ('user'))
    LIMIT 1;
    IF FOUND THEN
        v_root := (CASE WHEN v_evidence_tuple.subject_id = '*' THEN jsonb_build_object('type', 'wildcard', 'users', jsonb_build_array(jsonb_build_object('type', v_evidence_tuple.subject_type, 'id', '*')), 'result', true) ELSE jsonb_build_object('type', 'direct', 'label', ('direct or implied grant via ' || v_evidence_tuple.relation), 'evidence', jsonb_build_array(jsonb_build_object('subject_type', v_evidence_tuple.subject_type, 'subject_id', v_evidence_tuple.subject_id, 'relation', v_evidence_tuple.relation, 'object_type', v_evidence_tuple.object_type, 'object_id', v_evidence_tuple.object_id)), 'result', true) END);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('organization' || ':' || p_object_id),
        'relation', 'can_delete',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', true,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    ELSE
        v_node_count := v_node_count + 1;
        v_attempts := v_attempts || jsonb_build_array(jsonb_build_object('type', 'direct', 'label', 'no direct grant', 'result', false));
    END IF;
    IF v_node_count >= v_max_nodes THEN
        v_truncated := TRUE;
    END IF;
    -- All recorded attempts failed
    v_root := jsonb_build_object('type', 'union', 'children', v_attempts, 'result', false);
    v_node_count := v_node_count + 1;
    RETURN jsonb_build_object(
        'object', ('organization' || ':' || p_object_id),
        'relation', 'can_delete',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';


-- Generated explain function for organization.can_manage
-- Features: Implied
CREATE OR REPLACE FUNCTION "public"."explain_organization_can_manage"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[],
    p_max_nodes INTEGER DEFAULT NULL
) RETURNS JSONB AS $$
DECLARE
    v_key TEXT := 'organization:' || p_object_id || ':can_manage';
    v_node_count INTEGER := 0;
    v_evidence_tuple RECORD;
    v_root JSONB;
    v_attempts JSONB := '[]'::JSONB;
    v_userset_check INTEGER := 0;
    v_max_nodes INTEGER := COALESCE(p_max_nodes, current_setting('melange.max_explain_nodes', true)::INTEGER, 100);
    v_truncated BOOLEAN := FALSE;
BEGIN
    -- Cycle detection
    IF v_key = ANY(p_visited) THEN
        v_root := jsonb_build_object('type', 'cycle', 'label', v_key);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('organization' || ':' || p_object_id),
        'relation', 'can_manage',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
    IF array_length(p_visited, 1) >= 25 THEN
        RAISE EXCEPTION 'resolution too complex' USING ERRCODE = 'M2002';
    END IF;
    IF v_node_count >= v_max_nodes THEN
        v_root := jsonb_build_object('type', 'truncated');
        v_node_count := v_node_count + 1;
        v_truncated := TRUE;
        RETURN jsonb_build_object(
        'object', ('organization' || ':' || p_object_id),
        'relation', 'can_manage',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
    -- Userset subject handling (subject is itself a userset reference)
    IF position('#' in p_subject_id) > 0 THEN
        -- Case 1: self-referential userset (subject's userset resolves to this object)
        IF (p_subject_type = 'organization' AND split_part(p_subject_id, '#', 1) = p_object_id) THEN
        SELECT INTO v_userset_check 1
    WHERE substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('can_manage', 'owner')
    LIMIT 1;
        IF v_userset_check = 1 THEN
        v_root := jsonb_build_object('type', 'userset', 'label', 'self-referential userset matches relation closure', 'result', true);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('organization' || ':' || p_object_id),
        'relation', 'can_manage',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', true,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
    END IF;
    END IF;
    -- Direct/Implied grant attempt
    SELECT INTO v_evidence_tuple t.subject_type, t.subject_id, t.relation, t.object_type, t.object_id
    FROM melange_tuples AS t
    WHERE (t.object_type = 'organization' AND t.relation IN ('can_manage', 'owner') AND t.object_id = p_object_id AND t.subject_type = p_subject_type AND (t.subject_id = p_subject_id AND NOT (t.subject_id = '*')) AND t.subject_type IN ('user'))
    LIMIT 1;
    IF FOUND THEN
        v_root := (CASE WHEN v_evidence_tuple.subject_id = '*' THEN jsonb_build_object('type', 'wildcard', 'users', jsonb_build_array(jsonb_build_object('type', v_evidence_tuple.subject_type, 'id', '*')), 'result', true) ELSE jsonb_build_object('type', 'direct', 'label', ('direct or implied grant via ' || v_evidence_tuple.relation), 'evidence', jsonb_build_array(jsonb_build_object('subject_type', v_evidence_tuple.subject_type, 'subject_id', v_evidence_tuple.subject_id, 'relation', v_evidence_tuple.relation, 'object_type', v_evidence_tuple.object_type, 'object_id', v_evidence_tuple.object_id)), 'result', true) END);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('organization' || ':' || p_object_id),
        'relation', 'can_manage',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', true,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    ELSE
        v_node_count := v_node_count + 1;
        v_attempts := v_attempts || jsonb_build_array(jsonb_build_object('type', 'direct', 'label', 'no direct grant', 'result', false));
    END IF;
    IF v_node_count >= v_max_nodes THEN
        v_truncated := TRUE;
    END IF;
    -- All recorded attempts failed
    v_root := jsonb_build_object('type', 'union', 'children', v_attempts, 'result', false);
    v_node_count := v_node_count + 1;
    RETURN jsonb_build_object(
        'object', ('organization' || ':' || p_object_id),
        'relation', 'can_manage',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';


-- Generated explain function for project.can_delete
-- Features: Recursive
CREATE OR REPLACE FUNCTION "public"."explain_project_can_delete"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[],
    p_max_nodes INTEGER DEFAULT NULL
) RETURNS JSONB AS $$
DECLARE
    v_key TEXT := 'project:' || p_object_id || ':can_delete';
    v_node_count INTEGER := 0;
    v_evidence_tuple RECORD;
    v_root JSONB;
    v_attempts JSONB := '[]'::JSONB;
    v_userset_check INTEGER := 0;
    v_max_nodes INTEGER := COALESCE(p_max_nodes, current_setting('melange.max_explain_nodes', true)::INTEGER, 100);
    v_truncated BOOLEAN := FALSE;
    v_child_trace JSONB;
    v_parent_link RECORD;
BEGIN
    -- Cycle detection
    IF v_key = ANY(p_visited) THEN
        v_root := jsonb_build_object('type', 'cycle', 'label', v_key);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('project' || ':' || p_object_id),
        'relation', 'can_delete',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
    IF array_length(p_visited, 1) >= 25 THEN
        RAISE EXCEPTION 'resolution too complex' USING ERRCODE = 'M2002';
    END IF;
    IF v_node_count >= v_max_nodes THEN
        v_root := jsonb_build_object('type', 'truncated');
        v_node_count := v_node_count + 1;
        v_truncated := TRUE;
        RETURN jsonb_build_object(
        'object', ('project' || ':' || p_object_id),
        'relation', 'can_delete',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
    -- Userset subject handling (subject is itself a userset reference)
    IF position('#' in p_subject_id) > 0 THEN
        -- Case 1: self-referential userset (subject's userset resolves to this object)
        IF (p_subject_type = 'project' AND split_part(p_subject_id, '#', 1) = p_object_id) THEN
        SELECT INTO v_userset_check 1
    WHERE substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('can_delete')
    LIMIT 1;
        IF v_userset_check = 1 THEN
        v_root := jsonb_build_object('type', 'userset', 'label', 'self-referential userset matches relation closure', 'result', true);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('project' || ':' || p_object_id),
        'relation', 'can_delete',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', true,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
    END IF;
    END IF;
    -- TTU / parent-relation attempts
    FOR v_parent_link IN
        SELECT link.subject_type AS parent_type, link.subject_id AS parent_id
    FROM melange_tuples AS link
    WHERE (link.object_type = 'project' AND link.relation IN ('organization') AND link.object_id = p_object_id AND link.subject_type IN ('organization'))
    LOOP
        v_child_trace := COALESCE("public"."explain_permission_internal"(p_subject_type, p_subject_id, 'owner', v_parent_link.parent_type, v_parent_link.parent_id, p_visited || ARRAY[v_key], p_max_nodes), '{}'::jsonb);
        v_node_count := v_node_count + COALESCE((v_child_trace->>'node_count')::INTEGER, 0);
        IF v_node_count >= v_max_nodes THEN
        v_root := jsonb_build_object('type', 'truncated');
        v_node_count := v_node_count + 1;
        v_truncated := TRUE;
        RETURN jsonb_build_object(
        'object', ('project' || ':' || p_object_id),
        'relation', 'can_delete',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
        IF COALESCE((v_child_trace->>'result')::boolean, FALSE) THEN
        v_root := jsonb_build_object('type', 'ttu', 'label', ('via organization → ' || v_parent_link.parent_type || ':' || v_parent_link.parent_id || ' ⇒ owner'), 'children', jsonb_build_array(v_child_trace->'root'), 'result', true);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('project' || ':' || p_object_id),
        'relation', 'can_delete',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', true,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    ELSE
        v_node_count := v_node_count + 1;
        v_attempts := v_attempts || jsonb_build_array(jsonb_build_object('type', 'ttu', 'label', ('via organization → ' || v_parent_link.parent_type || ':' || v_parent_link.parent_id || ' ⇒ owner'), 'children', jsonb_build_array(v_child_trace->'root'), 'result', false));
    END IF;
    END LOOP;
    IF v_node_count >= v_max_nodes THEN
        v_truncated := TRUE;
    END IF;
    -- All recorded attempts failed
    v_root := jsonb_build_object('type', 'union', 'children', v_attempts, 'result', false);
    v_node_count := v_node_count + 1;
    RETURN jsonb_build_object(
        'object', ('project' || ':' || p_object_id),
        'relation', 'can_delete',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
END;
$$ LANGUAGE plpgsql STABLE COST 1000
SET search_path = 'public';


-- Generated explain function for project.can_manage
-- Features: Implied
CREATE OR REPLACE FUNCTION "public"."explain_project_can_manage"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[],
    p_max_nodes INTEGER DEFAULT NULL
) RETURNS JSONB AS $$
DECLARE
    v_key TEXT := 'project:' || p_object_id || ':can_manage';
    v_node_count INTEGER := 0;
    v_evidence_tuple RECORD;
    v_root JSONB;
    v_attempts JSONB := '[]'::JSONB;
    v_userset_check INTEGER := 0;
    v_max_nodes INTEGER := COALESCE(p_max_nodes, current_setting('melange.max_explain_nodes', true)::INTEGER, 100);
    v_truncated BOOLEAN := FALSE;
BEGIN
    -- Cycle detection
    IF v_key = ANY(p_visited) THEN
        v_root := jsonb_build_object('type', 'cycle', 'label', v_key);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('project' || ':' || p_object_id),
        'relation', 'can_manage',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
    IF array_length(p_visited, 1) >= 25 THEN
        RAISE EXCEPTION 'resolution too complex' USING ERRCODE = 'M2002';
    END IF;
    IF v_node_count >= v_max_nodes THEN
        v_root := jsonb_build_object('type', 'truncated');
        v_node_count := v_node_count + 1;
        v_truncated := TRUE;
        RETURN jsonb_build_object(
        'object', ('project' || ':' || p_object_id),
        'relation', 'can_manage',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
    -- Userset subject handling (subject is itself a userset reference)
    IF position('#' in p_subject_id) > 0 THEN
        -- Case 1: self-referential userset (subject's userset resolves to this object)
        IF (p_subject_type = 'project' AND split_part(p_subject_id, '#', 1) = p_object_id) THEN
        SELECT INTO v_userset_check 1
    WHERE substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('can_manage', 'maintainer')
    LIMIT 1;
        IF v_userset_check = 1 THEN
        v_root := jsonb_build_object('type', 'userset', 'label', 'self-referential userset matches relation closure', 'result', true);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('project' || ':' || p_object_id),
        'relation', 'can_manage',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', true,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
    END IF;
    END IF;
    -- Direct/Implied grant attempt
    SELECT INTO v_evidence_tuple t.subject_type, t.subject_id, t.relation, t.object_type, t.object_id
    FROM melange_tuples AS t
    WHERE (t.object_type = 'project' AND t.relation IN ('can_manage', 'maintainer') AND t.object_id = p_object_id AND t.subject_type = p_subject_type AND (t.subject_id = p_subject_id AND NOT (t.subject_id = '*')) AND t.subject_type IN ('user'))
    LIMIT 1;
    IF FOUND THEN
        v_root := (CASE WHEN v_evidence_tuple.subject_id = '*' THEN jsonb_build_object('type', 'wildcard', 'users', jsonb_build_array(jsonb_build_object('type', v_evidence_tuple.subject_type, 'id', '*')), 'result', true) ELSE jsonb_build_object('type', 'direct', 'label', ('direct or implied grant via ' || v_evidence_tuple.relation), 'evidence', jsonb_build_array(jsonb_build_object('subject_type', v_evidence_tuple.subject_type, 'subject_id', v_evidence_tuple.subject_id, 'relation', v_evidence_tuple.relation, 'object_type', v_evidence_tuple.object_type, 'object_id', v_evidence_tuple.object_id)), 'result', true) END);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('project' || ':' || p_object_id),
        'relation', 'can_manage',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', true,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    ELSE
        v_node_count := v_node_count + 1;
        v_attempts := v_attempts || jsonb_build_array(jsonb_build_object('type', 'direct', 'label', 'no direct grant', 'result', false));
    END IF;
    IF v_node_count >= v_max_nodes THEN
        v_truncated := TRUE;
    END IF;
    -- All recorded attempts failed
    v_root := jsonb_build_object('type', 'union', 'children', v_attempts, 'result', false);
    v_node_count := v_node_count + 1;
    RETURN jsonb_build_object(
        'object', ('project' || ':' || p_object_id),
        'relation', 'can_manage',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';


-- Generated explain function for project.can_write
-- Features: Implied
CREATE OR REPLACE FUNCTION "public"."explain_project_can_write"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[],
    p_max_nodes INTEGER DEFAULT NULL
) RETURNS JSONB AS $$
DECLARE
    v_key TEXT := 'project:' || p_object_id || ':can_write';
    v_node_count INTEGER := 0;
    v_evidence_tuple RECORD;
    v_root JSONB;
    v_attempts JSONB := '[]'::JSONB;
    v_userset_check INTEGER := 0;
    v_max_nodes INTEGER := COALESCE(p_max_nodes, current_setting('melange.max_explain_nodes', true)::INTEGER, 100);
    v_truncated BOOLEAN := FALSE;
BEGIN
    -- Cycle detection
    IF v_key = ANY(p_visited) THEN
        v_root := jsonb_build_object('type', 'cycle', 'label', v_key);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('project' || ':' || p_object_id),
        'relation', 'can_write',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
    IF array_length(p_visited, 1) >= 25 THEN
        RAISE EXCEPTION 'resolution too complex' USING ERRCODE = 'M2002';
    END IF;
    IF v_node_count >= v_max_nodes THEN
        v_root := jsonb_build_object('type', 'truncated');
        v_node_count := v_node_count + 1;
        v_truncated := TRUE;
        RETURN jsonb_build_object(
        'object', ('project' || ':' || p_object_id),
        'relation', 'can_write',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
    -- Userset subject handling (subject is itself a userset reference)
    IF position('#' in p_subject_id) > 0 THEN
        -- Case 1: self-referential userset (subject's userset resolves to this object)
        IF (p_subject_type = 'project' AND split_part(p_subject_id, '#', 1) = p_object_id) THEN
        SELECT INTO v_userset_check 1
    WHERE substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('can_write', 'maintainer')
    LIMIT 1;
        IF v_userset_check = 1 THEN
        v_root := jsonb_build_object('type', 'userset', 'label', 'self-referential userset matches relation closure', 'result', true);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('project' || ':' || p_object_id),
        'relation', 'can_write',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', true,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
    END IF;
    END IF;
    -- Direct/Implied grant attempt
    SELECT INTO v_evidence_tuple t.subject_type, t.subject_id, t.relation, t.object_type, t.object_id
    FROM melange_tuples AS t
    WHERE (t.object_type = 'project' AND t.relation IN ('can_write', 'maintainer') AND t.object_id = p_object_id AND t.subject_type = p_subject_type AND (t.subject_id = p_subject_id AND NOT (t.subject_id = '*')) AND t.subject_type IN ('user'))
    LIMIT 1;
    IF FOUND THEN
        v_root := (CASE WHEN v_evidence_tuple.subject_id = '*' THEN jsonb_build_object('type', 'wildcard', 'users', jsonb_build_array(jsonb_build_object('type', v_evidence_tuple.subject_type, 'id', '*')), 'result', true) ELSE jsonb_build_object('type', 'direct', 'label', ('direct or implied grant via ' || v_evidence_tuple.relation), 'evidence', jsonb_build_array(jsonb_build_object('subject_type', v_evidence_tuple.subject_type, 'subject_id', v_evidence_tuple.subject_id, 'relation', v_evidence_tuple.relation, 'object_type', v_evidence_tuple.object_type, 'object_id', v_evidence_tuple.object_id)), 'result', true) END);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('project' || ':' || p_object_id),
        'relation', 'can_write',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', true,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    ELSE
        v_node_count := v_node_count + 1;
        v_attempts := v_attempts || jsonb_build_array(jsonb_build_object('type', 'direct', 'label', 'no direct grant', 'result', false));
    END IF;
    IF v_node_count >= v_max_nodes THEN
        v_truncated := TRUE;
    END IF;
    -- All recorded attempts failed
    v_root := jsonb_build_object('type', 'union', 'children', v_attempts, 'result', false);
    v_node_count := v_node_count + 1;
    RETURN jsonb_build_object(
        'object', ('project' || ':' || p_object_id),
        'relation', 'can_write',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';


-- Generated explain function for organization.can_create_project
-- Features: Implied
CREATE OR REPLACE FUNCTION "public"."explain_organization_can_create_project"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[],
    p_max_nodes INTEGER DEFAULT NULL
) RETURNS JSONB AS $$
DECLARE
    v_key TEXT := 'organization:' || p_object_id || ':can_create_project';
    v_node_count INTEGER := 0;
    v_evidence_tuple RECORD;
    v_root JSONB;
    v_attempts JSONB := '[]'::JSONB;
    v_userset_check INTEGER := 0;
    v_max_nodes INTEGER := COALESCE(p_max_nodes, current_setting('melange.max_explain_nodes', true)::INTEGER, 100);
    v_truncated BOOLEAN := FALSE;
BEGIN
    -- Cycle detection
    IF v_key = ANY(p_visited) THEN
        v_root := jsonb_build_object('type', 'cycle', 'label', v_key);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('organization' || ':' || p_object_id),
        'relation', 'can_create_project',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
    IF array_length(p_visited, 1) >= 25 THEN
        RAISE EXCEPTION 'resolution too complex' USING ERRCODE = 'M2002';
    END IF;
    IF v_node_count >= v_max_nodes THEN
        v_root := jsonb_build_object('type', 'truncated');
        v_node_count := v_node_count + 1;
        v_truncated := TRUE;
        RETURN jsonb_build_object(
        'object', ('organization' || ':' || p_object_id),
        'relation', 'can_create_project',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
    -- Userset subject handling (subject is itself a userset reference)
    IF position('#' in p_subject_id) > 0 THEN
        -- Case 1: self-referential userset (subject's userset resolves to this object)
        IF (p_subject_type = 'organization' AND split_part(p_subject_id, '#', 1) = p_object_id) THEN
        SELECT INTO v_userset_check 1
    WHERE substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('admin', 'can_create_project', 'owner')
    LIMIT 1;
        IF v_userset_check = 1 THEN
        v_root := jsonb_build_object('type', 'userset', 'label', 'self-referential userset matches relation closure', 'result', true);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('organization' || ':' || p_object_id),
        'relation', 'can_create_project',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', true,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
    END IF;
    END IF;
    -- Direct/Implied grant attempt
    SELECT INTO v_evidence_tuple t.subject_type, t.subject_id, t.relation, t.object_type, t.object_id
    FROM melange_tuples AS t
    WHERE (t.object_type = 'organization' AND t.relation IN ('can_create_project', 'admin', 'owner') AND t.object_id = p_object_id AND t.subject_type = p_subject_type AND (t.subject_id = p_subject_id AND NOT (t.subject_id = '*')) AND t.subject_type IN ('user'))
    LIMIT 1;
    IF FOUND THEN
        v_root := (CASE WHEN v_evidence_tuple.subject_id = '*' THEN jsonb_build_object('type', 'wildcard', 'users', jsonb_build_array(jsonb_build_object('type', v_evidence_tuple.subject_type, 'id', '*')), 'result', true) ELSE jsonb_build_object('type', 'direct', 'label', ('direct or implied grant via ' || v_evidence_tuple.relation), 'evidence', jsonb_build_array(jsonb_build_object('subject_type', v_evidence_tuple.subject_type, 'subject_id', v_evidence_tuple.subject_id, 'relation', v_evidence_tuple.relation, 'object_type', v_evidence_tuple.object_type, 'object_id', v_evidence_tuple.object_id)), 'result', true) END);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('organization' || ':' || p_object_id),
        'relation', 'can_create_project',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', true,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    ELSE
        v_node_count := v_node_count + 1;
        v_attempts := v_attempts || jsonb_build_array(jsonb_build_object('type', 'direct', 'label', 'no direct grant', 'result', false));
    END IF;
    IF v_node_count >= v_max_nodes THEN
        v_truncated := TRUE;
    END IF;
    -- All recorded attempts failed
    v_root := jsonb_build_object('type', 'union', 'children', v_attempts, 'result', false);
    v_node_count := v_node_count + 1;
    RETURN jsonb_build_object(
        'object', ('organization' || ':' || p_object_id),
        'relation', 'can_create_project',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';


-- Generated explain function for organization.can_manage_members
-- Features: Implied
CREATE OR REPLACE FUNCTION "public"."explain_organization_can_manage_members"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[],
    p_max_nodes INTEGER DEFAULT NULL
) RETURNS JSONB AS $$
DECLARE
    v_key TEXT := 'organization:' || p_object_id || ':can_manage_members';
    v_node_count INTEGER := 0;
    v_evidence_tuple RECORD;
    v_root JSONB;
    v_attempts JSONB := '[]'::JSONB;
    v_userset_check INTEGER := 0;
    v_max_nodes INTEGER := COALESCE(p_max_nodes, current_setting('melange.max_explain_nodes', true)::INTEGER, 100);
    v_truncated BOOLEAN := FALSE;
BEGIN
    -- Cycle detection
    IF v_key = ANY(p_visited) THEN
        v_root := jsonb_build_object('type', 'cycle', 'label', v_key);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('organization' || ':' || p_object_id),
        'relation', 'can_manage_members',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
    IF array_length(p_visited, 1) >= 25 THEN
        RAISE EXCEPTION 'resolution too complex' USING ERRCODE = 'M2002';
    END IF;
    IF v_node_count >= v_max_nodes THEN
        v_root := jsonb_build_object('type', 'truncated');
        v_node_count := v_node_count + 1;
        v_truncated := TRUE;
        RETURN jsonb_build_object(
        'object', ('organization' || ':' || p_object_id),
        'relation', 'can_manage_members',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
    -- Userset subject handling (subject is itself a userset reference)
    IF position('#' in p_subject_id) > 0 THEN
        -- Case 1: self-referential userset (subject's userset resolves to this object)
        IF (p_subject_type = 'organization' AND split_part(p_subject_id, '#', 1) = p_object_id) THEN
        SELECT INTO v_userset_check 1
    WHERE substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('admin', 'can_manage_members', 'owner')
    LIMIT 1;
        IF v_userset_check = 1 THEN
        v_root := jsonb_build_object('type', 'userset', 'label', 'self-referential userset matches relation closure', 'result', true);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('organization' || ':' || p_object_id),
        'relation', 'can_manage_members',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', true,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
    END IF;
    END IF;
    -- Direct/Implied grant attempt
    SELECT INTO v_evidence_tuple t.subject_type, t.subject_id, t.relation, t.object_type, t.object_id
    FROM melange_tuples AS t
    WHERE (t.object_type = 'organization' AND t.relation IN ('can_manage_members', 'admin', 'owner') AND t.object_id = p_object_id AND t.subject_type = p_subject_type AND (t.subject_id = p_subject_id AND NOT (t.subject_id = '*')) AND t.subject_type IN ('user'))
    LIMIT 1;
    IF FOUND THEN
        v_root := (CASE WHEN v_evidence_tuple.subject_id = '*' THEN jsonb_build_object('type', 'wildcard', 'users', jsonb_build_array(jsonb_build_object('type', v_evidence_tuple.subject_type, 'id', '*')), 'result', true) ELSE jsonb_build_object('type', 'direct', 'label', ('direct or implied grant via ' || v_evidence_tuple.relation), 'evidence', jsonb_build_array(jsonb_build_object('subject_type', v_evidence_tuple.subject_type, 'subject_id', v_evidence_tuple.subject_id, 'relation', v_evidence_tuple.relation, 'object_type', v_evidence_tuple.object_type, 'object_id', v_evidence_tuple.object_id)), 'result', true) END);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('organization' || ':' || p_object_id),
        'relation', 'can_manage_members',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', true,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    ELSE
        v_node_count := v_node_count + 1;
        v_attempts := v_attempts || jsonb_build_array(jsonb_build_object('type', 'direct', 'label', 'no direct grant', 'result', false));
    END IF;
    IF v_node_count >= v_max_nodes THEN
        v_truncated := TRUE;
    END IF;
    -- All recorded attempts failed
    v_root := jsonb_build_object('type', 'union', 'children', v_attempts, 'result', false);
    v_node_count := v_node_count + 1;
    RETURN jsonb_build_object(
        'object', ('organization' || ':' || p_object_id),
        'relation', 'can_manage_members',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';


-- Generated explain function for organization.member
-- Features: Direct+Implied
CREATE OR REPLACE FUNCTION "public"."explain_organization_member"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[],
    p_max_nodes INTEGER DEFAULT NULL
) RETURNS JSONB AS $$
DECLARE
    v_key TEXT := 'organization:' || p_object_id || ':member';
    v_node_count INTEGER := 0;
    v_evidence_tuple RECORD;
    v_root JSONB;
    v_attempts JSONB := '[]'::JSONB;
    v_userset_check INTEGER := 0;
    v_max_nodes INTEGER := COALESCE(p_max_nodes, current_setting('melange.max_explain_nodes', true)::INTEGER, 100);
    v_truncated BOOLEAN := FALSE;
BEGIN
    -- Cycle detection
    IF v_key = ANY(p_visited) THEN
        v_root := jsonb_build_object('type', 'cycle', 'label', v_key);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('organization' || ':' || p_object_id),
        'relation', 'member',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
    IF array_length(p_visited, 1) >= 25 THEN
        RAISE EXCEPTION 'resolution too complex' USING ERRCODE = 'M2002';
    END IF;
    IF v_node_count >= v_max_nodes THEN
        v_root := jsonb_build_object('type', 'truncated');
        v_node_count := v_node_count + 1;
        v_truncated := TRUE;
        RETURN jsonb_build_object(
        'object', ('organization' || ':' || p_object_id),
        'relation', 'member',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
    -- Userset subject handling (subject is itself a userset reference)
    IF position('#' in p_subject_id) > 0 THEN
        -- Case 1: self-referential userset (subject's userset resolves to this object)
        IF (p_subject_type = 'organization' AND split_part(p_subject_id, '#', 1) = p_object_id) THEN
        SELECT INTO v_userset_check 1
    WHERE substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('admin', 'member', 'owner')
    LIMIT 1;
        IF v_userset_check = 1 THEN
        v_root := jsonb_build_object('type', 'userset', 'label', 'self-referential userset matches relation closure', 'result', true);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('organization' || ':' || p_object_id),
        'relation', 'member',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', true,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
    END IF;
    END IF;
    -- Direct/Implied grant attempt
    SELECT INTO v_evidence_tuple t.subject_type, t.subject_id, t.relation, t.object_type, t.object_id
    FROM melange_tuples AS t
    WHERE (t.object_type = 'organization' AND t.relation IN ('member', 'admin', 'owner') AND t.object_id = p_object_id AND t.subject_type = p_subject_type AND (t.subject_id = p_subject_id AND NOT (t.subject_id = '*')) AND t.subject_type IN ('user'))
    LIMIT 1;
    IF FOUND THEN
        v_root := (CASE WHEN v_evidence_tuple.subject_id = '*' THEN jsonb_build_object('type', 'wildcard', 'users', jsonb_build_array(jsonb_build_object('type', v_evidence_tuple.subject_type, 'id', '*')), 'result', true) ELSE jsonb_build_object('type', 'direct', 'label', ('direct or implied grant via ' || v_evidence_tuple.relation), 'evidence', jsonb_build_array(jsonb_build_object('subject_type', v_evidence_tuple.subject_type, 'subject_id', v_evidence_tuple.subject_id, 'relation', v_evidence_tuple.relation, 'object_type', v_evidence_tuple.object_type, 'object_id', v_evidence_tuple.object_id)), 'result', true) END);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('organization' || ':' || p_object_id),
        'relation', 'member',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', true,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    ELSE
        v_node_count := v_node_count + 1;
        v_attempts := v_attempts || jsonb_build_array(jsonb_build_object('type', 'direct', 'label', 'no direct grant', 'result', false));
    END IF;
    IF v_node_count >= v_max_nodes THEN
        v_truncated := TRUE;
    END IF;
    -- All recorded attempts failed
    v_root := jsonb_build_object('type', 'union', 'children', v_attempts, 'result', false);
    v_node_count := v_node_count + 1;
    RETURN jsonb_build_object(
        'object', ('organization' || ':' || p_object_id),
        'relation', 'member',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';


-- Generated explain function for repository.can_delete
-- Features: Recursive
CREATE OR REPLACE FUNCTION "public"."explain_repository_can_delete"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[],
    p_max_nodes INTEGER DEFAULT NULL
) RETURNS JSONB AS $$
DECLARE
    v_key TEXT := 'repository:' || p_object_id || ':can_delete';
    v_node_count INTEGER := 0;
    v_evidence_tuple RECORD;
    v_root JSONB;
    v_attempts JSONB := '[]'::JSONB;
    v_userset_check INTEGER := 0;
    v_max_nodes INTEGER := COALESCE(p_max_nodes, current_setting('melange.max_explain_nodes', true)::INTEGER, 100);
    v_truncated BOOLEAN := FALSE;
    v_child_trace JSONB;
    v_parent_link RECORD;
BEGIN
    -- Cycle detection
    IF v_key = ANY(p_visited) THEN
        v_root := jsonb_build_object('type', 'cycle', 'label', v_key);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('repository' || ':' || p_object_id),
        'relation', 'can_delete',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
    IF array_length(p_visited, 1) >= 25 THEN
        RAISE EXCEPTION 'resolution too complex' USING ERRCODE = 'M2002';
    END IF;
    IF v_node_count >= v_max_nodes THEN
        v_root := jsonb_build_object('type', 'truncated');
        v_node_count := v_node_count + 1;
        v_truncated := TRUE;
        RETURN jsonb_build_object(
        'object', ('repository' || ':' || p_object_id),
        'relation', 'can_delete',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
    -- Userset subject handling (subject is itself a userset reference)
    IF position('#' in p_subject_id) > 0 THEN
        -- Case 1: self-referential userset (subject's userset resolves to this object)
        IF (p_subject_type = 'repository' AND split_part(p_subject_id, '#', 1) = p_object_id) THEN
        SELECT INTO v_userset_check 1
    WHERE substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('can_delete')
    LIMIT 1;
        IF v_userset_check = 1 THEN
        v_root := jsonb_build_object('type', 'userset', 'label', 'self-referential userset matches relation closure', 'result', true);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('repository' || ':' || p_object_id),
        'relation', 'can_delete',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', true,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
    END IF;
    END IF;
    -- TTU / parent-relation attempts
    FOR v_parent_link IN
        SELECT link.subject_type AS parent_type, link.subject_id AS parent_id
    FROM melange_tuples AS link
    WHERE (link.object_type = 'repository' AND link.relation IN ('project') AND link.object_id = p_object_id AND link.subject_type IN ('project'))
    LOOP
        v_child_trace := COALESCE("public"."explain_permission_internal"(p_subject_type, p_subject_id, 'can_delete', v_parent_link.parent_type, v_parent_link.parent_id, p_visited || ARRAY[v_key], p_max_nodes), '{}'::jsonb);
        v_node_count := v_node_count + COALESCE((v_child_trace->>'node_count')::INTEGER, 0);
        IF v_node_count >= v_max_nodes THEN
        v_root := jsonb_build_object('type', 'truncated');
        v_node_count := v_node_count + 1;
        v_truncated := TRUE;
        RETURN jsonb_build_object(
        'object', ('repository' || ':' || p_object_id),
        'relation', 'can_delete',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
        IF COALESCE((v_child_trace->>'result')::boolean, FALSE) THEN
        v_root := jsonb_build_object('type', 'ttu', 'label', ('via project → ' || v_parent_link.parent_type || ':' || v_parent_link.parent_id || ' ⇒ can_delete'), 'children', jsonb_build_array(v_child_trace->'root'), 'result', true);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('repository' || ':' || p_object_id),
        'relation', 'can_delete',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', true,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    ELSE
        v_node_count := v_node_count + 1;
        v_attempts := v_attempts || jsonb_build_array(jsonb_build_object('type', 'ttu', 'label', ('via project → ' || v_parent_link.parent_type || ':' || v_parent_link.parent_id || ' ⇒ can_delete'), 'children', jsonb_build_array(v_child_trace->'root'), 'result', false));
    END IF;
    END LOOP;
    IF v_node_count >= v_max_nodes THEN
        v_truncated := TRUE;
    END IF;
    -- All recorded attempts failed
    v_root := jsonb_build_object('type', 'union', 'children', v_attempts, 'result', false);
    v_node_count := v_node_count + 1;
    RETURN jsonb_build_object(
        'object', ('repository' || ':' || p_object_id),
        'relation', 'can_delete',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
END;
$$ LANGUAGE plpgsql STABLE COST 1000
SET search_path = 'public';


-- Generated explain function for agent.can_manage
-- Features: Recursive
CREATE OR REPLACE FUNCTION "public"."explain_agent_can_manage"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[],
    p_max_nodes INTEGER DEFAULT NULL
) RETURNS JSONB AS $$
DECLARE
    v_key TEXT := 'agent:' || p_object_id || ':can_manage';
    v_node_count INTEGER := 0;
    v_evidence_tuple RECORD;
    v_root JSONB;
    v_attempts JSONB := '[]'::JSONB;
    v_userset_check INTEGER := 0;
    v_max_nodes INTEGER := COALESCE(p_max_nodes, current_setting('melange.max_explain_nodes', true)::INTEGER, 100);
    v_truncated BOOLEAN := FALSE;
    v_child_trace JSONB;
    v_parent_link RECORD;
BEGIN
    -- Cycle detection
    IF v_key = ANY(p_visited) THEN
        v_root := jsonb_build_object('type', 'cycle', 'label', v_key);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('agent' || ':' || p_object_id),
        'relation', 'can_manage',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
    IF array_length(p_visited, 1) >= 25 THEN
        RAISE EXCEPTION 'resolution too complex' USING ERRCODE = 'M2002';
    END IF;
    IF v_node_count >= v_max_nodes THEN
        v_root := jsonb_build_object('type', 'truncated');
        v_node_count := v_node_count + 1;
        v_truncated := TRUE;
        RETURN jsonb_build_object(
        'object', ('agent' || ':' || p_object_id),
        'relation', 'can_manage',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
    -- Userset subject handling (subject is itself a userset reference)
    IF position('#' in p_subject_id) > 0 THEN
        -- Case 1: self-referential userset (subject's userset resolves to this object)
        IF (p_subject_type = 'agent' AND split_part(p_subject_id, '#', 1) = p_object_id) THEN
        SELECT INTO v_userset_check 1
    WHERE substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('can_manage')
    LIMIT 1;
        IF v_userset_check = 1 THEN
        v_root := jsonb_build_object('type', 'userset', 'label', 'self-referential userset matches relation closure', 'result', true);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('agent' || ':' || p_object_id),
        'relation', 'can_manage',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', true,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
    END IF;
    END IF;
    -- TTU / parent-relation attempts
    FOR v_parent_link IN
        SELECT link.subject_type AS parent_type, link.subject_id AS parent_id
    FROM melange_tuples AS link
    WHERE (link.object_type = 'agent' AND link.relation IN ('project') AND link.object_id = p_object_id AND link.subject_type IN ('project'))
    LOOP
        v_child_trace := COALESCE("public"."explain_permission_internal"(p_subject_type, p_subject_id, 'can_manage', v_parent_link.parent_type, v_parent_link.parent_id, p_visited || ARRAY[v_key], p_max_nodes), '{}'::jsonb);
        v_node_count := v_node_count + COALESCE((v_child_trace->>'node_count')::INTEGER, 0);
        IF v_node_count >= v_max_nodes THEN
        v_root := jsonb_build_object('type', 'truncated');
        v_node_count := v_node_count + 1;
        v_truncated := TRUE;
        RETURN jsonb_build_object(
        'object', ('agent' || ':' || p_object_id),
        'relation', 'can_manage',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
        IF COALESCE((v_child_trace->>'result')::boolean, FALSE) THEN
        v_root := jsonb_build_object('type', 'ttu', 'label', ('via project → ' || v_parent_link.parent_type || ':' || v_parent_link.parent_id || ' ⇒ can_manage'), 'children', jsonb_build_array(v_child_trace->'root'), 'result', true);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('agent' || ':' || p_object_id),
        'relation', 'can_manage',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', true,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    ELSE
        v_node_count := v_node_count + 1;
        v_attempts := v_attempts || jsonb_build_array(jsonb_build_object('type', 'ttu', 'label', ('via project → ' || v_parent_link.parent_type || ':' || v_parent_link.parent_id || ' ⇒ can_manage'), 'children', jsonb_build_array(v_child_trace->'root'), 'result', false));
    END IF;
    END LOOP;
    IF v_node_count >= v_max_nodes THEN
        v_truncated := TRUE;
    END IF;
    -- All recorded attempts failed
    v_root := jsonb_build_object('type', 'union', 'children', v_attempts, 'result', false);
    v_node_count := v_node_count + 1;
    RETURN jsonb_build_object(
        'object', ('agent' || ':' || p_object_id),
        'relation', 'can_manage',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
END;
$$ LANGUAGE plpgsql STABLE COST 1000
SET search_path = 'public';


-- Generated explain function for agent.can_execute
-- Features: Recursive
CREATE OR REPLACE FUNCTION "public"."explain_agent_can_execute"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[],
    p_max_nodes INTEGER DEFAULT NULL
) RETURNS JSONB AS $$
DECLARE
    v_key TEXT := 'agent:' || p_object_id || ':can_execute';
    v_node_count INTEGER := 0;
    v_evidence_tuple RECORD;
    v_root JSONB;
    v_attempts JSONB := '[]'::JSONB;
    v_userset_check INTEGER := 0;
    v_max_nodes INTEGER := COALESCE(p_max_nodes, current_setting('melange.max_explain_nodes', true)::INTEGER, 100);
    v_truncated BOOLEAN := FALSE;
    v_child_trace JSONB;
    v_parent_link RECORD;
BEGIN
    -- Cycle detection
    IF v_key = ANY(p_visited) THEN
        v_root := jsonb_build_object('type', 'cycle', 'label', v_key);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('agent' || ':' || p_object_id),
        'relation', 'can_execute',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
    IF array_length(p_visited, 1) >= 25 THEN
        RAISE EXCEPTION 'resolution too complex' USING ERRCODE = 'M2002';
    END IF;
    IF v_node_count >= v_max_nodes THEN
        v_root := jsonb_build_object('type', 'truncated');
        v_node_count := v_node_count + 1;
        v_truncated := TRUE;
        RETURN jsonb_build_object(
        'object', ('agent' || ':' || p_object_id),
        'relation', 'can_execute',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
    -- Userset subject handling (subject is itself a userset reference)
    IF position('#' in p_subject_id) > 0 THEN
        -- Case 1: self-referential userset (subject's userset resolves to this object)
        IF (p_subject_type = 'agent' AND split_part(p_subject_id, '#', 1) = p_object_id) THEN
        SELECT INTO v_userset_check 1
    WHERE substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('can_execute')
    LIMIT 1;
        IF v_userset_check = 1 THEN
        v_root := jsonb_build_object('type', 'userset', 'label', 'self-referential userset matches relation closure', 'result', true);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('agent' || ':' || p_object_id),
        'relation', 'can_execute',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', true,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
    END IF;
    END IF;
    -- TTU / parent-relation attempts
    FOR v_parent_link IN
        SELECT link.subject_type AS parent_type, link.subject_id AS parent_id
    FROM melange_tuples AS link
    WHERE (link.object_type = 'agent' AND link.relation IN ('project') AND link.object_id = p_object_id AND link.subject_type IN ('project'))
    LOOP
        v_child_trace := COALESCE("public"."explain_permission_internal"(p_subject_type, p_subject_id, 'can_write', v_parent_link.parent_type, v_parent_link.parent_id, p_visited || ARRAY[v_key], p_max_nodes), '{}'::jsonb);
        v_node_count := v_node_count + COALESCE((v_child_trace->>'node_count')::INTEGER, 0);
        IF v_node_count >= v_max_nodes THEN
        v_root := jsonb_build_object('type', 'truncated');
        v_node_count := v_node_count + 1;
        v_truncated := TRUE;
        RETURN jsonb_build_object(
        'object', ('agent' || ':' || p_object_id),
        'relation', 'can_execute',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
        IF COALESCE((v_child_trace->>'result')::boolean, FALSE) THEN
        v_root := jsonb_build_object('type', 'ttu', 'label', ('via project → ' || v_parent_link.parent_type || ':' || v_parent_link.parent_id || ' ⇒ can_write'), 'children', jsonb_build_array(v_child_trace->'root'), 'result', true);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('agent' || ':' || p_object_id),
        'relation', 'can_execute',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', true,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    ELSE
        v_node_count := v_node_count + 1;
        v_attempts := v_attempts || jsonb_build_array(jsonb_build_object('type', 'ttu', 'label', ('via project → ' || v_parent_link.parent_type || ':' || v_parent_link.parent_id || ' ⇒ can_write'), 'children', jsonb_build_array(v_child_trace->'root'), 'result', false));
    END IF;
    END LOOP;
    IF v_node_count >= v_max_nodes THEN
        v_truncated := TRUE;
    END IF;
    -- All recorded attempts failed
    v_root := jsonb_build_object('type', 'union', 'children', v_attempts, 'result', false);
    v_node_count := v_node_count + 1;
    RETURN jsonb_build_object(
        'object', ('agent' || ':' || p_object_id),
        'relation', 'can_execute',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
END;
$$ LANGUAGE plpgsql STABLE COST 1000
SET search_path = 'public';


-- Generated explain function for repository.can_write
-- Features: Recursive
CREATE OR REPLACE FUNCTION "public"."explain_repository_can_write"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[],
    p_max_nodes INTEGER DEFAULT NULL
) RETURNS JSONB AS $$
DECLARE
    v_key TEXT := 'repository:' || p_object_id || ':can_write';
    v_node_count INTEGER := 0;
    v_evidence_tuple RECORD;
    v_root JSONB;
    v_attempts JSONB := '[]'::JSONB;
    v_userset_check INTEGER := 0;
    v_max_nodes INTEGER := COALESCE(p_max_nodes, current_setting('melange.max_explain_nodes', true)::INTEGER, 100);
    v_truncated BOOLEAN := FALSE;
    v_child_trace JSONB;
    v_parent_link RECORD;
BEGIN
    -- Cycle detection
    IF v_key = ANY(p_visited) THEN
        v_root := jsonb_build_object('type', 'cycle', 'label', v_key);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('repository' || ':' || p_object_id),
        'relation', 'can_write',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
    IF array_length(p_visited, 1) >= 25 THEN
        RAISE EXCEPTION 'resolution too complex' USING ERRCODE = 'M2002';
    END IF;
    IF v_node_count >= v_max_nodes THEN
        v_root := jsonb_build_object('type', 'truncated');
        v_node_count := v_node_count + 1;
        v_truncated := TRUE;
        RETURN jsonb_build_object(
        'object', ('repository' || ':' || p_object_id),
        'relation', 'can_write',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
    -- Userset subject handling (subject is itself a userset reference)
    IF position('#' in p_subject_id) > 0 THEN
        -- Case 1: self-referential userset (subject's userset resolves to this object)
        IF (p_subject_type = 'repository' AND split_part(p_subject_id, '#', 1) = p_object_id) THEN
        SELECT INTO v_userset_check 1
    WHERE substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('can_write')
    LIMIT 1;
        IF v_userset_check = 1 THEN
        v_root := jsonb_build_object('type', 'userset', 'label', 'self-referential userset matches relation closure', 'result', true);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('repository' || ':' || p_object_id),
        'relation', 'can_write',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', true,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
    END IF;
    END IF;
    -- TTU / parent-relation attempts
    FOR v_parent_link IN
        SELECT link.subject_type AS parent_type, link.subject_id AS parent_id
    FROM melange_tuples AS link
    WHERE (link.object_type = 'repository' AND link.relation IN ('project') AND link.object_id = p_object_id AND link.subject_type IN ('project'))
    LOOP
        v_child_trace := COALESCE("public"."explain_permission_internal"(p_subject_type, p_subject_id, 'can_write', v_parent_link.parent_type, v_parent_link.parent_id, p_visited || ARRAY[v_key], p_max_nodes), '{}'::jsonb);
        v_node_count := v_node_count + COALESCE((v_child_trace->>'node_count')::INTEGER, 0);
        IF v_node_count >= v_max_nodes THEN
        v_root := jsonb_build_object('type', 'truncated');
        v_node_count := v_node_count + 1;
        v_truncated := TRUE;
        RETURN jsonb_build_object(
        'object', ('repository' || ':' || p_object_id),
        'relation', 'can_write',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
        IF COALESCE((v_child_trace->>'result')::boolean, FALSE) THEN
        v_root := jsonb_build_object('type', 'ttu', 'label', ('via project → ' || v_parent_link.parent_type || ':' || v_parent_link.parent_id || ' ⇒ can_write'), 'children', jsonb_build_array(v_child_trace->'root'), 'result', true);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('repository' || ':' || p_object_id),
        'relation', 'can_write',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', true,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    ELSE
        v_node_count := v_node_count + 1;
        v_attempts := v_attempts || jsonb_build_array(jsonb_build_object('type', 'ttu', 'label', ('via project → ' || v_parent_link.parent_type || ':' || v_parent_link.parent_id || ' ⇒ can_write'), 'children', jsonb_build_array(v_child_trace->'root'), 'result', false));
    END IF;
    END LOOP;
    IF v_node_count >= v_max_nodes THEN
        v_truncated := TRUE;
    END IF;
    -- All recorded attempts failed
    v_root := jsonb_build_object('type', 'union', 'children', v_attempts, 'result', false);
    v_node_count := v_node_count + 1;
    RETURN jsonb_build_object(
        'object', ('repository' || ':' || p_object_id),
        'relation', 'can_write',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
END;
$$ LANGUAGE plpgsql STABLE COST 1000
SET search_path = 'public';


-- Generated explain function for organization.can_read
-- Features: Implied
CREATE OR REPLACE FUNCTION "public"."explain_organization_can_read"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[],
    p_max_nodes INTEGER DEFAULT NULL
) RETURNS JSONB AS $$
DECLARE
    v_key TEXT := 'organization:' || p_object_id || ':can_read';
    v_node_count INTEGER := 0;
    v_evidence_tuple RECORD;
    v_root JSONB;
    v_attempts JSONB := '[]'::JSONB;
    v_userset_check INTEGER := 0;
    v_max_nodes INTEGER := COALESCE(p_max_nodes, current_setting('melange.max_explain_nodes', true)::INTEGER, 100);
    v_truncated BOOLEAN := FALSE;
BEGIN
    -- Cycle detection
    IF v_key = ANY(p_visited) THEN
        v_root := jsonb_build_object('type', 'cycle', 'label', v_key);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('organization' || ':' || p_object_id),
        'relation', 'can_read',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
    IF array_length(p_visited, 1) >= 25 THEN
        RAISE EXCEPTION 'resolution too complex' USING ERRCODE = 'M2002';
    END IF;
    IF v_node_count >= v_max_nodes THEN
        v_root := jsonb_build_object('type', 'truncated');
        v_node_count := v_node_count + 1;
        v_truncated := TRUE;
        RETURN jsonb_build_object(
        'object', ('organization' || ':' || p_object_id),
        'relation', 'can_read',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
    -- Userset subject handling (subject is itself a userset reference)
    IF position('#' in p_subject_id) > 0 THEN
        -- Case 1: self-referential userset (subject's userset resolves to this object)
        IF (p_subject_type = 'organization' AND split_part(p_subject_id, '#', 1) = p_object_id) THEN
        SELECT INTO v_userset_check 1
    WHERE substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('admin', 'can_read', 'member', 'owner')
    LIMIT 1;
        IF v_userset_check = 1 THEN
        v_root := jsonb_build_object('type', 'userset', 'label', 'self-referential userset matches relation closure', 'result', true);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('organization' || ':' || p_object_id),
        'relation', 'can_read',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', true,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
    END IF;
    END IF;
    -- Direct/Implied grant attempt
    SELECT INTO v_evidence_tuple t.subject_type, t.subject_id, t.relation, t.object_type, t.object_id
    FROM melange_tuples AS t
    WHERE (t.object_type = 'organization' AND t.relation IN ('can_read', 'admin', 'member', 'owner') AND t.object_id = p_object_id AND t.subject_type = p_subject_type AND (t.subject_id = p_subject_id AND NOT (t.subject_id = '*')) AND t.subject_type IN ('user'))
    LIMIT 1;
    IF FOUND THEN
        v_root := (CASE WHEN v_evidence_tuple.subject_id = '*' THEN jsonb_build_object('type', 'wildcard', 'users', jsonb_build_array(jsonb_build_object('type', v_evidence_tuple.subject_type, 'id', '*')), 'result', true) ELSE jsonb_build_object('type', 'direct', 'label', ('direct or implied grant via ' || v_evidence_tuple.relation), 'evidence', jsonb_build_array(jsonb_build_object('subject_type', v_evidence_tuple.subject_type, 'subject_id', v_evidence_tuple.subject_id, 'relation', v_evidence_tuple.relation, 'object_type', v_evidence_tuple.object_type, 'object_id', v_evidence_tuple.object_id)), 'result', true) END);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('organization' || ':' || p_object_id),
        'relation', 'can_read',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', true,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    ELSE
        v_node_count := v_node_count + 1;
        v_attempts := v_attempts || jsonb_build_array(jsonb_build_object('type', 'direct', 'label', 'no direct grant', 'result', false));
    END IF;
    IF v_node_count >= v_max_nodes THEN
        v_truncated := TRUE;
    END IF;
    -- All recorded attempts failed
    v_root := jsonb_build_object('type', 'union', 'children', v_attempts, 'result', false);
    v_node_count := v_node_count + 1;
    RETURN jsonb_build_object(
        'object', ('organization' || ':' || p_object_id),
        'relation', 'can_read',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';


-- Generated explain function for state_volume.can_manage
-- Features: Recursive
CREATE OR REPLACE FUNCTION "public"."explain_state_volume_can_manage"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[],
    p_max_nodes INTEGER DEFAULT NULL
) RETURNS JSONB AS $$
DECLARE
    v_key TEXT := 'state_volume:' || p_object_id || ':can_manage';
    v_node_count INTEGER := 0;
    v_evidence_tuple RECORD;
    v_root JSONB;
    v_attempts JSONB := '[]'::JSONB;
    v_userset_check INTEGER := 0;
    v_max_nodes INTEGER := COALESCE(p_max_nodes, current_setting('melange.max_explain_nodes', true)::INTEGER, 100);
    v_truncated BOOLEAN := FALSE;
    v_child_trace JSONB;
    v_parent_link RECORD;
BEGIN
    -- Cycle detection
    IF v_key = ANY(p_visited) THEN
        v_root := jsonb_build_object('type', 'cycle', 'label', v_key);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('state_volume' || ':' || p_object_id),
        'relation', 'can_manage',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
    IF array_length(p_visited, 1) >= 25 THEN
        RAISE EXCEPTION 'resolution too complex' USING ERRCODE = 'M2002';
    END IF;
    IF v_node_count >= v_max_nodes THEN
        v_root := jsonb_build_object('type', 'truncated');
        v_node_count := v_node_count + 1;
        v_truncated := TRUE;
        RETURN jsonb_build_object(
        'object', ('state_volume' || ':' || p_object_id),
        'relation', 'can_manage',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
    -- Userset subject handling (subject is itself a userset reference)
    IF position('#' in p_subject_id) > 0 THEN
        -- Case 1: self-referential userset (subject's userset resolves to this object)
        IF (p_subject_type = 'state_volume' AND split_part(p_subject_id, '#', 1) = p_object_id) THEN
        SELECT INTO v_userset_check 1
    WHERE substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('can_manage')
    LIMIT 1;
        IF v_userset_check = 1 THEN
        v_root := jsonb_build_object('type', 'userset', 'label', 'self-referential userset matches relation closure', 'result', true);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('state_volume' || ':' || p_object_id),
        'relation', 'can_manage',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', true,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
    END IF;
    END IF;
    -- TTU / parent-relation attempts
    FOR v_parent_link IN
        SELECT link.subject_type AS parent_type, link.subject_id AS parent_id
    FROM melange_tuples AS link
    WHERE (link.object_type = 'state_volume' AND link.relation IN ('agent') AND link.object_id = p_object_id AND link.subject_type IN ('agent'))
    LOOP
        v_child_trace := COALESCE("public"."explain_permission_internal"(p_subject_type, p_subject_id, 'can_manage', v_parent_link.parent_type, v_parent_link.parent_id, p_visited || ARRAY[v_key], p_max_nodes), '{}'::jsonb);
        v_node_count := v_node_count + COALESCE((v_child_trace->>'node_count')::INTEGER, 0);
        IF v_node_count >= v_max_nodes THEN
        v_root := jsonb_build_object('type', 'truncated');
        v_node_count := v_node_count + 1;
        v_truncated := TRUE;
        RETURN jsonb_build_object(
        'object', ('state_volume' || ':' || p_object_id),
        'relation', 'can_manage',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
        IF COALESCE((v_child_trace->>'result')::boolean, FALSE) THEN
        v_root := jsonb_build_object('type', 'ttu', 'label', ('via agent → ' || v_parent_link.parent_type || ':' || v_parent_link.parent_id || ' ⇒ can_manage'), 'children', jsonb_build_array(v_child_trace->'root'), 'result', true);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('state_volume' || ':' || p_object_id),
        'relation', 'can_manage',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', true,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    ELSE
        v_node_count := v_node_count + 1;
        v_attempts := v_attempts || jsonb_build_array(jsonb_build_object('type', 'ttu', 'label', ('via agent → ' || v_parent_link.parent_type || ':' || v_parent_link.parent_id || ' ⇒ can_manage'), 'children', jsonb_build_array(v_child_trace->'root'), 'result', false));
    END IF;
    END LOOP;
    IF v_node_count >= v_max_nodes THEN
        v_truncated := TRUE;
    END IF;
    -- All recorded attempts failed
    v_root := jsonb_build_object('type', 'union', 'children', v_attempts, 'result', false);
    v_node_count := v_node_count + 1;
    RETURN jsonb_build_object(
        'object', ('state_volume' || ':' || p_object_id),
        'relation', 'can_manage',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
END;
$$ LANGUAGE plpgsql STABLE COST 1000
SET search_path = 'public';


-- Generated explain function for state_volume.can_restore
-- Features: Recursive
CREATE OR REPLACE FUNCTION "public"."explain_state_volume_can_restore"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[],
    p_max_nodes INTEGER DEFAULT NULL
) RETURNS JSONB AS $$
DECLARE
    v_key TEXT := 'state_volume:' || p_object_id || ':can_restore';
    v_node_count INTEGER := 0;
    v_evidence_tuple RECORD;
    v_root JSONB;
    v_attempts JSONB := '[]'::JSONB;
    v_userset_check INTEGER := 0;
    v_max_nodes INTEGER := COALESCE(p_max_nodes, current_setting('melange.max_explain_nodes', true)::INTEGER, 100);
    v_truncated BOOLEAN := FALSE;
    v_child_trace JSONB;
    v_parent_link RECORD;
BEGIN
    -- Cycle detection
    IF v_key = ANY(p_visited) THEN
        v_root := jsonb_build_object('type', 'cycle', 'label', v_key);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('state_volume' || ':' || p_object_id),
        'relation', 'can_restore',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
    IF array_length(p_visited, 1) >= 25 THEN
        RAISE EXCEPTION 'resolution too complex' USING ERRCODE = 'M2002';
    END IF;
    IF v_node_count >= v_max_nodes THEN
        v_root := jsonb_build_object('type', 'truncated');
        v_node_count := v_node_count + 1;
        v_truncated := TRUE;
        RETURN jsonb_build_object(
        'object', ('state_volume' || ':' || p_object_id),
        'relation', 'can_restore',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
    -- Userset subject handling (subject is itself a userset reference)
    IF position('#' in p_subject_id) > 0 THEN
        -- Case 1: self-referential userset (subject's userset resolves to this object)
        IF (p_subject_type = 'state_volume' AND split_part(p_subject_id, '#', 1) = p_object_id) THEN
        SELECT INTO v_userset_check 1
    WHERE substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('can_restore')
    LIMIT 1;
        IF v_userset_check = 1 THEN
        v_root := jsonb_build_object('type', 'userset', 'label', 'self-referential userset matches relation closure', 'result', true);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('state_volume' || ':' || p_object_id),
        'relation', 'can_restore',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', true,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
    END IF;
    END IF;
    -- TTU / parent-relation attempts
    FOR v_parent_link IN
        SELECT link.subject_type AS parent_type, link.subject_id AS parent_id
    FROM melange_tuples AS link
    WHERE (link.object_type = 'state_volume' AND link.relation IN ('agent') AND link.object_id = p_object_id AND link.subject_type IN ('agent'))
    LOOP
        v_child_trace := COALESCE("public"."explain_permission_internal"(p_subject_type, p_subject_id, 'can_manage', v_parent_link.parent_type, v_parent_link.parent_id, p_visited || ARRAY[v_key], p_max_nodes), '{}'::jsonb);
        v_node_count := v_node_count + COALESCE((v_child_trace->>'node_count')::INTEGER, 0);
        IF v_node_count >= v_max_nodes THEN
        v_root := jsonb_build_object('type', 'truncated');
        v_node_count := v_node_count + 1;
        v_truncated := TRUE;
        RETURN jsonb_build_object(
        'object', ('state_volume' || ':' || p_object_id),
        'relation', 'can_restore',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
        IF COALESCE((v_child_trace->>'result')::boolean, FALSE) THEN
        v_root := jsonb_build_object('type', 'ttu', 'label', ('via agent → ' || v_parent_link.parent_type || ':' || v_parent_link.parent_id || ' ⇒ can_manage'), 'children', jsonb_build_array(v_child_trace->'root'), 'result', true);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('state_volume' || ':' || p_object_id),
        'relation', 'can_restore',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', true,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    ELSE
        v_node_count := v_node_count + 1;
        v_attempts := v_attempts || jsonb_build_array(jsonb_build_object('type', 'ttu', 'label', ('via agent → ' || v_parent_link.parent_type || ':' || v_parent_link.parent_id || ' ⇒ can_manage'), 'children', jsonb_build_array(v_child_trace->'root'), 'result', false));
    END IF;
    END LOOP;
    IF v_node_count >= v_max_nodes THEN
        v_truncated := TRUE;
    END IF;
    -- All recorded attempts failed
    v_root := jsonb_build_object('type', 'union', 'children', v_attempts, 'result', false);
    v_node_count := v_node_count + 1;
    RETURN jsonb_build_object(
        'object', ('state_volume' || ':' || p_object_id),
        'relation', 'can_restore',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
END;
$$ LANGUAGE plpgsql STABLE COST 1000
SET search_path = 'public';


-- Generated explain function for run.can_cancel
-- Features: Recursive
CREATE OR REPLACE FUNCTION "public"."explain_run_can_cancel"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[],
    p_max_nodes INTEGER DEFAULT NULL
) RETURNS JSONB AS $$
DECLARE
    v_key TEXT := 'run:' || p_object_id || ':can_cancel';
    v_node_count INTEGER := 0;
    v_evidence_tuple RECORD;
    v_root JSONB;
    v_attempts JSONB := '[]'::JSONB;
    v_userset_check INTEGER := 0;
    v_max_nodes INTEGER := COALESCE(p_max_nodes, current_setting('melange.max_explain_nodes', true)::INTEGER, 100);
    v_truncated BOOLEAN := FALSE;
    v_child_trace JSONB;
    v_parent_link RECORD;
BEGIN
    -- Cycle detection
    IF v_key = ANY(p_visited) THEN
        v_root := jsonb_build_object('type', 'cycle', 'label', v_key);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('run' || ':' || p_object_id),
        'relation', 'can_cancel',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
    IF array_length(p_visited, 1) >= 25 THEN
        RAISE EXCEPTION 'resolution too complex' USING ERRCODE = 'M2002';
    END IF;
    IF v_node_count >= v_max_nodes THEN
        v_root := jsonb_build_object('type', 'truncated');
        v_node_count := v_node_count + 1;
        v_truncated := TRUE;
        RETURN jsonb_build_object(
        'object', ('run' || ':' || p_object_id),
        'relation', 'can_cancel',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
    -- Userset subject handling (subject is itself a userset reference)
    IF position('#' in p_subject_id) > 0 THEN
        -- Case 1: self-referential userset (subject's userset resolves to this object)
        IF (p_subject_type = 'run' AND split_part(p_subject_id, '#', 1) = p_object_id) THEN
        SELECT INTO v_userset_check 1
    WHERE substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('can_cancel')
    LIMIT 1;
        IF v_userset_check = 1 THEN
        v_root := jsonb_build_object('type', 'userset', 'label', 'self-referential userset matches relation closure', 'result', true);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('run' || ':' || p_object_id),
        'relation', 'can_cancel',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', true,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
    END IF;
    END IF;
    -- TTU / parent-relation attempts
    FOR v_parent_link IN
        SELECT link.subject_type AS parent_type, link.subject_id AS parent_id
    FROM melange_tuples AS link
    WHERE (link.object_type = 'run' AND link.relation IN ('agent') AND link.object_id = p_object_id AND link.subject_type IN ('agent'))
    LOOP
        v_child_trace := COALESCE("public"."explain_permission_internal"(p_subject_type, p_subject_id, 'can_execute', v_parent_link.parent_type, v_parent_link.parent_id, p_visited || ARRAY[v_key], p_max_nodes), '{}'::jsonb);
        v_node_count := v_node_count + COALESCE((v_child_trace->>'node_count')::INTEGER, 0);
        IF v_node_count >= v_max_nodes THEN
        v_root := jsonb_build_object('type', 'truncated');
        v_node_count := v_node_count + 1;
        v_truncated := TRUE;
        RETURN jsonb_build_object(
        'object', ('run' || ':' || p_object_id),
        'relation', 'can_cancel',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
        IF COALESCE((v_child_trace->>'result')::boolean, FALSE) THEN
        v_root := jsonb_build_object('type', 'ttu', 'label', ('via agent → ' || v_parent_link.parent_type || ':' || v_parent_link.parent_id || ' ⇒ can_execute'), 'children', jsonb_build_array(v_child_trace->'root'), 'result', true);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('run' || ':' || p_object_id),
        'relation', 'can_cancel',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', true,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    ELSE
        v_node_count := v_node_count + 1;
        v_attempts := v_attempts || jsonb_build_array(jsonb_build_object('type', 'ttu', 'label', ('via agent → ' || v_parent_link.parent_type || ':' || v_parent_link.parent_id || ' ⇒ can_execute'), 'children', jsonb_build_array(v_child_trace->'root'), 'result', false));
    END IF;
    END LOOP;
    FOR v_parent_link IN
        SELECT link.subject_type AS parent_type, link.subject_id AS parent_id
    FROM melange_tuples AS link
    WHERE (link.object_type = 'run' AND link.relation IN ('agent') AND link.object_id = p_object_id AND link.subject_type IN ('agent'))
    LOOP
        v_child_trace := COALESCE("public"."explain_permission_internal"(p_subject_type, p_subject_id, 'can_manage', v_parent_link.parent_type, v_parent_link.parent_id, p_visited || ARRAY[v_key], p_max_nodes), '{}'::jsonb);
        v_node_count := v_node_count + COALESCE((v_child_trace->>'node_count')::INTEGER, 0);
        IF v_node_count >= v_max_nodes THEN
        v_root := jsonb_build_object('type', 'truncated');
        v_node_count := v_node_count + 1;
        v_truncated := TRUE;
        RETURN jsonb_build_object(
        'object', ('run' || ':' || p_object_id),
        'relation', 'can_cancel',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
        IF COALESCE((v_child_trace->>'result')::boolean, FALSE) THEN
        v_root := jsonb_build_object('type', 'ttu', 'label', ('via agent → ' || v_parent_link.parent_type || ':' || v_parent_link.parent_id || ' ⇒ can_manage'), 'children', jsonb_build_array(v_child_trace->'root'), 'result', true);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('run' || ':' || p_object_id),
        'relation', 'can_cancel',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', true,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    ELSE
        v_node_count := v_node_count + 1;
        v_attempts := v_attempts || jsonb_build_array(jsonb_build_object('type', 'ttu', 'label', ('via agent → ' || v_parent_link.parent_type || ':' || v_parent_link.parent_id || ' ⇒ can_manage'), 'children', jsonb_build_array(v_child_trace->'root'), 'result', false));
    END IF;
    END LOOP;
    IF v_node_count >= v_max_nodes THEN
        v_truncated := TRUE;
    END IF;
    -- All recorded attempts failed
    v_root := jsonb_build_object('type', 'union', 'children', v_attempts, 'result', false);
    v_node_count := v_node_count + 1;
    RETURN jsonb_build_object(
        'object', ('run' || ':' || p_object_id),
        'relation', 'can_cancel',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
END;
$$ LANGUAGE plpgsql STABLE COST 1000
SET search_path = 'public';


-- Generated explain function for state_volume.can_attach
-- Features: Recursive
CREATE OR REPLACE FUNCTION "public"."explain_state_volume_can_attach"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[],
    p_max_nodes INTEGER DEFAULT NULL
) RETURNS JSONB AS $$
DECLARE
    v_key TEXT := 'state_volume:' || p_object_id || ':can_attach';
    v_node_count INTEGER := 0;
    v_evidence_tuple RECORD;
    v_root JSONB;
    v_attempts JSONB := '[]'::JSONB;
    v_userset_check INTEGER := 0;
    v_max_nodes INTEGER := COALESCE(p_max_nodes, current_setting('melange.max_explain_nodes', true)::INTEGER, 100);
    v_truncated BOOLEAN := FALSE;
    v_child_trace JSONB;
    v_parent_link RECORD;
BEGIN
    -- Cycle detection
    IF v_key = ANY(p_visited) THEN
        v_root := jsonb_build_object('type', 'cycle', 'label', v_key);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('state_volume' || ':' || p_object_id),
        'relation', 'can_attach',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
    IF array_length(p_visited, 1) >= 25 THEN
        RAISE EXCEPTION 'resolution too complex' USING ERRCODE = 'M2002';
    END IF;
    IF v_node_count >= v_max_nodes THEN
        v_root := jsonb_build_object('type', 'truncated');
        v_node_count := v_node_count + 1;
        v_truncated := TRUE;
        RETURN jsonb_build_object(
        'object', ('state_volume' || ':' || p_object_id),
        'relation', 'can_attach',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
    -- Userset subject handling (subject is itself a userset reference)
    IF position('#' in p_subject_id) > 0 THEN
        -- Case 1: self-referential userset (subject's userset resolves to this object)
        IF (p_subject_type = 'state_volume' AND split_part(p_subject_id, '#', 1) = p_object_id) THEN
        SELECT INTO v_userset_check 1
    WHERE substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('can_attach')
    LIMIT 1;
        IF v_userset_check = 1 THEN
        v_root := jsonb_build_object('type', 'userset', 'label', 'self-referential userset matches relation closure', 'result', true);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('state_volume' || ':' || p_object_id),
        'relation', 'can_attach',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', true,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
    END IF;
    END IF;
    -- TTU / parent-relation attempts
    FOR v_parent_link IN
        SELECT link.subject_type AS parent_type, link.subject_id AS parent_id
    FROM melange_tuples AS link
    WHERE (link.object_type = 'state_volume' AND link.relation IN ('agent') AND link.object_id = p_object_id AND link.subject_type IN ('agent'))
    LOOP
        v_child_trace := COALESCE("public"."explain_permission_internal"(p_subject_type, p_subject_id, 'can_execute', v_parent_link.parent_type, v_parent_link.parent_id, p_visited || ARRAY[v_key], p_max_nodes), '{}'::jsonb);
        v_node_count := v_node_count + COALESCE((v_child_trace->>'node_count')::INTEGER, 0);
        IF v_node_count >= v_max_nodes THEN
        v_root := jsonb_build_object('type', 'truncated');
        v_node_count := v_node_count + 1;
        v_truncated := TRUE;
        RETURN jsonb_build_object(
        'object', ('state_volume' || ':' || p_object_id),
        'relation', 'can_attach',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
        IF COALESCE((v_child_trace->>'result')::boolean, FALSE) THEN
        v_root := jsonb_build_object('type', 'ttu', 'label', ('via agent → ' || v_parent_link.parent_type || ':' || v_parent_link.parent_id || ' ⇒ can_execute'), 'children', jsonb_build_array(v_child_trace->'root'), 'result', true);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('state_volume' || ':' || p_object_id),
        'relation', 'can_attach',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', true,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    ELSE
        v_node_count := v_node_count + 1;
        v_attempts := v_attempts || jsonb_build_array(jsonb_build_object('type', 'ttu', 'label', ('via agent → ' || v_parent_link.parent_type || ':' || v_parent_link.parent_id || ' ⇒ can_execute'), 'children', jsonb_build_array(v_child_trace->'root'), 'result', false));
    END IF;
    END LOOP;
    IF v_node_count >= v_max_nodes THEN
        v_truncated := TRUE;
    END IF;
    -- All recorded attempts failed
    v_root := jsonb_build_object('type', 'union', 'children', v_attempts, 'result', false);
    v_node_count := v_node_count + 1;
    RETURN jsonb_build_object(
        'object', ('state_volume' || ':' || p_object_id),
        'relation', 'can_attach',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
END;
$$ LANGUAGE plpgsql STABLE COST 1000
SET search_path = 'public';


-- Generated explain function for project.can_read
-- Features: Implied+Recursive
CREATE OR REPLACE FUNCTION "public"."explain_project_can_read"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[],
    p_max_nodes INTEGER DEFAULT NULL
) RETURNS JSONB AS $$
DECLARE
    v_key TEXT := 'project:' || p_object_id || ':can_read';
    v_node_count INTEGER := 0;
    v_evidence_tuple RECORD;
    v_root JSONB;
    v_attempts JSONB := '[]'::JSONB;
    v_userset_check INTEGER := 0;
    v_max_nodes INTEGER := COALESCE(p_max_nodes, current_setting('melange.max_explain_nodes', true)::INTEGER, 100);
    v_truncated BOOLEAN := FALSE;
    v_child_trace JSONB;
    v_parent_link RECORD;
BEGIN
    -- Cycle detection
    IF v_key = ANY(p_visited) THEN
        v_root := jsonb_build_object('type', 'cycle', 'label', v_key);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('project' || ':' || p_object_id),
        'relation', 'can_read',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
    IF array_length(p_visited, 1) >= 25 THEN
        RAISE EXCEPTION 'resolution too complex' USING ERRCODE = 'M2002';
    END IF;
    IF v_node_count >= v_max_nodes THEN
        v_root := jsonb_build_object('type', 'truncated');
        v_node_count := v_node_count + 1;
        v_truncated := TRUE;
        RETURN jsonb_build_object(
        'object', ('project' || ':' || p_object_id),
        'relation', 'can_read',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
    -- Userset subject handling (subject is itself a userset reference)
    IF position('#' in p_subject_id) > 0 THEN
        -- Case 1: self-referential userset (subject's userset resolves to this object)
        IF (p_subject_type = 'project' AND split_part(p_subject_id, '#', 1) = p_object_id) THEN
        SELECT INTO v_userset_check 1
    WHERE substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('can_read', 'maintainer')
    LIMIT 1;
        IF v_userset_check = 1 THEN
        v_root := jsonb_build_object('type', 'userset', 'label', 'self-referential userset matches relation closure', 'result', true);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('project' || ':' || p_object_id),
        'relation', 'can_read',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', true,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
    END IF;
    END IF;
    -- Direct/Implied grant attempt
    SELECT INTO v_evidence_tuple t.subject_type, t.subject_id, t.relation, t.object_type, t.object_id
    FROM melange_tuples AS t
    WHERE (t.object_type = 'project' AND t.relation IN ('can_read', 'maintainer') AND t.object_id = p_object_id AND t.subject_type = p_subject_type AND (t.subject_id = p_subject_id AND NOT (t.subject_id = '*')) AND t.subject_type IN ('user'))
    LIMIT 1;
    IF FOUND THEN
        v_root := (CASE WHEN v_evidence_tuple.subject_id = '*' THEN jsonb_build_object('type', 'wildcard', 'users', jsonb_build_array(jsonb_build_object('type', v_evidence_tuple.subject_type, 'id', '*')), 'result', true) ELSE jsonb_build_object('type', 'direct', 'label', ('direct or implied grant via ' || v_evidence_tuple.relation), 'evidence', jsonb_build_array(jsonb_build_object('subject_type', v_evidence_tuple.subject_type, 'subject_id', v_evidence_tuple.subject_id, 'relation', v_evidence_tuple.relation, 'object_type', v_evidence_tuple.object_type, 'object_id', v_evidence_tuple.object_id)), 'result', true) END);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('project' || ':' || p_object_id),
        'relation', 'can_read',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', true,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    ELSE
        v_node_count := v_node_count + 1;
        v_attempts := v_attempts || jsonb_build_array(jsonb_build_object('type', 'direct', 'label', 'no direct grant', 'result', false));
    END IF;
    -- TTU / parent-relation attempts
    FOR v_parent_link IN
        SELECT link.subject_type AS parent_type, link.subject_id AS parent_id
    FROM melange_tuples AS link
    WHERE (link.object_type = 'project' AND link.relation IN ('organization') AND link.object_id = p_object_id AND link.subject_type IN ('organization'))
    LOOP
        v_child_trace := COALESCE("public"."explain_permission_internal"(p_subject_type, p_subject_id, 'can_read', v_parent_link.parent_type, v_parent_link.parent_id, p_visited || ARRAY[v_key], p_max_nodes), '{}'::jsonb);
        v_node_count := v_node_count + COALESCE((v_child_trace->>'node_count')::INTEGER, 0);
        IF v_node_count >= v_max_nodes THEN
        v_root := jsonb_build_object('type', 'truncated');
        v_node_count := v_node_count + 1;
        v_truncated := TRUE;
        RETURN jsonb_build_object(
        'object', ('project' || ':' || p_object_id),
        'relation', 'can_read',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
        IF COALESCE((v_child_trace->>'result')::boolean, FALSE) THEN
        v_root := jsonb_build_object('type', 'ttu', 'label', ('via organization → ' || v_parent_link.parent_type || ':' || v_parent_link.parent_id || ' ⇒ can_read'), 'children', jsonb_build_array(v_child_trace->'root'), 'result', true);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('project' || ':' || p_object_id),
        'relation', 'can_read',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', true,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    ELSE
        v_node_count := v_node_count + 1;
        v_attempts := v_attempts || jsonb_build_array(jsonb_build_object('type', 'ttu', 'label', ('via organization → ' || v_parent_link.parent_type || ':' || v_parent_link.parent_id || ' ⇒ can_read'), 'children', jsonb_build_array(v_child_trace->'root'), 'result', false));
    END IF;
    END LOOP;
    IF v_node_count >= v_max_nodes THEN
        v_truncated := TRUE;
    END IF;
    -- All recorded attempts failed
    v_root := jsonb_build_object('type', 'union', 'children', v_attempts, 'result', false);
    v_node_count := v_node_count + 1;
    RETURN jsonb_build_object(
        'object', ('project' || ':' || p_object_id),
        'relation', 'can_read',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
END;
$$ LANGUAGE plpgsql STABLE COST 1000
SET search_path = 'public';


-- Generated explain function for agent.can_read
-- Features: Recursive
CREATE OR REPLACE FUNCTION "public"."explain_agent_can_read"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[],
    p_max_nodes INTEGER DEFAULT NULL
) RETURNS JSONB AS $$
DECLARE
    v_key TEXT := 'agent:' || p_object_id || ':can_read';
    v_node_count INTEGER := 0;
    v_evidence_tuple RECORD;
    v_root JSONB;
    v_attempts JSONB := '[]'::JSONB;
    v_userset_check INTEGER := 0;
    v_max_nodes INTEGER := COALESCE(p_max_nodes, current_setting('melange.max_explain_nodes', true)::INTEGER, 100);
    v_truncated BOOLEAN := FALSE;
    v_child_trace JSONB;
    v_parent_link RECORD;
BEGIN
    -- Cycle detection
    IF v_key = ANY(p_visited) THEN
        v_root := jsonb_build_object('type', 'cycle', 'label', v_key);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('agent' || ':' || p_object_id),
        'relation', 'can_read',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
    IF array_length(p_visited, 1) >= 25 THEN
        RAISE EXCEPTION 'resolution too complex' USING ERRCODE = 'M2002';
    END IF;
    IF v_node_count >= v_max_nodes THEN
        v_root := jsonb_build_object('type', 'truncated');
        v_node_count := v_node_count + 1;
        v_truncated := TRUE;
        RETURN jsonb_build_object(
        'object', ('agent' || ':' || p_object_id),
        'relation', 'can_read',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
    -- Userset subject handling (subject is itself a userset reference)
    IF position('#' in p_subject_id) > 0 THEN
        -- Case 1: self-referential userset (subject's userset resolves to this object)
        IF (p_subject_type = 'agent' AND split_part(p_subject_id, '#', 1) = p_object_id) THEN
        SELECT INTO v_userset_check 1
    WHERE substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('can_read')
    LIMIT 1;
        IF v_userset_check = 1 THEN
        v_root := jsonb_build_object('type', 'userset', 'label', 'self-referential userset matches relation closure', 'result', true);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('agent' || ':' || p_object_id),
        'relation', 'can_read',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', true,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
    END IF;
    END IF;
    -- TTU / parent-relation attempts
    FOR v_parent_link IN
        SELECT link.subject_type AS parent_type, link.subject_id AS parent_id
    FROM melange_tuples AS link
    WHERE (link.object_type = 'agent' AND link.relation IN ('project') AND link.object_id = p_object_id AND link.subject_type IN ('project'))
    LOOP
        v_child_trace := COALESCE("public"."explain_permission_internal"(p_subject_type, p_subject_id, 'can_read', v_parent_link.parent_type, v_parent_link.parent_id, p_visited || ARRAY[v_key], p_max_nodes), '{}'::jsonb);
        v_node_count := v_node_count + COALESCE((v_child_trace->>'node_count')::INTEGER, 0);
        IF v_node_count >= v_max_nodes THEN
        v_root := jsonb_build_object('type', 'truncated');
        v_node_count := v_node_count + 1;
        v_truncated := TRUE;
        RETURN jsonb_build_object(
        'object', ('agent' || ':' || p_object_id),
        'relation', 'can_read',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
        IF COALESCE((v_child_trace->>'result')::boolean, FALSE) THEN
        v_root := jsonb_build_object('type', 'ttu', 'label', ('via project → ' || v_parent_link.parent_type || ':' || v_parent_link.parent_id || ' ⇒ can_read'), 'children', jsonb_build_array(v_child_trace->'root'), 'result', true);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('agent' || ':' || p_object_id),
        'relation', 'can_read',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', true,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    ELSE
        v_node_count := v_node_count + 1;
        v_attempts := v_attempts || jsonb_build_array(jsonb_build_object('type', 'ttu', 'label', ('via project → ' || v_parent_link.parent_type || ':' || v_parent_link.parent_id || ' ⇒ can_read'), 'children', jsonb_build_array(v_child_trace->'root'), 'result', false));
    END IF;
    END LOOP;
    IF v_node_count >= v_max_nodes THEN
        v_truncated := TRUE;
    END IF;
    -- All recorded attempts failed
    v_root := jsonb_build_object('type', 'union', 'children', v_attempts, 'result', false);
    v_node_count := v_node_count + 1;
    RETURN jsonb_build_object(
        'object', ('agent' || ':' || p_object_id),
        'relation', 'can_read',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
END;
$$ LANGUAGE plpgsql STABLE COST 1000
SET search_path = 'public';


-- Generated explain function for repository.can_read
-- Features: Implied+Wildcard+Recursive
CREATE OR REPLACE FUNCTION "public"."explain_repository_can_read"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[],
    p_max_nodes INTEGER DEFAULT NULL
) RETURNS JSONB AS $$
DECLARE
    v_key TEXT := 'repository:' || p_object_id || ':can_read';
    v_node_count INTEGER := 0;
    v_evidence_tuple RECORD;
    v_root JSONB;
    v_attempts JSONB := '[]'::JSONB;
    v_userset_check INTEGER := 0;
    v_max_nodes INTEGER := COALESCE(p_max_nodes, current_setting('melange.max_explain_nodes', true)::INTEGER, 100);
    v_truncated BOOLEAN := FALSE;
    v_child_trace JSONB;
    v_parent_link RECORD;
BEGIN
    -- Cycle detection
    IF v_key = ANY(p_visited) THEN
        v_root := jsonb_build_object('type', 'cycle', 'label', v_key);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('repository' || ':' || p_object_id),
        'relation', 'can_read',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
    IF array_length(p_visited, 1) >= 25 THEN
        RAISE EXCEPTION 'resolution too complex' USING ERRCODE = 'M2002';
    END IF;
    IF v_node_count >= v_max_nodes THEN
        v_root := jsonb_build_object('type', 'truncated');
        v_node_count := v_node_count + 1;
        v_truncated := TRUE;
        RETURN jsonb_build_object(
        'object', ('repository' || ':' || p_object_id),
        'relation', 'can_read',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
    -- Userset subject handling (subject is itself a userset reference)
    IF position('#' in p_subject_id) > 0 THEN
        -- Case 1: self-referential userset (subject's userset resolves to this object)
        IF (p_subject_type = 'repository' AND split_part(p_subject_id, '#', 1) = p_object_id) THEN
        SELECT INTO v_userset_check 1
    WHERE substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('can_read', 'public')
    LIMIT 1;
        IF v_userset_check = 1 THEN
        v_root := jsonb_build_object('type', 'userset', 'label', 'self-referential userset matches relation closure', 'result', true);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('repository' || ':' || p_object_id),
        'relation', 'can_read',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', true,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
    END IF;
    END IF;
    -- Direct/Implied grant attempt
    SELECT INTO v_evidence_tuple t.subject_type, t.subject_id, t.relation, t.object_type, t.object_id
    FROM melange_tuples AS t
    WHERE (t.object_type = 'repository' AND t.relation IN ('can_read', 'public') AND t.object_id = p_object_id AND t.subject_type = p_subject_type AND (t.subject_id = p_subject_id OR t.subject_id = '*') AND t.subject_type IN ('user'))
    LIMIT 1;
    IF FOUND THEN
        v_root := (CASE WHEN v_evidence_tuple.subject_id = '*' THEN jsonb_build_object('type', 'wildcard', 'users', jsonb_build_array(jsonb_build_object('type', v_evidence_tuple.subject_type, 'id', '*')), 'result', true) ELSE jsonb_build_object('type', 'direct', 'label', ('direct or implied grant via ' || v_evidence_tuple.relation), 'evidence', jsonb_build_array(jsonb_build_object('subject_type', v_evidence_tuple.subject_type, 'subject_id', v_evidence_tuple.subject_id, 'relation', v_evidence_tuple.relation, 'object_type', v_evidence_tuple.object_type, 'object_id', v_evidence_tuple.object_id)), 'result', true) END);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('repository' || ':' || p_object_id),
        'relation', 'can_read',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', true,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    ELSE
        v_node_count := v_node_count + 1;
        v_attempts := v_attempts || jsonb_build_array(jsonb_build_object('type', 'direct', 'label', 'no direct grant', 'result', false));
    END IF;
    -- TTU / parent-relation attempts
    FOR v_parent_link IN
        SELECT link.subject_type AS parent_type, link.subject_id AS parent_id
    FROM melange_tuples AS link
    WHERE (link.object_type = 'repository' AND link.relation IN ('project') AND link.object_id = p_object_id AND link.subject_type IN ('project'))
    LOOP
        v_child_trace := COALESCE("public"."explain_permission_internal"(p_subject_type, p_subject_id, 'can_read', v_parent_link.parent_type, v_parent_link.parent_id, p_visited || ARRAY[v_key], p_max_nodes), '{}'::jsonb);
        v_node_count := v_node_count + COALESCE((v_child_trace->>'node_count')::INTEGER, 0);
        IF v_node_count >= v_max_nodes THEN
        v_root := jsonb_build_object('type', 'truncated');
        v_node_count := v_node_count + 1;
        v_truncated := TRUE;
        RETURN jsonb_build_object(
        'object', ('repository' || ':' || p_object_id),
        'relation', 'can_read',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
        IF COALESCE((v_child_trace->>'result')::boolean, FALSE) THEN
        v_root := jsonb_build_object('type', 'ttu', 'label', ('via project → ' || v_parent_link.parent_type || ':' || v_parent_link.parent_id || ' ⇒ can_read'), 'children', jsonb_build_array(v_child_trace->'root'), 'result', true);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('repository' || ':' || p_object_id),
        'relation', 'can_read',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', true,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    ELSE
        v_node_count := v_node_count + 1;
        v_attempts := v_attempts || jsonb_build_array(jsonb_build_object('type', 'ttu', 'label', ('via project → ' || v_parent_link.parent_type || ':' || v_parent_link.parent_id || ' ⇒ can_read'), 'children', jsonb_build_array(v_child_trace->'root'), 'result', false));
    END IF;
    END LOOP;
    IF v_node_count >= v_max_nodes THEN
        v_truncated := TRUE;
    END IF;
    -- All recorded attempts failed
    v_root := jsonb_build_object('type', 'union', 'children', v_attempts, 'result', false);
    v_node_count := v_node_count + 1;
    RETURN jsonb_build_object(
        'object', ('repository' || ':' || p_object_id),
        'relation', 'can_read',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
END;
$$ LANGUAGE plpgsql STABLE COST 1000
SET search_path = 'public';


-- Generated explain function for run.can_read
-- Features: Recursive
CREATE OR REPLACE FUNCTION "public"."explain_run_can_read"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[],
    p_max_nodes INTEGER DEFAULT NULL
) RETURNS JSONB AS $$
DECLARE
    v_key TEXT := 'run:' || p_object_id || ':can_read';
    v_node_count INTEGER := 0;
    v_evidence_tuple RECORD;
    v_root JSONB;
    v_attempts JSONB := '[]'::JSONB;
    v_userset_check INTEGER := 0;
    v_max_nodes INTEGER := COALESCE(p_max_nodes, current_setting('melange.max_explain_nodes', true)::INTEGER, 100);
    v_truncated BOOLEAN := FALSE;
    v_child_trace JSONB;
    v_parent_link RECORD;
BEGIN
    -- Cycle detection
    IF v_key = ANY(p_visited) THEN
        v_root := jsonb_build_object('type', 'cycle', 'label', v_key);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('run' || ':' || p_object_id),
        'relation', 'can_read',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
    IF array_length(p_visited, 1) >= 25 THEN
        RAISE EXCEPTION 'resolution too complex' USING ERRCODE = 'M2002';
    END IF;
    IF v_node_count >= v_max_nodes THEN
        v_root := jsonb_build_object('type', 'truncated');
        v_node_count := v_node_count + 1;
        v_truncated := TRUE;
        RETURN jsonb_build_object(
        'object', ('run' || ':' || p_object_id),
        'relation', 'can_read',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
    -- Userset subject handling (subject is itself a userset reference)
    IF position('#' in p_subject_id) > 0 THEN
        -- Case 1: self-referential userset (subject's userset resolves to this object)
        IF (p_subject_type = 'run' AND split_part(p_subject_id, '#', 1) = p_object_id) THEN
        SELECT INTO v_userset_check 1
    WHERE substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('can_read')
    LIMIT 1;
        IF v_userset_check = 1 THEN
        v_root := jsonb_build_object('type', 'userset', 'label', 'self-referential userset matches relation closure', 'result', true);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('run' || ':' || p_object_id),
        'relation', 'can_read',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', true,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
    END IF;
    END IF;
    -- TTU / parent-relation attempts
    FOR v_parent_link IN
        SELECT link.subject_type AS parent_type, link.subject_id AS parent_id
    FROM melange_tuples AS link
    WHERE (link.object_type = 'run' AND link.relation IN ('agent') AND link.object_id = p_object_id AND link.subject_type IN ('agent'))
    LOOP
        v_child_trace := COALESCE("public"."explain_permission_internal"(p_subject_type, p_subject_id, 'can_read', v_parent_link.parent_type, v_parent_link.parent_id, p_visited || ARRAY[v_key], p_max_nodes), '{}'::jsonb);
        v_node_count := v_node_count + COALESCE((v_child_trace->>'node_count')::INTEGER, 0);
        IF v_node_count >= v_max_nodes THEN
        v_root := jsonb_build_object('type', 'truncated');
        v_node_count := v_node_count + 1;
        v_truncated := TRUE;
        RETURN jsonb_build_object(
        'object', ('run' || ':' || p_object_id),
        'relation', 'can_read',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
        IF COALESCE((v_child_trace->>'result')::boolean, FALSE) THEN
        v_root := jsonb_build_object('type', 'ttu', 'label', ('via agent → ' || v_parent_link.parent_type || ':' || v_parent_link.parent_id || ' ⇒ can_read'), 'children', jsonb_build_array(v_child_trace->'root'), 'result', true);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('run' || ':' || p_object_id),
        'relation', 'can_read',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', true,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    ELSE
        v_node_count := v_node_count + 1;
        v_attempts := v_attempts || jsonb_build_array(jsonb_build_object('type', 'ttu', 'label', ('via agent → ' || v_parent_link.parent_type || ':' || v_parent_link.parent_id || ' ⇒ can_read'), 'children', jsonb_build_array(v_child_trace->'root'), 'result', false));
    END IF;
    END LOOP;
    IF v_node_count >= v_max_nodes THEN
        v_truncated := TRUE;
    END IF;
    -- All recorded attempts failed
    v_root := jsonb_build_object('type', 'union', 'children', v_attempts, 'result', false);
    v_node_count := v_node_count + 1;
    RETURN jsonb_build_object(
        'object', ('run' || ':' || p_object_id),
        'relation', 'can_read',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
END;
$$ LANGUAGE plpgsql STABLE COST 1000
SET search_path = 'public';


-- Generated explain function for state_volume.can_read
-- Features: Recursive
CREATE OR REPLACE FUNCTION "public"."explain_state_volume_can_read"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[],
    p_max_nodes INTEGER DEFAULT NULL
) RETURNS JSONB AS $$
DECLARE
    v_key TEXT := 'state_volume:' || p_object_id || ':can_read';
    v_node_count INTEGER := 0;
    v_evidence_tuple RECORD;
    v_root JSONB;
    v_attempts JSONB := '[]'::JSONB;
    v_userset_check INTEGER := 0;
    v_max_nodes INTEGER := COALESCE(p_max_nodes, current_setting('melange.max_explain_nodes', true)::INTEGER, 100);
    v_truncated BOOLEAN := FALSE;
    v_child_trace JSONB;
    v_parent_link RECORD;
BEGIN
    -- Cycle detection
    IF v_key = ANY(p_visited) THEN
        v_root := jsonb_build_object('type', 'cycle', 'label', v_key);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('state_volume' || ':' || p_object_id),
        'relation', 'can_read',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
    IF array_length(p_visited, 1) >= 25 THEN
        RAISE EXCEPTION 'resolution too complex' USING ERRCODE = 'M2002';
    END IF;
    IF v_node_count >= v_max_nodes THEN
        v_root := jsonb_build_object('type', 'truncated');
        v_node_count := v_node_count + 1;
        v_truncated := TRUE;
        RETURN jsonb_build_object(
        'object', ('state_volume' || ':' || p_object_id),
        'relation', 'can_read',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
    -- Userset subject handling (subject is itself a userset reference)
    IF position('#' in p_subject_id) > 0 THEN
        -- Case 1: self-referential userset (subject's userset resolves to this object)
        IF (p_subject_type = 'state_volume' AND split_part(p_subject_id, '#', 1) = p_object_id) THEN
        SELECT INTO v_userset_check 1
    WHERE substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('can_read')
    LIMIT 1;
        IF v_userset_check = 1 THEN
        v_root := jsonb_build_object('type', 'userset', 'label', 'self-referential userset matches relation closure', 'result', true);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('state_volume' || ':' || p_object_id),
        'relation', 'can_read',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', true,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
    END IF;
    END IF;
    -- TTU / parent-relation attempts
    FOR v_parent_link IN
        SELECT link.subject_type AS parent_type, link.subject_id AS parent_id
    FROM melange_tuples AS link
    WHERE (link.object_type = 'state_volume' AND link.relation IN ('agent') AND link.object_id = p_object_id AND link.subject_type IN ('agent'))
    LOOP
        v_child_trace := COALESCE("public"."explain_permission_internal"(p_subject_type, p_subject_id, 'can_read', v_parent_link.parent_type, v_parent_link.parent_id, p_visited || ARRAY[v_key], p_max_nodes), '{}'::jsonb);
        v_node_count := v_node_count + COALESCE((v_child_trace->>'node_count')::INTEGER, 0);
        IF v_node_count >= v_max_nodes THEN
        v_root := jsonb_build_object('type', 'truncated');
        v_node_count := v_node_count + 1;
        v_truncated := TRUE;
        RETURN jsonb_build_object(
        'object', ('state_volume' || ':' || p_object_id),
        'relation', 'can_read',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    END IF;
        IF COALESCE((v_child_trace->>'result')::boolean, FALSE) THEN
        v_root := jsonb_build_object('type', 'ttu', 'label', ('via agent → ' || v_parent_link.parent_type || ':' || v_parent_link.parent_id || ' ⇒ can_read'), 'children', jsonb_build_array(v_child_trace->'root'), 'result', true);
        v_node_count := v_node_count + 1;
        RETURN jsonb_build_object(
        'object', ('state_volume' || ':' || p_object_id),
        'relation', 'can_read',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', true,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
    ELSE
        v_node_count := v_node_count + 1;
        v_attempts := v_attempts || jsonb_build_array(jsonb_build_object('type', 'ttu', 'label', ('via agent → ' || v_parent_link.parent_type || ':' || v_parent_link.parent_id || ' ⇒ can_read'), 'children', jsonb_build_array(v_child_trace->'root'), 'result', false));
    END IF;
    END LOOP;
    IF v_node_count >= v_max_nodes THEN
        v_truncated := TRUE;
    END IF;
    -- All recorded attempts failed
    v_root := jsonb_build_object('type', 'union', 'children', v_attempts, 'result', false);
    v_node_count := v_node_count + 1;
    RETURN jsonb_build_object(
        'object', ('state_volume' || ':' || p_object_id),
        'relation', 'can_read',
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', v_root,
        'truncated', v_truncated,
        'node_count', v_node_count);
END;
$$ LANGUAGE plpgsql STABLE COST 1000
SET search_path = 'public';


-- ============================================================
-- Expand Functions (31 functions)
-- ============================================================

-- Generated expand function for agent.project
-- Returns OpenFGA-shaped UsersetTree JSONB. Shallow by default — computed
-- rewrites surface as Leaf.Computed pointers; callers chase them with
-- follow-up Expand calls or use Checker.ExpandRecursive.
CREATE OR REPLACE FUNCTION "public"."expand_agent_project"(
    p_object_id TEXT,
    p_subject_type TEXT DEFAULT NULL,
    p_max_leaf INTEGER DEFAULT NULL
) RETURNS JSONB AS $$
BEGIN
    RETURN jsonb_build_object('root', jsonb_build_object('name', ('agent' || ':' || p_object_id || '#project')) || jsonb_build_object('leaf', jsonb_build_object('users', (jsonb_build_object('users', COALESCE((SELECT jsonb_agg(u) FROM (SELECT subject_type || ':' || subject_id AS u FROM "public"."melange_tuples" WHERE object_type = 'agent' AND object_id = p_object_id AND relation = 'project' AND subject_type IN ('project') AND (p_subject_type IS NULL OR subject_type = p_subject_type) ORDER BY subject_type, subject_id LIMIT p_max_leaf) capped), '[]'::jsonb)) || CASE WHEN (p_max_leaf IS NOT NULL AND EXISTS (SELECT 1 FROM "public"."melange_tuples" WHERE object_type = 'agent' AND object_id = p_object_id AND relation = 'project' AND subject_type IN ('project') AND (p_subject_type IS NULL OR subject_type = p_subject_type) OFFSET p_max_leaf)) THEN jsonb_build_object('users_truncated', true) ELSE '{}'::jsonb END))));
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';


-- Generated expand function for organization.owner
-- Returns OpenFGA-shaped UsersetTree JSONB. Shallow by default — computed
-- rewrites surface as Leaf.Computed pointers; callers chase them with
-- follow-up Expand calls or use Checker.ExpandRecursive.
CREATE OR REPLACE FUNCTION "public"."expand_organization_owner"(
    p_object_id TEXT,
    p_subject_type TEXT DEFAULT NULL,
    p_max_leaf INTEGER DEFAULT NULL
) RETURNS JSONB AS $$
BEGIN
    RETURN jsonb_build_object('root', jsonb_build_object('name', ('organization' || ':' || p_object_id || '#owner')) || jsonb_build_object('leaf', jsonb_build_object('users', (jsonb_build_object('users', COALESCE((SELECT jsonb_agg(u) FROM (SELECT subject_type || ':' || subject_id AS u FROM "public"."melange_tuples" WHERE object_type = 'organization' AND object_id = p_object_id AND relation = 'owner' AND subject_type IN ('user') AND (p_subject_type IS NULL OR subject_type = p_subject_type) ORDER BY subject_type, subject_id LIMIT p_max_leaf) capped), '[]'::jsonb)) || CASE WHEN (p_max_leaf IS NOT NULL AND EXISTS (SELECT 1 FROM "public"."melange_tuples" WHERE object_type = 'organization' AND object_id = p_object_id AND relation = 'owner' AND subject_type IN ('user') AND (p_subject_type IS NULL OR subject_type = p_subject_type) OFFSET p_max_leaf)) THEN jsonb_build_object('users_truncated', true) ELSE '{}'::jsonb END))));
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';


-- Generated expand function for project.maintainer
-- Returns OpenFGA-shaped UsersetTree JSONB. Shallow by default — computed
-- rewrites surface as Leaf.Computed pointers; callers chase them with
-- follow-up Expand calls or use Checker.ExpandRecursive.
CREATE OR REPLACE FUNCTION "public"."expand_project_maintainer"(
    p_object_id TEXT,
    p_subject_type TEXT DEFAULT NULL,
    p_max_leaf INTEGER DEFAULT NULL
) RETURNS JSONB AS $$
BEGIN
    RETURN jsonb_build_object('root', jsonb_build_object('name', ('project' || ':' || p_object_id || '#maintainer')) || jsonb_build_object('leaf', jsonb_build_object('users', (jsonb_build_object('users', COALESCE((SELECT jsonb_agg(u) FROM (SELECT subject_type || ':' || subject_id AS u FROM "public"."melange_tuples" WHERE object_type = 'project' AND object_id = p_object_id AND relation = 'maintainer' AND subject_type IN ('user') AND (p_subject_type IS NULL OR subject_type = p_subject_type) ORDER BY subject_type, subject_id LIMIT p_max_leaf) capped), '[]'::jsonb)) || CASE WHEN (p_max_leaf IS NOT NULL AND EXISTS (SELECT 1 FROM "public"."melange_tuples" WHERE object_type = 'project' AND object_id = p_object_id AND relation = 'maintainer' AND subject_type IN ('user') AND (p_subject_type IS NULL OR subject_type = p_subject_type) OFFSET p_max_leaf)) THEN jsonb_build_object('users_truncated', true) ELSE '{}'::jsonb END))));
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';


-- Generated expand function for project.organization
-- Returns OpenFGA-shaped UsersetTree JSONB. Shallow by default — computed
-- rewrites surface as Leaf.Computed pointers; callers chase them with
-- follow-up Expand calls or use Checker.ExpandRecursive.
CREATE OR REPLACE FUNCTION "public"."expand_project_organization"(
    p_object_id TEXT,
    p_subject_type TEXT DEFAULT NULL,
    p_max_leaf INTEGER DEFAULT NULL
) RETURNS JSONB AS $$
BEGIN
    RETURN jsonb_build_object('root', jsonb_build_object('name', ('project' || ':' || p_object_id || '#organization')) || jsonb_build_object('leaf', jsonb_build_object('users', (jsonb_build_object('users', COALESCE((SELECT jsonb_agg(u) FROM (SELECT subject_type || ':' || subject_id AS u FROM "public"."melange_tuples" WHERE object_type = 'project' AND object_id = p_object_id AND relation = 'organization' AND subject_type IN ('organization') AND (p_subject_type IS NULL OR subject_type = p_subject_type) ORDER BY subject_type, subject_id LIMIT p_max_leaf) capped), '[]'::jsonb)) || CASE WHEN (p_max_leaf IS NOT NULL AND EXISTS (SELECT 1 FROM "public"."melange_tuples" WHERE object_type = 'project' AND object_id = p_object_id AND relation = 'organization' AND subject_type IN ('organization') AND (p_subject_type IS NULL OR subject_type = p_subject_type) OFFSET p_max_leaf)) THEN jsonb_build_object('users_truncated', true) ELSE '{}'::jsonb END))));
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';


-- Generated expand function for repository.project
-- Returns OpenFGA-shaped UsersetTree JSONB. Shallow by default — computed
-- rewrites surface as Leaf.Computed pointers; callers chase them with
-- follow-up Expand calls or use Checker.ExpandRecursive.
CREATE OR REPLACE FUNCTION "public"."expand_repository_project"(
    p_object_id TEXT,
    p_subject_type TEXT DEFAULT NULL,
    p_max_leaf INTEGER DEFAULT NULL
) RETURNS JSONB AS $$
BEGIN
    RETURN jsonb_build_object('root', jsonb_build_object('name', ('repository' || ':' || p_object_id || '#project')) || jsonb_build_object('leaf', jsonb_build_object('users', (jsonb_build_object('users', COALESCE((SELECT jsonb_agg(u) FROM (SELECT subject_type || ':' || subject_id AS u FROM "public"."melange_tuples" WHERE object_type = 'repository' AND object_id = p_object_id AND relation = 'project' AND subject_type IN ('project') AND (p_subject_type IS NULL OR subject_type = p_subject_type) ORDER BY subject_type, subject_id LIMIT p_max_leaf) capped), '[]'::jsonb)) || CASE WHEN (p_max_leaf IS NOT NULL AND EXISTS (SELECT 1 FROM "public"."melange_tuples" WHERE object_type = 'repository' AND object_id = p_object_id AND relation = 'project' AND subject_type IN ('project') AND (p_subject_type IS NULL OR subject_type = p_subject_type) OFFSET p_max_leaf)) THEN jsonb_build_object('users_truncated', true) ELSE '{}'::jsonb END))));
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';


-- Generated expand function for repository.public
-- Returns OpenFGA-shaped UsersetTree JSONB. Shallow by default — computed
-- rewrites surface as Leaf.Computed pointers; callers chase them with
-- follow-up Expand calls or use Checker.ExpandRecursive.
CREATE OR REPLACE FUNCTION "public"."expand_repository_public"(
    p_object_id TEXT,
    p_subject_type TEXT DEFAULT NULL,
    p_max_leaf INTEGER DEFAULT NULL
) RETURNS JSONB AS $$
BEGIN
    RETURN jsonb_build_object('root', jsonb_build_object('name', ('repository' || ':' || p_object_id || '#public')) || jsonb_build_object('leaf', jsonb_build_object('users', (jsonb_build_object('users', COALESCE((SELECT jsonb_agg(u) FROM (SELECT subject_type || ':' || subject_id AS u FROM "public"."melange_tuples" WHERE object_type = 'repository' AND object_id = p_object_id AND relation = 'public' AND subject_type IN ('user') AND (p_subject_type IS NULL OR subject_type = p_subject_type) ORDER BY subject_type, subject_id LIMIT p_max_leaf) capped), '[]'::jsonb)) || CASE WHEN (p_max_leaf IS NOT NULL AND EXISTS (SELECT 1 FROM "public"."melange_tuples" WHERE object_type = 'repository' AND object_id = p_object_id AND relation = 'public' AND subject_type IN ('user') AND (p_subject_type IS NULL OR subject_type = p_subject_type) OFFSET p_max_leaf)) THEN jsonb_build_object('users_truncated', true) ELSE '{}'::jsonb END))));
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';


-- Generated expand function for run.agent
-- Returns OpenFGA-shaped UsersetTree JSONB. Shallow by default — computed
-- rewrites surface as Leaf.Computed pointers; callers chase them with
-- follow-up Expand calls or use Checker.ExpandRecursive.
CREATE OR REPLACE FUNCTION "public"."expand_run_agent"(
    p_object_id TEXT,
    p_subject_type TEXT DEFAULT NULL,
    p_max_leaf INTEGER DEFAULT NULL
) RETURNS JSONB AS $$
BEGIN
    RETURN jsonb_build_object('root', jsonb_build_object('name', ('run' || ':' || p_object_id || '#agent')) || jsonb_build_object('leaf', jsonb_build_object('users', (jsonb_build_object('users', COALESCE((SELECT jsonb_agg(u) FROM (SELECT subject_type || ':' || subject_id AS u FROM "public"."melange_tuples" WHERE object_type = 'run' AND object_id = p_object_id AND relation = 'agent' AND subject_type IN ('agent') AND (p_subject_type IS NULL OR subject_type = p_subject_type) ORDER BY subject_type, subject_id LIMIT p_max_leaf) capped), '[]'::jsonb)) || CASE WHEN (p_max_leaf IS NOT NULL AND EXISTS (SELECT 1 FROM "public"."melange_tuples" WHERE object_type = 'run' AND object_id = p_object_id AND relation = 'agent' AND subject_type IN ('agent') AND (p_subject_type IS NULL OR subject_type = p_subject_type) OFFSET p_max_leaf)) THEN jsonb_build_object('users_truncated', true) ELSE '{}'::jsonb END))));
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';


-- Generated expand function for state_volume.agent
-- Returns OpenFGA-shaped UsersetTree JSONB. Shallow by default — computed
-- rewrites surface as Leaf.Computed pointers; callers chase them with
-- follow-up Expand calls or use Checker.ExpandRecursive.
CREATE OR REPLACE FUNCTION "public"."expand_state_volume_agent"(
    p_object_id TEXT,
    p_subject_type TEXT DEFAULT NULL,
    p_max_leaf INTEGER DEFAULT NULL
) RETURNS JSONB AS $$
BEGIN
    RETURN jsonb_build_object('root', jsonb_build_object('name', ('state_volume' || ':' || p_object_id || '#agent')) || jsonb_build_object('leaf', jsonb_build_object('users', (jsonb_build_object('users', COALESCE((SELECT jsonb_agg(u) FROM (SELECT subject_type || ':' || subject_id AS u FROM "public"."melange_tuples" WHERE object_type = 'state_volume' AND object_id = p_object_id AND relation = 'agent' AND subject_type IN ('agent') AND (p_subject_type IS NULL OR subject_type = p_subject_type) ORDER BY subject_type, subject_id LIMIT p_max_leaf) capped), '[]'::jsonb)) || CASE WHEN (p_max_leaf IS NOT NULL AND EXISTS (SELECT 1 FROM "public"."melange_tuples" WHERE object_type = 'state_volume' AND object_id = p_object_id AND relation = 'agent' AND subject_type IN ('agent') AND (p_subject_type IS NULL OR subject_type = p_subject_type) OFFSET p_max_leaf)) THEN jsonb_build_object('users_truncated', true) ELSE '{}'::jsonb END))));
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';


-- Generated expand function for organization.admin
-- Returns OpenFGA-shaped UsersetTree JSONB. Shallow by default — computed
-- rewrites surface as Leaf.Computed pointers; callers chase them with
-- follow-up Expand calls or use Checker.ExpandRecursive.
CREATE OR REPLACE FUNCTION "public"."expand_organization_admin"(
    p_object_id TEXT,
    p_subject_type TEXT DEFAULT NULL,
    p_max_leaf INTEGER DEFAULT NULL
) RETURNS JSONB AS $$
BEGIN
    RETURN jsonb_build_object('root', jsonb_build_object('name', ('organization' || ':' || p_object_id || '#admin')) || jsonb_build_object('union', jsonb_build_object('nodes', jsonb_build_array(jsonb_build_object('name', ('organization' || ':' || p_object_id || '#admin')) || jsonb_build_object('leaf', jsonb_build_object('users', (jsonb_build_object('users', COALESCE((SELECT jsonb_agg(u) FROM (SELECT subject_type || ':' || subject_id AS u FROM "public"."melange_tuples" WHERE object_type = 'organization' AND object_id = p_object_id AND relation = 'admin' AND subject_type IN ('user') AND (p_subject_type IS NULL OR subject_type = p_subject_type) ORDER BY subject_type, subject_id LIMIT p_max_leaf) capped), '[]'::jsonb)) || CASE WHEN (p_max_leaf IS NOT NULL AND EXISTS (SELECT 1 FROM "public"."melange_tuples" WHERE object_type = 'organization' AND object_id = p_object_id AND relation = 'admin' AND subject_type IN ('user') AND (p_subject_type IS NULL OR subject_type = p_subject_type) OFFSET p_max_leaf)) THEN jsonb_build_object('users_truncated', true) ELSE '{}'::jsonb END))), jsonb_build_object('name', ('organization' || ':' || p_object_id || '#admin')) || jsonb_build_object('leaf', jsonb_build_object('computed', jsonb_build_object('userset', ('organization' || ':' || p_object_id || '#owner'))))))));
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';


-- Generated expand function for organization.can_delete
-- Returns OpenFGA-shaped UsersetTree JSONB. Shallow by default — computed
-- rewrites surface as Leaf.Computed pointers; callers chase them with
-- follow-up Expand calls or use Checker.ExpandRecursive.
CREATE OR REPLACE FUNCTION "public"."expand_organization_can_delete"(
    p_object_id TEXT,
    p_subject_type TEXT DEFAULT NULL,
    p_max_leaf INTEGER DEFAULT NULL
) RETURNS JSONB AS $$
BEGIN
    RETURN jsonb_build_object('root', jsonb_build_object('name', ('organization' || ':' || p_object_id || '#can_delete')) || jsonb_build_object('leaf', jsonb_build_object('computed', jsonb_build_object('userset', ('organization' || ':' || p_object_id || '#owner')))));
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';


-- Generated expand function for organization.can_manage
-- Returns OpenFGA-shaped UsersetTree JSONB. Shallow by default — computed
-- rewrites surface as Leaf.Computed pointers; callers chase them with
-- follow-up Expand calls or use Checker.ExpandRecursive.
CREATE OR REPLACE FUNCTION "public"."expand_organization_can_manage"(
    p_object_id TEXT,
    p_subject_type TEXT DEFAULT NULL,
    p_max_leaf INTEGER DEFAULT NULL
) RETURNS JSONB AS $$
BEGIN
    RETURN jsonb_build_object('root', jsonb_build_object('name', ('organization' || ':' || p_object_id || '#can_manage')) || jsonb_build_object('leaf', jsonb_build_object('computed', jsonb_build_object('userset', ('organization' || ':' || p_object_id || '#owner')))));
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';


-- Generated expand function for project.can_delete
-- Returns OpenFGA-shaped UsersetTree JSONB. Shallow by default — computed
-- rewrites surface as Leaf.Computed pointers; callers chase them with
-- follow-up Expand calls or use Checker.ExpandRecursive.
CREATE OR REPLACE FUNCTION "public"."expand_project_can_delete"(
    p_object_id TEXT,
    p_subject_type TEXT DEFAULT NULL,
    p_max_leaf INTEGER DEFAULT NULL
) RETURNS JSONB AS $$
BEGIN
    RETURN jsonb_build_object('root', jsonb_build_object('name', ('project' || ':' || p_object_id || '#can_delete')) || jsonb_build_object('leaf', jsonb_build_object('tuple_to_userset', jsonb_build_object('tupleset', ('project' || ':' || p_object_id || '#organization'), 'computed', COALESCE((SELECT jsonb_agg(jsonb_build_object('userset', subject_type || ':' || subject_id || '#owner') ORDER BY subject_type, subject_id) FROM "public"."melange_tuples" WHERE object_type = 'project' AND object_id = p_object_id AND relation = 'organization' AND subject_type IN ('organization')), '[]'::jsonb)))));
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';


-- Generated expand function for project.can_manage
-- Returns OpenFGA-shaped UsersetTree JSONB. Shallow by default — computed
-- rewrites surface as Leaf.Computed pointers; callers chase them with
-- follow-up Expand calls or use Checker.ExpandRecursive.
CREATE OR REPLACE FUNCTION "public"."expand_project_can_manage"(
    p_object_id TEXT,
    p_subject_type TEXT DEFAULT NULL,
    p_max_leaf INTEGER DEFAULT NULL
) RETURNS JSONB AS $$
BEGIN
    RETURN jsonb_build_object('root', jsonb_build_object('name', ('project' || ':' || p_object_id || '#can_manage')) || jsonb_build_object('leaf', jsonb_build_object('computed', jsonb_build_object('userset', ('project' || ':' || p_object_id || '#maintainer')))));
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';


-- Generated expand function for project.can_write
-- Returns OpenFGA-shaped UsersetTree JSONB. Shallow by default — computed
-- rewrites surface as Leaf.Computed pointers; callers chase them with
-- follow-up Expand calls or use Checker.ExpandRecursive.
CREATE OR REPLACE FUNCTION "public"."expand_project_can_write"(
    p_object_id TEXT,
    p_subject_type TEXT DEFAULT NULL,
    p_max_leaf INTEGER DEFAULT NULL
) RETURNS JSONB AS $$
BEGIN
    RETURN jsonb_build_object('root', jsonb_build_object('name', ('project' || ':' || p_object_id || '#can_write')) || jsonb_build_object('leaf', jsonb_build_object('computed', jsonb_build_object('userset', ('project' || ':' || p_object_id || '#maintainer')))));
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';


-- Generated expand function for organization.can_create_project
-- Returns OpenFGA-shaped UsersetTree JSONB. Shallow by default — computed
-- rewrites surface as Leaf.Computed pointers; callers chase them with
-- follow-up Expand calls or use Checker.ExpandRecursive.
CREATE OR REPLACE FUNCTION "public"."expand_organization_can_create_project"(
    p_object_id TEXT,
    p_subject_type TEXT DEFAULT NULL,
    p_max_leaf INTEGER DEFAULT NULL
) RETURNS JSONB AS $$
BEGIN
    RETURN jsonb_build_object('root', jsonb_build_object('name', ('organization' || ':' || p_object_id || '#can_create_project')) || jsonb_build_object('leaf', jsonb_build_object('computed', jsonb_build_object('userset', ('organization' || ':' || p_object_id || '#admin')))));
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';


-- Generated expand function for organization.can_manage_members
-- Returns OpenFGA-shaped UsersetTree JSONB. Shallow by default — computed
-- rewrites surface as Leaf.Computed pointers; callers chase them with
-- follow-up Expand calls or use Checker.ExpandRecursive.
CREATE OR REPLACE FUNCTION "public"."expand_organization_can_manage_members"(
    p_object_id TEXT,
    p_subject_type TEXT DEFAULT NULL,
    p_max_leaf INTEGER DEFAULT NULL
) RETURNS JSONB AS $$
BEGIN
    RETURN jsonb_build_object('root', jsonb_build_object('name', ('organization' || ':' || p_object_id || '#can_manage_members')) || jsonb_build_object('leaf', jsonb_build_object('computed', jsonb_build_object('userset', ('organization' || ':' || p_object_id || '#admin')))));
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';


-- Generated expand function for organization.member
-- Returns OpenFGA-shaped UsersetTree JSONB. Shallow by default — computed
-- rewrites surface as Leaf.Computed pointers; callers chase them with
-- follow-up Expand calls or use Checker.ExpandRecursive.
CREATE OR REPLACE FUNCTION "public"."expand_organization_member"(
    p_object_id TEXT,
    p_subject_type TEXT DEFAULT NULL,
    p_max_leaf INTEGER DEFAULT NULL
) RETURNS JSONB AS $$
BEGIN
    RETURN jsonb_build_object('root', jsonb_build_object('name', ('organization' || ':' || p_object_id || '#member')) || jsonb_build_object('union', jsonb_build_object('nodes', jsonb_build_array(jsonb_build_object('name', ('organization' || ':' || p_object_id || '#member')) || jsonb_build_object('leaf', jsonb_build_object('users', (jsonb_build_object('users', COALESCE((SELECT jsonb_agg(u) FROM (SELECT subject_type || ':' || subject_id AS u FROM "public"."melange_tuples" WHERE object_type = 'organization' AND object_id = p_object_id AND relation = 'member' AND subject_type IN ('user') AND (p_subject_type IS NULL OR subject_type = p_subject_type) ORDER BY subject_type, subject_id LIMIT p_max_leaf) capped), '[]'::jsonb)) || CASE WHEN (p_max_leaf IS NOT NULL AND EXISTS (SELECT 1 FROM "public"."melange_tuples" WHERE object_type = 'organization' AND object_id = p_object_id AND relation = 'member' AND subject_type IN ('user') AND (p_subject_type IS NULL OR subject_type = p_subject_type) OFFSET p_max_leaf)) THEN jsonb_build_object('users_truncated', true) ELSE '{}'::jsonb END))), jsonb_build_object('name', ('organization' || ':' || p_object_id || '#member')) || jsonb_build_object('leaf', jsonb_build_object('computed', jsonb_build_object('userset', ('organization' || ':' || p_object_id || '#admin'))))))));
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';


-- Generated expand function for repository.can_delete
-- Returns OpenFGA-shaped UsersetTree JSONB. Shallow by default — computed
-- rewrites surface as Leaf.Computed pointers; callers chase them with
-- follow-up Expand calls or use Checker.ExpandRecursive.
CREATE OR REPLACE FUNCTION "public"."expand_repository_can_delete"(
    p_object_id TEXT,
    p_subject_type TEXT DEFAULT NULL,
    p_max_leaf INTEGER DEFAULT NULL
) RETURNS JSONB AS $$
BEGIN
    RETURN jsonb_build_object('root', jsonb_build_object('name', ('repository' || ':' || p_object_id || '#can_delete')) || jsonb_build_object('leaf', jsonb_build_object('tuple_to_userset', jsonb_build_object('tupleset', ('repository' || ':' || p_object_id || '#project'), 'computed', COALESCE((SELECT jsonb_agg(jsonb_build_object('userset', subject_type || ':' || subject_id || '#can_delete') ORDER BY subject_type, subject_id) FROM "public"."melange_tuples" WHERE object_type = 'repository' AND object_id = p_object_id AND relation = 'project' AND subject_type IN ('project')), '[]'::jsonb)))));
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';


-- Generated expand function for agent.can_manage
-- Returns OpenFGA-shaped UsersetTree JSONB. Shallow by default — computed
-- rewrites surface as Leaf.Computed pointers; callers chase them with
-- follow-up Expand calls or use Checker.ExpandRecursive.
CREATE OR REPLACE FUNCTION "public"."expand_agent_can_manage"(
    p_object_id TEXT,
    p_subject_type TEXT DEFAULT NULL,
    p_max_leaf INTEGER DEFAULT NULL
) RETURNS JSONB AS $$
BEGIN
    RETURN jsonb_build_object('root', jsonb_build_object('name', ('agent' || ':' || p_object_id || '#can_manage')) || jsonb_build_object('leaf', jsonb_build_object('tuple_to_userset', jsonb_build_object('tupleset', ('agent' || ':' || p_object_id || '#project'), 'computed', COALESCE((SELECT jsonb_agg(jsonb_build_object('userset', subject_type || ':' || subject_id || '#can_manage') ORDER BY subject_type, subject_id) FROM "public"."melange_tuples" WHERE object_type = 'agent' AND object_id = p_object_id AND relation = 'project' AND subject_type IN ('project')), '[]'::jsonb)))));
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';


-- Generated expand function for agent.can_execute
-- Returns OpenFGA-shaped UsersetTree JSONB. Shallow by default — computed
-- rewrites surface as Leaf.Computed pointers; callers chase them with
-- follow-up Expand calls or use Checker.ExpandRecursive.
CREATE OR REPLACE FUNCTION "public"."expand_agent_can_execute"(
    p_object_id TEXT,
    p_subject_type TEXT DEFAULT NULL,
    p_max_leaf INTEGER DEFAULT NULL
) RETURNS JSONB AS $$
BEGIN
    RETURN jsonb_build_object('root', jsonb_build_object('name', ('agent' || ':' || p_object_id || '#can_execute')) || jsonb_build_object('leaf', jsonb_build_object('tuple_to_userset', jsonb_build_object('tupleset', ('agent' || ':' || p_object_id || '#project'), 'computed', COALESCE((SELECT jsonb_agg(jsonb_build_object('userset', subject_type || ':' || subject_id || '#can_write') ORDER BY subject_type, subject_id) FROM "public"."melange_tuples" WHERE object_type = 'agent' AND object_id = p_object_id AND relation = 'project' AND subject_type IN ('project')), '[]'::jsonb)))));
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';


-- Generated expand function for repository.can_write
-- Returns OpenFGA-shaped UsersetTree JSONB. Shallow by default — computed
-- rewrites surface as Leaf.Computed pointers; callers chase them with
-- follow-up Expand calls or use Checker.ExpandRecursive.
CREATE OR REPLACE FUNCTION "public"."expand_repository_can_write"(
    p_object_id TEXT,
    p_subject_type TEXT DEFAULT NULL,
    p_max_leaf INTEGER DEFAULT NULL
) RETURNS JSONB AS $$
BEGIN
    RETURN jsonb_build_object('root', jsonb_build_object('name', ('repository' || ':' || p_object_id || '#can_write')) || jsonb_build_object('leaf', jsonb_build_object('tuple_to_userset', jsonb_build_object('tupleset', ('repository' || ':' || p_object_id || '#project'), 'computed', COALESCE((SELECT jsonb_agg(jsonb_build_object('userset', subject_type || ':' || subject_id || '#can_write') ORDER BY subject_type, subject_id) FROM "public"."melange_tuples" WHERE object_type = 'repository' AND object_id = p_object_id AND relation = 'project' AND subject_type IN ('project')), '[]'::jsonb)))));
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';


-- Generated expand function for organization.can_read
-- Returns OpenFGA-shaped UsersetTree JSONB. Shallow by default — computed
-- rewrites surface as Leaf.Computed pointers; callers chase them with
-- follow-up Expand calls or use Checker.ExpandRecursive.
CREATE OR REPLACE FUNCTION "public"."expand_organization_can_read"(
    p_object_id TEXT,
    p_subject_type TEXT DEFAULT NULL,
    p_max_leaf INTEGER DEFAULT NULL
) RETURNS JSONB AS $$
BEGIN
    RETURN jsonb_build_object('root', jsonb_build_object('name', ('organization' || ':' || p_object_id || '#can_read')) || jsonb_build_object('leaf', jsonb_build_object('computed', jsonb_build_object('userset', ('organization' || ':' || p_object_id || '#member')))));
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';


-- Generated expand function for state_volume.can_manage
-- Returns OpenFGA-shaped UsersetTree JSONB. Shallow by default — computed
-- rewrites surface as Leaf.Computed pointers; callers chase them with
-- follow-up Expand calls or use Checker.ExpandRecursive.
CREATE OR REPLACE FUNCTION "public"."expand_state_volume_can_manage"(
    p_object_id TEXT,
    p_subject_type TEXT DEFAULT NULL,
    p_max_leaf INTEGER DEFAULT NULL
) RETURNS JSONB AS $$
BEGIN
    RETURN jsonb_build_object('root', jsonb_build_object('name', ('state_volume' || ':' || p_object_id || '#can_manage')) || jsonb_build_object('leaf', jsonb_build_object('tuple_to_userset', jsonb_build_object('tupleset', ('state_volume' || ':' || p_object_id || '#agent'), 'computed', COALESCE((SELECT jsonb_agg(jsonb_build_object('userset', subject_type || ':' || subject_id || '#can_manage') ORDER BY subject_type, subject_id) FROM "public"."melange_tuples" WHERE object_type = 'state_volume' AND object_id = p_object_id AND relation = 'agent' AND subject_type IN ('agent')), '[]'::jsonb)))));
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';


-- Generated expand function for state_volume.can_restore
-- Returns OpenFGA-shaped UsersetTree JSONB. Shallow by default — computed
-- rewrites surface as Leaf.Computed pointers; callers chase them with
-- follow-up Expand calls or use Checker.ExpandRecursive.
CREATE OR REPLACE FUNCTION "public"."expand_state_volume_can_restore"(
    p_object_id TEXT,
    p_subject_type TEXT DEFAULT NULL,
    p_max_leaf INTEGER DEFAULT NULL
) RETURNS JSONB AS $$
BEGIN
    RETURN jsonb_build_object('root', jsonb_build_object('name', ('state_volume' || ':' || p_object_id || '#can_restore')) || jsonb_build_object('leaf', jsonb_build_object('tuple_to_userset', jsonb_build_object('tupleset', ('state_volume' || ':' || p_object_id || '#agent'), 'computed', COALESCE((SELECT jsonb_agg(jsonb_build_object('userset', subject_type || ':' || subject_id || '#can_manage') ORDER BY subject_type, subject_id) FROM "public"."melange_tuples" WHERE object_type = 'state_volume' AND object_id = p_object_id AND relation = 'agent' AND subject_type IN ('agent')), '[]'::jsonb)))));
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';


-- Generated expand function for run.can_cancel
-- Returns OpenFGA-shaped UsersetTree JSONB. Shallow by default — computed
-- rewrites surface as Leaf.Computed pointers; callers chase them with
-- follow-up Expand calls or use Checker.ExpandRecursive.
CREATE OR REPLACE FUNCTION "public"."expand_run_can_cancel"(
    p_object_id TEXT,
    p_subject_type TEXT DEFAULT NULL,
    p_max_leaf INTEGER DEFAULT NULL
) RETURNS JSONB AS $$
BEGIN
    RETURN jsonb_build_object('root', jsonb_build_object('name', ('run' || ':' || p_object_id || '#can_cancel')) || jsonb_build_object('union', jsonb_build_object('nodes', jsonb_build_array(jsonb_build_object('name', ('run' || ':' || p_object_id || '#can_cancel')) || jsonb_build_object('leaf', jsonb_build_object('tuple_to_userset', jsonb_build_object('tupleset', ('run' || ':' || p_object_id || '#agent'), 'computed', COALESCE((SELECT jsonb_agg(jsonb_build_object('userset', subject_type || ':' || subject_id || '#can_execute') ORDER BY subject_type, subject_id) FROM "public"."melange_tuples" WHERE object_type = 'run' AND object_id = p_object_id AND relation = 'agent' AND subject_type IN ('agent')), '[]'::jsonb)))), jsonb_build_object('name', ('run' || ':' || p_object_id || '#can_cancel')) || jsonb_build_object('leaf', jsonb_build_object('tuple_to_userset', jsonb_build_object('tupleset', ('run' || ':' || p_object_id || '#agent'), 'computed', COALESCE((SELECT jsonb_agg(jsonb_build_object('userset', subject_type || ':' || subject_id || '#can_manage') ORDER BY subject_type, subject_id) FROM "public"."melange_tuples" WHERE object_type = 'run' AND object_id = p_object_id AND relation = 'agent' AND subject_type IN ('agent')), '[]'::jsonb))))))));
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';


-- Generated expand function for state_volume.can_attach
-- Returns OpenFGA-shaped UsersetTree JSONB. Shallow by default — computed
-- rewrites surface as Leaf.Computed pointers; callers chase them with
-- follow-up Expand calls or use Checker.ExpandRecursive.
CREATE OR REPLACE FUNCTION "public"."expand_state_volume_can_attach"(
    p_object_id TEXT,
    p_subject_type TEXT DEFAULT NULL,
    p_max_leaf INTEGER DEFAULT NULL
) RETURNS JSONB AS $$
BEGIN
    RETURN jsonb_build_object('root', jsonb_build_object('name', ('state_volume' || ':' || p_object_id || '#can_attach')) || jsonb_build_object('leaf', jsonb_build_object('tuple_to_userset', jsonb_build_object('tupleset', ('state_volume' || ':' || p_object_id || '#agent'), 'computed', COALESCE((SELECT jsonb_agg(jsonb_build_object('userset', subject_type || ':' || subject_id || '#can_execute') ORDER BY subject_type, subject_id) FROM "public"."melange_tuples" WHERE object_type = 'state_volume' AND object_id = p_object_id AND relation = 'agent' AND subject_type IN ('agent')), '[]'::jsonb)))));
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';


-- Generated expand function for project.can_read
-- Returns OpenFGA-shaped UsersetTree JSONB. Shallow by default — computed
-- rewrites surface as Leaf.Computed pointers; callers chase them with
-- follow-up Expand calls or use Checker.ExpandRecursive.
CREATE OR REPLACE FUNCTION "public"."expand_project_can_read"(
    p_object_id TEXT,
    p_subject_type TEXT DEFAULT NULL,
    p_max_leaf INTEGER DEFAULT NULL
) RETURNS JSONB AS $$
BEGIN
    RETURN jsonb_build_object('root', jsonb_build_object('name', ('project' || ':' || p_object_id || '#can_read')) || jsonb_build_object('union', jsonb_build_object('nodes', jsonb_build_array(jsonb_build_object('name', ('project' || ':' || p_object_id || '#can_read')) || jsonb_build_object('leaf', jsonb_build_object('computed', jsonb_build_object('userset', ('project' || ':' || p_object_id || '#maintainer')))), jsonb_build_object('name', ('project' || ':' || p_object_id || '#can_read')) || jsonb_build_object('leaf', jsonb_build_object('tuple_to_userset', jsonb_build_object('tupleset', ('project' || ':' || p_object_id || '#organization'), 'computed', COALESCE((SELECT jsonb_agg(jsonb_build_object('userset', subject_type || ':' || subject_id || '#can_read') ORDER BY subject_type, subject_id) FROM "public"."melange_tuples" WHERE object_type = 'project' AND object_id = p_object_id AND relation = 'organization' AND subject_type IN ('organization')), '[]'::jsonb))))))));
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';


-- Generated expand function for agent.can_read
-- Returns OpenFGA-shaped UsersetTree JSONB. Shallow by default — computed
-- rewrites surface as Leaf.Computed pointers; callers chase them with
-- follow-up Expand calls or use Checker.ExpandRecursive.
CREATE OR REPLACE FUNCTION "public"."expand_agent_can_read"(
    p_object_id TEXT,
    p_subject_type TEXT DEFAULT NULL,
    p_max_leaf INTEGER DEFAULT NULL
) RETURNS JSONB AS $$
BEGIN
    RETURN jsonb_build_object('root', jsonb_build_object('name', ('agent' || ':' || p_object_id || '#can_read')) || jsonb_build_object('leaf', jsonb_build_object('tuple_to_userset', jsonb_build_object('tupleset', ('agent' || ':' || p_object_id || '#project'), 'computed', COALESCE((SELECT jsonb_agg(jsonb_build_object('userset', subject_type || ':' || subject_id || '#can_read') ORDER BY subject_type, subject_id) FROM "public"."melange_tuples" WHERE object_type = 'agent' AND object_id = p_object_id AND relation = 'project' AND subject_type IN ('project')), '[]'::jsonb)))));
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';


-- Generated expand function for repository.can_read
-- Returns OpenFGA-shaped UsersetTree JSONB. Shallow by default — computed
-- rewrites surface as Leaf.Computed pointers; callers chase them with
-- follow-up Expand calls or use Checker.ExpandRecursive.
CREATE OR REPLACE FUNCTION "public"."expand_repository_can_read"(
    p_object_id TEXT,
    p_subject_type TEXT DEFAULT NULL,
    p_max_leaf INTEGER DEFAULT NULL
) RETURNS JSONB AS $$
BEGIN
    RETURN jsonb_build_object('root', jsonb_build_object('name', ('repository' || ':' || p_object_id || '#can_read')) || jsonb_build_object('union', jsonb_build_object('nodes', jsonb_build_array(jsonb_build_object('name', ('repository' || ':' || p_object_id || '#can_read')) || jsonb_build_object('leaf', jsonb_build_object('computed', jsonb_build_object('userset', ('repository' || ':' || p_object_id || '#public')))), jsonb_build_object('name', ('repository' || ':' || p_object_id || '#can_read')) || jsonb_build_object('leaf', jsonb_build_object('tuple_to_userset', jsonb_build_object('tupleset', ('repository' || ':' || p_object_id || '#project'), 'computed', COALESCE((SELECT jsonb_agg(jsonb_build_object('userset', subject_type || ':' || subject_id || '#can_read') ORDER BY subject_type, subject_id) FROM "public"."melange_tuples" WHERE object_type = 'repository' AND object_id = p_object_id AND relation = 'project' AND subject_type IN ('project')), '[]'::jsonb))))))));
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';


-- Generated expand function for run.can_read
-- Returns OpenFGA-shaped UsersetTree JSONB. Shallow by default — computed
-- rewrites surface as Leaf.Computed pointers; callers chase them with
-- follow-up Expand calls or use Checker.ExpandRecursive.
CREATE OR REPLACE FUNCTION "public"."expand_run_can_read"(
    p_object_id TEXT,
    p_subject_type TEXT DEFAULT NULL,
    p_max_leaf INTEGER DEFAULT NULL
) RETURNS JSONB AS $$
BEGIN
    RETURN jsonb_build_object('root', jsonb_build_object('name', ('run' || ':' || p_object_id || '#can_read')) || jsonb_build_object('leaf', jsonb_build_object('tuple_to_userset', jsonb_build_object('tupleset', ('run' || ':' || p_object_id || '#agent'), 'computed', COALESCE((SELECT jsonb_agg(jsonb_build_object('userset', subject_type || ':' || subject_id || '#can_read') ORDER BY subject_type, subject_id) FROM "public"."melange_tuples" WHERE object_type = 'run' AND object_id = p_object_id AND relation = 'agent' AND subject_type IN ('agent')), '[]'::jsonb)))));
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';


-- Generated expand function for state_volume.can_read
-- Returns OpenFGA-shaped UsersetTree JSONB. Shallow by default — computed
-- rewrites surface as Leaf.Computed pointers; callers chase them with
-- follow-up Expand calls or use Checker.ExpandRecursive.
CREATE OR REPLACE FUNCTION "public"."expand_state_volume_can_read"(
    p_object_id TEXT,
    p_subject_type TEXT DEFAULT NULL,
    p_max_leaf INTEGER DEFAULT NULL
) RETURNS JSONB AS $$
BEGIN
    RETURN jsonb_build_object('root', jsonb_build_object('name', ('state_volume' || ':' || p_object_id || '#can_read')) || jsonb_build_object('leaf', jsonb_build_object('tuple_to_userset', jsonb_build_object('tupleset', ('state_volume' || ':' || p_object_id || '#agent'), 'computed', COALESCE((SELECT jsonb_agg(jsonb_build_object('userset', subject_type || ':' || subject_id || '#can_read') ORDER BY subject_type, subject_id) FROM "public"."melange_tuples" WHERE object_type = 'state_volume' AND object_id = p_object_id AND relation = 'agent' AND subject_type IN ('agent')), '[]'::jsonb)))));
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';


-- ============================================================
-- List Objects Functions (31 functions)
-- ============================================================

-- Generated list_objects function for agent.project
-- Features: Direct
CREATE OR REPLACE FUNCTION "public"."list_agent_project_obj"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_limit INT DEFAULT NULL,
    p_after TEXT DEFAULT NULL
) RETURNS TABLE(object_id TEXT, next_cursor TEXT) ROWS 100 AS $$
BEGIN
    RETURN QUERY
        WITH base_results AS (
            -- Direct tuple lookup with simple closure relations
                -- Type guard: only return results if subject type is in allowed subject types
                SELECT DISTINCT t.object_id
                FROM melange_tuples AS t
                WHERE (t.object_type = 'agent' AND t.relation IN ('project') AND t.subject_type = p_subject_type AND p_subject_type IN ('project') AND (t.subject_id = p_subject_id AND NOT (t.subject_id = '*')))
                UNION
                -- Self-candidate: subject is userset on same object type
                SELECT split_part(p_subject_id, '#', 1)
                WHERE (position('#' in p_subject_id) > 0 AND p_subject_type = 'agent' AND substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('project'))
        ),
        paged AS (
            SELECT br.object_id
            FROM base_results br
            WHERE (p_after IS NULL OR br.object_id > p_after)
            ORDER BY br.object_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.object_id FROM paged p ORDER BY p.object_id LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT max(r.object_id) FROM returned r)
            END AS next_cursor
        )
        SELECT r.object_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';

-- Generated list_objects function for organization.owner
-- Features: Direct
CREATE OR REPLACE FUNCTION "public"."list_organization_owner_obj"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_limit INT DEFAULT NULL,
    p_after TEXT DEFAULT NULL
) RETURNS TABLE(object_id TEXT, next_cursor TEXT) ROWS 100 AS $$
BEGIN
    RETURN QUERY
        WITH base_results AS (
            -- Direct tuple lookup with simple closure relations
                -- Type guard: only return results if subject type is in allowed subject types
                SELECT DISTINCT t.object_id
                FROM melange_tuples AS t
                WHERE (t.object_type = 'organization' AND t.relation IN ('owner') AND t.subject_type = p_subject_type AND p_subject_type IN ('user') AND (t.subject_id = p_subject_id AND NOT (t.subject_id = '*')))
                UNION
                -- Self-candidate: subject is userset on same object type
                SELECT split_part(p_subject_id, '#', 1)
                WHERE (position('#' in p_subject_id) > 0 AND p_subject_type = 'organization' AND substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('owner'))
        ),
        paged AS (
            SELECT br.object_id
            FROM base_results br
            WHERE (p_after IS NULL OR br.object_id > p_after)
            ORDER BY br.object_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.object_id FROM paged p ORDER BY p.object_id LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT max(r.object_id) FROM returned r)
            END AS next_cursor
        )
        SELECT r.object_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';

-- Generated list_objects function for project.maintainer
-- Features: Direct
CREATE OR REPLACE FUNCTION "public"."list_project_maintainer_obj"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_limit INT DEFAULT NULL,
    p_after TEXT DEFAULT NULL
) RETURNS TABLE(object_id TEXT, next_cursor TEXT) ROWS 100 AS $$
BEGIN
    RETURN QUERY
        WITH base_results AS (
            -- Direct tuple lookup with simple closure relations
                -- Type guard: only return results if subject type is in allowed subject types
                SELECT DISTINCT t.object_id
                FROM melange_tuples AS t
                WHERE (t.object_type = 'project' AND t.relation IN ('maintainer') AND t.subject_type = p_subject_type AND p_subject_type IN ('user') AND (t.subject_id = p_subject_id AND NOT (t.subject_id = '*')))
                UNION
                -- Self-candidate: subject is userset on same object type
                SELECT split_part(p_subject_id, '#', 1)
                WHERE (position('#' in p_subject_id) > 0 AND p_subject_type = 'project' AND substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('maintainer'))
        ),
        paged AS (
            SELECT br.object_id
            FROM base_results br
            WHERE (p_after IS NULL OR br.object_id > p_after)
            ORDER BY br.object_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.object_id FROM paged p ORDER BY p.object_id LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT max(r.object_id) FROM returned r)
            END AS next_cursor
        )
        SELECT r.object_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';

-- Generated list_objects function for project.organization
-- Features: Direct
CREATE OR REPLACE FUNCTION "public"."list_project_organization_obj"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_limit INT DEFAULT NULL,
    p_after TEXT DEFAULT NULL
) RETURNS TABLE(object_id TEXT, next_cursor TEXT) ROWS 100 AS $$
BEGIN
    RETURN QUERY
        WITH base_results AS (
            -- Direct tuple lookup with simple closure relations
                -- Type guard: only return results if subject type is in allowed subject types
                SELECT DISTINCT t.object_id
                FROM melange_tuples AS t
                WHERE (t.object_type = 'project' AND t.relation IN ('organization') AND t.subject_type = p_subject_type AND p_subject_type IN ('organization') AND (t.subject_id = p_subject_id AND NOT (t.subject_id = '*')))
                UNION
                -- Self-candidate: subject is userset on same object type
                SELECT split_part(p_subject_id, '#', 1)
                WHERE (position('#' in p_subject_id) > 0 AND p_subject_type = 'project' AND substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('organization'))
        ),
        paged AS (
            SELECT br.object_id
            FROM base_results br
            WHERE (p_after IS NULL OR br.object_id > p_after)
            ORDER BY br.object_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.object_id FROM paged p ORDER BY p.object_id LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT max(r.object_id) FROM returned r)
            END AS next_cursor
        )
        SELECT r.object_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';

-- Generated list_objects function for repository.project
-- Features: Direct
CREATE OR REPLACE FUNCTION "public"."list_repository_project_obj"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_limit INT DEFAULT NULL,
    p_after TEXT DEFAULT NULL
) RETURNS TABLE(object_id TEXT, next_cursor TEXT) ROWS 100 AS $$
BEGIN
    RETURN QUERY
        WITH base_results AS (
            -- Direct tuple lookup with simple closure relations
                -- Type guard: only return results if subject type is in allowed subject types
                SELECT DISTINCT t.object_id
                FROM melange_tuples AS t
                WHERE (t.object_type = 'repository' AND t.relation IN ('project') AND t.subject_type = p_subject_type AND p_subject_type IN ('project') AND (t.subject_id = p_subject_id AND NOT (t.subject_id = '*')))
                UNION
                -- Self-candidate: subject is userset on same object type
                SELECT split_part(p_subject_id, '#', 1)
                WHERE (position('#' in p_subject_id) > 0 AND p_subject_type = 'repository' AND substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('project'))
        ),
        paged AS (
            SELECT br.object_id
            FROM base_results br
            WHERE (p_after IS NULL OR br.object_id > p_after)
            ORDER BY br.object_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.object_id FROM paged p ORDER BY p.object_id LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT max(r.object_id) FROM returned r)
            END AS next_cursor
        )
        SELECT r.object_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';

-- Generated list_objects function for repository.public
-- Features: Direct+Wildcard
CREATE OR REPLACE FUNCTION "public"."list_repository_public_obj"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_limit INT DEFAULT NULL,
    p_after TEXT DEFAULT NULL
) RETURNS TABLE(object_id TEXT, next_cursor TEXT) ROWS 100 AS $$
BEGIN
    RETURN QUERY
        WITH base_results AS (
            -- Direct tuple lookup with simple closure relations
                -- Type guard: only return results if subject type is in allowed subject types
                SELECT DISTINCT t.object_id
                FROM melange_tuples AS t
                WHERE (t.object_type = 'repository' AND t.relation IN ('public') AND t.subject_type = p_subject_type AND p_subject_type IN ('user') AND (t.subject_id = p_subject_id OR t.subject_id = '*'))
                UNION
                -- Self-candidate: subject is userset on same object type
                SELECT split_part(p_subject_id, '#', 1)
                WHERE (position('#' in p_subject_id) > 0 AND p_subject_type = 'repository' AND substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('public'))
        ),
        paged AS (
            SELECT br.object_id
            FROM base_results br
            WHERE (p_after IS NULL OR br.object_id > p_after)
            ORDER BY br.object_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.object_id FROM paged p ORDER BY p.object_id LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT max(r.object_id) FROM returned r)
            END AS next_cursor
        )
        SELECT r.object_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';

-- Generated list_objects function for run.agent
-- Features: Direct
CREATE OR REPLACE FUNCTION "public"."list_run_agent_obj"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_limit INT DEFAULT NULL,
    p_after TEXT DEFAULT NULL
) RETURNS TABLE(object_id TEXT, next_cursor TEXT) ROWS 100 AS $$
BEGIN
    RETURN QUERY
        WITH base_results AS (
            -- Direct tuple lookup with simple closure relations
                -- Type guard: only return results if subject type is in allowed subject types
                SELECT DISTINCT t.object_id
                FROM melange_tuples AS t
                WHERE (t.object_type = 'run' AND t.relation IN ('agent') AND t.subject_type = p_subject_type AND p_subject_type IN ('agent') AND (t.subject_id = p_subject_id AND NOT (t.subject_id = '*')))
                UNION
                -- Self-candidate: subject is userset on same object type
                SELECT split_part(p_subject_id, '#', 1)
                WHERE (position('#' in p_subject_id) > 0 AND p_subject_type = 'run' AND substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('agent'))
        ),
        paged AS (
            SELECT br.object_id
            FROM base_results br
            WHERE (p_after IS NULL OR br.object_id > p_after)
            ORDER BY br.object_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.object_id FROM paged p ORDER BY p.object_id LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT max(r.object_id) FROM returned r)
            END AS next_cursor
        )
        SELECT r.object_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';

-- Generated list_objects function for state_volume.agent
-- Features: Direct
CREATE OR REPLACE FUNCTION "public"."list_state_volume_agent_obj"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_limit INT DEFAULT NULL,
    p_after TEXT DEFAULT NULL
) RETURNS TABLE(object_id TEXT, next_cursor TEXT) ROWS 100 AS $$
BEGIN
    RETURN QUERY
        WITH base_results AS (
            -- Direct tuple lookup with simple closure relations
                -- Type guard: only return results if subject type is in allowed subject types
                SELECT DISTINCT t.object_id
                FROM melange_tuples AS t
                WHERE (t.object_type = 'state_volume' AND t.relation IN ('agent') AND t.subject_type = p_subject_type AND p_subject_type IN ('agent') AND (t.subject_id = p_subject_id AND NOT (t.subject_id = '*')))
                UNION
                -- Self-candidate: subject is userset on same object type
                SELECT split_part(p_subject_id, '#', 1)
                WHERE (position('#' in p_subject_id) > 0 AND p_subject_type = 'state_volume' AND substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('agent'))
        ),
        paged AS (
            SELECT br.object_id
            FROM base_results br
            WHERE (p_after IS NULL OR br.object_id > p_after)
            ORDER BY br.object_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.object_id FROM paged p ORDER BY p.object_id LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT max(r.object_id) FROM returned r)
            END AS next_cursor
        )
        SELECT r.object_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';

-- Generated list_objects function for organization.admin
-- Features: Direct+Implied
CREATE OR REPLACE FUNCTION "public"."list_organization_admin_obj"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_limit INT DEFAULT NULL,
    p_after TEXT DEFAULT NULL
) RETURNS TABLE(object_id TEXT, next_cursor TEXT) ROWS 100 AS $$
BEGIN
    RETURN QUERY
        WITH base_results AS (
            -- Direct tuple lookup with simple closure relations
                -- Type guard: only return results if subject type is in allowed subject types
                SELECT DISTINCT t.object_id
                FROM melange_tuples AS t
                WHERE (t.object_type = 'organization' AND t.relation IN ('admin', 'owner') AND t.subject_type = p_subject_type AND p_subject_type IN ('user') AND (t.subject_id = p_subject_id AND NOT (t.subject_id = '*')))
                UNION
                -- Self-candidate: subject is userset on same object type
                SELECT split_part(p_subject_id, '#', 1)
                WHERE (position('#' in p_subject_id) > 0 AND p_subject_type = 'organization' AND substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('admin', 'owner'))
        ),
        paged AS (
            SELECT br.object_id
            FROM base_results br
            WHERE (p_after IS NULL OR br.object_id > p_after)
            ORDER BY br.object_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.object_id FROM paged p ORDER BY p.object_id LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT max(r.object_id) FROM returned r)
            END AS next_cursor
        )
        SELECT r.object_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';

-- Generated list_objects function for organization.can_delete
-- Features: Implied
CREATE OR REPLACE FUNCTION "public"."list_organization_can_delete_obj"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_limit INT DEFAULT NULL,
    p_after TEXT DEFAULT NULL
) RETURNS TABLE(object_id TEXT, next_cursor TEXT) ROWS 100 AS $$
BEGIN
    RETURN QUERY
        WITH base_results AS (
            -- Direct tuple lookup with simple closure relations
                -- Type guard: only return results if subject type is in allowed subject types
                SELECT DISTINCT t.object_id
                FROM melange_tuples AS t
                WHERE (t.object_type = 'organization' AND t.relation IN ('can_delete', 'owner') AND t.subject_type = p_subject_type AND p_subject_type IN ('user') AND (t.subject_id = p_subject_id AND NOT (t.subject_id = '*')))
                UNION
                -- Self-candidate: subject is userset on same object type
                SELECT split_part(p_subject_id, '#', 1)
                WHERE (position('#' in p_subject_id) > 0 AND p_subject_type = 'organization' AND substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('can_delete', 'owner'))
        ),
        paged AS (
            SELECT br.object_id
            FROM base_results br
            WHERE (p_after IS NULL OR br.object_id > p_after)
            ORDER BY br.object_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.object_id FROM paged p ORDER BY p.object_id LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT max(r.object_id) FROM returned r)
            END AS next_cursor
        )
        SELECT r.object_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';

-- Generated list_objects function for organization.can_manage
-- Features: Implied
CREATE OR REPLACE FUNCTION "public"."list_organization_can_manage_obj"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_limit INT DEFAULT NULL,
    p_after TEXT DEFAULT NULL
) RETURNS TABLE(object_id TEXT, next_cursor TEXT) ROWS 100 AS $$
BEGIN
    RETURN QUERY
        WITH base_results AS (
            -- Direct tuple lookup with simple closure relations
                -- Type guard: only return results if subject type is in allowed subject types
                SELECT DISTINCT t.object_id
                FROM melange_tuples AS t
                WHERE (t.object_type = 'organization' AND t.relation IN ('can_manage', 'owner') AND t.subject_type = p_subject_type AND p_subject_type IN ('user') AND (t.subject_id = p_subject_id AND NOT (t.subject_id = '*')))
                UNION
                -- Self-candidate: subject is userset on same object type
                SELECT split_part(p_subject_id, '#', 1)
                WHERE (position('#' in p_subject_id) > 0 AND p_subject_type = 'organization' AND substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('can_manage', 'owner'))
        ),
        paged AS (
            SELECT br.object_id
            FROM base_results br
            WHERE (p_after IS NULL OR br.object_id > p_after)
            ORDER BY br.object_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.object_id FROM paged p ORDER BY p.object_id LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT max(r.object_id) FROM returned r)
            END AS next_cursor
        )
        SELECT r.object_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';

-- Generated list_objects function for project.can_delete
-- Features: Recursive
-- Indirect anchor: organization.owner via ttu
CREATE OR REPLACE FUNCTION "public"."list_project_can_delete_obj"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_limit INT DEFAULT NULL,
    p_after TEXT DEFAULT NULL
) RETURNS TABLE(object_id TEXT, next_cursor TEXT) ROWS 100 AS $$
BEGIN
    -- Self-candidate check: when subject is a userset on the same object type
    IF EXISTS (
    SELECT split_part(p_subject_id, '#', 1) AS object_id
    WHERE (p_subject_type = 'project' AND position('#' in p_subject_id) > 0 AND substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('can_delete'))
    ) THEN
        RETURN QUERY
        WITH base_results AS (
            SELECT split_part(p_subject_id, '#', 1) AS object_id
            WHERE (p_subject_type = 'project' AND position('#' in p_subject_id) > 0 AND substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('can_delete'))
        ),
        paged AS (
            SELECT br.object_id
            FROM base_results br
            WHERE (p_after IS NULL OR br.object_id > p_after)
            ORDER BY br.object_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.object_id FROM paged p ORDER BY p.object_id LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT max(r.object_id) FROM returned r)
            END AS next_cursor
        )
        SELECT r.object_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
        RETURN;
    END IF;
    -- Type guard: only return results if subject type is allowed
    -- Skip the guard for userset subjects since composed inner calls handle userset subjects
    IF (position('#' in p_subject_id) = 0 AND p_subject_type NOT IN ('user')) THEN
        RETURN;
    END IF;
    RETURN QUERY
        WITH base_results AS (
            -- TTU composition: organization -> organization
                SELECT DISTINCT t.object_id
                FROM melange_tuples AS t
                WHERE (t.object_type = 'project' AND t.relation = 'organization' AND t.subject_type = 'organization' AND t.subject_id IN (SELECT obj.object_id FROM "public"."list_organization_owner_obj"(p_subject_type, p_subject_id, NULL, NULL) obj))
        ),
        paged AS (
            SELECT br.object_id
            FROM base_results br
            WHERE (p_after IS NULL OR br.object_id > p_after)
            ORDER BY br.object_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.object_id FROM paged p ORDER BY p.object_id LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT max(r.object_id) FROM returned r)
            END AS next_cursor
        )
        SELECT r.object_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';

-- Generated list_objects function for project.can_manage
-- Features: Implied
CREATE OR REPLACE FUNCTION "public"."list_project_can_manage_obj"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_limit INT DEFAULT NULL,
    p_after TEXT DEFAULT NULL
) RETURNS TABLE(object_id TEXT, next_cursor TEXT) ROWS 100 AS $$
BEGIN
    RETURN QUERY
        WITH base_results AS (
            -- Direct tuple lookup with simple closure relations
                -- Type guard: only return results if subject type is in allowed subject types
                SELECT DISTINCT t.object_id
                FROM melange_tuples AS t
                WHERE (t.object_type = 'project' AND t.relation IN ('can_manage', 'maintainer') AND t.subject_type = p_subject_type AND p_subject_type IN ('user') AND (t.subject_id = p_subject_id AND NOT (t.subject_id = '*')))
                UNION
                -- Self-candidate: subject is userset on same object type
                SELECT split_part(p_subject_id, '#', 1)
                WHERE (position('#' in p_subject_id) > 0 AND p_subject_type = 'project' AND substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('can_manage', 'maintainer'))
        ),
        paged AS (
            SELECT br.object_id
            FROM base_results br
            WHERE (p_after IS NULL OR br.object_id > p_after)
            ORDER BY br.object_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.object_id FROM paged p ORDER BY p.object_id LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT max(r.object_id) FROM returned r)
            END AS next_cursor
        )
        SELECT r.object_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';

-- Generated list_objects function for project.can_write
-- Features: Implied
CREATE OR REPLACE FUNCTION "public"."list_project_can_write_obj"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_limit INT DEFAULT NULL,
    p_after TEXT DEFAULT NULL
) RETURNS TABLE(object_id TEXT, next_cursor TEXT) ROWS 100 AS $$
BEGIN
    RETURN QUERY
        WITH base_results AS (
            -- Direct tuple lookup with simple closure relations
                -- Type guard: only return results if subject type is in allowed subject types
                SELECT DISTINCT t.object_id
                FROM melange_tuples AS t
                WHERE (t.object_type = 'project' AND t.relation IN ('can_write', 'maintainer') AND t.subject_type = p_subject_type AND p_subject_type IN ('user') AND (t.subject_id = p_subject_id AND NOT (t.subject_id = '*')))
                UNION
                -- Self-candidate: subject is userset on same object type
                SELECT split_part(p_subject_id, '#', 1)
                WHERE (position('#' in p_subject_id) > 0 AND p_subject_type = 'project' AND substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('can_write', 'maintainer'))
        ),
        paged AS (
            SELECT br.object_id
            FROM base_results br
            WHERE (p_after IS NULL OR br.object_id > p_after)
            ORDER BY br.object_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.object_id FROM paged p ORDER BY p.object_id LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT max(r.object_id) FROM returned r)
            END AS next_cursor
        )
        SELECT r.object_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';

-- Generated list_objects function for organization.can_create_project
-- Features: Implied
CREATE OR REPLACE FUNCTION "public"."list_organization_can_create_project_obj"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_limit INT DEFAULT NULL,
    p_after TEXT DEFAULT NULL
) RETURNS TABLE(object_id TEXT, next_cursor TEXT) ROWS 100 AS $$
BEGIN
    RETURN QUERY
        WITH base_results AS (
            -- Direct tuple lookup with simple closure relations
                -- Type guard: only return results if subject type is in allowed subject types
                SELECT DISTINCT t.object_id
                FROM melange_tuples AS t
                WHERE (t.object_type = 'organization' AND t.relation IN ('can_create_project', 'admin', 'owner') AND t.subject_type = p_subject_type AND p_subject_type IN ('user') AND (t.subject_id = p_subject_id AND NOT (t.subject_id = '*')))
                UNION
                -- Self-candidate: subject is userset on same object type
                SELECT split_part(p_subject_id, '#', 1)
                WHERE (position('#' in p_subject_id) > 0 AND p_subject_type = 'organization' AND substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('admin', 'can_create_project', 'owner'))
        ),
        paged AS (
            SELECT br.object_id
            FROM base_results br
            WHERE (p_after IS NULL OR br.object_id > p_after)
            ORDER BY br.object_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.object_id FROM paged p ORDER BY p.object_id LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT max(r.object_id) FROM returned r)
            END AS next_cursor
        )
        SELECT r.object_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';

-- Generated list_objects function for organization.can_manage_members
-- Features: Implied
CREATE OR REPLACE FUNCTION "public"."list_organization_can_manage_members_obj"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_limit INT DEFAULT NULL,
    p_after TEXT DEFAULT NULL
) RETURNS TABLE(object_id TEXT, next_cursor TEXT) ROWS 100 AS $$
BEGIN
    RETURN QUERY
        WITH base_results AS (
            -- Direct tuple lookup with simple closure relations
                -- Type guard: only return results if subject type is in allowed subject types
                SELECT DISTINCT t.object_id
                FROM melange_tuples AS t
                WHERE (t.object_type = 'organization' AND t.relation IN ('can_manage_members', 'admin', 'owner') AND t.subject_type = p_subject_type AND p_subject_type IN ('user') AND (t.subject_id = p_subject_id AND NOT (t.subject_id = '*')))
                UNION
                -- Self-candidate: subject is userset on same object type
                SELECT split_part(p_subject_id, '#', 1)
                WHERE (position('#' in p_subject_id) > 0 AND p_subject_type = 'organization' AND substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('admin', 'can_manage_members', 'owner'))
        ),
        paged AS (
            SELECT br.object_id
            FROM base_results br
            WHERE (p_after IS NULL OR br.object_id > p_after)
            ORDER BY br.object_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.object_id FROM paged p ORDER BY p.object_id LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT max(r.object_id) FROM returned r)
            END AS next_cursor
        )
        SELECT r.object_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';

-- Generated list_objects function for organization.member
-- Features: Direct+Implied
CREATE OR REPLACE FUNCTION "public"."list_organization_member_obj"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_limit INT DEFAULT NULL,
    p_after TEXT DEFAULT NULL
) RETURNS TABLE(object_id TEXT, next_cursor TEXT) ROWS 100 AS $$
BEGIN
    RETURN QUERY
        WITH base_results AS (
            -- Direct tuple lookup with simple closure relations
                -- Type guard: only return results if subject type is in allowed subject types
                SELECT DISTINCT t.object_id
                FROM melange_tuples AS t
                WHERE (t.object_type = 'organization' AND t.relation IN ('member', 'admin', 'owner') AND t.subject_type = p_subject_type AND p_subject_type IN ('user') AND (t.subject_id = p_subject_id AND NOT (t.subject_id = '*')))
                UNION
                -- Self-candidate: subject is userset on same object type
                SELECT split_part(p_subject_id, '#', 1)
                WHERE (position('#' in p_subject_id) > 0 AND p_subject_type = 'organization' AND substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('admin', 'member', 'owner'))
        ),
        paged AS (
            SELECT br.object_id
            FROM base_results br
            WHERE (p_after IS NULL OR br.object_id > p_after)
            ORDER BY br.object_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.object_id FROM paged p ORDER BY p.object_id LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT max(r.object_id) FROM returned r)
            END AS next_cursor
        )
        SELECT r.object_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';

-- Generated list_objects function for repository.can_delete
-- Features: Recursive
-- Indirect anchor: organization.owner via ttu
CREATE OR REPLACE FUNCTION "public"."list_repository_can_delete_obj"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_limit INT DEFAULT NULL,
    p_after TEXT DEFAULT NULL
) RETURNS TABLE(object_id TEXT, next_cursor TEXT) ROWS 100 AS $$
BEGIN
    -- Self-candidate check: when subject is a userset on the same object type
    IF EXISTS (
    SELECT split_part(p_subject_id, '#', 1) AS object_id
    WHERE (p_subject_type = 'repository' AND position('#' in p_subject_id) > 0 AND substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('can_delete'))
    ) THEN
        RETURN QUERY
        WITH base_results AS (
            SELECT split_part(p_subject_id, '#', 1) AS object_id
            WHERE (p_subject_type = 'repository' AND position('#' in p_subject_id) > 0 AND substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('can_delete'))
        ),
        paged AS (
            SELECT br.object_id
            FROM base_results br
            WHERE (p_after IS NULL OR br.object_id > p_after)
            ORDER BY br.object_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.object_id FROM paged p ORDER BY p.object_id LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT max(r.object_id) FROM returned r)
            END AS next_cursor
        )
        SELECT r.object_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
        RETURN;
    END IF;
    -- Type guard: only return results if subject type is allowed
    -- Skip the guard for userset subjects since composed inner calls handle userset subjects
    IF (position('#' in p_subject_id) = 0 AND p_subject_type NOT IN ('user')) THEN
        RETURN;
    END IF;
    RETURN QUERY
        WITH base_results AS (
            -- TTU composition: project -> project
                SELECT DISTINCT t.object_id
                FROM melange_tuples AS t
                WHERE (t.object_type = 'repository' AND t.relation = 'project' AND t.subject_type = 'project' AND t.subject_id IN (SELECT obj.object_id FROM "public"."list_project_can_delete_obj"(p_subject_type, p_subject_id, NULL, NULL) obj))
        ),
        paged AS (
            SELECT br.object_id
            FROM base_results br
            WHERE (p_after IS NULL OR br.object_id > p_after)
            ORDER BY br.object_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.object_id FROM paged p ORDER BY p.object_id LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT max(r.object_id) FROM returned r)
            END AS next_cursor
        )
        SELECT r.object_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';

-- Generated list_objects function for agent.can_manage
-- Features: Recursive
-- Indirect anchor: project.can_manage via ttu
CREATE OR REPLACE FUNCTION "public"."list_agent_can_manage_obj"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_limit INT DEFAULT NULL,
    p_after TEXT DEFAULT NULL
) RETURNS TABLE(object_id TEXT, next_cursor TEXT) ROWS 100 AS $$
BEGIN
    -- Self-candidate check: when subject is a userset on the same object type
    IF EXISTS (
    SELECT split_part(p_subject_id, '#', 1) AS object_id
    WHERE (p_subject_type = 'agent' AND position('#' in p_subject_id) > 0 AND substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('can_manage'))
    ) THEN
        RETURN QUERY
        WITH base_results AS (
            SELECT split_part(p_subject_id, '#', 1) AS object_id
            WHERE (p_subject_type = 'agent' AND position('#' in p_subject_id) > 0 AND substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('can_manage'))
        ),
        paged AS (
            SELECT br.object_id
            FROM base_results br
            WHERE (p_after IS NULL OR br.object_id > p_after)
            ORDER BY br.object_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.object_id FROM paged p ORDER BY p.object_id LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT max(r.object_id) FROM returned r)
            END AS next_cursor
        )
        SELECT r.object_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
        RETURN;
    END IF;
    -- Type guard: only return results if subject type is allowed
    -- Skip the guard for userset subjects since composed inner calls handle userset subjects
    IF (position('#' in p_subject_id) = 0 AND p_subject_type NOT IN ('user')) THEN
        RETURN;
    END IF;
    RETURN QUERY
        WITH base_results AS (
            -- TTU composition: project -> project
                SELECT DISTINCT t.object_id
                FROM melange_tuples AS t
                WHERE (t.object_type = 'agent' AND t.relation = 'project' AND t.subject_type = 'project' AND t.subject_id IN (SELECT obj.object_id FROM "public"."list_project_can_manage_obj"(p_subject_type, p_subject_id, NULL, NULL) obj))
        ),
        paged AS (
            SELECT br.object_id
            FROM base_results br
            WHERE (p_after IS NULL OR br.object_id > p_after)
            ORDER BY br.object_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.object_id FROM paged p ORDER BY p.object_id LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT max(r.object_id) FROM returned r)
            END AS next_cursor
        )
        SELECT r.object_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';

-- Generated list_objects function for agent.can_execute
-- Features: Recursive
-- Indirect anchor: project.can_write via ttu
CREATE OR REPLACE FUNCTION "public"."list_agent_can_execute_obj"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_limit INT DEFAULT NULL,
    p_after TEXT DEFAULT NULL
) RETURNS TABLE(object_id TEXT, next_cursor TEXT) ROWS 100 AS $$
BEGIN
    -- Self-candidate check: when subject is a userset on the same object type
    IF EXISTS (
    SELECT split_part(p_subject_id, '#', 1) AS object_id
    WHERE (p_subject_type = 'agent' AND position('#' in p_subject_id) > 0 AND substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('can_execute'))
    ) THEN
        RETURN QUERY
        WITH base_results AS (
            SELECT split_part(p_subject_id, '#', 1) AS object_id
            WHERE (p_subject_type = 'agent' AND position('#' in p_subject_id) > 0 AND substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('can_execute'))
        ),
        paged AS (
            SELECT br.object_id
            FROM base_results br
            WHERE (p_after IS NULL OR br.object_id > p_after)
            ORDER BY br.object_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.object_id FROM paged p ORDER BY p.object_id LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT max(r.object_id) FROM returned r)
            END AS next_cursor
        )
        SELECT r.object_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
        RETURN;
    END IF;
    -- Type guard: only return results if subject type is allowed
    -- Skip the guard for userset subjects since composed inner calls handle userset subjects
    IF (position('#' in p_subject_id) = 0 AND p_subject_type NOT IN ('user')) THEN
        RETURN;
    END IF;
    RETURN QUERY
        WITH base_results AS (
            -- TTU composition: project -> project
                SELECT DISTINCT t.object_id
                FROM melange_tuples AS t
                WHERE (t.object_type = 'agent' AND t.relation = 'project' AND t.subject_type = 'project' AND t.subject_id IN (SELECT obj.object_id FROM "public"."list_project_can_write_obj"(p_subject_type, p_subject_id, NULL, NULL) obj))
        ),
        paged AS (
            SELECT br.object_id
            FROM base_results br
            WHERE (p_after IS NULL OR br.object_id > p_after)
            ORDER BY br.object_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.object_id FROM paged p ORDER BY p.object_id LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT max(r.object_id) FROM returned r)
            END AS next_cursor
        )
        SELECT r.object_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';

-- Generated list_objects function for repository.can_write
-- Features: Recursive
-- Indirect anchor: project.can_write via ttu
CREATE OR REPLACE FUNCTION "public"."list_repository_can_write_obj"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_limit INT DEFAULT NULL,
    p_after TEXT DEFAULT NULL
) RETURNS TABLE(object_id TEXT, next_cursor TEXT) ROWS 100 AS $$
BEGIN
    -- Self-candidate check: when subject is a userset on the same object type
    IF EXISTS (
    SELECT split_part(p_subject_id, '#', 1) AS object_id
    WHERE (p_subject_type = 'repository' AND position('#' in p_subject_id) > 0 AND substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('can_write'))
    ) THEN
        RETURN QUERY
        WITH base_results AS (
            SELECT split_part(p_subject_id, '#', 1) AS object_id
            WHERE (p_subject_type = 'repository' AND position('#' in p_subject_id) > 0 AND substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('can_write'))
        ),
        paged AS (
            SELECT br.object_id
            FROM base_results br
            WHERE (p_after IS NULL OR br.object_id > p_after)
            ORDER BY br.object_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.object_id FROM paged p ORDER BY p.object_id LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT max(r.object_id) FROM returned r)
            END AS next_cursor
        )
        SELECT r.object_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
        RETURN;
    END IF;
    -- Type guard: only return results if subject type is allowed
    -- Skip the guard for userset subjects since composed inner calls handle userset subjects
    IF (position('#' in p_subject_id) = 0 AND p_subject_type NOT IN ('user')) THEN
        RETURN;
    END IF;
    RETURN QUERY
        WITH base_results AS (
            -- TTU composition: project -> project
                SELECT DISTINCT t.object_id
                FROM melange_tuples AS t
                WHERE (t.object_type = 'repository' AND t.relation = 'project' AND t.subject_type = 'project' AND t.subject_id IN (SELECT obj.object_id FROM "public"."list_project_can_write_obj"(p_subject_type, p_subject_id, NULL, NULL) obj))
        ),
        paged AS (
            SELECT br.object_id
            FROM base_results br
            WHERE (p_after IS NULL OR br.object_id > p_after)
            ORDER BY br.object_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.object_id FROM paged p ORDER BY p.object_id LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT max(r.object_id) FROM returned r)
            END AS next_cursor
        )
        SELECT r.object_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';

-- Generated list_objects function for organization.can_read
-- Features: Implied
CREATE OR REPLACE FUNCTION "public"."list_organization_can_read_obj"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_limit INT DEFAULT NULL,
    p_after TEXT DEFAULT NULL
) RETURNS TABLE(object_id TEXT, next_cursor TEXT) ROWS 100 AS $$
BEGIN
    RETURN QUERY
        WITH base_results AS (
            -- Direct tuple lookup with simple closure relations
                -- Type guard: only return results if subject type is in allowed subject types
                SELECT DISTINCT t.object_id
                FROM melange_tuples AS t
                WHERE (t.object_type = 'organization' AND t.relation IN ('can_read', 'admin', 'member', 'owner') AND t.subject_type = p_subject_type AND p_subject_type IN ('user') AND (t.subject_id = p_subject_id AND NOT (t.subject_id = '*')))
                UNION
                -- Self-candidate: subject is userset on same object type
                SELECT split_part(p_subject_id, '#', 1)
                WHERE (position('#' in p_subject_id) > 0 AND p_subject_type = 'organization' AND substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('admin', 'can_read', 'member', 'owner'))
        ),
        paged AS (
            SELECT br.object_id
            FROM base_results br
            WHERE (p_after IS NULL OR br.object_id > p_after)
            ORDER BY br.object_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.object_id FROM paged p ORDER BY p.object_id LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT max(r.object_id) FROM returned r)
            END AS next_cursor
        )
        SELECT r.object_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';

-- Generated list_objects function for state_volume.can_manage
-- Features: Recursive
-- Indirect anchor: project.can_manage via ttu
CREATE OR REPLACE FUNCTION "public"."list_state_volume_can_manage_obj"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_limit INT DEFAULT NULL,
    p_after TEXT DEFAULT NULL
) RETURNS TABLE(object_id TEXT, next_cursor TEXT) ROWS 100 AS $$
BEGIN
    -- Self-candidate check: when subject is a userset on the same object type
    IF EXISTS (
    SELECT split_part(p_subject_id, '#', 1) AS object_id
    WHERE (p_subject_type = 'state_volume' AND position('#' in p_subject_id) > 0 AND substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('can_manage'))
    ) THEN
        RETURN QUERY
        WITH base_results AS (
            SELECT split_part(p_subject_id, '#', 1) AS object_id
            WHERE (p_subject_type = 'state_volume' AND position('#' in p_subject_id) > 0 AND substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('can_manage'))
        ),
        paged AS (
            SELECT br.object_id
            FROM base_results br
            WHERE (p_after IS NULL OR br.object_id > p_after)
            ORDER BY br.object_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.object_id FROM paged p ORDER BY p.object_id LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT max(r.object_id) FROM returned r)
            END AS next_cursor
        )
        SELECT r.object_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
        RETURN;
    END IF;
    -- Type guard: only return results if subject type is allowed
    -- Skip the guard for userset subjects since composed inner calls handle userset subjects
    IF (position('#' in p_subject_id) = 0 AND p_subject_type NOT IN ('user')) THEN
        RETURN;
    END IF;
    RETURN QUERY
        WITH base_results AS (
            -- TTU composition: agent -> agent
                SELECT DISTINCT t.object_id
                FROM melange_tuples AS t
                WHERE (t.object_type = 'state_volume' AND t.relation = 'agent' AND t.subject_type = 'agent' AND t.subject_id IN (SELECT obj.object_id FROM "public"."list_agent_can_manage_obj"(p_subject_type, p_subject_id, NULL, NULL) obj))
        ),
        paged AS (
            SELECT br.object_id
            FROM base_results br
            WHERE (p_after IS NULL OR br.object_id > p_after)
            ORDER BY br.object_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.object_id FROM paged p ORDER BY p.object_id LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT max(r.object_id) FROM returned r)
            END AS next_cursor
        )
        SELECT r.object_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';

-- Generated list_objects function for state_volume.can_restore
-- Features: Recursive
-- Indirect anchor: project.can_manage via ttu
CREATE OR REPLACE FUNCTION "public"."list_state_volume_can_restore_obj"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_limit INT DEFAULT NULL,
    p_after TEXT DEFAULT NULL
) RETURNS TABLE(object_id TEXT, next_cursor TEXT) ROWS 100 AS $$
BEGIN
    -- Self-candidate check: when subject is a userset on the same object type
    IF EXISTS (
    SELECT split_part(p_subject_id, '#', 1) AS object_id
    WHERE (p_subject_type = 'state_volume' AND position('#' in p_subject_id) > 0 AND substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('can_restore'))
    ) THEN
        RETURN QUERY
        WITH base_results AS (
            SELECT split_part(p_subject_id, '#', 1) AS object_id
            WHERE (p_subject_type = 'state_volume' AND position('#' in p_subject_id) > 0 AND substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('can_restore'))
        ),
        paged AS (
            SELECT br.object_id
            FROM base_results br
            WHERE (p_after IS NULL OR br.object_id > p_after)
            ORDER BY br.object_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.object_id FROM paged p ORDER BY p.object_id LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT max(r.object_id) FROM returned r)
            END AS next_cursor
        )
        SELECT r.object_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
        RETURN;
    END IF;
    -- Type guard: only return results if subject type is allowed
    -- Skip the guard for userset subjects since composed inner calls handle userset subjects
    IF (position('#' in p_subject_id) = 0 AND p_subject_type NOT IN ('user')) THEN
        RETURN;
    END IF;
    RETURN QUERY
        WITH base_results AS (
            -- TTU composition: agent -> agent
                SELECT DISTINCT t.object_id
                FROM melange_tuples AS t
                WHERE (t.object_type = 'state_volume' AND t.relation = 'agent' AND t.subject_type = 'agent' AND t.subject_id IN (SELECT obj.object_id FROM "public"."list_agent_can_manage_obj"(p_subject_type, p_subject_id, NULL, NULL) obj))
        ),
        paged AS (
            SELECT br.object_id
            FROM base_results br
            WHERE (p_after IS NULL OR br.object_id > p_after)
            ORDER BY br.object_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.object_id FROM paged p ORDER BY p.object_id LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT max(r.object_id) FROM returned r)
            END AS next_cursor
        )
        SELECT r.object_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';

-- Generated list_objects function for run.can_cancel
-- Features: Recursive
-- Indirect anchor: project.can_write via ttu
CREATE OR REPLACE FUNCTION "public"."list_run_can_cancel_obj"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_limit INT DEFAULT NULL,
    p_after TEXT DEFAULT NULL
) RETURNS TABLE(object_id TEXT, next_cursor TEXT) ROWS 100 AS $$
BEGIN
    -- Self-candidate check: when subject is a userset on the same object type
    IF EXISTS (
    SELECT split_part(p_subject_id, '#', 1) AS object_id
    WHERE (p_subject_type = 'run' AND position('#' in p_subject_id) > 0 AND substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('can_cancel'))
    ) THEN
        RETURN QUERY
        WITH base_results AS (
            SELECT split_part(p_subject_id, '#', 1) AS object_id
            WHERE (p_subject_type = 'run' AND position('#' in p_subject_id) > 0 AND substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('can_cancel'))
        ),
        paged AS (
            SELECT br.object_id
            FROM base_results br
            WHERE (p_after IS NULL OR br.object_id > p_after)
            ORDER BY br.object_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.object_id FROM paged p ORDER BY p.object_id LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT max(r.object_id) FROM returned r)
            END AS next_cursor
        )
        SELECT r.object_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
        RETURN;
    END IF;
    -- Type guard: only return results if subject type is allowed
    -- Skip the guard for userset subjects since composed inner calls handle userset subjects
    IF (position('#' in p_subject_id) = 0 AND p_subject_type NOT IN ('user')) THEN
        RETURN;
    END IF;
    RETURN QUERY
        WITH base_results AS (
            -- TTU composition: agent -> agent
                SELECT DISTINCT t.object_id
                FROM melange_tuples AS t
                WHERE (t.object_type = 'run' AND t.relation = 'agent' AND t.subject_type = 'agent' AND t.subject_id IN (SELECT obj.object_id FROM "public"."list_agent_can_execute_obj"(p_subject_type, p_subject_id, NULL, NULL) obj))
        ),
        paged AS (
            SELECT br.object_id
            FROM base_results br
            WHERE (p_after IS NULL OR br.object_id > p_after)
            ORDER BY br.object_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.object_id FROM paged p ORDER BY p.object_id LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT max(r.object_id) FROM returned r)
            END AS next_cursor
        )
        SELECT r.object_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';

-- Generated list_objects function for state_volume.can_attach
-- Features: Recursive
-- Indirect anchor: project.can_write via ttu
CREATE OR REPLACE FUNCTION "public"."list_state_volume_can_attach_obj"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_limit INT DEFAULT NULL,
    p_after TEXT DEFAULT NULL
) RETURNS TABLE(object_id TEXT, next_cursor TEXT) ROWS 100 AS $$
BEGIN
    -- Self-candidate check: when subject is a userset on the same object type
    IF EXISTS (
    SELECT split_part(p_subject_id, '#', 1) AS object_id
    WHERE (p_subject_type = 'state_volume' AND position('#' in p_subject_id) > 0 AND substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('can_attach'))
    ) THEN
        RETURN QUERY
        WITH base_results AS (
            SELECT split_part(p_subject_id, '#', 1) AS object_id
            WHERE (p_subject_type = 'state_volume' AND position('#' in p_subject_id) > 0 AND substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('can_attach'))
        ),
        paged AS (
            SELECT br.object_id
            FROM base_results br
            WHERE (p_after IS NULL OR br.object_id > p_after)
            ORDER BY br.object_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.object_id FROM paged p ORDER BY p.object_id LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT max(r.object_id) FROM returned r)
            END AS next_cursor
        )
        SELECT r.object_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
        RETURN;
    END IF;
    -- Type guard: only return results if subject type is allowed
    -- Skip the guard for userset subjects since composed inner calls handle userset subjects
    IF (position('#' in p_subject_id) = 0 AND p_subject_type NOT IN ('user')) THEN
        RETURN;
    END IF;
    RETURN QUERY
        WITH base_results AS (
            -- TTU composition: agent -> agent
                SELECT DISTINCT t.object_id
                FROM melange_tuples AS t
                WHERE (t.object_type = 'state_volume' AND t.relation = 'agent' AND t.subject_type = 'agent' AND t.subject_id IN (SELECT obj.object_id FROM "public"."list_agent_can_execute_obj"(p_subject_type, p_subject_id, NULL, NULL) obj))
        ),
        paged AS (
            SELECT br.object_id
            FROM base_results br
            WHERE (p_after IS NULL OR br.object_id > p_after)
            ORDER BY br.object_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.object_id FROM paged p ORDER BY p.object_id LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT max(r.object_id) FROM returned r)
            END AS next_cursor
        )
        SELECT r.object_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';

-- Generated list_objects function for project.can_read
-- Features: Implied+Recursive
CREATE OR REPLACE FUNCTION "public"."list_project_can_read_obj"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_limit INT DEFAULT NULL,
    p_after TEXT DEFAULT NULL
) RETURNS TABLE(object_id TEXT, next_cursor TEXT) ROWS 100 AS $$
DECLARE
    v_max_depth INTEGER;
BEGIN
    v_max_depth := 0;
    IF v_max_depth >= 25 THEN
        RAISE EXCEPTION 'resolution too complex' USING ERRCODE = 'M2002';
    END IF;
    RETURN QUERY
        WITH base_results AS (
            WITH RECURSIVE accessible(object_id, depth, propagatable) AS (
                -- Direct tuple lookup with simple closure relations
                    SELECT DISTINCT base.object_id, 0 AS depth, FALSE AS propagatable
                    FROM (
                    SELECT DISTINCT t.object_id
                    FROM melange_tuples AS t
                    WHERE (t.object_type = 'project' AND t.relation IN ('can_read', 'maintainer') AND t.subject_type = p_subject_type AND p_subject_type IN ('user') AND (t.subject_id = p_subject_id AND NOT (t.subject_id = '*')))
                    ) AS base
                    UNION
                    -- Cross-type TTU subject-first: organization -> organization.can_read
                    SELECT DISTINCT base.object_id, 0 AS depth, FALSE AS propagatable
                    FROM (
                    SELECT DISTINCT child.object_id
                    FROM "public"."list_organization_can_read_obj"(p_subject_type, p_subject_id, NULL, NULL) AS parent_obj
                    INNER JOIN melange_tuples AS child ON (child.object_type = 'project' AND child.relation = 'organization' AND child.subject_type = 'organization' AND child.subject_id = parent_obj.object_id)
                    ) AS base
                    UNION
                    -- Cross-type TTU userset-subject parity: organization -> can_read
                    SELECT DISTINCT base.object_id, 0 AS depth, FALSE AS propagatable
                    FROM (
                    SELECT DISTINCT child.object_id
                    FROM melange_tuples AS child
                    WHERE (child.object_type = 'project' AND child.relation IN ('organization') AND child.subject_type IN ('organization') AND position('#' in p_subject_id) > 0 AND "public"."check_permission_internal"(p_subject_type, p_subject_id, 'can_read', child.subject_type, child.subject_id, ARRAY[]::TEXT[]) = 1)
                    ) AS base
            )
            SELECT DISTINCT acc.object_id
            FROM accessible AS acc
            WHERE TRUE
                UNION
            SELECT split_part(p_subject_id, '#', 1)
            WHERE (position('#' in p_subject_id) > 0 AND p_subject_type = 'project' AND substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('can_read', 'maintainer'))
        ),
        paged AS (
            SELECT br.object_id
            FROM base_results br
            WHERE (p_after IS NULL OR br.object_id > p_after)
            ORDER BY br.object_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.object_id FROM paged p ORDER BY p.object_id LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT max(r.object_id) FROM returned r)
            END AS next_cursor
        )
        SELECT r.object_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';

-- Generated list_objects function for agent.can_read
-- Features: Recursive
-- Indirect anchor: project.can_read via ttu
CREATE OR REPLACE FUNCTION "public"."list_agent_can_read_obj"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_limit INT DEFAULT NULL,
    p_after TEXT DEFAULT NULL
) RETURNS TABLE(object_id TEXT, next_cursor TEXT) ROWS 100 AS $$
BEGIN
    -- Self-candidate check: when subject is a userset on the same object type
    IF EXISTS (
    SELECT split_part(p_subject_id, '#', 1) AS object_id
    WHERE (p_subject_type = 'agent' AND position('#' in p_subject_id) > 0 AND substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('can_read'))
    ) THEN
        RETURN QUERY
        WITH base_results AS (
            SELECT split_part(p_subject_id, '#', 1) AS object_id
            WHERE (p_subject_type = 'agent' AND position('#' in p_subject_id) > 0 AND substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('can_read'))
        ),
        paged AS (
            SELECT br.object_id
            FROM base_results br
            WHERE (p_after IS NULL OR br.object_id > p_after)
            ORDER BY br.object_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.object_id FROM paged p ORDER BY p.object_id LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT max(r.object_id) FROM returned r)
            END AS next_cursor
        )
        SELECT r.object_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
        RETURN;
    END IF;
    -- Type guard: only return results if subject type is allowed
    -- Skip the guard for userset subjects since composed inner calls handle userset subjects
    IF (position('#' in p_subject_id) = 0 AND p_subject_type NOT IN ('user')) THEN
        RETURN;
    END IF;
    RETURN QUERY
        WITH base_results AS (
            -- TTU composition: project -> project
                SELECT DISTINCT t.object_id
                FROM melange_tuples AS t
                WHERE (t.object_type = 'agent' AND t.relation = 'project' AND t.subject_type = 'project' AND t.subject_id IN (SELECT obj.object_id FROM "public"."list_project_can_read_obj"(p_subject_type, p_subject_id, NULL, NULL) obj))
        ),
        paged AS (
            SELECT br.object_id
            FROM base_results br
            WHERE (p_after IS NULL OR br.object_id > p_after)
            ORDER BY br.object_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.object_id FROM paged p ORDER BY p.object_id LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT max(r.object_id) FROM returned r)
            END AS next_cursor
        )
        SELECT r.object_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';

-- Generated list_objects function for repository.can_read
-- Features: Implied+Wildcard+Recursive
CREATE OR REPLACE FUNCTION "public"."list_repository_can_read_obj"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_limit INT DEFAULT NULL,
    p_after TEXT DEFAULT NULL
) RETURNS TABLE(object_id TEXT, next_cursor TEXT) ROWS 100 AS $$
DECLARE
    v_max_depth INTEGER;
BEGIN
    v_max_depth := 0;
    IF v_max_depth >= 25 THEN
        RAISE EXCEPTION 'resolution too complex' USING ERRCODE = 'M2002';
    END IF;
    RETURN QUERY
        WITH base_results AS (
            WITH RECURSIVE accessible(object_id, depth, propagatable) AS (
                -- Direct tuple lookup with simple closure relations
                    SELECT DISTINCT base.object_id, 0 AS depth, FALSE AS propagatable
                    FROM (
                    SELECT DISTINCT t.object_id
                    FROM melange_tuples AS t
                    WHERE (t.object_type = 'repository' AND t.relation IN ('can_read', 'public') AND t.subject_type = p_subject_type AND p_subject_type IN ('user') AND (t.subject_id = p_subject_id OR t.subject_id = '*'))
                    ) AS base
                    UNION
                    -- Cross-type TTU subject-first: project -> project.can_read
                    SELECT DISTINCT base.object_id, 0 AS depth, FALSE AS propagatable
                    FROM (
                    SELECT DISTINCT child.object_id
                    FROM "public"."list_project_can_read_obj"(p_subject_type, p_subject_id, NULL, NULL) AS parent_obj
                    INNER JOIN melange_tuples AS child ON (child.object_type = 'repository' AND child.relation = 'project' AND child.subject_type = 'project' AND child.subject_id = parent_obj.object_id)
                    ) AS base
                    UNION
                    -- Cross-type TTU userset-subject parity: project -> can_read
                    SELECT DISTINCT base.object_id, 0 AS depth, FALSE AS propagatable
                    FROM (
                    SELECT DISTINCT child.object_id
                    FROM melange_tuples AS child
                    WHERE (child.object_type = 'repository' AND child.relation IN ('project') AND child.subject_type IN ('project') AND position('#' in p_subject_id) > 0 AND "public"."check_permission_internal"(p_subject_type, p_subject_id, 'can_read', child.subject_type, child.subject_id, ARRAY[]::TEXT[]) = 1)
                    ) AS base
            )
            SELECT DISTINCT acc.object_id
            FROM accessible AS acc
            WHERE TRUE
                UNION
            SELECT split_part(p_subject_id, '#', 1)
            WHERE (position('#' in p_subject_id) > 0 AND p_subject_type = 'repository' AND substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('can_read', 'public'))
        ),
        paged AS (
            SELECT br.object_id
            FROM base_results br
            WHERE (p_after IS NULL OR br.object_id > p_after)
            ORDER BY br.object_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.object_id FROM paged p ORDER BY p.object_id LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT max(r.object_id) FROM returned r)
            END AS next_cursor
        )
        SELECT r.object_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';

-- Generated list_objects function for run.can_read
-- Features: Recursive
-- Indirect anchor: project.can_read via ttu
CREATE OR REPLACE FUNCTION "public"."list_run_can_read_obj"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_limit INT DEFAULT NULL,
    p_after TEXT DEFAULT NULL
) RETURNS TABLE(object_id TEXT, next_cursor TEXT) ROWS 100 AS $$
BEGIN
    -- Self-candidate check: when subject is a userset on the same object type
    IF EXISTS (
    SELECT split_part(p_subject_id, '#', 1) AS object_id
    WHERE (p_subject_type = 'run' AND position('#' in p_subject_id) > 0 AND substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('can_read'))
    ) THEN
        RETURN QUERY
        WITH base_results AS (
            SELECT split_part(p_subject_id, '#', 1) AS object_id
            WHERE (p_subject_type = 'run' AND position('#' in p_subject_id) > 0 AND substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('can_read'))
        ),
        paged AS (
            SELECT br.object_id
            FROM base_results br
            WHERE (p_after IS NULL OR br.object_id > p_after)
            ORDER BY br.object_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.object_id FROM paged p ORDER BY p.object_id LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT max(r.object_id) FROM returned r)
            END AS next_cursor
        )
        SELECT r.object_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
        RETURN;
    END IF;
    -- Type guard: only return results if subject type is allowed
    -- Skip the guard for userset subjects since composed inner calls handle userset subjects
    IF (position('#' in p_subject_id) = 0 AND p_subject_type NOT IN ('user')) THEN
        RETURN;
    END IF;
    RETURN QUERY
        WITH base_results AS (
            -- TTU composition: agent -> agent
                SELECT DISTINCT t.object_id
                FROM melange_tuples AS t
                WHERE (t.object_type = 'run' AND t.relation = 'agent' AND t.subject_type = 'agent' AND t.subject_id IN (SELECT obj.object_id FROM "public"."list_agent_can_read_obj"(p_subject_type, p_subject_id, NULL, NULL) obj))
        ),
        paged AS (
            SELECT br.object_id
            FROM base_results br
            WHERE (p_after IS NULL OR br.object_id > p_after)
            ORDER BY br.object_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.object_id FROM paged p ORDER BY p.object_id LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT max(r.object_id) FROM returned r)
            END AS next_cursor
        )
        SELECT r.object_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';

-- Generated list_objects function for state_volume.can_read
-- Features: Recursive
-- Indirect anchor: project.can_read via ttu
CREATE OR REPLACE FUNCTION "public"."list_state_volume_can_read_obj"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_limit INT DEFAULT NULL,
    p_after TEXT DEFAULT NULL
) RETURNS TABLE(object_id TEXT, next_cursor TEXT) ROWS 100 AS $$
BEGIN
    -- Self-candidate check: when subject is a userset on the same object type
    IF EXISTS (
    SELECT split_part(p_subject_id, '#', 1) AS object_id
    WHERE (p_subject_type = 'state_volume' AND position('#' in p_subject_id) > 0 AND substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('can_read'))
    ) THEN
        RETURN QUERY
        WITH base_results AS (
            SELECT split_part(p_subject_id, '#', 1) AS object_id
            WHERE (p_subject_type = 'state_volume' AND position('#' in p_subject_id) > 0 AND substring(p_subject_id from position('#' in p_subject_id) + 1) IN ('can_read'))
        ),
        paged AS (
            SELECT br.object_id
            FROM base_results br
            WHERE (p_after IS NULL OR br.object_id > p_after)
            ORDER BY br.object_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.object_id FROM paged p ORDER BY p.object_id LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT max(r.object_id) FROM returned r)
            END AS next_cursor
        )
        SELECT r.object_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
        RETURN;
    END IF;
    -- Type guard: only return results if subject type is allowed
    -- Skip the guard for userset subjects since composed inner calls handle userset subjects
    IF (position('#' in p_subject_id) = 0 AND p_subject_type NOT IN ('user')) THEN
        RETURN;
    END IF;
    RETURN QUERY
        WITH base_results AS (
            -- TTU composition: agent -> agent
                SELECT DISTINCT t.object_id
                FROM melange_tuples AS t
                WHERE (t.object_type = 'state_volume' AND t.relation = 'agent' AND t.subject_type = 'agent' AND t.subject_id IN (SELECT obj.object_id FROM "public"."list_agent_can_read_obj"(p_subject_type, p_subject_id, NULL, NULL) obj))
        ),
        paged AS (
            SELECT br.object_id
            FROM base_results br
            WHERE (p_after IS NULL OR br.object_id > p_after)
            ORDER BY br.object_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.object_id FROM paged p ORDER BY p.object_id LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT max(r.object_id) FROM returned r)
            END AS next_cursor
        )
        SELECT r.object_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';

-- ============================================================
-- List Subjects Functions (31 functions)
-- ============================================================

-- Generated list_subjects function for agent.project
-- Features: Direct
CREATE OR REPLACE FUNCTION "public"."list_agent_project_sub"(
    p_object_id TEXT,
    p_subject_type TEXT,
    p_limit INT DEFAULT NULL,
    p_after TEXT DEFAULT NULL
) RETURNS TABLE(subject_id TEXT, next_cursor TEXT) ROWS 100 AS $$
DECLARE
    v_filter_type TEXT;
    v_filter_relation TEXT;
BEGIN
    -- Check if subject_type is a userset filter (e.g., "document#viewer")
    IF position('#' in p_subject_type) > 0 THEN
        v_filter_type := substring(p_subject_type from 1 for position('#' in p_subject_type) - 1);
        v_filter_relation := substring(p_subject_type from position('#' in p_subject_type) + 1);
        RETURN QUERY
        WITH base_results AS (
            -- Userset filter: find userset tuples that match and return normalized references
                SELECT DISTINCT split_part(t.subject_id, '#', 1) || '#' || v_filter_relation AS subject_id
                		FROM melange_tuples AS t
                		WHERE (t.object_type = 'agent' AND t.relation IN ('project') AND t.object_id = p_object_id AND t.subject_type = v_filter_type AND position('#' in t.subject_id) > 0 AND (substring(t.subject_id from position('#' in t.subject_id) + 1) = v_filter_relation OR EXISTS (
                SELECT 1
                FROM (VALUES ('agent', 'can_execute', 'can_execute'), ('agent', 'can_manage', 'can_manage'), ('agent', 'can_read', 'can_read'), ('agent', 'project', 'project'), ('project', 'can_delete', 'can_delete'), ('project', 'can_manage', 'can_manage'), ('project', 'can_manage', 'maintainer'), ('project', 'can_read', 'can_read'), ('project', 'can_read', 'maintainer'), ('project', 'can_write', 'can_write'), ('project', 'can_write', 'maintainer'), ('project', 'maintainer', 'maintainer'), ('project', 'organization', 'organization')) AS subj_c(object_type, relation, satisfying_relation)
                WHERE (subj_c.object_type = v_filter_type AND subj_c.relation = substring(t.subject_id from position('#' in t.subject_id) + 1) AND subj_c.satisfying_relation = v_filter_relation)
                )) AND "public"."check_permission_internal"(v_filter_type, t.subject_id, 'project', 'agent', p_object_id, ARRAY[]::TEXT[]) = 1)
                UNION
                -- Self-candidate: when filter type matches object type
                -- e.g., querying document:1.viewer with filter document#writer
                -- should return document:1#writer if writer satisfies the relation
                SELECT p_object_id || '#' || v_filter_relation AS subject_id
                		WHERE (v_filter_type = 'agent' AND EXISTS (
                SELECT 1
                FROM (VALUES ('agent', 'can_execute', 'can_execute'), ('agent', 'can_manage', 'can_manage'), ('agent', 'can_read', 'can_read'), ('agent', 'project', 'project'), ('project', 'can_delete', 'can_delete'), ('project', 'can_manage', 'can_manage'), ('project', 'can_manage', 'maintainer'), ('project', 'can_read', 'can_read'), ('project', 'can_read', 'maintainer'), ('project', 'can_write', 'can_write'), ('project', 'can_write', 'maintainer'), ('project', 'maintainer', 'maintainer'), ('project', 'organization', 'organization')) AS c(object_type, relation, satisfying_relation)
                WHERE (c.object_type = 'agent' AND c.relation = 'project' AND c.satisfying_relation = v_filter_relation)
                ))
        ),
        paged AS (
            SELECT br.subject_id
            FROM base_results br
            WHERE p_after IS NULL OR (
                -- Compound comparison for wildcard-first ordering:
                -- (is_not_wildcard, subject_id) > (cursor_is_not_wildcard, cursor)
                (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END, br.subject_id) >
                (CASE WHEN p_after = '*' THEN 0 ELSE 1 END, p_after)
            )
            ORDER BY (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END), br.subject_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.subject_id FROM paged p
            ORDER BY (CASE WHEN p.subject_id = '*' THEN 0 ELSE 1 END), p.subject_id
            LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT r.subject_id FROM returned r
                      ORDER BY (CASE WHEN r.subject_id = '*' THEN 0 ELSE 1 END) DESC, r.subject_id DESC
                      LIMIT 1)
            END AS next_cursor
        )
        SELECT r.subject_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
    ELSE
        -- Guard: return empty if subject type is not allowed by the model
        IF p_subject_type NOT IN ('project') THEN
        RETURN;
    END IF;
        -- Regular subject type (no userset filter)
        RETURN QUERY
        WITH base_results AS (
            -- Path 1: Direct tuple lookup with simple closure relations
                SELECT DISTINCT t.subject_id
                FROM melange_tuples AS t
                WHERE (t.object_type = 'agent' AND t.relation IN ('project') AND t.object_id = p_object_id AND t.subject_type = p_subject_type AND position('#' in t.subject_id) = 0 AND t.subject_id <> '*')
        ),
        paged AS (
            SELECT br.subject_id
            FROM base_results br
            WHERE p_after IS NULL OR (
                -- Compound comparison for wildcard-first ordering:
                -- (is_not_wildcard, subject_id) > (cursor_is_not_wildcard, cursor)
                (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END, br.subject_id) >
                (CASE WHEN p_after = '*' THEN 0 ELSE 1 END, p_after)
            )
            ORDER BY (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END), br.subject_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.subject_id FROM paged p
            ORDER BY (CASE WHEN p.subject_id = '*' THEN 0 ELSE 1 END), p.subject_id
            LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT r.subject_id FROM returned r
                      ORDER BY (CASE WHEN r.subject_id = '*' THEN 0 ELSE 1 END) DESC, r.subject_id DESC
                      LIMIT 1)
            END AS next_cursor
        )
        SELECT r.subject_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
    END IF;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';

-- Generated list_subjects function for organization.owner
-- Features: Direct
CREATE OR REPLACE FUNCTION "public"."list_organization_owner_sub"(
    p_object_id TEXT,
    p_subject_type TEXT,
    p_limit INT DEFAULT NULL,
    p_after TEXT DEFAULT NULL
) RETURNS TABLE(subject_id TEXT, next_cursor TEXT) ROWS 100 AS $$
DECLARE
    v_filter_type TEXT;
    v_filter_relation TEXT;
BEGIN
    -- Check if subject_type is a userset filter (e.g., "document#viewer")
    IF position('#' in p_subject_type) > 0 THEN
        v_filter_type := substring(p_subject_type from 1 for position('#' in p_subject_type) - 1);
        v_filter_relation := substring(p_subject_type from position('#' in p_subject_type) + 1);
        RETURN QUERY
        WITH base_results AS (
            -- Userset filter: find userset tuples that match and return normalized references
                SELECT DISTINCT split_part(t.subject_id, '#', 1) || '#' || v_filter_relation AS subject_id
                		FROM melange_tuples AS t
                		WHERE (t.object_type = 'organization' AND t.relation IN ('owner') AND t.object_id = p_object_id AND t.subject_type = v_filter_type AND position('#' in t.subject_id) > 0 AND (substring(t.subject_id from position('#' in t.subject_id) + 1) = v_filter_relation OR EXISTS (
                SELECT 1
                FROM (VALUES ('organization', 'admin', 'admin'), ('organization', 'admin', 'owner'), ('organization', 'can_create_project', 'admin'), ('organization', 'can_create_project', 'can_create_project'), ('organization', 'can_create_project', 'owner'), ('organization', 'can_delete', 'can_delete'), ('organization', 'can_delete', 'owner'), ('organization', 'can_manage', 'can_manage'), ('organization', 'can_manage', 'owner'), ('organization', 'can_manage_members', 'admin'), ('organization', 'can_manage_members', 'can_manage_members'), ('organization', 'can_manage_members', 'owner'), ('organization', 'can_read', 'admin'), ('organization', 'can_read', 'can_read'), ('organization', 'can_read', 'member'), ('organization', 'can_read', 'owner'), ('organization', 'member', 'admin'), ('organization', 'member', 'member'), ('organization', 'member', 'owner'), ('organization', 'owner', 'owner')) AS subj_c(object_type, relation, satisfying_relation)
                WHERE (subj_c.object_type = v_filter_type AND subj_c.relation = substring(t.subject_id from position('#' in t.subject_id) + 1) AND subj_c.satisfying_relation = v_filter_relation)
                )) AND "public"."check_permission_internal"(v_filter_type, t.subject_id, 'owner', 'organization', p_object_id, ARRAY[]::TEXT[]) = 1)
                UNION
                -- Self-candidate: when filter type matches object type
                -- e.g., querying document:1.viewer with filter document#writer
                -- should return document:1#writer if writer satisfies the relation
                SELECT p_object_id || '#' || v_filter_relation AS subject_id
                		WHERE (v_filter_type = 'organization' AND EXISTS (
                SELECT 1
                FROM (VALUES ('organization', 'admin', 'admin'), ('organization', 'admin', 'owner'), ('organization', 'can_create_project', 'admin'), ('organization', 'can_create_project', 'can_create_project'), ('organization', 'can_create_project', 'owner'), ('organization', 'can_delete', 'can_delete'), ('organization', 'can_delete', 'owner'), ('organization', 'can_manage', 'can_manage'), ('organization', 'can_manage', 'owner'), ('organization', 'can_manage_members', 'admin'), ('organization', 'can_manage_members', 'can_manage_members'), ('organization', 'can_manage_members', 'owner'), ('organization', 'can_read', 'admin'), ('organization', 'can_read', 'can_read'), ('organization', 'can_read', 'member'), ('organization', 'can_read', 'owner'), ('organization', 'member', 'admin'), ('organization', 'member', 'member'), ('organization', 'member', 'owner'), ('organization', 'owner', 'owner')) AS c(object_type, relation, satisfying_relation)
                WHERE (c.object_type = 'organization' AND c.relation = 'owner' AND c.satisfying_relation = v_filter_relation)
                ))
        ),
        paged AS (
            SELECT br.subject_id
            FROM base_results br
            WHERE p_after IS NULL OR (
                -- Compound comparison for wildcard-first ordering:
                -- (is_not_wildcard, subject_id) > (cursor_is_not_wildcard, cursor)
                (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END, br.subject_id) >
                (CASE WHEN p_after = '*' THEN 0 ELSE 1 END, p_after)
            )
            ORDER BY (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END), br.subject_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.subject_id FROM paged p
            ORDER BY (CASE WHEN p.subject_id = '*' THEN 0 ELSE 1 END), p.subject_id
            LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT r.subject_id FROM returned r
                      ORDER BY (CASE WHEN r.subject_id = '*' THEN 0 ELSE 1 END) DESC, r.subject_id DESC
                      LIMIT 1)
            END AS next_cursor
        )
        SELECT r.subject_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
    ELSE
        -- Guard: return empty if subject type is not allowed by the model
        IF p_subject_type NOT IN ('user') THEN
        RETURN;
    END IF;
        -- Regular subject type (no userset filter)
        RETURN QUERY
        WITH base_results AS (
            -- Path 1: Direct tuple lookup with simple closure relations
                SELECT DISTINCT t.subject_id
                FROM melange_tuples AS t
                WHERE (t.object_type = 'organization' AND t.relation IN ('owner') AND t.object_id = p_object_id AND t.subject_type = p_subject_type AND position('#' in t.subject_id) = 0 AND t.subject_id <> '*')
        ),
        paged AS (
            SELECT br.subject_id
            FROM base_results br
            WHERE p_after IS NULL OR (
                -- Compound comparison for wildcard-first ordering:
                -- (is_not_wildcard, subject_id) > (cursor_is_not_wildcard, cursor)
                (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END, br.subject_id) >
                (CASE WHEN p_after = '*' THEN 0 ELSE 1 END, p_after)
            )
            ORDER BY (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END), br.subject_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.subject_id FROM paged p
            ORDER BY (CASE WHEN p.subject_id = '*' THEN 0 ELSE 1 END), p.subject_id
            LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT r.subject_id FROM returned r
                      ORDER BY (CASE WHEN r.subject_id = '*' THEN 0 ELSE 1 END) DESC, r.subject_id DESC
                      LIMIT 1)
            END AS next_cursor
        )
        SELECT r.subject_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
    END IF;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';

-- Generated list_subjects function for project.maintainer
-- Features: Direct
CREATE OR REPLACE FUNCTION "public"."list_project_maintainer_sub"(
    p_object_id TEXT,
    p_subject_type TEXT,
    p_limit INT DEFAULT NULL,
    p_after TEXT DEFAULT NULL
) RETURNS TABLE(subject_id TEXT, next_cursor TEXT) ROWS 100 AS $$
DECLARE
    v_filter_type TEXT;
    v_filter_relation TEXT;
BEGIN
    -- Check if subject_type is a userset filter (e.g., "document#viewer")
    IF position('#' in p_subject_type) > 0 THEN
        v_filter_type := substring(p_subject_type from 1 for position('#' in p_subject_type) - 1);
        v_filter_relation := substring(p_subject_type from position('#' in p_subject_type) + 1);
        RETURN QUERY
        WITH base_results AS (
            -- Userset filter: find userset tuples that match and return normalized references
                SELECT DISTINCT split_part(t.subject_id, '#', 1) || '#' || v_filter_relation AS subject_id
                		FROM melange_tuples AS t
                		WHERE (t.object_type = 'project' AND t.relation IN ('maintainer') AND t.object_id = p_object_id AND t.subject_type = v_filter_type AND position('#' in t.subject_id) > 0 AND (substring(t.subject_id from position('#' in t.subject_id) + 1) = v_filter_relation OR EXISTS (
                SELECT 1
                FROM (VALUES ('project', 'can_delete', 'can_delete'), ('project', 'can_manage', 'can_manage'), ('project', 'can_manage', 'maintainer'), ('project', 'can_read', 'can_read'), ('project', 'can_read', 'maintainer'), ('project', 'can_write', 'can_write'), ('project', 'can_write', 'maintainer'), ('project', 'maintainer', 'maintainer'), ('project', 'organization', 'organization')) AS subj_c(object_type, relation, satisfying_relation)
                WHERE (subj_c.object_type = v_filter_type AND subj_c.relation = substring(t.subject_id from position('#' in t.subject_id) + 1) AND subj_c.satisfying_relation = v_filter_relation)
                )) AND "public"."check_permission_internal"(v_filter_type, t.subject_id, 'maintainer', 'project', p_object_id, ARRAY[]::TEXT[]) = 1)
                UNION
                -- Self-candidate: when filter type matches object type
                -- e.g., querying document:1.viewer with filter document#writer
                -- should return document:1#writer if writer satisfies the relation
                SELECT p_object_id || '#' || v_filter_relation AS subject_id
                		WHERE (v_filter_type = 'project' AND EXISTS (
                SELECT 1
                FROM (VALUES ('project', 'can_delete', 'can_delete'), ('project', 'can_manage', 'can_manage'), ('project', 'can_manage', 'maintainer'), ('project', 'can_read', 'can_read'), ('project', 'can_read', 'maintainer'), ('project', 'can_write', 'can_write'), ('project', 'can_write', 'maintainer'), ('project', 'maintainer', 'maintainer'), ('project', 'organization', 'organization')) AS c(object_type, relation, satisfying_relation)
                WHERE (c.object_type = 'project' AND c.relation = 'maintainer' AND c.satisfying_relation = v_filter_relation)
                ))
        ),
        paged AS (
            SELECT br.subject_id
            FROM base_results br
            WHERE p_after IS NULL OR (
                -- Compound comparison for wildcard-first ordering:
                -- (is_not_wildcard, subject_id) > (cursor_is_not_wildcard, cursor)
                (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END, br.subject_id) >
                (CASE WHEN p_after = '*' THEN 0 ELSE 1 END, p_after)
            )
            ORDER BY (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END), br.subject_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.subject_id FROM paged p
            ORDER BY (CASE WHEN p.subject_id = '*' THEN 0 ELSE 1 END), p.subject_id
            LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT r.subject_id FROM returned r
                      ORDER BY (CASE WHEN r.subject_id = '*' THEN 0 ELSE 1 END) DESC, r.subject_id DESC
                      LIMIT 1)
            END AS next_cursor
        )
        SELECT r.subject_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
    ELSE
        -- Guard: return empty if subject type is not allowed by the model
        IF p_subject_type NOT IN ('user') THEN
        RETURN;
    END IF;
        -- Regular subject type (no userset filter)
        RETURN QUERY
        WITH base_results AS (
            -- Path 1: Direct tuple lookup with simple closure relations
                SELECT DISTINCT t.subject_id
                FROM melange_tuples AS t
                WHERE (t.object_type = 'project' AND t.relation IN ('maintainer') AND t.object_id = p_object_id AND t.subject_type = p_subject_type AND position('#' in t.subject_id) = 0 AND t.subject_id <> '*')
        ),
        paged AS (
            SELECT br.subject_id
            FROM base_results br
            WHERE p_after IS NULL OR (
                -- Compound comparison for wildcard-first ordering:
                -- (is_not_wildcard, subject_id) > (cursor_is_not_wildcard, cursor)
                (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END, br.subject_id) >
                (CASE WHEN p_after = '*' THEN 0 ELSE 1 END, p_after)
            )
            ORDER BY (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END), br.subject_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.subject_id FROM paged p
            ORDER BY (CASE WHEN p.subject_id = '*' THEN 0 ELSE 1 END), p.subject_id
            LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT r.subject_id FROM returned r
                      ORDER BY (CASE WHEN r.subject_id = '*' THEN 0 ELSE 1 END) DESC, r.subject_id DESC
                      LIMIT 1)
            END AS next_cursor
        )
        SELECT r.subject_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
    END IF;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';

-- Generated list_subjects function for project.organization
-- Features: Direct
CREATE OR REPLACE FUNCTION "public"."list_project_organization_sub"(
    p_object_id TEXT,
    p_subject_type TEXT,
    p_limit INT DEFAULT NULL,
    p_after TEXT DEFAULT NULL
) RETURNS TABLE(subject_id TEXT, next_cursor TEXT) ROWS 100 AS $$
DECLARE
    v_filter_type TEXT;
    v_filter_relation TEXT;
BEGIN
    -- Check if subject_type is a userset filter (e.g., "document#viewer")
    IF position('#' in p_subject_type) > 0 THEN
        v_filter_type := substring(p_subject_type from 1 for position('#' in p_subject_type) - 1);
        v_filter_relation := substring(p_subject_type from position('#' in p_subject_type) + 1);
        RETURN QUERY
        WITH base_results AS (
            -- Userset filter: find userset tuples that match and return normalized references
                SELECT DISTINCT split_part(t.subject_id, '#', 1) || '#' || v_filter_relation AS subject_id
                		FROM melange_tuples AS t
                		WHERE (t.object_type = 'project' AND t.relation IN ('organization') AND t.object_id = p_object_id AND t.subject_type = v_filter_type AND position('#' in t.subject_id) > 0 AND (substring(t.subject_id from position('#' in t.subject_id) + 1) = v_filter_relation OR EXISTS (
                SELECT 1
                FROM (VALUES ('organization', 'admin', 'admin'), ('organization', 'admin', 'owner'), ('organization', 'can_create_project', 'admin'), ('organization', 'can_create_project', 'can_create_project'), ('organization', 'can_create_project', 'owner'), ('organization', 'can_delete', 'can_delete'), ('organization', 'can_delete', 'owner'), ('organization', 'can_manage', 'can_manage'), ('organization', 'can_manage', 'owner'), ('organization', 'can_manage_members', 'admin'), ('organization', 'can_manage_members', 'can_manage_members'), ('organization', 'can_manage_members', 'owner'), ('organization', 'can_read', 'admin'), ('organization', 'can_read', 'can_read'), ('organization', 'can_read', 'member'), ('organization', 'can_read', 'owner'), ('organization', 'member', 'admin'), ('organization', 'member', 'member'), ('organization', 'member', 'owner'), ('organization', 'owner', 'owner'), ('project', 'can_delete', 'can_delete'), ('project', 'can_manage', 'can_manage'), ('project', 'can_manage', 'maintainer'), ('project', 'can_read', 'can_read'), ('project', 'can_read', 'maintainer'), ('project', 'can_write', 'can_write'), ('project', 'can_write', 'maintainer'), ('project', 'maintainer', 'maintainer'), ('project', 'organization', 'organization')) AS subj_c(object_type, relation, satisfying_relation)
                WHERE (subj_c.object_type = v_filter_type AND subj_c.relation = substring(t.subject_id from position('#' in t.subject_id) + 1) AND subj_c.satisfying_relation = v_filter_relation)
                )) AND "public"."check_permission_internal"(v_filter_type, t.subject_id, 'organization', 'project', p_object_id, ARRAY[]::TEXT[]) = 1)
                UNION
                -- Self-candidate: when filter type matches object type
                -- e.g., querying document:1.viewer with filter document#writer
                -- should return document:1#writer if writer satisfies the relation
                SELECT p_object_id || '#' || v_filter_relation AS subject_id
                		WHERE (v_filter_type = 'project' AND EXISTS (
                SELECT 1
                FROM (VALUES ('organization', 'admin', 'admin'), ('organization', 'admin', 'owner'), ('organization', 'can_create_project', 'admin'), ('organization', 'can_create_project', 'can_create_project'), ('organization', 'can_create_project', 'owner'), ('organization', 'can_delete', 'can_delete'), ('organization', 'can_delete', 'owner'), ('organization', 'can_manage', 'can_manage'), ('organization', 'can_manage', 'owner'), ('organization', 'can_manage_members', 'admin'), ('organization', 'can_manage_members', 'can_manage_members'), ('organization', 'can_manage_members', 'owner'), ('organization', 'can_read', 'admin'), ('organization', 'can_read', 'can_read'), ('organization', 'can_read', 'member'), ('organization', 'can_read', 'owner'), ('organization', 'member', 'admin'), ('organization', 'member', 'member'), ('organization', 'member', 'owner'), ('organization', 'owner', 'owner'), ('project', 'can_delete', 'can_delete'), ('project', 'can_manage', 'can_manage'), ('project', 'can_manage', 'maintainer'), ('project', 'can_read', 'can_read'), ('project', 'can_read', 'maintainer'), ('project', 'can_write', 'can_write'), ('project', 'can_write', 'maintainer'), ('project', 'maintainer', 'maintainer'), ('project', 'organization', 'organization')) AS c(object_type, relation, satisfying_relation)
                WHERE (c.object_type = 'project' AND c.relation = 'organization' AND c.satisfying_relation = v_filter_relation)
                ))
        ),
        paged AS (
            SELECT br.subject_id
            FROM base_results br
            WHERE p_after IS NULL OR (
                -- Compound comparison for wildcard-first ordering:
                -- (is_not_wildcard, subject_id) > (cursor_is_not_wildcard, cursor)
                (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END, br.subject_id) >
                (CASE WHEN p_after = '*' THEN 0 ELSE 1 END, p_after)
            )
            ORDER BY (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END), br.subject_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.subject_id FROM paged p
            ORDER BY (CASE WHEN p.subject_id = '*' THEN 0 ELSE 1 END), p.subject_id
            LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT r.subject_id FROM returned r
                      ORDER BY (CASE WHEN r.subject_id = '*' THEN 0 ELSE 1 END) DESC, r.subject_id DESC
                      LIMIT 1)
            END AS next_cursor
        )
        SELECT r.subject_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
    ELSE
        -- Guard: return empty if subject type is not allowed by the model
        IF p_subject_type NOT IN ('organization') THEN
        RETURN;
    END IF;
        -- Regular subject type (no userset filter)
        RETURN QUERY
        WITH base_results AS (
            -- Path 1: Direct tuple lookup with simple closure relations
                SELECT DISTINCT t.subject_id
                FROM melange_tuples AS t
                WHERE (t.object_type = 'project' AND t.relation IN ('organization') AND t.object_id = p_object_id AND t.subject_type = p_subject_type AND position('#' in t.subject_id) = 0 AND t.subject_id <> '*')
        ),
        paged AS (
            SELECT br.subject_id
            FROM base_results br
            WHERE p_after IS NULL OR (
                -- Compound comparison for wildcard-first ordering:
                -- (is_not_wildcard, subject_id) > (cursor_is_not_wildcard, cursor)
                (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END, br.subject_id) >
                (CASE WHEN p_after = '*' THEN 0 ELSE 1 END, p_after)
            )
            ORDER BY (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END), br.subject_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.subject_id FROM paged p
            ORDER BY (CASE WHEN p.subject_id = '*' THEN 0 ELSE 1 END), p.subject_id
            LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT r.subject_id FROM returned r
                      ORDER BY (CASE WHEN r.subject_id = '*' THEN 0 ELSE 1 END) DESC, r.subject_id DESC
                      LIMIT 1)
            END AS next_cursor
        )
        SELECT r.subject_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
    END IF;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';

-- Generated list_subjects function for repository.project
-- Features: Direct
CREATE OR REPLACE FUNCTION "public"."list_repository_project_sub"(
    p_object_id TEXT,
    p_subject_type TEXT,
    p_limit INT DEFAULT NULL,
    p_after TEXT DEFAULT NULL
) RETURNS TABLE(subject_id TEXT, next_cursor TEXT) ROWS 100 AS $$
DECLARE
    v_filter_type TEXT;
    v_filter_relation TEXT;
BEGIN
    -- Check if subject_type is a userset filter (e.g., "document#viewer")
    IF position('#' in p_subject_type) > 0 THEN
        v_filter_type := substring(p_subject_type from 1 for position('#' in p_subject_type) - 1);
        v_filter_relation := substring(p_subject_type from position('#' in p_subject_type) + 1);
        RETURN QUERY
        WITH base_results AS (
            -- Userset filter: find userset tuples that match and return normalized references
                SELECT DISTINCT split_part(t.subject_id, '#', 1) || '#' || v_filter_relation AS subject_id
                		FROM melange_tuples AS t
                		WHERE (t.object_type = 'repository' AND t.relation IN ('project') AND t.object_id = p_object_id AND t.subject_type = v_filter_type AND position('#' in t.subject_id) > 0 AND (substring(t.subject_id from position('#' in t.subject_id) + 1) = v_filter_relation OR EXISTS (
                SELECT 1
                FROM (VALUES ('project', 'can_delete', 'can_delete'), ('project', 'can_manage', 'can_manage'), ('project', 'can_manage', 'maintainer'), ('project', 'can_read', 'can_read'), ('project', 'can_read', 'maintainer'), ('project', 'can_write', 'can_write'), ('project', 'can_write', 'maintainer'), ('project', 'maintainer', 'maintainer'), ('project', 'organization', 'organization'), ('repository', 'can_delete', 'can_delete'), ('repository', 'can_read', 'can_read'), ('repository', 'can_read', 'public'), ('repository', 'can_write', 'can_write'), ('repository', 'project', 'project'), ('repository', 'public', 'public')) AS subj_c(object_type, relation, satisfying_relation)
                WHERE (subj_c.object_type = v_filter_type AND subj_c.relation = substring(t.subject_id from position('#' in t.subject_id) + 1) AND subj_c.satisfying_relation = v_filter_relation)
                )) AND "public"."check_permission_internal"(v_filter_type, t.subject_id, 'project', 'repository', p_object_id, ARRAY[]::TEXT[]) = 1)
                UNION
                -- Self-candidate: when filter type matches object type
                -- e.g., querying document:1.viewer with filter document#writer
                -- should return document:1#writer if writer satisfies the relation
                SELECT p_object_id || '#' || v_filter_relation AS subject_id
                		WHERE (v_filter_type = 'repository' AND EXISTS (
                SELECT 1
                FROM (VALUES ('project', 'can_delete', 'can_delete'), ('project', 'can_manage', 'can_manage'), ('project', 'can_manage', 'maintainer'), ('project', 'can_read', 'can_read'), ('project', 'can_read', 'maintainer'), ('project', 'can_write', 'can_write'), ('project', 'can_write', 'maintainer'), ('project', 'maintainer', 'maintainer'), ('project', 'organization', 'organization'), ('repository', 'can_delete', 'can_delete'), ('repository', 'can_read', 'can_read'), ('repository', 'can_read', 'public'), ('repository', 'can_write', 'can_write'), ('repository', 'project', 'project'), ('repository', 'public', 'public')) AS c(object_type, relation, satisfying_relation)
                WHERE (c.object_type = 'repository' AND c.relation = 'project' AND c.satisfying_relation = v_filter_relation)
                ))
        ),
        paged AS (
            SELECT br.subject_id
            FROM base_results br
            WHERE p_after IS NULL OR (
                -- Compound comparison for wildcard-first ordering:
                -- (is_not_wildcard, subject_id) > (cursor_is_not_wildcard, cursor)
                (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END, br.subject_id) >
                (CASE WHEN p_after = '*' THEN 0 ELSE 1 END, p_after)
            )
            ORDER BY (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END), br.subject_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.subject_id FROM paged p
            ORDER BY (CASE WHEN p.subject_id = '*' THEN 0 ELSE 1 END), p.subject_id
            LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT r.subject_id FROM returned r
                      ORDER BY (CASE WHEN r.subject_id = '*' THEN 0 ELSE 1 END) DESC, r.subject_id DESC
                      LIMIT 1)
            END AS next_cursor
        )
        SELECT r.subject_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
    ELSE
        -- Guard: return empty if subject type is not allowed by the model
        IF p_subject_type NOT IN ('project') THEN
        RETURN;
    END IF;
        -- Regular subject type (no userset filter)
        RETURN QUERY
        WITH base_results AS (
            -- Path 1: Direct tuple lookup with simple closure relations
                SELECT DISTINCT t.subject_id
                FROM melange_tuples AS t
                WHERE (t.object_type = 'repository' AND t.relation IN ('project') AND t.object_id = p_object_id AND t.subject_type = p_subject_type AND position('#' in t.subject_id) = 0 AND t.subject_id <> '*')
        ),
        paged AS (
            SELECT br.subject_id
            FROM base_results br
            WHERE p_after IS NULL OR (
                -- Compound comparison for wildcard-first ordering:
                -- (is_not_wildcard, subject_id) > (cursor_is_not_wildcard, cursor)
                (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END, br.subject_id) >
                (CASE WHEN p_after = '*' THEN 0 ELSE 1 END, p_after)
            )
            ORDER BY (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END), br.subject_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.subject_id FROM paged p
            ORDER BY (CASE WHEN p.subject_id = '*' THEN 0 ELSE 1 END), p.subject_id
            LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT r.subject_id FROM returned r
                      ORDER BY (CASE WHEN r.subject_id = '*' THEN 0 ELSE 1 END) DESC, r.subject_id DESC
                      LIMIT 1)
            END AS next_cursor
        )
        SELECT r.subject_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
    END IF;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';

-- Generated list_subjects function for repository.public
-- Features: Direct+Wildcard
CREATE OR REPLACE FUNCTION "public"."list_repository_public_sub"(
    p_object_id TEXT,
    p_subject_type TEXT,
    p_limit INT DEFAULT NULL,
    p_after TEXT DEFAULT NULL
) RETURNS TABLE(subject_id TEXT, next_cursor TEXT) ROWS 100 AS $$
DECLARE
    v_filter_type TEXT;
    v_filter_relation TEXT;
BEGIN
    -- Check if subject_type is a userset filter (e.g., "document#viewer")
    IF position('#' in p_subject_type) > 0 THEN
        v_filter_type := substring(p_subject_type from 1 for position('#' in p_subject_type) - 1);
        v_filter_relation := substring(p_subject_type from position('#' in p_subject_type) + 1);
        RETURN QUERY
        WITH base_results AS (
            -- Userset filter: find userset tuples that match and return normalized references
                SELECT DISTINCT split_part(t.subject_id, '#', 1) || '#' || v_filter_relation AS subject_id
                		FROM melange_tuples AS t
                		WHERE (t.object_type = 'repository' AND t.relation IN ('public') AND t.object_id = p_object_id AND t.subject_type = v_filter_type AND position('#' in t.subject_id) > 0 AND (substring(t.subject_id from position('#' in t.subject_id) + 1) = v_filter_relation OR EXISTS (
                SELECT 1
                FROM (VALUES ('repository', 'can_delete', 'can_delete'), ('repository', 'can_read', 'can_read'), ('repository', 'can_read', 'public'), ('repository', 'can_write', 'can_write'), ('repository', 'project', 'project'), ('repository', 'public', 'public')) AS subj_c(object_type, relation, satisfying_relation)
                WHERE (subj_c.object_type = v_filter_type AND subj_c.relation = substring(t.subject_id from position('#' in t.subject_id) + 1) AND subj_c.satisfying_relation = v_filter_relation)
                )) AND "public"."check_permission_internal"(v_filter_type, t.subject_id, 'public', 'repository', p_object_id, ARRAY[]::TEXT[]) = 1)
                UNION
                -- Self-candidate: when filter type matches object type
                -- e.g., querying document:1.viewer with filter document#writer
                -- should return document:1#writer if writer satisfies the relation
                SELECT p_object_id || '#' || v_filter_relation AS subject_id
                		WHERE (v_filter_type = 'repository' AND EXISTS (
                SELECT 1
                FROM (VALUES ('repository', 'can_delete', 'can_delete'), ('repository', 'can_read', 'can_read'), ('repository', 'can_read', 'public'), ('repository', 'can_write', 'can_write'), ('repository', 'project', 'project'), ('repository', 'public', 'public')) AS c(object_type, relation, satisfying_relation)
                WHERE (c.object_type = 'repository' AND c.relation = 'public' AND c.satisfying_relation = v_filter_relation)
                ))
        ),
        paged AS (
            SELECT br.subject_id
            FROM base_results br
            WHERE p_after IS NULL OR (
                -- Compound comparison for wildcard-first ordering:
                -- (is_not_wildcard, subject_id) > (cursor_is_not_wildcard, cursor)
                (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END, br.subject_id) >
                (CASE WHEN p_after = '*' THEN 0 ELSE 1 END, p_after)
            )
            ORDER BY (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END), br.subject_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.subject_id FROM paged p
            ORDER BY (CASE WHEN p.subject_id = '*' THEN 0 ELSE 1 END), p.subject_id
            LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT r.subject_id FROM returned r
                      ORDER BY (CASE WHEN r.subject_id = '*' THEN 0 ELSE 1 END) DESC, r.subject_id DESC
                      LIMIT 1)
            END AS next_cursor
        )
        SELECT r.subject_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
    ELSE
        -- Guard: return empty if subject type is not allowed by the model
        IF p_subject_type NOT IN ('user') THEN
        RETURN;
    END IF;
        -- Regular subject type (no userset filter)
        RETURN QUERY
        WITH base_results AS (
            -- Path 1: Direct tuple lookup with simple closure relations
                SELECT DISTINCT t.subject_id
                FROM melange_tuples AS t
                WHERE (t.object_type = 'repository' AND t.relation IN ('public') AND t.object_id = p_object_id AND t.subject_type = p_subject_type AND position('#' in t.subject_id) = 0)
        ),
        paged AS (
            SELECT br.subject_id
            FROM base_results br
            WHERE p_after IS NULL OR (
                -- Compound comparison for wildcard-first ordering:
                -- (is_not_wildcard, subject_id) > (cursor_is_not_wildcard, cursor)
                (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END, br.subject_id) >
                (CASE WHEN p_after = '*' THEN 0 ELSE 1 END, p_after)
            )
            ORDER BY (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END), br.subject_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.subject_id FROM paged p
            ORDER BY (CASE WHEN p.subject_id = '*' THEN 0 ELSE 1 END), p.subject_id
            LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT r.subject_id FROM returned r
                      ORDER BY (CASE WHEN r.subject_id = '*' THEN 0 ELSE 1 END) DESC, r.subject_id DESC
                      LIMIT 1)
            END AS next_cursor
        )
        SELECT r.subject_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
    END IF;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';

-- Generated list_subjects function for run.agent
-- Features: Direct
CREATE OR REPLACE FUNCTION "public"."list_run_agent_sub"(
    p_object_id TEXT,
    p_subject_type TEXT,
    p_limit INT DEFAULT NULL,
    p_after TEXT DEFAULT NULL
) RETURNS TABLE(subject_id TEXT, next_cursor TEXT) ROWS 100 AS $$
DECLARE
    v_filter_type TEXT;
    v_filter_relation TEXT;
BEGIN
    -- Check if subject_type is a userset filter (e.g., "document#viewer")
    IF position('#' in p_subject_type) > 0 THEN
        v_filter_type := substring(p_subject_type from 1 for position('#' in p_subject_type) - 1);
        v_filter_relation := substring(p_subject_type from position('#' in p_subject_type) + 1);
        RETURN QUERY
        WITH base_results AS (
            -- Userset filter: find userset tuples that match and return normalized references
                SELECT DISTINCT split_part(t.subject_id, '#', 1) || '#' || v_filter_relation AS subject_id
                		FROM melange_tuples AS t
                		WHERE (t.object_type = 'run' AND t.relation IN ('agent') AND t.object_id = p_object_id AND t.subject_type = v_filter_type AND position('#' in t.subject_id) > 0 AND (substring(t.subject_id from position('#' in t.subject_id) + 1) = v_filter_relation OR EXISTS (
                SELECT 1
                FROM (VALUES ('agent', 'can_execute', 'can_execute'), ('agent', 'can_manage', 'can_manage'), ('agent', 'can_read', 'can_read'), ('agent', 'project', 'project'), ('run', 'agent', 'agent'), ('run', 'can_cancel', 'can_cancel'), ('run', 'can_read', 'can_read')) AS subj_c(object_type, relation, satisfying_relation)
                WHERE (subj_c.object_type = v_filter_type AND subj_c.relation = substring(t.subject_id from position('#' in t.subject_id) + 1) AND subj_c.satisfying_relation = v_filter_relation)
                )) AND "public"."check_permission_internal"(v_filter_type, t.subject_id, 'agent', 'run', p_object_id, ARRAY[]::TEXT[]) = 1)
                UNION
                -- Self-candidate: when filter type matches object type
                -- e.g., querying document:1.viewer with filter document#writer
                -- should return document:1#writer if writer satisfies the relation
                SELECT p_object_id || '#' || v_filter_relation AS subject_id
                		WHERE (v_filter_type = 'run' AND EXISTS (
                SELECT 1
                FROM (VALUES ('agent', 'can_execute', 'can_execute'), ('agent', 'can_manage', 'can_manage'), ('agent', 'can_read', 'can_read'), ('agent', 'project', 'project'), ('run', 'agent', 'agent'), ('run', 'can_cancel', 'can_cancel'), ('run', 'can_read', 'can_read')) AS c(object_type, relation, satisfying_relation)
                WHERE (c.object_type = 'run' AND c.relation = 'agent' AND c.satisfying_relation = v_filter_relation)
                ))
        ),
        paged AS (
            SELECT br.subject_id
            FROM base_results br
            WHERE p_after IS NULL OR (
                -- Compound comparison for wildcard-first ordering:
                -- (is_not_wildcard, subject_id) > (cursor_is_not_wildcard, cursor)
                (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END, br.subject_id) >
                (CASE WHEN p_after = '*' THEN 0 ELSE 1 END, p_after)
            )
            ORDER BY (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END), br.subject_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.subject_id FROM paged p
            ORDER BY (CASE WHEN p.subject_id = '*' THEN 0 ELSE 1 END), p.subject_id
            LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT r.subject_id FROM returned r
                      ORDER BY (CASE WHEN r.subject_id = '*' THEN 0 ELSE 1 END) DESC, r.subject_id DESC
                      LIMIT 1)
            END AS next_cursor
        )
        SELECT r.subject_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
    ELSE
        -- Guard: return empty if subject type is not allowed by the model
        IF p_subject_type NOT IN ('agent') THEN
        RETURN;
    END IF;
        -- Regular subject type (no userset filter)
        RETURN QUERY
        WITH base_results AS (
            -- Path 1: Direct tuple lookup with simple closure relations
                SELECT DISTINCT t.subject_id
                FROM melange_tuples AS t
                WHERE (t.object_type = 'run' AND t.relation IN ('agent') AND t.object_id = p_object_id AND t.subject_type = p_subject_type AND position('#' in t.subject_id) = 0 AND t.subject_id <> '*')
        ),
        paged AS (
            SELECT br.subject_id
            FROM base_results br
            WHERE p_after IS NULL OR (
                -- Compound comparison for wildcard-first ordering:
                -- (is_not_wildcard, subject_id) > (cursor_is_not_wildcard, cursor)
                (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END, br.subject_id) >
                (CASE WHEN p_after = '*' THEN 0 ELSE 1 END, p_after)
            )
            ORDER BY (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END), br.subject_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.subject_id FROM paged p
            ORDER BY (CASE WHEN p.subject_id = '*' THEN 0 ELSE 1 END), p.subject_id
            LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT r.subject_id FROM returned r
                      ORDER BY (CASE WHEN r.subject_id = '*' THEN 0 ELSE 1 END) DESC, r.subject_id DESC
                      LIMIT 1)
            END AS next_cursor
        )
        SELECT r.subject_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
    END IF;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';

-- Generated list_subjects function for state_volume.agent
-- Features: Direct
CREATE OR REPLACE FUNCTION "public"."list_state_volume_agent_sub"(
    p_object_id TEXT,
    p_subject_type TEXT,
    p_limit INT DEFAULT NULL,
    p_after TEXT DEFAULT NULL
) RETURNS TABLE(subject_id TEXT, next_cursor TEXT) ROWS 100 AS $$
DECLARE
    v_filter_type TEXT;
    v_filter_relation TEXT;
BEGIN
    -- Check if subject_type is a userset filter (e.g., "document#viewer")
    IF position('#' in p_subject_type) > 0 THEN
        v_filter_type := substring(p_subject_type from 1 for position('#' in p_subject_type) - 1);
        v_filter_relation := substring(p_subject_type from position('#' in p_subject_type) + 1);
        RETURN QUERY
        WITH base_results AS (
            -- Userset filter: find userset tuples that match and return normalized references
                SELECT DISTINCT split_part(t.subject_id, '#', 1) || '#' || v_filter_relation AS subject_id
                		FROM melange_tuples AS t
                		WHERE (t.object_type = 'state_volume' AND t.relation IN ('agent') AND t.object_id = p_object_id AND t.subject_type = v_filter_type AND position('#' in t.subject_id) > 0 AND (substring(t.subject_id from position('#' in t.subject_id) + 1) = v_filter_relation OR EXISTS (
                SELECT 1
                FROM (VALUES ('agent', 'can_execute', 'can_execute'), ('agent', 'can_manage', 'can_manage'), ('agent', 'can_read', 'can_read'), ('agent', 'project', 'project'), ('state_volume', 'agent', 'agent'), ('state_volume', 'can_attach', 'can_attach'), ('state_volume', 'can_manage', 'can_manage'), ('state_volume', 'can_read', 'can_read'), ('state_volume', 'can_restore', 'can_restore')) AS subj_c(object_type, relation, satisfying_relation)
                WHERE (subj_c.object_type = v_filter_type AND subj_c.relation = substring(t.subject_id from position('#' in t.subject_id) + 1) AND subj_c.satisfying_relation = v_filter_relation)
                )) AND "public"."check_permission_internal"(v_filter_type, t.subject_id, 'agent', 'state_volume', p_object_id, ARRAY[]::TEXT[]) = 1)
                UNION
                -- Self-candidate: when filter type matches object type
                -- e.g., querying document:1.viewer with filter document#writer
                -- should return document:1#writer if writer satisfies the relation
                SELECT p_object_id || '#' || v_filter_relation AS subject_id
                		WHERE (v_filter_type = 'state_volume' AND EXISTS (
                SELECT 1
                FROM (VALUES ('agent', 'can_execute', 'can_execute'), ('agent', 'can_manage', 'can_manage'), ('agent', 'can_read', 'can_read'), ('agent', 'project', 'project'), ('state_volume', 'agent', 'agent'), ('state_volume', 'can_attach', 'can_attach'), ('state_volume', 'can_manage', 'can_manage'), ('state_volume', 'can_read', 'can_read'), ('state_volume', 'can_restore', 'can_restore')) AS c(object_type, relation, satisfying_relation)
                WHERE (c.object_type = 'state_volume' AND c.relation = 'agent' AND c.satisfying_relation = v_filter_relation)
                ))
        ),
        paged AS (
            SELECT br.subject_id
            FROM base_results br
            WHERE p_after IS NULL OR (
                -- Compound comparison for wildcard-first ordering:
                -- (is_not_wildcard, subject_id) > (cursor_is_not_wildcard, cursor)
                (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END, br.subject_id) >
                (CASE WHEN p_after = '*' THEN 0 ELSE 1 END, p_after)
            )
            ORDER BY (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END), br.subject_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.subject_id FROM paged p
            ORDER BY (CASE WHEN p.subject_id = '*' THEN 0 ELSE 1 END), p.subject_id
            LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT r.subject_id FROM returned r
                      ORDER BY (CASE WHEN r.subject_id = '*' THEN 0 ELSE 1 END) DESC, r.subject_id DESC
                      LIMIT 1)
            END AS next_cursor
        )
        SELECT r.subject_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
    ELSE
        -- Guard: return empty if subject type is not allowed by the model
        IF p_subject_type NOT IN ('agent') THEN
        RETURN;
    END IF;
        -- Regular subject type (no userset filter)
        RETURN QUERY
        WITH base_results AS (
            -- Path 1: Direct tuple lookup with simple closure relations
                SELECT DISTINCT t.subject_id
                FROM melange_tuples AS t
                WHERE (t.object_type = 'state_volume' AND t.relation IN ('agent') AND t.object_id = p_object_id AND t.subject_type = p_subject_type AND position('#' in t.subject_id) = 0 AND t.subject_id <> '*')
        ),
        paged AS (
            SELECT br.subject_id
            FROM base_results br
            WHERE p_after IS NULL OR (
                -- Compound comparison for wildcard-first ordering:
                -- (is_not_wildcard, subject_id) > (cursor_is_not_wildcard, cursor)
                (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END, br.subject_id) >
                (CASE WHEN p_after = '*' THEN 0 ELSE 1 END, p_after)
            )
            ORDER BY (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END), br.subject_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.subject_id FROM paged p
            ORDER BY (CASE WHEN p.subject_id = '*' THEN 0 ELSE 1 END), p.subject_id
            LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT r.subject_id FROM returned r
                      ORDER BY (CASE WHEN r.subject_id = '*' THEN 0 ELSE 1 END) DESC, r.subject_id DESC
                      LIMIT 1)
            END AS next_cursor
        )
        SELECT r.subject_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
    END IF;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';

-- Generated list_subjects function for organization.admin
-- Features: Direct+Implied
CREATE OR REPLACE FUNCTION "public"."list_organization_admin_sub"(
    p_object_id TEXT,
    p_subject_type TEXT,
    p_limit INT DEFAULT NULL,
    p_after TEXT DEFAULT NULL
) RETURNS TABLE(subject_id TEXT, next_cursor TEXT) ROWS 100 AS $$
DECLARE
    v_filter_type TEXT;
    v_filter_relation TEXT;
BEGIN
    -- Check if subject_type is a userset filter (e.g., "document#viewer")
    IF position('#' in p_subject_type) > 0 THEN
        v_filter_type := substring(p_subject_type from 1 for position('#' in p_subject_type) - 1);
        v_filter_relation := substring(p_subject_type from position('#' in p_subject_type) + 1);
        RETURN QUERY
        WITH base_results AS (
            -- Userset filter: find userset tuples that match and return normalized references
                SELECT DISTINCT split_part(t.subject_id, '#', 1) || '#' || v_filter_relation AS subject_id
                		FROM melange_tuples AS t
                		WHERE (t.object_type = 'organization' AND t.relation IN ('admin', 'owner') AND t.object_id = p_object_id AND t.subject_type = v_filter_type AND position('#' in t.subject_id) > 0 AND (substring(t.subject_id from position('#' in t.subject_id) + 1) = v_filter_relation OR EXISTS (
                SELECT 1
                FROM (VALUES ('organization', 'admin', 'admin'), ('organization', 'admin', 'owner'), ('organization', 'can_create_project', 'admin'), ('organization', 'can_create_project', 'can_create_project'), ('organization', 'can_create_project', 'owner'), ('organization', 'can_delete', 'can_delete'), ('organization', 'can_delete', 'owner'), ('organization', 'can_manage', 'can_manage'), ('organization', 'can_manage', 'owner'), ('organization', 'can_manage_members', 'admin'), ('organization', 'can_manage_members', 'can_manage_members'), ('organization', 'can_manage_members', 'owner'), ('organization', 'can_read', 'admin'), ('organization', 'can_read', 'can_read'), ('organization', 'can_read', 'member'), ('organization', 'can_read', 'owner'), ('organization', 'member', 'admin'), ('organization', 'member', 'member'), ('organization', 'member', 'owner'), ('organization', 'owner', 'owner')) AS subj_c(object_type, relation, satisfying_relation)
                WHERE (subj_c.object_type = v_filter_type AND subj_c.relation = substring(t.subject_id from position('#' in t.subject_id) + 1) AND subj_c.satisfying_relation = v_filter_relation)
                )) AND "public"."check_permission_internal"(v_filter_type, t.subject_id, 'admin', 'organization', p_object_id, ARRAY[]::TEXT[]) = 1)
                UNION
                -- Self-candidate: when filter type matches object type
                -- e.g., querying document:1.viewer with filter document#writer
                -- should return document:1#writer if writer satisfies the relation
                SELECT p_object_id || '#' || v_filter_relation AS subject_id
                		WHERE (v_filter_type = 'organization' AND EXISTS (
                SELECT 1
                FROM (VALUES ('organization', 'admin', 'admin'), ('organization', 'admin', 'owner'), ('organization', 'can_create_project', 'admin'), ('organization', 'can_create_project', 'can_create_project'), ('organization', 'can_create_project', 'owner'), ('organization', 'can_delete', 'can_delete'), ('organization', 'can_delete', 'owner'), ('organization', 'can_manage', 'can_manage'), ('organization', 'can_manage', 'owner'), ('organization', 'can_manage_members', 'admin'), ('organization', 'can_manage_members', 'can_manage_members'), ('organization', 'can_manage_members', 'owner'), ('organization', 'can_read', 'admin'), ('organization', 'can_read', 'can_read'), ('organization', 'can_read', 'member'), ('organization', 'can_read', 'owner'), ('organization', 'member', 'admin'), ('organization', 'member', 'member'), ('organization', 'member', 'owner'), ('organization', 'owner', 'owner')) AS c(object_type, relation, satisfying_relation)
                WHERE (c.object_type = 'organization' AND c.relation = 'admin' AND c.satisfying_relation = v_filter_relation)
                ))
        ),
        paged AS (
            SELECT br.subject_id
            FROM base_results br
            WHERE p_after IS NULL OR (
                -- Compound comparison for wildcard-first ordering:
                -- (is_not_wildcard, subject_id) > (cursor_is_not_wildcard, cursor)
                (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END, br.subject_id) >
                (CASE WHEN p_after = '*' THEN 0 ELSE 1 END, p_after)
            )
            ORDER BY (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END), br.subject_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.subject_id FROM paged p
            ORDER BY (CASE WHEN p.subject_id = '*' THEN 0 ELSE 1 END), p.subject_id
            LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT r.subject_id FROM returned r
                      ORDER BY (CASE WHEN r.subject_id = '*' THEN 0 ELSE 1 END) DESC, r.subject_id DESC
                      LIMIT 1)
            END AS next_cursor
        )
        SELECT r.subject_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
    ELSE
        -- Guard: return empty if subject type is not allowed by the model
        IF p_subject_type NOT IN ('user') THEN
        RETURN;
    END IF;
        -- Regular subject type (no userset filter)
        RETURN QUERY
        WITH base_results AS (
            -- Path 1: Direct tuple lookup with simple closure relations
                SELECT DISTINCT t.subject_id
                FROM melange_tuples AS t
                WHERE (t.object_type = 'organization' AND t.relation IN ('admin', 'owner') AND t.object_id = p_object_id AND t.subject_type = p_subject_type AND position('#' in t.subject_id) = 0 AND t.subject_id <> '*')
        ),
        paged AS (
            SELECT br.subject_id
            FROM base_results br
            WHERE p_after IS NULL OR (
                -- Compound comparison for wildcard-first ordering:
                -- (is_not_wildcard, subject_id) > (cursor_is_not_wildcard, cursor)
                (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END, br.subject_id) >
                (CASE WHEN p_after = '*' THEN 0 ELSE 1 END, p_after)
            )
            ORDER BY (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END), br.subject_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.subject_id FROM paged p
            ORDER BY (CASE WHEN p.subject_id = '*' THEN 0 ELSE 1 END), p.subject_id
            LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT r.subject_id FROM returned r
                      ORDER BY (CASE WHEN r.subject_id = '*' THEN 0 ELSE 1 END) DESC, r.subject_id DESC
                      LIMIT 1)
            END AS next_cursor
        )
        SELECT r.subject_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
    END IF;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';

-- Generated list_subjects function for organization.can_delete
-- Features: Implied
CREATE OR REPLACE FUNCTION "public"."list_organization_can_delete_sub"(
    p_object_id TEXT,
    p_subject_type TEXT,
    p_limit INT DEFAULT NULL,
    p_after TEXT DEFAULT NULL
) RETURNS TABLE(subject_id TEXT, next_cursor TEXT) ROWS 100 AS $$
DECLARE
    v_filter_type TEXT;
    v_filter_relation TEXT;
BEGIN
    -- Check if subject_type is a userset filter (e.g., "document#viewer")
    IF position('#' in p_subject_type) > 0 THEN
        v_filter_type := substring(p_subject_type from 1 for position('#' in p_subject_type) - 1);
        v_filter_relation := substring(p_subject_type from position('#' in p_subject_type) + 1);
        RETURN QUERY
        WITH base_results AS (
            -- Userset filter: find userset tuples that match and return normalized references
                SELECT DISTINCT split_part(t.subject_id, '#', 1) || '#' || v_filter_relation AS subject_id
                		FROM melange_tuples AS t
                		WHERE (t.object_type = 'organization' AND t.relation IN ('can_delete', 'owner') AND t.object_id = p_object_id AND t.subject_type = v_filter_type AND position('#' in t.subject_id) > 0 AND (substring(t.subject_id from position('#' in t.subject_id) + 1) = v_filter_relation OR EXISTS (
                SELECT 1
                FROM (VALUES ('organization', 'admin', 'admin'), ('organization', 'admin', 'owner'), ('organization', 'can_create_project', 'admin'), ('organization', 'can_create_project', 'can_create_project'), ('organization', 'can_create_project', 'owner'), ('organization', 'can_delete', 'can_delete'), ('organization', 'can_delete', 'owner'), ('organization', 'can_manage', 'can_manage'), ('organization', 'can_manage', 'owner'), ('organization', 'can_manage_members', 'admin'), ('organization', 'can_manage_members', 'can_manage_members'), ('organization', 'can_manage_members', 'owner'), ('organization', 'can_read', 'admin'), ('organization', 'can_read', 'can_read'), ('organization', 'can_read', 'member'), ('organization', 'can_read', 'owner'), ('organization', 'member', 'admin'), ('organization', 'member', 'member'), ('organization', 'member', 'owner'), ('organization', 'owner', 'owner')) AS subj_c(object_type, relation, satisfying_relation)
                WHERE (subj_c.object_type = v_filter_type AND subj_c.relation = substring(t.subject_id from position('#' in t.subject_id) + 1) AND subj_c.satisfying_relation = v_filter_relation)
                )) AND "public"."check_permission_internal"(v_filter_type, t.subject_id, 'can_delete', 'organization', p_object_id, ARRAY[]::TEXT[]) = 1)
                UNION
                -- Self-candidate: when filter type matches object type
                -- e.g., querying document:1.viewer with filter document#writer
                -- should return document:1#writer if writer satisfies the relation
                SELECT p_object_id || '#' || v_filter_relation AS subject_id
                		WHERE (v_filter_type = 'organization' AND EXISTS (
                SELECT 1
                FROM (VALUES ('organization', 'admin', 'admin'), ('organization', 'admin', 'owner'), ('organization', 'can_create_project', 'admin'), ('organization', 'can_create_project', 'can_create_project'), ('organization', 'can_create_project', 'owner'), ('organization', 'can_delete', 'can_delete'), ('organization', 'can_delete', 'owner'), ('organization', 'can_manage', 'can_manage'), ('organization', 'can_manage', 'owner'), ('organization', 'can_manage_members', 'admin'), ('organization', 'can_manage_members', 'can_manage_members'), ('organization', 'can_manage_members', 'owner'), ('organization', 'can_read', 'admin'), ('organization', 'can_read', 'can_read'), ('organization', 'can_read', 'member'), ('organization', 'can_read', 'owner'), ('organization', 'member', 'admin'), ('organization', 'member', 'member'), ('organization', 'member', 'owner'), ('organization', 'owner', 'owner')) AS c(object_type, relation, satisfying_relation)
                WHERE (c.object_type = 'organization' AND c.relation = 'can_delete' AND c.satisfying_relation = v_filter_relation)
                ))
        ),
        paged AS (
            SELECT br.subject_id
            FROM base_results br
            WHERE p_after IS NULL OR (
                -- Compound comparison for wildcard-first ordering:
                -- (is_not_wildcard, subject_id) > (cursor_is_not_wildcard, cursor)
                (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END, br.subject_id) >
                (CASE WHEN p_after = '*' THEN 0 ELSE 1 END, p_after)
            )
            ORDER BY (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END), br.subject_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.subject_id FROM paged p
            ORDER BY (CASE WHEN p.subject_id = '*' THEN 0 ELSE 1 END), p.subject_id
            LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT r.subject_id FROM returned r
                      ORDER BY (CASE WHEN r.subject_id = '*' THEN 0 ELSE 1 END) DESC, r.subject_id DESC
                      LIMIT 1)
            END AS next_cursor
        )
        SELECT r.subject_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
    ELSE
        -- Guard: return empty if subject type is not allowed by the model
        IF p_subject_type NOT IN ('user') THEN
        RETURN;
    END IF;
        -- Regular subject type (no userset filter)
        RETURN QUERY
        WITH base_results AS (
            -- Path 1: Direct tuple lookup with simple closure relations
                SELECT DISTINCT t.subject_id
                FROM melange_tuples AS t
                WHERE (t.object_type = 'organization' AND t.relation IN ('can_delete', 'owner') AND t.object_id = p_object_id AND t.subject_type = p_subject_type AND position('#' in t.subject_id) = 0 AND t.subject_id <> '*')
        ),
        paged AS (
            SELECT br.subject_id
            FROM base_results br
            WHERE p_after IS NULL OR (
                -- Compound comparison for wildcard-first ordering:
                -- (is_not_wildcard, subject_id) > (cursor_is_not_wildcard, cursor)
                (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END, br.subject_id) >
                (CASE WHEN p_after = '*' THEN 0 ELSE 1 END, p_after)
            )
            ORDER BY (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END), br.subject_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.subject_id FROM paged p
            ORDER BY (CASE WHEN p.subject_id = '*' THEN 0 ELSE 1 END), p.subject_id
            LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT r.subject_id FROM returned r
                      ORDER BY (CASE WHEN r.subject_id = '*' THEN 0 ELSE 1 END) DESC, r.subject_id DESC
                      LIMIT 1)
            END AS next_cursor
        )
        SELECT r.subject_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
    END IF;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';

-- Generated list_subjects function for organization.can_manage
-- Features: Implied
CREATE OR REPLACE FUNCTION "public"."list_organization_can_manage_sub"(
    p_object_id TEXT,
    p_subject_type TEXT,
    p_limit INT DEFAULT NULL,
    p_after TEXT DEFAULT NULL
) RETURNS TABLE(subject_id TEXT, next_cursor TEXT) ROWS 100 AS $$
DECLARE
    v_filter_type TEXT;
    v_filter_relation TEXT;
BEGIN
    -- Check if subject_type is a userset filter (e.g., "document#viewer")
    IF position('#' in p_subject_type) > 0 THEN
        v_filter_type := substring(p_subject_type from 1 for position('#' in p_subject_type) - 1);
        v_filter_relation := substring(p_subject_type from position('#' in p_subject_type) + 1);
        RETURN QUERY
        WITH base_results AS (
            -- Userset filter: find userset tuples that match and return normalized references
                SELECT DISTINCT split_part(t.subject_id, '#', 1) || '#' || v_filter_relation AS subject_id
                		FROM melange_tuples AS t
                		WHERE (t.object_type = 'organization' AND t.relation IN ('can_manage', 'owner') AND t.object_id = p_object_id AND t.subject_type = v_filter_type AND position('#' in t.subject_id) > 0 AND (substring(t.subject_id from position('#' in t.subject_id) + 1) = v_filter_relation OR EXISTS (
                SELECT 1
                FROM (VALUES ('organization', 'admin', 'admin'), ('organization', 'admin', 'owner'), ('organization', 'can_create_project', 'admin'), ('organization', 'can_create_project', 'can_create_project'), ('organization', 'can_create_project', 'owner'), ('organization', 'can_delete', 'can_delete'), ('organization', 'can_delete', 'owner'), ('organization', 'can_manage', 'can_manage'), ('organization', 'can_manage', 'owner'), ('organization', 'can_manage_members', 'admin'), ('organization', 'can_manage_members', 'can_manage_members'), ('organization', 'can_manage_members', 'owner'), ('organization', 'can_read', 'admin'), ('organization', 'can_read', 'can_read'), ('organization', 'can_read', 'member'), ('organization', 'can_read', 'owner'), ('organization', 'member', 'admin'), ('organization', 'member', 'member'), ('organization', 'member', 'owner'), ('organization', 'owner', 'owner')) AS subj_c(object_type, relation, satisfying_relation)
                WHERE (subj_c.object_type = v_filter_type AND subj_c.relation = substring(t.subject_id from position('#' in t.subject_id) + 1) AND subj_c.satisfying_relation = v_filter_relation)
                )) AND "public"."check_permission_internal"(v_filter_type, t.subject_id, 'can_manage', 'organization', p_object_id, ARRAY[]::TEXT[]) = 1)
                UNION
                -- Self-candidate: when filter type matches object type
                -- e.g., querying document:1.viewer with filter document#writer
                -- should return document:1#writer if writer satisfies the relation
                SELECT p_object_id || '#' || v_filter_relation AS subject_id
                		WHERE (v_filter_type = 'organization' AND EXISTS (
                SELECT 1
                FROM (VALUES ('organization', 'admin', 'admin'), ('organization', 'admin', 'owner'), ('organization', 'can_create_project', 'admin'), ('organization', 'can_create_project', 'can_create_project'), ('organization', 'can_create_project', 'owner'), ('organization', 'can_delete', 'can_delete'), ('organization', 'can_delete', 'owner'), ('organization', 'can_manage', 'can_manage'), ('organization', 'can_manage', 'owner'), ('organization', 'can_manage_members', 'admin'), ('organization', 'can_manage_members', 'can_manage_members'), ('organization', 'can_manage_members', 'owner'), ('organization', 'can_read', 'admin'), ('organization', 'can_read', 'can_read'), ('organization', 'can_read', 'member'), ('organization', 'can_read', 'owner'), ('organization', 'member', 'admin'), ('organization', 'member', 'member'), ('organization', 'member', 'owner'), ('organization', 'owner', 'owner')) AS c(object_type, relation, satisfying_relation)
                WHERE (c.object_type = 'organization' AND c.relation = 'can_manage' AND c.satisfying_relation = v_filter_relation)
                ))
        ),
        paged AS (
            SELECT br.subject_id
            FROM base_results br
            WHERE p_after IS NULL OR (
                -- Compound comparison for wildcard-first ordering:
                -- (is_not_wildcard, subject_id) > (cursor_is_not_wildcard, cursor)
                (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END, br.subject_id) >
                (CASE WHEN p_after = '*' THEN 0 ELSE 1 END, p_after)
            )
            ORDER BY (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END), br.subject_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.subject_id FROM paged p
            ORDER BY (CASE WHEN p.subject_id = '*' THEN 0 ELSE 1 END), p.subject_id
            LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT r.subject_id FROM returned r
                      ORDER BY (CASE WHEN r.subject_id = '*' THEN 0 ELSE 1 END) DESC, r.subject_id DESC
                      LIMIT 1)
            END AS next_cursor
        )
        SELECT r.subject_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
    ELSE
        -- Guard: return empty if subject type is not allowed by the model
        IF p_subject_type NOT IN ('user') THEN
        RETURN;
    END IF;
        -- Regular subject type (no userset filter)
        RETURN QUERY
        WITH base_results AS (
            -- Path 1: Direct tuple lookup with simple closure relations
                SELECT DISTINCT t.subject_id
                FROM melange_tuples AS t
                WHERE (t.object_type = 'organization' AND t.relation IN ('can_manage', 'owner') AND t.object_id = p_object_id AND t.subject_type = p_subject_type AND position('#' in t.subject_id) = 0 AND t.subject_id <> '*')
        ),
        paged AS (
            SELECT br.subject_id
            FROM base_results br
            WHERE p_after IS NULL OR (
                -- Compound comparison for wildcard-first ordering:
                -- (is_not_wildcard, subject_id) > (cursor_is_not_wildcard, cursor)
                (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END, br.subject_id) >
                (CASE WHEN p_after = '*' THEN 0 ELSE 1 END, p_after)
            )
            ORDER BY (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END), br.subject_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.subject_id FROM paged p
            ORDER BY (CASE WHEN p.subject_id = '*' THEN 0 ELSE 1 END), p.subject_id
            LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT r.subject_id FROM returned r
                      ORDER BY (CASE WHEN r.subject_id = '*' THEN 0 ELSE 1 END) DESC, r.subject_id DESC
                      LIMIT 1)
            END AS next_cursor
        )
        SELECT r.subject_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
    END IF;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';

-- Generated list_subjects function for project.can_delete
-- Features: Recursive
-- Indirect anchor: organization.owner via ttu
CREATE OR REPLACE FUNCTION "public"."list_project_can_delete_sub"(
    p_object_id TEXT,
    p_subject_type TEXT,
    p_limit INT DEFAULT NULL,
    p_after TEXT DEFAULT NULL
) RETURNS TABLE(subject_id TEXT, next_cursor TEXT) ROWS 100 AS $$
DECLARE
    v_is_userset_filter BOOLEAN;
    v_filter_type TEXT;
    v_filter_relation TEXT;
BEGIN
    v_is_userset_filter := position('#' in p_subject_type) > 0;
    IF v_is_userset_filter THEN
        v_filter_type := split_part(p_subject_type, '#', 1);
        v_filter_relation := split_part(p_subject_type, '#', 2);
        -- Self-candidate: when filter type matches object type
        IF v_filter_type = 'project' THEN
        IF EXISTS (
    		SELECT p_object_id || '#' || v_filter_relation AS subject_id
    		WHERE (v_filter_type = 'project' AND EXISTS (
    SELECT 1
    FROM (VALUES ('organization', 'admin', 'admin'), ('organization', 'admin', 'owner'), ('organization', 'can_create_project', 'admin'), ('organization', 'can_create_project', 'can_create_project'), ('organization', 'can_create_project', 'owner'), ('organization', 'can_delete', 'can_delete'), ('organization', 'can_delete', 'owner'), ('organization', 'can_manage', 'can_manage'), ('organization', 'can_manage', 'owner'), ('organization', 'can_manage_members', 'admin'), ('organization', 'can_manage_members', 'can_manage_members'), ('organization', 'can_manage_members', 'owner'), ('organization', 'can_read', 'admin'), ('organization', 'can_read', 'can_read'), ('organization', 'can_read', 'member'), ('organization', 'can_read', 'owner'), ('organization', 'member', 'admin'), ('organization', 'member', 'member'), ('organization', 'member', 'owner'), ('organization', 'owner', 'owner'), ('project', 'can_delete', 'can_delete'), ('project', 'can_manage', 'can_manage'), ('project', 'can_manage', 'maintainer'), ('project', 'can_read', 'can_read'), ('project', 'can_read', 'maintainer'), ('project', 'can_write', 'can_write'), ('project', 'can_write', 'maintainer'), ('project', 'maintainer', 'maintainer'), ('project', 'organization', 'organization')) AS c(object_type, relation, satisfying_relation)
    WHERE (c.object_type = 'project' AND c.relation = 'can_delete' AND c.satisfying_relation = v_filter_relation)
    ))
    ) THEN
        RETURN QUERY
        WITH base_results AS (
            SELECT p_object_id || '#' || v_filter_relation AS subject_id
            		WHERE (v_filter_type = 'project' AND EXISTS (
            SELECT 1
            FROM (VALUES ('organization', 'admin', 'admin'), ('organization', 'admin', 'owner'), ('organization', 'can_create_project', 'admin'), ('organization', 'can_create_project', 'can_create_project'), ('organization', 'can_create_project', 'owner'), ('organization', 'can_delete', 'can_delete'), ('organization', 'can_delete', 'owner'), ('organization', 'can_manage', 'can_manage'), ('organization', 'can_manage', 'owner'), ('organization', 'can_manage_members', 'admin'), ('organization', 'can_manage_members', 'can_manage_members'), ('organization', 'can_manage_members', 'owner'), ('organization', 'can_read', 'admin'), ('organization', 'can_read', 'can_read'), ('organization', 'can_read', 'member'), ('organization', 'can_read', 'owner'), ('organization', 'member', 'admin'), ('organization', 'member', 'member'), ('organization', 'member', 'owner'), ('organization', 'owner', 'owner'), ('project', 'can_delete', 'can_delete'), ('project', 'can_manage', 'can_manage'), ('project', 'can_manage', 'maintainer'), ('project', 'can_read', 'can_read'), ('project', 'can_read', 'maintainer'), ('project', 'can_write', 'can_write'), ('project', 'can_write', 'maintainer'), ('project', 'maintainer', 'maintainer'), ('project', 'organization', 'organization')) AS c(object_type, relation, satisfying_relation)
            WHERE (c.object_type = 'project' AND c.relation = 'can_delete' AND c.satisfying_relation = v_filter_relation)
            ))
        ),
        paged AS (
            SELECT br.subject_id
            FROM base_results br
            WHERE p_after IS NULL OR (
                -- Compound comparison for wildcard-first ordering:
                -- (is_not_wildcard, subject_id) > (cursor_is_not_wildcard, cursor)
                (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END, br.subject_id) >
                (CASE WHEN p_after = '*' THEN 0 ELSE 1 END, p_after)
            )
            ORDER BY (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END), br.subject_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.subject_id FROM paged p
            ORDER BY (CASE WHEN p.subject_id = '*' THEN 0 ELSE 1 END), p.subject_id
            LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT r.subject_id FROM returned r
                      ORDER BY (CASE WHEN r.subject_id = '*' THEN 0 ELSE 1 END) DESC, r.subject_id DESC
                      LIMIT 1)
            END AS next_cursor
        )
        SELECT r.subject_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
        RETURN;
    END IF;
    END IF;
        -- Userset filter case
        RETURN QUERY
        WITH base_results AS (
            WITH subject_candidates AS (
                -- From organization parents
                    SELECT DISTINCT s.subject_id
                    FROM melange_tuples AS link
                    CROSS JOIN LATERAL "public"."list_organization_owner_sub"(link.subject_id, p_subject_type) AS s
                    WHERE (link.object_type = 'project' AND link.object_id = p_object_id AND link.relation = 'organization' AND link.subject_type = 'organization')
            )
            SELECT DISTINCT sc.subject_id
            FROM subject_candidates AS sc
            WHERE "public"."check_permission_internal"(v_filter_type, sc.subject_id, 'can_delete', 'project', p_object_id, ARRAY[]::TEXT[]) = 1
        ),
        paged AS (
            SELECT br.subject_id
            FROM base_results br
            WHERE p_after IS NULL OR (
                -- Compound comparison for wildcard-first ordering:
                -- (is_not_wildcard, subject_id) > (cursor_is_not_wildcard, cursor)
                (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END, br.subject_id) >
                (CASE WHEN p_after = '*' THEN 0 ELSE 1 END, p_after)
            )
            ORDER BY (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END), br.subject_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.subject_id FROM paged p
            ORDER BY (CASE WHEN p.subject_id = '*' THEN 0 ELSE 1 END), p.subject_id
            LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT r.subject_id FROM returned r
                      ORDER BY (CASE WHEN r.subject_id = '*' THEN 0 ELSE 1 END) DESC, r.subject_id DESC
                      LIMIT 1)
            END AS next_cursor
        )
        SELECT r.subject_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
    ELSE
        -- Direct subject type case
        IF p_subject_type NOT IN ('user') THEN
        RETURN;
    END IF;
        RETURN QUERY
        WITH base_results AS (
            WITH subject_candidates AS (
                -- From organization parents
                    SELECT DISTINCT s.subject_id
                    FROM melange_tuples AS link
                    CROSS JOIN LATERAL "public"."list_organization_owner_sub"(link.subject_id, p_subject_type) AS s
                    WHERE (link.object_type = 'project' AND link.object_id = p_object_id AND link.relation = 'organization' AND link.subject_type = 'organization')
            )
            SELECT DISTINCT sc.subject_id
            FROM subject_candidates AS sc
        ),
        paged AS (
            SELECT br.subject_id
            FROM base_results br
            WHERE p_after IS NULL OR (
                -- Compound comparison for wildcard-first ordering:
                -- (is_not_wildcard, subject_id) > (cursor_is_not_wildcard, cursor)
                (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END, br.subject_id) >
                (CASE WHEN p_after = '*' THEN 0 ELSE 1 END, p_after)
            )
            ORDER BY (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END), br.subject_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.subject_id FROM paged p
            ORDER BY (CASE WHEN p.subject_id = '*' THEN 0 ELSE 1 END), p.subject_id
            LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT r.subject_id FROM returned r
                      ORDER BY (CASE WHEN r.subject_id = '*' THEN 0 ELSE 1 END) DESC, r.subject_id DESC
                      LIMIT 1)
            END AS next_cursor
        )
        SELECT r.subject_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
    END IF;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';

-- Generated list_subjects function for project.can_manage
-- Features: Implied
CREATE OR REPLACE FUNCTION "public"."list_project_can_manage_sub"(
    p_object_id TEXT,
    p_subject_type TEXT,
    p_limit INT DEFAULT NULL,
    p_after TEXT DEFAULT NULL
) RETURNS TABLE(subject_id TEXT, next_cursor TEXT) ROWS 100 AS $$
DECLARE
    v_filter_type TEXT;
    v_filter_relation TEXT;
BEGIN
    -- Check if subject_type is a userset filter (e.g., "document#viewer")
    IF position('#' in p_subject_type) > 0 THEN
        v_filter_type := substring(p_subject_type from 1 for position('#' in p_subject_type) - 1);
        v_filter_relation := substring(p_subject_type from position('#' in p_subject_type) + 1);
        RETURN QUERY
        WITH base_results AS (
            -- Userset filter: find userset tuples that match and return normalized references
                SELECT DISTINCT split_part(t.subject_id, '#', 1) || '#' || v_filter_relation AS subject_id
                		FROM melange_tuples AS t
                		WHERE (t.object_type = 'project' AND t.relation IN ('can_manage', 'maintainer') AND t.object_id = p_object_id AND t.subject_type = v_filter_type AND position('#' in t.subject_id) > 0 AND (substring(t.subject_id from position('#' in t.subject_id) + 1) = v_filter_relation OR EXISTS (
                SELECT 1
                FROM (VALUES ('project', 'can_delete', 'can_delete'), ('project', 'can_manage', 'can_manage'), ('project', 'can_manage', 'maintainer'), ('project', 'can_read', 'can_read'), ('project', 'can_read', 'maintainer'), ('project', 'can_write', 'can_write'), ('project', 'can_write', 'maintainer'), ('project', 'maintainer', 'maintainer'), ('project', 'organization', 'organization')) AS subj_c(object_type, relation, satisfying_relation)
                WHERE (subj_c.object_type = v_filter_type AND subj_c.relation = substring(t.subject_id from position('#' in t.subject_id) + 1) AND subj_c.satisfying_relation = v_filter_relation)
                )) AND "public"."check_permission_internal"(v_filter_type, t.subject_id, 'can_manage', 'project', p_object_id, ARRAY[]::TEXT[]) = 1)
                UNION
                -- Self-candidate: when filter type matches object type
                -- e.g., querying document:1.viewer with filter document#writer
                -- should return document:1#writer if writer satisfies the relation
                SELECT p_object_id || '#' || v_filter_relation AS subject_id
                		WHERE (v_filter_type = 'project' AND EXISTS (
                SELECT 1
                FROM (VALUES ('project', 'can_delete', 'can_delete'), ('project', 'can_manage', 'can_manage'), ('project', 'can_manage', 'maintainer'), ('project', 'can_read', 'can_read'), ('project', 'can_read', 'maintainer'), ('project', 'can_write', 'can_write'), ('project', 'can_write', 'maintainer'), ('project', 'maintainer', 'maintainer'), ('project', 'organization', 'organization')) AS c(object_type, relation, satisfying_relation)
                WHERE (c.object_type = 'project' AND c.relation = 'can_manage' AND c.satisfying_relation = v_filter_relation)
                ))
        ),
        paged AS (
            SELECT br.subject_id
            FROM base_results br
            WHERE p_after IS NULL OR (
                -- Compound comparison for wildcard-first ordering:
                -- (is_not_wildcard, subject_id) > (cursor_is_not_wildcard, cursor)
                (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END, br.subject_id) >
                (CASE WHEN p_after = '*' THEN 0 ELSE 1 END, p_after)
            )
            ORDER BY (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END), br.subject_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.subject_id FROM paged p
            ORDER BY (CASE WHEN p.subject_id = '*' THEN 0 ELSE 1 END), p.subject_id
            LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT r.subject_id FROM returned r
                      ORDER BY (CASE WHEN r.subject_id = '*' THEN 0 ELSE 1 END) DESC, r.subject_id DESC
                      LIMIT 1)
            END AS next_cursor
        )
        SELECT r.subject_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
    ELSE
        -- Guard: return empty if subject type is not allowed by the model
        IF p_subject_type NOT IN ('user') THEN
        RETURN;
    END IF;
        -- Regular subject type (no userset filter)
        RETURN QUERY
        WITH base_results AS (
            -- Path 1: Direct tuple lookup with simple closure relations
                SELECT DISTINCT t.subject_id
                FROM melange_tuples AS t
                WHERE (t.object_type = 'project' AND t.relation IN ('can_manage', 'maintainer') AND t.object_id = p_object_id AND t.subject_type = p_subject_type AND position('#' in t.subject_id) = 0 AND t.subject_id <> '*')
        ),
        paged AS (
            SELECT br.subject_id
            FROM base_results br
            WHERE p_after IS NULL OR (
                -- Compound comparison for wildcard-first ordering:
                -- (is_not_wildcard, subject_id) > (cursor_is_not_wildcard, cursor)
                (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END, br.subject_id) >
                (CASE WHEN p_after = '*' THEN 0 ELSE 1 END, p_after)
            )
            ORDER BY (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END), br.subject_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.subject_id FROM paged p
            ORDER BY (CASE WHEN p.subject_id = '*' THEN 0 ELSE 1 END), p.subject_id
            LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT r.subject_id FROM returned r
                      ORDER BY (CASE WHEN r.subject_id = '*' THEN 0 ELSE 1 END) DESC, r.subject_id DESC
                      LIMIT 1)
            END AS next_cursor
        )
        SELECT r.subject_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
    END IF;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';

-- Generated list_subjects function for project.can_write
-- Features: Implied
CREATE OR REPLACE FUNCTION "public"."list_project_can_write_sub"(
    p_object_id TEXT,
    p_subject_type TEXT,
    p_limit INT DEFAULT NULL,
    p_after TEXT DEFAULT NULL
) RETURNS TABLE(subject_id TEXT, next_cursor TEXT) ROWS 100 AS $$
DECLARE
    v_filter_type TEXT;
    v_filter_relation TEXT;
BEGIN
    -- Check if subject_type is a userset filter (e.g., "document#viewer")
    IF position('#' in p_subject_type) > 0 THEN
        v_filter_type := substring(p_subject_type from 1 for position('#' in p_subject_type) - 1);
        v_filter_relation := substring(p_subject_type from position('#' in p_subject_type) + 1);
        RETURN QUERY
        WITH base_results AS (
            -- Userset filter: find userset tuples that match and return normalized references
                SELECT DISTINCT split_part(t.subject_id, '#', 1) || '#' || v_filter_relation AS subject_id
                		FROM melange_tuples AS t
                		WHERE (t.object_type = 'project' AND t.relation IN ('can_write', 'maintainer') AND t.object_id = p_object_id AND t.subject_type = v_filter_type AND position('#' in t.subject_id) > 0 AND (substring(t.subject_id from position('#' in t.subject_id) + 1) = v_filter_relation OR EXISTS (
                SELECT 1
                FROM (VALUES ('project', 'can_delete', 'can_delete'), ('project', 'can_manage', 'can_manage'), ('project', 'can_manage', 'maintainer'), ('project', 'can_read', 'can_read'), ('project', 'can_read', 'maintainer'), ('project', 'can_write', 'can_write'), ('project', 'can_write', 'maintainer'), ('project', 'maintainer', 'maintainer'), ('project', 'organization', 'organization')) AS subj_c(object_type, relation, satisfying_relation)
                WHERE (subj_c.object_type = v_filter_type AND subj_c.relation = substring(t.subject_id from position('#' in t.subject_id) + 1) AND subj_c.satisfying_relation = v_filter_relation)
                )) AND "public"."check_permission_internal"(v_filter_type, t.subject_id, 'can_write', 'project', p_object_id, ARRAY[]::TEXT[]) = 1)
                UNION
                -- Self-candidate: when filter type matches object type
                -- e.g., querying document:1.viewer with filter document#writer
                -- should return document:1#writer if writer satisfies the relation
                SELECT p_object_id || '#' || v_filter_relation AS subject_id
                		WHERE (v_filter_type = 'project' AND EXISTS (
                SELECT 1
                FROM (VALUES ('project', 'can_delete', 'can_delete'), ('project', 'can_manage', 'can_manage'), ('project', 'can_manage', 'maintainer'), ('project', 'can_read', 'can_read'), ('project', 'can_read', 'maintainer'), ('project', 'can_write', 'can_write'), ('project', 'can_write', 'maintainer'), ('project', 'maintainer', 'maintainer'), ('project', 'organization', 'organization')) AS c(object_type, relation, satisfying_relation)
                WHERE (c.object_type = 'project' AND c.relation = 'can_write' AND c.satisfying_relation = v_filter_relation)
                ))
        ),
        paged AS (
            SELECT br.subject_id
            FROM base_results br
            WHERE p_after IS NULL OR (
                -- Compound comparison for wildcard-first ordering:
                -- (is_not_wildcard, subject_id) > (cursor_is_not_wildcard, cursor)
                (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END, br.subject_id) >
                (CASE WHEN p_after = '*' THEN 0 ELSE 1 END, p_after)
            )
            ORDER BY (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END), br.subject_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.subject_id FROM paged p
            ORDER BY (CASE WHEN p.subject_id = '*' THEN 0 ELSE 1 END), p.subject_id
            LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT r.subject_id FROM returned r
                      ORDER BY (CASE WHEN r.subject_id = '*' THEN 0 ELSE 1 END) DESC, r.subject_id DESC
                      LIMIT 1)
            END AS next_cursor
        )
        SELECT r.subject_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
    ELSE
        -- Guard: return empty if subject type is not allowed by the model
        IF p_subject_type NOT IN ('user') THEN
        RETURN;
    END IF;
        -- Regular subject type (no userset filter)
        RETURN QUERY
        WITH base_results AS (
            -- Path 1: Direct tuple lookup with simple closure relations
                SELECT DISTINCT t.subject_id
                FROM melange_tuples AS t
                WHERE (t.object_type = 'project' AND t.relation IN ('can_write', 'maintainer') AND t.object_id = p_object_id AND t.subject_type = p_subject_type AND position('#' in t.subject_id) = 0 AND t.subject_id <> '*')
        ),
        paged AS (
            SELECT br.subject_id
            FROM base_results br
            WHERE p_after IS NULL OR (
                -- Compound comparison for wildcard-first ordering:
                -- (is_not_wildcard, subject_id) > (cursor_is_not_wildcard, cursor)
                (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END, br.subject_id) >
                (CASE WHEN p_after = '*' THEN 0 ELSE 1 END, p_after)
            )
            ORDER BY (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END), br.subject_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.subject_id FROM paged p
            ORDER BY (CASE WHEN p.subject_id = '*' THEN 0 ELSE 1 END), p.subject_id
            LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT r.subject_id FROM returned r
                      ORDER BY (CASE WHEN r.subject_id = '*' THEN 0 ELSE 1 END) DESC, r.subject_id DESC
                      LIMIT 1)
            END AS next_cursor
        )
        SELECT r.subject_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
    END IF;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';

-- Generated list_subjects function for organization.can_create_project
-- Features: Implied
CREATE OR REPLACE FUNCTION "public"."list_organization_can_create_project_sub"(
    p_object_id TEXT,
    p_subject_type TEXT,
    p_limit INT DEFAULT NULL,
    p_after TEXT DEFAULT NULL
) RETURNS TABLE(subject_id TEXT, next_cursor TEXT) ROWS 100 AS $$
DECLARE
    v_filter_type TEXT;
    v_filter_relation TEXT;
BEGIN
    -- Check if subject_type is a userset filter (e.g., "document#viewer")
    IF position('#' in p_subject_type) > 0 THEN
        v_filter_type := substring(p_subject_type from 1 for position('#' in p_subject_type) - 1);
        v_filter_relation := substring(p_subject_type from position('#' in p_subject_type) + 1);
        RETURN QUERY
        WITH base_results AS (
            -- Userset filter: find userset tuples that match and return normalized references
                SELECT DISTINCT split_part(t.subject_id, '#', 1) || '#' || v_filter_relation AS subject_id
                		FROM melange_tuples AS t
                		WHERE (t.object_type = 'organization' AND t.relation IN ('admin', 'can_create_project', 'owner') AND t.object_id = p_object_id AND t.subject_type = v_filter_type AND position('#' in t.subject_id) > 0 AND (substring(t.subject_id from position('#' in t.subject_id) + 1) = v_filter_relation OR EXISTS (
                SELECT 1
                FROM (VALUES ('organization', 'admin', 'admin'), ('organization', 'admin', 'owner'), ('organization', 'can_create_project', 'admin'), ('organization', 'can_create_project', 'can_create_project'), ('organization', 'can_create_project', 'owner'), ('organization', 'can_delete', 'can_delete'), ('organization', 'can_delete', 'owner'), ('organization', 'can_manage', 'can_manage'), ('organization', 'can_manage', 'owner'), ('organization', 'can_manage_members', 'admin'), ('organization', 'can_manage_members', 'can_manage_members'), ('organization', 'can_manage_members', 'owner'), ('organization', 'can_read', 'admin'), ('organization', 'can_read', 'can_read'), ('organization', 'can_read', 'member'), ('organization', 'can_read', 'owner'), ('organization', 'member', 'admin'), ('organization', 'member', 'member'), ('organization', 'member', 'owner'), ('organization', 'owner', 'owner')) AS subj_c(object_type, relation, satisfying_relation)
                WHERE (subj_c.object_type = v_filter_type AND subj_c.relation = substring(t.subject_id from position('#' in t.subject_id) + 1) AND subj_c.satisfying_relation = v_filter_relation)
                )) AND "public"."check_permission_internal"(v_filter_type, t.subject_id, 'can_create_project', 'organization', p_object_id, ARRAY[]::TEXT[]) = 1)
                UNION
                -- Self-candidate: when filter type matches object type
                -- e.g., querying document:1.viewer with filter document#writer
                -- should return document:1#writer if writer satisfies the relation
                SELECT p_object_id || '#' || v_filter_relation AS subject_id
                		WHERE (v_filter_type = 'organization' AND EXISTS (
                SELECT 1
                FROM (VALUES ('organization', 'admin', 'admin'), ('organization', 'admin', 'owner'), ('organization', 'can_create_project', 'admin'), ('organization', 'can_create_project', 'can_create_project'), ('organization', 'can_create_project', 'owner'), ('organization', 'can_delete', 'can_delete'), ('organization', 'can_delete', 'owner'), ('organization', 'can_manage', 'can_manage'), ('organization', 'can_manage', 'owner'), ('organization', 'can_manage_members', 'admin'), ('organization', 'can_manage_members', 'can_manage_members'), ('organization', 'can_manage_members', 'owner'), ('organization', 'can_read', 'admin'), ('organization', 'can_read', 'can_read'), ('organization', 'can_read', 'member'), ('organization', 'can_read', 'owner'), ('organization', 'member', 'admin'), ('organization', 'member', 'member'), ('organization', 'member', 'owner'), ('organization', 'owner', 'owner')) AS c(object_type, relation, satisfying_relation)
                WHERE (c.object_type = 'organization' AND c.relation = 'can_create_project' AND c.satisfying_relation = v_filter_relation)
                ))
        ),
        paged AS (
            SELECT br.subject_id
            FROM base_results br
            WHERE p_after IS NULL OR (
                -- Compound comparison for wildcard-first ordering:
                -- (is_not_wildcard, subject_id) > (cursor_is_not_wildcard, cursor)
                (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END, br.subject_id) >
                (CASE WHEN p_after = '*' THEN 0 ELSE 1 END, p_after)
            )
            ORDER BY (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END), br.subject_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.subject_id FROM paged p
            ORDER BY (CASE WHEN p.subject_id = '*' THEN 0 ELSE 1 END), p.subject_id
            LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT r.subject_id FROM returned r
                      ORDER BY (CASE WHEN r.subject_id = '*' THEN 0 ELSE 1 END) DESC, r.subject_id DESC
                      LIMIT 1)
            END AS next_cursor
        )
        SELECT r.subject_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
    ELSE
        -- Guard: return empty if subject type is not allowed by the model
        IF p_subject_type NOT IN ('user') THEN
        RETURN;
    END IF;
        -- Regular subject type (no userset filter)
        RETURN QUERY
        WITH base_results AS (
            -- Path 1: Direct tuple lookup with simple closure relations
                SELECT DISTINCT t.subject_id
                FROM melange_tuples AS t
                WHERE (t.object_type = 'organization' AND t.relation IN ('can_create_project', 'admin', 'owner') AND t.object_id = p_object_id AND t.subject_type = p_subject_type AND position('#' in t.subject_id) = 0 AND t.subject_id <> '*')
        ),
        paged AS (
            SELECT br.subject_id
            FROM base_results br
            WHERE p_after IS NULL OR (
                -- Compound comparison for wildcard-first ordering:
                -- (is_not_wildcard, subject_id) > (cursor_is_not_wildcard, cursor)
                (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END, br.subject_id) >
                (CASE WHEN p_after = '*' THEN 0 ELSE 1 END, p_after)
            )
            ORDER BY (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END), br.subject_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.subject_id FROM paged p
            ORDER BY (CASE WHEN p.subject_id = '*' THEN 0 ELSE 1 END), p.subject_id
            LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT r.subject_id FROM returned r
                      ORDER BY (CASE WHEN r.subject_id = '*' THEN 0 ELSE 1 END) DESC, r.subject_id DESC
                      LIMIT 1)
            END AS next_cursor
        )
        SELECT r.subject_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
    END IF;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';

-- Generated list_subjects function for organization.can_manage_members
-- Features: Implied
CREATE OR REPLACE FUNCTION "public"."list_organization_can_manage_members_sub"(
    p_object_id TEXT,
    p_subject_type TEXT,
    p_limit INT DEFAULT NULL,
    p_after TEXT DEFAULT NULL
) RETURNS TABLE(subject_id TEXT, next_cursor TEXT) ROWS 100 AS $$
DECLARE
    v_filter_type TEXT;
    v_filter_relation TEXT;
BEGIN
    -- Check if subject_type is a userset filter (e.g., "document#viewer")
    IF position('#' in p_subject_type) > 0 THEN
        v_filter_type := substring(p_subject_type from 1 for position('#' in p_subject_type) - 1);
        v_filter_relation := substring(p_subject_type from position('#' in p_subject_type) + 1);
        RETURN QUERY
        WITH base_results AS (
            -- Userset filter: find userset tuples that match and return normalized references
                SELECT DISTINCT split_part(t.subject_id, '#', 1) || '#' || v_filter_relation AS subject_id
                		FROM melange_tuples AS t
                		WHERE (t.object_type = 'organization' AND t.relation IN ('admin', 'can_manage_members', 'owner') AND t.object_id = p_object_id AND t.subject_type = v_filter_type AND position('#' in t.subject_id) > 0 AND (substring(t.subject_id from position('#' in t.subject_id) + 1) = v_filter_relation OR EXISTS (
                SELECT 1
                FROM (VALUES ('organization', 'admin', 'admin'), ('organization', 'admin', 'owner'), ('organization', 'can_create_project', 'admin'), ('organization', 'can_create_project', 'can_create_project'), ('organization', 'can_create_project', 'owner'), ('organization', 'can_delete', 'can_delete'), ('organization', 'can_delete', 'owner'), ('organization', 'can_manage', 'can_manage'), ('organization', 'can_manage', 'owner'), ('organization', 'can_manage_members', 'admin'), ('organization', 'can_manage_members', 'can_manage_members'), ('organization', 'can_manage_members', 'owner'), ('organization', 'can_read', 'admin'), ('organization', 'can_read', 'can_read'), ('organization', 'can_read', 'member'), ('organization', 'can_read', 'owner'), ('organization', 'member', 'admin'), ('organization', 'member', 'member'), ('organization', 'member', 'owner'), ('organization', 'owner', 'owner')) AS subj_c(object_type, relation, satisfying_relation)
                WHERE (subj_c.object_type = v_filter_type AND subj_c.relation = substring(t.subject_id from position('#' in t.subject_id) + 1) AND subj_c.satisfying_relation = v_filter_relation)
                )) AND "public"."check_permission_internal"(v_filter_type, t.subject_id, 'can_manage_members', 'organization', p_object_id, ARRAY[]::TEXT[]) = 1)
                UNION
                -- Self-candidate: when filter type matches object type
                -- e.g., querying document:1.viewer with filter document#writer
                -- should return document:1#writer if writer satisfies the relation
                SELECT p_object_id || '#' || v_filter_relation AS subject_id
                		WHERE (v_filter_type = 'organization' AND EXISTS (
                SELECT 1
                FROM (VALUES ('organization', 'admin', 'admin'), ('organization', 'admin', 'owner'), ('organization', 'can_create_project', 'admin'), ('organization', 'can_create_project', 'can_create_project'), ('organization', 'can_create_project', 'owner'), ('organization', 'can_delete', 'can_delete'), ('organization', 'can_delete', 'owner'), ('organization', 'can_manage', 'can_manage'), ('organization', 'can_manage', 'owner'), ('organization', 'can_manage_members', 'admin'), ('organization', 'can_manage_members', 'can_manage_members'), ('organization', 'can_manage_members', 'owner'), ('organization', 'can_read', 'admin'), ('organization', 'can_read', 'can_read'), ('organization', 'can_read', 'member'), ('organization', 'can_read', 'owner'), ('organization', 'member', 'admin'), ('organization', 'member', 'member'), ('organization', 'member', 'owner'), ('organization', 'owner', 'owner')) AS c(object_type, relation, satisfying_relation)
                WHERE (c.object_type = 'organization' AND c.relation = 'can_manage_members' AND c.satisfying_relation = v_filter_relation)
                ))
        ),
        paged AS (
            SELECT br.subject_id
            FROM base_results br
            WHERE p_after IS NULL OR (
                -- Compound comparison for wildcard-first ordering:
                -- (is_not_wildcard, subject_id) > (cursor_is_not_wildcard, cursor)
                (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END, br.subject_id) >
                (CASE WHEN p_after = '*' THEN 0 ELSE 1 END, p_after)
            )
            ORDER BY (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END), br.subject_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.subject_id FROM paged p
            ORDER BY (CASE WHEN p.subject_id = '*' THEN 0 ELSE 1 END), p.subject_id
            LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT r.subject_id FROM returned r
                      ORDER BY (CASE WHEN r.subject_id = '*' THEN 0 ELSE 1 END) DESC, r.subject_id DESC
                      LIMIT 1)
            END AS next_cursor
        )
        SELECT r.subject_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
    ELSE
        -- Guard: return empty if subject type is not allowed by the model
        IF p_subject_type NOT IN ('user') THEN
        RETURN;
    END IF;
        -- Regular subject type (no userset filter)
        RETURN QUERY
        WITH base_results AS (
            -- Path 1: Direct tuple lookup with simple closure relations
                SELECT DISTINCT t.subject_id
                FROM melange_tuples AS t
                WHERE (t.object_type = 'organization' AND t.relation IN ('can_manage_members', 'admin', 'owner') AND t.object_id = p_object_id AND t.subject_type = p_subject_type AND position('#' in t.subject_id) = 0 AND t.subject_id <> '*')
        ),
        paged AS (
            SELECT br.subject_id
            FROM base_results br
            WHERE p_after IS NULL OR (
                -- Compound comparison for wildcard-first ordering:
                -- (is_not_wildcard, subject_id) > (cursor_is_not_wildcard, cursor)
                (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END, br.subject_id) >
                (CASE WHEN p_after = '*' THEN 0 ELSE 1 END, p_after)
            )
            ORDER BY (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END), br.subject_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.subject_id FROM paged p
            ORDER BY (CASE WHEN p.subject_id = '*' THEN 0 ELSE 1 END), p.subject_id
            LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT r.subject_id FROM returned r
                      ORDER BY (CASE WHEN r.subject_id = '*' THEN 0 ELSE 1 END) DESC, r.subject_id DESC
                      LIMIT 1)
            END AS next_cursor
        )
        SELECT r.subject_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
    END IF;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';

-- Generated list_subjects function for organization.member
-- Features: Direct+Implied
CREATE OR REPLACE FUNCTION "public"."list_organization_member_sub"(
    p_object_id TEXT,
    p_subject_type TEXT,
    p_limit INT DEFAULT NULL,
    p_after TEXT DEFAULT NULL
) RETURNS TABLE(subject_id TEXT, next_cursor TEXT) ROWS 100 AS $$
DECLARE
    v_filter_type TEXT;
    v_filter_relation TEXT;
BEGIN
    -- Check if subject_type is a userset filter (e.g., "document#viewer")
    IF position('#' in p_subject_type) > 0 THEN
        v_filter_type := substring(p_subject_type from 1 for position('#' in p_subject_type) - 1);
        v_filter_relation := substring(p_subject_type from position('#' in p_subject_type) + 1);
        RETURN QUERY
        WITH base_results AS (
            -- Userset filter: find userset tuples that match and return normalized references
                SELECT DISTINCT split_part(t.subject_id, '#', 1) || '#' || v_filter_relation AS subject_id
                		FROM melange_tuples AS t
                		WHERE (t.object_type = 'organization' AND t.relation IN ('admin', 'member', 'owner') AND t.object_id = p_object_id AND t.subject_type = v_filter_type AND position('#' in t.subject_id) > 0 AND (substring(t.subject_id from position('#' in t.subject_id) + 1) = v_filter_relation OR EXISTS (
                SELECT 1
                FROM (VALUES ('organization', 'admin', 'admin'), ('organization', 'admin', 'owner'), ('organization', 'can_create_project', 'admin'), ('organization', 'can_create_project', 'can_create_project'), ('organization', 'can_create_project', 'owner'), ('organization', 'can_delete', 'can_delete'), ('organization', 'can_delete', 'owner'), ('organization', 'can_manage', 'can_manage'), ('organization', 'can_manage', 'owner'), ('organization', 'can_manage_members', 'admin'), ('organization', 'can_manage_members', 'can_manage_members'), ('organization', 'can_manage_members', 'owner'), ('organization', 'can_read', 'admin'), ('organization', 'can_read', 'can_read'), ('organization', 'can_read', 'member'), ('organization', 'can_read', 'owner'), ('organization', 'member', 'admin'), ('organization', 'member', 'member'), ('organization', 'member', 'owner'), ('organization', 'owner', 'owner')) AS subj_c(object_type, relation, satisfying_relation)
                WHERE (subj_c.object_type = v_filter_type AND subj_c.relation = substring(t.subject_id from position('#' in t.subject_id) + 1) AND subj_c.satisfying_relation = v_filter_relation)
                )) AND "public"."check_permission_internal"(v_filter_type, t.subject_id, 'member', 'organization', p_object_id, ARRAY[]::TEXT[]) = 1)
                UNION
                -- Self-candidate: when filter type matches object type
                -- e.g., querying document:1.viewer with filter document#writer
                -- should return document:1#writer if writer satisfies the relation
                SELECT p_object_id || '#' || v_filter_relation AS subject_id
                		WHERE (v_filter_type = 'organization' AND EXISTS (
                SELECT 1
                FROM (VALUES ('organization', 'admin', 'admin'), ('organization', 'admin', 'owner'), ('organization', 'can_create_project', 'admin'), ('organization', 'can_create_project', 'can_create_project'), ('organization', 'can_create_project', 'owner'), ('organization', 'can_delete', 'can_delete'), ('organization', 'can_delete', 'owner'), ('organization', 'can_manage', 'can_manage'), ('organization', 'can_manage', 'owner'), ('organization', 'can_manage_members', 'admin'), ('organization', 'can_manage_members', 'can_manage_members'), ('organization', 'can_manage_members', 'owner'), ('organization', 'can_read', 'admin'), ('organization', 'can_read', 'can_read'), ('organization', 'can_read', 'member'), ('organization', 'can_read', 'owner'), ('organization', 'member', 'admin'), ('organization', 'member', 'member'), ('organization', 'member', 'owner'), ('organization', 'owner', 'owner')) AS c(object_type, relation, satisfying_relation)
                WHERE (c.object_type = 'organization' AND c.relation = 'member' AND c.satisfying_relation = v_filter_relation)
                ))
        ),
        paged AS (
            SELECT br.subject_id
            FROM base_results br
            WHERE p_after IS NULL OR (
                -- Compound comparison for wildcard-first ordering:
                -- (is_not_wildcard, subject_id) > (cursor_is_not_wildcard, cursor)
                (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END, br.subject_id) >
                (CASE WHEN p_after = '*' THEN 0 ELSE 1 END, p_after)
            )
            ORDER BY (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END), br.subject_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.subject_id FROM paged p
            ORDER BY (CASE WHEN p.subject_id = '*' THEN 0 ELSE 1 END), p.subject_id
            LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT r.subject_id FROM returned r
                      ORDER BY (CASE WHEN r.subject_id = '*' THEN 0 ELSE 1 END) DESC, r.subject_id DESC
                      LIMIT 1)
            END AS next_cursor
        )
        SELECT r.subject_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
    ELSE
        -- Guard: return empty if subject type is not allowed by the model
        IF p_subject_type NOT IN ('user') THEN
        RETURN;
    END IF;
        -- Regular subject type (no userset filter)
        RETURN QUERY
        WITH base_results AS (
            -- Path 1: Direct tuple lookup with simple closure relations
                SELECT DISTINCT t.subject_id
                FROM melange_tuples AS t
                WHERE (t.object_type = 'organization' AND t.relation IN ('member', 'admin', 'owner') AND t.object_id = p_object_id AND t.subject_type = p_subject_type AND position('#' in t.subject_id) = 0 AND t.subject_id <> '*')
        ),
        paged AS (
            SELECT br.subject_id
            FROM base_results br
            WHERE p_after IS NULL OR (
                -- Compound comparison for wildcard-first ordering:
                -- (is_not_wildcard, subject_id) > (cursor_is_not_wildcard, cursor)
                (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END, br.subject_id) >
                (CASE WHEN p_after = '*' THEN 0 ELSE 1 END, p_after)
            )
            ORDER BY (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END), br.subject_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.subject_id FROM paged p
            ORDER BY (CASE WHEN p.subject_id = '*' THEN 0 ELSE 1 END), p.subject_id
            LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT r.subject_id FROM returned r
                      ORDER BY (CASE WHEN r.subject_id = '*' THEN 0 ELSE 1 END) DESC, r.subject_id DESC
                      LIMIT 1)
            END AS next_cursor
        )
        SELECT r.subject_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
    END IF;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';

-- Generated list_subjects function for repository.can_delete
-- Features: Recursive
-- Indirect anchor: organization.owner via ttu
CREATE OR REPLACE FUNCTION "public"."list_repository_can_delete_sub"(
    p_object_id TEXT,
    p_subject_type TEXT,
    p_limit INT DEFAULT NULL,
    p_after TEXT DEFAULT NULL
) RETURNS TABLE(subject_id TEXT, next_cursor TEXT) ROWS 100 AS $$
DECLARE
    v_is_userset_filter BOOLEAN;
    v_filter_type TEXT;
    v_filter_relation TEXT;
BEGIN
    v_is_userset_filter := position('#' in p_subject_type) > 0;
    IF v_is_userset_filter THEN
        v_filter_type := split_part(p_subject_type, '#', 1);
        v_filter_relation := split_part(p_subject_type, '#', 2);
        -- Self-candidate: when filter type matches object type
        IF v_filter_type = 'repository' THEN
        IF EXISTS (
    		SELECT p_object_id || '#' || v_filter_relation AS subject_id
    		WHERE (v_filter_type = 'repository' AND EXISTS (
    SELECT 1
    FROM (VALUES ('organization', 'admin', 'admin'), ('organization', 'admin', 'owner'), ('organization', 'can_create_project', 'admin'), ('organization', 'can_create_project', 'can_create_project'), ('organization', 'can_create_project', 'owner'), ('organization', 'can_delete', 'can_delete'), ('organization', 'can_delete', 'owner'), ('organization', 'can_manage', 'can_manage'), ('organization', 'can_manage', 'owner'), ('organization', 'can_manage_members', 'admin'), ('organization', 'can_manage_members', 'can_manage_members'), ('organization', 'can_manage_members', 'owner'), ('organization', 'can_read', 'admin'), ('organization', 'can_read', 'can_read'), ('organization', 'can_read', 'member'), ('organization', 'can_read', 'owner'), ('organization', 'member', 'admin'), ('organization', 'member', 'member'), ('organization', 'member', 'owner'), ('organization', 'owner', 'owner'), ('project', 'can_delete', 'can_delete'), ('project', 'can_manage', 'can_manage'), ('project', 'can_manage', 'maintainer'), ('project', 'can_read', 'can_read'), ('project', 'can_read', 'maintainer'), ('project', 'can_write', 'can_write'), ('project', 'can_write', 'maintainer'), ('project', 'maintainer', 'maintainer'), ('project', 'organization', 'organization'), ('repository', 'can_delete', 'can_delete'), ('repository', 'can_read', 'can_read'), ('repository', 'can_read', 'public'), ('repository', 'can_write', 'can_write'), ('repository', 'project', 'project'), ('repository', 'public', 'public')) AS c(object_type, relation, satisfying_relation)
    WHERE (c.object_type = 'repository' AND c.relation = 'can_delete' AND c.satisfying_relation = v_filter_relation)
    ))
    ) THEN
        RETURN QUERY
        WITH base_results AS (
            SELECT p_object_id || '#' || v_filter_relation AS subject_id
            		WHERE (v_filter_type = 'repository' AND EXISTS (
            SELECT 1
            FROM (VALUES ('organization', 'admin', 'admin'), ('organization', 'admin', 'owner'), ('organization', 'can_create_project', 'admin'), ('organization', 'can_create_project', 'can_create_project'), ('organization', 'can_create_project', 'owner'), ('organization', 'can_delete', 'can_delete'), ('organization', 'can_delete', 'owner'), ('organization', 'can_manage', 'can_manage'), ('organization', 'can_manage', 'owner'), ('organization', 'can_manage_members', 'admin'), ('organization', 'can_manage_members', 'can_manage_members'), ('organization', 'can_manage_members', 'owner'), ('organization', 'can_read', 'admin'), ('organization', 'can_read', 'can_read'), ('organization', 'can_read', 'member'), ('organization', 'can_read', 'owner'), ('organization', 'member', 'admin'), ('organization', 'member', 'member'), ('organization', 'member', 'owner'), ('organization', 'owner', 'owner'), ('project', 'can_delete', 'can_delete'), ('project', 'can_manage', 'can_manage'), ('project', 'can_manage', 'maintainer'), ('project', 'can_read', 'can_read'), ('project', 'can_read', 'maintainer'), ('project', 'can_write', 'can_write'), ('project', 'can_write', 'maintainer'), ('project', 'maintainer', 'maintainer'), ('project', 'organization', 'organization'), ('repository', 'can_delete', 'can_delete'), ('repository', 'can_read', 'can_read'), ('repository', 'can_read', 'public'), ('repository', 'can_write', 'can_write'), ('repository', 'project', 'project'), ('repository', 'public', 'public')) AS c(object_type, relation, satisfying_relation)
            WHERE (c.object_type = 'repository' AND c.relation = 'can_delete' AND c.satisfying_relation = v_filter_relation)
            ))
        ),
        paged AS (
            SELECT br.subject_id
            FROM base_results br
            WHERE p_after IS NULL OR (
                -- Compound comparison for wildcard-first ordering:
                -- (is_not_wildcard, subject_id) > (cursor_is_not_wildcard, cursor)
                (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END, br.subject_id) >
                (CASE WHEN p_after = '*' THEN 0 ELSE 1 END, p_after)
            )
            ORDER BY (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END), br.subject_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.subject_id FROM paged p
            ORDER BY (CASE WHEN p.subject_id = '*' THEN 0 ELSE 1 END), p.subject_id
            LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT r.subject_id FROM returned r
                      ORDER BY (CASE WHEN r.subject_id = '*' THEN 0 ELSE 1 END) DESC, r.subject_id DESC
                      LIMIT 1)
            END AS next_cursor
        )
        SELECT r.subject_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
        RETURN;
    END IF;
    END IF;
        -- Userset filter case
        RETURN QUERY
        WITH base_results AS (
            WITH subject_candidates AS (
                -- From project parents
                    SELECT DISTINCT s.subject_id
                    FROM melange_tuples AS link
                    CROSS JOIN LATERAL "public"."list_project_can_delete_sub"(link.subject_id, p_subject_type) AS s
                    WHERE (link.object_type = 'repository' AND link.object_id = p_object_id AND link.relation = 'project' AND link.subject_type = 'project')
            )
            SELECT DISTINCT sc.subject_id
            FROM subject_candidates AS sc
            WHERE "public"."check_permission_internal"(v_filter_type, sc.subject_id, 'can_delete', 'repository', p_object_id, ARRAY[]::TEXT[]) = 1
        ),
        paged AS (
            SELECT br.subject_id
            FROM base_results br
            WHERE p_after IS NULL OR (
                -- Compound comparison for wildcard-first ordering:
                -- (is_not_wildcard, subject_id) > (cursor_is_not_wildcard, cursor)
                (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END, br.subject_id) >
                (CASE WHEN p_after = '*' THEN 0 ELSE 1 END, p_after)
            )
            ORDER BY (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END), br.subject_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.subject_id FROM paged p
            ORDER BY (CASE WHEN p.subject_id = '*' THEN 0 ELSE 1 END), p.subject_id
            LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT r.subject_id FROM returned r
                      ORDER BY (CASE WHEN r.subject_id = '*' THEN 0 ELSE 1 END) DESC, r.subject_id DESC
                      LIMIT 1)
            END AS next_cursor
        )
        SELECT r.subject_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
    ELSE
        -- Direct subject type case
        IF p_subject_type NOT IN ('user') THEN
        RETURN;
    END IF;
        RETURN QUERY
        WITH base_results AS (
            WITH subject_candidates AS (
                -- From project parents
                    SELECT DISTINCT s.subject_id
                    FROM melange_tuples AS link
                    CROSS JOIN LATERAL "public"."list_project_can_delete_sub"(link.subject_id, p_subject_type) AS s
                    WHERE (link.object_type = 'repository' AND link.object_id = p_object_id AND link.relation = 'project' AND link.subject_type = 'project')
            )
            SELECT DISTINCT sc.subject_id
            FROM subject_candidates AS sc
        ),
        paged AS (
            SELECT br.subject_id
            FROM base_results br
            WHERE p_after IS NULL OR (
                -- Compound comparison for wildcard-first ordering:
                -- (is_not_wildcard, subject_id) > (cursor_is_not_wildcard, cursor)
                (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END, br.subject_id) >
                (CASE WHEN p_after = '*' THEN 0 ELSE 1 END, p_after)
            )
            ORDER BY (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END), br.subject_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.subject_id FROM paged p
            ORDER BY (CASE WHEN p.subject_id = '*' THEN 0 ELSE 1 END), p.subject_id
            LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT r.subject_id FROM returned r
                      ORDER BY (CASE WHEN r.subject_id = '*' THEN 0 ELSE 1 END) DESC, r.subject_id DESC
                      LIMIT 1)
            END AS next_cursor
        )
        SELECT r.subject_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
    END IF;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';

-- Generated list_subjects function for agent.can_manage
-- Features: Recursive
-- Indirect anchor: project.can_manage via ttu
CREATE OR REPLACE FUNCTION "public"."list_agent_can_manage_sub"(
    p_object_id TEXT,
    p_subject_type TEXT,
    p_limit INT DEFAULT NULL,
    p_after TEXT DEFAULT NULL
) RETURNS TABLE(subject_id TEXT, next_cursor TEXT) ROWS 100 AS $$
DECLARE
    v_is_userset_filter BOOLEAN;
    v_filter_type TEXT;
    v_filter_relation TEXT;
BEGIN
    v_is_userset_filter := position('#' in p_subject_type) > 0;
    IF v_is_userset_filter THEN
        v_filter_type := split_part(p_subject_type, '#', 1);
        v_filter_relation := split_part(p_subject_type, '#', 2);
        -- Self-candidate: when filter type matches object type
        IF v_filter_type = 'agent' THEN
        IF EXISTS (
    		SELECT p_object_id || '#' || v_filter_relation AS subject_id
    		WHERE (v_filter_type = 'agent' AND EXISTS (
    SELECT 1
    FROM (VALUES ('agent', 'can_execute', 'can_execute'), ('agent', 'can_manage', 'can_manage'), ('agent', 'can_read', 'can_read'), ('agent', 'project', 'project'), ('project', 'can_delete', 'can_delete'), ('project', 'can_manage', 'can_manage'), ('project', 'can_manage', 'maintainer'), ('project', 'can_read', 'can_read'), ('project', 'can_read', 'maintainer'), ('project', 'can_write', 'can_write'), ('project', 'can_write', 'maintainer'), ('project', 'maintainer', 'maintainer'), ('project', 'organization', 'organization')) AS c(object_type, relation, satisfying_relation)
    WHERE (c.object_type = 'agent' AND c.relation = 'can_manage' AND c.satisfying_relation = v_filter_relation)
    ))
    ) THEN
        RETURN QUERY
        WITH base_results AS (
            SELECT p_object_id || '#' || v_filter_relation AS subject_id
            		WHERE (v_filter_type = 'agent' AND EXISTS (
            SELECT 1
            FROM (VALUES ('agent', 'can_execute', 'can_execute'), ('agent', 'can_manage', 'can_manage'), ('agent', 'can_read', 'can_read'), ('agent', 'project', 'project'), ('project', 'can_delete', 'can_delete'), ('project', 'can_manage', 'can_manage'), ('project', 'can_manage', 'maintainer'), ('project', 'can_read', 'can_read'), ('project', 'can_read', 'maintainer'), ('project', 'can_write', 'can_write'), ('project', 'can_write', 'maintainer'), ('project', 'maintainer', 'maintainer'), ('project', 'organization', 'organization')) AS c(object_type, relation, satisfying_relation)
            WHERE (c.object_type = 'agent' AND c.relation = 'can_manage' AND c.satisfying_relation = v_filter_relation)
            ))
        ),
        paged AS (
            SELECT br.subject_id
            FROM base_results br
            WHERE p_after IS NULL OR (
                -- Compound comparison for wildcard-first ordering:
                -- (is_not_wildcard, subject_id) > (cursor_is_not_wildcard, cursor)
                (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END, br.subject_id) >
                (CASE WHEN p_after = '*' THEN 0 ELSE 1 END, p_after)
            )
            ORDER BY (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END), br.subject_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.subject_id FROM paged p
            ORDER BY (CASE WHEN p.subject_id = '*' THEN 0 ELSE 1 END), p.subject_id
            LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT r.subject_id FROM returned r
                      ORDER BY (CASE WHEN r.subject_id = '*' THEN 0 ELSE 1 END) DESC, r.subject_id DESC
                      LIMIT 1)
            END AS next_cursor
        )
        SELECT r.subject_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
        RETURN;
    END IF;
    END IF;
        -- Userset filter case
        RETURN QUERY
        WITH base_results AS (
            WITH subject_candidates AS (
                -- From project parents
                    SELECT DISTINCT s.subject_id
                    FROM melange_tuples AS link
                    CROSS JOIN LATERAL "public"."list_project_can_manage_sub"(link.subject_id, p_subject_type) AS s
                    WHERE (link.object_type = 'agent' AND link.object_id = p_object_id AND link.relation = 'project' AND link.subject_type = 'project')
            )
            SELECT DISTINCT sc.subject_id
            FROM subject_candidates AS sc
            WHERE "public"."check_permission_internal"(v_filter_type, sc.subject_id, 'can_manage', 'agent', p_object_id, ARRAY[]::TEXT[]) = 1
        ),
        paged AS (
            SELECT br.subject_id
            FROM base_results br
            WHERE p_after IS NULL OR (
                -- Compound comparison for wildcard-first ordering:
                -- (is_not_wildcard, subject_id) > (cursor_is_not_wildcard, cursor)
                (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END, br.subject_id) >
                (CASE WHEN p_after = '*' THEN 0 ELSE 1 END, p_after)
            )
            ORDER BY (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END), br.subject_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.subject_id FROM paged p
            ORDER BY (CASE WHEN p.subject_id = '*' THEN 0 ELSE 1 END), p.subject_id
            LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT r.subject_id FROM returned r
                      ORDER BY (CASE WHEN r.subject_id = '*' THEN 0 ELSE 1 END) DESC, r.subject_id DESC
                      LIMIT 1)
            END AS next_cursor
        )
        SELECT r.subject_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
    ELSE
        -- Direct subject type case
        IF p_subject_type NOT IN ('user') THEN
        RETURN;
    END IF;
        RETURN QUERY
        WITH base_results AS (
            WITH subject_candidates AS (
                -- From project parents
                    SELECT DISTINCT s.subject_id
                    FROM melange_tuples AS link
                    CROSS JOIN LATERAL "public"."list_project_can_manage_sub"(link.subject_id, p_subject_type) AS s
                    WHERE (link.object_type = 'agent' AND link.object_id = p_object_id AND link.relation = 'project' AND link.subject_type = 'project')
            )
            SELECT DISTINCT sc.subject_id
            FROM subject_candidates AS sc
        ),
        paged AS (
            SELECT br.subject_id
            FROM base_results br
            WHERE p_after IS NULL OR (
                -- Compound comparison for wildcard-first ordering:
                -- (is_not_wildcard, subject_id) > (cursor_is_not_wildcard, cursor)
                (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END, br.subject_id) >
                (CASE WHEN p_after = '*' THEN 0 ELSE 1 END, p_after)
            )
            ORDER BY (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END), br.subject_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.subject_id FROM paged p
            ORDER BY (CASE WHEN p.subject_id = '*' THEN 0 ELSE 1 END), p.subject_id
            LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT r.subject_id FROM returned r
                      ORDER BY (CASE WHEN r.subject_id = '*' THEN 0 ELSE 1 END) DESC, r.subject_id DESC
                      LIMIT 1)
            END AS next_cursor
        )
        SELECT r.subject_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
    END IF;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';

-- Generated list_subjects function for agent.can_execute
-- Features: Recursive
-- Indirect anchor: project.can_write via ttu
CREATE OR REPLACE FUNCTION "public"."list_agent_can_execute_sub"(
    p_object_id TEXT,
    p_subject_type TEXT,
    p_limit INT DEFAULT NULL,
    p_after TEXT DEFAULT NULL
) RETURNS TABLE(subject_id TEXT, next_cursor TEXT) ROWS 100 AS $$
DECLARE
    v_is_userset_filter BOOLEAN;
    v_filter_type TEXT;
    v_filter_relation TEXT;
BEGIN
    v_is_userset_filter := position('#' in p_subject_type) > 0;
    IF v_is_userset_filter THEN
        v_filter_type := split_part(p_subject_type, '#', 1);
        v_filter_relation := split_part(p_subject_type, '#', 2);
        -- Self-candidate: when filter type matches object type
        IF v_filter_type = 'agent' THEN
        IF EXISTS (
    		SELECT p_object_id || '#' || v_filter_relation AS subject_id
    		WHERE (v_filter_type = 'agent' AND EXISTS (
    SELECT 1
    FROM (VALUES ('agent', 'can_execute', 'can_execute'), ('agent', 'can_manage', 'can_manage'), ('agent', 'can_read', 'can_read'), ('agent', 'project', 'project'), ('project', 'can_delete', 'can_delete'), ('project', 'can_manage', 'can_manage'), ('project', 'can_manage', 'maintainer'), ('project', 'can_read', 'can_read'), ('project', 'can_read', 'maintainer'), ('project', 'can_write', 'can_write'), ('project', 'can_write', 'maintainer'), ('project', 'maintainer', 'maintainer'), ('project', 'organization', 'organization')) AS c(object_type, relation, satisfying_relation)
    WHERE (c.object_type = 'agent' AND c.relation = 'can_execute' AND c.satisfying_relation = v_filter_relation)
    ))
    ) THEN
        RETURN QUERY
        WITH base_results AS (
            SELECT p_object_id || '#' || v_filter_relation AS subject_id
            		WHERE (v_filter_type = 'agent' AND EXISTS (
            SELECT 1
            FROM (VALUES ('agent', 'can_execute', 'can_execute'), ('agent', 'can_manage', 'can_manage'), ('agent', 'can_read', 'can_read'), ('agent', 'project', 'project'), ('project', 'can_delete', 'can_delete'), ('project', 'can_manage', 'can_manage'), ('project', 'can_manage', 'maintainer'), ('project', 'can_read', 'can_read'), ('project', 'can_read', 'maintainer'), ('project', 'can_write', 'can_write'), ('project', 'can_write', 'maintainer'), ('project', 'maintainer', 'maintainer'), ('project', 'organization', 'organization')) AS c(object_type, relation, satisfying_relation)
            WHERE (c.object_type = 'agent' AND c.relation = 'can_execute' AND c.satisfying_relation = v_filter_relation)
            ))
        ),
        paged AS (
            SELECT br.subject_id
            FROM base_results br
            WHERE p_after IS NULL OR (
                -- Compound comparison for wildcard-first ordering:
                -- (is_not_wildcard, subject_id) > (cursor_is_not_wildcard, cursor)
                (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END, br.subject_id) >
                (CASE WHEN p_after = '*' THEN 0 ELSE 1 END, p_after)
            )
            ORDER BY (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END), br.subject_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.subject_id FROM paged p
            ORDER BY (CASE WHEN p.subject_id = '*' THEN 0 ELSE 1 END), p.subject_id
            LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT r.subject_id FROM returned r
                      ORDER BY (CASE WHEN r.subject_id = '*' THEN 0 ELSE 1 END) DESC, r.subject_id DESC
                      LIMIT 1)
            END AS next_cursor
        )
        SELECT r.subject_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
        RETURN;
    END IF;
    END IF;
        -- Userset filter case
        RETURN QUERY
        WITH base_results AS (
            WITH subject_candidates AS (
                -- From project parents
                    SELECT DISTINCT s.subject_id
                    FROM melange_tuples AS link
                    CROSS JOIN LATERAL "public"."list_project_can_write_sub"(link.subject_id, p_subject_type) AS s
                    WHERE (link.object_type = 'agent' AND link.object_id = p_object_id AND link.relation = 'project' AND link.subject_type = 'project')
            )
            SELECT DISTINCT sc.subject_id
            FROM subject_candidates AS sc
            WHERE "public"."check_permission_internal"(v_filter_type, sc.subject_id, 'can_execute', 'agent', p_object_id, ARRAY[]::TEXT[]) = 1
        ),
        paged AS (
            SELECT br.subject_id
            FROM base_results br
            WHERE p_after IS NULL OR (
                -- Compound comparison for wildcard-first ordering:
                -- (is_not_wildcard, subject_id) > (cursor_is_not_wildcard, cursor)
                (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END, br.subject_id) >
                (CASE WHEN p_after = '*' THEN 0 ELSE 1 END, p_after)
            )
            ORDER BY (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END), br.subject_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.subject_id FROM paged p
            ORDER BY (CASE WHEN p.subject_id = '*' THEN 0 ELSE 1 END), p.subject_id
            LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT r.subject_id FROM returned r
                      ORDER BY (CASE WHEN r.subject_id = '*' THEN 0 ELSE 1 END) DESC, r.subject_id DESC
                      LIMIT 1)
            END AS next_cursor
        )
        SELECT r.subject_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
    ELSE
        -- Direct subject type case
        IF p_subject_type NOT IN ('user') THEN
        RETURN;
    END IF;
        RETURN QUERY
        WITH base_results AS (
            WITH subject_candidates AS (
                -- From project parents
                    SELECT DISTINCT s.subject_id
                    FROM melange_tuples AS link
                    CROSS JOIN LATERAL "public"."list_project_can_write_sub"(link.subject_id, p_subject_type) AS s
                    WHERE (link.object_type = 'agent' AND link.object_id = p_object_id AND link.relation = 'project' AND link.subject_type = 'project')
            )
            SELECT DISTINCT sc.subject_id
            FROM subject_candidates AS sc
        ),
        paged AS (
            SELECT br.subject_id
            FROM base_results br
            WHERE p_after IS NULL OR (
                -- Compound comparison for wildcard-first ordering:
                -- (is_not_wildcard, subject_id) > (cursor_is_not_wildcard, cursor)
                (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END, br.subject_id) >
                (CASE WHEN p_after = '*' THEN 0 ELSE 1 END, p_after)
            )
            ORDER BY (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END), br.subject_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.subject_id FROM paged p
            ORDER BY (CASE WHEN p.subject_id = '*' THEN 0 ELSE 1 END), p.subject_id
            LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT r.subject_id FROM returned r
                      ORDER BY (CASE WHEN r.subject_id = '*' THEN 0 ELSE 1 END) DESC, r.subject_id DESC
                      LIMIT 1)
            END AS next_cursor
        )
        SELECT r.subject_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
    END IF;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';

-- Generated list_subjects function for repository.can_write
-- Features: Recursive
-- Indirect anchor: project.can_write via ttu
CREATE OR REPLACE FUNCTION "public"."list_repository_can_write_sub"(
    p_object_id TEXT,
    p_subject_type TEXT,
    p_limit INT DEFAULT NULL,
    p_after TEXT DEFAULT NULL
) RETURNS TABLE(subject_id TEXT, next_cursor TEXT) ROWS 100 AS $$
DECLARE
    v_is_userset_filter BOOLEAN;
    v_filter_type TEXT;
    v_filter_relation TEXT;
BEGIN
    v_is_userset_filter := position('#' in p_subject_type) > 0;
    IF v_is_userset_filter THEN
        v_filter_type := split_part(p_subject_type, '#', 1);
        v_filter_relation := split_part(p_subject_type, '#', 2);
        -- Self-candidate: when filter type matches object type
        IF v_filter_type = 'repository' THEN
        IF EXISTS (
    		SELECT p_object_id || '#' || v_filter_relation AS subject_id
    		WHERE (v_filter_type = 'repository' AND EXISTS (
    SELECT 1
    FROM (VALUES ('project', 'can_delete', 'can_delete'), ('project', 'can_manage', 'can_manage'), ('project', 'can_manage', 'maintainer'), ('project', 'can_read', 'can_read'), ('project', 'can_read', 'maintainer'), ('project', 'can_write', 'can_write'), ('project', 'can_write', 'maintainer'), ('project', 'maintainer', 'maintainer'), ('project', 'organization', 'organization'), ('repository', 'can_delete', 'can_delete'), ('repository', 'can_read', 'can_read'), ('repository', 'can_read', 'public'), ('repository', 'can_write', 'can_write'), ('repository', 'project', 'project'), ('repository', 'public', 'public')) AS c(object_type, relation, satisfying_relation)
    WHERE (c.object_type = 'repository' AND c.relation = 'can_write' AND c.satisfying_relation = v_filter_relation)
    ))
    ) THEN
        RETURN QUERY
        WITH base_results AS (
            SELECT p_object_id || '#' || v_filter_relation AS subject_id
            		WHERE (v_filter_type = 'repository' AND EXISTS (
            SELECT 1
            FROM (VALUES ('project', 'can_delete', 'can_delete'), ('project', 'can_manage', 'can_manage'), ('project', 'can_manage', 'maintainer'), ('project', 'can_read', 'can_read'), ('project', 'can_read', 'maintainer'), ('project', 'can_write', 'can_write'), ('project', 'can_write', 'maintainer'), ('project', 'maintainer', 'maintainer'), ('project', 'organization', 'organization'), ('repository', 'can_delete', 'can_delete'), ('repository', 'can_read', 'can_read'), ('repository', 'can_read', 'public'), ('repository', 'can_write', 'can_write'), ('repository', 'project', 'project'), ('repository', 'public', 'public')) AS c(object_type, relation, satisfying_relation)
            WHERE (c.object_type = 'repository' AND c.relation = 'can_write' AND c.satisfying_relation = v_filter_relation)
            ))
        ),
        paged AS (
            SELECT br.subject_id
            FROM base_results br
            WHERE p_after IS NULL OR (
                -- Compound comparison for wildcard-first ordering:
                -- (is_not_wildcard, subject_id) > (cursor_is_not_wildcard, cursor)
                (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END, br.subject_id) >
                (CASE WHEN p_after = '*' THEN 0 ELSE 1 END, p_after)
            )
            ORDER BY (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END), br.subject_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.subject_id FROM paged p
            ORDER BY (CASE WHEN p.subject_id = '*' THEN 0 ELSE 1 END), p.subject_id
            LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT r.subject_id FROM returned r
                      ORDER BY (CASE WHEN r.subject_id = '*' THEN 0 ELSE 1 END) DESC, r.subject_id DESC
                      LIMIT 1)
            END AS next_cursor
        )
        SELECT r.subject_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
        RETURN;
    END IF;
    END IF;
        -- Userset filter case
        RETURN QUERY
        WITH base_results AS (
            WITH subject_candidates AS (
                -- From project parents
                    SELECT DISTINCT s.subject_id
                    FROM melange_tuples AS link
                    CROSS JOIN LATERAL "public"."list_project_can_write_sub"(link.subject_id, p_subject_type) AS s
                    WHERE (link.object_type = 'repository' AND link.object_id = p_object_id AND link.relation = 'project' AND link.subject_type = 'project')
            )
            SELECT DISTINCT sc.subject_id
            FROM subject_candidates AS sc
            WHERE "public"."check_permission_internal"(v_filter_type, sc.subject_id, 'can_write', 'repository', p_object_id, ARRAY[]::TEXT[]) = 1
        ),
        paged AS (
            SELECT br.subject_id
            FROM base_results br
            WHERE p_after IS NULL OR (
                -- Compound comparison for wildcard-first ordering:
                -- (is_not_wildcard, subject_id) > (cursor_is_not_wildcard, cursor)
                (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END, br.subject_id) >
                (CASE WHEN p_after = '*' THEN 0 ELSE 1 END, p_after)
            )
            ORDER BY (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END), br.subject_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.subject_id FROM paged p
            ORDER BY (CASE WHEN p.subject_id = '*' THEN 0 ELSE 1 END), p.subject_id
            LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT r.subject_id FROM returned r
                      ORDER BY (CASE WHEN r.subject_id = '*' THEN 0 ELSE 1 END) DESC, r.subject_id DESC
                      LIMIT 1)
            END AS next_cursor
        )
        SELECT r.subject_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
    ELSE
        -- Direct subject type case
        IF p_subject_type NOT IN ('user') THEN
        RETURN;
    END IF;
        RETURN QUERY
        WITH base_results AS (
            WITH subject_candidates AS (
                -- From project parents
                    SELECT DISTINCT s.subject_id
                    FROM melange_tuples AS link
                    CROSS JOIN LATERAL "public"."list_project_can_write_sub"(link.subject_id, p_subject_type) AS s
                    WHERE (link.object_type = 'repository' AND link.object_id = p_object_id AND link.relation = 'project' AND link.subject_type = 'project')
            )
            SELECT DISTINCT sc.subject_id
            FROM subject_candidates AS sc
        ),
        paged AS (
            SELECT br.subject_id
            FROM base_results br
            WHERE p_after IS NULL OR (
                -- Compound comparison for wildcard-first ordering:
                -- (is_not_wildcard, subject_id) > (cursor_is_not_wildcard, cursor)
                (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END, br.subject_id) >
                (CASE WHEN p_after = '*' THEN 0 ELSE 1 END, p_after)
            )
            ORDER BY (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END), br.subject_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.subject_id FROM paged p
            ORDER BY (CASE WHEN p.subject_id = '*' THEN 0 ELSE 1 END), p.subject_id
            LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT r.subject_id FROM returned r
                      ORDER BY (CASE WHEN r.subject_id = '*' THEN 0 ELSE 1 END) DESC, r.subject_id DESC
                      LIMIT 1)
            END AS next_cursor
        )
        SELECT r.subject_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
    END IF;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';

-- Generated list_subjects function for organization.can_read
-- Features: Implied
CREATE OR REPLACE FUNCTION "public"."list_organization_can_read_sub"(
    p_object_id TEXT,
    p_subject_type TEXT,
    p_limit INT DEFAULT NULL,
    p_after TEXT DEFAULT NULL
) RETURNS TABLE(subject_id TEXT, next_cursor TEXT) ROWS 100 AS $$
DECLARE
    v_filter_type TEXT;
    v_filter_relation TEXT;
BEGIN
    -- Check if subject_type is a userset filter (e.g., "document#viewer")
    IF position('#' in p_subject_type) > 0 THEN
        v_filter_type := substring(p_subject_type from 1 for position('#' in p_subject_type) - 1);
        v_filter_relation := substring(p_subject_type from position('#' in p_subject_type) + 1);
        RETURN QUERY
        WITH base_results AS (
            -- Userset filter: find userset tuples that match and return normalized references
                SELECT DISTINCT split_part(t.subject_id, '#', 1) || '#' || v_filter_relation AS subject_id
                		FROM melange_tuples AS t
                		WHERE (t.object_type = 'organization' AND t.relation IN ('admin', 'can_read', 'member', 'owner') AND t.object_id = p_object_id AND t.subject_type = v_filter_type AND position('#' in t.subject_id) > 0 AND (substring(t.subject_id from position('#' in t.subject_id) + 1) = v_filter_relation OR EXISTS (
                SELECT 1
                FROM (VALUES ('organization', 'admin', 'admin'), ('organization', 'admin', 'owner'), ('organization', 'can_create_project', 'admin'), ('organization', 'can_create_project', 'can_create_project'), ('organization', 'can_create_project', 'owner'), ('organization', 'can_delete', 'can_delete'), ('organization', 'can_delete', 'owner'), ('organization', 'can_manage', 'can_manage'), ('organization', 'can_manage', 'owner'), ('organization', 'can_manage_members', 'admin'), ('organization', 'can_manage_members', 'can_manage_members'), ('organization', 'can_manage_members', 'owner'), ('organization', 'can_read', 'admin'), ('organization', 'can_read', 'can_read'), ('organization', 'can_read', 'member'), ('organization', 'can_read', 'owner'), ('organization', 'member', 'admin'), ('organization', 'member', 'member'), ('organization', 'member', 'owner'), ('organization', 'owner', 'owner')) AS subj_c(object_type, relation, satisfying_relation)
                WHERE (subj_c.object_type = v_filter_type AND subj_c.relation = substring(t.subject_id from position('#' in t.subject_id) + 1) AND subj_c.satisfying_relation = v_filter_relation)
                )) AND "public"."check_permission_internal"(v_filter_type, t.subject_id, 'can_read', 'organization', p_object_id, ARRAY[]::TEXT[]) = 1)
                UNION
                -- Self-candidate: when filter type matches object type
                -- e.g., querying document:1.viewer with filter document#writer
                -- should return document:1#writer if writer satisfies the relation
                SELECT p_object_id || '#' || v_filter_relation AS subject_id
                		WHERE (v_filter_type = 'organization' AND EXISTS (
                SELECT 1
                FROM (VALUES ('organization', 'admin', 'admin'), ('organization', 'admin', 'owner'), ('organization', 'can_create_project', 'admin'), ('organization', 'can_create_project', 'can_create_project'), ('organization', 'can_create_project', 'owner'), ('organization', 'can_delete', 'can_delete'), ('organization', 'can_delete', 'owner'), ('organization', 'can_manage', 'can_manage'), ('organization', 'can_manage', 'owner'), ('organization', 'can_manage_members', 'admin'), ('organization', 'can_manage_members', 'can_manage_members'), ('organization', 'can_manage_members', 'owner'), ('organization', 'can_read', 'admin'), ('organization', 'can_read', 'can_read'), ('organization', 'can_read', 'member'), ('organization', 'can_read', 'owner'), ('organization', 'member', 'admin'), ('organization', 'member', 'member'), ('organization', 'member', 'owner'), ('organization', 'owner', 'owner')) AS c(object_type, relation, satisfying_relation)
                WHERE (c.object_type = 'organization' AND c.relation = 'can_read' AND c.satisfying_relation = v_filter_relation)
                ))
        ),
        paged AS (
            SELECT br.subject_id
            FROM base_results br
            WHERE p_after IS NULL OR (
                -- Compound comparison for wildcard-first ordering:
                -- (is_not_wildcard, subject_id) > (cursor_is_not_wildcard, cursor)
                (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END, br.subject_id) >
                (CASE WHEN p_after = '*' THEN 0 ELSE 1 END, p_after)
            )
            ORDER BY (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END), br.subject_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.subject_id FROM paged p
            ORDER BY (CASE WHEN p.subject_id = '*' THEN 0 ELSE 1 END), p.subject_id
            LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT r.subject_id FROM returned r
                      ORDER BY (CASE WHEN r.subject_id = '*' THEN 0 ELSE 1 END) DESC, r.subject_id DESC
                      LIMIT 1)
            END AS next_cursor
        )
        SELECT r.subject_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
    ELSE
        -- Guard: return empty if subject type is not allowed by the model
        IF p_subject_type NOT IN ('user') THEN
        RETURN;
    END IF;
        -- Regular subject type (no userset filter)
        RETURN QUERY
        WITH base_results AS (
            -- Path 1: Direct tuple lookup with simple closure relations
                SELECT DISTINCT t.subject_id
                FROM melange_tuples AS t
                WHERE (t.object_type = 'organization' AND t.relation IN ('can_read', 'admin', 'member', 'owner') AND t.object_id = p_object_id AND t.subject_type = p_subject_type AND position('#' in t.subject_id) = 0 AND t.subject_id <> '*')
        ),
        paged AS (
            SELECT br.subject_id
            FROM base_results br
            WHERE p_after IS NULL OR (
                -- Compound comparison for wildcard-first ordering:
                -- (is_not_wildcard, subject_id) > (cursor_is_not_wildcard, cursor)
                (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END, br.subject_id) >
                (CASE WHEN p_after = '*' THEN 0 ELSE 1 END, p_after)
            )
            ORDER BY (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END), br.subject_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.subject_id FROM paged p
            ORDER BY (CASE WHEN p.subject_id = '*' THEN 0 ELSE 1 END), p.subject_id
            LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT r.subject_id FROM returned r
                      ORDER BY (CASE WHEN r.subject_id = '*' THEN 0 ELSE 1 END) DESC, r.subject_id DESC
                      LIMIT 1)
            END AS next_cursor
        )
        SELECT r.subject_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
    END IF;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';

-- Generated list_subjects function for state_volume.can_manage
-- Features: Recursive
-- Indirect anchor: project.can_manage via ttu
CREATE OR REPLACE FUNCTION "public"."list_state_volume_can_manage_sub"(
    p_object_id TEXT,
    p_subject_type TEXT,
    p_limit INT DEFAULT NULL,
    p_after TEXT DEFAULT NULL
) RETURNS TABLE(subject_id TEXT, next_cursor TEXT) ROWS 100 AS $$
DECLARE
    v_is_userset_filter BOOLEAN;
    v_filter_type TEXT;
    v_filter_relation TEXT;
BEGIN
    v_is_userset_filter := position('#' in p_subject_type) > 0;
    IF v_is_userset_filter THEN
        v_filter_type := split_part(p_subject_type, '#', 1);
        v_filter_relation := split_part(p_subject_type, '#', 2);
        -- Self-candidate: when filter type matches object type
        IF v_filter_type = 'state_volume' THEN
        IF EXISTS (
    		SELECT p_object_id || '#' || v_filter_relation AS subject_id
    		WHERE (v_filter_type = 'state_volume' AND EXISTS (
    SELECT 1
    FROM (VALUES ('agent', 'can_execute', 'can_execute'), ('agent', 'can_manage', 'can_manage'), ('agent', 'can_read', 'can_read'), ('agent', 'project', 'project'), ('project', 'can_delete', 'can_delete'), ('project', 'can_manage', 'can_manage'), ('project', 'can_manage', 'maintainer'), ('project', 'can_read', 'can_read'), ('project', 'can_read', 'maintainer'), ('project', 'can_write', 'can_write'), ('project', 'can_write', 'maintainer'), ('project', 'maintainer', 'maintainer'), ('project', 'organization', 'organization'), ('state_volume', 'agent', 'agent'), ('state_volume', 'can_attach', 'can_attach'), ('state_volume', 'can_manage', 'can_manage'), ('state_volume', 'can_read', 'can_read'), ('state_volume', 'can_restore', 'can_restore')) AS c(object_type, relation, satisfying_relation)
    WHERE (c.object_type = 'state_volume' AND c.relation = 'can_manage' AND c.satisfying_relation = v_filter_relation)
    ))
    ) THEN
        RETURN QUERY
        WITH base_results AS (
            SELECT p_object_id || '#' || v_filter_relation AS subject_id
            		WHERE (v_filter_type = 'state_volume' AND EXISTS (
            SELECT 1
            FROM (VALUES ('agent', 'can_execute', 'can_execute'), ('agent', 'can_manage', 'can_manage'), ('agent', 'can_read', 'can_read'), ('agent', 'project', 'project'), ('project', 'can_delete', 'can_delete'), ('project', 'can_manage', 'can_manage'), ('project', 'can_manage', 'maintainer'), ('project', 'can_read', 'can_read'), ('project', 'can_read', 'maintainer'), ('project', 'can_write', 'can_write'), ('project', 'can_write', 'maintainer'), ('project', 'maintainer', 'maintainer'), ('project', 'organization', 'organization'), ('state_volume', 'agent', 'agent'), ('state_volume', 'can_attach', 'can_attach'), ('state_volume', 'can_manage', 'can_manage'), ('state_volume', 'can_read', 'can_read'), ('state_volume', 'can_restore', 'can_restore')) AS c(object_type, relation, satisfying_relation)
            WHERE (c.object_type = 'state_volume' AND c.relation = 'can_manage' AND c.satisfying_relation = v_filter_relation)
            ))
        ),
        paged AS (
            SELECT br.subject_id
            FROM base_results br
            WHERE p_after IS NULL OR (
                -- Compound comparison for wildcard-first ordering:
                -- (is_not_wildcard, subject_id) > (cursor_is_not_wildcard, cursor)
                (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END, br.subject_id) >
                (CASE WHEN p_after = '*' THEN 0 ELSE 1 END, p_after)
            )
            ORDER BY (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END), br.subject_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.subject_id FROM paged p
            ORDER BY (CASE WHEN p.subject_id = '*' THEN 0 ELSE 1 END), p.subject_id
            LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT r.subject_id FROM returned r
                      ORDER BY (CASE WHEN r.subject_id = '*' THEN 0 ELSE 1 END) DESC, r.subject_id DESC
                      LIMIT 1)
            END AS next_cursor
        )
        SELECT r.subject_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
        RETURN;
    END IF;
    END IF;
        -- Userset filter case
        RETURN QUERY
        WITH base_results AS (
            WITH subject_candidates AS (
                -- From agent parents
                    SELECT DISTINCT s.subject_id
                    FROM melange_tuples AS link
                    CROSS JOIN LATERAL "public"."list_agent_can_manage_sub"(link.subject_id, p_subject_type) AS s
                    WHERE (link.object_type = 'state_volume' AND link.object_id = p_object_id AND link.relation = 'agent' AND link.subject_type = 'agent')
            )
            SELECT DISTINCT sc.subject_id
            FROM subject_candidates AS sc
            WHERE "public"."check_permission_internal"(v_filter_type, sc.subject_id, 'can_manage', 'state_volume', p_object_id, ARRAY[]::TEXT[]) = 1
        ),
        paged AS (
            SELECT br.subject_id
            FROM base_results br
            WHERE p_after IS NULL OR (
                -- Compound comparison for wildcard-first ordering:
                -- (is_not_wildcard, subject_id) > (cursor_is_not_wildcard, cursor)
                (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END, br.subject_id) >
                (CASE WHEN p_after = '*' THEN 0 ELSE 1 END, p_after)
            )
            ORDER BY (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END), br.subject_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.subject_id FROM paged p
            ORDER BY (CASE WHEN p.subject_id = '*' THEN 0 ELSE 1 END), p.subject_id
            LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT r.subject_id FROM returned r
                      ORDER BY (CASE WHEN r.subject_id = '*' THEN 0 ELSE 1 END) DESC, r.subject_id DESC
                      LIMIT 1)
            END AS next_cursor
        )
        SELECT r.subject_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
    ELSE
        -- Direct subject type case
        IF p_subject_type NOT IN ('user') THEN
        RETURN;
    END IF;
        RETURN QUERY
        WITH base_results AS (
            WITH subject_candidates AS (
                -- From agent parents
                    SELECT DISTINCT s.subject_id
                    FROM melange_tuples AS link
                    CROSS JOIN LATERAL "public"."list_agent_can_manage_sub"(link.subject_id, p_subject_type) AS s
                    WHERE (link.object_type = 'state_volume' AND link.object_id = p_object_id AND link.relation = 'agent' AND link.subject_type = 'agent')
            )
            SELECT DISTINCT sc.subject_id
            FROM subject_candidates AS sc
        ),
        paged AS (
            SELECT br.subject_id
            FROM base_results br
            WHERE p_after IS NULL OR (
                -- Compound comparison for wildcard-first ordering:
                -- (is_not_wildcard, subject_id) > (cursor_is_not_wildcard, cursor)
                (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END, br.subject_id) >
                (CASE WHEN p_after = '*' THEN 0 ELSE 1 END, p_after)
            )
            ORDER BY (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END), br.subject_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.subject_id FROM paged p
            ORDER BY (CASE WHEN p.subject_id = '*' THEN 0 ELSE 1 END), p.subject_id
            LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT r.subject_id FROM returned r
                      ORDER BY (CASE WHEN r.subject_id = '*' THEN 0 ELSE 1 END) DESC, r.subject_id DESC
                      LIMIT 1)
            END AS next_cursor
        )
        SELECT r.subject_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
    END IF;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';

-- Generated list_subjects function for state_volume.can_restore
-- Features: Recursive
-- Indirect anchor: project.can_manage via ttu
CREATE OR REPLACE FUNCTION "public"."list_state_volume_can_restore_sub"(
    p_object_id TEXT,
    p_subject_type TEXT,
    p_limit INT DEFAULT NULL,
    p_after TEXT DEFAULT NULL
) RETURNS TABLE(subject_id TEXT, next_cursor TEXT) ROWS 100 AS $$
DECLARE
    v_is_userset_filter BOOLEAN;
    v_filter_type TEXT;
    v_filter_relation TEXT;
BEGIN
    v_is_userset_filter := position('#' in p_subject_type) > 0;
    IF v_is_userset_filter THEN
        v_filter_type := split_part(p_subject_type, '#', 1);
        v_filter_relation := split_part(p_subject_type, '#', 2);
        -- Self-candidate: when filter type matches object type
        IF v_filter_type = 'state_volume' THEN
        IF EXISTS (
    		SELECT p_object_id || '#' || v_filter_relation AS subject_id
    		WHERE (v_filter_type = 'state_volume' AND EXISTS (
    SELECT 1
    FROM (VALUES ('agent', 'can_execute', 'can_execute'), ('agent', 'can_manage', 'can_manage'), ('agent', 'can_read', 'can_read'), ('agent', 'project', 'project'), ('project', 'can_delete', 'can_delete'), ('project', 'can_manage', 'can_manage'), ('project', 'can_manage', 'maintainer'), ('project', 'can_read', 'can_read'), ('project', 'can_read', 'maintainer'), ('project', 'can_write', 'can_write'), ('project', 'can_write', 'maintainer'), ('project', 'maintainer', 'maintainer'), ('project', 'organization', 'organization'), ('state_volume', 'agent', 'agent'), ('state_volume', 'can_attach', 'can_attach'), ('state_volume', 'can_manage', 'can_manage'), ('state_volume', 'can_read', 'can_read'), ('state_volume', 'can_restore', 'can_restore')) AS c(object_type, relation, satisfying_relation)
    WHERE (c.object_type = 'state_volume' AND c.relation = 'can_restore' AND c.satisfying_relation = v_filter_relation)
    ))
    ) THEN
        RETURN QUERY
        WITH base_results AS (
            SELECT p_object_id || '#' || v_filter_relation AS subject_id
            		WHERE (v_filter_type = 'state_volume' AND EXISTS (
            SELECT 1
            FROM (VALUES ('agent', 'can_execute', 'can_execute'), ('agent', 'can_manage', 'can_manage'), ('agent', 'can_read', 'can_read'), ('agent', 'project', 'project'), ('project', 'can_delete', 'can_delete'), ('project', 'can_manage', 'can_manage'), ('project', 'can_manage', 'maintainer'), ('project', 'can_read', 'can_read'), ('project', 'can_read', 'maintainer'), ('project', 'can_write', 'can_write'), ('project', 'can_write', 'maintainer'), ('project', 'maintainer', 'maintainer'), ('project', 'organization', 'organization'), ('state_volume', 'agent', 'agent'), ('state_volume', 'can_attach', 'can_attach'), ('state_volume', 'can_manage', 'can_manage'), ('state_volume', 'can_read', 'can_read'), ('state_volume', 'can_restore', 'can_restore')) AS c(object_type, relation, satisfying_relation)
            WHERE (c.object_type = 'state_volume' AND c.relation = 'can_restore' AND c.satisfying_relation = v_filter_relation)
            ))
        ),
        paged AS (
            SELECT br.subject_id
            FROM base_results br
            WHERE p_after IS NULL OR (
                -- Compound comparison for wildcard-first ordering:
                -- (is_not_wildcard, subject_id) > (cursor_is_not_wildcard, cursor)
                (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END, br.subject_id) >
                (CASE WHEN p_after = '*' THEN 0 ELSE 1 END, p_after)
            )
            ORDER BY (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END), br.subject_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.subject_id FROM paged p
            ORDER BY (CASE WHEN p.subject_id = '*' THEN 0 ELSE 1 END), p.subject_id
            LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT r.subject_id FROM returned r
                      ORDER BY (CASE WHEN r.subject_id = '*' THEN 0 ELSE 1 END) DESC, r.subject_id DESC
                      LIMIT 1)
            END AS next_cursor
        )
        SELECT r.subject_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
        RETURN;
    END IF;
    END IF;
        -- Userset filter case
        RETURN QUERY
        WITH base_results AS (
            WITH subject_candidates AS (
                -- From agent parents
                    SELECT DISTINCT s.subject_id
                    FROM melange_tuples AS link
                    CROSS JOIN LATERAL "public"."list_agent_can_manage_sub"(link.subject_id, p_subject_type) AS s
                    WHERE (link.object_type = 'state_volume' AND link.object_id = p_object_id AND link.relation = 'agent' AND link.subject_type = 'agent')
            )
            SELECT DISTINCT sc.subject_id
            FROM subject_candidates AS sc
            WHERE "public"."check_permission_internal"(v_filter_type, sc.subject_id, 'can_restore', 'state_volume', p_object_id, ARRAY[]::TEXT[]) = 1
        ),
        paged AS (
            SELECT br.subject_id
            FROM base_results br
            WHERE p_after IS NULL OR (
                -- Compound comparison for wildcard-first ordering:
                -- (is_not_wildcard, subject_id) > (cursor_is_not_wildcard, cursor)
                (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END, br.subject_id) >
                (CASE WHEN p_after = '*' THEN 0 ELSE 1 END, p_after)
            )
            ORDER BY (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END), br.subject_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.subject_id FROM paged p
            ORDER BY (CASE WHEN p.subject_id = '*' THEN 0 ELSE 1 END), p.subject_id
            LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT r.subject_id FROM returned r
                      ORDER BY (CASE WHEN r.subject_id = '*' THEN 0 ELSE 1 END) DESC, r.subject_id DESC
                      LIMIT 1)
            END AS next_cursor
        )
        SELECT r.subject_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
    ELSE
        -- Direct subject type case
        IF p_subject_type NOT IN ('user') THEN
        RETURN;
    END IF;
        RETURN QUERY
        WITH base_results AS (
            WITH subject_candidates AS (
                -- From agent parents
                    SELECT DISTINCT s.subject_id
                    FROM melange_tuples AS link
                    CROSS JOIN LATERAL "public"."list_agent_can_manage_sub"(link.subject_id, p_subject_type) AS s
                    WHERE (link.object_type = 'state_volume' AND link.object_id = p_object_id AND link.relation = 'agent' AND link.subject_type = 'agent')
            )
            SELECT DISTINCT sc.subject_id
            FROM subject_candidates AS sc
        ),
        paged AS (
            SELECT br.subject_id
            FROM base_results br
            WHERE p_after IS NULL OR (
                -- Compound comparison for wildcard-first ordering:
                -- (is_not_wildcard, subject_id) > (cursor_is_not_wildcard, cursor)
                (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END, br.subject_id) >
                (CASE WHEN p_after = '*' THEN 0 ELSE 1 END, p_after)
            )
            ORDER BY (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END), br.subject_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.subject_id FROM paged p
            ORDER BY (CASE WHEN p.subject_id = '*' THEN 0 ELSE 1 END), p.subject_id
            LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT r.subject_id FROM returned r
                      ORDER BY (CASE WHEN r.subject_id = '*' THEN 0 ELSE 1 END) DESC, r.subject_id DESC
                      LIMIT 1)
            END AS next_cursor
        )
        SELECT r.subject_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
    END IF;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';

-- Generated list_subjects function for run.can_cancel
-- Features: Recursive
-- Indirect anchor: project.can_write via ttu
CREATE OR REPLACE FUNCTION "public"."list_run_can_cancel_sub"(
    p_object_id TEXT,
    p_subject_type TEXT,
    p_limit INT DEFAULT NULL,
    p_after TEXT DEFAULT NULL
) RETURNS TABLE(subject_id TEXT, next_cursor TEXT) ROWS 100 AS $$
DECLARE
    v_is_userset_filter BOOLEAN;
    v_filter_type TEXT;
    v_filter_relation TEXT;
BEGIN
    v_is_userset_filter := position('#' in p_subject_type) > 0;
    IF v_is_userset_filter THEN
        v_filter_type := split_part(p_subject_type, '#', 1);
        v_filter_relation := split_part(p_subject_type, '#', 2);
        -- Self-candidate: when filter type matches object type
        IF v_filter_type = 'run' THEN
        IF EXISTS (
    		SELECT p_object_id || '#' || v_filter_relation AS subject_id
    		WHERE (v_filter_type = 'run' AND EXISTS (
    SELECT 1
    FROM (VALUES ('agent', 'can_execute', 'can_execute'), ('agent', 'can_manage', 'can_manage'), ('agent', 'can_read', 'can_read'), ('agent', 'project', 'project'), ('project', 'can_delete', 'can_delete'), ('project', 'can_manage', 'can_manage'), ('project', 'can_manage', 'maintainer'), ('project', 'can_read', 'can_read'), ('project', 'can_read', 'maintainer'), ('project', 'can_write', 'can_write'), ('project', 'can_write', 'maintainer'), ('project', 'maintainer', 'maintainer'), ('project', 'organization', 'organization'), ('run', 'agent', 'agent'), ('run', 'can_cancel', 'can_cancel'), ('run', 'can_read', 'can_read')) AS c(object_type, relation, satisfying_relation)
    WHERE (c.object_type = 'run' AND c.relation = 'can_cancel' AND c.satisfying_relation = v_filter_relation)
    ))
    ) THEN
        RETURN QUERY
        WITH base_results AS (
            SELECT p_object_id || '#' || v_filter_relation AS subject_id
            		WHERE (v_filter_type = 'run' AND EXISTS (
            SELECT 1
            FROM (VALUES ('agent', 'can_execute', 'can_execute'), ('agent', 'can_manage', 'can_manage'), ('agent', 'can_read', 'can_read'), ('agent', 'project', 'project'), ('project', 'can_delete', 'can_delete'), ('project', 'can_manage', 'can_manage'), ('project', 'can_manage', 'maintainer'), ('project', 'can_read', 'can_read'), ('project', 'can_read', 'maintainer'), ('project', 'can_write', 'can_write'), ('project', 'can_write', 'maintainer'), ('project', 'maintainer', 'maintainer'), ('project', 'organization', 'organization'), ('run', 'agent', 'agent'), ('run', 'can_cancel', 'can_cancel'), ('run', 'can_read', 'can_read')) AS c(object_type, relation, satisfying_relation)
            WHERE (c.object_type = 'run' AND c.relation = 'can_cancel' AND c.satisfying_relation = v_filter_relation)
            ))
        ),
        paged AS (
            SELECT br.subject_id
            FROM base_results br
            WHERE p_after IS NULL OR (
                -- Compound comparison for wildcard-first ordering:
                -- (is_not_wildcard, subject_id) > (cursor_is_not_wildcard, cursor)
                (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END, br.subject_id) >
                (CASE WHEN p_after = '*' THEN 0 ELSE 1 END, p_after)
            )
            ORDER BY (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END), br.subject_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.subject_id FROM paged p
            ORDER BY (CASE WHEN p.subject_id = '*' THEN 0 ELSE 1 END), p.subject_id
            LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT r.subject_id FROM returned r
                      ORDER BY (CASE WHEN r.subject_id = '*' THEN 0 ELSE 1 END) DESC, r.subject_id DESC
                      LIMIT 1)
            END AS next_cursor
        )
        SELECT r.subject_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
        RETURN;
    END IF;
    END IF;
        -- Userset filter case
        RETURN QUERY
        WITH base_results AS (
            WITH subject_candidates AS (
                -- From agent parents
                    SELECT DISTINCT s.subject_id
                    FROM melange_tuples AS link
                    CROSS JOIN LATERAL "public"."list_agent_can_execute_sub"(link.subject_id, p_subject_type) AS s
                    WHERE (link.object_type = 'run' AND link.object_id = p_object_id AND link.relation = 'agent' AND link.subject_type = 'agent')
            )
            SELECT DISTINCT sc.subject_id
            FROM subject_candidates AS sc
            WHERE "public"."check_permission_internal"(v_filter_type, sc.subject_id, 'can_cancel', 'run', p_object_id, ARRAY[]::TEXT[]) = 1
        ),
        paged AS (
            SELECT br.subject_id
            FROM base_results br
            WHERE p_after IS NULL OR (
                -- Compound comparison for wildcard-first ordering:
                -- (is_not_wildcard, subject_id) > (cursor_is_not_wildcard, cursor)
                (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END, br.subject_id) >
                (CASE WHEN p_after = '*' THEN 0 ELSE 1 END, p_after)
            )
            ORDER BY (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END), br.subject_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.subject_id FROM paged p
            ORDER BY (CASE WHEN p.subject_id = '*' THEN 0 ELSE 1 END), p.subject_id
            LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT r.subject_id FROM returned r
                      ORDER BY (CASE WHEN r.subject_id = '*' THEN 0 ELSE 1 END) DESC, r.subject_id DESC
                      LIMIT 1)
            END AS next_cursor
        )
        SELECT r.subject_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
    ELSE
        -- Direct subject type case
        IF p_subject_type NOT IN ('user') THEN
        RETURN;
    END IF;
        RETURN QUERY
        WITH base_results AS (
            WITH subject_candidates AS (
                -- From agent parents
                    SELECT DISTINCT s.subject_id
                    FROM melange_tuples AS link
                    CROSS JOIN LATERAL "public"."list_agent_can_execute_sub"(link.subject_id, p_subject_type) AS s
                    WHERE (link.object_type = 'run' AND link.object_id = p_object_id AND link.relation = 'agent' AND link.subject_type = 'agent')
            )
            SELECT DISTINCT sc.subject_id
            FROM subject_candidates AS sc
        ),
        paged AS (
            SELECT br.subject_id
            FROM base_results br
            WHERE p_after IS NULL OR (
                -- Compound comparison for wildcard-first ordering:
                -- (is_not_wildcard, subject_id) > (cursor_is_not_wildcard, cursor)
                (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END, br.subject_id) >
                (CASE WHEN p_after = '*' THEN 0 ELSE 1 END, p_after)
            )
            ORDER BY (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END), br.subject_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.subject_id FROM paged p
            ORDER BY (CASE WHEN p.subject_id = '*' THEN 0 ELSE 1 END), p.subject_id
            LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT r.subject_id FROM returned r
                      ORDER BY (CASE WHEN r.subject_id = '*' THEN 0 ELSE 1 END) DESC, r.subject_id DESC
                      LIMIT 1)
            END AS next_cursor
        )
        SELECT r.subject_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
    END IF;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';

-- Generated list_subjects function for state_volume.can_attach
-- Features: Recursive
-- Indirect anchor: project.can_write via ttu
CREATE OR REPLACE FUNCTION "public"."list_state_volume_can_attach_sub"(
    p_object_id TEXT,
    p_subject_type TEXT,
    p_limit INT DEFAULT NULL,
    p_after TEXT DEFAULT NULL
) RETURNS TABLE(subject_id TEXT, next_cursor TEXT) ROWS 100 AS $$
DECLARE
    v_is_userset_filter BOOLEAN;
    v_filter_type TEXT;
    v_filter_relation TEXT;
BEGIN
    v_is_userset_filter := position('#' in p_subject_type) > 0;
    IF v_is_userset_filter THEN
        v_filter_type := split_part(p_subject_type, '#', 1);
        v_filter_relation := split_part(p_subject_type, '#', 2);
        -- Self-candidate: when filter type matches object type
        IF v_filter_type = 'state_volume' THEN
        IF EXISTS (
    		SELECT p_object_id || '#' || v_filter_relation AS subject_id
    		WHERE (v_filter_type = 'state_volume' AND EXISTS (
    SELECT 1
    FROM (VALUES ('agent', 'can_execute', 'can_execute'), ('agent', 'can_manage', 'can_manage'), ('agent', 'can_read', 'can_read'), ('agent', 'project', 'project'), ('project', 'can_delete', 'can_delete'), ('project', 'can_manage', 'can_manage'), ('project', 'can_manage', 'maintainer'), ('project', 'can_read', 'can_read'), ('project', 'can_read', 'maintainer'), ('project', 'can_write', 'can_write'), ('project', 'can_write', 'maintainer'), ('project', 'maintainer', 'maintainer'), ('project', 'organization', 'organization'), ('state_volume', 'agent', 'agent'), ('state_volume', 'can_attach', 'can_attach'), ('state_volume', 'can_manage', 'can_manage'), ('state_volume', 'can_read', 'can_read'), ('state_volume', 'can_restore', 'can_restore')) AS c(object_type, relation, satisfying_relation)
    WHERE (c.object_type = 'state_volume' AND c.relation = 'can_attach' AND c.satisfying_relation = v_filter_relation)
    ))
    ) THEN
        RETURN QUERY
        WITH base_results AS (
            SELECT p_object_id || '#' || v_filter_relation AS subject_id
            		WHERE (v_filter_type = 'state_volume' AND EXISTS (
            SELECT 1
            FROM (VALUES ('agent', 'can_execute', 'can_execute'), ('agent', 'can_manage', 'can_manage'), ('agent', 'can_read', 'can_read'), ('agent', 'project', 'project'), ('project', 'can_delete', 'can_delete'), ('project', 'can_manage', 'can_manage'), ('project', 'can_manage', 'maintainer'), ('project', 'can_read', 'can_read'), ('project', 'can_read', 'maintainer'), ('project', 'can_write', 'can_write'), ('project', 'can_write', 'maintainer'), ('project', 'maintainer', 'maintainer'), ('project', 'organization', 'organization'), ('state_volume', 'agent', 'agent'), ('state_volume', 'can_attach', 'can_attach'), ('state_volume', 'can_manage', 'can_manage'), ('state_volume', 'can_read', 'can_read'), ('state_volume', 'can_restore', 'can_restore')) AS c(object_type, relation, satisfying_relation)
            WHERE (c.object_type = 'state_volume' AND c.relation = 'can_attach' AND c.satisfying_relation = v_filter_relation)
            ))
        ),
        paged AS (
            SELECT br.subject_id
            FROM base_results br
            WHERE p_after IS NULL OR (
                -- Compound comparison for wildcard-first ordering:
                -- (is_not_wildcard, subject_id) > (cursor_is_not_wildcard, cursor)
                (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END, br.subject_id) >
                (CASE WHEN p_after = '*' THEN 0 ELSE 1 END, p_after)
            )
            ORDER BY (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END), br.subject_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.subject_id FROM paged p
            ORDER BY (CASE WHEN p.subject_id = '*' THEN 0 ELSE 1 END), p.subject_id
            LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT r.subject_id FROM returned r
                      ORDER BY (CASE WHEN r.subject_id = '*' THEN 0 ELSE 1 END) DESC, r.subject_id DESC
                      LIMIT 1)
            END AS next_cursor
        )
        SELECT r.subject_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
        RETURN;
    END IF;
    END IF;
        -- Userset filter case
        RETURN QUERY
        WITH base_results AS (
            WITH subject_candidates AS (
                -- From agent parents
                    SELECT DISTINCT s.subject_id
                    FROM melange_tuples AS link
                    CROSS JOIN LATERAL "public"."list_agent_can_execute_sub"(link.subject_id, p_subject_type) AS s
                    WHERE (link.object_type = 'state_volume' AND link.object_id = p_object_id AND link.relation = 'agent' AND link.subject_type = 'agent')
            )
            SELECT DISTINCT sc.subject_id
            FROM subject_candidates AS sc
            WHERE "public"."check_permission_internal"(v_filter_type, sc.subject_id, 'can_attach', 'state_volume', p_object_id, ARRAY[]::TEXT[]) = 1
        ),
        paged AS (
            SELECT br.subject_id
            FROM base_results br
            WHERE p_after IS NULL OR (
                -- Compound comparison for wildcard-first ordering:
                -- (is_not_wildcard, subject_id) > (cursor_is_not_wildcard, cursor)
                (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END, br.subject_id) >
                (CASE WHEN p_after = '*' THEN 0 ELSE 1 END, p_after)
            )
            ORDER BY (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END), br.subject_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.subject_id FROM paged p
            ORDER BY (CASE WHEN p.subject_id = '*' THEN 0 ELSE 1 END), p.subject_id
            LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT r.subject_id FROM returned r
                      ORDER BY (CASE WHEN r.subject_id = '*' THEN 0 ELSE 1 END) DESC, r.subject_id DESC
                      LIMIT 1)
            END AS next_cursor
        )
        SELECT r.subject_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
    ELSE
        -- Direct subject type case
        IF p_subject_type NOT IN ('user') THEN
        RETURN;
    END IF;
        RETURN QUERY
        WITH base_results AS (
            WITH subject_candidates AS (
                -- From agent parents
                    SELECT DISTINCT s.subject_id
                    FROM melange_tuples AS link
                    CROSS JOIN LATERAL "public"."list_agent_can_execute_sub"(link.subject_id, p_subject_type) AS s
                    WHERE (link.object_type = 'state_volume' AND link.object_id = p_object_id AND link.relation = 'agent' AND link.subject_type = 'agent')
            )
            SELECT DISTINCT sc.subject_id
            FROM subject_candidates AS sc
        ),
        paged AS (
            SELECT br.subject_id
            FROM base_results br
            WHERE p_after IS NULL OR (
                -- Compound comparison for wildcard-first ordering:
                -- (is_not_wildcard, subject_id) > (cursor_is_not_wildcard, cursor)
                (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END, br.subject_id) >
                (CASE WHEN p_after = '*' THEN 0 ELSE 1 END, p_after)
            )
            ORDER BY (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END), br.subject_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.subject_id FROM paged p
            ORDER BY (CASE WHEN p.subject_id = '*' THEN 0 ELSE 1 END), p.subject_id
            LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT r.subject_id FROM returned r
                      ORDER BY (CASE WHEN r.subject_id = '*' THEN 0 ELSE 1 END) DESC, r.subject_id DESC
                      LIMIT 1)
            END AS next_cursor
        )
        SELECT r.subject_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
    END IF;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';

-- Generated list_subjects function for project.can_read
-- Features: Implied+Recursive
CREATE OR REPLACE FUNCTION "public"."list_project_can_read_sub"(
    p_object_id TEXT,
    p_subject_type TEXT,
    p_limit INT DEFAULT NULL,
    p_after TEXT DEFAULT NULL
) RETURNS TABLE(subject_id TEXT, next_cursor TEXT) ROWS 100 AS $$
DECLARE
    v_filter_type TEXT;
    v_filter_relation TEXT;
BEGIN
    -- Check if p_subject_type is a userset filter (contains '#')
    IF position('#' in p_subject_type) > 0 THEN
        v_filter_type := substring(p_subject_type from 1 for position('#' in p_subject_type) - 1);
        v_filter_relation := substring(p_subject_type from position('#' in p_subject_type) + 1);
        RETURN QUERY
        WITH base_results AS (
            -- Direct userset tuples
                SELECT DISTINCT split_part(t.subject_id, '#', 1) || '#' || v_filter_relation AS subject_id
                		FROM melange_tuples AS t
                		WHERE (t.object_type = 'project' AND t.object_id = p_object_id AND t.relation IN ('can_read', 'maintainer') AND position('#' in t.subject_id) > 0 AND t.subject_type = v_filter_type AND EXISTS (
                SELECT 1
                FROM (VALUES ('organization', 'admin', 'admin'), ('organization', 'admin', 'owner'), ('organization', 'can_create_project', 'admin'), ('organization', 'can_create_project', 'can_create_project'), ('organization', 'can_create_project', 'owner'), ('organization', 'can_delete', 'can_delete'), ('organization', 'can_delete', 'owner'), ('organization', 'can_manage', 'can_manage'), ('organization', 'can_manage', 'owner'), ('organization', 'can_manage_members', 'admin'), ('organization', 'can_manage_members', 'can_manage_members'), ('organization', 'can_manage_members', 'owner'), ('organization', 'can_read', 'admin'), ('organization', 'can_read', 'can_read'), ('organization', 'can_read', 'member'), ('organization', 'can_read', 'owner'), ('organization', 'member', 'admin'), ('organization', 'member', 'member'), ('organization', 'member', 'owner'), ('organization', 'owner', 'owner'), ('project', 'can_delete', 'can_delete'), ('project', 'can_manage', 'can_manage'), ('project', 'can_manage', 'maintainer'), ('project', 'can_read', 'can_read'), ('project', 'can_read', 'maintainer'), ('project', 'can_write', 'can_write'), ('project', 'can_write', 'maintainer'), ('project', 'maintainer', 'maintainer'), ('project', 'organization', 'organization')) AS c(object_type, relation, satisfying_relation)
                WHERE (c.object_type = v_filter_type AND c.relation = substring(t.subject_id from position('#' in t.subject_id) + 1) AND c.satisfying_relation = v_filter_relation)
                ) AND "public"."check_permission_internal"(v_filter_type, t.subject_id, 'can_read', 'project', p_object_id, ARRAY[]::TEXT[]) = 1)
                UNION
                -- TTU userset: organization -> can_read
                SELECT DISTINCT substring(pt.subject_id from 1 for position('#' in pt.subject_id) - 1) || '#' || v_filter_relation AS subject_id
                		FROM melange_tuples AS link
                		INNER JOIN melange_tuples AS pt ON (pt.object_type = link.subject_type AND pt.object_id = link.subject_id AND pt.relation IN (SELECT c.satisfying_relation
                FROM (VALUES ('organization', 'admin', 'admin'), ('organization', 'admin', 'owner'), ('organization', 'can_create_project', 'admin'), ('organization', 'can_create_project', 'can_create_project'), ('organization', 'can_create_project', 'owner'), ('organization', 'can_delete', 'can_delete'), ('organization', 'can_delete', 'owner'), ('organization', 'can_manage', 'can_manage'), ('organization', 'can_manage', 'owner'), ('organization', 'can_manage_members', 'admin'), ('organization', 'can_manage_members', 'can_manage_members'), ('organization', 'can_manage_members', 'owner'), ('organization', 'can_read', 'admin'), ('organization', 'can_read', 'can_read'), ('organization', 'can_read', 'member'), ('organization', 'can_read', 'owner'), ('organization', 'member', 'admin'), ('organization', 'member', 'member'), ('organization', 'member', 'owner'), ('organization', 'owner', 'owner'), ('project', 'can_delete', 'can_delete'), ('project', 'can_manage', 'can_manage'), ('project', 'can_manage', 'maintainer'), ('project', 'can_read', 'can_read'), ('project', 'can_read', 'maintainer'), ('project', 'can_write', 'can_write'), ('project', 'can_write', 'maintainer'), ('project', 'maintainer', 'maintainer'), ('project', 'organization', 'organization')) AS c(object_type, relation, satisfying_relation)
                WHERE (c.object_type = link.subject_type AND c.relation = 'can_read')))
                		WHERE (link.object_type = 'project' AND link.object_id = p_object_id AND link.relation = 'organization' AND pt.subject_type = v_filter_type AND position('#' in pt.subject_id) > 0 AND (substring(pt.subject_id from position('#' in pt.subject_id) + 1) = v_filter_relation OR EXISTS (
                SELECT 1
                FROM (VALUES ('organization', 'admin', 'admin'), ('organization', 'admin', 'owner'), ('organization', 'can_create_project', 'admin'), ('organization', 'can_create_project', 'can_create_project'), ('organization', 'can_create_project', 'owner'), ('organization', 'can_delete', 'can_delete'), ('organization', 'can_delete', 'owner'), ('organization', 'can_manage', 'can_manage'), ('organization', 'can_manage', 'owner'), ('organization', 'can_manage_members', 'admin'), ('organization', 'can_manage_members', 'can_manage_members'), ('organization', 'can_manage_members', 'owner'), ('organization', 'can_read', 'admin'), ('organization', 'can_read', 'can_read'), ('organization', 'can_read', 'member'), ('organization', 'can_read', 'owner'), ('organization', 'member', 'admin'), ('organization', 'member', 'member'), ('organization', 'member', 'owner'), ('organization', 'owner', 'owner'), ('project', 'can_delete', 'can_delete'), ('project', 'can_manage', 'can_manage'), ('project', 'can_manage', 'maintainer'), ('project', 'can_read', 'can_read'), ('project', 'can_read', 'maintainer'), ('project', 'can_write', 'can_write'), ('project', 'can_write', 'maintainer'), ('project', 'maintainer', 'maintainer'), ('project', 'organization', 'organization')) AS subj_c(object_type, relation, satisfying_relation)
                WHERE (subj_c.object_type = v_filter_type AND subj_c.relation = substring(pt.subject_id from position('#' in pt.subject_id) + 1) AND subj_c.satisfying_relation = v_filter_relation)
                )) AND link.subject_type IN ('organization'))
                UNION
                -- TTU intermediate: parent object as userset reference
                SELECT DISTINCT link.subject_id || '#' || v_filter_relation AS subject_id
                		FROM melange_tuples AS link
                		WHERE (link.object_type = 'project' AND link.object_id = p_object_id AND link.relation = 'organization' AND link.subject_type = v_filter_type AND EXISTS (
                SELECT 1
                FROM (VALUES ('organization', 'admin', 'admin'), ('organization', 'admin', 'owner'), ('organization', 'can_create_project', 'admin'), ('organization', 'can_create_project', 'can_create_project'), ('organization', 'can_create_project', 'owner'), ('organization', 'can_delete', 'can_delete'), ('organization', 'can_delete', 'owner'), ('organization', 'can_manage', 'can_manage'), ('organization', 'can_manage', 'owner'), ('organization', 'can_manage_members', 'admin'), ('organization', 'can_manage_members', 'can_manage_members'), ('organization', 'can_manage_members', 'owner'), ('organization', 'can_read', 'admin'), ('organization', 'can_read', 'can_read'), ('organization', 'can_read', 'member'), ('organization', 'can_read', 'owner'), ('organization', 'member', 'admin'), ('organization', 'member', 'member'), ('organization', 'member', 'owner'), ('organization', 'owner', 'owner'), ('project', 'can_delete', 'can_delete'), ('project', 'can_manage', 'can_manage'), ('project', 'can_manage', 'maintainer'), ('project', 'can_read', 'can_read'), ('project', 'can_read', 'maintainer'), ('project', 'can_write', 'can_write'), ('project', 'can_write', 'maintainer'), ('project', 'maintainer', 'maintainer'), ('project', 'organization', 'organization')) AS c(object_type, relation, satisfying_relation)
                WHERE (c.object_type = link.subject_type AND c.relation = 'can_read' AND c.satisfying_relation = v_filter_relation)
                ) AND link.subject_type IN ('organization'))
                UNION
                -- TTU nested: multi-hop chain resolution
                SELECT nested.subject_id
                FROM melange_tuples AS link
                CROSS JOIN LATERAL "public"."list_accessible_subjects"(link.subject_type, link.subject_id, 'can_read', p_subject_type) AS nested
                WHERE (link.object_type = 'project' AND link.object_id = p_object_id AND link.relation = 'organization' AND link.subject_type IN ('organization'))
                UNION
                -- Self-candidate: when filter type matches object type
                -- e.g., querying document:1.viewer with filter document#writer
                -- should return document:1#writer if writer satisfies the relation
                SELECT p_object_id || '#' || v_filter_relation AS subject_id
                		WHERE (v_filter_type = 'project' AND EXISTS (
                SELECT 1
                FROM (VALUES ('organization', 'admin', 'admin'), ('organization', 'admin', 'owner'), ('organization', 'can_create_project', 'admin'), ('organization', 'can_create_project', 'can_create_project'), ('organization', 'can_create_project', 'owner'), ('organization', 'can_delete', 'can_delete'), ('organization', 'can_delete', 'owner'), ('organization', 'can_manage', 'can_manage'), ('organization', 'can_manage', 'owner'), ('organization', 'can_manage_members', 'admin'), ('organization', 'can_manage_members', 'can_manage_members'), ('organization', 'can_manage_members', 'owner'), ('organization', 'can_read', 'admin'), ('organization', 'can_read', 'can_read'), ('organization', 'can_read', 'member'), ('organization', 'can_read', 'owner'), ('organization', 'member', 'admin'), ('organization', 'member', 'member'), ('organization', 'member', 'owner'), ('organization', 'owner', 'owner'), ('project', 'can_delete', 'can_delete'), ('project', 'can_manage', 'can_manage'), ('project', 'can_manage', 'maintainer'), ('project', 'can_read', 'can_read'), ('project', 'can_read', 'maintainer'), ('project', 'can_write', 'can_write'), ('project', 'can_write', 'maintainer'), ('project', 'maintainer', 'maintainer'), ('project', 'organization', 'organization')) AS c(object_type, relation, satisfying_relation)
                WHERE (c.object_type = 'project' AND c.relation = 'can_read' AND c.satisfying_relation = v_filter_relation)
                ))
        ),
        paged AS (
            SELECT br.subject_id
            FROM base_results br
            WHERE p_after IS NULL OR (
                -- Compound comparison for wildcard-first ordering:
                -- (is_not_wildcard, subject_id) > (cursor_is_not_wildcard, cursor)
                (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END, br.subject_id) >
                (CASE WHEN p_after = '*' THEN 0 ELSE 1 END, p_after)
            )
            ORDER BY (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END), br.subject_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.subject_id FROM paged p
            ORDER BY (CASE WHEN p.subject_id = '*' THEN 0 ELSE 1 END), p.subject_id
            LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT r.subject_id FROM returned r
                      ORDER BY (CASE WHEN r.subject_id = '*' THEN 0 ELSE 1 END) DESC, r.subject_id DESC
                      LIMIT 1)
            END AS next_cursor
        )
        SELECT r.subject_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
    ELSE
        -- Regular subject type: find direct subjects and expand usersets
        RETURN QUERY
        WITH base_results AS (
            WITH RECURSIVE parent_closure AS (
                SELECT link.subject_type, link.subject_id, 0 AS depth
                FROM melange_tuples AS link
                WHERE (link.object_type = 'project' AND link.object_id = p_object_id AND link.relation IN ('organization') AND link.subject_type IN ('organization'))
                        UNION
                        SELECT link.subject_type, link.subject_id, p.depth + 1 AS depth
                FROM parent_closure AS p
                INNER JOIN melange_tuples AS link ON (link.object_type = p.subject_type AND link.object_id = p.subject_id)
                WHERE (p.subject_type = 'project' AND link.relation IN ('organization') AND p.depth < 25)
            ),
            base_results AS (
                -- Direct tuple lookup with simple closure relations
                    SELECT DISTINCT t.subject_id
                    FROM melange_tuples AS t
                    WHERE (t.object_type = 'project' AND t.relation IN ('can_read', 'maintainer') AND t.object_id = p_object_id AND t.subject_type = p_subject_type AND position('#' in t.subject_id) = 0 AND t.subject_id <> '*')
                    UNION
                    -- TTU: subjects via organization -> can_read (parent closure optimization)
                    SELECT DISTINCT t.subject_id
                    FROM parent_closure AS p
                    INNER JOIN melange_tuples AS t ON (t.object_type = p.subject_type AND t.object_id = p.subject_id)
                    WHERE (t.subject_type = p_subject_type AND t.relation IN ('admin', 'can_read', 'member', 'owner') AND position('#' in t.subject_id) = 0 AND t.subject_id <> '*')
            ),
            has_wildcard AS (
                SELECT EXISTS (SELECT 1 FROM base_results br WHERE br.subject_id = '*') AS has_wildcard
            )
            SELECT br.subject_id
            FROM base_results AS br
        ),
        paged AS (
            SELECT br.subject_id
            FROM base_results br
            WHERE p_after IS NULL OR (
                -- Compound comparison for wildcard-first ordering:
                -- (is_not_wildcard, subject_id) > (cursor_is_not_wildcard, cursor)
                (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END, br.subject_id) >
                (CASE WHEN p_after = '*' THEN 0 ELSE 1 END, p_after)
            )
            ORDER BY (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END), br.subject_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.subject_id FROM paged p
            ORDER BY (CASE WHEN p.subject_id = '*' THEN 0 ELSE 1 END), p.subject_id
            LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT r.subject_id FROM returned r
                      ORDER BY (CASE WHEN r.subject_id = '*' THEN 0 ELSE 1 END) DESC, r.subject_id DESC
                      LIMIT 1)
            END AS next_cursor
        )
        SELECT r.subject_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
    END IF;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';

-- Generated list_subjects function for agent.can_read
-- Features: Recursive
-- Indirect anchor: project.can_read via ttu
CREATE OR REPLACE FUNCTION "public"."list_agent_can_read_sub"(
    p_object_id TEXT,
    p_subject_type TEXT,
    p_limit INT DEFAULT NULL,
    p_after TEXT DEFAULT NULL
) RETURNS TABLE(subject_id TEXT, next_cursor TEXT) ROWS 100 AS $$
DECLARE
    v_is_userset_filter BOOLEAN;
    v_filter_type TEXT;
    v_filter_relation TEXT;
BEGIN
    v_is_userset_filter := position('#' in p_subject_type) > 0;
    IF v_is_userset_filter THEN
        v_filter_type := split_part(p_subject_type, '#', 1);
        v_filter_relation := split_part(p_subject_type, '#', 2);
        -- Self-candidate: when filter type matches object type
        IF v_filter_type = 'agent' THEN
        IF EXISTS (
    		SELECT p_object_id || '#' || v_filter_relation AS subject_id
    		WHERE (v_filter_type = 'agent' AND EXISTS (
    SELECT 1
    FROM (VALUES ('agent', 'can_execute', 'can_execute'), ('agent', 'can_manage', 'can_manage'), ('agent', 'can_read', 'can_read'), ('agent', 'project', 'project'), ('project', 'can_delete', 'can_delete'), ('project', 'can_manage', 'can_manage'), ('project', 'can_manage', 'maintainer'), ('project', 'can_read', 'can_read'), ('project', 'can_read', 'maintainer'), ('project', 'can_write', 'can_write'), ('project', 'can_write', 'maintainer'), ('project', 'maintainer', 'maintainer'), ('project', 'organization', 'organization')) AS c(object_type, relation, satisfying_relation)
    WHERE (c.object_type = 'agent' AND c.relation = 'can_read' AND c.satisfying_relation = v_filter_relation)
    ))
    ) THEN
        RETURN QUERY
        WITH base_results AS (
            SELECT p_object_id || '#' || v_filter_relation AS subject_id
            		WHERE (v_filter_type = 'agent' AND EXISTS (
            SELECT 1
            FROM (VALUES ('agent', 'can_execute', 'can_execute'), ('agent', 'can_manage', 'can_manage'), ('agent', 'can_read', 'can_read'), ('agent', 'project', 'project'), ('project', 'can_delete', 'can_delete'), ('project', 'can_manage', 'can_manage'), ('project', 'can_manage', 'maintainer'), ('project', 'can_read', 'can_read'), ('project', 'can_read', 'maintainer'), ('project', 'can_write', 'can_write'), ('project', 'can_write', 'maintainer'), ('project', 'maintainer', 'maintainer'), ('project', 'organization', 'organization')) AS c(object_type, relation, satisfying_relation)
            WHERE (c.object_type = 'agent' AND c.relation = 'can_read' AND c.satisfying_relation = v_filter_relation)
            ))
        ),
        paged AS (
            SELECT br.subject_id
            FROM base_results br
            WHERE p_after IS NULL OR (
                -- Compound comparison for wildcard-first ordering:
                -- (is_not_wildcard, subject_id) > (cursor_is_not_wildcard, cursor)
                (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END, br.subject_id) >
                (CASE WHEN p_after = '*' THEN 0 ELSE 1 END, p_after)
            )
            ORDER BY (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END), br.subject_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.subject_id FROM paged p
            ORDER BY (CASE WHEN p.subject_id = '*' THEN 0 ELSE 1 END), p.subject_id
            LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT r.subject_id FROM returned r
                      ORDER BY (CASE WHEN r.subject_id = '*' THEN 0 ELSE 1 END) DESC, r.subject_id DESC
                      LIMIT 1)
            END AS next_cursor
        )
        SELECT r.subject_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
        RETURN;
    END IF;
    END IF;
        -- Userset filter case
        RETURN QUERY
        WITH base_results AS (
            WITH subject_candidates AS (
                -- From project parents
                    SELECT DISTINCT s.subject_id
                    FROM melange_tuples AS link
                    CROSS JOIN LATERAL "public"."list_project_can_read_sub"(link.subject_id, p_subject_type) AS s
                    WHERE (link.object_type = 'agent' AND link.object_id = p_object_id AND link.relation = 'project' AND link.subject_type = 'project')
            )
            SELECT DISTINCT sc.subject_id
            FROM subject_candidates AS sc
            WHERE "public"."check_permission_internal"(v_filter_type, sc.subject_id, 'can_read', 'agent', p_object_id, ARRAY[]::TEXT[]) = 1
        ),
        paged AS (
            SELECT br.subject_id
            FROM base_results br
            WHERE p_after IS NULL OR (
                -- Compound comparison for wildcard-first ordering:
                -- (is_not_wildcard, subject_id) > (cursor_is_not_wildcard, cursor)
                (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END, br.subject_id) >
                (CASE WHEN p_after = '*' THEN 0 ELSE 1 END, p_after)
            )
            ORDER BY (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END), br.subject_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.subject_id FROM paged p
            ORDER BY (CASE WHEN p.subject_id = '*' THEN 0 ELSE 1 END), p.subject_id
            LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT r.subject_id FROM returned r
                      ORDER BY (CASE WHEN r.subject_id = '*' THEN 0 ELSE 1 END) DESC, r.subject_id DESC
                      LIMIT 1)
            END AS next_cursor
        )
        SELECT r.subject_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
    ELSE
        -- Direct subject type case
        IF p_subject_type NOT IN ('user') THEN
        RETURN;
    END IF;
        RETURN QUERY
        WITH base_results AS (
            WITH subject_candidates AS (
                -- From project parents
                    SELECT DISTINCT s.subject_id
                    FROM melange_tuples AS link
                    CROSS JOIN LATERAL "public"."list_project_can_read_sub"(link.subject_id, p_subject_type) AS s
                    WHERE (link.object_type = 'agent' AND link.object_id = p_object_id AND link.relation = 'project' AND link.subject_type = 'project')
            )
            SELECT DISTINCT sc.subject_id
            FROM subject_candidates AS sc
        ),
        paged AS (
            SELECT br.subject_id
            FROM base_results br
            WHERE p_after IS NULL OR (
                -- Compound comparison for wildcard-first ordering:
                -- (is_not_wildcard, subject_id) > (cursor_is_not_wildcard, cursor)
                (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END, br.subject_id) >
                (CASE WHEN p_after = '*' THEN 0 ELSE 1 END, p_after)
            )
            ORDER BY (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END), br.subject_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.subject_id FROM paged p
            ORDER BY (CASE WHEN p.subject_id = '*' THEN 0 ELSE 1 END), p.subject_id
            LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT r.subject_id FROM returned r
                      ORDER BY (CASE WHEN r.subject_id = '*' THEN 0 ELSE 1 END) DESC, r.subject_id DESC
                      LIMIT 1)
            END AS next_cursor
        )
        SELECT r.subject_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
    END IF;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';

-- Generated list_subjects function for repository.can_read
-- Features: Implied+Wildcard+Recursive
CREATE OR REPLACE FUNCTION "public"."list_repository_can_read_sub"(
    p_object_id TEXT,
    p_subject_type TEXT,
    p_limit INT DEFAULT NULL,
    p_after TEXT DEFAULT NULL
) RETURNS TABLE(subject_id TEXT, next_cursor TEXT) ROWS 100 AS $$
DECLARE
    v_filter_type TEXT;
    v_filter_relation TEXT;
BEGIN
    -- Check if p_subject_type is a userset filter (contains '#')
    IF position('#' in p_subject_type) > 0 THEN
        v_filter_type := substring(p_subject_type from 1 for position('#' in p_subject_type) - 1);
        v_filter_relation := substring(p_subject_type from position('#' in p_subject_type) + 1);
        RETURN QUERY
        WITH base_results AS (
            -- Direct userset tuples
                SELECT DISTINCT split_part(t.subject_id, '#', 1) || '#' || v_filter_relation AS subject_id
                		FROM melange_tuples AS t
                		WHERE (t.object_type = 'repository' AND t.object_id = p_object_id AND t.relation IN ('can_read', 'public') AND position('#' in t.subject_id) > 0 AND t.subject_type = v_filter_type AND EXISTS (
                SELECT 1
                FROM (VALUES ('project', 'can_delete', 'can_delete'), ('project', 'can_manage', 'can_manage'), ('project', 'can_manage', 'maintainer'), ('project', 'can_read', 'can_read'), ('project', 'can_read', 'maintainer'), ('project', 'can_write', 'can_write'), ('project', 'can_write', 'maintainer'), ('project', 'maintainer', 'maintainer'), ('project', 'organization', 'organization'), ('repository', 'can_delete', 'can_delete'), ('repository', 'can_read', 'can_read'), ('repository', 'can_read', 'public'), ('repository', 'can_write', 'can_write'), ('repository', 'project', 'project'), ('repository', 'public', 'public')) AS c(object_type, relation, satisfying_relation)
                WHERE (c.object_type = v_filter_type AND c.relation = substring(t.subject_id from position('#' in t.subject_id) + 1) AND c.satisfying_relation = v_filter_relation)
                ) AND "public"."check_permission_internal"(v_filter_type, t.subject_id, 'can_read', 'repository', p_object_id, ARRAY[]::TEXT[]) = 1)
                UNION
                -- TTU userset: project -> can_read
                SELECT DISTINCT substring(pt.subject_id from 1 for position('#' in pt.subject_id) - 1) || '#' || v_filter_relation AS subject_id
                		FROM melange_tuples AS link
                		INNER JOIN melange_tuples AS pt ON (pt.object_type = link.subject_type AND pt.object_id = link.subject_id AND pt.relation IN (SELECT c.satisfying_relation
                FROM (VALUES ('project', 'can_delete', 'can_delete'), ('project', 'can_manage', 'can_manage'), ('project', 'can_manage', 'maintainer'), ('project', 'can_read', 'can_read'), ('project', 'can_read', 'maintainer'), ('project', 'can_write', 'can_write'), ('project', 'can_write', 'maintainer'), ('project', 'maintainer', 'maintainer'), ('project', 'organization', 'organization'), ('repository', 'can_delete', 'can_delete'), ('repository', 'can_read', 'can_read'), ('repository', 'can_read', 'public'), ('repository', 'can_write', 'can_write'), ('repository', 'project', 'project'), ('repository', 'public', 'public')) AS c(object_type, relation, satisfying_relation)
                WHERE (c.object_type = link.subject_type AND c.relation = 'can_read')))
                		WHERE (link.object_type = 'repository' AND link.object_id = p_object_id AND link.relation = 'project' AND pt.subject_type = v_filter_type AND position('#' in pt.subject_id) > 0 AND (substring(pt.subject_id from position('#' in pt.subject_id) + 1) = v_filter_relation OR EXISTS (
                SELECT 1
                FROM (VALUES ('project', 'can_delete', 'can_delete'), ('project', 'can_manage', 'can_manage'), ('project', 'can_manage', 'maintainer'), ('project', 'can_read', 'can_read'), ('project', 'can_read', 'maintainer'), ('project', 'can_write', 'can_write'), ('project', 'can_write', 'maintainer'), ('project', 'maintainer', 'maintainer'), ('project', 'organization', 'organization'), ('repository', 'can_delete', 'can_delete'), ('repository', 'can_read', 'can_read'), ('repository', 'can_read', 'public'), ('repository', 'can_write', 'can_write'), ('repository', 'project', 'project'), ('repository', 'public', 'public')) AS subj_c(object_type, relation, satisfying_relation)
                WHERE (subj_c.object_type = v_filter_type AND subj_c.relation = substring(pt.subject_id from position('#' in pt.subject_id) + 1) AND subj_c.satisfying_relation = v_filter_relation)
                )) AND link.subject_type IN ('project'))
                UNION
                -- TTU intermediate: parent object as userset reference
                SELECT DISTINCT link.subject_id || '#' || v_filter_relation AS subject_id
                		FROM melange_tuples AS link
                		WHERE (link.object_type = 'repository' AND link.object_id = p_object_id AND link.relation = 'project' AND link.subject_type = v_filter_type AND EXISTS (
                SELECT 1
                FROM (VALUES ('project', 'can_delete', 'can_delete'), ('project', 'can_manage', 'can_manage'), ('project', 'can_manage', 'maintainer'), ('project', 'can_read', 'can_read'), ('project', 'can_read', 'maintainer'), ('project', 'can_write', 'can_write'), ('project', 'can_write', 'maintainer'), ('project', 'maintainer', 'maintainer'), ('project', 'organization', 'organization'), ('repository', 'can_delete', 'can_delete'), ('repository', 'can_read', 'can_read'), ('repository', 'can_read', 'public'), ('repository', 'can_write', 'can_write'), ('repository', 'project', 'project'), ('repository', 'public', 'public')) AS c(object_type, relation, satisfying_relation)
                WHERE (c.object_type = link.subject_type AND c.relation = 'can_read' AND c.satisfying_relation = v_filter_relation)
                ) AND link.subject_type IN ('project'))
                UNION
                -- TTU nested: multi-hop chain resolution
                SELECT nested.subject_id
                FROM melange_tuples AS link
                CROSS JOIN LATERAL "public"."list_accessible_subjects"(link.subject_type, link.subject_id, 'can_read', p_subject_type) AS nested
                WHERE (link.object_type = 'repository' AND link.object_id = p_object_id AND link.relation = 'project' AND link.subject_type IN ('project'))
                UNION
                -- Self-candidate: when filter type matches object type
                -- e.g., querying document:1.viewer with filter document#writer
                -- should return document:1#writer if writer satisfies the relation
                SELECT p_object_id || '#' || v_filter_relation AS subject_id
                		WHERE (v_filter_type = 'repository' AND EXISTS (
                SELECT 1
                FROM (VALUES ('project', 'can_delete', 'can_delete'), ('project', 'can_manage', 'can_manage'), ('project', 'can_manage', 'maintainer'), ('project', 'can_read', 'can_read'), ('project', 'can_read', 'maintainer'), ('project', 'can_write', 'can_write'), ('project', 'can_write', 'maintainer'), ('project', 'maintainer', 'maintainer'), ('project', 'organization', 'organization'), ('repository', 'can_delete', 'can_delete'), ('repository', 'can_read', 'can_read'), ('repository', 'can_read', 'public'), ('repository', 'can_write', 'can_write'), ('repository', 'project', 'project'), ('repository', 'public', 'public')) AS c(object_type, relation, satisfying_relation)
                WHERE (c.object_type = 'repository' AND c.relation = 'can_read' AND c.satisfying_relation = v_filter_relation)
                ))
        ),
        paged AS (
            SELECT br.subject_id
            FROM base_results br
            WHERE p_after IS NULL OR (
                -- Compound comparison for wildcard-first ordering:
                -- (is_not_wildcard, subject_id) > (cursor_is_not_wildcard, cursor)
                (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END, br.subject_id) >
                (CASE WHEN p_after = '*' THEN 0 ELSE 1 END, p_after)
            )
            ORDER BY (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END), br.subject_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.subject_id FROM paged p
            ORDER BY (CASE WHEN p.subject_id = '*' THEN 0 ELSE 1 END), p.subject_id
            LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT r.subject_id FROM returned r
                      ORDER BY (CASE WHEN r.subject_id = '*' THEN 0 ELSE 1 END) DESC, r.subject_id DESC
                      LIMIT 1)
            END AS next_cursor
        )
        SELECT r.subject_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
    ELSE
        -- Regular subject type: find direct subjects and expand usersets
        RETURN QUERY
        WITH base_results AS (
            WITH subject_pool AS (
                SELECT DISTINCT t.subject_id
                FROM melange_tuples AS t
                WHERE (t.subject_type = p_subject_type AND p_subject_type IN ('user'))
            ),
            base_results AS (
                -- Direct tuple lookup with simple closure relations
                    SELECT DISTINCT t.subject_id
                    FROM melange_tuples AS t
                    WHERE (t.object_type = 'repository' AND t.relation IN ('can_read', 'public') AND t.object_id = p_object_id AND t.subject_type = p_subject_type AND position('#' in t.subject_id) = 0)
                    UNION
                    -- TTU: subjects via project -> can_read (complex parent relation - using subject_pool)
                    SELECT DISTINCT sp.subject_id
                    FROM subject_pool AS sp
                    CROSS JOIN melange_tuples AS link
                    WHERE (link.object_type = 'repository' AND link.object_id = p_object_id AND link.relation = 'project' AND link.subject_type IN ('project') AND "public"."check_permission_internal"(p_subject_type, sp.subject_id, 'can_read', link.subject_type, link.subject_id) = 1)
            ),
            has_wildcard AS (
                SELECT EXISTS (SELECT 1 FROM base_results br WHERE br.subject_id = '*') AS has_wildcard
            )
            SELECT br.subject_id
            FROM base_results AS br
            CROSS JOIN has_wildcard AS hw
            WHERE (NOT (hw.has_wildcard) OR br.subject_id = '*' OR (br.subject_id <> '*' AND "public"."check_permission_nw_internal"(p_subject_type, br.subject_id, 'can_read', 'repository', p_object_id, ARRAY[]::TEXT[]) = 1))
        ),
        paged AS (
            SELECT br.subject_id
            FROM base_results br
            WHERE p_after IS NULL OR (
                -- Compound comparison for wildcard-first ordering:
                -- (is_not_wildcard, subject_id) > (cursor_is_not_wildcard, cursor)
                (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END, br.subject_id) >
                (CASE WHEN p_after = '*' THEN 0 ELSE 1 END, p_after)
            )
            ORDER BY (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END), br.subject_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.subject_id FROM paged p
            ORDER BY (CASE WHEN p.subject_id = '*' THEN 0 ELSE 1 END), p.subject_id
            LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT r.subject_id FROM returned r
                      ORDER BY (CASE WHEN r.subject_id = '*' THEN 0 ELSE 1 END) DESC, r.subject_id DESC
                      LIMIT 1)
            END AS next_cursor
        )
        SELECT r.subject_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
    END IF;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';

-- Generated list_subjects function for run.can_read
-- Features: Recursive
-- Indirect anchor: project.can_read via ttu
CREATE OR REPLACE FUNCTION "public"."list_run_can_read_sub"(
    p_object_id TEXT,
    p_subject_type TEXT,
    p_limit INT DEFAULT NULL,
    p_after TEXT DEFAULT NULL
) RETURNS TABLE(subject_id TEXT, next_cursor TEXT) ROWS 100 AS $$
DECLARE
    v_is_userset_filter BOOLEAN;
    v_filter_type TEXT;
    v_filter_relation TEXT;
BEGIN
    v_is_userset_filter := position('#' in p_subject_type) > 0;
    IF v_is_userset_filter THEN
        v_filter_type := split_part(p_subject_type, '#', 1);
        v_filter_relation := split_part(p_subject_type, '#', 2);
        -- Self-candidate: when filter type matches object type
        IF v_filter_type = 'run' THEN
        IF EXISTS (
    		SELECT p_object_id || '#' || v_filter_relation AS subject_id
    		WHERE (v_filter_type = 'run' AND EXISTS (
    SELECT 1
    FROM (VALUES ('agent', 'can_execute', 'can_execute'), ('agent', 'can_manage', 'can_manage'), ('agent', 'can_read', 'can_read'), ('agent', 'project', 'project'), ('project', 'can_delete', 'can_delete'), ('project', 'can_manage', 'can_manage'), ('project', 'can_manage', 'maintainer'), ('project', 'can_read', 'can_read'), ('project', 'can_read', 'maintainer'), ('project', 'can_write', 'can_write'), ('project', 'can_write', 'maintainer'), ('project', 'maintainer', 'maintainer'), ('project', 'organization', 'organization'), ('run', 'agent', 'agent'), ('run', 'can_cancel', 'can_cancel'), ('run', 'can_read', 'can_read')) AS c(object_type, relation, satisfying_relation)
    WHERE (c.object_type = 'run' AND c.relation = 'can_read' AND c.satisfying_relation = v_filter_relation)
    ))
    ) THEN
        RETURN QUERY
        WITH base_results AS (
            SELECT p_object_id || '#' || v_filter_relation AS subject_id
            		WHERE (v_filter_type = 'run' AND EXISTS (
            SELECT 1
            FROM (VALUES ('agent', 'can_execute', 'can_execute'), ('agent', 'can_manage', 'can_manage'), ('agent', 'can_read', 'can_read'), ('agent', 'project', 'project'), ('project', 'can_delete', 'can_delete'), ('project', 'can_manage', 'can_manage'), ('project', 'can_manage', 'maintainer'), ('project', 'can_read', 'can_read'), ('project', 'can_read', 'maintainer'), ('project', 'can_write', 'can_write'), ('project', 'can_write', 'maintainer'), ('project', 'maintainer', 'maintainer'), ('project', 'organization', 'organization'), ('run', 'agent', 'agent'), ('run', 'can_cancel', 'can_cancel'), ('run', 'can_read', 'can_read')) AS c(object_type, relation, satisfying_relation)
            WHERE (c.object_type = 'run' AND c.relation = 'can_read' AND c.satisfying_relation = v_filter_relation)
            ))
        ),
        paged AS (
            SELECT br.subject_id
            FROM base_results br
            WHERE p_after IS NULL OR (
                -- Compound comparison for wildcard-first ordering:
                -- (is_not_wildcard, subject_id) > (cursor_is_not_wildcard, cursor)
                (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END, br.subject_id) >
                (CASE WHEN p_after = '*' THEN 0 ELSE 1 END, p_after)
            )
            ORDER BY (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END), br.subject_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.subject_id FROM paged p
            ORDER BY (CASE WHEN p.subject_id = '*' THEN 0 ELSE 1 END), p.subject_id
            LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT r.subject_id FROM returned r
                      ORDER BY (CASE WHEN r.subject_id = '*' THEN 0 ELSE 1 END) DESC, r.subject_id DESC
                      LIMIT 1)
            END AS next_cursor
        )
        SELECT r.subject_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
        RETURN;
    END IF;
    END IF;
        -- Userset filter case
        RETURN QUERY
        WITH base_results AS (
            WITH subject_candidates AS (
                -- From agent parents
                    SELECT DISTINCT s.subject_id
                    FROM melange_tuples AS link
                    CROSS JOIN LATERAL "public"."list_agent_can_read_sub"(link.subject_id, p_subject_type) AS s
                    WHERE (link.object_type = 'run' AND link.object_id = p_object_id AND link.relation = 'agent' AND link.subject_type = 'agent')
            )
            SELECT DISTINCT sc.subject_id
            FROM subject_candidates AS sc
            WHERE "public"."check_permission_internal"(v_filter_type, sc.subject_id, 'can_read', 'run', p_object_id, ARRAY[]::TEXT[]) = 1
        ),
        paged AS (
            SELECT br.subject_id
            FROM base_results br
            WHERE p_after IS NULL OR (
                -- Compound comparison for wildcard-first ordering:
                -- (is_not_wildcard, subject_id) > (cursor_is_not_wildcard, cursor)
                (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END, br.subject_id) >
                (CASE WHEN p_after = '*' THEN 0 ELSE 1 END, p_after)
            )
            ORDER BY (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END), br.subject_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.subject_id FROM paged p
            ORDER BY (CASE WHEN p.subject_id = '*' THEN 0 ELSE 1 END), p.subject_id
            LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT r.subject_id FROM returned r
                      ORDER BY (CASE WHEN r.subject_id = '*' THEN 0 ELSE 1 END) DESC, r.subject_id DESC
                      LIMIT 1)
            END AS next_cursor
        )
        SELECT r.subject_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
    ELSE
        -- Direct subject type case
        IF p_subject_type NOT IN ('user') THEN
        RETURN;
    END IF;
        RETURN QUERY
        WITH base_results AS (
            WITH subject_candidates AS (
                -- From agent parents
                    SELECT DISTINCT s.subject_id
                    FROM melange_tuples AS link
                    CROSS JOIN LATERAL "public"."list_agent_can_read_sub"(link.subject_id, p_subject_type) AS s
                    WHERE (link.object_type = 'run' AND link.object_id = p_object_id AND link.relation = 'agent' AND link.subject_type = 'agent')
            )
            SELECT DISTINCT sc.subject_id
            FROM subject_candidates AS sc
        ),
        paged AS (
            SELECT br.subject_id
            FROM base_results br
            WHERE p_after IS NULL OR (
                -- Compound comparison for wildcard-first ordering:
                -- (is_not_wildcard, subject_id) > (cursor_is_not_wildcard, cursor)
                (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END, br.subject_id) >
                (CASE WHEN p_after = '*' THEN 0 ELSE 1 END, p_after)
            )
            ORDER BY (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END), br.subject_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.subject_id FROM paged p
            ORDER BY (CASE WHEN p.subject_id = '*' THEN 0 ELSE 1 END), p.subject_id
            LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT r.subject_id FROM returned r
                      ORDER BY (CASE WHEN r.subject_id = '*' THEN 0 ELSE 1 END) DESC, r.subject_id DESC
                      LIMIT 1)
            END AS next_cursor
        )
        SELECT r.subject_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
    END IF;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';

-- Generated list_subjects function for state_volume.can_read
-- Features: Recursive
-- Indirect anchor: project.can_read via ttu
CREATE OR REPLACE FUNCTION "public"."list_state_volume_can_read_sub"(
    p_object_id TEXT,
    p_subject_type TEXT,
    p_limit INT DEFAULT NULL,
    p_after TEXT DEFAULT NULL
) RETURNS TABLE(subject_id TEXT, next_cursor TEXT) ROWS 100 AS $$
DECLARE
    v_is_userset_filter BOOLEAN;
    v_filter_type TEXT;
    v_filter_relation TEXT;
BEGIN
    v_is_userset_filter := position('#' in p_subject_type) > 0;
    IF v_is_userset_filter THEN
        v_filter_type := split_part(p_subject_type, '#', 1);
        v_filter_relation := split_part(p_subject_type, '#', 2);
        -- Self-candidate: when filter type matches object type
        IF v_filter_type = 'state_volume' THEN
        IF EXISTS (
    		SELECT p_object_id || '#' || v_filter_relation AS subject_id
    		WHERE (v_filter_type = 'state_volume' AND EXISTS (
    SELECT 1
    FROM (VALUES ('agent', 'can_execute', 'can_execute'), ('agent', 'can_manage', 'can_manage'), ('agent', 'can_read', 'can_read'), ('agent', 'project', 'project'), ('project', 'can_delete', 'can_delete'), ('project', 'can_manage', 'can_manage'), ('project', 'can_manage', 'maintainer'), ('project', 'can_read', 'can_read'), ('project', 'can_read', 'maintainer'), ('project', 'can_write', 'can_write'), ('project', 'can_write', 'maintainer'), ('project', 'maintainer', 'maintainer'), ('project', 'organization', 'organization'), ('state_volume', 'agent', 'agent'), ('state_volume', 'can_attach', 'can_attach'), ('state_volume', 'can_manage', 'can_manage'), ('state_volume', 'can_read', 'can_read'), ('state_volume', 'can_restore', 'can_restore')) AS c(object_type, relation, satisfying_relation)
    WHERE (c.object_type = 'state_volume' AND c.relation = 'can_read' AND c.satisfying_relation = v_filter_relation)
    ))
    ) THEN
        RETURN QUERY
        WITH base_results AS (
            SELECT p_object_id || '#' || v_filter_relation AS subject_id
            		WHERE (v_filter_type = 'state_volume' AND EXISTS (
            SELECT 1
            FROM (VALUES ('agent', 'can_execute', 'can_execute'), ('agent', 'can_manage', 'can_manage'), ('agent', 'can_read', 'can_read'), ('agent', 'project', 'project'), ('project', 'can_delete', 'can_delete'), ('project', 'can_manage', 'can_manage'), ('project', 'can_manage', 'maintainer'), ('project', 'can_read', 'can_read'), ('project', 'can_read', 'maintainer'), ('project', 'can_write', 'can_write'), ('project', 'can_write', 'maintainer'), ('project', 'maintainer', 'maintainer'), ('project', 'organization', 'organization'), ('state_volume', 'agent', 'agent'), ('state_volume', 'can_attach', 'can_attach'), ('state_volume', 'can_manage', 'can_manage'), ('state_volume', 'can_read', 'can_read'), ('state_volume', 'can_restore', 'can_restore')) AS c(object_type, relation, satisfying_relation)
            WHERE (c.object_type = 'state_volume' AND c.relation = 'can_read' AND c.satisfying_relation = v_filter_relation)
            ))
        ),
        paged AS (
            SELECT br.subject_id
            FROM base_results br
            WHERE p_after IS NULL OR (
                -- Compound comparison for wildcard-first ordering:
                -- (is_not_wildcard, subject_id) > (cursor_is_not_wildcard, cursor)
                (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END, br.subject_id) >
                (CASE WHEN p_after = '*' THEN 0 ELSE 1 END, p_after)
            )
            ORDER BY (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END), br.subject_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.subject_id FROM paged p
            ORDER BY (CASE WHEN p.subject_id = '*' THEN 0 ELSE 1 END), p.subject_id
            LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT r.subject_id FROM returned r
                      ORDER BY (CASE WHEN r.subject_id = '*' THEN 0 ELSE 1 END) DESC, r.subject_id DESC
                      LIMIT 1)
            END AS next_cursor
        )
        SELECT r.subject_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
        RETURN;
    END IF;
    END IF;
        -- Userset filter case
        RETURN QUERY
        WITH base_results AS (
            WITH subject_candidates AS (
                -- From agent parents
                    SELECT DISTINCT s.subject_id
                    FROM melange_tuples AS link
                    CROSS JOIN LATERAL "public"."list_agent_can_read_sub"(link.subject_id, p_subject_type) AS s
                    WHERE (link.object_type = 'state_volume' AND link.object_id = p_object_id AND link.relation = 'agent' AND link.subject_type = 'agent')
            )
            SELECT DISTINCT sc.subject_id
            FROM subject_candidates AS sc
            WHERE "public"."check_permission_internal"(v_filter_type, sc.subject_id, 'can_read', 'state_volume', p_object_id, ARRAY[]::TEXT[]) = 1
        ),
        paged AS (
            SELECT br.subject_id
            FROM base_results br
            WHERE p_after IS NULL OR (
                -- Compound comparison for wildcard-first ordering:
                -- (is_not_wildcard, subject_id) > (cursor_is_not_wildcard, cursor)
                (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END, br.subject_id) >
                (CASE WHEN p_after = '*' THEN 0 ELSE 1 END, p_after)
            )
            ORDER BY (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END), br.subject_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.subject_id FROM paged p
            ORDER BY (CASE WHEN p.subject_id = '*' THEN 0 ELSE 1 END), p.subject_id
            LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT r.subject_id FROM returned r
                      ORDER BY (CASE WHEN r.subject_id = '*' THEN 0 ELSE 1 END) DESC, r.subject_id DESC
                      LIMIT 1)
            END AS next_cursor
        )
        SELECT r.subject_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
    ELSE
        -- Direct subject type case
        IF p_subject_type NOT IN ('user') THEN
        RETURN;
    END IF;
        RETURN QUERY
        WITH base_results AS (
            WITH subject_candidates AS (
                -- From agent parents
                    SELECT DISTINCT s.subject_id
                    FROM melange_tuples AS link
                    CROSS JOIN LATERAL "public"."list_agent_can_read_sub"(link.subject_id, p_subject_type) AS s
                    WHERE (link.object_type = 'state_volume' AND link.object_id = p_object_id AND link.relation = 'agent' AND link.subject_type = 'agent')
            )
            SELECT DISTINCT sc.subject_id
            FROM subject_candidates AS sc
        ),
        paged AS (
            SELECT br.subject_id
            FROM base_results br
            WHERE p_after IS NULL OR (
                -- Compound comparison for wildcard-first ordering:
                -- (is_not_wildcard, subject_id) > (cursor_is_not_wildcard, cursor)
                (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END, br.subject_id) >
                (CASE WHEN p_after = '*' THEN 0 ELSE 1 END, p_after)
            )
            ORDER BY (CASE WHEN br.subject_id = '*' THEN 0 ELSE 1 END), br.subject_id
            LIMIT CASE WHEN p_limit IS NULL THEN NULL ELSE p_limit + 1 END
        ),
        returned AS (
            SELECT p.subject_id FROM paged p
            ORDER BY (CASE WHEN p.subject_id = '*' THEN 0 ELSE 1 END), p.subject_id
            LIMIT p_limit
        ),
        next AS (
            SELECT CASE
                WHEN p_limit IS NOT NULL AND (SELECT count(*) FROM paged) > p_limit
                THEN (SELECT r.subject_id FROM returned r
                      ORDER BY (CASE WHEN r.subject_id = '*' THEN 0 ELSE 1 END) DESC, r.subject_id DESC
                      LIMIT 1)
            END AS next_cursor
        )
        SELECT r.subject_id, n.next_cursor
        FROM returned r
        CROSS JOIN next n;
    END IF;
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';

-- ============================================================
-- Check Dispatchers
-- ============================================================

-- Generated internal dispatcher for check_permission_internal
-- Routes to specialized functions with p_visited for cycle detection in TTU patterns
-- Enforces depth limit of 25 to prevent stack overflow from deep permission chains
-- Phase 5: All relations use specialized functions - no generic fallback
CREATE OR REPLACE FUNCTION "public"."check_permission_internal"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_relation TEXT,
    p_object_type TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[]
) RETURNS INTEGER AS $$
BEGIN
    -- Depth limit check: prevent excessively deep permission resolution chains
    -- This catches both recursive TTU patterns and long userset chains
    IF array_length(p_visited, 1) >= 25 THEN
        RAISE EXCEPTION 'resolution too complex' USING ERRCODE = 'M2002';
    END IF;
    IF p_object_type = 'agent' THEN
        IF p_relation = 'project' THEN
        RETURN "public"."check_agent_project"(p_subject_type, p_subject_id, p_object_id, p_visited);
    END IF;
        IF p_relation = 'can_manage' THEN
        RETURN "public"."check_agent_can_manage"(p_subject_type, p_subject_id, p_object_id, p_visited);
    END IF;
        IF p_relation = 'can_execute' THEN
        RETURN "public"."check_agent_can_execute"(p_subject_type, p_subject_id, p_object_id, p_visited);
    END IF;
        IF p_relation = 'can_read' THEN
        RETURN "public"."check_agent_can_read"(p_subject_type, p_subject_id, p_object_id, p_visited);
    END IF;
        RETURN 0;
    END IF;
    IF p_object_type = 'organization' THEN
        IF p_relation = 'owner' THEN
        RETURN "public"."check_organization_owner"(p_subject_type, p_subject_id, p_object_id, p_visited);
    END IF;
        IF p_relation = 'admin' THEN
        RETURN "public"."check_organization_admin"(p_subject_type, p_subject_id, p_object_id, p_visited);
    END IF;
        IF p_relation = 'can_delete' THEN
        RETURN "public"."check_organization_can_delete"(p_subject_type, p_subject_id, p_object_id, p_visited);
    END IF;
        IF p_relation = 'can_manage' THEN
        RETURN "public"."check_organization_can_manage"(p_subject_type, p_subject_id, p_object_id, p_visited);
    END IF;
        IF p_relation = 'can_create_project' THEN
        RETURN "public"."check_organization_can_create_project"(p_subject_type, p_subject_id, p_object_id, p_visited);
    END IF;
        IF p_relation = 'can_manage_members' THEN
        RETURN "public"."check_organization_can_manage_members"(p_subject_type, p_subject_id, p_object_id, p_visited);
    END IF;
        IF p_relation = 'member' THEN
        RETURN "public"."check_organization_member"(p_subject_type, p_subject_id, p_object_id, p_visited);
    END IF;
        IF p_relation = 'can_read' THEN
        RETURN "public"."check_organization_can_read"(p_subject_type, p_subject_id, p_object_id, p_visited);
    END IF;
        RETURN 0;
    END IF;
    IF p_object_type = 'project' THEN
        IF p_relation = 'maintainer' THEN
        RETURN "public"."check_project_maintainer"(p_subject_type, p_subject_id, p_object_id, p_visited);
    END IF;
        IF p_relation = 'organization' THEN
        RETURN "public"."check_project_organization"(p_subject_type, p_subject_id, p_object_id, p_visited);
    END IF;
        IF p_relation = 'can_delete' THEN
        RETURN "public"."check_project_can_delete"(p_subject_type, p_subject_id, p_object_id, p_visited);
    END IF;
        IF p_relation = 'can_manage' THEN
        RETURN "public"."check_project_can_manage"(p_subject_type, p_subject_id, p_object_id, p_visited);
    END IF;
        IF p_relation = 'can_write' THEN
        RETURN "public"."check_project_can_write"(p_subject_type, p_subject_id, p_object_id, p_visited);
    END IF;
        IF p_relation = 'can_read' THEN
        RETURN "public"."check_project_can_read"(p_subject_type, p_subject_id, p_object_id, p_visited);
    END IF;
        RETURN 0;
    END IF;
    IF p_object_type = 'repository' THEN
        IF p_relation = 'project' THEN
        RETURN "public"."check_repository_project"(p_subject_type, p_subject_id, p_object_id, p_visited);
    END IF;
        IF p_relation = 'public' THEN
        RETURN "public"."check_repository_public"(p_subject_type, p_subject_id, p_object_id, p_visited);
    END IF;
        IF p_relation = 'can_delete' THEN
        RETURN "public"."check_repository_can_delete"(p_subject_type, p_subject_id, p_object_id, p_visited);
    END IF;
        IF p_relation = 'can_write' THEN
        RETURN "public"."check_repository_can_write"(p_subject_type, p_subject_id, p_object_id, p_visited);
    END IF;
        IF p_relation = 'can_read' THEN
        RETURN "public"."check_repository_can_read"(p_subject_type, p_subject_id, p_object_id, p_visited);
    END IF;
        RETURN 0;
    END IF;
    IF p_object_type = 'run' THEN
        IF p_relation = 'agent' THEN
        RETURN "public"."check_run_agent"(p_subject_type, p_subject_id, p_object_id, p_visited);
    END IF;
        IF p_relation = 'can_cancel' THEN
        RETURN "public"."check_run_can_cancel"(p_subject_type, p_subject_id, p_object_id, p_visited);
    END IF;
        IF p_relation = 'can_read' THEN
        RETURN "public"."check_run_can_read"(p_subject_type, p_subject_id, p_object_id, p_visited);
    END IF;
        RETURN 0;
    END IF;
    IF p_object_type = 'state_volume' THEN
        IF p_relation = 'agent' THEN
        RETURN "public"."check_state_volume_agent"(p_subject_type, p_subject_id, p_object_id, p_visited);
    END IF;
        IF p_relation = 'can_manage' THEN
        RETURN "public"."check_state_volume_can_manage"(p_subject_type, p_subject_id, p_object_id, p_visited);
    END IF;
        IF p_relation = 'can_restore' THEN
        RETURN "public"."check_state_volume_can_restore"(p_subject_type, p_subject_id, p_object_id, p_visited);
    END IF;
        IF p_relation = 'can_attach' THEN
        RETURN "public"."check_state_volume_can_attach"(p_subject_type, p_subject_id, p_object_id, p_visited);
    END IF;
        IF p_relation = 'can_read' THEN
        RETURN "public"."check_state_volume_can_read"(p_subject_type, p_subject_id, p_object_id, p_visited);
    END IF;
        RETURN 0;
    END IF;
    RETURN 0;
END;
$$ LANGUAGE plpgsql STABLE COST 1000;

-- Generated dispatcher for check_permission
-- Routes to specialized functions for all known type/relation pairs
CREATE OR REPLACE FUNCTION "public"."check_permission"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_relation TEXT,
    p_object_type TEXT,
    p_object_id TEXT
) RETURNS INTEGER AS $$
    SELECT "public"."check_permission_internal"(p_subject_type, p_subject_id, p_relation, p_object_type, p_object_id, ARRAY[]::TEXT[]);
$$ LANGUAGE sql STABLE;


-- Generated internal dispatcher for check_permission_nw_internal
-- Routes to specialized functions with p_visited for cycle detection in TTU patterns
-- Enforces depth limit of 25 to prevent stack overflow from deep permission chains
-- Phase 5: All relations use specialized functions - no generic fallback
CREATE OR REPLACE FUNCTION "public"."check_permission_nw_internal"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_relation TEXT,
    p_object_type TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[]
) RETURNS INTEGER AS $$
BEGIN
    -- Depth limit check: prevent excessively deep permission resolution chains
    -- This catches both recursive TTU patterns and long userset chains
    IF array_length(p_visited, 1) >= 25 THEN
        RAISE EXCEPTION 'resolution too complex' USING ERRCODE = 'M2002';
    END IF;
    IF p_object_type = 'agent' THEN
        IF p_relation = 'project' THEN
        RETURN "public"."check_agent_project_nw"(p_subject_type, p_subject_id, p_object_id, p_visited);
    END IF;
        IF p_relation = 'can_manage' THEN
        RETURN "public"."check_agent_can_manage_nw"(p_subject_type, p_subject_id, p_object_id, p_visited);
    END IF;
        IF p_relation = 'can_execute' THEN
        RETURN "public"."check_agent_can_execute_nw"(p_subject_type, p_subject_id, p_object_id, p_visited);
    END IF;
        IF p_relation = 'can_read' THEN
        RETURN "public"."check_agent_can_read_nw"(p_subject_type, p_subject_id, p_object_id, p_visited);
    END IF;
        RETURN 0;
    END IF;
    IF p_object_type = 'organization' THEN
        IF p_relation = 'owner' THEN
        RETURN "public"."check_organization_owner_nw"(p_subject_type, p_subject_id, p_object_id, p_visited);
    END IF;
        IF p_relation = 'admin' THEN
        RETURN "public"."check_organization_admin_nw"(p_subject_type, p_subject_id, p_object_id, p_visited);
    END IF;
        IF p_relation = 'can_delete' THEN
        RETURN "public"."check_organization_can_delete_nw"(p_subject_type, p_subject_id, p_object_id, p_visited);
    END IF;
        IF p_relation = 'can_manage' THEN
        RETURN "public"."check_organization_can_manage_nw"(p_subject_type, p_subject_id, p_object_id, p_visited);
    END IF;
        IF p_relation = 'can_create_project' THEN
        RETURN "public"."check_organization_can_create_project_nw"(p_subject_type, p_subject_id, p_object_id, p_visited);
    END IF;
        IF p_relation = 'can_manage_members' THEN
        RETURN "public"."check_organization_can_manage_members_nw"(p_subject_type, p_subject_id, p_object_id, p_visited);
    END IF;
        IF p_relation = 'member' THEN
        RETURN "public"."check_organization_member_nw"(p_subject_type, p_subject_id, p_object_id, p_visited);
    END IF;
        IF p_relation = 'can_read' THEN
        RETURN "public"."check_organization_can_read_nw"(p_subject_type, p_subject_id, p_object_id, p_visited);
    END IF;
        RETURN 0;
    END IF;
    IF p_object_type = 'project' THEN
        IF p_relation = 'maintainer' THEN
        RETURN "public"."check_project_maintainer_nw"(p_subject_type, p_subject_id, p_object_id, p_visited);
    END IF;
        IF p_relation = 'organization' THEN
        RETURN "public"."check_project_organization_nw"(p_subject_type, p_subject_id, p_object_id, p_visited);
    END IF;
        IF p_relation = 'can_delete' THEN
        RETURN "public"."check_project_can_delete_nw"(p_subject_type, p_subject_id, p_object_id, p_visited);
    END IF;
        IF p_relation = 'can_manage' THEN
        RETURN "public"."check_project_can_manage_nw"(p_subject_type, p_subject_id, p_object_id, p_visited);
    END IF;
        IF p_relation = 'can_write' THEN
        RETURN "public"."check_project_can_write_nw"(p_subject_type, p_subject_id, p_object_id, p_visited);
    END IF;
        IF p_relation = 'can_read' THEN
        RETURN "public"."check_project_can_read_nw"(p_subject_type, p_subject_id, p_object_id, p_visited);
    END IF;
        RETURN 0;
    END IF;
    IF p_object_type = 'repository' THEN
        IF p_relation = 'project' THEN
        RETURN "public"."check_repository_project_nw"(p_subject_type, p_subject_id, p_object_id, p_visited);
    END IF;
        IF p_relation = 'public' THEN
        RETURN "public"."check_repository_public_nw"(p_subject_type, p_subject_id, p_object_id, p_visited);
    END IF;
        IF p_relation = 'can_delete' THEN
        RETURN "public"."check_repository_can_delete_nw"(p_subject_type, p_subject_id, p_object_id, p_visited);
    END IF;
        IF p_relation = 'can_write' THEN
        RETURN "public"."check_repository_can_write_nw"(p_subject_type, p_subject_id, p_object_id, p_visited);
    END IF;
        IF p_relation = 'can_read' THEN
        RETURN "public"."check_repository_can_read_nw"(p_subject_type, p_subject_id, p_object_id, p_visited);
    END IF;
        RETURN 0;
    END IF;
    IF p_object_type = 'run' THEN
        IF p_relation = 'agent' THEN
        RETURN "public"."check_run_agent_nw"(p_subject_type, p_subject_id, p_object_id, p_visited);
    END IF;
        IF p_relation = 'can_cancel' THEN
        RETURN "public"."check_run_can_cancel_nw"(p_subject_type, p_subject_id, p_object_id, p_visited);
    END IF;
        IF p_relation = 'can_read' THEN
        RETURN "public"."check_run_can_read_nw"(p_subject_type, p_subject_id, p_object_id, p_visited);
    END IF;
        RETURN 0;
    END IF;
    IF p_object_type = 'state_volume' THEN
        IF p_relation = 'agent' THEN
        RETURN "public"."check_state_volume_agent_nw"(p_subject_type, p_subject_id, p_object_id, p_visited);
    END IF;
        IF p_relation = 'can_manage' THEN
        RETURN "public"."check_state_volume_can_manage_nw"(p_subject_type, p_subject_id, p_object_id, p_visited);
    END IF;
        IF p_relation = 'can_restore' THEN
        RETURN "public"."check_state_volume_can_restore_nw"(p_subject_type, p_subject_id, p_object_id, p_visited);
    END IF;
        IF p_relation = 'can_attach' THEN
        RETURN "public"."check_state_volume_can_attach_nw"(p_subject_type, p_subject_id, p_object_id, p_visited);
    END IF;
        IF p_relation = 'can_read' THEN
        RETURN "public"."check_state_volume_can_read_nw"(p_subject_type, p_subject_id, p_object_id, p_visited);
    END IF;
        RETURN 0;
    END IF;
    RETURN 0;
END;
$$ LANGUAGE plpgsql STABLE COST 1000;

-- Generated dispatcher for check_permission_nw
-- Routes to specialized functions for all known type/relation pairs
CREATE OR REPLACE FUNCTION "public"."check_permission_nw"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_relation TEXT,
    p_object_type TEXT,
    p_object_id TEXT
) RETURNS INTEGER AS $$
    SELECT "public"."check_permission_nw_internal"(p_subject_type, p_subject_id, p_relation, p_object_type, p_object_id, ARRAY[]::TEXT[]);
$$ LANGUAGE sql STABLE;


-- Generated bulk dispatcher for check_permission_bulk
-- Routes 31 (object_type, relation) pairs across 6 object types
-- Uses separate IF blocks to execute only branches for object types present in the batch
CREATE OR REPLACE FUNCTION "public"."check_permission_bulk"(
    p_subject_types TEXT[],
    p_subject_ids TEXT[],
    p_relations TEXT[],
    p_object_types TEXT[],
    p_object_ids TEXT[]
) RETURNS TABLE(idx INTEGER, allowed INTEGER) AS $$
BEGIN
    IF 'agent' = ANY(p_object_types) THEN
        RETURN QUERY
        WITH requests AS MATERIALIZED (
        SELECT t.* FROM UNNEST(p_subject_types, p_subject_ids, p_relations, p_object_types, p_object_ids)
            WITH ORDINALITY AS t(subject_type, subject_id, relation, object_type, object_id, idx)
            WHERE t.object_type = 'agent'
    )
    		SELECT r.idx::INTEGER, CASE
            WHEN (r.subject_type = 'agent' AND position('#' in r.subject_id) > 0 AND split_part(r.subject_id, '#', 1) = r.object_id AND substring(r.subject_id from position('#' in r.subject_id) + 1) IN ('project')) THEN 1
            WHEN EXISTS (
    SELECT 1
    FROM melange_tuples AS t
    WHERE (t.subject_type = r.subject_type AND t.subject_id = r.subject_id AND t.relation = 'project' AND t.object_type = 'agent' AND t.object_id = r.object_id AND r.subject_type IN ('project'))
    ) THEN 1
            ELSE 0
        END
    		FROM requests AS r
    		WHERE r.relation = 'project'
    
    UNION ALL
    
    SELECT r.idx::INTEGER, "public"."check_agent_can_manage"(r.subject_type, r.subject_id, r.object_id, ARRAY[]::TEXT[])
    FROM requests AS r
    WHERE r.relation = 'can_manage'
    
    UNION ALL
    
    SELECT r.idx::INTEGER, "public"."check_agent_can_execute"(r.subject_type, r.subject_id, r.object_id, ARRAY[]::TEXT[])
    FROM requests AS r
    WHERE r.relation = 'can_execute'
    
    UNION ALL
    
    SELECT r.idx::INTEGER, "public"."check_agent_can_read"(r.subject_type, r.subject_id, r.object_id, ARRAY[]::TEXT[])
    FROM requests AS r
    WHERE r.relation = 'can_read'
    
    UNION ALL
    
    SELECT r.idx::INTEGER, 0
    FROM requests AS r
    WHERE r.relation NOT IN ('project', 'can_manage', 'can_execute', 'can_read');
    END IF;
    IF 'organization' = ANY(p_object_types) THEN
        RETURN QUERY
        WITH requests AS MATERIALIZED (
        SELECT t.* FROM UNNEST(p_subject_types, p_subject_ids, p_relations, p_object_types, p_object_ids)
            WITH ORDINALITY AS t(subject_type, subject_id, relation, object_type, object_id, idx)
            WHERE t.object_type = 'organization'
    )
    		SELECT r.idx::INTEGER, CASE
            WHEN (r.subject_type = 'organization' AND position('#' in r.subject_id) > 0 AND split_part(r.subject_id, '#', 1) = r.object_id AND substring(r.subject_id from position('#' in r.subject_id) + 1) IN ('owner')) THEN 1
            WHEN EXISTS (
    SELECT 1
    FROM melange_tuples AS t
    WHERE (t.subject_type = r.subject_type AND t.subject_id = r.subject_id AND t.relation = 'owner' AND t.object_type = 'organization' AND t.object_id = r.object_id AND r.subject_type IN ('user'))
    ) THEN 1
            ELSE 0
        END
    		FROM requests AS r
    		WHERE r.relation = 'owner'
    
    UNION ALL
    
    SELECT r.idx::INTEGER, "public"."check_organization_admin"(r.subject_type, r.subject_id, r.object_id, ARRAY[]::TEXT[])
    FROM requests AS r
    WHERE r.relation = 'admin'
    
    UNION ALL
    
    SELECT r.idx::INTEGER, "public"."check_organization_can_delete"(r.subject_type, r.subject_id, r.object_id, ARRAY[]::TEXT[])
    FROM requests AS r
    WHERE r.relation = 'can_delete'
    
    UNION ALL
    
    SELECT r.idx::INTEGER, "public"."check_organization_can_manage"(r.subject_type, r.subject_id, r.object_id, ARRAY[]::TEXT[])
    FROM requests AS r
    WHERE r.relation = 'can_manage'
    
    UNION ALL
    
    SELECT r.idx::INTEGER, "public"."check_organization_can_create_project"(r.subject_type, r.subject_id, r.object_id, ARRAY[]::TEXT[])
    FROM requests AS r
    WHERE r.relation = 'can_create_project'
    
    UNION ALL
    
    SELECT r.idx::INTEGER, "public"."check_organization_can_manage_members"(r.subject_type, r.subject_id, r.object_id, ARRAY[]::TEXT[])
    FROM requests AS r
    WHERE r.relation = 'can_manage_members'
    
    UNION ALL
    
    SELECT r.idx::INTEGER, "public"."check_organization_member"(r.subject_type, r.subject_id, r.object_id, ARRAY[]::TEXT[])
    FROM requests AS r
    WHERE r.relation = 'member'
    
    UNION ALL
    
    SELECT r.idx::INTEGER, "public"."check_organization_can_read"(r.subject_type, r.subject_id, r.object_id, ARRAY[]::TEXT[])
    FROM requests AS r
    WHERE r.relation = 'can_read'
    
    UNION ALL
    
    SELECT r.idx::INTEGER, 0
    FROM requests AS r
    WHERE r.relation NOT IN ('owner', 'admin', 'can_delete', 'can_manage', 'can_create_project', 'can_manage_members', 'member', 'can_read');
    END IF;
    IF 'project' = ANY(p_object_types) THEN
        RETURN QUERY
        WITH requests AS MATERIALIZED (
        SELECT t.* FROM UNNEST(p_subject_types, p_subject_ids, p_relations, p_object_types, p_object_ids)
            WITH ORDINALITY AS t(subject_type, subject_id, relation, object_type, object_id, idx)
            WHERE t.object_type = 'project'
    )
    		SELECT r.idx::INTEGER, CASE
            WHEN (r.subject_type = 'project' AND position('#' in r.subject_id) > 0 AND split_part(r.subject_id, '#', 1) = r.object_id AND substring(r.subject_id from position('#' in r.subject_id) + 1) IN ('maintainer')) THEN 1
            WHEN EXISTS (
    SELECT 1
    FROM melange_tuples AS t
    WHERE (t.subject_type = r.subject_type AND t.subject_id = r.subject_id AND t.relation = 'maintainer' AND t.object_type = 'project' AND t.object_id = r.object_id AND r.subject_type IN ('user'))
    ) THEN 1
            ELSE 0
        END
    		FROM requests AS r
    		WHERE r.relation = 'maintainer'
    
    UNION ALL
    
    		SELECT r.idx::INTEGER, CASE
            WHEN (r.subject_type = 'project' AND position('#' in r.subject_id) > 0 AND split_part(r.subject_id, '#', 1) = r.object_id AND substring(r.subject_id from position('#' in r.subject_id) + 1) IN ('organization')) THEN 1
            WHEN EXISTS (
    SELECT 1
    FROM melange_tuples AS t
    WHERE (t.subject_type = r.subject_type AND t.subject_id = r.subject_id AND t.relation = 'organization' AND t.object_type = 'project' AND t.object_id = r.object_id AND r.subject_type IN ('organization'))
    ) THEN 1
            ELSE 0
        END
    		FROM requests AS r
    		WHERE r.relation = 'organization'
    
    UNION ALL
    
    SELECT r.idx::INTEGER, "public"."check_project_can_delete"(r.subject_type, r.subject_id, r.object_id, ARRAY[]::TEXT[])
    FROM requests AS r
    WHERE r.relation = 'can_delete'
    
    UNION ALL
    
    SELECT r.idx::INTEGER, "public"."check_project_can_manage"(r.subject_type, r.subject_id, r.object_id, ARRAY[]::TEXT[])
    FROM requests AS r
    WHERE r.relation = 'can_manage'
    
    UNION ALL
    
    SELECT r.idx::INTEGER, "public"."check_project_can_write"(r.subject_type, r.subject_id, r.object_id, ARRAY[]::TEXT[])
    FROM requests AS r
    WHERE r.relation = 'can_write'
    
    UNION ALL
    
    SELECT r.idx::INTEGER, "public"."check_project_can_read"(r.subject_type, r.subject_id, r.object_id, ARRAY[]::TEXT[])
    FROM requests AS r
    WHERE r.relation = 'can_read'
    
    UNION ALL
    
    SELECT r.idx::INTEGER, 0
    FROM requests AS r
    WHERE r.relation NOT IN ('maintainer', 'organization', 'can_delete', 'can_manage', 'can_write', 'can_read');
    END IF;
    IF 'repository' = ANY(p_object_types) THEN
        RETURN QUERY
        WITH requests AS MATERIALIZED (
        SELECT t.* FROM UNNEST(p_subject_types, p_subject_ids, p_relations, p_object_types, p_object_ids)
            WITH ORDINALITY AS t(subject_type, subject_id, relation, object_type, object_id, idx)
            WHERE t.object_type = 'repository'
    )
    		SELECT r.idx::INTEGER, CASE
            WHEN (r.subject_type = 'repository' AND position('#' in r.subject_id) > 0 AND split_part(r.subject_id, '#', 1) = r.object_id AND substring(r.subject_id from position('#' in r.subject_id) + 1) IN ('project')) THEN 1
            WHEN EXISTS (
    SELECT 1
    FROM melange_tuples AS t
    WHERE (t.subject_type = r.subject_type AND t.subject_id = r.subject_id AND t.relation = 'project' AND t.object_type = 'repository' AND t.object_id = r.object_id AND r.subject_type IN ('project'))
    ) THEN 1
            ELSE 0
        END
    		FROM requests AS r
    		WHERE r.relation = 'project'
    
    UNION ALL
    
    SELECT r.idx::INTEGER, "public"."check_repository_public"(r.subject_type, r.subject_id, r.object_id, ARRAY[]::TEXT[])
    FROM requests AS r
    WHERE r.relation = 'public'
    
    UNION ALL
    
    SELECT r.idx::INTEGER, "public"."check_repository_can_delete"(r.subject_type, r.subject_id, r.object_id, ARRAY[]::TEXT[])
    FROM requests AS r
    WHERE r.relation = 'can_delete'
    
    UNION ALL
    
    SELECT r.idx::INTEGER, "public"."check_repository_can_write"(r.subject_type, r.subject_id, r.object_id, ARRAY[]::TEXT[])
    FROM requests AS r
    WHERE r.relation = 'can_write'
    
    UNION ALL
    
    SELECT r.idx::INTEGER, "public"."check_repository_can_read"(r.subject_type, r.subject_id, r.object_id, ARRAY[]::TEXT[])
    FROM requests AS r
    WHERE r.relation = 'can_read'
    
    UNION ALL
    
    SELECT r.idx::INTEGER, 0
    FROM requests AS r
    WHERE r.relation NOT IN ('project', 'public', 'can_delete', 'can_write', 'can_read');
    END IF;
    IF 'run' = ANY(p_object_types) THEN
        RETURN QUERY
        WITH requests AS MATERIALIZED (
        SELECT t.* FROM UNNEST(p_subject_types, p_subject_ids, p_relations, p_object_types, p_object_ids)
            WITH ORDINALITY AS t(subject_type, subject_id, relation, object_type, object_id, idx)
            WHERE t.object_type = 'run'
    )
    		SELECT r.idx::INTEGER, CASE
            WHEN (r.subject_type = 'run' AND position('#' in r.subject_id) > 0 AND split_part(r.subject_id, '#', 1) = r.object_id AND substring(r.subject_id from position('#' in r.subject_id) + 1) IN ('agent')) THEN 1
            WHEN EXISTS (
    SELECT 1
    FROM melange_tuples AS t
    WHERE (t.subject_type = r.subject_type AND t.subject_id = r.subject_id AND t.relation = 'agent' AND t.object_type = 'run' AND t.object_id = r.object_id AND r.subject_type IN ('agent'))
    ) THEN 1
            ELSE 0
        END
    		FROM requests AS r
    		WHERE r.relation = 'agent'
    
    UNION ALL
    
    SELECT r.idx::INTEGER, "public"."check_run_can_cancel"(r.subject_type, r.subject_id, r.object_id, ARRAY[]::TEXT[])
    FROM requests AS r
    WHERE r.relation = 'can_cancel'
    
    UNION ALL
    
    SELECT r.idx::INTEGER, "public"."check_run_can_read"(r.subject_type, r.subject_id, r.object_id, ARRAY[]::TEXT[])
    FROM requests AS r
    WHERE r.relation = 'can_read'
    
    UNION ALL
    
    SELECT r.idx::INTEGER, 0
    FROM requests AS r
    WHERE r.relation NOT IN ('agent', 'can_cancel', 'can_read');
    END IF;
    IF 'state_volume' = ANY(p_object_types) THEN
        RETURN QUERY
        WITH requests AS MATERIALIZED (
        SELECT t.* FROM UNNEST(p_subject_types, p_subject_ids, p_relations, p_object_types, p_object_ids)
            WITH ORDINALITY AS t(subject_type, subject_id, relation, object_type, object_id, idx)
            WHERE t.object_type = 'state_volume'
    )
    		SELECT r.idx::INTEGER, CASE
            WHEN (r.subject_type = 'state_volume' AND position('#' in r.subject_id) > 0 AND split_part(r.subject_id, '#', 1) = r.object_id AND substring(r.subject_id from position('#' in r.subject_id) + 1) IN ('agent')) THEN 1
            WHEN EXISTS (
    SELECT 1
    FROM melange_tuples AS t
    WHERE (t.subject_type = r.subject_type AND t.subject_id = r.subject_id AND t.relation = 'agent' AND t.object_type = 'state_volume' AND t.object_id = r.object_id AND r.subject_type IN ('agent'))
    ) THEN 1
            ELSE 0
        END
    		FROM requests AS r
    		WHERE r.relation = 'agent'
    
    UNION ALL
    
    SELECT r.idx::INTEGER, "public"."check_state_volume_can_manage"(r.subject_type, r.subject_id, r.object_id, ARRAY[]::TEXT[])
    FROM requests AS r
    WHERE r.relation = 'can_manage'
    
    UNION ALL
    
    SELECT r.idx::INTEGER, "public"."check_state_volume_can_restore"(r.subject_type, r.subject_id, r.object_id, ARRAY[]::TEXT[])
    FROM requests AS r
    WHERE r.relation = 'can_restore'
    
    UNION ALL
    
    SELECT r.idx::INTEGER, "public"."check_state_volume_can_attach"(r.subject_type, r.subject_id, r.object_id, ARRAY[]::TEXT[])
    FROM requests AS r
    WHERE r.relation = 'can_attach'
    
    UNION ALL
    
    SELECT r.idx::INTEGER, "public"."check_state_volume_can_read"(r.subject_type, r.subject_id, r.object_id, ARRAY[]::TEXT[])
    FROM requests AS r
    WHERE r.relation = 'can_read'
    
    UNION ALL
    
    SELECT r.idx::INTEGER, 0
    FROM requests AS r
    WHERE r.relation NOT IN ('agent', 'can_manage', 'can_restore', 'can_attach', 'can_read');
    END IF;
    RETURN QUERY
        SELECT t.idx::INTEGER, 0
    FROM UNNEST(p_subject_types, p_subject_ids, p_relations, p_object_types, p_object_ids)
      WITH ORDINALITY AS t(subject_type, subject_id, relation, object_type, object_id, idx)
    WHERE (t.object_type, t.relation) NOT IN (('agent', 'project'), ('organization', 'owner'), ('project', 'maintainer'), ('project', 'organization'), ('repository', 'project'), ('repository', 'public'), ('run', 'agent'), ('state_volume', 'agent'), ('organization', 'admin'), ('organization', 'can_delete'), ('organization', 'can_manage'), ('project', 'can_delete'), ('project', 'can_manage'), ('project', 'can_write'), ('organization', 'can_create_project'), ('organization', 'can_manage_members'), ('organization', 'member'), ('repository', 'can_delete'), ('agent', 'can_manage'), ('agent', 'can_execute'), ('repository', 'can_write'), ('organization', 'can_read'), ('state_volume', 'can_manage'), ('state_volume', 'can_restore'), ('run', 'can_cancel'), ('state_volume', 'can_attach'), ('project', 'can_read'), ('agent', 'can_read'), ('repository', 'can_read'), ('run', 'can_read'), ('state_volume', 'can_read'));
END;
$$ LANGUAGE plpgsql STABLE
SET search_path = 'public';


-- ============================================================
-- Explain Dispatcher
-- ============================================================

-- Generated internal dispatcher for explain_permission
-- Routes (object_type, relation) to specialised explain_* functions
-- Returns a no-entry Trace JSONB when the pair is unknown so callers
-- never see NULL
CREATE OR REPLACE FUNCTION "public"."explain_permission_internal"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_relation TEXT,
    p_object_type TEXT,
    p_object_id TEXT,
    p_visited TEXT [] DEFAULT ARRAY[]::TEXT[],
    p_max_nodes INTEGER DEFAULT NULL
) RETURNS JSONB AS $$
BEGIN
    -- Depth limit check shared with check_permission_internal
    IF array_length(p_visited, 1) >= 25 THEN
        RAISE EXCEPTION 'resolution too complex' USING ERRCODE = 'M2002';
    END IF;
    IF p_object_type = 'agent' THEN
        IF p_relation = 'project' THEN
        RETURN "public"."explain_agent_project"(p_subject_type, p_subject_id, p_object_id, p_visited, p_max_nodes);
    END IF;
        IF p_relation = 'can_manage' THEN
        RETURN "public"."explain_agent_can_manage"(p_subject_type, p_subject_id, p_object_id, p_visited, p_max_nodes);
    END IF;
        IF p_relation = 'can_execute' THEN
        RETURN "public"."explain_agent_can_execute"(p_subject_type, p_subject_id, p_object_id, p_visited, p_max_nodes);
    END IF;
        IF p_relation = 'can_read' THEN
        RETURN "public"."explain_agent_can_read"(p_subject_type, p_subject_id, p_object_id, p_visited, p_max_nodes);
    END IF;
        RETURN jsonb_build_object(
        'object', (p_object_type || ':' || p_object_id),
        'relation', p_relation,
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', jsonb_build_object('type', 'union', 'label', 'explain not yet supported for this (object_type, relation) — no generated explain function for the requested pair. Confirm the pair exists in the migrated schema.', 'children', '[]'::jsonb, 'result', false),
        'truncated', false,
        'node_count', 1);
    END IF;
    IF p_object_type = 'organization' THEN
        IF p_relation = 'owner' THEN
        RETURN "public"."explain_organization_owner"(p_subject_type, p_subject_id, p_object_id, p_visited, p_max_nodes);
    END IF;
        IF p_relation = 'admin' THEN
        RETURN "public"."explain_organization_admin"(p_subject_type, p_subject_id, p_object_id, p_visited, p_max_nodes);
    END IF;
        IF p_relation = 'can_delete' THEN
        RETURN "public"."explain_organization_can_delete"(p_subject_type, p_subject_id, p_object_id, p_visited, p_max_nodes);
    END IF;
        IF p_relation = 'can_manage' THEN
        RETURN "public"."explain_organization_can_manage"(p_subject_type, p_subject_id, p_object_id, p_visited, p_max_nodes);
    END IF;
        IF p_relation = 'can_create_project' THEN
        RETURN "public"."explain_organization_can_create_project"(p_subject_type, p_subject_id, p_object_id, p_visited, p_max_nodes);
    END IF;
        IF p_relation = 'can_manage_members' THEN
        RETURN "public"."explain_organization_can_manage_members"(p_subject_type, p_subject_id, p_object_id, p_visited, p_max_nodes);
    END IF;
        IF p_relation = 'member' THEN
        RETURN "public"."explain_organization_member"(p_subject_type, p_subject_id, p_object_id, p_visited, p_max_nodes);
    END IF;
        IF p_relation = 'can_read' THEN
        RETURN "public"."explain_organization_can_read"(p_subject_type, p_subject_id, p_object_id, p_visited, p_max_nodes);
    END IF;
        RETURN jsonb_build_object(
        'object', (p_object_type || ':' || p_object_id),
        'relation', p_relation,
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', jsonb_build_object('type', 'union', 'label', 'explain not yet supported for this (object_type, relation) — no generated explain function for the requested pair. Confirm the pair exists in the migrated schema.', 'children', '[]'::jsonb, 'result', false),
        'truncated', false,
        'node_count', 1);
    END IF;
    IF p_object_type = 'project' THEN
        IF p_relation = 'maintainer' THEN
        RETURN "public"."explain_project_maintainer"(p_subject_type, p_subject_id, p_object_id, p_visited, p_max_nodes);
    END IF;
        IF p_relation = 'organization' THEN
        RETURN "public"."explain_project_organization"(p_subject_type, p_subject_id, p_object_id, p_visited, p_max_nodes);
    END IF;
        IF p_relation = 'can_delete' THEN
        RETURN "public"."explain_project_can_delete"(p_subject_type, p_subject_id, p_object_id, p_visited, p_max_nodes);
    END IF;
        IF p_relation = 'can_manage' THEN
        RETURN "public"."explain_project_can_manage"(p_subject_type, p_subject_id, p_object_id, p_visited, p_max_nodes);
    END IF;
        IF p_relation = 'can_write' THEN
        RETURN "public"."explain_project_can_write"(p_subject_type, p_subject_id, p_object_id, p_visited, p_max_nodes);
    END IF;
        IF p_relation = 'can_read' THEN
        RETURN "public"."explain_project_can_read"(p_subject_type, p_subject_id, p_object_id, p_visited, p_max_nodes);
    END IF;
        RETURN jsonb_build_object(
        'object', (p_object_type || ':' || p_object_id),
        'relation', p_relation,
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', jsonb_build_object('type', 'union', 'label', 'explain not yet supported for this (object_type, relation) — no generated explain function for the requested pair. Confirm the pair exists in the migrated schema.', 'children', '[]'::jsonb, 'result', false),
        'truncated', false,
        'node_count', 1);
    END IF;
    IF p_object_type = 'repository' THEN
        IF p_relation = 'project' THEN
        RETURN "public"."explain_repository_project"(p_subject_type, p_subject_id, p_object_id, p_visited, p_max_nodes);
    END IF;
        IF p_relation = 'public' THEN
        RETURN "public"."explain_repository_public"(p_subject_type, p_subject_id, p_object_id, p_visited, p_max_nodes);
    END IF;
        IF p_relation = 'can_delete' THEN
        RETURN "public"."explain_repository_can_delete"(p_subject_type, p_subject_id, p_object_id, p_visited, p_max_nodes);
    END IF;
        IF p_relation = 'can_write' THEN
        RETURN "public"."explain_repository_can_write"(p_subject_type, p_subject_id, p_object_id, p_visited, p_max_nodes);
    END IF;
        IF p_relation = 'can_read' THEN
        RETURN "public"."explain_repository_can_read"(p_subject_type, p_subject_id, p_object_id, p_visited, p_max_nodes);
    END IF;
        RETURN jsonb_build_object(
        'object', (p_object_type || ':' || p_object_id),
        'relation', p_relation,
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', jsonb_build_object('type', 'union', 'label', 'explain not yet supported for this (object_type, relation) — no generated explain function for the requested pair. Confirm the pair exists in the migrated schema.', 'children', '[]'::jsonb, 'result', false),
        'truncated', false,
        'node_count', 1);
    END IF;
    IF p_object_type = 'run' THEN
        IF p_relation = 'agent' THEN
        RETURN "public"."explain_run_agent"(p_subject_type, p_subject_id, p_object_id, p_visited, p_max_nodes);
    END IF;
        IF p_relation = 'can_cancel' THEN
        RETURN "public"."explain_run_can_cancel"(p_subject_type, p_subject_id, p_object_id, p_visited, p_max_nodes);
    END IF;
        IF p_relation = 'can_read' THEN
        RETURN "public"."explain_run_can_read"(p_subject_type, p_subject_id, p_object_id, p_visited, p_max_nodes);
    END IF;
        RETURN jsonb_build_object(
        'object', (p_object_type || ':' || p_object_id),
        'relation', p_relation,
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', jsonb_build_object('type', 'union', 'label', 'explain not yet supported for this (object_type, relation) — no generated explain function for the requested pair. Confirm the pair exists in the migrated schema.', 'children', '[]'::jsonb, 'result', false),
        'truncated', false,
        'node_count', 1);
    END IF;
    IF p_object_type = 'state_volume' THEN
        IF p_relation = 'agent' THEN
        RETURN "public"."explain_state_volume_agent"(p_subject_type, p_subject_id, p_object_id, p_visited, p_max_nodes);
    END IF;
        IF p_relation = 'can_manage' THEN
        RETURN "public"."explain_state_volume_can_manage"(p_subject_type, p_subject_id, p_object_id, p_visited, p_max_nodes);
    END IF;
        IF p_relation = 'can_restore' THEN
        RETURN "public"."explain_state_volume_can_restore"(p_subject_type, p_subject_id, p_object_id, p_visited, p_max_nodes);
    END IF;
        IF p_relation = 'can_attach' THEN
        RETURN "public"."explain_state_volume_can_attach"(p_subject_type, p_subject_id, p_object_id, p_visited, p_max_nodes);
    END IF;
        IF p_relation = 'can_read' THEN
        RETURN "public"."explain_state_volume_can_read"(p_subject_type, p_subject_id, p_object_id, p_visited, p_max_nodes);
    END IF;
        RETURN jsonb_build_object(
        'object', (p_object_type || ':' || p_object_id),
        'relation', p_relation,
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', jsonb_build_object('type', 'union', 'label', 'explain not yet supported for this (object_type, relation) — no generated explain function for the requested pair. Confirm the pair exists in the migrated schema.', 'children', '[]'::jsonb, 'result', false),
        'truncated', false,
        'node_count', 1);
    END IF;
    RETURN jsonb_build_object(
        'object', (p_object_type || ':' || p_object_id),
        'relation', p_relation,
        'subject', (p_subject_type || ':' || p_subject_id),
        'result', false,
        'root', jsonb_build_object('type', 'union', 'label', 'explain not yet supported for this (object_type, relation) — no generated explain function for the requested pair. Confirm the pair exists in the migrated schema.', 'children', '[]'::jsonb, 'result', false),
        'truncated', false,
        'node_count', 1);
END;
$$ LANGUAGE plpgsql STABLE COST 1000;

-- Generated public dispatcher for explain_permission
-- Companion to check_permission — returns a JSONB Trace describing
-- why the check decision was reached
CREATE OR REPLACE FUNCTION "public"."explain_permission"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_relation TEXT,
    p_object_type TEXT,
    p_object_id TEXT,
    p_max_nodes INTEGER DEFAULT NULL
) RETURNS JSONB AS $$
    SELECT "public"."explain_permission_internal"(p_subject_type, p_subject_id, p_relation, p_object_type, p_object_id, ARRAY[]::TEXT[], p_max_nodes);
$$ LANGUAGE sql STABLE;


-- ============================================================
-- Expand Dispatcher
-- ============================================================

-- Generated internal dispatcher for expand_permission
-- Routes (object_type, relation) to specialised expand_* functions
-- Returns an empty Leaf.Users sentinel for unknown / not-yet-supported
-- pairs so OpenFGA tooling deserialises without special-casing.
-- Callers that need to distinguish 'no one has access' from 'expand
-- not supported for this relation' should compare against Check.
CREATE OR REPLACE FUNCTION "public"."expand_permission_internal"(
    p_object_type TEXT,
    p_object_id TEXT,
    p_relation TEXT,
    p_subject_type TEXT DEFAULT NULL,
    p_max_leaf INTEGER DEFAULT NULL
) RETURNS JSONB AS $$
BEGIN
    IF p_object_type = 'agent' THEN
        IF p_relation = 'project' THEN
        RETURN "public"."expand_agent_project"(p_object_id, p_subject_type, p_max_leaf);
    END IF;
        IF p_relation = 'can_manage' THEN
        RETURN "public"."expand_agent_can_manage"(p_object_id, p_subject_type, p_max_leaf);
    END IF;
        IF p_relation = 'can_execute' THEN
        RETURN "public"."expand_agent_can_execute"(p_object_id, p_subject_type, p_max_leaf);
    END IF;
        IF p_relation = 'can_read' THEN
        RETURN "public"."expand_agent_can_read"(p_object_id, p_subject_type, p_max_leaf);
    END IF;
        RETURN jsonb_build_object('root', jsonb_build_object('name', (p_object_type || ':' || p_object_id || '#' || p_relation)) || jsonb_build_object('leaf', jsonb_build_object('users', jsonb_build_object('users', '[]'::jsonb))));
    END IF;
    IF p_object_type = 'organization' THEN
        IF p_relation = 'owner' THEN
        RETURN "public"."expand_organization_owner"(p_object_id, p_subject_type, p_max_leaf);
    END IF;
        IF p_relation = 'admin' THEN
        RETURN "public"."expand_organization_admin"(p_object_id, p_subject_type, p_max_leaf);
    END IF;
        IF p_relation = 'can_delete' THEN
        RETURN "public"."expand_organization_can_delete"(p_object_id, p_subject_type, p_max_leaf);
    END IF;
        IF p_relation = 'can_manage' THEN
        RETURN "public"."expand_organization_can_manage"(p_object_id, p_subject_type, p_max_leaf);
    END IF;
        IF p_relation = 'can_create_project' THEN
        RETURN "public"."expand_organization_can_create_project"(p_object_id, p_subject_type, p_max_leaf);
    END IF;
        IF p_relation = 'can_manage_members' THEN
        RETURN "public"."expand_organization_can_manage_members"(p_object_id, p_subject_type, p_max_leaf);
    END IF;
        IF p_relation = 'member' THEN
        RETURN "public"."expand_organization_member"(p_object_id, p_subject_type, p_max_leaf);
    END IF;
        IF p_relation = 'can_read' THEN
        RETURN "public"."expand_organization_can_read"(p_object_id, p_subject_type, p_max_leaf);
    END IF;
        RETURN jsonb_build_object('root', jsonb_build_object('name', (p_object_type || ':' || p_object_id || '#' || p_relation)) || jsonb_build_object('leaf', jsonb_build_object('users', jsonb_build_object('users', '[]'::jsonb))));
    END IF;
    IF p_object_type = 'project' THEN
        IF p_relation = 'maintainer' THEN
        RETURN "public"."expand_project_maintainer"(p_object_id, p_subject_type, p_max_leaf);
    END IF;
        IF p_relation = 'organization' THEN
        RETURN "public"."expand_project_organization"(p_object_id, p_subject_type, p_max_leaf);
    END IF;
        IF p_relation = 'can_delete' THEN
        RETURN "public"."expand_project_can_delete"(p_object_id, p_subject_type, p_max_leaf);
    END IF;
        IF p_relation = 'can_manage' THEN
        RETURN "public"."expand_project_can_manage"(p_object_id, p_subject_type, p_max_leaf);
    END IF;
        IF p_relation = 'can_write' THEN
        RETURN "public"."expand_project_can_write"(p_object_id, p_subject_type, p_max_leaf);
    END IF;
        IF p_relation = 'can_read' THEN
        RETURN "public"."expand_project_can_read"(p_object_id, p_subject_type, p_max_leaf);
    END IF;
        RETURN jsonb_build_object('root', jsonb_build_object('name', (p_object_type || ':' || p_object_id || '#' || p_relation)) || jsonb_build_object('leaf', jsonb_build_object('users', jsonb_build_object('users', '[]'::jsonb))));
    END IF;
    IF p_object_type = 'repository' THEN
        IF p_relation = 'project' THEN
        RETURN "public"."expand_repository_project"(p_object_id, p_subject_type, p_max_leaf);
    END IF;
        IF p_relation = 'public' THEN
        RETURN "public"."expand_repository_public"(p_object_id, p_subject_type, p_max_leaf);
    END IF;
        IF p_relation = 'can_delete' THEN
        RETURN "public"."expand_repository_can_delete"(p_object_id, p_subject_type, p_max_leaf);
    END IF;
        IF p_relation = 'can_write' THEN
        RETURN "public"."expand_repository_can_write"(p_object_id, p_subject_type, p_max_leaf);
    END IF;
        IF p_relation = 'can_read' THEN
        RETURN "public"."expand_repository_can_read"(p_object_id, p_subject_type, p_max_leaf);
    END IF;
        RETURN jsonb_build_object('root', jsonb_build_object('name', (p_object_type || ':' || p_object_id || '#' || p_relation)) || jsonb_build_object('leaf', jsonb_build_object('users', jsonb_build_object('users', '[]'::jsonb))));
    END IF;
    IF p_object_type = 'run' THEN
        IF p_relation = 'agent' THEN
        RETURN "public"."expand_run_agent"(p_object_id, p_subject_type, p_max_leaf);
    END IF;
        IF p_relation = 'can_cancel' THEN
        RETURN "public"."expand_run_can_cancel"(p_object_id, p_subject_type, p_max_leaf);
    END IF;
        IF p_relation = 'can_read' THEN
        RETURN "public"."expand_run_can_read"(p_object_id, p_subject_type, p_max_leaf);
    END IF;
        RETURN jsonb_build_object('root', jsonb_build_object('name', (p_object_type || ':' || p_object_id || '#' || p_relation)) || jsonb_build_object('leaf', jsonb_build_object('users', jsonb_build_object('users', '[]'::jsonb))));
    END IF;
    IF p_object_type = 'state_volume' THEN
        IF p_relation = 'agent' THEN
        RETURN "public"."expand_state_volume_agent"(p_object_id, p_subject_type, p_max_leaf);
    END IF;
        IF p_relation = 'can_manage' THEN
        RETURN "public"."expand_state_volume_can_manage"(p_object_id, p_subject_type, p_max_leaf);
    END IF;
        IF p_relation = 'can_restore' THEN
        RETURN "public"."expand_state_volume_can_restore"(p_object_id, p_subject_type, p_max_leaf);
    END IF;
        IF p_relation = 'can_attach' THEN
        RETURN "public"."expand_state_volume_can_attach"(p_object_id, p_subject_type, p_max_leaf);
    END IF;
        IF p_relation = 'can_read' THEN
        RETURN "public"."expand_state_volume_can_read"(p_object_id, p_subject_type, p_max_leaf);
    END IF;
        RETURN jsonb_build_object('root', jsonb_build_object('name', (p_object_type || ':' || p_object_id || '#' || p_relation)) || jsonb_build_object('leaf', jsonb_build_object('users', jsonb_build_object('users', '[]'::jsonb))));
    END IF;
    RETURN jsonb_build_object('root', jsonb_build_object('name', (p_object_type || ':' || p_object_id || '#' || p_relation)) || jsonb_build_object('leaf', jsonb_build_object('users', jsonb_build_object('users', '[]'::jsonb))));
END;
$$ LANGUAGE plpgsql STABLE;

-- Generated public dispatcher for expand_permission
-- Companion to list_accessible_subjects — returns an OpenFGA-shaped
-- UsersetTree JSONB describing who has the relation on the object.
-- Shallow by default: computed/TTU rewrites surface as unresolved
-- pointers (use Checker.ExpandRecursive client-side to chase).
CREATE OR REPLACE FUNCTION "public"."expand_permission"(
    p_object_type TEXT,
    p_object_id TEXT,
    p_relation TEXT,
    p_subject_type TEXT DEFAULT NULL,
    p_max_leaf INTEGER DEFAULT NULL
) RETURNS JSONB AS $$
    SELECT "public"."expand_permission_internal"(p_object_type, p_object_id, p_relation, p_subject_type, p_max_leaf);
$$ LANGUAGE sql STABLE;


-- ============================================================
-- List Dispatchers
-- ============================================================

-- Generated dispatcher for list_accessible_objects
-- Routes to specialized functions for all type/relation pairs
CREATE OR REPLACE FUNCTION "public"."list_accessible_objects"(
    p_subject_type TEXT,
    p_subject_id TEXT,
    p_relation TEXT,
    p_object_type TEXT,
    p_limit INT DEFAULT NULL,
    p_after TEXT DEFAULT NULL
) RETURNS TABLE (object_id TEXT, next_cursor TEXT) ROWS 100 AS $$
BEGIN
    -- Route to specialized functions for all type/relation pairs
    IF (p_object_type = 'agent' AND p_relation = 'project') THEN
        RETURN QUERY
        SELECT * FROM "public"."list_agent_project_obj"(p_subject_type, p_subject_id, p_limit, p_after);
        RETURN;
    END IF;
    IF (p_object_type = 'organization' AND p_relation = 'owner') THEN
        RETURN QUERY
        SELECT * FROM "public"."list_organization_owner_obj"(p_subject_type, p_subject_id, p_limit, p_after);
        RETURN;
    END IF;
    IF (p_object_type = 'project' AND p_relation = 'maintainer') THEN
        RETURN QUERY
        SELECT * FROM "public"."list_project_maintainer_obj"(p_subject_type, p_subject_id, p_limit, p_after);
        RETURN;
    END IF;
    IF (p_object_type = 'project' AND p_relation = 'organization') THEN
        RETURN QUERY
        SELECT * FROM "public"."list_project_organization_obj"(p_subject_type, p_subject_id, p_limit, p_after);
        RETURN;
    END IF;
    IF (p_object_type = 'repository' AND p_relation = 'project') THEN
        RETURN QUERY
        SELECT * FROM "public"."list_repository_project_obj"(p_subject_type, p_subject_id, p_limit, p_after);
        RETURN;
    END IF;
    IF (p_object_type = 'repository' AND p_relation = 'public') THEN
        RETURN QUERY
        SELECT * FROM "public"."list_repository_public_obj"(p_subject_type, p_subject_id, p_limit, p_after);
        RETURN;
    END IF;
    IF (p_object_type = 'run' AND p_relation = 'agent') THEN
        RETURN QUERY
        SELECT * FROM "public"."list_run_agent_obj"(p_subject_type, p_subject_id, p_limit, p_after);
        RETURN;
    END IF;
    IF (p_object_type = 'state_volume' AND p_relation = 'agent') THEN
        RETURN QUERY
        SELECT * FROM "public"."list_state_volume_agent_obj"(p_subject_type, p_subject_id, p_limit, p_after);
        RETURN;
    END IF;
    IF (p_object_type = 'organization' AND p_relation = 'admin') THEN
        RETURN QUERY
        SELECT * FROM "public"."list_organization_admin_obj"(p_subject_type, p_subject_id, p_limit, p_after);
        RETURN;
    END IF;
    IF (p_object_type = 'organization' AND p_relation = 'can_delete') THEN
        RETURN QUERY
        SELECT * FROM "public"."list_organization_can_delete_obj"(p_subject_type, p_subject_id, p_limit, p_after);
        RETURN;
    END IF;
    IF (p_object_type = 'organization' AND p_relation = 'can_manage') THEN
        RETURN QUERY
        SELECT * FROM "public"."list_organization_can_manage_obj"(p_subject_type, p_subject_id, p_limit, p_after);
        RETURN;
    END IF;
    IF (p_object_type = 'project' AND p_relation = 'can_delete') THEN
        RETURN QUERY
        SELECT * FROM "public"."list_project_can_delete_obj"(p_subject_type, p_subject_id, p_limit, p_after);
        RETURN;
    END IF;
    IF (p_object_type = 'project' AND p_relation = 'can_manage') THEN
        RETURN QUERY
        SELECT * FROM "public"."list_project_can_manage_obj"(p_subject_type, p_subject_id, p_limit, p_after);
        RETURN;
    END IF;
    IF (p_object_type = 'project' AND p_relation = 'can_write') THEN
        RETURN QUERY
        SELECT * FROM "public"."list_project_can_write_obj"(p_subject_type, p_subject_id, p_limit, p_after);
        RETURN;
    END IF;
    IF (p_object_type = 'organization' AND p_relation = 'can_create_project') THEN
        RETURN QUERY
        SELECT * FROM "public"."list_organization_can_create_project_obj"(p_subject_type, p_subject_id, p_limit, p_after);
        RETURN;
    END IF;
    IF (p_object_type = 'organization' AND p_relation = 'can_manage_members') THEN
        RETURN QUERY
        SELECT * FROM "public"."list_organization_can_manage_members_obj"(p_subject_type, p_subject_id, p_limit, p_after);
        RETURN;
    END IF;
    IF (p_object_type = 'organization' AND p_relation = 'member') THEN
        RETURN QUERY
        SELECT * FROM "public"."list_organization_member_obj"(p_subject_type, p_subject_id, p_limit, p_after);
        RETURN;
    END IF;
    IF (p_object_type = 'repository' AND p_relation = 'can_delete') THEN
        RETURN QUERY
        SELECT * FROM "public"."list_repository_can_delete_obj"(p_subject_type, p_subject_id, p_limit, p_after);
        RETURN;
    END IF;
    IF (p_object_type = 'agent' AND p_relation = 'can_manage') THEN
        RETURN QUERY
        SELECT * FROM "public"."list_agent_can_manage_obj"(p_subject_type, p_subject_id, p_limit, p_after);
        RETURN;
    END IF;
    IF (p_object_type = 'agent' AND p_relation = 'can_execute') THEN
        RETURN QUERY
        SELECT * FROM "public"."list_agent_can_execute_obj"(p_subject_type, p_subject_id, p_limit, p_after);
        RETURN;
    END IF;
    IF (p_object_type = 'repository' AND p_relation = 'can_write') THEN
        RETURN QUERY
        SELECT * FROM "public"."list_repository_can_write_obj"(p_subject_type, p_subject_id, p_limit, p_after);
        RETURN;
    END IF;
    IF (p_object_type = 'organization' AND p_relation = 'can_read') THEN
        RETURN QUERY
        SELECT * FROM "public"."list_organization_can_read_obj"(p_subject_type, p_subject_id, p_limit, p_after);
        RETURN;
    END IF;
    IF (p_object_type = 'state_volume' AND p_relation = 'can_manage') THEN
        RETURN QUERY
        SELECT * FROM "public"."list_state_volume_can_manage_obj"(p_subject_type, p_subject_id, p_limit, p_after);
        RETURN;
    END IF;
    IF (p_object_type = 'state_volume' AND p_relation = 'can_restore') THEN
        RETURN QUERY
        SELECT * FROM "public"."list_state_volume_can_restore_obj"(p_subject_type, p_subject_id, p_limit, p_after);
        RETURN;
    END IF;
    IF (p_object_type = 'run' AND p_relation = 'can_cancel') THEN
        RETURN QUERY
        SELECT * FROM "public"."list_run_can_cancel_obj"(p_subject_type, p_subject_id, p_limit, p_after);
        RETURN;
    END IF;
    IF (p_object_type = 'state_volume' AND p_relation = 'can_attach') THEN
        RETURN QUERY
        SELECT * FROM "public"."list_state_volume_can_attach_obj"(p_subject_type, p_subject_id, p_limit, p_after);
        RETURN;
    END IF;
    IF (p_object_type = 'project' AND p_relation = 'can_read') THEN
        RETURN QUERY
        SELECT * FROM "public"."list_project_can_read_obj"(p_subject_type, p_subject_id, p_limit, p_after);
        RETURN;
    END IF;
    IF (p_object_type = 'agent' AND p_relation = 'can_read') THEN
        RETURN QUERY
        SELECT * FROM "public"."list_agent_can_read_obj"(p_subject_type, p_subject_id, p_limit, p_after);
        RETURN;
    END IF;
    IF (p_object_type = 'repository' AND p_relation = 'can_read') THEN
        RETURN QUERY
        SELECT * FROM "public"."list_repository_can_read_obj"(p_subject_type, p_subject_id, p_limit, p_after);
        RETURN;
    END IF;
    IF (p_object_type = 'run' AND p_relation = 'can_read') THEN
        RETURN QUERY
        SELECT * FROM "public"."list_run_can_read_obj"(p_subject_type, p_subject_id, p_limit, p_after);
        RETURN;
    END IF;
    IF (p_object_type = 'state_volume' AND p_relation = 'can_read') THEN
        RETURN QUERY
        SELECT * FROM "public"."list_state_volume_can_read_obj"(p_subject_type, p_subject_id, p_limit, p_after);
        RETURN;
    END IF;
    -- Unknown type/relation pair - return empty result (relation not defined in model)
    -- This matches check_permission behavior for unknown relations (returns 0/denied)
    RETURN;
END;
$$ LANGUAGE plpgsql STABLE;

-- Generated dispatcher for list_accessible_subjects
-- Routes to specialized functions for all type/relation pairs
CREATE OR REPLACE FUNCTION "public"."list_accessible_subjects"(
    p_object_type TEXT,
    p_object_id TEXT,
    p_relation TEXT,
    p_subject_type TEXT,
    p_limit INT DEFAULT NULL,
    p_after TEXT DEFAULT NULL
) RETURNS TABLE (subject_id TEXT, next_cursor TEXT) ROWS 100 AS $$
BEGIN
    -- Route to specialized functions for all type/relation pairs
    IF (p_object_type = 'agent' AND p_relation = 'project') THEN
        RETURN QUERY
        SELECT * FROM "public"."list_agent_project_sub"(p_object_id, p_subject_type, p_limit, p_after);
        RETURN;
    END IF;
    IF (p_object_type = 'organization' AND p_relation = 'owner') THEN
        RETURN QUERY
        SELECT * FROM "public"."list_organization_owner_sub"(p_object_id, p_subject_type, p_limit, p_after);
        RETURN;
    END IF;
    IF (p_object_type = 'project' AND p_relation = 'maintainer') THEN
        RETURN QUERY
        SELECT * FROM "public"."list_project_maintainer_sub"(p_object_id, p_subject_type, p_limit, p_after);
        RETURN;
    END IF;
    IF (p_object_type = 'project' AND p_relation = 'organization') THEN
        RETURN QUERY
        SELECT * FROM "public"."list_project_organization_sub"(p_object_id, p_subject_type, p_limit, p_after);
        RETURN;
    END IF;
    IF (p_object_type = 'repository' AND p_relation = 'project') THEN
        RETURN QUERY
        SELECT * FROM "public"."list_repository_project_sub"(p_object_id, p_subject_type, p_limit, p_after);
        RETURN;
    END IF;
    IF (p_object_type = 'repository' AND p_relation = 'public') THEN
        RETURN QUERY
        SELECT * FROM "public"."list_repository_public_sub"(p_object_id, p_subject_type, p_limit, p_after);
        RETURN;
    END IF;
    IF (p_object_type = 'run' AND p_relation = 'agent') THEN
        RETURN QUERY
        SELECT * FROM "public"."list_run_agent_sub"(p_object_id, p_subject_type, p_limit, p_after);
        RETURN;
    END IF;
    IF (p_object_type = 'state_volume' AND p_relation = 'agent') THEN
        RETURN QUERY
        SELECT * FROM "public"."list_state_volume_agent_sub"(p_object_id, p_subject_type, p_limit, p_after);
        RETURN;
    END IF;
    IF (p_object_type = 'organization' AND p_relation = 'admin') THEN
        RETURN QUERY
        SELECT * FROM "public"."list_organization_admin_sub"(p_object_id, p_subject_type, p_limit, p_after);
        RETURN;
    END IF;
    IF (p_object_type = 'organization' AND p_relation = 'can_delete') THEN
        RETURN QUERY
        SELECT * FROM "public"."list_organization_can_delete_sub"(p_object_id, p_subject_type, p_limit, p_after);
        RETURN;
    END IF;
    IF (p_object_type = 'organization' AND p_relation = 'can_manage') THEN
        RETURN QUERY
        SELECT * FROM "public"."list_organization_can_manage_sub"(p_object_id, p_subject_type, p_limit, p_after);
        RETURN;
    END IF;
    IF (p_object_type = 'project' AND p_relation = 'can_delete') THEN
        RETURN QUERY
        SELECT * FROM "public"."list_project_can_delete_sub"(p_object_id, p_subject_type, p_limit, p_after);
        RETURN;
    END IF;
    IF (p_object_type = 'project' AND p_relation = 'can_manage') THEN
        RETURN QUERY
        SELECT * FROM "public"."list_project_can_manage_sub"(p_object_id, p_subject_type, p_limit, p_after);
        RETURN;
    END IF;
    IF (p_object_type = 'project' AND p_relation = 'can_write') THEN
        RETURN QUERY
        SELECT * FROM "public"."list_project_can_write_sub"(p_object_id, p_subject_type, p_limit, p_after);
        RETURN;
    END IF;
    IF (p_object_type = 'organization' AND p_relation = 'can_create_project') THEN
        RETURN QUERY
        SELECT * FROM "public"."list_organization_can_create_project_sub"(p_object_id, p_subject_type, p_limit, p_after);
        RETURN;
    END IF;
    IF (p_object_type = 'organization' AND p_relation = 'can_manage_members') THEN
        RETURN QUERY
        SELECT * FROM "public"."list_organization_can_manage_members_sub"(p_object_id, p_subject_type, p_limit, p_after);
        RETURN;
    END IF;
    IF (p_object_type = 'organization' AND p_relation = 'member') THEN
        RETURN QUERY
        SELECT * FROM "public"."list_organization_member_sub"(p_object_id, p_subject_type, p_limit, p_after);
        RETURN;
    END IF;
    IF (p_object_type = 'repository' AND p_relation = 'can_delete') THEN
        RETURN QUERY
        SELECT * FROM "public"."list_repository_can_delete_sub"(p_object_id, p_subject_type, p_limit, p_after);
        RETURN;
    END IF;
    IF (p_object_type = 'agent' AND p_relation = 'can_manage') THEN
        RETURN QUERY
        SELECT * FROM "public"."list_agent_can_manage_sub"(p_object_id, p_subject_type, p_limit, p_after);
        RETURN;
    END IF;
    IF (p_object_type = 'agent' AND p_relation = 'can_execute') THEN
        RETURN QUERY
        SELECT * FROM "public"."list_agent_can_execute_sub"(p_object_id, p_subject_type, p_limit, p_after);
        RETURN;
    END IF;
    IF (p_object_type = 'repository' AND p_relation = 'can_write') THEN
        RETURN QUERY
        SELECT * FROM "public"."list_repository_can_write_sub"(p_object_id, p_subject_type, p_limit, p_after);
        RETURN;
    END IF;
    IF (p_object_type = 'organization' AND p_relation = 'can_read') THEN
        RETURN QUERY
        SELECT * FROM "public"."list_organization_can_read_sub"(p_object_id, p_subject_type, p_limit, p_after);
        RETURN;
    END IF;
    IF (p_object_type = 'state_volume' AND p_relation = 'can_manage') THEN
        RETURN QUERY
        SELECT * FROM "public"."list_state_volume_can_manage_sub"(p_object_id, p_subject_type, p_limit, p_after);
        RETURN;
    END IF;
    IF (p_object_type = 'state_volume' AND p_relation = 'can_restore') THEN
        RETURN QUERY
        SELECT * FROM "public"."list_state_volume_can_restore_sub"(p_object_id, p_subject_type, p_limit, p_after);
        RETURN;
    END IF;
    IF (p_object_type = 'run' AND p_relation = 'can_cancel') THEN
        RETURN QUERY
        SELECT * FROM "public"."list_run_can_cancel_sub"(p_object_id, p_subject_type, p_limit, p_after);
        RETURN;
    END IF;
    IF (p_object_type = 'state_volume' AND p_relation = 'can_attach') THEN
        RETURN QUERY
        SELECT * FROM "public"."list_state_volume_can_attach_sub"(p_object_id, p_subject_type, p_limit, p_after);
        RETURN;
    END IF;
    IF (p_object_type = 'project' AND p_relation = 'can_read') THEN
        RETURN QUERY
        SELECT * FROM "public"."list_project_can_read_sub"(p_object_id, p_subject_type, p_limit, p_after);
        RETURN;
    END IF;
    IF (p_object_type = 'agent' AND p_relation = 'can_read') THEN
        RETURN QUERY
        SELECT * FROM "public"."list_agent_can_read_sub"(p_object_id, p_subject_type, p_limit, p_after);
        RETURN;
    END IF;
    IF (p_object_type = 'repository' AND p_relation = 'can_read') THEN
        RETURN QUERY
        SELECT * FROM "public"."list_repository_can_read_sub"(p_object_id, p_subject_type, p_limit, p_after);
        RETURN;
    END IF;
    IF (p_object_type = 'run' AND p_relation = 'can_read') THEN
        RETURN QUERY
        SELECT * FROM "public"."list_run_can_read_sub"(p_object_id, p_subject_type, p_limit, p_after);
        RETURN;
    END IF;
    IF (p_object_type = 'state_volume' AND p_relation = 'can_read') THEN
        RETURN QUERY
        SELECT * FROM "public"."list_state_volume_can_read_sub"(p_object_id, p_subject_type, p_limit, p_after);
        RETURN;
    END IF;
    -- Unknown type/relation pair - return empty result (relation not defined in model)
    -- This matches check_permission behavior for unknown relations (returns 0/denied)
    RETURN;
END;
$$ LANGUAGE plpgsql STABLE;

