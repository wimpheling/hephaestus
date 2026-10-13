//! Sealed `SQLx` journal evidence from real embedded SQL and complete snapshots.
use super::{
    fixture,
    snapshots::{Phase, Row, Snapshot},
};
use sha2::{Digest, Sha384};
use sqlx::migrate::Migration;
use std::{collections::BTreeMap, fmt::Display, str::FromStr};
use time::{Date, Month, OffsetDateTime, Time};

const COLUMNS: [(&str, i16, i64, &str); 6] = [
    ("version", 1, 20, "bigint"),
    ("description", 2, 25, "text"),
    ("installed_on", 3, 1184, "timestamp with time zone"),
    ("success", 4, 16, "boolean"),
    ("checksum", 5, 17, "bytea"),
    ("execution_time", 6, 20, "bigint"),
];

/// Full observed rows admitted by `validate` or `validate_fresh`, with snapshot custody.
#[derive(Debug)]
pub struct Journal<'a> {
    phase: Phase,
    rows: BTreeMap<i64, &'a Row>,
    installed108: Option<OffsetDateTime>,
}

impl Journal<'_> {
    /// Phase whose exact embedded prefix admitted these observed rows.
    #[must_use]
    pub const fn phase(&self) -> Phase {
        self.phase
    }

    /// Actual migration108 transaction clock, absent in the Public107 phase.
    #[must_use]
    pub const fn installed108(&self) -> Option<OffsetDateTime> {
        self.installed108
    }

    /// Preserve every observed138 journal field during the additive139 upgrade.
    ///
    /// # Errors
    /// Rejects a different checkpoint or any changed original complete journal row.
    pub fn assert_public138_prefix(&self, successor: &Self) -> Result<(), sqlx::Error> {
        require(
            self.phase == Phase::Public138 && successor.phase == Phase::Public139,
            "139 journal must extend a qualified138 checkpoint",
        )?;
        require(
            self.rows
                .iter()
                .all(|(version, row)| successor.rows.get(version) == Some(row)),
            "139 changed an original138 six-field JSONB/text journal row",
        )
    }

    /// Preserve the complete observed previous checkpoint during one forward step.
    ///
    /// # Errors
    /// Rejects anything except139→140→141→142→143 or drift in any original six-field row.
    pub fn assert_forward_prefix(&self, successor: &Self) -> Result<(), sqlx::Error> {
        require(
            matches!(
                (self.phase, successor.phase),
                (Phase::Public139, Phase::Public140)
                    | (Phase::Public140, Phase::Public141)
                    | (Phase::Public141, Phase::Public142)
                    | (Phase::Public142, Phase::Public143)
            ),
            "forward journal must extend the exact preceding checkpoint",
        )?;
        require(
            self.rows
                .iter()
                .all(|(version, row)| successor.rows.get(version) == Some(row)),
            "forward migration changed an original complete journal row",
        )
    }

    /// Require exact same-phase replay, including raw JSONB text and both timings.
    ///
    /// # Errors
    /// Returns an error if the phase or any complete observed row changed.
    pub fn assert_replay(&self, replay: &Self) -> Result<(), sqlx::Error> {
        require(
            self.phase == replay.phase && self.rows == replay.rows,
            "same-phase replay changed complete SQLx journal",
        )
    }
}

/// Validate the real phase prefix; upgraded phases require a validated107 baseline.
///
/// # Errors
/// Rejects changed column identities, fields, types, migration bytes or old rows.
pub fn validate<'a>(
    snapshot: &'a Snapshot,
    before: Option<&Journal<'_>>,
) -> Result<Journal<'a>, sqlx::Error> {
    let phase = snapshot.phase();
    require(
        match (phase, before) {
            (Phase::Public107, None) => true,
            (
                Phase::Public137
                | Phase::Public138
                | Phase::Public139
                | Phase::Public140
                | Phase::Public141
                | Phase::Public142
                | Phase::Public143,
                Some(original),
            ) => original.phase == Phase::Public107,
            _ => false,
        },
        "journal requires107 qualification before upgrade",
    )?;
    let journal = validate_rows(snapshot)?;
    let rows = &journal.rows;
    if let Some(original) = before {
        require(
            original
                .rows
                .iter()
                .all(|(version, row)| rows.get(version) == Some(row)),
            "upgrade changed an original107 six-field JSONB/text row",
        )?;
    }
    Ok(journal)
}

