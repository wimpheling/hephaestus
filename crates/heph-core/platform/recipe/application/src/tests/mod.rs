mod identity;
mod planner;
mod planning;

use std::collections::BTreeMap;

use capability_domain::CapabilitySlotKey;
use forge_domain::ProjectId;
use recipe_domain::ValidatedRecipe;
use release_domain::{ParameterName, ParameterValue};
use uuid::Uuid;

use crate::{DeploymentId, DeploymentIntent, DeploymentKey};

const VOLUME: &str = r#"
contract_version = 1
recipe_id = "private-volume"
recipe_version = "1.0.0"
[[inputs]]
name = "capacity"
default = 16777216
value_type = { type = "integer", minimum = 16777216, maximum = 1073741824 }
[[resources]]
kind = "volume"
name = "data"
source = { type = "created", capacity_bytes = { source = "input", name = "capacity" } }
"#;

fn key(value: &str) -> CapabilitySlotKey {
    CapabilitySlotKey::parse(value).expect("test resource key")
}

fn deployment(value: u128) -> DeploymentId {
    DeploymentId::from_uuid(Uuid::from_u128(value)).expect("test deployment")
}

fn intent(
    recipe: &ValidatedRecipe,
    inputs: &BTreeMap<ParameterName, ParameterValue>,
) -> DeploymentIntent {
    let resolved = recipe
        .resolve(inputs, &BTreeMap::new(), &[])
        .expect("resolve volume");
    DeploymentIntent::new(
        deployment(1),
        ProjectId::from_uuid(Uuid::from_u128(2)),
        DeploymentKey::parse("database").expect("key"),
        recipe,
        &resolved,
    )
    .expect("intent")
}
