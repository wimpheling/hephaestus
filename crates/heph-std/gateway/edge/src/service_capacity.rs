//! Supervisor-owned capacity accounting for persistent gateway services.

use std::{collections::HashMap, time::Duration};

use uuid::Uuid;

use crate::{
    GatewayServiceLeasePolicy, MAX_SERVICE_REGISTRY_CAPACITY, MAX_SERVICE_REQUEST_CAPACITY,
    ServiceInstancePolicy,
};

/// Default number of gateways allowed to have a serving or replacement
/// instance.
pub const DEFAULT_SERVICE_SERVING_CAPACITY: usize = 8;
/// Default number of extra instances reserved for replacement and draining.
pub const DEFAULT_SERVICE_REPLACEMENT_CAPACITY: usize = 2;
/// Default maximum number of concurrent revisions for one gateway.
pub const DEFAULT_SERVICE_MAX_REVISIONS_PER_GATEWAY: usize = 2;
/// Default number of starts that may be in progress at once.
pub const DEFAULT_SERVICE_MAX_STARTUPS: usize = 2;
/// Default concurrent HTTP request capacity for one service instance.
pub const DEFAULT_SERVICE_REQUEST_CAPACITY: usize = 16;
/// Default health-check interval.
pub const DEFAULT_SERVICE_HEALTH_INTERVAL: Duration = Duration::from_secs(10);
/// Default consecutive health failures before a service is considered lost.
pub const DEFAULT_SERVICE_HEALTH_FAILURES: u32 = 3;
/// Default drain grace period.
pub const DEFAULT_SERVICE_DRAIN_TIMEOUT: Duration = Duration::from_secs(30);

const MAX_SERVICE_HEALTH_INTERVAL: Duration = Duration::from_secs(300);
const MAX_SERVICE_HEALTH_FAILURES: u32 = 32;
const MAX_SERVICE_DRAIN_TIMEOUT: Duration = Duration::from_secs(600);

/// Validated platform policy shared by the persistent-service supervisor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GatewayServiceSupervisorPolicy {
    /// Maximum number of distinct gateways with live instances.
    pub serving_gateway_capacity: usize,
    /// Additional instance slots for replacement and draining.
    pub replacement_capacity: usize,
    /// Maximum live revisions for one gateway.
    pub max_revisions_per_gateway: usize,
    /// Maximum starts whose readiness has not completed.
    pub max_simultaneous_startups: usize,
    /// Maximum concurrent HTTP requests admitted to one instance.
    pub requests_per_instance: usize,
    /// Durable ownership heartbeat policy.
    pub lease: GatewayServiceLeasePolicy,
    /// VM start, readiness, and shutdown policy.
    pub instance: ServiceInstancePolicy,
    /// Delay between health probes while the instance is serving.
    pub health_interval: Duration,
    /// Consecutive health failures tolerated before draining.
    pub health_failure_threshold: u32,
    /// Maximum time allowed for a draining instance to finish.
    pub drain_timeout: Duration,
}

impl Default for GatewayServiceSupervisorPolicy {
    fn default() -> Self {
        Self {
            serving_gateway_capacity: DEFAULT_SERVICE_SERVING_CAPACITY,
            replacement_capacity: DEFAULT_SERVICE_REPLACEMENT_CAPACITY,
            max_revisions_per_gateway: DEFAULT_SERVICE_MAX_REVISIONS_PER_GATEWAY,
            max_simultaneous_startups: DEFAULT_SERVICE_MAX_STARTUPS,
            requests_per_instance: DEFAULT_SERVICE_REQUEST_CAPACITY,
            lease: GatewayServiceLeasePolicy {
                lease_duration: Duration::from_secs(30),
                renewal_interval: Duration::from_secs(5),
            },
            instance: ServiceInstancePolicy::new(
                Duration::from_secs(120),
                Duration::from_millis(500),
                Duration::from_secs(2),
                Duration::from_secs(10),
            ),
            health_interval: DEFAULT_SERVICE_HEALTH_INTERVAL,
            health_failure_threshold: DEFAULT_SERVICE_HEALTH_FAILURES,
            drain_timeout: DEFAULT_SERVICE_DRAIN_TIMEOUT,
        }
    }
}

