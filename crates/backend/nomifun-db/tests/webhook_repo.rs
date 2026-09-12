//! Integration tests for the webhook + tag_settings repositories.

use nomifun_db::models::{TagSettingPatch, WebhookPatch, WebhookRow};
use nomifun_db::{
    ITagSettingRepository, IWebhookRepository, SqliteTagSettingRepository, SqliteWebhookRepository,
    init_database_memory,
};
use std::sync::Arc;

fn sample_webhook() -> WebhookRow {
    WebhookRow {
        webhook_id: nomifun_common::generate_id(),
        name: "Team bot".into(),
        platform: "lark".into(),
        url: "https://open.feishu.cn/open-apis/bot/v2/hook/abc".into(),
        secret: Some("s3cr3t".into()),
        description: "team notifications".into(),
        enabled: true,
        created_at: 1,
        updated_at: 1,
    }
}

#[tokio::test]
async fn webhook_crud_roundtrip() {
    let db = init_database_memory().await.unwrap();
    let repo: Arc<dyn IWebhookRepository> = Arc::new(SqliteWebhookRepository::new(db.pool().clone()));

    // create
    let created = repo.insert(&sample_webhook()).await.unwrap();
    assert!(nomifun_common::validate_uuidv7(&created.webhook_id).is_ok());
    // get
    let got = repo
        .get_by_webhook_id(&created.webhook_id)
        .await
        .unwrap()
        .expect("present");
    assert_eq!(got.webhook_id, created.webhook_id);
    assert_eq!(got.name, "Team bot");
    assert_eq!(got.secret.as_deref(), Some("s3cr3t"));
    // list
    let second = repo.insert(&sample_webhook()).await.unwrap();
    assert_ne!(second.webhook_id, created.webhook_id);
    let all = repo.list_all().await.unwrap();
    assert_eq!(all.len(), 2);
    // update
    let updated = repo.update(&got.webhook_id, &WebhookPatch {
        name: Some("Renamed".into()), enabled: Some(false), updated_at: 9,
        ..Default::default()
    }).await.unwrap();
    assert_eq!(updated.name, "Renamed");
    assert_eq!(updated.created_at, got.created_at);
    assert_eq!(updated.secret, got.secret);
    let after = repo
        .get_by_webhook_id(&created.webhook_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after.name, "Renamed");
    assert!(!after.enabled);
    let changed = repo.update(&created.webhook_id, &WebhookPatch {
        url: Some("https://example.invalid/new".into()),
        platform: Some("http".into()),
        description: Some(String::new()),
        secret: Some(Some("replacement".into())),
        enabled: Some(true),
        updated_at: 10,
        ..Default::default()
    }).await.unwrap();
    assert_eq!(changed.name, "Renamed");
    assert_eq!(changed.url, "https://example.invalid/new");
    assert_eq!(changed.platform, "http");
    assert!(changed.description.is_empty());
    assert_eq!(changed.secret.as_deref(), Some("replacement"));
    assert!(changed.enabled);
    let cleared = repo.update(&created.webhook_id, &WebhookPatch {
        secret: Some(None), updated_at: 11, ..Default::default()
    }).await.unwrap();
    assert_eq!(cleared.secret, None);
    assert_eq!(cleared.url, changed.url);
    // delete
    repo.delete(&created.webhook_id).await.unwrap();
    assert!(
        repo.get_by_webhook_id(&created.webhook_id)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn webhook_update_and_delete_missing_is_not_found() {
    let db = init_database_memory().await.unwrap();
    let repo: Arc<dyn IWebhookRepository> = Arc::new(SqliteWebhookRepository::new(db.pool().clone()));
    let missing = "0190f5fe-7c00-7a00-8000-000000000999";
    let err = repo.delete(missing).await.unwrap_err();
    assert!(matches!(err, nomifun_db::DbError::NotFound(_)));
    let err = repo.update(missing, &WebhookPatch::default()).await.unwrap_err();
    assert!(matches!(err, nomifun_db::DbError::NotFound(_)));
}

#[tokio::test]
async fn webhook_delete_sets_tag_setting_reference_null() {
    let db = init_database_memory().await.unwrap();
    let webhook_repo = SqliteWebhookRepository::new(db.pool().clone());
    let tag_repo = SqliteTagSettingRepository::new(db.pool().clone());
    let webhook_id = webhook_repo
        .insert(&sample_webhook())
        .await
        .unwrap()
        .webhook_id;
    tag_repo
        .upsert("alpha", &TagSettingPatch {
            webhook_id: Some(Some(webhook_id.clone())),
            description: Some("bound".into()),
            notify_events: Some("done".into()),
            updated_at: 1,
        })
        .await
        .unwrap();

    webhook_repo.delete(&webhook_id).await.unwrap();

    let setting = tag_repo.get("alpha").await.unwrap().unwrap();
    assert_eq!(setting.webhook_id, None);
    assert_eq!(setting.description, "bound");
}

#[tokio::test]
async fn tag_setting_upsert_get_list_delete() {
    let db = init_database_memory().await.unwrap();
    let repo: Arc<dyn ITagSettingRepository> = Arc::new(SqliteTagSettingRepository::new(db.pool().clone()));
    let webhook_repo = SqliteWebhookRepository::new(db.pool().clone());
    let webhook_id = webhook_repo
        .insert(&sample_webhook())
        .await
        .unwrap()
        .webhook_id;

    // absent → None
    assert!(repo.get("alpha").await.unwrap().is_none());

    // upsert (insert)
    repo.upsert("alpha", &TagSettingPatch {
        webhook_id: Some(Some(webhook_id.clone())),
        description: Some("queue alpha".into()),
        updated_at: 5,
        ..Default::default()
    })
    .await
    .unwrap();
    let got = repo.get("alpha").await.unwrap().unwrap();
    assert_eq!(got.webhook_id, Some(webhook_id));

    // upsert (update — explicit null clears; omitted events are retained)
    repo.upsert("alpha", &TagSettingPatch {
        webhook_id: Some(None),
        description: Some("unbound now".into()),
        updated_at: 6,
        ..Default::default()
    })
    .await
    .unwrap();
    let got = repo.get("alpha").await.unwrap().unwrap();
    assert_eq!(got.webhook_id, None);
    assert_eq!(got.description, "unbound now");
    assert_eq!(got.notify_events, "done,failed,needs_review");
    assert_eq!(got.updated_at, 6);

    // list
    repo.upsert("beta", &TagSettingPatch { updated_at: 7, ..Default::default() })
    .await
    .unwrap();
    assert_eq!(repo.list_all().await.unwrap().len(), 2);

    // delete (idempotent)
    repo.delete("alpha").await.unwrap();
    assert!(repo.get("alpha").await.unwrap().is_none());
    repo.delete("alpha").await.unwrap(); // no error on absent
}

#[tokio::test]
async fn invalid_tag_binding_rolls_back_all_patch_fields() {
    let db = init_database_memory().await.unwrap();
    let repo = SqliteTagSettingRepository::new(db.pool().clone());
    repo.upsert("alpha", &TagSettingPatch {
        description: Some("keep".into()), notify_events: Some(String::new()),
        updated_at: 7, ..Default::default()
    }).await.unwrap();
    for missing in ["invalid", "0190f5fe-7c00-7a00-8000-000000000999"] {
        let error = repo.upsert("alpha", &TagSettingPatch {
            webhook_id: Some(Some(missing.into())), description: Some("lost".into()),
            updated_at: 8, ..Default::default()
        }).await.unwrap_err();
        assert!(matches!(error, nomifun_db::DbError::Conflict(_)));
        let kept = repo.get("alpha").await.unwrap().unwrap();
        assert_eq!(kept.description, "keep");
        assert_eq!(kept.webhook_id, None);
        assert!(kept.notify_events.is_empty());
        assert_eq!(kept.updated_at, 7);
    }
}

#[tokio::test]
async fn webhook_repository_rejects_noncanonical_business_ids() {
    let db = init_database_memory().await.unwrap();
    let repo = SqliteWebhookRepository::new(db.pool().clone());

    for value in [
        "42",
        "550e8400-e29b-41d4-a716-446655440000",
        "0190F5FE-7C00-7A00-8000-000000000042",
        "webhook_0190f5fe-7c00-7a00-8000-000000000042",
    ] {
        let mut row = sample_webhook();
        row.webhook_id = value.to_string();
        let err = repo.insert(&row).await.unwrap_err();
        assert!(matches!(err, nomifun_db::DbError::Conflict(_)));
    }
}
