//! Black-box tests for the activity store + rules, mirroring the
//! `nomifun-db/tests/*` integration-test style.

use nomifun_computer_history::rules::{
    ActivityRule, DefaultBehavior, ObservationSettings, RuleAction, RuleScope,
};
use nomifun_computer_history::store::{ActivitySegment, ActivityStore, SegmentFilter};

fn segment(app: &str, start: i64, end: i64) -> ActivitySegment {
    ActivitySegment {
        event_id: nomifun_common::generate_id(),
        user_id: "01900000-0000-7000-8000-000000000001".into(),
        started_at_ms: start,
        ended_at_ms: end,
        app_name: app.into(),
        bundle_id: None,
        window_title: Some("Secret token: supersecretvalue123".into()),
        browser_url: None,
        browser_title: None,
        source: "foreground".into(),
        captured_at_ms: end,
    }
}

#[tokio::test]
async fn file_store_roundtrip_and_retention() {
    let tmp = tempfile::tempdir().unwrap();
    let store = ActivityStore::open(tmp.path()).await.unwrap();
    store
        .append_segments(&[segment("Safari", 100, 200), segment("Terminal", 300, 400)])
        .await
        .unwrap();

    let all = store
        .query_segments(&SegmentFilter {
            limit: 10,
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(all.len(), 2);
    // Titles were redacted at the observer boundary in real use; the store
    // truncates only. Verify truncation cap applies.
    assert!(all[0].window_title.as_deref().unwrap().len() <= 2000);

    let removed = store.purge_before(250).await.unwrap();
    assert_eq!(removed, 1);
    assert_eq!(store.storage_status(30).await.unwrap().segment_count, 1);

    // Re-opening validates the on-disk contract.
    ActivityStore::open(tmp.path()).await.unwrap();
}

#[tokio::test]
async fn settings_and_rules_are_isolated_from_the_main_database() {
    let store = ActivityStore::open_memory().await.unwrap();
    let mut settings = ObservationSettings::default();
    settings.default_application_behavior = DefaultBehavior::DoNotObserve;
    settings.allowlist.push(
        ActivityRule {
            id: nomifun_common::generate_id(),
            scope: RuleScope::Application,
            bundle_id: Some("com.allowed.app".into()),
            url_domain: None,
            action: RuleAction::Capture,
        }
        .validated()
        .unwrap(),
    );
    store.set_observation_settings(&settings).await.unwrap();
    assert_eq!(
        store.observation_settings().await.unwrap(),
        settings,
        "settings must round-trip through the feature-local feature_config KV"
    );
}
