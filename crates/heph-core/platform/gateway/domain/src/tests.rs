use capability_domain::{CapabilityOperation, CapabilityResourceKind, CapabilitySlotKey};
use serde::Serialize;
use sha2::{Digest, Sha256};

use super::*;

#[derive(Serialize)]
struct LegacyDeclaration<'a> {
    name: &'a GatewayName,
    agent_name: &'a str,
    handler_contract: &'a str,
    exposure: Exposure,
    routes: &'a Vec<RouteIntent>,
    parameters: &'a serde_json::Value,
    secret_slots: &'a Vec<String>,
    mailbox_publication_slots: &'a Vec<GatewayMailboxPublicationSlot>,
}

#[test]
fn rejects_ambiguous_paths() {
    assert!(RoutePath::parse("/a/../b").is_err());
    assert!(RoutePath::parse("/a%2fb").is_err());
}

#[test]
fn validates_service_configuration_and_contract_pairing() {
    let service = GatewayServiceConfig::new(
        8080,
        ServiceProbePath::parse("/ready").unwrap(),
        ServiceProbePath::parse("/health").unwrap(),
    )
    .unwrap();
    let declaration = GatewayDeclaration {
        name: GatewayName::parse("service").unwrap(),
        agent_name: String::from("service-handler"),
        handler_contract: HTTP_SERVICE_HANDLER_CONTRACT_V1.into(),
        service: Some(service),
        exposure: Exposure::Public,
        routes: vec![
            RouteIntent::new(RoutePath::parse("/service").unwrap(), [HttpMethod::Get]).unwrap(),
        ],
        parameters: serde_json::json!({}),
        secret_slots: vec![],
        mailbox_publication_slots: vec![],
    };
    assert!(declaration.validate().is_ok());

    let mut missing_service = declaration.clone();
    missing_service.service = None;
    assert_eq!(
        missing_service.validate(),
        Err(GatewayError::ServiceConfigurationRequired)
    );

    let mut service_with_mailbox = declaration.clone();
    service_with_mailbox.mailbox_publication_slots = vec![
        GatewayMailboxPublicationSlot::new(
            CapabilitySlotKey::parse("deliver").unwrap(),
            "Deliver an accepted event.",
        )
        .unwrap(),
    ];
    assert_eq!(
        service_with_mailbox.validate(),
        Err(GatewayError::ServiceMailboxPublicationForbidden)
    );

    let mut stateless_with_service = declaration;
    stateless_with_service.handler_contract = HTTP_HANDLER_CONTRACT_V1.into();
    assert_eq!(
        stateless_with_service.validate(),
        Err(GatewayError::ServiceConfigurationForbidden)
    );
}

#[test]
fn rejects_privileged_service_ports_and_ambiguous_service_paths() {
    assert_eq!(
        GatewayServiceConfig::new(
            1023,
            ServiceProbePath::parse("/ready").unwrap(),
            ServiceProbePath::parse("/health").unwrap(),
        ),
        Err(GatewayError::InvalidServicePort)
    );
    assert!(ServiceProbePath::parse("/").is_ok());
    for invalid in [
        "/ready?probe",
        "/ready%2fprobe",
        "/ready//probe",
        "/ready/",
        "/ready/../probe",
        "/ready\\probe",
        "/ready probe",
        "/ready\nprobe",
    ] {
        assert!(ServiceProbePath::parse(invalid).is_err(), "{invalid:?}");
    }
    let invalid_json =
        r#"{"loopback_port":8080,"readiness_path":"/ready?probe","health_path":"/health"}"#;
    assert!(serde_json::from_str::<GatewayServiceConfig>(invalid_json).is_err());
    let root_json = r#"{"loopback_port":8080,"readiness_path":"/","health_path":"/health"}"#;
    assert!(serde_json::from_str::<GatewayServiceConfig>(root_json).is_ok());
}

#[test]
fn stateless_service_field_is_omitted_from_legacy_hash_serialization() {
    let declaration = GatewayDeclaration {
        name: GatewayName::parse("telegram").unwrap(),
        agent_name: String::from("telegram-handler"),
        handler_contract: HTTP_HANDLER_CONTRACT_V1.into(),
        service: None,
        exposure: Exposure::Public,
        routes: vec![
            RouteIntent::new(RoutePath::parse("/telegram").unwrap(), [HttpMethod::Post]).unwrap(),
        ],
        parameters: serde_json::json!({"enabled": true}),
        secret_slots: vec!["telegram_secret".into()],
        mailbox_publication_slots: vec![],
    };
    let expected: [u8; 32] = Sha256::digest(
        serde_json::to_vec(&LegacyDeclaration {
            name: &declaration.name,
            agent_name: &declaration.agent_name,
            handler_contract: &declaration.handler_contract,
            exposure: declaration.exposure,
            routes: &declaration.routes,
            parameters: &declaration.parameters,
            secret_slots: &declaration.secret_slots,
            mailbox_publication_slots: &declaration.mailbox_publication_slots,
        })
        .unwrap(),
    )
    .into();
    assert_eq!(declaration.validate().unwrap(), expected);
    assert!(
        !serde_json::to_string(&declaration)
            .unwrap()
            .contains("service")
    );
}

