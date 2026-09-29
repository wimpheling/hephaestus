use super::{
    AuthorizationDecision, BearerToken, KeyId, RegistryAction, RegistryService, RegistryTokenError,
    RegistryTokenIssuer, RegistryTokenVerifier, RepositoryActions, RepositoryName, ScopeRequest,
    SigningKey, TokenLifetime, TokenSubject, UnixTimestamp, VerificationKey,
};
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use serde_json::json;

const SECRET_A: &[u8] = b"01234567890123456789012345678901";
const SECRET_B: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEF";
const RSA_PRIVATE_KEY: &[u8] = br"-----BEGIN PRIVATE KEY-----
MIIEvgIBADANBgkqhkiG9w0BAQEFAASCBKgwggSkAgEAAoIBAQDKANJrMBPSeW30
/erR+lT4rSSDG9pYE/0SjSZFjG79pJ1fxDT/IZmDaoz8pq4dgYA+XHS3UP68XZdS
zRW4H5RCbvHtRW3y9/snPHK3Q6p6zXj5TPAA0k3uZV3cBcVJneEUVHwjvs7ZQ/7d
fF6FuaavsEW4CblgXwoZ2jV7tw961ZfvXWNCJ4DPdkRAKXAs7L/YwHIUAG0OpcH+
Sb54fZYaZSUvmkSVXsENA4wM3F9ivQcRL/xtTka4IBg03e9Mbol2WvdKs631aS9P
RPlbqdAEnm0hoc7AEwAvyIpL6K8m6IuwcQaH7w6/FVNnu6DWQNAbCDy7TA+mv6jb
5SZ0xQN7AgMBAAECggEATc2HPhWkbNqsSUJLYVjDxYwalgzySh5YyP5okT0HutXe
b3ZI20N7tywg5WblhSPN2zcNFVYy5yY9FH09Mk+ncPb+Y17sfDqbF3+mx4NedDIT
uCG0Bvz5WyrbvdTTKgmPGZ94uOPTE8emsHQoi+T3mI+SKtJD/iRc5ZwwIVhes/ZF
drTnmUoYLJKNjdHSDQdGZODNsp4KfrjjY+Vas5W14HTM6ZV6ahquWiwtTVA0idVh
/4S9/vS0svRELAff3WQAmGoLjHTAIJoZhqJnCAj3ggFske70fiUjLIm9f+R8hNjp
RUt5yQ2IAitXGfFjvFRPoLfKxlQdf/fSIosn9AiXIQKBgQDo+4Ij6HoGk/y54iM+
VRFODF/Onj16VfSQMPFPm03lsBP9KYN6y87DW/kC9736dAuCt9remNIcaY19E1Nc
M0VLLmTmRHTRD9dH/jWj5kW3CX6JA/B14wOSF3mu+hj9L8Bm9RPL5UJgU/u7ZrgU
8u6rzay3VBdnHzb+KRjFhcFrwwKBgQDd9czIOPFkgHO883Cf1TT2T1Y/WIPf/e0x
RE0wmpx0ng3CmgOEzo+5AKD8E2Eb1GgXhJw8ZopeCJ/2JAklSj+3vELcXOoh08Iy
miSDarjJVmQ9vFYGmiXzqoYMVB+wkpiiq8dcyaqpmAjtkVO7Iabti9EXHqRctVIp
340hEZJl6QKBgCFqsafk2FvJLh6bSOLP4MOJEtTX7Yl2erWTz4jThcDEGJnfMnSS
dv2eW4EJd75MlroRFNuIn9pjaV/fPb2jvPSjmuVMPFUgKIiy9Y6koKs4OWX9oqfF
/+UcaN+oD52BE9+wlz5Pi821Pg4LFawrjAAoZ/WDojewSnr5+guau7txAoGBAJPy
14FOk3jONldoXVXso9TapT6sHZscgxIn2Nvg8xC4matxRY8ssJg8VxIvSLdoKcoj
VpDcOLbdQOKsunvktfwevOJt/JJ3uCZKoLQIWwu5Ti/obd8QuONmctuc51KnJJ6p
qcWrltpcwPa5u/osQDxuyfyDLEOviQjoPgYg1FihAoGBALvz4LG/JJaTpGZ4bLCR
59lnHzoitoNLcqktDSAY2XXBeOR1jY1NYKOrZfj3TVp2c1H+GT4Eedafsu2+iLsY
L8S2YO4q1QLTkgaZcmAag9KeqlwvP6wRyVFysUoHohae5c0mVoIkgX+7/tlUUrMK
LGejB33bH5767bhefE86++/W
-----END PRIVATE KEY-----
";
const RSA_PUBLIC_KEY: &[u8] = br"-----BEGIN PUBLIC KEY-----
MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEAygDSazAT0nlt9P3q0fpU
+K0kgxvaWBP9Eo0mRYxu/aSdX8Q0/yGZg2qM/KauHYGAPlx0t1D+vF2XUs0VuB+U
Qm7x7UVt8vf7Jzxyt0Oqes14+UzwANJN7mVd3AXFSZ3hFFR8I77O2UP+3Xxehbmm
r7BFuAm5YF8KGdo1e7cPetWX711jQieAz3ZEQClwLOy/2MByFABtDqXB/km+eH2W
GmUlL5pElV7BDQOMDNxfYr0HES/8bU5GuCAYNN3vTG6Jdlr3SrOt9WkvT0T5W6nQ
BJ5tIaHOwBMAL8iKS+ivJuiLsHEGh+8OvxVTZ7ug1kDQGwg8u0wPpr+o2+UmdMUD
ewIDAQAB
-----END PUBLIC KEY-----
";
pub const NOW: UnixTimestamp = UnixTimestamp::new(1_700_000_000);

