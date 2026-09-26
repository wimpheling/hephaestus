use super::{ProjectRpc, map_forge_error, opaque};
use crate::rpc::{into_connect_error, request};
use connectrpc::{ConnectError, RequestContext, Response, ServiceRequest, ServiceResult};
use forge_domain::OrganizationId;
use rpc_proto::messages::hephaestus::project::v1::{CreateProjectRequest, CreateProjectResponse};
use std::future::Future;

pub(super) async fn handle(
    service: &ProjectRpc,
    ctx: RequestContext,
    message: ServiceRequest<'_, CreateProjectRequest>,
) -> ServiceResult<CreateProjectResponse> {
    let budget = request::RequestBudget::from_transport(&ctx);
    let request = message.to_owned_message();
    let identity = request::mutation_identity(
        &ctx,
        &service.authenticator,
        "/hephaestus.project.v1.ProjectService/CreateProject",
        request.context.as_option(),
    )
    .map_err(into_connect_error)?;
    let organization_id = request
        .organization_id
        .as_option()
        .ok_or_else(|| into_connect_error(crate::rpc::RpcError::InvalidArgument))?
        .value
        .parse::<OrganizationId>()
        .map_err(|_| into_connect_error(crate::rpc::RpcError::InvalidArgument))?;
    let (project, receipt) = run_creation(
        &budget,
        service.forge.create_project_with_description(
            &identity,
            organization_id,
            &request.name,
            &request.description,
        ),
        |error| into_connect_error(map_forge_error(&error)),
        crate::rpc::mutation_receipt(
            &service.receipts,
            identity.idempotency_id,
            identity.user_id,
            "project",
            "project",
        ),
    )
    .await?;
    Response::ok(CreateProjectResponse {
        project_id: opaque(project.id.as_uuid()).into(),
        receipt: receipt.into(),
        ..Default::default()
    })
}

async fn run_creation<Create, CreateError, ReceiptFuture, Project, Receipt>(
    budget: &request::RequestBudget,
    create: Create,
    map_create_error: impl FnOnce(CreateError) -> connectrpc::ConnectError,
    receipt: ReceiptFuture,
) -> Result<(Project, Receipt), ConnectError>
where
    Create: Future<Output = Result<Project, CreateError>>,
    ReceiptFuture: Future<Output = Result<Receipt, ConnectError>>,
{
    let project = request::run_with_budget(budget, create)
        .await
        .map_err(into_connect_error)?
        .map_err(map_create_error)?;
    let receipt = request::run_with_budget(budget, receipt)
        .await
        .map_err(into_connect_error)??;
    Ok((project, receipt))
}

#[cfg(test)]
mod tests {
    use super::run_creation;
    use crate::rpc::{RpcError, into_connect_error, request};
    use connectrpc::{ErrorCode, RequestContext as TransportContext};
    use http::HeaderMap;
    use std::{
        future::Future,
        pin::Pin,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        task::{Context, Poll},
        time::{Duration, Instant},
    };

    struct BlockingCreate {
        dropped: Arc<AtomicBool>,
    }

    impl Future for BlockingCreate {
        type Output = Result<(), RpcError>;

        fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
            Poll::Pending
        }
    }

    impl Drop for BlockingCreate {
        fn drop(&mut self) {
            self.dropped.store(true, Ordering::SeqCst);
        }
    }

    struct ReceiptProbe {
        polled: Arc<AtomicBool>,
    }

    impl Future for ReceiptProbe {
        type Output = Result<(), connectrpc::ConnectError>;

        fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
            self.polled.store(true, Ordering::SeqCst);
            Poll::Ready(Ok(()))
        }
    }

    fn expired_transport() -> TransportContext {
        TransportContext::new(HeaderMap::new()).with_deadline(Some(
            Instant::now()
                .checked_sub(Duration::from_millis(1))
                .expect("instant supports a one millisecond subtraction"),
        ))
    }

    #[tokio::test]
    async fn expired_transport_deadline_drops_create_and_skips_receipt() {
        let transport = expired_transport();
        let budget = request::RequestBudget::from_transport(&transport);
        let create_dropped = Arc::new(AtomicBool::new(false));
        let receipt_polled = Arc::new(AtomicBool::new(false));
        let result = run_creation(
            &budget,
            BlockingCreate {
                dropped: Arc::clone(&create_dropped),
            },
            into_connect_error,
            ReceiptProbe {
                polled: Arc::clone(&receipt_polled),
            },
        )
        .await;

        assert_eq!(
            result.expect_err("expired request must fail").code,
            ErrorCode::DeadlineExceeded
        );
        assert!(create_dropped.load(Ordering::SeqCst));
        assert!(!receipt_polled.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn dropped_request_drops_create_and_skips_receipt() {
        let create_dropped = Arc::new(AtomicBool::new(false));
        let receipt_polled = Arc::new(AtomicBool::new(false));
        let task = tokio::spawn({
            let create_dropped = Arc::clone(&create_dropped);
            let receipt_polled = Arc::clone(&receipt_polled);
            async move {
                let transport = TransportContext::new(HeaderMap::new());
                let budget = request::RequestBudget::from_transport(&transport);
                let _ = run_creation(
                    &budget,
                    BlockingCreate {
                        dropped: create_dropped,
                    },
                    into_connect_error,
                    ReceiptProbe {
                        polled: receipt_polled,
                    },
                )
                .await;
            }
        });
        tokio::task::yield_now().await;
        task.abort();
        assert!(task.await.is_err());
        assert!(create_dropped.load(Ordering::SeqCst));
        assert!(!receipt_polled.load(Ordering::SeqCst));
    }
}
