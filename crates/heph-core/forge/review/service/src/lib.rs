//! Provider-neutral review control orchestration and trusted Git publication.

mod command_transport;
mod error;
mod models;
mod publisher;
mod service;

use review_domain::{CONTROL_EXECUTE_SUBJECT, ControlCommand};

pub use command_transport::{
    NatsControlHandler, ReviewOutboxPublishError, ReviewOutboxPublisher, ReviewOutboxRecord,
    ReviewOutboxStore, ReviewOutboxStoreError,
};
pub use error::{ControlHandlingError, ControlServiceError, ReviewRepositoryError};
pub use models::{
    ApprovalDisposition, ApprovalPreparation, ApprovalProposal, ControlOutcome, RepositoryLocator,
    ReviewGit, ReviewRepository,
};
pub use publisher::GitReviewPublisher;
pub use service::ReviewControlService;
