use super::*;

#[test]
fn accepts_only_exact_short_lived_audience_bound_assertions() {
    let key = mediator_signing_key(TOKEN);
    let authenticator = MediatorAuthenticator::new(&key);
    let user_id = Uuid::new_v4();
    let assertion_id = Uuid::new_v4();
    let now = OffsetDateTime::now_utc().unix_timestamp();
    let token = assertion(&key, AUDIENCE, user_id, assertion_id, now, now + 30);
    let valid_headers = headers(&token);

    let principal = authenticator
        .authenticate(&valid_headers, AUDIENCE)
        .expect("valid assertion");
    assert_eq!(principal.user_id.to_string(), user_id.to_string());
    assert_eq!(principal.assertion_id, assertion_id);
    assert!(principal.sid.to_protocol_string().parse::<Uuid>().is_ok());
    assert!(
        authenticator
            .authenticate(&valid_headers, "/wrong.Service/Method")
            .is_err()
    );

    let overlong = assertion(&key, AUDIENCE, user_id, assertion_id, now, now + 31);
    assert!(
        authenticator
            .authenticate(&headers(&overlong), AUDIENCE)
            .is_err()
    );

    // Keep a generous future margin so a fresh truncated clock cannot
    // cross the boundary between constructing and authenticating.
    let future_iat = OffsetDateTime::now_utc().unix_timestamp() + CLOCK_SKEW_SECONDS + 60;
    let future = assertion(
        &key,
        AUDIENCE,
        user_id,
        assertion_id,
        future_iat,
        future_iat + 1,
    );
    assert!(
        authenticator
            .authenticate(&headers(&future), AUDIENCE)
            .is_err()
    );
}

#[test]
fn errors_never_contain_assertion_material() {
    let authenticator = MediatorAuthenticator::new(&mediator_signing_key(TOKEN));
    let sentinel = "sensitive-assertion-sentinel";
    let error = authenticator
        .authenticate(&headers(sentinel), AUDIENCE)
        .expect_err("malformed assertion must fail");
    assert!(!error.to_string().contains(sentinel));
    assert!(!format!("{error:?}").contains(sentinel));
}

#[test]
fn normal_assertions_require_a_sid_without_leaking_token_material() {
    let key = mediator_signing_key(TOKEN);
    let authenticator = MediatorAuthenticator::new(&key);
    let now = OffsetDateTime::now_utc().unix_timestamp();
    let token = encode(
        &Header::new(Algorithm::HS256),
        &Claims {
            iss: ISSUER,
            aud: AUDIENCE,
            sub: Uuid::new_v4().to_string(),
            jti: Uuid::new_v4().to_string(),
            iat: now,
            nbf: now,
            exp: now + 30,
            sid: None,
        },
        &EncodingKey::from_secret(&key),
    )
    .expect("encode sidless assertion");
    let error = authenticator
        .authenticate(&headers(&token), AUDIENCE)
        .expect_err("normal assertion without sid must fail");
    assert!(!error.to_string().contains(&token));
    assert!(!format!("{error:?}").contains(&token));
}

#[test]
fn bootstrap_and_revoke_are_the_only_signed_session_exceptions() {
    assert!(!requires_mediator_auth(BOOTSTRAP_AUDIENCE));
    assert!(!requires_mediator_auth(CREATE_BOOTSTRAP_AUDIENCE));
    assert!(requires_mediator_auth(REVOKE_AUDIENCE));
    assert!(requires_mediator_auth(
        "/hephaestus.identity.v1.IdentityService/ResolveIdentity/extra"
    ));
}

#[test]
fn middleware_authenticates_only_connect_paths() {
    assert!(requires_mediator_auth(
        "/hephaestus.agent.v1.AgentService/ImportAgent"
    ));
    assert!(!requires_mediator_auth(BOOTSTRAP_AUDIENCE));
    assert!(!requires_mediator_auth("/healthz"));
    assert!(!requires_mediator_auth("/git/repository/info/refs"));
    assert!(!requires_mediator_auth("/"));
}

