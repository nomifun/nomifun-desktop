//! Integration tests for the webhook + tag_settings repositories.

use nomifun_db::models::{TagSettingRow, WebhookRow};
use nomifun_db::{
    ITagSettingRepository, IWebhookRepository, SqliteTagSettingRepository, SqliteWebhookRepository,
    init_database_memory,
};
use std::sync::Arc;

fn sample_webhook() -> WebhookRow {
    WebhookRow {
        // id is ignored on insert (AUTOINCREMENT assigns it); any value works.
        id: 0,
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
    let id = repo.insert(&sample_webhook()).await.unwrap();
    // get
    let got = repo.get_by_id(id).await.unwrap().expect("present");
    assert_eq!(got.name, "Team bot");
    assert_eq!(got.secret.as_deref(), Some("s3cr3t"));
    // list
    repo.insert(&sample_webhook()).await.unwrap();
    let all = repo.list_all().await.unwrap();
    assert_eq!(all.len(), 2);
    // update
    let mut upd = got.clone();
    upd.name = "Renamed".into();
    upd.enabled = false;
    upd.updated_at = 9;
    repo.update(&upd).await.unwrap();
    let after = repo.get_by_id(id).await.unwrap().unwrap();
    assert_eq!(after.name, "Renamed");
    assert!(!after.enabled);
    // delete
    repo.delete(id).await.unwrap();
    assert!(repo.get_by_id(id).await.unwrap().is_none());
}

#[tokio::test]
async fn webhook_update_and_delete_missing_is_not_found() {
    let db = init_database_memory().await.unwrap();
    let repo: Arc<dyn IWebhookRepository> = Arc::new(SqliteWebhookRepository::new(db.pool().clone()));
    let err = repo.delete(9999).await.unwrap_err();
    assert!(matches!(err, nomifun_db::DbError::NotFound(_)));
    let mut ghost = sample_webhook();
    ghost.id = 9999;
    let err = repo.update(&ghost).await.unwrap_err();
    assert!(matches!(err, nomifun_db::DbError::NotFound(_)));
}

#[tokio::test]
async fn tag_setting_upsert_get_list_delete() {
    let db = init_database_memory().await.unwrap();
    let repo: Arc<dyn ITagSettingRepository> = Arc::new(SqliteTagSettingRepository::new(db.pool().clone()));

    // Seed a webhook so the tag_settings.webhook_id FK is satisfiable.
    let wh_repo: Arc<dyn IWebhookRepository> = Arc::new(SqliteWebhookRepository::new(db.pool().clone()));
    let wh_id = wh_repo.insert(&sample_webhook()).await.unwrap();

    // absent → None
    assert!(repo.get("alpha").await.unwrap().is_none());

    // upsert (insert)
    repo.upsert(&TagSettingRow {
        tag: "alpha".into(),
        webhook_id: Some(wh_id),
        description: "queue alpha".into(),
        notify_events: "done,failed,needs_review".to_string(),
        updated_at: 5,
    })
    .await
    .unwrap();
    let got = repo.get("alpha").await.unwrap().unwrap();
    assert_eq!(got.webhook_id, Some(wh_id));

    // upsert (update — same key replaces)
    repo.upsert(&TagSettingRow {
        tag: "alpha".into(),
        webhook_id: None,
        description: "unbound now".into(),
        notify_events: "done,failed,needs_review".to_string(),
        updated_at: 6,
    })
    .await
    .unwrap();
    let got = repo.get("alpha").await.unwrap().unwrap();
    assert_eq!(got.webhook_id, None);
    assert_eq!(got.description, "unbound now");

    // list
    repo.upsert(&TagSettingRow {
        tag: "beta".into(),
        webhook_id: None,
        description: String::new(),
        notify_events: "done,failed,needs_review".to_string(),
        updated_at: 7,
    })
    .await
    .unwrap();
    assert_eq!(repo.list_all().await.unwrap().len(), 2);

    // delete (idempotent)
    repo.delete("alpha").await.unwrap();
    assert!(repo.get("alpha").await.unwrap().is_none());
    repo.delete("alpha").await.unwrap(); // no error on absent
}
