use super::projection::event;
use crate::{
    application::event::{ApplicationEvent, EventScope, ScopeKind},
    event_cursor::EventCursorCodec,
};
use rpc_proto::messages::hephaestus::event::v1::{
    AggregateType, ChangeKind, EventScopeKind, LifecycleState, product_event,
};
use time::OffsetDateTime;
use uuid::Uuid;

#[test]
fn project_scoped_run_event_projects_the_typed_run_payload() {
    let project_id = Uuid::new_v4();
    let repository_id = Uuid::new_v4();
    let scope = EventScope {
        kind: ScopeKind::Project,
        id: project_id,
    };
    let projected = event(
        &EventCursorCodec::new([7; 32]),
        scope,
        &ApplicationEvent {
            id: Uuid::new_v4(),
            cursor: 1,
            aggregate_type: String::from("run"),
            aggregate_id: Uuid::new_v4(),
            aggregate_version: 1,
            event_type: String::from("run.changed"),
            schema_version: 1,
            change_kind: String::from("created"),
            safe_state: Some(String::from("queued")),
            related_id_one: Some(project_id),
            related_id_two: Some(repository_id),
            actor_id: None,
            request_id: None,
            occurred_at: OffsetDateTime::now_utc(),
        },
    )
    .expect("project run event should project");

    assert_eq!(
        projected.scope.as_option().map(|scope| scope.kind),
        Some(EventScopeKind::Project.into())
    );
    assert!(matches!(
        projected.payload,
        Some(product_event::Payload::RunChanged(payload))
            if payload.project_id.as_option().map(|id| id.value.clone())
                == Some(project_id.to_string())
                && payload.repository_id.as_option().map(|id| id.value.clone())
                    == Some(repository_id.to_string())
    ));
}

#[test]
fn project_scoped_registry_publication_event_projects_the_typed_payload() {
    let project_id = Uuid::new_v4();
    let projected = event(
        &EventCursorCodec::new([7; 32]),
        EventScope {
            kind: ScopeKind::Project,
            id: project_id,
        },
        &ApplicationEvent {
            id: Uuid::new_v4(),
            cursor: 1,
            aggregate_type: String::from("registry_publication"),
            aggregate_id: Uuid::new_v4(),
            aggregate_version: 1,
            event_type: String::from("registry.publication_changed"),
            schema_version: 1,
            change_kind: String::from("state_changed"),
            safe_state: Some(String::from("published")),
            related_id_one: None,
            related_id_two: None,
            actor_id: None,
            request_id: None,
            occurred_at: OffsetDateTime::now_utc(),
        },
    )
    .expect("registry publication event should project");

    assert_eq!(
        projected.aggregate_type.as_known(),
        Some(AggregateType::RegistryPublication)
    );
    assert!(matches!(
        projected.payload,
        Some(product_event::Payload::RegistryPublicationChanged(payload))
            if payload.change.as_known() == Some(ChangeKind::StateChanged)
                && payload.state.as_known() == Some(LifecycleState::Published)
    ));
}

#[test]
fn project_scoped_gateway_event_projects_only_the_safe_invalidation() {
    let project_id = Uuid::new_v4();
    let projected = event(
        &EventCursorCodec::new([7; 32]),
        EventScope {
            kind: ScopeKind::Project,
            id: project_id,
        },
        &ApplicationEvent {
            id: Uuid::new_v4(),
            cursor: 1,
            aggregate_type: String::from("gateway"),
            aggregate_id: Uuid::new_v4(),
            aggregate_version: 1,
            event_type: String::from("gateway.changed"),
            schema_version: 1,
            change_kind: String::from("state_changed"),
            safe_state: Some(String::from("paused")),
            related_id_one: None,
            related_id_two: None,
            actor_id: None,
            request_id: None,
            occurred_at: OffsetDateTime::now_utc(),
        },
    )
    .expect("gateway event should project");

    assert_eq!(
        projected.aggregate_type.as_known(),
        Some(AggregateType::Gateway)
    );
    assert!(matches!(
        projected.payload,
        Some(product_event::Payload::GatewayChanged(payload))
            if payload.change.as_known() == Some(ChangeKind::StateChanged)
                && payload.state.as_known() == Some(LifecycleState::Paused)
    ));
}
