//! A standalone consumer workspace, deliberately outside the product Cargo workspace.
//! The same public-port contract harness runs without model-invoke, a database or networking.
#[cfg(test)]
#[path = "../../../../crates/backend/nomifun-voice-core/tests/port_contract.rs"]
mod port_contract;
