//! Media provider adapters. **Empty in M0** — the skeleton wires no backend, so
//! `POST /api/creation/tasks` enqueues then fails `adapter_unavailable`.
//!
//! M2 lands the real adapters here (each a [`crate::provider::MediaProvider`]):
//! `openai_images` (sync `/images`), `media_async` (generic submit→poll),
//! `gemini_image`; then `ark` (火山方舟) + `modelscope` (P1); `comfyui` +
//! `runninghub` (P2). Register them on the [`crate::service::CreationService`]
//! at app assembly time.
