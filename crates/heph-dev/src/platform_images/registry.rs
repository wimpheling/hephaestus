use crate::{
    context::DevContext,
    process::{DevError, Result},
};
use registry_token::{
    AuthorizationDecision, KeyId, RegistryService, RegistryTokenIssuer, RepositoryActions,
    RepositoryName, ScopeRequest, SigningKey, TokenIssuer, TokenLifetime, TokenSubject,
    UnixTimestamp,
};
use std::fs;
use time::OffsetDateTime;
use zeroize::Zeroizing;

pub(super) fn issue_zot_token(
    context: &DevContext,
    repository: &str,
    actions: RepositoryActions,
) -> Result<String> {
    let service = context
        .zot_service()
        .parse::<RegistryService>()
        .map_err(|error| DevError::Invalid(format!("invalid local Zot service: {error}")))?;
    let private_key = Zeroizing::new(fs::read(context.zot_signing_key())?);
    let key = SigningKey::rs256_pem(
        "local-v1"
            .parse::<KeyId>()
            .map_err(|error| DevError::Invalid(format!("invalid local Zot key id: {error}")))?,
        &private_key,
    )
    .map_err(|error| DevError::Invalid(format!("invalid local Zot signing key: {error}")))?;
    let issuer = RegistryTokenIssuer::new(
        DevContext::zot_token_realm()
            .parse::<TokenIssuer>()
            .map_err(|error| DevError::Invalid(format!("invalid local Zot issuer: {error}")))?,
        service.clone(),
        key,
        TokenLifetime::new(300).map_err(|error| {
            DevError::Invalid(format!("invalid local Zot token lifetime: {error}"))
        })?,
    );
    let repository = repository
        .parse::<RepositoryName>()
        .map_err(|error| DevError::Invalid(format!("invalid platform base repository: {error}")))?;
    let request = ScopeRequest::parse(
        service.as_str(),
        &format!("repository:{repository}:pull,push"),
    )
    .map_err(|error| DevError::Invalid(format!("invalid local Zot token scope: {error}")))?;
    let mut authorization = AuthorizationDecision::deny_all();
    authorization.grant(repository, actions);
    let now = u64::try_from(OffsetDateTime::now_utc().unix_timestamp()).map_err(|error| {
        DevError::Invalid(format!("invalid local clock for Zot token: {error}"))
    })?;
    issuer
        .issue(
            "workload:platform-base-import"
                .parse::<TokenSubject>()
                .map_err(|error| {
                    DevError::Invalid(format!("invalid local Zot subject: {error}"))
                })?,
            &request,
            &authorization,
            UnixTimestamp::new(now),
        )
        .map(|token| token.token().as_str().to_owned())
        .map_err(|error| DevError::Invalid(format!("could not issue local Zot token: {error}")))
}