impl GatewayServiceSupervisorPolicy {
    /// Validates all supervisor bounds and timing relationships.
    ///
    /// # Errors
    ///
    /// Returns [`GatewayServiceCapacityError::InvalidPolicy`] when any
    /// capacity, duration, or lifecycle ratio is outside the bounded policy.
    pub fn validate(self) -> Result<(), GatewayServiceCapacityError> {
        if self.serving_gateway_capacity == 0
            || self.replacement_capacity == 0
            || self.max_revisions_per_gateway != 2
            || self.max_simultaneous_startups == 0
            || self.requests_per_instance == 0
            || self.serving_gateway_capacity > MAX_SERVICE_REGISTRY_CAPACITY
            || self
                .serving_gateway_capacity
                .checked_add(self.replacement_capacity)
                .is_none_or(|total| total > MAX_SERVICE_REGISTRY_CAPACITY)
            || self.requests_per_instance > MAX_SERVICE_REQUEST_CAPACITY
            || self.max_simultaneous_startups
                > self
                    .serving_gateway_capacity
                    .saturating_add(self.replacement_capacity)
        {
            return Err(GatewayServiceCapacityError::InvalidPolicy);
        }
        self.lease
            .validate()
            .map_err(|_| GatewayServiceCapacityError::InvalidPolicy)?;
        self.instance
            .validate()
            .map_err(|_| GatewayServiceCapacityError::InvalidPolicy)?;
        if self.instance.probe_timeout > self.instance.startup_timeout
            || self.health_interval.is_zero()
            || self.health_interval > MAX_SERVICE_HEALTH_INTERVAL
            || !(1..=MAX_SERVICE_HEALTH_FAILURES).contains(&self.health_failure_threshold)
            || self.drain_timeout.is_zero()
            || self.drain_timeout > MAX_SERVICE_DRAIN_TIMEOUT
        {
            return Err(GatewayServiceCapacityError::InvalidPolicy);
        }
        Ok(())
    }
}

/// Capacity accounting failures that the supervisor can handle without
/// mutating durable ownership.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum GatewayServiceCapacityError {
    /// The policy or gateway/revision identity is invalid.
    #[error("invalid gateway service capacity policy or identity")]
    InvalidPolicy,
    /// The distinct-gateway serving capacity is full for a new gateway.
    #[error("gateway service serving capacity is exhausted")]
    ServingCapacityExhausted,
    /// The total instance capacity is full.
    #[error("gateway service replacement capacity is exhausted")]
    TotalCapacityExhausted,
    /// This gateway already has its maximum live revisions.
    #[error("gateway service revision capacity is exhausted")]
    RevisionCapacityExhausted,
    /// The startup allowance is full.
    #[error("gateway service startup capacity is exhausted")]
    StartupCapacityExhausted,
    /// The exact gateway/revision pair is already reserved.
    #[error("gateway service revision is already reserved")]
    DuplicateRevision,
    /// The token does not identify a live reservation.
    #[error("unknown gateway service capacity reservation")]
    UnknownReservation,
    /// Startup was already completed for this token.
    #[error("gateway service startup is already complete")]
    StartupAlreadyComplete,
}

/// Opaque token for one capacity reservation. Dropping it never releases
/// capacity; the supervisor must call [`GatewayServiceCapacity::complete`]
/// after cleanup is confirmed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GatewayServiceCapacityToken {
    id: Uuid,
}

/// Bounded capacity counts for supervisor reconciliation and diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GatewayServiceCapacitySnapshot {
    /// Number of distinct gateways with a live reservation.
    pub serving_gateways: usize,
    /// Number of live instance reservations, including draining instances.
    pub live_instances: usize,
    /// Number of reservations that have not completed startup.
    pub starting_instances: usize,
}

#[derive(Debug, Clone, Copy)]
struct Reservation {
    gateway_id: Uuid,
    revision_id: Uuid,
    startup_complete: bool,
}

/// Single-owner mutable capacity accounting for the service supervisor.
#[derive(Debug)]
pub struct GatewayServiceCapacity {
    policy: GatewayServiceSupervisorPolicy,
    reservations: HashMap<Uuid, Reservation>,
}

impl GatewayServiceCapacity {
    /// Creates an empty accounting state after validating the policy.
    ///
    /// # Errors
    ///
    /// Returns [`GatewayServiceCapacityError::InvalidPolicy`] when the policy
    /// exceeds the registry/request bounds or has invalid timings.
    pub fn new(
        policy: GatewayServiceSupervisorPolicy,
    ) -> Result<Self, GatewayServiceCapacityError> {
        policy.validate()?;
        Ok(Self {
            policy,
            reservations: HashMap::new(),
        })
    }

