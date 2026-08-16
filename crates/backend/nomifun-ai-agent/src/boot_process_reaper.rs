//! Boot-time reaper for durable agent-process registry entries.
//!
//! After an unclean shutdown the durable registry
//! (`agent-process-registry.json`) may still list agent processes from the
//! previous run. This module walks that file exactly once at boot and, for
//! every entry, either **proves** the recorded process instance is gone (and
//! removes the entry) or retains the entry fail-closed. The resulting
//! [`AgentProcessReapReport`] is the only authority conversation healing may
//! use to conclude "no orphan of this conversation can still be running".
//!
//! Proof rules per entry:
//! - v2 entry with an [`ExactProcessIdentity`]: probe the pid. A proven-absent
//!   pid, or a live process whose identity does not match the recorded one
//!   (pid + platform start key), proves the recorded instance dead. A matching
//!   live process is an orphan survivor: it is terminated via
//!   [`terminate_verified_orphan`] and its entry removed only when
//!   termination is proven.
//! - v1 legacy entry without an identity token: a proven-absent pid is proof;
//!   a live pid cannot be attributed to the recorded instance (PID reuse), so
//!   the entry is retained and the process is never killed.
//! - Any probe or termination failure retains the entry: absence of proof is
//!   never treated as proof of absence.

use std::collections::HashMap;
use std::io;
use std::path::Path;
use std::time::Duration;

use nomi_process_runtime::{
    probe_process_identity, same_recorded_process, terminate_verified_orphan,
};
use tracing::{error, info, warn};

use crate::manager::process_registry::{
    ProcessRegistry, RegisteredAgentProcess, agent_process_registry_path, read_registry_file,
    with_registry_lock, write_registry_file,
};

/// Upper bound for proving termination of a single verified orphan survivor.
const ORPHAN_TERMINATION_TIMEOUT: Duration = Duration::from_secs(10);

/// Per-conversation verdict from boot-time durable-registry reaping.
#[derive(Clone, Debug, Default)]
pub struct ConversationProcessReapVerdict {
    /// Number of durable entries recorded for this conversation before
    /// reaping.
    pub entries_found: usize,
    /// True only when `entries_found > 0` and every entry was proven dead and
    /// removed. The zero-entry case is answered by
    /// [`AgentProcessReapReport::conversation_tree_proven_empty`], which also
    /// requires the whole registry pass to have completed.
    pub all_proven_dead: bool,
}

/// Outcome of one boot-time pass over the durable agent-process registry.
#[derive(Clone, Debug, Default)]
pub struct AgentProcessReapReport {
    /// Verdicts keyed by conversation id, for conversations that had at least
    /// one durable entry when the pass started.
    verdicts: HashMap<String, ConversationProcessReapVerdict>,
    /// True when the registry file was parsed and every entry was iterated.
    /// False (the `Default`) means nothing was proven this boot.
    registry_fully_processed: bool,
}

impl AgentProcessReapReport {
    /// No surviving durable entry for this conversation (none existed, or
    /// every one was proven dead and removed).
    ///
    /// Only meaningful proof when the registry was fully processed: a parse
    /// failure yields `false` for every conversation, fail-closed.
    pub fn conversation_tree_proven_empty(&self, conversation_id: &str) -> bool {
        if !self.registry_fully_processed {
            return false;
        }
        match self.verdicts.get(conversation_id) {
            None => true,
            Some(verdict) => verdict.all_proven_dead,
        }
    }

    /// At least one entry existed for this conversation at boot (before
    /// reaping).
    pub fn conversation_had_entries(&self, conversation_id: &str) -> bool {
        self.verdicts
            .get(conversation_id)
            .is_some_and(|verdict| verdict.entries_found > 0)
    }

    /// The reaper ran to completion over the whole registry file (parse +
    /// iteration succeeded).
    pub fn registry_fully_processed(&self) -> bool {
        self.registry_fully_processed
    }

    /// Verdict for one conversation, when it had entries at boot.
    pub fn conversation_verdict(&self, conversation_id: &str) -> Option<&ConversationProcessReapVerdict> {
        self.verdicts.get(conversation_id)
    }
}

