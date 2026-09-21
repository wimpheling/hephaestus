use super::{
    MAX_REPOSITORY_UIS, MAX_UI_APIS, MAX_UI_FILES, REPOSITORY_UIS_VERSION, RepositoryUiConfig,
    RepositoryUisConfig, UI_KIT_VERSION, UiContent,
};
use crate::Diagnostic;
use release_domain::ui::{UiRepositoryGitAccess, UiRoutePath, UiScope};
use std::collections::HashSet;

pub(super) fn validate(config: &RepositoryUisConfig) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    if config.version != REPOSITORY_UIS_VERSION {
        diagnostic(
            &mut diagnostics,
            "unsupported_repository_uis_version",
            "version",
            format!(
                "repository UI version {} is unsupported; expected {REPOSITORY_UIS_VERSION}",
                config.version
            ),
        );
        return diagnostics;
    }
    if config.uis.len() > MAX_REPOSITORY_UIS {
        diagnostic(
            &mut diagnostics,
            "too_many_repository_uis",
            "uis",
            format!("a repository may define at most {MAX_REPOSITORY_UIS} UIs"),
        );
        return diagnostics;
    }
    let mut keys = HashSet::new();
    for (index, ui) in config.uis.iter().enumerate() {
        let path = format!("uis[{index}]");
        if !keys.insert(&ui.key) {
            diagnostic(
                &mut diagnostics,
                "duplicate_repository_ui_key",
                format!("{path}.key"),
                "UI keys must be unique within a repository",
            );
        }
        if ui.ui_kit_version != UI_KIT_VERSION {
            diagnostic(
                &mut diagnostics,
                "unsupported_repository_ui_kit_version",
                format!("{path}.ui_kit_version"),
                format!(
                    "UI kit version {} is unsupported; expected {UI_KIT_VERSION}",
                    ui.ui_kit_version
                ),
            );
        }
        validate_scope_duplicates(&mut diagnostics, config, index, ui);
        if !matches!(ui.scope, UiScope::Repository)
            && !matches!(ui.repository_git_access, UiRepositoryGitAccess::None)
        {
            diagnostic(
                &mut diagnostics,
                "repository_git_access_requires_repository_scope",
                format!("{path}.repository_git_access"),
                "generic repository Git access requires a repository-scoped UI",
            );
        }
        validate_api_bindings(&mut diagnostics, index, ui);
        validate_content(&mut diagnostics, index, ui);
    }
    diagnostics.extend(route_collision_diagnostics(config));
    diagnostics
}

/// Finds platform-reserved and GET/HEAD serving-route collisions.
pub(super) fn route_collision_diagnostics(config: &RepositoryUisConfig) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    for (ui_index, ui) in config.uis.iter().enumerate() {
        let ui_path = format!("uis[{ui_index}]");
        if reserved_relative_route(ui.route_base.as_str()) {
            diagnostic(
                &mut diagnostics,
                "reserved_repository_ui_route_namespace",
                format!("{ui_path}.route_base"),
                "UI route bases under _heph are reserved for platform bootstrap routes",
            );
        }
        for (api_index, api) in ui.apis.iter().enumerate() {
            let api_path = format!("{ui_path}.apis[{api_index}].route");
            if reserved_absolute_route(api.route.as_str()) {
                diagnostic(
                    &mut diagnostics,
                    "reserved_repository_ui_route_namespace",
                    api_path.clone(),
                    "UI API routes under /_heph are reserved for platform bootstrap routes",
                );
            }
            if !matches!(
                api.method,
                gateway_domain::HttpMethod::Get | gateway_domain::HttpMethod::Head
            ) {
                continue;
            }
            match &ui.content {
                UiContent::Static { entrypoint, files } => {
                    if static_served_paths(ui.route_base.as_str(), entrypoint.as_str(), files)
                        .contains(api.route.as_str())
                    {
                        diagnostic(
                            &mut diagnostics,
                            "repository_ui_api_static_route_collision",
                            api_path,
                            "GET and HEAD UI APIs cannot collide with a served static document or file",
                        );
                    }
                }
                UiContent::ManagedService { .. }
                    if managed_served_path(ui.route_base.as_str(), api.route.as_str()) =>
                {
                    diagnostic(
                        &mut diagnostics,
                        "repository_ui_api_managed_route_collision",
                        api_path,
                        "GET and HEAD UI APIs cannot collide with a managed UI document or descendant",
                    );
                }
                UiContent::ManagedService { .. } => {}
            }
        }
    }
    diagnostics
}

fn static_served_paths(
    route_base: &str,
    entrypoint: &str,
    files: &[super::UiStaticFile],
) -> HashSet<String> {
    let mut paths = HashSet::new();
    for file in files {
        paths.insert(format!("/{route_base}/{}", file.route.as_str()));
        if file.route.as_str() == entrypoint {
            paths.insert(format!("/{route_base}"));
        }
    }
    paths
}

