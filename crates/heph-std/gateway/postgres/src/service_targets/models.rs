//! Database row models and conversion for gateway service targets.

use gateway_domain::{
    GatewayEdgeError, GatewayServiceConfig, GatewayServiceOwnedTarget,
    GatewayServiceRevisionTarget, GatewayServiceTarget, ServiceLogCaptureMode, ServiceProbePath,
};
use sqlx::FromRow;
use std::convert::TryFrom;
use uuid::Uuid;

#[derive(Debug, FromRow)]
pub(super) struct ServiceTargetRow {
    pub(super) gateway_id: Uuid,
    pub(super) lifecycle: String,
    pub(super) active_revision_id: Option<Uuid>,
    pub(super) desired_service_revision_id: Option<Uuid>,
    pub(super) active_service_revision_id: Option<Uuid>,
    pub(super) active_release_id: Option<Uuid>,
    pub(super) active_release_state: Option<String>,
    pub(super) active_service_loopback_port: Option<i32>,
    pub(super) active_service_readiness_path: Option<String>,
    pub(super) active_service_health_path: Option<String>,
    pub(super) active_service_log_capture_mode: Option<String>,
    pub(super) desired_service_revision_row_id: Option<Uuid>,
    pub(super) desired_release_id: Option<Uuid>,
    pub(super) desired_release_state: Option<String>,
    pub(super) desired_service_loopback_port: Option<i32>,
    pub(super) desired_service_readiness_path: Option<String>,
    pub(super) desired_service_health_path: Option<String>,
    pub(super) desired_service_log_capture_mode: Option<String>,
}

impl ServiceTargetRow {
    pub(super) fn try_into_target(self) -> Result<GatewayServiceTarget, GatewayEdgeError> {
        Ok(GatewayServiceTarget {
            gateway_id: self.gateway_id,
            lifecycle: self.lifecycle,
            active_revision_id: self.active_revision_id,
            desired_service_revision_id: self.desired_service_revision_id,
            active_service_revision: service_revision(
                self.active_service_revision_id,
                self.active_release_id,
                self.active_release_state,
                self.active_service_loopback_port,
                self.active_service_readiness_path,
                self.active_service_health_path,
                self.active_service_log_capture_mode.as_deref(),
            )?,
            desired_service_revision: service_revision(
                self.desired_service_revision_row_id,
                self.desired_release_id,
                self.desired_release_state,
                self.desired_service_loopback_port,
                self.desired_service_readiness_path,
                self.desired_service_health_path,
                self.desired_service_log_capture_mode.as_deref(),
            )?,
        })
    }
}

#[derive(Debug, FromRow)]
pub(super) struct OwnedServiceTargetRow {
    pub(super) gateway_id: Uuid,
    pub(super) lifecycle: String,
    pub(super) active_revision_id: Option<Uuid>,
    pub(super) desired_service_revision_id: Option<Uuid>,
    pub(super) revision_id: Uuid,
    pub(super) release_id: Option<Uuid>,
    pub(super) release_state: Option<String>,
    pub(super) service_loopback_port: Option<i32>,
    pub(super) service_readiness_path: Option<String>,
    pub(super) service_health_path: Option<String>,
    pub(super) service_log_capture_mode: String,
}

impl OwnedServiceTargetRow {
    pub(super) fn try_into_target(self) -> Result<GatewayServiceOwnedTarget, GatewayEdgeError> {
        let revision = service_revision(
            Some(self.revision_id),
            self.release_id,
            self.release_state,
            self.service_loopback_port,
            self.service_readiness_path,
            self.service_health_path,
            Some(self.service_log_capture_mode.as_str()),
        )?
        .ok_or(GatewayEdgeError::Unavailable)?;
        Ok(GatewayServiceOwnedTarget {
            gateway_id: self.gateway_id,
            lifecycle: self.lifecycle,
            active_revision_id: self.active_revision_id,
            desired_service_revision_id: self.desired_service_revision_id,
            revision,
        })
    }
}

pub(super) fn service_revision(
    revision_id: Option<Uuid>,
    release_id: Option<Uuid>,
    release_state: Option<String>,
    loopback_port: Option<i32>,
    readiness_path: Option<String>,
    health_path: Option<String>,
    log_capture_mode: Option<&str>,
) -> Result<Option<GatewayServiceRevisionTarget>, GatewayEdgeError> {
    let Some(revision_id) = revision_id else {
        return Ok(None);
    };
    let loopback_port = loopback_port
        .and_then(|port| u16::try_from(port).ok())
        .ok_or(GatewayEdgeError::Unavailable)?;
    let readiness_path =
        ServiceProbePath::parse(readiness_path.ok_or(GatewayEdgeError::Unavailable)?)
            .map_err(|_| GatewayEdgeError::Unavailable)?;
    let health_path = ServiceProbePath::parse(health_path.ok_or(GatewayEdgeError::Unavailable)?)
        .map_err(|_| GatewayEdgeError::Unavailable)?;
    let log_capture_mode =
        ServiceLogCaptureMode::from_name(log_capture_mode.ok_or(GatewayEdgeError::Unavailable)?)
            .ok_or(GatewayEdgeError::Unavailable)?;
    let service = GatewayServiceConfig::new(loopback_port, readiness_path, health_path)
        .map_err(|_| GatewayEdgeError::Unavailable)?
        .with_log_capture_mode(log_capture_mode);
    let publication_eligible = release_state.as_deref() == Some("published");
    Ok(Some(GatewayServiceRevisionTarget {
        revision_id,
        release_id,
        release_state,
        publication_eligible,
        service,
    }))
}
