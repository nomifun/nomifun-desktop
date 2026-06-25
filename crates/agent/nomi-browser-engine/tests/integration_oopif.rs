//! **OOPIF 真页验证**（`#[ignore]`，本机/打包 chrome）：跨源 http 多源 → Chrome site-isolation 把
//! 跨站 iframe 另起**跨进程子 session（OOPIF）**,走 `cdp.rs::spawn_oopif_arm_loop`(`TODO(verify-oopif)`)。
//! file:// 离线 fixture 触发不到(同进程 iframe/srcdoc 不另起子 session),故须真 http 多源。
//!
//! 起两个本地 http server:外页 `http://127.0.0.1:PA/` 内嵌跨站 iframe `http://localhost:PB/`
//! （`127.0.0.1` 与 `localhost` 是不同 host → 不同 site → site-isolation 起 OOPIF）。验证:
//! ① `oopif_session_count_for_test() >= 1`（跨进程 OOPIF 子 session 真被 arm）;
//! ② 内页内容缝入 observe（跨帧 `f<seq>e<n>` ref）;③ 内页 password 脱敏。
//!
//! 手动跑：`set NOMIFUN_CHROME_BINARY=...` 后 `cargo nextest run -p nomi-browser-engine --run-ignored all -E 'test(oopif)'`。
//! 跑完核对无残留 chrome（Builder kill_on_drop 自动清）。
//!
//! 注:本地 server 在 loopback,默认防火墙封 RFC1918/loopback,故用放行 loopback 的 FirewallConfig。

use std::io::{Read, Write};
use std::net::TcpListener;
use std::time::Duration;

use nomi_browser_engine::firewall::FirewallConfig;
use nomi_browser_engine::{BrowserEngine, ObserveOpts};

mod common;

/// 起极简静态 http server（`127.0.0.1:0` → OS 分配端口）,对任意请求回固定 HTML。返回端口。
/// 后台线程持有 listener,进程退出即随之结束（测试用,不 join）。
fn serve_html(html: String) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind 127.0.0.1:0");
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut s) = stream else { continue };
            let mut buf = [0u8; 2048];
            let _ = s.read(&mut buf); // 读请求(不解析,任意路径回同一页)
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                html.len(),
                html
            );
            let _ = s.write_all(resp.as_bytes());
            let _ = s.flush();
        }
    });
    port
}

