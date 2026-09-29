//! Strict timestamp parsing for Zot `CloudEvents`.

use crate::NotificationError;
use std::str::FromStr;
use time::{Date, Month, OffsetDateTime, PrimitiveDateTime, Time, UtcOffset};

pub fn parse_rfc3339(value: &str) -> Result<OffsetDateTime, NotificationError> {
    let Some((date, time_and_offset)) = value.split_once('T') else {
        return Err(NotificationError::InvalidTimestamp);
    };
    let mut date_parts = date.split('-');
    let (Some(year), Some(month), Some(day), None) = (
        date_parts.next(),
        date_parts.next(),
        date_parts.next(),
        date_parts.next(),
    ) else {
        return Err(NotificationError::InvalidTimestamp);
    };
    let year = parse_fixed_decimal::<i32>(year, 4)?;
    let month = Month::try_from(parse_fixed_decimal::<u8>(month, 2)?)
        .map_err(|_| NotificationError::InvalidTimestamp)?;
    let day = parse_fixed_decimal::<u8>(day, 2)?;
    let date = Date::from_calendar_date(year, month, day)
        .map_err(|_| NotificationError::InvalidTimestamp)?;

    let (time, offset) = if let Some(time) = time_and_offset.strip_suffix('Z') {
        (time, UtcOffset::UTC)
    } else {
        let Some(offset_index) = time_and_offset
            .char_indices()
            .skip(8)
            .find_map(|(index, character)| matches!(character, '+' | '-').then_some(index))
        else {
            return Err(NotificationError::InvalidTimestamp);
        };
        let (time, offset) = time_and_offset.split_at(offset_index);
        (time, parse_utc_offset(offset)?)
    };
    let time = parse_time(time)?;

    Ok(PrimitiveDateTime::new(date, time).assume_offset(offset))
}

fn parse_time(value: &str) -> Result<Time, NotificationError> {
    let (whole_seconds, fractional) = match value.split_once('.') {
        Some((whole_seconds, fractional)) => (whole_seconds, Some(fractional)),
        None => (value, None),
    };
    let mut time_parts = whole_seconds.split(':');
    let (Some(hour), Some(minute), Some(second), None) = (
        time_parts.next(),
        time_parts.next(),
        time_parts.next(),
        time_parts.next(),
    ) else {
        return Err(NotificationError::InvalidTimestamp);
    };
    let nanosecond = fractional.map_or(Ok(0), parse_nanosecond)?;
    Time::from_hms_nano(
        parse_fixed_decimal::<u8>(hour, 2)?,
        parse_fixed_decimal::<u8>(minute, 2)?,
        parse_fixed_decimal::<u8>(second, 2)?,
        nanosecond,
    )
    .map_err(|_| NotificationError::InvalidTimestamp)
}

fn parse_utc_offset(value: &str) -> Result<UtcOffset, NotificationError> {
    if value.len() != 6
        || !matches!(value.as_bytes().first(), Some(b'+' | b'-'))
        || value.as_bytes()[3] != b':'
    {
        return Err(NotificationError::InvalidTimestamp);
    }
    let sign = if value.starts_with('-') { -1 } else { 1 };
    let hours = parse_fixed_decimal::<i8>(&value[1..3], 2)?;
    let minutes = parse_fixed_decimal::<i8>(&value[4..6], 2)?;
    UtcOffset::from_hms(sign * hours, sign * minutes, 0)
        .map_err(|_| NotificationError::InvalidTimestamp)
}

fn parse_nanosecond(value: &str) -> Result<u32, NotificationError> {
    if value.is_empty() || value.len() > 9 || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(NotificationError::InvalidTimestamp);
    }
    let parsed = value
        .parse::<u32>()
        .map_err(|_| NotificationError::InvalidTimestamp)?;
    let scale = 9_u32
        .checked_sub(u32::try_from(value.len()).map_err(|_| NotificationError::InvalidTimestamp)?)
        .ok_or(NotificationError::InvalidTimestamp)?;
    Ok(parsed * 10_u32.pow(scale))
}

fn parse_fixed_decimal<T>(value: &str, length: usize) -> Result<T, NotificationError>
where
    T: FromStr,
{
    if value.len() != length || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(NotificationError::InvalidTimestamp);
    }
    value
        .parse()
        .map_err(|_| NotificationError::InvalidTimestamp)
}
