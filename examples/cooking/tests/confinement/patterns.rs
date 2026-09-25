use base64::Engine as _;
use std::{fmt::Write as _, sync::LazyLock};

pub(crate) static CREDENTIAL_PATTERNS: LazyLock<Vec<Vec<u8>>> = LazyLock::new(|| {
    super::super::cooking::FIXTURE_CREDENTIAL_SENTINELS
        .iter()
        .flat_map(|value| {
            let mut hexadecimal = String::with_capacity(value.len() * 2);
            for byte in value.as_bytes() {
                write!(hexadecimal, "{byte:02x}").expect("write fixture encoding");
            }
            [
                value.as_bytes().to_vec(),
                base64::engine::general_purpose::STANDARD
                    .encode(value)
                    .into_bytes(),
                hexadecimal.as_bytes().to_vec(),
                hexadecimal.to_ascii_uppercase().into_bytes(),
            ]
        })
        .collect()
});

/// Returns encoded fixture fingerprints for the in-process guest probe.
///
/// The caller receives a clone so the runtime observer can discard its copy
/// after each provider specification has been checked.
pub fn credential_patterns() -> Vec<Vec<u8>> {
    CREDENTIAL_PATTERNS.clone()
}

#[track_caller]
pub(crate) fn assert_bytes_have_no_credentials(bytes: &[u8]) {
    for pattern in CREDENTIAL_PATTERNS.iter() {
        assert!(
            !bytes.windows(pattern.len()).any(|window| window == pattern),
            "fixture credential encoding exposed"
        );
    }
}

/// Decode native VM log payloads while retaining overlap only within one
/// ordered run/stream. Never include payload contents in failure diagnostics.
#[derive(Default)]
pub(crate) struct VmLogScan {
    current: Option<(uuid::Uuid, String)>,
    suffix: Vec<u8>,
    pub(crate) byte_count: usize,
}

impl VmLogScan {
    #[track_caller]
    pub(crate) fn inspect(&mut self, run: uuid::Uuid, stream: String, payload: &serde_json::Value) {
        assert!(
            matches!(stream.as_str(), "Stdout" | "Stderr"),
            "invalid VM log stream"
        );
        let values = payload.as_array().expect("VM log bytes must be an array");
        assert!(
            values.len() <= 1_048_576,
            "VM log chunk exceeds fixture bound"
        );
        let decoded: Vec<u8> = values
            .iter()
            .map(|value| {
                value
                    .as_u64()
                    .and_then(|value| u8::try_from(value).ok())
                    .expect("VM log contains an invalid byte")
            })
            .collect();
        self.inspect_bytes(run, stream, &decoded);
    }

    #[track_caller]
    pub(crate) fn inspect_bytes(&mut self, run: uuid::Uuid, stream: String, bytes: &[u8]) {
        let key = (run, stream);
        if self.current.as_ref() != Some(&key) {
            self.suffix.clear();
            self.current = Some(key);
        }
        assert!(
            bytes.len() <= 1_048_576,
            "stored log chunk exceeds fixture bound"
        );
        self.byte_count = self
            .byte_count
            .checked_add(bytes.len())
            .filter(|count| *count <= 256 * 1024 * 1024)
            .expect("stored log scan exceeds fixture bound");
        self.suffix.extend_from_slice(bytes);
        assert_bytes_have_no_credentials(&self.suffix);
        let longest = CREDENTIAL_PATTERNS
            .iter()
            .map(Vec::len)
            .max()
            .expect("fixture patterns");
        let discarded = self.suffix.len().saturating_sub(longest - 1);
        self.suffix.drain(..discarded);
    }
}
