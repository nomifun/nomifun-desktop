//! End-to-end: crawl a mock site and assert the pages land as real files
//! inside the knowledge base root on disk.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use nomifun_common::{CrawlJobId, UserId, now_ms};
use nomifun_crawl::claim;
use nomifun_crawl::events::NoopEvents;
use nomifun_crawl::executor::LocalExecutor;
use nomifun_crawl::fetcher::HttpCrawlFetcher;
use nomifun_crawl::frontier::{self, ScopeMatcher};
use nomifun_crawl::model::{
    CrawlJob, CrawlScope, CrawlSink, DiscoveredUrl, JobStatus, RenderMode,
};
use nomifun_crawl::politeness::{HttpRobotsSource, Politeness};
use nomifun_crawl::runner::{RunnerConfig, spawn_job};
use nomifun_crawl::sink::KnowledgeSink;
use nomifun_crawl::store;
use nomifun_db::{SqliteKnowledgeRepository, init_database_memory};
use nomifun_knowledge::events::KnowledgeEventEmitter;
use nomifun_knowledge::service::KnowledgeService;
use nomifun_knowledge::source_url::HttpFetcher;
use sqlx::SqlitePool;
use tokio_util::sync::CancellationToken;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const UA: &str = "NomiFun-Crawler/1.0 (+https://www.nomifun.com; test)";

struct NoopUserEvents;

impl nomifun_realtime::UserEventSink for NoopUserEvents {
    fn send_to_user(
        &self,
        _user_id: &str,
        _event: nomifun_api_types::WebSocketMessage<serde_json::Value>,
    ) {
    }
}

fn page_html(title: &str, body: &str, links: &[&str]) -> String {
    let anchors: String = links
        .iter()
        .map(|href| format!("<a href=\"{href}\">{href}</a> "))
        .collect();
    format!(
        "<!doctype html><html><head><title>{title}</title></head><body>\
         <nav>site nav that readability should drop</nav>\
         <main><h1>{title}</h1><p>{body}</p><p>{body}</p><p>{anchors}</p></main>\
         </body></html>"
    )
}

/// Two linked pages plus a permissive robots.txt.
async fn mock_site() -> MockServer {
    let server = MockServer::start().await;
    let filler = "This paragraph exists so the readability extractor sees a real \
                  article body instead of a navigation shell. ";
    let long = filler.repeat(4);

    Mock::given(method("GET"))
        .and(path("/robots.txt"))
        .respond_with(ResponseTemplate::new(200).set_body_raw("User-agent: *\nAllow: /\n", "text/plain"))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            page_html("Seed Page", &long, &["/page-2"]),
            "text/html; charset=utf-8",
        ))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/page-2"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            page_html("Second Page", &long, &[]),
            "text/html; charset=utf-8",
        ))
        .mount(&server)
        .await;

    server
}

async fn knowledge_service(data_dir: &Path, pool: SqlitePool) -> Arc<KnowledgeService> {
    Arc::new(KnowledgeService::new(
        Arc::new(SqliteKnowledgeRepository::new(pool)),
        data_dir,
        KnowledgeEventEmitter::new(
            Arc::new(NoopUserEvents),
            Arc::from(UserId::new().into_string()),
        ),
    ))
}

async fn seed_user(pool: &SqlitePool) -> UserId {
    let user_id = UserId::new();
    let now = now_ms();
    sqlx::query(
        "INSERT INTO users (user_id, username, password_hash, created_at, updated_at) \
         VALUES (?, ?, 'x', ?, ?)",
    )
    .bind(user_id.as_str())
    .bind(format!("u{}", &user_id.as_str()[..8]))
    .bind(now)
    .bind(now)
    .execute(pool)
    .await
    .expect("seed user");
    user_id
}

fn job_for(user_id: &UserId, seed: &str, sink: CrawlSink) -> CrawlJob {
    let now = now_ms();
    CrawlJob {
        job_id: CrawlJobId::new(),
        user_id: user_id.clone(),
        name: "E2E Crawl".into(),
        seeds: vec![seed.to_string()],
        scope: CrawlScope::default(),
        max_depth: 2,
        max_urls: 50,
        render_mode: RenderMode::Auto,
        concurrency: 2,
        per_host_concurrency: 2,
        delay_ms: 0,
        respect_robots: true,
        user_agent: Some(UA.to_string()),
        sink,
        status: JobStatus::Draft,
        error_detail: None,
        started_at: None,
        finished_at: None,
        created_at: now,
        updated_at: now,
    }
}

