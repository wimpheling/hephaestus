//! Shared fixtures for static UI resolution tests.

use super::{
    MAX_STATIC_UI_FILE_BYTES, MAX_STATIC_UI_TOTAL_BYTES, ResolvedStaticUis,
    StaticArtifactCandidate, StaticResolutionError, resolve_static_uis,
};
use crate::parse_repository_uis;
use release_domain::{ArtifactKind, ArtifactPath, ReleaseArtifactId};
use uuid::Uuid;

fn config(source: &str) -> crate::ui::RepositoryUisConfig {
    parse_repository_uis(source.as_bytes())
        .config
        .expect("test manifest is valid")
}

fn id(value: u128) -> ReleaseArtifactId {
    ReleaseArtifactId::from_uuid(Uuid::from_u128(value))
}

fn candidate(
    path: &str,
    value: u128,
    kind: ArtifactKind,
    media_type: &str,
) -> StaticArtifactCandidate {
    StaticArtifactCandidate {
        path: ArtifactPath::parse(path).expect("test path"),
        id: id(value),
        kind,
        media_type: media_type.to_owned(),
        size_bytes: 1,
    }
}

fn candidate_with_size(
    path: &str,
    value: u128,
    kind: ArtifactKind,
    media_type: &str,
    size_bytes: u64,
) -> StaticArtifactCandidate {
    StaticArtifactCandidate {
        path: ArtifactPath::parse(path).expect("test path"),
        id: id(value),
        kind,
        media_type: media_type.to_owned(),
        size_bytes,
    }
}

fn static_config(files: &str) -> crate::ui::RepositoryUisConfig {
    config(&format!(
        "version = 1\n\n[[uis]]\nkey = \"static-ui\"\nscope = \"global\"\nlabel = \"Static UI\"\nicon = \"app\"\npresentation = \"iframe\"\nroute_base = \"static-ui\"\nui_kit_version = 1\ncache = \"no_store\"\n\n[uis.content]\nkind = \"static\"\nentrypoint = \"index.html\"\n\n{files}"
    ))
}

mod cases;
