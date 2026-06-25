//! **P3 W4d：storage_state 持久化 vault（加密）+ 启动 inject 持久登录 端到端集成**
//! （`#[ignore]`，本机/打包 chrome）。
//!
//! 验证 W4d（吸收原 P6 持久登录）：W4b/c 是**内存往返**（capture↔restore），W4d 在两端之间塞**磁盘
//! vault**，证**跨引擎/会话的持久登录闭环**：
//!
//! ```text
//!   引擎 A（会话 A 登录）：navigate example.com → 写 cookie + localStorage（模拟登录）
//!                          → capture_cookies + capture_local_storage → save_storage_state(vault)  [加密]
//!   ── A close（登录态只在加密 vault 文件里）──
//!   引擎 B（新引擎/会话 B）：load_storage_state(vault) → EngineConfig.storage_state
//!                          → 引擎启动注入 cookie（navigate 前即灌）
//!                          → navigate example.com（带上恢复的 cookie）→ restore_local_storage（origin-bound）
//!                          → cookie + localStorage **都恢复存活**（读回）= 持久登录成立。
//! ```
//!
//! **加密验收**：vault 文件落盘是 AES-256-GCM 密文（不含明文 cookie token / JWT），换 key 解不开
//! （纯逻辑已在 `vault::tests` 钉死；本集成跑真 save/load 往返证密文可解回真登录态）。
//!
//! **默认 None 零回归**：`storage_state=None`（不灌）→ 引擎启动不碰 cookie/localStorage（现行为）。
//!
//! 手动跑（本机 Windows 有系统 Chrome）：
//!   set NOMIFUN_CHROME_BINARY=...\chrome.exe
//!   cargo nextest run -p nomi-browser-engine --run-ignored all -E 'test(w4d)'
//! 跑完核对任务管理器无残留 chrome（Builder kill_on_drop + disposeOnDetach 自动清）。
//!
//! 真实结果（本机首跑 eprintln 出 vault 路径 + 恢复读回的 cookie/localStorage——填回任务汇报）。

use nomi_browser_engine::storage_state::{SameSite, StorageState, StorageStateCookie};
use nomi_browser_engine::{
    load_storage_state, save_storage_state, shared_storage_state_path, storage_state_path,
};
use nomi_browser_engine::BrowserEngine;

mod common;

const ORIGIN: &str = "https://example.com";

/// 测试用 32 字节 key（机器绑定 key 的占位；真机用 app provision 的 encryption_key）。
fn machine_key() -> [u8; 32] {
    [0x5a; 32]
}

/// 造一条登录态 cookie（持久 + sameSite=Lax，绑 example.com——与 navigate 目标同域）。
fn login_cookie(name: &str, value: &str) -> StorageStateCookie {
    StorageStateCookie {
        name: name.into(),
        value: value.into(),
        domain: ".example.com".into(),
        path: "/".into(),
        expires: 4_102_444_800.0, // 2100-01-01（Chrome 钳到 ~400d，仍是持久 cookie）
        http_only: false,
        secure: false,
        session: false,
        same_site: Some(SameSite::Lax),
        priority: nomi_browser_engine::storage_state::Priority::Medium,
        source_scheme: nomi_browser_engine::storage_state::SourceScheme::NonSecure,
        source_port: -1,
        partition_key: None,
    }
}

/// 在当前页面 `setItem` 一组 localStorage 键值（模拟「页面写了 localStorage 登录态」）。
async fn seed_local_storage(backend: &nomi_browser_engine::backend::CdpBackend, pairs: &[(&str, &str)]) {
    let pairs_owned: Vec<[&str; 2]> = pairs.iter().map(|(k, v)| [*k, *v]).collect();
    let pairs_json = serde_json::to_string(&pairs_owned).expect("json pairs");
    let script = format!(
        "(() => {{ const pairs = {pairs_json}; for (const [k, v] of pairs) localStorage.setItem(k, v); return localStorage.length; }})()"
    );
    backend
        .__eval_page_world_for_test(&script)
        .await
        .expect("seed localStorage");
}

/// 读回当前页面某 localStorage 键（None = 不存在）。
async fn read_local_storage(backend: &nomi_browser_engine::backend::CdpBackend, key: &str) -> Option<String> {
    let key_json = serde_json::to_string(key).expect("json key");
    let script = format!("localStorage.getItem({key_json})");
    let r = backend
        .__eval_page_world_for_test(&script)
        .await
        .expect("read localStorage");
    r.get("value").and_then(|v| v.as_str()).map(|s| s.to_string())
}

fn find_cookie<'a>(state: &'a StorageState, name: &str) -> Option<&'a StorageStateCookie> {
    state.cookies.iter().find(|c| c.name == name)
}