    /// Returns the validated policy used by this accounting state.
    #[must_use]
    pub const fn policy(&self) -> GatewayServiceSupervisorPolicy {
        self.policy
    }

    /// Reserves startup and live-instance capacity before durable DB claim.
    ///
    /// # Errors
    ///
    /// Returns a capacity error when the identity is malformed, the exact
    /// revision is already reserved, or any bounded capacity is exhausted.
    pub fn reserve(
        &mut self,
        gateway_id: Uuid,
        revision_id: Uuid,
    ) -> Result<GatewayServiceCapacityToken, GatewayServiceCapacityError> {
        if gateway_id.is_nil() || revision_id.is_nil() {
            return Err(GatewayServiceCapacityError::InvalidPolicy);
        }
        let snapshot = self.snapshot();
        if self.reservations.values().any(|reservation| {
            reservation.gateway_id == gateway_id && reservation.revision_id == revision_id
        }) {
            return Err(GatewayServiceCapacityError::DuplicateRevision);
        }
        let gateway_instances = self
            .reservations
            .values()
            .filter(|reservation| reservation.gateway_id == gateway_id)
            .count();
        if gateway_instances >= self.policy.max_revisions_per_gateway {
            return Err(GatewayServiceCapacityError::RevisionCapacityExhausted);
        }
        if snapshot.starting_instances >= self.policy.max_simultaneous_startups {
            return Err(GatewayServiceCapacityError::StartupCapacityExhausted);
        }
        let gateway_exists = gateway_instances > 0;
        if !gateway_exists && snapshot.serving_gateways >= self.policy.serving_gateway_capacity {
            return Err(GatewayServiceCapacityError::ServingCapacityExhausted);
        }
        if snapshot.live_instances
            >= self
                .policy
                .serving_gateway_capacity
                .saturating_add(self.policy.replacement_capacity)
        {
            return Err(GatewayServiceCapacityError::TotalCapacityExhausted);
        }

        let token = loop {
            let candidate = GatewayServiceCapacityToken { id: Uuid::new_v4() };
            if !self.reservations.contains_key(&candidate.id) {
                break candidate;
            }
        };
        self.reservations.insert(
            token.id,
            Reservation {
                gateway_id,
                revision_id,
                startup_complete: false,
            },
        );
        Ok(token)
    }

    /// Marks startup complete and releases only this reservation's startup
    /// allowance while retaining its live capacity.
    ///
    /// # Errors
    ///
    /// Returns [`GatewayServiceCapacityError::UnknownReservation`] for a
    /// stale token or [`GatewayServiceCapacityError::StartupAlreadyComplete`]
    /// when startup was already completed.
    pub fn finish_startup(
        &mut self,
        token: GatewayServiceCapacityToken,
    ) -> Result<(), GatewayServiceCapacityError> {
        let reservation = self
            .reservations
            .get_mut(&token.id)
            .ok_or(GatewayServiceCapacityError::UnknownReservation)?;
        if reservation.startup_complete {
            return Err(GatewayServiceCapacityError::StartupAlreadyComplete);
        }
        reservation.startup_complete = true;
        Ok(())
    }

    /// Completes exact-token cleanup and releases its retained capacity.
    ///
    /// # Errors
    ///
    /// Returns [`GatewayServiceCapacityError::UnknownReservation`] when the
    /// token was already completed or does not belong to this manager.
    pub fn complete(
        &mut self,
        token: GatewayServiceCapacityToken,
    ) -> Result<(), GatewayServiceCapacityError> {
        self.reservations
            .remove(&token.id)
            .map(|_| ())
            .ok_or(GatewayServiceCapacityError::UnknownReservation)
    }

    /// Returns bounded current counts. Every reservation remains counted until
    /// its explicit completion, including draining or cleanup-failed instances.
    #[must_use]
    pub fn snapshot(&self) -> GatewayServiceCapacitySnapshot {
        let mut gateways = std::collections::HashSet::new();
        let mut starting_instances = 0;
        for reservation in self.reservations.values() {
            gateways.insert(reservation.gateway_id);
            if !reservation.startup_complete {
                starting_instances += 1;
            }
        }
        GatewayServiceCapacitySnapshot {
            serving_gateways: gateways.len(),
            live_instances: self.reservations.len(),
            starting_instances,
        }
    }
}

#[cfg(test)]
#[path = "service_capacity/tests.rs"]
mod tests;