#[test]
fn every_service_domain_requires_its_exact_audience() {
    let audiences = [
        "/hephaestus.artifact.v1.ArtifactService/GetArtifactPreview",
        "/hephaestus.build.v1.BuildService/GetBuild",
        "/hephaestus.instance.v1.AgentInstanceService/GetInstance",
        "/hephaestus.organization.v1.OrganizationService/ListOrganizations",
        "/hephaestus.project.v1.ProjectService/GetProject",
        "/hephaestus.release.v1.ReleaseService/GetRelease",
        "/hephaestus.repository.v1.RepositoryService/GetRepository",
        "/hephaestus.repository_browser.v1.RepositoryBrowserService/ListBranches",
        "/hephaestus.run.v1.RunService/GetRun",
        "/hephaestus.secret.v1.SecretService/ListProjectSecrets",
    ];
    let key = mediator_signing_key(TOKEN);
    let authenticator = MediatorAuthenticator::new(&key);
    let now = OffsetDateTime::now_utc().unix_timestamp();
    for audience in audiences {
        let token = assertion(
            &key,
            audience,
            Uuid::new_v4(),
            Uuid::new_v4(),
            now,
            now + 30,
        );
        let headers = headers(&token);
        assert!(authenticator.authenticate(&headers, audience).is_ok());
        assert!(
            authenticator
                .authenticate(&headers, "/wrong.Service/Method")
                .is_err()
        );
    }
}

#[test]
fn bootstrap_assertion_binds_every_verified_identity_field() {
    let key = mediator_signing_key(TOKEN);
    let authenticator = MediatorAuthenticator::new(&key);
    let assertion_id = Uuid::new_v4();
    let now = OffsetDateTime::now_utc().unix_timestamp();
    let audience = "/hephaestus.identity.v1.IdentityService/ResolveIdentity";
    let expected = BootstrapIdentity {
        issuer: "https://issuer.example",
        subject: "external-subject",
        display_name: "Ada",
        email: "ada@example.test",
        email_verified: true,
    };
    let token = encode(
        &Header::new(Algorithm::HS256),
        &BootstrapClaims {
            iss: ISSUER,
            aud: audience,
            sub: BOOTSTRAP_SUBJECT,
            jti: assertion_id.to_string(),
            iat: now,
            nbf: now,
            exp: now + 30,
            actor_kind: BOOTSTRAP_ACTOR_KIND,
            oidc_iss: expected.issuer,
            oidc_sub: expected.subject,
            name: expected.display_name,
            email: expected.email,
            email_verified: expected.email_verified,
        },
        &EncodingKey::from_secret(&key),
    )
    .expect("encode bootstrap assertion");
    assert_eq!(
        authenticator
            .authenticate_bootstrap(&headers(&token), audience, &expected)
            .expect("valid bootstrap"),
        assertion_id
    );

    let altered = BootstrapIdentity {
        email: "attacker@example.test",
        ..expected
    };
    assert!(
        authenticator
            .authenticate_bootstrap(&headers(&token), audience, &altered)
            .is_err()
    );
}

#[test]
fn session_bootstrap_binds_only_verified_oidc_identity() {
    let key = mediator_signing_key(TOKEN);
    let authenticator = MediatorAuthenticator::new(&key);
    let assertion_id = Uuid::new_v4();
    let now = OffsetDateTime::now_utc().unix_timestamp();
    let token = encode(
        &Header::new(Algorithm::HS256),
        &MinimalBootstrapClaims {
            iss: ISSUER,
            aud: CREATE_BOOTSTRAP_AUDIENCE,
            sub: BOOTSTRAP_SUBJECT,
            jti: assertion_id.to_string(),
            iat: now,
            nbf: now,
            exp: now + 30,
            actor_kind: BOOTSTRAP_ACTOR_KIND,
            oidc_iss: "https://issuer.example",
            oidc_sub: "external-subject",
        },
        &EncodingKey::from_secret(&key),
    )
    .expect("encode session bootstrap assertion");
    let headers = headers(&token);
    assert_eq!(
        authenticator
            .authenticate_session_bootstrap(
                &headers,
                CREATE_BOOTSTRAP_AUDIENCE,
                "https://issuer.example",
                "external-subject",
            )
            .expect("valid session bootstrap"),
        assertion_id
    );
    assert!(
        authenticator
            .authenticate_session_bootstrap(
                &headers,
                CREATE_BOOTSTRAP_AUDIENCE,
                "https://attacker.example",
                "external-subject",
            )
            .is_err()
    );
    assert!(
        authenticator
            .authenticate_session_bootstrap(
                &headers,
                CREATE_BOOTSTRAP_AUDIENCE,
                "https://issuer.example",
                "other-subject",
            )
            .is_err()
    );
}
