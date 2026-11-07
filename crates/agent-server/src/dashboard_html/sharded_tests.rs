use super::ShardedResilientDashboardService;

#[test]
fn with_default_shards_creates_sixteen_dashboard_shards() {
    let service = ShardedResilientDashboardService::with_default_shards();

    assert_eq!(service.num_shards(), 16);
}

#[test]
fn shard_idx_is_stable_and_always_within_the_shard_count() {
    let service = ShardedResilientDashboardService::new(4);

    for key in ["", "dashboard", "another dashboard"] {
        let idx = service.shard_idx(key);
        assert!(
            idx < service.num_shards(),
            "index {idx} out of range for {key:?}"
        );
        assert_eq!(service.shard_idx(key), idx, "index changed for {key:?}");
    }

    let single_shard_service = ShardedResilientDashboardService::new(0);
    assert_eq!(single_shard_service.num_shards(), 1);
    assert_eq!(single_shard_service.shard_idx(""), 0);
}

#[test]
fn shard_wraps_indices_and_selects_the_requested_shard() {
    let service = ShardedResilientDashboardService::new(3);

    assert!(std::ptr::eq(service.shard(0), service.shard(3)));
    assert!(std::ptr::eq(
        service.shard(usize::MAX % service.num_shards()),
        service.shard(usize::MAX)
    ));
}

#[test]
fn shard_for_selects_the_shard_returned_by_shard_idx() {
    let service = ShardedResilientDashboardService::new(5);
    let key = "customer-dashboard";
    let idx = service.shard_idx(key);

    assert!(std::ptr::eq(service.shard_for(key), service.shard(idx)));
    assert!(std::ptr::eq(service.shard_for(key), service.shard_for(key)));
}
