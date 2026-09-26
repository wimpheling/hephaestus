use std::{collections::BTreeMap, path::PathBuf, sync::Arc};

use crate::{
    AppError, EncryptedFileHandoffStore, GATEWAY_SERVICE_REPLACEMENT_CAPACITY,
    GATEWAY_SERVICE_REQUEST_CAPACITY, GATEWAY_SERVICE_SERVING_CAPACITY, GatewayDispatcher,
    GatewayEdgeConfig, GatewayEdgeRuntime, GatewayInboundSecretResolver,
    GatewayReleaseMaterializer, GatewayRequestDispatcher, GatewayRuntimeAuthorityIssuer,
    GatewayRuntimeService, GatewayServiceBootRecoveryContext, GatewayServiceClaimResolutionStore,
    GatewayServiceCleanupDriverPolicy, GatewayServiceExpiredClaimRecovery, GatewayServiceHandler,
    GatewayServiceLogStore, GatewayServiceLogWriterConfig, GatewayServiceMaterializer,
    GatewayServiceOwner, GatewayServiceRegistry, GatewayServiceSupervisor,
    GatewayServiceSupervisorContext, GatewayServiceSupervisorPolicy, LocalCaddyAdministration,
    LocalCaddyConfigurationTemplate, LocalCaddyGatewayProvider, LocalGatewayReleaseMaterializer,
    LocalKeyProvider, PgGatewayRuntimeAuthorityIssuer, PgPool, PostgresGatewayEdgeAuthority,
    PostgresGatewayExecutionTargetResolver, PostgresGatewayMailboxPublisher,
    PostgresGatewayReleaseResolver, PostgresGatewayServiceFailureStore,
    PostgresGatewayServiceLaunchResolver, PostgresGatewayServiceOwnership,
    PostgresGatewayServiceTargets, PrivateHttpVmGatewayHandler, ProviderGatewayRuntimeLauncher,
    RootFilesystem, RuntimeHandoffStore, ServiceLogWriterPolicy, clone_service_supervisor_context,
    component, connect_control_plane, connect_oci_worker, gateway_limits,
};
use heph_runtime::VmProvider;
use secret_postgres::GatewayIngressSecretResolver;
use secret_store::EncryptedStore;
use std::time::Duration;
use uuid::Uuid;
pub struct GatewayInputs {
    pub database_url: String,
    pub gateway_service_host_id: String,
    pub pool: PgPool,
    pub gateway_release_runtime: LocalGatewayReleaseMaterializer,
    pub gateway_edge_config: Option<GatewayEdgeConfig>,
    pub gateway_secret_keys: LocalKeyProvider,
    pub gateway_handoff_root: PathBuf,
    pub gateway_handoff_key: [u8; 32],
    pub gateway_root_images: BTreeMap<String, RootFilesystem>,
    pub service_log_store: Arc<dyn GatewayServiceLogStore>,
    pub provider: Arc<dyn VmProvider>,
}

