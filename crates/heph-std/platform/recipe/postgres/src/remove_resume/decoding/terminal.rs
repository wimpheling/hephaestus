//! Native143 wire plus original rows; no current authority comes from a tag.
#[path = "terminal/current.rs"]
mod current;
#[path = "terminal/loading.rs"]
mod loading;
#[path = "terminal/wire.rs"]
mod wire143;

use super::{Roots, wire};
use recipe_application::{
    DeploymentError, EffectClaim, PreparedInstanceSafetyTarget, PreparedInstanceTerminalPublisher,
    RemoveResumeConfiguration,
};
use serde_json::Value;
use sqlx::{Postgres, Transaction};

#[derive(Default)]
pub(super) struct Native143 {
    pub detach: Vec<wire143::Entry>,
    pub terminal: Vec<wire143::Entry>,
    loaded: Option<LoadedInventory>,
}
struct LoadedInventory {
    detach: Vec<Loaded>,
    terminal: Vec<Loaded>,
    config: RemoveResumeConfiguration,
}
struct Loaded {
    wire: wire143::Entry,
    target: PreparedInstanceSafetyTarget,
    claim: EffectClaim,
    publisher: Option<PreparedInstanceTerminalPublisher>,
}
impl Native143 {
    pub(super) fn parse(data: &Value, bound: usize) -> Result<Self, DeploymentError> {
        let detach = wire143::entries(data, "detach", bound)?;
        let terminal = wire143::entries(data, "terminal", bound)?;
        let mut names = std::collections::BTreeSet::new();
        for item in detach.iter().chain(&terminal) {
            if !names.insert(&item.resource)
                || ["pending", "recovery"].into_iter().any(|field| {
                    data[field].as_array().is_some_and(|rows| {
                        rows.iter()
                            .any(|row| row["resource"] == item.resource.as_str())
                    })
                })
            {
                return Err(DeploymentError::IntentMismatch);
            }
        }
        Ok(Self {
            detach,
            terminal,
            loaded: None,
        })
    }
    pub(super) const fn is_empty(&self) -> bool {
        self.detach.is_empty() && self.terminal.is_empty()
    }
    pub(super) fn matches_active(&self, claim: &EffectClaim) -> bool {
        self.loaded.as_ref().is_some_and(|loaded| {
            loaded.detach.iter().chain(&loaded.terminal).any(|item| {
                item.claim == *claim
                    && item.wire.state.active_attempt == Some(claim.provenance.attempt_id.as_uuid())
            })
        })
    }
    pub(super) fn recovery(&self) -> bool {
        self.detach
            .iter()
            .chain(&self.terminal)
            .any(|item| item.state.removal == recipe_application::RemovalProgress::RecoveryRequired)
    }
}
pub(super) async fn load(
    tx: &mut Transaction<'_, Postgres>,
    decoded: &mut Native143,
    roots: &Roots<'_>,
) -> Result<(), DeploymentError> {
    if decoded.is_empty() {
        return Ok(());
    }
    let snapshot = crate::hydration::load(tx, roots.intent.id()).await?;
    if snapshot.intent != *roots.intent {
        return Err(DeploymentError::IntentMismatch);
    }
    let mut detach = Vec::new();
    let mut terminal = Vec::new();
    for entry in &decoded.detach {
        detach.push(loading::entry(tx, entry, roots, &snapshot).await?);
    }
    for entry in &decoded.terminal {
        terminal.push(loading::entry(tx, entry, roots, &snapshot).await?);
    }
    decoded.loaded = Some(LoadedInventory {
        detach,
        terminal,
        config: roots.config.clone(),
    });
    Ok(())
}
pub(super) use current::assemble;
pub(super) use current::compare_replay;
pub(in crate::remove_resume) use wire143::Entry as HistoryEntry;
pub(super) fn history_entries(data: &Value) -> Result<Vec<HistoryEntry>, DeploymentError> {
    wire143::entries(data, "terminal", 1)
}
pub(super) async fn command(
    tx: &mut Transaction<'_, Postgres>,
    id: uuid::Uuid,
) -> Result<recipe_application::CommandIdentity, DeploymentError> {
    loading::command(tx, id).await
}
