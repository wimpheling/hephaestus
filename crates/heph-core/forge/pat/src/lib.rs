//! Provider-neutral developer personal access token contracts.
//!
//! A [`PersonalAccessToken`] is plaintext bearer material. It can be exposed
//! only through an explicit method, is redacted from formatting and
//! serialization, and should live only long enough to cross the issuance or
//! authentication boundary. Durable state is represented by
//! [`PersonalAccessTokenRecord`], which contains only a domain-separated
//! one-way verifier and safe lifecycle metadata.
use time::Duration;

/// Version of the plaintext PAT envelope and verifier construction.
pub const PAT_FORMAT_VERSION: u16 = 1;
/// Maximum lifetime accepted for a newly issued developer PAT.
pub const MAX_PAT_LIFETIME: Duration = Duration::days(90);
/// Maximum number of exact repository restrictions on one PAT.
pub const MAX_REPOSITORY_RESTRICTIONS: usize = 128;
/// Maximum number of UTF-8 bytes in a user-visible PAT label.
pub const MAX_PAT_LABEL_BYTES: usize = 128;

const PAT_PREFIX: &str = "heph_pat_v1";
const PAT_SECRET_BYTES: usize = 32;
const PAT_VERIFIER_DOMAIN: &[u8] = b"hephaestus-developer-pat-verifier-v1\0";
const REDACTED: &str = "[REDACTED]";

mod errors;
mod record;
mod scope;
mod token;

pub use errors::{PersonalAccessTokenAuthorizationError, PersonalAccessTokenError};
pub use record::PersonalAccessTokenRecord;
pub use scope::{PersonalAccessTokenMetadata, PersonalAccessTokenScope, PersonalAccessTokenStatus};
pub use token::{
    PersonalAccessToken, PersonalAccessTokenId, PersonalAccessTokenLabel,
    PersonalAccessTokenVerifier,
};

#[cfg(test)]
mod tests {
    use super::{
        MAX_PAT_LIFETIME, MAX_REPOSITORY_RESTRICTIONS, PersonalAccessToken,
        PersonalAccessTokenAuthorizationError, PersonalAccessTokenError, PersonalAccessTokenId,
        PersonalAccessTokenLabel, PersonalAccessTokenRecord, PersonalAccessTokenScope,
        PersonalAccessTokenStatus,
    };
    use forge_domain::RepositoryId;
    use git_capability_domain::GitOperation;
    use identity_domain::{RequestId, UserId};
    use time::{Duration, OffsetDateTime};

    const SENTINEL_SECRET: [u8; 32] = [0x5a; 32];

    fn issued_record(
        token: &PersonalAccessToken,
        owner: UserId,
        repository: RepositoryId,
        created_at: OffsetDateTime,
    ) -> PersonalAccessTokenRecord {
        PersonalAccessTokenRecord::issue(
            token,
            owner,
            PersonalAccessTokenLabel::parse("developer laptop").expect("label should validate"),
            PersonalAccessTokenScope::new(
                [GitOperation::Discover, GitOperation::Fetch],
                Some([repository]),
            )
            .expect("scope should validate"),
            created_at,
            created_at + Duration::days(30),
            RequestId::new(),
        )
        .expect("record should validate")
    }

    #[test]
    fn canonical_token_round_trips_and_rejects_noncanonical_forms() {
        let token = PersonalAccessToken::from_secret(PersonalAccessTokenId::new(), SENTINEL_SECRET);
        let exposed = token.expose();
        let parsed = PersonalAccessToken::parse(&exposed).expect("canonical token should parse");
        assert_eq!(parsed.id(), token.id());
        assert!(token.verifier().verifies(&parsed));

        for malformed in [
            "",
            "heph_pat_v2.00000000-0000-0000-0000-000000000000.00",
            "heph_pat_v1.00000000000000000000000000000000.0000000000000000000000000000000000000000000000000000000000000000",
            "heph_pat_v1.00000000-0000-0000-0000-000000000000.000000000000000000000000000000000000000000000000000000000000000G",
            "heph_pat_v1.00000000-0000-0000-0000-000000000000.0000000000000000000000000000000000000000000000000000000000000000.extra",
        ] {
            assert_eq!(
                PersonalAccessToken::parse(malformed).expect_err("input must fail"),
                PersonalAccessTokenError::InvalidToken
            );
        }
    }