/// Same wiring as `CrawlService::start`, except the fetchers allow loopback so
/// the SSRF guard does not reject the mock server.
async fn run_to_completion(pool: &SqlitePool, job: &CrawlJob, knowledge: Arc<KnowledgeService>) {
    store::create_job(pool, job).await.expect("create job");

    let url = frontier::normalize(&job.seeds[0]).expect("seed normalizes");
    let discovered = DiscoveredUrl {
        fingerprint: frontier::fingerprint(&url),
        host: url.host_str().unwrap().to_ascii_lowercase(),
        url: url.to_string(),
        depth: 0,
    };
    claim::enqueue(pool, &job.job_id, None, &discovered, 100)
        .await
        .expect("enqueue seed");
    store::start_job(pool, &job.job_id).await.expect("start job");

    let matcher = Arc::new(ScopeMatcher::build(&job.scope, &job.seeds).expect("scope"));
    let politeness = Arc::new(Politeness::new(
        Arc::new(HttpRobotsSource::new(
            HttpFetcher::new().user_agent(UA).allow_private_for_tests(),
        )),
        UA.to_string(),
        job.respect_robots,
        Duration::from_millis(job.delay_ms),
    ));
    let executor = Arc::new(LocalExecutor::new(
        Arc::new(HttpCrawlFetcher::from_fetcher(
            HttpFetcher::new().user_agent(UA).allow_private_for_tests(),
        )),
        politeness,
        Arc::new(KnowledgeSink::new(knowledge)),
        matcher,
    ));

    spawn_job(
        pool.clone(),
        job.clone(),
        executor,
        Arc::new(NoopEvents),
        RunnerConfig::default(),
        CancellationToken::new(),
    );

    for _ in 0..300 {
        let current = store::get_job(pool, &job.job_id).await.expect("job row");
        if matches!(current.status, JobStatus::Done | JobStatus::Failed | JobStatus::Cancelled) {
            dump_tasks(pool, &job.job_id).await;
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("crawl job did not finish in time");
}

/// Printed on every run: a skipped or failed task is otherwise invisible in
/// the assertion output.
async fn dump_tasks(pool: &SqlitePool, job_id: &CrawlJobId) {
    for t in claim::list_tasks(pool, job_id, None, 100).await.unwrap() {
        println!(
            "task {} {:?} http={:?} code={:?} detail={:?}",
            t.url, t.status, t.http_status, t.error_code, t.error_detail
        );
    }
}

/// Every `.md` under `root`, as paths relative to it.
fn markdown_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|e| e == "md") {
                out.push(p.strip_prefix(root).unwrap().to_path_buf());
            }
        }
    }
    out.sort();
    out
}

