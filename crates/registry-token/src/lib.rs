//! Strict Docker Distribution-compatible registry bearer tokens.
//!
//! This crate deliberately owns neither caller authentication nor registry
//! namespace authorization. The HTTP and identity adapters parse a caller,
//! resolve live ownership, and inject an [`AuthorizationDecision`]. This
//! crate then intersects that decision with a strictly parsed token request.

use jsonwebtoken::{
    Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, decode_header, encode,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    str::FromStr,
};
use uuid::Uuid;

const MAX_SERVICE_LENGTH: usize = 255;
const MAX_REPOSITORY_LENGTH: usize = 255;
const MAX_CLAIM_TEXT_LENGTH: usize = 512;
const MAX_KEY_ID_LENGTH: usize = 128;
const MIN_HMAC_SECRET_LENGTH: usize = 32;
const MAX_TOKEN_LIFETIME_SECONDS: u64 = 900;

/// A configured registry service, used as the exact JWT audience.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct RegistryService(String);

impl RegistryService {
    /// Returns the canonical service text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for RegistryService {
    type Err = RegistryTokenError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.is_empty()
            || value.len() > MAX_SERVICE_LENGTH
            || !value.bytes().all(is_service_character)
            || value.starts_with(['.', '-', ':'])
            || value.ends_with(['.', '-', ':'])
        {
            return Err(RegistryTokenError::InvalidService);
        }
        let mut pieces = value.split(':');
        let host = pieces.next().ok_or(RegistryTokenError::InvalidService)?;
        let port = pieces.next();
        if pieces.next().is_some()
            || host.split('.').any(str::is_empty)
            || host
                .split('.')
                .any(|label| label.starts_with(['-', '.']) || label.ends_with(['-', '.']))
            || port.is_some_and(|number| {
                number.is_empty()
                    || number.parse::<u16>().map_or(true, |parsed| parsed == 0)
                    || number
                        .parse::<u16>()
                        .is_ok_and(|parsed| parsed.to_string() != number)
            })
        {
            return Err(RegistryTokenError::InvalidService);
        }
        Ok(Self(value.to_owned()))
    }
}

impl TryFrom<String> for RegistryService {
    type Error = RegistryTokenError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}

impl From<RegistryService> for String {
    fn from(value: RegistryService) -> Self {
        value.0
    }
}

impl fmt::Display for RegistryService {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// A canonical OCI repository path.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct RepositoryName(String);

impl RepositoryName {
    /// Returns the canonical repository path.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for RepositoryName {
    type Err = RegistryTokenError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.is_empty()
            || value.len() > MAX_REPOSITORY_LENGTH
            || !value.bytes().all(is_repository_character)
            || value
                .split('/')
                .any(|component| !is_repository_component(component))
        {
            return Err(RegistryTokenError::InvalidRepository);
        }
        Ok(Self(value.to_owned()))
    }
}

impl TryFrom<String> for RepositoryName {
    type Error = RegistryTokenError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}

impl From<RepositoryName> for String {
    fn from(value: RepositoryName) -> Self {
        value.0
    }
}

impl fmt::Display for RepositoryName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// One registry action accepted by this service.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RegistryAction {
    /// Read manifests, blobs, and referrers.
    Pull,
    /// Upload blobs and publish manifests or tags.
    Push,
}

impl RegistryAction {
    const fn bit(self) -> u8 {
        match self {
            Self::Pull => 0b01,
            Self::Push => 0b10,
        }
    }
}

/// A set of pull and/or push actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RepositoryActions(u8);

impl RepositoryActions {
    /// An empty action set.
    #[must_use]
    pub const fn empty() -> Self {
        Self(0)
    }

    /// A pull-only action set.
    #[must_use]
    pub const fn pull() -> Self {
        Self(RegistryAction::Pull.bit())
    }

    /// A push-only action set.
    #[must_use]
    pub const fn push() -> Self {
        Self(RegistryAction::Push.bit())
    }

