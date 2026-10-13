//! Preserve original ordinary gateway owners while starting their existing loop.
use super::{
    AppError, Arc, CancellationToken, GatewayServiceBootRecovery, HephaestusApp,
    clone_service_supervisor_context, component, gateway_reconciliation_loop_with_context,
};
use crate::managed_backend::OwnedTasks;
pub(super) fn spawn(
    app: &HephaestusApp,
    cancellation: &CancellationToken,
    tasks: &mut OwnedTasks,
) -> Result<(), AppError> {
    if let Some(gateway) = &app.gateway_edge {
        let gateway_reconcile_cancel = cancellation.clone();
        let gateway_authority = gateway.authority.clone();
        let gateway_recovery_authority = gateway.recovery_authority.clone();
        let service_supervisor_context =
            clone_service_supervisor_context(&gateway.service_supervisor_context);
        let service_boot_recovery =
            GatewayServiceBootRecovery::new(clone_boot_context(&gateway.service_boot_context))
                .map_err(component("gateway service boot recovery"))?;
        let service_claim_resolution = Arc::clone(&gateway.service_claim_resolution);
        let service_expired_claim_recovery = Arc::clone(&gateway.service_expired_claim_recovery);
        let service_log_writer = gateway.service_log_writer.clone();
        let service_targets = Arc::clone(&gateway.service_supervisor_context.targets);
        let gateway_provider = Arc::clone(&gateway.provider);
        tasks.push(tokio::spawn(async move {
            gateway_reconciliation_loop_with_context(
                gateway_authority,
                gateway_recovery_authority,
                service_supervisor_context,
                service_boot_recovery,
                Some(service_claim_resolution),
                Some(service_expired_claim_recovery),
                Some(service_log_writer),
                service_targets,
                gateway_provider,
                gateway_reconcile_cancel,
            )
            .await;
            Ok(())
        }));
    }
    Ok(())
}
fn clone_boot_context(
    context: &gateway_edge::GatewayServiceBootRecoveryContext,
) -> gateway_edge::GatewayServiceBootRecoveryContext {
    gateway_edge::GatewayServiceBootRecoveryContext {
        owner: context.owner.clone(),
        cleanup_policy: context.cleanup_policy,
        shutdown_timeout: context.shutdown_timeout,
        ownership: Arc::clone(&context.ownership),
        exact_recovery: Arc::clone(&context.exact_recovery),
        targets: Arc::clone(&context.targets),
        failure_store: Arc::clone(&context.failure_store),
        resolver: Arc::clone(&context.resolver),
        provider: Arc::clone(&context.provider),
    }
}
