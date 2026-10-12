//! Real-engine desktop browser acceptance fixture. Loopback-only, disposable
//! documents; actual Office routes/service/admission are unchanged.
//! NOMIFUN_OFFICECLI_PATH=/verified/native/officecli cargo run -p nomifun-office --example preview_browser_fixture
use axum::{Router, Extension, Json, extract::State, routing::{get, post}};
use nomifun_auth::CurrentUser;
use nomifun_common::UserId;
use nomifun_office::{OfficePreviewService, OfficeRouterState, SnapshotService, office_routes, office_artifact_routes};
use nomifun_runtime::{ToolId, VerifiedTool};
use nomi_process_runtime::ChildProcessBuilder;
use serde::{Deserialize, Serialize};
use std::{path::{Path, PathBuf}, sync::Arc};
use tokio::sync::Mutex;

#[derive(Clone)]
struct Fixture { root: PathBuf, engine: VerifiedTool, version: Arc<Mutex<u32>> }
#[derive(Serialize)]
struct Info { ppt: String, word: String, excel: String, workspace: String, engine_version: String, cache_entries: usize }
#[derive(Deserialize)]
struct Update { doc_type: String }
async fn command(engine: &VerifiedTool, args: &[&str]) -> Result<(), String> {
    let mut command = ChildProcessBuilder::clean_cli(&engine.executable_path);
    command.args(args).env("OFFICECLI_NO_AUTO_RESIDENT", "1");
    let output = command.output().await.map_err(|error| error.to_string())?;
    if output.status.success() { Ok(()) } else { Err(String::from_utf8_lossy(&output.stderr).into_owned()) }
}
fn document(root: &Path, kind: &str) -> PathBuf {
    root.join(match kind { "ppt" => "acceptance.pptx", "word" => "acceptance.docx", _ => "acceptance.xlsx" })
}
async fn populate(fixture: &Fixture) -> Result<(), String> {
    let word = document(&fixture.root, "word"); let word = word.to_str().unwrap();
    command(&fixture.engine, &["create", word, "--locale", "en-US"]).await?;
    command(&fixture.engine, &["add", word, "/body", "--type", "paragraph", "--prop", "text=Word acceptance — genuine OfficeCLI", "--prop", "style=Heading1"]).await?;
    for index in 0..20 {
        let text = format!("text=Paragraph {}: The actual managed Office engine rendered this document. Source updates retain a painted preview until the replacement is ready. This paragraph supplies enough content to inspect document scrolling.", index + 1);
        command(&fixture.engine, &["add", word, "/body", "--type", "paragraph", "--prop", &text]).await?;
    }
    let ppt = document(&fixture.root, "ppt"); let ppt = ppt.to_str().unwrap();
    command(&fixture.engine, &["create", ppt, "--locale", "en-US"]).await?;
    for index in 0..3 {
        let title = format!("title=PPT acceptance — slide {}", index + 1);
        command(&fixture.engine, &["add", ppt, "/", "--type", "slide", "--prop", &title, "--prop", "background=1A1A2E"]).await?;
        let parent = format!("/slide[{}]", index + 1);
        command(&fixture.engine, &["add", ppt, &parent, "--type", "shape", "--prop", "text=Genuine managed OfficeCLI — sandbox first paint", "--prop", "x=2cm", "--prop", "y=5cm", "--prop", "width=27cm", "--prop", "height=3cm", "--prop", "font=Arial", "--prop", "size=24", "--prop", "color=FFFFFF"]).await?;
    }
    let excel = document(&fixture.root, "excel"); let excel = excel.to_str().unwrap();
    command(&fixture.engine, &["create", excel, "--locale", "en-US"]).await?;
    command(&fixture.engine, &["set", excel, "/Sheet1/A1", "--prop", "value=Excel acceptance — genuine OfficeCLI"]).await?;
    command(&fixture.engine, &["set", excel, "/Sheet1/B2", "--prop", "value=42"]).await?;
    command(&fixture.engine, &["set", excel, "/Sheet1/C2", "--prop", "formula=B2*2"]).await?;
    Ok(())
}
async fn info(State(fixture): State<Fixture>) -> Json<Info> {
    let entries = std::fs::read_dir(fixture.root.join("office-preview-cache/artifacts")).map(|items| items.filter_map(Result::ok).count()).unwrap_or(0);
    Json(Info { ppt: document(&fixture.root,"ppt").display().to_string(), word: document(&fixture.root,"word").display().to_string(), excel: document(&fixture.root,"excel").display().to_string(), workspace: fixture.root.display().to_string(), engine_version:fixture.engine.version, cache_entries:entries })
}
async fn update(State(fixture): State<Fixture>, Json(request): Json<Update>) -> Result<Json<u32>, (axum::http::StatusCode,String)> {
    if !matches!(request.doc_type.as_str(), "ppt"|"word"|"excel") { return Err((axum::http::StatusCode::BAD_REQUEST,"invalid fixture type".into())); }
    let mut version = fixture.version.lock().await; *version += 1;
    let path = document(&fixture.root,&request.doc_type); let path = path.to_str().unwrap();
    let target = match request.doc_type.as_str() { "ppt"=>"/slide[1]/shape[1]", "word"=>"/body/p[1]", _=>"/Sheet1/A1" };
    let property = format!("{}=Updated {} acceptance generation {}", if request.doc_type=="excel" {"value"} else {"text"},request.doc_type,*version);
    command(&fixture.engine,&["set",path,target,"--prop",&property]).await.map_err(|error|(axum::http::StatusCode::INTERNAL_SERVER_ERROR,error))?;
    Ok(Json(*version))
}
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = std::fs::canonicalize(directory.path())?;
    nomifun_runtime::init(&root);
    let engine = nomifun_runtime::resolve_tool(ToolId::OfficeCli).await?;
    let fixture = Fixture {root:root.clone(),engine,version:Arc::new(Mutex::new(1))};
    populate(&fixture).await.map_err(std::io::Error::other)?;
    let state = OfficeRouterState {preview_service:Arc::new(OfficePreviewService::new(&root)),snapshot_service:Arc::new(SnapshotService::new(&root)),allowed_roots:vec![root]};
    let routes = office_routes(state.clone()).layer(Extension(CurrentUser {id:UserId::parse("01900000-0000-7000-8000-000000000001".to_owned()).unwrap(),username:"isolated-browser-fixture".into()}));
    let app = Router::new().merge(routes).merge(office_artifact_routes(state)).merge(Router::new().route("/api/office-browser-fixture/info",get(info)).route("/api/office-browser-fixture/update",post(update)).with_state(fixture));
    let port = std::env::var("NOMIFUN_OFFICE_FIXTURE_PORT").unwrap_or_else(|_|"17931".into()).parse::<u16>()?;
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST,port)).await?;
    println!("Office browser fixture ready: http://127.0.0.1:{port}");
    axum::serve(listener,app).with_graceful_shutdown(async {let _=tokio::signal::ctrl_c().await;}).await?;
    Ok(())
}
