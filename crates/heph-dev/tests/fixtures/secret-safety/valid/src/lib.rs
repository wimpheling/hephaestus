pub fn redacted() -> &'static str {
    "opaque-id"
}

#[cfg(test)]
mod tests {
    const SENTINEL: &str = "test-only-secret-sentinel";
}
