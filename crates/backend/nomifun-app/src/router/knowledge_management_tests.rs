use super::*;
use nomifun_db::{Database, IKnowledgeRepository, SqliteKnowledgeRepository};
use nomifun_knowledge::{FetchedPage, KnowledgeEventEmitter, PageFetcher};
use nomifun_realtime::BroadcastEventBus;

struct Fixture {
    owner: Owner,
    directory: tempfile::TempDir,
    database: Database,
    repository: Arc<SqliteKnowledgeRepository>,
    events: Arc<BroadcastEventBus>,
}

#[derive(Clone)]
struct FixtureFetcher;
#[async_trait]
impl PageFetcher for FixtureFetcher {
    async fn fetch_page(&self, raw_url: &str) -> Result<FetchedPage, nomifun_common::AppError> {
        if raw_url.ends_with("/unavailable") {
            return Err(nomifun_common::AppError::BadRequest("fixture source unavailable".into()));
        }
        Ok(FetchedPage {
            final_url: raw_url.into(),
            title: Some("Fixture source".into()),
            markdown: format!("# Fixture source\n\nCaptured from {raw_url}."),
            truncated: false,
        })
    }
}

async fn fixture() -> Fixture {
    let database = nomifun_db::init_database_memory().await.unwrap();
    let repository = Arc::new(SqliteKnowledgeRepository::new(database.pool().clone()));
    let directory = tempfile::tempdir().unwrap();
    let events = Arc::new(BroadcastEventBus::new(256));
    let service = Arc::new(KnowledgeService::new(
        repository.clone(),
        directory.path(),
        KnowledgeEventEmitter::new(events.clone(), Arc::from("owner")),
    ).with_url_fetcher(FixtureFetcher));
    service.set_entry_repository(repository.clone());
    service.set_source_repository(repository.clone());
    service.set_tree_operation_repository(Arc::new(nomifun_db::SqliteKnowledgeTreeOperationRepository::new(database.pool().clone())));
    Fixture {
        owner: Owner { owner_id: Arc::from("owner"), service },
        directory,
        database,
        repository,
        events,
    }
}

fn principal() -> PrincipalRef {
    PrincipalRef { principal_kind: "user".into(), principal_id: "owner".into() }
}

#[test]
fn all_actions_have_concrete_schema_and_exact_host_port() {
    for action in ACTION_IDS {
        let schema = input_schema(action);
        jsonschema::validator_for(&schema).unwrap();
        assert_eq!(schema["type"], "object");
        if action != UPDATE {
            assert_eq!(schema["additionalProperties"], false);
        } else {
            for variant in schema["oneOf"].as_array().unwrap() {
                assert_eq!(variant["additionalProperties"], false);
            }
        }
    }
    let create = jsonschema::validator_for(&input_schema(CREATE)).unwrap();
    assert!(!create.is_valid(&json!({"name":"Knowledge","root_path":"C:/private"})));
    assert!(!create.is_valid(&json!({"name":"Knowledge","credentials":{"key":"not-admitted"}})));
    assert!(!create.is_valid(&json!({"name":"Knowledge","source":{"mode":"live","entries":[{"url":"https://example.test","sourceItemId":"invented"}]}})));
    let update = jsonschema::validator_for(&input_schema(UPDATE)).unwrap();
    assert!(!update.is_valid(&json!({"operation":"metadata","knowledge_base_id":"id"})));
    assert!(!update.is_valid(&json!({"operation":"refresh_sources","knowledge_base_id":"id","name":"Not allowed"})));
}

