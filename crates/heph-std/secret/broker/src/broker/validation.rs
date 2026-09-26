use super::{
    common::{
        BrokerAdapterError, BrokeredEgressError, BrokeredSecretRule, HttpInjectionLocation,
        MAX_ADAPTER_BODY_BYTES, MAX_HTTPS_HEADERS, MAX_HTTPS_PATH_BYTES, VerifiedBrokeredHttpsRule,
    },
    types::{BrokeredHttpsHeader, BrokeredHttpsRequest},
};

pub(super) fn validate_and_substitute_headers(
    request: &BrokeredHttpsRequest,
    rule: &BrokeredSecretRule,
    credential: &[u8],
) -> Result<Vec<BrokeredHttpsHeader>, BrokerAdapterError> {
    let (rule_header, value_prefix) = match &rule.location {
        HttpInjectionLocation::OutboundHeaderValue { header } => (header.as_str(), ""),
        HttpInjectionLocation::OutboundHeaderPrefix { header, prefix } => {
            (header.as_str(), prefix.as_str())
        }
        HttpInjectionLocation::InboundGatewayHeader { .. } => {
            return Err(BrokerAdapterError::Rejected);
        }
    };
    let placeholder = rule.placeholder();
    validate_and_substitute_headers_with_binding(
        request,
        rule_header,
        value_prefix,
        &placeholder,
        credential,
    )
}

pub(super) fn validate_and_substitute_headers_with_binding(
    request: &BrokeredHttpsRequest,
    rule_header: &str,
    value_prefix: &str,
    placeholder: &str,
    credential: &[u8],
) -> Result<Vec<BrokeredHttpsHeader>, BrokerAdapterError> {
    if request.headers.len() > MAX_HTTPS_HEADERS || request.body.len() > MAX_ADAPTER_BODY_BYTES {
        return Err(BrokerAdapterError::Rejected);
    }
    let mut seen = std::collections::HashSet::new();
    let mut substitutions = 0_u8;
    let mut headers = Vec::with_capacity(request.headers.len());
    for header in &request.headers {
        if !valid_header_name(&header.name)
            || header.name == "host"
            || header.name == "proxy-authorization"
            || contains_http_control(&header.value)
            || !seen.insert(header.name.as_str())
        {
            return Err(BrokerAdapterError::Rejected);
        }
        let value = if header.name == rule_header {
            let expected = format!("{value_prefix}{placeholder}");
            if header.value != expected || credential.is_empty() {
                return Err(BrokerAdapterError::Rejected);
            }
            substitutions = substitutions.saturating_add(1);
            let mut replaced = String::with_capacity(value_prefix.len() + credential.len());
            replaced.push_str(value_prefix);
            let suffix =
                std::str::from_utf8(credential).map_err(|_| BrokerAdapterError::Rejected)?;
            replaced.push_str(suffix);
            replaced
        } else {
            header.value.clone()
        };
        headers.push(BrokeredHttpsHeader {
            name: header.name.clone(),
            value,
        });
    }
    if substitutions != 1 {
        return Err(BrokerAdapterError::Rejected);
    }
    Ok(headers)
}

pub(super) fn validate_and_substitute_verified_headers(
    request: &BrokeredHttpsRequest,
    rule: &VerifiedBrokeredHttpsRule,
    credential: &[u8],
) -> Result<Vec<BrokeredHttpsHeader>, BrokerAdapterError> {
    let value_prefix = rule.header_prefix.as_deref().unwrap_or("");
    let placeholder = format!("heph-placeholder:v1:{}", rule.rule_id);
    validate_and_substitute_headers_with_binding(
        request,
        &rule.header_name,
        value_prefix,
        &placeholder,
        credential,
    )
}

pub(super) const fn map_rule_error(_error: BrokeredEgressError) -> BrokerAdapterError {
    BrokerAdapterError::Rejected
}

pub(super) fn validate_https_path(value: &str) -> Result<String, BrokerAdapterError> {
    if !(1..=MAX_HTTPS_PATH_BYTES).contains(&value.len())
        || !value.starts_with('/')
        || value.starts_with("//")
        || value.contains('#')
        || contains_http_control(value)
    {
        return Err(BrokerAdapterError::Rejected);
    }
    Ok(value.to_owned())
}

fn valid_header_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn contains_http_control(value: &str) -> bool {
    value
        .bytes()
        .any(|byte| matches!(byte, b'\r' | b'\n' | b'\0'))
}

pub(super) const fn public_address(address: std::net::IpAddr) -> bool {
    match address {
        std::net::IpAddr::V4(address) => {
            !address.is_private()
                && !address.is_loopback()
                && !address.is_link_local()
                && !address.is_broadcast()
                && !address.is_unspecified()
                && !address.is_multicast()
                && address.octets()[0] != 0
        }
        std::net::IpAddr::V6(address) => {
            !address.is_loopback()
                && !address.is_unspecified()
                && !address.is_multicast()
                && !address.is_unicast_link_local()
                && !address.is_unique_local()
        }
    }
}

pub(super) fn valid_dns_destination(value: &str) -> bool {
    let labels = value.split('.').collect::<Vec<_>>();
    (1..=253).contains(&value.len())
        && labels.len() >= 2
        && labels.iter().all(|label| {
            (1..=63).contains(&label.len())
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
                && label
                    .as_bytes()
                    .first()
                    .is_some_and(u8::is_ascii_alphanumeric)
                && label
                    .as_bytes()
                    .last()
                    .is_some_and(u8::is_ascii_alphanumeric)
        })
        && value.parse::<std::net::IpAddr>().is_err()
        && !matches!(
            labels.last().copied(),
            Some("internal" | "local" | "localhost")
        )
}

pub(super) const fn valid_bearer_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~' | b'+' | b'/' | b'=')
}
