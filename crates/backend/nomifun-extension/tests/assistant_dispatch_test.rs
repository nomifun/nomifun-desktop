//! Integration coverage for the `/api/skills/assistant-rule/*` and
//! `/api/skills/assistant-skill/*` source dispatch introduced by T1b.
//!
//! Exercises the three dispatch paths (builtin → assets, extension → empty,
//! user → writable dir) using a fake [`AssistantRuleDispatcher`] that
//! captures call inputs so we can assert the handler routed through it.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use nomifun_api_types::{ApiResponse, AssistantSource};
use nomifun_common::AppError;
use nomifun_extension::classifier::{AssistantClassifier, AssistantRuleDispatcher};
use nomifun_extension::external_paths::ExternalPathsManager;
use nomifun_extension::skill_routes::{SkillRouterState, skill_routes};
use nomifun_extension::skill_service::SkillPaths;
use tower::ServiceExt;

// ---------------------------------------------------------------------------
// Fake dispatcher
// ---------------------------------------------------------------------------

#[derive(Default)]
struct CallLog {
    rule_reads: Vec<(String, Option<String>)>,
    rule_writes: Vec<(String, Option<String>, String)>,
    rule_deletes: Vec<String>,
    skill_reads: Vec<(String, Option<String>)>,
    skill_writes: Vec<(String, Option<String>, String)>,
    skill_deletes: Vec<String>,
}

struct FakeDispatcher {
    // Pre-seeded responses by id.
    rule_content: std::collections::HashMap<String, String>,
    skill_content: std::collections::HashMap<String, String>,
    // Ids that should be treated as builtin (write rejects with 400).
    reject_writes_for: std::collections::HashSet<String>,
    log: Mutex<CallLog>,
}

#[async_trait::async_trait]
impl AssistantClassifier for FakeDispatcher {
    async fn classify(&self, id: &str) -> AssistantSource {
        if self.reject_writes_for.contains(id) {
            AssistantSource::Builtin
        } else {
            AssistantSource::User
        }
    }
}

#[async_trait::async_trait]
impl AssistantRuleDispatcher for FakeDispatcher {
    async fn read_rule(&self, id: &str, locale: Option<&str>) -> Result<String, AppError> {
        self.log
            .lock()
            .unwrap()
            .rule_reads
            .push((id.to_string(), locale.map(str::to_string)));
        Ok(self.rule_content.get(id).cloned().unwrap_or_default())
    }

    async fn write_rule(&self, id: &str, locale: Option<&str>, content: &str) -> Result<(), AppError> {
        if self.reject_writes_for.contains(id) {
            return Err(AppError::BadRequest("Cannot write rule for built-in assistant".into()));
        }
        self.log
            .lock()
            .unwrap()
            .rule_writes
            .push((id.to_string(), locale.map(str::to_string), content.to_string()));
        Ok(())
    }

    async fn delete_rule(&self, id: &str) -> Result<bool, AppError> {
        if self.reject_writes_for.contains(id) {
            return Err(AppError::BadRequest("Cannot delete rule for built-in assistant".into()));
        }
        self.log.lock().unwrap().rule_deletes.push(id.to_string());
        Ok(true)
    }

    async fn read_skill(&self, id: &str, locale: Option<&str>) -> Result<String, AppError> {
        self.log
            .lock()
            .unwrap()
            .skill_reads
            .push((id.to_string(), locale.map(str::to_string)));
        Ok(self.skill_content.get(id).cloned().unwrap_or_default())
    }

    async fn write_skill(&self, id: &str, locale: Option<&str>, content: &str) -> Result<(), AppError> {
        if self.reject_writes_for.contains(id) {
            return Err(AppError::BadRequest("Cannot write skill for built-in assistant".into()));
        }
        self.log
            .lock()
            .unwrap()
            .skill_writes
            .push((id.to_string(), locale.map(str::to_string), content.to_string()));
        Ok(())
    }

    async fn delete_skill(&self, id: &str) -> Result<bool, AppError> {
        if self.reject_writes_for.contains(id) {
            return Err(AppError::BadRequest(
                "Cannot delete skill for built-in assistant".into(),
            ));
        }
        self.log.lock().unwrap().skill_deletes.push(id.to_string());
        Ok(true)
    }
}

// ---------------------------------------------------------------------------
// Router fixture
// ---------------------------------------------------------------------------

