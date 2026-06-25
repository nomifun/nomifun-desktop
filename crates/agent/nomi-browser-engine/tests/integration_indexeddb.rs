//! **IndexedDB storage_state 捕获端到端集成**（`#[ignore]`，需 `NOMIFUN_CHROME_BINARY`）。
//!
//! 验证 IndexedDB 完整序列化（含二进制值 base64 哨兵编码）的 capture 链：
//! - 在真实 origin 页面写 IndexedDB 记录（含 ArrayBuffer 二进制值）→ `capture_index_db` 收集。
//!
//! 手动跑：
//!   NOMIFUN_CHROME_BINARY="/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" \
//!   cargo nextest run -p nomi-browser-engine --run-ignored all -E 'test(capture_index_db)'

mod common;

use nomi_browser_engine::BrowserEngine;

const ORIGIN: &str = "https://example.com";

/// **capture_index_db_collects_records**: 在 example.com 写 2 条 IDB 记录（一普通/一二进制）→
/// capture 返回 IndexedDbDump 含这 2 条记录 + 正确的 db name/version/store name/keyPath。
#[tokio::test]
#[ignore = "需 NOMIFUN_CHROME_BINARY（真 Chrome）：IndexedDB capture 集成"]
async fn capture_index_db_collects_records() {
    let backend = common::build_backend_for_fixture("idb-cap").await;
    backend
        .navigate(ORIGIN, false)
        .await
        .expect("navigate to example.com");

    // Inject IDB with 2 records (one with binary ArrayBuffer).
    let seed_script = r#"(async () => {
        const req = indexedDB.open("testdb", 2);
        req.onupgradeneeded = (e) => {
            const db = e.target.result;
            if (!db.objectStoreNames.contains("items")) {
                db.createObjectStore("items", { keyPath: "id" });
            }
        };
        const db = await new Promise((resolve, reject) => {
            req.onsuccess = () => resolve(req.result);
            req.onerror = () => reject(req.error);
        });
        const tx = db.transaction("items", "readwrite");
        const store = tx.objectStore("items");
        store.put({ id: 1, name: "hello", tags: ["a", "b"] });
        store.put({ id: 2, data: new Uint8Array([0xCA, 0xFE, 0xBA, 0xBE]).buffer });
        await new Promise((resolve, reject) => {
            tx.oncomplete = resolve;
            tx.onerror = () => reject(tx.error);
        });
        db.close();
        return "seeded";
    })()"#;
    let r = backend
        .__eval_page_world_await_for_test(seed_script)
        .await
        .expect("seed IDB");
    eprintln!("=== IDB seed result: {r:?}");

    // Capture IndexedDB.
    let dump = backend
        .capture_index_db()
        .await
        .expect("capture_index_db must succeed");
    let dump = dump.expect("IndexedDB dump must be Some on https://example.com");

    eprintln!("=== capture_index_db result: {dump:?}");

    assert!(!dump.databases.is_empty(), "must capture at least 1 database");
    let db = dump
        .databases
        .iter()
        .find(|d| d.name == "testdb")
        .expect("testdb found");
    assert_eq!(db.version, 2);
    assert!(!db.stores.is_empty(), "must have stores");
    let store = db
        .stores
        .iter()
        .find(|s| s.name == "items")
        .expect("items store");
    assert_eq!(store.key_path.as_deref(), Some("id"));
    assert_eq!(store.records.len(), 2, "must have 2 records");

    // Verify first record content.
    let rec1 = store
        .records
        .iter()
        .find(|r| r.get("id") == Some(&serde_json::json!(1)));
    assert!(rec1.is_some(), "record with id=1 must exist");
    let rec1 = rec1.unwrap();
    assert_eq!(rec1.get("name"), Some(&serde_json::json!("hello")));

    // Verify binary record contains __b64__ sentinel.
    let rec2 = store
        .records
        .iter()
        .find(|r| r.get("id") == Some(&serde_json::json!(2)));
    assert!(rec2.is_some(), "record with id=2 must exist");
    let rec2 = rec2.unwrap();
    let data_field = rec2.get("data").expect("data field in record 2");
    // Should be a base64 sentinel: {"__b64__": "..."}
    assert!(
        data_field.get("__b64__").is_some(),
        "binary field must be encoded as __b64__ sentinel, got: {data_field}"
    );
    // Decode and verify bytes.
    let decoded =
        nomi_browser_engine::decode_binary_sentinel(data_field).expect("decode base64 sentinel");
    assert_eq!(decoded, vec![0xCA, 0xFE, 0xBA, 0xBE], "binary must round-trip");

    eprintln!("=== PASS: capture_index_db_collects_records");
}