    /// A pull-and-push action set.
    #[must_use]
    pub const fn pull_push() -> Self {
        Self(Self::pull().0 | Self::push().0)
    }

    /// Returns whether this set includes an action.
    #[must_use]
    pub const fn contains(self, action: RegistryAction) -> bool {
        self.0 & action.bit() != 0
    }

    /// Returns whether no actions are granted.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    const fn intersection(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }

    fn actions(self) -> Vec<RegistryAction> {
        [RegistryAction::Pull, RegistryAction::Push]
            .into_iter()
            .filter(|action| self.contains(*action))
            .collect()
    }
}

/// A requested repository scope.
#[derive(Clone, PartialEq, Eq)]
pub struct RepositoryScope {
    repository: RepositoryName,
    actions: RepositoryActions,
}

impl RepositoryScope {
    /// Returns the requested repository.
    #[must_use]
    pub const fn repository(&self) -> &RepositoryName {
        &self.repository
    }

    /// Returns the requested actions.
    #[must_use]
    pub const fn actions(&self) -> RepositoryActions {
        self.actions
    }
}

/// A parsed Docker bearer-token request.
#[derive(Clone, PartialEq, Eq)]
pub struct ScopeRequest {
    service: RegistryService,
    scopes: Vec<RepositoryScope>,
}

impl ScopeRequest {
    /// Parses one service and its space-separated repository scopes.
    ///
    /// An empty scope string is valid and requests no repository access.
    /// Repeated repositories, wildcard paths, non-repository scope types, and
    /// unsupported actions are rejected rather than normalized permissively.
    ///
    /// # Errors
    ///
    /// Returns a typed error when either parameter is not canonical.
    pub fn parse(service: &str, scopes: &str) -> Result<Self, RegistryTokenError> {
        let service = service.parse()?;
        if scopes.is_empty() {
            return Ok(Self {
                service,
                scopes: Vec::new(),
            });
        }
        if !scopes.is_ascii() || scopes.split(' ').any(str::is_empty) {
            return Err(RegistryTokenError::InvalidScope);
        }

        let mut repositories = BTreeSet::new();
        let mut parsed_scopes = Vec::new();
        for scope in scopes.split(' ') {
            let parsed = parse_repository_scope(scope)?;
            if !repositories.insert(parsed.repository.clone()) {
                return Err(RegistryTokenError::DuplicateRepositoryScope);
            }
            parsed_scopes.push(parsed);
        }
        Ok(Self {
            service,
            scopes: parsed_scopes,
        })
    }

    /// Returns the requested registry service.
    #[must_use]
    pub const fn service(&self) -> &RegistryService {
        &self.service
    }

    /// Returns requested repository scopes in request order.
    #[must_use]
    pub fn scopes(&self) -> &[RepositoryScope] {
        &self.scopes
    }
}

/// The authorization result injected by the caller's live policy adapter.
#[derive(Default)]
pub struct AuthorizationDecision {
    grants: BTreeMap<RepositoryName, RepositoryActions>,
}

impl AuthorizationDecision {
    /// Creates an authorization decision with no repository grants.
    #[must_use]
    pub fn deny_all() -> Self {
        Self::default()
    }

    /// Adds or replaces the actions authorized for a repository.
    pub fn grant(&mut self, repository: RepositoryName, actions: RepositoryActions) {
        if actions.is_empty() {
            self.grants.remove(&repository);
        } else {
            self.grants.insert(repository, actions);
        }
    }

    fn actions_for(&self, repository: &RepositoryName) -> RepositoryActions {
        self.grants
            .get(repository)
            .copied()
            .unwrap_or_else(RepositoryActions::empty)
    }
}

/// A JWT issuer identifier.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct TokenIssuer(String);

impl TokenIssuer {
    /// Returns the issuer identifier.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for TokenIssuer {
    type Err = RegistryTokenError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        validate_claim_text(value).map(|()| Self(value.to_owned()))
    }
}