async fn router_with_dispatcher(dispatcher: Arc<FakeDispatcher>) -> axum::Router {
    let tmp = tempfile::TempDir::new().unwrap();
    let root: PathBuf = tmp.path().to_path_buf();
    let paths = SkillPaths {
        data_dir: root.clone(),
        user_skills_dir: root.join("skills"),
        cron_skills_dir: root.join("cron").join("skills"),
        builtin_skills_dir: root.join("builtin-skills"),
        builtin_rules_dir: root.join("builtin-rules"),
        assistant_rules_dir: root.join("assistant-rules"),
        assistant_skills_dir: root.join("assistant-skills"),
    };
    let ext_mgr = Arc::new(ExternalPathsManager::with_file(root.join("paths.json")).await);
    std::mem::forget(tmp);

    let state = SkillRouterState {
        skill_paths: paths,
        external_paths_manager: ext_mgr,
        assistant_dispatcher: Some(dispatcher),
        skill_tag_repo: {
            let db = nomifun_db::init_database_memory().await.unwrap();
            Arc::new(nomifun_db::SqliteSkillTagRepository::new(db.pool().clone()))
        },
        builtin_skill_tags: Arc::new(std::collections::HashMap::new()),
    };
    skill_routes(state)
}

async fn body_json<T: serde::de::DeserializeOwned>(resp: axum::response::Response) -> T {
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&body).unwrap()
}

/// Construct the request body for rule/skill read using the wire field names.
fn read_body(assistant_id: &str, locale: Option<&str>) -> Vec<u8> {
    let body = match locale {
        Some(loc) => serde_json::json!({ "assistant_id": assistant_id, "locale": loc }),
        None => serde_json::json!({ "assistant_id": assistant_id }),
    };
    serde_json::to_vec(&body).unwrap()
}

fn write_body(assistant_id: &str, content: &str, locale: Option<&str>) -> Vec<u8> {
    let body = match locale {
        Some(loc) => serde_json::json!({
            "assistant_id": assistant_id,
            "content": content,
            "locale": loc,
        }),
        None => serde_json::json!({
            "assistant_id": assistant_id,
            "content": content,
        }),
    };
    serde_json::to_vec(&body).unwrap()
}

// ---------------------------------------------------------------------------
// Test cases
// ---------------------------------------------------------------------------

#[tokio::test]
async fn read_rule_routes_through_dispatcher_for_builtin() {
    let mut rule_content = std::collections::HashMap::new();
    rule_content.insert("builtin-office".into(), "office rule body".into());
    let dispatcher = Arc::new(FakeDispatcher {
        rule_content,
        skill_content: Default::default(),
        reject_writes_for: Default::default(),
        log: Mutex::new(CallLog::default()),
    });
    let router = router_with_dispatcher(dispatcher.clone()).await;

    let req_body = read_body("builtin-office", Some("en-US"));
    let req = Request::builder()
        .method("POST")
        .uri("/api/skills/assistant-rule/read")
        .header("content-type", "application/json")
        .body(Body::from(req_body))
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body: ApiResponse<String> = body_json(resp).await;
    assert_eq!(body.data.unwrap(), "office rule body");

    let log = dispatcher.log.lock().unwrap();
    assert_eq!(log.rule_reads.len(), 1);
    assert_eq!(log.rule_reads[0].0, "builtin-office");
    assert_eq!(log.rule_reads[0].1.as_deref(), Some("en-US"));
}

#[tokio::test]
async fn read_rule_routes_through_dispatcher_for_user() {
    // Classification returns User by default (not in reject set).
    let dispatcher = Arc::new(FakeDispatcher {
        rule_content: std::collections::HashMap::from([("u1".into(), "user body".into())]),
        skill_content: Default::default(),
        reject_writes_for: Default::default(),
        log: Mutex::new(CallLog::default()),
    });
    let router = router_with_dispatcher(dispatcher.clone()).await;

    let req_body = read_body("u1", None);
    let req = Request::builder()
        .method("POST")
        .uri("/api/skills/assistant-rule/read")
        .header("content-type", "application/json")
        .body(Body::from(req_body))
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body: ApiResponse<String> = body_json(resp).await;
    assert_eq!(body.data.unwrap(), "user body");
}

#[tokio::test]
async fn read_rule_routes_through_dispatcher_for_extension_returns_empty() {
    let dispatcher = Arc::new(FakeDispatcher {
        rule_content: Default::default(), // no content for any id
        skill_content: Default::default(),
        reject_writes_for: Default::default(),
        log: Mutex::new(CallLog::default()),
    });
    let router = router_with_dispatcher(dispatcher).await;

    let req_body = read_body("ext-assistant", Some("en-US"));
    let req = Request::builder()
        .method("POST")
        .uri("/api/skills/assistant-rule/read")
        .header("content-type", "application/json")
        .body(Body::from(req_body))
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: ApiResponse<String> = body_json(resp).await;
    assert_eq!(body.data.unwrap(), "");
}