fn managed_served_path(route_base: &str, api_route: &str) -> bool {
    let base = format!("/{route_base}");
    api_route == base
        || api_route
            .strip_prefix(&base)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

fn reserved_relative_route(route: &str) -> bool {
    route == "_heph" || route.starts_with("_heph/")
}

fn reserved_absolute_route(route: &str) -> bool {
    route == "/_heph" || route.starts_with("/_heph/")
}

pub(super) fn normalized(mut config: RepositoryUisConfig) -> RepositoryUisConfig {
    for ui in &mut config.uis {
        ui.apis
            .sort_unstable_by(|left, right| left.key.cmp(&right.key));
        if let UiContent::Static { files, .. } = &mut ui.content {
            files.sort_unstable_by(|left, right| left.route.cmp(&right.route));
        }
    }
    config
        .uis
        .sort_unstable_by(|left, right| left.key.cmp(&right.key));
    config
}

fn validate_scope_duplicates(
    diagnostics: &mut Vec<Diagnostic>,
    config: &RepositoryUisConfig,
    index: usize,
    ui: &RepositoryUiConfig,
) {
    for (other_index, other) in config.uis.iter().enumerate().take(index) {
        if other.scope != ui.scope {
            continue;
        }
        if other.label == ui.label {
            diagnostic(
                diagnostics,
                "duplicate_repository_ui_label",
                format!("uis[{index}].label"),
                "UI labels must be unique within a scope",
            );
        }
        if route_overlaps(&other.route_base, &ui.route_base) {
            diagnostic(
                diagnostics,
                "conflicting_repository_ui_route_base",
                format!("uis[{index}].route_base"),
                format!("UI route base conflicts with uis[{other_index}] in the same scope"),
            );
        }
    }
}

fn route_overlaps(left: &UiRoutePath, right: &UiRoutePath) -> bool {
    left == right
        || right
            .as_str()
            .strip_prefix(left.as_str())
            .is_some_and(|suffix| suffix.starts_with('/'))
        || left
            .as_str()
            .strip_prefix(right.as_str())
            .is_some_and(|suffix| suffix.starts_with('/'))
}

fn validate_api_bindings(diagnostics: &mut Vec<Diagnostic>, index: usize, ui: &RepositoryUiConfig) {
    if ui.apis.len() > MAX_UI_APIS {
        diagnostic(
            diagnostics,
            "too_many_repository_ui_apis",
            format!("uis[{index}].apis"),
            format!("a UI may declare at most {MAX_UI_APIS} APIs"),
        );
        return;
    }
    let mut keys = HashSet::new();
    for (api_index, api) in ui.apis.iter().enumerate() {
        if !keys.insert(&api.key) {
            diagnostic(
                diagnostics,
                "duplicate_repository_ui_api_key",
                format!("uis[{index}].apis[{api_index}].key"),
                "API keys must be unique within a UI",
            );
        }
        if !strict_gateway_route(&api.route) {
            diagnostic(
                diagnostics,
                "invalid_repository_ui_api_route",
                format!("uis[{index}].apis[{api_index}].route"),
                "API routes must be strict absolute gateway paths without ambiguous separators",
            );
        }
    }
}

fn validate_content(diagnostics: &mut Vec<Diagnostic>, index: usize, ui: &RepositoryUiConfig) {
    match &ui.content {
        UiContent::Static { entrypoint, files } => {
            if files.is_empty() || files.len() > MAX_UI_FILES {
                diagnostic(
                    diagnostics,
                    "invalid_repository_ui_static_files",
                    format!("uis[{index}].content.files"),
                    format!("static UIs must declare 1 to {MAX_UI_FILES} files"),
                );
                return;
            }
            let mut routes = HashSet::new();
            for (file_index, file) in files.iter().enumerate() {
                if !routes.insert(&file.route) {
                    diagnostic(
                        diagnostics,
                        "duplicate_repository_ui_static_route",
                        format!("uis[{index}].content.files[{file_index}].route"),
                        "static file routes must be unique within a UI",
                    );
                }
            }
            let has_html_entrypoint = files.iter().any(|file| {
                file.route == *entrypoint
                    && file.media_type == release_domain::ui::UiMediaType::TextHtml
            });
            if !has_html_entrypoint {
                diagnostic(
                    diagnostics,
                    "invalid_repository_ui_entrypoint",
                    format!("uis[{index}].content.entrypoint"),
                    "the entrypoint must match one declared text/html static file",
                );
            }
        }
        UiContent::ManagedService { route, .. } => {
            if !strict_gateway_route(route) {
                diagnostic(
                    diagnostics,
                    "invalid_repository_ui_service_route",
                    format!("uis[{index}].content.route"),
                    "service routes must be strict absolute gateway paths without ambiguous separators",
                );
            }
        }
    }
}

fn strict_gateway_route(route: &gateway_domain::RoutePath) -> bool {
    route
        .as_str()
        .strip_prefix('/')
        .is_some_and(|relative| UiRoutePath::parse(relative.to_owned()).is_ok())
}

fn diagnostic(
    diagnostics: &mut Vec<Diagnostic>,
    code: impl Into<String>,
    path: impl Into<String>,
    message: impl Into<String>,
) {
    diagnostics.push(Diagnostic {
        code: code.into(),
        path: Some(path.into()),
        message: message.into(),
    });
}
