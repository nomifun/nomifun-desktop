# Screenshot Manifest

This manifest records the repository-local screenshots used by the Desktop
README and technical guides. The current set was captured on **August 25, 2026**
from the 0.7.2 codebase with an isolated data root.

The Creation set covers Canvas Library, the retired Image and Video
Workbenches (kept as historical captures), Prompt Center, My Assets, Template
Studio, Template Editor, and a visible native desktop companion.
Do not restore retired screenshots or introduce temporary aliases into the
numbered gallery.

## Ownership and storage

- Product-use guides and their screenshots are self-contained in this
  repository (`docs/guides/` + `docs/images/`).
- Repository-local images are intentional so README pages remain readable
  offline. Do not replace them with external image URLs.

## README showcase

| File | Current surface |
| --- | --- |
| `readme/en/workspace.png` / `readme/zh/workspace.png` | Current Desktop workspace and session hub |
| `readme/en/models.png` / `readme/zh/models.png` | Model Management and provider model configuration |
| `readme/en/companions.png` / `readme/zh/companions.png` | Current workspace with the live desktop companion visible |
| `readme/en/skills.png` / `readme/zh/skills.png` | Skills Hub with Creation skills |

## Creation gallery

English captures live under `creative-studio/en-US/`; Chinese captures live
under `creative-studio/zh-CN/`. Both locale sets use the same route order:

| File | Route / subject |
| --- | --- |
| `01-canvas-library.png` | `#/nomi/canvases` · Canvas Library |
| `03-image-workbench.png` | retired standalone Image Workbench (historical capture; the route no longer exists) |
| `04-video-workbench.png` | retired standalone Video Workbench (historical capture; the route no longer exists) |
| `05-prompt-center.png` | `#/asset-library/prompts` · searchable Prompt Center |
| `06-asset-library.png` | `#/asset-library/materials` · My Assets and reusable inputs |
| `07-template-studio.png` | `#/asset-library/templates` · private Template Studio, including multi-image series setup |
| `08-template-editor.png` | Template Editor and bounded AI Create review flow |
| `11-companion-settings.png` | Companion workspace with figure, persona, model, memory, Skills, and desktop visibility control |
| `12-companion-workspace.png` | Companion surface kept visible beside the creative workspace |

The Creation captures use a 1440×900 viewport. The companion images were
captured from the running companion-enabled product surface and the native
transparent companion window. The numbered
`11-companion-settings.png` and `12-companion-workspace.png` captures are the
gallery references. Neither is a
management-page thumbnail substituted for the native companion experience.

The companion settings capture shows the desktop-visibility toggle, while the
companion workspace capture shows the companion surface alongside the product.
The separate top-level
`readme/en/skills.png` and `readme/zh/skills.png` captures document the
reusable Skills Hub packages; there is no numbered `11-creative-skills.png`
gallery asset.

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
   rectangle. For the numbered gallery, also capture the companion workspace
   state at `12-companion-workspace.png`. Do not use a management-page
   thumbnail as a substitute.
5. Verify every expected PNG is non-empty, resolve every Markdown reference,
   and run `git diff --check` before committing.

## Older guide captures

The existing `autowork-*`, `channels-*`, `cron-*`, `gs-*`, `mcp-*`, `terminal-*`,
and `webui-*` files remain only where a technical guide still references them.
They are not part of the Creation gallery. When a guide stops needing one,
remove the old file and update its references instead of keeping duplicate
aliases.
