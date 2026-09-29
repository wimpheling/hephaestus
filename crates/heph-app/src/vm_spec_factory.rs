use super::{
    BTreeMap, Deserialize, GuestCommand, NetworkMode, PgPool, RootFilesystem, Run, RunKind,
    RuntimePolicy, Uuid, VmError, VmMount, VmResources, VmSpec,
};
use async_trait::async_trait;
use control_plane_postgres::load_vm_launch_contract;
use heph_run::VmSpecFactory;
pub struct PgAgentVmSpecFactory {
    pub pool: PgPool,
    pub root_images: BTreeMap<String, RootFilesystem>,
    pub runtime_policy: RuntimePolicy,
}

#[derive(Deserialize)]
struct StoredRuntimeContract {
    #[serde(alias = "executable")]
    command: String,
    arguments: Vec<String>,
    working_directory: String,
    image_reference: String,
}

#[derive(Deserialize)]
struct StoredEffectivePolicy {
    vcpus: u8,
    memory_mib: u32,
    network: StoredNetworkAccess,
}

#[derive(Deserialize)]
struct StoredUpdateHook {
    command: String,
    arguments: Vec<String>,
    timeout_seconds: u32,
    resources: StoredHookResources,
}

#[derive(Deserialize)]
struct StoredHookResources {
    vcpus: u8,
    memory_mib: u32,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StoredNetworkAccess {
    Disabled,
    BrokerOnly,
    Egress,
}

#[async_trait]
impl VmSpecFactory for PgAgentVmSpecFactory {
    // Loading and validating the entire immutable launch contract in one
    // place keeps the no-substitution boundary directly auditable.
    #[allow(clippy::too_many_lines)]
    async fn build(&self, run: &Run) -> Result<VmSpec, VmError> {
        let stored = load_vm_launch_contract(&self.pool, run.id.as_uuid())
            .await
            .map_err(vm_factory_error)?
            .ok_or_else(|| invalid_spec("run", "exact reusable run provenance is missing"))?;
        if stored.release_state != "published" {
            return Err(invalid_spec(
                "release",
                "release is not currently available",
            ));
        }
        if !stored.revision_runnable || !stored.attachment_runnable {
            return Err(invalid_spec(
                "instance_revision",
                "the exact revision or attachment is not runnable",
            ));
        }
        if stored.requires_state != run.requires_state {
            return Err(invalid_spec(
                "requires_state",
                "run state capability does not match the immutable release agent",
            ));
        }
        let contract: StoredRuntimeContract =
            serde_json::from_value(stored.runtime_contract).map_err(vm_factory_error)?;
        let policy: StoredEffectivePolicy =
            serde_json::from_value(stored.effective_runtime_policy).map_err(vm_factory_error)?;
        let root = self
            .root_images
            .get(&contract.image_reference)
            .cloned()
            .ok_or_else(|| invalid_spec("guest.image", "OCI image is not materialized"))?;
        let network_access = policy.network;
        let network = match network_access {
            StoredNetworkAccess::Disabled => NetworkMode::Disabled,
            StoredNetworkAccess::Egress => NetworkMode::UserMode {
                ingress: Vec::new(),
            },
            StoredNetworkAccess::BrokerOnly => NetworkMode::BrokerOnly,
        };
        let (program, arguments, working_directory, resources, timeout_seconds) = match run.kind {
            RunKind::Normal => (
                format!("/release/{}", contract.command),
                contract.arguments,
                format!("/release/{}", contract.working_directory),
                VmResources {
                    vcpus: policy.vcpus,
                    memory_mib: policy.memory_mib,
                },
                None,
            ),
            RunKind::Update => {
                let hook: StoredUpdateHook = serde_json::from_value(
                    stored
                        .update_hook
                        .ok_or_else(|| invalid_spec("update_hook", "update hook is missing"))?,
                )
                .map_err(vm_factory_error)?;
                (
                    format!("/release/{}", hook.command),
                    hook.arguments,
                    String::from("/release"),
                    VmResources {
                        vcpus: hook.resources.vcpus,
                        memory_mib: hook.resources.memory_mib,
                    },
                    Some(hook.timeout_seconds),
                )
            }
        };
        validate_runtime_policy(&self.runtime_policy, &resources, network_access)?;
        let mut labels = BTreeMap::from([
            (
                String::from("hephaestus.instance"),
                run.instance_id.to_string(),
            ),
            (
                String::from("hephaestus.instance-revision"),
                run.instance_revision_id.to_string(),
            ),
            (
                String::from("hephaestus.release"),
                run.release_id.to_string(),
            ),
            (
                String::from("hephaestus.platform-policy"),
                self.runtime_policy.version.clone(),
            ),
        ]);
        if let Some(seconds) = timeout_seconds {
            labels.insert(
                String::from("hephaestus.wall-clock-timeout-seconds"),
                seconds.to_string(),
            );
        }
        let env = guest_environment(run.kind, stored.agent_update_id)?;
        Ok(VmSpec {
            id: heph_runtime::VmId(run.id.to_string()),
            root,
            disks: Vec::new(),
            mounts: Vec::<VmMount>::new(),
            resources,
            network,
            command: GuestCommand {
                program,
                args: arguments,
                env,
                working_dir: Some(working_directory.into()),
            },
            runtime_authority: None,
            runtime_git_bridge: None,
            private_http_service: None,
            labels,
        })
    }
}

pub fn guest_environment(
    kind: RunKind,
    update_id: Option<Uuid>,
) -> Result<BTreeMap<String, String>, VmError> {
    match (kind, update_id) {
        (RunKind::Normal, _) => Ok(BTreeMap::new()),
        (RunKind::Update, Some(update_id)) => Ok(BTreeMap::from([(
            String::from("HEPHAESTUS_UPDATE_ID"),
            update_id.to_string(),
        )])),
        (RunKind::Update, None) => Err(invalid_spec(
            "update",
            "stable update identity is missing from the hook run",
        )),
    }
}

pub fn validate_runtime_policy(
    current: &RuntimePolicy,
    resources: &VmResources,
    network: StoredNetworkAccess,
) -> Result<(), VmError> {
    if resources.vcpus > current.max_vcpus || resources.memory_mib > current.max_memory_mib {
        return Err(invalid_spec(
            "effective_runtime_policy.resources",
            "the immutable resource selection exceeds the current platform policy",
        ));
    }
    let network_allowed = match network {
        StoredNetworkAccess::Disabled => true,
        StoredNetworkAccess::BrokerOnly => current.allow_broker_only,
        StoredNetworkAccess::Egress => current.allow_egress,
    };
    if !network_allowed {
        return Err(invalid_spec(
            "effective_runtime_policy.network",
            "the immutable network selection is no longer allowed by platform policy",
        ));
    }
    Ok(())
}

pub fn invalid_spec(field: &str, reason: &str) -> VmError {
    VmError::InvalidSpec {
        field: field.to_owned(),
        reason: reason.to_owned(),
    }
}

fn vm_factory_error(error: impl std::error::Error + Send + Sync + 'static) -> VmError {
    VmError::Provider {
        provider: String::from("hephaestus-app"),
        code: String::from("spec-factory"),
        source: Box::new(error),
    }
}
