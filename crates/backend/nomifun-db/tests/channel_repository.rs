//! Black-box integration tests for the v3 logical-reference channel schema.

use std::sync::Arc;

use nomifun_db::models::{
    ChannelPendingPromptRow, ChannelPluginRow, ChannelSessionRow, ChannelUserRow,
    NewChannelPairingCodeRow, NewChannelPluginRow, NewChannelSessionRow, NewChannelUserRow,
};
use nomifun_db::{
    DbError, IChannelRepository, PairingApprovalOutcome, SqliteChannelRepository,
    UpdatePluginStatusParams,
    init_database_memory,
};

async fn repo() -> (Arc<dyn IChannelRepository>, nomifun_db::Database) {
    let db = init_database_memory().await.unwrap();
    let repo = Arc::new(SqliteChannelRepository::new(db.pool().clone()));
    (repo as Arc<dyn IChannelRepository>, db)
}

#[tokio::test]
async fn channel_schema_has_only_canonical_tables_and_no_physical_foreign_keys() {
    let (_repo, db) = repo().await;

    let canonical_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master \
         WHERE type = 'table' AND name IN \
         ('channel_plugins', 'channel_users', 'channel_sessions', 'channel_pairing_codes')",
    )
    .fetch_one(db.pool())
    .await
    .unwrap();
    assert_eq!(canonical_count, 4);

    let legacy_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master \
         WHERE type = 'table' AND name IN \
         ('assistant_plugins', 'assistant_users', 'assistant_sessions', 'assistant_pairing_codes')",
    )
    .fetch_one(db.pool())
    .await
    .unwrap();
    assert_eq!(legacy_count, 0);

    let mut physical_fk_count = 0_i64;
    for table in [
        "channel_plugins",
        "channel_users",
        "channel_sessions",
        "channel_pairing_codes",
    ] {
        let sql = format!("SELECT COUNT(*) FROM pragma_foreign_key_list('{table}')");
        physical_fk_count += sqlx::query_scalar::<_, i64>(&sql)
            .fetch_one(db.pool())
            .await
            .unwrap();
    }
    assert_eq!(physical_fk_count, 0);
}

#[tokio::test]
async fn channel_user_has_no_reverse_session_relation_or_index() {
    let (_repo, db) = repo().await;

    let columns: Vec<String> =
        sqlx::query_scalar("SELECT name FROM pragma_table_info('channel_users') ORDER BY cid")
            .fetch_all(db.pool())
            .await
            .unwrap();
    assert!(!columns.iter().any(|column| column == "channel_session_id"));

    let indexes: Vec<String> =
        sqlx::query_scalar("SELECT name FROM pragma_index_list('channel_users')")
            .fetch_all(db.pool())
            .await
            .unwrap();
    assert!(!indexes
        .iter()
        .any(|index| index == "idx_channel_users_channel_session_id"));
}

fn plugin_fixture(plugin_type: &str, bot_key: &str) -> NewChannelPluginRow {
    let now = nomifun_common::now_ms();
    NewChannelPluginRow {
        r#type: plugin_type.into(),
        name: format!("{plugin_type} bot"),
        enabled: false,
        config: r#"{"credentials":{}}"#.into(),
        status: None,
        last_connected: None,
        companion_id: None,
        bot_key: Some(bot_key.into()),
        owner_domain: nomifun_db::models::default_owner_domain(),
        group_access_mode: nomifun_db::models::default_group_access_mode(),
        created_at: now,
        updated_at: now,
    }
}

async fn create_plugin(
    repo: &Arc<dyn IChannelRepository>,
    plugin_type: &str,
    bot_key: &str,
) -> ChannelPluginRow {
    repo.create_plugin(&plugin_fixture(plugin_type, bot_key))
        .await
        .unwrap()
}

fn user_fixture(
    channel_plugin_id: &str,
    platform_user_id: &str,
    platform_type: &str,
) -> NewChannelUserRow {
    let now = nomifun_common::now_ms();
    NewChannelUserRow {
        platform_user_id: platform_user_id.into(),
        platform_type: platform_type.into(),
        channel_plugin_id: Some(channel_plugin_id.to_owned()),
        display_name: Some(format!("User {platform_user_id}")),
        authorization_kind: nomifun_db::models::default_channel_user_authorization_kind(),
        authorized_at: now,
        last_active: None,
    }
}

async fn create_user(
    repo: &Arc<dyn IChannelRepository>,
    channel_plugin_id: &str,
    platform_user_id: &str,
) -> ChannelUserRow {
    repo.create_user(&user_fixture(
        channel_plugin_id,
        platform_user_id,
        "telegram",
    ))
    .await
    .unwrap()
}

fn session_fixture(
    channel_user_id: &str,
    channel_plugin_id: &str,
    chat_id: &str,
) -> NewChannelSessionRow {
    let now = nomifun_common::now_ms();
    NewChannelSessionRow {
        channel_session_id: nomifun_common::ChannelSessionId::new().into_string(),
        channel_user_id: channel_user_id.to_owned(),
        agent_type: "acp".into(),
        conversation_id: None,
        workspace: None,
        chat_id: Some(chat_id.into()),
        channel_plugin_id: Some(channel_plugin_id.to_owned()),
        chat_kind: nomifun_db::models::default_channel_chat_kind(),
        created_at: now,
        last_activity: now,
    }
}

async fn create_session_with_kind(
    repo: &Arc<dyn IChannelRepository>,
    user: &ChannelUserRow,
    plugin: &ChannelPluginRow,
    chat_id: &str,
    chat_kind: &str,
) -> ChannelSessionRow {
    let mut fixture = session_fixture(&user.channel_user_id, &plugin.channel_plugin_id, chat_id);
    fixture.chat_kind = chat_kind.to_owned();
    repo.get_or_create_session(
        &user.channel_user_id,
        chat_id,
        &plugin.channel_plugin_id,
        &fixture,
    )
    .await
    .unwrap()
}

