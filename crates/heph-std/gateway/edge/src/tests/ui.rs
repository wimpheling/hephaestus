use super::*;
#[path = "ui_fixtures.rs"]
mod fixtures;
use fixtures::*;

#[tokio::test]
async fn ui_set_cookie_is_failed_and_completed_after_ui_acceptance() {
    let accepted = Arc::new(AtomicUsize::new(0));
    let completed = Arc::new(Mutex::new(Vec::new()));
    let recorder = UiRecorder {
        accepted: Arc::clone(&accepted),
        completed: Arc::clone(&completed),
    };
    let dispatcher =
        GatewayDispatcher::new(Resolver(ui_admission().route), SetCookieHandler, recorder);
    let result = dispatcher
        .dispatch_ui_detailed(
            ui_request("echo/index.html"),
            &UiProvider {
                admission: ui_admission(),
            },
        )
        .await;
    assert_eq!(result.response.response.status, StatusCode::BAD_GATEWAY);
    assert_eq!(
        result.disposition,
        UiDispatchDisposition::Admitted {
            outcome: GatewayInvocationOutcome::Failed,
            completion_persisted: true,
        }
    );
    assert_eq!(accepted.load(Ordering::SeqCst), 1);
    assert_eq!(
        completed.lock().expect("completion lock").as_slice(),
        &[GatewayInvocationOutcome::Failed]
    );
}

#[tokio::test]
async fn ui_authority_path_mismatch_fails_before_ui_acceptance() {
    let accepted = Arc::new(AtomicUsize::new(0));
    let completed = Arc::new(Mutex::new(Vec::new()));
    let recorder = UiRecorder {
        accepted: Arc::clone(&accepted),
        completed: Arc::clone(&completed),
    };
    let dispatcher =
        GatewayDispatcher::new(Resolver(ui_admission().route), SetCookieHandler, recorder);
    let result = dispatcher
        .dispatch_ui_detailed(
            ui_request("echo/other.html"),
            &UiProvider {
                admission: ui_admission(),
            },
        )
        .await;
    assert_eq!(result.response.response.status, StatusCode::BAD_REQUEST);
    assert_eq!(result.disposition, UiDispatchDisposition::StructuralInvalid);
    assert_eq!(accepted.load(Ordering::SeqCst), 0);
    assert!(completed.lock().expect("completion lock").is_empty());
}

