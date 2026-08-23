# Screenshot Manifest

This file tracks the user-facing screenshots used by the Desktop documentation.
On 2026-08-15, the complete active set was freshly captured from the current
`nomifun-desktop` checkout. The two Creative Studio README captures were
replaced again on 2026-08-23 from the production SPA and an isolated data root.
These are real running-app captures, not mockups or copies of the retired
product UI. Do not restore retired screenshots.

## Naming scheme

```text
<module-prefix>-<NN>-<slug>.png
```

| Prefix | Owner doc area |
| --- | --- |
| `gs-` | `getting-started/` |
| `desktop-` | `guides/desktop-app` |
| `webserver-` | `guides/web-server-deployment` |
| `webui-` | `guides/webui-remote-access` |
| `terminal-` | `guides/terminal` |
| `autowork-` | `guides/autowork-requirements` |
| `cron-` | `guides/scheduled-tasks` |
| `channels-` | `guides/channels` |
| `mcp-` | `guides/mcp-and-skills` |
| `readme-` | root README showcase |

## Current capture process

The desktop app and `nomifun-web` render the same production SPA (`ui/dist`).
Capture app content from a loopback-only web host with an isolated data
directory, then synchronize the verified files to Desktop and Portal.

1. Build the current SPA with `bun run build:ui`.
2. Start an isolated no-auth host:

   ```powershell
   cargo run -p nomifun-web --features static-webui -- `
     --host 127.0.0.1 --port 8799 --dist ui/dist `
     --data-dir $env:TEMP/nomifun-doc-captures --insecure-no-auth
   ```

3. Create only synthetic demo content through the visible current UI, such as
   a requirement, a companion, and a terminal session. Never use the normal
   NomiFun data directory or real credentials.
4. Capture current routes in the Codex in-app browser at `1280x720`, including
   list, dialog, settings, model, skill, companion, terminal, Mini App and
   Creative Studio views.
5. For authentication screens, start a second isolated host without
   `--insecure-no-auth` and capture exactly what the current SPA renders.
6. Before copying, verify every expected file is non-empty. Afterwards compare
   hashes across Desktop and Portal, resolve all Markdown/HTML image references,
   and run both repositories' normal builds/checks.

## Fresh synchronized guide set

| File | Current screen |
| --- | --- |
| `autowork-01-tag-sessions.png` | AutoWork session binding |
| `autowork-02-list.png` | Requirements list |
| `autowork-03-kanban.png` | Requirements board |
| `autowork-05-webhook-binding.png` | Notify and webhook binding |
| `channels-01-overview.png` | Companion remote-channel overview |
| `channels-02-pairing.png` | Current channel connection dialog |
| `cron-01-list.png` | Scheduled Tasks list |
| `cron-02-create-dialog.png` | Create scheduled task dialog |
| `gs-03-web-first-run-setup.png` | Current WebUI authentication entry |
| `gs-04-quickstart-login.png` | Current WebUI authentication entry |
| `gs-05-quickstart-guid.png` | Current home and session page |
| `gs-06-quickstart-model-settings.png` | Current Models and Agents management |
| `mcp-01-capabilities.png` | Current MCP Hub |
| `mcp-03-skills.png` | Current Skills Hub |
| `terminal-01-session.png` | Live in-app terminal session |
| `terminal-02-create-page.png` | Terminal creation page |
| `webserver-02-first-run-setup.png` | Current WebUI authentication entry |
| `webui-01-settings-overview.png` | Remote and Open capabilities settings |
| `webui-03-login-screen.png` | Current remote-browser authentication entry |
| `webui-04-cross-device.png` | Current authenticated remote workspace |

The retired duplicate files `autowork-04-tag-sessions.png` and
`gs-01-introduction-hero.png` were removed from Portal rather than preserved as
hidden old-image aliases. `cron-03-detail.png` has no current product capture;
the guide uses text instead of an obsolete visual.

## README and product gallery set

- `readme/en/` and `readme/zh/` contain current workspace, models, companions,
  skills, and Creative Studio captures. The Creative Studio images show the
  canonical Canvas editor with `返回画布库`, four real persisted nodes, and no
  Project product navigation. The English image is intentionally unedited: its
  focused shell follows English while the Canvas editor body still exposes the
  documented Simplified-Chinese limitation.
- `getting-started/en/` and `getting-started/zh/` contain current home captures.
- Portal copies must be synchronized and hash-checked separately before a
  Portal publication; this manifest does not claim that synchronization.
- `desktop-01-main-window.png` shows the current application content. It does
  not claim to document platform-specific native titlebar chrome.

All screenshots are repository-local so the documentation remains usable
offline. Do not replace them with externally hosted image URLs.