#[tokio::test]
async fn creation_materializes_registry_and_documents_without_session_binding() {
    let fixture = fixture().await;
    let mut events = fixture.events.subscribe_user();
    let created = fixture.owner.execute(&principal(), CREATE, json!({
        "name":"Research","description":"Curated notes","tags":["reference"],
        "initial_documents":[{"path":"notes/topic.md","content":"# Topic\n\nCurated facts."}]
    })).await.unwrap();
    assert_eq!(created["created"], true);
    assert_eq!(created["documents_complete"], true);
    assert_eq!(created["mounted_in_current_session"], false);
    let id = created["base"]["knowledge_base_id"].as_str().unwrap();
    let row = fixture.repository.get_base(id).await.unwrap().unwrap();
    assert!(row.managed);
    assert_eq!(row.name, "Research");
    let registered_root = std::fs::canonicalize(&row.root_path).unwrap();
    let fixture_root = std::fs::canonicalize(fixture.directory.path()).unwrap();
    assert!(registered_root.starts_with(&fixture_root), "{registered_root:?} must remain within {fixture_root:?}");
    assert_eq!(tokio::fs::read_to_string(std::path::Path::new(&row.root_path).join("notes/topic.md")).await.unwrap(), "# Topic\n\nCurated facts.");
    let inspection = fixture.owner.execute(&principal(), INSPECT, json!({"knowledge_base_id":id})).await.unwrap();
    assert_eq!(inspection["base"]["tags"], json!(["reference"]));
    assert_eq!(inspection["document_total"], 1);
    assert_eq!(inspection["documents"][0]["rel_path"], "notes/topic.md");
    let list = fixture.owner.execute(&principal(), LIST, json!({"limit":1})).await.unwrap();
    assert_eq!(list["bases"][0]["knowledge_base_id"], id);
    assert_eq!(list["total"], 1);
    assert_eq!(list["next_offset"], Value::Null);
    for table in ["agent_sessions", "knowledge_bindings"] {
        let count: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}")).fetch_one(fixture.database.pool()).await.unwrap();
        assert_eq!(count, 0, "management must not manufacture a Session or mutate a binding");
    }
    assert_eq!(events.try_recv().unwrap().user_id, "owner");
}

#[tokio::test]
async fn owner_authority_and_input_bounds_reject_before_mutation() {
    let fixture = fixture().await;
    for unauthorized in [
        PrincipalRef { principal_kind: "user".into(), principal_id: "other-owner".into() },
        PrincipalRef { principal_kind: "public".into(), principal_id: "owner".into() },
        PrincipalRef { principal_kind: "channel".into(), principal_id: "owner".into() },
    ] {
        let error = fixture.owner.execute(&unauthorized, CREATE, json!({"name":"Denied"})).await.unwrap_err();
        assert!(error.contains("installation owner"));
    }
    for path in ["../escaped.md", "/absolute.md", "C:/absolute.md", "nested\\file.md", "notes.txt"] {
        assert!(fixture.owner.execute(&principal(), CREATE, json!({"name":"Denied","initial_documents":[{"path":path,"content":"body"}]})).await.is_err());
    }
    assert!(fixture.owner.execute(&principal(), CREATE, json!({"name":"Denied","initial_documents":[{"path":"Topic.md","content":"a"},{"path":"topic.md","content":"b"}]})).await.is_err());
    assert!(fixture.owner.execute(&principal(), CREATE, json!({"name":"Denied","source":{"mode":"live","entries":[{"url":"https://user:secret@example.test/docs"}]}})).await.is_err());
    assert!(fixture.owner.execute(&principal(), CREATE, json!({"name":"Denied","description":"x".repeat(MAX_INPUT_BYTES + 1)})).await.unwrap_err().contains("1 MiB"));
    assert!(fixture.repository.list_bases().await.unwrap().is_empty());
}

#[tokio::test]
async fn metadata_and_source_updates_use_domain_services_and_report_fetch_outcomes() {
    let fixture = fixture().await;
    let created = fixture.owner.execute(&principal(), CREATE, json!({
        "name":"Source notes","tags":["web"],
        "source":{"mode":"snapshot","entries":[{"url":"https://example.test/first"}]},
        "initial_documents":[{"path":"notes.md","content":"# Notes"}]
    })).await.unwrap();
    assert_eq!(created["base"]["source_fetch"]["fetched"], 1, "tag update must preserve the real creation fetch receipt");
    let id = created["base"]["knowledge_base_id"].as_str().unwrap();
    let updated = fixture.owner.execute(&principal(), UPDATE, json!({"operation":"metadata","knowledge_base_id":id,"name":"Updated notes","description":"Revised","tags":[]})).await.unwrap();
    assert_eq!(updated["base"]["name"], "Updated notes");
    assert_eq!(updated["base"]["tags"], json!([]));
    let added = fixture.owner.execute(&principal(), UPDATE, json!({"operation":"add_sources","knowledge_base_id":id,"entries":[{"url":"https://example.test/second"},{"url":"https://example.test/unavailable"}]})).await.unwrap();
    assert_eq!(added["source_update"]["added"], 2);
    assert_eq!(added["source_update"]["fetched"], 1);
    assert_eq!(added["source_update"]["failed"], 1);
    let refreshed = fixture.owner.execute(&principal(), UPDATE, json!({"operation":"refresh_sources","knowledge_base_id":id})).await.unwrap();
    assert_eq!(refreshed["source_fetch"]["fetched"], 2);
    assert_eq!(refreshed["source_fetch"]["failed"], 1);
    let info = fixture.owner.service.get_base_info(id).await.unwrap();
    assert_eq!(info.source.unwrap().entries.len(), 3);
    assert!(fixture.owner.execute(&principal(), UPDATE, json!({"operation":"set_source","knowledge_base_id":id,"source":null})).await.unwrap_err().contains("managed document"), "clearing a source must preserve managed provenance");
}

