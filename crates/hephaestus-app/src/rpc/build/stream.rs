use crate::rpc::RpcError;
use rpc_proto::messages::hephaestus::common::v1::OpaqueId;
use uuid::Uuid;

pub(super) const DEFAULT_MAX_EVENTS: u32 = 256;
pub(super) const MAX_EVENTS: u32 = 1_000;
pub(super) const DEFAULT_MAX_TOTAL_BYTES: u64 = 1024 * 1024;
pub(super) const MAX_TOTAL_BYTES: u64 = 16 * 1024 * 1024;

pub(super) fn parse_id(value: Option<&OpaqueId>) -> Result<Uuid, RpcError> {
    let value = value.ok_or(RpcError::InvalidArgument)?.value.as_str();
    Uuid::parse_str(value).map_err(|_| RpcError::InvalidArgument)
}

pub(super) const fn limits(max_events: u32, max_total_bytes: u64) -> Result<(u32, u64), RpcError> {
    let events = if max_events == 0 {
        DEFAULT_MAX_EVENTS
    } else {
        max_events
    };
    let bytes = if max_total_bytes == 0 {
        DEFAULT_MAX_TOTAL_BYTES
    } else {
        max_total_bytes
    };
    if events > MAX_EVENTS || bytes > MAX_TOTAL_BYTES {
        return Err(RpcError::InvalidArgument);
    }
    Ok((events, bytes))
}

pub(super) fn validate_build_cursor(
    value: Option<&str>,
    id: Uuid,
) -> Result<Option<String>, RpcError> {
    let Some(value) = value.filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    let prefix = format!("v1:build:{id}:");
    if value.starts_with(&prefix) {
        Ok(Some(value.to_owned()))
    } else {
        Err(RpcError::InvalidArgument)
    }
}

pub(super) fn log_cursor(id: Uuid, offset: usize) -> String {
    format!("v1:build-log:{id}:{offset}")
}

pub(super) fn parse_log_cursor(value: Option<&str>, id: Uuid) -> Result<usize, RpcError> {
    let Some(value) = value.filter(|value| !value.is_empty()) else {
        return Ok(0);
    };
    let prefix = format!("v1:build-log:{id}:");
    let offset = value
        .strip_prefix(&prefix)
        .ok_or(RpcError::InvalidArgument)?
        .parse::<usize>()
        .map_err(|_| RpcError::InvalidArgument)?;
    Ok(offset)
}

#[cfg(test)]
mod tests {
    use super::{log_cursor, parse_log_cursor, validate_build_cursor};
    use uuid::Uuid;

    #[test]
    fn cursors_are_scoped_to_the_build() {
        let id = Uuid::new_v4();
        let other = Uuid::new_v4();
        let cursor = format!("v1:build:{id}:123:running:0");
        assert_eq!(
            validate_build_cursor(Some(&cursor), id).unwrap(),
            Some(cursor.clone())
        );
        assert!(validate_build_cursor(Some(&cursor), other).is_err());
    }

    #[test]
    fn log_cursors_round_trip_offsets() {
        let id = Uuid::new_v4();
        let cursor = log_cursor(id, 17);
        assert_eq!(parse_log_cursor(Some(&cursor), id).unwrap(), 17);
        assert!(parse_log_cursor(Some(&cursor), Uuid::new_v4()).is_err());
    }
}
