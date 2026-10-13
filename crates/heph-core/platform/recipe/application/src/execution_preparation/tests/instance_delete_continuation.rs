//! Controlled checked projections; no PG lineage, current authority or physical proof.
use self::fixture::{DeleteFixture, fresh_command};
use super::fixture as model;
use super::instance_delete_fixture as fixture;
use crate::{
    CommandIdentity, DeploymentOperation, InstanceClosurePublicationOrigin,
    PreparedInstanceDeleteContinuation, PreparedInstanceDeleteObservation,
    PreparedInstanceDeletePublication, PreparedInstanceDeletePublicationRecord, ResourceAction,
};
use identity_domain::RequestId;
use uuid::Uuid;

fn record(exact: &DeleteFixture) -> PreparedInstanceDeletePublicationRecord {
    let publisher = fresh_command();
    let observed = PreparedInstanceDeleteObservation::new(
        exact.request_for(publisher, 7).unwrap(),
        Uuid::from_u128(90),
        8,
    )
    .unwrap();
    PreparedInstanceDeletePublicationRecord::new(
        observed,
        publisher,
        publisher.id(),
        model::identity(13, 21).request_id,
    )
    .unwrap()
}

#[test]
fn unwritten_and_committed_are_distinct_and_preserve_original_before_cas() {
    let exact = DeleteFixture::new(ResourceAction::Delete);
    let request = exact.delete(7).unwrap();
    let unwritten = PreparedInstanceDeleteContinuation::unwritten(request.clone());
    assert!(matches!(
        unwritten.publication(),
        PreparedInstanceDeletePublication::Unwritten
    ));
    let committed =
        PreparedInstanceDeleteContinuation::committed(request.clone(), record(&exact)).unwrap();
    let PreparedInstanceDeletePublication::Committed(publication) = committed.publication() else {
        panic!("committed original lost")
    };
    assert_eq!(committed.original(), &request);
    assert_eq!(publication.terminal_operation_id(), Uuid::from_u128(90));
    assert_eq!(publication.before_instance_version(), 7);
    assert_eq!(publication.after_instance_version(), 8);
    assert_ne!(
        request.expected_instance_version(),
        request.effect().resource_version
    );
    assert_eq!(
        publication.observation().request().intent(),
        request.intent()
    );
    assert_eq!(
        publication.observation().request().target(),
        request.target()
    );
    assert_eq!(
        publication.observation().request().closure(),
        request.closure()
    );
    assert_eq!(
        publication.observation().request().effect(),
        request.effect()
    );
    assert_eq!(
        publication.observation().request().configuration(),
        request.configuration()
    );
}

#[test]
fn cleanup_command_can_change_without_replacing_delete_or_saved_publisher() {
    let exact = DeleteFixture::new(ResourceAction::Delete);
    let continuation =
        PreparedInstanceDeleteContinuation::committed(exact.delete(7).unwrap(), record(&exact))
            .unwrap();
    let PreparedInstanceDeletePublication::Committed(publication) = continuation.publication()
    else {
        panic!("saved publisher lost")
    };
    assert_eq!(publication.publisher(), fresh_command());
    assert_eq!(publication.native_command_id(), fresh_command().id());
    assert_eq!(
        publication.publication_request(),
        model::identity(13, 21).request_id
    );
    assert_ne!(publication.publisher(), exact.effect.command);
    assert_ne!(
        publication.publication_request(),
        exact.effect.provenance.request_id
    );
    let current =
        CommandIdentity::from_identity(&model::identity(17, 22), DeploymentOperation::Remove)
            .unwrap();
    for command in [exact.effect.command, current] {
        let rebuilt = continuation.rebuild_for_remove(command).unwrap();
        assert_eq!(rebuilt.current_command(), command);
        assert_eq!(rebuilt.expected_instance_version(), 7);
        assert_eq!(rebuilt.effect(), &exact.effect);
        assert_eq!(rebuilt.closure(), &exact.closure);
        assert_eq!(rebuilt.configuration(), &exact.configuration);
        assert_ne!(
            rebuilt.expected_instance_version(),
            publication.after_instance_version()
        );
    }
    assert_eq!(continuation.original().current_command(), exact.current);
    assert_eq!(publication.publisher(), fresh_command());
    let install =
        CommandIdentity::from_identity(&model::identity(17, 22), DeploymentOperation::Install)
            .unwrap();
    assert!(continuation.rebuild_for_remove(install).is_err());
}

