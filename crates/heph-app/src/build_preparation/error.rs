pub fn component<Error: std::fmt::Display>(
    name: &'static str,
) -> impl FnOnce(Error) -> AppError + Copy {
    move |error| AppError::Component {
        component: name,
        message: error.to_string(),
    }
}

/// Application construction, startup, supervision, or shutdown failure.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum AppError {
    /// Static or migration configuration is invalid.
    #[error("invalid application configuration: {0}")]
    Configuration(String),
    /// One application component failed.
    #[error("{component} failed: {message}")]
    Component {
        /// Component name.
        component: &'static str,
        /// Non-sensitive failure.
        message: String,
    },
    /// Actual managed acquisition error; preserve its private original custody.
    /// Converting this source to text or dropping it proves no cleanup/recovery.
    #[error("managed backend preparation failed: {source}")]
    ManagedBackend {
        /// Original error chain, which may retain native descriptors and owners.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    /// Readiness barrier failed.
    #[error("application readiness failed: {0}")]
    Readiness(String),
    /// A supervised task failed.
    #[error("supervised application task failed: {0}")]
    Task(String),
    /// An operation timed out.
    #[error("application operation timed out: {0}")]
    Timeout(String),
    /// Resource shutdown failed.
    #[error("resource shutdown failed: {0}")]
    Shutdown(String),
}
