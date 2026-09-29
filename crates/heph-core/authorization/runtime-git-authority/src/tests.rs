use super::RuntimeGitCredential;

#[test]
fn token_round_trip_is_canonical_redacted_and_domain_hashed() {
    let credential = RuntimeGitCredential::from_secret([0x5c; 32]);
    let token = credential.expose_token();
    assert!(token.starts_with("heph_git_v1_"));
    assert_eq!(
        RuntimeGitCredential::parse(&token)
            .expect("canonical token")
            .expose(),
        credential.expose()
    );
    assert!(!format!("{credential:?}").contains("92"));
    assert_ne!(credential.storage_hash().as_bytes(), *credential.expose());
    assert!(RuntimeGitCredential::parse("heph_pat_v1_not-runtime").is_err());
}
