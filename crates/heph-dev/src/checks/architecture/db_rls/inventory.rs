//! Application-role pool inventory and source discovery data.

use super::{Diagnostic, RULE};
use std::path::Path;
use syn::File;

#[derive(Clone, Copy)]
pub(super) struct PoolBinding {
    pub(super) source: &'static str,
    pub(super) owner: &'static str,
    pub(super) field: &'static str,
}

#[derive(Clone, Copy)]
pub(super) struct PoolField {
    pub(super) source: &'static str,
    pub(super) owner: &'static str,
    pub(super) field: &'static str,
    pub(super) application_role: bool,
}

// These are the exact application-role fields established by the app
// composition. Generic `pool` fields are listed explicitly rather than
// treating every PostgreSQL pool as an application-role pool.
pub(super) const PRODUCTION_BINDINGS: [PoolBinding; 6] = [
    PoolBinding {
        source: "crates/heph-core/identity/postgres/src/session.rs",
        owner: "PostgresBrowserSessionStore",
        field: "application_pool",
    },
    PoolBinding {
        source: "crates/heph-core/forge/release/postgres/src/ui_browser.rs",
        owner: "PgUiBrowserSessionStore",
        field: "app_pool",
    },
    PoolBinding {
        source: "crates/heph-core/forge/release/postgres/src/ui_browser_resources.rs",
        owner: "PgUiGenerationHostResolver",
        field: "app_pool",
    },
    PoolBinding {
        source: "crates/heph-core/forge/release/postgres/src/ui_browser_resources.rs",
        owner: "PgUiBrowserServingStore",
        field: "app_pool",
    },
    PoolBinding {
        source: "crates/heph-core/forge/release/postgres/src/ui_installation_navigation.rs",
        owner: "PgUiInstallationNavigator",
        field: "pool",
    },
    PoolBinding {
        source: "crates/heph-core/gateway/postgres/src/service_log_reader.rs",
        owner: "PostgresGatewayServiceLogReader",
        field: "pool",
    },
];

pub(super) const PRODUCTION_POOL_FIELDS: [PoolField; 8] = [
    PoolField {
        source: "crates/heph-core/identity/postgres/src/session.rs",
        owner: "PostgresBrowserSessionStore",
        field: "worker_pool",
        application_role: false,
    },
    PoolField {
        source: "crates/heph-core/identity/postgres/src/session.rs",
        owner: "PostgresBrowserSessionStore",
        field: "application_pool",
        application_role: true,
    },
    PoolField {
        source: "crates/heph-core/forge/release/postgres/src/ui_browser.rs",
        owner: "PgUiBrowserSessionStore",
        field: "worker_pool",
        application_role: false,
    },
    PoolField {
        source: "crates/heph-core/forge/release/postgres/src/ui_browser.rs",
        owner: "PgUiBrowserSessionStore",
        field: "app_pool",
        application_role: true,
    },
    PoolField {
        source: "crates/heph-core/forge/release/postgres/src/ui_browser_resources.rs",
        owner: "PgUiGenerationHostResolver",
        field: "app_pool",
        application_role: true,
    },
    PoolField {
        source: "crates/heph-core/forge/release/postgres/src/ui_browser_resources.rs",
        owner: "PgUiBrowserServingStore",
        field: "app_pool",
        application_role: true,
    },
    PoolField {
        source: "crates/heph-core/forge/release/postgres/src/ui_installation_navigation.rs",
        owner: "PgUiInstallationNavigator",
        field: "pool",
        application_role: true,
    },
    PoolField {
        source: "crates/heph-core/gateway/postgres/src/service_log_reader.rs",
        owner: "PostgresGatewayServiceLogReader",
        field: "pool",
        application_role: true,
    },
];

pub(super) const PRODUCTION_SOURCES: [&str; 5] = [
    "crates/heph-core/identity/postgres/src/session.rs",
    "crates/heph-core/forge/release/postgres/src/ui_browser.rs",
    "crates/heph-core/forge/release/postgres/src/ui_browser_resources.rs",
    "crates/heph-core/forge/release/postgres/src/ui_installation_navigation.rs",
    "crates/heph-core/gateway/postgres/src/service_log_reader.rs",
];

pub(super) fn validate_pool_inventory(
    file: &File,
    relative: &Path,
    inventory: &[PoolField],
    bindings: &[PoolBinding],
    diagnostics: &mut Vec<Diagnostic>,
) {
    for binding in inventory
        .iter()
        .filter(|entry| entry.application_role && entry.source == relative.to_string_lossy())
    {
        if !bindings
            .iter()
            .any(|entry| entry.owner == binding.owner && entry.field == binding.field)
        {
            diagnostics.push(Diagnostic::new(
                RULE,
                format!(
                    "{}::{}::{} is missing from the application-role pool inventory",
                    binding.source, binding.owner, binding.field
                ),
            ));
        }
    }
    for item in &file.items {
        let syn::Item::Struct(item) = item else {
            continue;
        };
        let owner = item.ident.to_string();
        let syn::Fields::Named(fields) = &item.fields else {
            continue;
        };
        for field in fields
            .named
            .iter()
            .filter(|field| is_pg_pool_type(&field.ty))
        {
            let Some(name) = field.ident.as_ref().map(ToString::to_string) else {
                continue;
            };
            if !inventory.iter().any(|entry| {
                entry.source == relative.to_string_lossy()
                    && entry.owner == owner
                    && entry.field == name
            }) {
                diagnostics.push(Diagnostic::new(
                    RULE,
                    format!(
                        "{}::{owner}::{name} is an unclassified PostgreSQL pool field; declare its role explicitly",
                        relative.display()
                    ),
                ));
            }
        }
    }
}

fn is_pg_pool_type(ty: &syn::Type) -> bool {
    let syn::Type::Path(path) = ty else {
        return false;
    };
    path.path
        .segments
        .last()
        .is_some_and(|segment| segment.ident == "PgPool")
}
