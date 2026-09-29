use super::*;

pub type FailedGatewayServiceRow = (
    uuid::Uuid,
    i64,
    String,
    Option<String>,
    Option<OffsetDateTime>,
    Option<i32>,
    Option<i32>,
    Option<uuid::Uuid>,
    Option<i32>,
    Option<OffsetDateTime>,
);

#[derive(Debug)]
pub struct FailedGatewayServiceEvidence {
    pub gateway_id: uuid::Uuid,
    pub revision_id: uuid::Uuid,
    pub instance_id: uuid::Uuid,
    pub initial_fencing_token: i64,
    pub fencing_token: i64,
    pub state: String,
    pub failure_code: Option<String>,
    pub failed_at: Option<OffsetDateTime>,
    pub exit_code: Option<i32>,
    pub exit_signal: Option<i32>,
    pub observed_ready: bool,
    pub observed_promoted: bool,
    pub retry: Option<(i32, Option<OffsetDateTime>)>,
}

pub async fn wait_for_failed_gateway_service_candidate(
    pool: &sqlx::PgPool,
    gateway_id: uuid::Uuid,
    revision_id: uuid::Uuid,
    old_identity: (uuid::Uuid, uuid::Uuid, i64),
    old_proof: &GatewayServiceRequestProof,
    public_url: &str,
) -> FailedGatewayServiceEvidence {
    let (old_instance_id, old_revision_id, old_fencing_token) = old_identity;
    tokio::time::timeout(Duration::from_secs(120), async {
        // The ownership schema has no historical ready_at column. Poll the
        // complete launch lifetime and record every observed Ready state;
        // the durable gateway event count in the caller proves that no active
        // pointer transition occurred after B was declared.
        let mut initial_fencing_token = None;
        let mut candidate_id = None;
        let mut observed_ready = false;
        let mut observed_promoted = false;
        let mut served_during_failure = false;
        loop {
            let row: Option<FailedGatewayServiceRow> = sqlx::query_as(
                "SELECT instance.id, instance.fencing_token, instance.state,
                        instance.failure_code, instance.failed_at,
                        instance.exit_code, instance.exit_signal,
                        gateway.active_revision_id,
                        retry.failure_streak, retry.next_retry_at
                   FROM gateway_service_instances AS instance
                   JOIN gateways AS gateway ON gateway.id = instance.gateway_id
                   LEFT JOIN gateway_service_retry_state AS retry
                     ON retry.gateway_id = instance.gateway_id
                    AND retry.revision_id = instance.revision_id
                  WHERE instance.gateway_id = $1
                    AND instance.revision_id = $2
                    AND (($3::uuid IS NULL AND instance.id <> $4)
                         OR instance.id = $3)
                  ORDER BY instance.created_at ASC, instance.id ASC
                  LIMIT 1",
            )
            .bind(gateway_id)
            .bind(revision_id)
            .bind(candidate_id)
            .bind(old_instance_id)
            .fetch_optional(pool)
            .await
            .expect("read failed candidate lifecycle evidence");
            if let Some((
                observed_id,
                fencing_token,
                state,
                failure_code,
                failed_at,
                exit_code,
                exit_signal,
                active_revision,
                failure_streak,
                next_retry_at,
            )) = row
            {
                if let Some(expected_id) = candidate_id {
                    assert_eq!(
                        observed_id, expected_id,
                        "failed-candidate evidence must stay bound to its first instance"
                    );
                } else {
                    candidate_id = Some(observed_id);
                    initial_fencing_token = Some(fencing_token);
                }
                observed_ready |= state == "ready";
                observed_promoted |= active_revision == Some(revision_id);
                if failure_code.is_some() && !served_during_failure {
                    let proof = exercise_gateway_service_requests(public_url).await;
                    assert_eq!(proof.startup_id, old_proof.startup_id);
                    let current_old_fence = read_gateway_service_fencing_token(
                        pool,
                        old_instance_id,
                        gateway_id,
                        old_revision_id,
                    )
                    .await;
                    assert_eq!(current_old_fence, old_fencing_token);
                    served_during_failure = true;
                }
                if let (true, Some(failure_streak)) =
                    (state == "cleaned" && failure_code.is_some(), failure_streak)
                {
                    return FailedGatewayServiceEvidence {
                        gateway_id,
                        revision_id,
                        instance_id: candidate_id.expect("capture B candidate identity"),
                        initial_fencing_token: initial_fencing_token
                            .expect("capture B initial fencing token"),
                        fencing_token,
                        state,
                        failure_code,
                        failed_at,
                        exit_code,
                        exit_signal,
                        observed_ready,
                        observed_promoted,
                        retry: Some((failure_streak, next_retry_at)),
                    };
                }
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("failed service candidate reaches durable cleanup")
}
