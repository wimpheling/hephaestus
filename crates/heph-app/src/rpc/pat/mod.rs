//! Developer personal access token RPC boundary.

mod handler;
mod support;

pub use handler::register;
// Preserve the implementation type's historical visibility within the PAT module.
#[allow(unused_imports)]
pub use handler::PersonalAccessTokenRpc;
use support::{
    application_error, identity_receipt, metadata, mutation_identity, scope, timestamp, token_id,
};

#[cfg(test)]
mod tests {
    use super::{scope, timestamp};
    use rpc_proto::messages::hephaestus::pat::v1::{GitOperation, PersonalAccessTokenScope};

    #[test]
    fn scope_rejects_unspecified_and_accepts_exact_operations() {
        assert!(scope(Some(&PersonalAccessTokenScope::default())).is_err());
        let valid = PersonalAccessTokenScope {
            operations: vec![GitOperation::Discover.into(), GitOperation::Fetch.into()],
            ..Default::default()
        };
        assert!(scope(Some(&valid)).is_ok());
    }

    #[test]
    fn timestamp_rejects_invalid_nanos() {
        assert!(
            timestamp(Some(&buffa_types::google::protobuf::Timestamp {
                seconds: 1,
                nanos: 1_000_000_000,
                ..Default::default()
            }))
            .is_err()
        );
    }
}