impl TryFrom<String> for TokenIssuer {
    type Error = RegistryTokenError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}

impl From<TokenIssuer> for String {
    fn from(value: TokenIssuer) -> Self {
        value.0
    }
}

/// A stable caller subject included in an issued token.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct TokenSubject(String);

impl TokenSubject {
    /// Returns the stable subject text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for TokenSubject {
    type Err = RegistryTokenError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        validate_claim_text(value).map(|()| Self(value.to_owned()))
    }
}

impl TryFrom<String> for TokenSubject {
    type Error = RegistryTokenError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}

impl From<TokenSubject> for String {
    fn from(value: TokenSubject) -> Self {
        value.0
    }
}

/// A non-secret key identifier carried in the JWT `kid` header.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct KeyId(String);

impl KeyId {
    /// Returns the key identifier text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for KeyId {
    type Err = RegistryTokenError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.is_empty()
            || value.len() > MAX_KEY_ID_LENGTH
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        {
            return Err(RegistryTokenError::InvalidKeyId);
        }
        Ok(Self(value.to_owned()))
    }
}

/// A bounded short-lived token duration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TokenLifetime(u64);

impl TokenLifetime {
    /// Creates a duration from one second through fifteen minutes.
    ///
    /// # Errors
    ///
    /// Returns an error for zero or excessively long lifetimes.
    pub const fn new(seconds: u64) -> Result<Self, RegistryTokenError> {
        if seconds == 0 || seconds > MAX_TOKEN_LIFETIME_SECONDS {
            Err(RegistryTokenError::InvalidLifetime)
        } else {
            Ok(Self(seconds))
        }
    }

    /// Returns the duration in seconds.
    #[must_use]
    pub const fn seconds(self) -> u64 {
        self.0
    }
}

/// A Unix timestamp supplied by the transport or runtime clock adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct UnixTimestamp(u64);

impl UnixTimestamp {
    /// Creates a timestamp from whole Unix seconds.
    #[must_use]
    pub const fn new(seconds: u64) -> Self {
        Self(seconds)
    }

    /// Returns whole Unix seconds.
    #[must_use]
    pub const fn seconds(self) -> u64 {
        self.0
    }
}

/// HMAC-SHA256 signing material held by the secret/runtime adapter.
pub struct SigningKey {
    key_id: KeyId,
    algorithm: Algorithm,
    key: EncodingKey,
}

impl SigningKey {
    /// Creates HMAC-SHA256 signing material.
    ///
    /// # Errors
    ///
    /// Returns an error when the supplied key material is too short.
    pub fn hs256(key_id: KeyId, secret: &[u8]) -> Result<Self, RegistryTokenError> {
        if secret.len() < MIN_HMAC_SECRET_LENGTH {
            return Err(RegistryTokenError::InsufficientKeyMaterial);
        }
        Ok(Self {
            key_id,
            algorithm: Algorithm::HS256,
            key: EncodingKey::from_secret(secret),
        })
    }

    /// Creates RSA-SHA256 signing material from a private PEM key.
    ///
    /// # Errors
    ///
    /// Returns an error when the PEM does not contain a usable RSA private key.
    pub fn rs256_pem(key_id: KeyId, private_key_pem: &[u8]) -> Result<Self, RegistryTokenError> {
        let key = EncodingKey::from_rsa_pem(private_key_pem)
            .map_err(RegistryTokenError::InvalidKeyMaterial)?;
        Ok(Self {
            key_id,
            algorithm: Algorithm::RS256,
            key,
        })
    }
}

/// HMAC-SHA256 verification material that can overlap during key rotation.
pub struct VerificationKey {
    key_id: KeyId,
    algorithm: Algorithm,
    key: DecodingKey,
}

