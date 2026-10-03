use super::{RuntimeGitBridge, VmInstance, VmProvider};
use uuid::Uuid;

const fn assert_runtime_trait<T: ?Sized + Send + Sync + 'static>() {}

#[test]
const fn provider_traits_are_object_safe() {
    assert_runtime_trait::<dyn VmProvider>();
    assert_runtime_trait::<dyn VmInstance>();
}

#[test]
fn runtime_git_remote_url_contains_only_route_metadata() {
    let bridge = RuntimeGitBridge::new(Uuid::nil(), 19_100);
    assert_eq!(
        bridge.remote_url(),
        "http://127.0.0.1:19100/00000000-0000-0000-0000-000000000000"
    );
}

struct UnscopedProvider;

#[async_trait::async_trait]
impl VmProvider for UnscopedProvider {
    fn name(&self) -> &'static str {
        "unscoped-test"
    }
    async fn provision(
        &self,
        _spec: crate::VmSpec,
    ) -> Result<std::sync::Arc<dyn VmInstance>, crate::VmError> {
        Err(crate::VmError::Unsupported {
            feature: "test provisioning".into(),
            provider: self.name().into(),
        })
    }
    async fn cleanup_orphan(&self, _id: &crate::VmId) -> Result<(), crate::VmError> {
        Ok(())
    }
}

#[tokio::test]
async fn unsupported_scope_never_delegates_to_unscoped_absence() {
    let provider = UnscopedProvider;
    let scope = crate::VmProviderOwnerScope::new("owner".into(), "host".into()).unwrap();
    assert!(matches!(
        provider.owner_scope(),
        Err(crate::VmError::Unsupported { .. })
    ));
    assert!(matches!(
        provider
            .cleanup_orphan_scoped(&scope, &crate::VmId("absent".into()))
            .await,
        Err(crate::VmError::Unsupported { .. })
    ));
}
