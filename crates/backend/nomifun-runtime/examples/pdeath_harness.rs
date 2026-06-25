//! 手动校验 macOS 父死安全网（fork-based watchdog，2026-06-19 重写）。
//!
//! 经真实 `Builder::spawn` 起一个长命子进程（`process_group(0)` 自成进程组），打印其 pid,然后父进程
//! 挂起。外部脚本 `kill -9` 本进程（父），随后检查子进程是否被 **fork 出来的独立 watchdog 进程** 收掉
//! （`kill -0 <child>` → ESRCH = 已收）。这是 PLATFORM-VERIFICATION.md Task 4 ③「kill -9 父 → 子被回收」
//! 的可执行复现器（进程内单测杀不了测试自身,故用 example + 脚本编排）。
use std::io::Write;

#[tokio::main]
async fn main() {
    let mut b = nomifun_runtime::Builder::new("sleep");
    b.arg("600");
    let child = b.spawn().expect("spawn child");
    println!("CHILD_PID={}", child.id().expect("child pid"));
    std::io::stdout().flush().ok();
    // 保持父进程存活;外部 kill -9 本进程 → watchdog（独立进程,存活于父 SIGKILL）应收掉子。
    std::thread::sleep(std::time::Duration::from_secs(600));
    drop(child);
}
