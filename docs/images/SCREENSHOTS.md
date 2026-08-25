# Screenshot Manifest

This manifest records the repository-local screenshots used by the Desktop
README and technical guides. The current set was captured on **August 25, 2026**
from the 0.7.2 codebase with an isolated data root.

The Creative Studio set covers Canvas Library, the rich Canvas editor, Image
Workbench, Video Workbench, Prompt Center, My Assets, Template Studio, Template
Editor, Director, and a visible native desktop companion. These are current
running-app captures, not legacy mockups. Do not restore retired screenshots.

## Ownership and storage

- Product-use documentation and its canonical screenshot library are owned by
  [NomiFun Portal](https://www.nomifun.com/docs/).
- Desktop keeps technical contracts and a small offline README showcase. Portal
  owns the end-user walkthroughs and production gallery.
- Repository-local images are intentional so README pages remain readable
  offline. Do not replace them with external image URLs.

## README showcase

| File | Current surface |
| --- | --- |
| `readme/en/workspace.png` / `readme/zh/workspace.png` | Current Desktop workspace and session hub |
| `readme/en/models.png` / `readme/zh/models.png` | Model Management and task-aware model catalog |
| `readme/en/companions.png` / `readme/zh/companions.png` | Current workspace with the live desktop companion visible |
| `readme/en/skills.png` / `readme/zh/skills.png` | Skills Hub with Creative Studio skills |
| `readme/en/creative-workshop.png` / `readme/zh/creative-workshop.png` | Rich Creative Studio Canvas editor |

## Creative Studio gallery

English captures live under `creative-studio/en-US/`; Chinese captures live
under `creative-studio/zh-CN/`. Both locale sets use the same route order:

| File | Route / subject |
| --- | --- |
| `01-canvas-library.png` | `#/workshop/canvases` · Canvas Library |
| `02-canvas-editor-rich.png` | `#/workshop/canvas/:canvasId` · rich Canvas editor, assets, Assistant, and Director panel |
| `03-image-workbench.png` | `#/workshop/image` · standalone T2I/I2I workbench |
| `04-video-workbench.png` | `#/workshop/video` · standalone T2V/I2V workbench |
| `05-prompt-center.png` | `#/workshop/prompts` · searchable Prompt Center |
| `06-asset-library.png` | `#/workshop/assets` · My Assets and reusable inputs |
| `07-template-studio.png` | `#/workshop/templates` · Template Studio |
| `08-template-editor.png` | Template Editor and AI Create review flow |
| `09-director-timeline.png` | Director timeline, cameras, keyframes, and capture |
| `companion.png` | Current workspace with the real transparent companion window visible |

The Creative Studio captures use a 1440×900 viewport. The companion image was
captured from the native transparent Tauri window and composited over the
current workspace; the companion itself was not replaced with a web mockup.

## Capture recipe

1. Build the current UI with `bun run build:ui` or run the Desktop dev host.
2. Use only the isolated data root `%TEMP%\nomifun-doc-desktop` (or the
   equivalent path under the current user profile). Never use production data
   or real credentials.
3. Seed synthetic Canvas, asset, template, and companion records through the
   current UI/API, then capture visible product routes with Puppeteer/Chrome or
   the running Tauri app.
4. For a desktop companion, confirm
   `appearance.companion_enabled=true`, find the native
   `companion-<companion_id>` window, and capture its own transparent window
   rectangle. Do not use a management-page thumbnail as a substitute.
5. Verify every expected PNG is non-empty, resolve every Markdown reference,
   and run `git diff --check` before committing.

## Older guide captures

The existing `autowork-*`, `channels-*`, `cron-*`, `gs-*`, `mcp-*`, `terminal-*`,
and `webui-*` files remain only where a technical guide still references them.
They are not part of the Creative Studio gallery. When a Portal walkthrough
supersedes one, remove the old file and update its references instead of
keeping duplicate aliases.