impl VerificationKey {
    /// Creates HMAC-SHA256 verification material.
    ///
    /// # Errors
    ///
    /// Returns an error when the supplied key material is too short.
    pub fn hs256(key_id: KeyId, secret: &[u8]) -> Result<Self, RegistryTokenError> {
        if secret.len() < MIN_HMAC_SECRET_LENGTH {
            return Err(RegistryTokenError::InsufficientKeyMaterial);
        }
        Ok(Self {
            key_id,
            algorithm: Algorithm::HS256,
            key: DecodingKey::from_secret(secret),
        })
    }

    /// Creates RSA-SHA256 verification material from a public PEM key or
    /// certificate accepted by `jsonwebtoken`.
    ///
    /// # Errors
    ///
    /// Returns an error when the PEM does not contain usable RSA public key
    /// material.
    pub fn rs256_pem(key_id: KeyId, public_key_pem: &[u8]) -> Result<Self, RegistryTokenError> {
        let key = DecodingKey::from_rsa_pem(public_key_pem)
            .map_err(RegistryTokenError::InvalidKeyMaterial)?;
        Ok(Self {
            key_id,
            algorithm: Algorithm::RS256,
            key,
        })
    }
}

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

/// A signed bearer token.
pub struct BearerToken(String);

impl BearerToken {
    /// Returns the token only for writing the bearer-token HTTP response.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for BearerToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("BearerToken(REDACTED)")
    }
}

/// An issued bearer token and its non-secret response metadata.
pub struct IssuedToken {
    token: BearerToken,
    expires_in: TokenLifetime,
    claims: RegistryTokenClaims,
}

impl IssuedToken {
    /// Returns the signed bearer token for the HTTP response body.
    #[must_use]
    pub const fn token(&self) -> &BearerToken {
        &self.token
    }

    /// Returns the requested token lifetime.
    #[must_use]
    pub const fn expires_in(&self) -> TokenLifetime {
        self.expires_in
    }

    /// Returns the claims that were signed.
    #[must_use]
    pub const fn claims(&self) -> &RegistryTokenClaims {
        &self.claims
    }
}

impl fmt::Debug for IssuedToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("IssuedToken")
            .field("token", &"REDACTED")
            .field("expires_in", &self.expires_in)
            .finish_non_exhaustive()
    }
}

/// Docker Distribution-compatible signed registry claims.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistryTokenClaims {
    /// Exact configured token issuer.
    pub iss: TokenIssuer,
    /// Exact registry service audience.
    pub aud: RegistryService,
    /// Stable caller subject.
    pub sub: TokenSubject,
    /// Issued-at Unix timestamp.
    pub iat: u64,
    /// Not-before Unix timestamp.
    pub nbf: u64,
    /// Expiry Unix timestamp.
    pub exp: u64,
    /// Unique token identifier.
    pub jti: Uuid,
    /// Docker Distribution repository access entries.
    pub access: Vec<RegistryAccess>,
}

/// One Docker Distribution `access` claim entry.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistryAccess {
    #[serde(rename = "type")]
    resource_type: String,
    name: RepositoryName,
    actions: Vec<RegistryAction>,
}

impl RegistryAccess {
    fn from_grant(repository: &RepositoryName, actions: RepositoryActions) -> Self {
        Self {
            resource_type: "repository".to_owned(),
            name: repository.clone(),
            actions: actions.actions(),
        }
    }

    /// Returns the granted repository.
    #[must_use]
    pub const fn repository(&self) -> &RepositoryName {
        &self.name
    }

    /// Returns the granted actions in canonical pull, push order.
    #[must_use]
    pub fn actions(&self) -> &[RegistryAction] {
        &self.actions
    }
}

