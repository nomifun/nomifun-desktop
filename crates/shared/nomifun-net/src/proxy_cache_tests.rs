use super::*;
use std::sync::{
    Barrier,
    atomic::{AtomicUsize, Ordering},
};

#[test]
fn concurrent_cache_misses_share_one_detection_including_absent_proxy() {
    let cache = Mutex::new(None);
    let calls = AtomicUsize::new(0);
    let ready = Barrier::new(8);
    std::thread::scope(|scope| {
        for _ in 0..8 {
            scope.spawn(|| {
                ready.wait();
                assert_eq!(
                    cached_system_proxy_config(&cache, Duration::from_secs(60), || {
                        calls.fetch_add(1, Ordering::SeqCst);
                        std::thread::sleep(Duration::from_millis(50));
                        None
                    }),
                    None
                );
            });
        }
    });
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn cache_ttl_starts_when_detection_finishes() {
    let cache = Mutex::new(None);
    let ttl = Duration::from_millis(100);
    cached_system_proxy_config(&cache, ttl, || {
        std::thread::sleep(Duration::from_millis(150));
        None
    });
    cached_system_proxy_config(&cache, ttl, || {
        panic!("fresh detection expired immediately")
    });
}

#[test]
fn zero_ttl_refreshes_and_publishes_the_latest_configuration() {
    let cache = Mutex::new(None);
    cached_system_proxy_config(&cache, Duration::ZERO, || None);
    let latest =
        SystemProxyConfig::detected(Some("http://127.0.0.1:8080".into()), None, None, vec![]);
    assert_eq!(
        cached_system_proxy_config(&cache, Duration::ZERO, || latest.clone()),
        latest
    );
    assert_eq!(
        cached_system_proxy_config(&cache, Duration::from_secs(60), || panic!("cache missed")),
        latest
    );
}
