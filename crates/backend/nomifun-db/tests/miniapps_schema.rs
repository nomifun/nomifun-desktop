//! miniapps must satisfy the id-schema contract that runs on every boot, the
//! repository must enforce owner isolation, and a deleted source conversation
//! must leave the app runnable.
use nomifun_common::{ConversationId, MiniAppId};
use nomifun_db::{
    CreateMiniAppParams, IConversationRepository, IMiniAppRepository, SqliteConversationRepository,
    SqliteMiniAppRepository, UpdateMiniAppParams,
};

async fn seed_user(pool: &sqlx::SqlitePool) -> String {
    let user_id = nomifun_common::UserId::new().as_str().to_string();
    sqlx::query(
        "INSERT INTO users (user_id, username, password_hash, jwt_secret, created_at, updated_at) \
         VALUES (?, ?, '', '', 0, 0)",
    )
    .bind(&user_id)
    .bind(&user_id)
    .execute(pool)
    .await
    .expect("seed user");
    user_id
}

#[tokio::test]
async fn migrations_apply_and_id_contract_passes_with_miniapps() {
    // init_database_memory runs all migrations + validate_id_schema_contract.
    let db = nomifun_db::init_database_memory()
        .await
        .expect("init in-memory db with miniapps migration + contract");
    let exists: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM sqlite_schema WHERE type='table' AND name='miniapps'",
    )
    .fetch_one(db.pool())
    .await
    .unwrap();
    assert_eq!(exists, 1, "miniapps table must exist after migration");
}

#[tokio::test]
async fn create_list_update_delete_is_owner_scoped() {
    let db = nomifun_db::init_database_memory().await.expect("db");
    // Real UUIDv7 owner ids (the users.user_id CHECK enforces the format), so the
    // miniapps.user_id logical reference resolves against real rows.
    let user_a = seed_user(db.pool()).await;
    let user_b = seed_user(db.pool()).await;

    let repo = SqliteMiniAppRepository::new(db.pool().clone());
    let created = repo
        .create(
            &user_a,
            CreateMiniAppParams {
                name: "Pomodoro",
                description: "25/5 timer",
                icon: Some("⏱"),
                html: "<h1>timer</h1>",
                ..Default::default()
            },
        )
        .await
        .expect("create");
    let id = MiniAppId::parse(created.miniapp_id.clone()).unwrap();
    assert_eq!(created.html_size, "<h1>timer</h1>".len() as i64);
    assert_eq!(created.source_conversation_id, None);

    // Owner A sees it; owner B does not.
    assert_eq!(repo.list(&user_a).await.unwrap().len(), 1);
    assert_eq!(repo.list(&user_b).await.unwrap().len(), 0);
    // Cross-owner find is indistinguishable from absent.
    assert!(repo.find(&user_b, &id).await.unwrap().is_none());
    assert!(repo.find(&user_a, &id).await.unwrap().is_some());

    // The serve read is the ONE unscoped one, and it hands back the body only.
    let document = repo
        .find_by_id_any_owner(&id)
        .await
        .unwrap()
        .expect("serve read resolves by id alone");
    assert_eq!(document.html, "<h1>timer</h1>");

    // Cross-owner update fails; owner update rewrites the body and the size.
    assert!(
        repo.update(
            &user_b,
            &id,
            UpdateMiniAppParams {
                name: Some("stolen"),
                ..Default::default()
            },
        )
        .await
        .is_err()
    );
    let updated = repo
        .update(
            &user_a,
            &id,
            UpdateMiniAppParams {
                name: Some("Pomodoro v2"),
                icon: Some(None),
                html: Some("<h1>timer v2</h1>"),
                ..Default::default()
            },
        )
        .await
        .expect("update");
    assert_eq!(updated.name, "Pomodoro v2");
    assert_eq!(updated.icon, None);
    assert_eq!(updated.html_size, "<h1>timer v2</h1>".len() as i64);
    assert_eq!(
        repo.find_by_id_any_owner(&id).await.unwrap().unwrap().html,
        "<h1>timer v2</h1>",
        "an update that supplies html must replace the stored document"
    );

    // A rename that supplies no html must leave the document alone (COALESCE, not
    // an overwrite with NULL — which the NOT NULL column would reject anyway, and
    // an overwrite with '' would silently blank a working app).
    repo.update(
        &user_a,
        &id,
        UpdateMiniAppParams {
            name: Some("Pomodoro v3"),
            ..Default::default()
        },
    )
    .await
    .expect("rename");
    assert_eq!(
        repo.find_by_id_any_owner(&id).await.unwrap().unwrap().html,
        "<h1>timer v2</h1>"
    );

    // Cross-owner delete fails; owner delete succeeds.
    assert!(repo.delete(&user_b, &id).await.is_err());
    assert!(repo.delete(&user_a, &id).await.is_ok());
    assert_eq!(repo.list(&user_a).await.unwrap().len(), 0);
}

