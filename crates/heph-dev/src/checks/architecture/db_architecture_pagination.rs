use super::{Diagnostic, PAGINATION_RULE};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, serde::Deserialize)]
pub(super) struct PaginationFile {
    #[serde(default)]
    queries: Vec<PaginationContract>,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub(super) struct PaginationContract {
    pub(super) item: String,
    pub(super) query_index: usize,
    pub(super) order: Vec<String>,
    pub(super) cursor_keys: Vec<String>,
    pub(super) cursor_operator: String,
    pub(super) unique_tie_breaker: String,
    pub(super) unique_keys: Vec<String>,
    #[serde(default = "default_cursor_mode")]
    pub(super) cursor_mode: String,
}

fn default_cursor_mode() -> String {
    String::from("scalar")
}

#[derive(Clone, Default)]
pub(super) struct PaginationContracts {
    queries: BTreeMap<(String, usize), PaginationContract>,
}

#[derive(Default)]
pub(super) struct PaginationRegistry {
    contracts: BTreeMap<PathBuf, PaginationContracts>,
    seen_queries: BTreeMap<PathBuf, BTreeSet<(String, usize)>>,
}

impl PaginationContracts {
    pub(super) fn load(package_root: &Path, diagnostics: &mut Vec<Diagnostic>) -> Self {
        let path = package_root.join("pagination.toml");
        let Ok(source) = fs::read_to_string(&path) else {
            return Self::default();
        };
        let Ok(file) = toml::from_str::<PaginationFile>(&source) else {
            diagnostics.push(Diagnostic::new(
                PAGINATION_RULE,
                format!(
                    "pagination declaration {} is not valid TOML",
                    path.display()
                ),
            ));
            return Self::default();
        };
        let mut queries = BTreeMap::new();
        for contract in file.queries {
            let key = (contract.item.clone(), contract.query_index);
            if contract.query_index == 0 {
                diagnostics.push(Diagnostic::new(
                    PAGINATION_RULE,
                    format!(
                        "pagination declaration {}#{} has query_index 0; use a 1-based SQL query index",
                        path.display(), contract.item
                    ),
                ));
            }
            if contract.unique_keys.is_empty()
                || !contract
                    .unique_keys
                    .iter()
                    .any(|key| key == &contract.unique_tie_breaker)
            {
                diagnostics.push(Diagnostic::new(
                    PAGINATION_RULE,
                    format!(
                        "pagination declaration {}#{} must explicitly list `{}` in unique_keys",
                        path.display(),
                        contract.item,
                        contract.unique_tie_breaker
                    ),
                ));
            }
            if !matches!(
                contract.cursor_mode.as_str(),
                "scalar" | "tuple" | "uuid_row_lookup" | "stored_function"
            ) {
                diagnostics.push(Diagnostic::new(
                    PAGINATION_RULE,
                    format!(
                        "pagination declaration {}#{} uses unsupported cursor_mode `{}`",
                        path.display(),
                        contract.item,
                        contract.cursor_mode
                    ),
                ));
            }
            if queries.insert(key, contract).is_some() {
                diagnostics.push(Diagnostic::new(
                    PAGINATION_RULE,
                    format!(
                        "pagination declaration {} has a duplicate item/query_index pair",
                        path.display()
                    ),
                ));
            }
        }
        Self { queries }
    }

    pub(super) fn get(
        &self,
        item: Option<&str>,
        query_index: usize,
    ) -> Option<&PaginationContract> {
        item.and_then(|item| self.queries.get(&(item.to_owned(), query_index)))
    }

    pub(super) fn iter(&self) -> impl Iterator<Item = (&(String, usize), &PaginationContract)> {
        self.queries.iter()
    }
}

impl PaginationRegistry {
    pub(super) fn contracts_for(
        &mut self,
        package_root: &Path,
        diagnostics: &mut Vec<Diagnostic>,
    ) -> PaginationContracts {
        self.contracts
            .entry(package_root.to_path_buf())
            .or_insert_with(|| PaginationContracts::load(package_root, diagnostics))
            .clone()
    }

    pub(super) fn record_queries(
        &mut self,
        package_root: &Path,
        queries: BTreeSet<(String, usize)>,
    ) {
        self.seen_queries
            .entry(package_root.to_path_buf())
            .or_default()
            .extend(queries);
    }

    pub(super) fn validate_stale(&self, diagnostics: &mut Vec<Diagnostic>) {
        for (package_root, contracts) in &self.contracts {
            let seen = self.seen_queries.get(package_root);
            for ((item, query_index), _) in contracts.iter() {
                if seen.is_none_or(|queries| !queries.contains(&(item.clone(), *query_index))) {
                    diagnostics.push(Diagnostic::new(
                        PAGINATION_RULE,
                        format!(
                            "pagination declaration {}#{item} query {query_index} targets no SQLx query",
                            package_root.join("pagination.toml").display()
                        ),
                    ));
                }
            }
        }
    }
}