    #[test]
    fn verifier_is_bound_to_identifier_and_secret() {
        let id = PersonalAccessTokenId::new();
        let token = PersonalAccessToken::from_secret(id, SENTINEL_SECRET);
        let other_id =
            PersonalAccessToken::from_secret(PersonalAccessTokenId::new(), SENTINEL_SECRET);
        let mut other_secret = SENTINEL_SECRET;
        other_secret[31] ^= 1;
        let other_secret = PersonalAccessToken::from_secret(id, other_secret);
        let verifier = token.verifier();

        assert_eq!(verifier.version(), 1);
        assert!(verifier.verifies(&token));
        assert!(!verifier.verifies(&other_id));
        assert!(!verifier.verifies(&other_secret));
    }

    #[test]
    fn plaintext_is_redacted_from_formatting_and_serialization() {
        let token = PersonalAccessToken::from_secret(PersonalAccessTokenId::new(), SENTINEL_SECRET);
        let secret_hex = "5a".repeat(32);
        let representations = [
            format!("{token:?}"),
            format!("{token}"),
            serde_json::to_string(&token).expect("redacted token should serialize"),
            format!("{:?}", token.verifier()),
            serde_json::to_string(&token.verifier()).expect("verifier should serialize"),
        ];
        for representation in representations {
            assert!(!representation.contains(&secret_hex));
            assert!(!representation.contains(&token.expose()));
        }
    }

    #[test]
    fn listing_metadata_excludes_bearer_and_verifier() {
        let created_at = OffsetDateTime::from_unix_timestamp(1_700_000_000)
            .expect("fixture timestamp should validate");
        let owner = UserId::new();
        let repository = RepositoryId::new();
        let token = PersonalAccessToken::from_secret(PersonalAccessTokenId::new(), SENTINEL_SECRET);
        let record = issued_record(&token, owner, repository, created_at);
        let serialized =
            serde_json::to_string(&record.metadata()).expect("safe metadata should serialize");

        assert!(serialized.contains(&token.id().to_string()));
        assert!(!serialized.contains("verifier"));
        assert!(!serialized.contains(&token.expose()));
        assert!(!serialized.contains(&"5a".repeat(32)));
    }

    #[test]
    fn scopes_are_explicit_normalized_and_optionally_repository_bound() {
        let allowed = RepositoryId::new();
        let denied = RepositoryId::new();
        let restricted = PersonalAccessTokenScope::new(
            [GitOperation::Fetch, GitOperation::Fetch],
            Some([allowed, allowed]),
        )
        .expect("scope should normalize");
        assert_eq!(restricted.operations().len(), 1);
        assert_eq!(
            restricted.repository_restrictions().map(BTreeSet::len),
            Some(1)
        );
        assert!(restricted.permits(GitOperation::Fetch, allowed));
        assert!(!restricted.permits(GitOperation::Fetch, denied));
        assert!(!restricted.permits(GitOperation::Receive, allowed));

        let unrestricted =
            PersonalAccessTokenScope::new([GitOperation::Discover], None::<[RepositoryId; 0]>)
                .expect("unrestricted scope should validate");
        assert!(unrestricted.permits(GitOperation::Discover, denied));
        assert_eq!(
            PersonalAccessTokenScope::new([], None::<[RepositoryId; 0]>)
                .expect_err("empty operations must fail"),
            PersonalAccessTokenError::EmptyScope
        );
        assert_eq!(
            PersonalAccessTokenScope::new([GitOperation::Fetch], Some([]))
                .expect_err("empty explicit restrictions must fail"),
            PersonalAccessTokenError::InvalidRepositoryRestrictions
        );
        let too_many = (0..=MAX_REPOSITORY_RESTRICTIONS)
            .map(|_| RepositoryId::new())
            .collect::<Vec<_>>();
        assert_eq!(
            PersonalAccessTokenScope::new([GitOperation::Fetch], Some(too_many))
                .expect_err("oversized restrictions must fail"),
            PersonalAccessTokenError::InvalidRepositoryRestrictions
        );
    }

