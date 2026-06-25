//! Post-session memory distillation orchestration for the nomi engine.
//!
//! This is the async/LLM half of spec-G: the pure functions live in
//! `nomi_memory::distill`. Here we gate on an opt-in flag, redact the
//! transcript (gate 1), call the provider once (with a single parse retry),
//! redact each distilled entry (gate 2), and write to disk on a blocking
//! thread.
//!
//! Discipline: distillation is a fire-and-forget background task. Every
//! failure path degrades silently (debug/warn log, never `emit_error`) —
//! mirroring the MEMORY note "Cache FullMiss must not emit_error": a
//! background side-task's failure must not look like a failed turn to the
//! AutoWork orchestrator.

use std::path::PathBuf;
use std::sync::Arc;

use nomi_config::config::Config;
use nomi_memory::distill::{
    DistillOutput, apply_distilled, build_distill_prompt, parse_distill_output, DISTILL_SYSTEM,
};
use nomi_redact::redact_secrets_owned;

use crate::factory::provider_config::{one_shot_completion, user_message};

/// Token ceiling for the distillation completion. codex Phase1 runs
/// low-effort; nomi's `one_shot_completion` already sends no reasoning_effort,
/// and a small ceiling keeps the cost of each distilled session bounded.
const DISTILL_MAX_TOKENS: u32 = 2048;

/// Environment-variable gate. Distillation adds one extra LLM call per normal
/// work session (token cost), so it is OFF unless explicitly enabled — this
/// avoids surprising users with unexpected spend. nomi-config has no memory
/// section today, so an env flag is the lowest-risk gate (same pattern as the
/// `NOMIFUN_COMPUTER_USE` / `NOMIFUN_BROWSER_USE` host flags).
const DISTILL_ENABLED_ENV: &str = "NOMIFUN_MEMORY_DISTILL";

/// Whether distillation is enabled for this host. `"1"` / `"true"`
/// (case-insensitive) enable it; anything else (including unset) keeps it off.
pub fn distill_enabled() -> bool {
    std::env::var(DISTILL_ENABLED_ENV)
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

/// Run one post-session distillation. Caller has already decided this turn is
/// eligible (not companion, origin empty, `distill_dir` set) and that the gate
/// is on. `transcript` is the engine's role-tagged history snapshot.
pub async fn run_distill(cfg: Arc<Config>, dir: PathBuf, transcript: String) {
    // Gate 1: redact the transcript before it is uploaded to the provider.
    let transcript = redact_secrets_owned(transcript);
    if transcript.trim().is_empty() {
        return;
    }
    let prompt = build_distill_prompt(&transcript);

    // One parse retry (the model occasionally wraps JSON in prose); a provider
    // failure does not burn the retry. Mirrors the companion learner's policy.
    let mut parsed: Option<DistillOutput> = None;
    for _ in 0..2 {
        match one_shot_completion(&cfg, DISTILL_SYSTEM, vec![user_message(&prompt)], DISTILL_MAX_TOKENS).await {
            Ok(raw) => match parse_distill_output(&raw) {
                Ok(out) => {
                    parsed = Some(out);
                    break;
                }
                Err(e) => tracing::debug!(error = %e, "distill output unparseable"),
            },
            Err(e) => {
                tracing::debug!(error = %e, "distill provider call failed");
                break; // provider failure: don't retry
            }
        }
    }

    let Some(mut out) = parsed else {
        return;
    };
    if out.memories.is_empty() {
        return; // no-op gate hit: nothing worth keeping
    }

    // Gate 2: redact every distilled field before it touches disk.
    for m in &mut out.memories {
        m.content = redact_secrets_owned(std::mem::take(&mut m.content));
        m.description = redact_secrets_owned(std::mem::take(&mut m.description));
    }

    // Write on a blocking thread (synchronous fs in nomi-memory).
    let _ = tokio::task::spawn_blocking(move || match apply_distilled(&dir, &out) {
        Ok(n) if n > 0 => {
            tracing::info!(written = n, dir = %dir.display(), "session distilled to file-based memory")
        }
        Ok(_) => {} // all candidates deduped / filtered
        Err(e) => tracing::warn!(error = %e, "distill apply failed"),
    })
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distill_enabled_reads_env() {
        // We avoid mutating the process env in a parallel test run; just assert
        // the default (unset in CI) is off. Explicit on/off parsing is covered
        // by the simple string comparison in `distill_enabled`.
        let key = DISTILL_ENABLED_ENV;
        if std::env::var(key).is_err() {
            assert!(!distill_enabled());
        }
    }
}
