//! Provider-neutral review control orchestration and external effect ports.

mod error;
mod models;
mod outbox;
mod service;

pub use error::{ControlServiceError, ReviewRepositoryError};
pub use models::{
    ApprovalDisposition, ApprovalPreparation, ApprovalProposal, ControlOutcome, RepositoryLocator,
    ReviewGit, ReviewRepository,
};
pub use outbox::{ReviewOutboxRecord, ReviewOutboxStore, ReviewOutboxStoreError};
pub use service::ReviewControlService;
