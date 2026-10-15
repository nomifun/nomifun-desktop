# NomiFun documentation

[简体中文](README.zh.md) · [Project README](../README.md) · [Published mirror](https://www.nomifun.com/docs/)

This is the maintained documentation for the current Desktop product and its desktop-class WebUI. Start with a task below. Product guides live here; NomiFun Portal publishes a mirror. Implementation facts belong to the current source and stable architecture pages.

## Get started

- [Introduction](getting-started/introduction.md)
- [Installation](getting-started/installation.md)
- [Quick start](getting-started/quick-start.md)
- [Product ecosystem](architecture/product-ecosystem.md)

## Use and operate the product

- [Desktop app](guides/desktop-app.md)
- [Models and routing](guides/model-routing.md)
- [Agents and presets](guides/presets.md)
- [Creative Studio](guides/creative-studio.md)
- [Application hosting](guides/app-hosting.md)
- [Companions](guides/companions.md)
- [XiaoZhi robot](guides/xiaozhi-robot.md)
- [MCP and Skills](guides/mcp-and-skills.md)
- [Computer / Browser Use](guides/computer-browser-use.md)
- [Terminal](guides/terminal.md)
- [SSH sessions](guides/ssh-sessions.md)
- [Requirements / AutoWork](guides/autowork-requirements.md)
- [Intelligent decision (IDMM)](guides/intelligent-decision.md)
- [Scheduled tasks](guides/scheduled-tasks.md)
- [Channels](guides/channels.md)
- [WebUI remote access](guides/webui-remote-access.md)
- [Remote capability API](guides/remote-capability-api.md)
- [Web server deployment](guides/web-server-deployment.md)

## Understand the architecture

- [System overview](architecture/overview.md)
- [Backend crates](architecture/backend-crates.md)
- [Frontend](architecture/frontend.md)
- [Agent engine](architecture/agent-engine.md)
- [Canonical Agent Session](architecture/agent-session.md)
- [Agent execution (Chinese)](architecture/agent-execution.zh.md)
- [Model providers](architecture/model-providers.md)
- [Voice](architecture/voice.md)
- [Communication](architecture/communication.md)
- [Channel admission](architecture/channels.md)
- [Browser platform](architecture/browser-platform.md)
- [Application hosting](architecture/app-hosting.md)
- [Data and storage](architecture/data-and-storage.md)
- [ID system](architecture/id-system.md)
- [External tools](architecture/external-tools.md)
- [External knowledge MCP (Chinese)](architecture/external-knowledge-mcp.zh.md)

## Develop, validate and release

- [Contributing](../CONTRIBUTING.md)
- [Repository instructions](../AGENTS.md)
- [Project structure](contributing/project-structure.md)
- [Development and checks](contributing/development.md)
- [Styling contract](contributing/styling.md)
- [Data and identifier standards](contributing/data-and-identifier-standards.md)
- [Building and packaging](contributing/building-and-packaging.md)
- [Release process](../RELEASING.md)
- [Signing (Chinese)](../apps/desktop/signing/README.md)
- [Updater](../apps/desktop/updater/README.md)
- [Linux service deployment (Chinese)](../packaging/linux/README.md)
- [Browser validation](validation/browser-platform.md)
- [Connectivity stack (Chinese)](validation/connectivity-stack.md)
- [Screenshot manifest](images/SCREENSHOTS.md)
- [External agent integration Skill](skills/drive-nomifun/SKILL.md)

## Look up a detail

- [Configuration](reference/configuration.md)
- [API overview](reference/api-overview.md)
- [Troubleshooting](reference/troubleshooting.md)
- [FAQ](reference/faq.md)
- [Changelog](../CHANGELOG.md)
- [Security](../SECURITY.md)
- [Code of conduct](../CODE_OF_CONDUCT.md)

## Maintain these docs

- Keep user procedures in `guides/`, current contracts in `architecture/`, configuration/API references in `reference/`, contributor workflows in `contributing/`, and repeatable acceptance methods with explicit coverage gaps in `validation/`. Extend the existing owner page first.
- Update English and Simplified Chinese siblings together. Chinese-only technical pages are marked in this index; do not link missing translations.
- Link source entry points instead of copying crate counts, route inventories or generated schemas. Routes come from [Router.tsx](../ui/src/renderer/components/layout/Router.tsx); commands come from [package.json](../package.json) and `bun run help`.
- The shared Desktop renderer supports Tauri and desktop browsers with an **880×600** minimum. The separate Mobile client and its wire contracts do not define renderer layouts.
- When retiring a design, merge its still-valid decisions into the owner page, repair consumers and remove plans, reports and unused assets together. Runtime-embedded guidance, shipped Skills, fixtures, vendor documents and license notices have separate consumers; inspect them before changing Markdown files.
- Retain historical plans, handoffs, status snapshots and per-attempt evidence in **Git history**. Use `git log --all -- <path>` and `git show <commit>:<path>` to retrieve them. Do not create an archive tree that can be mistaken for current requirements. Keep actionable unresolved items in their current owner or validation page with accurate evidence limits.
- Run `bun run check:docs` for local links, images and Markdown anchors, then `git diff --check`. Documentation-only edits do not require a full build; code or contract changes require the affected checks from [AGENTS.md](../AGENTS.md).
