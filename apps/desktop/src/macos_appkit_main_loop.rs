//! Public main-run-loop dispatch shared by production and native audit tools.
use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    sync::Mutex,
};

use anyhow::Context;
use block2::RcBlock;
use objc2_app_kit::NSModalPanelRunLoopMode;
use objc2_core_foundation::{CFArray, CFRunLoop, CFString, kCFRunLoopCommonModes, kCFRunLoopDefaultMode};

pub(crate) fn post_to_main(work: impl FnOnce() + Send + 'static) -> anyhow::Result<()> {
    let run_loop = CFRunLoop::main().context("AppKit main run loop is unavailable")?;
    let modal = CFString::from_str(&unsafe { NSModalPanelRunLoopMode }.to_string());
    let default = unsafe { kCFRunLoopDefaultMode }.context("default run loop mode is unavailable")?;
    let common = unsafe { kCFRunLoopCommonModes }.context("common run loop modes are unavailable")?;
    let modes = CFArray::from_objects(&[&*modal, default, common]);
    // CF copies a callable block. Consume the FnOnce payload only once even
    // if a runtime unexpectedly invokes its copied block more than once.
    let work = Mutex::new(Some(work));
    let block = RcBlock::new(move || {
        let work = work.lock().ok().and_then(|mut work| work.take());
        if let Some(work) = work {
            if catch_unwind(AssertUnwindSafe(work)).is_err() {
                let _ = catch_unwind(AssertUnwindSafe(|| {
                    tracing::error!("AppKit main-loop callback panicked; completion remains unverified");
                }));
            }
        }
    });
    // SAFETY: The copied block captures only Send Rust state and runs on the
    // main run loop in explicit public modes, including native modal waits.
    unsafe { run_loop.perform_block(Some(&modes), Some(&block)); }
    run_loop.wake_up();
    Ok(())
}