#[tokio::test]
#[ignore = "需本机/打包 chrome + 显示器（headful）：跨源 http 多源触发 OOPIF（cdp.rs TODO(verify-oopif)）"]
async fn cross_origin_oopif_child_session_armed() {
    // 内页（origin = localhost:PB）：含 password + 可观测锚点。
    let inner = r##"<!doctype html><html><head><meta charset="utf-8"></head><body>
        <p>INNERMARKER</p>
        <label>pw <input type="password" value="sekretoopifpw"></label>
        <a href="#inner">InnerLink</a>
        </body></html>"##
        .to_string();
    let port_b = serve_html(inner);
    // 外页（site = a.nomitest）：跨站内嵌内页（site = b.nomitest）。两域经 --host-resolver-rules
    // 都映射到 127.0.0.1 的各自端口,但 Chrome 视作不同 registrable site → site-isolation 起 OOPIF。
    let outer = format!(
        r#"<!doctype html><html><head><meta charset="utf-8"></head><body>
        <h1>Outer</h1>
        <iframe src="http://b.nomitest:{port_b}/inner" width="360" height="240"></iframe>
        </body></html>"#
    );
    let port_a = serve_html(outer);

    // 默认防火墙封 loopback;本地 server 在 loopback,故放行（block_private_ips=false）。其余不限制。
    let fw = FirewallConfig {
        block_private_ips: false,
        gate_cross_origin_post: false,
        allow_etld1: vec![],
        deny_etld1: vec![],
    };
    // headless Chrome 默认不对 localhost/127.0.0.1 做站点隔离 → 用 --host-resolver-rules 把两个
    // **不同 registrable site**（a.nomitest / b.nomitest）映射到 127.0.0.1,再 --site-per-process 强制
    // 站点隔离 → 跨站 iframe 真成 OOPIF。经 launch.rs 的 NOMI_CHROME_EXTRA_ARGS escape hatch（每行一参）注入。
    // SAFETY: nextest 进程级隔离每个测试,env 仅影响本测试进程;launch 前设、launch 后清。
    unsafe {
        std::env::set_var(
            "NOMI_CHROME_EXTRA_ARGS",
            "--host-resolver-rules=MAP *.nomitest 127.0.0.1\n--site-per-process",
        );
    }
    let backend = common::build_backend_for_fixture_headful_with_firewall("oopif", fw).await;
    // chrome 已带 flag 启动;清掉 env（卫生）。
    unsafe {
        std::env::remove_var("NOMI_CHROME_EXTRA_ARGS");
    }

    backend
        .navigate(&format!("http://a.nomitest:{port_a}/"), false)
        .await
        .expect("navigate outer http page");

    // 等跨进程 OOPIF 子 session arm（site-isolation 起 type=="iframe" 子 session → spawn_oopif_arm_loop）。
    let mut oopif_n = 0usize;
    for _ in 0..50 {
        oopif_n = backend.oopif_session_count_for_test().await;
        if oopif_n >= 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    eprintln!("oopif_session_count = {oopif_n}");
    // OOPIF arm 后其 utility world 物化 + aria 注入是异步的;给一点 settle 再 observe。
    tokio::time::sleep(Duration::from_millis(1200)).await;

    let obs = backend.observe(&ObserveOpts::default()).await.expect("observe");
    eprintln!("=== oopif observe yaml ===\n{}\n=== end ===", obs.yaml);

    // ① **跨进程 OOPIF 子 session 真被 arm**（本测试主断言 + 2026-06-19 修复回归）。修前为 0：
    //    引擎只在 browser-root 设 setAutoAttach,OOPIF（page 的跨进程子帧）不自动 attach;修后
    //    handle_attached 对 page/iframe 子 session 级联 setAutoAttach → OOPIF 自动 attach →
    //    spawn_oopif_arm_loop 入 oopif_managers。实测 0→1。
    assert!(
        oopif_n >= 1,
        "跨站 iframe（a.nomitest ↔ b.nomitest）应触发跨进程 OOPIF 子 session arm（修后应=1）;实得 {oopif_n}。\
         注：必须 headful（真浏览器进程模型）+ --site-per-process;--headless=new 单渲染进程不起 OOPIF"
    );
    // ② **OOPIF 内页内容缝入 observe**（2026-06-19 缝合修复回归）。修前：observe 能 arm + 快照 OOPIF
    //    子帧,但 `resolve_owner_iframe_ref` 在 **OOPIF 自身 session** 上发 `getFrameOwner(自身根帧)`
    //    → `-32000 "Frame ... does not belong to the target"`（owner iframe 元素在**父 target**里,
    //    不在 OOPIF 自己 target）→ 路由失败 → 内页不内联（iframe 仍是叶子）。修后：对每个候选父帧在
    //    **该父帧 session** 上发 getFrameOwner,真父帧（主帧/page session）命中 → resolveNode + _ariaRef.ref
    //    缝合。内页 `<a>InnerLink</a>` / `<p>INNERMARKER</p>` 应出现在 observe（作 `iframe` 子节点）。
    assert!(
        obs.yaml.contains("InnerLink") && obs.yaml.contains("INNERMARKER"),
        "OOPIF 内页内容应缝入 observe（iframe 节点下出现 InnerLink/INNERMARKER）;实得:\n{}",
        obs.yaml
    );
    // ②b 缝合结构：内页内容必须是 `iframe` 节点的**子节点**（跨帧 `f1e<n>` ref）,而非游离顶层。
    //     渲染按缩进表达父子;断言内页 ref 用了非 0 帧序前缀（`f1`+）= 真嵌在子帧里。
    assert!(
        obs.yaml.contains("[ref=f1e"),
        "OOPIF 内页应以子帧 ref（f1e<n>）缝在父 iframe 下,而非顶层 f0;实得:\n{}",
        obs.yaml
    );
    // ③ 红线守卫：无论是否缝合,内页 password 明文都绝不得出现在 observe 输出。
    assert!(
        !obs.yaml.contains("sekretoopifpw"),
        "OOPIF 内页 password 明文泄漏:\n{}",
        obs.yaml
    );
}
