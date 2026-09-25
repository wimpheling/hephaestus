use super::{ClaimResolutionCompletion, CleanupCompletion, Future, JobCompletion, Pin, Poll};

pub(super) async fn poll_next_job(
    jobs: &mut Vec<Pin<Box<dyn Future<Output = JobCompletion> + Send>>>,
) -> Option<JobCompletion> {
    std::future::poll_fn(|context| {
        for index in (0..jobs.len()).rev() {
            if let Poll::Ready(completion) = jobs[index].as_mut().poll(context) {
                drop(jobs.swap_remove(index));
                return Poll::Ready(Some(completion));
            }
        }
        if jobs.is_empty() {
            Poll::Ready(None)
        } else {
            Poll::Pending
        }
    })
    .await
}

pub(super) async fn poll_next_cleanup(
    jobs: &mut Vec<Pin<Box<dyn Future<Output = CleanupCompletion> + Send>>>,
) -> Option<CleanupCompletion> {
    std::future::poll_fn(|context| {
        for index in (0..jobs.len()).rev() {
            if let Poll::Ready(completion) = jobs[index].as_mut().poll(context) {
                drop(jobs.swap_remove(index));
                return Poll::Ready(Some(completion));
            }
        }
        if jobs.is_empty() {
            Poll::Ready(None)
        } else {
            Poll::Pending
        }
    })
    .await
}

pub(super) async fn poll_next_claim_resolution(
    jobs: &mut Vec<Pin<Box<dyn Future<Output = ClaimResolutionCompletion> + Send>>>,
) -> Option<ClaimResolutionCompletion> {
    std::future::poll_fn(|context| {
        for index in (0..jobs.len()).rev() {
            if let Poll::Ready(completion) = jobs[index].as_mut().poll(context) {
                drop(jobs.swap_remove(index));
                return Poll::Ready(Some(completion));
            }
        }
        if jobs.is_empty() {
            Poll::Ready(None)
        } else {
            Poll::Pending
        }
    })
    .await
}
