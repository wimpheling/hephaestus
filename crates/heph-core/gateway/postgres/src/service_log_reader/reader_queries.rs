use super::{
    CandidateRow, GatewayServiceLogReadCursor, GatewayServiceLogReadMetadata,
    GatewayServiceLogReadRecord, GatewayServiceLogReadScope, GatewayServiceLogReaderError,
    LogMetadataRow, LogStream, MAX_SERVICE_LOG_CHUNK_BYTES, MAX_SERVICE_LOG_READ_PAGE_BYTES,
    PayloadRow, Postgres, Transaction,
};

pub(super) async fn fetch_metadata(
    transaction: &mut Transaction<'_, Postgres>,
    scope: GatewayServiceLogReadScope,
) -> Result<Option<LogMetadataRow>, GatewayServiceLogReaderError> {
    sqlx::query_as::<_, LogMetadataRow>(
        "SELECT instance.fencing_token AS current_fencing_token,
                epoch.fencing_token AS epoch_fencing_token,
                epoch.acknowledged_through,
                epoch.retained_bytes,
                epoch.retained_chunks,
                epoch.producer_dropped_chunks,
                epoch.producer_dropped_bytes,
                epoch.provider_lagged_events,
                epoch.storage_dropped_chunks,
                epoch.storage_dropped_bytes,
                epoch.evicted_chunks,
                epoch.evicted_bytes,
                min(chunk.sequence) AS earliest_retained_sequence
           FROM gateway_service_instances AS instance
           JOIN gateways AS gateway
             ON gateway.id = instance.gateway_id
           LEFT JOIN gateway_service_log_epochs AS epoch
             ON epoch.instance_id = instance.id
            AND epoch.gateway_id = instance.gateway_id
            AND epoch.revision_id = instance.revision_id
            AND epoch.project_id = gateway.project_id
            AND epoch.fencing_token = $5
           LEFT JOIN gateway_service_log_chunks AS chunk
             ON chunk.instance_id = epoch.instance_id
            AND chunk.gateway_id = epoch.gateway_id
            AND chunk.revision_id = epoch.revision_id
            AND chunk.project_id = epoch.project_id
            AND chunk.fencing_token = epoch.fencing_token
          WHERE instance.id = $1
            AND instance.gateway_id = $2
            AND instance.revision_id = $3
            AND gateway.project_id = $4
          GROUP BY instance.fencing_token,
                   epoch.fencing_token,
                   epoch.acknowledged_through,
                   epoch.retained_bytes,
                   epoch.retained_chunks,
                   epoch.producer_dropped_chunks,
                   epoch.producer_dropped_bytes,
                   epoch.provider_lagged_events,
                   epoch.storage_dropped_chunks,
                   epoch.storage_dropped_bytes,
                   epoch.evicted_chunks,
                   epoch.evicted_bytes",
    )
    .bind(scope.instance_id)
    .bind(scope.gateway_id)
    .bind(scope.revision_id)
    .bind(scope.project_id)
    .bind(scope.fencing_token)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|_| GatewayServiceLogReaderError::Unavailable)
}

pub(super) fn select_sequences(
    candidates: &[CandidateRow],
    limit: u16,
) -> Result<(Vec<i64>, bool), GatewayServiceLogReaderError> {
    let mut selected = Vec::with_capacity(usize::from(limit));
    let mut total_bytes = 0_usize;
    for candidate in candidates {
        let byte_len = usize::try_from(candidate.byte_len)
            .map_err(|_| GatewayServiceLogReaderError::InvalidArgument)?;
        if candidate.sequence < 0 || byte_len > MAX_SERVICE_LOG_CHUNK_BYTES {
            return Err(GatewayServiceLogReaderError::InvalidArgument);
        }
        if selected.len() >= usize::from(limit)
            || total_bytes.saturating_add(byte_len) > MAX_SERVICE_LOG_READ_PAGE_BYTES
        {
            break;
        }
        total_bytes = total_bytes.saturating_add(byte_len);
        selected.push(candidate.sequence);
    }
    let has_more = selected.len() < candidates.len();
    Ok((selected, has_more))
}

pub(super) fn payload_records(
    rows: Vec<PayloadRow>,
    selected_sequences: &[i64],
) -> Result<Vec<GatewayServiceLogReadRecord>, GatewayServiceLogReaderError> {
    if rows.len() != selected_sequences.len() {
        return Err(GatewayServiceLogReaderError::Unavailable);
    }
    let mut total_bytes = 0_usize;
    let mut records = Vec::with_capacity(rows.len());
    for (row, expected_sequence) in rows.into_iter().zip(selected_sequences) {
        if row.sequence != *expected_sequence || row.bytes.len() > MAX_SERVICE_LOG_CHUNK_BYTES {
            return Err(GatewayServiceLogReaderError::InvalidArgument);
        }
        total_bytes = total_bytes.saturating_add(row.bytes.len());
        if total_bytes > MAX_SERVICE_LOG_READ_PAGE_BYTES {
            return Err(GatewayServiceLogReaderError::InvalidArgument);
        }
        records.push(GatewayServiceLogReadRecord {
            sequence: u64::try_from(row.sequence)
                .map_err(|_| GatewayServiceLogReaderError::InvalidArgument)?,
            stream: parse_stream(&row.stream)?,
            observed_at: row.observed_at,
            stored_at: row.stored_at,
            bytes: row.bytes,
        });
    }
    Ok(records)
}

pub(super) fn parse_stream(stream: &str) -> Result<LogStream, GatewayServiceLogReaderError> {
    match stream {
        "stdout" => Ok(LogStream::Stdout),
        "stderr" => Ok(LogStream::Stderr),
        _ => Err(GatewayServiceLogReaderError::InvalidArgument),
    }
}

pub(super) fn history_incomplete(
    metadata: &GatewayServiceLogReadMetadata,
    after: Option<GatewayServiceLogReadCursor>,
) -> bool {
    let expected = after.map_or(0, |cursor| cursor.sequence().saturating_add(1));
    metadata.earliest_retained_sequence.map_or_else(
        || {
            metadata
                .acknowledged_through
                .is_some_and(|acknowledged| expected <= acknowledged)
        },
        |earliest| expected < earliest,
    )
}
