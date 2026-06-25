# Quick Start

A short walkthrough for the first useful NomiFun session. This assumes the repo
is already installed; otherwise start with [Installation](installation.md).

## 1. Launch

Desktop development:

```bash
bun run dev
```

The desktop shell starts the backend on a private loopback port and injects a
local trust token into its own webview. There is no desktop login screen.

Web server:

```bash
bun run serve:web
# open http://127.0.0.1:8787
```

The web host requires login by default. On a fresh data directory, the first
visitor creates the initial admin unless you pre-seed `NOMIFUN_ADMIN_PASSWORD`.

![First-run admin setup](../images/gs-04-quickstart-login.png)

## 2. Start From `/guid`

After auth, the app opens `/guid`. This is the default session start surface.

You can choose:

- an agent/runtime,
- a model/provider when the runtime supports it,
- an assistant persona,
- skills or MCP tools,
- a workspace path,
- and the first prompt.

![The /guid landing page](../images/gs-05-quickstart-guid.png)

## 3. Configure A Model

Open **Models** (`/models`) and configure at least one provider or detected CLI
agent. The page owns provider credentials, agent availability, and global
reliability settings such as IDMM and model failover.

![Model settings](../images/gs-06-quickstart-model-settings.png)

For the simplest first run, use the built-in Nomi engine with an API provider
you have credentials for. External CLIs such as Claude Code, Codex, Gemini CLI,
Qwen, and OpenCode must also be installed on the host before NomiFun can use
them.

## 4. Send The First Message

Back on `/guid`:

1. Choose an agent.
2. Choose a model if required.
3. Optionally choose an assistant.
4. Type a prompt.
5. Send with the button or `Ctrl/Cmd+Enter`.

NomiFun creates a conversation and navigates to `/conversation/<id>`.

## 5. Use The Workspace

Each conversation can use a working directory. Inside a conversation you can
inspect messages, tool calls, file edits, previews, and terminal sessions.

Useful next pages:

- [Terminal](../guides/terminal.md)
- [MCP & Skills](../guides/mcp-and-skills.md)
- [Assistants](../guides/assistants.md)
- [AutoWork & Requirements](../guides/autowork-requirements.md)
- [Scheduled Tasks](../guides/scheduled-tasks.md)
- [Web Server Deployment](../guides/web-server-deployment.md)
- [Architecture Overview](../architecture/overview.md)