async fn enqueue_session_prompt(
    repo: &Arc<dyn IChannelRepository>,
    plugin: &ChannelPluginRow,
    session: &ChannelSessionRow,
    text: &str,
) -> ChannelPendingPromptRow {
    let mut prompt = pending_prompt_fixture(
        &nomifun_common::ConversationId::new().into_string(),
        session.chat_id.as_deref().unwrap(),
        text,
    );
    prompt.channel_plugin_id = plugin.channel_plugin_id.clone();
    prompt.channel_session_id = session.channel_session_id.clone();
    let nomifun_db::PendingPromptEnqueue::Queued { row, .. } = repo
        .enqueue_pending_prompt(&prompt, nomifun_common::now_ms())
        .await
        .unwrap()
    else {
        panic!("fixture prompt must be queued");
    };
    row
}

fn pairing_fixture(
    code: &str,
    platform_user_id: &str,
    expires_offset_ms: i64,
) -> NewChannelPairingCodeRow {
    let now = nomifun_common::now_ms();
    NewChannelPairingCodeRow {
        code: code.into(),
        platform_user_id: platform_user_id.into(),
        platform_type: "telegram".into(),
        channel_plugin_id: None,
        display_name: Some("Tester".into()),
        requested_at: now,
        expires_at: now + expires_offset_ms,
        status: "pending".into(),
    }
}

