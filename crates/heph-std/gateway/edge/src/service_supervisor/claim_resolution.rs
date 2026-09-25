use super::{
    Arc, ClaimResolutionCompletion, ClaimResolutionResult, CleanupRetryState,
    GatewayServiceClaimResolutionStore, GatewayServiceCleanup,
    GatewayServiceCoordinatorFailureReason, GatewayServiceInstanceLease,
    GatewayServiceOwnershipError, GatewayServiceStartupRequest, GatewayServiceSupervisorContext,
    GatewayServiceSupervisorError, Instant, Uuid, time,
};

pub(super) async fn run_claim_resolution(
    id: Uuid,
    request: GatewayServiceStartupRequest,
    known_lease: Option<GatewayServiceInstanceLease>,
    reason: Option<GatewayServiceCoordinatorFailureReason>,
    resolver: Arc<dyn GatewayServiceClaimResolutionStore>,
    deadline: Instant,
    context: Arc<GatewayServiceSupervisorContext>,
) -> ClaimResolutionCompletion {
    let result = match time::timeout_at(
        deadline,
        resolver.resolve_revision_claim(request.gateway_id, request.revision_id),
    )
    .await
    {
        Ok(Ok(None)) => Ok(ClaimResolutionResult::Absent),
        Ok(Ok(Some(lease))) => resolve_claim_result(request, known_lease, reason, lease, &context),
        Ok(Err(GatewayServiceOwnershipError::Unavailable)) | Err(_) => {
            Err(GatewayServiceSupervisorError::ClaimResolutionUnavailable)
        }
        Ok(Err(_)) => Ok(ClaimResolutionResult::Retained(None)),
    };
    ClaimResolutionCompletion { id, result }
}

pub(super) fn resolve_claim_result(
    request: GatewayServiceStartupRequest,
    known_lease: Option<GatewayServiceInstanceLease>,
    reason: Option<GatewayServiceCoordinatorFailureReason>,
    lease: GatewayServiceInstanceLease,
    context: &GatewayServiceSupervisorContext,
) -> Result<ClaimResolutionResult, GatewayServiceSupervisorError> {
    if !valid_claim_identity(&lease)
        || lease.identity.gateway_id != request.gateway_id
        || lease.identity.revision_id != request.revision_id
        || lease.owner_host_id != context.owner.host_id
        || lease.owner_uuid != context.owner.owner_uuid
        || lease.lease_expires_at <= ::time::OffsetDateTime::now_utc()
        || !lease.state.is_live()
    {
        return Ok(ClaimResolutionResult::Retained(Some(lease)));
    }
    if let Some(known) = known_lease {
        if !same_claim(&known, &lease) {
            return Ok(ClaimResolutionResult::Retained(Some(lease)));
        }
    }
    let cleanup = GatewayServiceCleanup::new(
        lease.identity,
        None,
        context.policy.instance.shutdown_timeout,
    )
    .map_err(|_| GatewayServiceSupervisorError::InvalidInput)?;
    Ok(ClaimResolutionResult::Owned(CleanupRetryState {
        cleanup,
        lease,
        pending_failure: None,
        reason,
    }))
}

pub(super) fn valid_claim_identity(lease: &GatewayServiceInstanceLease) -> bool {
    lease.identity.instance_id != Uuid::nil()
        && lease.identity.gateway_id != Uuid::nil()
        && lease.identity.revision_id != Uuid::nil()
        && lease.fencing_token > 0
        && lease.vm_id == format!("gateway-service-{}", lease.identity.instance_id)
        && lease.lease_expires_at > lease.heartbeat_at
}

pub(super) fn same_claim(
    left: &GatewayServiceInstanceLease,
    right: &GatewayServiceInstanceLease,
) -> bool {
    left.identity == right.identity
        && left.vm_id == right.vm_id
        && left.owner_host_id == right.owner_host_id
        && left.owner_uuid == right.owner_uuid
        && left.fencing_token == right.fencing_token
}
