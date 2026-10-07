use super::*;
use nomifun_voice_contracts::{DigestHex, VoiceProfile, digest_payload};
use serde_json::json;

async fn data_fixture() -> (tempfile::TempDir, VoiceJournal) {
    let dir = tempfile::tempdir().unwrap();
    let journal = VoiceJournal::open(dir.path()).unwrap();
    journal
        .activate(VoiceActivationFact {
            voice_session_id: "voice".into(),
            epoch: 1,
            owner_id: "owner".into(),
            agent_session_id: "session".into(),
            binding_version: 1,
            route_digest: "a".repeat(64),
            started_ms: now_ms(),
            context_floor: Some(0),
            lease_revision: Some("lease".into()),
        })
        .await
        .unwrap();
    // A valid saved configuration is data; no provider connection or model
    // authority is opened by saving the optional profile.
    journal
        .save_profile("owner".into(), profile("profile", "session"), 0)
        .await
        .unwrap();
    (dir, journal)
}
fn profile(id: &str, session: &str) -> VoiceProfile {
    let config = json!({});
    let mut required = native_duplex_requirements();
    required.insert(VoiceFeature::TypedTools);
    VoiceProfile {
        work_steering_policy: Default::default(),
        profile_id: id.into(),
        revision: 1,
        agent_session_id: session.into(),
        binding_version: 1,
        enabled: false,
        label: "Saved optional voice configuration".into(),
        route: VoiceRouteRecord {
            schema: VOICE_ROUTE_SCHEMA.into(),
            route_id: format!("voice-route:{id}"),
            revision: 1,
            provider_id: "configured-provider".into(),
            model: "configured-model".into(),
            model_revision: 0,
            connection_config_ref: "provider:configured-provider:default".into(),
            credential_ref: "provider:configured-provider:default".into(),
            adapter_id: "registered-adapter".into(),
            adapter_contract_version: VOICE_CONTRACT_VERSION,
            adapter_config_digest: digest_payload(&config).unwrap(),
            adapter_config: config,
            connection_config_digest: DigestHex("a".repeat(64)),
            required_features: required,
            transport: VoiceTransportPreference::Relay,
        },
    }
}
async fn end_data_lease(journal: &VoiceJournal) {
    journal
        .close(
            "voice".into(),
            1,
            VoiceTermination {
                reason: VoiceCloseReason::UserEnded,
                finalization_confirmed: true,
                message: None,
            },
        )
        .await
        .unwrap();
}
#[tokio::test]
async fn voice_data_export_is_owner_bounded_sqlite_snapshot_and_never_main_backup() {
    let (dir, journal) = data_fixture().await;
    assert_eq!(
        journal.referenced_sessions("owner".into()).await.unwrap(),
        vec!["session"]
    );
    assert_eq!(
        journal
            .export_snapshot("other".into())
            .await
            .unwrap_err()
            .kind,
        VoiceErrorKind::Authentication
    );
    let (link, _) = journal
        .reserve_trigger(
            "voice".into(),
            1,
            "export-trigger".into(),
            "input-revision".into(),
        )
        .await
        .unwrap();
    journal
        .associate_receipt(link.operation_key, "original-canonical-reference".into())
        .await
        .unwrap();
    let sequence = journal
        .append(
            "voice".into(),
            1,
            "export-event".into(),
            "voice_event".into(),
            None,
            serde_json::to_value(VoiceModelEvent::Transcript {
                fragment: TranscriptFragment {
                    speaker: VoiceSpeaker::User,
                    fragment_id: "committed-user".into(),
                    revision: 1,
                    commit: TranscriptCommit::Committed,
                    text: "Explicit user input".into(),
                    media_range: None,
                },
            })
            .unwrap(),
        )
        .await
        .unwrap();
    let exported = journal.export_snapshot("owner".into()).await.unwrap();
    assert!(exported.bytes.starts_with(b"SQLite format 3\0"));
    assert_eq!(exported.boundary.schema_version, SCHEMA_VERSION);
    assert_eq!(exported.boundary.activation_count, 1);
    let path = dir.path().join("read-snapshot.sqlite3");
    std::fs::write(&path, &exported.bytes).unwrap();
    let snapshot =
        Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let owner: String = snapshot
        .query_row("SELECT owner_id FROM voice_profiles", [], |row| row.get(0))
        .unwrap();
    assert_eq!(owner, "owner");
    assert!(exported.bytes.len() <= 32 * 1024 * 1024);
    assert_eq!(
        snapshot
            .query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0))
            .unwrap(),
        "ok"
    );
    assert_eq!(
        exported.boundary.schema_version,
        snapshot
            .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .unwrap()
    );
    assert_eq!(exported.boundary.max_sequence, sequence);
    assert_eq!(
        exported.boundary.max_sequence,
        snapshot
            .query_row("SELECT MAX(sequence) FROM voice_facts", [], |row| row
                .get::<_, i64>(0))
            .unwrap()
    );
    assert_eq!(
        exported.boundary.activation_count,
        snapshot
            .query_row("SELECT COUNT(*) FROM voice_activations", [], |row| row
                .get::<_, i64>(0))
            .unwrap()
    );
    assert_eq!(
        exported.boundary.work_link_count,
        snapshot
            .query_row("SELECT COUNT(*) FROM voice_work_links", [], |row| row
                .get::<_, i64>(0))
            .unwrap()
    );
    assert_eq!(exported.boundary.work_link_count, 1);
    journal
        .append(
            "voice".into(),
            1,
            "after-snapshot".into(),
            "test_context".into(),
            None,
            json!({"content":"later durable voice record"}),
        )
        .await
        .unwrap();
    assert_eq!(
        snapshot
            .query_row("SELECT COUNT(*) FROM voice_facts", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        1,
        "the exported bytes remain a self-consistent point-in-time snapshot after later writes"
    );
    assert!(!dir.path().join("nomifun-backend.db").exists());
    journal
        .save_profile("other".into(), profile("foreign", "foreign-session"), 0)
        .await
        .unwrap();
    assert_eq!(
        journal.referenced_sessions("owner".into()).await.unwrap(),
        vec!["session"]
    );
    assert_eq!(
        journal.referenced_sessions("other".into()).await.unwrap(),
        vec!["foreign-session"]
    );
    assert_eq!(
        journal
            .export_snapshot("owner".into())
            .await
            .unwrap_err()
            .kind,
        VoiceErrorKind::Authentication,
        "whole-database export cannot disclose another owner"
    );
}
#[tokio::test]
async fn voice_data_delete_intent_mode_survives_restart_and_preserves_non_agent_profiles() {
    let (dir, journal) = data_fixture().await;
    let main_path = dir.path().join("nomifun-backend.db");
    let main = Connection::open(&main_path).unwrap();
    main.execute_batch("CREATE TABLE main_sentinel(value TEXT); INSERT INTO main_sentinel VALUES ('original Desktop data');").unwrap();
    drop(main);
    let original_main = std::fs::read(&main_path).unwrap();
    let (link, _) = journal
        .reserve_trigger(
            "voice".into(),
            1,
            "delete-queued".into(),
            "input-revision".into(),
        )
        .await
        .unwrap();
    journal
        .record_pending_intent(
            "owner".into(),
            "session".into(),
            1,
            0,
            link.operation_key,
            "Explicit pending voice input".into(),
        )
        .await
        .unwrap();
    journal
        .begin_owned_session_delete("owner".into(), "session".into(), false)
        .await
        .unwrap();
    assert_eq!(
        journal
            .finish_owned_session_delete("owner".into(), "session".into(), false)
            .await
            .unwrap_err()
            .kind,
        VoiceErrorKind::Closed,
        "active lease cannot be silently deleted"
    );
    assert!(
        journal
            .profile("owner".into(), "profile".into())
            .await
            .unwrap()
            .is_some()
    );
    assert_eq!(
        journal
            .begin_owned_session_delete("other".into(), "session".into(), false)
            .await
            .unwrap_err()
            .kind,
        VoiceErrorKind::Authentication
    );
    assert!(
        journal
            .begin_owned_session_delete("owner".into(), "session".into(), true)
            .await
            .is_err(),
        "a retry cannot upgrade passive intent to profile deletion"
    );
    assert!(
        journal.request_delete("session".into()).await.is_err(),
        "a legacy explicit request cannot upgrade a passive durable intent"
    );
    end_data_lease(&journal).await;
    drop(journal);
    let journal = VoiceJournal::open(dir.path()).unwrap();
    assert_eq!(journal.pending_deletes().await.unwrap(), vec!["session"]);
    assert!(
        journal
            .finish_owned_session_delete("owner".into(), "session".into(), true)
            .await
            .is_err(),
        "mode cannot change after process recovery"
    );
    journal.finish_delete("session".into()).await.unwrap();
    let profiles: i64 = journal
        .run(|conn| {
            conn.query_row("SELECT COUNT(*) FROM voice_profiles", [], |row| row.get(0))
                .map_err(failure)
        })
        .await
        .unwrap();
    assert_eq!(profiles, 1);
    assert_eq!(
        journal
            .profile("owner".into(), "profile".into())
            .await
            .unwrap(),
        Some(profile("profile", "session"))
    );
    assert!(journal.pending_deletes().await.unwrap().is_empty());
    assert!(
        journal.queued_intents().await.unwrap().is_empty(),
        "deletion cascades through original voice links and pending input facts"
    );
    journal
        .begin_owned_session_delete("owner".into(), "session".into(), true)
        .await
        .unwrap();
    assert!(
        journal
            .begin_owned_session_delete("owner".into(), "session".into(), false)
            .await
            .is_err(),
        "an explicit deletion retry cannot change its immutable mode either"
    );
    assert_eq!(
        journal
            .finish_owned_session_delete("other".into(), "session".into(), true)
            .await
            .unwrap_err()
            .kind,
        VoiceErrorKind::Authentication
    );
    journal
        .finish_owned_session_delete("owner".into(), "session".into(), true)
        .await
        .unwrap();
    assert!(
        journal
            .referenced_sessions("owner".into())
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        std::fs::read(&main_path).unwrap(),
        original_main,
        "voice export/delete/recovery never rewrites the existing Main SQLite bytes"
    );
}
#[tokio::test]
async fn voice_data_legacy_unknown_delete_mode_is_conservative_and_wrong_owner_cannot_delete() {
    let (_dir, journal) = data_fixture().await;
    assert!(
        journal
            .begin_owned_session_delete("other".into(), "session".into(), false)
            .await
            .is_err()
    );
    end_data_lease(&journal).await;
    journal.run(|conn|{conn.execute("INSERT INTO voice_delete_intents(agent_session_id,requested_ms) VALUES ('session',1)",[]).map_err(failure)?;Ok(())}).await.unwrap();
    journal.finish_delete("session".into()).await.unwrap();
    let count: i64 = journal
        .run(|conn| {
            conn.query_row("SELECT COUNT(*) FROM voice_profiles", [], |row| row.get(0))
                .map_err(failure)
        })
        .await
        .unwrap();
    assert_eq!(count, 1);
}
#[tokio::test]
async fn voice_snapshot_over_32_mib_is_rejected_using_legal_bounded_fact_rows() {
    let (dir, journal) = data_fixture().await;
    let payload = json!({"content":"x".repeat(240*1024)});
    assert!(payload.to_string().len() < MAX_FACT_BYTES);
    for index in 0..140 {
        journal
            .append(
                "voice".into(),
                1,
                format!("bounded-export-size:{index}"),
                "test_context".into(),
                None,
                payload.clone(),
            )
            .await
            .unwrap();
    }
    let error = journal.export_snapshot("owner".into()).await.unwrap_err();
    assert_eq!(error.kind, VoiceErrorKind::Backlog);
    assert!(error.message.contains("bounded export size"));
    assert!(
        journal
            .profile("owner".into(), "profile".into())
            .await
            .unwrap()
            .is_some(),
        "size refusal does not delete admitted data"
    );
    assert!(!dir.path().join("nomifun-backend.db").exists());
}

