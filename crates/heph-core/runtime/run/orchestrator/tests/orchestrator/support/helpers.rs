use run_domain::{RunKind, StartRun};
use runtime_types::{
    AgentAttachmentId, AgentInstanceId, AgentInstanceRevisionId, CommandId, ReleaseAgentId,
    ReleaseId, RunId,
};
use std::sync::{Mutex as StdMutex, MutexGuard};

pub fn lock<T>(mutex: &StdMutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

pub fn normal_stateless_command() -> StartRun {
    StartRun {
        command_id: CommandId::new(),
        run_id: RunId::new(),
        instance_id: AgentInstanceId::new(),
        instance_revision_id: AgentInstanceRevisionId::new(),
        release_id: ReleaseId::new(),
        release_agent_id: ReleaseAgentId::new(),
        attachment_id: Some(AgentAttachmentId::new()),
        kind: RunKind::Normal,
        requires_state: false,
    }
}

pub fn assert_launch_order(log: &StdMutex<Vec<&'static str>>) {
    let entries = lock(log);
    let authorization = entries
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| (*entry == "authorize").then_some(index))
        .collect::<Vec<_>>();
    let runtime_prepare = entries
        .iter()
        .position(|entry| *entry == "runtime-prepare")
        .expect("runtime preparation");
    let authority_prepare = entries
        .iter()
        .position(|entry| *entry == "authority-prepare")
        .expect("authority preparation");
    let authority_reauthorize = entries
        .iter()
        .position(|entry| *entry == "authority-reauthorize")
        .expect("authority reauthorization");
    let provision = entries
        .iter()
        .position(|entry| *entry == "provision")
        .expect("VM provision");
    let resource_recorded = entries
        .iter()
        .position(|entry| *entry == "resource-recorded")
        .expect("resource evidence");
    drop(entries);
    assert_eq!(authorization.len(), 2);
    assert!(authorization[0] < runtime_prepare);
    assert!(runtime_prepare < authority_prepare);
    assert!(authority_prepare < resource_recorded);
    assert!(resource_recorded < authorization[1]);
    assert!(authorization[1] < authority_reauthorize && authority_reauthorize < provision);
}