/// **restore_index_db_writes_back**: build a dump, restore to a new engine, verify records present.
#[tokio::test]
#[ignore = "需 NOMIFUN_CHROME_BINARY（真 Chrome）：IndexedDB restore 集成"]
async fn restore_index_db_writes_back() {
    use nomi_browser_engine::storage_state::{
        IdbDatabase, IdbStore, IndexedDbDump, OriginStorage, StorageState,
    };
    use nomi_browser_engine::encode_binary_sentinel;

    // Build a StorageState with IndexedDB dump to restore.
    let dump = IndexedDbDump {
        databases: vec![IdbDatabase {
            name: "restoredb".into(),
            version: 1,
            stores: vec![IdbStore {
                name: "docs".into(),
                key_path: Some("id".into()),
                auto_increment: false,
                records: vec![
                    serde_json::json!({"id": "doc1", "title": "First"}),
                    serde_json::json!({"id": "doc2", "payload": encode_binary_sentinel(&[0xDE, 0xAD])}),
                ],
            }],
        }],
    };
    let state = StorageState {
        cookies: vec![],
        local_storage: vec![OriginStorage {
            origin: ORIGIN.into(),
            local_storage: vec![],
            index_db: Some(dump),
        }],
    };

    // Engine: navigate to the origin, restore IndexedDB, then read back.
    let backend = common::build_backend_for_fixture("idb-restore").await;
    backend.navigate(ORIGIN, false).await.expect("navigate");

    // Restore IndexedDB for this origin.
    backend
        .restore_index_db(&state)
        .await
        .expect("restore_index_db must succeed");

    // Verify: read back the records from IndexedDB via page eval.
    let verify_script = r#"(async () => {
        const req = indexedDB.open("restoredb", 1);
        const db = await new Promise((resolve, reject) => {
            req.onsuccess = () => resolve(req.result);
            req.onerror = () => reject(req.error);
        });
        const tx = db.transaction("docs", "readonly");
        const store = tx.objectStore("docs");
        const all = await new Promise((resolve, reject) => {
            const r = store.getAll();
            r.onsuccess = () => resolve(r.result);
            r.onerror = () => reject(r.error);
        });
        db.close();
        return all.map(rec => {
            const out = {...rec};
            if (out.payload instanceof ArrayBuffer) {
                out.payload = Array.from(new Uint8Array(out.payload));
            }
            return out;
        });
    })()"#;
    let result = backend
        .__eval_page_world_await_for_test(verify_script)
        .await
        .expect("verify IDB");
    let value = result.get("value").cloned().unwrap_or(serde_json::Value::Null);
    let records = value.as_array().expect("should return array of records");
    assert_eq!(records.len(), 2, "restored 2 records");

    let doc1 = records
        .iter()
        .find(|r| r.get("id") == Some(&serde_json::json!("doc1")));
    assert!(doc1.is_some(), "doc1 must exist");
    assert_eq!(
        doc1.unwrap().get("title"),
        Some(&serde_json::json!("First"))
    );

    let doc2 = records
        .iter()
        .find(|r| r.get("id") == Some(&serde_json::json!("doc2")));
    assert!(doc2.is_some(), "doc2 must exist");
    // Binary payload: restored as ArrayBuffer → we converted to array [0xDE, 0xAD].
    let payload = doc2.unwrap().get("payload").expect("payload field");
    let bytes: Vec<u8> = payload
        .as_array()
        .expect("array")
        .iter()
        .map(|v| v.as_u64().unwrap() as u8)
        .collect();
    assert_eq!(bytes, vec![0xDE, 0xAD], "binary payload must round-trip");

    eprintln!("=== PASS: restore_index_db_writes_back");
}
