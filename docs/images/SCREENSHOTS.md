# Screenshot manifest

The README images were captured or supplied for **NomiFun Desktop 0.8.0 on October 8, 2026**.
They illustrate example workflows; they are not a current feature inventory or acceptance
proof for later source. Product procedures live in the [documentation index](../README.md).

## Current README images

| Files under `docs/images/` | Purpose |
| --- | --- |
| `readme/en/home.png`, `readme/zh/home.png` | User-supplied header images, preserved at their original 2560×1664 resolution |
| `readme/en/workspace.jpg`, `readme/zh/workspace.jpg` | Conversation workspace with a completed coffee-brand brief and color table |
| `creative-studio/en-US/02-canvas-workspace.jpg`, `creative-studio/zh-CN/02-canvas-workspace.jpg` | Canvas workspace with the brief and three generated images |

Both README languages use the same three subjects. Shared-renderer captures use a 1440×900
desktop viewport and JPEG storage; the supplied headers remain lossless PNG. Technical-guide
images stay under this directory while their guides reference them. Replaced captures and
unused gallery images are retired together with their references; Git history preserves them.

## Provenance

- **Autumn Atelier** is a fictional coffee brand. Conversations, assets, preferences and
  canvases use isolated example data rather than production user data.
- The captured conversation used `step-3.7-flash`. Canvas images were generated through the
  creation-task API with `step-image-edit-2` on StepFun Step Plan, then saved through current
  product APIs. These names describe the capture, not today's recommended models.
- The user-supplied headers have separate provenance. The remaining captures use a running
  WebUI and Rust backend rather than a mock renderer or fabricated replies.
- Credentials, runtime databases, temporary scripts and raw generation media are not gallery
  assets. Keep them out of the repository.

## Refresh procedure

1. Create an isolated data root with `NOMIFUN_DATA_DIR`. Start the current Rust backend and
   Vite frontend on free loopback ports; use `NOMIFUN_WEB_PORT` for the API port.
2. Configure intended models, create examples through normal product surfaces and wait for
   real replies, media, saved state and completed work. Keep credentials hidden.
3. Capture matching English and Chinese subjects at 1440×900. The shared renderer supports
   desktop viewports only, with a minimum of 880×600.
4. Inspect the final images and their context. Update both README references and this manifest
   together, recording the new version, date and provenance accurately.
5. Remove unused captures, run `bun run check:docs` and `git diff --check`, then stop the hosts
   and remove this run's temporary credentials and data.