pub fn issuer() -> RegistryTokenIssuer {
    RegistryTokenIssuer::new(
        "https://forge.example/registry".parse().expect("issuer"),
        "registry.forge.example:5000".parse().expect("service"),
        SigningKey::hs256("active-2026".parse().expect("key id"), SECRET_A).expect("signing key"),
        TokenLifetime::new(300).expect("lifetime"),
    )
}

fn verifier() -> RegistryTokenVerifier {
    RegistryTokenVerifier::new(
        "https://forge.example/registry".parse().expect("issuer"),
        "registry.forge.example:5000".parse().expect("service"),
        [
            VerificationKey::hs256("active-2026".parse().expect("key id"), SECRET_A)
                .expect("verification key"),
        ],
        TokenLifetime::new(300).expect("lifetime"),
    )
    .expect("verifier")
}

pub fn request(scopes: &str) -> ScopeRequest {
    ScopeRequest::parse("registry.forge.example:5000", scopes).expect("scope request")
}

pub fn subject() -> TokenSubject {
    "workload:publication-42".parse().expect("subject")
}

fn decision(actions: RepositoryActions) -> AuthorizationDecision {
    let mut decision = AuthorizationDecision::deny_all();
    decision.grant(
            "projects/123e4567-e89b-12d3-a456-426614174000/repository-builders/987e6543-e21b-12d3-a456-426614174000"
                .parse()
                .expect("repository"),
            actions,
        );
    decision
}

#[test]
fn rejects_malformed_and_wildcard_scopes() {
    for invalid in [
        "repository:platform/builders/*:pull",
        "repository:platform//builders:test",
        "repository:platform/builders/foo.-bar:pull",
        "repository:platform/builders/x:pull,pull",
        "registry:platform/builders/x:pull",
        "repository:platform/builders/x:delete",
        "repository:platform/builders/x:",
        "repository:platform/builders/x:pull  repository:platform/builders/y:push",
    ] {
        assert!(ScopeRequest::parse("registry.forge.example", invalid).is_err());
    }
    assert!(
        "registry.forge.example:*"
            .parse::<RegistryService>()
            .is_err()
    );
    assert!("Registry.forge.example".parse::<RegistryService>().is_err());
    assert!("Platform/builders/x".parse::<RepositoryName>().is_err());
}

