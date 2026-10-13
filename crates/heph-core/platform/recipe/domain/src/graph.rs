use std::collections::{BTreeMap, BTreeSet};

use capability_domain::CapabilitySlotKey;
use release_domain::ParameterValue;
use volume_domain::{MAX_VOLUME_CAPACITY_BYTES, MAX_VOLUME_SLOTS, VolumeAccessMode};

use crate::{
    InputType, MAX_RECIPE_ITEMS, RecipeError, RecipeManifest, ResourceDeclaration, ValueReference,
    VolumeSource, errors::invalid, inputs::bounded_value,
};

pub fn validate_graph(manifest: &RecipeManifest) -> Result<Vec<CapabilitySlotKey>, RecipeError> {
    if manifest.resources.is_empty()
        || manifest.resources.len() > MAX_RECIPE_ITEMS
        || manifest.inputs.len() > MAX_RECIPE_ITEMS
        || manifest.outputs.len() > MAX_RECIPE_ITEMS
    {
        return Err(invalid("recipe_item_limit", "recipe"));
    }
    let mut inputs = BTreeMap::new();
    for input in &manifest.inputs {
        if inputs.insert(input.name.clone(), input).is_some() {
            return Err(invalid("duplicate_input", input.name.as_str()));
        }
        input.validate()?;
    }
    let mut resources = BTreeMap::new();
    for resource in &manifest.resources {
        if resources
            .insert(resource.name().clone(), resource)
            .is_some()
        {
            return Err(invalid("duplicate_resource", resource.name().as_str()));
        }
    }
    let mut edges = BTreeMap::new();
    let mut consumers = BTreeMap::new();
    for resource in &manifest.resources {
        let name = resource.name();
        let mut dependencies = BTreeSet::new();
        if resource.dependencies().len() > MAX_RECIPE_ITEMS {
            return Err(invalid("dependency_limit", name.as_str()));
        }
        for dependency in resource.dependencies() {
            if !dependencies.insert(dependency.clone()) {
                return Err(invalid("duplicate_dependency", name.as_str()));
            }
            if !resources.contains_key(dependency) {
                return Err(invalid("unknown_dependency", name.as_str()));
            }
        }
        dependencies.extend(resource_dependencies(
            manifest,
            resource,
            &resources,
            &mut consumers,
        )?);
        edges.insert(name.clone(), dependencies);
    }
    let mut outputs = BTreeSet::new();
    for output in &manifest.outputs {
        if !outputs.insert(&output.name) {
            return Err(invalid("duplicate_output", output.name.as_str()));
        }
        validate_value_reference(manifest, &output.value, true)?;
    }
    topological_order(edges)
}

fn resource_dependencies(
    manifest: &RecipeManifest,
    resource: &ResourceDeclaration,
    resources: &BTreeMap<CapabilitySlotKey, &ResourceDeclaration>,
    consumers: &mut BTreeMap<CapabilitySlotKey, VolumeAccessMode>,
) -> Result<BTreeSet<CapabilitySlotKey>, RecipeError> {
    let name = resource.name();
    let mut dependencies = BTreeSet::new();
    match resource {
        ResourceDeclaration::Volume(volume) => match &volume.source {
            VolumeSource::Created { capacity_bytes } => {
                validate_value_reference(manifest, capacity_bytes, false)?;
                let valid = match capacity_bytes {
                    ValueReference::Literal {
                        value: ParameterValue::Integer(value),
                    } => u64::try_from(*value)
                        .is_ok_and(|value| value > 0 && value <= MAX_VOLUME_CAPACITY_BYTES),
                    ValueReference::Input { name } => manifest
                        .inputs
                        .iter()
                        .find(|input| input.name == *name)
                        .is_some_and(|input| {
                            matches!(
                                &input.value_type,
                                InputType::Integer { minimum, maximum }
                                    if *minimum > 0 && u64::try_from(*maximum)
                                        .is_ok_and(|value| value <= MAX_VOLUME_CAPACITY_BYTES)
                            )
                        }),
                    _ => false,
                };
                if !valid {
                    return Err(invalid("invalid_capacity", name.as_str()));
                }
            }
            VolumeSource::External {} if volume.removal == crate::RemovalPolicy::Delete => {
                return Err(invalid("external_delete_forbidden", name.as_str()));
            }
            VolumeSource::External {} => {}
        },
        ResourceDeclaration::Instance(instance) => {
            if instance.release.release_id.as_uuid().is_nil()
                || instance.release.release_agent_id.as_uuid().is_nil()
            {
                return Err(invalid("invalid_release_pin", name.as_str()));
            }
            if instance.parameters.len() > MAX_RECIPE_ITEMS
                || instance.volume_bindings.len() > MAX_VOLUME_SLOTS
            {
                return Err(invalid("instance_item_limit", name.as_str()));
            }
            for value in instance.parameters.values() {
                validate_value_reference(manifest, value, false)?;
            }
            let mut slots = BTreeSet::new();
            let mut bound_resources = BTreeSet::new();
            for binding in &instance.volume_bindings {
                if !slots.insert(&binding.slot) {
                    return Err(invalid("duplicate_volume_slot", name.as_str()));
                }
                if !bound_resources.insert(&binding.resource) {
                    return Err(invalid("volume_alias_reuse", name.as_str()));
                }
                if !matches!(
                    resources.get(&binding.resource),
                    Some(ResourceDeclaration::Volume(_))
                ) {
                    return Err(invalid("unknown_volume_resource", name.as_str()));
                }
                if let Some(previous_mode) =
                    consumers.insert(binding.resource.clone(), binding.access_mode)
                {
                    let code = if previous_mode == VolumeAccessMode::ReadWrite
                        && binding.access_mode == VolumeAccessMode::ReadWrite
                    {
                        "multiple_volume_writers"
                    } else {
                        "shared_volume_unsupported"
                    };
                    return Err(invalid(code, name.as_str()));
                }
                dependencies.insert(binding.resource.clone());
            }
        }
    }
    Ok(dependencies)
}

fn validate_value_reference(
    manifest: &RecipeManifest,
    reference: &ValueReference,
    allow_resource: bool,
) -> Result<(), RecipeError> {
    match reference {
        ValueReference::Literal { value } if bounded_value(value) => Ok(()),
        ValueReference::Input { name }
            if manifest.inputs.iter().any(|input| input.name == *name) =>
        {
            Ok(())
        }
        ValueReference::Resource { name }
            if allow_resource
                && manifest
                    .resources
                    .iter()
                    .any(|resource| resource.name() == name) =>
        {
            Ok(())
        }
        _ => Err(invalid("invalid_reference", "reference")),
    }
}

fn topological_order(
    mut edges: BTreeMap<CapabilitySlotKey, BTreeSet<CapabilitySlotKey>>,
) -> Result<Vec<CapabilitySlotKey>, RecipeError> {
    let mut order = Vec::with_capacity(edges.len());
    while !edges.is_empty() {
        let Some(name) = edges
            .iter()
            .find(|(_, dependencies)| dependencies.is_empty())
            .map(|(name, _)| name.clone())
        else {
            return Err(invalid("dependency_cycle", "resources"));
        };
        edges.remove(&name);
        for dependencies in edges.values_mut() {
            dependencies.remove(&name);
        }
        order.push(name);
    }
    Ok(order)
}