/// Validate a genuine fresh full143 install without claiming a107 baseline.
///
/// # Errors
/// Rejects any other phase and all malformed six-field journal evidence.
pub fn validate_fresh(snapshot: &Snapshot) -> Result<Journal<'_>, sqlx::Error> {
    require(
        snapshot.phase() == Phase::Public143,
        "fresh journal requires exact Public143 phase",
    )?;
    validate_rows(snapshot)
}

fn validate_rows(snapshot: &Snapshot) -> Result<Journal<'_>, sqlx::Error> {
    let phase = snapshot.phase();
    let table = snapshot
        .tables()
        .get("_sqlx_migrations")
        .ok_or_else(|| sqlx::Error::Protocol("snapshot lacks SQLx journal".into()))?;
    require(
        table.columns.len() == COLUMNS.len()
            && table.columns.iter().zip(COLUMNS).all(
                |(column, (name, position, oid, type_name))| {
                    column.name == name
                        && column.position == position
                        && column.type_oid == oid
                        && column.type_name == type_name
                },
            ),
        "SQLx journal six column identities or types changed",
    )?;
    let head = head(phase);
    let embedded = fixture::MIGRATIONS;
    let migrations: Vec<_> = embedded.iter().filter(|m| m.version <= head).collect();
    require(
        migrations.iter().map(|m| m.version).collect::<Vec<_>>() == (1..=head).collect::<Vec<_>>(),
        "embedded SQLx prefix is not complete and contiguous",
    )?;
    require(
        table.rows.len() == migrations.len(),
        "SQLx journal has missing or extra rows",
    )?;
    let mut rows = BTreeMap::new();
    for row in &table.rows {
        let version = row.json["version"]
            .as_i64()
            .ok_or_else(|| sqlx::Error::Protocol("journal version is not bigint".into()))?;
        require(
            rows.insert(version, row).is_none(),
            "SQLx journal has duplicate versions",
        )?;
    }
    require(
        rows.keys().copied().collect::<Vec<_>>() == (1..=head).collect::<Vec<_>>(),
        "SQLx journal versions differ from exact phase prefix",
    )?;
    let mut installed108 = None;
    for migration in migrations {
        let row = rows[&migration.version];
        let installed = validate_row(row, migration)?;
        if migration.version == 108 {
            installed108 = Some(installed);
        }
    }
    Ok(Journal {
        phase,
        rows,
        installed108,
    })
}

fn validate_row(row: &Row, migration: &Migration) -> Result<OffsetDateTime, sqlx::Error> {
    let object = row
        .json
        .as_object()
        .ok_or_else(|| sqlx::Error::Protocol("journal row is not a JSON object".into()))?;
    require(
        object.len() == COLUMNS.len()
            && COLUMNS
                .iter()
                .all(|(name, _, _, _)| object.contains_key(*name)),
        "journal row must contain exactly six original fields",
    )?;
    require(
        object["version"].as_i64() == Some(migration.version)
            && object["description"].as_str() == Some(migration.description.as_ref())
            && object["success"].as_bool() == Some(true)
            && object["execution_time"].as_i64().is_some(),
        "journal migration identity, success or bigint timing is invalid",
    )?;
    let checksum = object["checksum"]
        .as_str()
        .ok_or_else(|| sqlx::Error::Protocol("journal checksum is not bytea text".into()))?;
    let digest = Sha384::digest(migration.sql.as_bytes());
    let digest_bytes: &[u8] = digest.as_ref();
    require(
        migration.checksum.as_ref() == digest_bytes && checksum_matches(checksum, digest_bytes),
        "journal checksum differs from independent SHA384 of embedded SQL",
    )?;
    let installed = object["installed_on"].as_str().ok_or_else(|| {
        sqlx::Error::Protocol("journal installed_on is not timestamp text".into())
    })?;
    timestamp(installed)
}

