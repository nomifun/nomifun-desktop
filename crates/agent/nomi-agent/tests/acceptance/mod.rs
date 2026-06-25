#![allow(dead_code, unused_macros, unused_imports)]
/// Acceptance tests for evolution features (Phase 6).
///
/// These tests validate end-to-end behavior of each evolution feature
/// against real LLM providers. They are skipped when provider credentials
/// are absent, making them safe to run in any environment.
///
/// Required env vars (at least one):
///   OPENAI_API_KEY                          — runs OpenAI provider tests
///   AWS_PROFILE + CLAUDE_CODE_USE_BEDROCK=1 — runs Bedrock provider tests
///
/// Run manually:
///   OPENAI_API_KEY=sk-... cargo nextest run -p nomi-agent --profile e2e --test acceptance
#[macro_use]
mod helpers;
mod compact_test;
mod cross_feature_test;
mod file_cache_test;
mod memory_test;
mod plan_mode_test;
mod tool_desc_test;
