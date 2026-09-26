use super::gateway_reconciliation_loop::gateway_reconciliation_loop_with_boot;
use super::{
    Arc, CancellationToken, GatewayProvider, GatewayServiceBootRecovery,
    GatewayServiceClaimResolutionStore, GatewayServiceExpiredClaimRecovery,
    GatewayServiceLogWriterConfig, GatewayServiceSupervisor, GatewayServiceSupervisorContext,
    PostgresGatewayEdgeAuthority,
};

/// Reconstructs Caddy exclusively from authoritative route records. Ordinary
/// passes apply revision cutovers promptly; a bounded forced pass repairs a
/// Caddy process which restarted after this daemon observed the same revision.
// Keep separately owned adapters explicit at this daemon composition boundary.
// The supervisor owns cancellation and cleanup handles until the parent loop
// finishes; keeping it in this named binding makes that lifetime explicit.
#[allow(clippy::significant_drop_tightening, clippy::too_many_arguments)]
pub async fn gateway_reconciliation_loop_with_context(
    authority: PostgresGatewayEdgeAuthority,
    recovery_authority: PostgresGatewayEdgeAuthority,
    supervisor_context: GatewayServiceSupervisorContext,
    boot_recovery: GatewayServiceBootRecovery,
    service_claim_resolution: Option<Arc<dyn GatewayServiceClaimResolutionStore>>,
    service_expired_claim_recovery: Option<Arc<dyn GatewayServiceExpiredClaimRecovery>>,
    service_log_writer: Option<GatewayServiceLogWriterConfig>,
    service_targets: Arc<dyn gateway_edge::GatewayServiceTargetStore>,
    provider: Arc<dyn GatewayProvider>,
    cancellation: CancellationToken,
) {
    let mut service_supervisor =
        GatewayServiceSupervisor::new(supervisor_context).expect("validated service supervisor");
    if let Some(writer) = service_log_writer {
        service_supervisor = service_supervisor.with_log_writer(writer);
    }
    gateway_reconciliation_loop_with_boot(
        authority,
        recovery_authority,
        service_supervisor,
        Some(boot_recovery),
        service_claim_resolution,
        service_expired_claim_recovery,
        service_targets,
        provider,
        cancellation,
    )
    .await;
}

#[cfg(test)]
pub async fn gateway_reconciliation_loop(
    authority: PostgresGatewayEdgeAuthority,
    recovery_authority: PostgresGatewayEdgeAuthority,
    supervisor_context: GatewayServiceSupervisorContext,
    provider: Arc<dyn GatewayProvider>,
    cancellation: CancellationToken,
) {
    gateway_reconciliation_loop_with_supervisor(
        authority,
        recovery_authority,
        supervisor_context,
        provider,
        cancellation,
    )
    .await;
}

// Kept as a narrow test-facing composition helper for existing daemon-loop
// tests. Production supplies the already-constructed boot gate below.
#[cfg(test)]
pub async fn gateway_reconciliation_loop_with_supervisor(
    authority: PostgresGatewayEdgeAuthority,
    recovery_authority: PostgresGatewayEdgeAuthority,
    supervisor_context: GatewayServiceSupervisorContext,
    provider: Arc<dyn GatewayProvider>,
    cancellation: CancellationToken,
) {
    let targets = Arc::clone(&supervisor_context.targets);
    gateway_reconciliation_loop_with_boot(
        authority,
        recovery_authority,
        GatewayServiceSupervisor::new(supervisor_context).expect("validated service supervisor"),
        None,
        None,
        None,
        targets,
        provider,
        cancellation,
    )
    .await;
}
