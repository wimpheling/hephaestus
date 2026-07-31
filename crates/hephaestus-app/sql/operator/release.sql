SELECT jsonb_build_object(
    'release_id', release.id,
    'state', release.state,
    'repository_id', release.repository_id,
    'source_commit', release.source_commit,
    'source_ref', release.source_ref,
    'build_request_id', release.build_request_id,
    'manifest_hash', encode(release.manifest_hash, 'hex'),
    'artifacts', (
        SELECT COALESCE(jsonb_agg(jsonb_build_object(
            'id', artifact.id, 'path', artifact.path, 'kind', artifact.kind,
            'size_bytes', artifact.size_bytes,
            'content_hash', encode(artifact.content_hash, 'hex')
        ) ORDER BY artifact.path), '[]'::jsonb)
        FROM release_artifacts artifact WHERE artifact.release_id = release.id
    ),
    'bindings', (
        SELECT COALESCE(jsonb_agg(to_jsonb(binding)), '[]'::jsonb)
        FROM release_provenance_inspection binding
        WHERE binding.release_id = release.id
    )
) FROM releases release WHERE release.id = $1
