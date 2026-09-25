//! Bounded in-memory registry for ready persistent gateway service instances.

use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};

use crate::GatewayServiceInstanceKey;
use tokio::{
    sync::{OwnedSemaphorePermit, Semaphore, watch},
    time::{self, Instant},
};
use tokio_util::sync::CancellationToken;
use vm_trait::{VmId, VmInstance};

use crate::{
    GatewayEdgeError, GatewayRequest, GatewayResponse, ServiceHttpPolicy, ServiceWorkerState,
    exchange_private_service_http,
};

/// Maximum number of live service instances held by one registry.
pub const MAX_SERVICE_REGISTRY_CAPACITY: usize = 128;
/// Maximum concurrent HTTP exchanges admitted to one service instance.
pub const MAX_SERVICE_REQUEST_CAPACITY: usize = 31;

/// Redacted registry admission and lifecycle errors.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum GatewayServiceRegistryError {
    /// The key contains a nil identity or zero fencing token.
    #[error("invalid gateway service registry key")]
    InvalidKey,
    /// A registry or per-instance capacity is outside the reviewed bound.
    #[error("invalid gateway service registry capacity")]
    InvalidCapacity,
    /// The VM identifier does not match the service instance identity.
    #[error("gateway service VM identity does not match the registry key")]
    VmIdentityMismatch,
    /// Registration requires a worker that has reached readiness.
    #[error("gateway service is not ready")]
    NotReady,
    /// The exact key is already registered.
    #[error("gateway service instance is already registered")]
    Duplicate,
    /// The registry has no free instance slot or request slot.
    #[error("gateway service capacity is exhausted")]
    CapacityExhausted,
    /// The exact key is not registered.
    #[error("gateway service instance is not registered")]
    NotFound,
}

/// Bounded registry of ready service VMs. The registry retains no worker
/// control handle and is not an authorization boundary.
#[derive(Clone)]
pub struct GatewayServiceRegistry {
    inner: Arc<RegistryInner>,
}

struct RegistryInner {
    entries: RwLock<HashMap<GatewayServiceInstanceKey, Arc<RegistryEntry>>>,
    instance_slots: Arc<Semaphore>,
    requests_per_instance: usize,
}

struct RegistryEntry {
    vm: Arc<dyn VmInstance>,
    state: watch::Receiver<ServiceWorkerState>,
    cancellation: CancellationToken,
    request_slots: Arc<Semaphore>,
    _instance_slot: OwnedSemaphorePermit,
}

impl GatewayServiceRegistry {
    /// Creates a registry with explicit bounded instance and request capacity.
    ///
    /// # Errors
    ///
    /// Returns an error when either capacity is outside the reviewed bound.
    pub fn new(
        instance_capacity: usize,
        requests_per_instance: usize,
    ) -> Result<Self, GatewayServiceRegistryError> {
        if !(1..=MAX_SERVICE_REGISTRY_CAPACITY).contains(&instance_capacity)
            || !(1..=MAX_SERVICE_REQUEST_CAPACITY).contains(&requests_per_instance)
        {
            return Err(GatewayServiceRegistryError::InvalidCapacity);
        }
        Ok(Self {
            inner: Arc::new(RegistryInner {
                entries: RwLock::new(HashMap::new()),
                instance_slots: Arc::new(Semaphore::new(instance_capacity)),
                requests_per_instance,
            }),
        })
    }

