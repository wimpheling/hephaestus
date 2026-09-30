use thiserror::Error;

/// Safe validation failure without input or secret disclosure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum BrokeredEgressError {
    /// The destination is not one exact safe HTTPS origin.
    #[error("brokered destination must be an exact HTTPS origin")]
    InvalidOrigin,
    /// The header name is outside the closed HTTP token grammar.
    #[error("brokered header name is invalid")]
    InvalidHeader,
    /// The header prefix is ambiguous or unsafe.
    #[error("brokered header prefix is invalid")]
    InvalidHeaderPrefix,
    /// Destination/direction/route fields do not describe one exact scope.
    #[error("brokered rule scope is ambiguous")]
    AmbiguousRuleScope,
}