/// **核心：跨引擎/会话持久登录闭环（cookie + localStorage 经加密 vault 持久 → 启动注入恢复）**。
#[tokio::test]
#[ignore = "需本机/打包 chrome：set NOMIFUN_CHROME_BINARY 后 --run-ignored all"]
async fn w4d_persistent_login_cross_engine_via_encrypted_vault() {
    // vault 目录（临时目录当 workspace）。
    let vault_dir = tempfile::tempdir().expect("vault tempdir");
    let vault_path = storage_state_path(vault_dir.path());
    let key = machine_key();
    eprintln!("=== w4d vault path = {} ===", vault_path.display());

    // ── 会话 A：登录 → capture → save 加密 vault ──────────────────────────────────
    {
        let backend_a = common::build_backend_for_fixture("w4d-A").await;
        // 灌一条登录态 cookie（模拟「会话 A 登录拿到了 cookie」）。
        let seed_state = StorageState {
            cookies: vec![login_cookie("nomi_session", "persisted-login-tok-abc")],
            ..Default::default()
        };
        backend_a.restore_cookies(&seed_state).await.expect("seed cookie into A");
        // navigate + 写 localStorage 登录态。
        backend_a.navigate(ORIGIN, false).await.expect("A navigate example.com");
        seed_local_storage(&backend_a, &[("ls_auth", "jwt.persisted.signature")]).await;

        // capture A 的登录态（cookie + localStorage）。
        let cookies = backend_a.capture_cookies().await.expect("capture A cookies");
        let ls = backend_a
            .capture_local_storage()
            .await
            .expect("capture A localStorage")
            .expect("A page has origin");
        let state = StorageState {
            cookies: cookies.cookies,
            local_storage: vec![ls],
        };
        assert!(find_cookie(&state, "nomi_session").is_some(), "A must have the login cookie before save");
        eprintln!(
            "=== w4d A captured: {} cookie(s), localStorage origin={} ===",
            state.cookies.len(),
            state.local_storage[0].origin
        );

        // **save 加密 vault**（登录态落盘，加密）。
        save_storage_state(&state, &vault_path, &key).expect("save_storage_state encrypted vault");
    }

    // **加密验收**：vault 文件内容是密文，不含明文 cookie token / JWT。
    let raw = std::fs::read_to_string(&vault_path).expect("read raw vault");
    assert!(!raw.contains("persisted-login-tok-abc"), "cookie token must NOT be plaintext in vault");
    assert!(!raw.contains("jwt.persisted.signature"), "localStorage JWT must NOT be plaintext in vault");
    eprintln!("=== w4d vault is ciphertext ({} base64 chars), no plaintext login state ===", raw.len());

    // ── 会话 B：load 加密 vault → 启动注入 → 登录态恢复 ───────────────────────────
    // load_storage_state 解密读回（坏 vault 会返 None，这里应 Some）。
    let loaded = load_storage_state(&vault_path, &key).expect("load_storage_state decrypts vault");
    assert!(find_cookie(&loaded, "nomi_session").is_some(), "vault must carry the login cookie");
    // 喂给新引擎的 EngineConfig.storage_state（JSON 形态）。
    let inject = loaded.to_json().expect("storage_state to_json for inject");

    // **全新引擎 B + storage_state 注入**——启动即 restore_cookies。
    let backend_b = common::build_backend_for_fixture_with_storage_state(
        "w4d-B",
        Some(inject),
    )
    .await;

    // navigate example.com：恢复的 cookie 是启动时已灌，navigate 后该域 cookie 仍在。
    backend_b.navigate(ORIGIN, false).await.expect("B navigate example.com");
    let after = backend_b.capture_cookies().await.expect("B capture cookies after nav");
    eprintln!("=== w4d B has {} cookie(s) after startup inject + nav ===", after.cookies.len());
    let c = find_cookie(&after, "nomi_session")
        .expect("login cookie must be restored into B at startup (persistent login via vault)");
    assert_eq!(c.value, "persisted-login-tok-abc", "restored cookie value must match the persisted login");

    // localStorage 是 origin-bound：B 现已在 example.com，restore_local_storage 灌回该 origin 的项。
    backend_b
        .restore_local_storage(&loaded)
        .await
        .expect("restore localStorage into B (now on the matching origin)");
    let ls_auth = read_local_storage(&backend_b, "ls_auth").await;
    eprintln!("=== w4d B localStorage ls_auth after restore = {ls_auth:?} ===");
    assert_eq!(
        ls_auth.as_deref(),
        Some("jwt.persisted.signature"),
        "localStorage login state must be restored into B (round-trip via vault)"
    );
}