#[test]
fn original_before_cas_and_exact_committed_successor_cannot_be_substituted() {
    let exact = DeleteFixture::new(ResourceAction::Delete);
    for before in [0, i64::MAX.cast_unsigned() + 1] {
        assert!(exact.delete(before).is_err());
    }
    for (before, after, operation) in [
        (7, 7, Uuid::from_u128(90)),
        (7, 9, Uuid::from_u128(90)),
        (7, 8, Uuid::nil()),
        (
            i64::MAX.cast_unsigned(),
            i64::MAX.cast_unsigned() + 1,
            Uuid::from_u128(90),
        ),
    ] {
        assert!(
            PreparedInstanceDeleteObservation::new(exact.delete(before).unwrap(), operation, after)
                .is_err()
        );
    }
    assert!(
        PreparedInstanceDeleteContinuation::committed(exact.delete(8).unwrap(), record(&exact))
            .is_err()
    );
    let maximal = PreparedInstanceDeleteContinuation::unwritten(
        exact.delete(i64::MAX.cast_unsigned()).unwrap(),
    );
    assert_eq!(
        maximal.original().expected_instance_version(),
        i64::MAX.cast_unsigned()
    );
}

#[test]
fn publication_rejects_wrong_publisher_native_id_or_nil_actual_request() {
    let exact = DeleteFixture::new(ResourceAction::Delete);
    let observation = record(&exact).observation().clone();
    let install =
        CommandIdentity::from_identity(&model::identity(13, 21), DeploymentOperation::Install)
            .unwrap();
    for (publisher, native, request) in [
        (
            exact.current,
            exact.current.id(),
            model::identity(13, 21).request_id,
        ),
        (install, install.id(), model::identity(13, 21).request_id),
        (
            fresh_command(),
            exact.current.id(),
            model::identity(13, 21).request_id,
        ),
        (
            fresh_command(),
            fresh_command().id(),
            RequestId::from_uuid(Uuid::nil()),
        ),
    ] {
        assert!(
            PreparedInstanceDeletePublicationRecord::new(
                observation.clone(),
                publisher,
                native,
                request
            )
            .is_err()
        );
    }
}

#[test]
fn committed_comparison_rejects_changed_full_claim_generation_owner_and_tag() {
    let original = DeleteFixture::new(ResourceAction::Delete);
    for change in 0..5 {
        let mut expected = DeleteFixture::new(ResourceAction::Delete);
        match change {
            0 => {
                expected.effect.generation += 1;
            }
            1 => {
                expected.effect.resource_version += 1;
            }
            2 => {
                expected.change_owner("foreign-namespace", "host-a");
            }
            3 => {
                expected.change_owner("actual-owner:opaque", "foreign-host");
            }
            _ => {
                expected.fresh_closure();
            }
        }
        let checked = expected.delete(7).unwrap();
        assert!(PreparedInstanceDeleteContinuation::committed(checked, record(&original)).is_err());
    }
    let mut tagged = DeleteFixture::new(ResourceAction::Delete);
    tagged.fresh_closure();
    let continuation =
        PreparedInstanceDeleteContinuation::committed(tagged.delete(7).unwrap(), record(&tagged))
            .unwrap();
    assert!(matches!(
        &continuation.original().closure().publication_origin,
        InstanceClosurePublicationOrigin::QualifiedFreshRemove(_)
    ));
    assert_eq!(continuation.original().closure(), &tagged.closure);
}

#[test]
fn full_intent_and_owned_delete_policy_remain_checked_before_projection() {
    let original = DeleteFixture::new(ResourceAction::Delete);
    let other = DeleteFixture::for_intent(
        model::intent(
            &model::SOURCE.replace(
                "kind = \"instance\"",
                "kind = \"instance\"\nremoval = \"delete\"",
            ),
            2,
        ),
        ResourceAction::Delete,
    );
    assert!(
        PreparedInstanceDeleteContinuation::committed(other.delete(7).unwrap(), record(&original))
            .is_err()
    );
    let retained =
        DeleteFixture::for_intent(model::intent(model::SOURCE, 1), ResourceAction::Delete);
    assert!(retained.delete(7).is_err());
    let detach = DeleteFixture::new(ResourceAction::Detach);
    assert!(detach.delete(7).is_err());
}