#[tokio::test]
async fn write_rule_rejects_builtin() {
    let mut reject = std::collections::HashSet::new();
    reject.insert("builtin-office".to_string());
    let dispatcher = Arc::new(FakeDispatcher {
        rule_content: Default::default(),
        skill_content: Default::default(),
        reject_writes_for: reject,
        log: Mutex::new(CallLog::default()),
    });
    let router = router_with_dispatcher(dispatcher).await;

    let req_body = write_body("builtin-office", "hack", None);
    let req = Request::builder()
        .method("POST")
        .uri("/api/skills/assistant-rule/write")
        .header("content-type", "application/json")
        .body(Body::from(req_body))
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn write_rule_allows_user() {
    let dispatcher = Arc::new(FakeDispatcher {
        rule_content: Default::default(),
        skill_content: Default::default(),
        reject_writes_for: Default::default(),
        log: Mutex::new(CallLog::default()),
    });
    let router = router_with_dispatcher(dispatcher.clone()).await;

    let req_body = write_body("u1", "rule!", Some("en-US"));
    let req = Request::builder()
        .method("POST")
        .uri("/api/skills/assistant-rule/write")
        .header("content-type", "application/json")
        .body(Body::from(req_body))
        .unwrap();

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body: ApiResponse<bool> = body_json(resp).await;
    assert!(body.data.unwrap());

    let log = dispatcher.log.lock().unwrap();
    assert_eq!(log.rule_writes.len(), 1);
    assert_eq!(log.rule_writes[0].2, "rule!");
}

#[tokio::test]
async fn delete_rule_rejects_builtin() {
    let mut reject = std::collections::HashSet::new();
    reject.insert("builtin-office".to_string());
    let dispatcher = Arc::new(FakeDispatcher {
        rule_content: Default::default(),
        skill_content: Default::default(),
        reject_writes_for: reject,
        log: Mutex::new(CallLog::default()),
    });
    let router = router_with_dispatcher(dispatcher).await;

    let req = Request::builder()
        .method("DELETE")
        .uri("/api/skills/assistant-rule/builtin-office")
        .body(Body::empty())
        .unwrap();
    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn delete_rule_user_dispatches() {
    let dispatcher = Arc::new(FakeDispatcher {
        rule_content: Default::default(),
        skill_content: Default::default(),
        reject_writes_for: Default::default(),
        log: Mutex::new(CallLog::default()),
    });
    let router = router_with_dispatcher(dispatcher.clone()).await;

    let req = Request::builder()
        .method("DELETE")
        .uri("/api/skills/assistant-rule/u1")
        .body(Body::empty())
        .unwrap();
    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let log = dispatcher.log.lock().unwrap();
    assert_eq!(log.rule_deletes, vec!["u1".to_string()]);
}

#[tokio::test]
async fn read_skill_routes_through_dispatcher_for_builtin() {
    let dispatcher = Arc::new(FakeDispatcher {
        rule_content: Default::default(),
        skill_content: std::collections::HashMap::from([("builtin-office".into(), "skill body".into())]),
        reject_writes_for: Default::default(),
        log: Mutex::new(CallLog::default()),
    });
    let router = router_with_dispatcher(dispatcher.clone()).await;

    let req_body = read_body("builtin-office", Some("en-US"));
    let req = Request::builder()
        .method("POST")
        .uri("/api/skills/assistant-skill/read")
        .header("content-type", "application/json")
        .body(Body::from(req_body))
        .unwrap();
    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body: ApiResponse<String> = body_json(resp).await;
    assert_eq!(body.data.unwrap(), "skill body");

    let log = dispatcher.log.lock().unwrap();
    assert_eq!(log.skill_reads.len(), 1);
}

#[tokio::test]
async fn write_skill_rejects_builtin() {
    let mut reject = std::collections::HashSet::new();
    reject.insert("builtin-office".to_string());
    let dispatcher = Arc::new(FakeDispatcher {
        rule_content: Default::default(),
        skill_content: Default::default(),
        reject_writes_for: reject,
        log: Mutex::new(CallLog::default()),
    });
    let router = router_with_dispatcher(dispatcher).await;

    let req_body = write_body("builtin-office", "x", None);
    let req = Request::builder()
        .method("POST")
        .uri("/api/skills/assistant-skill/write")
        .header("content-type", "application/json")
        .body(Body::from(req_body))
        .unwrap();
    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn delete_skill_user_dispatches() {
    let dispatcher = Arc::new(FakeDispatcher {
        rule_content: Default::default(),
        skill_content: Default::default(),
        reject_writes_for: Default::default(),
        log: Mutex::new(CallLog::default()),
    });
    let router = router_with_dispatcher(dispatcher.clone()).await;

    let req = Request::builder()
        .method("DELETE")
        .uri("/api/skills/assistant-skill/u1")
        .body(Body::empty())
        .unwrap();
    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let log = dispatcher.log.lock().unwrap();
    assert_eq!(log.skill_deletes, vec!["u1".to_string()]);
}
