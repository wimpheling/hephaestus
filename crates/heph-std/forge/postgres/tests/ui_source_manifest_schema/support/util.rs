pub const VALID_COMMIT: &str = "a";
pub const OTHER_COMMIT: &str = "b";

pub fn commit(prefix: &str) -> String {
    format!("{prefix}{}", "0".repeat(39))
}