fn checksum_matches(actual: &str, digest: &[u8]) -> bool {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    actual.len() == 2 + digest.len() * 2
        && actual.starts_with("\\x")
        && digest.iter().enumerate().all(|(index, byte)| {
            actual.as_bytes()[2 + index * 2] == HEX[usize::from(*byte >> 4)]
                && actual.as_bytes()[3 + index * 2] == HEX[usize::from(*byte & 15)]
        })
}

fn timestamp(value: &str) -> Result<OffsetDateTime, sqlx::Error> {
    // Snapshot pins UTC. Parse its actual PostgreSQL JSON timestamp using core
    // time constructors; the workspace does not enable time's parsing feature.
    let stamp = value
        .strip_suffix("+00:00")
        .ok_or_else(|| sqlx::Error::Protocol("journal timestamp is not snapshot UTC".into()))?;
    require(
        stamp.is_ascii()
            && (19..=26).contains(&stamp.len())
            && stamp.as_bytes()[4] == b'-'
            && stamp.as_bytes()[7] == b'-'
            && stamp.as_bytes()[10] == b'T'
            && stamp.as_bytes()[13] == b':'
            && stamp.as_bytes()[16] == b':',
        "journal timestamp shape is invalid",
    )?;
    let fraction = if stamp.len() == 19 {
        0
    } else {
        require(
            stamp.as_bytes()[19] == b'.' && stamp.len() >= 21,
            "journal timestamp fraction is invalid",
        )?;
        let digits = &stamp[20..];
        require(
            digits.bytes().all(|byte| byte.is_ascii_digit()),
            "journal timestamp fraction is not numeric",
        )?;
        let precision = u32::try_from(digits.len())
            .map_err(|error| sqlx::Error::Protocol(error.to_string()))?;
        number::<u32>(digits)? * 10_u32.pow(6 - precision)
    };
    let month = Month::try_from(number::<u8>(&stamp[5..7])?)
        .map_err(|error| sqlx::Error::Protocol(error.to_string()))?;
    let year = number::<i32>(&stamp[..4])?;
    require(
        (1..=9999).contains(&year),
        "journal timestamp year is invalid",
    )?;
    let date = Date::from_calendar_date(year, month, number(&stamp[8..10])?)
        .map_err(|error| sqlx::Error::Protocol(error.to_string()))?;
    let time = Time::from_hms_micro(
        number(&stamp[11..13])?,
        number(&stamp[14..16])?,
        number(&stamp[17..19])?,
        fraction,
    )
    .map_err(|error| sqlx::Error::Protocol(error.to_string()))?;
    Ok(date.with_time(time).assume_utc())
}

fn number<T: FromStr>(value: &str) -> Result<T, sqlx::Error>
where
    T::Err: Display,
{
    require(
        value.bytes().all(|byte| byte.is_ascii_digit()),
        "journal timestamp component is not numeric",
    )?;
    value
        .parse()
        .map_err(|error: T::Err| sqlx::Error::Protocol(error.to_string()))
}

const fn head(phase: Phase) -> i64 {
    match phase {
        Phase::Public107 => 107,
        Phase::Public137 => 137,
        Phase::Public138 => 138,
        Phase::Public139 => 139,
        Phase::Public140 => 140,
        Phase::Public141 => 141,
        Phase::Public142 => 142,
        Phase::Public143 => 143,
    }
}

fn require(condition: bool, message: &str) -> Result<(), sqlx::Error> {
    if condition {
        Ok(())
    } else {
        Err(sqlx::Error::Protocol(message.into()))
    }
}
