//! Owner of the actual running daemon and genuine publication pair.

use std::{path::PathBuf, sync::Arc, time::Duration};

use hephaestus_app::{AppConfig, HephaestusApp, RunningHephaestus};
use release_domain::{NetworkAccess, RuntimePolicy};
use release_service::InstanceExecutionConfiguration;
use run_runtime_local::LocalRunRuntimeConfig;
use sqlx::PgPool;
use vm_libkrun::LibkrunProvider;
use vm_trait::VmProvider;
use volume_local::{LocalVolumeConfig, VolumeRootOwner};

use super::{
    build_helpers::{BuildError, CookingBuildContext, CookingIdentity},
    publication::{self, ConfiguredBackend, PublicationPins, PublishedVariant},
    publication_provider::PublicationProvider,
    publication_setup::{FreshSetup, PublicationInputs, publication_seed::PublicationSeed},
    source_variants::{OwnedSourceVariant, VolumeVariant},
};

#[path = "publication_backend_drain.rs"]
mod drain;
#[path = "publication_backend_identity.rs"]
mod identity;
pub(super) use drain::{DrainProof, PublicationDrainJob};

/// Retain this owner before polling setup/start, and through later physical drain.
///
/// Partial failures leave its pool/config/actual owners and persistent roots
/// retained. Attempt flags are conservative and offer no retry/reopen/reset API.
pub(super) struct PublicationBackend {
    inputs: PublicationInputs,
    allocation: FreshSetup,
    seed: Option<PublicationSeed>,
    owner: Option<Arc<VolumeRootOwner>>,
    actual_provider: Option<Arc<LibkrunProvider>>,
    provider: Option<Arc<PublicationProvider>>,
    local: Option<LocalVolumeConfig>,
    runtime: Option<LocalRunRuntimeConfig>,
    configuration: Option<InstanceExecutionConfiguration>,
    app_config: Option<AppConfig>,
    setup_attempted: bool,
    start_attempted: bool,
    running: Option<RunningHephaestus>,
    app_pools: Option<[PgPool; 2]>,
    publications: [Option<Arc<PublishedVariant>>; 2],
    pins: [Option<PublicationPins>; 2],
    publication_attempted: bool,
}

fn unavailable() -> BuildError {
    std::io::Error::other("publication backend phase is unavailable or already attempted").into()
}

impl PublicationBackend {
    /// Creates exclusive persistent allocation history before any awaited setup.
    pub(super) fn allocate(inputs: PublicationInputs) -> Result<Self, BuildError> {
        let allocation = FreshSetup::allocate(&inputs)?;
        Ok(Self {
            inputs,
            allocation,
            seed: None,
            owner: None,
            actual_provider: None,
            provider: None,
            local: None,
            runtime: None,
            configuration: None,
            app_config: None,
            setup_attempted: false,
            start_attempted: false,
            running: None,
            app_pools: None,
            publications: [None, None],
            pins: [None, None],
            publication_attempted: false,
        })
    }

    /// Performs one setup attempt using the existing explicit owned bootstrap APIs.
    pub(super) async fn setup(&mut self) -> Result<(), BuildError> {
        if self.setup_attempted {
            return Err(unavailable());
        }
        self.setup_attempted = true;
        self.allocation.create_paths()?;
        self.allocation.create_database().await?;
        self.seed = Some(
            PublicationSeed::create(
                self.allocation.pool()?,
                &self.inputs,
                &self.allocation.root,
                &self.allocation.image,
            )
            .await?,
        );
        let local = self.allocation.local_config(&self.inputs);
        let runtime = self.allocation.runtime_config();
        let configuration = self.execution_configuration(&local)?;
        self.owner = Some(Arc::new(VolumeRootOwner::bootstrap_empty_with_namespace(
            &local.volume_root,
            &local.host_id,
            self.inputs.volume_namespace,
        )?));
        let publication_actual: Arc<dyn VmProvider> = {
            let actual = Arc::new(LibkrunProvider::bootstrap_owned(
                self.allocation.provider_config(&self.inputs),
                &self.inputs.provider_scope,
            )?);
            self.actual_provider = Some(actual.clone());
            actual
        };
        let provider = Arc::new(PublicationProvider::new(
            publication_actual,
            &self.inputs.provider_scope,
            &self.allocation.root.join("vm-runtime"),
            &self.allocation.cgroup_root,
        )?);
        let configured: Arc<dyn VmProvider> = provider.clone();
        self.app_config = Some(self.allocation.app_config(&self.inputs, configured));
        self.provider = Some(provider);
        self.local = Some(local);
        self.runtime = Some(runtime);
        self.configuration = Some(configuration);
        self.validate()
    }

