-- Protect the existing private recipe trigger hooks from attachment to
-- caller-owned tables. The captured hook identities remain trusted internal
-- trigger entry points; this migration grants no outside execution capability.
-- SQLx applies this migration and its journal row in the same transaction.

REVOKE ALL ON FUNCTION public.advance_owned_observation_head() FROM PUBLIC;
REVOKE ALL ON FUNCTION public.guard_owned_observation_head() FROM PUBLIC;
REVOKE ALL ON FUNCTION public.guard_owned_partial_receipt() FROM PUBLIC;
REVOKE ALL ON FUNCTION public.guard_recipe_owned_volume_preparation() FROM PUBLIC;
REVOKE ALL ON FUNCTION public.reject_instance_activation_rewrite() FROM PUBLIC;
REVOKE ALL ON FUNCTION public.reject_retired_owned_birth_progress() FROM PUBLIC;
REVOKE ALL ON FUNCTION public.verify_partial_birth_retirement_commit() FROM PUBLIC;

-- Resolve the six SECURITY DEFINER hooks against trusted catalog/public tables
-- before any temporary relation. Function bodies, owners and captured scopes
-- remain unchanged; this does not harden every SECURITY DEFINER function.

ALTER FUNCTION public.advance_owned_observation_head() SET search_path TO pg_catalog, public, pg_temp;
ALTER FUNCTION public.guard_owned_observation_head() SET search_path TO pg_catalog, public, pg_temp;
ALTER FUNCTION public.guard_owned_partial_receipt() SET search_path TO pg_catalog, public, pg_temp;
ALTER FUNCTION public.guard_recipe_owned_volume_preparation() SET search_path TO pg_catalog, public, pg_temp;
ALTER FUNCTION public.reject_retired_owned_birth_progress() SET search_path TO pg_catalog, public, pg_temp;
ALTER FUNCTION public.verify_partial_birth_retirement_commit() SET search_path TO pg_catalog, public, pg_temp;