    #[test]
    fn lifecycle_expiry_revocation_and_last_use_fail_closed() {
        let created_at = OffsetDateTime::from_unix_timestamp(1_700_000_000)
            .expect("fixture timestamp should validate");
        let owner = UserId::new();
        let repository = RepositoryId::new();
        let token = PersonalAccessToken::from_secret(PersonalAccessTokenId::new(), SENTINEL_SECRET);
        let mut record = issued_record(&token, owner, repository, created_at);

        assert_eq!(
            record.status_at(created_at - Duration::SECOND),
            PersonalAccessTokenStatus::NotYetValid
        );
        assert_eq!(
            record.status_at(created_at),
            PersonalAccessTokenStatus::Active
        );
        assert!(
            record
                .authorize_at(&token, owner, GitOperation::Fetch, repository, created_at)
                .is_ok()
        );
        assert_eq!(
            record.authorize_at(
                &token,
                UserId::new(),
                GitOperation::Fetch,
                repository,
                created_at
            ),
            Err(PersonalAccessTokenAuthorizationError::WrongOwner)
        );

        let used_at = created_at + Duration::HOUR;
        record
            .record_use(used_at)
            .expect("active use should record");
        assert_eq!(record.last_used_at(), Some(used_at));
        assert_eq!(
            record
                .record_use(created_at + Duration::MINUTE)
                .expect_err("last use cannot move backwards"),
            PersonalAccessTokenError::NonMonotonicLastUse
        );

        let revoked_at = created_at + Duration::hours(2);
        record
            .revoke(revoked_at)
            .expect("active token should revoke");
        assert_eq!(
            record.status_at(revoked_at),
            PersonalAccessTokenStatus::Revoked
        );
        assert_eq!(
            record.authorize_at(&token, owner, GitOperation::Fetch, repository, revoked_at),
            Err(PersonalAccessTokenAuthorizationError::Revoked)
        );
        assert_eq!(
            record.revoke(revoked_at).expect_err("revocation is final"),
            PersonalAccessTokenError::AlreadyRevoked
        );
    }

    #[test]
    fn lifetime_is_positive_bounded_and_exclusive() {
        let created_at = OffsetDateTime::from_unix_timestamp(1_700_000_000)
            .expect("fixture timestamp should validate");
        let owner = UserId::new();
        let repository = RepositoryId::new();
        let token = PersonalAccessToken::from_secret(PersonalAccessTokenId::new(), SENTINEL_SECRET);
        let label = PersonalAccessTokenLabel::parse("ci workstation").expect("valid label");
        let scope = PersonalAccessTokenScope::new([GitOperation::Fetch], None::<[RepositoryId; 0]>)
            .expect("valid scope");
        assert_eq!(
            PersonalAccessTokenRecord::issue(
                &token,
                owner,
                label.clone(),
                scope.clone(),
                created_at,
                created_at,
                RequestId::new()
            )
            .expect_err("zero lifetime must fail"),
            PersonalAccessTokenError::InvalidLifetime
        );
        assert_eq!(
            PersonalAccessTokenRecord::issue(
                &token,
                owner,
                label,
                scope,
                created_at,
                created_at + MAX_PAT_LIFETIME + Duration::SECOND,
                RequestId::new()
            )
            .expect_err("oversized lifetime must fail"),
            PersonalAccessTokenError::InvalidLifetime
        );

        let record = issued_record(&token, owner, repository, created_at);
        assert_eq!(
            record.status_at(record.expires_at()),
            PersonalAccessTokenStatus::Expired
        );
    }

    #[test]
    fn rejected_values_never_enter_errors() {
        let sentinel = "sentinel-token-label-never-log";
        let errors = [
            PersonalAccessToken::parse(sentinel).expect_err("invalid token"),
            PersonalAccessTokenLabel::parse(format!(" {sentinel}")).expect_err("invalid label"),
        ];
        for error in errors {
            assert!(!format!("{error:?}").contains(sentinel));
            assert!(!error.to_string().contains(sentinel));
        }
    }

    use std::collections::BTreeSet;
}
