use super::*;

#[async_trait]

impl GatewayServiceLaunchResolver for FailLaunchResolver {
    async fn resolve_service_launch(
        &self,
        request: GatewayServiceLaunchRequest,
    ) -> Result<GatewayServiceLaunch, GatewayEdgeError> {
        if request.identity.revision_id == self.revision_id {
            return Err(GatewayEdgeError::Unavailable);
        }
        NoopLaunchResolver.resolve_service_launch(request).await
    }

    async fn cleanup_service_launch(
        &self,
        identity: gateway_edge::GatewayServiceIdentity,
    ) -> Result<(), GatewayEdgeError> {
        NoopLaunchResolver.cleanup_service_launch(identity).await
    }
}

#[async_trait]
impl GatewayServiceLaunchResolver for NoopLaunchResolver {
    async fn resolve_service_launch(
        &self,
        request: GatewayServiceLaunchRequest,
    ) -> Result<GatewayServiceLaunch, GatewayEdgeError> {
        let service = GatewayServiceConfig::new(
            18_080,
            ServiceProbePath::parse("/ready").expect("readiness path"),
            ServiceProbePath::parse("/health").expect("health path"),
        )
        .expect("service config");
        Ok(GatewayServiceLaunch {
            identity: request.identity,
            service,
            spec: VmSpec {
                id: VmId(format!("gateway-service-{}", request.identity.instance_id)),
                root: RootFilesystem::Directory {
                    host_path: PathBuf::from("/tmp"),
                },
                disks: Vec::new(),
                mounts: Vec::new(),
                resources: VmResources {
                    vcpus: 1,
                    memory_mib: 64,
                },
                network: NetworkMode::Disabled,
                private_http_service: Some(PrivateHttpServiceSpec {
                    loopback_port: 18_080,
                    max_connections: 32,
                    connect_timeout: StdDuration::from_secs(2),
                }),
                command: GuestCommand {
                    program: String::from("/service"),
                    args: Vec::new(),
                    env: BTreeMap::new(),
                    working_dir: None,
                },
                runtime_authority: None,
                runtime_git_bridge: None,
                labels: BTreeMap::new(),
            },
        })
    }

    async fn cleanup_service_launch(
        &self,
        _identity: gateway_edge::GatewayServiceIdentity,
    ) -> Result<(), GatewayEdgeError> {
        Ok(())
    }
}

#[async_trait]
impl GatewayServiceLaunchResolver for ApplicationLogLaunchResolver {
    async fn resolve_service_launch(
        &self,
        request: GatewayServiceLaunchRequest,
    ) -> Result<GatewayServiceLaunch, GatewayEdgeError> {
        let mut launch = NoopLaunchResolver.resolve_service_launch(request).await?;
        launch.service = launch
            .service
            .with_log_capture_mode(gateway_domain::ServiceLogCaptureMode::Application);
        Ok(launch)
    }

    async fn cleanup_service_launch(
        &self,
        identity: gateway_edge::GatewayServiceIdentity,
    ) -> Result<(), GatewayEdgeError> {
        NoopLaunchResolver.cleanup_service_launch(identity).await
    }
}

#[async_trait]
impl GatewayServiceLaunchResolver for RecordingLaunchResolver {
    async fn resolve_service_launch(
        &self,
        request: GatewayServiceLaunchRequest,
    ) -> Result<GatewayServiceLaunch, GatewayEdgeError> {
        NoopLaunchResolver.resolve_service_launch(request).await
    }

    async fn cleanup_service_launch(
        &self,
        identity: gateway_edge::GatewayServiceIdentity,
    ) -> Result<(), GatewayEdgeError> {
        self.cleanup_calls.fetch_add(1, Ordering::AcqRel);
        NoopLaunchResolver.cleanup_service_launch(identity).await
    }
}

#[async_trait]
impl GatewayProvider for BlockingCaddyProvider {
    async fn reconcile(
        &self,
        desired: &GatewayDesiredConfiguration,
    ) -> Result<GatewayConfigRevision, GatewayEdgeError> {
        self.reconciles.fetch_add(1, Ordering::Release);
        self.started.notify_one();
        self.release.notified().await;
        Ok(desired.revision)
    }

    async fn forward(&self, _request: GatewayRequest) -> GatewayProviderResponse {
        GatewayProviderResponse {
            response: GatewayResponse {
                status: StatusCode::INTERNAL_SERVER_ERROR,
                headers: HeaderMap::new(),
                body: Bytes::new(),
                mailbox_publication: None,
            },
            invocation_id: Uuid::new_v4(),
        }
    }
}

#[async_trait]
impl GatewayProvider for RecoveryProvider {
    async fn reconcile(
        &self,
        desired: &GatewayDesiredConfiguration,
    ) -> Result<GatewayConfigRevision, GatewayEdgeError> {
        self.reconciles.fetch_add(1, Ordering::Release);
        Ok(desired.revision)
    }

    async fn forward(&self, _request: GatewayRequest) -> GatewayProviderResponse {
        GatewayProviderResponse {
            response: GatewayResponse {
                status: StatusCode::INTERNAL_SERVER_ERROR,
                headers: HeaderMap::new(),
                body: Bytes::new(),
                mailbox_publication: None,
            },
            invocation_id: Uuid::new_v4(),
        }
    }
}
