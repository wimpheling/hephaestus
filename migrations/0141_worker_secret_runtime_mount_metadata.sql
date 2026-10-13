-- Ordinary runtime metadata contains opaque identifiers, not secret plaintext.
-- The worker adapter reads, materializes and marks destruction; it never deletes.
-- This is additive after the independently reviewed canonical migration 0140.
GRANT SELECT, INSERT, UPDATE ON public.secret_runtime_mounts TO hephaestus_worker;
