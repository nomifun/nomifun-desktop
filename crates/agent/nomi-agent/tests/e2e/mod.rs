/// End-to-end tests that hit real LLM provider APIs.
///
/// These tests are skipped when the required environment variable is absent,
/// making them safe to compile and run in any environment while still providing
/// full coverage in CI when secrets are available.
///
/// Required env vars (at least one):
///   ANTHROPIC_API_KEY — runs Anthropic provider tests
///   OPENAI_API_KEY    — runs OpenAI provider tests
///
/// Run manually:
///   ANTHROPIC_API_KEY=sk-ant-... cargo test -p nomi-agent --test e2e -- --nocapture
mod anthropic;
mod compaction;
mod openai;