/// **默认 None 零回归**：`storage_state=None` → 引擎启动不注入任何登录态（干净起点）。
#[tokio::test]
#[ignore = "需本机/打包 chrome：set NOMIFUN_CHROME_BINARY 后 --run-ignored all"]
async fn w4d_none_storage_state_injects_nothing() {
    // 无注入（storage_state=None）——与现行为完全一致。
    let backend = common::build_backend_for_fixture_with_storage_state(
        "w4d-none",
        None,
    )
    .await;
    backend.navigate(ORIGIN, false).await.expect("navigate example.com");
    // 未注入 → example.com 无任何我们灌的 cookie（干净起点；可能有页面自设 cookie，
    // 但绝不该有 w4d 的 nomi_session）。
    let captured = backend.capture_cookies().await.expect("capture");
    eprintln!("=== w4d none-inject: {} cookie(s) (must not contain injected login) ===", captured.cookies.len());
    assert!(
        find_cookie(&captured, "nomi_session").is_none(),
        "None storage_state must inject nothing (zero regression): {:?}",
        captured.cookies
    );
    // localStorage 同样干净（未注入）。
    assert_eq!(
        read_local_storage(&backend, "ls_auth").await,
        None,
        "None storage_state must not inject localStorage"
    );
}

/// **共享浏览器身份端到端**：伙伴 A 登录 → save 到**共享** vault
/// `{data_dir}/browser-state/storage_state.enc` → 伙伴 B load **同一份共享** vault
/// → 启动注入 → B 直接拥有 A 的登录态（cookie + localStorage 跨伙伴共享）。
///
///   set NOMIFUN_CHROME_BINARY=...\chrome.exe
///   cargo nextest run -p nomi-browser-engine --run-ignored all -E 'test(shared_identity)'
#[tokio::test]
#[ignore = "需本机/打包 chrome：set NOMIFUN_CHROME_BINARY 后 --run-ignored all -E 'test(shared_identity)'"]
async fn shared_identity_login_crosses_companions_via_shared_vault() {
    // 共享 vault 落 {data_dir}/browser-state/storage_state.enc（data_dir 用临时目录当 app data-dir）。
    let data_dir = tempfile::tempdir().expect("data_dir tempdir");
    let vault_path = shared_storage_state_path(data_dir.path());
    let key = machine_key();
    eprintln!("=== shared identity vault path = {} ===", vault_path.display());
    // 共享单例硬证据：两次解析同一份（与伙伴/会话无关）。
    assert_eq!(vault_path, shared_storage_state_path(data_dir.path()), "shared vault path is a singleton");

    // ── 伙伴 A：登录 → capture → save 到共享 vault ──────────────────────────────────
    {
        let backend_a = common::build_backend_for_fixture("shared-A").await;
        let seed_state = StorageState {
            cookies: vec![login_cookie("nomi_session", "shared-login-tok-xyz")],
            ..Default::default()
        };
        backend_a.restore_cookies(&seed_state).await.expect("seed cookie into A");
        backend_a.navigate(ORIGIN, false).await.expect("A navigate example.com");
        seed_local_storage(&backend_a, &[("ls_auth", "jwt.shared.signature")]).await;

        let cookies = backend_a.capture_cookies().await.expect("capture A cookies");
        let ls = backend_a
            .capture_local_storage()
            .await
            .expect("capture A localStorage")
            .expect("A page has origin");
        let state = StorageState { cookies: cookies.cookies, local_storage: vec![ls] };
        // **save 到共享 vault**（不绑 A 的 workspace——这是「共享」的本质）。
        save_storage_state(&state, &vault_path, &key).expect("save to SHARED vault");
    }

    // ── 伙伴 B：load 同一共享 vault → 启动注入 → 登录态恢复 ──
    let loaded = load_storage_state(&vault_path, &key).expect("B loads the SHARED vault");
    assert!(
        find_cookie(&loaded, "nomi_session").is_some(),
        "shared vault must carry A's login cookie (cross-companion sharing)"
    );
    let inject = loaded.to_json().expect("storage_state to_json");
    // B 用独立引擎但共享同一登录 vault。
    let backend_b = common::build_backend_for_fixture_with_storage_state(
        "shared-B",
        Some(inject),
    )
    .await;
    backend_b.navigate(ORIGIN, false).await.expect("B navigate example.com");
    let after = backend_b.capture_cookies().await.expect("B capture cookies after nav");
    let c = find_cookie(&after, "nomi_session")
        .expect("A's login cookie must be present in B via the SHARED vault (shared identity)");
    assert_eq!(c.value, "shared-login-tok-xyz", "B inherits A's login (shared browser identity)");

    backend_b
        .restore_local_storage(&loaded)
        .await
        .expect("restore localStorage into B");
    assert_eq!(
        read_local_storage(&backend_b, "ls_auth").await.as_deref(),
        Some("jwt.shared.signature"),
        "A's localStorage login must be visible to B (shared identity via shared vault)"
    );

    // 共享 vault 不在 B 的 per-workspace 下（证明用的是共享路径）。
    let b_workspace_vault = storage_state_path(data_dir.path().join("companionB").join("workspace").as_path());
    assert_ne!(vault_path, b_workspace_vault, "B reads the SHARED vault, not a per-workspace one");
}
