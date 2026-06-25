//! Headless smoke for the Linux AT-SPI backend (run inside the dev container's
//! `smoke.sh`: Xvfb + dbus + at-spi2 + a GTK app). Connects, observes the
//! focused window, prints the element list, and optionally activates the
//! element whose name matches $NOMI_A11Y_CLICK (to exercise `do_action`).
//!
//! Usage: `cargo run -p nomi-a11y --example linux_smoke`

fn main() {
    let engine = match nomi_a11y::create_engine() {
        Ok(e) => e,
        Err(e) => {
            eprintln!("create_engine failed: {e}");
            std::process::exit(1);
        }
    };

    println!("capabilities: {:?}", engine.capabilities());

    let snap = match engine.observe(&nomi_a11y::ObserveOpts::default()) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("observe failed: {e}");
            std::process::exit(2);
        }
    };

    println!(
        "observed {} interactable element(s)  app={:?}  window={:?}  truncated={}",
        snap.entries.len(),
        snap.app_name,
        snap.window_title,
        snap.truncated,
    );
    println!("--- element list ---\n{}", snap.text);

    // Optional: activate the first element whose name contains $NOMI_A11Y_CLICK.
    if let Ok(needle) = std::env::var("NOMI_A11Y_CLICK") {
        if let Some(e) = snap
            .entries
            .iter()
            .find(|e| e.name.as_deref().is_some_and(|n| n.contains(&needle)))
        {
            println!("activating element [{}] {:?}…", e.r#ref, e.name);
            match engine.invoke(
                &nomi_a11y::Target::Ref(e.r#ref),
                snap.generation,
                nomi_a11y::ElementAction::LeftClick,
            ) {
                Ok(eff) => println!("invoke ok: {}", eff.message),
                Err(err) => eprintln!("invoke failed: {err}"),
            }
        } else {
            eprintln!("no element matching {needle:?} to click");
        }
    }
}