/// Reap the durable agent-process registry once at boot.
///
/// The whole pass — read, per-entry proofs (including blocking orphan kills),
/// and the single atomic write-back of retained entries — runs on a blocking
/// thread under the registry lock, so concurrent register/unregister calls
/// cannot interleave with it.
pub async fn reap_orphan_agent_processes(data_dir: &Path) -> AgentProcessReapReport {
    let data_dir = data_dir.to_path_buf();
    match tokio::task::spawn_blocking(move || reap_registry_sync(&data_dir)).await {
        Ok(report) => report,
        Err(join_error) => {
            error!(
                error = %join_error,
                "Boot process reaper task did not complete; reporting zero proofs"
            );
            AgentProcessReapReport::default()
        }
    }
}

fn reap_registry_sync(data_dir: &Path) -> AgentProcessReapReport {
    let path = agent_process_registry_path(data_dir);
    let pass = with_registry_lock(|| -> io::Result<AgentProcessReapReport> {
        // A missing file reads as an empty registry: that IS full processing.
        let registry = read_registry_file(&path)?;
        Ok(reap_parsed_registry(&path, registry))
    });

    match pass {
        Ok(report) => report,
        Err(e) => {
            // Parse or read failure: nothing is proven for any conversation.
            error!(
                path = %path.display(),
                error = %e,
                "Failed to read agent process registry at boot; retaining all durable proofs as unproven"
            );
            AgentProcessReapReport::default()
        }
    }
}

fn reap_parsed_registry(path: &Path, registry: ProcessRegistry) -> AgentProcessReapReport {
    #[derive(Default)]
    struct Tally {
        found: usize,
        retained: usize,
    }

    let original_len = registry.processes.len();
    let mut tallies: HashMap<String, Tally> = HashMap::new();
    let mut retained: Vec<RegisteredAgentProcess> = Vec::new();

    for entry in registry.processes {
        let proven_dead = entry_proven_dead(&entry);
        let tally = tallies.entry(entry.conversation_id.clone()).or_default();
        tally.found += 1;
        if !proven_dead {
            tally.retained += 1;
            retained.push(entry);
        }
    }

    if retained.len() != original_len {
        let updated = ProcessRegistry {
            processes: retained,
            ..ProcessRegistry::default()
        };
        if let Err(e) = write_registry_file(path, &updated) {
            // The removals were each backed by a real liveness/termination
            // proof, so this boot's report stays valid; the stale entries
            // merely survive on disk and will be re-proven next boot.
            error!(
                path = %path.display(),
                error = %e,
                "Failed to persist reaped agent process registry; stale entries will be re-verified next boot"
            );
        }
    }

    AgentProcessReapReport {
        verdicts: tallies
            .into_iter()
            .map(|(conversation_id, tally)| {
                (
                    conversation_id,
                    ConversationProcessReapVerdict {
                        entries_found: tally.found,
                        all_proven_dead: tally.found > 0 && tally.retained == 0,
                    },
                )
            })
            .collect(),
        registry_fully_processed: true,
    }
}

