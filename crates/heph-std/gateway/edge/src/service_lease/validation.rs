use std::{convert::TryFrom, time::Duration};
use tokio::sync::watch;

use super::types::{GatewayServiceLeaseLossReason, GatewayServiceLeaseStatus};
use crate::{GatewayServiceInstanceLease, GatewayServiceOwner, GatewayServiceOwnershipError};

pub(super) fn mark_stopped(status: &watch::Sender<GatewayServiceLeaseStatus>) {
    status.send_modify(|current| {
        if matches!(current, GatewayServiceLeaseStatus::Active { .. }) {
            *current = GatewayServiceLeaseStatus::Stopped;
        }
    });
}

pub(super) fn validate_lease(
    lease: &GatewayServiceInstanceLease,
    expected: &GatewayServiceInstanceLease,
    owner: &GatewayServiceOwner,
) -> Result<(), GatewayServiceLeaseLossReason> {
    if lease.identity != expected.identity || lease.vm_id != expected.vm_id {
        return Err(GatewayServiceLeaseLossReason::Invalid);
    }
    if lease.owner_host_id != owner.host_id {
        return Err(GatewayServiceLeaseLossReason::Invalid);
    }
    if lease.owner_uuid != owner.owner_uuid {
        return Err(GatewayServiceLeaseLossReason::Invalid);
    }
    if lease.fencing_token != expected.fencing_token || lease.fencing_token <= 0 {
        return Err(GatewayServiceLeaseLossReason::Invalid);
    }
    if lease.vm_id != format!("gateway-service-{}", lease.identity.instance_id) {
        return Err(GatewayServiceLeaseLossReason::Invalid);
    }
    if lease.lease_expires_at <= lease.heartbeat_at {
        return Err(GatewayServiceLeaseLossReason::Invalid);
    }
    if !lease.state.is_live()
        || lease.identity.instance_id.is_nil()
        || lease.identity.gateway_id.is_nil()
        || lease.identity.revision_id.is_nil()
    {
        return Err(GatewayServiceLeaseLossReason::Invalid);
    }
    Ok(())
}

pub(super) fn database_budget(lease: &GatewayServiceInstanceLease) -> Option<Duration> {
    let budget = lease.lease_expires_at - lease.heartbeat_at;
    Duration::try_from(budget)
        .ok()
        .filter(|value| !value.is_zero())
}

pub(super) const fn loss_reason(
    error: GatewayServiceOwnershipError,
) -> GatewayServiceLeaseLossReason {
    match error {
        GatewayServiceOwnershipError::StaleLease => GatewayServiceLeaseLossReason::Stale,
        GatewayServiceOwnershipError::Conflict => GatewayServiceLeaseLossReason::Conflict,
        GatewayServiceOwnershipError::InvalidArgument
        | GatewayServiceOwnershipError::Unavailable => GatewayServiceLeaseLossReason::Invalid,
    }
}
