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
mod tests {
    use super::*;

    fn id(value: u128) -> Uuid {
        Uuid::from_u128(value)
    }

    fn manager() -> GatewayServiceCapacity {
        GatewayServiceCapacity::new(GatewayServiceSupervisorPolicy::default())
            .expect("default policy")
    }

    #[test]
    fn defaults_are_validated_and_bounded() {
        let policy = GatewayServiceSupervisorPolicy::default();
        policy.validate().expect("default policy is valid");
        assert_eq!(policy.serving_gateway_capacity, 8);
        assert_eq!(policy.replacement_capacity, 2);
        assert_eq!(policy.requests_per_instance, 16);
        assert_eq!(policy.lease.lease_duration, Duration::from_secs(30));
        assert_eq!(policy.lease.renewal_interval, Duration::from_secs(5));
        assert_eq!(policy.instance.startup_timeout, Duration::from_secs(120));
        assert_eq!(policy.instance.probe_interval, Duration::from_millis(500));
        assert_eq!(policy.instance.probe_timeout, Duration::from_secs(2));
        assert_eq!(policy.instance.shutdown_timeout, Duration::from_secs(10));
        assert_eq!(policy.health_failure_threshold, 3);
        assert_eq!(policy.drain_timeout, Duration::from_secs(30));
    }

    #[test]
    fn invalid_policy_and_identity_fail_closed() {
        let policy = GatewayServiceSupervisorPolicy {
            requests_per_instance: MAX_SERVICE_REQUEST_CAPACITY + 1,
            ..GatewayServiceSupervisorPolicy::default()
        };
        assert_eq!(
            GatewayServiceCapacity::new(policy).unwrap_err(),
            GatewayServiceCapacityError::InvalidPolicy
        );
        let policy = GatewayServiceSupervisorPolicy {
            health_failure_threshold: 4,
            health_interval: Duration::ZERO,
            ..GatewayServiceSupervisorPolicy::default()
        };
        assert_eq!(
            GatewayServiceCapacity::new(policy).unwrap_err(),
            GatewayServiceCapacityError::InvalidPolicy
        );
        let policy = GatewayServiceSupervisorPolicy {
            replacement_capacity: 0,
            ..GatewayServiceSupervisorPolicy::default()
        };
        assert_eq!(
            GatewayServiceCapacity::new(policy).unwrap_err(),
            GatewayServiceCapacityError::InvalidPolicy
        );
        let policy = GatewayServiceSupervisorPolicy {
            max_revisions_per_gateway: 1,
            ..GatewayServiceSupervisorPolicy::default()
        };
        assert_eq!(
            GatewayServiceCapacity::new(policy).unwrap_err(),
            GatewayServiceCapacityError::InvalidPolicy
        );
        let mut capacity = manager();
        assert_eq!(
            capacity.reserve(Uuid::nil(), id(1)).unwrap_err(),
            GatewayServiceCapacityError::InvalidPolicy
        );
    }

    #[test]
    fn full_serving_pool_allows_replacement_but_not_new_gateway() {
        let mut capacity = manager();
        for gateway in 1..=8 {
            let token = capacity
                .reserve(id(gateway), id(gateway + 100))
                .expect("serving slot");
            capacity.finish_startup(token).expect("serving instance");
        }
        assert_eq!(capacity.snapshot().serving_gateways, 8);
        let first_replacement = capacity.reserve(id(1), id(201)).expect("replacement slot");
        capacity
            .finish_startup(first_replacement)
            .expect("replacement serving");
        assert_eq!(capacity.snapshot().live_instances, 9);
        assert_eq!(
            capacity.reserve(id(9), id(109)).unwrap_err(),
            GatewayServiceCapacityError::ServingCapacityExhausted
        );
        let second_replacement = capacity
            .reserve(id(2), id(202))
            .expect("second replacement slot");
        capacity
            .finish_startup(second_replacement)
            .expect("second replacement serving");
        assert_eq!(capacity.snapshot().live_instances, 10);
        assert_eq!(
            capacity.reserve(id(3), id(203)).unwrap_err(),
            GatewayServiceCapacityError::TotalCapacityExhausted
        );
    }

    #[test]
    fn revision_and_startup_bounds_are_independent() {
        let mut capacity = manager();
        let first = capacity.reserve(id(1), id(101)).expect("first startup");
        let second = capacity.reserve(id(2), id(102)).expect("second startup");
        assert_eq!(
            capacity.reserve(id(3), id(103)).unwrap_err(),
            GatewayServiceCapacityError::StartupCapacityExhausted
        );
        capacity.finish_startup(first).expect("first ready");
        let third = capacity
            .reserve(id(3), id(103))
            .expect("startup allowance released");
        assert_eq!(
            capacity.reserve(id(1), id(101)).unwrap_err(),
            GatewayServiceCapacityError::DuplicateRevision
        );
        capacity.finish_startup(second).expect("second ready");
        capacity.finish_startup(third).expect("third ready");
        assert_eq!(capacity.snapshot().starting_instances, 0);
    }

    #[test]
    fn draining_blocks_replacement_until_exact_cleanup() {
        let mut capacity = manager();
        let old = capacity.reserve(id(1), id(101)).expect("old");
        capacity.finish_startup(old).expect("old ready");
        let candidate = capacity.reserve(id(1), id(102)).expect("candidate");
        assert_eq!(
            capacity.reserve(id(1), id(103)).unwrap_err(),
            GatewayServiceCapacityError::RevisionCapacityExhausted
        );
        capacity.complete(old).expect("old cleanup");
        let next = capacity.reserve(id(1), id(103)).expect("next replacement");
        capacity.complete(candidate).expect("candidate cleanup");
        capacity.complete(next).expect("next cleanup");
        assert_eq!(capacity.snapshot().live_instances, 0);
    }

    #[test]
    fn stale_or_dropped_tokens_cannot_release_a_new_reservation() {
        let mut capacity = manager();
        let old = capacity.reserve(id(1), id(101)).expect("old");
        let stale = old;
        capacity.complete(old).expect("old cleanup");
        let replacement = capacity.reserve(id(1), id(102)).expect("replacement");
        assert_eq!(
            capacity.complete(stale).unwrap_err(),
            GatewayServiceCapacityError::UnknownReservation
        );
        assert_eq!(capacity.snapshot().live_instances, 1);
        assert_eq!(capacity.finish_startup(replacement), Ok(()));
        assert_eq!(
            capacity.finish_startup(replacement).unwrap_err(),
            GatewayServiceCapacityError::StartupAlreadyComplete
        );
        capacity.complete(replacement).expect("replacement cleanup");
    }

    #[test]
    fn counts_do_not_leak_between_gateways() {
        let mut capacity = manager();
        let first = capacity.reserve(id(1), id(101)).expect("first");
        capacity.finish_startup(first).expect("first ready");
        let second = capacity.reserve(id(2), id(101)).expect("different gateway");
        assert_eq!(capacity.snapshot().serving_gateways, 2);
        assert_eq!(capacity.snapshot().live_instances, 2);
        capacity.complete(first).expect("first cleanup");
        capacity.complete(second).expect("second cleanup");
    }

    #[test]
    fn reservation_drop_does_not_implicitly_release_capacity() {
        let mut capacity = manager();
        let _token = capacity.reserve(id(1), id(101)).expect("reservation");
        assert_eq!(capacity.snapshot().live_instances, 1);
    }
}
