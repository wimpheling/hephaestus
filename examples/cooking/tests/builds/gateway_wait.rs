// Reuse the parent facade so sibling fixture phases share one boundary context.
#[allow(unused_imports)]
use super::*;
/// Waits for an installed HTTP service gateway to become the serving revision
/// before a managed UI asks the release service to resolve its route. Gateway
/// installation publishes the desired revision first; the daemon promotes it
/// only after the service readiness probe succeeds.
pub(crate) async fn wait_for_cooking_gateway_active(
    context: &CookingBuildContext<'_>,
    gateway: InstalledCookingGateway,
) -> Result<(), BuildError> {
    let deadline = tokio::time::Instant::now() + context.timeout.min(Duration::from_secs(120));
    let mut last_observed = None;
    loop {
        let client = rpc_gateway_client(
            context.running,
            context.identity.rpc_token,
            "/hephaestus.gateway.v1.GatewayService/GetGateway",
        )?;
        let response = if let Ok(response) = tokio::time::timeout_at(
            deadline,
            client.get_gateway(GetGatewayRequest {
                gateway_id: opaque(gateway.gateway_id).into(),
                ..Default::default()
            }),
        )
        .await
        {
            response?
        } else {
            return Err(cooking_gateway_readiness_timeout(
                context,
                gateway,
                last_observed.as_deref(),
            )
            .await);
        };
        let summary = response.into_owned().gateway.into_option().ok_or_else(|| {
            invalid_state("GetGateway returned no gateway while waiting for readiness")
        })?;
        let active_revision_id = summary.active_revision_id.into_option();
        last_observed = Some(format!(
            "lifecycle={} active_revision={} desired_revision={}",
            summary.lifecycle.to_i32(),
            active_revision_id
                .as_ref()
                .map_or("none", |id| id.value.as_str()),
            summary
                .desired_service_revision_id
                .as_option()
                .map_or("none", |id| id.value.as_str()),
        ));
        if summary.lifecycle.to_i32() == GatewayLifecycle::Enabled as i32
            && active_revision_id.is_some_and(|id| id.value == gateway.revision_id.to_string())
        {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(cooking_gateway_readiness_timeout(
                context,
                gateway,
                last_observed.as_deref(),
            )
            .await);
        }
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        sleep(Duration::from_millis(250).min(remaining)).await;
    }
}

pub(crate) async fn cooking_gateway_readiness_timeout(
    context: &CookingBuildContext<'_>,
    gateway: InstalledCookingGateway,
    last_observed: Option<&str>,
) -> BuildError {
    let diagnostic = tokio::time::timeout(
        Duration::from_secs(5),
        cooking_gateway_readiness_diagnostic(context, gateway, last_observed),
    )
    .await
    .unwrap_or_else(|_| String::from("readiness_diagnostic=timeout"));
    invalid_state(&format!(
        "cooking gateway readiness polling deadline elapsed; {diagnostic}"
    ))
}

/// Reads only the authorized, bounded service-log page after readiness stalls.
/// The database lookup supplies the exact fencing scope; output is reduced to
/// a fixed diagnostic category so guest payloads never enter the host error.
pub(crate) async fn cooking_gateway_readiness_diagnostic(
    context: &CookingBuildContext<'_>,
    gateway: InstalledCookingGateway,
    last_observed: Option<&str>,
) -> String {
    let instance: Option<CookingGatewayServiceInstance> = match sqlx::query_as(
        "SELECT id, fencing_token, state, failure_code, exit_code, exit_signal
           FROM gateway_service_instances
          WHERE gateway_id = $1 AND revision_id = $2
          ORDER BY created_at DESC
          LIMIT 1",
    )
    .bind(gateway.gateway_id)
    .bind(gateway.revision_id)
    .fetch_optional(context.pool)
    .await
    {
        Ok(instance) => instance,
        Err(_) => {
            return format!(
                "last_gateway_status={}; service_instance_diagnostic=unavailable",
                last_observed.unwrap_or("none")
            );
        }
    };
    let Some((instance_id, fencing_token, state, failure_code, exit_code, exit_signal)) = instance
    else {
        return format!(
            "last_gateway_status={}; service_instance=none",
            last_observed.unwrap_or("none")
        );
    };
    let mut diagnostic = format!(
        "last_gateway_status={}; service_instance_state={state} failure_code={} exit_code={} exit_signal={}",
        last_observed.unwrap_or("none"),
        failure_code.as_deref().unwrap_or("none"),
        exit_code.map_or_else(|| String::from("none"), |value| value.to_string()),
        exit_signal.map_or_else(|| String::from("none"), |value| value.to_string()),
    );
    let Ok(client) = rpc_gateway_client(
        context.running,
        context.identity.rpc_token,
        "/hephaestus.gateway.v1.GatewayService/ListGatewayServiceLogs",
    ) else {
        diagnostic.push_str(" service_logs=client_unavailable");
        return diagnostic;
    };
    let response = client
        .list_gateway_service_logs(ListGatewayServiceLogsRequest {
            scope: GatewayServiceLogScope {
                project_id: opaque(context.project_id.as_uuid()).into(),
                gateway_id: opaque(gateway.gateway_id).into(),
                revision_id: opaque(gateway.revision_id).into(),
                instance_id: opaque(instance_id).into(),
                fencing_token: u64::try_from(fencing_token).unwrap_or_default(),
                ..Default::default()
            }
            .into(),
            limit: 16,
            after: None::<Cursor>.into(),
            ..Default::default()
        })
        .await;
    let Ok(response) = response else {
        diagnostic.push_str(" service_logs=read_unavailable");
        return diagnostic;
    };
    let response = response.into_owned();
    let mut stderr_category = "no_output";
    for record in response.records.iter().take(16) {
        if record.stream == GatewayServiceLogStream::Stderr {
            stderr_category = classify_service_stderr(&record.contents);
            if stderr_category != "other_output" {
                break;
            }
        }
    }
    diagnostic.push_str(" service_log_stderr_category=");
    diagnostic.push_str(stderr_category);
    diagnostic
}

pub(crate) fn classify_service_stderr(contents: &[u8]) -> &'static str {
    let text = String::from_utf8_lossy(contents).to_ascii_lowercase();
    if text.contains("no such file") && text.contains("python") {
        "python_command_missing"
    } else if text.contains("permission denied") {
        "permission_denied"
    } else if text.contains("traceback") {
        "python_traceback"
    } else if text.contains("killed") || text.contains("sigkill") {
        "process_killed"
    } else {
        "other_output"
    }
}
