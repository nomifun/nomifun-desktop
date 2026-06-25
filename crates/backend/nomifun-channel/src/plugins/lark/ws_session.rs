use std::collections::HashMap;
use std::time::{Duration, Instant};

use serde::Deserialize;

pub struct FragmentCache {
    pub entries: HashMap<String, FragmentEntry>,
}

pub struct FragmentEntry {
    pub sum: usize,
    pub fragments: Vec<Option<Vec<u8>>>,
    pub created_at: Instant,
}

impl FragmentCache {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    pub fn push(&mut self, message_id: &str, sum: usize, seq: usize, data: &[u8]) -> Option<Vec<u8>> {
        let entry = self
            .entries
            .entry(message_id.to_owned())
            .or_insert_with(|| FragmentEntry {
                sum,
                fragments: vec![None; sum],
                created_at: Instant::now(),
            });

        if seq < entry.sum {
            entry.fragments[seq] = Some(data.to_vec());
        }

        if entry.fragments.iter().all(|f| f.is_some()) {
            let merged: Vec<u8> = entry
                .fragments
                .iter()
                .flat_map(|f| f.as_ref().unwrap().iter().copied())
                .collect();
            self.entries.remove(message_id);
            Some(merged)
        } else {
            None
        }
    }

    pub fn cleanup(&mut self, ttl: Duration) {
        self.entries.retain(|_, entry| entry.created_at.elapsed() < ttl);
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct PongConfig {
    pub ping_interval_secs: u64,
    pub reconnect_count: i32,
    pub reconnect_interval_secs: u64,
    pub reconnect_nonce_secs: u64,
}

#[derive(Deserialize)]
struct PongPayload {
    #[serde(rename = "PingInterval")]
    ping_interval: u64,
    #[serde(rename = "ReconnectCount")]
    reconnect_count: i32,
    #[serde(rename = "ReconnectInterval")]
    reconnect_interval: u64,
    #[serde(rename = "ReconnectNonce")]
    reconnect_nonce: u64,
}

pub fn parse_pong_config(payload: &[u8]) -> Option<PongConfig> {
    let p: PongPayload = serde_json::from_slice(payload).ok()?;
    Some(PongConfig {
        ping_interval_secs: p.ping_interval,
        reconnect_count: p.reconnect_count,
        reconnect_interval_secs: p.reconnect_interval,
        reconnect_nonce_secs: p.reconnect_nonce,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_fragment_returns_immediately() {
        let mut cache = FragmentCache::new();
        let result = cache.push("msg_1", 1, 0, b"hello");
        assert_eq!(result, Some(b"hello".to_vec()));
    }

    #[test]
    fn multi_fragment_returns_on_complete() {
        let mut cache = FragmentCache::new();
        let r1 = cache.push("msg_2", 3, 0, b"aaa");
        assert_eq!(r1, None);
        let r2 = cache.push("msg_2", 3, 2, b"ccc");
        assert_eq!(r2, None);
        let r3 = cache.push("msg_2", 3, 1, b"bbb");
        assert_eq!(r3, Some(b"aaabbbccc".to_vec()));
    }

    #[test]
    fn different_message_ids_independent() {
        let mut cache = FragmentCache::new();
        cache.push("msg_a", 2, 0, b"A0");
        let r = cache.push("msg_b", 1, 0, b"B");
        assert_eq!(r, Some(b"B".to_vec()));
        let r2 = cache.push("msg_a", 2, 1, b"A1");
        assert_eq!(r2, Some(b"A0A1".to_vec()));
    }

    #[test]
    fn cleanup_removes_old_entries() {
        let mut cache = FragmentCache::new();
        cache.push("old_msg", 3, 0, b"partial");
        if let Some(entry) = cache.entries.get_mut("old_msg") {
            entry.created_at = Instant::now() - Duration::from_secs(600);
        }
        cache.cleanup(Duration::from_secs(300));
        assert!(!cache.entries.contains_key("old_msg"));
    }

    #[test]
    fn parse_pong_config_extracts_intervals() {
        let payload = br#"{"PingInterval":120,"ReconnectCount":10,"ReconnectInterval":120,"ReconnectNonce":30}"#;
        let config = parse_pong_config(payload).unwrap();
        assert_eq!(config.ping_interval_secs, 120);
        assert_eq!(config.reconnect_count, 10);
    }

    #[test]
    fn parse_pong_config_invalid_json_returns_none() {
        let result = parse_pong_config(b"not json");
        assert!(result.is_none());
    }
}
