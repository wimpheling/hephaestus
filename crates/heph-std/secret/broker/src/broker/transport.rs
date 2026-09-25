/// Default redirect-free, proxy-free TLS transport.
///
/// It uses deployment-pinned DNS addresses. `reqwest` verifies the upstream
/// certificate and SNI against the exact DNS destination; no
/// invalid-certificate escape hatch is exposed.
use super::{
    common::{
        BrokerAdapterError, BrokerResponse, BrokerStatus, IpAddr, MAX_UPSTREAM_RESPONSE_BYTES,
        SocketAddr, UPSTREAM_TIMEOUT, async_trait,
    },
    types::{BrokeredHttpsMethod, PinnedHttpsTransport, UpstreamHttpsRequest},
    validation::{public_address, valid_dns_destination},
};

/// Default redirect-free, proxy-free TLS transport.
///
/// It uses deployment-pinned DNS addresses. `reqwest` verifies the upstream
/// certificate and SNI against the exact DNS destination; no
/// invalid-certificate escape hatch is exposed.
pub struct ReqwestPinnedHttpsTransport {
    client: reqwest::Client,
}

impl ReqwestPinnedHttpsTransport {
    /// Builds a TLS client pinned to public resolved addresses for one host.
    ///
    /// The caller obtains and rotates these addresses through trusted control
    /// plane DNS resolution. Pinning prevents a guest-selected DNS rebinding.
    ///
    /// # Errors
    ///
    /// Rejects invalid destinations, empty or non-public address pins, and
    /// TLS client construction failures.
    pub fn new(
        destination: &str,
        addresses: &[std::net::IpAddr],
    ) -> Result<Self, BrokerAdapterError> {
        Self::with_pins(destination, 443, addresses, false)
    }

    fn with_pins(
        destination: &str,
        port: u16,
        addresses: &[std::net::IpAddr],
        allow_private_addresses: bool,
    ) -> Result<Self, BrokerAdapterError> {
        if !valid_dns_destination(destination)
            || addresses.is_empty()
            || port == 0
            || (!allow_private_addresses
                && addresses.iter().any(|address| !public_address(*address)))
        {
            return Err(BrokerAdapterError::Rejected);
        }
        let addresses = addresses
            .iter()
            .copied()
            .map(|address| SocketAddr::new(address, port))
            .collect::<Vec<_>>();
        let client = reqwest::Client::builder()
            .https_only(true)
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(UPSTREAM_TIMEOUT)
            .timeout(UPSTREAM_TIMEOUT)
            .resolve_to_addrs(destination, &addresses)
            .build()
            .map_err(|_| BrokerAdapterError::Rejected)?;
        Ok(Self { client })
    }

    #[cfg(test)]
    pub(super) fn test_only_local_pin(
        destination: &str,
        port: u16,
        address: IpAddr,
    ) -> Result<Self, BrokerAdapterError> {
        Self::with_pins(destination, port, &[address], true)
    }

    #[cfg(any(test, feature = "test-fixtures"))]
    pub(super) fn test_only_local_trusted_pin(
        destination: &str,
        port: u16,
        address: IpAddr,
        certificate_pem: &[u8],
    ) -> Result<Self, BrokerAdapterError> {
        if !valid_dns_destination(destination) || port == 0 {
            return Err(BrokerAdapterError::Rejected);
        }
        let certificate = reqwest::Certificate::from_pem(certificate_pem)
            .map_err(|_| BrokerAdapterError::Rejected)?;
        let client = reqwest::Client::builder()
            .https_only(true)
            .no_proxy()
            .tls_built_in_root_certs(false)
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(UPSTREAM_TIMEOUT)
            .timeout(UPSTREAM_TIMEOUT)
            .add_root_certificate(certificate)
            .resolve_to_addrs(destination, &[SocketAddr::new(address, port)])
            .build()
            .map_err(|_| BrokerAdapterError::Rejected)?;
        Ok(Self { client })
    }
}

#[async_trait]
impl PinnedHttpsTransport for ReqwestPinnedHttpsTransport {
    async fn send(
        &self,
        request: UpstreamHttpsRequest,
    ) -> Result<BrokerResponse, BrokerAdapterError> {
        let url = format!("https://{}{}", request.destination, request.path_and_query);
        let method = match request.method {
            BrokeredHttpsMethod::Get => reqwest::Method::GET,
            BrokeredHttpsMethod::Post => reqwest::Method::POST,
            BrokeredHttpsMethod::Put => reqwest::Method::PUT,
            BrokeredHttpsMethod::Delete => reqwest::Method::DELETE,
        };
        let mut headers = reqwest::header::HeaderMap::new();
        for header in request.headers {
            let name = reqwest::header::HeaderName::from_bytes(header.name.as_bytes())
                .map_err(|_| BrokerAdapterError::Rejected)?;
            let value = reqwest::header::HeaderValue::from_str(&header.value)
                .map_err(|_| BrokerAdapterError::Rejected)?;
            headers.append(name, value);
        }
        let response = self
            .client
            .request(method, url)
            .headers(headers)
            .body(request.body)
            .send()
            .await
            .map_err(|_| BrokerAdapterError::Retryable)?;
        let status = response.status();
        if status.is_redirection() {
            return Err(BrokerAdapterError::Rejected);
        }
        let body = response
            .bytes()
            .await
            .map_err(|_| BrokerAdapterError::Retryable)?;
        if body.len()
            > usize::try_from(MAX_UPSTREAM_RESPONSE_BYTES).expect("response bound fits usize")
        {
            return Err(BrokerAdapterError::Rejected);
        }
        Ok(BrokerResponse {
            status: if status.is_server_error() {
                BrokerStatus::Retryable
            } else if status.is_success() {
                BrokerStatus::Succeeded
            } else {
                BrokerStatus::Rejected
            },
            body: body.to_vec(),
        })
    }
}
