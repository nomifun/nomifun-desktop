use super::*;

#[tokio::test]
async fn diagnostics_observe_admission_and_transaction_without_changing_pool_or_query_behavior() {
    let pool = sqlx::sqlite::SqlitePoolOptions::new().max_connections(1)
        .acquire_timeout(Duration::from_millis(10)).connect("sqlite::memory:").await.unwrap();
    let observed = DatabaseOperationGuard::new(&pool);
    let id = observed.0.id;
    let mut transaction = pool.begin().await.unwrap();
    observed.acquired();
    assert!(matches!(pool.acquire().await, Err(sqlx::Error::PoolTimedOut)));
    assert!(observed.0.phase.lock().unwrap().acquired.is_some());
    assert!(observed.0.phase.lock().unwrap().released.is_none());
    assert_eq!(sqlx::query_scalar::<_, i64>("SELECT 7").fetch_one(&mut *transaction).await.unwrap(), 7);
    transaction.commit().await.unwrap();
    observed.released();
    assert!(observed.0.phase.lock().unwrap().released.is_some());
    assert_eq!(sqlx::query_scalar::<_, i64>("SELECT 8").fetch_one(&pool).await.unwrap(), 8);
    drop(observed);
    assert!(!OPERATIONS.get().unwrap().lock().unwrap().contains_key(&id));
    assert_eq!(pool.options().get_max_connections(), 1);
    pool.close().await;
}

#[tokio::test]
async fn sqlite_writer_progress_requires_a_polled_owner_while_an_outer_operation_waits() {
    // Reproduce the resource dependency with a real five-connection SQLite
    // pool: one borrowed business future holds the writer, three peers await
    // BEGIN, and an outer scheduler operation takes the remaining connection.
    // A notification alone cannot poll the borrowed owner through COMMIT.
    let root=tempfile::tempdir().unwrap();
    let options=sqlx::sqlite::SqliteConnectOptions::new().filename(root.path().join("writers.db"))
        .create_if_missing(true).journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
        .busy_timeout(Duration::from_secs(5));
    let pool=sqlx::sqlite::SqlitePoolOptions::new().max_connections(5).connect_with(options).await.unwrap();
    sqlx::query("CREATE TABLE work(id INTEGER PRIMARY KEY)").execute(&pool).await.unwrap();
    for owned in [false,true] {
        let (entered,entry)=tokio::sync::oneshot::channel();
        let (release,released)=tokio::sync::oneshot::channel();
        let owner_pool=pool.clone();
        let owner=async move {
            let (mut tx,observed)=begin_agent_write(&owner_pool).await.unwrap();
            let phase=observed.0.clone();
            entered.send(phase).unwrap();
            released.await.unwrap();
            sqlx::query("INSERT INTO work DEFAULT VALUES").execute(&mut *tx).await.unwrap();
            observed.commit(tx).await.unwrap();
        };
        let mut borrowed=Some(Box::pin(owner));
        let (job,phase)=if owned {
            let job=tokio::spawn(borrowed.take().unwrap());
            (Some(job),entry.await.unwrap())
        }else {
            let mut entry=Box::pin(entry);
            let phase=tokio::select! {phase=&mut entry=>phase.unwrap(),_=borrowed.as_mut().unwrap()=>panic!("writer finished before release")};
            (None,phase)
        };
        assert!(phase.phase.lock().unwrap().transaction_started.is_some());
        let mut peers=Vec::new();
        for _ in 0..3 {
            let pool=pool.clone();
            peers.push(tokio::spawn(async move {let (tx,observed)=begin_agent_write(&pool).await.unwrap();observed.commit(tx).await.unwrap();}));
        }
        release.send(()).unwrap();
        let outer=begin_agent_write(&pool);tokio::pin!(outer);
        if owned {
            let (tx,observed)=tokio::time::timeout(Duration::from_secs(2),&mut outer).await.unwrap().unwrap();
            observed.commit(tx).await.unwrap();job.unwrap().await.unwrap();
        }else {
            assert!(tokio::time::timeout(Duration::from_millis(80),&mut outer).await.is_err(),
                "old scheduler shape must reproduce the stalled writer without model/provider involvement");
            assert!(phase.phase.lock().unwrap().released.is_none());
            borrowed.take().unwrap().await;
            let (tx,observed)=tokio::time::timeout(Duration::from_secs(2),&mut outer).await.unwrap().unwrap();
            observed.commit(tx).await.unwrap();
        }
        assert!(phase.phase.lock().unwrap().released.is_some());
        for peer in peers {peer.await.unwrap();}
        assert_eq!(pool.options().get_max_connections(),5);
    }
    assert_eq!(sqlx::query_scalar::<_,i64>("SELECT COUNT(*) FROM work").fetch_one(&pool).await.unwrap(),2);
    pool.close().await;
}