    fn execution_configuration(
        &self,
        local: &LocalVolumeConfig,
    ) -> Result<InstanceExecutionConfiguration, BuildError> {
        // Match composition/construction's exact policy mapping, without clamping.
        let policy = &self.inputs.policy;
        Ok(InstanceExecutionConfiguration::new(
            self.inputs.provider_scope.clone(),
            local.volume_root.to_str().unwrap().to_owned(),
            self.inputs.volume_namespace,
            RuntimePolicy {
                vcpus: policy.max_vcpus,
                memory_mib: policy.max_memory_mib,
                network: if policy.allow_egress {
                    NetworkAccess::Egress
                } else if policy.allow_broker_only {
                    NetworkAccess::BrokerOnly
                } else {
                    NetworkAccess::Disabled
                },
            },
            policy.version.clone(),
        )?)
    }

    /// Starts the stock App once; this owner stays with the caller on any error.
    ///
    /// This does not consume shutdown or prove that background/physical work drained.
    pub(super) async fn start(&mut self) -> Result<(), BuildError> {
        if self.start_attempted {
            return Err(unavailable());
        }
        self.start_attempted = true;
        self.validate()?;
        let running = Box::pin(HephaestusApp::build(
            self.app_config.as_ref().ok_or_else(unavailable)?.clone(),
        ))
        .await?
        .start()
        .await?;
        // Store the genuine handle before any subsequent fallible step or await.
        self.running = Some(running);
        self.app_pools = Some({
            let running = self.running.as_ref().ok_or_else(unavailable)?;
            [
                running.application_pool_for_test(),
                running.service_log_pool_for_test(),
            ]
        });
        Ok(())
    }

    /// Runs both genuine publications with short-lived contexts borrowing this owner.
    pub(super) async fn publish_pair(&mut self) -> Result<(), BuildError> {
        if self.publication_attempted {
            return Err(unavailable());
        }
        self.publication_attempted = true;
        for (index, variant) in [VolumeVariant::Zero, VolumeVariant::Two]
            .into_iter()
            .enumerate()
        {
            let published = self.publish_variant(variant).await?;
            // Keep each real return before pinning or starting the other producer.
            self.publications[index] = Some(published);
            self.pins[index] = Some(
                PublicationPins::capture(
                    self.publications[index].as_ref().ok_or_else(unavailable)?,
                    &self.allocation.root,
                )
                .await?,
            );
        }
        self.validate_pair(&self.pair()?)?;
        Ok(())
    }

    // Running stays owned by self; its early borrowed check precedes filesystem work.
    // ConfiguredBackend retains provider/root pins through validation and awaited publication.
    #[allow(clippy::significant_drop_tightening)]
    async fn publish_variant(
        &self,
        variant: VolumeVariant,
    ) -> Result<Arc<PublishedVariant>, BuildError> {
        self.validate()?;
        let running = self.running.as_ref().ok_or_else(unavailable)?;
        let seed = self.seed.as_ref().ok_or_else(unavailable)?;
        let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/local-sqlite")
            .canonicalize()?;
        let git_token = seed.git_token(&self.inputs);
        let rpc_token = |audience: &str| seed.rpc_token(&self.inputs, audience);
        let build = CookingBuildContext {
            pool: self.pool()?,
            running,
            root: &self.allocation.root,
            source_root: &source,
            project_id: seed.project,
            repositories: &seed.repositories,
            identity: CookingIdentity {
                actor: &seed.actor,
                git_token: &git_token,
                rpc_token: &rpc_token,
            },
            timeout: Duration::from_secs(120),
        };
        let provider: Arc<dyn VmProvider> = self.provider()?.clone();
        let configured = ConfiguredBackend {
            build: &build,
            runtime: self.runtime.as_ref().ok_or_else(unavailable)?,
            local: self.local.as_ref().ok_or_else(unavailable)?,
            owner: self.owner.as_ref().ok_or_else(unavailable)?.clone(),
            provider,
            configuration: self.configuration.as_ref().ok_or_else(unavailable)?.clone(),
        };
        configured.validate();
        let source = OwnedSourceVariant::create(&source, &self.allocation.root, variant);
        Ok(Arc::new(publication::publish(&configured, source).await))
    }

    fn pair(&self) -> Result<[Arc<PublishedVariant>; 2], BuildError> {
        Ok([
            self.publications[0]
                .as_ref()
                .ok_or_else(unavailable)?
                .clone(),
            self.publications[1]
                .as_ref()
                .ok_or_else(unavailable)?
                .clone(),
        ])
    }

    /// Moves the complete owner into one protected job; no arbitrary handle is accepted.
    pub(super) fn begin_drain(self) -> PublicationDrainJob {
        PublicationDrainJob::start(self)
    }

    /// Returns the exact independent admin pool; never an App-owned pool clone.
    pub(super) fn pool(&self) -> Result<&PgPool, BuildError> {
        self.allocation.pool()
    }

    /// Returns the same real decorator for future admission closure/scoped cleanup.
    fn provider(&self) -> Result<&Arc<PublicationProvider>, BuildError> {
        self.provider.as_ref().ok_or_else(unavailable)
    }

    /// Retains persistent allocation information for future C failure archives.
    pub(super) const fn allocation(&self) -> &FreshSetup {
        &self.allocation
    }
}
