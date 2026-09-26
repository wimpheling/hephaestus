use super::*;

#[test]
fn retains_exact_ids_for_multiple_files() {
    let config = static_config(
        "[[uis.content.files]]\nroute = \"index.html\"\nartifact = \"dist/index.html\"\nmedia_type = \"text/html\"\n\n[[uis.content.files]]\nroute = \"app.js\"\nartifact = \"dist/app.js\"\nmedia_type = \"text/javascript\"\n",
    );
    let result = resolve_static_uis(
        &config,
        &[
            candidate("dist/index.html", 1, ArtifactKind::File, "text/html"),
            candidate("dist/app.js", 2, ArtifactKind::File, "text/javascript"),
        ],
    )
    .expect("static files resolve");
    // Parsing normalizes static files by route, so app.js precedes
    // index.html even though the candidate list uses the source order.
    assert_eq!(result.uis[0].files[0].artifact_id, id(2));
    assert_eq!(result.uis[0].files[1].artifact_id, id(1));
}

#[test]
fn missing_artifact_is_redacted_and_indexed() {
    let error = resolve_static_uis(
        &static_config(
            "[[uis.content.files]]\nroute = \"index.html\"\nartifact = \"dist/index.html\"\nmedia_type = \"text/html\"\n",
        ),
        &[],
    )
    .expect_err("missing candidate");
    assert_eq!(
        error,
        StaticResolutionError::MissingArtifact {
            ui_index: 0,
            file_index: 0,
        }
    );
    assert!(!error.to_string().contains("dist/index.html"));
    assert!(!error.to_string().contains("text/html"));
}

#[test]
fn non_file_kinds_are_rejected() {
    for kind in [
        ArtifactKind::Executable,
        ArtifactKind::Manifest,
        ArtifactKind::BuildLog,
    ] {
        let error = resolve_static_uis(
            &static_config(
                "[[uis.content.files]]\nroute = \"index.html\"\nartifact = \"dist/index.html\"\nmedia_type = \"text/html\"\n",
            ),
            &[candidate("dist/index.html", 1, kind, "text/html")],
        )
        .expect_err("non-file candidate");
        assert!(matches!(
            error,
            StaticResolutionError::WrongArtifactKind { .. }
        ));
    }
}

#[test]
fn mime_mismatch_including_default_octet_stream_is_rejected() {
    let error = resolve_static_uis(
        &static_config(
            "[[uis.content.files]]\nroute = \"index.html\"\nartifact = \"dist/index.html\"\nmedia_type = \"text/html\"\n",
        ),
        &[candidate(
            "dist/index.html",
            1,
            ArtifactKind::File,
            "application/octet-stream",
        )],
    )
    .expect_err("MIME mismatch");
    assert!(matches!(
        error,
        StaticResolutionError::MediaTypeMismatch { .. }
    ));
}

#[test]
fn per_file_size_bound_accepts_exact_limit_and_rejects_next_byte() {
    let config = static_config(
        "[[uis.content.files]]\nroute = \"index.html\"\nartifact = \"dist/index.html\"\nmedia_type = \"text/html\"\n",
    );
    resolve_static_uis(
        &config,
        &[candidate_with_size(
            "dist/index.html",
            1,
            ArtifactKind::File,
            "text/html",
            MAX_STATIC_UI_FILE_BYTES,
        )],
    )
    .expect("exact per-file limit is accepted");

    let error = resolve_static_uis(
        &config,
        &[candidate_with_size(
            "dist/index.html",
            1,
            ArtifactKind::File,
            "text/html",
            MAX_STATIC_UI_FILE_BYTES + 1,
        )],
    )
    .expect_err("the next byte exceeds the per-file limit");
    assert_eq!(
        error,
        StaticResolutionError::StaticArtifactTooLarge {
            ui_index: 0,
            file_index: 0,
            candidate_index: 0,
        }
    );
    assert!(!error.to_string().contains("16777217"));
}

#[test]
fn aggregate_size_bound_accepts_exact_limit_and_rejects_next_byte() {
    let exact_files = [
        "[[uis.content.files]]\nroute = \"index.html\"\nartifact = \"dist/index.html\"\nmedia_type = \"text/html\"",
        "[[uis.content.files]]\nroute = \"one.js\"\nartifact = \"dist/one.js\"\nmedia_type = \"text/javascript\"",
        "[[uis.content.files]]\nroute = \"two.js\"\nartifact = \"dist/two.js\"\nmedia_type = \"text/javascript\"",
        "[[uis.content.files]]\nroute = \"three.js\"\nartifact = \"dist/three.js\"\nmedia_type = \"text/javascript\"",
    ]
    .join("\n\n");
    let exact_config = static_config(&exact_files);
    let file_size = MAX_STATIC_UI_TOTAL_BYTES / 4;
    resolve_static_uis(
        &exact_config,
        &[
            candidate_with_size(
                "dist/index.html",
                1,
                ArtifactKind::File,
                "text/html",
                file_size,
            ),
            candidate_with_size(
                "dist/one.js",
                2,
                ArtifactKind::File,
                "text/javascript",
                file_size,
            ),
            candidate_with_size(
                "dist/two.js",
                3,
                ArtifactKind::File,
                "text/javascript",
                file_size,
            ),
            candidate_with_size(
                "dist/three.js",
                4,
                ArtifactKind::File,
                "text/javascript",
                file_size,
            ),
        ],
    )
    .expect("exact aggregate limit is accepted");

    let over_files = format!(
        "{exact_files}\n\n[[uis.content.files]]\nroute = \"four.js\"\nartifact = \"dist/four.js\"\nmedia_type = \"text/javascript\""
    );
    let error = resolve_static_uis(
        &static_config(&over_files),
        &[
            candidate_with_size(
                "dist/index.html",
                1,
                ArtifactKind::File,
                "text/html",
                file_size,
            ),
            candidate_with_size(
                "dist/one.js",
                2,
                ArtifactKind::File,
                "text/javascript",
                file_size,
            ),
            candidate_with_size(
                "dist/two.js",
                3,
                ArtifactKind::File,
                "text/javascript",
                file_size,
            ),
            candidate_with_size(
                "dist/three.js",
                4,
                ArtifactKind::File,
                "text/javascript",
                file_size,
            ),
            candidate_with_size("dist/four.js", 5, ArtifactKind::File, "text/javascript", 1),
        ],
    )
    .expect_err("the next aggregate byte is rejected");
    assert!(matches!(
        error,
        StaticResolutionError::StaticArtifactTotalTooLarge { .. }
    ));
}

