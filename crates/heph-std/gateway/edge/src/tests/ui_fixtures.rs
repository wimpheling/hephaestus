use super::*;

pub(super) struct UiProvider {
    pub(super) admission: UiGatewayAdmission,
}

#[async_trait]
impl UiGatewayAdmissionProvider for UiProvider {
    async fn admit(
        &self,
        _: &UiGatewayRequest,
    ) -> Result<UiGatewayAdmission, UiGatewayAdmissionError> {
        Ok(self.admission.clone())
    }
}

pub(super) struct DenyingUiProvider {
    pub(super) calls: Arc<AtomicUsize>,
}

#[async_trait]
impl UiGatewayAdmissionProvider for DenyingUiProvider {
    async fn admit(
        &self,
        _: &UiGatewayRequest,
    ) -> Result<UiGatewayAdmission, UiGatewayAdmissionError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(UiGatewayAdmissionError::Denied)
    }
}

pub(super) struct ErrorUiProvider(pub(super) UiGatewayAdmissionError);

#[async_trait]
impl UiGatewayAdmissionProvider for ErrorUiProvider {
    async fn admit(
        &self,
        _: &UiGatewayRequest,
    ) -> Result<UiGatewayAdmission, UiGatewayAdmissionError> {
        Err(self.0)
    }
}

pub(super) struct UiRecorder {
    pub(super) accepted: Arc<AtomicUsize>,
    pub(super) completed: Arc<Mutex<Vec<GatewayInvocationOutcome>>>,
}

#[async_trait]
impl GatewayInvocationRecorder for UiRecorder {
    async fn accepted(&self, _: &GatewayRouteBinding, _: Uuid) -> Result<Uuid, GatewayEdgeError> {
        Ok(Uuid::new_v4())
    }

    async fn accepted_ui(
        &self,
        route: &GatewayRouteBinding,
        authority: &UiGatewayAuthority,
        _: Uuid,
    ) -> Result<Uuid, GatewayEdgeError> {
        assert_eq!(route.exposure, Exposure::HephAuthenticated);
        assert!(!authority.child_session_id.is_nil());
        self.accepted.fetch_add(1, Ordering::SeqCst);
        Ok(Uuid::new_v4())
    }

    async fn completed(
        &self,
        _: Uuid,
        outcome: GatewayInvocationOutcome,
    ) -> Result<(), GatewayEdgeError> {
        self.completed
            .lock()
            .expect("completion lock")
            .push(outcome);
        Ok(())
    }
}

pub(super) struct SetCookieHandler;

#[async_trait]
impl GatewayVmHandler for SetCookieHandler {
    async fn invoke(
        &self,
        _: &GatewayRouteBinding,
        _: Uuid,
        request: GatewayRequest,
    ) -> Result<GatewayResponse, GatewayEdgeError> {
        for name in [
            "authorization",
            "cookie",
            "host",
            "proxy-authorization",
            "x-api-key",
            "x-auth-token",
            "x-access-token",
            "forwarded",
            "x-forwarded-for",
            "x-forwarded-host",
            "x-forwarded-proto",
            "x-forwarded-port",
            "x-forwarded-prefix",
            "x-forwarded-server",
        ] {
            assert!(
                !request.headers.contains_key(name),
                "leaked UI header: {name}"
            );
        }
        let mut headers = HeaderMap::new();
        headers.insert("set-cookie", HeaderValue::from_static("sid=guest"));
        Ok(GatewayResponse {
            status: StatusCode::OK,
            headers,
            body: Bytes::new(),
            mailbox_publication: None,
        })
    }
}

pub(super) struct ForbiddenHandler;

#[async_trait]
impl GatewayVmHandler for ForbiddenHandler {
    async fn invoke(
        &self,
        _: &GatewayRouteBinding,
        _: Uuid,
        _: GatewayRequest,
    ) -> Result<GatewayResponse, GatewayEdgeError> {
        Ok(GatewayResponse {
            status: StatusCode::FORBIDDEN,
            headers: HeaderMap::new(),
            body: Bytes::from_static(b"denied"),
            mailbox_publication: None,
        })
    }
}

pub(super) fn ui_request(path: &str) -> UiGatewayRequest {
    UiGatewayRequest {
        authority: UiGatewayAuthority {
            child_session_id: Uuid::new_v4(),
            actor_id: Uuid::new_v4(),
            organization_id: Uuid::new_v4(),
            installation_id: Uuid::new_v4(),
            generation_id: Uuid::new_v4(),
            canonical_request_path: "echo/index.html".to_owned(),
            request_kind: UiGatewayRequestKind::Managed,
            method: Method::GET,
        },
        method: Method::GET,
        request_path_and_query: path.to_owned(),
        headers: {
            let mut headers = HeaderMap::new();
            headers.insert("authorization", HeaderValue::from_static("secret"));
            headers.insert("cookie", HeaderValue::from_static("sid=secret"));
            headers
        },
        body: Bytes::new(),
        trusted: TrustedRequestMetadata {
            scheme: GatewayScheme::Https,
            authority: "ui.heph.test".to_owned(),
            client_address: "127.0.0.1".parse().expect("address"),
            request_id: Uuid::new_v4(),
        },
    }
}

pub(super) fn ui_admission() -> UiGatewayAdmission {
    let mut route = route();
    route.exposure = Exposure::HephAuthenticated;
    route.methods = BTreeSet::from([Method::GET]);
    UiGatewayAdmission {
        route,
        gateway_path_and_query: "/gateway/echo/index.html".to_owned(),
    }
}
