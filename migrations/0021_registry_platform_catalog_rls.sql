-- Authenticated users may inspect safe platform image publication metadata
-- through the OCI image catalog. This does not grant any OCI registry pull
-- scope: token issuance remains a separate authorization boundary.
CREATE POLICY registry_namespaces_image_catalog_select ON registry_namespaces
    FOR SELECT TO hephaestus_app USING (
        owner_kind = 'platform_image'
        AND hephaestus_actor_id() IS NOT NULL
        AND current_setting('hephaestus.subject_type', true) = 'user'
    );

CREATE POLICY registry_publications_image_catalog_select ON registry_publications
    FOR SELECT TO hephaestus_app USING (
        owner_kind = 'platform_image'
        AND hephaestus_actor_id() IS NOT NULL
        AND current_setting('hephaestus.subject_type', true) = 'user'
    );
