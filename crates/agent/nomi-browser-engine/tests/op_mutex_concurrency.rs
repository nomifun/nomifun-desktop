//! **引擎级 observe⊥act 互斥真 Chrome 冒烟**（`#[ignore]`，需 `NOMIFUN_CHROME_BINARY`）。
//!
//! 证明 `observe` 与 `act` 在**同一** `CdpBackend` 上无法交错（DESIGN §22 observe⊥act）：并发对同一
//! 引擎发一个 `act`（scroll）和一个 `observe`，二者都必须返回 `Ok`（被 `op_mutex` 串行化），且引擎绝不
//! 因「动作改 DOM 撞上快照序列化」而报陈旧 ref / NodeStale 崩。`CdpBackend` 无 Chrome 不可构造,故常驻
//! 单测只能钉 `op_mutex` 原语序列化（见 cdp.rs `op_mutex_tests`）；真交错证明在此真 Chrome 测试。
//!
//! 手动跑：`NOMIFUN_CHROME_BINARY="/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" \
//!   cargo nextest run -p nomi-browser-engine --run-ignored all -E 'test(observe_act_do_not_interleave)'`

use std::sync::Arc;
use std::time::Duration;

use nomi_browser_engine::progress::Progress;
use nomi_browser_engine::{
    create_engine, ActSpec, EngineConfig, ObserveOpts, ScrollDir, ScrollTarget,
};

#[tokio::test]
#[ignore = "需 NOMIFUN_CHROME_BINARY（真 Chrome）：observe⊥act 引擎级互斥交错冒烟"]
async fn observe_act_do_not_interleave() {
    let engine = create_engine(EngineConfig::default())
        .await
        .expect("engine builds with NOMIFUN_CHROME_BINARY set");
    engine
        .navigate("data:text/html,<button id=b>hi</button>", false)
        .await
        .expect("navigate");
    // 先 observe 建一张 ref 表，让后续动作有可瞄准的页面态。
    engine
        .observe(&ObserveOpts::default())
        .await
        .expect("prime observe");

    let e1 = Arc::clone(&engine);
    let e2 = Arc::clone(&engine);

    // 并发两操作：一个 scroll act + 一个 observe。二者必须被 op_mutex 串行化——都返回 Ok，
    // 且引擎不在动作中途被快照撞出 stale-ref 崩。
    let act = tokio::spawn(async move {
        let p = Progress::new(Duration::from_secs(10));
        e1.act(
            &ActSpec::Scroll {
                target: ScrollTarget::Viewport,
                direction: ScrollDir::Down,
                amount: Some(100.0),
            },
            &p,
        )
        .await
    });
    let obs = tokio::spawn(async move { e2.observe(&ObserveOpts::default()).await });

    let (a, o) = tokio::join!(act, obs);
    assert!(a.unwrap().is_ok(), "并发 act 必须成功（被串行化）");
    assert!(o.unwrap().is_ok(), "并发 observe 必须成功（被串行化）");
}
