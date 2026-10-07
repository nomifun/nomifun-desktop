use std::collections::BTreeMap;
use std::sync::mpsc;
use std::time::Duration;

use nomifun_agent_contracts::PluginDraftId;
use nomifun_plugin_platform::{
    CancellationFlag, NeverCancel, PluginDraftRecord, PluginDraftStatus, PluginDraftStore,
    PluginDraftStoreError, PluginRepository, PluginRepositoryError, SqlitePluginRepository,
};
use serde_json::json;
use uuid::Uuid;

struct Fixture {
    _temp: tempfile::TempDir,
    store: PluginDraftStore,
    owner: String,
    draft_id: PluginDraftId,
}

impl Fixture {
    fn new() -> Self {
        Self::with_owner(Uuid::now_v7().to_string())
    }

    fn with_owner(owner: String) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let store = PluginDraftStore::new(temp.path().join("drafts")).unwrap();
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

#[tokio::test]
async fn losing_edit_cas_rolls_back_to_the_winning_files_and_database_record() {
    let database = nomifun_db::init_database_memory().await.unwrap();
    let owner = nomifun_db::installation_owner_id(database.pool()).await.unwrap();
    let fixture = Fixture::with_owner(owner);
    let repository = SqlitePluginRepository::new(database.pool().clone());
    let draft = PluginDraftRecord {
        owner_user_id: fixture.owner.clone(), draft_id: fixture.draft_id.clone(), revision: 1,
        plugin_id: None, base_revision: None, name: "Original".into(),
        workspace_path: fixture.store.open(&fixture.owner, &fixture.draft_id)
            .unwrap().to_string_lossy().into_owned(),
        source_conversation_id: None, source_message_id: None, source_operation_key: None,
        source_request_digest: None, verification: json!({"edit_revision": 1}),
        imported_context: json!({}), status: PluginDraftStatus::Ready, last_error: None,
        created_at_ms: 1, updated_at_ms: 1,
    };
    repository.create_draft(&draft).await.unwrap();

    // Both editors read the same row and tree before either commits. Run their
    // commits in a deterministic order to exercise the stale-editor interleaving.
    let mut winner = repository.get_draft(&fixture.owner, &fixture.draft_id).await.unwrap().unwrap();
    let mut loser = repository.get_draft(&fixture.owner, &fixture.draft_id).await.unwrap().unwrap();
    let mut winning_files = fixture.snapshot();
    let mut rejected_files = winning_files.clone();
    winning_files.insert("ui/index.html".into(), b"accepted ui".to_vec());
    rejected_files.insert("ui/index.html".into(), b"rejected ui".to_vec());
    rejected_files.remove("source/obsolete.ts");
    assert_eq!(winner.revision, loser.revision);

    let mut replacement = fixture.store.stage_exact_replacement(
        &fixture.owner, &fixture.draft_id, &winning_files, &NeverCancel,
    ).unwrap();
    winner.name = "Accepted".into();
    winner.verification["edit_revision"] = json!(winner.revision + 1);
    replacement.publish().unwrap();
    let committed = repository.update_draft(&winner, winner.revision).await.unwrap();
    replacement.commit().unwrap();

    let mut replacement = fixture.store.stage_exact_replacement(
        &fixture.owner, &fixture.draft_id, &rejected_files, &NeverCancel,
    ).unwrap();
    loser.name = "Rejected".into();
    loser.verification["edit_revision"] = json!(loser.revision + 1);
    replacement.publish().unwrap();
    assert!(matches!(repository.update_draft(&loser, loser.revision).await,
        Err(PluginRepositoryError::Conflict)));
    replacement.rollback().unwrap();

    assert_eq!(fixture.snapshot(), winning_files,
        "a rejected replacement and deletion must restore the accepted editor's entire tree");
    assert_eq!(repository.get_draft(&fixture.owner, &fixture.draft_id).await.unwrap().unwrap(), committed);
    assert_eq!(committed.revision, 2);
    assert_eq!(committed.verification["edit_revision"], json!(2));
}
