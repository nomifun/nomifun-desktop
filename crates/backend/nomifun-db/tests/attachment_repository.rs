use nomifun_db::models::AttachmentRow;
use nomifun_db::{IAttachmentRepository, SqliteAttachmentRepository, init_database_memory};
use nomifun_common::RequirementId;

fn row(id: &str, requirement_id: &str, name: &str) -> AttachmentRow {
    AttachmentRow {
        id: id.into(),
        requirement_id: requirement_id.to_owned(),
        file_name: name.into(),
        rel_path: format!("attachments/{requirement_id}/{id}.png"),
        mime: "image/png".into(),
        size_bytes: 123,
        created_by: Some("user".into()),
        created_at: 1,
    }
}

/// Insert a minimal `requirements` row so attachment FK
/// (`attachments.requirement_id → requirements(id)`) is satisfiable. The
async fn seed_requirement(pool: &sqlx::SqlitePool, id: &str) {
    sqlx::query(
        "INSERT INTO requirements (id, title, tag, created_at, updated_at) \
         VALUES (?, 'Req', 'default', 0, 0)",
    )
    .bind(id)
    .execute(pool)
    .await
    .unwrap();
}

#[tokio::test]
async fn insert_list_get_delete_roundtrip() {
    let db = init_database_memory().await.unwrap();
    let repo = SqliteAttachmentRepository::new(db.pool().clone());

    let requirement_1 = RequirementId::new().into_string();
    let requirement_2 = RequirementId::new().into_string();
    let requirement_3 = RequirementId::new().into_string();
    seed_requirement(db.pool(), &requirement_1).await;
    seed_requirement(db.pool(), &requirement_2).await;

    repo.insert(&row("att_1", &requirement_1, "one.png")).await.unwrap();
    repo.insert(&row("att_2", &requirement_1, "two.png")).await.unwrap();
    repo.insert(&row("att_3", &requirement_2, "other.png")).await.unwrap();

    let listed = repo.list_for_requirement(&requirement_1).await.unwrap();
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0].id, "att_1", "oldest first");
    assert_eq!(listed[1].id, "att_2");

    let got = repo.get_by_id("att_1").await.unwrap().expect("att_1 exists");
    assert_eq!(got.file_name, "one.png");
    assert_eq!(
        got.rel_path,
        format!("attachments/{requirement_1}/att_1.png")
    );

    assert!(repo.delete("att_1").await.unwrap());
    assert!(!repo.delete("att_1").await.unwrap(), "second delete is a no-op");
    assert!(repo.get_by_id("att_1").await.unwrap().is_none());
    assert_eq!(repo.list_for_requirement(&requirement_1).await.unwrap().len(), 1);

    // a requirement with no attachments returns nothing
    seed_requirement(db.pool(), &requirement_3).await;
    assert!(
        repo.list_for_requirement(&requirement_3)
            .await
            .unwrap()
            .is_empty()
    );
}