/// Prove that the process instance recorded by `entry` no longer runs.
/// Returns `true` only on positive proof; every uncertain path returns
/// `false` (retain the entry, never kill).
fn entry_proven_dead(entry: &RegisteredAgentProcess) -> bool {
    let Some(recorded) = &entry.identity else {
        // Legacy v1 entry: a bare pid cannot be attributed to the recorded
        // instance once something is running under it.
        return match probe_process_identity(entry.pid) {
            Ok(None) => true,
            Ok(Some(_)) => {
                warn!(
                    pid = entry.pid,
                    conversation_id = %entry.conversation_id,
                    "Legacy registry entry without identity token has a live pid; refusing to kill or clear"
                );
                false
            }
            Err(e) => {
                warn!(
                    pid = entry.pid,
                    conversation_id = %entry.conversation_id,
                    error = %e,
                    "Could not probe legacy registry entry; retaining it unproven"
                );
                false
            }
        };
    };

    match probe_process_identity(entry.pid) {
        // Proven absent: nothing runs under this pid at all.
        Ok(None) => true,
        Ok(Some(live)) => {
            if !same_recorded_process(recorded, &live) {
                // The pid was recycled by an unrelated process, which proves
                // the recorded instance already exited. Never touch the
                // current pid owner.
                info!(
                    pid = entry.pid,
                    conversation_id = %entry.conversation_id,
                    "Registry pid is owned by a different process instance; recorded orphan proven dead"
                );
                return true;
            }
            // Exact orphan survivor: terminate it and require proof.
            match terminate_verified_orphan(recorded, ORPHAN_TERMINATION_TIMEOUT) {
                Ok(outcome) => {
                    info!(
                        pid = entry.pid,
                        conversation_id = %entry.conversation_id,
                        ?outcome,
                        "Terminated verified orphan agent process at boot"
                    );
                    true
                }
                Err(e) => {
                    warn!(
                        pid = entry.pid,
                        conversation_id = %entry.conversation_id,
                        error = %e,
                        "Failed to prove termination of orphan agent process; retaining registry entry"
                    );
                    false
                }
            }
        }
        Err(e) => {
            warn!(
                pid = entry.pid,
                conversation_id = %entry.conversation_id,
                error = %e,
                "Could not probe registry entry identity; retaining it unproven"
            );
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use std::process::Stdio;

    use nomi_process_runtime::{ExactProcessIdentity, capture_child_identity};
    use nomifun_common::AgentType;

    use super::*;

    fn test_entry(
        pid: u32,
        conversation_id: &str,
        identity: Option<ExactProcessIdentity>,
    ) -> RegisteredAgentProcess {
        RegisteredAgentProcess {
            pid,
            process_group_id: None,
            conversation_id: conversation_id.into(),
            agent_type: AgentType::Nomi.serde_name().into(),
            backend: Some("nomi".into()),
            command_preview: Some("test-agent".into()),
            registered_at_ms: 1,
            identity,
        }
    }

    fn write_registry(data_dir: &Path, processes: Vec<RegisteredAgentProcess>) {
        let registry = ProcessRegistry {
            processes,
            ..ProcessRegistry::default()
        };
        write_registry_file(&agent_process_registry_path(data_dir), &registry).unwrap();
    }

    fn read_registry(data_dir: &Path) -> ProcessRegistry {
        read_registry_file(&agent_process_registry_path(data_dir)).unwrap()
    }

    /// Spawn a quiet ~60s child the reaper can probe / terminate. On Unix the
    /// child leads its own process group, matching the contained-spawn
    /// contract `terminate_verified_orphan` requires for group kills.
    fn spawn_sleeper() -> (tokio::process::Child, ExactProcessIdentity) {
        let mut cmd = if cfg!(windows) {
            let mut cmd = tokio::process::Command::new("ping");
            cmd.args(["-n", "60", "127.0.0.1"]);
            cmd
        } else {
            let mut cmd = tokio::process::Command::new("sleep");
            cmd.arg("60");
            cmd
        };
        cmd.stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        #[cfg(unix)]
        cmd.process_group(0);
        let child = cmd.spawn().expect("spawn test sleeper");
        let identity = capture_child_identity(&child).expect("capture test child identity");
        (child, identity)
    }

    #[tokio::test]
    async fn dead_pid_v2_entry_is_removed_and_proven() {
        let dir = tempfile::tempdir().unwrap();
        let (mut child, identity) = spawn_sleeper();
        let pid = identity.pid;
        write_registry(dir.path(), vec![test_entry(pid, "conv-dead", Some(identity))]);

        // Kill and reap the child BEFORE the pass: the recorded instance is
        // genuinely dead, whatever now owns the pid.
        child.kill().await.unwrap();
        child.wait().await.unwrap();

        let report = reap_orphan_agent_processes(dir.path()).await;

        assert!(report.registry_fully_processed());
        assert!(report.conversation_had_entries("conv-dead"));
        assert!(report.conversation_tree_proven_empty("conv-dead"));
        assert!(read_registry(dir.path()).processes.is_empty());
    }

    #[tokio::test]
    async fn live_pid_with_wrong_start_key_is_removed_without_killing() {
        let dir = tempfile::tempdir().unwrap();
        let (mut child, real_identity) = spawn_sleeper();
        let fabricated = ExactProcessIdentity {
            platform_start_key: real_identity.platform_start_key + 1,
            ..real_identity.clone()
        };
        write_registry(
            dir.path(),
            vec![test_entry(real_identity.pid, "conv-reused", Some(fabricated))],
        );

        let report = reap_orphan_agent_processes(dir.path()).await;

        assert!(report.conversation_tree_proven_empty("conv-reused"));
        assert!(read_registry(dir.path()).processes.is_empty());
        // The current pid owner is a different instance and must survive.
        assert!(
            child.try_wait().unwrap().is_none(),
            "reaper must not kill a non-matching pid owner"
        );

        child.kill().await.unwrap();
        child.wait().await.unwrap();
    }

    #[tokio::test]
    async fn legacy_entry_with_live_pid_is_retained_unproven() {
        let dir = tempfile::tempdir().unwrap();
        let (mut child, identity) = spawn_sleeper();
        write_registry(dir.path(), vec![test_entry(identity.pid, "conv-legacy", None)]);

        let report = reap_orphan_agent_processes(dir.path()).await;

        assert!(report.registry_fully_processed());
        assert!(report.conversation_had_entries("conv-legacy"));
        assert!(!report.conversation_tree_proven_empty("conv-legacy"));
        let survivors = read_registry(dir.path()).processes;
        assert_eq!(survivors.len(), 1);
        assert_eq!(survivors[0].pid, identity.pid);
        // Without an identity token the reaper must not kill.
        assert!(
            child.try_wait().unwrap().is_none(),
            "reaper must not kill a legacy entry's live pid"
        );

        child.kill().await.unwrap();
        child.wait().await.unwrap();
    }

    #[tokio::test]
    async fn matching_live_orphan_is_killed_and_proven() {
        let dir = tempfile::tempdir().unwrap();
        let (mut child, identity) = spawn_sleeper();
        let pid = identity.pid;
        write_registry(dir.path(), vec![test_entry(pid, "conv-orphan", Some(identity))]);

        let report = reap_orphan_agent_processes(dir.path()).await;

        assert!(report.conversation_tree_proven_empty("conv-orphan"));
        let verdict = report.conversation_verdict("conv-orphan").unwrap();
        assert_eq!(verdict.entries_found, 1);
        assert!(verdict.all_proven_dead);
        assert!(read_registry(dir.path()).processes.is_empty());
        // The orphan must actually be gone, not merely deregistered; reaching
        // a wait() status within the timeout is the liveness proof.
        tokio::time::timeout(Duration::from_secs(5), child.wait())
            .await
            .expect("orphan child should exit after reaping")
            .unwrap();
    }

    #[tokio::test]
    async fn conversation_without_entries_is_proven_empty_when_fully_processed() {
        let dir = tempfile::tempdir().unwrap();
        // No registry file at all: an empty registry is fully processed.
        let report = reap_orphan_agent_processes(dir.path()).await;

        assert!(report.registry_fully_processed());
        assert!(!report.conversation_had_entries("conv-none"));
        assert!(report.conversation_tree_proven_empty("conv-none"));
    }

    #[tokio::test]
    async fn parse_error_registry_proves_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = agent_process_registry_path(dir.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"{ not json ").unwrap();

        let report = reap_orphan_agent_processes(dir.path()).await;

        assert!(!report.registry_fully_processed());
        assert!(!report.conversation_tree_proven_empty("conv-any"));
        assert!(!report.conversation_had_entries("conv-any"));
        // The unreadable file must be left untouched for operators.
        assert_eq!(std::fs::read(&path).unwrap(), b"{ not json ");
    }
}