#[tokio::test]
async fn durable_delete_intent_fences_profile_cas_and_new_profile_creation_until_settled() {
    for explicit in [false, true] {
        let (dir, journal) = data_fixture().await;
        end_data_lease(&journal).await;
        journal
            .begin_owned_session_delete("owner".into(), "session".into(), explicit)
            .await
            .unwrap();
        let mut revised = profile("profile", "session");
        revised.revision = 2;
        revised.label = "New configuration must not disappear after a successful save".into();
        assert_eq!(
            journal
                .save_profile("owner".into(), revised.clone(), 1)
                .await
                .unwrap_err()
                .kind,
            VoiceErrorKind::StaleBinding
        );
        assert_eq!(
            journal
                .save_profile("owner".into(), profile("new-profile", "session"), 0)
                .await
                .unwrap_err()
                .kind,
            VoiceErrorKind::StaleBinding
        );
        assert_eq!(
            journal
                .profile("owner".into(), "profile".into())
                .await
                .unwrap(),
            Some(profile("profile", "session")),
            "failed CAS cannot mutate the saved revision"
        );
        drop(journal);
        let journal = VoiceJournal::open(dir.path()).unwrap();
        assert_eq!(
            journal
                .save_profile("owner".into(), revised.clone(), 1)
                .await
                .unwrap_err()
                .kind,
            VoiceErrorKind::StaleBinding,
            "reopening does not remove the durable deletion fence"
        );
        assert_eq!(
            journal
                .save_profile("owner".into(), profile("new-profile", "session"), 0)
                .await
                .unwrap_err()
                .kind,
            VoiceErrorKind::StaleBinding
        );
        journal
            .finish_owned_session_delete("owner".into(), "session".into(), explicit)
            .await
            .unwrap();
        if explicit {
            journal
                .save_profile("owner".into(), profile("new-profile", "session"), 0)
                .await
                .unwrap();
        } else {
            assert_eq!(
                journal
                    .save_profile("owner".into(), revised.clone(), 1)
                    .await
                    .unwrap(),
                revised
            );
        }
        assert!(!dir.path().join("nomifun-backend.db").exists());
    }
}
