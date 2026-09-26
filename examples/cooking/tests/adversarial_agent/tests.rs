// Reuse the adversarial facade imports across the focused phases.
#[allow(unused_imports)]
use super::*;
#[cfg(test)]
#[test]
pub(crate) fn generated_adversarial_update_ids_fit_gateway_bounds_and_remain_unique() {
    assert_eq!(bounded_update_id(u64::MAX), MAX_SIGNED_UPDATE_ID);
    assert_eq!(bounded_update_id(0), 1);
    let ids: std::collections::HashSet<_> = (0..256).map(|_| unique_update_id()).collect();
    assert_eq!(ids.len(), 256);
    assert!(ids.iter().all(|id| *id <= MAX_SIGNED_UPDATE_ID));
}
