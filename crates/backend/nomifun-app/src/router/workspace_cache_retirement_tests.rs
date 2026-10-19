use super::*;

#[test]
fn captured_workspace_cache_ticket_does_not_pin_its_exact_native_owner() {
    let directory=tempfile::tempdir().unwrap();let cache=NomiCoreWave2Host::default();
    let root=canonical_workspace_root(directory.path()).unwrap();
    let original=cache.host_for_root(root.clone()).unwrap();
    let witness=Arc::downgrade(&original);let ticket=cache.capture_workspace_host_retirement(&root).unwrap().unwrap();
    drop(original);assert_eq!(witness.strong_count(),1,"only the original cache may retain this owner");
    cache.retire_captured_workspace_host(&ticket).unwrap();
    assert!(witness.upgrade().is_none());assert_eq!(cache.cached_root_count(),0);
    cache.retire_captured_workspace_host(&ticket).unwrap();
    assert!(directory.path().exists(),"retiring transport/cache references cannot delete user files");
}

#[test]
fn captured_workspace_cache_preserves_retained_capabilities_and_has_one_weak_retry_owner() {
    let directory=tempfile::tempdir().unwrap();let cache=NomiCoreWave2Host::default();
    let root=canonical_workspace_root(directory.path()).unwrap();let invocation=cache.host_for_root(root.clone()).unwrap();
    let ticket=cache.capture_workspace_host_retirement(&root).unwrap().unwrap();
    let first=ticket.begin_retry().expect("the original owner accepts one cleanup responsibility");
    assert!(ticket.clone().begin_retry().is_none(),"shared Sessions cannot create duplicate retry loops for the same cache instance");
    assert_eq!(Arc::strong_count(&invocation),2,"a weak retry flight cannot pin its capability/native owner");
    assert!(cache.retire_captured_workspace_host(first.ticket()).is_err());assert_eq!(cache.cached_root_count(),1);
    drop(first);let replacement=ticket.begin_retry().expect("a dropped cleanup observer returns its process-local single-flight claim");
    drop(invocation);cache.retire_captured_workspace_host(replacement.ticket()).unwrap();
    assert!(ticket.owner.upgrade().is_none());assert_eq!(cache.cached_root_count(),0);
}

#[test]
fn captured_workspace_cache_ticket_never_retires_a_successor_at_the_same_root() {
    let directory=tempfile::tempdir().unwrap();let cache=NomiCoreWave2Host::default();
    let root=canonical_workspace_root(directory.path()).unwrap();
    cache.host_for_root(root.clone()).unwrap();let old=cache.capture_workspace_host_retirement(&root).unwrap().unwrap();
    cache.release_idle_workspace_host(&root).unwrap();
    let successor=cache.host_for_root(root.clone()).unwrap();let witness=Arc::downgrade(&successor);drop(successor);
    cache.retire_captured_workspace_host(&old).unwrap();
    assert_eq!(cache.cached_root_count(),1);assert_eq!(witness.strong_count(),1);
    let current=cache.capture_workspace_host_retirement(&root).unwrap().unwrap();
    cache.retire_captured_workspace_host(&current).unwrap();assert!(witness.upgrade().is_none());
}
