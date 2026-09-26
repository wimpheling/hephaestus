use super::rows::parse_state;
use release_domain::ReleaseVersion;

#[test]
fn release_state_and_version_contracts_match_product_values() {
    assert!(parse_state("draft").is_ok());
    assert!(parse_state("published").is_ok());
    assert!(parse_state("revoked").is_ok());
    assert!(parse_state("running").is_err());
    for version in ["v1.0.0", "2026.07", "experimental-4"] {
        assert!(ReleaseVersion::parse(version).is_ok());
    }
    assert!(ReleaseVersion::parse("bad/version").is_err());
}
