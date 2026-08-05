pub mod sshd;

// Re-exported for the test binaries that use them; not every binary uses both,
// so silence the per-binary unused-import lint on this shared support module.
#[allow(unused_imports)]
pub use sshd::{connect, start_pubkey_sshd};