#[tokio::test]
async fn deleting_external_registration_cannot_purge_its_files() {
    let fixture = fixture().await;
    let external = tempfile::tempdir().unwrap();
    tokio::fs::write(external.path().join("owned-by-user.md"), "Keep me").await.unwrap();
    let base = fixture.owner.service.create_base_with_access("External", "", Some(external.path().to_str().unwrap()), None, None).await.unwrap();
    let id = base.knowledge_base_id.as_str();
    let error = fixture.owner.execute(&principal(), DELETE, json!({"knowledge_base_id":id,"purge":true})).await.unwrap_err();
    assert!(error.contains("cannot purge an external"));
    assert!(fixture.repository.get_base(id).await.unwrap().is_some());
    let deleted = fixture.owner.execute(&principal(), DELETE, json!({"knowledge_base_id":id})).await.unwrap();
    assert_eq!(deleted["registration_deleted"], true);
    assert_eq!(deleted["purge_requested"], false);
    assert!(fixture.repository.get_base(id).await.unwrap().is_none());
    assert_eq!(tokio::fs::read_to_string(external.path().join("owned-by-user.md")).await.unwrap(), "Keep me");
}

#[tokio::test]
async fn failed_source_capture_returns_the_created_base_instead_of_a_bare_error() {
    let fixture = fixture().await;
    let created = fixture.owner.execute(&principal(), CREATE, json!({
        "name":"Partially captured",
        "source":{"mode":"snapshot","entries":[{"url":"https://example.test/unavailable"}]},
        "initial_documents":[{"path":"notes.md","content":"# Useful notes"}]
    })).await.unwrap();
    assert_eq!(created["created"], true);
    assert_eq!(created["completed"], false);
    assert_eq!(created["documents_complete"], true);
    assert_eq!(created["base"]["source_fetch"]["fetched"], 0);
    assert_eq!(created["base"]["source_fetch"]["failed"], 1);
    let id = created["base"]["knowledge_base_id"].as_str().unwrap();
    assert_eq!(fixture.repository.list_bases().await.unwrap().len(), 1);
    assert_eq!(fixture.owner.service.read_file(id, "notes.md").await.unwrap().content, "# Useful notes");
    let retried = fixture.owner.execute(&principal(), UPDATE, json!({"operation":"refresh_sources","knowledge_base_id":id})).await.unwrap();
    assert_eq!(retried["source_fetch"]["failed"], 1);
    assert_eq!(fixture.repository.list_bases().await.unwrap().len(), 1);
}

#[tokio::test]
async fn descriptor_declares_owner_management_effects_without_resource_mounts() {
    let fixture = fixture().await;
    let registration = registration_with_owner(Arc::new(fixture.owner)).unwrap();
    let capability = &registration.metadata.manifest.payload.contributions.capabilities[0];
    assert_eq!(capability.id.as_ref(), CAPABILITY_ID);
    assert_eq!(capability.contributions.actions.len(), ACTION_IDS.len());
    assert_eq!(capability.contributions.host_ports[0].id.as_ref(), "host.knowledge-management.invoke");
    assert_eq!(registration.metadata.context.host_ports.len(), 1);
    for action in &capability.contributions.actions {
        assert!(action.required_resource_operations.as_ref().is_some_and(|operations| operations.is_empty()));
        assert_eq!(action.effect_class, if matches!(action.action_id.as_ref(), LIST | INSPECT) { EffectClass::ReadSensitive } else { EffectClass::WriteDurable });
        let schema = input_schema(action.action_id.as_ref());
        assert_eq!(action.input_schema, schema_ref(action.action_id.as_ref(), "input", &schema));
    }
}
