//! Retention prune body, called from the observer loop's prune tick.

use crate::store::ActivityStore;
use nomifun_common::AppError;

/// Delete activity segments that ended before `ts_ms`.
pub async fn prune(store: &ActivityStore, ts_ms: i64) -> Result<u64, AppError> {
    store.purge_before(ts_ms).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{ActivitySegment, SegmentFilter};

    fn segment(end: i64) -> ActivitySegment {
        ActivitySegment {
            event_id: nomifun_common::generate_id(),
            user_id: "01900000-0000-7000-8000-000000000001".into(),
            started_at_ms: end - 100,
            ended_at_ms: end,
            app_name: "App".into(),
            bundle_id: None,
            window_title: None,
            browser_url: None,
            browser_title: None,
            source: "foreground".into(),
            captured_at_ms: end,
        }
    }

    #[tokio::test]
    async fn prunes_only_old_rows() {
        let store = ActivityStore::open_memory().await.unwrap();
        store.append_segments(&[segment(100), segment(10_000)]).await.unwrap();
        let removed = prune(&store, 5_000).await.unwrap();
        assert_eq!(removed, 1);
        assert_eq!(
            store.query_segments(&SegmentFilter::default()).await.unwrap().len(),
            1
        );
    }
}
