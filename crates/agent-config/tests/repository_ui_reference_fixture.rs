//! Semantic validation of the checked-in Cooking reference UI fixture.

use agent_config::ui::static_resolution::{StaticArtifactCandidate, resolve_static_uis};
use agent_config::{parse, parse_repository_uis};
use release_domain::ui::UiScope;
use release_domain::{ArtifactKind, ArtifactPath, ReleaseArtifactId};
use uuid::Uuid;

const AGENT: &[u8] = include_bytes!("../../../examples/cooking/cooking-reference-ui/agent.toml");
const UI: &[u8] = include_bytes!("../../../examples/cooking/cooking-reference-ui/heph.ui.toml");
const HTML: &[u8] = include_bytes!("../../../examples/cooking/cooking-reference-ui/index.html");
const CSS: &[u8] = include_bytes!(
    "../../../examples/cooking/cooking-reference-ui/vendor/release-ui-kit/v1.0.0/dist/heph-ui-kit-v1.0.0.css"
);
const JS: &[u8] = include_bytes!(
    "../../../examples/cooking/cooking-reference-ui/vendor/release-ui-kit/v1.0.0/dist/heph-ui-kit-v1.0.0.js"
);

#[test]
fn reference_fixture_agent_and_static_ui_bind_the_declared_file_artifacts() {
    let parsed_agent = parse(AGENT);
    assert!(
        parsed_agent.diagnostics.is_empty(),
        "reference agent must validate: {:?}",
        parsed_agent.diagnostics
    );
    let agent = parsed_agent.config.expect("validated reference agent");
    let build = agent.build.expect("version-2 build configuration");
    let parsed_ui = parse_repository_uis(UI);
    assert!(
        parsed_ui.diagnostics.is_empty(),
        "reference UI must validate: {:?}",
        parsed_ui.diagnostics
    );
    let ui = parsed_ui.config.expect("validated reference UI");
    assert_eq!(ui.uis.len(), 3);
    for (declaration, (key, scope, route_base)) in ui.uis.iter().zip([
        ("release-reference", UiScope::Project, "reference"),
        (
            "release-reference-global",
            UiScope::Global,
            "reference-global",
        ),
        (
            "release-reference-repository",
            UiScope::Repository,
            "reference-repository",
        ),
    ]) {
        assert_eq!(declaration.key.as_str(), key);
        assert_eq!(declaration.scope, scope);
        assert_eq!(declaration.route_base.as_str(), route_base);
        assert!(matches!(
            declaration.content,
            agent_config::ui::UiContent::Static { .. }
        ));
    }

    let declared = build
        .artifacts
        .iter()
        .filter(|artifact| artifact.path.starts_with("dist/"))
        .collect::<Vec<_>>();
    assert_eq!(declared.len(), 3, "fixture has exactly three UI artifacts");
    let mut declared_paths = declared
        .iter()
        .map(|artifact| artifact.path.as_str())
        .collect::<Vec<_>>();
    declared_paths.sort_unstable();
    assert_eq!(
        declared_paths,
        [
            "dist/heph-ui-kit-v1.0.0.css",
            "dist/heph-ui-kit-v1.0.0.js",
            "dist/index.html",
        ]
    );
    let candidates = declared
        .iter()
        .enumerate()
        .map(|(index, artifact)| {
            let size_bytes = match artifact.path.as_str() {
                "dist/index.html" => HTML.len(),
                "dist/heph-ui-kit-v1.0.0.css" => CSS.len(),
                "dist/heph-ui-kit-v1.0.0.js" => JS.len(),
                _ => panic!("unexpected reference UI artifact path"),
            };
            StaticArtifactCandidate {
                path: ArtifactPath::parse(artifact.path.clone()).expect("declared artifact path"),
                id: ReleaseArtifactId::from_uuid(Uuid::from_u128(index as u128 + 1)),
                kind: match artifact.kind {
                    agent_config::BuildArtifactKind::File => ArtifactKind::File,
                    _ => panic!("reference UI artifact is not a file"),
                },
                media_type: artifact
                    .media_type
                    .clone()
                    .expect("reference UI artifact MIME"),
                size_bytes: size_bytes as u64,
            }
        })
        .collect::<Vec<_>>();
    let resolved = resolve_static_uis(&ui, &candidates).expect("fixture artifacts resolve");
    assert_eq!(resolved.uis.len(), 3);
    for (resolved_ui, key) in resolved.uis.iter().zip([
        "release-reference",
        "release-reference-global",
        "release-reference-repository",
    ]) {
        assert_resolved_static_ui(resolved_ui, key);
    }
}

fn assert_resolved_static_ui(
    resolved_ui: &agent_config::ui::static_resolution::ResolvedStaticUi,
    key: &str,
) {
    assert_eq!(resolved_ui.key.as_str(), key);
    assert_eq!(resolved_ui.files.len(), 3);
    let html = resolved_ui
        .files
        .iter()
        .find(|file| file.route.as_str() == "index.html")
        .expect("HTML route binding");
    assert_eq!(html.media_type.as_str(), "text/html");
    let css = resolved_ui
        .files
        .iter()
        .find(|file| file.route.as_str() == "heph-ui-kit-v1.0.0.css")
        .expect("CSS route binding");
    assert_eq!(css.media_type.as_str(), "text/css");
    let js = resolved_ui
        .files
        .iter()
        .find(|file| file.route.as_str() == "heph-ui-kit-v1.0.0.js")
        .expect("JavaScript route binding");
    assert_eq!(js.media_type.as_str(), "text/javascript");
}
