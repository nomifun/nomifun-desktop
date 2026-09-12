use super::*;

async fn seed() -> (SqliteConversationRepository, nomifun_db::Database, String) {
    let (repo, db) = setup().await;
    let conversation_id = repo
        .create(&make_conversation("pagination-boundary"))
        .await
        .unwrap();
    repo.insert_message(&make_message(&conversation_id, "boundary"))
        .await
        .unwrap();
    (repo, db, conversation_id)
}

#[tokio::test]
async fn offset_message_pages_do_not_overflow_or_wrap_to_the_first_page() {
    let (repo, _db, conversation_id) = seed().await;
    for (page, size, expected) in [
        (0, 0, 1),
        (1, u32::MAX, 1),
        (u32::MAX, 2, 0),
        (u32::MAX, u32::MAX, 0),
    ] {
        let result = repo
            .get_messages(&conversation_id, page, size, SortOrder::Asc)
            .await
            .unwrap();
        assert_eq!(result.items.len(), expected, "page={page}, size={size}");
        assert_eq!(result.total, 1);
        assert!(!result.has_more);
    }
}

#[tokio::test]
async fn offset_search_pages_do_not_overflow_or_wrap_to_the_first_page() {
    let (repo, _db, _conversation_id) = seed().await;
    for (page, size, expected) in [
        (0, 0, 1),
        (1, u32::MAX, 1),
        (u32::MAX, 2, 0),
        (u32::MAX, u32::MAX, 0),
    ] {
        let result = repo
            .search_messages(USER_ID, "boundary", page, size)
            .await
            .unwrap();
        assert_eq!(result.items.len(), expected, "page={page}, size={size}");
        assert_eq!(result.total, 1);
        assert!(!result.has_more);
    }
}

#[tokio::test]
async fn conversation_list_accepts_the_full_u32_limit_without_overflow() {
    let (repo, _db, _) = seed().await;
    let result = repo
        .list_paginated(
            USER_ID,
            &ConversationFilters {
                limit: u32::MAX,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(result.items.len(), 1);
    assert!(!result.has_more);
}

#[tokio::test]
async fn offset_message_page_product_is_computed_without_u32_overflow() {
    let (repo, _db, conversation_id) = seed().await;
    let result = repo
        .get_messages(&conversation_id, u32::MAX, 2, SortOrder::Asc)
        .await
        .unwrap();
    assert!(result.items.is_empty());
    assert_eq!(result.total, 1);
}

#[tokio::test]
async fn offset_search_page_product_is_computed_without_u32_overflow() {
    let (repo, _db, _) = seed().await;
    let result = repo
        .search_messages(USER_ID, "boundary", u32::MAX, 2)
        .await
        .unwrap();
    assert!(result.items.is_empty());
    assert_eq!(result.total, 1);
}

#[tokio::test]
async fn keyset_message_page_accepts_the_full_u32_limit_without_overflow() {
    let (repo, _db, conversation_id) = seed().await;
    let result = repo
        .get_messages_keyset(&conversation_id, None, u32::MAX)
        .await
        .unwrap();
    assert_eq!(result.items.len(), 1);
    assert_eq!(result.total, 0);
    assert!(!result.has_more);
}

#[tokio::test]
async fn local_day_message_page_accepts_the_full_u32_limit_without_overflow() {
    let (repo, _db, conversation_id) = seed().await;
    let days = repo
        .message_local_day_index(&conversation_id)
        .await
        .unwrap();
    let result = repo
        .get_messages_for_local_day(&conversation_id, &days[0].day, u32::MAX)
        .await
        .unwrap();
    assert_eq!(result.items.len(), 1);
    assert_eq!(result.total, 1);
    assert!(!result.has_more);
}

#[tokio::test]
async fn keyset_tiebreaker_has_no_gaps_or_duplicates_when_new_messages_arrive() {
    let (repo, _db) = setup().await;
    let conversation_id = repo
        .create(&make_conversation("keyset-ties"))
        .await
        .unwrap();
    let mut ids = Vec::new();
    for _ in 0..4 {
        let mut row = make_message(&conversation_id, "same timestamp");
        row.created_at = 1000;
        ids.push(row.message_id.clone());
        repo.insert_message(&row).await.unwrap();
    }
    ids.sort();
    ids.reverse();
    let newest = repo
        .get_messages_keyset(&conversation_id, None, 2)
        .await
        .unwrap();
    assert!(newest.has_more);
    assert_eq!(
        newest
            .items
            .iter()
            .map(|row| &row.message_id)
            .collect::<Vec<_>>(),
        ids[..2].iter().collect::<Vec<_>>()
    );
    let oldest = newest.items.last().unwrap();
    let mut inserted = make_message(&conversation_id, "arrived between pages");
    inserted.created_at = 2000;
    repo.insert_message(&inserted).await.unwrap();
    let older = repo
        .get_messages_keyset(
            &conversation_id,
            Some((oldest.created_at, oldest.message_id.clone())),
            2,
        )
        .await
        .unwrap();
    assert!(!older.has_more);
    assert_eq!(
        older
            .items
            .iter()
            .map(|row| &row.message_id)
            .collect::<Vec<_>>(),
        ids[2..].iter().collect::<Vec<_>>()
    );
}
