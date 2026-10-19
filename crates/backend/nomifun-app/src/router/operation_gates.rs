//! A weak cache of process-local operation locks, with no task or resource
//! ownership. Canonical facts remain responsible for admission and recovery.
use std::{borrow::Borrow,collections::HashMap,hash::Hash,sync::{Arc,Mutex,Weak}};
use tokio::sync::RwLock;

pub(crate) struct WeakGateCache<K,G> {
    gates:Mutex<HashMap<K,Weak<G>>>,
}

pub(crate) type OperationGateCache=WeakGateCache<String,RwLock<()>>;
pub(crate) type DeleteCleanupGateCache=WeakGateCache<(String,String),tokio::sync::Mutex<()>>;

impl<K,G> Default for WeakGateCache<K,G> {
    fn default()->Self {Self {gates:Mutex::new(HashMap::new())}}
}

impl<K:Eq+Hash,G:Default> WeakGateCache<K,G> {
    pub(crate) fn get<Q:Eq+Hash+ToOwned<Owned=K>+?Sized>(&self,key:&Q)->Arc<G> where K:Borrow<Q> {
        // Upgrade/create and dead-key removal share one short, non-async lock.
        // A waiting future or Owned guard keeps its Arc alive, so another
        // caller cannot create a second gate for an operation still in flight.
        let mut gates=self.gates.lock().expect("operation gate cache is poisoned");
        gates.retain(|_,gate|gate.strong_count()>0);
        if let Some(gate)=gates.get(key).and_then(Weak::upgrade) {return gate;}
        let gate=Arc::new(G::default());
        gates.insert(key.to_owned(),Arc::downgrade(&gate));
        gate
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operation_gate_cache_concurrent_lookups_reuse_one_live_gate() {
        let cache=Arc::new(OperationGateCache::default());
        let barrier=Arc::new(std::sync::Barrier::new(16));
        let workers=(0..16).map(|_| {
            let cache=cache.clone();let barrier=barrier.clone();
            std::thread::spawn(move|| {barrier.wait();cache.get("same-session")})
        }).collect::<Vec<_>>();
        let gates=workers.into_iter().map(|worker|worker.join().unwrap()).collect::<Vec<_>>();
        assert!(gates.iter().all(|gate|Arc::ptr_eq(gate,&gates[0])));
        assert_eq!(cache.gates.lock().unwrap().len(),1);
    }

    #[tokio::test]
    async fn operation_gate_cache_preserves_the_same_gate_for_queued_and_owned_guards() {
        let cache=Arc::new(OperationGateCache::default());
        let gate=cache.get("session");
        let identity=Arc::downgrade(&gate);
        let write=gate.write_owned().await;
        let waiting=cache.get("session");
        let (queued,entered)=tokio::sync::oneshot::channel();
        let (holding,held)=tokio::sync::oneshot::channel();
        let release=Arc::new(tokio::sync::Notify::new());
        let waiter_release=release.clone();
        let waiter=tokio::spawn(async move {
            let queued_read=waiting.read_owned();
            queued.send(()).unwrap();
            let guard=queued_read.await;
            holding.send(()).unwrap();
            waiter_release.notified().await;
            drop(guard);
        });
        entered.await.unwrap();
        drop(cache.get("trigger-prune"));
        let probe=cache.get("session");
        assert_eq!(Arc::as_ptr(&probe),identity.as_ptr());
        assert!(probe.clone().try_write_owned().is_err());
        drop(write);
        held.await.unwrap();
        drop(cache.get("another-prune"));
        assert_eq!(Arc::as_ptr(&cache.get("session")),identity.as_ptr());
        assert!(probe.clone().try_write_owned().is_err(),"waiting reader still excludes a new writer");
        drop(probe);
        release.notify_one();waiter.await.unwrap();
        assert!(identity.upgrade().is_none());
        drop(cache.get("final-prune"));
        assert!(!cache.gates.lock().unwrap().contains_key("session"));
    }

    #[test]
    fn operation_gate_cache_reuses_a_dead_key_without_pinning_the_previous_lock() {
        let cache=OperationGateCache::default();
        let original=cache.get("session");
        let old=Arc::downgrade(&original);
        drop(original);
        assert!(old.upgrade().is_none());
        let successor=cache.get("session");
        assert_ne!(Arc::as_ptr(&successor),old.as_ptr());
        assert!(old.upgrade().is_none());
        assert!(Arc::ptr_eq(&successor,&cache.get("session")));
    }

    #[test]
    fn operation_gate_cache_does_not_accumulate_historical_session_or_request_keys() {
        let cache=OperationGateCache::default();
        let live=cache.get("legitimate-live-owner");
        let mut previous=Vec::new();
        for round in 0..4 {
            for request in 0..100 {
                let gate=cache.get(&format!("native-resume-request:{round}:{request}"));
                previous.push(Arc::downgrade(&gate));
                drop(gate);
            }
            let same=cache.get("legitimate-live-owner");
            assert!(Arc::ptr_eq(&live,&same));
            assert_eq!(cache.gates.lock().unwrap().len(),1);
            assert!(previous.iter().all(|gate|gate.upgrade().is_none()));
        }
    }

    #[tokio::test]
    async fn operation_gate_cache_delete_mutex_keeps_original_tuple_and_pending_waiter() {
        let cache=Arc::new(DeleteCleanupGateCache::default());
        let key=("owner".to_owned(),"session".to_owned());
        let original=cache.get(&key);
        let old=Arc::downgrade(&original);
        let held=original.lock_owned().await;
        let waiting=cache.get(&key);
        let (queued,entered)=tokio::sync::oneshot::channel();
        let (holding,acquired)=tokio::sync::oneshot::channel();
        let release=Arc::new(tokio::sync::Notify::new());
        let waiter_release=release.clone();
        let waiter=tokio::spawn(async move {
            let pending=waiting.lock_owned();queued.send(()).unwrap();
            let guard=pending.await;holding.send(()).unwrap();
            waiter_release.notified().await;drop(guard);
        });
        entered.await.unwrap();
        drop(cache.get(&("different-owner".to_owned(),"session".to_owned())));
        let probe=cache.get(&key);
        assert_eq!(Arc::as_ptr(&probe),old.as_ptr());
        assert!(probe.clone().try_lock_owned().is_err());
        drop(held);acquired.await.unwrap();
        assert!(probe.clone().try_lock_owned().is_err());
        drop(probe);release.notify_one();waiter.await.unwrap();
        assert!(old.upgrade().is_none());
        let successor=cache.get(&key);
        assert_ne!(Arc::as_ptr(&successor),old.as_ptr());
        assert_eq!(cache.gates.lock().unwrap().len(),1);
        let other_owner=cache.get(&("different-owner".to_owned(),"session".to_owned()));
        assert!(!Arc::ptr_eq(&successor,&other_owner));
    }
}
