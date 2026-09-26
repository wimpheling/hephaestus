use std::collections::BTreeSet;

use crate::{
    RegistryAccess, RegistryTokenClaims, RegistryTokenError, TokenLifetime, UnixTimestamp,
};

pub const fn validate_times(
    claims: &RegistryTokenClaims,
    now: UnixTimestamp,
    maximum_lifetime: TokenLifetime,
) -> Result<(), RegistryTokenError> {
    if claims.nbf < claims.iat || claims.exp <= claims.nbf {
        return Err(RegistryTokenError::InvalidTimeBounds);
    }
    if claims.exp - claims.iat > maximum_lifetime.0 {
        return Err(RegistryTokenError::LifetimeExceeded);
    }
    if claims.iat > now.0 {
        return Err(RegistryTokenError::IssuedInFuture);
    }
    if claims.nbf > now.0 {
        return Err(RegistryTokenError::NotYetValid);
    }
    if claims.exp <= now.0 {
        return Err(RegistryTokenError::Expired);
    }
    Ok(())
}

pub fn validate_access(access: &[RegistryAccess]) -> Result<(), RegistryTokenError> {
    let mut repositories = BTreeSet::new();
    for entry in access {
        if entry.resource_type != "repository"
            || entry.actions.is_empty()
            || !repositories.insert(entry.name.clone())
        {
            return Err(RegistryTokenError::InvalidAccessClaims);
        }
        let mut actions = BTreeSet::new();
        if entry.actions.iter().any(|action| !actions.insert(*action)) {
            return Err(RegistryTokenError::InvalidAccessClaims);
        }
    }
    Ok(())
}
