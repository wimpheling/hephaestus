use super::*;

pub fn golden_registry_config() -> RegistryConfig {
    let local = env::var("HEPHAESTUS_TEST_REGISTRY_SERVICE")
        .ok()
        .zip(env::var("HEPHAESTUS_TEST_REGISTRY_ORIGIN").ok())
        .zip(env::var("HEPHAESTUS_TEST_REGISTRY_PRIVATE_KEY").ok())
        .zip(env::var("HEPHAESTUS_TEST_REGISTRY_KEY_ID").ok());
    let (token_issuer, zot) = if let Some((((service, origin), key_path), key_id)) = local {
        let authority = RegistryAuthority::parse(&service).expect("local golden registry service");
        let key = std::fs::read(key_path).expect("local golden registry signing key");
        let issuer = RegistryTokenIssuer::new(
            "http://127.0.0.1:0/v1/registry/token"
                .parse()
                .expect("local golden registry issuer"),
            service.parse().expect("local golden registry audience"),
            SigningKey::rs256_pem(key_id.parse().expect("local golden registry key ID"), &key)
                .expect("local golden registry signing key material"),
            TokenLifetime::new(300).expect("local golden registry token lifetime"),
        );
        (
            Arc::new(issuer),
            registry_zot::ZotClientConfig::new(authority, &origin)
                .expect("local golden Zot configuration"),
        )
    } else {
        let authority =
            RegistryAuthority::parse("registry.golden.invalid").expect("registry service");
        (
            Arc::new(RegistryTokenIssuer::new(
                "https://forge.golden.invalid/v1/registry/token"
                    .parse()
                    .expect("registry issuer"),
                "registry.golden.invalid".parse().expect("registry service"),
                SigningKey::hs256(
                    "golden-v1".parse().expect("registry key id"),
                    SIGNING_SECRET,
                )
                .expect("registry signing key"),
                TokenLifetime::new(300).expect("registry token lifetime"),
            )),
            registry_zot::ZotClientConfig::new(authority, "http://127.0.0.1:1/")
                .expect("Zot client configuration"),
        )
    };
    RegistryConfig {
        token_issuer,
        notification_callback: registry_notification::CallbackCredential::parse(
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        )
        .expect("registry notification callback"),
        zot,
        reconciliation_lease: Duration::from_secs(30),
        reconciliation_interval: Duration::from_secs(30),
    }
}
