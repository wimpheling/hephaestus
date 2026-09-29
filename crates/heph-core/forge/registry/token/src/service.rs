use jsonwebtoken::{Algorithm, DecodingKey, Header, Validation, decode, decode_header, encode};
use std::collections::BTreeMap;
use uuid::Uuid;

use crate::validation::{validate_access, validate_times};
use crate::{
    AuthorizationDecision, BearerToken, IssuedToken, KeyId, RegistryAccess, RegistryService,
    RegistryTokenClaims, RegistryTokenError, ScopeRequest, SigningKey, TokenIssuer, TokenLifetime,
    TokenSubject, UnixTimestamp, VerificationKey,
};

/// A JWT signing service for one issuer, registry audience, and active key.
pub struct RegistryTokenIssuer {
    issuer: TokenIssuer,
    service: RegistryService,
    key: SigningKey,
    lifetime: TokenLifetime,
}

impl RegistryTokenIssuer {
    /// Creates a token issuer with one active signing key.
    #[must_use]
    pub const fn new(
        issuer: TokenIssuer,
        service: RegistryService,
        key: SigningKey,
        lifetime: TokenLifetime,
    ) -> Self {
        Self {
            issuer,
            service,
            key,
            lifetime,
        }
    }

    /// Returns the exact registry service used as the token audience.
    #[must_use]
    pub const fn service(&self) -> &RegistryService {
        &self.service
    }

    /// Issues a token containing only the requested and authorized actions.
    ///
    /// A valid request with no authorized actions produces an empty `access`
    /// claim. This lets the caller return a standards-compatible token without
    /// accidentally turning a denial into a broad grant.
    ///
    /// # Errors
    ///
    /// Returns an error for a mismatched requested service, timestamp
    /// overflow, or JWT encoding failure.
    pub fn issue(
        &self,
        subject: TokenSubject,
        request: &ScopeRequest,
        authorization: &AuthorizationDecision,
        now: UnixTimestamp,
    ) -> Result<IssuedToken, RegistryTokenError> {
        if request.service != self.service {
            return Err(RegistryTokenError::ServiceMismatch);
        }
        let expiration = now
            .0
            .checked_add(self.lifetime.0)
            .ok_or(RegistryTokenError::TimestampOverflow)?;
        let access = request
            .scopes
            .iter()
            .filter_map(|scope| {
                let actions = scope
                    .actions
                    .intersection(authorization.actions_for(&scope.repository));
                (!actions.is_empty())
                    .then(|| RegistryAccess::from_grant(&scope.repository, actions))
            })
            .collect();
        let claims = RegistryTokenClaims {
            iss: self.issuer.clone(),
            aud: self.service.clone(),
            sub: subject,
            iat: now.0,
            nbf: now.0,
            exp: expiration,
            jti: Uuid::new_v4(),
            access,
        };
        let mut header = Header::new(self.key.algorithm);
        header.kid = Some(self.key.key_id.as_str().to_owned());
        let token = encode(&header, &claims, &self.key.key).map_err(RegistryTokenError::Encode)?;
        Ok(IssuedToken {
            token: BearerToken(token),
            expires_in: self.lifetime,
            claims,
        })
    }
}

/// A JWT verifier with all currently valid verification keys.
pub struct RegistryTokenVerifier {
    issuer: TokenIssuer,
    service: RegistryService,
    keys: BTreeMap<KeyId, (Algorithm, DecodingKey)>,
    maximum_lifetime: TokenLifetime,
}

impl RegistryTokenVerifier {
    /// Creates a verifier that accepts all supplied rotation-window keys.
    ///
    /// # Errors
    ///
    /// Returns an error when no keys are supplied or key identifiers repeat.
    pub fn new(
        issuer: TokenIssuer,
        service: RegistryService,
        keys: impl IntoIterator<Item = VerificationKey>,
        maximum_lifetime: TokenLifetime,
    ) -> Result<Self, RegistryTokenError> {
        let mut resolved_keys = BTreeMap::new();
        for key in keys {
            if resolved_keys
                .insert(key.key_id, (key.algorithm, key.key))
                .is_some()
            {
                return Err(RegistryTokenError::DuplicateKeyId);
            }
        }
        if resolved_keys.is_empty() {
            return Err(RegistryTokenError::NoVerificationKeys);
        }
        Ok(Self {
            issuer,
            service,
            keys: resolved_keys,
            maximum_lifetime,
        })
    }

    /// Verifies a token's signature, key identifier, claims, and time bounds.
    ///
    /// # Errors
    ///
    /// Returns a typed failure without including the bearer token or key
    /// material in its message.
    pub fn verify(
        &self,
        token: &BearerToken,
        now: UnixTimestamp,
    ) -> Result<RegistryTokenClaims, RegistryTokenError> {
        let header = decode_header(token.as_str()).map_err(RegistryTokenError::Decode)?;
        let key_id = header
            .kid
            .as_deref()
            .ok_or(RegistryTokenError::MissingKeyId)?
            .parse()?;
        let (algorithm, key) = self
            .keys
            .get(&key_id)
            .ok_or(RegistryTokenError::UnknownKeyId)?;
        if header.alg != *algorithm {
            return Err(RegistryTokenError::UnexpectedAlgorithm);
        }
        let mut validation = Validation::new(*algorithm);
        validation.validate_exp = false;
        validation.validate_nbf = false;
        validation.validate_aud = false;
        validation.set_required_spec_claims(&["iss", "aud", "sub", "iat", "nbf", "exp", "jti"]);
        let claims = decode::<RegistryTokenClaims>(token.as_str(), key, &validation)
            .map_err(RegistryTokenError::Decode)?
            .claims;
        if claims.iss != self.issuer {
            return Err(RegistryTokenError::IssuerMismatch);
        }
        if claims.aud != self.service {
            return Err(RegistryTokenError::AudienceMismatch);
        }
        validate_times(&claims, now, self.maximum_lifetime)?;
        validate_access(&claims.access)?;
        Ok(claims)
    }
}
