//! Real remote edge ownership, isolated from the developer's data and LAN choice.
use clap::Parser as _;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn relay_edge_preserves_lan_choice_and_never_accepts_webview_trust() {
    let root = tempfile::TempDir::new().unwrap();
    let work = root.path().join("work");
    std::fs::create_dir_all(&work).unwrap();
    let cli = nomifun_app::cli::Cli::parse_from([
        "relay-edge-test".to_owned(), "--data-dir".to_owned(), root.path().display().to_string(),
        "--work-dir".to_owned(), work.display().to_string(),
    ]);
    let spa = root.path().join("spa");
    std::fs::create_dir_all(&spa).unwrap();
    std::fs::write(spa.join("index.html"), "<!doctype html><title>isolated Relay test</title>").unwrap();
    let (server, _keep) = nomifun_app::DesktopServer::start_nomi_core(
        &cli, &std::env::var("PATH").unwrap_or_default(), Some(spa), None, None,
    ).await.unwrap();
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    assert!(!server.status().running);
    let local = format!("http://127.0.0.1:{}", server.loopback_port());
    let setup_body = serde_json::json!({
        "username": "smoke_owner", "password": "IsolatedPass123!",
        "device_name": "Untrusted phone", "protocol_version": 2,
        "installation_key": "a".repeat(64),
    });
    for path in ["/api/auth/setup", "/api/auth/device-setup"] {
        let response = client.post(format!("{local}{path}"))
            .header("Origin", "https://attacker.example")
            .json(&setup_body).send().await.unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::FORBIDDEN, "{path}");
        assert!(response.headers().get("access-control-allow-origin").is_none());
        assert!(response.json::<serde_json::Value>().await.unwrap().get("token").is_none());
    }
    let status: serde_json::Value = client.get(format!("{local}/api/auth/status"))
        .send().await.unwrap().json().await.unwrap();
    assert_eq!(status["needs_setup"], true, "cross-origin requests must not claim the owner");
    let devices: serde_json::Value = client.get(format!("{local}/api/connections/devices"))
        .header("x-nomi-local-trust", server.local_trust_secret())
        .send().await.unwrap().json().await.unwrap();
    assert_eq!(devices["data"], serde_json::json!([]), "denied setup must not register a device");

    // Test the actual listener, including preflight headers needed by the
    // packaged webview and tauri.conf.json's localhost:5173 development URL.
    for origin in ["tauri://localhost", "http://tauri.localhost", "https://tauri.localhost",
        "http://localhost:5173", "https://127.0.0.1:5173", "http://[::1]:5173"] {
        let response = client.request(reqwest::Method::OPTIONS, format!("{local}/api/auth/status"))
            .header("Origin", origin).header("Access-Control-Request-Method", "GET")
            .header("Access-Control-Request-Headers", "x-nomi-local-trust")
            .send().await.unwrap();
        assert_eq!(response.headers().get("access-control-allow-origin").unwrap(), origin);
        assert!(response.headers().get("access-control-allow-headers").is_some(), "{origin}");
    }
    for origin in ["https://attacker.example", "null", "tauri://attacker.example", "http://localhost.attacker.example",
        "http://localhost@attacker.example", "http://attacker.example@localhost", "http://127.0.0.1.attacker.example",
        "http://192.168.1.2:5173", "http://localhost:5173/", "http://localhost:5173?claim=1"] {
        let response = client.get(format!("{local}/api/auth/status")).header("Origin", origin)
            .send().await.unwrap();
        assert!(response.headers().get("access-control-allow-origin").is_none(), "{origin}");
    }
    for (origin, allowed) in [("null", true), ("tauri://localhost", false), ("http://localhost:5173", false), ("https://attacker.example", false)] {
        let response = client.get(format!("{local}/api/plugins/isolated/surface/assets/not-present.js"))
            .header("Origin", origin).send().await.unwrap();
        assert_eq!(response.headers().get("access-control-allow-origin").is_some(), allowed, "plugin asset / {origin}");
    }
    let response = client.post(format!("{local}/api/auth/setup"))
        .header("x-nomi-local-trust", server.local_trust_secret())
        .json(&serde_json::json!({"username":"smoke_owner","password":"IsolatedPass123!"})).send().await.unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK, "the legitimate local owner retains first-run setup");
    let port = server.start_relay().await.unwrap();
    assert_eq!(port, 25811);
    assert!(!server.status().running, "Relay must not open LAN sharing");
    let edge = format!("http://127.0.0.1:{port}");
    let status: serde_json::Value = client.get(format!("{edge}/api/auth/status"))
        .send().await.unwrap().json().await.unwrap();
    assert_eq!(status["needs_setup"], false, "owner is provisioned before exposing the edge");
    assert_eq!(status["is_authenticated"], false);
    assert_eq!(client.get(format!("{edge}/api/system/info"))
        .header("x-nomi-local-trust", server.local_trust_secret())
        .send().await.unwrap().status(), reqwest::StatusCode::FORBIDDEN);
    assert_eq!(client.get(format!("{edge}/ws"))
        .header("Sec-WebSocket-Protocol", server.local_trust_secret())
        .send().await.unwrap().status(), reqwest::StatusCode::FORBIDDEN);
    let relay_info: serde_json::Value = client.get(format!("{edge}/api/connections/info"))
        .send().await.unwrap().json().await.unwrap();
    let local_info: serde_json::Value = client.get(format!("http://127.0.0.1:{}/api/connections/info", server.loopback_port()))
        .header("x-nomi-local-trust", server.local_trust_secret())
        .send().await.unwrap().json().await.unwrap();
    assert_eq!(relay_info["data"]["desktop_id"], local_info["data"]["desktop_id"]);
    let expected = std::env::var("NOMIFUN_DESKTOP_NAME").ok().filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| "NomiFun Desktop".to_owned());
    assert_eq!(relay_info["data"]["desktop_name"], expected, "the public edge must not publish the computer name");
    let lan = server.start_lan().await;
    assert!(lan.running, "LAN can coexist with Relay: {:?}", lan.error);
    assert_ne!(lan.port, port);
    for path in ["/api/auth/setup", "/api/auth/device-setup"] {
        assert_eq!(client.post(format!("http://127.0.0.1:{}{path}", lan.port))
            .json(&setup_body).send().await.unwrap().status(), reqwest::StatusCode::FORBIDDEN,
            "the LAN edge shares the Desktop setup authority gate");
    }
    assert!(server.stop_lan().await.error.is_none());
    assert!(!server.status().running);
    assert!(client.get(format!("{edge}/api/auth/status")).send().await.unwrap().status().is_success());
    server.stop_relay().await.unwrap();
    assert!(client.get(format!("{edge}/api/auth/status")).send().await.is_err());
    assert!(client.get(format!("http://127.0.0.1:{}/api/system/info", server.loopback_port()))
        .header("x-nomi-local-trust", server.local_trust_secret()).send().await.unwrap().status().is_success());
    server.shutdown_all().await.unwrap();
}