#[tokio::test]
async fn list_is_ordered_by_updated_at_descending() {
    let db = nomifun_db::init_database_memory().await.expect("db");
    let user = seed_user(db.pool()).await;
    let repo = SqliteMiniAppRepository::new(db.pool().clone());

    for name in ["first", "second", "third"] {
        repo.create(
            &user,
            CreateMiniAppParams {
                name,
                description: "",
                html: "<p/>",
                ..Default::default()
            },
        )
        .await
        .expect("create");
    }
    // now_ms() can hand out the same millisecond to all three, so pin the order
    // explicitly rather than trusting the clock's resolution.
    for (name, updated_at) in [("first", 300_i64), ("second", 100), ("third", 200)] {
        sqlx::query("UPDATE miniapps SET updated_at = ? WHERE name = ?")
            .bind(updated_at)
            .bind(name)
            .execute(db.pool())
            .await
            .expect("stamp");
    }

    let names: Vec<String> = repo
        .list(&user)
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.name)
        .collect();
    assert_eq!(names, vec!["first", "third", "second"]);
}

#[tokio::test]
async fn deleting_the_source_conversation_keeps_the_app_and_the_data_contract() {
    let db = nomifun_db::init_database_memory().await.expect("db");
    let user = seed_user(db.pool()).await;
    let conversation_id = ConversationId::new().as_str().to_string();
    sqlx::query(
        "INSERT INTO conversations (conversation_id, user_id, name, type, created_at, updated_at) \
         VALUES (?, ?, 'builder', 'nomi', 0, 0)",
    )
    .bind(&conversation_id)
    .bind(&user)
    .execute(db.pool())
    .await
    .expect("seed conversation");

    let repo = SqliteMiniAppRepository::new(db.pool().clone());
    let created = repo
        .create(
            &user,
            CreateMiniAppParams {
                name: "From a chat",
                description: "",
                html: "<p>hi</p>",
                source_conversation_id: Some(&conversation_id),
                ..Default::default()
            },
        )
        .await
        .expect("create");
    let id = MiniAppId::parse(created.miniapp_id.clone()).unwrap();
    assert_eq!(
        created.source_conversation_id.as_deref(),
        Some(conversation_id.as_str())
    );

    SqliteConversationRepository::new(db.pool().clone())
        .delete(&conversation_id)
        .await
        .expect("a conversation with a solidified app must still be deletable");

    // The app is a finished artifact: it outlives its build log, keeps its
    // document, and only forgets where it came from.
    let survivor = repo
        .find(&user, &id)
        .await
        .unwrap()
        .expect("the app must outlive its source conversation");
    assert_eq!(survivor.source_conversation_id, None);
    assert_eq!(
        repo.find_by_id_any_owner(&id).await.unwrap().unwrap().html,
        "<p>hi</p>"
    );

    // The registry declares SetNull; the audit requires a parent for every
    // non-null value, so a dangling id here would fail the boot data contract.
    nomifun_db::validate_id_data_contract(db.pool())
        .await
        .expect("the id data contract must still hold after the source is gone");
}
