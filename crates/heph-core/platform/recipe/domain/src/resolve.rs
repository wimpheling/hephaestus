use std::collections::{BTreeMap, BTreeSet};

use capability_domain::CapabilitySlotKey;
use release_domain::{ParameterDocument, ParameterName, ParameterValue};
use runtime_types::VolumeId;
use volume_domain::MAX_VOLUME_CAPACITY_BYTES;

use crate::{
    MAX_RECIPE_ITEMS, RecipeError, ReleaseCatalogEntry, ResolvedInstance, ResolvedRecipe,
    ResolvedResource, ResolvedVolume, ResourceDeclaration, ValidatedRecipe, ValueReference,
    VolumeSource,
    bindings::validate_bindings,
    errors::invalid,
    inputs::{bounded_value, valid_parameter},
};

/// Exact externally bound volume metadata supplied by an authoritative application.
///
/// The application separately verifies ownership, live authorization, provider
/// capacity, and attachment safety. This view conveys no authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExternalVolume {
    /// Exact stable external resource identity.
    pub id: VolumeId,
    /// Authoritatively observed bounded capacity.
    pub capacity_bytes: u64,
}

impl ValidatedRecipe {
    /// Resolves ordinary inputs, exact external resources, and published release compatibility.
    ///
    /// Catalog/external views must come from authoritative application adapters.
    /// This produces immutable intent, never live grants or provider effects.
    ///
    /// # Errors
    ///
    /// Rejects input/default mismatches, external-binding aliases, unpublished or
    /// mismatched pins, unsupported requirements, and invalid release-slot bindings.
    pub fn resolve(
        &self,
        supplied: &BTreeMap<ParameterName, ParameterValue>,
        external: &BTreeMap<CapabilitySlotKey, ExternalVolume>,
        catalog: &[ReleaseCatalogEntry],
    ) -> Result<ResolvedRecipe, RecipeError> {
        if supplied.len() > MAX_RECIPE_ITEMS || external.len() > MAX_RECIPE_ITEMS {
            return Err(invalid("resolution_item_limit", "inputs"));
        }
        if supplied.values().any(|value| !bounded_value(value)) {
            return Err(invalid("input_value_limit", "inputs"));
        }
        let declarations = self
            .manifest
            .inputs
            .iter()
            .map(crate::InputDeclaration::declaration)
            .collect::<Vec<_>>();
        let inputs = ParameterDocument::resolve(&declarations, supplied)
            .map_err(|_| invalid("invalid_supplied_inputs", "inputs"))?;
        let mut resources = resolve_volumes(&self.manifest.resources, inputs.values(), external)?;
        resolve_instances(
            &self.manifest.resources,
            inputs.values(),
            catalog,
            &mut resources,
        )?;
        let outputs = self
            .manifest
            .outputs
            .iter()
            .map(|output| {
                let value = match &output.value {
                    ValueReference::Resource { .. } => output.value.clone(),
                    reference => ValueReference::Literal {
                        value: resolve_value(reference, inputs.values())?,
                    },
                };
                Ok((output.name.clone(), value))
            })
            .collect::<Result<_, RecipeError>>()?;
        Ok(ResolvedRecipe {
            contract_version: self.manifest.contract_version,
            recipe_id: self.manifest.recipe_id.clone(),
            recipe_version: self.manifest.recipe_version.clone(),
            declaration_hash: self.hash(),
            inputs: inputs.values().clone(),
            resources,
            execution_order: self.execution_order.clone(),
            outputs,
        })
    }
}