#[test]
fn intersection_prevents_action_escalation() {
    let issued = issuer()
            .issue(
                subject(),
                &request(
                    "repository:projects/123e4567-e89b-12d3-a456-426614174000/repository-builders/987e6543-e21b-12d3-a456-426614174000:pull,push",
                ),
                &decision(RepositoryActions::pull()),
                NOW,
            )
            .expect("token issuance");
    let verified = verifier()
        .verify(issued.token(), NOW)
        .expect("token verification");
    assert_eq!(verified.access.len(), 1);
    assert_eq!(verified.access[0].actions(), &[RegistryAction::Pull]);
}

#[test]
fn issues_empty_access_for_empty_grants() {
    let issued = issuer()
        .issue(
            subject(),
            &request("repository:platform/builders/rust-ubuntu:pull"),
            &AuthorizationDecision::deny_all(),
            NOW,
        )
        .expect("denied token is still well formed");
    assert!(issued.claims().access.is_empty());
    assert!(verifier().verify(issued.token(), NOW).is_ok());
}

#[test]
fn verification_rejects_wrong_issuer_and_audience() {
    let request = request("repository:platform/builders/rust-ubuntu:pull");
    let issued = issuer()
        .issue(subject(), &request, &AuthorizationDecision::deny_all(), NOW)
        .expect("token");
    let wrong_issuer = RegistryTokenVerifier::new(
        "https://other.example/registry".parse().expect("issuer"),
        "registry.forge.example:5000".parse().expect("service"),
        [VerificationKey::hs256("active-2026".parse().expect("key id"), SECRET_A).expect("key")],
        TokenLifetime::new(300).expect("lifetime"),
    )
    .expect("verifier");
    assert!(matches!(
        wrong_issuer.verify(issued.token(), NOW),
        Err(RegistryTokenError::IssuerMismatch)
    ));
    let wrong_audience = RegistryTokenVerifier::new(
        "https://forge.example/registry".parse().expect("issuer"),
        "other-registry.example".parse().expect("service"),
        [VerificationKey::hs256("active-2026".parse().expect("key id"), SECRET_A).expect("key")],
        TokenLifetime::new(300).expect("lifetime"),
    )
    .expect("verifier");
    assert!(matches!(
        wrong_audience.verify(issued.token(), NOW),
        Err(RegistryTokenError::AudienceMismatch)
    ));
}

#[test]
fn verification_enforces_expiry_not_before_and_clock_bounds() {
    let issued = issuer()
        .issue(
            subject(),
            &request("repository:platform/builders/rust-ubuntu:pull"),
            &AuthorizationDecision::deny_all(),
            NOW,
        )
        .expect("token");
    let verifier = verifier();
    assert!(matches!(
        verifier.verify(issued.token(), UnixTimestamp::new(NOW.seconds() + 300)),
        Err(RegistryTokenError::Expired)
    ));

    let future = signed_token(&json!({
        "iss": "https://forge.example/registry",
        "aud": "registry.forge.example:5000",
        "sub": "workload:publication-42",
        "iat": NOW.seconds() + 10,
        "nbf": NOW.seconds() + 10,
        "exp": NOW.seconds() + 100,
        "jti": "123e4567-e89b-12d3-a456-426614174000",
        "access": []
    }));
    assert!(matches!(
        verifier.verify(&future, NOW),
        Err(RegistryTokenError::IssuedInFuture)
    ));
    let not_yet_valid = signed_token(&json!({
        "iss": "https://forge.example/registry",
        "aud": "registry.forge.example:5000",
        "sub": "workload:publication-42",
        "iat": NOW.seconds(),
        "nbf": NOW.seconds() + 10,
        "exp": NOW.seconds() + 100,
        "jti": "123e4567-e89b-12d3-a456-426614174002",
        "access": []
    }));
    assert!(matches!(
        verifier.verify(&not_yet_valid, NOW),
        Err(RegistryTokenError::NotYetValid)
    ));
    let invalid_window = signed_token(&json!({
        "iss": "https://forge.example/registry",
        "aud": "registry.forge.example:5000",
        "sub": "workload:publication-42",
        "iat": NOW.seconds(),
        "nbf": NOW.seconds() + 5,
        "exp": NOW.seconds() + 5,
        "jti": "123e4567-e89b-12d3-a456-426614174001",
        "access": []
    }));
    assert!(matches!(
        verifier.verify(&invalid_window, NOW),
        Err(RegistryTokenError::InvalidTimeBounds)
    ));
}

