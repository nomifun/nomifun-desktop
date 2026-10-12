//! Real HTTP Host and real child processes. No simulated hosting success.
use std::{sync::Arc, time::Duration};
use nomifun_app::{AppConfig, AuthPolicy, compatibility::{AppServices, create_router}};
use nomifun_app_hosting::{AppRecord};
use serde_json::{Value, json};

async fn host(config: &AppConfig) -> (AppServices, tokio::task::JoinHandle<()>, String) {
    let database=nomifun_db::init_database(&config.database_path()).await.unwrap();
    let services=AppServices::from_config(database, config).await.unwrap();
    let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url=format!("http://{}",listener.local_addr().unwrap());
    services.app_hosting.set_local_endpoint(url.clone());
    let router=create_router(&services).await;
    let task=tokio::spawn(async move { axum::serve(listener,router).await.unwrap(); });
    (services,task,url)
}

async fn wait_result(path: &std::path::Path) -> Value {
    for _ in 0..100 {
        if let Ok(bytes)=tokio::fs::read(path).await {
            if let Ok(value)=serde_json::from_slice(&bytes) { return value; }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("real hosted application did not finish its Host request");
}

#[tokio::test(flavor="multi_thread",worker_threads=4)]
async fn desktop_and_headless_children_connect_without_tokens_and_old_boot_proof_expires() {
    for policy in [AuthPolicy::TrustLocalToken,AuthPolicy::Required] {
        let temp=tempfile::tempdir().unwrap();
        let source=temp.path().join("source");
        std::fs::create_dir(&source).unwrap();
        std::fs::write(source.join("server.cjs"),r#"
const fs=require('node:fs');
const http=require('node:http');
const target=new URL('/api/apps',process.env.NOMIFUN_HOST_URL);
http.get(target,{headers:{'x-nomi-local-trust':process.env.NOMIFUN_LOCAL_TRUST}},res=>{
 let body='';res.on('data',part=>body+=part);res.on('end',()=>{
  fs.writeFileSync('connection-result.json',JSON.stringify({status:res.statusCode,body:JSON.parse(body)}));
 });
}).on('error',error=>{fs.writeFileSync('connection-result.json',JSON.stringify({error:error.message}));});
setInterval(()=>{},1000);
"#).unwrap();
        let config=AppConfig { data_dir:temp.path().join("data"),work_dir:temp.path().join("work"),auth_policy:policy,
            local_trust_secret: if policy==AuthPolicy::TrustLocalToken { Some(Arc::from("desktop-current-boot-proof")) } else { None }, ..Default::default() };
        let (services,server,url)=host(&config).await;
        let proof=services.local_trust_secret.as_ref().unwrap().to_string();
        let client=reqwest::Client::new();
        assert_eq!(client.get(format!("{url}/api/apps")).send().await.unwrap().status(),reqwest::StatusCode::FORBIDDEN);
        let imported:Value=client.post(format!("{url}/api/apps/import")).header("x-nomi-local-trust",&proof)
            .json(&json!({"source":source,"import_kind":"source","name":"Backend"})).send().await.unwrap().json().await.unwrap();
        assert_eq!(imported["success"],true,"{imported}");
        let app:AppRecord=serde_json::from_value(imported["data"].clone()).unwrap();
        let mut settings=app.settings.clone();settings.start_command=Some("node server.cjs".into());
        services.app_hosting.update(&app.id,app.name,settings).await.unwrap();
        let result:Value=client.post(format!("{url}/api/apps/{}/operate",app.id)).header("x-nomi-local-trust",&proof)
            .json(&json!({"operation":"use"})).send().await.unwrap().json().await.unwrap();
        assert_eq!(result["success"],true,"{result}");
        let connected=wait_result(&app.directory.join("connection-result.json")).await;
        assert_eq!(connected["status"],200,"{connected}");
        assert_eq!(connected["body"]["data"][0]["id"],app.id);
        assert!(!serde_json::to_string(&connected).unwrap().contains(&proof));
        assert_eq!(services.app_hosting.get(&app.id).await.unwrap().state,nomifun_app_hosting::AppState::Running);
        // The production Host cleanup stops this process tree before closing DB.
        server.abort();let _=server.await;
        services.cleanup_after_startup_failure(anyhow::anyhow!("e2e requested shutdown")).await;
        assert_eq!(services.app_hosting.get(&app.id).await.unwrap().state,nomifun_app_hosting::AppState::Stopped);
        drop(services);
        let next_config=AppConfig { local_trust_secret: if policy==AuthPolicy::TrustLocalToken { Some(Arc::from("desktop-next-boot-proof")) } else { None }, ..config };
        let (services,server,url)=host(&next_config).await;
        assert_eq!(client.get(format!("{url}/api/apps")).header("x-nomi-local-trust",&proof).send().await.unwrap().status(),reqwest::StatusCode::FORBIDDEN);
        let fresh=services.local_trust_secret.as_ref().unwrap();
        assert_ne!(proof,fresh.as_ref());
        assert_eq!(client.get(format!("{url}/api/apps")).header("x-nomi-local-trust",fresh.as_ref()).send().await.unwrap().status(),reqwest::StatusCode::OK);
        services.app_hosting.delete(&app.id).await.unwrap();
        assert!(source.join("server.cjs").exists());
        server.abort();let _=server.await;
        services.cleanup_after_startup_failure(anyhow::anyhow!("e2e requested shutdown")).await;
    }
}