#[test]
fn service_log_capture_default_preserves_legacy_hash_and_opt_in_changes_it() {
    let service = GatewayServiceConfig::new(
        8080,
        ServiceProbePath::parse("/ready").unwrap(),
        ServiceProbePath::parse("/health").unwrap(),
    )
    .unwrap();
    let declaration = GatewayDeclaration {
        name: GatewayName::parse("service").unwrap(),
        agent_name: String::from("service-handler"),
        handler_contract: HTTP_SERVICE_HANDLER_CONTRACT_V1.into(),
        service: Some(service),
        exposure: Exposure::Public,
        routes: vec![
            RouteIntent::new(RoutePath::parse("/service").unwrap(), [HttpMethod::Get]).unwrap(),
        ],
        parameters: serde_json::json!({}),
        secret_slots: vec![],
        mailbox_publication_slots: vec![],
    };
    let omitted_json = serde_json::to_vec(&declaration).unwrap();
    let omitted_hash = declaration.validate().unwrap();
    let frozen_json = br#"{"name":"service","agent_name":"service-handler","handler_contract":"http.service.v1","service":{"loopback_port":8080,"readiness_path":"/ready","health_path":"/health"},"exposure":"public","routes":[{"path":"/service","methods":["GET"]}],"parameters":{},"secret_slots":[],"mailbox_publication_slots":[]}"#;
    let frozen_hash: [u8; 32] = [
        0x76, 0x72, 0xda, 0x47, 0x84, 0x24, 0xac, 0x3c, 0xe3, 0x3b, 0x1b, 0xc1, 0x99, 0x51, 0xa9,
        0x49, 0x70, 0x88, 0x43, 0xf6, 0x38, 0x2d, 0x42, 0x74, 0xcf, 0xe9, 0x19, 0xe9, 0x1d, 0xcf,
        0xe1, 0xc8,
    ];
    assert_eq!(omitted_json, frozen_json);
    assert_eq!(omitted_hash, frozen_hash);

    let mut explicit_disabled = declaration.clone();
    explicit_disabled.service.as_mut().unwrap().log_capture_mode = ServiceLogCaptureMode::Disabled;
    assert_eq!(omitted_hash, explicit_disabled.validate().unwrap());
    assert_eq!(
        omitted_json,
        serde_json::to_vec(&explicit_disabled).unwrap()
    );
    assert!(
        !String::from_utf8(omitted_json)
            .unwrap()
            .contains("log_capture_mode")
    );

    let mut application = declaration;
    application.service.as_mut().unwrap().log_capture_mode = ServiceLogCaptureMode::Application;
    assert_ne!(omitted_hash, application.validate().unwrap());
    assert!(
        serde_json::to_string(&application)
            .unwrap()
            .contains("application")
    );
}

#[test]
fn rejects_unknown_service_log_capture_mode() {
    let source = r#"{
            "loopback_port": 8080,
            "readiness_path": "/ready",
            "health_path": "/health",
            "log_capture_mode": "future"
        }"#;
    assert!(serde_json::from_str::<GatewayServiceConfig>(source).is_err());
}

#[test]
fn validates_a_gateway() {
    let declaration = GatewayDeclaration {
        name: GatewayName::parse("telegram").unwrap(),
        agent_name: String::from("telegram-handler"),
        handler_contract: HTTP_HANDLER_CONTRACT_V1.into(),
        service: None,
        exposure: Exposure::Public,
        routes: vec![
            RouteIntent::new(RoutePath::parse("/telegram").unwrap(), [HttpMethod::Post]).unwrap(),
        ],
        parameters: serde_json::json!({}),
        secret_slots: vec!["telegram_secret".into()],
        mailbox_publication_slots: vec![
            GatewayMailboxPublicationSlot::new(
                CapabilitySlotKey::parse("agent_mailbox").unwrap(),
                "Deliver accepted gateway updates to the agent.",
            )
            .unwrap(),
        ],
    };
    assert!(declaration.validate().is_ok());
    let slot = &declaration.mailbox_publication_slots[0];
    assert_eq!(slot.resource_kind(), CapabilityResourceKind::Mailbox);
    assert_eq!(slot.required_operation(), CapabilityOperation::Publish);
    assert!(slot.required());
}

#[test]
fn rejects_duplicate_mailbox_publication_slots() {
    let slot = GatewayMailboxPublicationSlot::new(
        CapabilitySlotKey::parse("agent_mailbox").unwrap(),
        "Deliver accepted gateway updates to the agent.",
    )
    .unwrap();
    let declaration = GatewayDeclaration {
        name: GatewayName::parse("telegram").unwrap(),
        agent_name: String::from("telegram-handler"),
        handler_contract: HTTP_HANDLER_CONTRACT_V1.into(),
        service: None,
        exposure: Exposure::Public,
        routes: vec![
            RouteIntent::new(RoutePath::parse("/telegram").unwrap(), [HttpMethod::Post]).unwrap(),
        ],
        parameters: serde_json::json!({}),
        secret_slots: vec![],
        mailbox_publication_slots: vec![slot.clone(), slot],
    };
    assert_eq!(
        declaration.validate(),
        Err(GatewayError::DuplicateMailboxPublicationSlot)
    );
}
