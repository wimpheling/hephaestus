//! Memory-only original attempts, not physical backend proof.
use crate::{FakeProvider, tests::spec};
use std::sync::Arc;
use vm_trait::{VmError, VmProvider};

#[tokio::test]
async fn same_original_cleanup_cannot_touch_live_conflict_or_successor() {
    let provider = FakeProvider::new();
    let first = provider.prepare_provision_attempt(spec("same")).unwrap();
    let original = first.provision().await.unwrap();
    assert!(Arc::ptr_eq(&original, &first.provision().await.unwrap()));
    let rejected = provider.prepare_provision_attempt(spec("same")).unwrap();
    assert!(rejected.provision().await.is_err());
    rejected.cleanup().await.unwrap();
    original.start().await.unwrap();
    // External direct original destroy precedes first token cleanup. That
    // cleanup must not remove the reservation of a same-ID successor either.
    original.destroy().await.unwrap();
    let next = provider.prepare_provision_attempt(spec("same")).unwrap();
    let successor = next.provision().await.unwrap();
    first.cleanup().await.unwrap();
    first.cleanup().await.unwrap();
    successor.start().await.unwrap();
    assert!(matches!(
        provider.provision(spec("same")).await,
        Err(VmError::AlreadyExists(_))
    ));
    next.cleanup().await.unwrap();
    drop(successor);
    drop(original);
}
#[tokio::test]
async fn unused_cleanup_monotonically_closes_without_allocation() {
    let provider = FakeProvider::new();
    let token = provider.prepare_provision_attempt(spec("closed")).unwrap();
    token.cleanup().await.unwrap();
    assert!(matches!(token.provision().await, Err(VmError::Destroyed)));
    let other = provider.provision(spec("closed")).await.unwrap();
    token.cleanup().await.unwrap();
    other.start().await.unwrap();
    other.destroy().await.unwrap();
}