/// Registry token parsing, issuance, or verification failure.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RegistryTokenError {
    /// The registry service was not canonical.
    #[error("registry service is invalid")]
    InvalidService,
    /// The repository path was not canonical.
    #[error("repository name is invalid")]
    InvalidRepository,
    /// The scope did not use the exact repository grammar.
    #[error("registry scope is invalid")]
    InvalidScope,
    /// A scope included an unsupported or repeated action.
    #[error("registry scope actions are invalid")]
    InvalidActions,
    /// Multiple scopes tried to describe one repository.
    #[error("registry request repeats a repository scope")]
    DuplicateRepositoryScope,
    /// An issuer or subject contained invalid claim text.
    #[error("registry token claim text is invalid")]
    InvalidClaimText,
    /// A key identifier was invalid.
    #[error("registry key identifier is invalid")]
    InvalidKeyId,
    /// Token lifetime was zero or longer than the allowed maximum.
    #[error("registry token lifetime is invalid")]
    InvalidLifetime,
    /// HMAC key material was too short.
    #[error("registry signing material is insufficient")]
    InsufficientKeyMaterial,
    /// Asymmetric signing or verification key material could not be parsed.
    #[error("registry key material is invalid")]
    InvalidKeyMaterial(#[source] jsonwebtoken::errors::Error),
    /// The requested service did not match this issuer's configured audience.
    #[error("registry requested service does not match")]
    ServiceMismatch,
    /// Adding a lifetime to the supplied timestamp overflowed.
    #[error("registry token timestamp overflowed")]
    TimestampOverflow,
    /// JWT encoding failed.
    #[error("registry token encoding failed")]
    Encode(#[source] jsonwebtoken::errors::Error),
    /// JWT header or signature decoding failed.
    #[error("registry token decoding failed")]
    Decode(#[source] jsonwebtoken::errors::Error),
    /// The token did not carry a key identifier.
    #[error("registry token key identifier is missing")]
    MissingKeyId,
    /// No configured verifier key matched the token key identifier.
    #[error("registry token key identifier is unknown")]
    UnknownKeyId,
    /// The token algorithm did not match the selected verification key.
    #[error("registry token algorithm is invalid")]
    UnexpectedAlgorithm,
    /// A verifier key set contained no keys.
    #[error("registry verifier has no keys")]
    NoVerificationKeys,
    /// A verifier key set repeated a key identifier.
    #[error("registry verifier repeats a key identifier")]
    DuplicateKeyId,
    /// The token issuer did not match exactly.
    #[error("registry token issuer does not match")]
    IssuerMismatch,
    /// The token audience did not match exactly.
    #[error("registry token audience does not match")]
    AudienceMismatch,
    /// The token has expired.
    #[error("registry token has expired")]
    Expired,
    /// The token is not yet valid.
    #[error("registry token is not yet valid")]
    NotYetValid,
    /// The token issued-at timestamp is in the future.
    #[error("registry token issued-at timestamp is in the future")]
    IssuedInFuture,
    /// The token's time bounds are internally inconsistent.
    #[error("registry token time bounds are invalid")]
    InvalidTimeBounds,
    /// The token lifetime exceeds this verifier's policy.
    #[error("registry token lifetime exceeds verifier policy")]
    LifetimeExceeded,
    /// The signed access entries were not canonical.
    #[error("registry token access claims are invalid")]
    InvalidAccessClaims,
}

const fn is_service_character(byte: u8) -> bool {
    byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'-' | b':')
}

const fn is_repository_character(byte: u8) -> bool {
    byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'_' | b'-' | b'/')
}

fn is_repository_component(component: &str) -> bool {
    let bytes = component.as_bytes();
    if bytes.is_empty() || !bytes[0].is_ascii_alphanumeric() {
        return false;
    }
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            byte if byte.is_ascii_alphanumeric() => index += 1,
            b'-' => {
                while bytes.get(index) == Some(&b'-') {
                    index += 1;
                }
                if !bytes.get(index).is_some_and(u8::is_ascii_alphanumeric) {
                    return false;
                }
            }
            b'.' => {
                index += 1;
                if !bytes.get(index).is_some_and(u8::is_ascii_alphanumeric) {
                    return false;
                }
            }
            b'_' => {
                index += 1;
                if bytes.get(index) == Some(&b'_') {
                    index += 1;
                }
                if !bytes.get(index).is_some_and(u8::is_ascii_alphanumeric) {
                    return false;
                }
            }
            _ => return false,
        }
    }
    true
}

