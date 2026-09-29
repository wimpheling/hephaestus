use super::{
    Arc, CancellationToken, GatewayServiceCoordinator, GatewayServiceCoordinatorControl,
    GatewayServiceCoordinatorError, GatewayServiceCoordinatorStatus, GatewayServiceFailureStore,
    GatewayServiceInstanceLease, GatewayServiceInstanceState, GatewayServiceLaunchResolver,
    GatewayServiceLeaseMonitor, GatewayServiceLogWriterConfig, GatewayServiceOwner,
    GatewayServiceOwnership, GatewayServiceRegistry, GatewayServiceStartupIntent,
    GatewayServiceSupervisorPolicy, GatewayServiceTargetStore, Instant, VmProvider, watch,
};

impl GatewayServiceCoordinator {
    /// Creates a coordinator from an already claimed provisioning lease.
    ///
    /// Both deadlines must be captured before their respective blocking
    /// operations. The outer supervisor must retain and join [`Self::run`].
    ///
    /// # Errors
    ///
    /// Returns an error when the lease, owner, authority, or deadline policy
    /// is invalid.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        lease: GatewayServiceInstanceLease,
        owner: GatewayServiceOwner,
        initial_lease_deadline: Instant,
        startup_deadline: Instant,
        intent: GatewayServiceStartupIntent,
        ownership: Arc<dyn GatewayServiceOwnership>,
        failure_store: Arc<dyn GatewayServiceFailureStore>,
        resolver: Arc<dyn GatewayServiceLaunchResolver>,
        provider: Arc<dyn VmProvider>,
        targets: Arc<dyn GatewayServiceTargetStore>,
        registry: GatewayServiceRegistry,
        service_authority: impl Into<String>,
        supervisor_policy: GatewayServiceSupervisorPolicy,
    ) -> Result<(Self, GatewayServiceCoordinatorControl), GatewayServiceCoordinatorError> {
        let expected_vm = format!("gateway-service-{}", lease.identity.instance_id);
        if lease.state != GatewayServiceInstanceState::Provisioning
            || lease.identity.instance_id.is_nil()
            || lease.identity.gateway_id.is_nil()
            || lease.identity.revision_id.is_nil()
            || lease.fencing_token <= 0
            || lease.vm_id != expected_vm
            || startup_deadline <= Instant::now()
        {
            return Err(GatewayServiceCoordinatorError::InvalidIdentity);
        }
        owner
            .validate()
            .map_err(|_| GatewayServiceCoordinatorError::InvalidIdentity)?;
        supervisor_policy
            .validate()
            .map_err(|_| GatewayServiceCoordinatorError::InvalidLeasePolicy)?;
        let service_authority = service_authority.into();
        if service_authority.is_empty()
            || http::HeaderValue::try_from(service_authority.as_str()).is_err()
        {
            return Err(GatewayServiceCoordinatorError::InvalidAuthority);
        }
        let (lease_monitor, lease_control) = GatewayServiceLeaseMonitor::new(
            Arc::clone(&ownership),
            lease.clone(),
            owner.clone(),
            supervisor_policy.lease,
            initial_lease_deadline,
        )
        .map_err(|_| GatewayServiceCoordinatorError::InvalidLeasePolicy)?;
        let cancellation = CancellationToken::new();
        let (drain, drain_requested) = watch::channel(false);
        let (status, _) = watch::channel(GatewayServiceCoordinatorStatus::Preparing);
        let control = GatewayServiceCoordinatorControl {
            cancellation: cancellation.clone(),
            drain: drain.clone(),
            status: status.clone(),
        };
        Ok((
            Self {
                lease,
                owner,
                intent,
                ownership,
                failure_store,
                resolver,
                provider,
                targets,
                registry,
                service_authority,
                supervisor_policy,
                startup_deadline,
                lease_control,
                lease_monitor: Some(lease_monitor),
                log_writer: None,
                cancellation,
                drain,
                drain_requested,
                status,
            },
            control,
        ))
    }

    /// Attaches an optional parent-owned durable application-log writer.
    #[must_use]
    pub fn with_log_writer(mut self, config: GatewayServiceLogWriterConfig) -> Self {
        self.log_writer = Some(config);
        self
    }
}