#[test]
fn shared_ids_count_once_and_unreferenced_kinds_or_bytes_are_excluded() {
    let config = static_config(
        "[[uis.content.files]]\nroute = \"index.html\"\nartifact = \"dist/index.html\"\nmedia_type = \"text/html\"\n\n[[uis.content.files]]\nroute = \"app.js\"\nartifact = \"dist/shared-a.js\"\nmedia_type = \"text/javascript\"\n\n[[uis.content.files]]\nroute = \"vendor.js\"\nartifact = \"dist/shared-a.js\"\nmedia_type = \"text/javascript\"\n\n[[uis.content.files]]\nroute = \"feature.js\"\nartifact = \"dist/shared-b.js\"\nmedia_type = \"text/javascript\"\n\n[[uis.content.files]]\nroute = \"feature-vendor.js\"\nartifact = \"dist/shared-b.js\"\nmedia_type = \"text/javascript\"\n\n[[uis.content.files]]\nroute = \"runtime.js\"\nartifact = \"dist/shared-c.js\"\nmedia_type = \"text/javascript\"\n\n[[uis.content.files]]\nroute = \"runtime-vendor.js\"\nartifact = \"dist/shared-c.js\"\nmedia_type = \"text/javascript\"\n",
    );
    let shared_size = MAX_STATIC_UI_TOTAL_BYTES / 4;
    let result = resolve_static_uis(
        &config,
        &[
            candidate_with_size(
                "dist/index.html",
                1,
                ArtifactKind::File,
                "text/html",
                shared_size,
            ),
            candidate_with_size(
                "dist/shared-a.js",
                2,
                ArtifactKind::File,
                "text/javascript",
                shared_size,
            ),
            candidate_with_size(
                "dist/shared-b.js",
                3,
                ArtifactKind::File,
                "text/javascript",
                shared_size,
            ),
            candidate_with_size(
                "dist/shared-c.js",
                4,
                ArtifactKind::File,
                "text/javascript",
                shared_size,
            ),
            candidate_with_size(
                "dist/unused.bin",
                5,
                ArtifactKind::File,
                "application/octet-stream",
                MAX_STATIC_UI_FILE_BYTES + 1,
            ),
            candidate_with_size(
                "dist/tool",
                6,
                ArtifactKind::Executable,
                "application/octet-stream",
                MAX_STATIC_UI_TOTAL_BYTES,
            ),
            candidate_with_size(
                "dist/build.log",
                7,
                ArtifactKind::BuildLog,
                "text/plain",
                MAX_STATIC_UI_TOTAL_BYTES,
            ),
        ],
    )
    .expect("shared and unreferenced candidates do not exceed UI limits");
    assert_eq!(result.uis[0].files.len(), 7);
}

#[test]
fn duplicate_paths_and_ids_are_ambiguous() {
    let config = static_config(
        "[[uis.content.files]]\nroute = \"index.html\"\nartifact = \"dist/index.html\"\nmedia_type = \"text/html\"\n",
    );
    let error = resolve_static_uis(
        &config,
        &[
            candidate("dist/index.html", 1, ArtifactKind::File, "text/html"),
            candidate("dist/index.html", 2, ArtifactKind::File, "text/html"),
        ],
    )
    .expect_err("duplicate path");
    assert!(matches!(
        error,
        StaticResolutionError::DuplicateCandidatePath { .. }
    ));

    let error = resolve_static_uis(
        &config,
        &[
            candidate("dist/index.html", 1, ArtifactKind::File, "text/html"),
            candidate("dist/app.js", 1, ArtifactKind::File, "text/javascript"),
        ],
    )
    .expect_err("duplicate ID");
    assert!(matches!(
        error,
        StaticResolutionError::DuplicateCandidateId { .. }
    ));
}

#[test]
fn malformed_typed_manifest_fails_aggregate_validation() {
    let mut config = static_config(
        "[[uis.content.files]]\nroute = \"index.html\"\nartifact = \"dist/index.html\"\nmedia_type = \"text/html\"\n",
    );
    config.version = 99;
    let error = resolve_static_uis(&config, &[]).expect_err("invalid version");
    assert!(matches!(
        error,
        StaticResolutionError::InvalidManifest { .. }
    ));
}

#[test]
fn managed_uis_are_omitted() {
    let config = config(
        "version = 1\n\n[[uis]]\nkey = \"managed-ui\"\nscope = \"global\"\nlabel = \"Managed UI\"\nicon = \"app\"\npresentation = \"iframe\"\nroute_base = \"managed-ui\"\nui_kit_version = 1\ncache = \"no_store\"\n\n[uis.content]\nkind = \"managed_service\"\ngateway_name = \"service\"\nroute = \"/service\"\nentrypoint = \"index.html\"\n",
    );
    assert_eq!(
        resolve_static_uis(&config, &[]).expect("managed ignored"),
        ResolvedStaticUis { uis: Vec::new() }
    );
}
