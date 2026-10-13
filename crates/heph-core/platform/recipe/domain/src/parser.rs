use capability_domain::CapabilitySlotKey;
use release_domain::ContentHash;

use crate::{
    MAX_RECIPE_BYTES, RECIPE_CONTRACT_VERSION, RecipeError, RecipeManifest, ResourceDeclaration,
    errors::invalid, graph::validate_graph,
};

/// Syntax and graph-validated intent, not published-release approval or grants.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedRecipe {
    pub(super) manifest: RecipeManifest,
    pub(super) execution_order: Vec<CapabilitySlotKey>,
    canonical_json: Vec<u8>,
    hash: ContentHash,
}

impl ValidatedRecipe {
    /// Returns canonical immutable declaration intent.
    #[must_use]
    pub const fn manifest(&self) -> &RecipeManifest {
        &self.manifest
    }

    /// Returns deterministic JSON bytes used for immutable identity.
    #[must_use]
    pub fn canonical_json(&self) -> &[u8] {
        &self.canonical_json
    }

    /// Returns canonical TOML source for later reparsing and catalog revalidation.
    ///
    /// # Errors
    ///
    /// Returns an error if canonical declaration serialization unexpectedly fails.
    pub fn canonical_toml(&self) -> Result<String, RecipeError> {
        toml::to_string(&self.manifest).map_err(|_| RecipeError::Serialization)
    }

    /// Returns the canonical declaration hash.
    #[must_use]
    pub const fn hash(&self) -> ContentHash {
        self.hash
    }

    /// Rejects changed contents under an already recorded recipe identity/version.
    ///
    /// Callers enforce this check against authoritative persisted identity records.
    ///
    /// # Errors
    ///
    /// Returns an identity conflict when the same identity/version has changed.
    pub fn validate_identity(&self, previous: &Self) -> Result<(), RecipeError> {
        if self.manifest.recipe_id == previous.manifest.recipe_id
            && self.manifest.recipe_version == previous.manifest.recipe_version
            && self.hash != previous.hash
        {
            return Err(invalid("recipe_identity_conflict", "recipe_version"));
        }
        Ok(())
    }
}

/// Parses bounded TOML, validates declarations/references, and freezes canonical intent.
///
/// Release publication, actual resources, authorization, and provider effects are
/// deliberately separate. Use [`ValidatedRecipe::resolve`] with authoritative inputs.
///
/// # Errors
///
/// Rejects oversized/invalid syntax, unsupported versions, and invalid static graphs.
pub fn parse_recipe(source: &[u8]) -> Result<ValidatedRecipe, RecipeError> {
    if source.len() > MAX_RECIPE_BYTES {
        return Err(invalid("document_size_limit", "recipe"));
    }
    let text = std::str::from_utf8(source).map_err(|_| invalid("invalid_utf8", "recipe"))?;
    let mut manifest: RecipeManifest = toml::from_str(text).map_err(|error| {
        invalid(
            "invalid_toml",
            error.span().map_or_else(
                || "recipe".to_owned(),
                |span| format!("bytes {}..{}", span.start, span.end),
            ),
        )
    })?;
    if manifest.contract_version != RECIPE_CONTRACT_VERSION {
        return Err(invalid("unsupported_contract_version", "contract_version"));
    }
    let execution_order = validate_graph(&manifest)?;
    canonicalize(&mut manifest);
    let canonical_json = serde_json::to_vec(&manifest).map_err(|_| RecipeError::Serialization)?;
    let hash = ContentHash::digest(&canonical_json);
    Ok(ValidatedRecipe {
        manifest,
        execution_order,
        canonical_json,
        hash,
    })
}

fn canonicalize(manifest: &mut RecipeManifest) {
    manifest
        .inputs
        .sort_by(|left, right| left.name.cmp(&right.name));
    for input in &mut manifest.inputs {
        if let crate::InputType::Enum { values } = &mut input.value_type {
            values.sort();
        }
    }
    manifest
        .resources
        .sort_by(|left, right| left.name().cmp(right.name()));
    for resource in &mut manifest.resources {
        match resource {
            ResourceDeclaration::Volume(value) => value.depends_on.sort(),
            ResourceDeclaration::Instance(value) => {
                value.depends_on.sort();
                value
                    .volume_bindings
                    .sort_by(|left, right| left.slot.cmp(&right.slot));
            }
        }
    }
    manifest
        .outputs
        .sort_by(|left, right| left.name.cmp(&right.name));
}