// Gateway construction follows the runtime dependency order and stays isolated here.
#[allow(clippy::too_many_lines)]
pub async fn build(inputs: GatewayInputs) -> Result<Option<crate::GatewayEdgeRuntime>, AppError> {
    let GatewayInputs {
        database_url,
        gateway_service_host_id,
        pool,
        gateway_release_runtime,
        gateway_edge_config,
        gateway_secret_keys,
        gateway_handoff_root,
        gateway_handoff_key,
        gateway_root_images,
        service_log_store,
        provider,
    } = inputs;
    let gateway_edge = if let Some(gateway) = gateway_edge_config {
        // Gateway runtime snapshots and sessions are worker-owned
        // immutable authority records. Keep issuance on a dedicated
        // worker-role pool rather than leaking those writes through the
        // user-scoped control-plane pool.
        let gateway_authority_pool = connect_oci_worker(&database_url, 4)
            .await
            .map_err(component("gateway runtime authority PostgreSQL connection"))?;
        let service_owner = GatewayServiceOwner::new(gateway_service_host_id, Uuid::new_v4())
            .map_err(component("gateway service owner"))?;
        let service_registry = GatewayServiceRegistry::new(
            GATEWAY_SERVICE_SERVING_CAPACITY + GATEWAY_SERVICE_REPLACEMENT_CAPACITY,
            GATEWAY_SERVICE_REQUEST_CAPACITY,
        )
        .map_err(component("gateway service registry"))?;
        let issuer_handoff =
            EncryptedFileHandoffStore::new(gateway_handoff_root.clone(), gateway_handoff_key)
                .map_err(component("gateway runtime authority handoff"))?;
        let issuer: Arc<dyn GatewayRuntimeAuthorityIssuer> =
            Arc::new(PgGatewayRuntimeAuthorityIssuer::new(
                gateway_authority_pool.clone(),
                issuer_handoff,
                authz_postgres::AUTHORIZATION_MODEL_VERSION,
            ));
        let authority = PostgresGatewayEdgeAuthority::new(pool.clone(), gateway_limits())
            .with_runtime_authority(Arc::clone(&issuer), Duration::from_secs(30))
            .map_err(component("gateway runtime authority"))?;
        let recovery_authority =
            PostgresGatewayEdgeAuthority::new(gateway_authority_pool.clone(), gateway_limits());
        let ui_authority =
            PostgresGatewayEdgeAuthority::new(gateway_authority_pool.clone(), gateway_limits())
                .with_runtime_authority(Arc::clone(&issuer), Duration::from_secs(30))
                .map_err(component("UI gateway runtime authority"))?;
        let gateway_release_materializer = Arc::new(gateway_release_runtime);
        let gateway_release_materializer_port: Arc<dyn GatewayReleaseMaterializer> =
            gateway_release_materializer.clone();
        let gateway_service_materializer: Arc<dyn GatewayServiceMaterializer> =
            gateway_release_materializer.clone();
        let service_ownership = Arc::new(PostgresGatewayServiceOwnership::new(
            gateway_authority_pool.clone(),
        ));
        let service_claim_resolution: Arc<dyn GatewayServiceClaimResolutionStore> =
            service_ownership.clone();
        let service_expired_claim_recovery: Arc<dyn GatewayServiceExpiredClaimRecovery> =
            service_ownership.clone();
        let service_failure_store = Arc::new(PostgresGatewayServiceFailureStore::new(
            gateway_authority_pool.clone(),
        ));
        let service_log_writer = GatewayServiceLogWriterConfig::new(
            service_log_store.clone() as Arc<dyn GatewayServiceLogStore>,
            ServiceLogWriterPolicy::default(),
        );
        let service_launch_resolver = Arc::new(
            PostgresGatewayServiceLaunchResolver::new(
                gateway_authority_pool.clone(),
                gateway_root_images.clone(),
            )
            .with_service_materializer(Arc::clone(&gateway_service_materializer)),
        );
        let service_targets = Arc::new(PostgresGatewayServiceTargets::new(
            gateway_authority_pool.clone(),
        ));
        let resolver_handoff: Arc<dyn RuntimeHandoffStore> = Arc::new(
            EncryptedFileHandoffStore::new(gateway_handoff_root, gateway_handoff_key)
                .map_err(component("gateway runtime resolver handoff"))?,
        );
        let releases = PostgresGatewayReleaseResolver::new(
            pool.clone(),
            gateway_root_images.clone(),
            resolver_handoff,
        )
        .with_release_materializer(gateway_release_materializer_port);
        let runtime = GatewayRuntimeService::new(
            releases,
            ProviderGatewayRuntimeLauncher {
                provider: Arc::clone(&provider),
            },
        );
        let stateless_handler = PrivateHttpVmGatewayHandler::new(runtime);
        let service_supervisor_context = Arc::new(GatewayServiceSupervisorContext {
            owner: service_owner.clone(),
            policy: GatewayServiceSupervisorPolicy::default(),
            ownership: service_ownership.clone(),
            failure_store: service_failure_store.clone(),
            resolver: service_launch_resolver.clone(),
            provider: Arc::clone(&provider),
            targets: service_targets.clone(),
            registry: service_registry.clone(),
            service_authority: gateway.public_authority.clone(),
        });
        let service_policy = service_supervisor_context.policy;
        let service_boot_context = GatewayServiceBootRecoveryContext {
            owner: service_supervisor_context.owner.clone(),
            cleanup_policy: GatewayServiceCleanupDriverPolicy {
                lease: service_policy.lease,
                database_timeout: service_policy.instance.probe_timeout,
            },
            shutdown_timeout: service_policy.instance.shutdown_timeout,
            ownership: service_ownership.clone(),
            exact_recovery: service_ownership.clone(),
            targets: service_targets.clone(),
            failure_store: service_failure_store.clone(),
            resolver: service_launch_resolver.clone(),
            provider: Arc::clone(&service_supervisor_context.provider),
        };
        GatewayServiceSupervisor::new(clone_service_supervisor_context(
            &service_supervisor_context,
        ))
        .map_err(component("gateway service supervisor"))?;
        let handler = Arc::new(
            GatewayServiceHandler::new(
                PostgresGatewayExecutionTargetResolver::new(gateway_authority_pool.clone()),
                stateless_handler,
                service_registry,
                service_owner,
            )
            .map_err(component("gateway service handler"))?,
        );
        let ingress_pool = connect_control_plane(&database_url, 4)
            .await
            .map_err(component("gateway secret resolver PostgreSQL connection"))?;
        let inbound: Arc<dyn GatewayInboundSecretResolver> =
            Arc::new(GatewayIngressSecretResolver::new(
                ingress_pool,
                EncryptedStore::new(gateway_secret_keys),
            ));
        let mailbox: Arc<dyn gateway_edge::GatewayMailboxPublisher> =
            Arc::new(PostgresGatewayMailboxPublisher::new(pool.clone()));
        let dispatcher: Arc<dyn GatewayRequestDispatcher> = Arc::new(
            GatewayDispatcher::new(authority.clone(), Arc::clone(&handler), authority.clone())
                .with_inbound_secret_resolver(Arc::clone(&inbound))
                .with_mailbox_publisher(Arc::clone(&mailbox)),
        );
        let ui_dispatcher = gateway.ui_origin.as_ref().map(|ui| {
            let ui_core = Arc::new(
                GatewayDispatcher::new(
                    ui_authority.clone(),
                    Arc::clone(&handler),
                    ui_authority.clone(),
                )
                .with_inbound_secret_resolver(Arc::clone(&inbound))
                .with_mailbox_publisher(Arc::clone(&mailbox)),
            );
            Arc::new(crate::ui_origin_wiring::RealUiGatewayDispatcher::new(
                ui_core,
                Arc::new(ui_authority.clone()),
                ui.namespace().clone(),
                ui.public_port(),
            )) as Arc<dyn crate::ui_browser_content::UiGatewayDispatcher>
        });
        let administration = LocalCaddyAdministration::new(&gateway.caddy_admin_url)
            .map_err(component("gateway Caddy administration"))?;
        let template = LocalCaddyConfigurationTemplate::new(
            &gateway.caddy_configuration_template,
            gateway.caddy_server_name,
        )
        .map_err(component("gateway Caddy configuration template"))?;
        let template = match &gateway.ui_origin {
            Some(ui) => template
                .with_ui_namespace(
                    ui.namespace().as_str(),
                    ui.listener().ok_or_else(|| {
                        AppError::Configuration(String::from("missing UI origin listener"))
                    })?,
                )
                .map_err(component("gateway UI Caddy configuration"))?,
            None => template,
        };
        let provider: Arc<dyn gateway_edge::GatewayProvider> = Arc::new(
            LocalCaddyGatewayProvider::new(administration, Arc::clone(&dispatcher))
                .with_dispatcher_upstream(gateway.dispatcher_listen.to_string())
                .with_configuration_template(template),
        );
        Some(GatewayEdgeRuntime {
            authority,
            recovery_authority,
            service_supervisor_context,
            service_claim_resolution,
            service_expired_claim_recovery,
            service_boot_context,
            service_log_writer,
            provider,
            dispatcher,
            dispatcher_listen: gateway.dispatcher_listen,
            public_authority: gateway.public_authority,
            ui_dispatcher,
            ui_origin: gateway.ui_origin,
        })
    } else {
        None
    };
    Ok(gateway_edge)
}
