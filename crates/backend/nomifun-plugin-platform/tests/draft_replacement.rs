use std::collections::BTreeMap;
use std::sync::mpsc;
use std::time::Duration;

use nomifun_agent_contracts::PluginDraftId;
use nomifun_plugin_platform::{
    CancellationFlag, NeverCancel, PluginDraftStore, PluginDraftStoreError,
};
use uuid::Uuid;

struct Fixture {
    _temp: tempfile::TempDir,
    store: PluginDraftStore,
    owner: String,
    draft_id: PluginDraftId,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let store = PluginDraftStore::new(temp.path().join("drafts")).unwrap();
        let owner = Uuid::now_v7().to_string();
        let draft_id = PluginDraftId::from(Uuid::now_v7().to_string());
        store.create(&owner, &draft_id).unwrap();
        store
            .write(&owner, &draft_id, "nomifun.plugin.json", br#"{"old":true}"#)
            .unwrap();
        store
            .write(&owner, &draft_id, "ui/index.html", b"old ui")
            .unwrap();
        store
            .write(&owner, &draft_id, "source/obsolete.ts", b"obsolete")
            .unwrap();
        Self {
            _temp: temp,
            store,
            owner,
            draft_id,
        }
    }

    fn snapshot(&self) -> BTreeMap<String, Vec<u8>> {
        self.store.freeze(&self.owner, &self.draft_id).unwrap()
    }

    fn replacement() -> BTreeMap<String, Vec<u8>> {
        BTreeMap::from([
            (
                "nomifun.plugin.json".into(),
                br#"{"new":true}"#.to_vec(),
            ),
            ("ui/index.html".into(), b"new ui".to_vec()),
        ])
    }
}

#[test]
fn staged_validation_failure_and_drop_leave_original_tree_byte_exact() {
    let fixture = Fixture::new();
    let original = fixture.snapshot();

    let mut invalid = Fixture::replacement();
    invalid.insert("outside.txt".into(), b"forbidden".to_vec());
    assert!(fixture
        .store
        .stage_exact_replacement(
            &fixture.owner,
            &fixture.draft_id,
            &invalid,
            &NeverCancel,
        )
        .is_err());
    assert_eq!(fixture.snapshot(), original);

    let staged = fixture
        .store
        .stage_exact_replacement(
            &fixture.owner,
            &fixture.draft_id,
            &Fixture::replacement(),
            &NeverCancel,
        )
        .unwrap();
    drop(staged);
    assert_eq!(fixture.snapshot(), original);
}

#[test]
fn canceled_stage_leaves_original_tree_byte_exact() {
    let fixture = Fixture::new();
    let original = fixture.snapshot();
    let cancellation = CancellationFlag::default();
    cancellation.cancel();
    assert!(matches!(
        fixture.store.stage_exact_replacement(
            &fixture.owner,
            &fixture.draft_id,
            &Fixture::replacement(),
            &cancellation,
        ),
        Err(PluginDraftStoreError::Canceled)
    ));
    assert_eq!(fixture.snapshot(), original);
}

#[test]
fn publish_is_exact_and_rollback_restores_original_tree() {
    let fixture = Fixture::new();
    let original = fixture.snapshot();
    let replacement = Fixture::replacement();

    let mut staged = fixture
        .store
        .stage_exact_replacement(
            &fixture.owner,
            &fixture.draft_id,
            &replacement,
            &NeverCancel,
        )
        .unwrap();
    staged.publish().unwrap();
    staged.rollback().unwrap();
    assert_eq!(fixture.snapshot(), original);

    let mut staged = fixture
        .store
        .stage_exact_replacement(
            &fixture.owner,
            &fixture.draft_id,
            &replacement,
            &NeverCancel,
        )
        .unwrap();
    staged.publish().unwrap();
    staged.commit().unwrap();
    assert_eq!(fixture.snapshot(), replacement);
    assert!(!fixture.snapshot().contains_key("source/obsolete.ts"));
}

#[test]
fn readers_wait_for_staged_replacement_and_never_observe_a_mixed_tree() {
    let fixture = Fixture::new();
    let original = fixture.snapshot();
    let staged = fixture
        .store
        .stage_exact_replacement(
            &fixture.owner,
            &fixture.draft_id,
            &Fixture::replacement(),
            &NeverCancel,
        )
        .unwrap();
    let store = fixture.store.clone();
    let owner = fixture.owner.clone();
    let draft_id = fixture.draft_id.clone();
    let (send, receive) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        send.send(store.freeze(&owner, &draft_id).unwrap()).unwrap();
    });
    std::thread::sleep(Duration::from_millis(50));
    assert!(matches!(receive.try_recv(), Err(mpsc::TryRecvError::Empty)));
    drop(staged);
    assert_eq!(receive.recv_timeout(Duration::from_secs(2)).unwrap(), original);
    reader.join().unwrap();
}
