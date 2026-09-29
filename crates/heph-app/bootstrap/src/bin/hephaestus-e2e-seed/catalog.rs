use std::error::Error;

pub async fn seed_builder_catalog(pool: &sqlx::PgPool) -> Result<(), Box<dyn Error>> {
    sqlx::query(
        "INSERT INTO oci_images
           (id, key, display_name, image_reference, toolchains, architectures,
            availability_state, provenance, platform_policy_version)
         VALUES
           ('20000000-0000-4000-8000-000000000001', 'fixture-root',
            'Browser fixture build root',
            'registry.browser.invalid/platform/images/fixture-root@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
            '[{\"name\":\"shell\",\"version\":\"fixture\"}]'::jsonb,
            ARRAY['x86_64'], 'available',
            '{\"source\":\"e2e-fixture\"}'::jsonb, 'e2e-fixture-v1')
         ON CONFLICT (key) DO UPDATE SET
           image_reference = EXCLUDED.image_reference,
           availability_state = EXCLUDED.availability_state,
           provenance = EXCLUDED.provenance,
           platform_policy_version = EXCLUDED.platform_policy_version",
    )
    .execute(pool)
    .await?;
    sqlx::query(
        "INSERT INTO registry_namespaces
           (id, repository_path, owner_kind, platform_image_key)
         VALUES
           ('20000000-0000-4000-8000-000000000002',
            'platform/images/fixture-root', 'platform_image', 'fixture-root')
         ON CONFLICT (repository_path) DO NOTHING",
    )
    .execute(pool)
    .await?;
    sqlx::query(
        "INSERT INTO registry_publications
           (id, namespace_id, owner_kind, platform_image_key, registry_authority,
            expected_digest, expected_media_type, expected_size, policy_version,
            state)
         VALUES
           ('20000000-0000-4000-8000-000000000003',
            '20000000-0000-4000-8000-000000000002', 'platform_image',
            'fixture-root', 'registry.browser.invalid',
            'sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
            'application/vnd.oci.image.index.v1+json', 1, 'e2e-fixture-v1',
            'pending')
         ON CONFLICT (namespace_id, registry_authority, expected_digest, policy_version)
         DO NOTHING",
    )
    .execute(pool)
    .await?;
    seed_fixture_publication_verification(pool).await
}

async fn seed_fixture_publication_verification(pool: &sqlx::PgPool) -> Result<(), Box<dyn Error>> {
    sqlx::query(
        "INSERT INTO registry_publication_platforms
           (publication_id, digest, size, media_type, operating_system, architecture)
         SELECT '20000000-0000-4000-8000-000000000003',
                'sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
                1, 'application/vnd.oci.image.manifest.v1+json', 'linux', 'x86_64'
           WHERE EXISTS (
               SELECT 1
                 FROM registry_publications
                WHERE id = '20000000-0000-4000-8000-000000000003'
                  AND state IN ('pending', 'publishing')
           )
         ON CONFLICT (publication_id, digest) DO NOTHING",
    )
    .execute(pool)
    .await?;
    sqlx::query(
        "INSERT INTO registry_publication_evidence
           (publication_id, kind, subject_digest, digest, size, media_type, artifact_type)
         SELECT '20000000-0000-4000-8000-000000000003', evidence.kind,
                evidence.subject_digest, evidence.digest, evidence.size,
                evidence.media_type, evidence.artifact_type
           FROM (VALUES
               ('sbom',
                'sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
                'sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',
                1, 'application/vnd.oci.artifact.manifest.v1+json',
                'application/vnd.cyclonedx+json'),
               ('provenance',
                'sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
                'sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc',
                1, 'application/vnd.oci.artifact.manifest.v1+json',
                'application/vnd.in-toto+json'),
               ('scan',
                'sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
                'sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd',
                1, 'application/vnd.oci.artifact.manifest.v1+json',
                'application/vnd.cyclonedx+json')
           ) AS evidence(kind, subject_digest, digest, size, media_type, artifact_type)
          WHERE EXISTS (
              SELECT 1
                FROM registry_publications
               WHERE id = '20000000-0000-4000-8000-000000000003'
                 AND state IN ('pending', 'publishing')
          )
         ON CONFLICT (publication_id, kind) DO NOTHING",
    )
    .execute(pool)
    .await?;
    sqlx::query(
        "UPDATE registry_publications
            SET state = 'verified', verified_at = now()
          WHERE id = '20000000-0000-4000-8000-000000000003'
            AND state IN ('pending', 'publishing')",
    )
    .execute(pool)
    .await?;
    sqlx::query(
        "UPDATE registry_publications
            SET state = 'approved', approved_at = now()
          WHERE id = '20000000-0000-4000-8000-000000000003'
            AND state = 'verified'",
    )
    .execute(pool)
    .await?;
    Ok(())
}
