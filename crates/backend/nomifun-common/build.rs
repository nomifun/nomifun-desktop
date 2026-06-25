fn main() {
    // `channel::channel()` bakes `NOMI_CHANNEL` into this crate via
    // `option_env!`. Cargo does not track env vars read by `option_env!`, so
    // without this a channel switch (stable ⇄ dev) would NOT recompile and the
    // old channel would persist until a manual `cargo clean`. Telling cargo to
    // rerun this build script when the var changes marks the crate dirty and
    // forces the recompile that picks up the new channel.
    println!("cargo:rerun-if-env-changed=NOMI_CHANNEL");
}
