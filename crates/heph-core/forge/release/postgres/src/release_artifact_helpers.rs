use super::{
    AgentConfig, ArtifactKind, ContentHash, Digest, NetworkAccess, NetworkProfile,
    ParameterDeclaration, ParameterDefault, ParameterName, ParameterType, ParameterValue,
    ReleaseArtifactInput, ReleaseServiceError, RuntimePolicy, Sha256, Value,
};

pub fn artifact_manifest_hash(artifacts: &[ReleaseArtifactInput]) -> ContentHash {
    let mut sorted = artifacts.iter().collect::<Vec<_>>();
    sorted.sort_unstable_by_key(|artifact| artifact.path.as_str());
    let mut digest = Sha256::new();
    for artifact in sorted {
        update_field(&mut digest, artifact.path.as_str().as_bytes());
        update_field(&mut digest, artifact.content_hash.as_bytes());
        update_field(&mut digest, &artifact.size_bytes.to_be_bytes());
        update_field(&mut digest, &artifact.mode.to_be_bytes());
    }
    ContentHash::digest(&digest.finalize())
}

pub fn update_field(digest: &mut Sha256, value: &[u8]) {
    digest.update(value.len().to_be_bytes());
    digest.update(value);
}

pub const fn validate_artifact(artifact: &ReleaseArtifactInput) -> Result<(), ReleaseServiceError> {
    if artifact.media_type.is_empty() || artifact.media_type.len() > 256 || artifact.mode > 0o7777 {
        Err(ReleaseServiceError::InvalidArtifact)
    } else {
        Ok(())
    }
}

pub const fn artifact_kind_name(kind: ArtifactKind) -> &'static str {
    match kind {
        ArtifactKind::Executable => "executable",
        ArtifactKind::File => "file",
        ArtifactKind::Manifest => "manifest",
        ArtifactKind::BuildLog => "build_log",
    }
}

pub const fn runtime_policy(config: &AgentConfig) -> RuntimePolicy {
    RuntimePolicy {
        vcpus: config.resources.vcpus,
        memory_mib: config.resources.memory_mib,
        network: match config.network.profile {
            NetworkProfile::Disabled => NetworkAccess::Disabled,
            NetworkProfile::BrokerOnly => NetworkAccess::BrokerOnly,
            NetworkProfile::Egress => NetworkAccess::Egress,
        },
    }
}

pub fn parameter_schema(
    config: &AgentConfig,
) -> Result<Vec<ParameterDeclaration>, ReleaseServiceError> {
    config
        .parameters
        .iter()
        .map(|parameter| {
            let name = ParameterName::parse(parameter.name.clone())?;
            let value_type = match &parameter.value_type {
                agent_config::ParameterType::String {
                    minimum_length,
                    maximum_length,
                } => ParameterType::String {
                    minimum_length: *minimum_length,
                    maximum_length: *maximum_length,
                },
                agent_config::ParameterType::Integer { minimum, maximum } => {
                    ParameterType::Integer {
                        minimum: *minimum,
                        maximum: *maximum,
                    }
                }
                agent_config::ParameterType::Boolean => ParameterType::Boolean,
                agent_config::ParameterType::Enum { values } => ParameterType::Enum {
                    values: values.clone(),
                },
            };
            let default = parameter.default.as_ref().map(|value| match value {
                ParameterDefault::String(value) => ParameterValue::String(value.clone()),
                ParameterDefault::Integer(value) => ParameterValue::Integer(*value),
                ParameterDefault::Boolean(value) => ParameterValue::Boolean(*value),
            });
            Ok(ParameterDeclaration {
                name,
                value_type,
                required: parameter.required,
                default,
                sensitive: parameter.sensitive,
            })
        })
        .collect()
}

pub fn policy_from_contract(contract: &Value) -> Result<RuntimePolicy, ReleaseServiceError> {
    serde_json::from_value(
        contract
            .get("policy_ceiling")
            .cloned()
            .ok_or(ReleaseServiceError::InvalidStoredData)?,
    )
    .map_err(ReleaseServiceError::Serialization)
}