#[test]
fn verification_accepts_overlapping_rotation_keys() {
    let old_issuer = RegistryTokenIssuer::new(
        "https://forge.example/registry".parse().expect("issuer"),
        "registry.forge.example:5000".parse().expect("service"),
        SigningKey::hs256("old-2025".parse().expect("key id"), SECRET_B).expect("key"),
        TokenLifetime::new(300).expect("lifetime"),
    );
    let old = old_issuer
        .issue(
            subject(),
            &request("repository:platform/builders/rust-ubuntu:pull"),
            &AuthorizationDecision::deny_all(),
            NOW,
        )
        .expect("old token");
    let rotating_verifier = RegistryTokenVerifier::new(
        "https://forge.example/registry".parse().expect("issuer"),
        "registry.forge.example:5000".parse().expect("service"),
        [
            VerificationKey::hs256("active-2026".parse().expect("key id"), SECRET_A)
                .expect("active key"),
            VerificationKey::hs256("old-2025".parse().expect("key id"), SECRET_B).expect("old key"),
        ],
        TokenLifetime::new(300).expect("lifetime"),
    )
    .expect("rotating verifier");
    assert!(rotating_verifier.verify(old.token(), NOW).is_ok());
}

#[test]
fn rsa_signing_keeps_private_material_out_of_the_verifier() {
    let token_service = RegistryTokenIssuer::new(
        "https://forge.example/registry".parse().expect("issuer"),
        "registry.forge.example:5000".parse().expect("service"),
        SigningKey::rs256_pem("rsa-2026".parse().expect("key id"), RSA_PRIVATE_KEY)
            .expect("private RSA key"),
        TokenLifetime::new(300).expect("lifetime"),
    );
    let issued = token_service
        .issue(
            subject(),
            &request("repository:platform/builders/rust-ubuntu:pull"),
            &AuthorizationDecision::deny_all(),
            NOW,
        )
        .expect("RSA token");
    let verifier = RegistryTokenVerifier::new(
        "https://forge.example/registry".parse().expect("issuer"),
        "registry.forge.example:5000".parse().expect("service"),
        [
            VerificationKey::rs256_pem("rsa-2026".parse().expect("key id"), RSA_PUBLIC_KEY)
                .expect("public RSA key"),
        ],
        TokenLifetime::new(300).expect("lifetime"),
    )
    .expect("verifier");

    assert!(verifier.verify(issued.token(), NOW).is_ok());
}

fn signed_token(claims: &serde_json::Value) -> BearerToken {
    let mut header = Header::new(Algorithm::HS256);
    header.kid = Some("active-2026".to_owned());
    BearerToken(
        encode(&header, &claims, &EncodingKey::from_secret(SECRET_A)).expect("signed token"),
    )
}

#[test]
fn key_identifiers_are_not_credentials() {
    let key_id: KeyId = "active-2026".parse().expect("key id");
    assert_eq!(key_id.as_str(), "active-2026");
}