fn validate_claim_text(value: &str) -> Result<(), RegistryTokenError> {
    if value.is_empty()
        || value.len() > MAX_CLAIM_TEXT_LENGTH
        || !value
            .bytes()
            .all(|byte| byte.is_ascii() && !byte.is_ascii_control() && byte != b' ')
    {
        Err(RegistryTokenError::InvalidClaimText)
    } else {
        Ok(())
    }
}

fn parse_repository_scope(scope: &str) -> Result<RepositoryScope, RegistryTokenError> {
    let mut parts = scope.split(':');
    let scope_type = parts.next();
    let repository = parts.next();
    let actions = parts.next();
    if scope_type != Some("repository") || parts.next().is_some() {
        return Err(RegistryTokenError::InvalidScope);
    }
    let repository = repository
        .ok_or(RegistryTokenError::InvalidScope)?
        .parse()?;
    let actions = parse_actions(actions.ok_or(RegistryTokenError::InvalidScope)?)?;
    Ok(RepositoryScope {
        repository,
        actions,
    })
}

fn parse_actions(value: &str) -> Result<RepositoryActions, RegistryTokenError> {
    let mut actions = RepositoryActions::empty();
    for action in value.split(',') {
        let parsed = match action {
            "pull" => RegistryAction::Pull,
            "push" => RegistryAction::Push,
            _ => return Err(RegistryTokenError::InvalidActions),
        };
        if actions.contains(parsed) {
            return Err(RegistryTokenError::InvalidActions);
        }
        actions.0 |= parsed.bit();
    }
    if actions.is_empty() {
        Err(RegistryTokenError::InvalidActions)
    } else {
        Ok(actions)
    }
}

const fn validate_times(
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

fn validate_access(access: &[RegistryAccess]) -> Result<(), RegistryTokenError> {
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

#[cfg(test)]
mod tests {
    use super::{
        AuthorizationDecision, BearerToken, KeyId, RegistryAction, RegistryService,
        RegistryTokenError, RegistryTokenIssuer, RegistryTokenVerifier, RepositoryActions,
        RepositoryName, ScopeRequest, SigningKey, TokenLifetime, TokenSubject, UnixTimestamp,
        VerificationKey,
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
    const NOW: UnixTimestamp = UnixTimestamp::new(1_700_000_000);

    fn issuer() -> RegistryTokenIssuer {
        RegistryTokenIssuer::new(
            "https://forge.example/registry".parse().expect("issuer"),
            "registry.forge.example:5000".parse().expect("service"),
            SigningKey::hs256("active-2026".parse().expect("key id"), SECRET_A)
                .expect("signing key"),
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

    fn request(scopes: &str) -> ScopeRequest {
        ScopeRequest::parse("registry.forge.example:5000", scopes).expect("scope request")
    }

    fn subject() -> TokenSubject {
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
            [
                VerificationKey::hs256("active-2026".parse().expect("key id"), SECRET_A)
                    .expect("key"),
            ],
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
            [
                VerificationKey::hs256("active-2026".parse().expect("key id"), SECRET_A)
                    .expect("key"),
            ],
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
                VerificationKey::hs256("old-2025".parse().expect("key id"), SECRET_B)
                    .expect("old key"),
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

    #[test]
    fn token_debug_is_redacted() {
        let issued = issuer()
            .issue(
                subject(),
                &request("repository:platform/builders/rust-ubuntu:pull"),
                &AuthorizationDecision::deny_all(),
                NOW,
            )
            .expect("token");
        let token = issued.token().as_str();
        assert!(!format!("{issued:?}").contains(token));
        assert!(!format!("{:?}", issued.token()).contains(token));
        assert!(format!("{issued:?}").contains("REDACTED"));
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
}