#[tokio::test]
async fn ui_acceptance_is_default_deny_for_existing_recorders() {
    let calls = Arc::new(AtomicUsize::new(0));
    let dispatcher = GatewayDispatcher::new(
        Resolver(ui_admission().route),
        CountingHandler(Arc::clone(&calls)),
        Recorder,
    );
    let result = dispatcher
        .dispatch_ui_detailed(
            ui_request("echo/index.html"),
            &UiProvider {
                admission: ui_admission(),
            },
        )
        .await;
    assert_eq!(
        result.response.response.status,
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(result.disposition, UiDispatchDisposition::AcceptedUiFailure);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn ui_denial_provider_is_called_once_before_handler() {
    let provider_calls = Arc::new(AtomicUsize::new(0));
    let handler_calls = Arc::new(AtomicUsize::new(0));
    let dispatcher = GatewayDispatcher::new(
        Resolver(ui_admission().route),
        CountingHandler(Arc::clone(&handler_calls)),
        Recorder,
    );
    let result = dispatcher
        .dispatch_ui_detailed(
            ui_request("echo/index.html"),
            &DenyingUiProvider {
                calls: Arc::clone(&provider_calls),
            },
        )
        .await;
    assert_eq!(result.response.response.status, StatusCode::UNAUTHORIZED);
    assert_eq!(result.disposition, UiDispatchDisposition::ProviderDenied);
    assert_eq!(provider_calls.load(Ordering::SeqCst), 1);
    assert_eq!(handler_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn ui_provider_classes_are_preserved_without_status_inference() {
    let dispatcher = GatewayDispatcher::new(
        Resolver(ui_admission().route),
        CountingHandler(Arc::new(AtomicUsize::new(0))),
        Recorder,
    );
    for (error, expected) in [
        (
            UiGatewayAdmissionError::NotFound,
            UiDispatchDisposition::ProviderNotFound,
        ),
        (
            UiGatewayAdmissionError::Unavailable,
            UiDispatchDisposition::ProviderUnavailable,
        ),
    ] {
        let result = dispatcher
            .dispatch_ui_detailed(ui_request("echo/index.html"), &ErrorUiProvider(error))
            .await;
        assert_eq!(result.disposition, expected);
    }
}

#[tokio::test]
async fn ui_guest_forbidden_is_completed_not_provider_denied() {
    let completed = Arc::new(Mutex::new(Vec::new()));
    let recorder = UiRecorder {
        accepted: Arc::new(AtomicUsize::new(0)),
        completed: Arc::clone(&completed),
    };
    let dispatcher =
        GatewayDispatcher::new(Resolver(ui_admission().route), ForbiddenHandler, recorder);
    let result = dispatcher
        .dispatch_ui_detailed(
            ui_request("echo/index.html"),
            &UiProvider {
                admission: ui_admission(),
            },
        )
        .await;
    assert_eq!(result.response.response.status, StatusCode::FORBIDDEN);
    assert_eq!(
        result.disposition,
        UiDispatchDisposition::Admitted {
            outcome: GatewayInvocationOutcome::Completed,
            completion_persisted: true,
        }
    );
}

#[tokio::test]
async fn ui_rejected_inbound_secret_is_admitted_and_completed_as_rejected() {
    let completed = Arc::new(Mutex::new(Vec::new()));
    let recorder = UiRecorder {
        accepted: Arc::new(AtomicUsize::new(0)),
        completed: Arc::clone(&completed),
    };
    let dispatcher = GatewayDispatcher::new(Resolver(ui_admission().route), EchoHandler, recorder)
        .with_inbound_secret_resolver(Arc::new(InboundRules));
    let mut request = ui_request("echo/index.html");
    request
        .headers
        .insert("x-hook-secret", HeaderValue::from_static("wrong-secret"));
    let result = dispatcher
        .dispatch_ui_detailed(
            request,
            &UiProvider {
                admission: ui_admission(),
            },
        )
        .await;
    assert_eq!(result.response.response.status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        result.disposition,
        UiDispatchDisposition::Admitted {
            outcome: GatewayInvocationOutcome::Rejected,
            completion_persisted: true,
        }
    );
    assert_eq!(
        completed.lock().expect("completion lock").as_slice(),
        &[GatewayInvocationOutcome::Rejected]
    );
}

#[tokio::test]
async fn ui_completion_store_failure_is_admitted_but_not_persisted() {
    let recorder = CompletionFailureRecorder {
        completions: AtomicUsize::new(0),
    };
    let dispatcher = GatewayDispatcher::new(Resolver(ui_admission().route), EchoHandler, recorder);
    let result = dispatcher
        .dispatch_ui_detailed(
            ui_request("echo/index.html"),
            &UiProvider {
                admission: ui_admission(),
            },
        )
        .await;
    assert_eq!(
        result.response.response.status,
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(
        result.disposition,
        UiDispatchDisposition::Admitted {
            outcome: GatewayInvocationOutcome::Completed,
            completion_persisted: false,
        }
    );
}

#[tokio::test]
async fn ui_success_exposes_safe_admitted_metadata() {
    let dispatcher = GatewayDispatcher::new(
        Resolver(ui_admission().route),
        EchoHandler,
        UiRecorder {
            accepted: Arc::new(AtomicUsize::new(0)),
            completed: Arc::new(Mutex::new(Vec::new())),
        },
    );
    let result = dispatcher
        .dispatch_ui_detailed(
            ui_request("echo/index.html"),
            &UiProvider {
                admission: ui_admission(),
            },
        )
        .await;
    assert_eq!(result.response.response.status, StatusCode::ACCEPTED);
    assert_ne!(result.response.invocation_id, Uuid::nil());
    assert_eq!(
        result.disposition,
        UiDispatchDisposition::Admitted {
            outcome: GatewayInvocationOutcome::Completed,
            completion_persisted: true,
        }
    );
}
