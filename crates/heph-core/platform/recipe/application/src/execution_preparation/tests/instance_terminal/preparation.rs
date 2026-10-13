//! Controlled initial comparison inputs; no native or physical positive proof.
use super::{TerminalFixture, denied, fresh_command, preparation};
use crate::{
    DeploymentError, PreparedInstanceTerminalPreparationInput,
    PreparedInstanceTerminalPreparationRepository, ResourceAction,
};
use async_trait::async_trait;
use release_domain::ContentHash;
use uuid::Uuid;

fn input(
    exact: &TerminalFixture,
) -> Result<PreparedInstanceTerminalPreparationInput, DeploymentError> {
    PreparedInstanceTerminalPreparationInput::new(
        exact.intent.clone(),
        exact.target.clone(),
        exact.effect.clone(),
        exact.current,
        exact.configuration.clone(),
    )
}
#[test]
fn preparation_keeps_original_effect_and_real_current_command_without_inventing_closure_or_cas() {
    for action in [ResourceAction::Detach, ResourceAction::Delete] {
        let mut exact = TerminalFixture::new(action);
        let original = input(&exact).unwrap();
        exact.current = fresh_command();
        let fresh = input(&exact).unwrap();
        assert_eq!(fresh.intent(), &exact.intent);
        assert_eq!(fresh.target(), &exact.target);
        assert_eq!(fresh.effect(), original.effect());
        assert_eq!(fresh.current_command(), exact.current);
        assert_eq!(fresh.configuration(), &exact.configuration);
        assert_ne!(fresh.current_command(), fresh.effect().command);
    }
}
#[test]
fn preparation_denies_foreign_originals_wrong_action_and_changed_provenance() {
    for change in 0..10 {
        let mut exact = TerminalFixture::new(ResourceAction::Delete);
        match change {
            0 => exact.intent = preparation::intent(preparation::SOURCE, 2),
            1 => exact.effect.input_hash = ContentHash::digest(b"foreign plan"),
            2 => exact.effect.action = ResourceAction::Drain,
            3 => exact.effect.generation = 0,
            4 => exact.effect.provenance.actor_id = preparation::identity(20, 21).user_id,
            5 => {
                exact.effect.provenance.request_id =
                    identity_domain::RequestId::from_uuid(Uuid::nil());
            }
            6 => exact.effect.provenance.attempt_id = exact.target.original().provenance.attempt_id,
            7 => exact.current = preparation::command(),
            8 => {
                exact.configuration = crate::RemoveResumeConfiguration::new(
                    "foreign".into(),
                    exact.target.host_id().into(),
                    exact.configuration.platform().clone(),
                    exact.configuration.profile(),
                )
                .unwrap();
            }
            _ => exact.effect.resource_version = i64::MAX.cast_unsigned() + 1,
        }
        assert!(input(&exact).is_err(), "substitution {change}");
    }
}
struct Unsupported;
#[async_trait]
impl PreparedInstanceTerminalPreparationRepository for Unsupported {}
#[test]
fn unsupported_preparation_never_falls_back_to_a_guessed_version_or_closure() {
    let identity = preparation::identity(11, 12);
    let port = Unsupported;
    let detach = input(&TerminalFixture::new(ResourceAction::Detach)).unwrap();
    let delete = input(&TerminalFixture::new(ResourceAction::Delete)).unwrap();
    denied(port.prepare_detach(&identity, &detach));
    denied(port.prepare_delete(&identity, &delete));
}