fn to_slashes(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

/// Mirrors `sink::document_path`'s directory: `crawl/{name-slug}-{id-tail}/`.
fn doc_dir(job_id: &CrawlJobId) -> String {
    let raw = job_id.as_str();
    format!("crawl/e2e-crawl-{}/", &raw[raw.len() - 8..])
}

/// Manual inspection: keeps the knowledge base on disk instead of a temp dir
/// that vanishes with the test.
///
/// ```text
/// CRAWL_E2E_OUT=<dir> CRAWL_E2E_SEED=<url> \
///   cargo test -p nomifun-crawl --test knowledge_sink_e2e -- --ignored --nocapture
/// ```
///
/// `CRAWL_E2E_SEED` unset crawls the built-in mock site; set it to crawl a real
/// one. `CRAWL_E2E_OUT` defaults to `<temp>/nomifun-crawl-e2e` and is wiped on
/// each run.
#[ignore = "writes to a persistent directory; run explicitly"]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn keep_crawl_output_for_manual_inspection() {
    let out_root = std::env::var("CRAWL_E2E_OUT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("nomifun-crawl-e2e"));
    let _ = std::fs::remove_dir_all(&out_root);
    let kb_root = out_root.join("knowledge-base");
    let data_dir = out_root.join("app-data");
    std::fs::create_dir_all(&kb_root).unwrap();
    std::fs::create_dir_all(&data_dir).unwrap();

    // Hold the mock alive for the whole crawl when no real seed is given.
    let mock = match std::env::var("CRAWL_E2E_SEED") {
        Ok(_) => None,
        Err(_) => Some(mock_site().await),
    };
    let seed = std::env::var("CRAWL_E2E_SEED")
        .unwrap_or_else(|_| mock.as_ref().unwrap().uri());

    let db = init_database_memory().await.unwrap();
    let user_id = seed_user(db.pool()).await;
    let knowledge = knowledge_service(&data_dir, db.pool().clone()).await;
    let base = knowledge
        .create_base("Crawl KB", "", Some(kb_root.to_str().unwrap()), None)
        .await
        .expect("create knowledge base");

    let mut job = job_for(
        &user_id,
        &seed,
        CrawlSink {
            knowledge_base_id: Some(base.knowledge_base_id.to_string()),
        },
    );
    // Real sites need the politeness delay; the mock does not care.
    if mock.is_none() {
        job.delay_ms = 1_000;
        job.max_urls = 10;
        job.max_depth = 1;
    }
    run_to_completion(db.pool(), &job, knowledge).await;

    println!("\nseed:            {seed}");
    println!("knowledge base:  {}", kb_root.display());
    for rel in markdown_files(&kb_root) {
        let abs = kb_root.join(&rel);
        let bytes = std::fs::metadata(&abs).map(|m| m.len()).unwrap_or(0);
        println!("  {:>7} B  {}", bytes, to_slashes(&rel));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn crawled_pages_land_in_the_knowledge_base_on_disk() {
    let server = mock_site().await;
    let data_dir = tempfile::tempdir().unwrap();
    let kb_root = tempfile::tempdir().unwrap();

    let db = init_database_memory().await.unwrap();
    let user_id = seed_user(db.pool()).await;
    let knowledge = knowledge_service(data_dir.path(), db.pool().clone()).await;
    let base = knowledge
        .create_base("Crawl KB", "", Some(kb_root.path().to_str().unwrap()), None)
        .await
        .expect("create knowledge base");

    let job = job_for(
        &user_id,
        &server.uri(),
        CrawlSink {
            knowledge_base_id: Some(base.knowledge_base_id.to_string()),
        },
    );
    run_to_completion(db.pool(), &job, knowledge).await;

    let files = markdown_files(kb_root.path());
    let rels: Vec<String> = files.iter().map(|p| to_slashes(p)).collect();
    assert!(!files.is_empty(), "no markdown was written under {:?}", kb_root.path());

    let expected = doc_dir(&job.job_id);
    for rel in &rels {
        assert!(rel.starts_with(&expected), "unexpected path: {rel} (want {expected})");
        assert!(!rel.contains("_inbox"), "crawler writes must be direct: {rel}");
    }
    assert_eq!(files.len(), 2, "both linked pages should be written: {rels:?}");

    let bodies: Vec<String> = files
        .iter()
        .map(|rel| std::fs::read_to_string(kb_root.path().join(rel)).unwrap())
        .collect();
    let seed_doc = bodies
        .iter()
        .find(|b| b.contains("Seed Page"))
        .expect("seed page document");
    assert!(seed_doc.contains("source: nomifun-crawl"), "{seed_doc}");
    assert!(seed_doc.contains(&format!("source_url: \"{}/\"", server.uri())), "{seed_doc}");
    assert!(seed_doc.contains("readability extractor"), "body text missing: {seed_doc}");
    assert!(bodies.iter().any(|b| b.contains("Second Page")), "second page missing: {rels:?}");

    let progress = claim::progress(db.pool(), &job.job_id).await.unwrap();
    assert_eq!(progress.done, 2, "both tasks should be done: {progress:?}");
    assert_eq!(progress.failed, 0, "{progress:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn configured_sink_writes_the_base_body() {
    let server = mock_site().await;
    let data_dir = tempfile::tempdir().unwrap();
    let kb_root = tempfile::tempdir().unwrap();

    let db = init_database_memory().await.unwrap();
    let user_id = seed_user(db.pool()).await;
    let knowledge = knowledge_service(data_dir.path(), db.pool().clone()).await;
    let base = knowledge
        .create_base("Crawl KB", "", Some(kb_root.path().to_str().unwrap()), None)
        .await
        .expect("create knowledge base");

    let job = job_for(
        &user_id,
        &server.uri(),
        CrawlSink {
            knowledge_base_id: Some(base.knowledge_base_id.to_string()),
        },
    );
    run_to_completion(db.pool(), &job, knowledge).await;

    let rels: Vec<String> = markdown_files(kb_root.path()).iter().map(|p| to_slashes(p)).collect();
    assert!(!rels.is_empty(), "no markdown written");
    let expected = doc_dir(&job.job_id);
    for rel in &rels {
        assert!(rel.starts_with(&expected), "unexpected path: {rel} (want {expected})");
        assert!(!rel.contains("_inbox"), "crawler writes must be direct: {rel}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_job_without_a_knowledge_base_writes_nothing_to_disk() {
    let server = mock_site().await;
    let data_dir = tempfile::tempdir().unwrap();
    let kb_root = tempfile::tempdir().unwrap();

    let db = init_database_memory().await.unwrap();
    let user_id = seed_user(db.pool()).await;
    let knowledge = knowledge_service(data_dir.path(), db.pool().clone()).await;
    knowledge
        .create_base("Crawl KB", "", Some(kb_root.path().to_str().unwrap()), None)
        .await
        .expect("create knowledge base");

    let job = job_for(&user_id, &server.uri(), CrawlSink::default());
    run_to_completion(db.pool(), &job, knowledge).await;

    assert!(markdown_files(kb_root.path()).is_empty(), "sink with no target must not write");
    let progress = claim::progress(db.pool(), &job.job_id).await.unwrap();
    assert_eq!(progress.done, 2, "crawling still succeeds: {progress:?}");
}