fn resolve_volumes(
    declarations: &[ResourceDeclaration],
    inputs: &BTreeMap<ParameterName, ParameterValue>,
    external: &BTreeMap<CapabilitySlotKey, ExternalVolume>,
) -> Result<BTreeMap<CapabilitySlotKey, ResolvedResource>, RecipeError> {
    let mut resources = BTreeMap::new();
    let mut used_external = BTreeSet::new();
    let mut external_ids = BTreeSet::new();
    for resource in declarations {
        let ResourceDeclaration::Volume(volume) = resource else {
            continue;
        };
        let (capacity_bytes, external_id) = match &volume.source {
            VolumeSource::Created { capacity_bytes } => {
                let ParameterValue::Integer(value) = resolve_value(capacity_bytes, inputs)? else {
                    return Err(invalid("invalid_capacity", volume.name.as_str()));
                };
                let capacity = u64::try_from(value)
                    .map_err(|_| invalid("invalid_capacity", volume.name.as_str()))?;
                (capacity, None)
            }
            VolumeSource::External {} => {
                let binding = external
                    .get(&volume.name)
                    .ok_or_else(|| invalid("external_binding_missing", volume.name.as_str()))?;
                if binding.id.as_uuid().is_nil() || !external_ids.insert(binding.id) {
                    return Err(invalid("external_resource_alias", volume.name.as_str()));
                }
                used_external.insert(&volume.name);
                (binding.capacity_bytes, Some(binding.id))
            }
        };
        if capacity_bytes == 0 || capacity_bytes > MAX_VOLUME_CAPACITY_BYTES {
            return Err(invalid("invalid_capacity", volume.name.as_str()));
        }
        resources.insert(
            volume.name.clone(),
            ResolvedResource::Volume(ResolvedVolume {
                dependencies: volume.depends_on.clone(),
                capacity_bytes,
                external_id,
                removal: volume.removal,
            }),
        );
    }
    if external.keys().any(|name| !used_external.contains(name)) {
        return Err(invalid("undeclared_external_binding", "external"));
    }
    Ok(resources)
}

fn resolve_instances(
    declarations: &[ResourceDeclaration],
    inputs: &BTreeMap<ParameterName, ParameterValue>,
    catalog: &[ReleaseCatalogEntry],
    resources: &mut BTreeMap<CapabilitySlotKey, ResolvedResource>,
) -> Result<(), RecipeError> {
    if catalog.len() > MAX_RECIPE_ITEMS {
        return Err(invalid("catalog_item_limit", "catalog"));
    }
    let mut release_ids = BTreeSet::new();
    for entry in catalog {
        if !release_ids.insert(entry.pin.release_agent_id) {
            return Err(invalid("duplicate_catalog_export", "catalog"));
        }
    }
    for resource in declarations {
        let ResourceDeclaration::Instance(instance) = resource else {
            continue;
        };
        let release = catalog
            .iter()
            .find(|entry| entry.pin == instance.release)
            .ok_or_else(|| invalid("release_pin_not_in_catalog", instance.name.as_str()))?;
        if !release.published {
            return Err(invalid("release_not_published", instance.name.as_str()));
        }
        if release.required_secret_slot_count != 0 {
            return Err(invalid(
                "unsupported_required_secret_slots",
                instance.name.as_str(),
            ));
        }
        if release.parameters.len() > MAX_RECIPE_ITEMS
            || release
                .parameters
                .iter()
                .any(|parameter| !valid_parameter(parameter))
        {
            return Err(invalid(
                "unsupported_release_parameters",
                instance.name.as_str(),
            ));
        }
        validate_bindings(instance, release, resources)?;
        let parameters = instance
            .parameters
            .iter()
            .map(|(name, reference)| {
                resolve_value(reference, inputs).map(|value| (name.clone(), value))
            })
            .collect::<Result<BTreeMap<_, _>, _>>()?;
        let document = ParameterDocument::resolve(&release.parameters, &parameters)
            .map_err(|_| invalid("invalid_release_parameters", instance.name.as_str()))?;
        let mut requirements = release.capability_requirements.clone();
        requirements.sort_by(|left, right| left.slot().cmp(right.slot()));
        let mut volume_slots = release.volume_slots.clone();
        volume_slots.sort_by(|left, right| left.slot().cmp(right.slot()));
        resources.insert(
            instance.name.clone(),
            ResolvedResource::Instance(ResolvedInstance {
                dependencies: instance
                    .depends_on
                    .iter()
                    .cloned()
                    .chain(
                        instance
                            .volume_bindings
                            .iter()
                            .map(|binding| binding.resource.clone()),
                    )
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect(),
                release: instance.release,
                parameters: document.values().clone(),
                volume_bindings: instance.volume_bindings.clone(),
                volume_slots,
                capability_requirements: requirements,
                removal: instance.removal,
            }),
        );
    }
    Ok(())
}

fn resolve_value(
    reference: &ValueReference,
    inputs: &BTreeMap<ParameterName, ParameterValue>,
) -> Result<ParameterValue, RecipeError> {
    match reference {
        ValueReference::Literal { value } => Ok(value.clone()),
        ValueReference::Input { name } => inputs
            .get(name)
            .cloned()
            .ok_or_else(|| invalid("referenced_input_missing", name.as_str())),
        ValueReference::Resource { .. } => Err(invalid("resource_not_scalar", "reference")),
    }
}