    /// Registers one ready VM under its exact fenced identity.
    ///
    /// # Errors
    ///
    /// Returns an error when the key, VM identity, readiness, duplicate, or
    /// registry capacity check fails.
    pub fn register(
        &self,
        key: GatewayServiceInstanceKey,
        vm: Arc<dyn VmInstance>,
        state: watch::Receiver<ServiceWorkerState>,
    ) -> Result<(), GatewayServiceRegistryError> {
        if !key.is_valid() {
            return Err(GatewayServiceRegistryError::InvalidKey);
        }
        let expected = format!("gateway-service-{}", key.identity.instance_id);
        if vm.id() != &VmId(expected) {
            return Err(GatewayServiceRegistryError::VmIdentityMismatch);
        }
        if state.has_changed().is_err() || *state.borrow() != ServiceWorkerState::Ready {
            return Err(GatewayServiceRegistryError::NotReady);
        }
        {
            let entries = self.read_entries();
            if entries
                .keys()
                .any(|existing| existing.identity.instance_id == key.identity.instance_id)
            {
                return Err(GatewayServiceRegistryError::Duplicate);
            }
        }
        let instance_slot = self
            .inner
            .instance_slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| GatewayServiceRegistryError::CapacityExhausted)?;
        let mut entries = self.write_entries();
        if entries
            .keys()
            .any(|existing| existing.identity.instance_id == key.identity.instance_id)
        {
            drop(instance_slot);
            return Err(GatewayServiceRegistryError::Duplicate);
        }
        entries.insert(
            key,
            Arc::new(RegistryEntry {
                vm,
                state,
                cancellation: CancellationToken::new(),
                request_slots: Arc::new(Semaphore::new(self.inner.requests_per_instance)),
                _instance_slot: instance_slot,
            }),
        );
        drop(entries);
        Ok(())
    }

    /// Unregisters exactly one fenced key and cancels its active exchanges.
    ///
    /// # Errors
    ///
    /// Returns [`GatewayServiceRegistryError::NotFound`] for a stale key.
    pub fn unregister(
        &self,
        key: GatewayServiceInstanceKey,
    ) -> Result<(), GatewayServiceRegistryError> {
        if !key.is_valid() {
            return Err(GatewayServiceRegistryError::InvalidKey);
        }
        let entry = {
            let mut entries = self.write_entries();
            entries
                .remove(&key)
                .ok_or(GatewayServiceRegistryError::NotFound)?
        };
        entry.cancellation.cancel();
        drop(entry);
        Ok(())
    }

    /// Exchanges one request through the exact fenced ready service instance.
    ///
    /// The registry performs admission only. Callers must authorize the
    /// request and fencing token through the durable control plane first.
    ///
    /// # Errors
    ///
    /// Returns a contract error for an invalid key or policy and an
    /// unavailable error for a missing, stopping, or saturated instance.
    // The entry is intentionally held through every await so unregister
    // cannot release its global instance slot before this request quiesces.
    #[allow(clippy::significant_drop_tightening)]
    pub async fn exchange(
        &self,
        key: GatewayServiceInstanceKey,
        request: GatewayRequest,
        policy: ServiceHttpPolicy,
    ) -> Result<GatewayResponse, GatewayEdgeError> {
        if !key.is_valid() {
            return Err(GatewayEdgeError::Contract("invalid service instance key"));
        }
        policy.validate()?;
        let entry = {
            let entries = self.read_entries();
            entries
                .get(&key)
                .cloned()
                .ok_or(GatewayEdgeError::HandlerUnavailable)?
        };
        let mut state = entry.state.clone();
        let _request_slot = entry
            .request_slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| GatewayEdgeError::HandlerUnavailable)?;
        if state.has_changed().is_err() || *state.borrow() != ServiceWorkerState::Ready {
            return Err(GatewayEdgeError::HandlerUnavailable);
        }
        let deadline = Instant::now()
            .checked_add(policy.exchange_timeout)
            .ok_or(GatewayEdgeError::HandlerUnavailable)?;
        let open = time::timeout_at(deadline, entry.vm.open_private_service_connection());
        tokio::pin!(open);
        let connection = loop {
            tokio::select! {
                () = entry.cancellation.cancelled() => {
                    return Err(GatewayEdgeError::HandlerUnavailable);
                }
                changed = state.changed() => {
                    if changed.is_err() || *state.borrow() != ServiceWorkerState::Ready {
                        return Err(GatewayEdgeError::HandlerUnavailable);
                    }
                }
                result = &mut open => {
                    break result
                        .map_err(|_| GatewayEdgeError::HandlerUnavailable)?
                        .map_err(|_| GatewayEdgeError::HandlerUnavailable)?;
                }
            }
        };
        if state.has_changed().is_err() || *state.borrow() != ServiceWorkerState::Ready {
            return Err(GatewayEdgeError::HandlerUnavailable);
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(GatewayEdgeError::HandlerUnavailable);
        }
        let mut exchange_policy = policy;
        exchange_policy.exchange_timeout = remaining;
        let exchange = time::timeout_at(
            deadline,
            exchange_private_service_http(connection, request, exchange_policy),
        );
        tokio::pin!(exchange);
        loop {
            tokio::select! {
                () = entry.cancellation.cancelled() => {
                    return Err(GatewayEdgeError::HandlerUnavailable);
                }
                changed = state.changed() => {
                    if changed.is_err() || *state.borrow() != ServiceWorkerState::Ready {
                        return Err(GatewayEdgeError::HandlerUnavailable);
                    }
                }
                result = &mut exchange => {
                    return result.map_err(|_| GatewayEdgeError::HandlerUnavailable)?;
                }
            }
        }
    }

    fn read_entries(
        &self,
    ) -> std::sync::RwLockReadGuard<'_, HashMap<GatewayServiceInstanceKey, Arc<RegistryEntry>>>
    {
        self.inner
            .entries
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn write_entries(
        &self,
    ) -> std::sync::RwLockWriteGuard<'_, HashMap<GatewayServiceInstanceKey, Arc<RegistryEntry>>>
    {
        self.inner
            .entries
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

#[cfg(test)]
#[path = "service_registry/tests.rs"]
mod tests;
