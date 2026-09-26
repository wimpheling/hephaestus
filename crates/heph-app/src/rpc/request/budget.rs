use crate::rpc::RpcError;
use connectrpc::RequestContext as TransportContext;
use std::{future::Future, time::Instant};
use tokio_util::sync::CancellationToken;

/// The bounded lifetime shared by one RPC handler and its downstream calls.
///
/// The token is canceled when the handler future drops this owner. Downstream
/// adapters can clone the token and select it alongside their I/O future while
/// [`run_with_budget`] enforces the same absolute transport deadline.
pub struct RequestBudget {
    pub(super) deadline: Option<Instant>,
    cancellation: CancellationToken,
}

impl RequestBudget {
    pub fn from_transport(transport: &TransportContext) -> Self {
        Self::from_deadline(transport.deadline())
    }

    pub fn from_deadline(deadline: Option<Instant>) -> Self {
        Self {
            deadline,
            cancellation: CancellationToken::new(),
        }
    }

    #[cfg(test)]
    pub fn unbounded() -> Self {
        Self::from_deadline(None)
    }

    pub fn cancellation_token(&self) -> CancellationToken {
        self.cancellation.clone()
    }

    pub fn cancel(&self) {
        self.cancellation.cancel();
    }

    #[cfg(test)]
    pub const fn deadline(&self) -> Option<Instant> {
        self.deadline
    }
}

impl Drop for RequestBudget {
    fn drop(&mut self) {
        self.cancellation.cancel();
    }
}

/// Runs one downstream operation until the shared deadline or request
/// cancellation. The operation is dropped when either boundary wins.
pub async fn run_with_budget<T, Operation>(
    budget: &RequestBudget,
    operation: Operation,
) -> Result<T, RpcError>
where
    Operation: Future<Output = T>,
{
    let cancellation = budget.cancellation_token();
    match budget.deadline {
        Some(deadline) => {
            tokio::select! {
                result = tokio::time::timeout_at(deadline.into(), operation) => {
                    result.map_err(|_| RpcError::DeadlineExceeded)
                }
                () = cancellation.cancelled() => Err(RpcError::Canceled),
            }
        }
        None => {
            tokio::select! {
                result = operation => Ok(result),
                () = cancellation.cancelled() => Err(RpcError::Canceled),
            }
        }
    }
}

#[cfg(test)]
pub fn test_budget(deadline: Option<Instant>) -> RequestBudget {
    RequestBudget {
        deadline,
        cancellation: CancellationToken::new(),
    }
}
