//! Provider-neutral review control orchestration and trusted Git publication.

mod error;
mod models;
mod outbox;
mod publisher;
mod service;

pub use error::{ControlServiceError, ReviewRepositoryError};
pub use models::{
    ApprovalDisposition, ApprovalPreparation, ApprovalProposal, ControlOutcome, RepositoryLocator,
    ReviewGit, ReviewRepository,
};
pub use outbox::{ReviewOutboxRecord, ReviewOutboxStore, ReviewOutboxStoreError};
pub use publisher::GitReviewPublisher;
pub use service::ReviewControlService;
