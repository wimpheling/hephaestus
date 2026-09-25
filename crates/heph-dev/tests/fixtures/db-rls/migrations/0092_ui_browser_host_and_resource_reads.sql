CREATE FUNCTION resolve_active_ui_generation_host(uuid)
RETURNS TABLE (generation_id uuid)
LANGUAGE sql
SECURITY DEFINER
AS $$ SELECT $1 $$;
