use super::rows::RuntimeSessionRow;

pub fn average_revocation_latency(rows: &[RuntimeSessionRow]) -> u64 {
    let latencies = rows
        .iter()
        .filter_map(|row| row.revoked_at.map(|revoked| revoked - row.issued_at))
        .filter_map(|duration| u64::try_from(duration.whole_milliseconds()).ok())
        .collect::<Vec<_>>();
    if latencies.is_empty() {
        return 0;
    }
    latencies.iter().sum::<u64>() / u64::try_from(latencies.len()).unwrap_or(1)
}
