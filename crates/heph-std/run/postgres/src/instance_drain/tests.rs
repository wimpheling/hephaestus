//! Pure boundary mutations only; these rows are not genuine native originals.

use runtime_types::AgentInstanceId;
use serde_json::{Value, json};
use uuid::Uuid;

use super::{
    InstanceDrainOriginal,
    decode::{self, Row},
};

struct Comparison {
    instance: AgentInstanceId,
    drain: Uuid,
    create: Uuid,
    drain_claim: Value,
    create_claim: Value,
    prepared: Value,
    closure: Value,
}

impl Comparison {
    fn new() -> Self {
        let drain = Uuid::new_v4();
        let create = Uuid::new_v4();
        let command = Uuid::new_v4();
        Self {
            instance: AgentInstanceId::new(),
            drain,
            create,
            drain_claim: json!({"attempt_id":drain,"command_id":command}),
            create_claim: json!({"attempt_id":create,"command_id":Uuid::new_v4()}),
            prepared: json!({}),
            closure: json!({"drain_attempt":drain,"drain_command":command,
                "removal_id":Uuid::new_v4(),"command_key":vec![0u8;32],"input_hash":vec![0u8;32],
                "expected_version":1,"creator":Uuid::new_v4(),"request":Uuid::new_v4()}),
        }
    }

    fn input(&self) -> InstanceDrainOriginal<'_> {
        InstanceDrainOriginal {
            instance_id: self.instance,
            drain_attempt: self.drain,
            drain_claim: &self.drain_claim,
            create_attempt: self.create,
            create_claim: &self.create_claim,
            prepared: &self.prepared,
        }
    }

    fn row(&self) -> Row {
        let run = Uuid::new_v4();
        Row {
            run_id: run,
            cancel_json: Some(json!({"command_id":Uuid::new_v4(),"run_id":run,
            "reason":"instance removal requested"})),
            original: self.closure.clone(),
        }
    }

    fn ordinary(&self) -> Row {
        let mut row = self.row();
        let control = Uuid::new_v4();
        let drain = row.cancel_json.as_ref().unwrap();
        row.original = json!({"origin":"ordinary_cancel140", "controls":[{"id":control,"run_id":row.run_id}],
            "cancels":[{"command_id":control,"run_id":row.run_id,"reason":"actual ordinary reason"},drain],
            "removal":self.closure,"drain":self.closure,"drain_cancel":drain});
        row
    }
}

#[test]
fn trailing_invalid_row_cannot_return_an_earlier_delivery() {
    let comparison = Comparison::new();
    let first = comparison.row();
    let mut trailing = comparison.row();
    trailing.original["fields_extra"] = json!(true);
    assert!(decode::rows(vec![first, trailing], &comparison.input()).is_err());
}

#[test]
fn cancel_dto_rejects_extra_fields_mismatched_run_nil_id_and_json_null() {
    let comparison = Comparison::new();
    for mutation in 0..5 {
        let mut row = comparison.row();
        let value = row.cancel_json.as_mut().unwrap();
        match mutation {
            0 => value["fields_extra"] = json!(true),
            1 => value["run_id"] = json!(Uuid::new_v4()),
            2 => value["command_id"] = json!(Uuid::nil()),
            3 => *value = Value::Null,
            _ => value["reason"] = json!("wrong removal reason"),
        }
        assert!(decode::rows(vec![row], &comparison.input()).is_err());
    }
}

#[test]
fn original_must_match_the_exact_claimed_attempt_and_command() {
    let comparison = Comparison::new();
    for field in ["drain_attempt", "drain_command", "removal_id"] {
        let mut row = comparison.row();
        row.original[field] = json!(if field == "removal_id" {
            Uuid::nil()
        } else {
            Uuid::new_v4()
        });
        assert!(decode::rows(vec![row], &comparison.input()).is_err());
    }
    let mut row = comparison.row();
    row.original["input_hash"] = json!(vec![256u16; 32]);
    assert!(decode::rows(vec![row], &comparison.input()).is_err());
}

#[test]
fn duplicate_or_nil_runs_and_shared_cancel_commands_deny_the_complete_set() {
    let comparison = Comparison::new();
    let first = comparison.row();
    let mut second = comparison.row();
    second.run_id = first.run_id;
    second.cancel_json.as_mut().unwrap()["run_id"] = json!(first.run_id);
    assert!(decode::rows(vec![first, second], &comparison.input()).is_err());
    let first = comparison.row();
    let mut second = comparison.row();
    second.cancel_json.as_mut().unwrap()["command_id"] =
        first.cancel_json.as_ref().unwrap()["command_id"].clone();
    assert!(decode::rows(vec![first, second], &comparison.input()).is_err());
    let mut row = comparison.row();
    row.run_id = Uuid::nil();
    assert!(decode::rows(vec![row], &comparison.input()).is_err());
}

#[test]
fn sql_null_preserves_original_comparison_data_without_synthesizing_a_cancel() {
    let comparison = Comparison::new();
    let mut row = comparison.row();
    row.cancel_json = None;
    let original = row.original.clone();
    let observed = decode::rows(vec![row], &comparison.input()).unwrap();
    assert!(observed[0].cancel.is_none());
    assert_eq!(observed[0].original, original);
}

#[test]
fn ordinary_context_requires_the_same_drain_dto_and_complete_control_associations() {
    let comparison = Comparison::new();
    for mutation in 0..5 {
        let mut row = comparison.ordinary();
        match mutation {
            0 => row.original["controls"][0]["run_id"] = json!(Uuid::new_v4()),
            1 => row.original["drain_cancel"]["command_id"] = json!(Uuid::new_v4()),
            2 => row.original["drain"]["removal_id"] = json!(Uuid::new_v4()),
            3 => row.original["cancels"][0]["fields_extra"] = json!(true),
            _ => row.cancel_json = None,
        }
        assert!(decode::rows(vec![row], &comparison.input()).is_err());
    }
    let row = comparison.ordinary();
    let original = row.original.clone();
    assert_eq!(
        decode::rows(vec![row], &comparison.input()).unwrap()[0].original,
        original
    );
}

#[test]
fn nil_or_mismatched_input_does_not_become_a_successful_empty_inventory() {
    let mut comparison = Comparison::new();
    comparison.drain_claim["attempt_id"] = json!(Uuid::new_v4());
    assert!(decode::rows(vec![], &comparison.input()).is_err());
    comparison.drain_claim["attempt_id"] = json!(comparison.drain);
    comparison.drain_claim["command_id"] = json!(Uuid::nil());
    assert!(decode::rows(vec![], &comparison.input()).is_err());
    comparison.drain_claim["command_id"] = json!(Uuid::new_v4());
    comparison.instance = AgentInstanceId::from_uuid(Uuid::nil());
    assert!(decode::rows(vec![], &comparison.input()).is_err());
}