#[tokio::test]
async fn plugin_full_lifecycle() {
    let (repo, _db) = repo().await;
    let telegram = create_plugin(&repo, "telegram", "telegram-bot").await;
    let lark = create_plugin(&repo, "lark", "lark-bot").await;

    repo.update_plugin_status(
        &telegram.channel_plugin_id,
        &UpdatePluginStatusParams {
            status: Some("running".into()),
            enabled: Some(true),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let telegram = repo
        .get_plugin(&telegram.channel_plugin_id)
        .await
        .unwrap()
        .unwrap();
    assert!(telegram.enabled);
    assert_eq!(telegram.status.as_deref(), Some("running"));

    repo.delete_plugin(&lark.channel_plugin_id).await.unwrap();
    assert_eq!(repo.get_all_plugins().await.unwrap().len(), 1);
}

#[tokio::test]
async fn duplicate_platform_user_is_rejected_within_one_plugin() {
    let (repo, _db) = repo().await;
    let plugin = create_plugin(&repo, "telegram", "telegram-bot").await;
    create_user(&repo, &plugin.channel_plugin_id, "tg_100").await;

    let duplicate = user_fixture(&plugin.channel_plugin_id, "tg_100", "telegram");
    assert!(matches!(
        repo.create_user(&duplicate).await,
        Err(DbError::Conflict(_))
    ));
}

#[tokio::test]
async fn plugin_group_access_mode_roundtrips_and_rejects_unknown_values() {
    let (repo, _db) = repo().await;
    let plugin = create_plugin(&repo, "lark", "lark-bot").await;
    assert_eq!(plugin.group_access_mode, "allowlist");

    // Simulate a full-row writer that loaded the plugin before the dedicated
    // policy update. Its stale policy field must not overwrite the newer value.
    let mut stale_full_row = plugin.clone();

    repo.update_plugin_group_access_mode_and_clear_non_direct_sessions(
        &plugin.channel_plugin_id,
        "all_members",
    )
    .await
    .unwrap();

    stale_full_row.name = "renamed by stale full-row writer".into();
    stale_full_row.group_access_mode = "stale-invalid-policy".into();
    stale_full_row.updated_at += 1;
    let after_full_update = repo.update_plugin(&stale_full_row).await.unwrap();
    assert_eq!(after_full_update.name, "renamed by stale full-row writer");
    assert_eq!(after_full_update.group_access_mode, "all_members");

    assert_eq!(
        repo.get_plugin(&plugin.channel_plugin_id)
            .await
            .unwrap()
            .unwrap()
            .group_access_mode,
        "all_members"
    );

    assert!(matches!(
        repo.update_plugin_group_access_mode_and_clear_non_direct_sessions(
            &plugin.channel_plugin_id,
            "members",
        )
        .await,
        Err(DbError::Conflict(_))
    ));
    assert_eq!(
        repo.get_plugin(&plugin.channel_plugin_id)
            .await
            .unwrap()
            .unwrap()
            .group_access_mode,
        "all_members"
    );
}

#[tokio::test]
async fn auto_group_identity_is_idempotent_hidden_and_promotable() {
    let (repo, _db) = repo().await;
    let plugin = create_plugin(&repo, "lark", "lark-bot").await;
    let mut automatic = user_fixture(&plugin.channel_plugin_id, "ou_group", "lark");
    automatic.authorization_kind = "auto_group".into();
    assert!(matches!(
        repo.create_user(&automatic).await,
        Err(DbError::Conflict(_))
    ));

    let first = repo.ensure_auto_group_user(&automatic).await.unwrap();
    let replay = repo.ensure_auto_group_user(&automatic).await.unwrap();
    assert_eq!(replay.channel_user_id, first.channel_user_id);
    assert_eq!(first.authorization_kind, "auto_group");
    assert!(repo.get_all_users().await.unwrap().is_empty());
    assert_eq!(
        repo.get_user(&first.channel_user_id)
            .await
            .unwrap()
            .unwrap()
            .authorization_kind,
        "auto_group"
    );

    let mut approval = automatic.clone();
    approval.authorization_kind = "approved".into();
    approval.authorized_at += 1;
    let approved = repo.create_user(&approval).await.unwrap();
    assert_eq!(approved.channel_user_id, first.channel_user_id);
    assert_eq!(approved.authorization_kind, "approved");
    assert_eq!(repo.get_all_users().await.unwrap().len(), 1);

    // Seeing the approved user in another open-group event never downgrades it.
    let replay_after_approval = repo.ensure_auto_group_user(&automatic).await.unwrap();
    assert_eq!(replay_after_approval.channel_user_id, first.channel_user_id);
    assert_eq!(replay_after_approval.authorization_kind, "approved");
}

#[tokio::test]
async fn classifying_legacy_session_clears_conversation_atomically() {
    let (repo, db) = repo().await;
    let plugin = create_plugin(&repo, "lark", "lark-bot").await;
    let user = create_user(&repo, &plugin.channel_plugin_id, "ou_group").await;
    let legacy = repo
        .get_or_create_session(
            &user.channel_user_id,
            "oc_group",
            &plugin.channel_plugin_id,
            &session_fixture(
                &user.channel_user_id,
                &plugin.channel_plugin_id,
                "oc_group",
            ),
        )
        .await
        .unwrap();
    assert_eq!(legacy.chat_kind, "unknown");

    sqlx::query(
        "UPDATE channel_sessions SET conversation_id = ? WHERE channel_session_id = ?",
    )
    .bind("0190f5fe-7c00-7a00-8abc-0123456789dd")
    .bind(&legacy.channel_session_id)
    .execute(db.pool())
    .await
    .unwrap();

    let mut classified = session_fixture(
        &user.channel_user_id,
        &plugin.channel_plugin_id,
        "oc_group",
    );
    classified.chat_kind = "group".into();
    let group_session = repo
        .get_or_create_session(
            &user.channel_user_id,
            "oc_group",
            &plugin.channel_plugin_id,
            &classified,
        )
        .await
        .unwrap();
    assert_eq!(group_session.channel_session_id, legacy.channel_session_id);
    assert_eq!(group_session.chat_kind, "group");
    assert!(group_session.conversation_id.is_none());

    let mut conflicting = classified;
    conflicting.chat_kind = "direct".into();
    assert!(matches!(
        repo.get_or_create_session(
            &user.channel_user_id,
            "oc_group",
            &plugin.channel_plugin_id,
            &conflicting,
        )
        .await,
        Err(DbError::Conflict(_))
    ));
}

#[tokio::test]
async fn deleting_user_transactionally_cascades_authoritative_sessions() {
    let (repo, _db) = repo().await;
    let plugin = create_plugin(&repo, "telegram", "telegram-bot").await;
    let user = create_user(&repo, &plugin.channel_plugin_id, "tg_1").await;

    for chat_id in ["chat-a", "chat-b"] {
        repo.get_or_create_session(
            &user.channel_user_id,
            chat_id,
            &plugin.channel_plugin_id,
            &session_fixture(
                &user.channel_user_id,
                &plugin.channel_plugin_id,
                chat_id,
            ),
        )
        .await
        .unwrap();
    }
    assert_eq!(repo.get_all_sessions().await.unwrap().len(), 2);

    repo.delete_user(&user.channel_user_id).await.unwrap();
    assert!(repo.get_all_sessions().await.unwrap().is_empty());
}

#[tokio::test]
async fn session_identity_is_scoped_by_plugin_user_and_chat() {
    let (repo, _db) = repo().await;
    let plugin = create_plugin(&repo, "telegram", "telegram-bot").await;
    let user_a = create_user(&repo, &plugin.channel_plugin_id, "tg_1").await;
    let user_b = create_user(&repo, &plugin.channel_plugin_id, "tg_2").await;

    let a1 = repo
        .get_or_create_session(
            &user_a.channel_user_id,
            "chat-a",
            &plugin.channel_plugin_id,
            &session_fixture(
                &user_a.channel_user_id,
                &plugin.channel_plugin_id,
                "chat-a",
            ),
        )
        .await
        .unwrap();
    let a1_replayed = repo
        .get_or_create_session(
            &user_a.channel_user_id,
            "chat-a",
            &plugin.channel_plugin_id,
            &session_fixture(
                &user_a.channel_user_id,
                &plugin.channel_plugin_id,
                "chat-a",
            ),
        )
        .await
        .unwrap();
    let a2 = repo
        .get_or_create_session(
            &user_a.channel_user_id,
            "chat-b",
            &plugin.channel_plugin_id,
            &session_fixture(
                &user_a.channel_user_id,
                &plugin.channel_plugin_id,
                "chat-b",
            ),
        )
        .await
        .unwrap();
    let b1 = repo
        .get_or_create_session(
            &user_b.channel_user_id,
            "chat-a",
            &plugin.channel_plugin_id,
            &session_fixture(
                &user_b.channel_user_id,
                &plugin.channel_plugin_id,
                "chat-a",
            ),
        )
        .await
        .unwrap();

    assert_eq!(a1.channel_session_id, a1_replayed.channel_session_id);
    assert_ne!(a1.channel_session_id, a2.channel_session_id);
    assert_ne!(a1.channel_session_id, b1.channel_session_id);
}

#[tokio::test]
async fn deleting_group_sessions_cancels_only_their_queued_prompts() {
    let (repo, db) = repo().await;
    let first_plugin = create_plugin(&repo, "telegram", "telegram-bot-a").await;
    let second_plugin = create_plugin(&repo, "telegram", "telegram-bot-b").await;
    let first_user = create_user(&repo, &first_plugin.channel_plugin_id, "tg_1").await;
    let second_user = create_user(&repo, &second_plugin.channel_plugin_id, "tg_2").await;

    let mut group_fixture = session_fixture(
        &first_user.channel_user_id,
        &first_plugin.channel_plugin_id,
        "group-a",
    );
    group_fixture.chat_kind = "group".into();
    let group_session = repo
        .get_or_create_session(
            &first_user.channel_user_id,
            "group-a",
            &first_plugin.channel_plugin_id,
            &group_fixture,
        )
        .await
        .unwrap();

    let mut direct_fixture = session_fixture(
        &first_user.channel_user_id,
        &first_plugin.channel_plugin_id,
        "direct-a",
    );
    direct_fixture.chat_kind = "direct".into();
    let direct_session = repo
        .get_or_create_session(
            &first_user.channel_user_id,
            "direct-a",
            &first_plugin.channel_plugin_id,
            &direct_fixture,
        )
        .await
        .unwrap();

    let unknown_session = repo
        .get_or_create_session(
            &first_user.channel_user_id,
            "unknown-a",
            &first_plugin.channel_plugin_id,
            &session_fixture(
                &first_user.channel_user_id,
                &first_plugin.channel_plugin_id,
                "unknown-a",
            ),
        )
        .await
        .unwrap();

    let mut other_group_fixture = session_fixture(
        &second_user.channel_user_id,
        &second_plugin.channel_plugin_id,
        "group-b",
    );
    other_group_fixture.chat_kind = "group".into();
    let other_group_session = repo
        .get_or_create_session(
            &second_user.channel_user_id,
            "group-b",
            &second_plugin.channel_plugin_id,
            &other_group_fixture,
        )
        .await
        .unwrap();

    let now = nomifun_common::now_ms();
    let mut queued = Vec::new();
    for (plugin_id, chat_id, session_id, text) in [
        (
            first_plugin.channel_plugin_id.as_str(),
            "group-a",
            group_session.channel_session_id.as_str(),
            "group prompt",
        ),
        (
            first_plugin.channel_plugin_id.as_str(),
            "direct-a",
            direct_session.channel_session_id.as_str(),
            "direct prompt",
        ),
        (
            first_plugin.channel_plugin_id.as_str(),
            "unknown-a",
            unknown_session.channel_session_id.as_str(),
            "unknown prompt",
        ),
        (
            second_plugin.channel_plugin_id.as_str(),
            "group-b",
            other_group_session.channel_session_id.as_str(),
            "other plugin prompt",
        ),
    ] {
        let mut prompt = pending_prompt_fixture(
            &nomifun_common::ConversationId::new().into_string(),
            chat_id,
            text,
        );
        prompt.channel_plugin_id = plugin_id.to_owned();
        prompt.channel_session_id = session_id.to_owned();
        let nomifun_db::PendingPromptEnqueue::Queued { row, .. } =
            repo.enqueue_pending_prompt(&prompt, now).await.unwrap()
        else {
            panic!("fixture prompt must be queued");
        };
        queued.push(row);
    }

    repo.delete_group_sessions_by_channel(&first_plugin.channel_plugin_id)
        .await
        .unwrap();

    assert!(repo.get_session(&group_session.channel_session_id).await.unwrap().is_none());
    for retained in [&direct_session, &unknown_session, &other_group_session] {
        assert!(
            repo.get_session(&retained.channel_session_id)
                .await
                .unwrap()
                .is_some(),
            "non-target session {} must be retained",
            retained.channel_session_id
        );
    }

    let states: Vec<(String, String, Option<i64>)> = sqlx::query_as(
        "SELECT prompt_id, state, settled_at FROM channel_pending_prompts ORDER BY id",
    )
    .fetch_all(db.pool())
    .await
    .unwrap();
    assert_eq!(states.len(), 4);
    assert_eq!(states[0].0, queued[0].prompt_id);
    assert_eq!(states[0].1, "cancelled");
    assert!(states[0].2.is_some());
    for (index, retained) in states.iter().enumerate().skip(1) {
        assert_eq!(retained.0, queued[index].prompt_id);
        assert_eq!(retained.1, "queued");
        assert!(retained.2.is_none());
    }
}

#[tokio::test]
async fn atomic_group_access_update_rolls_back_then_clears_non_direct_sessions() {
    let (repo, db) = repo().await;
    let target_plugin = create_plugin(&repo, "lark", "lark-bot-a").await;
    let other_plugin = create_plugin(&repo, "lark", "lark-bot-b").await;
    let target_user = create_user(&repo, &target_plugin.channel_plugin_id, "ou_target").await;
    let other_user = create_user(&repo, &other_plugin.channel_plugin_id, "ou_other").await;

    let mut target_group_fixture = session_fixture(
        &target_user.channel_user_id,
        &target_plugin.channel_plugin_id,
        "target-group",
    );
    target_group_fixture.chat_kind = "group".into();
    let target_group = repo
        .get_or_create_session(
            &target_user.channel_user_id,
            "target-group",
            &target_plugin.channel_plugin_id,
            &target_group_fixture,
        )
        .await
        .unwrap();

    let target_unknown = repo
        .get_or_create_session(
            &target_user.channel_user_id,
            "target-unknown",
            &target_plugin.channel_plugin_id,
            &session_fixture(
                &target_user.channel_user_id,
                &target_plugin.channel_plugin_id,
                "target-unknown",
            ),
        )
        .await
        .unwrap();

    let mut target_direct_fixture = session_fixture(
        &target_user.channel_user_id,
        &target_plugin.channel_plugin_id,
        "target-direct",
    );
    target_direct_fixture.chat_kind = "direct".into();
    let target_direct = repo
        .get_or_create_session(
            &target_user.channel_user_id,
            "target-direct",
            &target_plugin.channel_plugin_id,
            &target_direct_fixture,
        )
        .await
        .unwrap();

    let mut other_group_fixture = session_fixture(
        &other_user.channel_user_id,
        &other_plugin.channel_plugin_id,
        "other-group",
    );
    other_group_fixture.chat_kind = "group".into();
    let other_group = repo
        .get_or_create_session(
            &other_user.channel_user_id,
            "other-group",
            &other_plugin.channel_plugin_id,
            &other_group_fixture,
        )
        .await
        .unwrap();

    let now = nomifun_common::now_ms();
    let mut queued = Vec::new();
    for (plugin_id, session, text) in [
        (
            target_plugin.channel_plugin_id.as_str(),
            &target_group,
            "target group prompt",
        ),
        (
            target_plugin.channel_plugin_id.as_str(),
            &target_unknown,
            "target unknown prompt",
        ),
        (
            target_plugin.channel_plugin_id.as_str(),
            &target_direct,
            "target direct prompt",
        ),
        (
            other_plugin.channel_plugin_id.as_str(),
            &other_group,
            "other group prompt",
        ),
    ] {
        let mut prompt = pending_prompt_fixture(
            &nomifun_common::ConversationId::new().into_string(),
            session.chat_id.as_deref().unwrap(),
            text,
        );
        prompt.channel_plugin_id = plugin_id.to_owned();
        prompt.channel_session_id = session.channel_session_id.clone();
        let nomifun_db::PendingPromptEnqueue::Queued { row, .. } =
            repo.enqueue_pending_prompt(&prompt, now).await.unwrap()
        else {
            panic!("fixture prompt must be queued");
        };
        queued.push(row);
    }

    // Force the final session deletion to fail after the policy write, queue
    // cancellation and binding deletion. Every earlier mutation must roll back.
    let trigger_sql = format!(
        "CREATE TRIGGER fail_non_direct_channel_session_delete \
         BEFORE DELETE ON channel_sessions \
         WHEN OLD.channel_plugin_id = '{}' \
           AND OLD.chat_kind IN ('group', 'unknown') \
         BEGIN \
             SELECT RAISE(ABORT, 'forced non-direct cleanup failure'); \
         END",
        target_plugin.channel_plugin_id
    );
    sqlx::query(&trigger_sql)
        .execute(db.pool())
        .await
        .unwrap();

    assert!(
        repo.update_plugin_group_access_mode_and_clear_non_direct_sessions(
            &target_plugin.channel_plugin_id,
            "all_members",
        )
        .await
        .is_err()
    );
    assert_eq!(
        repo.get_plugin(&target_plugin.channel_plugin_id)
            .await
            .unwrap()
            .unwrap()
            .group_access_mode,
        "allowlist"
    );
    for session in [&target_group, &target_unknown, &target_direct, &other_group] {
        assert!(
            repo.get_session(&session.channel_session_id)
                .await
                .unwrap()
                .is_some(),
            "failed transaction must retain session {}",
            session.channel_session_id
        );
    }
    let rolled_back_states: Vec<(String, Option<i64>)> = sqlx::query_as(
        "SELECT state, settled_at FROM channel_pending_prompts ORDER BY id",
    )
    .fetch_all(db.pool())
    .await
    .unwrap();
    assert!(
        rolled_back_states
            .iter()
            .all(|(state, settled_at)| state == "queued" && settled_at.is_none())
    );

    sqlx::query("DROP TRIGGER fail_non_direct_channel_session_delete")
        .execute(db.pool())
        .await
        .unwrap();

    repo.update_plugin_group_access_mode_and_clear_non_direct_sessions(
        &target_plugin.channel_plugin_id,
        "all_members",
    )
    .await
    .unwrap();

    assert_eq!(
        repo.get_plugin(&target_plugin.channel_plugin_id)
            .await
            .unwrap()
            .unwrap()
            .group_access_mode,
        "all_members"
    );
    for removed in [&target_group, &target_unknown] {
        assert!(
            repo.get_session(&removed.channel_session_id)
                .await
                .unwrap()
                .is_none(),
            "target non-direct session {} must be retired",
            removed.channel_session_id
        );
    }
    for retained in [&target_direct, &other_group] {
        assert!(
            repo.get_session(&retained.channel_session_id)
                .await
                .unwrap()
                .is_some(),
            "direct or other-plugin session {} must be retained",
            retained.channel_session_id
        );
    }

    let states: Vec<(String, String, Option<i64>)> = sqlx::query_as(
        "SELECT prompt_id, state, settled_at FROM channel_pending_prompts ORDER BY id",
    )
    .fetch_all(db.pool())
    .await
    .unwrap();
    assert_eq!(states.len(), 4);
    for index in [0_usize, 1] {
        assert_eq!(states[index].0, queued[index].prompt_id);
        assert_eq!(states[index].1, "cancelled");
        assert!(states[index].2.is_some());
    }
    for index in [2_usize, 3] {
        assert_eq!(states[index].0, queued[index].prompt_id);
        assert_eq!(states[index].1, "queued");
        assert!(states[index].2.is_none());
    }

    let missing = nomifun_common::ChannelPluginId::new().into_string();
    assert!(matches!(
        repo.update_plugin_group_access_mode_and_clear_non_direct_sessions(
            &missing,
            "disabled"
        )
        .await,
        Err(DbError::NotFound(_))
    ));
}

#[tokio::test]
async fn pairing_approval_atomically_retires_guest_non_direct_authority() {
    let (repo, db) = repo().await;
    let plugin = create_plugin(&repo, "telegram", "pairing-promotion-bot").await;
    let other_plugin = create_plugin(&repo, "telegram", "pairing-other-bot").await;
    let mut guest_fixture = user_fixture(
        &plugin.channel_plugin_id,
        "tg_guest_promotion",
        "telegram",
    );
    guest_fixture.authorization_kind = "auto_group".into();
    let guest = repo.ensure_auto_group_user(&guest_fixture).await.unwrap();
    let other_user = create_user(&repo, &other_plugin.channel_plugin_id, "tg_other").await;

    let group = create_session_with_kind(&repo, &guest, &plugin, "promotion-group", "group").await;
    let unknown =
        create_session_with_kind(&repo, &guest, &plugin, "promotion-unknown", "unknown").await;
    let direct =
        create_session_with_kind(&repo, &guest, &plugin, "promotion-direct", "direct").await;
    let other =
        create_session_with_kind(&repo, &other_user, &other_plugin, "other-group", "group").await;
    let prompts = vec![
        enqueue_session_prompt(&repo, &plugin, &group, "promotion group").await,
        enqueue_session_prompt(&repo, &plugin, &unknown, "promotion unknown").await,
        enqueue_session_prompt(&repo, &plugin, &direct, "promotion direct").await,
        enqueue_session_prompt(&repo, &other_plugin, &other, "other user").await,
    ];

    let now = nomifun_common::now_ms();
    let code = "730001";
    repo.create_pairing(&NewChannelPairingCodeRow {
        code: code.into(),
        platform_user_id: guest.platform_user_id.clone(),
        platform_type: guest.platform_type.clone(),
        channel_plugin_id: Some(plugin.channel_plugin_id.clone()),
        display_name: Some("Approved guest".into()),
        requested_at: now,
        expires_at: now + 60_000,
        status: "pending".into(),
    })
    .await
    .unwrap();

    // Abort late in the transition, after the user/status/queue writes. The
    // transaction must restore the original guest authority and queued work.
    let trigger_sql = format!(
        "CREATE TRIGGER fail_pairing_session_retirement \
         BEFORE DELETE ON channel_sessions \
         WHEN OLD.channel_user_id = '{}' \
           AND OLD.chat_kind IN ('group', 'unknown') \
         BEGIN \
             SELECT RAISE(ABORT, 'forced pairing retirement failure'); \
         END",
        guest.channel_user_id
    );
    sqlx::query(&trigger_sql)
        .execute(db.pool())
        .await
        .unwrap();
    assert!(
        repo.approve_pairing_and_retire_non_direct_sessions(code, now + 1)
            .await
            .is_err()
    );
    let rolled_back_user = repo
        .get_user(&guest.channel_user_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(rolled_back_user.authorization_kind, "auto_group");
    assert_eq!(
        repo.get_pairing_by_code(code)
            .await
            .unwrap()
            .unwrap()
            .status,
        "pending"
    );
    for session in [&group, &unknown, &direct, &other] {
        assert!(
            repo.get_session(&session.channel_session_id)
                .await
                .unwrap()
                .is_some()
        );
    }
    let rolled_back_states: Vec<String> =
        sqlx::query_scalar("SELECT state FROM channel_pending_prompts ORDER BY id")
            .fetch_all(db.pool())
            .await
            .unwrap();
    assert_eq!(rolled_back_states, vec!["queued"; 4]);

    sqlx::query("DROP TRIGGER fail_pairing_session_retirement")
        .execute(db.pool())
        .await
        .unwrap();
    let PairingApprovalOutcome::Approved(promoted) = repo
        .approve_pairing_and_retire_non_direct_sessions(code, now + 2)
        .await
        .unwrap()
    else {
        panic!("valid pending pairing must be approved");
    };
    assert_eq!(promoted.channel_user_id, guest.channel_user_id);
    assert_eq!(promoted.authorization_kind, "approved");
    assert_eq!(
        repo.get_pairing_by_code(code)
            .await
            .unwrap()
            .unwrap()
            .status,
        "approved"
    );
    for removed in [&group, &unknown] {
        assert!(
            repo.get_session(&removed.channel_session_id)
                .await
                .unwrap()
                .is_none()
        );
    }
    for retained in [&direct, &other] {
        assert!(
            repo.get_session(&retained.channel_session_id)
                .await
                .unwrap()
                .is_some()
        );
    }
    let states: Vec<(String, String)> = sqlx::query_as(
        "SELECT prompt_id, state FROM channel_pending_prompts ORDER BY id",
    )
    .fetch_all(db.pool())
    .await
    .unwrap();
    for index in [0_usize, 1] {
        assert_eq!(states[index], (prompts[index].prompt_id.clone(), "cancelled".into()));
    }
    for index in [2_usize, 3] {
        assert_eq!(states[index], (prompts[index].prompt_id.clone(), "queued".into()));
    }
}

#[tokio::test]
async fn user_revocation_atomically_cancels_every_chat_kind_and_preserves_others() {
    let (repo, db) = repo().await;
    let plugin = create_plugin(&repo, "lark", "revocation-bot").await;
    let target = create_user(&repo, &plugin.channel_plugin_id, "ou_revoked").await;
    let other = create_user(&repo, &plugin.channel_plugin_id, "ou_retained").await;

    let group = create_session_with_kind(&repo, &target, &plugin, "revoke-group", "group").await;
    let unknown =
        create_session_with_kind(&repo, &target, &plugin, "revoke-unknown", "unknown").await;
    let direct = create_session_with_kind(&repo, &target, &plugin, "revoke-direct", "direct").await;
    let other_direct =
        create_session_with_kind(&repo, &other, &plugin, "other-direct", "direct").await;
    let prompts = vec![
        enqueue_session_prompt(&repo, &plugin, &group, "revoke group").await,
        enqueue_session_prompt(&repo, &plugin, &unknown, "revoke unknown").await,
        enqueue_session_prompt(&repo, &plugin, &direct, "revoke direct").await,
        enqueue_session_prompt(&repo, &plugin, &other_direct, "other direct").await,
    ];

    let trigger_sql = format!(
        "CREATE TRIGGER fail_channel_user_revocation \
         BEFORE DELETE ON channel_users \
         WHEN OLD.channel_user_id = '{}' \
         BEGIN \
             SELECT RAISE(ABORT, 'forced user revocation failure'); \
         END",
        target.channel_user_id
    );
    sqlx::query(&trigger_sql)
        .execute(db.pool())
        .await
        .unwrap();
    assert!(
        repo.revoke_user_and_cancel_pending(&target.channel_user_id, nomifun_common::now_ms())
            .await
            .is_err()
    );
    assert!(repo.get_user(&target.channel_user_id).await.unwrap().is_some());
    for session in [&group, &unknown, &direct, &other_direct] {
        assert!(
            repo.get_session(&session.channel_session_id)
                .await
                .unwrap()
                .is_some()
        );
    }
    let rolled_back_states: Vec<String> =
        sqlx::query_scalar("SELECT state FROM channel_pending_prompts ORDER BY id")
            .fetch_all(db.pool())
            .await
            .unwrap();
    assert_eq!(rolled_back_states, vec!["queued"; 4]);

    sqlx::query("DROP TRIGGER fail_channel_user_revocation")
        .execute(db.pool())
        .await
        .unwrap();
    repo.revoke_user_and_cancel_pending(&target.channel_user_id, nomifun_common::now_ms())
        .await
        .unwrap();
    assert!(repo.get_user(&target.channel_user_id).await.unwrap().is_none());
    assert!(repo.get_user(&other.channel_user_id).await.unwrap().is_some());
    for removed in [&group, &unknown, &direct] {
        assert!(
            repo.get_session(&removed.channel_session_id)
                .await
                .unwrap()
                .is_none()
        );
    }
    assert!(
        repo.get_session(&other_direct.channel_session_id)
            .await
            .unwrap()
            .is_some()
    );
    let states: Vec<(String, String)> = sqlx::query_as(
        "SELECT prompt_id, state FROM channel_pending_prompts ORDER BY id",
    )
    .fetch_all(db.pool())
    .await
    .unwrap();
    for index in [0_usize, 1, 2] {
        assert_eq!(states[index], (prompts[index].prompt_id.clone(), "cancelled".into()));
    }
    assert_eq!(states[3], (prompts[3].prompt_id.clone(), "queued".into()));
}

#[tokio::test]
async fn pairing_expiry_and_status_transitions() {
    let (repo, _db) = repo().await;
    let now = nomifun_common::now_ms();
    repo.create_pairing(&pairing_fixture("111111", "tg_1", -1_000))
        .await
        .unwrap();
    repo.create_pairing(&pairing_fixture("222222", "tg_2", 600_000))
        .await
        .unwrap();

    assert_eq!(repo.cleanup_expired_pairings(now).await.unwrap(), 1);
    assert_eq!(
        repo.get_pairing_by_code("111111")
            .await
            .unwrap()
            .unwrap()
            .status,
        "expired"
    );

    repo.update_pairing_status("222222", "approved")
        .await
        .unwrap();
    assert_eq!(
        repo.get_pairing_by_code("222222")
            .await
            .unwrap()
            .unwrap()
            .status,
        "approved"
    );
}

// ── Busy-time pending prompt queue (spec D1) ─────────────────────────

fn pending_prompt_fixture(
    conversation_id: &str,
    chat_id: &str,
    text: &str,
) -> nomifun_db::models::NewChannelPendingPromptRow {
    nomifun_db::models::NewChannelPendingPromptRow {
        channel_plugin_id: nomifun_common::ChannelPluginId::new().into_string(),
        chat_id: chat_id.to_owned(),
        channel_session_id: nomifun_common::ChannelSessionId::new().into_string(),
        conversation_id: conversation_id.to_owned(),
        text: text.to_owned(),
        idempotency_key: format!("channel-turn:v1:key-{text}"),
    }
}

#[tokio::test]
async fn pending_prompt_enqueue_reports_fifo_position_and_peek_returns_head() {
    let (repo, _db) = repo().await;
    let conversation = nomifun_common::ConversationId::new().into_string();
    let now = nomifun_common::now_ms();

    let first = repo
        .enqueue_pending_prompt(&pending_prompt_fixture(&conversation, "chat-1", "one"), now)
        .await
        .unwrap();
    let nomifun_db::PendingPromptEnqueue::Queued { row: first_row, position } = first else {
        panic!("first enqueue must be queued");
    };
    assert_eq!(position, 1);
    assert_eq!(first_row.state, "queued");
    assert_eq!(first_row.attempts, 0);

    let second = repo
        .enqueue_pending_prompt(&pending_prompt_fixture(&conversation, "chat-1", "two"), now + 1)
        .await
        .unwrap();
    let nomifun_db::PendingPromptEnqueue::Queued { position, .. } = second else {
        panic!("second enqueue must be queued");
    };
    assert_eq!(position, 2);

    // FIFO head is the earliest queued row, even after the later insert.
    let head = repo.peek_next_queued(&conversation).await.unwrap().unwrap();
    assert_eq!(head.prompt_id, first_row.prompt_id);
    assert_eq!(head.text, "one");

    // Other conversations see an empty queue.
    let other = nomifun_common::ConversationId::new().into_string();
    assert!(repo.peek_next_queued(&other).await.unwrap().is_none());
}

#[tokio::test]
async fn pending_prompt_enqueue_rejects_when_conversation_queue_is_full() {
    let (repo, _db) = repo().await;
    let conversation = nomifun_common::ConversationId::new().into_string();
    let now = nomifun_common::now_ms();

    for index in 0..10 {
        let outcome = repo
            .enqueue_pending_prompt(
                &pending_prompt_fixture(&conversation, "chat-1", &format!("prompt {index}")),
                now + index,
            )
            .await
            .unwrap();
        assert!(matches!(outcome, nomifun_db::PendingPromptEnqueue::Queued { .. }));
    }

    let overflow = repo
        .enqueue_pending_prompt(&pending_prompt_fixture(&conversation, "chat-1", "over"), now + 11)
        .await
        .unwrap();
    assert_eq!(overflow, nomifun_db::PendingPromptEnqueue::QueueFull);

    // Settling one row frees capacity again.
    let head = repo.peek_next_queued(&conversation).await.unwrap().unwrap();
    repo.settle_prompt(&head.prompt_id, "delivered", now + 12)
        .await
        .unwrap();
    let after = repo
        .enqueue_pending_prompt(&pending_prompt_fixture(&conversation, "chat-1", "next"), now + 13)
        .await
        .unwrap();
    assert!(matches!(after, nomifun_db::PendingPromptEnqueue::Queued { position: 10, .. }));
}

#[tokio::test]
async fn pending_prompt_settlement_is_absorbing_and_validates_state() {
    let (repo, _db) = repo().await;
    let conversation = nomifun_common::ConversationId::new().into_string();
    let now = nomifun_common::now_ms();

    let nomifun_db::PendingPromptEnqueue::Queued { row, .. } = repo
        .enqueue_pending_prompt(&pending_prompt_fixture(&conversation, "chat-1", "one"), now)
        .await
        .unwrap()
    else {
        panic!("enqueue must succeed");
    };

    assert!(
        repo.settle_prompt(&row.prompt_id, "running", now).await.is_err(),
        "non-terminal state must be rejected"
    );
    repo.settle_prompt(&row.prompt_id, "failed", now + 1).await.unwrap();
    assert!(
        repo.settle_prompt(&row.prompt_id, "delivered", now + 2).await.is_err(),
        "terminal state is absorbing"
    );
    assert!(repo.peek_next_queued(&conversation).await.unwrap().is_none());
}

#[tokio::test]
async fn pending_prompt_attempts_increment_only_while_queued() {
    let (repo, _db) = repo().await;
    let conversation = nomifun_common::ConversationId::new().into_string();
    let now = nomifun_common::now_ms();

    let nomifun_db::PendingPromptEnqueue::Queued { row, .. } = repo
        .enqueue_pending_prompt(&pending_prompt_fixture(&conversation, "chat-1", "one"), now)
        .await
        .unwrap()
    else {
        panic!("enqueue must succeed");
    };

    assert_eq!(repo.increment_prompt_attempts(&row.prompt_id).await.unwrap(), 1);
    assert_eq!(repo.increment_prompt_attempts(&row.prompt_id).await.unwrap(), 2);
    repo.settle_prompt(&row.prompt_id, "failed", now + 1).await.unwrap();
    assert!(repo.increment_prompt_attempts(&row.prompt_id).await.is_err());
}

#[tokio::test]
async fn pending_prompt_expiry_settles_only_stale_queued_rows() {
    let (repo, _db) = repo().await;
    let conversation = nomifun_common::ConversationId::new().into_string();
    let now = nomifun_common::now_ms();

    let nomifun_db::PendingPromptEnqueue::Queued { row: stale, .. } = repo
        .enqueue_pending_prompt(&pending_prompt_fixture(&conversation, "chat-1", "stale"), now - 10)
        .await
        .unwrap()
    else {
        panic!("enqueue must succeed");
    };
    let nomifun_db::PendingPromptEnqueue::Queued { row: fresh, .. } = repo
        .enqueue_pending_prompt(&pending_prompt_fixture(&conversation, "chat-1", "fresh"), now + 10)
        .await
        .unwrap()
    else {
        panic!("enqueue must succeed");
    };

    let expired = repo.expire_stale(now, now + 20).await.unwrap();
    assert_eq!(expired.len(), 1);
    assert_eq!(expired[0].prompt_id, stale.prompt_id);
    assert_eq!(expired[0].state, "expired");
    assert_eq!(expired[0].settled_at, Some(now + 20));

    let head = repo.peek_next_queued(&conversation).await.unwrap().unwrap();
    assert_eq!(head.prompt_id, fresh.prompt_id, "fresh row survives the sweep");

    // Second sweep with the same watermark is a no-op.
    assert!(repo.expire_stale(now, now + 30).await.unwrap().is_empty());
}

#[tokio::test]
async fn pending_prompt_chat_cancel_clears_only_that_chat_scope() {
    let (repo, _db) = repo().await;
    let conversation = nomifun_common::ConversationId::new().into_string();
    let now = nomifun_common::now_ms();

    let mut fixture_a = pending_prompt_fixture(&conversation, "chat-a", "one");
    let plugin = fixture_a.channel_plugin_id.clone();
    repo.enqueue_pending_prompt(&fixture_a, now).await.unwrap();
    fixture_a.text = "two".into();
    fixture_a.idempotency_key = "channel-turn:v1:another-key".into();
    repo.enqueue_pending_prompt(&fixture_a, now + 1).await.unwrap();

    // Same plugin, different chat: untouched by the cancel.
    let mut fixture_b = pending_prompt_fixture(&conversation, "chat-b", "keep");
    fixture_b.channel_plugin_id = plugin.clone();
    repo.enqueue_pending_prompt(&fixture_b, now + 2).await.unwrap();

    assert_eq!(repo.cancel_chat_queue(&plugin, "chat-a", now + 3).await.unwrap(), 2);
    assert_eq!(repo.cancel_chat_queue(&plugin, "chat-a", now + 4).await.unwrap(), 0);

    let head = repo.peek_next_queued(&conversation).await.unwrap().unwrap();
    assert_eq!(head.text, "keep");

    assert_eq!(
        repo.list_queued_conversations().await.unwrap(),
        vec![conversation.clone()]
    );
}
