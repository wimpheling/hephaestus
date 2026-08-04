-- Authenticated users may inspect safe platform builder publication metadata
-- through the Builder Catalog. This does not grant any OCI registry pull
-- scope: token issuance remains a separate authorization boundary.
CREATE POLICY registry_namespaces_platform_catalog_select ON registry_namespaces
    FOR SELECT TO hephaestus_app USING (
        owner_kind = 'platform_builder'
        AND hephaestus_actor_id() IS NOT NULL
        AND current_setting('hephaestus.subject_type', true) = 'user'
    );

CREATE POLICY registry_publications_platform_catalog_select ON registry_publications
    FOR SELECT TO hephaestus_app USING (
        owner_kind = 'platform_builder'
        AND hephaestus_actor_id() IS NOT NULL
        AND current_setting('hephaestus.subject_type', true) = 'user'
    );
