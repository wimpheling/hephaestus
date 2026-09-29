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
